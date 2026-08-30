//! Local AI summaries (#23). A `Backend` seam selects the inference engine:
//! the Ollama HTTP path (GPU-accelerated), the bundled llama.cpp sidecar (#21,
//! the default), or an opt-in OpenAI-compatible endpoint (`crate::cloud`) for
//! when a hosted model is wanted over a local one. The public entry points
//! (`available`, `installed_models`,
//! `generate`, `embed`, `pull`) dispatch on the configured backend; everything
//! backend-agnostic (prompts, `pick_model`) stays free-standing; vector
//! similarity lives in `crate::semantic`.
//!
//! Everything degrades gracefully: if the active backend isn't reachable,
//! summaries are simply unavailable and the UI shows nothing.

use serde::Deserialize;

use crate::model::Repo;

/// The inference engine serving AI features, from `config.ai_backend`.
enum Backend {
    Ollama,
    LlamaCpp,
    /// An OpenAI-compatible HTTP endpoint (`crate::cloud`) — opt-in, for when a
    /// hosted model is wanted over a local one for speed.
    Cloud,
}

/// Parse a configured backend name. Pure so the accepted spellings (and the
/// local-by-default fallback for anything unknown) are unit-tested.
fn parse_backend(name: &str) -> Backend {
    match name {
        "llamaCpp" | "llama_cpp" | "llamacpp" => Backend::LlamaCpp,
        "cloud" | "openai" | "openaiCompat" | "openai_compat" => Backend::Cloud,
        _ => Backend::Ollama,
    }
}

fn active_backend() -> Backend {
    parse_backend(crate::config::load().ai_backend.as_str())
}

// ── Backend-dispatching entry points ───────────────────────────────────────
// Each delegates to the active backend. The llama.cpp arms drive the bundled
// `llama-server` sidecar (see `crate::llama`); embeddings stay Ollama-only, and
// model "pull" is a GGUF download from Settings rather than an Ollama registry
// pull. All degrade to "unavailable" when the backend isn't reachable.

/// Is the active backend reachable?
pub async fn available() -> bool {
    match active_backend() {
        Backend::Ollama => ollama_available().await,
        Backend::LlamaCpp => crate::llama::available().await,
        Backend::Cloud => crate::cloud::available().await,
    }
}

/// Installed/available models as (name, size_bytes) for the active backend.
/// Cloud models are hosted, so they report a zero size.
pub async fn installed_models() -> Vec<(String, u64)> {
    match active_backend() {
        Backend::Ollama => ollama_installed_models().await,
        Backend::LlamaCpp => crate::llama::installed_models(),
        Backend::Cloud => crate::cloud::models().await,
    }
}

/// Default token budget for short generations (summaries, ping, resume, …).
const DEFAULT_NUM_PREDICT: u32 = 120;
/// Token budget for Conventional Commit messages (subject + real body).
const COMMIT_NUM_PREDICT: u32 = 384;
/// Max diff characters fed into the commit prompt (keeps context bounded).
const COMMIT_DIFF_CLAMP: usize = 10_000;

/// Generate text from `prompt` using `model` on the active backend. The
/// llama.cpp backend serves the configured GGUF, so it ignores `model`.
pub async fn generate(model: &str, prompt: &str) -> Result<String, String> {
    generate_limited(model, prompt, DEFAULT_NUM_PREDICT).await
}

async fn generate_limited(model: &str, prompt: &str, num_predict: u32) -> Result<String, String> {
    match active_backend() {
        Backend::Ollama => ollama_generate(model, prompt, num_predict).await,
        Backend::LlamaCpp => crate::llama::generate(prompt, num_predict).await,
        Backend::Cloud => crate::cloud::generate(model, prompt, num_predict).await,
    }
}

/// Whether the active backend can embed at all. Embeddings are Ollama-only, so
/// semantic indexing/recall stays dormant (not broken) on the llama.cpp
/// backend — callers skip the work instead of erroring per call.
pub fn embeddings_supported() -> bool {
    matches!(active_backend(), Backend::Ollama)
}

/// True when the active backend sends prompts off the machine. The Settings
/// panel warns on this so the egress is never a surprise.
pub fn backend_is_remote() -> bool {
    matches!(active_backend(), Backend::Cloud)
}

/// Embed `text` with `model` on the active backend. Embeddings are Ollama-only
/// for now — semantic search stays hidden on the llama.cpp backend.
pub async fn embed(model: &str, text: &str) -> Result<Vec<f32>, String> {
    match active_backend() {
        Backend::Ollama => ollama_embed(model, text).await,
        Backend::LlamaCpp => Err("embeddings are not supported on the llama.cpp backend".into()),
        Backend::Cloud => {
            Err("embeddings stay local — switch to the Ollama backend to index".into())
        }
    }
}

/// Download/prepare `model` on the active backend, reporting progress. The
/// llama.cpp model download has its own command (`download_llama_model`).
pub async fn pull(model: &str, on_progress: impl FnMut(&str, u64, u64)) -> Result<(), String> {
    match active_backend() {
        Backend::Ollama => ollama_pull(model, on_progress).await,
        Backend::LlamaCpp => {
            Err("use the model download in Settings for the llama.cpp backend".into())
        }
        Backend::Cloud => Err("cloud models are hosted — nothing to download".into()),
    }
}

/// Base URL of the Ollama server, from config (default http://localhost:11434).
/// config::load() is cached, so this is cheap to call per request.
fn base() -> String {
    crate::config::load().ollama_host
}

/// Shared HTTP client so the many Ollama calls (status, per-repo summaries and
/// embeddings) reuse one connection pool. reqwest::Client is Arc-backed.
///
/// Only a `connect_timeout` is set — a dead/unreachable host fails fast — but no
/// overall request timeout, because generation and model pulls legitimately
/// stream for minutes and must not be cut off.
fn client() -> reqwest::Client {
    static CLIENT: std::sync::LazyLock<reqwest::Client> = std::sync::LazyLock::new(|| {
        reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(8))
            .build()
            .unwrap_or_default()
    });
    CLIENT.clone()
}

