//! Interactive PTY session + ANSI/VT100 terminal state + safe command extraction.
//!
//! Zero UI / GPUI dependencies: this crate drives pseudo-terminal processes via
//! `portable-pty` and maintains the screen grid with `vt100`, emitting dirty /
//! exit signals over an `async_channel::Sender<TerminalEvent>` so the GPUI app
//! redraws without blocking or spinning idle CPU.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use async_channel::{Receiver, Sender};
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};

/// Maximum scrollback history lines kept per terminal tab.
const SCROLLBACK_LEN: usize = 2_000;

/// Events sent from a background PTY reader thread to the UI foreground loop.
#[derive(Debug, Clone)]
pub enum TerminalEvent {
    Output(String),
    Exited { id: String, code: i32 },
}

/// Terminal ANSI / truecolor color representation (GPUI-agnostic).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermColor {
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

impl From<vt100::Color> for TermColor {
    fn from(c: vt100::Color) -> Self {
        match c {
            vt100::Color::Default => Self::Default,
            vt100::Color::Idx(i) => Self::Indexed(i),
            vt100::Color::Rgb(r, g, b) => Self::Rgb(r, g, b),
        }
    }
}

/// A contiguous run of cells on one row sharing styling attributes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TermSpan {
    pub text: String,
    pub fg: TermColor,
    pub bg: TermColor,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
    /// Display-column start index (0-based) for cursor placement.
    pub start_col: u16,
    /// Number of columns occupied by this span.
    pub cols: u16,
}

/// Render-ready snapshot of the terminal viewport.
#[derive(Debug, Clone)]
pub struct TerminalSnapshot {
    pub rows: u16,
    pub cols: u16,
    pub lines: Vec<Vec<TermSpan>>,
    pub cursor_row: u16,
    pub cursor_col: u16,
    pub cursor_visible: bool,
    pub scrollback_offset: usize,
    pub max_scrollback: usize,
    pub alternate_screen: bool,
    pub exited: Option<i32>,
}

/// Live PTY-backed shell session. Dropping it closes the master side and sends
/// a termination signal to the child process so tabs never leave orphan shells.
pub struct TerminalSession {
    pub id: String,
    pub cwd: String,
    writer: Mutex<Box<dyn Write + Send>>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    parser: Arc<Mutex<vt100::Parser>>,
    exited: Arc<AtomicBool>,
    exit_code: Arc<Mutex<Option<i32>>>,
}

/// Resolve the shell binary (`override_shell`, else `$SHELL`, else platform default shell).
pub fn resolve_shell(override_shell: &str) -> String {
    let custom = override_shell.trim();
    if !custom.is_empty() {
        return custom.to_string();
    }
    if let Ok(env_shell) = std::env::var("SHELL") {
        let trimmed = env_shell.trim();
        if !trimmed.is_empty() && std::path::Path::new(trimmed).exists() {
            return trimmed.to_string();
        }
    }
    #[cfg(target_os = "windows")]
    {
        if which::which("pwsh").is_ok() {
            return "pwsh.exe".into();
        }
        if which::which("powershell").is_ok() {
            return "powershell.exe".into();
        }
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into())
    }
    #[cfg(not(target_os = "windows"))]
    {
        if cfg!(target_os = "macos") && std::path::Path::new("/bin/zsh").exists() {
            "/bin/zsh".into()
        } else if std::path::Path::new("/bin/bash").exists() {
            "/bin/bash".into()
        } else {
            "/bin/sh".into()
        }
    }
}

impl TerminalSession {
    /// Spawn a new interactive shell session inside `cwd`.
    pub fn spawn(
        id: String,
        cwd: &str,
        shell_override: &str,
        rows: u16,
        cols: u16,
        tx: Sender<TerminalEvent>,
    ) -> Result<Self, String> {
        let rows = rows.max(4);
        let cols = cols.max(20);
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("failed to allocate PTY: {e}"))?;

        let shell = resolve_shell(shell_override);
        let mut cmd = CommandBuilder::new(&shell);
        if std::path::Path::new(cwd).is_dir() {
            cmd.cwd(cwd);
        } else if let Some(home) = dirs::home_dir() {
            cmd.cwd(home);
        }
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");

