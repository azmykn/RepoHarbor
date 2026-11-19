//! Embedded interactive terminal dock rendered directly in GPUI.
//!
//! Binds background `repoharbor_core::terminal::TerminalSession` PTYs into a
//! flat, resizable bottom dock supporting multiple repository tabs, ANSI 16/256/
//! truecolor spans, interactive key forwarding, quick-command execution, and
//! safety confirmation for destructive commands.

use std::sync::Arc;

use gpui::{
    Context, Entity, FocusHandle, FontStyle, FontWeight, Hsla, InteractiveElement, IntoElement,
    KeyDownEvent, MouseButton, ParentElement, Pixels, ScrollDelta, SharedString,
    StatefulInteractiveElement, Styled, StyledText, Subscription, TextRun, UnderlineStyle, Window,
    div, px, rgb,
};
use gpui_component::input::{Input, InputState};
use repoharbor_core::terminal::{
    TermColor, TermSpan, TerminalEvent, TerminalLogStream, TerminalSession, TerminalSnapshot,
    event_channel,
};

use crate::icon::lucide;
use crate::shell::RepoHarborApp;
use crate::theme::Theme;

/// Fallback monospace cell width before measuring from `Window::text_system` (`13px`).
const CELL_W: f32 = 7.8;
const LINE_H: f32 = 18.0;
pub const DOCK_MIN_H: f32 = 160.0;
pub const DOCK_MAX_H: f32 = 640.0;
const DOCK_CHROME_H: f32 = 82.0;

/// Backing engine for one terminal dock tab: interactive OS PTY or Action Log stream.
pub enum TerminalBackend {
    Pty(Arc<TerminalSession>),
    Log(Arc<TerminalLogStream>),
}

impl TerminalBackend {
    pub fn is_log(&self) -> bool {
        matches!(self, Self::Log(_))
    }

    pub fn as_pty(&self) -> Option<&Arc<TerminalSession>> {
        match self {
            Self::Pty(s) => Some(s),
            Self::Log(_) => None,
        }
    }

    pub fn as_log(&self) -> Option<&Arc<TerminalLogStream>> {
        match self {
            Self::Log(s) => Some(s),
            Self::Pty(_) => None,
        }
    }

    pub fn snapshot(&self) -> TerminalSnapshot {
        match self {
            Self::Pty(s) => s.snapshot(),
            Self::Log(s) => s.snapshot(),
        }
    }

    pub fn plain_text(&self) -> String {
        match self {
            Self::Pty(s) => s.plain_text(),
            Self::Log(s) => s.plain_text(),
        }
    }

    pub fn scroll_by(&self, delta: isize) {
        match self {
            Self::Pty(s) => s.scroll_by(delta),
            Self::Log(s) => s.scroll_by(delta),
        }
    }

    pub fn set_scrollback(&self, offset: usize) {
        match self {
            Self::Pty(s) => s.set_scrollback(offset),
            Self::Log(s) => s.set_scrollback(offset),
        }
    }

    pub fn resize(&self, rows: u16, cols: u16) {
        match self {
            Self::Pty(s) => s.resize(rows, cols),
            Self::Log(s) => s.resize(rows, cols),
        }
    }
}

/// One terminal dock tab backed by either a live PTY or the Action Log stream.
pub struct TerminalTab {
    pub id: String,
    pub title: SharedString,
    pub cwd: SharedString,
    pub backend: TerminalBackend,
    pub focus: FocusHandle,
    pub snapshot: TerminalSnapshot,
    pub exited: Option<i32>,
}

/// Command awaiting user confirmation before running inside the terminal.
#[derive(Clone, Debug)]
pub struct PendingCommand {
    pub cwd: SharedString,
    pub title: SharedString,
    pub command: SharedString,
}

/// Dock-wide UI + session state stored directly inside [`RepoHarborApp`].
pub struct TerminalDockState {
    pub open: bool,
    pub height: f32,
    pub maximized: bool,
    pub dragging: bool,
    pub tabs: Vec<TerminalTab>,
    pub active_tab: Option<String>,
    pub cmd_input: Option<Entity<InputState>>,
    pub _cmd_sub: Option<Subscription>,
    pub pending_confirm: Option<PendingCommand>,
    pub tx: async_channel::Sender<TerminalEvent>,
    pub rx: async_channel::Receiver<TerminalEvent>,
    pub draining: bool,
    pub pending_focus: bool,
    pub next_seq: u64,
    pub last_dims: (u16, u16),
}

impl TerminalDockState {
    pub fn new(height: f32) -> Self {
        let (tx, rx) = event_channel();
        Self {
            open: false,
            height: height.clamp(DOCK_MIN_H, DOCK_MAX_H),
            maximized: false,
            dragging: false,
            tabs: Vec::new(),
            active_tab: None,
            cmd_input: None,
            _cmd_sub: None,
            pending_confirm: None,
            tx,
            rx,
            draining: false,
            pending_focus: false,
            next_seq: 1,
            last_dims: (16, 180),
        }
    }

