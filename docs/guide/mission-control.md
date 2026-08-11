# Mission Control

Mission Control is the home view: a windowed grid of every repo RepoHarbor found across your workspace roots. It's built to scale to hundreds of repos — the grid is virtualized, so only what's on screen is rendered.

> **Screenshots:** re-capture from the native app after the RepoHarbor rebrand.
> Avoid private remotes and machine-specific home paths in published images
> (prefer generic `~/dev/…` layouts).

## The repo card

Each card fuses local git state with host enrichment and an optional AI summary:

- **Language logo** — a full-colour mark for the detected primary language.
- **Slug & path** — `owner/repo` (if it has a remote) and the abbreviated on-disk path.
- **Description** — the README's first line, with the AI summary on its own line when available.
- **Git status row** — branch, ahead/behind divergence, and uncommitted-change count. Clean repos stay quiet — no badge, since clean is the unremarkable default.
- **Host row** — a lock for private repos, CI state, stars, latest release, last-commit time, and the host icon.
- **Launch actions** — Open in IDE, Agent, reveal Folder, and Open on the host. See [Launchers](./launchers).

## Work modes & filtering

The toolbar uses **work modes** instead of a dense all-filters chip strip:

| Mode | Shows |
|------|--------|
| **Needs me** | Attention filter — repos that need action (reviews, dirty work, behind, …). Segment shows the count when &gt; 0. Empty state: **All clear**. |
| **Behind** | Repos behind upstream. **Pull behind** stays on the toolbar to fleet-pull them. |
| **Working** | Contextual chips: Dirty / Stageable / Pushable (no Commitable / Ahead duplicates). |
| **All** | No git filter; optional Public / Private / Starred / Stale chips. |

Also on the toolbar:

- **Filter…** — substring search over name / slug / path.
- **Pull behind** — fleet-pulls every repo currently behind upstream.
- **More ⋮** — Fetch all (host enrichment refresh) and Summarize (when local AI is ready).
- **Sort: recent** / **Sort: name** — card ordering (not a heatmap toggle).
- **Grid | List** — layout switch.

Row two holds select-all, **Actions ▾** (only when something is selected), and the mode's contextual chips.

### Projects & saved views

The sidebar adds two ways to carve up a large workspace:

- **Projects** — tag repos to group related work, then filter the grid to one tag.
- **Saved views** — capture the current filter/sort/visibility combination as a named preset and jump back to it in one click. Presets persist locally.

Roots & Languages in the sidebar still filter by workspace root and detected language.

## Selecting repos

Each card has a checkbox; select one or more (or use the toolbar's select-all) to bring up the **fleet bar** for batch git operations across the selection. See [Fleet operations](./fleet).

**Actions ▾** appears next to select-all only when there is a selection; it runs the same fleet ops (Fetch, Pull, Stage, Commit, Discard, Submodule Update, …). Keyboard shortcuts on the current selection:

- <kbd>Ctrl/Cmd+Shift+F</kbd> — Fetch selected
- <kbd>Ctrl/Cmd+Shift+P</kbd> — Pull selected

**Pull behind** (toolbar / command palette) selects every repo with `behind > 0` and fleet-pulls them — useful for upstream Odoo/core trees you keep current without hunting the Behind mode. Pair with **pull-only prefixes** in Settings so those trees never offer Push and upstream CI stays off the Needs me / Attention chips (silence — you'll only hear about it if a push is attempted and fails).

## List view

Switch to a compact, single-line view from the Grid | List control — useful when you're scanning a lot of repos at once.

## The repo drawer

Click a card to slide out a detail drawer with the repo's branches, recent commits, staged-diff view, README, a PR/CI panel, and per-repo notes — plus the same launch actions in the footer.

It's covered in full on its own page: [The repo drawer](./repo-drawer).

## Command palette

Press <kbd>⌘K</kbd> / <kbd>Ctrl K</kbd> to search repos and run commands without leaving the keyboard.

It also does **cross-repo code search**: type a query and RepoHarbor runs [ripgrep](https://github.com/BurntSushi/ripgrep) across your repos, returning matching files and lines you can open directly — a fast way to find that one call site across the whole fleet.
