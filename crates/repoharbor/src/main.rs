//! Native RepoHarbor (rewrite) — the desktop GPUI app. All logic comes from the
//! `repoharbor-core` crate (scan/git/forge/inbox/ai/cache/config); this crate is
//! purely the UI: theme, cards, shell, views. No webview, no IPC. Reading the
//! shipping `~/.local/share/repoharbor/cache.sqlite` is `repoharbor_core::cache`.
//!
//! Phase 1: real `--orr-*` theme + faithful RepoCard. Phase 2: the app shell —
//! header + sidebar nav + view switching (`shell.rs`).

mod activity_log;
mod assets;
mod card;
mod data;
mod drawer;
mod fleet;
mod heatmap;
mod icon;
mod identity;
mod live;
mod menu_actions;
mod palette;
mod shell;
mod task;
mod theme;
mod toast;
mod views;

use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, AppContext, Application, Bounds, KeyBinding, WindowBounds, WindowDecorations,
    WindowOptions, actions, px, size,
};
use gpui_component::{Root, TitleBar};

use shell::{RepoHarborApp, View};
use theme::Theme;

/// Window / taskbar icon (X11). Wayland picks the icon via `app_id` + `.desktop`.
fn window_icon() -> Option<Arc<image::RgbaImage>> {
    const PNG: &[u8] = include_bytes!("../../../packaging/icons/128x128.png");
    let img = image::load_from_memory(PNG).ok()?.into_rgba8();
    Some(Arc::new(img))
}

actions!(
    repoharbor,
    [
        CloseOverlay,
        OpenPalette,
        PaletteUp,
        PaletteDown,
        PaletteConfirm,
        FleetFetchSelected,
        FleetPullSelected
    ]
);