        let mut child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("failed to spawn `{shell}`: {e}"))?;
        // Drop slave in parent immediately so master reader observes EOF cleanly when child exits.
        drop(pair.slave);

        let killer = child.clone_killer();
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("failed to clone PTY reader: {e}"))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("failed to take PTY writer: {e}"))?;

        let parser = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, SCROLLBACK_LEN)));
        let exited = Arc::new(AtomicBool::new(false));
        let exit_code = Arc::new(Mutex::new(None));

        let id_clone = id.clone();
        let parser_clone = Arc::clone(&parser);
        let exited_clone = Arc::clone(&exited);
        let exit_code_clone = Arc::clone(&exit_code);
        thread::Builder::new()
            .name(format!("pty-reader-{id}"))
            .spawn(move || {
                let mut reader = reader;
                let mut buf = [0u8; 8192];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if let Ok(mut p) = parser_clone.lock() {
                                p.process(&buf[..n]);
                            }
                            let _ = tx.try_send(TerminalEvent::Output(id_clone.clone()));
                        }
                    }
                }
                let code = child
                    .wait()
                    .map(|status| i32::try_from(status.exit_code()).unwrap_or(1))
                    .unwrap_or(1);
                if let Ok(mut guard) = exit_code_clone.lock() {
                    *guard = Some(code);
                }
                exited_clone.store(true, Ordering::Release);
                let _ = tx.send_blocking(TerminalEvent::Exited { id: id_clone, code });
            })
            .map_err(|e| format!("failed to start PTY reader thread: {e}"))?;

        Ok(Self {
            id,
            cwd: cwd.to_string(),
            writer: Mutex::new(writer),
            master: Mutex::new(pair.master),
            killer: Mutex::new(killer),
            parser,
            exited,
            exit_code,
        })
    }

    /// Write raw bytes (keystrokes, escape sequences, pasted text) into the PTY.
    pub fn write_bytes(&self, bytes: &[u8]) -> Result<(), String> {
        if self.exited.load(Ordering::Acquire) {
            return Err("terminal process has exited".into());
        }
        let mut w = self.writer.lock().map_err(|e| e.to_string())?;
        w.write_all(bytes).map_err(|e| e.to_string())?;
        w.flush().map_err(|e| e.to_string())
    }

    /// Write `command` into the shell; executes immediately (`\r`) when `execute` is true.
    pub fn run_command(&self, command: &str, execute: bool) -> Result<(), String> {
        let trimmed = command.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            return Ok(());
        }
        // Reset scrollback to live bottom when typing/running a command.
        self.set_scrollback(0);
        if execute {
            let line = format!("{trimmed}\r");
            self.write_bytes(line.as_bytes())
        } else {
            self.write_bytes(trimmed.as_bytes())
        }
    }

    /// Send SIGINT (`Ctrl+C`) to the foreground process group.
    pub fn send_interrupt(&self) -> Result<(), String> {
        self.write_bytes(&[0x03])
    }

    /// Clear scrollback / visible screen cleanly (`Ctrl+L` + reset scrollback).
    pub fn clear_screen(&self) -> Result<(), String> {
        self.set_scrollback(0);
        self.write_bytes(b"\x0c")
    }

    /// Scroll backward (`delta > 0`) or forward (`delta < 0`) in scrollback history.
    pub fn scroll_by(&self, delta: isize) {
        if let Ok(mut p) = self.parser.lock() {
            let current = p.screen().scrollback() as isize;
            let next = (current + delta).max(0) as usize;
            p.set_scrollback(next);
        }
    }

    /// Pin scrollback to `offset` lines above the live prompt (`0` = bottom).
    pub fn set_scrollback(&self, offset: usize) {
        if let Ok(mut p) = self.parser.lock() {
            p.set_scrollback(offset);
        }
    }

    /// Resize both the kernel PTY and the VT100 parser screen.
    pub fn resize(&self, rows: u16, cols: u16) {
        let rows = rows.max(4);
        let cols = cols.max(20);
        if let Ok(mut p) = self.parser.lock() {
            let (cur_r, cur_c) = p.screen().size();
            if cur_r == rows && cur_c == cols {
                return;
            }
            p.set_size(rows, cols);
        }
        if let Ok(m) = self.master.lock() {
            let _ = m.resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
    }

    /// True once the spawned shell/command has exited.
    pub fn has_exited(&self) -> bool {
        self.exited.load(Ordering::Acquire)
    }

    /// Build a compact, styled-span snapshot of the current viewport.
    pub fn snapshot(&self) -> TerminalSnapshot {
        let guard = self.parser.lock().unwrap_or_else(|p| p.into_inner());
        let exited = *self.exit_code.lock().unwrap_or_else(|p| p.into_inner());
        snapshot_screen(guard.screen(), exited)
    }

    /// Plain text contents of the current visible screen (for testing/copy).
    pub fn plain_text(&self) -> String {
        let guard = self.parser.lock().unwrap_or_else(|p| p.into_inner());
        guard.screen().contents()
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        if !self.exited.load(Ordering::Acquire) {
            if let Ok(mut k) = self.killer.lock() {
                let _ = k.kill();
            }
        }
    }
}

