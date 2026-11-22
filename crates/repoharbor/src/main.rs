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
mod desktop;
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
mod terminal_view;
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
        FleetPullSelected,
        FocusSearch,
        ToggleTerminal,
        TerminalNewTab,
        TerminalCloseTab,
        TerminalCopy,
        TerminalPaste,
        TerminalEscape,
        TerminalSlash
    ]
);

fn run_cli_if_requested() -> Option<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let first = args.first()?.as_str();
    match first {
        "--version" | "-V" => {
            println!("RepoHarbor v{}", identity::APP_VERSION);
            Some(0)
        }
        "--help" | "-h" => {
            println!(
                "RepoHarbor v{}\n\n\
                 Usage:\n  \
                   repoharbor                        Launch the desktop UI\n  \
                   repoharbor update [--check] [--quiet]  Check or apply updates\n  \
                   repoharbor install-desktop [--autostart]  Install .desktop entry & icons\n  \
                   repoharbor --version              Show installed version",
                identity::APP_VERSION
            );
            Some(0)
        }
        "install-desktop" => {
            let autostart = args.iter().any(|a| a == "--autostart");
            match desktop::install_desktop_entry(autostart) {
                Ok(path) => {
                    println!("Installed desktop entry: {}", path.display());
                    if autostart {
                        println!("Autostart enabled on login.");
                    }
                    Some(0)
                }
                Err(e) => {
                    eprintln!("install-desktop failed: {e}");
                    Some(1)
                }
            }
        }
        "update" => Some(cli_update(&args[1..])),
        _ => None,
    }
}

fn cli_update(args: &[String]) -> i32 {
    use repoharbor_core::updater::{
        InstallKind, UpdateCheck, check_latest_release, detect_install_kind,
        download_and_apply_release_update,
    };

    let check_only = args.iter().any(|a| a == "--check");
    let quiet = args.iter().any(|a| a == "--quiet");
    let kind = detect_install_kind();

    if let InstallKind::SourceCheckout { repo_dir, .. } = &kind
        && !check_only
    {
        if !quiet {
            println!(
                "Source installation detected ({}). Checking git updates…",
                repo_dir.display()
            );
        }
        let dirty = std::process::Command::new("git")
            .args(["-C", &repo_dir.to_string_lossy(), "status", "--porcelain"])
            .output()
            .ok()
            .is_some_and(|o| !o.stdout.is_empty());
        if dirty {
            eprintln!(
                "Local source tree {} has uncommitted changes — commit or stash first, or check GitHub releases with `--check`.",
                repo_dir.display()
            );
            return 1;
        }
        let pull_ok = std::process::Command::new("git")
            .args(["-C", &repo_dir.to_string_lossy(), "pull", "--ff-only"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !pull_ok {
            eprintln!("git pull --ff-only failed in {}", repo_dir.display());
            return 1;
        }
        let build_ok = if cfg!(target_os = "windows") {
            let installer = repo_dir.join("scripts").join("install.ps1");
            std::process::Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-File",
                    installer.to_string_lossy().as_ref(),
                    "-FromSource",
                ])
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        } else {
            let installer = repo_dir.join("scripts/install.sh");
            std::process::Command::new("bash")
                .args([installer.to_string_lossy().as_ref(), "--from-source"])
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        };
        if build_ok {
            println!("RepoHarbor updated from source.");
            return 0;
        }
        eprintln!("source rebuild/install failed");
        return 1;
    }

    let Ok(rt) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        eprintln!("failed to initialize async runtime for update check");
        return 1;
    };

    match rt.block_on(check_latest_release(identity::APP_VERSION)) {
        Ok(UpdateCheck::UpToDate {
            current,
            latest_tag,
        }) => {
            if !quiet {
                match latest_tag {
                    Some(tag) => println!("RepoHarbor v{current} is up to date ({tag})."),
                    None => {
                        println!("RepoHarbor v{current} is installed (no newer published release).")
                    }
                }
            }
            0
        }
        Ok(UpdateCheck::Available { current, release }) => {
            if check_only {
                println!(
                    "Update available: v{current} -> {} ({})",
                    release.tag_name, release.html_url
                );
                return 0;
            }
            if !kind.can_self_update_in_place() {
                println!(
                    "Update available: v{current} -> {}\nRun: {}",
                    release.tag_name,
                    kind.external_update_command()
                );
                return 0;
            }
            if !quiet {
                println!("Updating RepoHarbor v{current} -> {}…", release.tag_name);
            }
            match rt.block_on(download_and_apply_release_update(&release, &kind)) {
                Ok(dest) => {
                    println!("Updated {} to {}.", dest.display(), release.tag_name);
                    let restarted = repoharbor_core::updater::restart_running_gui_instances(&dest);
                    if restarted && !quiet {
                        println!("Restarting running RepoHarbor application…");
                    }
                    let _ = rt.block_on(repoharbor_platform::notify::send(
                        "RepoHarbor Updated",
                        &format!(
                            "Updated to {} — {}",
                            release.tag_name,
                            if restarted {
                                "restarting automatically."
                            } else {
                                "launch RepoHarbor to use the new build."
                            }
                        ),
                    ));
                    0
                }
                Err(e) => {
                    eprintln!("Update failed: {e}");
                    1
                }
            }
        }
        Err(e) => {
            if !quiet {
                eprintln!("Update check failed: {e}");
            }
            1
        }
    }
}