/// Is a local Ollama server reachable?
async fn ollama_available() -> bool {
    client()
        .get(format!("{}/api/version", base()))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// Installed Ollama models as (name, size_bytes).
async fn ollama_installed_models() -> Vec<(String, u64)> {
    #[derive(Deserialize)]
    struct Tags {
        #[serde(default)]
        models: Vec<Model>,
    }
    #[derive(Deserialize)]
    struct Model {
        name: String,
        #[serde(default)]
        size: u64,
    }
    let resp = client().get(format!("{}/api/tags", base())).send().await;
    match resp {
        Ok(r) => match r.json::<Tags>().await {
            Ok(t) => t.models.into_iter().map(|m| (m.name, m.size)).collect(),
            Err(_) => Vec::new(),
        },
        Err(_) => Vec::new(),
    }
}

/// Heuristic: is this an embedding-only model? Such models reject `/api/generate`
/// (Ollama 400), so they must never be picked as a chat fallback.
pub fn is_embedding_model(name: &str) -> bool {
    let n = name.to_lowercase();
    n.contains("embed") || n.contains("minilm") || n.starts_with("bge") || n.contains("/bge")
}

/// True when an installed model name matches the preferred config value.
/// Exact match first; also accepts a bare name matching a tagged install
/// (`qwen2.5` → `qwen2.5:3b`) so a mistyped/untagged Settings value doesn't
/// silently fall back to the smallest leftover model (often a thinking tiny).
fn model_name_matches(preferred: &str, installed: &str) -> bool {
    if preferred == installed {
        return true;
    }
    let pref = preferred.trim();
    let inst = installed.trim();
    if pref.is_empty() || inst.is_empty() {
        return false;
    }
    // Config without tag: "qwen2.5" matches "qwen2.5:3b" / "qwen2.5:latest".
    if !pref.contains(':') {
        return inst == pref || inst.starts_with(&format!("{pref}:"));
    }
    // Config with tag: also accept installed bare name equal to the family.
    if !inst.contains(':') {
        let fam = pref.split(':').next().unwrap_or(pref);
        return inst == fam;
    }
    false
}

/// Choose the chat model: the preferred one if installed, otherwise the smallest
/// installed model that can actually generate (embedding models are excluded —
/// picking one would 400 on /api/generate). Pure for testing.
pub fn pick_model(preferred: &str, available: &[(String, u64)]) -> Option<String> {
    if let Some((name, _)) = available
        .iter()
        .find(|(name, _)| model_name_matches(preferred, name))
    {
        return Some(name.clone());
    }
    available
        .iter()
        .filter(|(name, _)| !is_embedding_model(name))
        .min_by_key(|(_, size)| *size)
        .map(|(name, _)| name.clone())
}

/// Build the summarization prompt from repo metadata. Pure for testing.
pub fn summary_prompt(repo: &Repo) -> String {
    let git = &repo.git;
    let changes = if git.dirty > 0 {
        format!("{} uncommitted change(s)", git.dirty)
    } else {
        "a clean tree".to_string()
    };
    format!(
        "You summarize a code repository in ONE concise, factual sentence for a developer dashboard. \
No preamble, no markdown, max 24 words.\n\n\
Name: {name}\n\
Language: {lang}\n\
Description: {desc}\n\
State: branch {branch}, {changes}, {ahead} ahead / {behind} behind upstream.\n\n\
Summary:",
        name = repo.display_name,
        lang = repo.language.as_deref().unwrap_or("unknown"),
        desc = repo.description.as_deref().unwrap_or("(none)"),
        branch = git.branch,
        changes = changes,
        ahead = git.ahead,
        behind = git.behind,
    )
}

/// Heuristic: models known to "think" into a hidden channel and often empty
/// `response` when `num_predict` is small (RepoHarbor's default chat budget).
fn likely_thinking_model(model: &str) -> bool {
    let n = model.to_lowercase();
    n.starts_with("qwen3")
        || n.contains("qwen3:")
        || n.starts_with("gemma3")
        || n.contains("gemma3:")
}

/// Generate a summary via Ollama.
///
/// Known thinking models (qwen3, gemma3, …) get `think:false` on the first
/// attempt so short prompts (commit messages, summaries) don't burn the whole
/// token budget on hidden reasoning. Other models try without the field first;
/// if the response is empty, we retry once with `think:false` — that way plain
/// models that might reject the field are never hit with it unnecessarily.
///
/// True when Ollama's llama runner crashed mid-load/gen — common on small-VRAM
/// GPUs (e.g. Quadro T2000 4GB) while desktop apps already fill most of VRAM.
fn is_runner_crash(err: &str) -> bool {
    let e = err.to_lowercase();
    e.contains("runner process has terminated")
        || e.contains("llama runner")
        || (e.contains("500") && e.contains("terminated"))
        // Other VRAM / GPU load failures Ollama surfaces without the runner phrase.
        || e.contains("cuda error")
        || e.contains("out of memory")
        || e.contains("insufficient memory")
        || e.contains("ggml_gallocr_reserve")
}

/// An empty final `response` is always an error — callers must not treat blank
/// Ok as a successful commit/summary (that used to wipe the drawer inputs).
///
/// On a GPU runner crash we retry once with `num_gpu: 0` (CPU) so commit/summary
/// still work when VRAM is exhausted by the desktop session.
async fn ollama_generate(model: &str, prompt: &str, num_predict: u32) -> Result<String, String> {
    let text = match ollama_generate_with_gpu(model, prompt, num_predict, None).await {
        Ok(t) => t,
        Err(err) if is_runner_crash(&err) => {
            eprintln!(
                "[ai] Ollama runner crashed on GPU ({err}); retrying {model} on CPU (num_gpu=0)"
            );
            ollama_generate_with_gpu(model, prompt, num_predict, Some(0))
                .await
                .map_err(|cpu_err| {
                    format!(
                        "{cpu_err} (GPU runner also failed — free VRAM, restart Ollama, or use a smaller model like qwen3:0.6b)"
                    )
                })?
        }
        Err(err) => return Err(err),
    };
    if text.is_empty() {
        return Err(format!(
            "model {model} returned an empty response — try another model or raise the token budget"
        ));
    }
    Ok(text)
}

async fn ollama_generate_with_gpu(
    model: &str,
    prompt: &str,
    num_predict: u32,
    num_gpu: Option<u32>,
) -> Result<String, String> {
    if likely_thinking_model(model) {
        generate_once(model, prompt, true, num_predict, num_gpu).await
    } else {
        let first = generate_once(model, prompt, false, num_predict, num_gpu).await?;
        if !first.is_empty() {
            Ok(first)
        } else {
            generate_once(model, prompt, true, num_predict, num_gpu).await
        }
    }
}

async fn generate_once(
    model: &str,
    prompt: &str,
    suppress_think: bool,
    num_predict: u32,
    num_gpu: Option<u32>,
) -> Result<String, String> {
    #[derive(Deserialize)]
    struct GenResp {
        #[serde(default)]
        response: String,
    }
    let mut options = serde_json::json!({
        "temperature": 0.2,
        "num_predict": num_predict
    });
    if let Some(n) = num_gpu {
        options["num_gpu"] = serde_json::json!(n);
    }
    let mut body = serde_json::json!({
        "model": model,
        "prompt": prompt,
        "stream": false,
        "options": options
    });
    if suppress_think {
        body["think"] = serde_json::Value::Bool(false);
    }
    let resp = client()
        .post(format!("{}/api/generate", base()))
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(ollama_error(resp).await);
    }
    let parsed: GenResp = resp.json().await.map_err(|e| e.to_string())?;
    Ok(parsed.response.trim().to_string())
}

