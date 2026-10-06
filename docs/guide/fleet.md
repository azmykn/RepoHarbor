# Fleet operations

RepoHarbor isn't just a viewer — it operates across many repos at once, and keeps track of the work you launch from it.

## Bulk actions

Select repos in [Mission Control](./mission-control) (each card / list row has a checkbox) and the ops row shows selection-scoped git verbs. Targets are always **selected ∩ visible** under the current Mission Control filters (work mode, chips, root, language, group, TREE, Needs me, search) — off-filter checkboxes never run.

| Action | What it does |
|---|---|
| **Fetch** | Fetch all remotes for the selection. |
| **Pull** | Fast-forward when possible; on pushable trees, diverged history may rebase onto upstream (`--empty=keep`). Pull-only vendor trees stay fast-forward-only. |
| **Push** | Push repos that are ahead of upstream. **Pull-only** paths (`core` / `custom` / configured prefixes) are dropped with **Skipped pull-only**; an all–pull-only selection shows an Error and does not run. Push stays disabled when every selected ahead repo is pull-only. |
| **Gen only** | AI commit message for dirty repos (one-click). Single target opens the drawer Changes composer; multi-repo drafts via the fleet engine. |
| **Gen & push** | AI message → commit all → push. **One dirty repo** stays one-click; **two or more** opens a centered confirm modal (repo list + Confirm / Cancel; Esc cancels) before running. |
| **Empty commit** | `git commit --allow-empty` on non–pull-only paths only (same skip toast as Push). Hidden when the selection is entirely pull-only. |
| **Actions ▾** | Stage all, Commit…, Discard, Prune, Reset hard, Mute attention, Open in IDE, Clear selection. Sync verbs (Fetch / Pull / Push / Gen / Empty) stay on the ops-row primaries so the gear stays slim. Card / TREE right-click still includes the full sync set. |

Always-on (no selection required):

| Action | What it does |
|---|---|
| **Refresh** | Re-scan workspace roots. |
| **Pull behind** | Fleet-pull every repo currently behind upstream. |
| **Fetch all** | Host enrichment refresh (ignores TTL). |
| **Submodules** | Update nested checkouts. With a selection → selected ∩ visible; with an **empty** selection → every *visible* parent with `child_count > 0`. Idle empty selection shows **Submodules (N)**; starting toasts list parent names. Progress is **parent repos** (`Updating submodules — name (2/3)…`); Log lines are `Submodules: parent — child: status`. |
| **Summarize** | One-line AI summaries when `aiReady`. |

Results stream back **per repo** as each finishes — done, skipped, or error — and a long run can be **cancelled** mid-flight. Clear the selection from the bar (or Esc) when you're done. Destructive ops (Discard / Prune / Reset) expand a confirm strip before running; multi-repo **Gen & push** uses a centered confirm modal instead.

## Job queue

Only **one** fleet job executes at a time (the engine may still fan out across repos *inside* that job). Starting another op while a run is active **enqueues** it — toast `Queued: Pull (3 repos)` — instead of silently doing nothing. The bottom strip shows the active progress plus `· N queued`. **Cancel** stops the active run and clears waiting jobs so nothing starts afterward. Same-repo writes stay sequential because jobs never overlap.

## Agent & terminal sessions

Every time you launch a terminal coding agent from a card, RepoHarbor tracks the process. The **Agents** view is a dashboard of those live sessions:

- See each session's repo, the command it was launched with, and how long it's been running.
- **Terminate** a session, **reopen** one in the same repo, or jump to the repo in your **IDE** or **file manager**.
- The list reaps dead sessions automatically and refreshes on a short poll.

It's the answer to "what did I leave running, and where?" across a busy day.
