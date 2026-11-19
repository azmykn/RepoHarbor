# Embedded Terminal & Action Log

RepoHarbor includes a built-in, GPU-rendered interactive PTY terminal dock (`portable-pty` + `vt100`) and a live **Action Log** stream directly at the bottom of the workspace.

## Opening & Navigating the Dock

- Press <kbd>Ctrl+\`</kbd> (or click **Terminal** in the top-right header) to toggle the bottom dock.
- Drag the top splitter bar to resize the dock height (persisted in `config.toml` as `terminalHeight`), or click **Expand** / **Restore**.
- Click **`+ New`** (or press <kbd>Ctrl+Shift+T</kbd>) to spawn a new shell tab in the currently selected repository or drawer target; close tabs with **`✕`** or <kbd>Ctrl+Shift+W</kbd>.

## Interactive PTY Features

- **Full ANSI 16/256/Truecolor & Scrollback**: Scroll back with the mouse wheel and snap back via the floating **Jump to bottom** pill.
- **Zero-Drift Cursor Alignment**: Cursor bounds are measured directly through GPUI's `TextSystem` shaper on the active monospace font.
- **Clipboard & Interrupts**: Use **Copy** (<kbd>Ctrl+Shift+C</kbd>), **Paste** (<kbd>Ctrl+Shift+V</kbd>), **Clear**, and **Ctrl+C** (`SIGINT`) buttons in the dock bar.
- **Quick Command Bar (`$`)**: Type any command and click **Run** (executes immediately) or **Paste** (stages on the shell prompt). When viewing **Action Log**, running a command opens or focuses an interactive shell tab automatically.

## Live `Action Log` Tab

The pinned **`Action Log`** tab streams every toolbar, drawer, and fleet operation in real time without interfering with your interactive shell tabs:

- Prints `[HH:MM:SS] $ <cli command> (N repo(s))` when an operation starts.
- Streams `[done/total] ✓/↷/✗ repo — outcome` live as worker threads complete.
- Covers all 14 fleet operations (`Fetch`, `Pull`, `Push`, `Submodules`, `StageAll`, `CommitAll`, `EmptyCommit`, `FastCommit`, `GenerateMessageOnly`, `GenerateCommitAndPush`, `Prune`, `ResetHard`, `DiscardChanges`, `MergeBranch`) plus `Refresh`, `Pull behind`, `Enrich hosts`, and `Summarize`.
- Toggle **Auto-log: On/Off** in the dock bar (persisted as `terminalAutoShowLog`) to control whether operations automatically reveal the `Action Log` tab.

## One-Click Runnable Commands & Safety Gate

RepoHarbor automatically extracts runnable shell commands from:

1. Repository **README** code blocks inside the Repo Drawer (**About** tab).
2. **Error / Notice Modals** (for example, `git checkout -b fix/update && git push -u origin HEAD` on `GH013` branch-protection rejections).
3. **Activity Log** detail cards (**Log** view).

Click **▶ Run** to execute immediately or **Paste** to inspect on the prompt first. Commands matching destructive patterns (`rm -rf`, `sudo`, `--force`, `curl | sh`, `git reset --hard`, `git push -f`) pause behind an inline **Press Enter / Dismiss** confirmation strip when `terminalConfirmDestructive` is enabled.
