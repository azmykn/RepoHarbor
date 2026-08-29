# Local AI

RepoHarbor's AI features run **on-device by default** — nothing leaves your machine. There are three interchangeable backends; pick one in **Settings → AI & search**. Two are local (Ollama, bundled llama.cpp) and one is an **opt-in** OpenAI-compatible cloud endpoint for when you want hosted speed. When AI is off or unreachable, every AI affordance is hidden rather than broken.

## What it powers

- **Repo summaries** — a synthesized "what is this / what's been happening" blurb on each card, generated on demand or in bulk.
- **Commit messages** — drafted from your staged diff in the repo drawer, enriched with the `CHANGELOG.md` nearest the files you touched and your recent commit subjects (see [Commit-message context](#commit-message-context)).
- **Changelogs** — summarised from recent history.
- **Daily briefing** — a one-line "here's where things stand" across your workspace.
- **Resume catch-up** — what changed in a repo since you last looked (see [The repo drawer](./repo-drawer)).
- **Semantic search** — embeddings-backed search over your repos. *(Ollama backend only.)*

## Choosing a backend

| | **Ollama** | **llama.cpp (bundled)** | **Cloud** |
|---|---|---|---|
| Setup | Install Ollama separately | Ships with release builds — download a model | Base URL + API key |
| Models | Any Ollama model | A GGUF you download in Settings | Whatever the endpoint offers |
| Embeddings / semantic search | ✅ | — (generation only) | — (always local) |
| Data stays on-device | ✅ | ✅ | ❌ prompts go to the endpoint |
| Best for | Existing Ollama users, semantic search | Zero-dependency, out-of-the-box generation | Fastest commit messages |

The backend selector takes effect immediately, and the panel only shows the controls relevant to the engine you picked.

## Setup — Ollama

1. [Install Ollama](https://ollama.com/download) and make sure it's running (default endpoint `http://localhost:11434`).
2. Pull a small chat model and an embedding model:

```sh
ollama pull qwen3:0.6b
ollama pull nomic-embed-text
```

3. In **Settings → AI & search**, choose the **Ollama** backend, confirm the endpoint, enable AI, and pick your models. Hit **Test** to verify the connection end-to-end.

::: tip Model choice
A small model like `qwen3:0.6b` is plenty for the short summaries RepoHarbor generates and keeps things fast. Point the endpoint at a remote Ollama host if you'd rather run the model elsewhere.
:::

## Setup — llama.cpp (bundled)

Release builds bundle a local `llama-server` engine, so there's nothing to install separately.

1. In **Settings → AI & search**, choose the **llama.cpp** backend.
2. Click **Download model** to fetch a small default GGUF (≈400 MB) into `~/.local/share/repoharbor/models/`. You can also point it at any GGUF URL.
3. Enable AI and generate a summary — the engine starts on demand and shuts down with the app.

The engine binary is discovered in this order: an explicit path you set in Settings, the bundled copy unpacked into `~/.local/share/repoharbor/bin/`, then `llama-server` on your `PATH`. If you build from source rather than installing a release, supply a [llama.cpp](https://github.com/ggml-org/llama.cpp) `llama-server` by any of those routes.

::: tip Generation only
The llama.cpp backend serves text generation. Embeddings — and therefore semantic search — stay on the Ollama backend, so that one feature is hidden while llama.cpp is selected.
:::

## Setup — Cloud (OpenAI-compatible)

A tiny local model takes tens of seconds to draft a commit message on a busy laptop; a small hosted one answers in a second or two. If that trade is worth it, point RepoHarbor at any OpenAI-compatible endpoint.

1. In **Settings → AI & search**, choose the **Cloud** backend.
2. Set **Base URL** to the endpoint's OpenAI-compatible root.
3. Set **Chat model** to a model that endpoint serves.
4. Paste your **API key** and hit **Save & rescan**, then **Test**.

Once the endpoint answers, the model list under the status row is populated from it — click a model to select it.

| Provider | Base URL | Model to start with |
|---|---|---|
| **Ollama Cloud** | `https://ollama.com/v1` | `gpt-oss:20b` (free tier; key from [ollama.com/settings/keys](https://ollama.com/settings/keys)) |
| **Groq** | `https://api.groq.com/openai/v1` | `llama-3.1-8b-instant` |
| **OpenAI** | `https://api.openai.com/v1` | `gpt-4o-mini` |
| **OpenRouter** | `https://openrouter.ai/api/v1` | any model ending in `:free` |
| **Google Gemini** | `https://generativelanguage.googleapis.com/v1beta/openai` | `gemini-3.6-flash` |
| **LM Studio** (local) | `http://localhost:1234/v1` | whatever you loaded |

::: tip Reasoning models
Models that "think" (gpt-oss, the Gemini 3 family, …) can spend an entire token budget on hidden reasoning and hand back an empty answer. RepoHarbor asks for `reasoning_effort: "low"` on the first attempt to avoid that — measured on Ollama Cloud's `gpt-oss:20b`, a full commit message in ~1.5 s instead of ~15 s. Providers that don't understand the field are retried without it.
:::

::: danger Your diffs leave the machine
On this backend every prompt — staged diffs, commit history, changelog notes, repo metadata — is sent to the endpoint you configured. Use it only for code you're comfortable sharing with that provider, and stay on Ollama / llama.cpp for anything else. Embeddings and semantic search never use the cloud backend; they remain local regardless.
:::

The key is **not** stored in `config.toml` (which people paste into bug reports). It's written owner-only (`0600`) to `~/.local/share/repoharbor/openai_api_key`, or you can supply `$REPOHARBOR_OPENAI_API_KEY` instead and leave the field empty. The field never displays a stored key: type to replace it, or use **Clear key** to forget it. The key is only ever sent to the base URL you configured, and that URL must be `https` unless it's `localhost`.

## Commit-message context

A diff alone rarely explains *why* something changed, so **Generate commit** adds two more sources before prompting the model:

- **The nearest changelog.** RepoHarbor walks up from each changed file to the repo root and takes the first `CHANGELOG.md` (also `.rst` / `.txt`, `CHANGES.md`, `HISTORY.md`, `NEWS.md`) it finds — up to three across a multi-module commit. It prefers the `## [Unreleased]` section, falling back to the newest entries. In a monorepo of Odoo modules this means you get the changelog of the module you actually touched, not dozens of unrelated ones.
- **Your last five commit subjects**, for scope naming and house style only.

So keeping a short `Unreleased` note per module pays off twice: the changelog stays current *and* the generated messages get sharply better. Odoo module versions from `__manifest__.py` are still injected as before.

## Where data is stored, and clearing it

Summaries and embeddings are cached in `~/.local/share/repoharbor/cache.sqlite`. A summary is keyed to the repo's last commit, so it regenerates after new work lands. Use **Clear AI cache** in settings to drop all summaries and embeddings.

## Turning it off

Uncheck **Enable AI features**. The grid, drawer, and toolbar drop every AI control — no empty placeholders, no broken buttons. Switching back to a local backend also stops all egress immediately; the stored API key is simply unused until you select **Cloud** again.