fn main() {
    if let Some(code) = run_cli_if_requested() {
        std::process::exit(code);
    }
    // Bound libgit2 mmap/cache memory and glibc arenas before spawning any threads.
    repoharbor_core::git_ops::init_runtime_limits();
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
                // Focus Mission Control repo search
                KeyBinding::new("cmd-f", FocusSearch, None),
                KeyBinding::new("ctrl-f", FocusSearch, None),
                KeyBinding::new("/", FocusSearch, None),
                // Embedded terminal dock toggle + terminal key-context overrides.
                KeyBinding::new("ctrl-`", ToggleTerminal, None),
                KeyBinding::new("cmd-`", ToggleTerminal, None),
                KeyBinding::new("ctrl-shift-t", TerminalNewTab, Some("Terminal")),
                KeyBinding::new("ctrl-shift-w", TerminalCloseTab, Some("Terminal")),
                KeyBinding::new("ctrl-shift-c", TerminalCopy, Some("Terminal")),
                KeyBinding::new("ctrl-shift-v", TerminalPaste, Some("Terminal")),
                KeyBinding::new("escape", TerminalEscape, Some("Terminal")),
                KeyBinding::new("/", TerminalSlash, Some("Terminal")),
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
                    window.set_window_title(&format!("RepoHarbor v{}", identity::APP_VERSION));
                    // Second zoom pass helps some Wayland CSD compositors that
                    // ignore the initial Maximized hint on first map.
                    if !window.is_maximized() {
                        window.zoom_window();
                    }
                    let sidebar_width = config
                        .sidebar_width
                        .clamp(shell::SIDEBAR_MIN, shell::SIDEBAR_MAX);
                    let sidebar_collapsed = config.sidebar_collapsed;
                    let terminal_height = config.terminal_height;
                    let view = cx.new(|cx| {
                        // Start the live wiring: filesystem watch, appearance,
                        // attention poll, and system tray all marshal back onto
                        // this entity. Returns the tray handle (if the tray
                        // came up — the app pushes attention summaries to it)
                        // plus the watcher handle (re-armed when repos are
                        // added).
                        let (tray, watcher) = live::spawn(cx);
                        let layout = config.layout;
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
                            fleet_queue: Default::default(),
                            fleet_seq: 0,
                            fleet_prune: None,
                            fleet_prune_seq: 0,
                            fleet_reset: None,
                            fleet_discard: None,
                            fleet_commit: None,
                            fleet_merge: None,
                            fleet_gen_push: None,
                            notice_detail: None,
                            last_pull: None,
                            toasts: Vec::new(),
                            toast_seq: 0,
                            activity_log: Default::default(),
                            grid: crate::shell::GridState {
                                layout,
                                ..Default::default()
                            },
                            view_filter: None,
                            focus: cx.focus_handle(),
                            sidebar_width,
                            sidebar_collapsed,
                            sidebar_dragging: false,
                            repo_search: None,
                            _repo_search_sub: None,
                            terminal: crate::terminal_view::TerminalDockState::new(terminal_height),
                            install_kind: repoharbor_core::updater::detect_install_kind(),
                            update_state: Default::default(),
                            autostart_enabled: repoharbor_core::config::is_autostart_enabled(),
                            rescan_in_flight: false,
                            rescan_queued: false,
                            rescan_rearm_pending: false,
                            activity_loading: false,
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
                        this.refresh_github_authed(cx);
                        this.ai_startup(cx);
                        this.enrich_hosts(cx);
                        this.refresh_ci(cx);
                        this.load_activity(cx);
                        // Detect dispatched-agent sessions that ended while
                        // RepoHarbor was closed, and resume supervising live ones
                        // (#185) — see `RepoHarborApp::agents_startup`.
                        this.agents_startup(cx);
                        this.updater_startup(cx);
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
