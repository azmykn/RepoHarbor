//! Shared fleet / repo action menus — one canonical order for:
//! - Mission Control **Actions** gear
//! - Card / list / TREE right-click (single repo)
//! - Right-click on a repo inside a multi-selection (selection scope)

use gpui::{Entity, SharedString};
use gpui_component::menu::{PopupMenu, PopupMenuItem};

use crate::fleet::FleetOp;
use crate::shell::RepoHarborApp;

/// Capabilities derived from a target repo set (grid order ids).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FleetMenuCaps {
    pub has_dirty: bool,
    pub can_push: bool,
    pub has_submodules: bool,
}

/// Compute menu enablement from the live grid rows for `targets`.
pub(crate) fn fleet_menu_caps(app: &RepoHarborApp, targets: &[String]) -> FleetMenuCaps {
    let mut caps = FleetMenuCaps::default();
    for id in targets {
        let Some(row) = app.rows.iter().find(|r| r.id.as_ref() == id.as_str()) else {
            continue;
        };
        if row.dirty > 0 {
            caps.has_dirty = true;
        }
        if row.ahead > 0 && !app.is_pull_only(id) {
            caps.can_push = true;
        }
        if row.child_count > 0 {
            caps.has_submodules = true;
        }
    }
    caps
}

/// Options for [`fill_fleet_actions_menu`].
pub(crate) struct FleetMenuOpts {
    pub ai_ready: bool,
    pub idle: bool,
    pub caps: FleetMenuCaps,
    /// Show "Clear selection" (Actions gear / multi-select context).
    pub clear_selection: bool,
    /// Single-target forge URL → "Open on GitHub" etc.
    pub open_remote: Option<(SharedString, SharedString)>,
    /// Prepend "Open drawer" for this repo id (context menus).
    pub open_drawer: Option<SharedString>,
    /// Section label above the ops (e.g. `Selection (4)`).
    pub section_label: Option<String>,
}

