# RepoHarbor — User Manual (`v2026.10.14`)

**Every repo at harbor.** A native Linux GPU-rendered desktop command center (`GPUI` + Vulkan, Wayland/X11) for managing dozens or hundreds of git repositories, submodules, forge reviews, AI workflows, and interactive terminals from one unified workspace.

---

## Table of Contents

1. [Installation & First Launch](#1-installation--first-launch)
2. [Mission Control & Sidebar Filters](#2-mission-control--sidebar-filters)
3. [Submodule Inspection (`N sub`)](#3-submodule-inspection-n-sub)
4. [Fleet Operations & Git Safety](#4-fleet-operations--git-safety)
5. [Repository Drawer (`Work`, `History`, `About`)](#5-repository-drawer-work-history-about)
6. [Embedded PTY Terminal & Live Action Log](#6-embedded-pty-terminal--live-action-log)
7. [AI Features, Provider Pool & Failover](#7-ai-features-provider-pool--failover)
8. [Inbox, Feed, Explore, Agents, Cleanup, Dev Tools & Log](#8-inbox-feed-explore-agents-cleanup-dev-tools--log)
9. [Command Palette & Keyboard Shortcuts](#9-command-palette--keyboard-shortcuts)
10. [Periodic Updates & CLI Subcommands](#10-periodic-updates--cli-subcommands)

---

## 1. Installation & First Launch

### Recommended One-Command Install (Zero `sudo`)

```bash
# Install latest GitHub Release into ~/.local/bin/repoharbor + desktop icon
curl -fsSL https://raw.githubusercontent.com/azmykn/RepoHarbor/main/scripts/install.sh | bash
```

Optional installer flags:

| Flag | Effect |
|---|---|
| `--from-source` | Installs dependencies (`scripts/setup.sh`), builds `--release`, and installs to `~/.local/bin/repoharbor` |
| `--autostart` | Enables login autostart (`~/.config/autostart/com.digitscode.repoharbor.desktop`) |
| `--with-auto-update` | Enables a daily `systemd --user` update timer (`repoharbor-update.timer`) |
| `--uninstall` / `--purge` | Removes binary, desktop shortcuts, icons, and timers (add `--purge` to delete config/cache) |

### First-Run Onboarding

On a fresh install, RepoHarbor automatically checks standard workspace locations (`~/odoo`, `~/Tools`, `~/dev`, `~/projects`, `~/work`, `~/code`, `~/src`, `~/repos`) and surfaces a compact **First-run setup** banner at the top of Mission Control:

1. **Pick workspace folders / Auto-detect folders** — choose or auto-discover directories containing git repos.
2. **Connect GitHub** — sign in via GitHub OAuth Device Flow (or automatic `gh` CLI token).
3. **Pin desktop shortcut** — registers `.desktop` entry and hicolor icons (`com.digitscode.repoharbor`).
4. **Autostart: On/Off** — toggle launching automatically on login.

---

## 2. Mission Control & Sidebar Filters

- **Layout Switch (`List | Grid`)**: Opens in dense single-line **List** rows by default; switch to **Grid** for multi-column cards.
- **Work Modes**:
  - **Needs me**: Pending PR reviews, failing CI, dirty/unpushed local work, behind upstream, and completed dispatched agents.
  - **Behind**: Repositories lagging behind remote upstream.
  - **Working**: Repositories with dirty working trees (`Dirty`), staged changes (`Stageable`), or unpushed commits (`Pushable`).
  - **All**: Full workspace catalog with optional `Public`, `Private`, `Starred`, and `Stale` filters.
- **Search Filter (`/` or `Ctrl+F`)**: Supports plain text plus structured tokens:
  - `is:dirty`, `is:behind`, `is:ahead`, `lang:rust`, `host:github`
  - Click the **`✕`** icon inside the search box (or press `Esc`) to clear immediately.
- **Left Sidebar**:
  - **GROUPS**: Workspace path groups (auto-seeded for Odoo `Core` / `Digits` / `Custom` trees).
  - **VIEWS**: Save and recall custom filter + sort presets in one click.
  - **ROOTS**: Filter by workspace root directory.
  - **LANGUAGES**: Filter by detected primary language.
  - **PULLED**: Lists files updated by the most recent fast-forward `Pull`.

---

## 3. Submodule Inspection (`N sub`)

- Child submodules are hidden from the main list by default unless flagged in **Needs me**.
- Parent repositories display a clickable **`N sub`** pill (e.g., `6 sub`) in both **List** and **Grid** views.
- Clicking **`N sub`** focuses Mission Control on that parent repository and its submodules, displaying a top banner:
  - `Showing submodules of <RepoName> (N submodule(s))` + **`Show all repos`** reset button.

---

## 4. Fleet Operations & Git Safety

- **Global / Visible Ops**: `Refresh`, `Pull behind`, `Enrich hosts`, `Submodules`, `Summarize`.
- **Selection-Scoped Ops** (select checkboxes or `Select all`): `Fetch`, `Pull`, `Push`, `Gen only`, `Gen & push`, `Empty commit`, plus **`Actions ▾`** (`Stage all`, `Commit…`, `Discard changes`, `Merge branch into HEAD…`, `Prune`, `Hard reset`, `Mute attention`, `Open in IDE`).
- **Diverged Rebase Recovery**: Pushable repositories that are both ahead and behind rebase cleanly (`git rebase --empty=keep @{upstream}`); dirty trees are never rewritten.
- **Pull-Only Protection**: Paths under `pullOnlyPrefixes` (e.g., vendor `core` / `custom`) allow `Pull` and `Gen only`, while blocking `Push`, `Gen & push`, and `Empty commit`.
- **GH013 Branch Protection Detection**: Distinguishes remote branch-protection rule rejections (`GH013`) from non-fast-forward errors and offers a one-click `git checkout -b fix/update && git push -u origin HEAD` command in the Notice modal.
- **Pre-Commit Secret Scanner**: Automatically aborts commits whose message contains raw API tokens (`gsk_*`, `mstrl_*`, `ghp_*`, `sk-*`).

---

## 5. Repository Drawer (`Work`, `History`, `About`)

Click any repository row or card to open its inspection drawer:

- **Work**: Staged & unstaged file lists, syntax-highlighted diff viewer, inline **Copy full path** / **Copy relative path** / **Open in IDE** / **Reveal in File Manager**, external diff launcher (`meld`, `code --diff`), **Fast commit**, **Gen only**, **Gen & push**, branch switcher, quick **Merge**, and **Abort merge** on conflicts.
- **History**: Commit timeline with hashes, authors, relative timestamps, and diff stats.
- **About**: Rendered Markdown `README.md`, extracted **Commands in README** (`▶ Run` / `Paste` into built-in terminal), remotes, and **Submodules** inspector.

---

## 6. Embedded PTY Terminal & Live Action Log

- Toggle with **<kbd>Ctrl+\`</kbd>** or the top-right **Terminal** button.
- **Multi-Tab Interactive PTY**: Spawn repository-scoped shells (`+ New` / `Ctrl+Shift+T`), close (`Ctrl+Shift+W`), drag-resize splitter, Expand/Restore, mouse drag / double-click word / triple-click line selection (copy with `Ctrl+C` while selected, `Ctrl+Shift+C`, **`Copy sel`**, or **right-click**), Paste (`Ctrl+Shift+V`), `Ctrl+C` (`SIGINT`), Clear, and scrollback with **Jump to bottom**.
- **Zero-Drift Cursor**: Aligned directly via GPUI's `TextSystem` monospace shaper.
- **Live `Action Log` Tab**: Streams every toolbar and fleet operation (`[HH:MM:SS] $ <command>`, per-repo `✓`/`↷`/`✗` status, and final summary). Toggle **Auto-log: On/Off** in the dock bar.
- **Safety Confirmation Strip**: Potentially destructive commands (`rm -rf`, `sudo`, `--force`, `curl | sh`, `git reset --hard`) clicked from READMEs, Notice modals, or Activity Log pause behind an inline **Press Enter / Dismiss** review bar when `terminalConfirmDestructive` is enabled.

---

## 7. AI Features, Provider Pool & Failover

Configure in **Settings → AI & search**:

- **Backends**: Local **Ollama**, bundled/external **llama.cpp** (GGUF), **OpenRouter**, **Groq**, **Mistral**, or any **OpenAI-compatible** endpoint.
- **Provider Pool & Failover Router**: Add unlimited ordered deployments with priority **↑ / ↓**, enable/disable switches, inline editing, live **Test** latency checks, and automatic 60-second cooldown failover on `429`/`402`/`503`/transport failures.
- **Secure Key Storage**: Per-row API keys are stored owner-only (`0600`) in `~/.local/share/repoharbor/pool_keys.toml` (never in `config.toml`); local loopback endpoints work keyless.
- **AI Capabilities**: Semantic diff commit messages, repo README summaries, AI changelog generation, and semantic vector search in `Ctrl+K` (using `nomic-embed-text`). All AI UI hides cleanly when offline/disabled.

---

## 8. Inbox, Feed, Explore, Agents, Cleanup, Dev Tools & Log

| View | Purpose |
|---|---|
| **Inbox** | Pending PR review requests, assigned PRs, and CI failures |
| **Feed** | Activity and release timeline for followed repositories |
| **Explore** | Search starred/org GitHub & GitLab repositories and clone in one click |
| **Agents** | Live `/proc` detector for running coding agents, prompt dispatch, and commit review diff |
| **Cleanup** | Identify and prune merged or stale local branches across the fleet |
| **Dev Tools** | Offline utility belt: UUID v4/v7, SHA-1/SHA-256 hashes, Base64/URL, JSON formatter, JWT decoder, timestamp & colour converters, regex tester |
| **Log** | Persistent session activity log with one-click runnable error recovery commands |

---

## 9. Command Palette & Keyboard Shortcuts

| Shortcut | Action |
|---|---|
| `Ctrl+K` / `Cmd+K` | Open Command Palette (fuzzy repo jump, git verbs, semantic & ripgrep code search) |
| `Ctrl+F` / `/` | Focus Mission Control repository filter |
| `Ctrl+\`` / `Cmd+\`` | Toggle Embedded Terminal & Action Log dock |
| `Ctrl+Shift+T` / `W` | New / close terminal tab (inside Terminal) |
| `Ctrl+Shift+C` / `V` | Copy / paste (inside Terminal) |
| `Ctrl+Shift+F` / `P` | Fetch / Pull selected repositories |
| `Esc` | Dismiss active overlay or clear selection |

---

## 10. Periodic Updates & CLI Subcommands

### CLI Commands

```bash
repoharbor                        # Launch desktop UI
repoharbor --version              # Print installed version (vYYYY.M.P)
repoharbor update                 # Check & apply update (binary SHA-256 swap or source rebuild)
repoharbor update --check         # Check only without installing
repoharbor update --quiet         # Silent mode for systemd timers
repoharbor install-desktop        # Repair user .desktop entry & icons (--autostart optional)
```

### In-App Periodic Updates

- RepoHarbor checks GitHub Releases automatically 4 seconds after launch and every 12 hours when **Check for periodic updates automatically** is enabled.
- When a newer version is available, a prominent **`Update <tag>`** button appears in the top header bar (next to `Refresh` and `Terminal`) alongside a clickable **`↑ vYYYY.M.P`** badge in the TitleBar.
- Click **`Update <tag>`** directly (or open **Settings → About → Updates & installation** to inspect release notes and click **Update & Restart**) to verify SHA-256, atomically swap the binary, and relaunch.
