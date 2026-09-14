# Mission Control

Mission Control is the home view: every repo RepoHarbor found across your workspace roots. It opens in **List** (compact rows) by default; **Grid** is the optional card layout. The list is virtualized, so only what's on screen is rendered.

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
| **Needs me** | Attention filter — repos that need action (reviews, dirty/unpushed work, behind, finished agents, …). Submodule children that need you stay listed even without a TREE focus; host-only review/CI (no local checkout) appears too. Badge / chip / header count only what the list can show. A live **Agent running** session is not a Needs-me hit (the card keeps a terminal icon; it does not replace the git/commit line). Segment shows the count when &gt; 0. Empty state: **All clear**. |
| **Behind** | Repos behind upstream. **Pull behind** on the ops row fleet-pulls them. |
| **Working** | Contextual chips: Dirty / Stageable / Pushable (no Commitable / Ahead duplicates). |
| **All** | No git filter; optional Public / Private / Starred / Stale chips. |

Filter row (top):

- **Filter…** — substring search over name / slug / path.
- **Sort: recent** / **Sort: name** — card ordering (not a heatmap toggle).
- **Grid | List** — layout switch (List is the default; persisted in `config.toml` as `layout`).

Ops row (bottom) — verbs that execute work, visible even with no selection:

- **Refresh** — labeled accent control that re-scans workspace roots (git status, attention, grid). The same action is on the chrome bar next to **+**. A toast reports **Scanning…** then **Scan finished**.
- **Pull behind** — fleet-pulls every repo currently behind upstream.
- **Fetch all** — host enrichment refresh (ignores TTL).
- **Summarize** — one-line AI summaries for every repo (when AI is ready). A toast tracks **Summarizing…** then success / already up to date / error; summaries appear on List rows and Grid cards (cached by commit).
- Select-all plus **Fetch / Pull / Push / Submodules / Gen commit / Empty commit** and **Actions ▾** when something is selected.

### Projects & saved views

The sidebar adds two ways to carve up a large workspace:

- **Projects** — tag repos to group related work, then filter the grid to one tag.
- **Saved views** — capture the current filter/sort/visibility combination as a named preset and jump back to it in one click. Presets persist locally.

Roots & Languages in the sidebar still filter by workspace root and detected language.

## Selecting repos

Each card has a checkbox; select one or more (or use the toolbar's select-all) to bring up the **fleet bar** for batch git operations across the selection. See [Fleet operations](./fleet).

**Actions ▾** appears on the ops row only when there is a selection; it runs the same fleet ops (Fetch, Pull, Stage, Commit, Discard, Submodule Update, …). Keyboard shortcuts on the current selection:

- <kbd>Ctrl/Cmd+Shift+F</kbd> — Fetch selected
- <kbd>Ctrl/Cmd+Shift+P</kbd> — Pull selected

**Pull behind** (ops row / command palette) selects every repo with `behind > 0` and fleet-pulls them — useful for upstream Odoo/core trees you keep current without hunting the Behind mode. Pair with **pull-only prefixes** in Settings so those trees never offer Push and upstream CI / local-only Ahead stay off Needs me (silence — you'll only hear about a push if one is attempted and fails).

## List view

List is the default home layout: compact single-line rows (AI summaries show as a truncated sparkles line when present). Switch to **Grid** for cards. The choice is saved in `~/.config/repoharbor/config.toml` (`layout = "list"` or `"grid"`). Parent → submodule browsing is the sidebar **TREE** section, not this switch.

## The repo drawer

Click a card to slide out a detail drawer with the repo's branches, recent commits, staged-diff view, README, a PR/CI panel, and per-repo notes — plus the same launch actions in the footer.

It's covered in full on its own page: [The repo drawer](./repo-drawer).

## Command palette

Press <kbd>⌘K</kbd> / <kbd>Ctrl K</kbd> to search repos and run commands without leaving the keyboard.

It also does **cross-repo code search**: type a query and RepoHarbor runs [ripgrep](https://github.com/BurntSushi/ripgrep) across your repos, returning matching files and lines you can open directly — a fast way to find that one call site across the whole fleet.
