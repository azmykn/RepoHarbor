//! Fleet operations UI (#100/#184): multi-select on the repo grid + the fleet
//! bar running bulk Fetch/Pull/Prune (and bulk "Open in IDE") through
//! `repoharbor_core::fleet`.
//!
//! Selection lives on [`RepoHarborApp::selected`] as repo ids. Changing Mission
//! Control filters (chip / root / language / group / TREE focus / saved view)
//! clears the selection so bulk ops can't accidentally target the wrong set.
//! The name search prunes selection to currently visible rows. Rescans (and
//! attention recomputes) also drop ids that left the visible set or vanished.
//! Fleet targets are always [`RepoHarborApp::selected_repos_ordered`] —
//! selected ∩ `visible_rows()` — so a stale off-filter id can never run.
//! One run at a time: [`RepoHarborApp::fleet_run`] carries the engine's cancel flag + live
//! counter and gates the bar's buttons while active. The engine fires progress
//! events on its worker threads; they're bridged over an `async-channel`
//! drained by one foreground task (the `live.rs` pattern) that keeps a keyed
//! Progress toast ("Pulling 12/40…") current. The completion resolves that
//! toast to an aggregate summary — "Pull succeeded" / "Pull failed" with
//! per-repo detail, Error (persists until clicked) when anything failed so a
//! 40-repo pull never fails silently — then rescans once so the
//! grid reflects the new state.
//!
//! Bulk **Prune** is confirm-gated (branch deletion is irreversible — the #173
//! pattern scaled up): the Prune button first scans the selection for prunable
//! branches on the background executor, then the bar expands into a confirm
//! strip with the per-repo breakdown ("repoharbor ×3 · zed ×2 · nothing to prune:
//! api"), and only an explicit Confirm click executes. The strip was chosen
//! over reusing the Cleanup view with a preselection because it keeps the
//! confirm adjacent to the button that armed it and adds no cross-view
//! navigation state; the pending plan ([`RepoHarborApp::fleet_prune`]) is dropped
//! by any selection change, Esc, Cancel, or another run starting, so a stale
//! plan can never execute. Execution re-derives what's prunable per repo
//! (`git_ops::prune_branches` re-checks), so even a racing external prune is
//! safe — the repo just reports "nothing prunable".

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui::{
    AppContext, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px, rgb,
};
use gpui_component::button::Button;
use gpui_component::menu::DropdownMenu;
use gpui_component::{IconName, Sizable};

use repoharbor_core::fleet::{self, FleetReport, Outcome};

use crate::icon::lucide;
use crate::shell::RepoHarborApp;
use crate::theme::Theme;
use crate::toast::ToastKind;

/// Failed repos shown in the resolution toast's detail before "+N more".
const MAX_FAILURES_SHOWN: usize = 4;
/// Longest per-repo failure reason (chars) before it's clipped.
const MAX_REASON_CHARS: usize = 60;
/// Per-repo entries shown in the prune confirm strip's breakdown before "+N more".
const MAX_BREAKDOWN_SHOWN: usize = 6;
/// Nothing-to-prune repo names listed in the breakdown before "+N more".
const MAX_SKIPPED_SHOWN: usize = 3;
/// Cap on bulk "Open in IDE" — spawning dozens of editor windows at once is a
/// footgun, so larger selections are refused with a toast instead.
pub const MAX_BULK_LAUNCH: usize = 10;

/// Selected repo ids that also appear in `visible` (indices into `rows`), in
/// visible/grid order. Shared by fleet ops and unit tests — the hard gate that
/// keeps Mission Control filters from leaking into bulk targets.
pub(crate) fn selected_visible_ordered(
    rows: &[crate::data::Row],
    selected: &std::collections::HashSet<SharedString>,
    visible: &[usize],
) -> Vec<String> {
    visible
        .iter()
        .filter_map(|&i| rows.get(i))
        .filter(|r| selected.contains(&r.id))
        .map(|r| r.id.to_string())
        .collect()
}

/// Submodule Update target set for the ops-row **Submodules** button.
///
/// - **Empty selection** → every *visible* parent with nested checkouts
///   (`child_count > 0`), so Mission Control filters still scope "الجميع".
/// - **Non-empty selection** → selected ∩ visible (same hard gate as other
///   fleet ops; the core skips repos without `.gitmodules`).
pub(crate) fn submodule_update_targets(
    rows: &[crate::data::Row],
    selected: &std::collections::HashSet<SharedString>,
    visible: &[usize],
) -> Vec<String> {
    if selected.is_empty() {
        visible
            .iter()
            .filter_map(|&i| rows.get(i))
            .filter(|r| r.child_count > 0)
            .map(|r| r.id.to_string())
            .collect()
    } else {
        selected_visible_ordered(rows, selected, visible)
    }
}

/// Which bulk operation the fleet bar runs.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FleetOp {
    Fetch,
    Pull,
    StageAll,
    Push,
    /// Init missing submodule checkouts, then pull each on its configured /
    /// current branch (not only the parent's recorded SHA).
    SubmoduleUpdate,
    /// Manual bulk commit — same user-typed message on every selected repo.
    /// Only started through the message strip ([`RepoHarborApp::confirm_fleet_commit`]).
    CommitAll,
    /// `git commit --allow-empty` with a default message (CI trigger).
    /// Skips pull-only paths (same as Push).
    EmptyCommit,
    /// Per-repo AI message only — no commit/push (toast + Log).
    GenerateMessageOnly,
    /// Per-repo AI message → `commit_all` → `push`. Multi-repo fleet runs
    /// start only through the confirm strip ([`RepoHarborApp::confirm_fleet_gen_push`]).
    GenerateCommitAndPush,
    /// Only ever started through the confirm strip
    /// ([`RepoHarborApp::confirm_fleet_prune`]) — never directly from a button.
    Prune,
    /// Only ever started through the confirm strip
    /// ([`RepoHarborApp::confirm_fleet_reset`]) — `git reset --hard @{upstream}`.
    ResetHard,
    /// Only ever started through the confirm strip
    /// ([`RepoHarborApp::confirm_fleet_discard`]) — discard uncommitted changes
    /// relative to HEAD (`reset --hard HEAD` + `clean -fd`). Keeps commits.
    DiscardChanges,
}

impl FleetOp {
    /// Present-progressive verb for the progress toast / bar counter.
    fn verb(self) -> &'static str {
        match self {
            FleetOp::Fetch => "Fetching",
            FleetOp::Pull => "Pulling",
            FleetOp::StageAll => "Staging",
            FleetOp::Push => "Pushing",
            FleetOp::SubmoduleUpdate => "Updating submodules",
            FleetOp::CommitAll => "Committing",
            FleetOp::EmptyCommit => "Creating empty commit",
            FleetOp::GenerateMessageOnly => "Generating",
            FleetOp::GenerateCommitAndPush => "Generating",
            FleetOp::Prune => "Pruning",
            FleetOp::ResetHard => "Resetting",
            FleetOp::DiscardChanges => "Discarding",
        }
    }

    /// Imperative name for the bar buttons and the summary toast.
    fn label(self) -> &'static str {
        match self {
            FleetOp::Fetch => "Fetch",
            FleetOp::Pull => "Pull",
            FleetOp::StageAll => "Stage all",
            FleetOp::Push => "Push",
            FleetOp::SubmoduleUpdate => "Update submodules",
            FleetOp::CommitAll => "Commit",
            FleetOp::EmptyCommit => "Empty commit",
            FleetOp::GenerateMessageOnly => "Generate message",
            FleetOp::GenerateCommitAndPush => "Generate, commit & push",
            FleetOp::Prune => "Prune",
            FleetOp::ResetHard => "Reset hard",
            FleetOp::DiscardChanges => "Discard changes",
        }
    }
}

/// Pending bulk-commit message prompt on the fleet bar. Dropped on selection
/// change / Esc / Cancel / another run starting.
pub struct CommitPlan {
    pub repos: Vec<String>,
    pub input: gpui::Entity<gpui_component::input::InputState>,
}

/// A pending bulk-prune confirm on the fleet bar (see the module docs for the
/// full flow). Dropped on any selection change / Esc / Cancel / run start.
pub enum PrunePlan {
    /// The background scan over the selection is in flight.
    Scanning,
    /// Scan done — the strip shows the breakdown and waits for the explicit
    /// Confirm click. `repos` is the full planned id set (grid order), so the
    /// run's aggregate toast accounts for every selected repo — the ones with
    /// nothing prunable report as engine skips.
    Ready {
        repos: Vec<String>,
        /// Per-repo prunable-branch counts (only repos with count > 0).
        entries: Vec<PruneEntry>,
        /// Display names of selected repos with nothing prunable.
        skipped: Vec<SharedString>,
    },
}

/// One row of the prune confirm breakdown: a repo and how many of its
/// branches would go.
pub struct PruneEntry {
    pub name: SharedString,
    pub count: usize,
}

/// An in-flight bulk run. `id` guards stale progress events (a late event
/// can't touch a later run's toast); `cancel` is the engine's flag — flipping
/// it lets in-flight git ops finish while everything not yet started skips.
pub struct FleetRun {
    pub id: u64,
    pub op: FleetOp,
    pub cancel: Arc<AtomicBool>,
    pub done: usize,
    pub total: usize,
}