/// Read-only ANSI/VT100 terminal log stream backing the `Action Log` tab.
///
/// Uses the same `vt100::Parser` + [`TerminalSnapshot`] pipeline as a live PTY
/// without spawning an OS shell or accepting accidental keyboard interference.
pub struct TerminalLogStream {
    pub id: String,
    parser: Arc<Mutex<vt100::Parser>>,
    tx: Sender<TerminalEvent>,
}

impl TerminalLogStream {
    /// Allocate a new log buffer initialized at `(rows, cols)` with cursor hidden.
    pub fn new(id: impl Into<String>, rows: u16, cols: u16, tx: Sender<TerminalEvent>) -> Self {
        let rows = rows.max(4);
        let cols = cols.max(20);
        let mut parser = vt100::Parser::new(rows, cols, SCROLLBACK_LEN);
        // Hide cursor inside the read-only Action Log stream.
        parser.process(b"\x1b[?25l");
        Self {
            id: id.into(),
            parser: Arc::new(Mutex::new(parser)),
            tx,
        }
    }

    /// Normalize bare `\n` into `\r\n` and append ANSI-formatted lines to the log.
    pub fn append_ansi(&self, ansi_text: &str) {
        if ansi_text.is_empty() {
            return;
        }
        let mut normalized = String::with_capacity(ansi_text.len() + 16);
        let bytes = ansi_text.as_bytes();
        for (i, ch) in ansi_text.char_indices() {
            if ch == '\n' && (i == 0 || bytes[i - 1] != b'\r') {
                normalized.push_str("\r\n");
            } else {
                normalized.push(ch);
            }
        }
        if let Ok(mut p) = self.parser.lock() {
            p.process(normalized.as_bytes());
        }
        let _ = self.tx.try_send(TerminalEvent::Output(self.id.clone()));
    }

    /// Clear all scrollback and visible content cleanly.
    pub fn clear(&self) {
        if let Ok(mut p) = self.parser.lock() {
            let (rows, cols) = p.screen().size();
            let mut fresh = vt100::Parser::new(rows, cols, SCROLLBACK_LEN);
            fresh.process(b"\x1b[?25l");
            *p = fresh;
        }
        let _ = self.tx.try_send(TerminalEvent::Output(self.id.clone()));
    }

    /// Scroll backward (`delta > 0`) or forward (`delta < 0`) in log scrollback.
    pub fn scroll_by(&self, delta: isize) {
        if let Ok(mut p) = self.parser.lock() {
            let current = p.screen().scrollback() as isize;
            let next = (current + delta).max(0) as usize;
            p.set_scrollback(next);
        }
    }

    /// Pin scrollback to `offset` lines (`0` = live bottom).
    pub fn set_scrollback(&self, offset: usize) {
        if let Ok(mut p) = self.parser.lock() {
            p.set_scrollback(offset);
        }
    }

    /// Update screen dimensions to match the dock viewport.
    pub fn resize(&self, rows: u16, cols: u16) {
        let rows = rows.max(4);
        let cols = cols.max(20);
        if let Ok(mut p) = self.parser.lock() {
            let (cur_r, cur_c) = p.screen().size();
            if cur_r != rows || cur_c != cols {
                p.set_size(rows, cols);
            }
        }
    }

    /// Snapshot the current log screen for GPUI rendering.
    pub fn snapshot(&self) -> TerminalSnapshot {
        let guard = self.parser.lock().unwrap_or_else(|p| p.into_inner());
        snapshot_screen(guard.screen(), None)
    }