fn main() {
    // Point the bundled llama.cpp backend at a runtime shipped next to the
    // binary, if any: packages install it to `<prefix>/lib/repoharbor/llama-runtime`
    // (the AppImage bundles one; deb/rpm stay lean). A no-op in source builds /
    // when nothing is there — `materialize_bundled` only acts if it finds a
    // `llama-server`, so the discovery falls through to Ollama / PATH otherwise.
    if let Ok(exe) = std::env::current_exe()
        && let Some(prefix) = exe.parent().and_then(|p| p.parent())
    {
        repoharbor_core::llama::set_bundled_dir(prefix.join("lib/repoharbor/llama-runtime"));
    }

    let now = data::now_unix();
    let snap = data::load(now);
    eprintln!(
        "[native] loaded {} repos across {} roots",
        snap.rows.len(),
        snap.roots
    );
    // Borrow the desktop's accent colour (KDE/portal) so the app harmonises
    // with the user's theme — the design system's runtime accent override.
    let accent = repoharbor_platform::appearance::read_blocking()
        .accent
        .map(|c| (c.r, c.g, c.b));
    if let Some((r, g, b)) = accent {
        eprintln!("[native] system accent #{r:02x}{g:02x}{b:02x}");
    }
    let theme = Rc::new(Theme::dark().with_system_accent(accent));
    let config = repoharbor_core::config::load();

    let platform = gpui_platform::current_platform(false);
    Application::with_platform(platform)
        .with_assets(assets::Assets)
        .run(move |cx: &mut App| {
            // Initialise gpui-component, then map its theme onto our --orr-* tokens
            // so its components match the rest of the UI.
            gpui_component::init(cx);
            theme::apply_gpui_component_theme(&theme, cx);
            // Esc closes the active overlay (drawer/palette/dialog).
            cx.bind_keys([KeyBinding::new("escape", CloseOverlay, None)]);
            // Command palette: Ctrl/Cmd+K opens from anywhere; arrows/enter are
            // scoped to the "Palette" key-context so they don't shadow a focused
            // text input's cursor/newline keys.
            cx.bind_keys([
                KeyBinding::new("cmd-k", OpenPalette, None),
                KeyBinding::new("ctrl-k", OpenPalette, None),
                KeyBinding::new("up", PaletteUp, Some("Palette")),
                KeyBinding::new("down", PaletteDown, Some("Palette")),
                KeyBinding::new("enter", PaletteConfirm, Some("Palette")),
                // Fleet fetch / pull on the current selection (Mission Control).
                KeyBinding::new("cmd-shift-f", FleetFetchSelected, None),
                KeyBinding::new("ctrl-shift-f", FleetFetchSelected, None),
                KeyBinding::new("cmd-shift-p", FleetPullSelected, None),
                KeyBinding::new("ctrl-shift-p", FleetPullSelected, None),
            ]);

            // Restore size if the user un-maximizes; open maximized (not exclusive
            // fullscreen) so Mission Control fills the display on first paint.
            // Centered 1320×880 is the un-maximize restore target; Maximized
            // triggers platform zoom() (see gpui WindowBounds match).
            let restore = Bounds::centered(None, size(px(1320.), px(880.)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Maximized(restore)),
                    // Client-side decorations (Wayland default) need an in-app
                    // TitleBar for minimize / maximize / close. Without this the
                    // window has no system chrome and no window-control buttons.
                    titlebar: Some(TitleBar::title_bar_options()),
                    window_decorations: Some(WindowDecorations::Client),
                    // Matches packaging desktop file / icons for taskbar pin.
                    app_id: Some("com.digitscode.repoharbor".into()),
                    icon: window_icon(),
                    ..Default::default()
                },
                |window, cx| {
                    window.set_window_title("RepoHarbor");
                    // Second zoom pass helps some Wayland CSD compositors that
                    // ignore the initial Maximized hint on first map.
                    if !window.is_maximized() {
                        window.zoom_window();
                    }
                    let sidebar_width = config
                        .sidebar_width
                        .clamp(shell::SIDEBAR_MIN, shell::SIDEBAR_MAX);
                    let sidebar_collapsed = config.sidebar_collapsed;
                    let view = cx.new(|cx| {
                        // Start the live wiring: filesystem watch, appearance,
                        // attention poll, and system tray all marshal back onto
                        // this entity. Returns the tray handle (if the tray
                        // came up — the app pushes attention summaries to it)
                        // plus the watcher handle (re-armed when repos are
                        // added).
                        let (tray, watcher) = live::spawn(cx);
                        RepoHarborApp {
                            view: View::Grid,
                            rows: snap.rows,
                            roots: snap.roots,
                            repos: snap.repos,
                            theme,
                            config,
                            attention: Vec::new(),
                            polled_inbox: None,
                            // Seed CI states from the cache so a failing
                            // default branch shows on the first attention
                            // pass, before the CI pass first runs (offline
                            // included). The pass refreshes them after.
                            ci_states: repoharbor_core::cache::all_ci_states(),
                            ci_last_error: None,
                            attention_items: Vec::new(),
                            attention_by_repo: Default::default(),
                            attention_seen: None,
                            tray_attention: Default::default(),
                            overlay: None,
                            drawer: Default::default(),
                            inbox: Default::default(),
                            feed: Default::default(),
                            explore: Default::default(),
                            cleanup: Default::default(),
                            cleanup_confirm: None,
                            cleanup_confirm_gen: 0,
                            agents: Default::default(),
                            active_agents: Default::default(),
                            agents_polling: false,
                            agents_confirm: None,
                            agents_confirm_gen: 0,
                            agents_review: None,
                            agents_review_diff: None,
                            agents_pr_busy: Default::default(),
                            explore_cloning: Default::default(),
                            explore_errors: Default::default(),
                            settings: None,
                            devtools: None,
                            services: Default::default(),
                            tray,
                            watcher,
                            selected: Default::default(),
                            fleet_run: None,
                            fleet_seq: 0,
                            fleet_prune: None,
                            fleet_prune_seq: 0,
                            fleet_reset: None,
                            fleet_discard: None,
                            fleet_commit: None,
                            generate_commit_prompt: None,
                            notice_detail: None,
                            last_pull: None,
                            toasts: Vec::new(),
                            toast_seq: 0,
                            activity_log: Default::default(),
                            grid: Default::default(),
                            view_filter: None,
                            focus: cx.focus_handle(),
                            sidebar_width,
                            sidebar_collapsed,
                            sidebar_dragging: false,
                            repo_search: None,
                            _repo_search_sub: None,
                        }
                    });
                    // Close (✕) quits the app. Minimize (−) still hides the
                    // window; the tray "Quit" entry also exits. (Earlier builds
                    // remapped close → tray, which felt like a broken close button.)
                    // Probe AI reachability and build the semantic index in the
                    // background, so Ctrl+K can search by meaning. Also kick off a
                    // host-enrichment pass so cards fill in stars/visibility.
                    view.update(cx, |this, cx| {
                        // First attention pass from the launch snapshot's local
                        // git facts (dirty/ahead/behind), so badges and card
                        // dots are right on the first paint; host/inbox/agent
                        // facts refine it as their sources load.
                        this.recompute_attention();
                        this.ai_startup(cx);
                        this.enrich_hosts(cx);
                        this.refresh_ci(cx);
                        this.load_activity(cx);
                        // Detect dispatched-agent sessions that ended while
                        // RepoHarbor was closed, and resume supervising live ones
                        // (#185) — see `RepoHarborApp::agents_startup`.
                        this.agents_startup(cx);
                    });
                    // Focus the app root so key bindings (Esc) dispatch to it.
                    let focus = view.read(cx).focus.clone();
                    window.focus(&focus, cx);
                    // gpui-component's Root provides the theme + popover/modal/
                    // notification layers its components need.
                    cx.new(|cx| Root::new(view, window, cx))
                },
            )
            .expect("failed to open window");
            cx.activate(true);
        });
}
