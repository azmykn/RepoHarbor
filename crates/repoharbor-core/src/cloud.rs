//! Optional OpenAI-compatible cloud backend for AI generation.
//!
//! One `/chat/completions` call covers Groq, OpenAI, OpenRouter, Together,
//! DeepSeek and local OpenAI-shaped servers (LM Studio, vLLM) — the user picks
//! the base URL and model in Settings. This exists purely for *speed*: a hosted
//! 8–70B model drafts a commit message in well under a second where a tiny
//! local model takes tens of seconds. Local backends stay the default.
//!
//! Egress rules, mirroring the GitLab token discipline in `forge.rs`:
//! the API key is only ever sent to the base URL the user configured, and that
//! URL must be `https` unless it is loopback. Generation only — embeddings stay
//! on the Ollama path, so semantic search never leaves the machine.

use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;

/// Filename of the API key under the app data dir. Kept out of `config.toml`
/// (which users paste into issues) and written owner-only, like the GitHub token.
const KEY_FILE: &str = "openai_api_key";

/// Overall request timeout. Unlike the local backends — where a slow CPU
/// generation must not be cut off — a hosted endpoint that hasn't answered in
/// this long is wedged, and the UI is waiting on it.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(45);

fn key_path() -> Option<PathBuf> {
    crate::paths::data_dir().map(|d| d.join(KEY_FILE))
}

/// The stored API key, if any (data dir, with the legacy-path read fallback).
pub fn stored_api_key() -> Option<String> {
    crate::paths::resolve_data_file(KEY_FILE)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// The API key to authenticate with: the stored file, else
/// `$REPOHARBOR_OPENAI_API_KEY`.
pub fn api_key() -> Option<String> {
    stored_api_key().or_else(|| {
        std::env::var("REPOHARBOR_OPENAI_API_KEY")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    })
}

/// Is a key configured at all? Used to gate the backend without reading it.
pub fn has_api_key() -> bool {
    api_key().is_some()
}

/// Persist `key` owner-only; an empty/whitespace value deletes the stored key
/// (the env var, if set, still applies).
pub fn store_api_key(key: &str) -> Result<(), String> {
    let path = key_path().ok_or("no data directory")?;
    let key = key.trim();
    if key.is_empty() {
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| e.to_string())?;
        }
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    // Create owner-only from the start (no umask race) — the key is a secret.
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)
            .map_err(|e| e.to_string())?;
        file.write_all(key.as_bytes()).map_err(|e| e.to_string())?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(&path, key).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Loopback hosts may use plain `http` (a local OpenAI-shaped server).
fn host_is_loopback(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host == "localhost" || host == "127.0.0.1" || host == "::1" || host.ends_with(".localhost")
}

/// Validate and normalize a configured base URL, returning it without a
/// trailing slash. Pure so the egress rule is unit-tested rather than trusted:
/// a key must never travel over plain http to a remote host.
pub fn normalize_base(raw: &str) -> Result<String, String> {
    let base = raw.trim().trim_end_matches('/');
    if base.is_empty() {
        return Err("set an AI base URL in Settings (e.g. a Groq or OpenAI endpoint)".into());
    }
    let (scheme, rest) = base
        .split_once("://")
        .ok_or("AI base URL must start with https://")?;
    let host = rest
        .split('/')
        .next()
        .unwrap_or("")
        .rsplit('@')
        .next()
        .unwrap_or("");
    let host_only = host.rsplit_once(':').map_or(host, |(h, _)| h);
    if host_only.is_empty() {
        return Err("AI base URL has no host".into());
    }
    match scheme {
        "https" => Ok(base.to_string()),
        "http" if host_is_loopback(host_only) => Ok(base.to_string()),
        "http" => {
            Err("AI base URL must use https (plain http is only allowed for localhost)".into())
        }
        other => Err(format!("unsupported AI base URL scheme `{other}`")),
    }
}

/// The configured, validated base URL.
fn base() -> Result<String, String> {
    normalize_base(&crate::config::load().openai_base)
}