    /// Plain text contents of the current visible log screen.
    pub fn plain_text(&self) -> String {
        let guard = self.parser.lock().unwrap_or_else(|p| p.into_inner());
        guard.screen().contents()
    }
}

/// Format a cyan bold CLI command header line for [`TerminalLogStream`].
pub fn format_log_command(hhmmss: &str, command: &str, scope: Option<&str>) -> String {
    match scope.map(str::trim).filter(|s| !s.is_empty()) {
        Some(sc) => {
            format!("\x1b[90m[{hhmmss}]\x1b[0m \x1b[1;36m$\x1b[0m \x1b[1m{command}\x1b[0m  \x1b[90m({sc})\x1b[0m\n")
        }
        None => format!("\x1b[90m[{hhmmss}]\x1b[0m \x1b[1;36m$\x1b[0m \x1b[1m{command}\x1b[0m\n"),
    }
}

/// Format a status/outcome line (`ok`, `skip`/`warn`, `err`, `info`) with optional detail.
pub fn format_log_entry(
    hhmmss: &str,
    tag: &str,
    sgr_color: &str,
    subject: &str,
    detail: Option<&str>,
) -> String {
    match detail
        .map(str::trim)
        .filter(|d| !d.is_empty())
    {
        Some(d) => format!(
            "\x1b[90m[{hhmmss}]\x1b[0m \x1b[{sgr_color}m{tag}\x1b[0m {subject} \x1b[90m— {d}\x1b[0m\n"
        ),
        None => format!("\x1b[90m[{hhmmss}]\x1b[0m \x1b[{sgr_color}m{tag}\x1b[0m {subject}\n"),
    }
}

/// Allocate an unbounded channel for PTY dirty/exit notifications.
pub fn event_channel() -> (Sender<TerminalEvent>, Receiver<TerminalEvent>) {
    async_channel::unbounded()
}

fn snapshot_screen(screen: &vt100::Screen, exited: Option<i32>) -> TerminalSnapshot {
    let (rows, cols) = screen.size();
    let (cursor_row, cursor_col) = screen.cursor_position();
    let isolate_cursor = !screen.hide_cursor() && screen.scrollback() == 0 && exited.is_none();
    let mut lines = Vec::with_capacity(rows as usize);

    for r in 0..rows {
        let mut row_spans: Vec<TermSpan> = Vec::new();
        // Find last non-blank / styled column so trailing empty cells aren't emitted needlessly.
        let mut last_used_col: Option<u16> = None;
        for c in (0..cols).rev() {
            if let Some(cell) = screen.cell(r, c) {
                if cell.has_contents() || cell.bgcolor() != vt100::Color::Default || cell.inverse()
                {
                    last_used_col = Some(c);
                    break;
                }
            }
        }
        if let Some(end_col) = last_used_col {
            for c in 0..=end_col {
                let Some(cell) = screen.cell(r, c) else {
                    continue;
                };
                if cell.is_wide_continuation() {
                    continue;
                }
                let raw = if cell.has_contents() {
                    cell.contents()
                } else {
                    " ".to_string()
                };
                let ch_str = if raw.is_empty() { " " } else { raw.as_str() };
                let fg = TermColor::from(cell.fgcolor());
                let bg = TermColor::from(cell.bgcolor());
                let bold = cell.bold();
                let italic = cell.italic();
                let underline = cell.underline();
                let inverse = cell.inverse();
                let width = if cell.is_wide() { 2 } else { 1 };

                let on_cursor_row = isolate_cursor && r == cursor_row;
                let is_cursor_cell =
                    on_cursor_row && (c == cursor_col || (width == 2 && c + 1 == cursor_col));
                let prev_is_cursor = on_cursor_row
                    && row_spans.last().is_some_and(|prev| {
                        cursor_col >= prev.start_col && cursor_col < prev.start_col + prev.cols
                    });

                let merged = if let Some(prev) = row_spans.last_mut() {
                    if !is_cursor_cell
                        && !prev_is_cursor
                        && prev.fg == fg
                        && prev.bg == bg
                        && prev.bold == bold
                        && prev.italic == italic
                        && prev.underline == underline
                        && prev.inverse == inverse
                        && prev.start_col + prev.cols == c
                    {
                        prev.text.push_str(ch_str);
                        prev.cols += width;
                        true
                    } else {
                        false
                    }
                } else {
                    false
                };
                if !merged {
                    row_spans.push(TermSpan {
                        text: ch_str.to_string(),
                        fg,
                        bg,
                        bold,
                        italic,
                        underline,
                        inverse,
                        start_col: c,
                        cols: width,
                    });
                }
            }
        }
        lines.push(row_spans);
    }

    TerminalSnapshot {
        rows,
        cols,
        lines,
        cursor_row,
        cursor_col,
        cursor_visible: !screen.hide_cursor(),
        scrollback_offset: screen.scrollback(),
        max_scrollback: SCROLLBACK_LEN,
        alternate_screen: screen.alternate_screen(),
        exited,
    }
}