impl RepoHarborApp {
    /// Toggle a repo in/out of the multi-selection (card checkbox, Ctrl+click).
    pub fn toggle_selected(&mut self, id: SharedString, cx: &mut Context<Self>) {
        // Editing the selection invalidates a pending prune/reset/commit confirm.
        self.fleet_prune = None;
        self.fleet_reset = None;
        self.fleet_discard = None;
        self.fleet_commit = None;
        self.fleet_gen_push = None;
        if !self.selected.remove(&id) {
            self.selected.insert(id);
        }
        cx.notify();
    }

    /// Clear the multi-selection (fleet bar "Clear", or Esc with no overlay).
    pub fn clear_selection(&mut self, cx: &mut Context<Self>) {
        if !self.selected.is_empty()
            || self.fleet_prune.is_some()
            || self.fleet_reset.is_some()
            || self.fleet_discard.is_some()
            || self.fleet_commit.is_some()
            || self.fleet_gen_push.is_some()
        {
            self.clear_selection_quiet();
            cx.notify();
        }
    }

    /// Drop selection + pending fleet confirms without notifying — used when a
    /// Mission Control filter changes and the caller will `cx.notify()` itself.
    pub(crate) fn clear_selection_quiet(&mut self) {
        self.selected.clear();
        self.fleet_prune = None;
        self.fleet_reset = None;
        self.fleet_discard = None;
        self.fleet_commit = None;
        self.fleet_gen_push = None;
    }

    /// True when every currently visible/filtered row is in the selection
    /// (and there is at least one). Drives the Mission Control select-all checkbox.
    pub fn all_visible_selected(&self) -> bool {
        let visible = self.visible_rows();
        !visible.is_empty()
            && visible
                .iter()
                .all(|&i| self.selected.contains(&self.rows[i].id))
    }

    /// Select every row passing the current filters. Drops any previously
    /// selected ids that are no longer visible, then adds every visible row —
    /// so select-all never re-introduces off-filter targets.
    pub fn select_all_visible(&mut self, cx: &mut Context<Self>) {
        self.fleet_prune = None;
        self.fleet_reset = None;
        self.fleet_discard = None;
        self.fleet_commit = None;
        self.fleet_gen_push = None;
        self.prune_selection_to_visible();
        for i in self.visible_rows() {
            let id = self.rows[i].id.clone();
            self.selected.insert(id);
        }
        cx.notify();
    }

    /// Checkbox toggle: select all visible repos, or clear the selection when
    /// they are already all selected.
    pub fn toggle_select_all_visible(&mut self, cx: &mut Context<Self>) {
        if self.all_visible_selected() {
            self.clear_selection(cx);
        } else {
            self.select_all_visible(cx);
        }
    }

    /// Replace the multi-selection with `repos` so selection-scoped fleet
    /// starters (commit / prune / reset / launch) target an explicit set —
    /// used by the Actions menu and right-click when the target isn't already
    /// the current selection.
    pub(crate) fn adopt_fleet_targets(&mut self, repos: &[String]) {
        self.selected = repos.iter().cloned().map(SharedString::from).collect();
    }

    /// True when a fleet run or confirm strip is armed — the slim bottom bar
    /// only paints in that case (actions live in the top Actions menu).
    pub(crate) fn fleet_strip_active(&self) -> bool {
        self.fleet_run.is_some()
            || self.fleet_prune.is_some()
            || self.fleet_reset.is_some()
            || self.fleet_discard.is_some()
            || self.fleet_commit.is_some()
            || self.fleet_gen_push.is_some()
    }

    /// Idle = no run and no confirm strip (Actions menu items enabled).
    pub(crate) fn fleet_actions_idle(&self) -> bool {
        self.fleet_run.is_none()
            && self.fleet_prune.is_none()
            && self.fleet_reset.is_none()
            && self.fleet_discard.is_none()
            && self.fleet_commit.is_none()
            && self.fleet_gen_push.is_none()
    }

    /// Replace the selection with the *currently visible* repos matching
    /// `pred` — the palette's select-by-filter verbs ("Select dirty" /
    /// "Select behind"). Scoped to `visible_rows()` so a later fleet op can't
    /// hit off-filter matches. When nothing matches, the existing selection is
    /// kept and an Info toast explains why (`what` reads as "No repos are
    /// {what}.").
    pub fn select_where(
        &mut self,
        pred: impl Fn(&crate::data::Row) -> bool,
        what: &str,
        cx: &mut Context<Self>,
    ) {
        let matched: std::collections::HashSet<SharedString> = self
            .visible_rows()
            .into_iter()
            .filter(|&i| pred(&self.rows[i]))
            .map(|i| self.rows[i].id.clone())
            .collect();
        if matched.is_empty() {
            self.push_toast(
                ToastKind::Info,
                "Nothing selected",
                Some(format!("No repos are {what}.").into()),
                cx,
            );
            return;
        }
        self.fleet_prune = None;
        self.fleet_reset = None;
        self.fleet_discard = None;
        self.fleet_commit = None;
        self.fleet_gen_push = None;
        self.selected = matched;
        cx.notify();
    }

    /// Drop selected ids that vanished or left the current Mission Control
    /// visible set. Called after rescans / attention recomputes / search edits
    /// so the "N selected" badge and fleet ops never include hidden repos.
    pub fn prune_selection(&mut self) {
        self.prune_selection_to_visible();
    }

    /// Retain only selected ids that currently pass `visible_rows()`. Clears
    /// pending fleet confirms when the set shrinks.
    pub(crate) fn prune_selection_to_visible(&mut self) {
        if self.selected.is_empty() {
            return;
        }
        let visible_ids: std::collections::HashSet<SharedString> = self
            .visible_rows()
            .into_iter()
            .map(|i| self.rows[i].id.clone())
            .collect();
        let before = self.selected.len();
        self.selected.retain(|id| visible_ids.contains(id));
        if self.selected.len() != before {
            self.fleet_prune = None;
            self.fleet_reset = None;
            self.fleet_discard = None;
            self.fleet_commit = None;
            self.fleet_gen_push = None;
        }
    }

    /// Flip the active run's cancel flag (fleet bar "Cancel"). In-flight git
    /// ops finish; repos not yet started report as skipped.
    pub fn cancel_fleet(&mut self, cx: &mut Context<Self>) {
        if let Some(run) = &self.fleet_run {
            run.cancel.store(true, Ordering::SeqCst);
            cx.notify();
        }
    }

    /// Run `op` across the selected *visible* repos (see [`Self::run_fleet_repos`]).
    pub fn run_fleet(&mut self, op: FleetOp, cx: &mut Context<Self>) {
        let repos = self.selected_repos_ordered();
        self.run_fleet_repos(op, repos, cx);
    }

    /// Update submodules for the current scope — selected ∩ visible when
    /// something is checked; otherwise every visible parent with nested
    /// checkouts (ops-row **Submodules** with an empty selection).
    pub fn run_submodule_update(&mut self, cx: &mut Context<Self>) {
        let visible = self.visible_rows();
        let empty_selection = self.selected.is_empty();
        let repos = submodule_update_targets(&self.rows, &self.selected, &visible);
        if repos.is_empty() {
            let detail = if empty_selection {
                "No visible repos declare nested checkouts."
            } else {
                "Nothing selected is visible under the current filters."
            };
            self.push_toast(
                ToastKind::Info,
                "No submodules to update",
                Some(detail.into()),
                cx,
            );
            return;
        }
        // Empty→all-visible cue: name the scope before the progress toast.
        if empty_selection {
            let n = repos.len();
            self.push_toast(
                ToastKind::Info,
                format!(
                    "Updating {n} visible {}",
                    if n == 1 { "parent" } else { "parents" }
                ),
                Some("No selection — every visible parent with nested checkouts.".into()),
                cx,
            );
        }
        self.run_fleet_repos(FleetOp::SubmoduleUpdate, repos, cx);
    }

    /// Run `op` across `repos` on the background executor (one bulk run at a
    /// time). Shared by the fleet bar (selection) and the palette's fleet
    /// verbs ("Fetch all", "Pull all behind" — no selection needed). Progress
    /// marshals onto the foreground via a channel and keeps a keyed Progress
    /// toast current; completion resolves the toast to the aggregate summary
    /// and rescans once.
    ///
    /// `commit_message` is required for [`FleetOp::CommitAll`] (one shared
    /// message). Generate ops ignore it and draft a fresh AI message per repo.
    pub fn run_fleet_repos(&mut self, op: FleetOp, repos: Vec<String>, cx: &mut Context<Self>) {
        self.run_fleet_repos_msg(op, repos, None, cx);
    }

