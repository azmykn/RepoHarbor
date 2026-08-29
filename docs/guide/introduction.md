# Introduction

**RepoHarbor** points at the directories where you keep your projects, discovers every git repo inside them, and lays them out in a dark, dense "mission control" grid. Each card fuses three sources of truth:

1. **Local git** — branch, ahead/behind, uncommitted changes, last commit, detected language
2. **Your git host** *(GitHub & GitLab, incl. self-hosted)* — stars, topics, releases, issues, visibility
3. **Local AI** — a synthesized "what is this / what's been happening" blurb, generated on-device

…and every card is a launchpad: one click to open the repo in your IDE, or to drop a terminal coding agent (Claude Code, Aider, Codex, …) straight into it.

::: tip Screenshots
Docs shots will be re-captured from the native RepoHarbor UI after the rebrand. Until then, run the app (`cargo run -p repoharbor`) for the live Mission Control grid.
:::

## Why?

There's no great *workspace dashboard* for Linux. GitKraken is heavy and git-focused; GitHub Desktop has no Linux build and is single-repo. RepoHarbor is the at-a-glance morning view of everything you're working on — and the fastest way to jump back in.

## Status

::: warning EARLY DEVELOPMENT
RepoHarbor is in active early development. Packaged Linux builds (AppImage, `.deb`, `.rpm`) are published on the [releases page](https://github.com/azmykn/RepoHarbor/releases), or you can build from source — see [Getting started](./getting-started). Expect rough edges.
:::

## What's inside

Beyond the [Mission Control](./mission-control) grid, RepoHarbor has grown a full command center:

- **[The repo drawer](./repo-drawer)** — branches, history, staged-diff with AI commit messages, a PR/CI panel with quick-merge, and per-repo notes with a "what changed since I last looked" catch-up.
- **[Fleet operations](./fleet)** — multi-select repos for batch git ops, and a dashboard of every agent/terminal session you've launched.
- **[Notifications & tray](./notifications)** — background polling for PRs, reviews, and CI, surfaced as desktop notifications and a tray glance.
- **[Maintenance & tools](./maintenance)** — a branch janitor for merged/gone branches, and an offline developer utility belt.
- **[Inbox, Feed & Explore](./inbox-feed-explore)** — what's waiting on you, a release radar, and a browser for your starred repos.
- **[AI features](./local-ai)** — summaries, commit messages, changelogs, briefings, and semantic search, on-device via Ollama or a bundled llama.cpp engine (or an opt-in hosted endpoint when you want speed).

## Stack

| Layer | Choice |
|---|---|
| UI | Native Rust on [GPUI](https://www.gpui.rs) (Zed's GPU framework) — no webview |
| Rendering | GPU via `blade` (Vulkan); Wayland/X11 direct; [gpui-component](https://github.com/longbridge/gpui-component) widgets |
| Git | `git2` (libgit2, vendored) |
| Persistence | SQLite + TOML config (XDG dirs) |
| Hosts | GitHub + GitLab REST/GraphQL (incl. self-hosted) |
| AI | [Ollama](https://ollama.com) or a bundled [llama.cpp](https://github.com/ggml-org/llama.cpp) sidecar, over HTTP; optionally any OpenAI-compatible endpoint |

## How it fits together

RepoHarbor is a native Rust app on GPUI — no webview, no IPC. A three-crate workspace: `repoharbor-core` does the heavy lifting (scanning, git, host APIs, caching, AI calls), `repoharbor-platform` handles Linux desktop integration (tray, notifications, appearance), and the `repoharbor` app crate is the GPUI UI calling the core directly. A SQLite cache persists the repo snapshot and host enrichment so the grid **paints instantly on launch** and keeps working offline. Configuration lives in a plain TOML file under `~/.config/repoharbor/`.

Read on for [building from source](./getting-started), or jump into the [feature tour](./mission-control).