// ── Command extraction & safety classification ───────────────────────────────

/// A shell or git command surfaced in READMEs, error notices, or logs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedCommand {
    pub command: String,
    pub label: String,
    pub dangerous: bool,
}

const SHELL_FENCE_LANGS: &[&str] = &["", "bash", "sh", "shell", "zsh", "console", "terminal"];
const KNOWN_CLI_HEADS: &[&str] = &[
    "git",
    "cargo",
    "rustc",
    "rustup",
    "npm",
    "pnpm",
    "yarn",
    "npx",
    "bun",
    "pip",
    "pip3",
    "uv",
    "python",
    "python3",
    "pytest",
    "odoo-bin",
    "docker",
    "docker-compose",
    "podman",
    "make",
    "cmake",
    "just",
    "gh",
    "glab",
    "ssh",
    "systemctl",
    "journalctl",
    "ls",
    "cat",
    "tail",
    "head",
    "rg",
    "grep",
    "find",
    "chmod",
    "kill",
    "pkill",
];

fn looks_like_cli_command(s: &str) -> bool {
    let trimmed = s.trim();
    if trimmed.is_empty() || trimmed.len() > 220 || trimmed.contains('\n') {
        return false;
    }
    let first = trimmed.split_whitespace().next().unwrap_or("");
    let bin = first.rsplit('/').next().unwrap_or(first);
    first.starts_with("./")
        || KNOWN_CLI_HEADS.contains(&bin)
        || (first == "sudo"
            && trimmed
                .split_whitespace()
                .nth(1)
                .is_some_and(|next| KNOWN_CLI_HEADS.contains(&next)))
}

/// Flag commands that delete files, force-overwrite git history, elevate root,
/// or pipe remote scripts directly into a shell. Pure for unit testing.
pub fn is_potentially_destructive(cmd: &str) -> bool {
    let lower = cmd.to_ascii_lowercase();
    let padded = format!(" {lower} ");

    // Privilege escalation or raw device writes.
    if padded.contains(" sudo ") || padded.contains(" mkfs") || padded.contains(" dd if=") {
        return true;
    }
    // Recursive / wildcard rm.
    if (padded.contains(" rm ") || lower.starts_with("rm "))
        && (lower.contains("-rf") || lower.contains("-fr") || lower.contains(" -r "))
    {
        return true;
    }
    // Forceful / history-destroying git operations.
    if lower.contains("git ")
        && (padded.contains(" --force")
            || padded.contains(" -f ")
            || padded.ends_with(" -f ")
            || lower.contains("push --force-with-lease")
            || lower.contains("reset --hard")
            || lower.contains("clean -fd")
            || lower.contains("clean -xfd")
            || (lower.contains("branch ") && cmd.split_whitespace().any(|tok| tok == "-D")))
    {
        return true;
    }
    // Piped remote execution (`curl ... | sh` / `wget ... | bash`).
    if (lower.contains("curl ") || lower.contains("wget "))
        && (lower.contains("| sh") || lower.contains("| bash") || lower.contains("| zsh"))
    {
        return true;
    }
    // Broad permission / killall mutations or fork bomb.
    lower.contains("chmod 777")
        || lower.contains("chmod -r 777")
        || lower.contains("chown -r")
        || lower.contains(":(){")
}

