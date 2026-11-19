# Configuration

RepoHarbor's configuration lives in a plain TOML file at `~/.config/repoharbor/config.toml`, editable from **Settings** or by hand. The cache and AI data live separately under `~/.local/share/repoharbor/`.

## Settings sections

The Settings screen switches sections from the sidebar:

- **Workspace roots** — directories scanned for git repos, scan depth, and ignore globs.
- **Launchers** — IDE and terminal-agent presets, built-in PTY terminal default/shell override, destructive command confirmation, and built-in agent dispatch toggle (see [Launchers](./launchers)).
- **GitHub** — connect for higher rate limits and private-repo enrichment.
- **AI & search** — inference backend (Ollama or the bundled llama.cpp engine), models, and the cache control (see [Local AI](./local-ai)).
- **Notifications & Startup** — background desktop notifications, login autostart (`~/.config/autostart`), and desktop shortcut/icon repair (see [Notifications & tray](./notifications)).
- **About & license** — product version (from the crate that built the binary), **Updates & installation** (install mode, periodic update check toggle, release notes, **Update & Restart**), repository, contact, MIT public-use terms, and attribution (see [Copyright & contact](./copyright)).

## Workspace roots

Point RepoHarbor at one or more directories. It walks each up to **scan depth** levels deep looking for git repos, skipping anything matching the **ignore** list.

| Setting | Default | Notes |
|---|---|---|
| Roots | auto-detected (`~/odoo`, `~/Tools`, `~/dev`, …) | One or more directories to scan. |
| Scan depth | `3` | How many levels deep to descend (1–8). |
| Ignore | `node_modules, .cache, vendor, target, dist, .git` | Comma-separated directory names to skip. |
| Pull-only paths | seeded `core` / `custom` under roots | Pull to update; hide Push / Gen & push / Empty commit; silence Ahead + upstream CI on Needs me. Removals persist via `pullOnlyOptOut` so cold-load heal won't undo them; new Odoo `core`/`custom` trees still auto-add. |
| Mute attention paths | _(empty)_ | Path prefixes excluded from Needs me / attention chips / tray (`muteAttentionPrefixes` in TOML). Does not edit repo files. |

## Hosts

Public repos enrich without signing in (and an authenticated `gh` CLI is used automatically if present). Connect an account for higher rate limits and private-repo data.

For **self-hosted GitLab**, only explicitly trusted domains are ever sent a token — a repo's remote domain can't trick RepoHarbor into leaking credentials to an arbitrary host.

## Where things live

| Path | Contents |
|---|---|
| `~/.config/repoharbor/config.toml` | Your settings (non-secret preferences, roots, launchers, pool deployment list). |
| `~/.local/share/repoharbor/pool_keys.toml` | Per-row AI provider API keys (`0600` owner-only permissions). |
| `~/.local/share/repoharbor/install-meta.toml` | Install mode (`binary`, `source`, `appimage`, `package`), source path & version. |
| `~/.config/autostart/com.digitscode.repoharbor.desktop` | Optional login autostart entry. |
| `~/.local/share/repoharbor/cache.sqlite` | Repo snapshot, host enrichment, favorites, notes, AI summaries & embeddings. |
| `~/.local/share/repoharbor/models/` | GGUF models downloaded for the llama.cpp backend. |
| `~/.local/share/repoharbor/bin/` | The llama.cpp engine (bundled with release builds, unpacked here on first use). |

The repo snapshot and host enrichment are rehydrated on launch, so Mission Control paints instantly and visibility/stars survive restarts without a re-fetch.

::: tip Don't commit these
Everything under `~/.config/repoharbor` and `~/.local/share/repoharbor` is machine-local — never check it into a repo. It can hold host tokens and your personal data. See [Privacy & local data](./privacy).
:::