    pub fn active(&self) -> Option<&TerminalTab> {
        let id = self.active_tab.as_deref()?;
        self.tabs.iter().find(|t| t.id == id)
    }

    pub fn active_mut(&mut self) -> Option<&mut TerminalTab> {
        let id = self.active_tab.clone()?;
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    pub fn next_id(&mut self) -> String {
        let n = self.next_seq;
        self.next_seq += 1;
        format!("term-{n}")
    }
}

/// Map a GPUI `KeyDownEvent` into VT100/xterm raw bytes for the active PTY.
/// Returns `None` for global/terminal shell shortcuts (`Ctrl+``, `Ctrl/Cmd+K`,
/// `Ctrl+Shift+C/V`) so their registered GPUI actions fire instead.
pub fn encode_keystroke(ev: &KeyDownEvent) -> Option<Vec<u8>> {
    let m = ev.keystroke.modifiers;
    let key = ev.keystroke.key.as_str();

    // Let global/panel shortcuts pass through to their GPUI action bindings.
    if (m.control || m.platform) && matches!(key, "`" | "k") {
        return None;
    }
    if m.control && m.shift && matches!(key, "c" | "v") {
        return None;
    }

    // Standard Ctrl+<letter> control sequences (`0x01..=0x1a`).
    if m.control && !m.alt && !m.platform && key.len() == 1 {
        let b = key.as_bytes()[0].to_ascii_lowercase();
        if b.is_ascii_lowercase() {
            return Some(vec![b - b'a' + 1]);
        }
        return match b {
            b'[' => Some(vec![0x1b]),
            b'\\' => Some(vec![0x1c]),
            b']' => Some(vec![0x1d]),
            b' ' => Some(vec![0x00]),
            _ => None,
        };
    }

    // Named non-printable keys + arrow modifier codes (`\x1b[1;<mod>...`).
    let modifier_param = match (m.shift, m.alt, m.control) {
        (false, false, false) => None,
        (true, false, false) => Some(2),
        (false, true, false) => Some(3),
        (true, true, false) => Some(4),
        (false, false, true) => Some(5),
        (true, false, true) => Some(6),
        (false, true, true) => Some(7),
        (true, true, true) => Some(8),
    };

    match key {
        "enter" => return Some(b"\r".to_vec()),
        "backspace" => {
            return Some(if m.alt { vec![0x1b, 0x7f] } else { vec![0x7f] });
        }
        "tab" => {
            return Some(if m.shift {
                b"\x1b[Z".to_vec()
            } else {
                b"\t".to_vec()
            });
        }
        "escape" => return Some(vec![0x1b]),
        "space" => {
            return Some(if m.alt {
                b"\x1b ".to_vec()
            } else {
                b" ".to_vec()
            });
        }
        "up" | "down" | "right" | "left" | "home" | "end" => {
            let suffix = match key {
                "up" => 'A',
                "down" => 'B',
                "right" => 'C',
                "left" => 'D',
                "home" => 'H',
                "end" => 'F',
                _ => unreachable!(),
            };
            let seq = match modifier_param {
                Some(p) => format!("\x1b[1;{p}{suffix}"),
                None => format!("\x1b[{suffix}"),
            };
            return Some(seq.into_bytes());
        }
        "pageup" => return Some(b"\x1b[5~".to_vec()),
        "pagedown" => return Some(b"\x1b[6~".to_vec()),
        "insert" => return Some(b"\x1b[2~".to_vec()),
        "delete" => return Some(b"\x1b[3~".to_vec()),
        "f1" => return Some(b"\x1bOP".to_vec()),
        "f2" => return Some(b"\x1bOQ".to_vec()),
        "f3" => return Some(b"\x1bOR".to_vec()),
        "f4" => return Some(b"\x1bOS".to_vec()),
        "f5" => return Some(b"\x1b[15~".to_vec()),
        "f6" => return Some(b"\x1b[17~".to_vec()),
        "f7" => return Some(b"\x1b[18~".to_vec()),
        "f8" => return Some(b"\x1b[19~".to_vec()),
        "f9" => return Some(b"\x1b[20~".to_vec()),
        "f10" => return Some(b"\x1b[21~".to_vec()),
        "f11" => return Some(b"\x1b[23~".to_vec()),
        "f12" => return Some(b"\x1b[24~".to_vec()),
        _ => {}
    }

    // Regular UTF-8 text from key_char (preserves shifted punctuation / IME).
    if let Some(ch) = ev.keystroke.key_char.as_deref().filter(|s| !s.is_empty()) {
        let mut out = Vec::with_capacity(ch.len() + 1);
        if m.alt {
            out.push(0x1b);
        }
        out.extend_from_slice(ch.as_bytes());
        return Some(out);
    }

    if !m.control && !m.platform && key.chars().count() == 1 {
        let mut out = Vec::with_capacity(key.len() + 1);
        if m.alt {
            out.push(0x1b);
        }
        out.extend_from_slice(key.as_bytes());
        return Some(out);
    }

    None
}

/// Resolve an ANSI 16/256/truecolor terminal color onto RepoHarbor's palette.
pub fn resolve_term_color(color: TermColor, is_fg: bool, t: &Theme) -> u32 {
    match color {
        TermColor::Default => {
            if is_fg {
                t.fg1
            } else {
                t.page
            }
        }
        TermColor::Rgb(r, g, b) => (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b),
        TermColor::Indexed(idx) => indexed_color(idx, t),
    }
}

fn indexed_color(idx: u8, t: &Theme) -> u32 {
    const ANSI16: [u32; 16] = [
        0x161b22, // 0 black
        0xf85149, // 1 red
        0x3fb950, // 2 green
        0xd29922, // 3 yellow
        0x58a6ff, // 4 blue
        0xbc8cff, // 5 magenta
        0x39c5cf, // 6 cyan
        0xc9d1d9, // 7 white
        0x6e7681, // 8 bright black
        0xff7b72, // 9 bright red
        0x56d364, // 10 bright green
        0xe3b341, // 11 bright yellow
        0x79c0ff, // 12 bright blue
        0xd2a8ff, // 13 bright magenta
        0x56d4dd, // 14 bright cyan
        0xf0f6fc, // 15 bright white
    ];
    match idx {
        0..=15 => {
            if idx == 6 {
                t.primary
            } else {
                ANSI16[idx as usize]
            }
        }
        16..=231 => {
            let n = idx - 16;
            let r = n / 36;
            let g = (n % 36) / 6;
            let b = n % 6;
            let lvl = |c: u8| -> u32 { if c == 0 { 0 } else { u32::from(c) * 40 + 55 } };
            (lvl(r) << 16) | (lvl(g) << 8) | lvl(b)
        }
        232..=255 => {
            let v = u32::from(idx - 232) * 10 + 8;
            (v << 16) | (v << 8) | v
        }
    }
}

/// Measure the desktop monospace glyph width (`13px`) directly from GPUI's text system.
pub fn measure_cell_width(window: &Window) -> f32 {
    let mut font = gpui::font("monospace");
    font.weight = FontWeight::NORMAL;
    let run = TextRun {
        len: 20,
        font,
        color: gpui::white(),
        ..Default::default()
    };
    let shaped = window.text_system().shape_line(
        SharedString::from("MMMMMMMMMMMMMMMMMMMM"),
        px(13.),
        &[run],
        None,
    );
    let w = f32::from(shaped.width) / 20.0;
    if w > 3.0 && w < 24.0 { w } else { CELL_W }
}

/// Compute target `(rows, cols)` for the terminal body area using a measured glyph width.
pub fn grid_dimensions_with(body_width_px: f32, body_height_px: f32, cell_w: f32) -> (u16, u16) {
    let w = if cell_w > 3.0 { cell_w } else { CELL_W };
    let cols = ((body_width_px - 24.0) / w).floor().clamp(30., 280.) as u16;
    let rows = ((body_height_px - 12.0) / LINE_H).floor().clamp(4., 80.) as u16;
    (rows, cols)
}

/// Render the bottom interactive terminal dock (`None` height overhead when closed).
pub fn render_dock(
    dock: &TerminalDockState,
    auto_show_log: bool,
    sidebar_w: f32,
    t: &Theme,
    app: &Entity<RepoHarborApp>,
    window: &Window,
    cx: &Context<RepoHarborApp>,
) -> impl IntoElement {
    let cell_w = measure_cell_width(window);
    let viewport_w = f32::from(window.viewport_size().width);
    let dock_h = if dock.maximized {
        DOCK_MAX_H
    } else {
        dock.height.clamp(DOCK_MIN_H, DOCK_MAX_H)
    };
    let body_w = (viewport_w - sidebar_w).max(320.0);
    let body_h = (dock_h - DOCK_CHROME_H).max(80.0);
    let (target_rows, target_cols) = grid_dimensions_with(body_w, body_h, cell_w);
    for tab in &dock.tabs {
        tab.backend.resize(target_rows, target_cols);
    }

    // 1. Top vertical drag-resize splitter.
    let app_drag = app.clone();
    let splitter = div()
        .id("term-splitter")
        .h(px(5.))
        .w_full()
        .cursor_ns_resize()
        .bg(rgb(t.border))
        .hover(|s| s.bg(rgb(t.primary)))
        .on_mouse_down(MouseButton::Left, move |_ev, _win, cx| {
            app_drag.update(cx, |this, cx| {
                this.terminal.dragging = true;
                cx.notify();
            });
        });

    // 2. Tab strip + dock actions bar.
    let mut tabs_row = div()
        .id("term-tabs")
        .flex()
        .flex_row()
        .items_center()
        .flex_1()
        .min_w(px(0.))
        .gap(px(4.))
        .overflow_x_scroll();
    for tab in &dock.tabs {
        let is_active = dock.active_tab.as_deref() == Some(tab.id.as_str());
        let is_log = tab.backend.is_log();
        let status_dot_color = if is_log {
            t.primary
        } else if tab.exited.is_some() {
            t.fg3
        } else {
            t.clean
        };
        let app_sel = app.clone();
        let app_close = app.clone();
        let tab_id_sel = tab.id.clone();
        let tab_id_close = tab.id.clone();
        let leading = if is_log {
            lucide("scroll-text", 12., status_dot_color).into_any_element()
        } else {
            div()
                .w(px(6.))
                .h(px(6.))
                .rounded(px(3.))
                .bg(rgb(status_dot_color))
                .into_any_element()
        };
        let mut pill = div()
            .id(SharedString::from(format!("term-tab-{}", tab.id)))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .px(px(8.))
            .py(px(3.))
            .rounded(px(t.r_xs))
            .bg(rgb(if is_active {
                t.accent_wash
            } else {
                t.button_bg
            }))
            .border_1()
            .border_color(rgb(if is_active { t.primary } else { t.border }))
            .cursor_pointer()
            .text_size(px(t.text_data_sm))
            .text_color(rgb(if is_active { t.fg0 } else { t.fg2 }))
            .on_click(move |_ev, window, cx| {
                let id = tab_id_sel.clone();
                app_sel.update(cx, |this, cx| this.select_terminal_tab(&id, window, cx));
            })
            .child(leading)
            .child(SharedString::from(match tab.exited {
                Some(c) => format!("{} (exit {c})", tab.title),
                None => tab.title.to_string(),
            }));
        if !is_log {
            pill = pill.child(
                div()
                    .id(SharedString::from(format!("term-close-{}", tab.id)))
                    .px(px(3.))
                    .rounded(px(t.r_xs))
                    .text_color(rgb(t.fg3))
                    .hover(|s| s.text_color(rgb(t.behind)))
                    .child("✕")
                    .on_click(move |_ev, _window, cx| {
                        cx.stop_propagation();
                        let id = tab_id_close.clone();
                        app_close.update(cx, |this, cx| this.close_terminal_tab(&id, cx));
                    }),
            );
        }
        tabs_row = tabs_row.child(pill);
    }

    let app_new = app.clone();
    tabs_row = tabs_row.child(term_btn("term-new-tab", "+ New", t, move |win, cx| {
        app_new.update(cx, |this, cx| this.open_default_terminal_tab(win, cx));
    }));

    let active_is_log = dock.active().is_some_and(|tab| tab.backend.is_log());
    let controls = {
        let (app_auto, app_sig, app_clr, app_cp, app_ps, app_max, app_hide) = (
            app.clone(),
            app.clone(),
            app.clone(),
            app.clone(),
            app.clone(),
            app.clone(),
            app.clone(),
        );
        let mut bar = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(3.))
            .flex_shrink_0()
            .child(term_toggle_btn(
                "term-auto-log",
                if auto_show_log {
                    "Auto-log: On"
                } else {
                    "Auto-log: Off"
                },
                auto_show_log,
                t,
                move |_, cx| {
                    app_auto.update(cx, |this, cx| this.toggle_terminal_auto_log(cx));
                },
            ));
        if !active_is_log {
            bar = bar.child(term_btn("term-ctrl-c", "Ctrl+C", t, move |_, cx| {
                app_sig.update(cx, |this, cx| this.terminal_send_interrupt(cx));
            }));
        }
        bar = bar
            .child(term_btn("term-clear", "Clear", t, move |_, cx| {
                app_clr.update(cx, |this, cx| this.terminal_clear(cx));
            }))
            .child(term_btn("term-copy", "Copy", t, move |_, cx| {
                app_cp.update(cx, |this, cx| this.terminal_copy_visible(cx));
            }));
        if !active_is_log {
            bar = bar.child(term_btn("term-paste", "Paste", t, move |_, cx| {
                app_ps.update(cx, |this, cx| this.terminal_paste_clipboard(cx));
            }));
        }
        bar.child(term_btn(
            "term-max",
            if dock.maximized { "Restore" } else { "Expand" },
            t,
            move |_, cx| {
                app_max.update(cx, |this, cx| {
                    this.terminal.maximized = !this.terminal.maximized;
                    cx.notify();
                });
            },
        ))
        .child(term_btn("term-hide", "Hide", t, move |_, cx| {
            app_hide.update(cx, |this, cx| {
                this.terminal.open = false;
                cx.notify();
            });
        }))
    };

