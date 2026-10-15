# The repo drawer

Click any card in [Mission Control](./mission-control) to slide out the repo drawer on **Work**. History and About load when you open those tabs, so a card click does not fetch pull requests or the README.

## Work

Everything for turning working changes into a commit, plus the stash and conflicts:

- **Unstaged / Staged lists** — per-file stage/unstage. Copy a relative path with the row's copy icon or right-click → **Copy path**.
- **Diff pane** — the selected file's staged or unstaged patch (hunk stage/unstage). **Copy diff** (header button or right-click the patch) puts the full unified patch on the clipboard; the header copy icon copies the path. Commit subject/body stay normal text fields (select + Ctrl+C).
- **Gen only / Gen & push** — side-by-side when [AI](./local-ai) is on: **Gen only** fills the commit composer from the staged/working diff (changelog-aware); **Gen & push** drafts, commits all, and pushes (single-repo, one-click from the drawer).
- **Changelog** — summarise recent history into release-style notes.
- **Open external diff** — launches the configured diff tool (`meld` by default) on the repo, or on the selected file.
- **Stash** — **Stash changes** saves uncommitted work (including untracked files). The list can Apply (keeps the entry) or Drop.
- **Conflicts** — when a merge or rebase left unmerged paths, a banner lists them. **Open in diff tool** uses the configured command (`meld` by default). **Open in IDE** uses the IDE launcher. The same conflict shows on the Mission Control card, with the first path in the subtitle. RepoHarbor does not embed a three-way merge editor.
- **Blame file** — one row per source line (author and short commit), capped so a large file stays light.

## History

A flat lane graph of the latest commits on HEAD (up to 80). Each lane is a vertical rail; the commit sits on a dot. The first parent stays on the same lane. A merge opens a side lane until that commit is drawn.

- Select a commit to list the files it touched. Select a file to blame it, line by line, as of that commit.

## About

The former Overview, PR, Notes, and Readme surfaces, stacked on one scroll:

- **Facts** — description, AI summary when present, branch, ahead/behind, host stars, release, and default-branch CI.
- **Branches** — every local branch; switch in a click.
- **Worktrees** — list, add, and remove linked git worktrees. Create one to work on a branch in parallel, or to drop an [agent](./launchers) into an isolated tree. Removing a worktree unlinks it; the folder stays on disk.
- **Dispatch agent** — start the configured agent in the repo, or on a fresh worktree.
- **Pull requests** — for GitHub repos, CI checks, reviews, mergeability, approve, and quick-merge (squash / rebase / merge, limited to methods the repository allows). See also the [Inbox](./inbox-feed-explore) for PRs across every repo at once.
- **Notes** — free-text notes per repo, autosaved in the local cache, plus a "what changed since I last looked" catch-up (and an AI resume when AI is on).
- **Readme** — the repo's rendered `README`.