    pub fn run_fleet_repos_msg(
        &mut self,
        op: FleetOp,
        mut repos: Vec<String>,
        commit_message: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if self.fleet_run.is_some() || repos.is_empty() {
            return;
        }
        // Empty commit and Push stay off vendor / pull-only trees. Mixed
        // selections drop those paths with an Info toast; all-pull-only → Error.
        if matches!(op, FleetOp::EmptyCommit | FleetOp::Push) {
            let prefixes = &self.config.pull_only_prefixes;
            let before = repos.len();
            repos.retain(|r| !repoharbor_core::model::path_is_pull_only(r, prefixes));
            let blocked = before - repos.len();
            let (blocked_title, blocked_detail, skip_detail) = match op {
                FleetOp::Push => (
                    "Push blocked",
                    "Selected repos are pull-only (upstream / vendor). Push is disabled.",
                    "skipped — push runs only on digits / pushable paths.",
                ),
                _ => (
                    "Empty commit blocked",
                    "Selected repos are pull-only (upstream / vendor). Empty commit is disabled.",
                    "skipped — empty commit runs only on digits / pushable paths.",
                ),
            };
            if repos.is_empty() {
                self.push_toast(
                    ToastKind::Error,
                    blocked_title,
                    Some(blocked_detail.into()),
                    cx,
                );
                return;
            }
            if blocked > 0 {
                self.push_toast(
                    ToastKind::Info,
                    "Skipped pull-only",
                    Some(
                        format!(
                            "{blocked} {} {skip_detail}",
                            if blocked == 1 { "repo" } else { "repos" }
                        )
                        .into(),
                    ),
                    cx,
                );
            }
        }
        if matches!(op, FleetOp::CommitAll)
            && commit_message
                .as_deref()
                .map(str::trim)
                .is_none_or(|m| m.is_empty())
        {
            self.push_toast(
                ToastKind::Error,
                "Commit message required",
                Some("Enter a message before committing the selection.".into()),
                cx,
            );
            return;
        }
        // Starting any run invalidates a pending prune/reset/commit confirm.
        self.fleet_prune = None;
        self.fleet_reset = None;
        self.fleet_discard = None;
        self.fleet_commit = None;
        self.fleet_gen_push = None;
        let total = repos.len();
        self.fleet_seq += 1;
        let run_id = self.fleet_seq;
        let cancel = Arc::new(AtomicBool::new(false));
        self.fleet_run = Some(FleetRun {
            id: run_id,
            op,
            cancel: cancel.clone(),
            done: 0,
            total,
        });
        let key = SharedString::from(format!("fleet:{run_id}"));
        self.upsert_toast(
            key.clone(),
            ToastKind::Progress,
            format!("{} 0/{total}…", op.verb()),
            None,
            cx,
        );

        // Progress events fire on the engine's worker threads; bridge them over
        // a channel drained by one foreground task (the live.rs pattern).
        let (tx, rx) = async_channel::unbounded::<fleet::FleetEvent>();
        {
            let key = key.clone();
            cx.spawn(async move |this, cx| {
                while let Ok(ev) = rx.recv().await {
                    let applied = this.update(cx, |this, cx| {
                        // Only the still-active run updates the toast: a stale
                        // queued event must not overwrite the resolution.
                        let verb = match &mut this.fleet_run {
                            Some(run) if run.id == run_id => {
                                run.done = ev.done;
                                run.op.verb()
                            }
                            _ => return,
                        };
                        this.upsert_toast(
                            key.clone(),
                            ToastKind::Progress,
                            format!("{verb} {}/{}…", ev.done, ev.total),
                            None,
                            cx,
                        );
                    });
                    if applied.is_err() {
                        break;
                    }
                }
            })
            .detach();
        }

        // Generate & commit hits the AI per repo — keep concurrency low so we
        // don't stampede Ollama/llama.cpp.
        let workers = match op {
            FleetOp::GenerateMessageOnly | FleetOp::GenerateCommitAndPush => {
                2.min(fleet::default_workers())
            }
            _ => fleet::default_workers(),
        };
        let pull_only_prefixes = self.config.pull_only_prefixes.clone();

        let pull_files =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::<(String, Vec<String>)>::new()));
        let pull_files_for_op = pull_files.clone();
        let pull_files_for_ui = pull_files.clone();

        cx.spawn(async move |this, cx| {
            let report = cx
                .background_executor()
                .spawn(async move {
                    let progress = move |ev: fleet::FleetEvent| {
                        let _ = tx.try_send(ev);
                        // `tx` (owned by this closure) drops when the engine
                        // returns, closing the channel and ending the drain.
                    };
                    match op {
                        FleetOp::Fetch => {
                            fleet::run(&repos, workers, &cancel, progress, fleet::fetch_op())
                        }
                        FleetOp::Pull => fleet::run(
                            &repos,
                            workers,
                            &cancel,
                            progress,
                            fleet::pull_op_collecting(
                                Some(pull_files_for_op),
                                pull_only_prefixes.clone(),
                            ),
                        ),
                        FleetOp::StageAll => {
                            fleet::run(&repos, workers, &cancel, progress, fleet::stage_all_op())
                        }
                        FleetOp::Push => {
                            fleet::run(&repos, workers, &cancel, progress, fleet::push_op())
                        }
                        FleetOp::SubmoduleUpdate => fleet::run(
                            &repos,
                            workers,
                            &cancel,
                            progress,
                            fleet::submodule_update_op(),
                        ),
                        FleetOp::CommitAll => {
                            let msg = commit_message.unwrap_or_default();
                            fleet::run(
                                &repos,
                                workers,
                                &cancel,
                                progress,
                                fleet::commit_all_op(msg),
                            )
                        }
                        FleetOp::EmptyCommit => {
                            let msg = commit_message
                                .filter(|m| !m.trim().is_empty())
                                .unwrap_or_else(|| "Empty commit".into());
                            fleet::run(
                                &repos,
                                workers,
                                &cancel,
                                progress,
                                fleet::empty_commit_op(msg),
                            )
                        }
                        FleetOp::GenerateMessageOnly => fleet::run(
                            &repos,
                            workers,
                            &cancel,
                            progress,
                            generate_message_only_op(),
                        ),
                        FleetOp::GenerateCommitAndPush => fleet::run(
                            &repos,
                            workers,
                            &cancel,
                            progress,
                            generate_commit_and_push_op(),
                        ),
                        FleetOp::Prune => {
                            fleet::run(&repos, workers, &cancel, progress, fleet::prune_op())
                        }
                        FleetOp::ResetHard => {
                            fleet::run(&repos, workers, &cancel, progress, fleet::reset_hard_op())
                        }
                        FleetOp::DiscardChanges => fleet::run(
                            &repos,
                            workers,
                            &cancel,
                            progress,
                            fleet::discard_changes_op(),
                        ),
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.fleet_run = None;
                if matches!(op, FleetOp::Pull) {
                    let collected = pull_files_for_ui
                        .lock()
                        .map(|g| g.clone())
                        .unwrap_or_default();
                    this.apply_last_pull(collected, cx);
                }
                let (kind, title, detail) = resolve_toast(op, &report);
                let ctx = fleet_log_context(this, &report);
                this.upsert_toast_ctx(key, kind, title, detail, ctx, cx);
                // One rescan for the whole run (not per repo) so the grid
                // reflects the new ahead/behind/dirty state.
                this.rescan(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Open the fleet-bar commit-message strip for the current selection.
    /// Nothing is committed until [`Self::confirm_fleet_commit`].
    pub fn start_fleet_commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.fleet_run.is_some()
            || self.fleet_commit.is_some()
            || self.fleet_prune.is_some()
            || self.fleet_reset.is_some()
            || self.fleet_discard.is_some()
            || self.fleet_gen_push.is_some()
        {
            return;
        }
        let repos = self.selected_repos_ordered();
        if repos.is_empty() {
            return;
        }
        let n = repos.len();
        let input = cx.new(|cx| {
            gpui_component::input::InputState::new(window, cx).placeholder(format!(
                "Commit message for {n} {}",
                if n == 1 { "repo" } else { "repos" }
            ))
        });
        self.fleet_commit = Some(CommitPlan { repos, input });
        cx.notify();
    }

    /// Commit the planned selection with the strip's message (one shared text).
    pub fn confirm_fleet_commit(&mut self, cx: &mut Context<Self>) {
        let Some(plan) = self.fleet_commit.take() else {
            return;
        };
        let message = plan.input.read(cx).value().to_string();
        if message.trim().is_empty() {
            self.fleet_commit = Some(plan);
            self.push_toast(
                ToastKind::Error,
                "Commit message required",
                Some("Enter a message before committing the selection.".into()),
                cx,
            );
            return;
        }
        self.run_fleet_repos_msg(FleetOp::CommitAll, plan.repos, Some(message), cx);
    }

    pub fn cancel_fleet_commit(&mut self, cx: &mut Context<Self>) {
        if self.fleet_commit.take().is_some() {
            cx.notify();
        }
    }

    /// Arm a Gen & push confirm for multiple dirty repos. Single-repo Gen &
    /// push stays one-click via [`Self::run_generate_commit`]. Nothing runs
    /// until [`Self::confirm_fleet_gen_push`].
    pub fn start_fleet_gen_push(&mut self, repos: Vec<String>, cx: &mut Context<Self>) {
        if self.fleet_run.is_some()
            || self.fleet_gen_push.is_some()
            || self.fleet_commit.is_some()
            || self.fleet_prune.is_some()
            || self.fleet_reset.is_some()
            || self.fleet_discard.is_some()
        {
            return;
        }
        if repos.len() < 2 {
            return;
        }
        self.fleet_gen_push = Some(repos);
        cx.notify();
    }

    /// Confirm strip → AI message → commit all → push for the planned repos.
    pub fn confirm_fleet_gen_push(&mut self, cx: &mut Context<Self>) {
        let Some(repos) = self.fleet_gen_push.take() else {
            return;
        };
        self.run_fleet_repos(FleetOp::GenerateCommitAndPush, repos, cx);
    }

    /// Drop the pending Gen & push confirm (strip Cancel, or Esc).
    pub fn cancel_fleet_gen_push(&mut self, cx: &mut Context<Self>) {
        if self.fleet_gen_push.take().is_some() {
            cx.notify();
        }
    }

    /// The fleet bar's Prune button: scan the selected repos for prunable
    /// branches on the background executor (the Cleanup view's scan), then
    /// expand the bar into the confirm strip with the per-repo breakdown.
    /// Nothing is deleted here — only [`Self::confirm_fleet_prune`] executes.
    pub fn start_fleet_prune(&mut self, cx: &mut Context<Self>) {
        if self.fleet_run.is_some()
            || self.fleet_prune.is_some()
            || self.fleet_reset.is_some()
            || self.fleet_discard.is_some()
            || self.fleet_commit.is_some()
            || self.fleet_gen_push.is_some()
        {
            return;
        }
        // Visible selection order, so the breakdown matches the grid.
        let targets = self.selected_repos_ordered();
        let rows: Vec<crate::data::Row> = targets
            .iter()
            .filter_map(|id| {
                self.rows
                    .iter()
                    .find(|r| r.id.as_ref() == id.as_str())
                    .cloned()
            })
            .collect();
        if rows.is_empty() {
            return;
        }
        let selected = rows.len();
        self.fleet_prune_seq += 1;
        let seq = self.fleet_prune_seq;
        self.fleet_prune = Some(PrunePlan::Scanning);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let scanned = cx
                .background_executor()
                .spawn(async move {
                    let prunable = crate::views::cleanup::scan(&rows);
                    (rows, prunable)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                // Apply only if this scan's confirm is still the pending one —
                // not cancelled, superseded, or invalidated by a selection edit.
                if this.fleet_prune_seq != seq
                    || !matches!(this.fleet_prune, Some(PrunePlan::Scanning))
                {
                    return;
                }
                let (rows, prunable) = scanned;
                let entries: Vec<PruneEntry> = prunable
                    .iter()
                    .map(|r| PruneEntry {
                        name: r.name.clone(),
                        count: r.branches.len(),
                    })
                    .collect();
                if entries.is_empty() {
                    this.fleet_prune = None;
                    this.push_toast(
                        ToastKind::Info,
                        "Nothing to prune",
                        Some(
                            format!("No prunable branches across {selected} selected repos.")
                                .into(),
                        ),
                        cx,
                    );
                } else {
                    let has: std::collections::HashSet<&SharedString> =
                        prunable.iter().map(|r| &r.id).collect();
                    let skipped: Vec<SharedString> = rows
                        .iter()
                        .filter(|r| !has.contains(&r.id))
                        .map(|r| r.name.clone())
                        .collect();
                    this.fleet_prune = Some(PrunePlan::Ready {
                        repos: rows.iter().map(|r| r.id.to_string()).collect(),
                        entries,
                        skipped,
                    });
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The confirm strip's Confirm click — the only path that executes a bulk
    /// prune. Runs over the full planned selection; repos with nothing
    /// prunable report as engine skips in the aggregate toast.
    pub fn confirm_fleet_prune(&mut self, cx: &mut Context<Self>) {
        let Some(PrunePlan::Ready { repos, .. }) = self.fleet_prune.take() else {
            return;
        };
        self.run_fleet_repos(FleetOp::Prune, repos, cx);
    }

    /// Drop the pending prune confirm (strip Cancel, or Esc).
    pub fn cancel_fleet_prune(&mut self, cx: &mut Context<Self>) {
        if self.fleet_prune.take().is_some() {
            cx.notify();
        }
    }

    /// Arm a hard-reset confirm for the current selection (`git reset --hard
    /// @{upstream}` per repo). Nothing is reset until [`Self::confirm_fleet_reset`].
    pub fn start_fleet_reset(&mut self, cx: &mut Context<Self>) {
        if self.fleet_run.is_some()
            || self.fleet_reset.is_some()
            || self.fleet_discard.is_some()
            || self.fleet_prune.is_some()
            || self.fleet_commit.is_some()
            || self.fleet_gen_push.is_some()
        {
            return;
        }
        let repos = self.selected_repos_ordered();
        if repos.is_empty() {
            return;
        }
        self.fleet_reset = Some(repos);
        cx.notify();
    }

    /// Execute the armed hard reset across the planned repos.
    pub fn confirm_fleet_reset(&mut self, cx: &mut Context<Self>) {
        let Some(repos) = self.fleet_reset.take() else {
            return;
        };
        self.run_fleet_repos(FleetOp::ResetHard, repos, cx);
    }

    /// Dismiss a pending hard-reset confirm without running it.
    pub fn cancel_fleet_reset(&mut self, cx: &mut Context<Self>) {
        if self.fleet_reset.take().is_some() {
            cx.notify();
        }
    }

    /// Arm a discard-changes confirm for the current selection (`reset --hard
    /// HEAD` + `clean -fd` per dirty repo). Nothing is discarded until
    /// [`Self::confirm_fleet_discard`].
    pub fn start_fleet_discard(&mut self, cx: &mut Context<Self>) {
        let selected = self.selected_repos_ordered();
        let repos: Vec<String> = selected
            .into_iter()
            .filter(|id| {
                self.rows
                    .iter()
                    .any(|r| r.id.as_ref() == id.as_str() && r.dirty > 0)
            })
            .collect();
        self.start_fleet_discard_repos(repos, cx);
    }

    /// Arm discard for an explicit repo list (fleet bar selection or a single
    /// dirty repo from the context menu).
    pub fn start_fleet_discard_repos(&mut self, repos: Vec<String>, cx: &mut Context<Self>) {
        if self.fleet_run.is_some()
            || self.fleet_discard.is_some()
            || self.fleet_reset.is_some()
            || self.fleet_prune.is_some()
            || self.fleet_commit.is_some()
            || self.fleet_gen_push.is_some()
        {
            return;
        }
        if repos.is_empty() {
            return;
        }
        self.fleet_discard = Some(repos);
        cx.notify();
    }

    /// Execute the armed discard across the planned repos.
    pub fn confirm_fleet_discard(&mut self, cx: &mut Context<Self>) {
        let Some(repos) = self.fleet_discard.take() else {
            return;
        };
        self.run_fleet_repos(FleetOp::DiscardChanges, repos, cx);
    }

    /// Dismiss a pending discard confirm without running it.
    pub fn cancel_fleet_discard(&mut self, cx: &mut Context<Self>) {
        if self.fleet_discard.take().is_some() {
            cx.notify();
        }
    }

    /// Bulk "Open in IDE": launch the configured editor on every selected
    /// repo (the card's launch path). Selections over [`MAX_BULK_LAUNCH`] are
    /// refused with a toast asking to narrow — spawning dozens of editor
    /// windows at once is a footgun.
    pub fn launch_selected(&mut self, cx: &mut Context<Self>) {
        let ids = self.selected_repos_ordered();
        if ids.is_empty() {
            return;
        }
        if ids.len() > MAX_BULK_LAUNCH {
            self.push_toast(
                ToastKind::Error,
                "Too many repos to open",
                Some(
                    format!(
                        "{} selected — narrow the selection to {MAX_BULK_LAUNCH} or fewer \
                         before opening in the IDE.",
                        ids.len()
                    )
                    .into(),
                ),
                cx,
            );
            return;
        }
        let total = ids.len();
        let failed = ids
            .iter()
            .filter(|id| repoharbor_core::launch::launch(&self.config.ide_command, id).is_err())
            .count();
        if failed > 0 {
            self.push_toast(
                ToastKind::Error,
                "Some IDE launches failed",
                Some(format!("{failed} of {total} repos failed to open.").into()),
                cx,
            );
        } else {
            self.push_toast(
                ToastKind::Success,
                format!(
                    "Opened {total} {} in the IDE",
                    if total == 1 { "repo" } else { "repos" }
                ),
                None,
                cx,
            );
        }
    }

    /// Slim bottom strip: in-flight progress / Cancel, plus confirm strips for
    /// commit / Gen & push / discard / reset / prune. Bulk action buttons live
    /// in the top Actions dropdown (and the repo context menu) — not here.
    pub fn fleet_bar(&self, t: &Theme, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if !self.fleet_strip_active() {
            return None;
        }
        let mut bar = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.))
            .px(px(16.))
            .py(px(10.))
            .border_t_1()
            .border_color(rgb(t.border))
            .bg(rgb(t.surface));
        let selected_n = self.selected_repos_ordered().len();
        if selected_n > 0 {
            bar = bar.child(lucide("check", 15., t.accent_bright)).child(
                div()
                    .font_weight(FontWeight::MEDIUM)
                    .text_size(px(t.text_small))
                    .text_color(rgb(t.fg0))
                    .child(SharedString::from(format!("{selected_n} selected"))),
            );
        }
        if let Some(run) = &self.fleet_run {
            bar = bar
                .child(
                    div()
                        .font_family("monospace")
                        .text_size(px(t.text_data_sm))
                        .text_color(rgb(t.fg2))
                        .child(SharedString::from(format!(
                            "{} {}/{}…",
                            run.op.verb(),
                            run.done,
                            run.total
                        ))),
                )
                .child(bar_btn(
                    "fleet-cancel",
                    "x",
                    "Cancel",
                    true,
                    true,
                    t,
                    cx.listener(|this, _e, _w, cx| this.cancel_fleet(cx)),
                ));
        }
        // A pending commit-message strip expands above the status row.
        if let Some(plan) = &self.fleet_commit {
            let n = plan.repos.len();
            let input = plan.input.clone();
            let strip = div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.))
                .px(px(16.))
                .py(px(10.))
                .border_t_1()
                .border_color(rgb(t.border))
                .bg(rgb(t.surface))
                .child(lucide("git-commit", 15., t.accent_bright))
                .child(
                    div()
                        .text_size(px(t.text_small))
                        .text_color(rgb(t.fg1))
                        .child(SharedString::from(format!(
                            "Message for {n} {}",
                            if n == 1 { "repo" } else { "repos" }
                        ))),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(120.))
                        .child(gpui_component::input::Input::new(&input)),
                )
                .child(bar_btn(
                    "fleet-commit-confirm",
                    "git-commit",
                    "Commit",
                    true,
                    false,
                    t,
                    cx.listener(|this, _e, _w, cx| this.confirm_fleet_commit(cx)),
                ))
                .child(bar_btn(
                    "fleet-commit-cancel",
                    "x",
                    "Cancel",
                    true,
                    false,
                    t,
                    cx.listener(|this, _e, _w, cx| this.cancel_fleet_commit(cx)),
                ));
            return Some(
                div()
                    .flex()
                    .flex_col()
                    .child(strip)
                    .child(bar)
                    .into_any_element(),
            );
        }
        // Multi-repo Gen & push: list targets + commit-all+push warning.
        if let Some(repos) = &self.fleet_gen_push {
            let n = repos.len();
            let names = gen_push_name_list(self, repos);
            let strip = div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.))
                .px(px(16.))
                .py(px(10.))
                .border_t_1()
                .border_color(rgb(t.border_accent))
                .bg(rgb(t.accent_wash))
                .child(lucide("sparkles", 15., t.accent_bright))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(t.text_small))
                        .text_color(rgb(t.fg0))
                        .child(SharedString::from(format!(
                            "Gen & push {n} repos — AI message, commit all, then push: {names}"
                        ))),
                )
                .child(bar_btn(
                    "fleet-gen-push-confirm",
                    "sparkles",
                    "Confirm Gen & push",
                    true,
                    false,
                    t,
                    cx.listener(|this, _e, _w, cx| this.confirm_fleet_gen_push(cx)),
                ))
                .child(bar_btn(
                    "fleet-gen-push-cancel",
                    "x",
                    "Cancel",
                    true,
                    false,
                    t,
                    cx.listener(|this, _e, _w, cx| this.cancel_fleet_gen_push(cx)),
                ));
            return Some(
                div()
                    .flex()
                    .flex_col()
                    .child(strip)
                    .child(bar)
                    .into_any_element(),
            );
        }
        // A pending discard confirm expands into a danger strip.
        if let Some(repos) = &self.fleet_discard {
            let n = repos.len();
            let strip = div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.))
                .px(px(16.))
                .py(px(10.))
                .border_t_1()
                .border_color(rgb(t.behind))
                .bg(rgb(t.danger_badge))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(t.text_small))
                        .text_color(rgb(t.fg0))
                        .child(SharedString::from(format!(
                            "Discard all uncommitted changes in {n} repo(s)? Resets tracked files to HEAD and removes untracked files. This cannot be undone."
                        ))),
                )
                .child(bar_btn(
                    "fleet-discard-confirm",
                    "trash-2",
                    "Confirm discard",
                    true,
                    true,
                    t,
                    cx.listener(|this, _e, _w, cx| this.confirm_fleet_discard(cx)),
                ))
                .child(bar_btn(
                    "fleet-discard-cancel",
                    "x",
                    "Cancel",
                    true,
                    false,
                    t,
                    cx.listener(|this, _e, _w, cx| this.cancel_fleet_discard(cx)),
                ));
            return Some(
                div()
                    .flex()
                    .flex_col()
                    .child(strip)
                    .child(bar)
                    .into_any_element(),
            );
        }
        // A pending reset confirm expands into a danger strip.
        if let Some(repos) = &self.fleet_reset {
            let n = repos.len();
            let strip = div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.))
                .px(px(16.))
                .py(px(10.))
                .border_t_1()
                .border_color(rgb(t.behind))
                .bg(rgb(t.danger_badge))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(t.text_small))
                        .text_color(rgb(t.fg0))
                        .child(SharedString::from(format!(
                            "Reset {n} repo(s) to origin/<branch>? Discards local commits and uncommitted changes."
                        ))),
                )
                .child(bar_btn(
                    "fleet-reset-confirm",
                    "history",
                    "Confirm reset",
                    true,
                    true,
                    t,
                    cx.listener(|this, _e, _w, cx| this.confirm_fleet_reset(cx)),
                ))
                .child(bar_btn(
                    "fleet-reset-cancel",
                    "x",
                    "Cancel",
                    true,
                    false,
                    t,
                    cx.listener(|this, _e, _w, cx| this.cancel_fleet_reset(cx)),
                ));
            return Some(
                div()
                    .flex()
                    .flex_col()
                    .child(strip)
                    .child(bar)
                    .into_any_element(),
            );
        }
        // A pending prune confirm expands into a strip with Confirm/Cancel.
        if let Some(plan) = &self.fleet_prune {
            return Some(
                div()
                    .flex()
                    .flex_col()
                    .child(self.prune_strip(plan, t, cx))
                    .child(bar)
                    .into_any_element(),
            );
        }
        Some(bar.into_any_element())
    }

    /// Selected *and currently visible* repo ids in grid/visible order.
    /// Hard gate for every selection-scoped fleet op, Actions menu, and badge.
    pub(crate) fn selected_repos_ordered(&self) -> Vec<String> {
        let visible = self.visible_rows();
        selected_visible_ordered(&self.rows, &self.selected, &visible)
    }

    /// Compact selection-scoped primaries beside Actions ▾:
    /// Fetch / Pull / Push / Gen only / Gen & push / [Empty commit].
    /// (**Submodules** lives on the always-on ops row next to Pull behind —
    /// empty selection updates every visible submodule parent.)
    /// Push and the Gen pair stay on the bar for every selection (a Needs-me
    /// mix of vendor + digits must not drop Gen). Push enables only when a
    /// non–pull-only path is ahead; Empty commit only when a non–pull-only
    /// path is selected. Gen buttons dim unless something is dirty and AI is
    /// ready. Gen & push on N>1 arms a confirm strip first.
    pub fn fleet_primary_sync_buttons(
        &self,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let idle = self.fleet_actions_idle();
        let ai_ready = self.services.ai_ready;
        let repos = self.selected_repos_ordered();
        let caps = crate::menu_actions::fleet_menu_caps(self, &repos);
        let mut row = div()
            .id("mc-fleet-primaries")
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.));
        row = row.child(bar_btn(
            "mc-fleet-fetch",
            "refresh-cw",
            "Fetch",
            idle,
            false,
            t,
            cx.listener(|this, _e, _w, cx| {
                let repos = this.selected_repos_ordered();
                this.run_fleet_repos(FleetOp::Fetch, repos, cx);
            }),
        ));
        row = row.child(bar_btn(
            "mc-fleet-pull",
            "arrow-down",
            "Pull",
            idle,
            false,
            t,
            cx.listener(|this, _e, _w, cx| {
                let repos = this.selected_repos_ordered();
                this.run_fleet_repos(FleetOp::Pull, repos, cx);
            }),
        ));
        let push_enabled = idle && caps.can_push;
        row = row.child(bar_btn_explain(
            "mc-fleet-push",
            "arrow-up",
            "Push",
            push_enabled,
            t,
            cx.listener(move |this, _e, _w, cx| {
                let repos = this.selected_repos_ordered();
                let caps = crate::menu_actions::fleet_menu_caps(this, &repos);
                if !(this.fleet_actions_idle() && caps.can_push) {
                    this.push_toast(
                        crate::toast::ToastKind::Info,
                        "Nothing to push",
                        Some(
                            "Push enables when a selected non–pull-only repo has commits that are not on its upstream."
                                .into(),
                        ),
                        cx,
                    );
                    return;
                }
                this.run_fleet_repos(FleetOp::Push, repos, cx);
            }),
        ));
        let gen_enabled = idle && ai_ready && caps.has_dirty;
        row = row.child(bar_btn_explain(
            "mc-fleet-gen-only",
            "sparkles",
            "Gen only",
            gen_enabled,
            t,
            cx.listener(move |this, _e, window, cx| {
                if !this.services.ai_ready {
                    this.push_toast(
                        crate::toast::ToastKind::Error,
                        "AI unavailable",
                        Some("Enable Ollama / AI in Settings first.".into()),
                        cx,
                    );
                    return;
                }
                if !(this.fleet_actions_idle()
                    && crate::menu_actions::fleet_menu_caps(this, &this.selected_repos_ordered())
                        .has_dirty)
                {
                    this.push_toast(
                        crate::toast::ToastKind::Info,
                        "Nothing to generate",
                        Some(
                            "Select dirty repos first (Gen only dims when the selection is clean)."
                                .into(),
                        ),
                        cx,
                    );
                    return;
                }
                this.run_generate_commit_selected(
                    crate::views::generate_commit::GenerateCommitChoice::MessageOnly,
                    window,
                    cx,
                );
            }),
        ));
        row = row.child(bar_btn_explain(
            "mc-fleet-gen-push",
            "sparkles",
            "Gen & push",
            gen_enabled,
            t,
            cx.listener(move |this, _e, window, cx| {
                if !this.services.ai_ready {
                    this.push_toast(
                        crate::toast::ToastKind::Error,
                        "AI unavailable",
                        Some("Enable Ollama / AI in Settings first.".into()),
                        cx,
                    );
                    return;
                }
                if !(this.fleet_actions_idle()
                    && crate::menu_actions::fleet_menu_caps(this, &this.selected_repos_ordered())
                        .has_dirty)
                {
                    this.push_toast(
                        crate::toast::ToastKind::Info,
                        "Nothing to generate",
                        Some(
                            "Select dirty repos first (Gen & push dims when the selection is clean)."
                                .into(),
                        ),
                        cx,
                    );
                    return;
                }
                this.run_generate_commit_selected(
                    crate::views::generate_commit::GenerateCommitChoice::CommitAndPush,
                    window,
                    cx,
                );
            }),
        ));
        // Same pull-only gating as Push — hide when selection is entirely vendor.
        if caps.has_pushable_path {
            row = row.child(bar_btn(
                "mc-fleet-empty-commit",
                "git-commit",
                "Empty commit",
                idle,
                false,
                t,
                cx.listener(|this, _e, _w, cx| {
                    let repos = this.selected_repos_ordered();
                    this.run_fleet_repos(FleetOp::EmptyCommit, repos, cx);
                }),
            ));
        }
        row
    }

    /// Gear "Actions" dropdown for the Mission Control chip row — stage /
    /// commit / discard / prune / reset / mute / IDE (sync primaries live on
    /// the ops row beside this button).
    pub fn fleet_actions_button(&self, _t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let app = cx.entity();
        let idle = self.fleet_actions_idle();
        let ai_ready = self.services.ai_ready;
        let repos = self.selected_repos_ordered();
        let n = repos.len();
        let caps = crate::menu_actions::fleet_menu_caps(self, &repos);
        let label = if n > 0 {
            format!("Actions ({n})")
        } else {
            "Actions".into()
        };
        let open_remote = if n == 1 {
            self.rows
                .iter()
                .find(|r| r.id.as_ref() == repos[0].as_str())
                .filter(|r| !r.url.is_empty())
                .map(|r| {
                    (
                        r.url.clone(),
                        SharedString::from(crate::data::open_on_host_label(r.host.as_ref())),
                    )
                })
        } else {
            None
        };
        Button::new("mc-fleet-actions")
            .outline()
            .small()
            .compact()
            .icon(IconName::Settings)
            .label(label)
            .dropdown_caret(true)
            .dropdown_menu(move |menu, _window, _cx| {
                crate::menu_actions::fill_fleet_actions_menu(
                    menu,
                    app.clone(),
                    repos.clone(),
                    crate::menu_actions::FleetMenuOpts {
                        ai_ready,
                        idle,
                        caps,
                        clear_selection: n > 0,
                        open_remote: open_remote.clone(),
                        open_drawer: if n == 1 {
                            Some(SharedString::from(repos[0].clone()))
                        } else {
                            None
                        },
                        section_label: None,
                        // Ops-row already has Fetch/Pull/Push/Subs/Gen/Empty.
                        include_sync_primaries: false,
                    },
                )
            })
    }
}