/// Pull a model via Ollama (`/api/pull`), streaming NDJSON progress. `on_progress`
/// is called with (status, completed_bytes, total_bytes) for each update — kept
/// as a callback so this module stays free of Tauri's event machinery.
async fn ollama_pull(
    model: &str,
    mut on_progress: impl FnMut(&str, u64, u64),
) -> Result<(), String> {
    use futures_util::StreamExt;

    // Ollama's /api/pull needs an explicit tag; default to :latest when none.
    let model = if model.contains(':') {
        model.to_string()
    } else {
        format!("{model}:latest")
    };
    let model = model.as_str();

    #[derive(Deserialize)]
    struct Line {
        #[serde(default)]
        status: String,
        #[serde(default)]
        completed: u64,
        #[serde(default)]
        total: u64,
        #[serde(default)]
        error: Option<String>,
    }

    let body = serde_json::json!({ "model": model, "stream": true });
    let resp = client()
        .post(format!("{}/api/pull", base()))
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(ollama_error(resp).await);
    }

    // Ollama streams newline-delimited JSON objects; buffer partial lines.
    let mut stream = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.next().await {
        buf.extend_from_slice(&chunk.map_err(|e| e.to_string())?);
        while let Some(nl) = buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = buf.drain(..=nl).collect();
            let trimmed = &line[..line.len().saturating_sub(1)];
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(l) = serde_json::from_slice::<Line>(trimmed) {
                if let Some(err) = l.error {
                    return Err(err);
                }
                on_progress(&l.status, l.completed, l.total);
            }
        }
    }
    Ok(())
}

/// Build a readable error from a non-2xx Ollama response, surfacing its
/// `{"error": "..."}` body (e.g. "… does not support generate") instead of a
/// bare status code.
async fn ollama_error(resp: reqwest::Response) -> String {
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    let detail = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
        .unwrap_or_else(|| body.trim().to_string());
    if detail.is_empty() {
        format!("Ollama API {status}")
    } else {
        format!("Ollama API {status}: {detail}")
    }
}

/// Embed `text` with an embedding model via Ollama (`/api/embed`).
async fn ollama_embed(model: &str, text: &str) -> Result<Vec<f32>, String> {
    #[derive(Deserialize)]
    struct Resp {
        #[serde(default)]
        embeddings: Vec<Vec<f32>>,
    }
    let body = serde_json::json!({ "model": model, "input": text });
    let resp = client()
        .post(format!("{}/api/embed", base()))
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(ollama_error(resp).await);
    }
    let parsed: Resp = resp.json().await.map_err(|e| e.to_string())?;
    parsed
        .embeddings
        .into_iter()
        .next()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| "model returned no embedding".to_string())
}

fn clamp_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect()
}

fn is_odoo_manifest_path(path: &str) -> bool {
    let base = path.rsplit('/').next().unwrap_or(path);
    base == "__manifest__.py" || base == "__openerp__.py"
}

/// Repo-relative paths touched by a unified diff, in first-seen order, read
/// from the `diff --git` / `+++ b/` headers. Deletions (`/dev/null`) are kept —
/// the containing directory is still where a changelog would live.
pub fn changed_paths_in_diff(diff: &str) -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();
    let mut push = |p: &str| {
        let p = p.trim();
        if p.is_empty() || p == "/dev/null" || paths.iter().any(|x| x == p) {
            return;
        }
        paths.push(p.to_string());
    };
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            for part in rest.split_whitespace() {
                push(
                    part.strip_prefix("a/")
                        .or_else(|| part.strip_prefix("b/"))
                        .unwrap_or(part),
                );
            }
        } else if let Some(rest) = line.strip_prefix("+++ b/") {
            push(rest.split('\t').next().unwrap_or(rest));
        }
    }
    paths
}

/// Relative paths of Odoo manifests touched by a unified diff.
pub fn manifest_paths_in_diff(diff: &str) -> Vec<String> {
    changed_paths_in_diff(diff)
        .into_iter()
        .filter(|p| is_odoo_manifest_path(p))
        .collect()
}

/// Filenames treated as a changelog, in preference order within one directory.
const CHANGELOG_NAMES: &[&str] = &[
    "CHANGELOG.md",
    "CHANGELOG.rst",
    "CHANGELOG.txt",
    "CHANGELOG",
    "changelog.md",
    "CHANGES.md",
    "HISTORY.md",
    "NEWS.md",
];

/// How many distinct changelog files feed one commit prompt. A commit can span
/// several modules; beyond a handful the notes crowd out the diff itself.
const MAX_CHANGELOGS: usize = 3;
/// Skip pathologically large changelogs rather than reading them to excerpt.
const MAX_CHANGELOG_BYTES: u64 = 200 * 1024;
/// Characters kept from each changelog.
const CHANGELOG_EXCERPT_CHARS: usize = 2_000;
/// Trailing lines used when a file has no `Unreleased` section.
const CHANGELOG_TAIL_LINES: usize = 80;
/// Recent commit subjects shown to the model for house style.
const RECENT_SUBJECTS: usize = 5;

/// True for a `## Unreleased` / `## [Unreleased]` heading (Keep a Changelog).
fn is_unreleased_heading(line: &str) -> bool {
    let l = line.trim();
    if !l.starts_with("##") || l.starts_with("###") {
        return false;
    }
    l.to_lowercase().contains("unreleased")
}

/// The most relevant slice of a changelog: its `Unreleased` section when there
/// is one (that's where the change being committed belongs), otherwise the
/// trailing lines — the newest entries, since these files grow at the top but
/// are read bottom-up by nobody. Pure so it's unit-tested without a filesystem.
pub fn changelog_excerpt(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if let Some(start) = lines.iter().position(|l| is_unreleased_heading(l)) {
        let end = lines
            .iter()
            .enumerate()
            .skip(start + 1)
            .find(|(_, l)| {
                let l = l.trim();
                l.starts_with("##") && !l.starts_with("###")
            })
            .map(|(i, _)| i)
            .unwrap_or(lines.len());
        let section = lines[start..end].join("\n").trim().to_string();
        // A bare heading with nothing under it says nothing — fall through.
        if section.lines().filter(|l| !l.trim().is_empty()).count() > 1 {
            return clamp_chars(&section, CHANGELOG_EXCERPT_CHARS);
        }
    }
    let tail = lines
        .iter()
        .skip(lines.len().saturating_sub(CHANGELOG_TAIL_LINES))
        .copied()
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();
    clamp_chars(&tail, CHANGELOG_EXCERPT_CHARS)
}

