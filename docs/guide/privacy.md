# Privacy & local data

RepoHarbor is local-first. Nothing in this guide should be committed to a public git
remote.

## Stay on the machine

| Path | Contents |
|------|----------|
| `~/.config/repoharbor/config.toml` | Roots, pull-only prefixes, AI/host settings. May list personal absolute paths. |
| `~/.local/share/repoharbor/cache.sqlite` | Repo snapshot, favorites, host enrichment, CI cache. |
| `~/.local/share/repoharbor/github_token` (or similar) | Host tokens — never share or commit. |

`.gitignore` in this repository blocks `.env`, PEMs, `*github_token*`, and
accidental `cache.sqlite` copies. Do not force-add those paths.

## Publishing the fork or docs

- Prefer `~/…` or fictional paths in docs, placeholders, and Settings examples —
  not a real `/home/<you>/…` tree.
- Recapture `docs/public/shots/` without private remotes, customer module names,
  or absolute client paths in the frame.
- The in-app **Log** view redacts your home directory to `~` before display so
  screenshots leak less identifying path prefix.

## Attention filter vs secrets

Mission Control **Attention** flags git/host work (dirty trees, PRs, CI), not a
secret scanner. Treat it as a work queue — not an audit that your public fork is
clean.