    let tab_bar = div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(6.))
        .w_full()
        .min_w(px(0.))
        .overflow_hidden()
        .h(px(34.))
        .px(px(10.))
        .bg(rgb(t.surface))
        .border_b_1()
        .border_color(rgb(t.border))
        .child(
            div()
                .flex_shrink_0()
                .child(lucide("square-terminal", 14., t.primary)),
        )
        .child(tabs_row)
        .child(controls);

    // 3. Quick Command Bar (`$ <command>` + Run + Paste).
    let mut quick_bar = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.))
        .w_full()
        .min_w(px(0.))
        .overflow_hidden()
        .h(px(34.))
        .px(px(10.))
        .bg(rgb(t.button_bg))
        .border_b_1()
        .border_color(rgb(t.border))
        .child(
            div()
                .flex_shrink_0()
                .font_family("monospace")
                .text_size(px(t.text_data_sm))
                .text_color(rgb(t.primary))
                .child("$"),
        );
    if let Some(cwd) = dock
        .active()
        .filter(|tab| !tab.cwd.is_empty())
        .map(|tab| tab.cwd.clone())
    {
        let short = repoharbor_core::scan::abbreviate(std::path::Path::new(cwd.as_ref()));
        quick_bar = quick_bar.child(
            div()
                .max_w(px(140.))
                .flex_shrink(1.)
                .truncate()
                .font_family("monospace")
                .text_size(px(t.text_data_sm))
                .text_color(rgb(t.fg3))
                .child(short),
        );
    } else if active_is_log {
        quick_bar = quick_bar.child(
            div()
                .flex_shrink_0()
                .font_family("monospace")
                .text_size(px(t.text_data_sm))
                .text_color(rgb(t.primary))
                .child("Log → Shell"),
        );
    }
    if let Some(cmd_input) = &dock.cmd_input {
        quick_bar = quick_bar.child(
            div()
                .flex_1()
                .min_w(px(60.))
                .overflow_hidden()
                .child(Input::new(cmd_input)),
        );
    }
    let (app_run, app_paste_cmd) = (app.clone(), app.clone());
    quick_bar = quick_bar
        .child(term_btn("term-run-cmd", "Run", t, move |win, cx| {
            app_run.update(cx, |this, cx| this.terminal_run_input(win, cx, true));
        }))
        .child(term_btn("term-paste-cmd", "Paste", t, move |win, cx| {
            app_paste_cmd.update(cx, |this, cx| this.terminal_run_input(win, cx, false));
        }));

    // 4. Optional destructive command safety confirmation strip.
    let confirm_strip = dock.pending_confirm.as_ref().map(|pending| {
        let (app_yes, app_no) = (app.clone(), app.clone());
        let cmd_str = pending.command.clone();
        let tab_label = pending.title.clone();
        let short_dir = repoharbor_core::scan::abbreviate(std::path::Path::new(pending.cwd.as_ref()));
        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(8.))
            .px(px(12.))
            .py(px(5.))
            .bg(rgb(t.surface))
            .border_b_1()
            .border_color(rgb(t.dirty))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .flex_1()
                    .min_w(px(0.))
                    .child(lucide("circle-alert", 14., t.dirty))
                    .child(
                        div()
                            .truncate()
                            .text_size(px(t.text_data_sm))
                            .text_color(rgb(t.dirty))
                            .child(SharedString::from(format!(
                                "Destructive command in {tab_label} ({short_dir}) staged for review: `{cmd_str}`"
                            ))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(6.))
                    .child(term_btn("term-confirm-exec", "Press Enter", t, move |win, cx| {
                        app_yes.update(cx, |this, cx| this.terminal_confirm_pending(win, cx));
                    }))
                    .child(term_btn("term-confirm-dismiss", "Dismiss", t, move |_, cx| {
                        app_no.update(cx, |this, cx| {
                            this.terminal.pending_confirm = None;
                            cx.notify();
                        });
                    })),
            )
    });

    // 5. Interactive VT100 terminal screen viewport.
    let viewport = match dock.active() {
        Some(tab) => render_viewport(tab, cell_w, t, app, window, cx).into_any_element(),
        None => div()
            .flex()
            .flex_1()
            .items_center()
            .justify_center()
            .text_size(px(t.text_small))
            .text_color(rgb(t.fg3))
            .child("No active terminal session — click + New")
            .into_any_element(),
    };

    let (app_new_tab, app_close_tab) = (app.clone(), app.clone());
    let mut root = div()
        .id("terminal-dock")
        .flex()
        .flex_col()
        .w_full()
        .min_w(px(0.))
        .overflow_hidden()
        .h(px(dock_h))
        .bg(rgb(t.page))
        .on_action(move |_: &crate::TerminalNewTab, win, cx| {
            cx.stop_propagation();
            app_new_tab.update(cx, |this, cx| this.open_default_terminal_tab(win, cx));
        })
        .on_action(move |_: &crate::TerminalCloseTab, _win, cx| {
            cx.stop_propagation();
            app_close_tab.update(cx, |this, cx| {
                if let Some(id) = this.terminal.active_tab.clone() {
                    this.close_terminal_tab(&id, cx);
                }
            });
        })
        .child(splitter)
        .child(tab_bar)
        .child(quick_bar);
    if let Some(strip) = confirm_strip {
        root = root.child(strip);
    }
    root.child(viewport)
}