/// Walk up from `dir` to `root` (inclusive) and return the first changelog file
/// found. `dir` must be inside `root`.
fn changelog_near(root: &std::path::Path, dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut cur = Some(dir);
    while let Some(d) = cur {
        for name in CHANGELOG_NAMES {
            let candidate = d.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        if d == root {
            break;
        }
        cur = d.parent();
    }
    None
}

/// Changelog notes relevant to a diff, as (repo-relative path, excerpt).
///
/// Walks *up from each changed file* to the repo root rather than scanning the
/// tree, so a monorepo of Odoo modules contributes the changelog of the module
/// actually being touched — not dozens of unrelated ones.
pub fn nearby_changelogs(diff: &str, repo_path: &str) -> Vec<(String, String)> {
    let root = std::path::Path::new(repo_path);
    if repo_path.is_empty() || !root.is_dir() {
        return Vec::new();
    }
    let mut found: Vec<(String, String)> = Vec::new();
    for rel in changed_paths_in_diff(diff) {
        if found.len() >= MAX_CHANGELOGS {
            break;
        }
        // Stay inside the repo: a header is always repo-relative, so anything
        // absolute or with a parent component is not something to walk from.
        let rel_path = std::path::Path::new(&rel);
        if rel_path.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::RootDir
            )
        }) {
            continue;
        }
        let abs = root.join(rel_path);
        let Some(dir) = abs.parent() else { continue };
        let Some(file) = changelog_near(root, dir) else {
            continue;
        };
        let label = file
            .strip_prefix(root)
            .unwrap_or(&file)
            .to_string_lossy()
            .into_owned();
        if found.iter().any(|(p, _)| *p == label) {
            continue;
        }
        if std::fs::metadata(&file)
            .map(|m| m.len())
            .unwrap_or(u64::MAX)
            > MAX_CHANGELOG_BYTES
        {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let excerpt = changelog_excerpt(&text);
        if !excerpt.trim().is_empty() {
            found.push((label, excerpt));
        }
    }
    found
}

/// Extra context beyond the diff itself, gathered by [`commit_message`].
#[derive(Debug, Default, Clone)]
pub struct CommitExtras {
    /// Changelog notes near the changed files, as (repo-relative path, excerpt).
    pub changelogs: Vec<(String, String)>,
    /// Recent commit subjects (newest first) — house style, not content.
    pub recent_subjects: Vec<String>,
}

impl CommitExtras {
    /// Collect changelog notes + recent subjects for `repo_path`. Every source
    /// is optional: a repo with no changelog and no history yields empty
    /// extras and the prompt is unchanged.
    pub fn gather(diff: &str, repo_path: &str) -> Self {
        CommitExtras {
            changelogs: nearby_changelogs(diff, repo_path),
            recent_subjects: crate::git_ops::recent_log(repo_path, RECENT_SUBJECTS)
                .unwrap_or_default()
                .into_iter()
                .map(|c| c.summary)
                .filter(|s| !s.trim().is_empty())
                .collect(),
        }
    }
}

/// Parse an Odoo-style `version` assignment from a single line
/// (`'version': '19.0.1.0.0'`, `"version": "…"`, optional leading diff `+/-`).
pub fn parse_odoo_version_line(line: &str) -> Option<String> {
    let raw = line.trim();
    let stripped = raw
        .strip_prefix('+')
        .or_else(|| raw.strip_prefix('-'))
        .unwrap_or(raw)
        .trim()
        .trim_end_matches(',')
        .trim();

    for key in ["'version'", "\"version\""] {
        let Some(idx) = stripped.find(key) else {
            continue;
        };
        let after_key = stripped[idx + key.len()..].trim_start();
        let after_sep = after_key
            .strip_prefix(':')
            .or_else(|| after_key.strip_prefix('='))?
            .trim_start();
        let quote = after_sep.chars().next()?;
        if quote != '\'' && quote != '"' {
            continue;
        }
        let inner = &after_sep[quote.len_utf8()..];
        let end = inner.find(quote)?;
        let val = inner[..end].trim();
        if !val.is_empty() && val.chars().any(|c| c.is_ascii_digit()) {
            return Some(val.to_string());
        }
    }
    None
}

fn version_from_added_diff_lines(diff: &str) -> Option<String> {
    // Prefer the new (+) value when a version line changed.
    for line in diff.lines() {
        if line.starts_with('+') && !line.starts_with("+++") {
            if let Some(v) = parse_odoo_version_line(line) {
                return Some(v);
            }
        }
    }
    // Fall back to any version line in the hunk (context / deletions).
    for line in diff.lines() {
        if let Some(v) = parse_odoo_version_line(line) {
            return Some(v);
        }
    }
    None
}

fn version_from_manifest_file(path: &std::path::Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    content.lines().find_map(parse_odoo_version_line)
}

/// Resolve an Odoo module version for commit prompts: prefer on-disk
/// `__manifest__.py` / `__openerp__.py` when `repo_path` is set (survives diff
/// truncation), else parse from the diff hunk. Returns a short hint line or
/// `None` when the change set does not touch a manifest.
pub fn odoo_manifest_version_hint(diff: &str, repo_path: Option<&str>) -> Option<String> {
    let paths = manifest_paths_in_diff(diff);
    let touches_manifest =
        !paths.is_empty() || diff.contains("__manifest__.py") || diff.contains("__openerp__.py");
    if !touches_manifest {
        return None;
    }

    let from_disk = repo_path.and_then(|root| {
        let root = std::path::Path::new(root);
        if !paths.is_empty() {
            paths
                .iter()
                .find_map(|rel| version_from_manifest_file(&root.join(rel)))
        } else {
            // Headers missed but the text mentions a manifest — walk common
            // single-module layouts is out of scope; stick to diff parse.
            None
        }
    });
    let version = from_disk.or_else(|| version_from_added_diff_lines(diff))?;
    Some(format!(
        "Odoo module version (from __manifest__.py): {version}"
    ))
}

/// Prompt to write a Conventional Commit message from a staged/working diff.
/// When `repo_path` is set and the diff touches an Odoo manifest, the current
/// module version is injected so it survives the diff character clamp.
/// Output shape (subject, blank line, body) is parsed by [`split_commit_message`].
pub fn commit_prompt(diff: &str) -> String {
    commit_prompt_with_context(diff, None)
}

/// Like [`commit_prompt`], with an optional repo root for on-disk manifest reads.
pub fn commit_prompt_with_context(diff: &str, repo_path: Option<&str>) -> String {
    commit_prompt_with_extras(diff, repo_path, &CommitExtras::default())
}

