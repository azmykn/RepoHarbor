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

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde::Deserialize;

static COOLDOWN_REGISTRY: LazyLock<Mutex<HashMap<String, Instant>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Remaining cooldown for `endpoint_key`, if still active.
pub fn cooldown_remaining(endpoint_key: &str) -> Option<Duration> {
    let mut reg = COOLDOWN_REGISTRY.lock().unwrap_or_else(|p| p.into_inner());
    let expires = reg.get(endpoint_key).copied()?;
    let now = Instant::now();
    if now < expires {
        Some(expires - now)
    } else {
        reg.remove(endpoint_key);
        None
    }
}

/// Check if an endpoint/model key is currently in temporary cooldown due to rate limit/quota failure.
pub fn is_in_cooldown(endpoint_key: &str) -> bool {
    cooldown_remaining(endpoint_key).is_some()
}

/// Place an endpoint/model in cooldown for the specified duration.
pub fn set_cooldown(endpoint_key: &str, duration: Duration) {
    let mut reg = COOLDOWN_REGISTRY.lock().unwrap_or_else(|p| p.into_inner());
    reg.insert(endpoint_key.to_string(), Instant::now() + duration);
}

/// Clear all active cooldowns.
pub fn clear_cooldowns() {
    let mut reg = COOLDOWN_REGISTRY.lock().unwrap_or_else(|p| p.into_inner());
    reg.clear();
}