struct BuiltTerminalLine {
    text: String,
    runs: Vec<TextRun>,
    cursor_bytes: Option<(usize, usize)>,
}

/// Flatten a VT100 row's spans into one contiguous UTF-8 buffer + exact `TextRun` slices,
/// preserving spaces and isolating any active cursor cell for zero-drift shaper lookup.
fn build_line_runs(spans: &[TermSpan], cursor_col: Option<u16>, t: &Theme) -> BuiltTerminalLine {
    let mut text = String::new();
    let mut runs = Vec::new();
    let mut cur_col = 0u16;
    let mut cursor_bytes: Option<(usize, usize)> = None;

    let push_gap = |text: &mut String,
                    runs: &mut Vec<TextRun>,
                    from: u16,
                    to: u16,
                    cursor_col: Option<u16>,
                    cursor_bytes: &mut Option<(usize, usize)>| {
        if to <= from {
            return;
        }
        let push_plain_spaces =
            |text: &mut String, runs: &mut Vec<TextRun>, count: usize, fg_hex: u32| {
                if count == 0 {
                    return;
                }
                for _ in 0..count {
                    text.push(' ');
                }
                runs.push(TextRun {
                    len: count,
                    font: gpui::font("monospace"),
                    color: Hsla::from(rgb(fg_hex)),
                    ..Default::default()
                });
            };
        if let Some(cc) = cursor_col
            && cc >= from
            && cc < to
        {
            push_plain_spaces(text, runs, (cc - from) as usize, t.fg1);
            let start = text.len();
            push_plain_spaces(text, runs, 1, t.page);
            *cursor_bytes = Some((start, text.len()));
            push_plain_spaces(text, runs, (to - 1 - cc) as usize, t.fg1);
        } else {
            push_plain_spaces(text, runs, (to - from) as usize, t.fg1);
        }
    };

    for span in spans {
        if span.start_col > cur_col {
            push_gap(
                &mut text,
                &mut runs,
                cur_col,
                span.start_col,
                cursor_col,
                &mut cursor_bytes,
            );
        }
        let is_cursor =
            cursor_col.is_some_and(|cc| cc >= span.start_col && cc < span.start_col + span.cols);
        let (fg_color, bg_color) = if is_cursor {
            (t.page, None)
        } else if span.inverse {
            (
                resolve_term_color(span.bg, false, t),
                Some(resolve_term_color(span.fg, true, t)),
            )
        } else {
            let fg = resolve_term_color(span.fg, true, t);
            let bg = resolve_term_color(span.bg, false, t);
            (fg, (bg != t.page).then_some(bg))
        };

        let mut font = gpui::font("monospace");
        if span.bold {
            font.weight = FontWeight::BOLD;
        }
        if span.italic {
            font.style = FontStyle::Italic;
        }
        let start = text.len();
        text.push_str(&span.text);
        let seg_len = span.text.len();
        if seg_len > 0 {
            runs.push(TextRun {
                len: seg_len,
                font,
                color: Hsla::from(rgb(fg_color)),
                background_color: bg_color.map(|c| Hsla::from(rgb(c))),
                underline: span.underline.then(|| UnderlineStyle {
                    thickness: px(1.),
                    color: None,
                    wavy: false,
                }),
                ..Default::default()
            });
            if is_cursor {
                cursor_bytes = Some((start, text.len()));
            }
        }
        cur_col = span.start_col + span.cols;
    }

    if let Some(cc) = cursor_col
        && cursor_bytes.is_none()
    {
        if cur_col < cc {
            push_gap(&mut text, &mut runs, cur_col, cc, None, &mut cursor_bytes);
        }
        let start = text.len();
        text.push(' ');
        runs.push(TextRun {
            len: 1,
            font: gpui::font("monospace"),
            color: Hsla::from(rgb(t.page)),
            ..Default::default()
        });
        cursor_bytes = Some((start, text.len()));
    }

    BuiltTerminalLine {
        text,
        runs,
        cursor_bytes,
    }
}