/// Render the changelog notes / recent-subject blocks. Empty extras render
/// nothing, so the prompt is byte-identical to the plain diff-only form.
fn extras_block(extras: &CommitExtras) -> String {
    let mut out = String::new();
    for (path, excerpt) in &extras.changelogs {
        out.push_str(&format!(
            "\nExisting changelog notes from `{path}` — use them to explain WHAT changed and WHY. \
Only rely on entries that match this diff; do not copy them verbatim:\n{excerpt}\n"
        ));
    }
    if !extras.recent_subjects.is_empty() {
        out.push_str(&format!(
            "\nRecent commit subjects in this repo, for scope naming and style only — do not \
describe their changes:\n{}\n",
            extras
                .recent_subjects
                .iter()
                .map(|s| format!("- {s}"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    out
}

/// The full commit prompt: the diff, the Odoo manifest version hint, and any
/// [`CommitExtras`] gathered from the repo (changelog notes, recent subjects).
pub fn commit_prompt_with_extras(
    diff: &str,
    repo_path: Option<&str>,
    extras: &CommitExtras,
) -> String {
    // The Odoo version requirement is only stated when a manifest version was
    // actually found. Stating it unconditionally taught models to invent one:
    // a Rust repo got `feat(ai): add cloud backend support (19.0.1.0.0)`.
    let version_hint = odoo_manifest_version_hint(diff, repo_path);
    let version_rule = if version_hint.is_some() {
        "- You MUST mention the module version shown below in the subject or body.\n"
    } else {
        ""
    };
    let hint = version_hint.map(|h| format!("\n{h}\n")).unwrap_or_default();
    let hint = format!("{hint}{}", extras_block(extras));
    format!(
        "Write a Conventional Commit message for these staged changes.\n\n\
Requirements:\n\
- Subject line in Conventional Commit form (e.g. `feat(scope): summary`), under ~72 characters.\n\
- Then a blank line, then a real body of 2–5 short sentences or bullets explaining WHAT changed and WHY — not a subject-only one-liner.\n\
- Do not invent version numbers, issue ids or file names that are not in the input.\n\
{version_rule}\
- Output ONLY the commit message — no code fences, no preamble, no quotes around the whole message.\n\
- Write in English.\n\
{hint}\n\
Diff:\n{}\n\nCommit message:",
        clamp_chars(diff, COMMIT_DIFF_CLAMP)
    )
}

/// Strip a wrapping markdown code fence (``` / ```text) so models that ignore
/// "no code fences" still yield a usable subject. Bare `trim_matches('`')` on a
/// fence opener line would wipe the subject to "" and blank the drawer.
fn strip_wrapping_fence(text: &str) -> &str {
    let mut t = text.trim();
    if let Some(rest) = t.strip_prefix("```") {
        // Drop the optional language tag on the opening fence line.
        t = match rest.find('\n') {
            Some(i) => rest[i + 1..].trim_start(),
            None => rest.trim_start_matches(|c: char| !c.is_whitespace()).trim(),
        };
        if let Some(i) = t.rfind("```") {
            t = t[..i].trim_end();
        }
    }
    t
}

/// Split an AI commit message into (subject, body): the first non-empty line
/// (stripped of wrapping quotes/backticks) is the subject, everything after it
/// is the body. Both come back trimmed; either may be empty.
pub fn split_commit_message(text: &str) -> (String, String) {
    let text = strip_wrapping_fence(text);
    let mut subject = String::new();
    let mut rest: Vec<&str> = Vec::new();
    let mut got_subject = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if !got_subject {
            if trimmed.is_empty() || trimmed.starts_with("```") {
                continue;
            }
            subject = trimmed.trim_matches(['"', '`']).to_string();
            if subject.is_empty() {
                continue;
            }
            got_subject = true;
            continue;
        }
        rest.push(line);
    }
    // Drop a trailing fence closer left in the body.
    while rest
        .last()
        .is_some_and(|l| l.trim().is_empty() || l.trim().starts_with("```"))
    {
        rest.pop();
    }
    let body = rest.join("\n").trim().to_string();
    (subject, body)
}

/// Prompt to summarize commits into a changelog / PR description.
pub fn changelog_prompt(commits: &[String]) -> String {
    format!(
        "Summarize these commits into a concise changelog as markdown bullet points, grouping related \
changes. No preamble.\n\nCommits:\n{}\n\nChangelog:",
        commits.join("\n")
    )
}

/// Prompt to draft a pull-request title + body from a branch's commit range.
/// The first output line is the title; the rest (after a blank line) is the
/// markdown body — [`split_pr_draft`] parses that shape back apart.
pub fn pr_prompt(branch: &str, commits: &[String]) -> String {
    format!(
        "Draft a pull request for branch \"{branch}\" from these commits (newest first). \
Output the PR title on the FIRST line (imperative, under 72 chars, no quotes), then a blank \
line, then a concise markdown body summarizing the changes as bullet points. No preamble, \
no code fences.\n\nCommits:\n{}\n\nPull request:",
        commits.join("\n")
    )
}

/// Split an AI PR draft into (title, body): first non-empty line is the
/// title, the rest is the body. `None` when there's no usable title.
pub fn split_pr_draft(text: &str) -> Option<(String, String)> {
    let mut lines = text.trim().lines();
    let title = lines.next()?.trim().trim_matches('"').to_string();
    if title.is_empty() {
        return None;
    }
    let body = lines.collect::<Vec<_>>().join("\n").trim().to_string();
    Some((title, body))
}

/// Non-AI Conventional Commit when generation is unreachable. Prefers the
/// first changelog bullet near the changed files; otherwise names the files.
/// Same degrade-gracefully contract as [`fallback_pr_draft`]: the git write the
/// user asked for still happens.
pub fn fallback_commit_message(diff: &str, repo_path: &str) -> String {
    let extras = if repo_path.is_empty() {
        CommitExtras::default()
    } else {
        CommitExtras::gather(diff, repo_path)
    };
    let subject = extras
        .changelogs
        .iter()
        .find_map(|(_, excerpt)| excerpt.lines().find_map(changelog_line_as_subject))
        .unwrap_or_else(|| files_subject(diff));
    append_odoo_version(subject, diff, repo_path)
}

fn changelog_line_as_subject(line: &str) -> Option<String> {
    let t = line.trim();
    if t.is_empty() || t.starts_with('#') {
        return None;
    }
    let t = t.trim_start_matches(['-', '*', '+']).trim();
    if t.is_empty() {
        return None;
    }
    let lower = t.to_ascii_lowercase();
    let conventional = [
        "feat", "fix", "chore", "docs", "refactor", "test", "perf", "ci", "build", "style",
    ];
    let subject = if conventional
        .iter()
        .any(|p| lower.starts_with(p) && t.contains(':'))
    {
        t.to_string()
    } else {
        format!("chore: {t}")
    };
    Some(clamp_chars(&subject, 72))
}

fn files_subject(diff: &str) -> String {
    let paths = changed_paths_in_diff(diff);
    let names: Vec<&str> = paths
        .iter()
        .map(|p| p.rsplit('/').next().unwrap_or(p.as_str()))
        .collect();
    let listed = match names.as_slice() {
        [] => "working tree".to_string(),
        [one] => (*one).to_string(),
        [a, b] => format!("{a} and {b}"),
        [a, b, rest @ ..] => format!("{a}, {b}, and {} more", rest.len()),
    };
    clamp_chars(&format!("chore: update {listed}"), 72)
}

fn append_odoo_version(subject: String, diff: &str, repo_path: &str) -> String {
    let path = (!repo_path.is_empty()).then_some(repo_path);
    let Some(hint) = odoo_manifest_version_hint(diff, path) else {
        return subject;
    };
    let Some(ver) = hint.rsplit(": ").next().map(str::trim) else {
        return subject;
    };
    if ver.is_empty() || subject.contains(ver) {
        return subject;
    }
    clamp_chars(&format!("{subject} ({ver})"), 72)
}

/// Non-AI PR draft from a commit range (newest first): the latest commit's
/// subject as the title, the commit list as the body. Works with an empty
/// range too, so "Open PR" never depends on AI being reachable.
pub fn fallback_pr_draft(branch: &str, commits: &[String]) -> (String, String) {
    let title = commits
        .first()
        .cloned()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| branch.to_string());
    let body = commits
        .iter()
        .map(|c| format!("- {c}"))
        .collect::<Vec<_>>()
        .join("\n");
    (title, body)
}

/// Prompt to catch the user up on what changed in a repo since they last looked.
pub fn resume_prompt(repo_name: &str, commits: &[String]) -> String {
    format!(
        "In 2–3 short sentences, catch me up on what changed in the \"{repo_name}\" repository since I \
last looked, based on these commits (newest first). Be specific and factual, no preamble, no markdown.\n\n\
Commits:\n{}\n\nWhat changed:",
        commits.join("\n")
    )
}

/// Run `prompt` through the configured chat model on the active backend.
/// Picks the configured model if installed, else the smallest non-embed —
/// returns an error when AI is unreachable or no model is installed, so callers
/// degrade gracefully (the `aiReady` contract).
async fn generate_with_model(prompt: &str) -> Result<String, String> {
    generate_with_budget(prompt, DEFAULT_NUM_PREDICT).await
}

async fn generate_with_budget(prompt: &str, num_predict: u32) -> Result<String, String> {
    let cfg = crate::config::load();
    // Hosted models aren't "installed", and listing them on every call would
    // add a round-trip to the very path we chose the cloud backend to speed up.
    if matches!(active_backend(), Backend::Cloud) {
        return generate_limited(&cfg.ai_model, prompt, num_predict).await;
    }
    let models = installed_models().await;
    let model = pick_model(&cfg.ai_model, &models).ok_or("no AI model installed")?;
    generate_limited(&model, prompt, num_predict).await
}

/// Generate a Conventional Commit message for a staged/working diff.
///
/// `repo_path` is the repo root used to read `__manifest__.py` when the diff
/// touches it (so the version survives the diff character clamp). Pass `""`
/// or any path when unknown — version still falls back to parsing the diff.
///
/// Returns `Err` when the model yields no usable subject (empty / fences-only),
/// so the UI can toast instead of blanking the composer.
pub async fn commit_message(repo_path: &str, diff: &str) -> Result<String, String> {
    let path = (!repo_path.is_empty()).then_some(repo_path);
    let extras = CommitExtras::gather(diff, repo_path);
    let prompt = commit_prompt_with_extras(diff, path, &extras);
    let raw = generate_with_budget(&prompt, COMMIT_NUM_PREDICT).await?;
    let (subject, body) = split_commit_message(&raw);
    if subject.is_empty() {
        return Err(
            "AI returned no commit subject — try again or pick another model in Settings".into(),
        );
    }
    // Re-join so callers that commit the raw string keep a clean subject/body
    // (fences stripped). Subject-only is still valid.
    if body.is_empty() {
        Ok(subject)
    } else {
        Ok(format!("{subject}\n\n{body}"))
    }
}

/// Summarize commit subjects into a markdown changelog.
pub async fn changelog(commits: &[String]) -> Result<String, String> {
    generate_with_model(&changelog_prompt(commits)).await
}

/// A 2–3 sentence catch-up on what changed in a repo since the last look.
pub async fn resume(repo_name: &str, commits: &[String]) -> Result<String, String> {
    generate_with_model(&resume_prompt(repo_name, commits)).await
}

/// Draft a PR title + body from a branch's commit range (first line = title —
/// see [`pr_prompt`] / [`split_pr_draft`]). Callers fall back to
/// [`fallback_pr_draft`] when AI is unavailable (the `aiReady` contract).
pub async fn pr_description(branch: &str, commits: &[String]) -> Result<String, String> {
    generate_with_model(&pr_prompt(branch, commits)).await
}

/// Prompt for a short daily briefing across recently-active repos.
pub fn briefing_prompt(lines: &[String]) -> String {
    format!(
        "You are a dev's morning briefing. In 2–4 short sentences, summarize what changed across these \
repositories. Be specific and factual, no preamble.\n\n{}\n\nBriefing:",
        lines.join("\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Activity, GitStatus};

    fn repo() -> Repo {
        Repo {
            id: "/x".into(),
            display_name: "RepoHarbor".into(),
            slug: Some("acme/widget".into()),
            path: "~/dev/widget".into(),
            description: Some("repo dashboard".into()),
            language: Some("Rust".into()),
            git: GitStatus {
                branch: "main".into(),
                ahead: 2,
                behind: 0,
                dirty: 7,
                ..Default::default()
            },
            last_commit_unix: 0,
            activity: Activity::Active,
            root: "~/dev".into(),
            host: None,
            remote_host: None,
            stars: 0,
            topics: vec![],
            open_issues: 0,
            latest_release: None,
            private: false,
            favorite: false,
            ai_summary: None,
            parent_id: None,
            submodule_path: None,
        }
    }

    #[test]
    fn pick_model_prefers_configured_then_smallest() {
        let avail = vec![
            ("big:70b".to_string(), 40_000),
            ("small:1b".to_string(), 1_000),
        ];
        assert_eq!(pick_model("big:70b", &avail).as_deref(), Some("big:70b"));
        // preferred absent → smallest
        assert_eq!(pick_model("missing", &avail).as_deref(), Some("small:1b"));
        assert_eq!(pick_model("x", &[]), None);
        // Untagged preferred still resolves a tagged install (avoids falling
        // back to a leftover tiny thinking model).
        assert_eq!(
            pick_model(
                "qwen2.5",
                &[("qwen2.5:3b".into(), 1900), ("qwen3:0.6b".into(), 500)]
            )
            .as_deref(),
            Some("qwen2.5:3b")
        );
    }

    #[test]
    fn likely_thinking_model_matches_qwen3_and_gemma3() {
        assert!(likely_thinking_model("qwen3:0.6b"));
        assert!(likely_thinking_model("gemma3:1b"));
        assert!(!likely_thinking_model("qwen2.5:1.5b"));
        assert!(!likely_thinking_model("llama3.2:1b"));
    }

    #[test]
    fn runner_crash_errors_are_detected_for_cpu_retry() {
        assert!(is_runner_crash(
            "Ollama API 500 Internal Server Error: llama runner process has terminated: %!w(<nil>)"
        ));
        assert!(is_runner_crash("runner process has terminated"));
        assert!(is_runner_crash("CUDA error: out of memory"));
        assert!(is_runner_crash("ggml_gallocr_reserve_n failed"));
        assert!(!is_runner_crash("model not found"));
    }

    #[test]
    fn parse_backend_accepts_spellings_and_defaults_local() {
        assert!(matches!(parse_backend("llamaCpp"), Backend::LlamaCpp));
        assert!(matches!(parse_backend("llama_cpp"), Backend::LlamaCpp));
        assert!(matches!(parse_backend("ollama"), Backend::Ollama));
        assert!(matches!(parse_backend("cloud"), Backend::Cloud));
        assert!(matches!(parse_backend("openai"), Backend::Cloud));
        assert!(matches!(parse_backend("openaiCompat"), Backend::Cloud));
        // Anything unrecognised must never silently become a remote backend.
        assert!(matches!(parse_backend(""), Backend::Ollama));
        assert!(matches!(parse_backend("groq"), Backend::Ollama));
    }

    #[test]
    fn pick_model_skips_embedding_models_in_fallback() {
        // nomic-embed-text is the smallest, but it can't generate — must be
        // skipped so the chat fallback never 400s on /api/generate.
        let avail = vec![
            ("nomic-embed-text".to_string(), 270),
            ("gemma:2b".to_string(), 1_600),
            ("qwen:9b".to_string(), 9_000),
        ];
        assert_eq!(pick_model("missing", &avail).as_deref(), Some("gemma:2b"));
        // A preferred embedding model is still honoured if explicitly chosen.
        assert_eq!(
            pick_model("nomic-embed-text", &avail).as_deref(),
            Some("nomic-embed-text")
        );
        // Only embedding models installed → no chat model.
        assert_eq!(pick_model("x", &[("all-minilm".to_string(), 50)]), None);
        assert!(is_embedding_model("nomic-embed-text"));
        assert!(!is_embedding_model("gemma:2b"));
    }

    #[test]
    fn summary_prompt_includes_key_facts() {
        let p = summary_prompt(&repo());
        assert!(p.contains("RepoHarbor"));
        assert!(p.contains("Rust"));
        assert!(p.contains("7 uncommitted"));
        assert!(p.contains("branch main"));
    }

    #[test]
    fn pr_prompt_and_draft_split_round_trip() {
        let commits = vec!["feat: two".to_string(), "feat: one".to_string()];
        let p = pr_prompt("feat/x", &commits);
        assert!(p.contains("feat/x"));
        assert!(p.contains("feat: two"));
        assert!(p.contains("FIRST line"));

        // A well-formed draft splits into title + body.
        let (title, body) = split_pr_draft("feat: add x\n\n- one\n- two").unwrap();
        assert_eq!(title, "feat: add x");
        assert_eq!(body, "- one\n- two");
        // Title-only drafts still work; empty drafts don't.
        assert_eq!(
            split_pr_draft("\"feat: quoted\"").unwrap(),
            ("feat: quoted".to_string(), String::new())
        );
        assert!(split_pr_draft("   \n").is_none());
    }

    #[test]
    fn split_commit_message_separates_subject_and_body() {
        let (s, b) = split_commit_message("feat(x): add y\n\nLonger reasoning.\nSecond line.");
        assert_eq!(s, "feat(x): add y");
        assert_eq!(b, "Longer reasoning.\nSecond line.");
        // Subject-only messages give an empty body; wrapping quotes/backticks drop.
        assert_eq!(
            split_commit_message("`fix: subject only`\n"),
            ("fix: subject only".to_string(), String::new())
        );
        // A body directly after the subject (no blank line) still splits.
        let (s, b) = split_commit_message("fix: a\nbody line");
        assert_eq!((s.as_str(), b.as_str()), ("fix: a", "body line"));
        assert_eq!(split_commit_message("  "), (String::new(), String::new()));
        // Fenced model output must not wipe the subject (trim_matches on ```).
        let (s, b) = split_commit_message("```\nfeat(foo): add bar\n\nWhy it changed.\n```");
        assert_eq!(s, "feat(foo): add bar");
        assert_eq!(b, "Why it changed.");
        let (s, b) = split_commit_message("```text\nfix: z\n\nBody.\n```");
        assert_eq!(s, "fix: z");
        assert_eq!(b, "Body.");
    }

    #[test]
    fn fallback_commit_message_uses_changelog_bullet() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("mod")).unwrap();
        std::fs::write(
            root.join("mod/CHANGELOG.md"),
            "# Changelog\n\n## [Unreleased]\n- Penalty bridge for absences.\n",
        )
        .unwrap();
        let diff = "diff --git a/mod/models/x.py b/mod/models/x.py\n+pass\n";
        let msg = fallback_commit_message(diff, root.to_str().unwrap());
        assert!(msg.starts_with("chore: Penalty bridge"), "{msg}");
        assert!(!msg.contains("working tree"), "{msg}");
    }

    #[test]
    fn fallback_commit_message_names_files_without_changelog() {
        let diff = "\
diff --git a/src/main.rs b/src/main.rs
+++ b/src/main.rs
diff --git a/src/lib.rs b/src/lib.rs
+++ b/src/lib.rs
";
        let msg = fallback_commit_message(diff, "");
        assert_eq!(msg, "chore: update main.rs and lib.rs");
    }

    #[test]
    fn fallback_commit_message_appends_odoo_version() {
        let diff = "\
diff --git a/addons/foo/__manifest__.py b/addons/foo/__manifest__.py
--- a/addons/foo/__manifest__.py
+++ b/addons/foo/__manifest__.py
@@ -1,3 +1,3 @@
 {
-    'version': '19.0.1.1.0',
+    'version': '19.0.1.2.0',
 }
";
        let msg = fallback_commit_message(diff, "");
        assert!(msg.contains("19.0.1.2.0"), "{msg}");
        assert!(msg.starts_with("chore: update"), "{msg}");
    }

    #[test]
    fn fallback_pr_draft_uses_latest_subject_and_lists_commits() {
        let commits = vec!["feat: two".to_string(), "feat: one".to_string()];
        let (title, body) = fallback_pr_draft("feat/x", &commits);
        assert_eq!(title, "feat: two");
        assert_eq!(body, "- feat: two\n- feat: one");
        // No commits (shouldn't happen, but): the branch name still titles it.
        let (title, body) = fallback_pr_draft("feat/x", &[]);
        assert_eq!(title, "feat/x");
        assert!(body.is_empty());
    }

    #[test]
    fn commit_prompt_includes_diff_and_clamps() {
        let p = commit_prompt("diff --git a/x b/x\n+hello");
        assert!(p.contains("Conventional Commit"));
        assert!(p.contains("WHAT changed and WHY"));
        assert!(p.contains("+hello"));
        // very long diffs are clamped (budget leaves room for the prompt wrapper)
        let big = "x".repeat(20_000);
        let clamped = commit_prompt(&big);
        assert!(clamped.len() < COMMIT_DIFF_CLAMP + 2_000);
        assert!(!clamped.contains(&"x".repeat(COMMIT_DIFF_CLAMP + 1)));
    }

    #[test]
    fn parse_odoo_version_line_accepts_common_forms() {
        assert_eq!(
            parse_odoo_version_line("    'version': '19.0.1.2.0',"),
            Some("19.0.1.2.0".into())
        );
        assert_eq!(
            parse_odoo_version_line("+    \"version\": \"18.0.1.0.1\""),
            Some("18.0.1.0.1".into())
        );
        assert_eq!(parse_odoo_version_line("name = 'foo'"), None);
        assert_eq!(parse_odoo_version_line("--- a/__manifest__.py"), None);
    }

    #[test]
    fn odoo_manifest_version_hint_from_diff_and_disk() {
        let diff = "\
diff --git a/addons/foo/__manifest__.py b/addons/foo/__manifest__.py
--- a/addons/foo/__manifest__.py
+++ b/addons/foo/__manifest__.py
@@ -1,5 +1,5 @@
 {
-    'version': '19.0.1.1.0',
+    'version': '19.0.1.2.0',
     'name': 'Foo',
 }
";
        let hint = odoo_manifest_version_hint(diff, None).expect("hint");
        assert!(hint.contains("19.0.1.2.0"), "{hint}");
        assert!(hint.contains("Odoo module version"));

        // No manifest in the change set → no hint.
        assert!(odoo_manifest_version_hint("diff --git a/x.rs b/x.rs\n+hi", None).is_none());

        // On-disk read wins when repo_path points at a real manifest.
        let dir = tempfile::tempdir().unwrap();
        let mod_dir = dir.path().join("addons/foo");
        std::fs::create_dir_all(&mod_dir).unwrap();
        std::fs::write(
            mod_dir.join("__manifest__.py"),
            "{\n    'version': '19.0.9.9.9',\n    'name': 'Foo',\n}\n",
        )
        .unwrap();
        let hint = odoo_manifest_version_hint(diff, Some(dir.path().to_str().unwrap())).unwrap();
        assert!(hint.contains("19.0.9.9.9"), "{hint}");

        let prompt = commit_prompt_with_context(diff, None);
        assert!(prompt.contains("19.0.1.2.0"));
        assert!(prompt.contains("MUST mention the module version"));
    }

    #[test]
    fn non_odoo_prompt_never_asks_for_a_module_version() {
        // Stating the Odoo rule unconditionally made models invent a version on
        // repos that have no manifest at all.
        let p = commit_prompt("diff --git a/src/main.rs b/src/main.rs\n+fn main() {}\n");
        assert!(!p.contains("MUST mention the module version"), "{p}");
        assert!(!p.to_lowercase().contains("__manifest__"), "{p}");
        assert!(p.contains("Do not invent version numbers"));
    }

    #[test]
    fn changed_paths_in_diff_lists_files_and_skips_dev_null() {
        let diff = "\
diff --git a/mod/views/foo.xml b/mod/views/foo.xml
--- a/mod/views/foo.xml
+++ b/mod/views/foo.xml
diff --git a/old.py b/old.py
--- a/old.py
+++ /dev/null
";
        let paths = changed_paths_in_diff(diff);
        assert_eq!(paths, vec!["mod/views/foo.xml", "old.py"]);
    }

    #[test]
    fn changelog_excerpt_prefers_unreleased_section() {
        let text = "\
# Changelog

## [Unreleased]
### Added
- Penalty bridge for absences.

## [19.0.1.1.0] - 2026-01-01
- Older entry nobody asked about.
";
        let ex = changelog_excerpt(text);
        assert!(ex.contains("Penalty bridge"), "{ex}");
        assert!(!ex.contains("Older entry"), "{ex}");

        // An empty Unreleased heading falls through to the trailing entries.
        let empty = "# Changelog\n\n## Unreleased\n\n## [1.0.0]\n- First release.\n";
        assert!(changelog_excerpt(empty).contains("First release"));

        // No Keep-a-Changelog structure at all: still yields the tail.
        assert!(changelog_excerpt("just some notes\nand more\n").contains("just some notes"));
        assert!(changelog_excerpt("").is_empty());
    }

    #[test]
    fn nearby_changelogs_walks_up_from_changed_files_only() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let touched = root.join("addons/touched");
        let other = root.join("addons/other");
        std::fs::create_dir_all(&touched).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(
            touched.join("CHANGELOG.md"),
            "## Unreleased\n- Adds the penalty bridge.\n",
        )
        .unwrap();
        std::fs::write(other.join("CHANGELOG.md"), "## Unreleased\n- Unrelated.\n").unwrap();

        let diff = "diff --git a/addons/touched/models/x.py b/addons/touched/models/x.py\n+pass\n";
        let found = nearby_changelogs(diff, root.to_str().unwrap());
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].0, "addons/touched/CHANGELOG.md");
        assert!(found[0].1.contains("penalty bridge"));

        // A module with no changelog of its own finds nothing (no repo-root file).
        let diff_other = "diff --git a/addons/empty/x.py b/addons/empty/x.py\n+pass\n";
        assert!(nearby_changelogs(diff_other, root.to_str().unwrap()).is_empty());
        // A missing repo path is not an error.
        assert!(nearby_changelogs(diff, "").is_empty());
    }

    #[test]
    fn commit_prompt_with_extras_injects_notes_and_style() {
        let extras = CommitExtras {
            changelogs: vec![(
                "addons/foo/CHANGELOG.md".to_string(),
                "## Unreleased\n- Adds a bridge.".to_string(),
            )],
            recent_subjects: vec!["feat(foo): earlier work".to_string()],
        };
        let p = commit_prompt_with_extras("diff --git a/x b/x\n+hi", None, &extras);
        assert!(p.contains("addons/foo/CHANGELOG.md"));
        assert!(p.contains("Adds a bridge"));
        assert!(p.contains("feat(foo): earlier work"));
        assert!(p.contains("do not copy them verbatim"));
        assert!(p.contains("+hi"));

        // Empty extras leave the prompt exactly as the diff-only form.
        assert_eq!(
            commit_prompt_with_extras("diff --git a/x b/x\n+hi", None, &CommitExtras::default()),
            commit_prompt("diff --git a/x b/x\n+hi")
        );
    }

    #[test]
    fn manifest_paths_in_diff_collects_headers() {
        let diff = "diff --git a/m/__manifest__.py b/m/__manifest__.py\n+++ b/m/__openerp__.py\n";
        let paths = manifest_paths_in_diff(diff);
        assert!(paths.iter().any(|p| p == "m/__manifest__.py"));
        assert!(paths.iter().any(|p| p == "m/__openerp__.py"));
    }
}
