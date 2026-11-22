//! Live wiring — marshal background desktop signals onto the GPUI foreground so
//! the running app reacts without a manual refresh. Several sources, each owning
//! its own thread + runtime in `repoharbor-platform`:
//!
//! - **filesystem watch** → rescan the roots and reload the grid;
//! - **appearance change** → recompute the theme with the new system accent;
//! - **attention poll** → feed the poll's inbox facts into the attention model
//!   (badges, tray summary, urgent notifications — all downstream of
//!   `recompute_attention`; the poller itself only notifies the non-model
//!   PR/CI kinds);
//! - **global shortcut** (Ctrl+Alt+O) → raise the window;
//! - **system tray** → show / rescan / quit / open-a-repo.
//!
//! GPUI is single-threaded: entity mutation needs `&mut App`, which only exists
//! on the foreground. So each background callback just pushes a [`Signal`] onto
//! an `async-channel`, and one gpui task drains it, updating the entity (with
//! `cx.notify()`) on the foreground. Heavy work (the rescan) is handed to the
//! background executor so it never blocks the UI.

use std::collections::VecDeque;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::Context;
use repoharbor_core::inbox::InboxItem;
use repoharbor_platform::appearance::Appearance;
use repoharbor_platform::notifier::Notice;
use repoharbor_platform::tray::TrayAction;

use crate::data;
use crate::shell::RepoHarborApp;
use crate::theme::Theme;

/// Minimum interval between automatic watcher-driven workspace rescans.
/// Bursts of filesystem events coalesce into at most one rescan per window.
const AUTO_RESCAN_COOLDOWN: Duration = Duration::from_secs(5);

/// A desktop signal to apply on the GPUI foreground.
enum Signal {
    /// Repos changed on disk — coalesced, cooldown-throttled rescan.
    ReposChanged,
    /// Explicit tray "Rescan" click — runs immediately via `rescan_user`.
    ManualRescan,
    /// Desktop theme/accent changed — recompute the theme.
    Appearance(Appearance),
    /// Latest attention-poll result: glance lines (the Inbox nav badge's
    /// fallback) + the raw inbox facts for the attention model + desktop
    /// notices the poller just fired (so the Log can record them).
    Attention(Vec<String>, Vec<InboxItem>, Vec<Notice>),
    /// Raise the main window (tray: left-click / "Show RepoHarbor").
    ShowWindow,
    /// Quit the app (tray: "Quit").
    Quit,
}

