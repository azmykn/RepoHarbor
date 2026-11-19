# Launchers

Every repo card is a launchpad. RepoHarbor has four launch actions, each with its own colour identity, and a one-click preset switcher in Settings so you never have to hand-write a command.

## The four actions

| Action | What it does |
|---|---|
| **Open in IDE** | Opens the repo in your configured editor. |
| **Agent / Terminal** | Opens the built-in PTY terminal dock in the repo by default (or drops into your external terminal / coding agent when toggled off in Settings). Right-click context menus always offer both **Open built-in terminal** and **Open external terminal**. |
| **Folder** | Reveals the repo's directory in your file manager. |
| **Open on host** | Opens the repo's GitHub/GitLab page in your browser (when it has a remote). |

## Embedded terminal dock

Press ``Ctrl+` `` (or click **Terminal** in the top bar) to toggle the bottom multi-tab PTY terminal dock:

- Run shell commands directly inside the active tab or from the quick `$` bar (`Run` / `Paste`).
- Monitor **Action Log**: triggering toolbar, fleet, or git actions (**Refresh**, **Pull**, **Fetch**, **Push**, **Submodules**, **Gen & push**, **Prune**, **Reset**, etc.) streams timestamped command lines and live per-repo results into the dedicated **Action Log** tab (`Auto-log: On/Off`).
- Commands surfaced in **README** blocks, **Notice** details, and **Activity Log** rows expose `▶ Run` and `Paste` chips that jump straight into a tab for that repository.
- Destructive patterns (`rm -rf`, `sudo`, `--force`, `reset --hard`, `clean -fd`, piped `curl | sh`, `git branch -D`) are staged unexecuted on the prompt with a confirmation bar unless disabled in **Settings → Launchers**.

The IDE and Agent buttons show the configured tool's logo and name (e.g. *VS Code*, *Claude Code*) and stay neutral so the brand reads cleanly; Folder and Open-on-host carry their own accent colours.

## Preset switcher

In **Settings → Launchers**, pick your editor and agent from preset chips with real brand logos — no command syntax required.

- **Open in IDE** — VS Code, VSCodium, Cursor, Windsurf, Zed, Sublime Text, Fleet, Lapce, the JetBrains family (IntelliJ, WebStorm, PyCharm, GoLand, CLion, Rider, RustRover, PhpStorm), Emacs, GNOME Builder, Kate, Geany.
- **Terminal agent** — two axes: a **terminal emulator** (Kitty, Alacritty, Ghostty, WezTerm, Foot, Konsole, GNOME Terminal, xterm) and an **agent CLI** (Claude Code, Codex, Gemini CLI, Aider, OpenCode, Cursor Agent, Crush, Qwen Code). Picking a terminal preserves your agent and vice-versa.

The active preset highlights in the action's colour. Detection round-trips, so the chips reflect whatever the saved command resolves to.

## Custom commands

Prefer to hand-roll it? The raw command field stays available under each preset row. Use `{path}` where the repo path should be substituted, e.g.:

```text
code {path}
kitty --working-directory {path} -e claude
```

Any command that doesn't match a preset simply shows no highlighted chip and falls back to a generic glyph on the cards.