/// Non-secret FNV-1a fingerprint so distinct keys on the same provider/model
/// never share a cooldown bucket or leak key bytes into logs.
fn key_fingerprint(key: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in key.trim().as_bytes() {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// Stable cooldown bucket key for an active endpoint candidate.
pub fn endpoint_cooldown_key(base_url: &str, model: &str, api_key: &str) -> String {
    format!("{base_url}:{model}:{:016x}", key_fingerprint(api_key))
}

/// HTTP status codes that trigger temporary 60s cooldown isolation.
pub(crate) fn should_cooldown(status: u16) -> bool {
    matches!(status, 402 | 429 | 503)
}

/// Failure status codes (including `0` for transport/timeout) that should
/// advance to the next deployment instead of aborting.
pub(crate) fn should_failover(status: u16) -> bool {
    matches!(status, 0 | 401..=404 | 408 | 429 | 500..=599)
}

/// Filename of the API key under the app data dir. Kept out of `config.toml`
/// (which users paste into issues) and written owner-only, like the GitHub token.
const KEY_FILE: &str = "openai_api_key";
const OPENROUTER_KEY_FILE: &str = "openrouter_api_key";
const GROQ_KEY_FILE: &str = "groq_api_key";
const MISTRAL_KEY_FILE: &str = "mistral_api_key";

pub const OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";
pub const GROQ_BASE_URL: &str = "https://api.groq.com/openai/v1";
pub const MISTRAL_BASE_URL: &str = "https://api.mistral.ai/v1";
pub const CEREBRAS_BASE_URL: &str = "https://api.cerebras.ai/v1";
pub const DEEPSEEK_BASE_URL: &str = "https://api.deepseek.com/v1";
pub const TOGETHER_BASE_URL: &str = "https://api.together.xyz/v1";
pub const OLLAMA_OPENAI_BASE_URL: &str = "http://localhost:11434/v1";
pub const OPENAI_BASE_URL: &str = "https://api.openai.com/v1";

/// True when the configured backend is OpenRouter.
pub fn is_openrouter_backend() -> bool {
    let backend = crate::config::load().ai_backend;
    matches!(
        backend.as_str(),
        "openrouter" | "open_router" | "OpenRouter"
    )
}

/// True when the configured backend is Groq.
pub fn is_groq_backend() -> bool {
    let backend = crate::config::load().ai_backend;
    matches!(backend.as_str(), "groq" | "Groq")
}

/// True when the configured backend is Mistral.
pub fn is_mistral_backend() -> bool {
    let backend = crate::config::load().ai_backend;
    matches!(backend.as_str(), "mistral" | "Mistral")
}

pub fn active_target() -> &'static str {
    if is_openrouter_backend() {
        "openrouter"
    } else if is_groq_backend() {
        "groq"
    } else if is_mistral_backend() {
        "mistral"
    } else {
        "cloud"
    }
}

/// Overall request timeout. Unlike the local backends — where a slow CPU
/// generation must not be cut off — a hosted endpoint that hasn't answered in
/// this long is wedged, and the UI is waiting on it.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(45);

fn key_path_for(target: &str) -> Option<PathBuf> {
    let file = match target {
        "openrouter" => OPENROUTER_KEY_FILE,
        "groq" => GROQ_KEY_FILE,
        "mistral" => MISTRAL_KEY_FILE,
        _ => KEY_FILE,
    };
    crate::paths::data_dir().map(|d| d.join(file))
}

/// Split key string by newlines, commas, or semicolons to support multiple keys.
pub fn parse_key_pool(raw: &str) -> Vec<String> {
    raw.split(['\n', ',', ';'])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Return all active keys for a named target ("groq", "mistral", "openrouter", "cloud").
pub fn keys_for_target(target: &str) -> Vec<String> {
    let mut keys = Vec::new();

    let file_name = match target {
        "openrouter" => Some(OPENROUTER_KEY_FILE),
        "groq" => Some(GROQ_KEY_FILE),
        "mistral" => Some(MISTRAL_KEY_FILE),
        "cloud" => Some(KEY_FILE),
        _ => None,
    };
    if let Some(f) = file_name {
        if let Some(content) =
            crate::paths::resolve_data_file(f).and_then(|p| std::fs::read_to_string(p).ok())
        {
            keys.extend(parse_key_pool(&content));
        }
    }

    let env_vars: &[&str] = match target {
        "groq" => &["GROQ_API_KEY", "REPOHARBOR_GROQ_API_KEY"],
        "mistral" => &["MISTRAL_API_KEY", "REPOHARBOR_MISTRAL_API_KEY"],
        "openrouter" => &["OPENROUTER_API_KEY", "REPOHARBOR_OPENROUTER_API_KEY"],
        "cloud" => &["OPENAI_API_KEY", "REPOHARBOR_OPENAI_API_KEY"],
        _ => &[],
    };
    for var in env_vars {
        if let Ok(val) = std::env::var(var) {
            keys.extend(parse_key_pool(&val));
        }
    }

    let mut deduped = Vec::new();
    for k in keys {
        if !deduped.contains(&k) {
            deduped.push(k);
        }
    }
    deduped
}

/// The stored API key, if any (data dir, with the legacy-path read fallback).
pub fn stored_api_key() -> Option<String> {
    keys_for_target(active_target()).into_iter().next()
}

/// The API key to authenticate with: the stored file, else
/// environment variables.
pub fn api_key() -> Option<String> {
    stored_api_key()
}

/// Is a key configured at all? Used to gate the backend without reading it.
pub fn has_api_key() -> bool {
    !keys_for_target(active_target()).is_empty()
}

/// Persist `key` owner-only; an empty/whitespace value deletes the stored key
/// (the env var, if set, still applies).
pub fn store_api_key(key: &str) -> Result<(), String> {
    store_api_key_for_target(key, active_target())
}

/// Persist `key` for either OpenRouter or the generic Cloud backend.
pub fn store_api_key_for(key: &str, is_openrouter: bool) -> Result<(), String> {
    store_api_key_for_target(key, if is_openrouter { "openrouter" } else { "cloud" })
}

/// Persist `key` for a named backend target ("openrouter", "groq", "mistral", or generic "cloud").
pub fn store_api_key_for_target(key: &str, target: &str) -> Result<(), String> {
    let path = key_path_for(target).ok_or("no data directory")?;
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

/// Extract host (without port) from a `scheme://host[:port][/...]` URL.
fn extract_host_only(url: &str) -> Option<&str> {
    let (_, rest) = url.split_once("://")?;
    let authority = rest
        .split('/')
        .next()
        .unwrap_or("")
        .rsplit('@')
        .next()
        .unwrap_or("");
    if authority.starts_with('[') {
        authority
            .find(']')
            .map(|end| &authority[..=end])
            .filter(|h| !h.is_empty())
    } else {
        let h = authority.rsplit_once(':').map_or(authority, |(h, _)| h);
        (!h.is_empty()).then_some(h)
    }
}

/// True when `url` points at a loopback host (`localhost`, `127.0.0.1`, `::1`).
pub fn url_is_loopback(url: &str) -> bool {
    extract_host_only(url.trim()).is_some_and(host_is_loopback)
}

/// Validate and normalize a configured base URL, returning it without a
/// trailing slash. Pure so the egress rule is unit-tested rather than trusted:
/// a key must never travel over plain http to a remote host.
pub fn normalize_base(raw: &str) -> Result<String, String> {
    let base = raw.trim().trim_end_matches('/');
    if base.is_empty() {
        return Err("set an AI base URL in Settings (e.g. a Groq or OpenAI endpoint)".into());
    }
    let (scheme, _) = base
        .split_once("://")
        .ok_or("AI base URL must start with https://")?;
    let host_only = extract_host_only(base).ok_or("AI base URL has no host")?;
    match scheme {
        "https" => Ok(base.to_string()),
        "http" if host_is_loopback(host_only) => Ok(base.to_string()),
        "http" => {
            Err("AI base URL must use https (plain http is only allowed for localhost)".into())
        }
        other => Err(format!("unsupported AI base URL scheme `{other}`")),
    }
}

/// Canonical provider key (`groq`, `mistral`, `openrouter`, `ollama`, `cloud`, …)
/// inferred from an explicit provider label or URL.
pub fn infer_provider_id(name_or_provider: &str, base_url: &str) -> String {
    let combined = format!(
        "{} {}",
        name_or_provider.to_ascii_lowercase(),
        base_url.to_ascii_lowercase()
    );
    if combined.contains("groq") {
        "groq".into()
    } else if combined.contains("mistral") || combined.contains("codestral") {
        "mistral".into()
    } else if combined.contains("openrouter") {
        "openrouter".into()
    } else if combined.contains("cerebras") {
        "cerebras".into()
    } else if combined.contains("deepseek") {
        "deepseek".into()
    } else if combined.contains("together") {
        "together".into()
    } else if combined.contains("ollama")
        || combined.contains("localhost:11434")
        || combined.contains("127.0.0.1:11434")
    {
        "ollama".into()
    } else {
        "cloud".into()
    }
}

/// Default OpenAI-compatible base URL for a known provider id/name.
pub fn default_base_for_provider(provider: &str) -> &'static str {
    match provider.trim().to_ascii_lowercase().as_str() {
        "groq" => GROQ_BASE_URL,
        "mistral" => MISTRAL_BASE_URL,
        "openrouter" => OPENROUTER_BASE_URL,
        "cerebras" => CEREBRAS_BASE_URL,
        "deepseek" => DEEPSEEK_BASE_URL,
        "together" => TOGETHER_BASE_URL,
        "ollama" => OLLAMA_OPENAI_BASE_URL,
        _ => OPENAI_BASE_URL,
    }
}

/// Default chat model for a known provider id/name.
pub fn default_model_for_provider(provider: &str) -> &'static str {
    match provider.trim().to_ascii_lowercase().as_str() {
        "groq" => "openai/gpt-oss-120b",
        "mistral" => "codestral-latest",
        "openrouter" => "deepseek/deepseek-chat",
        "cerebras" => "llama-3.3-70b",
        "deepseek" => "deepseek-chat",
        "together" => "meta-llama/Llama-3.3-70B-Instruct-Turbo",
        "ollama" => "qwen3:0.6b",
        _ => "gpt-4o-mini",
    }
}

/// The configured, validated base URL.
fn base() -> Result<String, String> {
    let cfg = crate::config::load();
    if is_openrouter_backend()
        && (cfg.openai_base.trim().is_empty()
            || cfg.openai_base.contains("api.groq.com")
            || cfg.openai_base.contains("mistral.ai"))
    {
        return Ok(OPENROUTER_BASE_URL.to_string());
    }
    if is_groq_backend()
        && (cfg.openai_base.trim().is_empty()
            || cfg.openai_base.contains("openrouter.ai")
            || cfg.openai_base.contains("mistral.ai")
            || cfg.openai_base.contains("ollama.com"))
    {
        return Ok(GROQ_BASE_URL.to_string());
    }
    if is_mistral_backend()
        && (cfg.openai_base.trim().is_empty()
            || cfg.openai_base.contains("api.groq.com")
            || cfg.openai_base.contains("openrouter.ai")
            || cfg.openai_base.contains("ollama.com"))
    {
        return Ok(MISTRAL_BASE_URL.to_string());
    }
    normalize_base(&cfg.openai_base)
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiCandidate {
    pub provider: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

fn push_unique_candidate(candidates: &mut Vec<ApiCandidate>, cand: ApiCandidate) {
    if !candidates
        .iter()
        .any(|c| c.base_url == cand.base_url && c.api_key == cand.api_key && c.model == cand.model)
    {
        candidates.push(cand);
    }
}

/// Resolve the normalized base URL and model name for a pool deployment row.
pub fn resolve_deployment_base_and_model(d: &crate::model::ModelDeployment) -> (String, String) {
    let prov = if d.provider.trim().is_empty() {
        infer_provider_id(&d.name, &d.base_url)
    } else {
        d.provider.clone()
    };
    let raw_base = if !d.base_url.trim().is_empty() {
        d.base_url.clone()
    } else {
        default_base_for_provider(&prov).to_string()
    };
    let model = if !d.model.trim().is_empty() {
        d.model.clone()
    } else {
        default_model_for_provider(&prov).to_string()
    };
    (raw_base, model)
}

/// Resolve credentials (`base_url`, `api_key`, `model`) for a single pool row
/// using stored per-row keys first, then legacy inline key, then provider keys.
pub fn resolve_deployment_credentials(
    d: &crate::model::ModelDeployment,
    pool_keys_map: &HashMap<String, String>,
) -> Result<(String, String, String), String> {
    let (raw_base, model) = resolve_deployment_base_and_model(d);
    let base_url = normalize_base(&raw_base)?;
    let prov = if d.provider.trim().is_empty() {
        infer_provider_id(&d.name, &base_url)
    } else {
        d.provider.clone()
    };
    let api_key = pool_keys_map
        .get(&d.id)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            let k = d.api_key.trim();
            (!k.is_empty()).then(|| k.to_string())
        })
        .or_else(|| keys_for_target(&prov).into_iter().next())
        .unwrap_or_default();
    if api_key.is_empty() && !url_is_loopback(&base_url) {
        return Err("no API key stored for this provider row or backend".into());
    }
    Ok((base_url, api_key, model))
}

/// Remaining cooldown duration for a deployment row (if currently rate-limited).
pub fn cooldown_remaining_for_deployment(
    d: &crate::model::ModelDeployment,
    pool_keys_map: &HashMap<String, String>,
) -> Option<Duration> {
    let (base_url, api_key, model) = resolve_deployment_credentials(d, pool_keys_map).ok()?;
    cooldown_remaining(&endpoint_cooldown_key(&base_url, &model, &api_key))
}

/// True when `config.model_pool` contains at least one enabled deployment.
pub fn has_enabled_pool() -> bool {
    crate::config::load().model_pool.iter().any(|d| d.enabled)
}

/// Build the candidate list purely from `cfg.model_pool` + optional legacy/fallback chain.
/// Pure so egress validation, loopback keyless allowance, and ordering are unit-tested.
pub fn build_candidates<F>(
    cfg: &crate::model::AppConfig,
    pool_keys_map: &HashMap<String, String>,
    preferred_model: &str,
    include_legacy_and_fallback: bool,
    mut provider_keys_lookup: F,
) -> Vec<ApiCandidate>
where
    F: FnMut(&str) -> Vec<String>,
{
    let mut candidates = Vec::new();

    // 0. Explicit model deployments from the model pool (ordered by priority).
    if !cfg.model_pool.is_empty() {
        let mut pool = cfg.model_pool.clone();
        pool.sort_by_key(|d| d.priority);
        for d in pool {
            if !d.enabled {
                continue;
            }
            let (raw_base, model) = resolve_deployment_base_and_model(&d);
            let Ok(base_url) = normalize_base(&raw_base) else {
                // Skip invalid or non-https remote URLs so keys never travel over cleartext.
                continue;
            };
            let prov = if d.provider.trim().is_empty() {
                infer_provider_id(&d.name, &base_url)
            } else {
                d.provider.clone()
            };
            let label = if d.name.trim().is_empty() {
                prov.clone()
            } else {
                d.name.clone()
            };

            let row_or_inline = pool_keys_map
                .get(&d.id)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    let k = d.api_key.trim();
                    (!k.is_empty()).then(|| k.to_string())
                });

            if let Some(raw_keys) = row_or_inline {
                for key in parse_key_pool(&raw_keys) {
                    push_unique_candidate(
                        &mut candidates,
                        ApiCandidate {
                            provider: label.clone(),
                            base_url: base_url.clone(),
                            api_key: key,
                            model: model.clone(),
                        },
                    );
                }
            } else {
                let keys = provider_keys_lookup(&prov);
                if keys.is_empty() {
                    if url_is_loopback(&base_url) {
                        push_unique_candidate(
                            &mut candidates,
                            ApiCandidate {
                                provider: label,
                                base_url,
                                api_key: String::new(),
                                model,
                            },
                        );
                    }
                } else {
                    for key in keys {
                        push_unique_candidate(
                            &mut candidates,
                            ApiCandidate {
                                provider: label.clone(),
                                base_url: base_url.clone(),
                                api_key: key,
                                model: model.clone(),
                            },
                        );
                    }
                }
            }
        }
    }

    if !include_legacy_and_fallback {
        return candidates;
    }

    let backend_lower = cfg.ai_backend.as_str();
    let (active_target, active_base, active_default_model) =
        if matches!(backend_lower, "mistral" | "Mistral") {
            ("mistral", MISTRAL_BASE_URL.to_string(), "codestral-latest")
        } else if matches!(backend_lower, "groq" | "Groq") {
            ("groq", GROQ_BASE_URL.to_string(), "openai/gpt-oss-120b")
        } else if matches!(backend_lower, "openrouter" | "open_router" | "OpenRouter") {
            (
                "openrouter",
                OPENROUTER_BASE_URL.to_string(),
                "deepseek/deepseek-chat",
            )
        } else {
            let base = if cfg.openai_base.trim().is_empty() {
                OPENAI_BASE_URL.to_string()
            } else {
                cfg.openai_base.clone()
            };
            ("cloud", base, "gpt-4o-mini")
        };

    let chosen_model = if preferred_model.trim().is_empty() {
        if !cfg.ai_model.trim().is_empty() {
            cfg.ai_model.clone()
        } else {
            active_default_model.to_string()
        }
    } else {
        preferred_model.to_string()
    };

    // 1. Primary provider keys (if base passes egress validation).
    if let Ok(valid_base) = normalize_base(&active_base) {
        let primary_keys = provider_keys_lookup(active_target);
        if primary_keys.is_empty() && url_is_loopback(&valid_base) {
            push_unique_candidate(
                &mut candidates,
                ApiCandidate {
                    provider: active_target.to_string(),
                    base_url: valid_base,
                    api_key: String::new(),
                    model: chosen_model.clone(),
                },
            );
        } else {
            for key in primary_keys {
                push_unique_candidate(
                    &mut candidates,
                    ApiCandidate {
                        provider: active_target.to_string(),
                        base_url: valid_base.clone(),
                        api_key: key,
                        model: chosen_model.clone(),
                    },
                );
            }
        }
    }

    // 2. Cross-provider fallback if failover enabled.
    if cfg.ai_failover_enabled {
        let providers: &[(&str, &str, &str)] = &[
            ("groq", GROQ_BASE_URL, "openai/gpt-oss-120b"),
            ("mistral", MISTRAL_BASE_URL, "codestral-latest"),
            ("openrouter", OPENROUTER_BASE_URL, "deepseek/deepseek-chat"),
            ("cloud", OPENAI_BASE_URL, "gpt-4o-mini"),
        ];

        for &(prov, default_base, default_model) in providers {
            if prov == active_target {
                continue;
            }
            let raw_base = if prov == "cloud" && !cfg.openai_base.trim().is_empty() {
                cfg.openai_base.clone()
            } else {
                default_base.to_string()
            };
            let Ok(valid_base) = normalize_base(&raw_base) else {
                continue;
            };
            for key in provider_keys_lookup(prov) {
                push_unique_candidate(
                    &mut candidates,
                    ApiCandidate {
                        provider: prov.to_string(),
                        base_url: valid_base.clone(),
                        api_key: key,
                        model: default_model.to_string(),
                    },
                );
            }
        }

        // 3. Legacy custom fallback endpoints (`ai_fallback_endpoints`).
        for custom in &cfg.ai_fallback_endpoints {
            let Ok(valid_base) = normalize_base(&custom.base_url) else {
                continue;
            };
            let key = custom.api_key.trim();
            if key.is_empty() && !url_is_loopback(&valid_base) {
                continue;
            }
            push_unique_candidate(
                &mut candidates,
                ApiCandidate {
                    provider: if custom.name.trim().is_empty() {
                        "custom".to_string()
                    } else {
                        custom.name.clone()
                    },
                    base_url: valid_base,
                    api_key: key.to_string(),
                    model: if custom.model.trim().is_empty() {
                        "gpt-4o-mini".to_string()
                    } else {
                        custom.model.clone()
                    },
                },
            );
        }
    }

    candidates
}