/// Start the background watchers and the gpui task that applies their signals.
/// Call once during app construction (inside `cx.new`). Returns the tray
/// handle if the system tray came up — the app stores it to push attention
/// summaries after each recompute, and the window's close-to-tray behaviour is
/// gated on its presence so there's always a way to quit when there's no tray
/// — and the fs-watcher handle, so the app can re-arm the watches when
/// repos/roots are added at runtime (Settings save, New Project, Explore
/// clone).
pub fn spawn(
    cx: &mut Context<RepoHarborApp>,
) -> (
    Option<repoharbor_platform::tray::TrayHandle>,
    repoharbor_platform::watcher::WatcherHandle,
) {
    let (tx, rx) = async_channel::unbounded::<Signal>();

    // Filesystem watch → rescan. Debounced inside the platform watcher.
    let watcher = {
        let tx = tx.clone();
        repoharbor_platform::watcher::spawn(move || {
            let _ = tx.try_send(Signal::ReposChanged);
        })
    };

    // Desktop appearance (theme/accent) → live theme. Fires once immediately, so
    // the launch accent is reconfirmed (a no-op past the synchronous startup read).
    {
        let tx = tx.clone();
        repoharbor_platform::appearance::watch(move |appearance| {
            let _ = tx.try_send(Signal::Appearance(appearance));
        });
    }

    // Attention poll → Inbox badge + attention-model inbox facts. The poller
    // still notifies its non-model kinds (new PR / CI alert) itself; urgent
    // model items notify from the app after each recompute.
    {
        let tx = tx.clone();
        repoharbor_platform::notifier::watch(move |lines, items, notices| {
            let _ = tx.try_send(Signal::Attention(lines, items, notices));
        });
    }

    // Global shortcut (Ctrl+Alt+O) → raise the window, via the portal.
    // Registration is gated off in `shortcut::spawn` so first-run does not
    // pop the desktop "add a shortcut" permission dialog; install stays opt-in
    // via `scripts/install-desktop.sh` / desktop settings.
    {
        let tx = tx.clone();
        repoharbor_platform::shortcut::spawn(move || {
            let _ = tx.try_send(Signal::ShowWindow);
        });
    }

    // KDE KRunner service. Self-contained — it reads the cached repo list and
    // opens a match in the IDE directly, so it needs no channel back to the app.
    repoharbor_platform::krunner::spawn();

    // System tray. Menu activations come back on the tray thread; forward the
    // app-level ones onto the channel (Open is handled inside the tray itself).
    let tray = {
        let tx = tx.clone();
        repoharbor_platform::tray::spawn(move |action| {
            let signal = match action {
                TrayAction::Show => Signal::ShowWindow,
                TrayAction::Rescan => Signal::ManualRescan,
                TrayAction::Quit => Signal::Quit,
                TrayAction::Open(_) => return, // handled in the tray
            };
            let _ = tx.try_send(signal);
        })
    };

    // The single foreground consumer. Holds a weak handle to the app entity; it
    // ends naturally when the entity is dropped (its `update` calls start failing
    // and the channel closes). Keeps a tray-handle clone to push panel-theme
    // updates (the attention summary is pushed by the app itself, from
    // `recompute_attention`, through the clone returned below).
    let tray_for_app = tray.clone();
    cx.spawn(async move |this, cx| {
        let mut pending: VecDeque<Signal> = VecDeque::new();
        let mut last_auto_scan: Option<Instant> = None;

        loop {
            let signal = if let Some(s) = pending.pop_front() {
                s
            } else {
                let Ok(s) = rx.recv().await else {
                    break;
                };
                s
            };

            match signal {
                Signal::ManualRescan => {
                    if this.update(cx, |app, cx| app.rescan_user(cx)).is_err() {
                        break;
                    }
                }
                Signal::ReposChanged => {
                    // Drain all queued signals immediately so a burst of watcher
                    // events collapses into a single rescan without dropping
                    // unrelated signals (Appearance / Attention / Quit).
                    while let Ok(next) = rx.try_recv() {
                        if !matches!(next, Signal::ReposChanged) {
                            pending.push_back(next);
                        }
                    }

                    // Enforce a minimum cooldown between automatic rescans.
                    if let Some(last) = last_auto_scan {
                        let elapsed = last.elapsed();
                        if elapsed < AUTO_RESCAN_COOLDOWN {
                            cx.background_executor()
                                .timer(AUTO_RESCAN_COOLDOWN - elapsed)
                                .await;
                            while let Ok(next) = rx.try_recv() {
                                if !matches!(next, Signal::ReposChanged) {
                                    pending.push_back(next);
                                }
                            }
                        }
                    }

                    // Skip automatic watcher rescans while a Fleet run or an
                    // explicit rescan is already in flight — Fleet triggers one
                    // authoritative rescan when it finishes.
                    let should_scan = this.update(cx, |app, cx| {
                        if app.fleet_run.is_some() || app.rescan_in_flight {
                            return false;
                        }
                        app.rescan_in_flight = true;
                        app.activity_log.push(
                            data::now_unix(),
                            crate::activity_log::LogLevel::Info,
                            "Scan started",
                        );
                        cx.notify();
                        true
                    });
                    let Ok(run_now) = should_scan else {
                        break;
                    };
                    if !run_now {
                        continue;
                    }

                    let snap = cx
                        .background_executor()
                        .spawn(async { data::rescan() })
                        .await;
                    last_auto_scan = Some(Instant::now());

                    // Collapse any duplicate watcher signals emitted during the scan.
                    while let Ok(next) = rx.try_recv() {
                        if !matches!(next, Signal::ReposChanged) {
                            pending.push_back(next);
                        }
                    }

                    let applied = this.update(cx, |app, cx| {
                        app.rescan_in_flight = false;
                        app.apply_snapshot(snap);
                        // Drop fleet-selected ids for repos that vanished.
                        app.prune_selection();
                        // Nested WT edits/deletes just triggered this rescan —
                        // keep the open Changes tab in sync with the new status.
                        app.refresh_open_changes(cx);
                        // Local file edits do not change remote stars/CI; keep
                        // enrich/CI driven by startup, 180s Attention poll,
                        // Fleet completion, and manual Refresh.
                        app.index_semantic();
                        app.load_activity(cx);
                        app.activity_log.push(
                            data::now_unix(),
                            crate::activity_log::LogLevel::Info,
                            format!("Scan finished — {} repos", app.rows.len()),
                        );
                        if app.rescan_queued {
                            let rearm = app.rescan_rearm_pending;
                            app.rescan_queued = false;
                            app.rescan_rearm_pending = false;
                            if rearm {
                                app.rescan_with_rearm(cx);
                            } else {
                                app.rescan(cx);
                            }
                        }
                        cx.notify();
                    });
                    if applied.is_err() {
                        break; // entity gone — stop draining
                    }
                }
                Signal::Appearance(appearance) => {
                    // Keep the tray glyph matching the panel (dark unless the
                    // scheme is explicitly light).
                    if let Some(tray) = &tray {
                        tray.set_panel_dark(appearance.color_scheme.as_deref() != Some("light"));
                    }
                    let accent = appearance.accent.map(|c| (c.r, c.g, c.b));
                    if this
                        .update(cx, |app, cx| {
                            app.theme = Rc::new(Theme::dark().with_system_accent(accent));
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                }
                Signal::Attention(lines, items, notices) => {
                    if this
                        .update(cx, |app, cx| {
                            let at = data::now_unix();
                            for notice in &notices {
                                let ctx = app.infer_log_context(&notice.title, Some(&notice.body));
                                app.activity_log.push_ctx(
                                    at,
                                    crate::activity_log::LogLevel::Info,
                                    format!("{} — {}", notice.title, notice.body),
                                    ctx,
                                );
                            }
                            app.attention = lines;
                            app.polled_inbox = Some(items);
                            // Fresh host facts → refresh the attention
                            // surfaces (badges, tray, urgent notifications).
                            app.recompute_attention();
                            // Piggyback the central CI pass on the poll's
                            // cadence (180s) — its own TTL (~5 min) makes
                            // most of these ticks a no-op.
                            app.refresh_ci(cx);
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                }
                Signal::ShowWindow => cx.update(|cx| cx.activate(true)),
                Signal::Quit => cx.update(|cx| cx.quit()),
            }
        }
    })
    .detach();

    (tray_for_app, watcher)
}