/// Shared HTTP client with an overall timeout (see [`REQUEST_TIMEOUT`]).
fn client() -> reqwest::Client {
    static CLIENT: std::sync::LazyLock<reqwest::Client> = std::sync::LazyLock::new(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(8))
            .timeout(REQUEST_TIMEOUT)
            .build()
            .unwrap_or_default()
    });
    CLIENT.clone()
}

/// Is the configured endpoint reachable and authenticated?
///
/// `GET /models` is the one endpoint every OpenAI-compatible provider serves
/// cheaply. A 401 counts as *unreachable* — a wrong key would fail every
/// generation, so the UI should hide AI rather than toast on each call.
pub async fn available() -> bool {
    let (Ok(base), Some(key)) = (base(), api_key()) else {
        return false;
    };
    client()
        .get(format!("{base}/models"))
        .bearer_auth(key)
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// Model ids offered by the endpoint, as (name, 0) to match the
/// `installed_models` shape (hosted models have no local size).
pub async fn models() -> Vec<(String, u64)> {
    #[derive(Deserialize)]
    struct List {
        #[serde(default)]
        data: Vec<Model>,
    }
    #[derive(Deserialize)]
    struct Model {
        #[serde(default)]
        id: String,
    }
    let (Ok(base), Some(key)) = (base(), api_key()) else {
        return Vec::new();
    };
    let resp = client()
        .get(format!("{base}/models"))
        .bearer_auth(key)
        .send()
        .await;
    let mut names: Vec<(String, u64)> = match resp {
        Ok(r) if r.status().is_success() => match r.json::<List>().await {
            Ok(l) => l
                .data
                .into_iter()
                .map(|m| m.id)
                .filter(|id| !id.is_empty())
                .map(|id| (id, 0))
                .collect(),
            Err(_) => Vec::new(),
        },
        _ => Vec::new(),
    };
    names.sort();
    names
}

/// One completion attempt. `(text, reasoned)` — `reasoned` is true when the
/// model spent tokens on a hidden reasoning channel, which is why `text` may be
/// empty even on a 200.
struct Attempt {
    text: String,
    reasoned: bool,
}

/// Does a 4xx body complain about `reasoning_effort` specifically? Providers
/// that don't know the field reject the whole request, so the retry has to tell
/// that apart from a real error (bad model, no credit). Pure for testing.
fn rejects_reasoning_effort(status: u16, body: &str) -> bool {
    (400..500).contains(&status) && body.to_lowercase().contains("reasoning_effort")
}

async fn chat_once(
    base: &str,
    key: &str,
    model: &str,
    prompt: &str,
    max_tokens: u32,
    low_reasoning: bool,
) -> Result<Attempt, (u16, String)> {
    #[derive(Deserialize)]
    struct Resp {
        #[serde(default)]
        choices: Vec<Choice>,
    }
    #[derive(Deserialize)]
    struct Choice {
        #[serde(default)]
        message: Message,
    }
    #[derive(Default, Deserialize)]
    struct Message {
        #[serde(default)]
        content: String,
        /// Non-standard, but what Ollama's OpenAI-compatible endpoint returns
        /// for thinking models (gpt-oss et al) — and where an otherwise empty
        /// `content` went.
        #[serde(default)]
        reasoning: Option<String>,
    }

    let mut body = serde_json::json!({
        "model": model,
        "temperature": 0.2,
        "max_tokens": max_tokens,
        "messages": [{ "role": "user", "content": prompt }],
    });
    if low_reasoning {
        body["reasoning_effort"] = serde_json::json!("low");
    }
    let resp = client()
        .post(format!("{base}/chat/completions"))
        .bearer_auth(key)
        .json(&body)
        .send()
        .await
        .map_err(|e| (0, e.to_string()))?;
    let status = resp.status();
    if !status.is_success() {
        return Err((status.as_u16(), api_error(resp).await));
    }
    let parsed: Resp = resp.json().await.map_err(|e| (0, e.to_string()))?;
    let message = parsed
        .choices
        .into_iter()
        .next()
        .map(|c| c.message)
        .unwrap_or_default();
    Ok(Attempt {
        text: message.content.trim().to_string(),
        reasoned: message.reasoning.is_some_and(|r| !r.trim().is_empty()),
    })
}

/// Generate a completion via `/chat/completions`.
///
/// Thinking models (gpt-oss on Ollama Cloud, the Gemini 3 family, …) will
/// happily spend the whole token budget on a hidden reasoning channel and
/// return an empty `content` — the same failure `think:false` avoids on the
/// native Ollama path. So the first attempt asks for `reasoning_effort: "low"`
/// (measured on Ollama Cloud's `gpt-oss:20b`: a usable subject in ~2s instead
/// of ~15s, or nothing at all on a short budget). Providers that don't know the
/// field reject the request outright, so that specific rejection retries once
/// without it.
pub async fn generate(model: &str, prompt: &str, max_tokens: u32) -> Result<String, String> {
    let base = base()?;
    let key = api_key().ok_or("no AI API key — add one in Settings → AI & search")?;
    let model = model.trim();
    if model.is_empty() {
        return Err("set an AI model in Settings (e.g. llama-3.1-8b-instant)".into());
    }
    let attempt = match chat_once(&base, &key, model, prompt, max_tokens, true).await {
        Ok(a) => a,
        Err((status, err)) if rejects_reasoning_effort(status, &err) => {
            chat_once(&base, &key, model, prompt, max_tokens, false)
                .await
                .map_err(|(_, e)| e)?
        }
        Err((_, err)) => return Err(err),
    };
    if attempt.text.is_empty() {
        return Err(if attempt.reasoned {
            format!(
                "model {model} spent its whole token budget thinking and returned no answer — pick a non-reasoning model in Settings"
            )
        } else {
            format!("model {model} returned an empty response — try another model in Settings")
        });
    }
    Ok(attempt.text)
}

/// Readable error from a non-2xx response, surfacing the provider's
/// `{"error": {"message": …}}` body instead of a bare status code. The key is
/// never echoed back.
async fn api_error(resp: reqwest::Response) -> String {
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    let detail = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| {
            v.get("error")
                .and_then(|e| {
                    e.get("message")
                        .and_then(|m| m.as_str())
                        .or_else(|| e.as_str())
                })
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.trim().chars().take(300).collect());
    if detail.is_empty() {
        format!("AI API {status}")
    } else {
        format!("AI API {status}: {detail}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_base_requires_https_off_loopback() {
        assert_eq!(
            normalize_base("https://api.groq.com/openai/v1/").unwrap(),
            "https://api.groq.com/openai/v1"
        );
        // Local OpenAI-shaped servers may stay on plain http.
        assert_eq!(
            normalize_base("http://localhost:1234/v1").unwrap(),
            "http://localhost:1234/v1"
        );
        assert!(normalize_base("http://127.0.0.1:8000/v1").is_ok());
        // A remote plain-http endpoint would leak the key in clear text.
        assert!(normalize_base("http://api.example.com/v1").is_err());
        assert!(normalize_base("ftp://example.com/v1").is_err());
        assert!(normalize_base("api.groq.com/openai/v1").is_err());
        assert!(normalize_base("   ").is_err());
    }

    #[test]
    fn only_a_reasoning_effort_complaint_triggers_the_retry() {
        assert!(rejects_reasoning_effort(
            400,
            "AI API 400: unknown parameter 'reasoning_effort'"
        ));
        // Real failures must not be retried without the field.
        assert!(!rejects_reasoning_effort(402, "requires a subscription"));
        assert!(!rejects_reasoning_effort(401, "invalid api key"));
        assert!(!rejects_reasoning_effort(404, "model not found"));
        assert!(!rejects_reasoning_effort(500, "reasoning_effort"));
    }
}