impl RepoHarborApp {
    /// The prune confirm strip (two-stage confirm, the #173 pattern scaled
    /// up): summary + per-repo breakdown while `Ready`, a scanning notice
    /// while the background scan runs. Danger-tinted like the Cleanup view's
    /// armed prune button — branch deletion is irreversible.
    fn prune_strip(&self, plan: &PrunePlan, t: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        let mut strip = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.))
            .px(px(16.))
            .py(px(8.))
            .border_t_1()
            .border_color(rgb(t.behind))
            .bg(rgb(t.surface))
            .child(lucide("scissors", 15., t.behind));
        match plan {
            PrunePlan::Scanning => {
                strip = strip.child(
                    div()
                        .flex_1()
                        .text_size(px(t.text_small))
                        .text_color(rgb(t.fg1))
                        .child("Scanning selection for prunable branches…"),
                );
            }
            PrunePlan::Ready {
                entries, skipped, ..
            } => {
                let total: usize = entries.iter().map(|e| e.count).sum();
                strip = strip
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .text_size(px(t.text_small))
                            .text_color(rgb(t.fg0))
                            .child(SharedString::from(prune_title(total, entries.len()))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .font_family("monospace")
                            .text_size(px(t.text_data_sm))
                            .text_color(rgb(t.fg2))
                            .child(SharedString::from(prune_breakdown(entries, skipped))),
                    )
                    .child(bar_btn(
                        "fleet-prune-confirm",
                        "scissors",
                        &format!("Confirm prune {total}"),
                        true,
                        true,
                        t,
                        cx.listener(|this, _e, _w, cx| this.confirm_fleet_prune(cx)),
                    ));
            }
        }
        strip
            .child(bar_btn(
                "fleet-prune-cancel",
                "x",
                "Cancel",
                true,
                false,
                t,
                cx.listener(|this, _e, _w, cx| this.cancel_fleet_prune(cx)),
            ))
            .into_any_element()
    }
}