fn render_viewport(
    tab: &TerminalTab,
    cell_w: f32,
    t: &Theme,
    app: &Entity<RepoHarborApp>,
    window: &Window,
    _cx: &Context<RepoHarborApp>,
) -> impl IntoElement {
    let snap = &tab.snapshot;
    let show_cursor = snap.cursor_visible && snap.scrollback_offset == 0 && snap.exited.is_none();
    let mut rows_col = div()
        .relative()
        .flex()
        .flex_col()
        .w_full()
        .flex_1()
        .min_h(px(0.))
        .overflow_hidden();

    for (r_idx, spans) in snap.lines.iter().enumerate() {
        let active_cursor_col =
            (show_cursor && r_idx == snap.cursor_row as usize).then_some(snap.cursor_col);
        let mut line_box = div()
            .relative()
            .h(px(LINE_H))
            .w_full()
            .overflow_hidden()
            .whitespace_nowrap()
            .font_family("monospace")
            .text_size(px(13.))
            .line_height(px(LINE_H));

        if spans.is_empty() && active_cursor_col.is_none() {
            rows_col = rows_col.child(line_box);
            continue;
        }

        let built = build_line_runs(spans, active_cursor_col, t);
        if let Some((b_start, b_end)) = built.cursor_bytes {
            let shaped = window.text_system().shape_line(
                SharedString::from(built.text.clone()),
                px(13.),
                &built.runs,
                None,
            );
            let cursor_x: Pixels = shaped.x_for_index(b_start);
            let end_x: Pixels = shaped.x_for_index(b_end);
            let cursor_w = px((f32::from(end_x) - f32::from(cursor_x)).max(cell_w * 0.65));
            line_box = line_box.child(
                div()
                    .absolute()
                    .left(cursor_x)
                    .top(px(0.))
                    .w(cursor_w)
                    .h(px(LINE_H))
                    .rounded(px(1.))
                    .bg(rgb(t.primary)),
            );
        }

        if !built.text.is_empty() {
            line_box = line_box
                .child(StyledText::new(SharedString::from(built.text)).with_runs(built.runs));
        }
        rows_col = rows_col.child(line_box);
    }

    let (app_focus, app_key, app_wheel, app_esc, app_slash, app_cp, app_ps) = (
        app.clone(),
        app.clone(),
        app.clone(),
        app.clone(),
        app.clone(),
        app.clone(),
        app.clone(),
    );
    let focus_handle = tab.focus.clone();

    let mut vp = div()
        .id("terminal-viewport")
        .key_context("Terminal")
        .track_focus(&tab.focus)
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .overflow_hidden()
        .px(px(12.))
        .py(px(6.))
        .bg(rgb(t.page))
        .on_mouse_down(MouseButton::Left, move |_ev, window, cx| {
            window.focus(&focus_handle, cx);
            app_focus.update(cx, |_, cx| cx.notify());
        })
        .on_key_down(move |ev, _window, cx| {
            if let Some(bytes) = encode_keystroke(ev) {
                cx.stop_propagation();
                app_key.update(cx, |this, cx| {
                    if let Some(tab) = this.terminal.active()
                        && let Some(pty) = tab.backend.as_pty()
                    {
                        pty.set_scrollback(0);
                        let _ = pty.write_bytes(&bytes);
                    }
                    cx.notify();
                });
            }
        })
        .on_scroll_wheel(move |ev, _window, cx| {
            let dy = match ev.delta {
                ScrollDelta::Lines(p) => (p.y * 3.0).round() as isize,
                ScrollDelta::Pixels(p) => (f32::from(p.y) / LINE_H).round() as isize,
            };
            if dy != 0 {
                cx.stop_propagation();
                app_wheel.update(cx, |this, cx| {
                    if let Some(tab) = this.terminal.active_mut() {
                        tab.backend.scroll_by(dy);
                        tab.snapshot = tab.backend.snapshot();
                    }
                    cx.notify();
                });
            }
        })
        .on_action(move |_: &crate::TerminalEscape, _win, cx| {
            cx.stop_propagation();
            app_esc.update(cx, |this, _| {
                if let Some(tab) = this.terminal.active()
                    && let Some(pty) = tab.backend.as_pty()
                {
                    let _ = pty.write_bytes(b"\x1b");
                }
            });
        })
        .on_action(move |_: &crate::TerminalSlash, _win, cx| {
            cx.stop_propagation();
            app_slash.update(cx, |this, _| {
                if let Some(tab) = this.terminal.active()
                    && let Some(pty) = tab.backend.as_pty()
                {
                    let _ = pty.write_bytes(b"/");
                }
            });
        })
        .on_action(move |_: &crate::TerminalCopy, _win, cx| {
            cx.stop_propagation();
            app_cp.update(cx, |this, cx| this.terminal_copy_visible(cx));
        })
        .on_action(move |_: &crate::TerminalPaste, _win, cx| {
            cx.stop_propagation();
            app_ps.update(cx, |this, cx| this.terminal_paste_clipboard(cx));
        })
        .child(rows_col);

    if snap.scrollback_offset > 0 {
        let app_bottom = app.clone();
        vp = vp.child(
            div()
                .id("term-scrollback-pill")
                .absolute()
                .right(px(16.))
                .bottom(px(10.))
                .px(px(8.))
                .py(px(3.))
                .rounded(px(t.r_xs))
                .bg(rgb(t.accent_wash))
                .border_1()
                .border_color(rgb(t.primary))
                .cursor_pointer()
                .text_size(px(t.text_data_sm))
                .text_color(rgb(t.primary))
                .child(SharedString::from(format!(
                    "Scrolled ↑ {} lines · Jump to bottom",
                    snap.scrollback_offset
                )))
                .on_click(move |_, _, cx| {
                    app_bottom.update(cx, |this, cx| {
                        if let Some(tab) = this.terminal.active_mut() {
                            tab.backend.set_scrollback(0);
                            tab.snapshot = tab.backend.snapshot();
                        }
                        cx.notify();
                    });
                }),
        );
    }

    vp
}