/// Canonical action order (sync-first):
/// Open drawer / Open on host → Fetch / Pull → Stage / Commit / Generate / Push
/// → Update submodules → Discard / Prune / Reset → Open in IDE → Clear selection.
pub(crate) fn fill_fleet_actions_menu(
    menu: PopupMenu,
    app: Entity<RepoHarborApp>,
    repos: Vec<String>,
    opts: FleetMenuOpts,
) -> PopupMenu {
    if repos.is_empty() {
        return menu.label("Select repos first");
    }

    let on = opts.idle;
    let dirty_on = on && opts.caps.has_dirty;
    let push_on = on && opts.caps.can_push;
    let sub_on = on && opts.caps.has_submodules;
    let mut m = menu;

    if let Some(label) = opts.section_label {
        m = m.label(label);
    }

    let has_nav = opts.open_drawer.is_some() || opts.open_remote.is_some();

    if let Some(repo) = opts.open_drawer {
        let (a, id) = (app.clone(), repo);
        m = m.item(
            PopupMenuItem::new("Open drawer").on_click(move |_, window, cx| {
                a.update(cx, |this, cx| this.open_drawer(id.clone(), window, cx));
            }),
        );
    }
    if let Some((url, label)) = opts.open_remote {
        m = m.item(PopupMenuItem::new(label).on_click(move |_, _, _cx| {
            let _ = repoharbor_core::launch::open(&url);
        }));
    }
    if has_nav {
        m = m.separator();
    }

    // ── Sync ───────────────────────────────────────────────────────────────
    let (a, r) = (app.clone(), repos.clone());
    m = m.item(
        PopupMenuItem::new("Fetch")
            .disabled(!on)
            .on_click(move |_, _, cx| {
                a.update(cx, |this, cx| {
                    this.run_fleet_repos(FleetOp::Fetch, r.clone(), cx);
                });
            }),
    );
    let (a, r) = (app.clone(), repos.clone());
    m = m.item(
        PopupMenuItem::new("Pull")
            .disabled(!on)
            .on_click(move |_, _, cx| {
                a.update(cx, |this, cx| {
                    this.run_fleet_repos(FleetOp::Pull, r.clone(), cx);
                });
            }),
    );
    m = m.separator();

    // ── Local changes ──────────────────────────────────────────────────────
    let (a, r) = (app.clone(), repos.clone());
    m = m.item(
        PopupMenuItem::new("Stage all")
            .disabled(!dirty_on)
            .on_click(move |_, _, cx| {
                a.update(cx, |this, cx| {
                    this.run_fleet_repos(FleetOp::StageAll, r.clone(), cx);
                });
            }),
    );
    let (a, r) = (app.clone(), repos.clone());
    m = m.item(
        PopupMenuItem::new("Commit…")
            .disabled(!dirty_on)
            .on_click(move |_, window, cx| {
                a.update(cx, |this, cx| {
                    this.adopt_fleet_targets(&r);
                    this.start_fleet_commit(window, cx);
                });
            }),
    );
    if opts.ai_ready {
        let (a, r) = (app.clone(), repos.clone());
        m = m.item(
            PopupMenuItem::new("Generate…")
                .disabled(!dirty_on)
                .on_click(move |_, _, cx| {
                    a.update(cx, |this, cx| {
                        this.adopt_fleet_targets(&r);
                        this.prompt_generate_commit_selected(cx);
                    });
                }),
        );
    }
    let (a, r) = (app.clone(), repos.clone());
    m = m.item(
        PopupMenuItem::new("Push")
            .disabled(!push_on)
            .on_click(move |_, _, cx| {
                a.update(cx, |this, cx| {
                    this.run_fleet_repos(FleetOp::Push, r.clone(), cx);
                });
            }),
    );
    let (a, r) = (app.clone(), repos.clone());
    m = m.item(
        PopupMenuItem::new("Update submodules")
            .disabled(!sub_on)
            .on_click(move |_, _, cx| {
                a.update(cx, |this, cx| {
                    this.run_fleet_repos(FleetOp::SubmoduleUpdate, r.clone(), cx);
                });
            }),
    );
    m = m.separator();

    // ── Destructive ────────────────────────────────────────────────────────
    let (a, r) = (app.clone(), repos.clone());
    m = m.item(
        PopupMenuItem::new("Discard changes")
            .disabled(!dirty_on)
            .on_click(move |_, _, cx| {
                a.update(cx, |this, cx| {
                    this.adopt_fleet_targets(&r);
                    this.start_fleet_discard(cx);
                });
            }),
    );
    let (a, r) = (app.clone(), repos.clone());
    m = m.item(
        PopupMenuItem::new("Prune")
            .disabled(!on)
            .on_click(move |_, _, cx| {
                a.update(cx, |this, cx| {
                    this.adopt_fleet_targets(&r);
                    this.start_fleet_prune(cx);
                });
            }),
    );
    let (a, r) = (app.clone(), repos.clone());
    m = m.item(
        PopupMenuItem::new("Reset hard")
            .disabled(!on)
            .on_click(move |_, _, cx| {
                a.update(cx, |this, cx| {
                    this.adopt_fleet_targets(&r);
                    this.start_fleet_reset(cx);
                });
            }),
    );
    m = m.separator();

    // ── Launch / selection ─────────────────────────────────────────────────
    let (a, r) = (app.clone(), repos.clone());
    m = m.item(
        PopupMenuItem::new("Open in IDE")
            .disabled(!on)
            .on_click(move |_, _, cx| {
                a.update(cx, |this, cx| {
                    this.adopt_fleet_targets(&r);
                    this.launch_selected(cx);
                });
            }),
    );
    if opts.clear_selection {
        let a = app;
        m = m.item(
            PopupMenuItem::new("Clear selection")
                .disabled(!on)
                .on_click(move |_, _, cx| {
                    a.update(cx, |this, cx| this.clear_selection(cx));
                }),
        );
    }
    m
}