/// "Prune 14 branches across 5 repos" (with singular forms where they apply).
fn prune_title(branches: usize, repos: usize) -> String {
    format!(
        "Prune {branches} {} across {repos} {}",
        if branches == 1 { "branch" } else { "branches" },
        if repos == 1 { "repo" } else { "repos" },
    )
}

/// Short display names for the Gen & push confirm strip (grid `name`, else
/// path tail), capped like the prune breakdown.
fn gen_push_name_list(app: &RepoHarborApp, repos: &[String]) -> String {
    const MAX_SHOWN: usize = 5;
    let mut names: Vec<String> = repos
        .iter()
        .take(MAX_SHOWN)
        .map(|id| {
            app.rows
                .iter()
                .find(|r| r.id.as_ref() == id.as_str())
                .map(|r| r.name.to_string())
                .unwrap_or_else(|| id.rsplit('/').next().unwrap_or(id).to_string())
        })
        .collect();
    if repos.len() > MAX_SHOWN {
        names.push(format!("+{} more", repos.len() - MAX_SHOWN));
    }
    names.join(", ")
}

/// The confirm strip's per-repo breakdown: "repoharbor ×3 · zed ×2 · +4 more",
/// then the selected repos with nothing prunable ("nothing to prune: api,
/// docs, +2 more") so the pre-confirm view accounts for the whole selection.
fn prune_breakdown(entries: &[PruneEntry], skipped: &[SharedString]) -> String {
    let mut parts: Vec<String> = entries
        .iter()
        .take(MAX_BREAKDOWN_SHOWN)
        .map(|e| format!("{} ×{}", e.name, e.count))
        .collect();
    if entries.len() > MAX_BREAKDOWN_SHOWN {
        parts.push(format!("+{} more", entries.len() - MAX_BREAKDOWN_SHOWN));
    }
    let mut out = parts.join(" · ");
    if !skipped.is_empty() {
        let mut names: Vec<String> = skipped
            .iter()
            .take(MAX_SKIPPED_SHOWN)
            .map(|s| s.to_string())
            .collect();
        if skipped.len() > MAX_SKIPPED_SHOWN {
            names.push(format!("+{} more", skipped.len() - MAX_SKIPPED_SHOWN));
        }
        out.push_str(&format!(" · nothing to prune: {}", names.join(", ")));
    }
    out
}