fn term_toggle_btn(
    id: &'static str,
    label: &'static str,
    active: bool,
    t: &Theme,
    on_click: impl Fn(&mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let hb = t.border_strong;
    div()
        .id(id)
        .flex_shrink_0()
        .px(px(6.))
        .py(px(3.))
        .rounded(px(t.r_xs))
        .bg(rgb(if active { t.accent_wash } else { t.button_bg }))
        .border_1()
        .border_color(rgb(if active { t.border_accent } else { t.border }))
        .hover(move |s| s.border_color(rgb(hb)))
        .cursor_pointer()
        .text_size(px(t.text_data_sm))
        .text_color(rgb(if active { t.accent_bright } else { t.fg2 }))
        .child(label)
        .on_click(move |_ev, win, cx| on_click(win, cx))
}

fn term_btn(
    id: &'static str,
    label: &'static str,
    t: &Theme,
    on_click: impl Fn(&mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let (hb, hf) = (t.border_strong, t.fg0);
    div()
        .id(id)
        .flex_shrink_0()
        .px(px(6.))
        .py(px(3.))
        .rounded(px(t.r_xs))
        .bg(rgb(t.button_bg))
        .border_1()
        .border_color(rgb(t.border))
        .hover(move |s| s.border_color(rgb(hb)).text_color(rgb(hf)))
        .cursor_pointer()
        .text_size(px(t.text_data_sm))
        .text_color(rgb(t.fg2))
        .child(label)
        .on_click(move |_ev, win, cx| on_click(win, cx))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_ctrl_c_and_arrows_and_passes_shortcuts() {
        use gpui::{Keystroke, Modifiers};

        let make = |key: &str, m: Modifiers, ch: Option<&str>| KeyDownEvent {
            keystroke: Keystroke {
                modifiers: m,
                key: key.to_string(),
                key_char: ch.map(str::to_string),
            },
            is_held: false,
            prefer_character_input: false,
        };

        assert_eq!(
            encode_keystroke(&make("c", Modifiers::control(), None)),
            Some(vec![0x03])
        );
        assert_eq!(
            encode_keystroke(&make("up", Modifiers::none(), None)),
            Some(b"\x1b[A".to_vec())
        );
        assert_eq!(
            encode_keystroke(&make("enter", Modifiers::none(), None)),
            Some(b"\r".to_vec())
        );
        assert_eq!(
            encode_keystroke(&make("/", Modifiers::none(), Some("/"))),
            Some(b"/".to_vec())
        );
        // Ctrl+` must be ignored by PTY encoder so ToggleTerminal fires cleanly.
        assert_eq!(
            encode_keystroke(&make("`", Modifiers::control(), None)),
            None
        );
    }

    #[test]
    fn ansi_colors_map_default_and_rgb() {
        let t = Theme::dark();
        assert_eq!(resolve_term_color(TermColor::Default, true, &t), t.fg1);
        assert_eq!(resolve_term_color(TermColor::Default, false, &t), t.page);
        assert_eq!(
            resolve_term_color(TermColor::Rgb(0x12, 0x34, 0x56), true, &t),
            0x123456
        );
    }

    #[test]
    fn build_line_runs_isolates_cursor_bytes_and_extends_trailing_spaces() {
        let t = Theme::dark();
        let spans = vec![TermSpan {
            text: "user@host:~$".into(),
            fg: TermColor::Indexed(2),
            bg: TermColor::Default,
            bold: true,
            italic: false,
            underline: false,
            inverse: false,
            start_col: 0,
            cols: 12,
        }];
        // Cursor sits one column past `$` (after a single trailing space at col 12).
        let built = build_line_runs(&spans, Some(13), &t);
        assert_eq!(built.text, "user@host:~$  ");
        let range = built.cursor_bytes.expect("cursor byte range");
        assert_eq!(range, (13, 14));
        assert_eq!(
            built.runs.iter().map(|r| r.len).sum::<usize>(),
            built.text.len()
        );
    }

    #[test]
    fn build_line_runs_highlights_midspan_cursor_cell() {
        let t = Theme::dark();
        let spans = vec![
            TermSpan {
                text: "git ".into(),
                fg: TermColor::Default,
                bg: TermColor::Default,
                bold: false,
                italic: false,
                underline: false,
                inverse: false,
                start_col: 0,
                cols: 4,
            },
            TermSpan {
                text: "s".into(),
                fg: TermColor::Default,
                bg: TermColor::Default,
                bold: false,
                italic: false,
                underline: false,
                inverse: false,
                start_col: 4,
                cols: 1,
            },
            TermSpan {
                text: "tatus".into(),
                fg: TermColor::Default,
                bg: TermColor::Default,
                bold: false,
                italic: false,
                underline: false,
                inverse: false,
                start_col: 5,
                cols: 5,
            },
        ];
        let built = build_line_runs(&spans, Some(4), &t);
        assert_eq!(built.text, "git status");
        assert_eq!(built.cursor_bytes, Some((4, 5)));
        assert_eq!(built.runs[1].color, Hsla::from(rgb(t.page)));
    }
}