fn push_unique_command(out: &mut Vec<ExtractedCommand>, raw: &str, cap: usize) {
    if out.len() >= cap {
        return;
    }
    let cleaned = raw.trim();
    if cleaned.is_empty() {
        return;
    }
    if out.iter().any(|c| c.command == cleaned) {
        return;
    }
    let label = if cleaned.chars().count() > 54 {
        let short: String = cleaned.chars().take(53).collect();
        format!("{short}…")
    } else {
        cleaned.to_string()
    };
    out.push(ExtractedCommand {
        command: cleaned.to_string(),
        label,
        dangerous: is_potentially_destructive(cleaned),
    });
}

/// Extract runnable commands from fenced shell code blocks inside Markdown (`README.md`).
pub fn extract_commands_from_markdown(src: &str) -> Vec<ExtractedCommand> {
    let mut out = Vec::new();
    let mut in_fence = false;
    let mut fence_lang = String::new();
    let mut fence_char = '`';
    let mut pending = String::new();

    for line in src.lines() {
        let trimmed = line.trim();
        if !in_fence {
            if let Some(rest) = trimmed
                .strip_prefix("```")
                .map(|r| ('`', r))
                .or_else(|| trimmed.strip_prefix("~~~").map(|r| ('~', r)))
            {
                fence_char = rest.0;
                fence_lang = rest
                    .1
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                in_fence = true;
                pending.clear();
            }
            continue;
        }

        let is_close = (fence_char == '`' && trimmed.starts_with("```"))
            || (fence_char == '~' && trimmed.starts_with("~~~"));
        if is_close {
            if !pending.trim().is_empty() {
                push_unique_command(&mut out, pending.trim(), 24);
                pending.clear();
            }
            in_fence = false;
            continue;
        }

        if !SHELL_FENCE_LANGS.contains(&fence_lang.as_str()) {
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let unprompted = trimmed
            .strip_prefix("$ ")
            .or_else(|| trimmed.strip_prefix("> "))
            .unwrap_or(trimmed)
            .trim();

        // For untagged ``` blocks, only keep lines with `$ ` prompt or recognized CLI heads.
        if fence_lang.is_empty()
            && !trimmed.starts_with("$ ")
            && pending.is_empty()
            && !looks_like_cli_command(unprompted)
        {
            continue;
        }

        if let Some(cont) = unprompted.strip_suffix('\\') {
            pending.push_str(cont.trim_end());
            pending.push(' ');
        } else {
            pending.push_str(unprompted);
            push_unique_command(&mut out, &pending, 24);
            pending.clear();
        }
    }

    out
}

/// Extract runnable commands from backticked spans or contextual git/error messages.
pub fn extract_commands_from_text(text: &str) -> Vec<ExtractedCommand> {
    let mut out = Vec::new();

    // 1. Backticked inline commands (`git ...`, `cargo ...`, etc.).
    let mut rest = text;
    while let Some(start) = rest.find('`') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('`') else {
            break;
        };
        let snippet = after[..end].trim();
        let candidate = snippet.strip_prefix("$ ").unwrap_or(snippet).trim();
        if looks_like_cli_command(candidate) {
            push_unique_command(&mut out, candidate, 8);
        }
        rest = &after[end + 1..];
    }

    // 2. Actionable suggestions from known RepoHarbor / git error messages.
    let lower = text.to_ascii_lowercase();
    let is_rule_block = lower.contains("repository rule")
        || lower.contains("gh013")
        || lower.contains("protected branch")
        || lower.contains("pull request");
    if is_rule_block {
        push_unique_command(
            &mut out,
            "git checkout -b fix/update && git push -u origin HEAD",
            8,
        );
        push_unique_command(&mut out, "git status", 8);
    } else if lower.contains("non-fast-forward") || lower.contains("pull or rebase first") {
        push_unique_command(&mut out, "git pull --rebase && git push", 8);
    }
    if lower.contains("conflict") || lower.contains("unmerged") {
        push_unique_command(&mut out, "git status", 8);
    }
    if lower.contains("rebase") && (lower.contains("abort") || lower.contains("failed")) {
        push_unique_command(&mut out, "git status", 8);
    }
    if lower.contains("detached") {
        push_unique_command(&mut out, "git branch -a", 8);
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_markdown_filters_shell_blocks_and_joins_continuations() {
        let md = r#"
# Project

```rust
fn main() { println!("skip me"); }
```

```bash
# Install deps
$ cargo build --workspace
cargo clippy --workspace \
  --all-targets
```

```
git status
Not a command line here
```
"#;
        let cmds: Vec<String> = extract_commands_from_markdown(md)
            .into_iter()
            .map(|c| c.command)
            .collect();
        assert_eq!(
            cmds,
            vec![
                "cargo build --workspace",
                "cargo clippy --workspace --all-targets",
                "git status",
            ]
        );
    }

    #[test]
    fn extract_text_finds_backticked_commands_and_error_hints() {
        let err = "Push failed: push rejected (non-fast-forward) — pull or rebase first, or run `git log -n 5`";
        let cmds: Vec<String> = extract_commands_from_text(err)
            .into_iter()
            .map(|c| c.command)
            .collect();
        assert!(cmds.contains(&"git log -n 5".to_string()));
        assert!(cmds.contains(&"git pull --rebase && git push".to_string()));

        let rule_cmds = extract_commands_from_text(
            "push blocked (error: GH013: Repository rule violations found for refs/heads/repo-clean.)",
        );
        assert!(rule_cmds
            .iter()
            .any(|c| c.command.contains("git checkout -b")));
        assert!(!rule_cmds
            .iter()
            .any(|c| c.command.contains("git pull --rebase")));
    }

    #[test]
    fn destructive_classifier_flags_dangerous_patterns_only() {
        assert!(is_potentially_destructive("rm -rf /tmp/foo"));
        assert!(is_potentially_destructive("git push --force origin main"));
        assert!(is_potentially_destructive("git reset --hard HEAD~1"));
        assert!(is_potentially_destructive("sudo systemctl restart odoo"));
        assert!(is_potentially_destructive(
            "curl -fsSL https://example.com/install.sh | sh"
        ));

        assert!(!is_potentially_destructive("git status"));
        assert!(!is_potentially_destructive("git pull --rebase && git push"));
        assert!(!is_potentially_destructive("cargo test --workspace"));
        assert!(!is_potentially_destructive("ls -la --color=always"));
    }

    #[test]
    fn live_pty_executes_command_and_updates_vt100_screen() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = event_channel();
        let session = TerminalSession::spawn(
            "t-test".into(),
            &dir.path().to_string_lossy(),
            "/bin/sh",
            24,
            80,
            tx,
        )
        .unwrap();

        session.run_command("echo HARBOR_PTY_OK", true).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut saw = false;
        while std::time::Instant::now() < deadline {
            let _ = rx.recv_blocking();
            if session.plain_text().contains("HARBOR_PTY_OK") {
                saw = true;
                break;
            }
        }
        assert!(saw, "screen output: {:?}", session.plain_text());

        session.resize(30, 100);
        let snap = session.snapshot();
        assert_eq!((snap.rows, snap.cols), (30, 100));

        session.run_command("exit 0", true).unwrap();
    }

    #[test]
    fn action_log_stream_appends_normalizes_and_clears() {
        let (tx, rx) = event_channel();
        let log = TerminalLogStream::new("term-action-log", 10, 60, tx);
        log.append_ansi(&format_log_command("12:00:01", "git pull", Some("3 repos")));
        log.append_ansi(&format_log_entry(
            "12:00:02",
            "✓",
            "32",
            "RepoHarbor",
            Some("up to date"),
        ));

        let snap = log.snapshot();
        assert!(!snap.cursor_visible);
        let text = log.plain_text();
        assert!(text.contains("[12:00:01] $ git pull  (3 repos)"));
        assert!(text.contains("[12:00:02] ✓ RepoHarbor — up to date"));
        assert!(rx.try_recv().is_ok());

        log.clear();
        assert!(log.plain_text().trim().is_empty());
    }

    #[test]
    fn snapshot_isolates_midline_cursor_cell_into_its_own_span() {
        let mut parser = vt100::Parser::new(6, 40, 0);
        parser.process(b"git status\x1b[1;5H");
        let snap = snapshot_screen(parser.screen(), None);
        assert_eq!((snap.cursor_row, snap.cursor_col), (0, 4));
        let row0 = &snap.lines[0];
        let cursor_span = row0
            .iter()
            .find(|s| s.start_col == 4)
            .expect("isolated cursor span at col 4");
        assert_eq!(cursor_span.cols, 1);
        assert_eq!(cursor_span.text, "s");
    }
}