/// One flat fleet-bar button. Disabled (`!enabled`) renders dimmed with no
/// click handler; `danger` tints the label/icon for Cancel.
fn bar_btn(
    id: &'static str,
    icon: &'static str,
    label: &str,
    enabled: bool,
    danger: bool,
    t: &Theme,
    on: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::AnyElement {
    let fg = match (enabled, danger) {
        (false, _) => t.fg3,
        (true, true) => t.behind,
        (true, false) => t.fg1,
    };
    let mut b = div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.))
        .px(px(10.))
        .py(px(5.))
        .rounded(px(t.r_sm))
        .bg(rgb(t.button_bg))
        .border_1()
        .border_color(rgb(t.border))
        .text_size(px(t.text_small))
        .text_color(rgb(fg))
        .child(lucide(icon, 14., fg))
        .child(SharedString::from(label.to_string()));
    if enabled {
        let hov = t.border_strong;
        b = b
            .cursor_pointer()
            .hover(move |s| s.border_color(rgb(hov)))
            .on_click(on);
    }
    b.into_any_element()
}

/// Like [`bar_btn`], but always clickable — dims when `active` is false so the
/// handler can explain (toast) instead of a silent no-op.
fn bar_btn_explain(
    id: &'static str,
    icon: &'static str,
    label: &str,
    active: bool,
    t: &Theme,
    on: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::AnyElement {
    let fg = if active { t.fg1 } else { t.fg3 };
    let hov = t.border_strong;
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.))
        .px(px(10.))
        .py(px(5.))
        .rounded(px(t.r_sm))
        .bg(rgb(t.button_bg))
        .border_1()
        .border_color(rgb(t.border))
        .text_size(px(t.text_small))
        .text_color(rgb(fg))
        .cursor_pointer()
        .hover(move |s| s.border_color(rgb(hov)))
        .child(lucide(icon, 14., fg))
        .child(SharedString::from(label.to_string()))
        .on_click(on)
        .into_any_element()
}