/// Build the candidate endpoints list (model pool first, then primary, then fallbacks).
pub fn active_candidates(preferred_model: &str) -> Vec<ApiCandidate> {
    let cfg = crate::config::load();
    let pool_keys = crate::pool_keys::load();
    build_candidates(&cfg, &pool_keys, preferred_model, true, keys_for_target)
}

/// Candidates exclusively from the configured `model_pool`.
pub fn pool_only_candidates() -> Vec<ApiCandidate> {
    let cfg = crate::config::load();
    let pool_keys = crate::pool_keys::load();
    build_candidates(&cfg, &pool_keys, "", false, keys_for_target)
}

async fn endpoint_reachable(c: &ApiCandidate) -> bool {
    let is_or = c.base_url.contains("openrouter.ai");
    let mut req = client().get(format!("{}/models", c.base_url.trim_end_matches('/')));
    if !c.api_key.trim().is_empty() {
        req = req.bearer_auth(&c.api_key);
    }
    if is_or {
        req = req
            .header("HTTP-Referer", "https://github.com/azmykn/RepoHarbor")
            .header("X-Title", "RepoHarbor");
    }
    req.send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// Is the configured endpoint (or any pool endpoint) reachable?
pub async fn available() -> bool {
    let candidates = active_candidates("");
    for c in candidates.iter().take(4) {
        if endpoint_reachable(c).await {
            return true;
        }
    }
    false
}

/// Is any enabled deployment in `model_pool` reachable? Used when the main
/// backend is Ollama / llama.cpp.
pub async fn pool_available() -> bool {
    let candidates = pool_only_candidates();
    for c in candidates.iter().take(4) {
        if endpoint_reachable(c).await {
            return true;
        }
    }
    false
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
    let is_or = is_openrouter_backend() || base.contains("openrouter.ai");
    let mut req = client().get(format!("{base}/models")).bearer_auth(key);
    if is_or {
        req = req
            .header("HTTP-Referer", "https://github.com/azmykn/RepoHarbor")
            .header("X-Title", "RepoHarbor");
    }
    let resp = req.send().await;
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
    let is_or = base.contains("openrouter.ai");
    let mut req = client().post(format!("{base}/chat/completions"));
    if !key.trim().is_empty() {
        req = req.bearer_auth(key.trim());
    }
    if is_or {
        req = req
            .header("HTTP-Referer", "https://github.com/azmykn/RepoHarbor")
            .header("X-Title", "RepoHarbor");
    }
    let resp = req
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

async fn chat_with_reasoning_fallback(
    base: &str,
    key: &str,
    model: &str,
    prompt: &str,
    max_tokens: u32,
) -> Result<Attempt, (u16, String)> {
    let is_or = base.contains("openrouter.ai");
    let try_reasoning = if is_or {
        model.starts_with("openai/o1") || model.starts_with("openai/o3")
    } else {
        true
    };
    match chat_once(base, key, model, prompt, max_tokens, try_reasoning).await {
        Ok(a) => Ok(a),
        Err((status, err)) if try_reasoning && rejects_reasoning_effort(status, &err) => {
            chat_once(base, key, model, prompt, max_tokens, false).await
        }
        Err(e) => Err(e),
    }
}

/// Strip `<think>...</think>` tags emitted by reasoning models (e.g. DeepSeek R1).
pub fn strip_think_tags(text: &str) -> String {
    let trimmed = text.trim();
    if let Some(start) = trimmed.find("<think>") {
        if let Some(end) = trimmed.find("</think>") {
            trimmed[end + "</think>".len()..].trim().to_string()
        } else {
            trimmed[start + "<think>".len()..].trim().to_string()
        }
    } else {
        trimmed.to_string()
    }
}

/// Probe a single provider deployment with a tiny 1-reply ping, returning latency in ms.
pub async fn test_deployment(base: &str, key: &str, model: &str) -> Result<u128, String> {
    let base = normalize_base(base)?;
    let key = key.trim();
    if key.is_empty() && !url_is_loopback(&base) {
        return Err("API key is required for non-localhost endpoints".into());
    }
    let model = model.trim();
    if model.is_empty() {
        return Err("model name is required".into());
    }
    let started = Instant::now();
    chat_with_reasoning_fallback(&base, key, model, "Reply with the single word: ok.", 24)
        .await
        .map_err(|(status, err)| {
            if status == 0 {
                err
            } else {
                format!("HTTP {status} — {err}")
            }
        })?;
    Ok(started.elapsed().as_millis())
}

async fn generate_candidates(
    candidates: &[ApiCandidate],
    prompt: &str,
    max_tokens: u32,
) -> Result<String, String> {
    if candidates.is_empty() {
        return Err("no AI API key — add one in Settings → AI & search".into());
    }

    let num_candidates = candidates.len();
    let mut errors = Vec::new();
    let mut skipped_cooldown = 0usize;

    for candidate in candidates {
        let endpoint_key =
            endpoint_cooldown_key(&candidate.base_url, &candidate.model, &candidate.api_key);
        if is_in_cooldown(&endpoint_key) {
            skipped_cooldown += 1;
            eprintln!(
                "RepoHarbor AI Router: skipping {} ({}) — in cooldown",
                candidate.provider, candidate.model
            );
            continue;
        }

        let result = chat_with_reasoning_fallback(
            &candidate.base_url,
            &candidate.api_key,
            &candidate.model,
            prompt,
            max_tokens,
        )
        .await;

        match result {
            Ok(attempt) => {
                let clean_text = strip_think_tags(&attempt.text);
                if !clean_text.is_empty() {
                    return Ok(clean_text);
                }
                if attempt.reasoned {
                    errors.push(format!(
                        "{}: model {} spent token budget thinking without answer",
                        candidate.provider, candidate.model
                    ));
                } else {
                    errors.push(format!(
                        "{}: model {} returned empty response",
                        candidate.provider, candidate.model
                    ));
                }
            }
            Err((status, err)) => {
                if should_cooldown(status) {
                    set_cooldown(&endpoint_key, Duration::from_secs(60));
                    eprintln!(
                        "RepoHarbor AI Router: {} ({}) hit HTTP {} — placed in 60s cooldown",
                        candidate.provider, candidate.model, status
                    );
                }
                errors.push(format!(
                    "{}: model {} HTTP {} — {}",
                    candidate.provider, candidate.model, status, err
                ));
                if !should_failover(status) && num_candidates == 1 {
                    break;
                }
            }
        }
    }

    if errors.is_empty() && skipped_cooldown > 0 {
        return Err(format!(
            "All {skipped_cooldown} AI endpoints are cooling down after rate limits — wait up to 60s or click Reset Cooldowns in Settings"
        ));
    }

    if num_candidates == 1 {
        Err(errors
            .into_iter()
            .next()
            .unwrap_or_else(|| "AI generation failed".into()))
    } else {
        Err(format!(
            "All {num_candidates} API providers in failover pool failed:\n{}",
            errors.join("\n")
        ))
    }
}

/// Generate a completion via `/chat/completions` with full pool + fallback cascade.
pub async fn generate(model: &str, prompt: &str, max_tokens: u32) -> Result<String, String> {
    let candidates = active_candidates(model);
    generate_candidates(&candidates, prompt, max_tokens).await
}

/// Run only the configured `model_pool` entries (used as priority front-end
/// when local Ollama / llama.cpp is the fallback backend).
pub async fn generate_pool_only(prompt: &str, max_tokens: u32) -> Result<String, String> {
    let candidates = pool_only_candidates();
    generate_candidates(&candidates, prompt, max_tokens).await
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

    #[test]
    fn cooldown_registry_tracks_and_expires() {
        let ep = "test-provider:https://api.test.com:model-1";
        assert!(!is_in_cooldown(ep));
        set_cooldown(ep, Duration::from_millis(50));
        assert!(is_in_cooldown(ep));
        std::thread::sleep(Duration::from_millis(60));
        assert!(!is_in_cooldown(ep));
    }

    #[test]
    fn parse_key_pool_splits_multiple_keys() {
        let raw = "key1\nkey2, key3;key4  \n\n  ";
        let parsed = parse_key_pool(raw);
        assert_eq!(parsed, vec!["key1", "key2", "key3", "key4"]);
    }

    #[test]
    fn strip_think_tags_removes_thinking_blocks() {
        assert_eq!(
            strip_think_tags("<think>\nThinking about the commit...\n</think>\nfeat: add support"),
            "feat: add support"
        );
        assert_eq!(
            strip_think_tags("<think>inline reasoning</think>fix: clean up"),
            "fix: clean up"
        );
        assert_eq!(
            strip_think_tags("plain commit subject"),
            "plain commit subject"
        );
        assert_eq!(
            strip_think_tags("<think>only thoughts with no closing tag"),
            "only thoughts with no closing tag"
        );
    }

    #[test]
    fn build_candidates_enforces_priority_egress_loopback_and_dedup() {
        use crate::model::{AppConfig, ModelDeployment};

        let dep = |id: &str,
                   name: &str,
                   prov: &str,
                   base: &str,
                   model: &str,
                   enabled: bool,
                   prio: u32| {
            ModelDeployment {
                id: id.into(),
                name: name.into(),
                provider: prov.into(),
                base_url: base.into(),
                model: model.into(),
                api_key: String::new(),
                enabled,
                priority: prio,
            }
        };

        let cfg = AppConfig {
            ai_backend: "groq".into(),
            ai_failover_enabled: true,
            model_pool: vec![
                dep(
                    "d-late",
                    "Local Ollama",
                    "ollama",
                    "http://localhost:11434/v1",
                    "qwen3:0.6b",
                    true,
                    30,
                ),
                dep(
                    "d-bad-http",
                    "Insecure",
                    "cloud",
                    "http://remote.example.com/v1",
                    "m",
                    true,
                    5,
                ),
                dep("d-off", "Disabled", "groq", GROQ_BASE_URL, "m2", false, 1),
                dep(
                    "d-first",
                    "Groq A",
                    "groq",
                    GROQ_BASE_URL,
                    "openai/gpt-oss-120b",
                    true,
                    10,
                ),
                dep(
                    "d-dup",
                    "Groq A dup",
                    "groq",
                    GROQ_BASE_URL,
                    "openai/gpt-oss-120b",
                    true,
                    15,
                ),
                dep(
                    "d-second",
                    "Groq B",
                    "groq",
                    GROQ_BASE_URL,
                    "openai/gpt-oss-120b",
                    true,
                    20,
                ),
            ],
            ..AppConfig::default()
        };

        let mut pool_keys = HashMap::new();
        pool_keys.insert("d-first".into(), "key-alpha".into());
        pool_keys.insert("d-dup".into(), "key-alpha".into()); // exact dup -> dropped
        pool_keys.insert("d-second".into(), "key-beta".into()); // distinct key -> kept!
        pool_keys.insert("d-bad-http".into(), "key-leak".into()); // http remote -> dropped!

        let cands = build_candidates(&cfg, &pool_keys, "", false, |_| Vec::new());
        let names: Vec<(&str, &str, &str)> = cands
            .iter()
            .map(|c| (c.provider.as_str(), c.base_url.as_str(), c.api_key.as_str()))
            .collect();
        assert_eq!(
            names,
            vec![
                ("Groq A", GROQ_BASE_URL, "key-alpha"),
                ("Groq B", GROQ_BASE_URL, "key-beta"),
                ("Local Ollama", "http://localhost:11434/v1", ""),
            ]
        );
    }

    #[test]
    fn failover_and_cooldown_classification() {
        for status in [0u16, 401, 402, 403, 404, 408, 429, 500, 503, 504] {
            assert!(should_failover(status), "status {status} should failover");
        }
        assert!(should_cooldown(429));
        assert!(should_cooldown(402));
        assert!(should_cooldown(503));
        assert!(!should_cooldown(401));
        assert!(!should_cooldown(404));
        assert!(!should_cooldown(0));
    }

    #[test]
    fn endpoint_cooldown_key_separates_distinct_keys() {
        let a = endpoint_cooldown_key(GROQ_BASE_URL, "m1", "gsk_key_one");
        let b = endpoint_cooldown_key(GROQ_BASE_URL, "m1", "gsk_key_two");
        assert_ne!(a, b);
        assert!(!a.contains("gsk_key"));
    }

    #[test]
    fn url_is_loopback_handles_ipv4_ipv6_and_remote() {
        assert!(url_is_loopback("http://localhost:11434/v1"));
        assert!(url_is_loopback("http://127.0.0.1:8000/v1"));
        assert!(url_is_loopback("http://[::1]:11434/v1"));
        assert!(url_is_loopback("http://[::1]/v1"));
        assert!(!url_is_loopback("https://api.groq.com/openai/v1"));
        assert!(normalize_base("http://[::1]/v1").is_ok());
    }
}