/// Full per-repo outcomes for the notice panel (toast detail stays clipped).
fn fleet_log_context(app: &RepoHarborApp, report: &FleetReport) -> crate::activity_log::LogContext {
    use crate::activity_log::LogContext;
    const MAX_FACTS: usize = 80;
    let mut ctx = if report.results.len() == 1 {
        app.log_context_for_id(&report.results[0].repo)
    } else {
        LogContext::default()
    };
    ctx = ctx.with_fact(
        "Repos",
        format!(
            "{} ok · {} failed · {} skipped",
            report.ok_count(),
            report.failed_count(),
            report.skipped_count()
        ),
    );
    for r in report.results.iter().take(MAX_FACTS) {
        let name = r.repo.rsplit('/').next().unwrap_or(&r.repo);
        let val = match &r.outcome {
            Outcome::Ok(m) => m.clone(),
            Outcome::Failed(m) => format!("failed: {m}"),
            Outcome::Skipped(m) => format!("skipped: {m}"),
        };
        ctx = ctx.with_fact(name.to_string(), crate::data::oneline(val));
    }
    if report.results.len() > MAX_FACTS {
        ctx = ctx.with_fact("…", format!("+{} more", report.results.len() - MAX_FACTS));
    }
    ctx
}

/// Kind + title + detail for the fleet resolution toast.
///
/// Titles read as outcomes ("Pull succeeded" / "Pull failed"), not bare
/// counters ("Pull: 1 ok") — success details name the repo and git outcome
/// (e.g. "up to date"); any failure is an Error toast that persists until
/// clicked so failed repos are never silently swept away.
fn resolve_toast(op: FleetOp, report: &FleetReport) -> (ToastKind, String, Option<SharedString>) {
    let (ok, failed, skipped) = (
        report.ok_count(),
        report.failed_count(),
        report.skipped_count(),
    );
    let kind = if failed > 0 {
        ToastKind::Error
    } else {
        ToastKind::Success
    };
    let title = if report.cancelled {
        format!("{} cancelled", op.label())
    } else if failed > 0 && ok == 0 {
        format!("{} failed", op.label())
    } else if failed > 0 {
        format!("{} finished with errors", op.label())
    } else {
        format!("{} succeeded", op.label())
    };
    let detail = if failed > 0 {
        failure_detail(report)
    } else {
        success_detail(report, ok, skipped)
    };
    (kind, title, detail)
}

/// Count line for cancelled / multi-repo success toasts, e.g. "2 ok, 1 skipped".
fn count_line(ok: usize, failed: usize, skipped: usize) -> String {
    let mut parts = vec![format!("{ok} ok")];
    if failed > 0 {
        parts.push(format!("{failed} failed"));
    }
    if skipped > 0 {
        parts.push(format!("{skipped} skipped"));
    }
    parts.join(", ")
}

/// Success-path detail: per-repo git outcome when there are few, else a count
/// summary. Prefers the engine's short messages ("up to date",
/// "fast-forwarded 3") so Pull/Fetch results are unambiguous.
fn success_detail(report: &FleetReport, ok: usize, skipped: usize) -> Option<SharedString> {
    if report.cancelled {
        return Some(count_line(ok, 0, skipped).into());
    }
    let named: Vec<String> = report
        .results
        .iter()
        .filter_map(|r| {
            let name = r.repo.rsplit('/').next().unwrap_or(&r.repo);
            match &r.outcome {
                Outcome::Ok(msg) if !msg.is_empty() => {
                    let msg = if msg == "up to date" {
                        "already up to date"
                    } else {
                        msg.as_str()
                    };
                    Some(format!(
                        "{name}: {}",
                        clip(&crate::data::oneline(msg.to_string()), MAX_REASON_CHARS)
                    ))
                }
                Outcome::Ok(_) => Some(name.to_string()),
                Outcome::Skipped(why) => Some(format!(
                    "{name}: skipped ({})",
                    clip(&crate::data::oneline(why.clone()), MAX_REASON_CHARS)
                )),
                Outcome::Failed(_) => None,
            }
        })
        .collect();
    if named.is_empty() {
        return None;
    }
    if named.len() <= MAX_FAILURES_SHOWN {
        return Some(named.join(" · ").into());
    }
    // Large fleets: keep the toast short — counts are enough.
    Some(count_line(ok, 0, skipped).into())
}

/// Failed repos + reasons for the toast detail, e.g.
/// "repoharbor: local changes would be overwritten · zed: histories diverged".
/// One flattened text run (GPUI single-line text panics on embedded newlines),
/// each reason clipped, capped at [`MAX_FAILURES_SHOWN`] entries + "+N more".
fn failure_detail(report: &FleetReport) -> Option<SharedString> {
    let failures: Vec<(&str, &str)> = report
        .results
        .iter()
        .filter_map(|r| match &r.outcome {
            Outcome::Failed(why) => {
                Some((r.repo.rsplit('/').next().unwrap_or(&r.repo), why.as_str()))
            }
            _ => None,
        })
        .collect();
    if failures.is_empty() {
        return None;
    }
    let mut parts: Vec<String> = failures
        .iter()
        .take(MAX_FAILURES_SHOWN)
        .map(|(name, why)| {
            format!(
                "{name}: {}",
                clip(&crate::data::oneline(why.to_string()), MAX_REASON_CHARS)
            )
        })
        .collect();
    if failures.len() > MAX_FAILURES_SHOWN {
        parts.push(format!("+{} more", failures.len() - MAX_FAILURES_SHOWN));
    }
    Some(parts.join(" · ").into())
}

/// Clip to at most `max` chars (char-boundary safe), ellipsised.
fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

/// AI message only — no git write. Surfaced in the fleet summary toast / Log.
fn generate_message_only_op() -> impl Fn(&str) -> Outcome + Sync {
    |path| match generate_commit_message(path) {
        Err(o) => o,
        Ok(message) => {
            let subject =
                crate::data::oneline(repoharbor_core::ai::split_commit_message(&message).0);
            if subject.is_empty() {
                Outcome::Failed("empty AI commit subject".into())
            } else {
                Outcome::Ok(format!("message ready — {subject}"))
            }
        }
    }
}

/// AI → commit_all → push. An explicit Generate, commit & push publishes
/// every selected repo, including ones under a pull-only prefix — Needs me
/// already asked for that publish. If AI is unreachable after a retry, a
/// local fallback message still commits (same degrade-gracefully contract as
/// PR drafting).
fn generate_commit_and_push_op() -> impl Fn(&str) -> Outcome + Sync {
    move |path| {
        let (message, used_fallback) = match generate_commit_message(path) {
            Ok(m) => (m, false),
            Err(Outcome::Skipped(s)) => return Outcome::Skipped(s),
            Err(_) => match working_or_staged_diff(path) {
                Err(o) => return o,
                Ok(diff) => (
                    repoharbor_core::ai::fallback_commit_message(&diff, path),
                    true,
                ),
            },
        };
        match commit_with_message(path, &message) {
            Outcome::Ok(commit_detail) => {
                let commit_detail = if used_fallback {
                    format!("{commit_detail} (AI fallback)")
                } else {
                    commit_detail
                };
                finish_push(path, commit_detail)
            }
            other => other,
        }
    }
}

fn working_or_staged_diff(path: &str) -> Result<String, Outcome> {
    let full = repoharbor_core::git_ops::working_diff(path).unwrap_or_default();
    let diff = if !full.trim().is_empty() {
        full
    } else {
        repoharbor_core::git_ops::staged_diff(path).unwrap_or_default()
    };
    if diff.trim().is_empty() {
        Err(Outcome::Skipped("nothing to commit".into()))
    } else {
        Ok(diff)
    }
}

/// Transport / timeout blips worth one retry before falling back.
fn is_transient_ai_error(err: &str) -> bool {
    let e = err.to_ascii_lowercase();
    e.contains("error sending request")
        || e.contains("error trying to connect")
        || e.contains("timed out")
        || e.contains("connection reset")
        || e.contains("dns error")
}

fn generate_commit_message(path: &str) -> Result<String, Outcome> {
    let diff = working_or_staged_diff(path)?;
    let msg = crate::task::block_on({
        let diff = diff.clone();
        let path = path.to_string();
        async move {
            let first = repoharbor_core::ai::commit_message(&path, &diff).await;
            if first.as_ref().is_err_and(|e| is_transient_ai_error(e)) {
                repoharbor_core::ai::commit_message(&path, &diff).await
            } else {
                first
            }
        }
    });
    let message = match msg {
        Ok(m) => m,
        Err(e) => return Err(Outcome::Failed(format!("generate: {e}"))),
    };
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return Err(Outcome::Failed("empty AI commit message".into()));
    }
    Ok(trimmed.to_string())
}

fn finish_push(path: &str, commit_detail: String) -> Outcome {
    match repoharbor_core::git_ops::push(path) {
        Ok(push_msg) => Outcome::Ok(format!("{commit_detail} — pushed ({push_msg})")),
        Err(e) => Outcome::Failed(format!("{commit_detail}; push failed: {e}")),
    }
}

fn commit_with_message(path: &str, message: &str) -> Outcome {
    match repoharbor_core::git_ops::commit_all(path, message.trim()) {
        Ok(hash) => {
            let subject =
                crate::data::oneline(repoharbor_core::ai::split_commit_message(message).0);
            Outcome::Ok(format!("committed {hash} — {subject}"))
        }
        Err(e) if e == "no staged changes to commit" => {
            Outcome::Skipped("nothing to commit".into())
        }
        Err(e) => Outcome::Failed(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use repoharbor_core::fleet::RepoResult;

    fn report(results: Vec<(&str, Outcome)>, cancelled: bool) -> FleetReport {
        let n = results.len();
        FleetReport {
            results: results
                .into_iter()
                .map(|(repo, outcome)| RepoResult {
                    repo: repo.to_string(),
                    outcome,
                })
                .collect(),
            started: n,
            completed: n,
            cancelled,
        }
    }

    #[test]
    fn summary_counts_and_kind() {
        let r = report(
            vec![
                ("/x/a", Outcome::Ok("done".into())),
                ("/x/b", Outcome::Failed("boom".into())),
                ("/x/c", Outcome::Skipped("no upstream".into())),
            ],
            false,
        );
        let (kind, title, detail) = resolve_toast(FleetOp::Pull, &r);
        assert!(kind == ToastKind::Error);
        assert_eq!(title, "Pull finished with errors");
        assert_eq!(detail.as_deref(), Some("b: boom"));

        let clean = report(vec![("/x/a", Outcome::Ok("up to date".into()))], false);
        let (kind, title, detail) = resolve_toast(FleetOp::Fetch, &clean);
        assert!(kind == ToastKind::Success);
        assert_eq!(title, "Fetch succeeded");
        assert_eq!(detail.as_deref(), Some("a: already up to date"));

        let all_fail = report(vec![("/x/b", Outcome::Failed("boom".into()))], false);
        let (kind, title, _) = resolve_toast(FleetOp::Pull, &all_fail);
        assert!(kind == ToastKind::Error);
        assert_eq!(title, "Pull failed");
    }

    #[test]
    fn summary_marks_cancelled_runs() {
        let r = report(
            vec![
                ("/x/a", Outcome::Ok("done".into())),
                ("/x/b", Outcome::Skipped("cancelled".into())),
            ],
            true,
        );
        let (_, title, detail) = resolve_toast(FleetOp::Pull, &r);
        assert_eq!(title, "Pull cancelled");
        assert_eq!(detail.as_deref(), Some("1 ok, 1 skipped"));
    }

    #[test]
    fn failure_detail_lists_names_reasons_and_truncates() {
        let ok = ("/r/fine", Outcome::Ok("done".into()));
        let mut results = vec![ok];
        for i in 0..6 {
            results.push((
                ["/r/a", "/r/b", "/r/c", "/r/d", "/r/e", "/r/f"][i],
                Outcome::Failed(format!("reason {i}\nsecond line")),
            ));
        }
        let detail = failure_detail(&report(results, false)).unwrap();
        // Names come from the path tail; newlines flatten; capped at 4 + more.
        assert!(detail.starts_with("a: reason 0 second line · b: reason 1"));
        assert!(detail.ends_with("+2 more"));
        assert!(!detail.contains('\n'));
    }

    #[test]
    fn failure_detail_none_when_nothing_failed() {
        let r = report(vec![("/x/a", Outcome::Ok("done".into()))], false);
        assert!(failure_detail(&r).is_none());
    }

    #[test]
    fn transient_ai_error_matches_reqwest_transport() {
        assert!(is_transient_ai_error(
            "error sending request for url (https://ollama.com/v1/chat/completions)"
        ));
        assert!(is_transient_ai_error("error trying to connect: dns error"));
        assert!(is_transient_ai_error("operation timed out"));
        assert!(!is_transient_ai_error("no AI model installed"));
        assert!(!is_transient_ai_error("empty AI commit message"));
    }

    #[test]
    fn clip_is_char_boundary_safe() {
        assert_eq!(clip("short", 10), "short");
        let clipped = clip("éééééééééé", 5);
        assert_eq!(clipped, "éééé…");
    }

    #[test]
    fn summary_covers_prune_runs() {
        let r = report(
            vec![
                ("/x/a", Outcome::Ok("pruned 3 branches".into())),
                ("/x/b", Outcome::Skipped("nothing prunable".into())),
            ],
            false,
        );
        let (kind, title, detail) = resolve_toast(FleetOp::Prune, &r);
        assert!(kind == ToastKind::Success);
        assert_eq!(title, "Prune succeeded");
        assert_eq!(
            detail.as_deref(),
            Some("a: pruned 3 branches · b: skipped (nothing prunable)")
        );
    }

    #[test]
    fn prune_title_handles_plurals() {
        assert_eq!(prune_title(14, 5), "Prune 14 branches across 5 repos");
        assert_eq!(prune_title(1, 1), "Prune 1 branch across 1 repo");
    }

    fn entry(name: &str, count: usize) -> PruneEntry {
        PruneEntry {
            name: name.into(),
            count,
        }
    }

    #[test]
    fn prune_breakdown_lists_counts_and_skips() {
        let entries = vec![entry("repoharbor", 3), entry("zed", 2)];
        assert_eq!(prune_breakdown(&entries, &[]), "repoharbor ×3 · zed ×2");
        assert_eq!(
            prune_breakdown(&entries, &["api".into(), "docs".into()]),
            "repoharbor ×3 · zed ×2 · nothing to prune: api, docs"
        );
    }

    #[test]
    fn prune_breakdown_caps_both_lists() {
        let entries: Vec<PruneEntry> = (0..8).map(|i| entry(&format!("r{i}"), 1)).collect();
        let skipped: Vec<SharedString> = (0..5)
            .map(|i| SharedString::from(format!("s{i}")))
            .collect();
        let out = prune_breakdown(&entries, &skipped);
        assert!(out.contains("r5 ×1 · +2 more"), "{out}");
        assert!(
            out.ends_with("nothing to prune: s0, s1, s2, +2 more"),
            "{out}"
        );
        assert!(!out.contains("r6"), "{out}");
    }

    fn stub_row(id: &str) -> crate::data::Row {
        crate::data::Row {
            id: id.into(),
            url: "".into(),
            name: id.into(),
            slug: "".into(),
            root: "".into(),
            path: id.into(),
            description: "".into(),
            language: "".into(),
            branch: "".into(),
            age: "".into(),
            release: "".into(),
            ai_summary: "".into(),
            ahead: 0,
            behind: 0,
            dirty: 0,
            staged: 0,
            unstaged: 0,
            stars: "".into(),
            host: "".into(),
            private: false,
            favorite: false,
            activity: repoharbor_core::model::Activity::Active,
            last_commit_unix: 0,
            parent_id: None,
            submodule_path: None,
            child_count: 0,
        }
    }

    #[test]
    fn selected_visible_ordered_intersects_and_keeps_visible_order() {
        let rows = vec![
            stub_row("/a"),
            stub_row("/b"),
            stub_row("/c"),
            stub_row("/d"),
        ];
        let selected = ["/a", "/c", "/d"]
            .into_iter()
            .map(SharedString::from)
            .collect();
        // Visible set is b, d, a (not grid order) — only a + d are selected.
        let visible = vec![1usize, 3, 0];
        let out = selected_visible_ordered(&rows, &selected, &visible);
        assert_eq!(out, vec!["/d".to_string(), "/a".to_string()]);
    }

    #[test]
    fn selected_visible_ordered_drops_hidden_selection() {
        let rows = vec![stub_row("/keep"), stub_row("/hidden")];
        let selected = ["/keep", "/hidden"]
            .into_iter()
            .map(SharedString::from)
            .collect();
        let visible = vec![0usize]; // only /keep shown
        let out = selected_visible_ordered(&rows, &selected, &visible);
        assert_eq!(out, vec!["/keep".to_string()]);
    }

    fn stub_parent(id: &str, child_count: u32) -> crate::data::Row {
        let mut r = stub_row(id);
        r.child_count = child_count;
        r
    }

    #[test]
    fn submodule_targets_empty_selection_uses_visible_parents() {
        let rows = vec![
            stub_parent("/plain", 0),
            stub_parent("/with-subs", 3),
            stub_parent("/hidden-subs", 2),
            stub_parent("/also", 1),
        ];
        let selected = std::collections::HashSet::new();
        // /hidden-subs is off-filter — must not run.
        let visible = vec![0usize, 1, 3];
        let out = submodule_update_targets(&rows, &selected, &visible);
        assert_eq!(
            out,
            vec!["/with-subs".to_string(), "/also".to_string()],
            "empty selection → visible parents with child_count > 0 only"
        );
    }

    #[test]
    fn submodule_targets_selection_intersects_visible() {
        let rows = vec![
            stub_parent("/a", 2),
            stub_parent("/b", 0),
            stub_parent("/c", 1),
        ];
        let selected = ["/a", "/b", "/c"]
            .into_iter()
            .map(SharedString::from)
            .collect();
        // Only /b and /c visible — keep non-submodule /b (core skips).
        let visible = vec![1usize, 2];
        let out = submodule_update_targets(&rows, &selected, &visible);
        assert_eq!(out, vec!["/b".to_string(), "/c".to_string()]);
    }
}
