//! App shell — TitleBar (brand + roots·repos once) + one chrome bar (nav tabs,
//! search, add/rescan), a context-only left rail (GROUPS / VIEWS / ROOTS / …),
//! and the main column.
//!
//! The tabs are live: clicking an item switches the active `View`; each view loads
//! its data lazily on first selection.

use std::rc::Rc;

use gpui::{
    AppContext, Context, CursorStyle, Entity, FocusHandle, Focusable, FontWeight,
    InteractiveElement, IntoElement, MouseButton, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window, div, prelude::FluentBuilder, px, rgb,
};
use gpui_component::TitleBar;
use gpui_component::input::{Input, InputState};

/// Expanded sidebar width bounds (px). Collapsed mode uses [`SIDEBAR_COLLAPSED`].
pub(crate) const SIDEBAR_DEFAULT: f32 = 236.;
pub(crate) const SIDEBAR_MIN: f32 = 180.;
pub(crate) const SIDEBAR_MAX: f32 = 420.;
const SIDEBAR_COLLAPSED: f32 = 56.;

use repoharbor_core::attention::{AttentionItem, AttentionKind, Severity};
use repoharbor_core::model::{AppConfig, MissionControlLayout};

use crate::card::card;
use crate::data::Row;
use crate::icon::lucide;
use crate::theme::Theme;
use crate::toast::ToastKind;

/// Grid row height without / with AI summary lines (the launcher row is the
/// bottom of the card, so the row must be tall enough not to clip it).
const ROW_H: f32 = 260.;
const ROW_H_AI: f32 = 288.;
/// Compact list row height without / with the truncated AI sparkles line.
const LIST_H: f32 = 72.;
const LIST_H_AI: f32 = 92.;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum View {
    Grid,
    Inbox,
    Feed,
    Explore,
    Agents,
    Tools,
    Janitor,
    Log,
    Settings,
}

/// A Mission Control quick filter. Single-select (radio): one is active at a
/// time, `All` meaning no filtering. Work modes ([`WorkMode`]) decide which
/// filters are offered as contextual chips; duplicates (`Commitable` /
/// `Ahead`) remain for saved-view compatibility but are no longer shown.
#[derive(Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum RepoFilter {
    #[default]
    All,
    Public,
    Private,
    Dirty,
    /// Working-tree changes not yet staged (`unstaged > 0`).
    Stageable,
    /// Anything commitable via Commit All (`dirty > 0`). Kept for saved views;
    /// the Working-mode strip uses [`RepoFilter::Dirty`] instead.
    Commitable,
    /// Local commits not pushed (`ahead > 0`). Kept for saved views; the
    /// Working-mode strip uses [`RepoFilter::Pushable`] instead.
    Ahead,
    /// Ready to push (`ahead > 0`).
    Pushable,
    /// Upstream commits not pulled (`behind > 0`) — needs pull.
    Behind,
    Starred,
    Stale,
    /// Repos with at least one *actionable* attention item
    /// (`AttentionKind::needs_me`) — dirty/unpushed, reviews, finished
    /// agents, …. Live `AgentRunning` sessions stay off this filter (terminal
    /// icon only). Driven by the Needs me work mode.
    Attention,
}

/// Mission Control work mode — the top-level segmented control that gates
/// which filter chips appear and sets the primary filter.
#[derive(Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum WorkMode {
    /// Attention filter — repos that need you. Shows the attention count badge.
    NeedsMe,
    /// Behind filter — upstream updates waiting to pull.
    Behind,
    /// Dirty / Stageable / Pushable chips (no Commitable / Ahead duplicates).
    Working,
    /// No git filter. Visibility chips (Public / Private / Starred / Stale) live
    /// in the always-on strip, not only in this mode.
    #[default]
    All,
}

/// Always-visible Mission Control chips (top filter strip) — visibility /
/// bookkeeping filters independent of the active work mode.
const VISIBILITY_CHIPS: [RepoFilter; 4] = [
    RepoFilter::Public,
    RepoFilter::Private,
    RepoFilter::Starred,
    RepoFilter::Stale,
];

/// True for Public / Private / Starred / Stale — orthogonal to work-mode filters
/// (AND’d, never switches Needs me / Behind / Working → All).
fn is_visibility_filter(f: RepoFilter) -> bool {
    matches!(
        f,
        RepoFilter::Public | RepoFilter::Private | RepoFilter::Starred | RepoFilter::Stale
    )
}

impl WorkMode {
    const ORDER: [WorkMode; 4] = [
        WorkMode::NeedsMe,
        WorkMode::Behind,
        WorkMode::Working,
        WorkMode::All,
    ];

    fn label(self) -> &'static str {
        match self {
            WorkMode::NeedsMe => "Needs me",
            WorkMode::Behind => "Behind",
            WorkMode::Working => "Working",
            WorkMode::All => "All",
        }
    }

    /// Primary filter applied when entering this mode.
    fn default_filter(self) -> RepoFilter {
        match self {
            WorkMode::NeedsMe => RepoFilter::Attention,
            WorkMode::Behind => RepoFilter::Behind,
            WorkMode::Working => RepoFilter::Dirty,
            WorkMode::All => RepoFilter::All,
        }
    }

    /// Contextual chip strip for this mode (empty for Needs me / Behind / All).
    /// Visibility chips are always rendered separately via [`VISIBILITY_CHIPS`].
    fn chips(self) -> &'static [RepoFilter] {
        match self {
            WorkMode::NeedsMe | WorkMode::Behind | WorkMode::All => &[],
            WorkMode::Working => &[
                RepoFilter::Dirty,
                RepoFilter::Stageable,
                RepoFilter::Pushable,
            ],
        }
    }

    /// Infer the mode that owns a (possibly saved) filter.
    fn from_filter(f: RepoFilter) -> WorkMode {
        match f {
            RepoFilter::Attention => WorkMode::NeedsMe,
            RepoFilter::Behind => WorkMode::Behind,
            RepoFilter::Dirty
            | RepoFilter::Stageable
            | RepoFilter::Pushable
            | RepoFilter::Commitable
            | RepoFilter::Ahead => WorkMode::Working,
            RepoFilter::All
            | RepoFilter::Public
            | RepoFilter::Private
            | RepoFilter::Starred
            | RepoFilter::Stale => WorkMode::All,
        }
    }
}

impl RepoFilter {
    fn label(self) -> &'static str {
        match self {
            RepoFilter::All => "All",
            RepoFilter::Public => "Public",
            RepoFilter::Private => "Private",
            RepoFilter::Dirty => "Dirty",
            RepoFilter::Stageable => "Stageable",
            RepoFilter::Commitable => "Commitable",
            RepoFilter::Pushable => "Pushable",
            RepoFilter::Ahead => "Ahead",
            RepoFilter::Behind => "Behind",
            RepoFilter::Starred => "Starred",
            RepoFilter::Stale => "Stale",
            RepoFilter::Attention => "Needs me",
        }
    }

    /// Lucide icon for the chip, if any (the visibility chips carry one).
    fn icon(self) -> Option<&'static str> {
        match self {
            RepoFilter::Public => Some("globe"),
            RepoFilter::Private => Some("lock"),
            RepoFilter::Dirty => Some("circle-dot"),
            RepoFilter::Stageable => Some("plus-square"),
            RepoFilter::Commitable => Some("git-commit-horizontal"),
            RepoFilter::Pushable => Some("upload"),
            RepoFilter::Ahead => Some("arrow-up"),
            RepoFilter::Behind => Some("arrow-down"),
            RepoFilter::Starred => Some("star"),
            RepoFilter::Stale => Some("clock"),
            RepoFilter::Attention => Some("circle-alert"),
            RepoFilter::All => None,
        }
    }

    /// Does `row` pass this filter? `attention` is the app's per-repo severity
    /// lookup (`RepoHarborApp::attention_by_repo`) — only the Attention filter
    /// consults it.
    fn matches(
        self,
        r: &Row,
        attention: &std::collections::HashMap<SharedString, Severity>,
    ) -> bool {
        use repoharbor_core::model::Activity;
        match self {
            RepoFilter::All => true,
            RepoFilter::Public => !r.private,
            RepoFilter::Private => r.private,
            RepoFilter::Dirty => r.dirty > 0,
            RepoFilter::Stageable => r.unstaged > 0,
            RepoFilter::Commitable => r.dirty > 0,
            RepoFilter::Pushable | RepoFilter::Ahead => r.ahead > 0,
            RepoFilter::Behind => r.behind > 0,
            RepoFilter::Starred => r.favorite,
            RepoFilter::Stale => r.activity == Activity::Stale,
            RepoFilter::Attention => attention.contains_key(&r.id),
        }
    }
}

/// Card ordering for Mission Control.
#[derive(Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum SortMode {
    /// Most recently committed first.
    #[default]
    Activity,
    /// Alphabetical by name.
    Name,
}

impl SortMode {
    /// Toolbar label — "Sort: recent" / "Sort: name" (not the activity heatmap).
    fn label(self) -> &'static str {
        match self {
            SortMode::Activity => "Sort: recent",
            SortMode::Name => "Sort: name",
        }
    }

    fn next(self) -> SortMode {
        match self {
            SortMode::Activity => SortMode::Name,
            SortMode::Name => SortMode::Activity,
        }
    }
}

/// Mission Control layout: multi-column cards, or a compact single-column list
/// (the default). TREE lives in the sidebar, not here.
pub type Layout = MissionControlLayout;

/// A persisted Mission Control "quick view": a named snapshot of the active
/// filter combo, restorable from the sidebar's VIEWS section.
#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SavedView {
    pub name: String,
    pub filter: RepoFilter,
    /// Optional Public / Private / Starred / Stale overlay (AND with `filter`).
    #[serde(default)]
    pub visibility: Option<RepoFilter>,
    #[serde(default)]
    pub root: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub sort: SortMode,
}

/// SQLite meta key holding the saved-views JSON array.
const SAVED_VIEWS_KEY: &str = "saved_views";

/// Load persisted saved views from the cache (empty if none / unparseable).
pub fn load_saved_views() -> Vec<SavedView> {
    repoharbor_core::cache::get_meta(SAVED_VIEWS_KEY)
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

fn persist_saved_views(views: &[SavedView]) {
    if let Ok(json) = serde_json::to_string(views) {
        repoharbor_core::cache::set_meta(SAVED_VIEWS_KEY, &json);
    }
}

/// One file that landed locally during a Pull fast-forward.
#[derive(Clone)]
pub struct PulledFile {
    pub repo_id: SharedString,
    pub repo_name: SharedString,
    pub path: SharedString,
}

/// Snapshot of the latest Pull that brought files — sidebar PULLED section.
#[derive(Clone, Default)]
pub struct LastPullReport {
    pub files: Vec<PulledFile>,
    /// Distinct repos that received at least one file.
    pub repo_count: usize,
}

/// A modal layered over the shell (drawer / palette / dialog). Rendered last in
/// `render`, above the active view; `Esc`/backdrop dismisses it.
pub enum Overlay {
    /// The repo detail drawer, keyed by repo id (stable across rescans), with
    /// the active tab.
    Drawer { repo: SharedString, tab: DrawerTab },
    /// The command palette (Ctrl+K).
    Palette(crate::palette::PaletteData),
    /// The new-project dialog (header "+").
    NewProject(crate::views::newproject::NewProjectData),
}

/// The repo workspace tabs. Work is the old Changes tab plus stash and
/// conflicts; History is the commit graph; About stacks the former Overview,
/// PR, Notes, and Readme sections.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DrawerTab {
    Work,
    History,
    About,
}

/// (view, lucide-icon, label) — labels match the real sidebar (route ≠ label).
const NAV: [(View, &str, &str); 9] = [
    (View::Grid, "layout-grid", "Mission Control"),
    (View::Inbox, "inbox", "Inbox"),
    (View::Feed, "rss", "Feed"),
    (View::Explore, "compass", "Explore"),
    (View::Agents, "square-terminal", "Agents"),
    (View::Tools, "wrench", "Dev Tools"),
    (View::Janitor, "scissors", "Cleanup"),
    (View::Log, "history", "Log"),
    (View::Settings, "settings", "Settings"),
];

pub struct RepoHarborApp {
    pub view: View,
    pub rows: Vec<Row>,
    pub roots: usize,
    /// The raw core snapshot behind `rows` — the attention model's local-git
    /// input (host/slug/git facts the flat `Row` doesn't carry). Updated in
    /// lockstep with `rows` via [`Self::apply_snapshot`].
    pub repos: Vec<repoharbor_core::model::Repo>,
    pub theme: Rc<Theme>,
    pub config: AppConfig,
    /// Current attention glance lines (PRs/reviews/CI) from the background
    /// poller — the Inbox nav badge's fallback until the inbox itself loads.
    /// Empty until the first poll lands.
    pub attention: Vec<String>,
    /// The raw inbox facts from the background attention poll — the attention
    /// model's host input until the Inbox view loads its own (fresher) copy.
    /// `None` until the first poll lands, which is how
    /// [`Self::recompute_attention`] tells "no host facts yet" from "inbox
    /// genuinely empty".
    pub polled_inbox: Option<Vec<repoharbor_core::inbox::InboxItem>>,
    /// Cached CI states keyed (remote host domain, slug) — the attention
    /// model's CI input and the drawer Overview's CI line. Seeded from
    /// `ci_cache` at launch (instant, offline-tolerant) and reloaded after
    /// each central CI pass ([`Self::refresh_ci`]).
    pub ci_states: std::collections::HashMap<(String, String), repoharbor_core::cache::CiEntry>,
    /// The last whole-pass CI failure surfaced as a toast, so an ongoing
    /// outage (expired token polled every few minutes) toasts once per
    /// distinct error, not per pass. Soft Info — not a sticky Error.
    pub ci_last_error: Option<String>,
    /// The ranked "needs you" list from `repoharbor_core::attention::compute`
    /// (Urgent first). Recomputed on each source update — rescan, inbox load,
    /// cleanup scan, agents poll — never per frame.
    pub attention_items: Vec<AttentionItem>,
    /// Highest severity per local repo id, derived from `attention_items` in
    /// [`Self::recompute_attention`] — the grid Attention filter + card-dot
    /// lookup.
    pub attention_by_repo: std::collections::HashMap<SharedString, Severity>,
    /// Keys of the urgent attention items already surfaced as desktop
    /// notifications, so an item notifies once per appearance, not per
    /// recompute. `None` until the first recompute with host facts, which
    /// seeds it from the persisted snapshot (so a restart doesn't re-notify
    /// everything still pending).
    pub attention_seen: Option<std::collections::HashSet<String>>,
    /// The attention summary last pushed to the tray — recomputes skip the
    /// (cross-thread) tray round-trip when nothing changed.
    pub tray_attention: repoharbor_platform::tray::TrayAttention,
    /// The modal layered over the active view, if any (drawer/palette/dialog).
    pub overlay: Option<Overlay>,
    /// Async-loaded git data for the open drawer (branches/commits/worktrees).
    pub drawer: crate::drawer::DrawerData,
    /// Inbox view state (lazy, loaded when the nav item is first selected).
    pub inbox: crate::views::inbox::InboxState,
    /// Feed / Explore / Cleanup view state (lazy, loaded on first select).
    pub feed: crate::views::feed::FeedState,
    pub explore: crate::views::explore::ExploreState,
    pub cleanup: crate::views::cleanup::CleanupState,
    /// Repo id whose Cleanup "Prune" button is armed, awaiting a confirming
    /// second click before the (irreversible) bulk branch delete runs.
    pub cleanup_confirm: Option<SharedString>,
    /// Bumped each time a confirm is armed, so a stale revert timer from an
    /// earlier arm can't clear a newer one.
    pub cleanup_confirm_gen: u64,
    /// Agents view state (lazy; detected agent sessions on the machine).
    pub agents: crate::views::agents::AgentsState,
    /// Repo ids (paths) with a live agent session — drives the card indicator.
    /// Refreshed on rescan and by the Agents-view poll.
    pub active_agents: std::collections::HashSet<SharedString>,
    /// Whether the Agents-view poll loop is running (guards against duplicates).
    pub agents_polling: bool,
    /// Worktree path whose Agents "Remove worktree" button is armed, awaiting a
    /// confirming second click (the cleanup-confirm pattern). Discard arms the
    /// same slot with a `discard:`-prefixed key so the two destructive buttons
    /// can't confirm each other.
    pub agents_confirm: Option<SharedString>,
    /// Bumped each time a worktree-remove confirm is armed, so a stale revert
    /// timer from an earlier arm can't clear a newer one.
    pub agents_confirm_gen: u64,
    /// Worktree path whose finished-session review diff is expanded on the
    /// Agents view, if any (one at a time — the diff is the heavy part).
    pub agents_review: Option<SharedString>,
    /// The loaded review diff for [`Self::agents_review`]; `None` while the
    /// background branch-diff load is in flight.
    pub agents_review_diff: Option<SharedString>,
    /// Worktree paths with an Open-PR flow in flight (re-click guard).
    pub agents_pr_busy: std::collections::HashSet<SharedString>,
    /// Slugs currently being cloned from the Explore view.
    pub explore_cloning: std::collections::HashSet<SharedString>,
    /// Explore clone failures keyed by slug, shown on the card; cleared on retry.
    pub explore_errors: std::collections::HashMap<SharedString, SharedString>,
    /// Settings editing session (draft config + field inputs); created on first
    /// open, kept so edits survive navigating away.
    pub settings: Option<crate::views::settings::SettingsState>,
    /// Dev Tools fields (created on first open).
    pub devtools: Option<crate::views::devtools::DevToolsState>,
    /// External-service status: GitHub auth + AI backend reachability.
    pub services: Services,
    /// Handle to the live tray (when it came up) — the attention summary is
    /// pushed through it after each [`Self::recompute_attention`]. Updates are
    /// a short synchronous round-trip to the tray thread.
    pub tray: Option<repoharbor_platform::tray::TrayHandle>,
    /// Handle to the fs-watcher thread — re-armed when repos/roots are added at
    /// runtime (Settings save, New Project, Explore clone) so new paths get
    /// live change events without a restart.
    pub watcher: repoharbor_platform::watcher::WatcherHandle,
    /// Fleet multi-selection: repo ids picked via card checkbox / Ctrl+click /
    /// TREE rows. Cleared when Mission Control filters change (chip / root /
    /// language / group / saved view) so a leftover selection can't act on the
    /// wrong visible set. Pruned to existing repos on rescan. Drives the fleet
    /// bar (see `fleet.rs`).
    pub selected: std::collections::HashSet<SharedString>,
    /// The in-flight fleet bulk run, if any — one execution slot; carries the
    /// engine's cancel flag + the live done/total counter.
    pub fleet_run: Option<crate::fleet::FleetRun>,
    /// FIFO jobs waiting for [`Self::fleet_run`] to finish. Cancel clears this
    /// with the active run so waiting work never starts after Cancel.
    pub fleet_queue: std::collections::VecDeque<crate::fleet::QueuedFleetJob>,
    /// Monotonic fleet-run id source — guards stale progress events.
    pub fleet_seq: u64,
    /// The pending bulk-prune confirm (the fleet bar's confirm strip), if any.
    /// Dropped on selection changes / Esc / Cancel / a run starting, so a
    /// stale plan can never execute (see `fleet.rs`).
    pub fleet_prune: Option<crate::fleet::PrunePlan>,
    /// Bumped each time a prune scan starts, so a stale scan result from a
    /// cancelled/superseded confirm can't resurrect the strip.
    pub fleet_prune_seq: u64,
    /// Pending hard-reset confirm: absolute paths to reset to `@{upstream}`.
    pub fleet_reset: Option<Vec<String>>,
    /// Pending discard-changes confirm: absolute paths to reset to HEAD
    /// (`reset --hard HEAD` + `clean -fd`). Distinct from [`Self::fleet_reset`].
    pub fleet_discard: Option<Vec<String>>,
    /// Pending bulk-commit message strip (one shared message for Commit All…).
    pub fleet_commit: Option<crate::fleet::CommitPlan>,
    /// Pending bulk-merge branch strip (one shared branch to merge into HEAD).
    pub fleet_merge: Option<crate::fleet::MergePlan>,
    /// Pending Gen & push confirm modal (multi-repo only) — AI message →
    /// commit all → push. Dropped on selection change / Esc / Cancel /
    /// backdrop / Confirm.
    pub fleet_gen_push: Option<Vec<String>>,
    /// Click-through detail panel for a toast / Log row.
    pub notice_detail: Option<crate::activity_log::LogEntry>,
    /// Files updated by the most recent successful Pull fast-forward(s) —
    /// shown in the Mission Control sidebar under PULLED.
    pub last_pull: Option<LastPullReport>,
    /// Active toasts, oldest first (rendered bottom-right by
    /// `toast::toast_layer`; see `toast.rs` for the lifecycle).
    pub toasts: Vec<crate::toast::Toast>,
    /// Monotonic toast-id source — unique ids double as the stale-timer guard.
    pub toast_seq: u64,
    /// In-memory activity log for the Log view (ring buffer; not persisted).
    pub activity_log: crate::activity_log::ActivityLog,
    /// Mission Control's UI state (filters, sort, layout, saved views, graph).
    pub grid: GridState,
    /// The active contextual sub-filter for the current non-Grid view (e.g. the
    /// Feed/Inbox category panels). Ephemeral: reset when the view changes so
    /// filters don't bleed across views.
    pub view_filter: Option<SharedString>,
    /// App-root focus handle, so global key bindings (Esc) dispatch here.
    pub focus: FocusHandle,
    /// Left-rail width when expanded (px). Clamped to [`SIDEBAR_MIN`]..=[`SIDEBAR_MAX`].
    pub sidebar_width: f32,
    /// Icon-only rail — hides labels and contextual panels.
    pub sidebar_collapsed: bool,
    /// True while the user is dragging the sidebar resize handle.
    pub sidebar_dragging: bool,
    /// Live text filter for the Mission Control repo grid (name / slug / path).
    /// Created on first paint — `InputState` needs a `Window`.
    pub repo_search: Option<Entity<InputState>>,
    /// Keeps the search input observation alive so the grid refilters on type.
    pub _repo_search_sub: Option<Subscription>,
    /// Embedded interactive terminal dock state (PTY sessions, tabs, quick bar).
    pub terminal: crate::terminal_view::TerminalDockState,
    /// Detected runtime installation layout (user binary, AppImage, source, or package).
    pub install_kind: repoharbor_core::updater::InstallKind,
    /// Live self-update status shown in TitleBar + Settings → About.
    pub update_state: AppUpdateState,
    /// Cached XDG autostart state (`~/.config/autostart/com.digitscode.repoharbor.desktop`).
    pub autostart_enabled: bool,
    /// True while a background workspace rescan (manual or watcher-driven) is running.
    pub rescan_in_flight: bool,
    /// Set when an explicit rescan is requested while another rescan is already running.
    pub rescan_queued: bool,
    /// Whether the queued follow-up rescan must also re-arm the filesystem watcher.
    pub rescan_rearm_pending: bool,
    /// True while a background `activity::compute` pass is running.
    pub activity_loading: bool,
}

/// Live application update-check & installation progress.
#[derive(Clone, Debug, Default)]
pub enum AppUpdateState {
    #[default]
    Idle,
    Checking,
    UpToDate(Option<String>),
    Available(repoharbor_core::updater::ReleaseInfo),
    Installing,
    Error(String),
}

/// Mission Control's UI state, grouped out of [`RepoHarborApp`]: the quick filter,
/// root/language facets, sort + layout, persisted saved views, and the
/// contribution graph.
pub struct GridState {
    /// Active work mode (Needs me / Behind / Working / All).
    pub mode: WorkMode,
    /// Active work-mode / git quick filter (All = no git filtering).
    /// Gated by [`Self::mode`]. Orthogonal to [`Self::visibility`].
    pub filter: RepoFilter,
    /// Optional visibility overlay (Public / Private / Starred / Stale).
    /// AND’d with [`Self::filter`] so chips never yank the work mode to All.
    pub visibility: Option<RepoFilter>,
    /// Active scanned-root filter (sidebar ROOTS); `None` = all roots.
    pub root: Option<SharedString>,
    /// Active language filter (sidebar LANGUAGES); `None` = all languages.
    pub language: Option<SharedString>,
    /// Card ordering.
    pub sort: SortMode,
    /// Repo layout (grid cards vs compact list).
    pub layout: Layout,
    /// Persisted quick views (sidebar VIEWS), loaded from the cache at launch.
    pub saved_views: Vec<SavedView>,
    /// Contribution-graph data (commits/day across repos), computed in the
    /// background; `None` until the first pass lands.
    pub activity: Option<repoharbor_core::activity::Activity>,
    /// Whether the contribution graph is shown (dismissible). Kept off —
    /// DigitsCode fleets skip the heatmap (noise + cost on 400+ repos).
    pub activity_open: bool,
    /// Case-insensitive substring filter over repo name / slug / path.
    /// Synced from [`RepoHarborApp::repo_search`] on each keystroke.
    pub query: String,
    /// When set, the grid shows this parent and its submodule children.
    /// `None` = hide submodule children from the flat grid.
    pub tree_focus: Option<SharedString>,
}

/// Status of the external integrations RepoHarbor talks to — GitHub (auth) and the
/// local AI backend — grouped out of [`RepoHarborApp`]. Surfaced in Settings;
/// `ai_ready`/`github_authed` also gate affordances app-wide.
#[derive(Default)]
pub struct Services {
    /// Whether a GitHub token is currently resolvable (Settings account panel).
    pub github_authed: bool,
    /// Where the active GitHub token came from (resolved off the UI thread).
    pub github_token_source: Option<repoharbor_core::oauth::GithubTokenSource>,
    /// Whether the RepoHarbor-only signed-out marker is active (resolved off the UI thread).
    pub github_signed_out: bool,
    /// An in-progress GitHub device-flow login, if any.
    pub github_device: Option<crate::views::settings::GithubDevice>,
    /// Live AI-backend reachability + model list (Settings AI panel).
    pub ai_status: crate::views::settings::AiStatus,
    /// AI is enabled and reachable — gates semantic search + AI affordances.
    pub ai_ready: bool,
}

impl Default for GridState {
    fn default() -> Self {
        GridState {
            mode: WorkMode::default(),
            filter: RepoFilter::default(),
            visibility: None,
            root: None,
            language: None,
            sort: SortMode::default(),
            layout: Layout::default(),
            saved_views: load_saved_views(),
            activity: None,
            // Contribution heatmap hidden for DigitsCode fleets (noise + costly
            // on 400+ repos). Do not re-add a toolbar heatmap toggle.
            activity_open: false,
            query: String::new(),
            tree_focus: None,
        }
    }
}

/// Recent commit subject lines for a repo (newest first) — the input for the
/// AI changelog / resume prompts. Empty on any git error.
fn recent_summaries(id: &str, limit: usize) -> Vec<String> {
    repoharbor_core::git_ops::recent_log(id, limit)
        .unwrap_or_default()
        .into_iter()
        .map(|c| c.summary)
        .collect()
}

/// Cache-meta key persisting the urgent-item keys already notified, so a
/// restart doesn't re-notify everything still pending. Distinct from the
/// platform notifier's `attention_seen` (whose keys are its own format).
const ATTENTION_SEEN_KEY: &str = "attention_urgent_seen";

/// Stable identity of an attention item for notification dedupe: kind + the
/// most specific repo key + summary. The same fact rendered from either inbox
/// source (background poll / Inbox view) produces the same key.
fn attention_key(item: &AttentionItem) -> String {
    let repo = item.repo.id.as_ref().cloned().unwrap_or_else(|| {
        format!(
            "{}/{}",
            item.repo.remote_host.as_deref().unwrap_or(""),
            item.repo.slug.as_deref().unwrap_or(&item.repo.name),
        )
    });
    format!("{:?}|{repo}|{}", item.kind, item.summary)
}

/// Does config allow desktop-notifying this urgent kind? Layered under the
/// `notify_enabled` + `notify_attention` master switches; the pre-existing
/// per-kind toggles keep their meaning now that these kinds notify from the
/// model instead of the platform poller.
fn urgent_kind_enabled(cfg: &AppConfig, kind: AttentionKind) -> bool {
    match kind {
        AttentionKind::ReviewRequested => cfg.notify_review_requested,
        AttentionKind::CiFailing => cfg.notify_ci_failure,
        _ => true,
    }
}

/// Max attention items surfaced in the tray menu.
const TRAY_TOP: usize = 3;

/// Max attention-reason chips shown on a Mission Control card / list row.
const REASON_CHIPS: usize = 2;

/// Top unique attention kinds for a local repo (items are already severity-
/// sorted). Returns chip labels, how many kinds were omitted ("+N"), and a
/// glance subtitle from the top item (summary · non-URL detail).
fn attention_reason_chips(
    items: &[AttentionItem],
    repo_id: &SharedString,
) -> (Vec<(SharedString, Severity)>, usize, Option<SharedString>) {
    let mut chips = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut more = 0usize;
    let mut subtitle = None;
    for item in items
        .iter()
        .filter(|i| i.repo.id.as_deref() == Some(repo_id.as_ref()))
        .filter(|i| i.kind.needs_me())
    {
        if !seen.insert(item.kind) {
            continue;
        }
        if chips.len() < REASON_CHIPS {
            chips.push((SharedString::from(item.kind.label()), item.severity));
            if subtitle.is_none() {
                // Avoid "hint — Hint" when the summary already embeds the action.
                let hint = item.kind.action_hint();
                let mut line = if item.summary.to_lowercase().contains(&hint.to_lowercase()) {
                    item.summary.clone()
                } else {
                    format!("{} — {}", item.summary, hint)
                };
                if let Some(detail) = item
                    .detail
                    .as_ref()
                    .filter(|d| !d.starts_with("http://") && !d.starts_with("https://"))
                {
                    line = format!("{line} · {detail}");
                }
                subtitle = Some(SharedString::from(line));
            }
        } else {
            more += 1;
        }
    }
    (chips, more, subtitle)
}

/// Per-repo highest Needs-me severity. Live `AgentRunning` items are omitted
/// so they don't inflate the filter count or rank a quiet repo into Needs me.
fn index_needs_me(items: &[AttentionItem]) -> std::collections::HashMap<SharedString, Severity> {
    let mut map = std::collections::HashMap::new();
    for item in items {
        if !item.kind.needs_me() {
            continue;
        }
        if let Some(id) = &item.repo.id {
            map.entry(SharedString::from(id.clone()))
                .or_insert(item.severity);
        }
    }
    map
}

/// Actionable (Urgent / Attention) Needs-me facts with no local checkout id —
/// review/CI that never linked to a scanned path. Mission Control must still
/// list these; counting them into the badge while showing **All clear** lies.
fn host_only_needs_me(items: &[AttentionItem]) -> Vec<&AttentionItem> {
    items
        .iter()
        .filter(|i| {
            i.kind.needs_me()
                && i.repo.id.is_none()
                && matches!(i.severity, Severity::Urgent | Severity::Attention)
        })
        .collect()
}

/// Parsed filter terms from a search query string.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct SearchQueryFilter {
    /// Plain free-text substring terms (lowercased) to match against name, slug, or path.
    pub terms: Vec<String>,
    /// `is:` filters, e.g. "dirty", "clean", "behind", "ahead", "pushable", "staged", "attention", "starred", "public", "private".
    pub is_filters: Vec<String>,
    /// `lang:` or `language:` values (lowercased).
    pub languages: Vec<String>,
    /// `host:` values (lowercased), e.g. "github", "gitlab".
    pub hosts: Vec<String>,
    /// `root:` values (lowercased).
    pub roots: Vec<String>,
}

impl SearchQueryFilter {
    /// Parse a query string into free text and structured filters (`is:`, `lang:`, `host:`, `root:`).
    pub(crate) fn parse(query: &str) -> Self {
        let mut filter = SearchQueryFilter::default();
        for token in query.split_whitespace() {
            let token_lower = token.to_lowercase();
            if let Some(val) = token_lower.strip_prefix("is:") {
                if !val.is_empty() {
                    filter.is_filters.push(val.to_string());
                }
            } else if let Some(val) = token_lower.strip_prefix("lang:") {
                if !val.is_empty() {
                    filter.languages.push(val.to_string());
                }
            } else if let Some(val) = token_lower.strip_prefix("language:") {
                if !val.is_empty() {
                    filter.languages.push(val.to_string());
                }
            } else if let Some(val) = token_lower.strip_prefix("host:") {
                if !val.is_empty() {
                    filter.hosts.push(val.to_string());
                }
            } else if let Some(val) = token_lower.strip_prefix("root:") {
                if !val.is_empty() {
                    filter.roots.push(val.to_string());
                }
            } else {
                filter.terms.push(token_lower);
            }
        }
        filter
    }

    /// Check if a query filter has any active criteria.
    pub(crate) fn is_empty(&self) -> bool {
        self.terms.is_empty()
            && self.is_filters.is_empty()
            && self.languages.is_empty()
            && self.hosts.is_empty()
            && self.roots.is_empty()
    }

    /// Check whether a single Row matches this parsed query.
    pub(crate) fn matches_row(
        &self,
        r: &Row,
        attention_by_repo: &std::collections::HashMap<SharedString, Severity>,
    ) -> bool {
        // Free text terms must all be contained somewhere in name, slug, or path.
        for term in &self.terms {
            let in_name = r.name.to_lowercase().contains(term);
            let in_slug = r.slug.to_lowercase().contains(term);
            let in_path = r.path.to_lowercase().contains(term);
            if !(in_name || in_slug || in_path) {
                return false;
            }
        }

        // Language filters (must match any specified)
        if !self.languages.is_empty() {
            let row_lang = r.language.to_lowercase();
            if !self.languages.iter().any(|l| row_lang.contains(l)) {
                return false;
            }
        }

        // Host filters
        if !self.hosts.is_empty() {
            let row_host = r.host.to_lowercase();
            if !self.hosts.iter().any(|h| row_host.contains(h)) {
                return false;
            }
        }

        // Root filters
        if !self.roots.is_empty() {
            let row_root = r.root.to_lowercase();
            if !self.roots.iter().any(|root| row_root.contains(root)) {
                return false;
            }
        }

        // `is:` predicates
        for is_val in &self.is_filters {
            match is_val.as_str() {
                "dirty" => {
                    if r.dirty == 0 {
                        return false;
                    }
                }
                "clean" => {
                    if r.dirty > 0 {
                        return false;
                    }
                }
                "behind" => {
                    if r.behind == 0 {
                        return false;
                    }
                }
                "ahead" | "pushable" => {
                    if r.ahead == 0 {
                        return false;
                    }
                }
                "staged" => {
                    if r.staged == 0 {
                        return false;
                    }
                }
                "unstaged" | "stageable" => {
                    if r.unstaged == 0 {
                        return false;
                    }
                }
                "attention" | "needs-me" | "needs_me" => {
                    if !attention_by_repo.contains_key(&r.id) {
                        return false;
                    }
                }
                "starred" | "favorite" => {
                    if !r.favorite {
                        return false;
                    }
                }
                "public" => {
                    if r.private {
                        return false;
                    }
                }
                "private" => {
                    if !r.private {
                        return false;
                    }
                }
                "stale" => {
                    if r.activity != repoharbor_core::model::Activity::Stale {
                        return false;
                    }
                }
                // Unknown `is:` values don't match
                _ => return false,
            }
        }

        true
    }
}

/// TREE / submodule visibility. By default submodule children stay off the
/// flat grid; Needs me reveals children that are in the attention index so
/// the chip/badge never claim work the list hides.
fn tree_allows_row(
    tree_focus: Option<&SharedString>,
    parent_id: Option<&SharedString>,
    row_id: &SharedString,
    reveal_needs_me_child: bool,
) -> bool {
    match (tree_focus, parent_id) {
        (None, Some(_)) => reveal_needs_me_child,
        (None, None) => true,
        (Some(focus), None) => row_id == focus,
        (Some(focus), Some(parent)) => parent == focus || row_id == focus,
    }
}

/// Fold the ranked attention list into the tray's compact summary: actionable
/// counts (Urgent + Attention — Info is ambient and stays off the tray) and
/// the top few lines. `items` is already severity-sorted, so the top lines
/// are the most urgent.
fn tray_summary(items: &[AttentionItem]) -> repoharbor_platform::tray::TrayAttention {
    let mut summary = repoharbor_platform::tray::TrayAttention::default();
    for item in items.iter().filter(|i| i.severity != Severity::Info) {
        summary.total += 1;
        if item.severity == Severity::Urgent {
            summary.urgent += 1;
        }
        if summary.top.len() < TRAY_TOP {
            summary
                .top
                .push(format!("{} · {}", item.repo.name, item.summary));
        }
    }
    summary
}

impl RepoHarborApp {
    /// Install a fresh fleet snapshot (rows + raw repos + root count) and
    /// recompute the attention model from it. Every path that reloads `rows`
    /// goes through here, so the attention surfaces (nav badges, grid filter,
    /// card dots) never go stale relative to the grid.
    pub fn apply_snapshot(&mut self, snap: crate::data::Snapshot) {
        self.rows = snap.rows;
        self.roots = snap.roots;
        self.repos = snap.repos;
        self.recompute_attention();
    }

    /// Structured facts for a local checkout — used when a toast/log line is
    /// about that repo so the notice panel can show branch / commit / git state.
    pub fn log_context_for_id(&self, id: &str) -> crate::activity_log::LogContext {
        use crate::activity_log::LogContext;
        let Some(row) = self.rows.iter().find(|r| r.id.as_ref() == id) else {
            return LogContext {
                repo_id: Some(id.into()),
                repo_name: Some(id.rsplit('/').next().unwrap_or(id).to_string().into()),
                path: Some(repoharbor_core::privacy::redact_user_paths(id).into()),
                ..LogContext::default()
            };
        };
        let (commit, commit_subject) = match repoharbor_core::git_ops::recent_log(id, 1)
            .ok()
            .and_then(|v| v.into_iter().next())
        {
            Some(c) => (Some(c.id.into()), Some(c.summary.into())),
            None => (None, None),
        };
        let host = if row.host.is_empty() {
            None
        } else {
            Some(row.host.clone())
        };
        let url = if row.url.is_empty() {
            None
        } else {
            Some(row.url.clone())
        };
        let slug = if row.slug.as_ref() == "no remote" {
            None
        } else {
            Some(row.slug.clone())
        };
        LogContext {
            repo_id: Some(row.id.clone()),
            repo_name: Some(row.name.clone()),
            slug,
            path: Some(row.path.clone()),
            branch: Some(row.branch.clone()).filter(|b| !b.is_empty()),
            commit,
            commit_subject,
            ahead: Some(row.ahead),
            behind: Some(row.behind),
            dirty: Some(row.dirty),
            host,
            url,
            facts: Vec::new(),
        }
    }

    /// Best-effort match of toast/log text to a unique local repo.
    pub fn infer_log_context(
        &self,
        title: &str,
        detail: Option<&str>,
    ) -> crate::activity_log::LogContext {
        use crate::activity_log::LogContext;
        let blob = match detail {
            Some(d) => format!("{title} {d}"),
            None => title.to_string(),
        };
        let mut hits: Vec<String> = Vec::new();
        for row in &self.rows {
            let name = row.name.as_ref();
            let slug = row.slug.as_ref();
            let path_hit = blob.contains(row.id.as_ref());
            let name_hit = name.len() >= 3 && blob.contains(name);
            let slug_hit = slug != "no remote" && slug.len() >= 3 && blob.contains(slug);
            if path_hit || name_hit || slug_hit {
                let id = row.id.to_string();
                if !hits.contains(&id) {
                    hits.push(id);
                }
            }
        }
        if hits.len() == 1 {
            self.log_context_for_id(&hits[0])
        } else {
            LogContext::default()
        }
    }

    pub fn open_notice(&mut self, entry: crate::activity_log::LogEntry, cx: &mut Context<Self>) {
        self.notice_detail = Some(entry);
        cx.notify();
    }

    pub fn close_notice(&mut self, cx: &mut Context<Self>) {
        if self.notice_detail.take().is_some() {
            cx.notify();
        }
    }

    pub fn open_notice_from_toast(&mut self, toast_id: u64, cx: &mut Context<Self>) {
        let Some(toast) = self.toasts.iter().find(|t| t.id == toast_id) else {
            return;
        };
        let log_id = toast.log_id;
        let title = toast.title.clone();
        let detail = toast.detail.clone();
        let kind = toast.kind;
        let entry = log_id
            .and_then(|id| self.activity_log.get(id).cloned())
            .unwrap_or_else(|| {
                let message = match detail.as_ref().filter(|d| !d.is_empty()) {
                    Some(d) => format!("{title} — {d}"),
                    None => title.to_string(),
                };
                let level = match kind {
                    crate::toast::ToastKind::Error => crate::activity_log::LogLevel::Error,
                    crate::toast::ToastKind::Info => crate::activity_log::LogLevel::Warn,
                    crate::toast::ToastKind::Success | crate::toast::ToastKind::Progress => {
                        crate::activity_log::LogLevel::Info
                    }
                };
                crate::activity_log::LogEntry {
                    id: 0,
                    at: crate::data::now_unix(),
                    level,
                    message: message.into(),
                    context: self
                        .infer_log_context(title.as_ref(), detail.as_ref().map(|d| d.as_ref())),
                }
            });
        self.open_notice(entry, cx);
    }

    /// After a fleet snapshot lands: if the Changes tab is open, re-read that
    /// repo's file lists so nested deletions/edits the watcher just noticed
    /// appear without switching tabs.
    pub fn refresh_open_changes(&mut self, cx: &mut Context<Self>) {
        let Some(Overlay::Drawer {
            repo,
            tab: DrawerTab::Work,
        }) = &self.overlay
        else {
            return;
        };
        let repo = repo.clone();
        if self.drawer.repo != repo {
            return;
        }
        self.drawer.diff = crate::drawer::DiffState::Loading;
        crate::drawer::load_changes(repo.clone(), cx);
        crate::drawer::load_work_meta(repo, cx);
    }

    /// Recompute the ranked attention list (and the per-repo severity lookup)
    /// from what the app already holds, then route it downstream: push the
    /// summary to the tray (if it changed) and desktop-notify urgent items
    /// that newly appeared. Cheap and foreground-safe — runs after each
    /// source update; no polling happens here (the notification send is a
    /// detached fire-and-forget task, and the tray push is a short
    /// cross-thread round-trip skipped when unchanged).
    ///
    /// Freshness follows source freshness: local git facts refresh with every
    /// (re)scan, inbox facts with each attention poll (and each Inbox load,
    /// which is fresher while it lasts), prunable counts with each Cleanup
    /// scan, agent sessions with the agents poll — and facts from a lazy view
    /// are simply absent until it has loaded. CI facts come from `ci_states`
    /// (cache-seeded at launch, refreshed by the central pass in
    /// [`Self::refresh_ci`]), restricted to remotes still in the fleet.
    pub fn recompute_attention(&mut self) {
        use repoharbor_core::attention::{self, AgentFact, PrunableFact};
        // Prefer the Inbox view's facts (they're the freshest the moment it
        // loads); fall back to the background poll's so the model — and the
        // tray/notifications behind it — works with the window never touched.
        let inbox: &[repoharbor_core::inbox::InboxItem] = match &self.inbox {
            crate::views::inbox::InboxState::Ready(d) => &d.raw,
            _ => self.polled_inbox.as_deref().unwrap_or(&[]),
        };
        let prunable: Vec<PrunableFact> = match &self.cleanup {
            crate::views::cleanup::CleanupState::Ready(repos) => repos
                .iter()
                .map(|r| PrunableFact {
                    repo_id: r.id.to_string(),
                    count: r.branches.len() as u32,
                })
                .collect(),
            _ => Vec::new(),
        };
        // Live sessions raise running facts; dispatched worktrees whose
        // session finished with commits to review (detected + persisted by
        // the agents scan, #185) raise finished facts against their origin
        // repo — see `views::agents::agent_facts` for the full mapping.
        let agents: Vec<AgentFact> = match &self.agents {
            crate::views::agents::AgentsState::Ready(data) => {
                crate::views::agents::agent_facts(data)
            }
            _ => Vec::new(),
        };
        let ci = repoharbor_core::ci::facts(&self.ci_states, &self.repos);
        let raw = attention::compute(&self.repos, inbox, &ci, &prunable, &agents);
        self.attention_items = attention::apply_attention_policy(
            raw,
            &self.config.pull_only_prefixes,
            &self.config.mute_attention_prefixes,
        );
        // Items are severity-sorted (Urgent first), so a repo's first
        // Needs-me occurrence is its highest actionable severity. Live agent
        // sessions are a readout, not a filter hit.
        self.attention_by_repo = index_needs_me(&self.attention_items);
        // Needs me / chip membership can change without an explicit filter
        // click — drop selection ids that left the visible set.
        self.prune_selection_to_visible();
        self.push_tray_attention();
        self.notify_fresh_urgent();
    }

    /// Mirror the attention model onto the tray: actionable counts + the top
    /// items. Skips the cross-thread update when the summary hasn't changed.
    fn push_tray_attention(&mut self) {
        let summary = tray_summary(&self.attention_items);
        if summary == self.tray_attention {
            return;
        }
        if let Some(tray) = &self.tray {
            tray.set_attention(summary.clone());
        }
        self.tray_attention = summary;
    }

    /// Desktop-notify urgent attention items that newly appeared in this
    /// recompute, once per appearance: the key-set of already-surfaced items
    /// is kept on the app (and persisted, so a restart doesn't re-notify
    /// what's still pending — the same trick as the platform notifier's poll
    /// dedupe).
    fn notify_fresh_urgent(&mut self) {
        use std::collections::HashSet;
        // Urgent kinds derive from host facts: review requests from the
        // inbox, CI failures from `ci_states`. Until an inbox source has
        // produced — the background poll or the Inbox view — an empty urgent
        // set means "not loaded yet", not "all clear": diffing against it
        // would first persist an empty snapshot, then re-notify every
        // still-pending item on the next poll of every launch. CI facts
        // don't need their own gate: they're cache-seeded at launch (so any
        // still-failing repo is already in the persisted seen-set) and only
        // ever refreshed by the pass, never blanked (an `Err` keeps the last
        // known state, see `repoharbor_core::ci`).
        let inbox_loaded = self.polled_inbox.is_some()
            || matches!(self.inbox, crate::views::inbox::InboxState::Ready(_));
        if !inbox_loaded {
            return;
        }
        let current: HashSet<String> = self
            .attention_items
            .iter()
            .filter(|i| i.severity == Severity::Urgent)
            .map(attention_key)
            .collect();
        // First recompute with facts this session → seed from the persisted
        // snapshot, so only items that appeared since the last run notify.
        // No snapshot at all (first-ever run) → baseline silently, like the
        // platform notifier's first poll.
        let prev = self.attention_seen.take().or_else(|| {
            repoharbor_core::cache::get_meta(ATTENTION_SEEN_KEY)
                .and_then(|s| serde_json::from_str(&s).ok())
        });
        let to_notify: Vec<AttentionItem> = if let Some(prev) = &prev
            && self.config.notify_enabled
            && self.config.notify_attention
        {
            self.attention_items
                .iter()
                .filter(|i| {
                    i.severity == Severity::Urgent
                        && urgent_kind_enabled(&self.config, i.kind)
                        && !prev.contains(&attention_key(i))
                })
                .cloned()
                .collect()
        } else {
            Vec::new()
        };
        for item in to_notify {
            let title = item.repo.name.clone();
            let body = item.summary.clone();
            let mut ctx = item
                .repo
                .id
                .as_deref()
                .map(|id| self.log_context_for_id(id))
                .unwrap_or_default();
            ctx = ctx.with_fact("Kind", item.kind.label());
            ctx = ctx.with_fact("Summary", item.summary.clone());
            if let Some(d) = &item.detail {
                if d.starts_with("http://") || d.starts_with("https://") {
                    ctx.url = Some(d.clone().into());
                } else {
                    ctx = ctx.with_fact("Detail", d.clone());
                }
            }
            self.activity_log.push_ctx(
                crate::data::now_unix(),
                crate::activity_log::LogLevel::Warn,
                format!("{title} — {body}"),
                ctx,
            );
            crate::task::spawn_detached(async move {
                let _ = repoharbor_platform::notify::send(&title, &body).await;
            });
        }
        if prev.as_ref() != Some(&current)
            && let Ok(blob) = serde_json::to_string(&current)
        {
            repoharbor_core::cache::set_meta(ATTENTION_SEEN_KEY, &blob);
        }
        self.attention_seen = Some(current);
    }

    /// Open the repo detail drawer for `repo` (id) on Overview, and kick off its
    /// async git load.
    pub fn open_drawer(&mut self, repo: SharedString, window: &mut Window, cx: &mut Context<Self>) {
        self.overlay = Some(Overlay::Drawer {
            repo: repo.clone(),
            tab: DrawerTab::Work,
        });
        self.drawer = crate::drawer::DrawerData::loading(repo.clone());
        // The Overview's CI line: the repo's cached default-branch CI state
        // (kept fresh by the central pass — see `refresh_ci`). Sync map
        // lookup, no I/O.
        self.drawer.ci = self
            .repos
            .iter()
            .find(|r| r.id.as_str() == repo.as_ref())
            .and_then(|r| {
                let slug = r.slug.as_deref()?;
                let host = r.remote_host.as_deref().unwrap_or_default();
                self.ci_states
                    .get(&(host.to_string(), slug.to_string()))
                    .cloned()
            });
        // The new-worktree field lives in Overview, shown immediately on open.
        self.drawer.worktree_input = Some(cx.new(|cx| {
            gpui_component::input::InputState::new(window, cx).placeholder("new-worktree-name")
        }));
        // The dispatch task-prompt field (Overview's "Dispatch agent" section) —
        // created here because InputState needs the Window.
        self.drawer.dispatch_input = Some(cx.new(|cx| {
            gpui_component::input::InputState::new(window, cx).placeholder("Task for the agent…")
        }));
        crate::drawer::load_overview(repo.clone(), cx);
        // Work is the tab the drawer opens on. About (README, PRs, notes) loads
        // when that tab is selected, so a card click does not fetch the host.
        self.ensure_drawer_changes_inputs(window, cx);
        self.drawer.diff = crate::drawer::DiffState::Loading;
        crate::drawer::load_changes(repo.clone(), cx);
        crate::drawer::load_work_meta(repo, cx);
        cx.notify();
    }

    /// Dismiss whatever overlay is open.
    pub fn close_overlay(&mut self) {
        self.overlay = None;
    }

    /// Open the command palette and focus its query field.
    pub fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = cx.new(|cx| {
            gpui_component::input::InputState::new(window, cx)
                .placeholder("Jump to a repo or run a command…")
        });
        // On each keystroke: reset the selection, kick off a (debounced) code
        // search, and re-render.
        let sub = cx.observe(&query, |this, _q, cx| {
            if let Some(Overlay::Palette(d)) = &mut this.overlay {
                d.selected = 0;
            }
            this.trigger_code_search(cx);
            this.trigger_semantic_search(cx);
            cx.notify();
        });
        let fh = query.read(cx).focus_handle(cx);
        self.overlay = Some(Overlay::Palette(crate::palette::PaletteData {
            query,
            selected: 0,
            code: Vec::new(),
            semantic: Vec::new(),
            embeddings: None,
            query_vecs: std::collections::HashMap::new(),
            generation: 0,
            _sub: sub,
        }));
        window.focus(&fh, cx);
        // Load the embedding index once per palette session (a per-keystroke
        // load would dominate recall latency). Gated on AI: with it off the
        // index stays `None` and the palette is name-matching only.
        if self.services.ai_ready {
            cx.spawn(async move |this, cx| {
                let index = cx
                    .background_executor()
                    .spawn(async { repoharbor_core::semantic::all_embeddings() })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    match &mut this.overlay {
                        Some(Overlay::Palette(d)) => {
                            d.embeddings = Some(std::sync::Arc::new(index))
                        }
                        _ => return, // palette closed while loading
                    }
                    // Rank whatever was typed while the index loaded.
                    this.trigger_semantic_search(cx);
                });
            })
            .detach();
        }
        cx.notify();
    }

    /// Open the add dialog on a specific tab.
    pub fn open_add_dialog(
        &mut self,
        mode: crate::views::newproject::NewMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::views::newproject::NewProjectData;
        use gpui_component::input::InputState;
        let url =
            cx.new(|cx| InputState::new(window, cx).placeholder("https://github.com/owner/repo"));
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("repo"));
        let remote =
            cx.new(|cx| InputState::new(window, cx).placeholder("git@github.com:owner/repo.git"));
        let template = cx.new(|cx| InputState::new(window, cx).placeholder("~/templates/rust"));
        let root_path =
            cx.new(|cx| InputState::new(window, cx).placeholder("~/Projects or ~/code/my-app"));
        let subs = vec![
            cx.observe(&url, |_this, _e, cx| cx.notify()),
            cx.observe(&name, |_this, _e, cx| cx.notify()),
            cx.observe(&remote, |_this, _e, cx| cx.notify()),
            cx.observe(&template, |_this, _e, cx| cx.notify()),
            cx.observe(&root_path, |_this, _e, cx| cx.notify()),
        ];
        self.overlay = Some(Overlay::NewProject(NewProjectData {
            mode,
            url,
            name,
            remote,
            template,
            root_path,
            first_commit: true,
            root: 0,
            status: "".into(),
            busy: false,
            _subs: subs,
        }));
        cx.notify();
    }

    /// Toggle the new-project "make initial commit" option.
    pub fn new_project_toggle_first_commit(&mut self, cx: &mut Context<Self>) {
        if let Some(Overlay::NewProject(d)) = &mut self.overlay {
            d.first_commit = !d.first_commit;
        }
        cx.notify();
    }

    /// Switch the new-project dialog's mode (clone vs create).
    pub fn new_project_set_mode(
        &mut self,
        mode: crate::views::newproject::NewMode,
        cx: &mut Context<Self>,
    ) {
        if let Some(Overlay::NewProject(d)) = &mut self.overlay {
            d.mode = mode;
            d.status = "".into();
        }
        cx.notify();
    }

    /// Cycle the new-project destination root.
    pub fn new_project_cycle_root(&mut self, cx: &mut Context<Self>) {
        let n = self.config.roots.len();
        if let Some(Overlay::NewProject(d)) = &mut self.overlay
            && n > 0
        {
            d.root = (d.root + 1) % n;
        }
        cx.notify();
    }

    /// Validate + run the add dialog (add root / clone / init), then rescan
    /// and close on success.
    pub fn submit_new_project(&mut self, cx: &mut Context<Self>) {
        use crate::views::newproject::NewMode;
        let Some(Overlay::NewProject(d)) = &self.overlay else {
            return;
        };
        if d.busy {
            return;
        }
        let mode = d.mode;

        // ── Add local path (single repo or scan folder) ───────────────────
        if mode == NewMode::AddRoot {
            let raw = d.root_path.read(cx).value().trim().to_string();
            match Self::prepare_workspace_root(&raw, &self.config.roots) {
                Ok((store, is_repo)) => {
                    self.config.roots.push(store.clone());
                    if let Err(e) = repoharbor_core::config::save(&self.config) {
                        let _ = self.config.roots.pop();
                        self.set_new_project_status(&format!("Save failed: {e}"), cx);
                        return;
                    }
                    self.close_overlay();
                    let (title, detail) = if is_repo {
                        (
                            "Added repo",
                            format!("Scanning {store} as a single repository."),
                        )
                    } else {
                        ("Added scan root", format!("Scanning repos under {store}."))
                    };
                    self.push_toast(ToastKind::Success, title, Some(detail.into()), cx);
                    self.rescan_with_rearm(cx);
                }
                Err(msg) => self.set_new_project_status(&msg, cx),
            }
            return;
        }

        // ── Clone / Create into an existing root ──────────────────────────
        let name = d.name.read(cx).value().trim().to_string();
        let url = d.url.read(cx).value().trim().to_string();
        let remote = d.remote.read(cx).value().trim().to_string();
        let template = d.template.read(cx).value().trim().to_string();
        let first_commit = d.first_commit;
        let Some(root) = self.config.roots.get(d.root).cloned() else {
            self.set_new_project_status("Add a local path first (Local path tab).", cx);
            return;
        };
        if name.is_empty() {
            self.set_new_project_status("Enter a folder name.", cx);
            return;
        }
        if mode == NewMode::Clone && url.is_empty() {
            self.set_new_project_status("Enter a repository URL.", cx);
            return;
        }
        let dest = format!("{}/{}", root.trim_end_matches('/'), name);
        if let Some(Overlay::NewProject(d)) = &mut self.overlay {
            d.busy = true;
            d.status = if mode == NewMode::Clone {
                "Cloning…".into()
            } else {
                "Creating…".into()
            };
        }
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    match mode {
                        NewMode::AddRoot => unreachable!("handled above"),
                        NewMode::Clone => repoharbor_core::git_ops::clone(&url, &dest),
                        NewMode::Create => repoharbor_core::git_ops::init(
                            &dest,
                            &name,
                            (!template.is_empty()).then_some(template.as_str()),
                            (!remote.is_empty()).then_some(remote.as_str()),
                            first_commit.then_some("Initial commit"),
                        ),
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(_) => {
                    this.close_overlay();
                    this.rescan_with_rearm(cx);
                }
                Err(e) => {
                    if let Some(Overlay::NewProject(d)) = &mut this.overlay {
                        d.busy = false;
                        d.status = format!("Failed: {e}").into();
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn set_new_project_status(&mut self, msg: &str, cx: &mut Context<Self>) {
        if let Some(Overlay::NewProject(d)) = &mut self.overlay {
            d.status = msg.to_string().into();
        }
        cx.notify();
    }

    /// Expand `raw`, require an existing directory, detect single-repo vs scan
    /// folder, and reject duplicates against `existing` (expanded path compare).
    /// Returns `(absolute_path, is_single_repo)`.
    fn prepare_workspace_root(raw: &str, existing: &[String]) -> Result<(String, bool), String> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Err("Enter a path.".into());
        }
        let path = repoharbor_core::scan::expand(raw);
        if !path.is_dir() {
            return Err(format!("Not a directory: {}", path.display()));
        }
        let store = path.to_string_lossy().into_owned();
        let already = existing
            .iter()
            .any(|r| repoharbor_core::scan::expand(r) == path);
        if already {
            return Err("That path is already configured.".into());
        }
        let is_repo = repoharbor_core::scan::is_git_workdir(&path);
        Ok((store, is_repo))
    }

    /// The current palette result list (actions + repos + code hits).
    fn palette_items(&self, cx: &Context<Self>) -> Vec<crate::palette::PaletteItem> {
        match &self.overlay {
            Some(Overlay::Palette(d)) => crate::palette::items(
                &self.rows,
                &d.code,
                &d.semantic,
                &d.query.read(cx).value(),
                !self.selected_repos_ordered().is_empty(),
            ),
            _ => Vec::new(),
        }
    }

    /// Move the palette selection by `delta` (wrapping), if it's open.
    fn move_palette(&mut self, delta: isize, cx: &mut Context<Self>) {
        let len = self.palette_items(cx).len();
        if let Some(Overlay::Palette(d)) = &mut self.overlay
            && len > 0
        {
            let i = d.selected as isize + delta;
            d.selected = i.rem_euclid(len as isize) as usize;
        }
        cx.notify();
    }

    /// Execute the currently-selected palette item (called on Enter).
    fn confirm_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let items = self.palette_items(cx);
        if items.is_empty() {
            return;
        }
        let selected = match &self.overlay {
            Some(Overlay::Palette(d)) => d.selected.min(items.len() - 1),
            _ => return,
        };
        if let Some(item) = items.get(selected).cloned() {
            let moved_focus = matches!(
                item,
                crate::palette::PaletteItem::Action(crate::palette::PaletteAction::ToggleTerminal)
            );
            self.run_palette_item(item, Some(window), cx);
            if !moved_focus {
                window.focus(&self.focus, cx);
            }
        }
    }

    /// Debounced cross-repo code search for the current query.
    fn trigger_code_search(&mut self, cx: &mut Context<Self>) {
        let (query, generation) = match &mut self.overlay {
            Some(Overlay::Palette(d)) => {
                d.generation += 1;
                (d.query.read(cx).value().to_string(), d.generation)
            }
            _ => return,
        };
        let paths: Vec<String> = self.rows.iter().map(|r| r.id.to_string()).collect();
        cx.spawn(async move |this, cx| {
            // Debounce keystrokes before doing the (expensive) ripgrep pass.
            cx.background_executor()
                .timer(std::time::Duration::from_millis(220))
                .await;
            // Bail if a newer keystroke superseded this search.
            let current = this
                .update(
                    cx,
                    |this, _| matches!(&this.overlay, Some(Overlay::Palette(d)) if d.generation == generation),
                )
                .unwrap_or(false);
            if !current {
                return;
            }
            let results = if query.trim().len() >= 2 {
                cx.background_executor()
                    .spawn(async move {
                        repoharbor_core::search::search(&query, &paths, 60).unwrap_or_default()
                    })
                    .await
            } else {
                Vec::new()
            };
            let _ = this.update(cx, |this, cx| {
                if let Some(Overlay::Palette(d)) = &mut this.overlay
                    && d.generation == generation {
                        d.code = results.into_iter().map(crate::palette::code_hit).collect();
                        cx.notify();
                    }
            });
        })
        .detach();
    }

    /// Debounced semantic recall for the current palette query: embed the
    /// query, rank it against the session-cached embedding index, and surface
    /// the best repo per match with its snippet. Gated on AI being ready (and
    /// the backend supporting embeddings); reuses the code-search generation
    /// for stale-drop. The query embedding is cached per palette session, so
    /// only genuinely new queries hit the backend — and never before the
    /// debounce window passes.
    fn trigger_semantic_search(&mut self, cx: &mut Context<Self>) {
        if !self.services.ai_ready || !repoharbor_core::ai::embeddings_supported() {
            return;
        }
        let (query, generation) = match &self.overlay {
            Some(Overlay::Palette(d)) => {
                (d.query.read(cx).value().trim().to_string(), d.generation)
            }
            _ => return,
        };
        if query.len() < 2 {
            if let Some(Overlay::Palette(d)) = &mut self.overlay {
                d.semantic.clear();
            }
            return;
        }
        // Session-cached query vector → rank right away, no debounce/backend.
        if let Some(Overlay::Palette(d)) = &self.overlay
            && let (Some(vec), Some(index)) =
                (d.query_vecs.get(&query).cloned(), d.embeddings.clone())
        {
            let hits = self.semantic_rank(&vec, &index);
            if let Some(Overlay::Palette(d)) = &mut self.overlay {
                d.semantic = hits;
            }
            cx.notify();
            return;
        }
        let model = self.config.embed_model.clone();
        cx.spawn(async move |this, cx| {
            // Debounce keystrokes before the (slow) embedding round-trip.
            cx.background_executor()
                .timer(std::time::Duration::from_millis(260))
                .await;
            // Bail if a newer keystroke superseded this search.
            let current = this
                .update(cx, |this, _| {
                    matches!(&this.overlay, Some(Overlay::Palette(d)) if d.generation == generation)
                })
                .unwrap_or(false);
            if !current {
                return;
            }
            let q = query.clone();
            let vec =
                crate::task::run(async move { repoharbor_core::ai::embed(&model, &q).await }).await;
            let _ = this.update(cx, |this, cx| {
                let Ok(vec) = vec else {
                    // Backend unreachable — recall just finds nothing.
                    if let Some(Overlay::Palette(d)) = &mut this.overlay
                        && d.generation == generation
                    {
                        d.semantic.clear();
                        cx.notify();
                    }
                    return;
                };
                let vec = std::sync::Arc::new(vec);
                let index = match &mut this.overlay {
                    Some(Overlay::Palette(d)) => {
                        // Cache even when stale — a re-typed query reuses it.
                        d.query_vecs.insert(query, vec.clone());
                        d.embeddings.clone()
                    }
                    _ => return,
                };
                // Index still loading → its completion re-triggers ranking.
                let Some(index) = index else { return };
                let hits = this.semantic_rank(&vec, &index);
                if let Some(Overlay::Palette(d)) = &mut this.overlay
                    && d.generation == generation
                {
                    d.semantic = hits;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Rank the loaded embedding index against a query vector and resolve the
    /// hits to repo rows: best chunk per repo (core `recall`), then
    /// (host, slug) store keys mapped back to repo ids for the palette.
    fn semantic_rank(
        &self,
        query: &[f32],
        index: &[repoharbor_core::semantic::EmbeddingRow],
    ) -> Vec<crate::palette::SemanticHit> {
        use repoharbor_core::semantic;
        let by_key: std::collections::HashMap<(String, String), &str> = self
            .repos
            .iter()
            .map(|r| (semantic::repo_key(r), r.id.as_str()))
            .collect();
        semantic::recall(query, index, semantic::MAX_HITS, semantic::MIN_SCORE)
            .into_iter()
            .filter_map(|h| {
                let id = by_key.get(&(h.host.clone(), h.slug.clone()))?;
                let snippet: String = crate::data::oneline(h.content).chars().take(120).collect();
                Some(crate::palette::SemanticHit {
                    id: SharedString::from(id.to_string()),
                    snippet: snippet.into(),
                })
            })
            .collect()
    }

    /// Close the palette and act on `item`.
    pub fn run_palette_item(
        &mut self,
        item: crate::palette::PaletteItem,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        use crate::palette::{PaletteAction, PaletteItem};
        // Resolve data living in the (about-to-close) palette first.
        let code_abs = match (&item, &self.overlay) {
            (PaletteItem::Code(i), Some(Overlay::Palette(d))) => {
                d.code.get(*i).map(|h| h.abs.to_string())
            }
            _ => None,
        };
        self.close_overlay();
        match item {
            PaletteItem::Action(PaletteAction::Rescan) => self.rescan_user(cx),
            PaletteItem::Action(PaletteAction::Settings) => {
                self.view = View::Settings;
                self.view_filter = Some("roots".into());
                if let Some(w) = window {
                    self.maybe_load_view(View::Settings, w, cx);
                }
            }
            PaletteItem::Action(PaletteAction::ToggleTerminal) => {
                self.toggle_terminal(window, cx);
            }
            // Fleet verbs (#184) — the same run plumbing as the fleet bar
            // (`fleet.rs`); one bulk run at a time is enforced there.
            PaletteItem::Action(PaletteAction::FetchAll) => {
                if !self.ensure_fleet_actions_idle(cx) {
                    return;
                }
                let repos: Vec<String> = self.rows.iter().map(|r| r.id.to_string()).collect();
                self.run_fleet_repos(crate::fleet::FleetOp::Fetch, repos, cx);
            }
            PaletteItem::Action(PaletteAction::PullBehind) => {
                self.pull_behind_repos(cx);
            }
            PaletteItem::Action(PaletteAction::FetchSelected) => {
                if !self.ensure_fleet_actions_idle(cx) {
                    return;
                }
                self.run_fleet(crate::fleet::FleetOp::Fetch, cx)
            }
            PaletteItem::Action(PaletteAction::PullSelected) => {
                if !self.ensure_fleet_actions_idle(cx) {
                    return;
                }
                self.run_fleet(crate::fleet::FleetOp::Pull, cx)
            }
            PaletteItem::Action(PaletteAction::SelectDirty) => {
                self.select_where(|r| r.dirty > 0, "dirty", cx)
            }
            PaletteItem::Action(PaletteAction::SelectBehind) => {
                self.select_where(|r| r.behind > 0, "behind their upstream", cx)
            }
            PaletteItem::Action(PaletteAction::SelectAll) => {
                self.toggle_select_all_visible(cx);
            }
            PaletteItem::Action(PaletteAction::ClearSelection) => {
                self.clear_selection(cx);
            }
            PaletteItem::Repo(i) | PaletteItem::Recall { row: i, .. } => {
                if let Some(r) = self.rows.get(i) {
                    let _ = repoharbor_core::launch::launch(&self.config.ide_command, &r.id);
                }
            }
            PaletteItem::Code(_) => {
                if let Some(abs) = code_abs {
                    let _ = repoharbor_core::launch::launch(&self.config.ide_command, &abs);
                }
            }
        }
        cx.notify();
    }

    /// Load the inbox (PRs / reviews / issues / notifications) over the network.
    pub fn load_inbox(&mut self, cx: &mut Context<Self>) {
        use crate::views::inbox::{InboxData, InboxState, inbox_row, notice_row};
        self.inbox = InboxState::Loading;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let items =
                crate::task::run(async { repoharbor_core::inbox::github_inbox().await }).await;
            let notes =
                crate::task::run(async { repoharbor_core::inbox::github_notifications().await })
                    .await;
            let _ = this.update(cx, |this, cx| {
                this.inbox = match items {
                    Ok(i) => InboxState::Ready(InboxData {
                        items: i.iter().cloned().map(inbox_row).collect(),
                        notifications: notes
                            .unwrap_or_default()
                            .into_iter()
                            .map(notice_row)
                            .collect(),
                        raw: i,
                    }),
                    Err(e) => InboxState::Error(e.into()),
                };
                // Fresh inbox facts → refresh the attention surfaces.
                this.recompute_attention();
                cx.notify();
            });
        })
        .detach();
    }

    /// Lazy-load a view's data the first time it's opened (Idle → Loading).
    fn maybe_load_view(&mut self, view: View, window: &mut Window, cx: &mut Context<Self>) {
        use crate::views;
        match view {
            View::Inbox if matches!(self.inbox, views::inbox::InboxState::Idle) => {
                self.load_inbox(cx)
            }
            View::Feed if matches!(self.feed, views::feed::FeedState::Idle) => self.load_feed(cx),
            View::Explore if matches!(self.explore, views::explore::ExploreState::Idle) => {
                self.load_starred(cx)
            }
            View::Janitor if matches!(self.cleanup, views::cleanup::CleanupState::Idle) => {
                self.load_cleanup(cx)
            }
            View::Agents => {
                if matches!(self.agents, views::agents::AgentsState::Idle) {
                    self.load_agents(cx);
                } else {
                    // Already loaded once — just (re)start the live poll.
                    self.start_agents_poll(cx);
                }
            }
            View::Settings if self.settings.is_none() => self.open_settings(window, cx),
            View::Tools if self.devtools.is_none() => self.open_devtools(window, cx),
            _ => {}
        }
    }

    /// Create the Dev Tools input fields + per-input observations (so each tool's
    /// output recomputes live as you type).
    fn open_devtools(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use crate::views::devtools::{DevToolsState, new_uuid};
        use gpui_component::input::InputState;
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Filter tools…"));
        let base64 = cx.new(|cx| InputState::new(window, cx).placeholder("text"));
        let hash = cx.new(|cx| InputState::new(window, cx).placeholder("text"));
        let json = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .placeholder("{ }")
        });
        let base_conv = cx.new(|cx| InputState::new(window, cx).placeholder("decimal number"));
        let case_conv = cx.new(|cx| InputState::new(window, cx).placeholder("text"));
        let url = cx.new(|cx| InputState::new(window, cx).placeholder("text"));
        let jwt = cx.new(|cx| InputState::new(window, cx).placeholder("eyJ… JWT"));
        let timestamp =
            cx.new(|cx| InputState::new(window, cx).placeholder("unix epoch or RFC 3339"));
        let colour = cx.new(|cx| InputState::new(window, cx).placeholder("#1f6feb or r,g,b"));
        let regex_pat = cx.new(|cx| InputState::new(window, cx).placeholder("pattern"));
        let regex_text = cx.new(|cx| InputState::new(window, cx).placeholder("text to match"));
        let mut subs = Vec::new();
        for input in [
            &search,
            &base64,
            &hash,
            &json,
            &base_conv,
            &case_conv,
            &url,
            &jwt,
            &timestamp,
            &colour,
            &regex_pat,
            &regex_text,
        ] {
            subs.push(cx.observe(input, |_this, _e, cx| cx.notify()));
        }
        self.devtools = Some(DevToolsState {
            search,
            uuid: new_uuid(),
            base64,
            hash,
            json,
            base_conv,
            case_conv,
            url,
            jwt,
            timestamp,
            colour,
            regex_pat,
            regex_text,
            _subs: subs,
        });
        cx.notify();
    }

    /// Start a settings editing session, seeding the field inputs from config,
    /// and kick off the live network checks (GitHub auth + AI reachability).
    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings = Some(crate::views::settings::SettingsState::new(
            &self.config,
            window,
            cx,
        ));
        self.refresh_github_authed(cx);
        self.ai_refresh(cx);
        self.load_index_stats(cx);
        cx.notify();
    }

    /// Re-resolve GitHub authentication state off the UI thread (may shell out to
    /// `gh` on cache miss, so never called inside `render()`).
    pub fn refresh_github_authed(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let (authed, source, signed_out) = cx
                .background_executor()
                .spawn(async { repoharbor_core::oauth::github_auth_state() })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.services.github_authed = authed;
                this.services.github_token_source = source;
                this.services.github_signed_out = signed_out;
                cx.notify();
            });
        })
        .detach();
    }

    /// Begin the GitHub device-flow login: request a code, show it, then poll
    /// until the user authorizes (or it fails / expires).
    pub fn github_connect(&mut self, cx: &mut Context<Self>) {
        use crate::views::settings::GithubDevice;
        if self.services.github_device.is_some() {
            return;
        }
        self.services.github_device = Some(GithubDevice {
            user_code: "…".into(),
            verification_uri: "https://github.com/login/device".into(),
            status: "Requesting a device code…".into(),
        });
        cx.notify();

        let client_id = repoharbor_core::oauth::github_client_id();
        cx.spawn(async move |this, cx| {
            let id = client_id.clone();
            let started =
                crate::task::run(async move { repoharbor_core::oauth::device_start(&id).await })
                    .await;
            let start = match started {
                Ok(d) => d,
                Err(e) => {
                    let _ = this.update(cx, |this, cx| {
                        if let Some(d) = &mut this.services.github_device {
                            d.status = format!("Failed: {e}").into();
                        }
                        cx.notify();
                    });
                    return;
                }
            };
            let device_code = start.device_code.clone();
            let interval = start.interval.max(1);
            if this
                .update(cx, |this, cx| {
                    this.services.github_device = Some(GithubDevice {
                        user_code: start.user_code.into(),
                        verification_uri: start.verification_uri.into(),
                        status: "Waiting for authorization…".into(),
                    });
                    cx.notify();
                })
                .is_err()
            {
                return;
            }

            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(interval))
                    .await;
                // Stop if the user dismissed the flow (e.g. navigated / signed out).
                if this
                    .update(cx, |this, _| this.services.github_device.is_none())
                    .unwrap_or(true)
                {
                    return;
                }
                let id = client_id.clone();
                let code = device_code.clone();
                let poll = crate::task::run(async move {
                    repoharbor_core::oauth::device_poll(&id, &code).await
                })
                .await;
                let status = match poll {
                    Ok(p) => p.status,
                    Err(e) => e,
                };
                match status.as_str() {
                    "authorized" => {
                        let _ = this.update(cx, |this, cx| {
                            this.services.github_device = None;
                            this.services.github_authed = true;
                            this.services.github_token_source =
                                Some(repoharbor_core::oauth::GithubTokenSource::AppOAuth);
                            this.services.github_signed_out = false;
                            cx.notify();
                        });
                        return;
                    }
                    "authorization_pending" | "slow_down" => continue,
                    other => {
                        let msg = match other {
                            "expired_token" => "The code expired — try again.".to_string(),
                            "access_denied" => "Authorization was denied.".to_string(),
                            e => format!("Login failed: {e}"),
                        };
                        let _ = this.update(cx, |this, cx| {
                            if let Some(d) = &mut this.services.github_device {
                                d.status = msg.into();
                            }
                            cx.notify();
                        });
                        return;
                    }
                }
            }
        })
        .detach();
    }

    /// Forget RepoHarbor's stored GitHub token and stop using env/`gh` fallbacks
    /// until Connect again. Does not touch `gh auth` or machine credentials.
    pub fn github_sign_out(&mut self, cx: &mut Context<Self>) {
        repoharbor_core::oauth::sign_out();
        // Flip the fallback toggle off so "turn it back on" clearly re-enables
        // `gh`/env after Sign out (signed-out alone already blocks them).
        self.config.github_allow_cli_token = false;
        if let Some(s) = &mut self.settings {
            s.draft.github_allow_cli_token = false;
        }
        let _ = repoharbor_core::config::save(&self.config);
        self.services.github_device = None;
        self.services.github_authed = false;
        self.services.github_token_source = None;
        self.services.github_signed_out = true;
        self.refresh_github_authed(cx);
        self.push_toast(
            ToastKind::Success,
            "Signed out of RepoHarbor",
            Some("Connect GitHub to sign in again. Your gh CLI login is unchanged.".into()),
            cx,
        );
        cx.notify();
    }

    /// Prefer RepoHarbor device-flow OAuth over the current `gh`/env token: start
    /// Connect; on success the stored token takes priority without touching `gh`.
    pub fn github_prefer_oauth(&mut self, cx: &mut Context<Self>) {
        repoharbor_core::oauth::clear_signed_out();
        self.services.github_device = None;
        self.github_connect(cx);
    }

    /// Persist whether RepoHarbor may fall back to `$REPOHARBOR_GITHUB_TOKEN` / `gh`.
    /// Turning it **on** clears the RepoHarbor-only signed-out marker so a previous
    /// Sign out can use machine credentials again without `gh auth login`.
    pub fn github_set_allow_cli_token(&mut self, on: bool, cx: &mut Context<Self>) {
        self.config.github_allow_cli_token = on;
        if let Some(s) = &mut self.settings {
            s.draft.github_allow_cli_token = on;
        }
        let _ = repoharbor_core::config::save(&self.config);
        if on {
            repoharbor_core::oauth::clear_signed_out();
        } else {
            repoharbor_core::oauth::invalidate_cli_token_cache();
        }
        self.refresh_github_authed(cx);
    }

    /// Forget the live AI status because the *pending* backend no longer matches
    /// the saved one. Re-probing here would be worse than useless: `ai::*` reads
    /// the saved config, so it would re-report the old engine's models under the
    /// newly picked backend.
    pub fn invalidate_ai_status(&mut self, cx: &mut Context<Self>) {
        use crate::views::settings::AiStatus;
        self.services.ai_status = AiStatus::Unknown;
        if let Some(s) = &mut self.settings {
            s.ai_note = "Backend changed — Save & rescan to apply, then Test.".into();
        }
        cx.notify();
    }

    /// Re-check AI-backend reachability and list installed models.
    pub fn ai_refresh(&mut self, cx: &mut Context<Self>) {
        use crate::views::settings::AiStatus;
        if matches!(self.services.ai_status, AiStatus::Pulling(_)) {
            return;
        }
        self.services.ai_status = AiStatus::Checking;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let up = crate::task::run(repoharbor_core::ai::available()).await;
            let status = if up {
                let models = crate::task::run(repoharbor_core::ai::installed_models()).await;
                AiStatus::Ready(
                    models
                        .into_iter()
                        .map(|(n, sz)| (n.into(), crate::data::human_bytes(sz).into()))
                        .collect(),
                )
            } else {
                AiStatus::Offline
            };
            let _ = this.update(cx, |this, cx| {
                let ready = up && this.config.ai_enabled;
                this.services.ai_status = status;
                this.services.ai_ready = ready;
                // Reachable now → (re)build the semantic index so the palette can
                // search by meaning.
                if ready {
                    this.index_semantic();
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Probe the AI backend with a tiny round-trip and report ok/latency in the
    /// Settings AI note. A quick way to confirm the model actually responds.
    pub fn ai_test(&mut self, cx: &mut Context<Self>) {
        if let Some(s) = &mut self.settings {
            s.ai_note = "Testing…".into();
        }
        cx.notify();
        let model = self.config.ai_model.clone();
        cx.spawn(async move |this, cx| {
            let note = crate::task::run(async move {
                let start = std::time::Instant::now();
                match repoharbor_core::ai::generate(&model, "Reply with the single word: pong.")
                    .await
                {
                    Ok(_) => format!("AI responded in {} ms", start.elapsed().as_millis()),
                    Err(e) => format!("AI test failed: {e}"),
                }
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                if let Some(s) = &mut this.settings {
                    s.ai_note = note.into();
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Forget the stored cloud API key (the field is masked and never shows it,
    /// so removal needs its own action). Reports in the Settings AI note.
    pub fn ai_clear_cloud_key(&mut self, cx: &mut Context<Self>) {
        let backend = self
            .settings
            .as_ref()
            .map(|s| s.draft.ai_backend.as_str())
            .unwrap_or(self.config.ai_backend.as_str());
        let target = if backend == "openrouter" {
            "openrouter"
        } else if backend == "groq" {
            "groq"
        } else if backend == "mistral" {
            "mistral"
        } else {
            "cloud"
        };
        let note = match repoharbor_core::cloud::store_api_key_for_target("", target) {
            Ok(()) => match target {
                "openrouter" => "OpenRouter API key removed.".to_string(),
                "groq" => "Groq API key removed.".to_string(),
                "mistral" => "Mistral API key removed.".to_string(),
                _ => "Cloud API key removed.".to_string(),
            },
            Err(e) => format!("Could not remove the key: {e}"),
        };
        if let Some(s) = &mut self.settings {
            s.ai_note = note.into();
        }
        cx.notify();
    }

    /// Clear cached AI summaries + embeddings, reporting the counts in the
    /// Settings AI note. Frees the semantic index and stale summaries.
    pub fn ai_clear_cache(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async { repoharbor_core::cache::clear_ai() })
                .await;
            let note = match result {
                Ok((summaries, embeddings)) => {
                    format!("Cleared {summaries} summaries, {embeddings} embeddings")
                }
                Err(e) => format!("Clear failed: {e}"),
            };
            let _ = this.update(cx, |this, cx| {
                if let Some(s) = &mut this.settings {
                    s.ai_note = note.into();
                }
                this.load_index_stats(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Refresh the semantic-index size line (chunks/repos/bytes) shown in the
    /// Settings AI section.
    pub fn load_index_stats(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let stats = cx
                .background_executor()
                .spawn(async { repoharbor_core::semantic::index_stats() })
                .await;
            let _ = this.update(cx, |this, cx| {
                if let Some(s) = &mut this.settings {
                    s.index_stats = Some(stats);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn next_pool_id() -> String {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        format!("p{ms}-{seq}")
    }

    fn normalize_and_migrate_pool(pool: &mut [repoharbor_core::model::ModelDeployment]) {
        for (i, d) in pool.iter_mut().enumerate() {
            if d.id.trim().is_empty() {
                d.id = Self::next_pool_id();
            }
            d.priority = (i + 1) as u32;
            let inline = d.api_key.trim().to_string();
            if !inline.is_empty() {
                let _ = repoharbor_core::pool_keys::set(&d.id, &inline);
                d.api_key.clear();
            }
        }
        let ids: Vec<String> = pool.iter().map(|d| d.id.clone()).collect();
        let _ = repoharbor_core::pool_keys::retain(&ids);
    }

    fn persist_model_pool(&mut self, cx: &mut Context<Self>) {
        if let Some(s) = &mut self.settings {
            Self::normalize_and_migrate_pool(&mut s.draft.model_pool);
            self.config.model_pool = s.draft.model_pool.clone();
            self.config.ai_failover_enabled = s.draft.ai_failover_enabled;
        }
        let _ = repoharbor_core::config::save(&self.config);
        self.ai_refresh(cx);
        cx.notify();
    }

    fn clear_pool_inputs(
        s: &crate::views::settings::SettingsState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for input in [&s.pool_name, &s.pool_base, &s.pool_model, &s.pool_key] {
            input.update(cx, |st, cx| st.set_value("", window, cx));
        }
    }

    /// Add a new provider row or save edits to the selected row in `model_pool`.
    pub fn pool_add_or_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(s) = &self.settings else { return };
        let raw_name = s.pool_name.read(cx).value().trim().to_string();
        let raw_base = s.pool_base.read(cx).value().trim().to_string();
        let raw_model = s.pool_model.read(cx).value().trim().to_string();
        let raw_key = s.pool_key.read(cx).value().trim().to_string();
        let edit_id = s.pool_edit.clone();

        if raw_name.is_empty() && raw_base.is_empty() {
            if let Some(st) = &mut self.settings {
                st.pool_note = "Provider name or Base URL is required.".into();
            }
            cx.notify();
            return;
        }

        let provider = repoharbor_core::cloud::infer_provider_id(&raw_name, &raw_base);
        let name_is_openai = raw_name.to_ascii_lowercase().contains("openai");
        let effective_base = if !raw_base.is_empty() {
            raw_base.clone()
        } else if provider != "cloud" || name_is_openai {
            repoharbor_core::cloud::default_base_for_provider(&provider).to_string()
        } else {
            if let Some(st) = &mut self.settings {
                st.pool_note =
                    "Base URL is required for custom providers (https://… or http://localhost:…)."
                        .into();
            }
            cx.notify();
            return;
        };

        let base_url = match repoharbor_core::cloud::normalize_base(&effective_base) {
            Ok(b) => b,
            Err(e) => {
                if let Some(st) = &mut self.settings {
                    st.pool_note = e.into();
                }
                cx.notify();
                return;
            }
        };

        let effective_model = if !raw_model.is_empty() {
            raw_model
        } else if provider != "cloud" || base_url.contains("api.openai.com") {
            repoharbor_core::cloud::default_model_for_provider(&provider).to_string()
        } else {
            if let Some(st) = &mut self.settings {
                st.pool_note = "Model id is required for custom endpoints.".into();
            }
            cx.notify();
            return;
        };

        let display_name = if raw_name.is_empty() {
            provider.clone()
        } else {
            raw_name
        };

        let row_id = edit_id.clone().unwrap_or_else(Self::next_pool_id);
        if !raw_key.is_empty()
            && let Err(e) = repoharbor_core::pool_keys::set(&row_id, &raw_key)
        {
            if let Some(st) = &mut self.settings {
                st.pool_note = format!("Failed to store key safely: {e}").into();
            }
            cx.notify();
            return;
        }

        let action_note;
        if let Some(st) = &mut self.settings {
            if let Some(existing) = st.draft.model_pool.iter_mut().find(|d| d.id == row_id) {
                existing.name = display_name.clone();
                existing.provider = provider;
                existing.base_url = base_url;
                existing.model = effective_model;
                action_note = format!("Updated {display_name}.");
            } else {
                let priority = (st.draft.model_pool.len() + 1) as u32;
                st.draft
                    .model_pool
                    .push(repoharbor_core::model::ModelDeployment {
                        id: row_id,
                        name: display_name.clone(),
                        provider,
                        base_url,
                        model: effective_model,
                        api_key: String::new(),
                        enabled: true,
                        priority,
                    });
                action_note = format!("Added {display_name} to list.");
            }
            st.pool_edit = None;
            st.pool_note = action_note.into();
        }
        if let Some(st) = &self.settings {
            Self::clear_pool_inputs(st, window, cx);
        }
        self.persist_model_pool(cx);
    }

    /// Load an existing row into the add/edit form.
    pub fn pool_edit(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(s) = &self.settings else { return };
        let Some(dep) = s.draft.model_pool.iter().find(|d| d.id == id).cloned() else {
            return;
        };
        let (resolved_base, resolved_model) =
            repoharbor_core::cloud::resolve_deployment_base_and_model(&dep);
        let has_own = repoharbor_core::pool_keys::has(&dep.id) || !dep.api_key.trim().is_empty();
        s.pool_name
            .update(cx, |st, cx| st.set_value(dep.name.clone(), window, cx));
        s.pool_base
            .update(cx, |st, cx| st.set_value(resolved_base, window, cx));
        s.pool_model
            .update(cx, |st, cx| st.set_value(resolved_model, window, cx));
        s.pool_key.update(cx, |st, cx| st.set_value("", window, cx));
        if let Some(st) = &mut self.settings {
            st.pool_edit = Some(dep.id);
            st.pool_note = if has_own {
                format!(
                    "Editing {} — leave API key blank to keep existing stored key.",
                    dep.name
                )
                .into()
            } else {
                format!(
                    "Editing {} — enter API key or leave blank for provider/local key.",
                    dep.name
                )
                .into()
            };
        }
        cx.notify();
    }

    /// Exit row editing without changing the pool.
    pub fn pool_cancel_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(s) = &self.settings {
            Self::clear_pool_inputs(s, window, cx);
        }
        if let Some(s) = &mut self.settings {
            s.pool_edit = None;
            s.pool_note = SharedString::default();
        }
        cx.notify();
    }

    /// Append a preset row with a unique id and persist immediately.
    pub fn pool_add_preset(
        &mut self,
        name: &str,
        provider: &str,
        base_url: &str,
        model: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(st) = &mut self.settings else { return };
        let priority = (st.draft.model_pool.len() + 1) as u32;
        st.draft
            .model_pool
            .push(repoharbor_core::model::ModelDeployment {
                id: Self::next_pool_id(),
                name: name.to_string(),
                provider: provider.to_string(),
                base_url: base_url.to_string(),
                model: model.to_string(),
                api_key: String::new(),
                enabled: true,
                priority,
            });
        st.pool_note = format!("Added preset {name} (#{priority}).").into();
        self.persist_model_pool(cx);
    }

    /// Move a row up (`delta = -1`) or down (`delta = 1`) in priority order.
    pub fn pool_move(&mut self, id: &str, delta: isize, cx: &mut Context<Self>) {
        let Some(st) = &mut self.settings else { return };
        let Some(idx) = st.draft.model_pool.iter().position(|d| d.id == id) else {
            return;
        };
        let target = idx as isize + delta;
        if target < 0 || (target as usize) >= st.draft.model_pool.len() {
            return;
        }
        st.draft.model_pool.swap(idx, target as usize);
        self.persist_model_pool(cx);
    }

    /// Enable or disable a row in the provider list.
    pub fn pool_toggle(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(target_dep) = self
            .settings
            .as_mut()
            .and_then(|st| st.draft.model_pool.iter_mut().find(|d| d.id == id))
        {
            target_dep.enabled = !target_dep.enabled;
        } else {
            return;
        }
        self.persist_model_pool(cx);
    }

    /// Remove a row from the list and delete any orphaned stored key.
    pub fn pool_remove(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(st) = &mut self.settings {
            st.draft.model_pool.retain(|d| d.id != id);
            if st.pool_edit.as_deref() == Some(id) {
                st.pool_edit = None;
            }
            st.pool_note = "Removed provider from list.".into();
        }
        self.persist_model_pool(cx);
    }

    /// Ping a single row in `model_pool` and report latency or error in `pool_note`.
    pub fn pool_test(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(st) = &mut self.settings else { return };
        let Some(dep) = st.draft.model_pool.iter().find(|d| d.id == id).cloned() else {
            return;
        };
        let label = if dep.name.trim().is_empty() {
            dep.provider.clone()
        } else {
            dep.name.clone()
        };
        let pool_keys_map = repoharbor_core::pool_keys::load();
        let creds =
            match repoharbor_core::cloud::resolve_deployment_credentials(&dep, &pool_keys_map) {
                Ok(c) => c,
                Err(e) => {
                    st.pool_note = format!("{label}: Test failed — {e}").into();
                    cx.notify();
                    return;
                }
            };
        st.pool_note = format!("Testing {label} ({})…", creds.2).into();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let res = crate::task::run(async move {
                repoharbor_core::cloud::test_deployment(&creds.0, &creds.1, &creds.2).await
            })
            .await;
            let note = match res {
                Ok(ms) => format!("{label}: responded in {ms} ms"),
                Err(e) => format!("{label}: Test failed — {e}"),
            };
            let _ = this.update(cx, |this, cx| {
                if let Some(st) = &mut this.settings {
                    st.pool_note = note.into();
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Settings: drop the whole semantic index (rows + skip signatures) and
    /// re-embed the corpus from scratch, reporting in the AI note. Gated on
    /// `aiReady` like every AI affordance.
    pub fn rebuild_semantic_index(&mut self, cx: &mut Context<Self>) {
        if !self.services.ai_ready {
            return;
        }
        if let Some(s) = &mut self.settings {
            s.ai_note = "Rebuilding index…".into();
        }
        cx.notify();
        let repos = self.repos.clone();
        cx.spawn(async move |this, cx| {
            let cleared = cx
                .background_executor()
                .spawn(async { repoharbor_core::semantic::clear_index() })
                .await;
            let note = match cleared {
                Ok(_) => {
                    let n = crate::task::run(async move {
                        repoharbor_core::semantic::index_fleet(repos).await
                    })
                    .await;
                    format!(
                        "Rebuilt index — embedded {n} repo{}",
                        if n == 1 { "" } else { "s" }
                    )
                }
                Err(e) => format!("Rebuild failed: {e}"),
            };
            let _ = this.update(cx, |this, cx| {
                if let Some(s) = &mut this.settings {
                    s.ai_note = note.into();
                }
                this.load_index_stats(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Download a GGUF model for the llama.cpp backend, streaming progress into
    /// the Settings AI note. The file lands in the app-data models dir; on
    /// success the model list refreshes so it can be selected.
    pub fn llama_download(&mut self, url: String, cx: &mut Context<Self>) {
        let url = url.trim().to_string();
        let note = |this: &mut Self, msg: SharedString| {
            if let Some(s) = &mut this.settings {
                s.ai_note = msg;
            }
        };
        if url.is_empty() {
            note(self, "Enter a model URL to download.".into());
            cx.notify();
            return;
        }
        note(self, "Starting download…".into());
        cx.notify();

        // Progress messages flow from the (core-runtime) download callback to the
        // UI over a channel — the live-wiring pattern, since the callback can't
        // touch the app entity directly.
        let (tx, rx) = async_channel::unbounded::<SharedString>();
        cx.spawn(async move |this, cx| {
            while let Ok(msg) = rx.recv().await {
                if this
                    .update(cx, |this, cx| {
                        if let Some(s) = &mut this.settings {
                            s.ai_note = msg;
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        cx.spawn(async move |this, cx| {
            let tx_progress = tx.clone();
            let result = crate::task::run(async move {
                let mut last_pct = u64::MAX;
                repoharbor_core::llama::download_model(&url, move |downloaded, total| {
                    let msg = match (downloaded * 100).checked_div(total) {
                        Some(pct) => {
                            if pct == last_pct {
                                return; // throttle to whole-percent steps
                            }
                            last_pct = pct;
                            format!(
                                "Downloading… {pct}% ({} / {})",
                                crate::data::human_bytes(downloaded),
                                crate::data::human_bytes(total)
                            )
                        }
                        None => format!("Downloading… {}", crate::data::human_bytes(downloaded)),
                    };
                    let _ = tx_progress.try_send(msg.into());
                })
                .await
            })
            .await;
            let final_msg = match result {
                Ok(path) => format!("Downloaded {}", path.rsplit('/').next().unwrap_or(&path)),
                Err(e) => format!("Download failed: {e}"),
            };
            let _ = tx.send(final_msg.into()).await;
            drop(tx); // close the channel so the drain task ends
            // Pick up the new model in the installed list.
            let _ = this.update(cx, |this, cx| this.ai_refresh(cx));
        })
        .detach();
    }

    /// Dirty repos in the current *visible* selection (grid order) — Gen only /
    /// Gen & push targets.
    pub fn dirty_selected_repos(&self) -> Vec<String> {
        self.selected_repos_ordered()
            .into_iter()
            .filter(|id| {
                self.rows
                    .iter()
                    .any(|r| r.id.as_ref() == id.as_str() && r.dirty > 0)
            })
            .collect()
    }

    /// Run a Generate outcome for `repos`. **Gen only** is always immediate.
    /// **Gen & push** with more than one repo arms the confirm modal first; a
    /// single dirty target (drawer or fleet) stays one-click.
    /// `from_drawer` keeps MessageOnly in the open Changes tab.
    pub fn run_generate_commit(
        &mut self,
        choice: crate::views::generate_commit::GenerateCommitChoice,
        repos: Vec<String>,
        from_drawer: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::views::generate_commit::GenerateCommitChoice;
        if choice != GenerateCommitChoice::FastCommit && !self.services.ai_ready {
            self.push_toast(
                ToastKind::Error,
                "AI unavailable",
                Some("Enable Ollama / AI in Settings first.".into()),
                cx,
            );
            return;
        }
        if repos.is_empty() {
            self.push_toast(
                ToastKind::Info,
                "Nothing to generate",
                Some("Select dirty repos first.".into()),
                cx,
            );
            return;
        }
        match choice {
            GenerateCommitChoice::MessageOnly => {
                if from_drawer || repos.len() == 1 {
                    let id = repos[0].clone();
                    if !from_drawer {
                        self.open_drawer_changes(SharedString::from(id), window, cx);
                    } else {
                        self.ensure_drawer_changes_inputs(window, cx);
                    }
                    self.drawer_generate_commit(window, cx);
                } else {
                    self.run_fleet_repos(crate::fleet::FleetOp::GenerateMessageOnly, repos, cx);
                }
            }
            GenerateCommitChoice::FastCommit => {
                self.run_fleet_repos(crate::fleet::FleetOp::FastCommit, repos, cx);
            }
            GenerateCommitChoice::CommitAndPush => {
                // Same pull-only filter as Push — drop vendor targets before
                // arming the modal / starting a one-click run.
                let Some(repos) = self.retain_non_pull_only_for_op(
                    crate::fleet::FleetOp::GenerateCommitAndPush,
                    repos,
                    cx,
                ) else {
                    return;
                };
                if repos.len() > 1 {
                    self.start_fleet_gen_push(repos, cx);
                } else {
                    self.run_fleet_repos(crate::fleet::FleetOp::GenerateCommitAndPush, repos, cx);
                }
            }
        }
    }

    /// Fleet bar / Actions: Generate for every dirty selected repo.
    pub fn run_generate_commit_selected(
        &mut self,
        choice: crate::views::generate_commit::GenerateCommitChoice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let repos = self.dirty_selected_repos();
        self.adopt_fleet_targets(&repos);
        self.run_generate_commit(choice, repos, false, window, cx);
    }

    /// Store the files that arrived in the latest Pull for the sidebar PULLED
    /// section. Empty results clear a prior panel (nothing new landed).
    pub fn apply_last_pull(
        &mut self,
        collected: Vec<(String, Vec<String>)>,
        cx: &mut Context<Self>,
    ) {
        const FILE_CAP: usize = 120;
        if collected.is_empty() {
            // Keep the previous panel if this Pull was all "up to date".
            return;
        }
        let mut files = Vec::new();
        let mut repos = 0usize;
        for (repo_id, paths) in collected {
            if paths.is_empty() {
                continue;
            }
            repos += 1;
            let name = repo_id
                .rsplit('/')
                .next()
                .unwrap_or(repo_id.as_str())
                .to_string();
            for path in paths {
                if files.len() >= FILE_CAP {
                    break;
                }
                files.push(PulledFile {
                    repo_id: SharedString::from(repo_id.clone()),
                    repo_name: SharedString::from(name.clone()),
                    path: SharedString::from(path),
                });
            }
        }
        if files.is_empty() {
            return;
        }
        let n = files.len();
        self.last_pull = Some(LastPullReport {
            files,
            repo_count: repos,
        });
        self.activity_log.push(
            crate::data::now_unix(),
            crate::activity_log::LogLevel::Info,
            format!("Pull updated {n} file(s) across {repos} repo(s)"),
        );
        cx.notify();
    }

    pub fn clear_last_pull(&mut self, cx: &mut Context<Self>) {
        if self.last_pull.take().is_some() {
            cx.notify();
        }
    }

    /// Open the repo drawer on the Changes tab (creates commit inputs).
    pub fn open_drawer_changes(
        &mut self,
        repo: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_drawer(repo.clone(), window, cx);
        if let Some(Overlay::Drawer { tab, .. }) = &mut self.overlay {
            *tab = DrawerTab::Work;
        }
        self.drawer.diff = crate::drawer::DiffState::Loading;
        self.ensure_drawer_changes_inputs(window, cx);
        crate::drawer::load_changes(repo.clone(), cx);
        crate::drawer::load_work_meta(repo, cx);
        cx.notify();
    }

    fn ensure_drawer_changes_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.drawer.commit_input.is_none() {
            self.drawer.commit_input = Some(cx.new(|cx| {
                gpui_component::input::InputState::new(window, cx).placeholder("Commit subject…")
            }));
        }
        if self.drawer.commit_body_input.is_none() {
            self.drawer.commit_body_input = Some(cx.new(|cx| {
                gpui_component::input::InputState::new(window, cx)
                    .auto_grow(2, 8)
                    .placeholder("Body (optional)…")
            }));
        }
    }

    /// Drawer Changes tab: generate a commit message from the full working-tree
    /// diff (staged + unstaged + untracked), like Cursor / PyCharm — gated on
    /// `aiReady`. The (subject, body) suggestion lands in
    /// `drawer.commit_suggestion` and, on success, pre-fills the composer's
    /// subject + body fields so it's editable before committing.
    ///
    /// Uses [`Context::spawn_in`] so the completion path always has a `Window`
    /// for `InputState::set_value` — plain `spawn` + `update_in` can fail with
    /// "entity has no current window" and silently leave "Generating…" stuck.
    pub fn drawer_generate_commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.services.ai_ready {
            self.push_toast(
                ToastKind::Error,
                "AI unavailable",
                Some("Enable Ollama / AI in Settings first.".into()),
                cx,
            );
            return;
        }
        let repo = self.drawer.repo.clone();
        self.drawer.commit_suggestion = Some(("Generating…".into(), "".into()));
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let id = repo.to_string();
            let id_for_diff = id.clone();
            let diff = cx
                .background_executor()
                .spawn(async move {
                    // Prefer the full working tree (what Commit All will record).
                    // Fall back to staged-only if somehow workdir is empty but
                    // the index isn't (shouldn't happen, but cheap).
                    let full =
                        repoharbor_core::git_ops::working_diff(&id_for_diff).unwrap_or_default();
                    if !full.trim().is_empty() {
                        full
                    } else {
                        repoharbor_core::git_ops::staged_diff(&id_for_diff).unwrap_or_default()
                    }
                })
                .await;
            let result = if diff.trim().is_empty() {
                Err("Working tree clean — nothing to commit.".to_string())
            } else {
                crate::task::run(
                    async move { repoharbor_core::ai::commit_message(&id, &diff).await },
                )
                .await
                .map(|m| repoharbor_core::ai::split_commit_message(&m))
                .and_then(|(subject, body)| {
                    if subject.trim().is_empty() {
                        Err(
                            "AI returned no commit subject — try again or pick another model."
                                .into(),
                        )
                    } else {
                        Ok((subject, body))
                    }
                })
            };
            let applied = this.update_in(cx, |this, window, cx| {
                if this.drawer.repo != repo {
                    return;
                }
                match result {
                    Ok((subject, body)) => {
                        // The subject renders in a single-line seg — GPUI
                        // panics on embedded newlines, so flatten defensively.
                        let subject = crate::data::oneline(subject);
                        if let Some(input) = &this.drawer.commit_input {
                            input.update(cx, |st, cx| st.set_value(subject.clone(), window, cx));
                        }
                        if let Some(input) = &this.drawer.commit_body_input {
                            input.update(cx, |st, cx| st.set_value(body.clone(), window, cx));
                        }
                        this.drawer.commit_suggestion = Some((subject.into(), body.into()));
                        this.push_toast(
                            ToastKind::Success,
                            "Commit message ready",
                            Some("Review subject/body, then Commit.".into()),
                            cx,
                        );
                    }
                    Err(e) => {
                        // Don't put the error into the suggestion card (that
                        // used to offer a broken "Commit this" on the error),
                        // and don't wipe the composer — leave any prior text.
                        this.drawer.commit_suggestion = None;
                        this.push_toast(
                            ToastKind::Error,
                            "Generate message failed",
                            Some(crate::data::oneline(e).into()),
                            cx,
                        );
                    }
                }
                cx.notify();
            });
            if let Err(err) = applied {
                eprintln!("[ai] drawer generate commit UI update failed: {err}");
                // Fall back without InputState fill so the user still gets a
                // toast / clears the stuck "Generating…" card.
                let _ = this.update(cx, |this, cx| {
                    if this.drawer.repo != repo {
                        return;
                    }
                    this.drawer.commit_suggestion = None;
                    this.push_toast(
                        ToastKind::Error,
                        "Generate message failed",
                        Some("Could not update the commit fields — try again.".into()),
                        cx,
                    );
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Fast commit: generate commit message (AI or heuristic fallback) and commit immediately (no push).
    pub fn fast_commit_repo(&mut self, repo_id: String, cx: &mut Context<Self>) {
        self.run_fleet_repos(crate::fleet::FleetOp::FastCommit, vec![repo_id], cx);
    }

    /// Drawer: fast commit — generate message (AI or heuristic fallback) and immediately commit.
    pub fn drawer_fast_commit(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let repo = self.drawer.repo.clone();
        let id = repo.to_string();
        self.fast_commit_repo(id, cx);
    }

    /// Drawer: generate a markdown changelog from recent commits (gated on
    /// `aiReady`). Lands in `drawer.changelog`.
    pub fn drawer_generate_changelog(&mut self, cx: &mut Context<Self>) {
        if !self.services.ai_ready {
            return;
        }
        let repo = self.drawer.repo.clone();
        self.drawer.changelog = Some("Generating…".into());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let id = repo.to_string();
            let commits = cx
                .background_executor()
                .spawn(async move { recent_summaries(&id, 30) })
                .await;
            let text =
                crate::task::run(async move { repoharbor_core::ai::changelog(&commits).await })
                    .await
                    .unwrap_or_else(|e| format!("Changelog failed: {e}"));
            let _ = this.update(cx, |this, cx| {
                if this.drawer.repo == repo {
                    this.drawer.changelog = Some(text.into());
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Drawer Notes tab: generate an AI "what changed" catch-up from recent
    /// commits (gated on `aiReady`). Lands in `drawer.notes.resume`.
    pub fn drawer_generate_resume(&mut self, cx: &mut Context<Self>) {
        if !self.services.ai_ready {
            return;
        }
        let repo = self.drawer.repo.clone();
        let name = repo.rsplit('/').next().unwrap_or(&repo).to_string();
        if let Some(n) = &mut self.drawer.notes {
            n.resume = Some("Generating…".into());
        }
        cx.notify();
        cx.spawn(async move |this, cx| {
            let id = repo.to_string();
            let commits = cx
                .background_executor()
                .spawn(async move { recent_summaries(&id, 30) })
                .await;
            let text =
                crate::task::run(async move { repoharbor_core::ai::resume(&name, &commits).await })
                    .await
                    .map(|m| m.trim().to_string())
                    .unwrap_or_else(|e| format!("Catch-up failed: {e}"));
            let _ = this.update(cx, |this, cx| {
                if this.drawer.repo == repo
                    && let Some(n) = &mut this.drawer.notes
                {
                    n.resume = Some(text.into());
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Drawer Changes tab: push the current branch (setting the upstream when
    /// there is none — see `git_ops::push`). Sync-but-network git work, so it
    /// runs on the background executor; progress and the outcome flow through
    /// one keyed toast ("Pushing…" → Pushed / Push failed).
    pub fn drawer_push(&mut self, cx: &mut Context<Self>) {
        if self.drawer.push_busy {
            return;
        }
        let repo = self.drawer.repo.clone();
        let name = repo.rsplit('/').next().unwrap_or(&repo).to_string();
        self.drawer.push_busy = true;
        self.log_action_cmd("git push", Some(&name), true, cx);
        let key = format!("push:{repo}");
        self.upsert_toast(
            key.clone(),
            ToastKind::Progress,
            format!("Pushing {name}…"),
            None,
            cx,
        );
        cx.spawn(async move |this, cx| {
            let id = repo.to_string();
            let result = cx
                .background_executor()
                .spawn(async move { repoharbor_core::git_ops::push(&id) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.drawer.repo == repo {
                    this.drawer.push_busy = false;
                }
                match result {
                    Ok(msg) => {
                        if this.drawer.repo == repo {
                            this.drawer.committed = false;
                        }
                        this.upsert_toast(key, ToastKind::Success, "Pushed", Some(msg.into()), cx);
                    }
                    Err(e) => {
                        this.upsert_toast(key, ToastKind::Error, "Push failed", Some(e.into()), cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Drawer: two-click `git reset --hard @{upstream}` (fetch + hard reset to
    /// `origin/<branch>`). First click arms the confirm label; second executes.
    pub fn drawer_reset_hard(&mut self, cx: &mut Context<Self>) {
        if self.drawer.reset_hard_busy {
            return;
        }
        if !self.drawer.reset_hard_armed {
            self.drawer.reset_hard_armed = true;
            cx.notify();
            return;
        }
        let repo = self.drawer.repo.clone();
        let name = repo.rsplit('/').next().unwrap_or(&repo).to_string();
        self.drawer.reset_hard_busy = true;
        self.drawer.reset_hard_armed = false;
        self.log_action_cmd(
            "git fetch && git reset --hard @{upstream}",
            Some(&name),
            true,
            cx,
        );
        let key = format!("reset-hard:{repo}");
        self.upsert_toast(
            key.clone(),
            ToastKind::Progress,
            format!("Resetting {name}…"),
            None,
            cx,
        );
        cx.spawn(async move |this, cx| {
            let id = repo.to_string();
            let result = cx
                .background_executor()
                .spawn(async move { repoharbor_core::git_ops::reset_hard_upstream(&id) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.drawer.repo == repo {
                    this.drawer.reset_hard_busy = false;
                }
                match result {
                    Ok(repoharbor_core::git_ops::OpOutcome::Done(msg)) => {
                        this.upsert_toast(
                            key,
                            ToastKind::Success,
                            "Reset hard",
                            Some(msg.into()),
                            cx,
                        );
                        this.rescan(cx);
                    }
                    Ok(repoharbor_core::git_ops::OpOutcome::Skipped(reason)) => {
                        this.upsert_toast(
                            key,
                            ToastKind::Info,
                            "Reset skipped",
                            Some(reason.into()),
                            cx,
                        );
                    }
                    Err(e) => {
                        this.upsert_toast(
                            key,
                            ToastKind::Error,
                            "Reset failed",
                            Some(e.into()),
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Drawer Changes tab: open a GitHub PR for the current branch — push
    /// first (ensuring the branch exists upstream), draft a title/body from
    /// the commit range (AI when `aiReady`, else the latest subject + commit
    /// list — the button works either way), then create the PR. Progress and
    /// the outcome flow through one keyed toast; the success toast links to
    /// the PR, as does a "View PR" affordance in the drawer.
    pub fn drawer_open_pr(&mut self, cx: &mut Context<Self>) {
        if self.drawer.pr_busy {
            return;
        }
        let repo = self.drawer.repo.clone();
        let Some(row) = self.rows.iter().find(|r| r.id == repo) else {
            return;
        };
        let slug = row.slug.to_string();
        let head = row.branch.to_string();
        let Some(base) = self.drawer.default_branch.as_ref().map(|b| b.to_string()) else {
            return;
        };
        if head == base {
            return;
        }
        let ai_ready = self.services.ai_ready;
        self.drawer.pr_busy = true;
        let key = format!("pr:{repo}");
        self.upsert_toast(
            key.clone(),
            ToastKind::Progress,
            "Opening PR…",
            Some(format!("{head} → {base}").into()),
            cx,
        );
        cx.spawn(async move |this, cx| {
            // 1. Ensure the branch is pushed (also sets the upstream if new).
            let id = repo.to_string();
            let pushed = cx
                .background_executor()
                .spawn(async move { repoharbor_core::git_ops::push(&id) })
                .await;
            if let Err(e) = pushed {
                let _ = this.update(cx, |this, cx| {
                    if this.drawer.repo == repo {
                        this.drawer.pr_busy = false;
                    }
                    this.upsert_toast(key, ToastKind::Error, "Push failed", Some(e.into()), cx);
                    cx.notify();
                });
                return;
            }
            // 2. The branch's commit range, for drafting the title/body.
            let (id, base2) = (repo.to_string(), base.clone());
            let commits: Vec<String> = cx
                .background_executor()
                .spawn(async move {
                    repoharbor_core::git_ops::commits_ahead_of(&id, &base2, 30)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|c| c.summary)
                        .collect()
                })
                .await;
            // 3. Title/body: AI-drafted when available, else the fallback —
            //    only the drafting is aiReady-gated, never the button.
            let drafted = if ai_ready && !commits.is_empty() {
                let (h, c) = (head.clone(), commits.clone());
                crate::task::run(async move { repoharbor_core::ai::pr_description(&h, &c).await })
                    .await
                    .ok()
                    .and_then(|d| repoharbor_core::ai::split_pr_draft(&d))
            } else {
                None
            };
            let (title, body) =
                drafted.unwrap_or_else(|| repoharbor_core::ai::fallback_pr_draft(&head, &commits));
            // 4. Create the PR (network → the shared tokio runtime).
            let (s, h, b) = (slug.clone(), head.clone(), base.clone());
            let (t2, b2) = (title.clone(), body.clone());
            let result = crate::task::run(async move {
                let token =
                    repoharbor_core::oauth::github_token().ok_or("connect GitHub to open a PR")?;
                repoharbor_core::forge::create_pr(&s, &h, &b, &t2, &b2, &token).await
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                if this.drawer.repo == repo {
                    this.drawer.pr_busy = false;
                }
                match result {
                    Ok(url) => {
                        if this.drawer.repo == repo {
                            this.drawer.pr_url = Some(url.clone().into());
                        }
                        this.upsert_toast_link(
                            key,
                            ToastKind::Success,
                            "PR opened",
                            Some(title.into()),
                            url.into(),
                            cx,
                        );
                    }
                    Err(e) => {
                        this.upsert_toast(
                            key,
                            ToastKind::Error,
                            "Open PR failed",
                            Some(e.into()),
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// One-shot at launch: if AI is enabled and reachable, mark it ready and
    /// kick off the semantic index (so Ctrl+K works without opening Settings).
    pub fn ai_startup(&mut self, cx: &mut Context<Self>) {
        if !self.config.ai_enabled {
            return;
        }
        cx.spawn(async move |this, cx| {
            let up = crate::task::run(repoharbor_core::ai::available()).await;
            let _ = this.update(cx, |this, _cx| {
                this.services.ai_ready = up;
                if up {
                    this.index_semantic();
                }
            });
        })
        .detach();
    }

    /// (Re)index the semantic corpus (readme/description/topics/notes/commits
    /// per repo) from the current snapshot, on the shared tokio runtime.
    /// Incremental and self-pacing: unchanged (repo, source) pairs skip via
    /// signatures and core throttles the embed bursts, so post-rescan calls
    /// are cheap. Gated on `aiReady` — no model, no indexing, no errors.
    pub fn index_semantic(&self) {
        if !self.services.ai_ready {
            return;
        }
        let repos = self.repos.clone();
        crate::task::spawn_detached(async move {
            let _ = repoharbor_core::semantic::index_fleet(repos).await;
        });
    }

    /// Pull (download) a model on the AI backend, then refresh the status.
    pub fn ai_pull(&mut self, model: String, cx: &mut Context<Self>) {
        use crate::views::settings::AiStatus;
        if model.trim().is_empty() || matches!(self.services.ai_status, AiStatus::Pulling(_)) {
            return;
        }
        self.services.ai_status = AiStatus::Pulling(format!("{model} · starting…").into());
        cx.notify();

        // The pull runs on the tokio runtime and streams (status, done, total)
        // back over a channel; a gpui task drains it to update the live %. When
        // the pull finishes the sender drops, closing the channel — our cue to
        // refresh the model list. (The one-shot `task::run` can't stream, hence
        // the detached spawn + channel.)
        let (tx, rx) = async_channel::unbounded::<(String, u64, u64)>();
        let m = model.clone();
        crate::task::spawn_detached(async move {
            let mut last_pct = u64::MAX;
            let _ = repoharbor_core::ai::pull(&m, |status, done, total| {
                // Throttle to ~1% steps (and every status-only update, total==0).
                match (done * 100).checked_div(total) {
                    Some(pct) if pct == last_pct => {}
                    pct => {
                        last_pct = pct.unwrap_or(u64::MAX);
                        let _ = tx.try_send((status.to_string(), done, total));
                    }
                }
            })
            .await;
        });

        cx.spawn(async move |this, cx| {
            while let Ok((status, done, total)) = rx.recv().await {
                let label = match (done * 100).checked_div(total) {
                    Some(pct) => format!("{model} · {pct}%"),
                    None => format!("{model} · {status}"),
                };
                if this
                    .update(cx, |this, cx| {
                        this.services.ai_status = AiStatus::Pulling(label.into());
                        cx.notify();
                    })
                    .is_err()
                {
                    return;
                }
            }
            // Channel closed → pull finished. Drop Pulling so the refresh isn't
            // short-circuited, then re-list models.
            let _ = this.update(cx, |this, cx| {
                this.services.ai_status = AiStatus::Unknown;
                this.ai_refresh(cx);
            });
        })
        .detach();
    }

    /// Remove a workspace root by draft index: update draft + live config,
    /// persist, toast, and rescan. No-op if the index is stale.
    pub fn settings_remove_root(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(s) = &self.settings else { return };
        if index >= s.draft.roots.len() {
            return;
        }
        let removed = s.draft.roots[index].clone();
        if let Some(s) = &mut self.settings {
            s.draft.roots.remove(index);
        }
        let removed_exp = repoharbor_core::scan::expand(&removed);
        self.config
            .roots
            .retain(|r| repoharbor_core::scan::expand(r) != removed_exp);
        if let Err(e) = repoharbor_core::config::save(&self.config) {
            self.push_toast(
                ToastKind::Error,
                "Save failed",
                Some(e.to_string().into()),
                cx,
            );
            cx.notify();
            return;
        }
        self.push_toast(
            ToastKind::Success,
            "Removed workspace root",
            Some(removed.into()),
            cx,
        );
        self.rescan_with_rearm(cx);
        cx.notify();
    }

    /// Append a pull-only prefix from the Settings input (expand ~).
    pub fn settings_add_pull_only(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(s) = &self.settings else { return };
        let raw = s.add_pull_only.read(cx).value().trim().to_string();
        if raw.is_empty() {
            return;
        }
        let store = repoharbor_core::scan::expand(&raw)
            .to_string_lossy()
            .into_owned();
        if let Some(s) = &mut self.settings {
            if !s
                .draft
                .pull_only_prefixes
                .iter()
                .any(|p| repoharbor_core::scan::expand(p) == repoharbor_core::scan::expand(&store))
            {
                s.draft.pull_only_prefixes.push(store.clone());
                s.saved = false;
            }
            // Re-adding clears a prior Settings removal so heal may treat it as
            // an intentional pull-only path again.
            repoharbor_core::config::clear_pull_only_opt_out(&mut s.draft, &store);
            let input = s.add_pull_only.clone();
            input.update(cx, |input, cx| {
                input.set_value("", window, cx);
            });
        }
        if let Some(s) = &self.settings {
            self.config.pull_only_prefixes = s.draft.pull_only_prefixes.clone();
            self.config.pull_only_opt_out = s.draft.pull_only_opt_out.clone();
        }
        let _ = repoharbor_core::config::save(&self.config);
        self.recompute_attention();
        cx.notify();
    }

    /// Remove a pull-only prefix by index and persist.
    pub fn settings_remove_pull_only(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(s) = &mut self.settings {
            if index < s.draft.pull_only_prefixes.len() {
                let removed = s.draft.pull_only_prefixes.remove(index);
                // Remember the removal so cold-load heal/seed won't undo it.
                repoharbor_core::config::note_pull_only_opt_out(&mut s.draft, &removed);
                s.saved = false;
            }
            self.config.pull_only_prefixes = s.draft.pull_only_prefixes.clone();
            self.config.pull_only_opt_out = s.draft.pull_only_opt_out.clone();
        }
        let _ = repoharbor_core::config::save(&self.config);
        self.recompute_attention();
        cx.notify();
    }

    /// Append a mute-attention prefix from the Settings input (expand ~).
    pub fn settings_add_mute_attention(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(s) = &self.settings else { return };
        let raw = s.add_mute_attention.read(cx).value().trim().to_string();
        if raw.is_empty() {
            return;
        }
        let store = repoharbor_core::scan::expand(&raw)
            .to_string_lossy()
            .into_owned();
        if let Some(s) = &mut self.settings {
            if !s
                .draft
                .mute_attention_prefixes
                .iter()
                .any(|p| repoharbor_core::scan::expand(p) == repoharbor_core::scan::expand(&store))
            {
                s.draft.mute_attention_prefixes.push(store.clone());
                s.saved = false;
            }
            let input = s.add_mute_attention.clone();
            input.update(cx, |input, cx| {
                input.set_value("", window, cx);
            });
        }
        self.config.mute_attention_prefixes = self
            .settings
            .as_ref()
            .map(|s| s.draft.mute_attention_prefixes.clone())
            .unwrap_or_default();
        let _ = repoharbor_core::config::save(&self.config);
        self.recompute_attention();
        cx.notify();
    }

    /// Remove a mute-attention prefix by index and persist.
    pub fn settings_remove_mute_attention(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(s) = &mut self.settings {
            if index < s.draft.mute_attention_prefixes.len() {
                s.draft.mute_attention_prefixes.remove(index);
                s.saved = false;
            }
            self.config.mute_attention_prefixes = s.draft.mute_attention_prefixes.clone();
        }
        let _ = repoharbor_core::config::save(&self.config);
        self.recompute_attention();
        cx.notify();
    }

    /// Append repo path(s) to `mute_attention_prefixes` and persist (context menu).
    pub fn mute_attention_for_repos(&mut self, repos: &[String], cx: &mut Context<Self>) {
        let mut added = 0usize;
        for raw in repos {
            let store = repoharbor_core::scan::expand(raw)
                .to_string_lossy()
                .into_owned();
            if store.is_empty() {
                continue;
            }
            let already =
                self.config.mute_attention_prefixes.iter().any(|p| {
                    repoharbor_core::scan::expand(p) == repoharbor_core::scan::expand(&store)
                });
            if already {
                continue;
            }
            self.config.mute_attention_prefixes.push(store.clone());
            if let Some(s) = &mut self.settings
                && !s.draft.mute_attention_prefixes.iter().any(|p| {
                    repoharbor_core::scan::expand(p) == repoharbor_core::scan::expand(&store)
                })
            {
                s.draft.mute_attention_prefixes.push(store);
                s.saved = false;
            }
            added += 1;
        }
        if added == 0 {
            self.push_toast(
                ToastKind::Info,
                "Already muted",
                Some("Those paths are already on the mute-attention list.".into()),
                cx,
            );
            cx.notify();
            return;
        }
        let _ = repoharbor_core::config::save(&self.config);
        self.recompute_attention();
        self.push_toast(
            ToastKind::Success,
            "Muted attention",
            Some(
                format!(
                    "Added {added} path{} — Needs me / chips / tray stay quiet for those repos.",
                    if added == 1 { "" } else { "s" }
                )
                .into(),
            ),
            cx,
        );
        cx.notify();
    }

    /// Open the external diff tool for a repo (and optional relative file).
    pub fn open_external_diff(&mut self, repo: &str, file: Option<&str>, cx: &mut Context<Self>) {
        let tmpl = if self.config.diff_command.trim().is_empty() {
            repoharbor_core::model::default_diff_command()
        } else {
            self.config.diff_command.clone()
        };
        let file = file.unwrap_or("");
        let cmd = tmpl.replace("{path}", repo).replace("{file}", file);
        match repoharbor_core::launch::launch(&cmd, repo) {
            Ok(()) => {}
            Err(e) => {
                self.push_toast(
                    ToastKind::Error,
                    "Couldn't open diff tool",
                    Some(e.into()),
                    cx,
                );
            }
        }
    }

    /// Open the configured IDE in `repo`.
    pub fn open_in_ide(&mut self, repo: &str, cx: &mut Context<Self>) {
        match repoharbor_core::launch::launch(&self.config.ide_command, repo) {
            Ok(()) => {}
            Err(e) => {
                self.push_toast(ToastKind::Error, "Couldn't open IDE", Some(e.into()), cx);
            }
        }
    }

    /// Append a typed workspace path to the draft + live config (smart detect),
    /// persist, toast, and rescan.
    pub fn settings_add_root(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(s) = &self.settings else { return };
        let val = s.add_root.read(cx).value().trim().to_string();
        if val.is_empty() {
            return;
        }
        let added = self.settings_add_root_paths(std::slice::from_ref(&val), cx);
        if added > 0 {
            let input = self.settings.as_ref().map(|s| s.add_root.clone());
            if let Some(input) = input {
                input.update(cx, |st, cx| st.set_value("", window, cx));
            }
        }
        cx.notify();
    }

    /// Open the native multi-folder picker and append any selected roots.
    pub fn settings_browse_roots(&mut self, cx: &mut Context<Self>) {
        if self.settings.is_none() {
            return;
        }
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async {
                    repoharbor_platform::folder_picker::pick_folders("Add workspace roots", "Add")
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(paths) if paths.is_empty() => {}
                    Ok(paths) => {
                        let raws: Vec<String> = paths
                            .iter()
                            .map(|p| p.to_string_lossy().into_owned())
                            .collect();
                        this.settings_add_root_paths(&raws, cx);
                    }
                    Err(e) => {
                        this.push_toast(
                            ToastKind::Error,
                            "Folder picker unavailable",
                            Some(e.into()),
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Validate + append one or more workspace roots (typed or browsed).
    /// Skips duplicates, persists once, toasts, and rescans when anything was
    /// added. Returns how many roots were newly appended.
    fn settings_add_root_paths(&mut self, raws: &[String], cx: &mut Context<Self>) -> usize {
        if raws.is_empty() || self.settings.is_none() {
            return 0;
        }

        let mut existing: Vec<String> = self
            .settings
            .as_ref()
            .map(|s| s.draft.roots.clone())
            .unwrap_or_else(|| self.config.roots.clone());

        let mut added: Vec<(String, bool)> = Vec::new();
        let mut first_err: Option<String> = None;
        let mut skipped_dup = 0usize;

        for raw in raws {
            match Self::prepare_workspace_root(raw, &existing) {
                Ok((store, is_repo)) => {
                    existing.push(store.clone());
                    added.push((store, is_repo));
                }
                Err(msg) if msg == "That path is already configured." => {
                    skipped_dup += 1;
                }
                Err(msg) => {
                    if first_err.is_none() {
                        first_err = Some(msg);
                    }
                }
            }
        }

        if added.is_empty() {
            if let Some(msg) = first_err {
                self.push_toast(ToastKind::Error, "Could not add path", Some(msg.into()), cx);
            } else if skipped_dup > 0 {
                self.push_toast(
                    ToastKind::Info,
                    "Already configured",
                    Some("Those folders are already in your workspace roots.".into()),
                    cx,
                );
            }
            return 0;
        }

        for (store, _) in &added {
            if let Some(s) = &mut self.settings {
                s.draft.roots.push(store.clone());
                s.saved = false;
            }
            if !self
                .config
                .roots
                .iter()
                .any(|r| repoharbor_core::scan::expand(r) == repoharbor_core::scan::expand(store))
            {
                self.config.roots.push(store.clone());
            }
        }

        if let Err(e) = repoharbor_core::config::save(&self.config) {
            self.push_toast(
                ToastKind::Error,
                "Save failed",
                Some(e.to_string().into()),
                cx,
            );
            return 0;
        }

        let n = added.len();
        let (title, detail) = if n == 1 {
            let (store, is_repo) = &added[0];
            if *is_repo {
                (
                    "Added repo",
                    format!("Scanning {store} as a single repository."),
                )
            } else {
                ("Added scan root", format!("Scanning repos under {store}."))
            }
        } else {
            (
                "Added workspace roots",
                format!("Added {n} folders; scanning…"),
            )
        };
        self.push_toast(ToastKind::Success, title, Some(detail.into()), cx);
        if let Some(msg) = first_err {
            self.push_toast(ToastKind::Info, "Some paths skipped", Some(msg.into()), cx);
        }
        self.rescan_with_rearm(cx);
        n
    }

    /// Read the field inputs into the draft, persist it, and rescan.
    pub fn settings_save(&mut self, cx: &mut Context<Self>) {
        let Some(s) = &self.settings else { return };
        let mut draft = s.draft.clone();
        draft.ide_command = s.ide.read(cx).value().to_string();
        draft.agent_command = s.agent.read(cx).value().to_string();
        draft.agent_dispatch_args = s.agent_dispatch.read(cx).value().to_string();
        draft.diff_command = s.diff_command.read(cx).value().to_string();
        draft.terminal_shell = s.terminal_shell.read(cx).value().trim().to_string();
        draft.ollama_host = s.ollama_host.read(cx).value().to_string();
        draft.ai_model = s.ai_model.read(cx).value().to_string();
        draft.embed_model = s.embed_model.read(cx).value().to_string();
        draft.llama_server_path = s.llama_server.read(cx).value().to_string();
        draft.openai_base = s.openai_base.read(cx).value().to_string();
        let is_or = draft.ai_backend == "openrouter";
        let is_groq = draft.ai_backend == "groq";
        let is_mistral = draft.ai_backend == "mistral";
        if is_or && draft.openai_base.trim().is_empty() {
            draft.openai_base = repoharbor_core::cloud::OPENROUTER_BASE_URL.to_string();
        }
        if is_or && draft.ai_model.trim().is_empty() {
            draft.ai_model = "deepseek/deepseek-chat".to_string();
        }
        if is_groq && draft.openai_base.trim().is_empty() {
            draft.openai_base = repoharbor_core::cloud::GROQ_BASE_URL.to_string();
        }
        if is_groq && draft.ai_model.trim().is_empty() {
            draft.ai_model = "openai/gpt-oss-120b".to_string();
        }
        if is_mistral && draft.openai_base.trim().is_empty() {
            draft.openai_base = repoharbor_core::cloud::MISTRAL_BASE_URL.to_string();
        }
        if is_mistral && draft.ai_model.trim().is_empty() {
            draft.ai_model = "codestral-latest".to_string();
        }
        // The key never round-trips through the draft or config.toml: a typed
        // value replaces the stored file, an empty field keeps what's there.
        let target = if is_or {
            "openrouter"
        } else if is_groq {
            "groq"
        } else if is_mistral {
            "mistral"
        } else {
            "cloud"
        };
        let typed_key = s.openai_key.read(cx).value().to_string();
        let key_note = if typed_key.trim().is_empty() {
            None
        } else {
            Some(
                match repoharbor_core::cloud::store_api_key_for_target(&typed_key, target) {
                    Ok(()) => match target {
                        "openrouter" => "OpenRouter API key saved.".to_string(),
                        "groq" => "Groq API key saved.".to_string(),
                        "mistral" => "Mistral API key saved.".to_string(),
                        _ => "Cloud API key saved.".to_string(),
                    },
                    Err(e) => format!("Could not save the API key: {e}"),
                },
            )
        };
        draft.github_client_id = s.client_id.read(cx).value().to_string();
        draft.ignore = s
            .ignore
            .read(cx)
            .value()
            .split(',')
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect();
        draft.scan_depth = s
            .scan_depth
            .read(cx)
            .value()
            .trim()
            .parse::<usize>()
            .unwrap_or(draft.scan_depth)
            .clamp(1, 8);
        // Chrome / groups are edited outside Settings — keep the live values.
        draft.sidebar_width = self.config.sidebar_width;
        draft.sidebar_collapsed = self.config.sidebar_collapsed;
        draft.terminal_height = self.config.terminal_height;
        draft.layout = self.config.layout;
        draft.workspace_groups = self.config.workspace_groups.clone();
        draft.active_workspace_group = self.config.active_workspace_group.clone();
        Self::normalize_and_migrate_pool(&mut draft.model_pool);

        let _ = repoharbor_core::config::save(&draft);
        self.config = draft.clone();
        if let Some(s) = &mut self.settings {
            s.draft = draft;
            s.saved = true;
            if let Some(note) = key_note {
                s.ai_note = note.into();
            }
        }
        self.rescan_with_rearm(cx);
        self.ai_refresh(cx);
        cx.notify();
    }

    /// Load the activity/release feed over the network.
    pub fn load_feed(&mut self, cx: &mut Context<Self>) {
        use crate::views::feed::{FeedState, feed_row};
        self.feed = FeedState::Loading;
        cx.notify();
        let now = crate::data::now_unix();
        // Items newer than the last time the Feed was viewed are "new". Read the
        // mark before the load, then advance it to now so the next visit compares
        // against this one.
        let since = repoharbor_core::cache::get_meta("feed_seen_at")
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);
        cx.spawn(async move |this, cx| {
            let res = crate::task::run(async { repoharbor_core::inbox::github_feed().await }).await;
            let _ = this.update(cx, |this, cx| {
                this.feed = match res {
                    Ok(items) => {
                        cx.background_executor()
                            .spawn(async move {
                                repoharbor_core::cache::set_meta("feed_seen_at", &now.to_string());
                            })
                            .detach();
                        FeedState::Ready(
                            items.into_iter().map(|f| feed_row(f, now, since)).collect(),
                        )
                    }
                    Err(e) => FeedState::Error(e.into()),
                };
                cx.notify();
            });
        })
        .detach();
    }

    /// Load the starred-repo browser over the network.
    pub fn load_starred(&mut self, cx: &mut Context<Self>) {
        use crate::views::explore::{ExploreState, star_row};
        self.explore = ExploreState::Loading;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let res =
                crate::task::run(async { repoharbor_core::inbox::github_starred().await }).await;
            let _ = this.update(cx, |this, cx| {
                this.explore = match res {
                    Ok(repos) => ExploreState::Ready(repos.into_iter().map(star_row).collect()),
                    Err(e) => ExploreState::Error(e.into()),
                };
                cx.notify();
            });
        })
        .detach();
    }

    /// Clone a starred repo into the first root, then rescan so it appears.
    pub fn clone_starred(
        &mut self,
        slug: SharedString,
        clone_url: SharedString,
        name: SharedString,
        cx: &mut Context<Self>,
    ) {
        let Some(root) = self.config.roots.first().cloned() else {
            return;
        };
        self.explore_cloning.insert(slug.clone());
        self.explore_errors.remove(&slug);
        // A keyed Progress toast tracks the clone; the completion below upserts
        // the same key to Success/Error, so the op resolves its own toast.
        let toast_key = SharedString::from(format!("clone:{slug}"));
        self.upsert_toast(
            toast_key.clone(),
            crate::toast::ToastKind::Progress,
            format!("Cloning {slug}…"),
            None,
            cx,
        );
        let dest = repoharbor_core::scan::expand(&root)
            .join(name.as_ref())
            .to_string_lossy()
            .into_owned();
        let url = clone_url.to_string();
        cx.spawn(async move |this, cx| {
            let (result, snap) = cx
                .background_executor()
                .spawn(async move {
                    let result = if std::path::Path::new(&dest).exists() {
                        Ok(())
                    } else {
                        repoharbor_core::git_ops::clone(&url, &dest).map(|_| ())
                    };
                    (result, crate::data::rescan())
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                use crate::toast::ToastKind;
                this.apply_snapshot(snap);
                this.explore_cloning.remove(&slug);
                match result {
                    Ok(()) => {
                        this.upsert_toast(
                            toast_key,
                            ToastKind::Success,
                            format!("Cloned {slug}"),
                            None,
                            cx,
                        );
                    }
                    Err(e) => {
                        let msg = SharedString::from(format!("Failed: {e}"));
                        // Keep the inline card error AND raise a toast, so the
                        // failure is visible even away from the Explore view.
                        this.explore_errors.insert(slug.clone(), msg.clone());
                        this.upsert_toast(
                            toast_key,
                            ToastKind::Error,
                            format!("Clone failed: {slug}"),
                            Some(msg),
                            cx,
                        );
                    }
                }
                // Watch the freshly cloned repo for live changes too.
                this.watcher.rearm();
                cx.notify();
            });
        })
        .detach();
    }

    /// Scan all repos for prunable branches (sync git, off-thread).
    pub fn load_cleanup(&mut self, cx: &mut Context<Self>) {
        use crate::views::cleanup::CleanupState;
        self.cleanup = CleanupState::Loading;
        self.cleanup_confirm = None;
        cx.notify();
        let rows = self.rows.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { crate::views::cleanup::scan(&rows) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.cleanup = CleanupState::Ready(result);
                // Fresh prunable counts → refresh the attention surfaces.
                this.recompute_attention();
                cx.notify();
            });
        })
        .detach();
    }

    /// Scan the machine for running agent sessions (off the UI thread).
    /// Initial load when the Agents view first opens: scan with the spinner, then
    /// start the live poll.
    pub fn load_agents(&mut self, cx: &mut Context<Self>) {
        self.scan_agents(true, cx);
        self.start_agents_poll(cx);
    }

    /// Scan running agents off the UI thread, then update both the Agents list and
    /// the `active_agents` set (which drives the card indicator). `loading` shows
    /// the spinner; the poll passes `false` to refresh in place. Only repaints when
    /// the active set changed or the Agents view is showing.
    fn scan_agents(&mut self, loading: bool, cx: &mut Context<Self>) {
        use crate::views::agents::AgentsState;
        if loading {
            self.agents = AgentsState::Loading;
            cx.notify();
        }
        let rows = self.rows.clone();
        let agent_command = self.config.agent_command.clone();
        cx.spawn(async move |this, cx| {
            let mut result = cx
                .background_executor()
                .spawn(async move { crate::views::agents::scan(&rows, &agent_command) })
                .await;
            let _ = this.update(cx, |this, cx| {
                // Card indicator: repos with a live session, including the
                // origin repo of any live dispatched-worktree session.
                let active: std::collections::HashSet<SharedString> = result
                    .sessions
                    .iter()
                    .map(|a| a.repo.clone())
                    .chain(
                        result
                            .dispatched
                            .iter()
                            .filter(|d| d.pid.is_some())
                            .map(|d| d.origin.clone()),
                    )
                    .collect();
                let changed = active != this.active_agents;
                let supervising = result.dispatched.iter().any(|d| d.pid.is_some());
                let finished = std::mem::take(&mut result.finished_now);
                // Collapse the review panel if its worktree vanished (e.g.
                // discarded from another surface).
                if let Some(open) = &this.agents_review
                    && !result.dispatched.iter().any(|d| d.worktree_path == *open)
                {
                    this.agents_review = None;
                    this.agents_review_diff = None;
                }
                this.active_agents = active;
                this.agents = AgentsState::Ready(result);
                // Fresh agent facts → refresh the attention surfaces (cheap;
                // the notify below only fires when something visible changed).
                this.recompute_attention();
                // Surface the one-shot finish events (#185): always a toast;
                // a desktop notification when config allows. AgentFinished is
                // Attention-tier (not Urgent, so the urgent-diff notifier
                // skips it), but a finished dispatch is the whole point of
                // the session — it notifies from the transition itself,
                // gated by `notify_agent_finished`. The persisted
                // `finished_at` means each finish fires exactly once, across
                // polls and restarts alike.
                for f in finished {
                    let commits = if f.commits == 1 {
                        "1 commit".to_string()
                    } else {
                        format!("{} commits", f.commits)
                    };
                    let title = format!("Agent finished on {}", f.origin_name);
                    let body = format!("{commits} on {} — review in Agents", f.branch);
                    this.push_toast(
                        crate::toast::ToastKind::Info,
                        title.clone(),
                        Some(body.clone().into()),
                        cx,
                    );
                    if this.config.notify_enabled
                        && this.config.notify_attention
                        && this.config.notify_agent_finished
                    {
                        crate::task::spawn_detached(async move {
                            let _ = repoharbor_platform::notify::send(&title, &body).await;
                        });
                    }
                }
                // Keep supervising live dispatched sessions even when the
                // Agents view isn't open, so the finish transition is
                // observed promptly (no-op if the poll is already running).
                if supervising {
                    this.start_agents_poll(cx);
                }
                if changed || this.view == View::Agents {
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Launch-time outcome sweep (#185): scan agent sessions once so a
    /// dispatched session that ended while RepoHarbor was closed is detected
    /// right away (persisted `last_seen_alive` + no live process = finished),
    /// and — via the scan's self-healing poll start — keep supervising any
    /// dispatched session that is still running.
    pub fn agents_startup(&mut self, cx: &mut Context<Self>) {
        self.scan_agents(false, cx);
    }

    /// Re-scan agents every 5s while the Agents view is open — so the list
    /// stays live (terminated sessions drop off, new ones appear) — or while
    /// a dispatched-worktree session is running, so its finish transition is
    /// observed even with the view closed (#185: the attention item and
    /// notification must not depend on the user watching the Agents view).
    /// Exits when neither holds; restarted by `maybe_load_view` on re-entry
    /// and by any scan that sees a live dispatched session.
    fn start_agents_poll(&mut self, cx: &mut Context<Self>) {
        if self.agents_polling {
            return;
        }
        self.agents_polling = true;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(5))
                    .await;
                let keep = this
                    .update(cx, |this, _| {
                        this.view == View::Agents || this.supervising_dispatch()
                    })
                    .unwrap_or(false);
                if !keep {
                    let _ = this.update(cx, |this, _| this.agents_polling = false);
                    break;
                }
                if this
                    .update(cx, |this, cx| this.scan_agents(false, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    /// Is a dispatched-worktree agent session currently alive? While true the
    /// agents poll keeps running off-view to catch the finish transition.
    fn supervising_dispatch(&self) -> bool {
        match &self.agents {
            crate::views::agents::AgentsState::Ready(data) => {
                data.dispatched.iter().any(|d| d.pid.is_some())
            }
            _ => false,
        }
    }

    /// Terminate an agent process by pid, then re-scan the list.
    pub fn terminate_agent(&mut self, pid: u32, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .spawn(async move { repoharbor_platform::agents::terminate(pid) })
                .await;
            let _ = this.update(cx, |this, cx| this.scan_agents(false, cx));
        })
        .detach();
    }

    /// Toggle the drawer's "fresh worktree" dispatch option.
    pub fn toggle_dispatch_fresh(&mut self, cx: &mut Context<Self>) {
        self.drawer.dispatch_fresh = !self.drawer.dispatch_fresh;
        cx.notify();
    }

    /// Dispatch a coding agent onto `repo` with a task `prompt` (#185). Plain
    /// dispatch launches `agent_command` + `agent_dispatch_args` in the repo
    /// directory, exactly like the drawer's Agent button plus the prompt as an
    /// argument. With `fresh`, a branch `agent/<slug>-<rand>` and a worktree
    /// under the app data dir are created first (sync git — background
    /// executor), the pairing is recorded in the cache, and the agent starts in
    /// the worktree. Success/failure lands as a toast either way.
    pub fn dispatch_agent(
        &mut self,
        repo: SharedString,
        prompt: String,
        fresh: bool,
        cx: &mut Context<Self>,
    ) {
        use crate::toast::ToastKind;
        let cmd = self.config.agent_command.clone();
        let args = self.config.agent_dispatch_args.clone();

        if !fresh {
            if self.config.terminal_dispatch_embedded {
                let title = repo_tab_title(repo.as_ref());
                let embedded_cmd = build_embedded_agent_command(&cmd, &args, &prompt);
                let run_now = embedded_cmd.is_some();
                self.open_terminal_tab(
                    repo.as_ref(),
                    &title,
                    embedded_cmd.as_deref(),
                    run_now,
                    false,
                    None,
                    cx,
                );
                self.push_toast(
                    ToastKind::Success,
                    if run_now {
                        "Agent started in built-in terminal"
                    } else {
                        "Built-in terminal opened (configure CLI agent to auto-run)"
                    },
                    Some(repo.clone()),
                    cx,
                );
                return;
            }
            match repoharbor_core::launch::spawn_with_prompt(&cmd, &args, &repo, &prompt) {
                Ok(_) => {
                    self.push_toast(
                        ToastKind::Success,
                        "Agent dispatched",
                        Some(repo.clone()),
                        cx,
                    );
                }
                Err(e) => {
                    self.push_toast(ToastKind::Error, "Dispatch failed", Some(e.into()), cx);
                }
            }
            return;
        }

        let names = repoharbor_core::dispatch::names(&prompt);
        let Some(dest) = repoharbor_core::dispatch::worktree_dest(&repo, &names.worktree) else {
            self.push_toast(
                ToastKind::Error,
                "Dispatch failed",
                Some("no data directory for worktrees".into()),
                cx,
            );
            return;
        };
        let toast_key = SharedString::from(format!("dispatch:{repo}"));
        self.upsert_toast(
            toast_key.clone(),
            ToastKind::Progress,
            format!("Creating worktree on {}…", names.branch),
            Some(repo.clone()),
            cx,
        );

        let branch = names.branch.clone();
        let id = repo.to_string();
        let now = crate::data::now_unix();
        cx.spawn(async move |this, cx| {
            // Worktree add is sync libgit2 work — keep it off the UI thread.
            let (id2, name, branch2, prompt2, dest2) = (
                id.clone(),
                names.worktree.clone(),
                names.branch.clone(),
                prompt.clone(),
                dest.to_string_lossy().into_owned(),
            );
            let created: Result<String, String> = cx
                .background_executor()
                .spawn(async move {
                    if let Some(parent) = std::path::Path::new(&dest2).parent() {
                        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                    }
                    let wt_path = repoharbor_core::git_ops::add_worktree_on_branch(
                        &id2, &name, &branch2, &dest2,
                    )?;
                    repoharbor_core::cache::record_agent_worktree(
                        &repoharbor_core::cache::AgentWorktree {
                            worktree_path: wt_path.clone(),
                            repo_id: id2,
                            branch: branch2,
                            worktree_name: name,
                            prompt: prompt2,
                            created_at: now,
                            last_seen_alive: 0,
                            finished_at: 0,
                            commits_ahead: 0,
                            pr_url: String::new(),
                        },
                    )?;
                    Ok(wt_path)
                })
                .await;

            let _ = this.update(cx, |this, cx| {
                use crate::toast::ToastKind;
                match created {
                    Ok(wt_path) => {
                        if this.config.terminal_dispatch_embedded {
                            let embedded_cmd = build_embedded_agent_command(
                                &this.config.agent_command,
                                &this.config.agent_dispatch_args,
                                &prompt,
                            );
                            let run_now = embedded_cmd.is_some();
                            this.open_terminal_tab(
                                &wt_path,
                                &branch,
                                embedded_cmd.as_deref(),
                                run_now,
                                false,
                                None,
                                cx,
                            );
                            this.upsert_toast(
                                toast_key,
                                ToastKind::Success,
                                if run_now {
                                    "Agent started in built-in terminal worktree"
                                } else {
                                    "Worktree ready in built-in terminal"
                                },
                                Some(branch.into()),
                                cx,
                            );
                        } else {
                            let launched = repoharbor_core::launch::spawn_with_prompt(
                                &this.config.agent_command,
                                &this.config.agent_dispatch_args,
                                &wt_path,
                                &prompt,
                            );
                            match launched {
                                Ok(_) => this.upsert_toast(
                                    toast_key,
                                    ToastKind::Success,
                                    "Agent dispatched on fresh worktree",
                                    Some(branch.into()),
                                    cx,
                                ),
                                Err(e) => this.upsert_toast(
                                    toast_key,
                                    ToastKind::Error,
                                    "Worktree created, but the agent failed to launch",
                                    Some(e.into()),
                                    cx,
                                ),
                            };
                        }
                        // Show the new worktree in the open drawer's Overview
                        // and the Agents view without waiting for the poll.
                        if this.drawer.repo == repo {
                            crate::drawer::load_overview(repo.clone(), cx);
                        }
                        this.scan_agents(false, cx);
                    }
                    Err(e) => {
                        this.upsert_toast(
                            toast_key,
                            ToastKind::Error,
                            "Dispatch failed",
                            Some(e.into()),
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// First click on a dispatched worktree's "Remove worktree" button: arm the
    /// two-stage confirm (same pattern as [`Self::arm_prune`]).
    pub fn arm_worktree_remove(&mut self, path: SharedString, cx: &mut Context<Self>) {
        self.agents_confirm = Some(path);
        self.agents_confirm_gen += 1;
        let generation = self.agents_confirm_gen;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(4))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.agents_confirm_gen == generation && this.agents_confirm.take().is_some() {
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Remove a dispatched agent worktree: refused (with a toast) while it has
    /// uncommitted changes, otherwise unlink it from the origin repo, delete
    /// its directory (it lives under our data dir), and forget the pairing.
    /// The `agent/…` branch is kept — it holds the agent's commits. Only
    /// reached via the two-stage confirm ([`Self::arm_worktree_remove`]).
    pub fn remove_dispatch_worktree(
        &mut self,
        path: SharedString,
        origin: SharedString,
        name: SharedString,
        cx: &mut Context<Self>,
    ) {
        self.agents_confirm = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let (p, o, n) = (path.to_string(), origin.to_string(), name.to_string());
            let result: Result<(), String> = cx
                .background_executor()
                .spawn(async move {
                    let dirty = !repoharbor_core::git_ops::changes(&p)
                        .map_err(|e| format!("can't inspect worktree: {e}"))?
                        .is_empty();
                    if dirty {
                        return Err("the worktree has uncommitted changes".into());
                    }
                    repoharbor_core::git_ops::remove_worktree(&o, &n)?;
                    // Unlinking leaves the files; this worktree is ours (under
                    // the app data dir), so clean up the directory too.
                    std::fs::remove_dir_all(&p).map_err(|e| e.to_string())?;
                    repoharbor_core::cache::remove_agent_worktree(&p)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                use crate::toast::ToastKind;
                match result {
                    Ok(()) => this.push_toast(
                        ToastKind::Success,
                        "Worktree removed",
                        Some(path.clone()),
                        cx,
                    ),
                    Err(e) => this.push_toast(
                        ToastKind::Error,
                        "Worktree not removed",
                        Some(e.into()),
                        cx,
                    ),
                };
                this.scan_agents(false, cx);
            });
        })
        .detach();
    }

    /// Toggle the inline review diff for a finished dispatched worktree
    /// (#185): the `agent/…` branch's changes vs the origin's default branch
    /// (merge-base scoped — see `git_ops::branch_diff`), loaded off the UI
    /// thread and rendered with the drawer's capped diff renderer. One panel
    /// at a time; toggling another card's Review replaces it.
    pub fn toggle_agent_review(
        &mut self,
        path: SharedString,
        origin: SharedString,
        cx: &mut Context<Self>,
    ) {
        if self.agents_review.as_ref() == Some(&path) {
            self.agents_review = None;
            self.agents_review_diff = None;
            cx.notify();
            return;
        }
        self.agents_review = Some(path.clone());
        self.agents_review_diff = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let (p, o) = (path.to_string(), origin.to_string());
            let diff = cx
                .background_executor()
                .spawn(async move {
                    let base = repoharbor_core::git_ops::default_branch(&o)
                        .ok_or("origin has no default branch")?;
                    repoharbor_core::git_ops::branch_diff(&p, &base)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                // Dropped if the panel moved to another card meanwhile.
                if this.agents_review.as_ref() == Some(&path) {
                    this.agents_review_diff =
                        Some(diff.unwrap_or_else(|e| format!("Diff failed: {e}")).into());
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Land a finished dispatched worktree as a PR (#185): push the `agent/…`
    /// branch from the worktree (its HEAD — sets the upstream on first push),
    /// draft a title/body from the commit range (AI when `aiReady`, fallback
    /// otherwise), create the PR against the origin's default branch, and
    /// persist the PR URL on the pairing row — which clears the worktree's
    /// AgentFinished attention item (the work is handed off). Mirrors
    /// [`Self::drawer_open_pr`], re-targeted at the worktree.
    pub fn open_worktree_pr(
        &mut self,
        path: SharedString,
        origin: SharedString,
        branch: SharedString,
        cx: &mut Context<Self>,
    ) {
        if self.agents_pr_busy.contains(&path) {
            return;
        }
        let Some(row) = self.rows.iter().find(|r| r.id == origin) else {
            self.push_toast(
                ToastKind::Error,
                "Open PR failed",
                Some("origin repo is not in the fleet".into()),
                cx,
            );
            return;
        };
        let Some(slug) = crate::drawer::github_slug(row) else {
            self.push_toast(
                ToastKind::Error,
                "Open PR failed",
                Some("origin has no GitHub remote".into()),
                cx,
            );
            return;
        };
        let slug = slug.to_string();
        let ai_ready = self.services.ai_ready;
        self.agents_pr_busy.insert(path.clone());
        let key = format!("wt-pr:{path}");
        self.upsert_toast(
            key.clone(),
            ToastKind::Progress,
            "Opening PR…",
            Some(branch.clone()),
            cx,
        );
        cx.spawn(async move |this, cx| {
            // 1. Resolve the base and push the branch from the worktree.
            let (p, o) = (path.to_string(), origin.to_string());
            let pushed: Result<String, String> = cx
                .background_executor()
                .spawn(async move {
                    let base = repoharbor_core::git_ops::default_branch(&o)
                        .ok_or("origin has no default branch")?;
                    repoharbor_core::git_ops::push(&p)?;
                    Ok(base)
                })
                .await;
            let base = match pushed {
                Ok(base) => base,
                Err(e) => {
                    let _ = this.update(cx, |this, cx| {
                        this.agents_pr_busy.remove(&path);
                        this.upsert_toast(key, ToastKind::Error, "Push failed", Some(e.into()), cx);
                        cx.notify();
                    });
                    return;
                }
            };
            // 2. The branch's commit range, for drafting the title/body.
            let (p2, base2) = (path.to_string(), base.clone());
            let commits: Vec<String> = cx
                .background_executor()
                .spawn(async move {
                    repoharbor_core::git_ops::commits_ahead_of(&p2, &base2, 30)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|c| c.summary)
                        .collect()
                })
                .await;
            // 3. Title/body: AI-drafted when available, else the fallback.
            let head = branch.to_string();
            let drafted = if ai_ready && !commits.is_empty() {
                let (h, c) = (head.clone(), commits.clone());
                crate::task::run(async move { repoharbor_core::ai::pr_description(&h, &c).await })
                    .await
                    .ok()
                    .and_then(|d| repoharbor_core::ai::split_pr_draft(&d))
            } else {
                None
            };
            let (title, body) =
                drafted.unwrap_or_else(|| repoharbor_core::ai::fallback_pr_draft(&head, &commits));
            // 4. Create the PR (network → the shared tokio runtime).
            let (s, h, b) = (slug.clone(), head.clone(), base.clone());
            let (t2, b2) = (title.clone(), body.clone());
            let result = crate::task::run(async move {
                let token =
                    repoharbor_core::oauth::github_token().ok_or("connect GitHub to open a PR")?;
                repoharbor_core::forge::create_pr(&s, &h, &b, &t2, &b2, &token).await
            })
            .await;
            // 5. Persist the hand-off so the attention item clears (and stays
            //    cleared across restarts), then refresh the cards.
            if let Ok(url) = &result {
                let (p3, u) = (path.to_string(), url.clone());
                cx.background_executor()
                    .spawn(async move {
                        let _ = repoharbor_core::cache::set_agent_worktree_pr(&p3, &u);
                    })
                    .await;
            }
            let _ = this.update(cx, |this, cx| {
                this.agents_pr_busy.remove(&path);
                match result {
                    Ok(url) => {
                        this.upsert_toast_link(
                            key,
                            ToastKind::Success,
                            "PR opened",
                            Some(title.into()),
                            url.into(),
                            cx,
                        );
                        this.scan_agents(false, cx);
                    }
                    Err(e) => {
                        this.upsert_toast(
                            key,
                            ToastKind::Error,
                            "Open PR failed",
                            Some(e.into()),
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Discard a finished dispatched worktree's outcome (#185): remove the
    /// worktree AND delete its `agent/…` branch — the commits go with it,
    /// which is the point (contrast [`Self::remove_dispatch_worktree`], which
    /// keeps the branch). Refused while the worktree has uncommitted changes,
    /// like the plain removal. Only reached via the two-stage confirm
    /// (`arm_worktree_remove` with the `discard:`-prefixed key). Dropping the
    /// cache row clears the AgentFinished attention item.
    pub fn discard_dispatch_worktree(
        &mut self,
        path: SharedString,
        origin: SharedString,
        name: SharedString,
        branch: SharedString,
        cx: &mut Context<Self>,
    ) {
        self.agents_confirm = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let (p, o, n, b) = (
                path.to_string(),
                origin.to_string(),
                name.to_string(),
                branch.to_string(),
            );
            let result: Result<(), String> = cx
                .background_executor()
                .spawn(async move {
                    let dirty = !repoharbor_core::git_ops::changes(&p)
                        .map_err(|e| format!("can't inspect worktree: {e}"))?
                        .is_empty();
                    if dirty {
                        return Err("the worktree has uncommitted changes".into());
                    }
                    // Unlink first so the branch is no longer checked out
                    // anywhere (delete_branch refuses otherwise), then drop
                    // the branch, the directory (ours, under the app data
                    // dir), and the pairing row.
                    repoharbor_core::git_ops::remove_worktree(&o, &n)?;
                    repoharbor_core::git_ops::delete_branch(&o, &b)?;
                    std::fs::remove_dir_all(&p).map_err(|e| e.to_string())?;
                    repoharbor_core::cache::remove_agent_worktree(&p)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                use crate::toast::ToastKind;
                match result {
                    Ok(()) => this.push_toast(
                        ToastKind::Success,
                        "Worktree discarded",
                        Some(branch.clone()),
                        cx,
                    ),
                    Err(e) => {
                        this.push_toast(ToastKind::Error, "Discard failed", Some(e.into()), cx)
                    }
                };
                if this.agents_review.as_ref() == Some(&path) {
                    this.agents_review = None;
                    this.agents_review_diff = None;
                }
                this.scan_agents(false, cx);
            });
        })
        .detach();
    }

    /// First click on a Cleanup "Prune" button: arm a two-stage confirm for that
    /// repo (the button flips to "Confirm prune {n}?"). Deleting branches is
    /// irreversible, so it only runs on the confirming second click. The armed
    /// state reverts after a few seconds, and arming another repo's button
    /// replaces it.
    pub fn arm_prune(&mut self, id: SharedString, cx: &mut Context<Self>) {
        self.cleanup_confirm = Some(id);
        self.cleanup_confirm_gen += 1;
        let generation = self.cleanup_confirm_gen;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(4))
                .await;
            // Revert only if this timer's arm is still the active one.
            let _ = this.update(cx, |this, cx| {
                if this.cleanup_confirm_gen == generation && this.cleanup_confirm.take().is_some() {
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Prune the given repo's stale branches, then refresh the Cleanup list.
    /// Only reached via the two-stage confirm in the Cleanup view (`arm_prune`).
    pub fn prune_repo(&mut self, id: SharedString, cx: &mut Context<Self>) {
        self.cleanup_confirm = None;
        cx.notify();
        let path = id.to_string();
        let title = repo_tab_title(&path);
        self.log_action_cmd("git branch -d (prune gone/merged)", Some(&title), true, cx);
        cx.spawn(async move |this, cx| {
            let pruned = cx
                .background_executor()
                .spawn(async move { repoharbor_core::git_ops::prune_branches(&path) })
                .await;
            let Ok(rows) = this.update(cx, |this, cx| {
                match &pruned {
                    Ok(names) if names.is_empty() => {
                        this.push_toast(
                            ToastKind::Info,
                            "Nothing to prune",
                            Some("No merged or gone branch was free to delete.".into()),
                            cx,
                        );
                    }
                    Ok(names) => {
                        this.push_toast(
                            ToastKind::Success,
                            "Pruned branches",
                            Some(names.join(", ").into()),
                            cx,
                        );
                    }
                    Err(e) => {
                        this.push_toast(
                            ToastKind::Error,
                            "Prune failed",
                            Some(e.clone().into()),
                            cx,
                        );
                    }
                }
                this.rows.clone()
            }) else {
                return;
            };
            let result = cx
                .background_executor()
                .spawn(async move { crate::views::cleanup::scan(&rows) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.cleanup = crate::views::cleanup::CleanupState::Ready(result);
                // Pruning changed the prunable counts → refresh attention.
                this.recompute_attention();
                cx.notify();
            });
        })
        .detach();
    }

    /// User-triggered workspace rescan (Refresh button / Palette): mirrors into Action Log.
    pub(crate) fn rescan_user(&mut self, cx: &mut Context<Self>) {
        self.log_action_cmd("repoharbor scan --roots", Some("workspace roots"), true, cx);
        self.rescan_with_rearm(cx);
    }

    /// Re-scan roots and re-arm the filesystem watcher (use when roots/repos may
    /// have been added/removed or on explicit manual Refresh).
    pub(crate) fn rescan_with_rearm(&mut self, cx: &mut Context<Self>) {
        self.rescan_inner(cx, true);
    }

    /// Re-scan the roots from disk (off the UI thread) and reload the grid, then
    /// refresh host enrichment. Does NOT tear down and rebuild filesystem watches.
    pub(crate) fn rescan(&mut self, cx: &mut Context<Self>) {
        self.rescan_inner(cx, false);
    }

    fn rescan_inner(&mut self, cx: &mut Context<Self>, rearm: bool) {
        if self.rescan_in_flight {
            self.rescan_queued = true;
            if rearm {
                self.rescan_rearm_pending = true;
            }
            return;
        }
        self.rescan_in_flight = true;
        if rearm {
            self.watcher.rearm();
        }
        self.activity_log.push(
            crate::data::now_unix(),
            crate::activity_log::LogLevel::Info,
            "Scan started",
        );
        self.upsert_toast(
            "rescan",
            ToastKind::Progress,
            "Scanning…",
            Some("Re-reading workspace roots".into()),
            cx,
        );
        cx.spawn(async move |this, cx| {
            let snap = cx
                .background_executor()
                .spawn(async { crate::data::rescan() })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.rescan_in_flight = false;
                let n = snap.rows.len();
                this.apply_snapshot(snap);
                // Drop selected ids for repos that vanished in the rescan.
                this.prune_selection();
                this.enrich_hosts(cx);
                this.refresh_ci(cx);
                this.load_activity(cx);
                // Refresh which repos have a live agent, so Mission Control shows
                // the indicator without needing the Agents view open.
                this.scan_agents(false, cx);
                // Fold the fresh snapshot into the semantic index (incremental;
                // a no-op with AI off/unreachable).
                this.index_semantic();
                this.activity_log.push(
                    crate::data::now_unix(),
                    crate::activity_log::LogLevel::Info,
                    format!("Scan finished — {n} repos"),
                );
                this.log_action_result(
                    "✓",
                    "32",
                    &format!("Scan finished — {n} repos"),
                    None,
                    false,
                    cx,
                );
                this.upsert_toast(
                    "rescan",
                    ToastKind::Success,
                    "Scan finished",
                    Some(format!("{n} repos").into()),
                    cx,
                );
                if this.rescan_queued {
                    let next_rearm = this.rescan_rearm_pending;
                    this.rescan_queued = false;
                    this.rescan_rearm_pending = false;
                    this.rescan_inner(cx, next_rearm);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Recompute the contribution graph (commits/day across all repos).
    /// No-op while the heatmap stays hidden — walking hundreds of repos is
    /// expensive and the Mission Control band is not shown.
    pub fn load_activity(&mut self, cx: &mut Context<Self>) {
        if (!self.grid.activity_open && self.grid.activity.is_none()) || self.activity_loading {
            // Keep idle: no graph, no overlapping background revwalk.
            let _ = cx;
            return;
        }
        self.activity_loading = true;
        let paths: Vec<String> = self.rows.iter().map(|r| r.id.to_string()).collect();
        cx.spawn(async move |this, cx| {
            let activity = cx
                .background_executor()
                .spawn(async move { repoharbor_core::activity::compute(&paths) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.activity_loading = false;
                this.grid.activity = Some(activity);
                cx.notify();
            });
        })
        .detach();
    }

    /// Set the active Mission Control quick filter.
    ///
    /// Visibility chips (Public / Private / Starred / Stale) are orthogonal:
    /// they AND with the work-mode filter and never switch Needs me / Behind /
    /// Working to All. Re-click toggles a visibility chip off.
    ///
    /// Clears the fleet selection so ops can't accidentally target repos that
    /// are no longer in the visible set after the chip change.
    pub fn set_filter(&mut self, f: RepoFilter, cx: &mut Context<Self>) {
        if is_visibility_filter(f) {
            self.grid.visibility = if self.grid.visibility == Some(f) {
                None
            } else {
                Some(f)
            };
            self.clear_selection_quiet();
            cx.notify();
            return;
        }
        if self.grid.filter == f {
            return;
        }
        self.grid.filter = f;
        self.grid.mode = WorkMode::from_filter(f);
        self.clear_selection_quiet();
        cx.notify();
    }

    /// Git/attention filter AND optional visibility overlay.
    fn row_matches_chips(&self, r: &crate::data::Row) -> bool {
        self.grid.filter.matches(r, &self.attention_by_repo)
            && self
                .grid
                .visibility
                .is_none_or(|v| v.matches(r, &self.attention_by_repo))
    }

    /// Switch Mission Control work mode (segmented control) and apply its
    /// primary filter.
    pub fn set_mode(&mut self, mode: WorkMode, cx: &mut Context<Self>) {
        if self.grid.mode == mode {
            // Re-clicking the active mode clears secondary refinements: Working
            // git chips and the orthogonal visibility overlay.
            let primary = mode.default_filter();
            let mut changed = false;
            if self.grid.filter != primary {
                self.grid.filter = primary;
                changed = true;
            }
            if self.grid.visibility.take().is_some() {
                changed = true;
            }
            if changed {
                self.clear_selection_quiet();
                cx.notify();
            }
            return;
        }
        self.grid.mode = mode;
        self.grid.filter = mode.default_filter();
        // Keep visibility overlay across mode switches (AND with the new mode).
        self.clear_selection_quiet();
        cx.notify();
    }

    /// Cycle the Mission Control sort order.
    pub fn cycle_sort(&mut self, cx: &mut Context<Self>) {
        self.grid.sort = self.grid.sort.next();
        cx.notify();
    }

    /// Switch the Mission Control layout (grid ↔ list) and persist it.
    pub fn set_layout(&mut self, layout: Layout, cx: &mut Context<Self>) {
        self.grid.layout = layout;
        self.config.layout = layout;
        let _ = repoharbor_core::config::save(&self.config);
        cx.notify();
    }

    /// How many Needs-me entries Mission Control can actually list: local
    /// repos that pass sidebar facets (with submodule reveal) plus host-only
    /// actionable facts. Matches the Needs me chip / empty-state invariant.
    fn attention_count(&self) -> usize {
        self.needs_me_listed_local_count() + host_only_needs_me(&self.attention_items).len()
    }

    /// Local rows that Needs me would show under the current root / language /
    /// group / TREE facets (submodule children in the attention index are
    /// revealed even without a TREE focus).
    fn needs_me_listed_local_count(&self) -> usize {
        self.rows
            .iter()
            .filter(|r| self.attention_by_repo.contains_key(&r.id))
            .filter(|r| self.row_passes_needs_me_facets(r))
            .count()
    }

    /// Sidebar facets for Needs me counting / listing (not the name query).
    fn row_passes_needs_me_facets(&self, r: &crate::data::Row) -> bool {
        if self
            .grid
            .visibility
            .is_some_and(|v| !v.matches(r, &self.attention_by_repo))
        {
            return false;
        }
        if self.grid.root.as_ref().is_some_and(|root| &r.root != root) {
            return false;
        }
        if self
            .grid
            .language
            .as_ref()
            .is_some_and(|lang| &r.language != lang)
        {
            return false;
        }
        if let Some(name) = self.config.active_workspace_group.as_deref() {
            let in_group = self
                .config
                .workspace_groups
                .iter()
                .find(|g| g.name == name)
                .is_some_and(|g| Self::group_matches_path(g, r.id.as_ref()));
            if !in_group {
                return false;
            }
        }
        tree_allows_row(
            self.grid.tree_focus.as_ref(),
            r.parent_id.as_ref(),
            &r.id,
            self.attention_by_repo.contains_key(&r.id),
        )
    }

    /// Collapse/expand the left rail (icon-only vs full labels + contextual panel).
    pub fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        self.sidebar_dragging = false;
        self.persist_sidebar_chrome();
        cx.notify();
    }

    /// Live width of the left rail (collapsed icon strip or the expanded width).
    fn sidebar_w(&self) -> f32 {
        if self.sidebar_collapsed {
            SIDEBAR_COLLAPSED
        } else {
            self.sidebar_width.clamp(SIDEBAR_MIN, SIDEBAR_MAX)
        }
    }

    /// Write sidebar width/collapsed into config.toml (keeps other settings).
    fn persist_sidebar_chrome(&mut self) {
        self.config.sidebar_width = self.sidebar_width.clamp(SIDEBAR_MIN, SIDEBAR_MAX);
        self.config.sidebar_collapsed = self.sidebar_collapsed;
        let _ = repoharbor_core::config::save(&self.config);
    }

    /// Persist workspace groups + active group name.
    fn persist_workspace_groups(&mut self) {
        let _ = repoharbor_core::config::save(&self.config);
    }

    /// Does `path` belong to the named group?
    fn group_matches_path(group: &repoharbor_core::model::WorkspaceGroup, path: &str) -> bool {
        group.prefixes.iter().any(|p| {
            let p = p.trim_end_matches('/');
            path == p || path.starts_with(&format!("{p}/"))
        })
    }

    /// Absolute paths of repos in the active workspace group (or empty).
    fn active_group_repo_paths(&self) -> Vec<String> {
        let Some(name) = self.config.active_workspace_group.as_deref() else {
            return Vec::new();
        };
        let Some(group) = self.config.workspace_groups.iter().find(|g| g.name == name) else {
            return Vec::new();
        };
        self.rows
            .iter()
            .filter(|r| Self::group_matches_path(group, r.id.as_ref()))
            .map(|r| r.id.to_string())
            .collect()
    }

    /// Activate a workspace group filter (or clear when `None`).
    pub fn set_workspace_group(&mut self, name: Option<String>, cx: &mut Context<Self>) {
        if self.config.active_workspace_group == name {
            return;
        }
        self.config.active_workspace_group = name;
        self.persist_workspace_groups();
        self.clear_selection_quiet();
        cx.notify();
    }

    /// Seed Odoo-style groups if none exist, then persist.
    pub fn seed_workspace_groups(&mut self, cx: &mut Context<Self>) {
        if repoharbor_core::config::seed_odoo_groups_if_empty(&mut self.config) {
            self.persist_workspace_groups();
            self.push_toast(
                ToastKind::Info,
                "Workspace groups",
                Some("Seeded Core / Digits / Custom under your roots.".into()),
                cx,
            );
        } else if self.config.workspace_groups.is_empty() {
            self.push_toast(
                ToastKind::Info,
                "No groups",
                Some("Add roots that contain core/, digits/, or custom/.".into()),
                cx,
            );
        }
        cx.notify();
    }

    /// Fleet Pull (or Fetch) every repo in the active workspace group.
    pub fn run_active_group_fleet(&mut self, op: crate::fleet::FleetOp, cx: &mut Context<Self>) {
        let repos = self.active_group_repo_paths();
        if repos.is_empty() {
            self.push_toast(
                ToastKind::Info,
                "No repos in group",
                Some("Select a workspace group with matching repos first.".into()),
                cx,
            );
            return;
        }
        // Mirror selection so the fleet bar reflects the run target.
        self.selected = repos.iter().cloned().map(SharedString::from).collect();
        self.run_fleet_repos(op, repos, cx);
    }

    /// Fleet-pull every repo that is behind its upstream (palette + toolbar).
    pub fn pull_behind_repos(&mut self, cx: &mut Context<Self>) {
        if !self.ensure_fleet_actions_idle(cx) {
            return;
        }
        let repos: Vec<String> = self
            .rows
            .iter()
            .filter(|r| r.behind > 0)
            .map(|r| r.id.to_string())
            .collect();
        if repos.is_empty() {
            self.log_action_cmd("git pull (behind)", Some("0 repos behind"), true, cx);
            self.log_action_result(
                "↷",
                "33",
                "Nothing behind",
                Some("every repo is up to date with upstream"),
                true,
                cx,
            );
            self.push_toast(
                crate::toast::ToastKind::Info,
                "Nothing behind",
                Some("Every repo is up to date with its upstream.".into()),
                cx,
            );
        } else {
            self.run_fleet_repos(crate::fleet::FleetOp::Pull, repos, cx);
        }
    }

    /// True when this checkout is under a configured pull-only (upstream) path.
    pub fn is_pull_only(&self, repo_id: &str) -> bool {
        repoharbor_core::model::path_is_pull_only(repo_id, &self.config.pull_only_prefixes)
    }

    pub fn is_attention_muted(&self, repo_id: &str) -> bool {
        repoharbor_core::model::path_is_attention_muted(
            repo_id,
            &self.config.mute_attention_prefixes,
        )
    }

    /// Force-refresh host enrichment for every repo (ignores the TTL), then
    /// reload the grid. The ops row's "Fetch all".
    pub fn fetch_all_hosts(&mut self, cx: &mut Context<Self>) {
        self.log_action_cmd(
            "repoharbor enrich --hosts",
            Some("GitHub / GitLab metadata"),
            true,
            cx,
        );
        cx.spawn(async move |this, cx| {
            let now = crate::data::now_unix();
            let updated =
                crate::task::run(
                    async move { repoharbor_core::enrich::refresh_cached_all(now).await },
                )
                .await;
            if updated == 0 {
                let _ = this.update(cx, |this, cx| {
                    this.log_action_result(
                        "↷",
                        "33",
                        "Host enrichment current",
                        Some("0 repos changed"),
                        false,
                        cx,
                    );
                });
                return;
            }
            let snap = cx
                .background_executor()
                .spawn(async { crate::data::load(crate::data::now_unix()) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.apply_snapshot(snap);
                this.log_action_result(
                    "✓",
                    "32",
                    &format!("Refreshed host metadata ({updated} repo(s))"),
                    None,
                    false,
                    cx,
                );
                cx.notify();
            });
        })
        .detach();
    }

    /// Set/toggle the current view's contextual sub-filter (Feed/Inbox panels).
    /// Passing the already-active key clears it; `None` clears unconditionally.
    pub fn set_view_filter(&mut self, key: Option<SharedString>, cx: &mut Context<Self>) {
        if self.view == View::Settings {
            self.view_filter = Some(key.unwrap_or_else(|| "roots".into()));
            cx.notify();
            return;
        }
        self.view_filter = if key.is_some() && key == self.view_filter {
            None
        } else {
            key
        };
        cx.notify();
    }

    /// Select the scanned root to filter by (sidebar ROOTS); `None` = all.
    pub fn set_root(&mut self, root: Option<SharedString>, cx: &mut Context<Self>) {
        if self.grid.root == root {
            return;
        }
        self.grid.root = root;
        self.clear_selection_quiet();
        cx.notify();
    }

    /// Toggle the language filter (sidebar LANGUAGES) — clicking the active one
    /// clears it.
    pub fn toggle_language(&mut self, lang: SharedString, cx: &mut Context<Self>) {
        self.grid.language = if self.grid.language.as_ref() == Some(&lang) {
            None
        } else {
            Some(lang)
        };
        self.clear_selection_quiet();
        cx.notify();
    }

    /// The current filter combo as a `SavedView` (with a generated name).
    fn current_view(&self) -> SavedView {
        let root = self.grid.root.as_ref().map(|r| r.to_string());
        let language = self.grid.language.as_ref().map(|l| l.to_string());
        // Name from the active facets, e.g. "Dirty · Rust · RepoHarbor"; "All repos"
        // when nothing is narrowed.
        let mut parts: Vec<String> = Vec::new();
        if self.grid.filter != RepoFilter::All {
            parts.push(self.grid.filter.label().to_string());
        }
        if let Some(v) = self.grid.visibility {
            parts.push(v.label().to_string());
        }
        if let Some(l) = &language {
            parts.push(l.clone());
        }
        if let Some(r) = &root {
            parts.push(r.rsplit('/').next().unwrap_or(r).to_string());
        }
        let name = if parts.is_empty() {
            "All repos".to_string()
        } else {
            parts.join(" · ")
        };
        SavedView {
            name,
            filter: self.grid.filter,
            visibility: self.grid.visibility,
            root,
            language,
            sort: self.grid.sort,
        }
    }

    /// Whether `v` matches the live filter combo (drives the active highlight).
    fn view_is_active(&self, v: &SavedView) -> bool {
        let (want_filter, want_vis) = if is_visibility_filter(v.filter) {
            (RepoFilter::All, v.visibility.or(Some(v.filter)))
        } else {
            (v.filter, v.visibility)
        };
        want_filter == self.grid.filter
            && want_vis == self.grid.visibility
            && v.sort == self.grid.sort
            && v.root.as_deref() == self.grid.root.as_deref()
            && v.language.as_deref() == self.grid.language.as_deref()
    }

    /// Save the current filter combo as a quick view (deduped by combo), persist,
    /// and refresh.
    pub fn save_current_view(&mut self, cx: &mut Context<Self>) {
        let view = self.current_view();
        if !self.grid.saved_views.iter().any(|v| self.view_is_active(v)) {
            self.grid.saved_views.push(view);
            persist_saved_views(&self.grid.saved_views);
            cx.notify();
        }
    }

    /// Apply a saved quick view's filter combo.
    pub fn apply_view(&mut self, idx: usize, cx: &mut Context<Self>) {
        if let Some(v) = self.grid.saved_views.get(idx) {
            // Legacy views stored Public/Private/… as `filter`; migrate to the
            // orthogonal visibility overlay so work modes stay intact.
            if is_visibility_filter(v.filter) {
                self.grid.mode = WorkMode::All;
                self.grid.filter = RepoFilter::All;
                self.grid.visibility = v.visibility.or(Some(v.filter));
            } else {
                self.grid.filter = v.filter;
                self.grid.mode = WorkMode::from_filter(v.filter);
                self.grid.visibility = v.visibility;
            }
            self.grid.sort = v.sort;
            self.grid.root = v.root.clone().map(SharedString::from);
            self.grid.language = v.language.clone().map(SharedString::from);
            self.clear_selection_quiet();
            cx.notify();
        }
    }

    /// Delete a saved quick view.
    pub fn delete_view(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx < self.grid.saved_views.len() {
            self.grid.saved_views.remove(idx);
            persist_saved_views(&self.grid.saved_views);
            cx.notify();
        }
    }

    /// Generate local-AI one-line summaries for every repo (cached by commit, so
    /// repeats are cheap), then reload the grid/list so rows show them. Always
    /// toasts progress then success / up-to-date / error — never silent.
    pub fn summarize_all(&mut self, cx: &mut Context<Self>) {
        use repoharbor_core::summarize::SummarizeOutcome;

        if !self.services.ai_ready {
            self.log_action_cmd("repoharbor ai summarize", Some("offline"), true, cx);
            self.log_action_result(
                "✗",
                "31",
                "Summarize failed",
                Some("AI is not ready"),
                true,
                cx,
            );
            self.upsert_toast(
                "summarize",
                ToastKind::Error,
                "Summarize failed",
                Some("AI is not ready".into()),
                cx,
            );
            return;
        }
        self.log_action_cmd(
            "repoharbor ai summarize",
            Some("cached by commit"),
            true,
            cx,
        );
        self.upsert_toast(
            "summarize",
            ToastKind::Progress,
            "Summarizing…",
            Some("One-line AI summaries (cached by commit)".into()),
            cx,
        );
        cx.spawn(async move |this, cx| {
            let outcome =
                crate::task::run(async { repoharbor_core::summarize::run_cached().await }).await;
            match outcome {
                SummarizeOutcome::Updated(n) => {
                    let snap = cx
                        .background_executor()
                        .spawn(async { crate::data::load(crate::data::now_unix()) })
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        this.apply_snapshot(snap);
                        let (kind, title, detail) = summarize_finish_toast(n);
                        this.log_action_result("✓", "32", &title, detail.as_deref(), false, cx);
                        this.upsert_toast("summarize", kind, title, detail, cx);
                        cx.notify();
                    });
                }
                SummarizeOutcome::UpToDate => {
                    let _ = this.update(cx, |this, cx| {
                        this.log_action_result(
                            "↷",
                            "33",
                            "Already up to date",
                            Some("summaries current for every repo"),
                            false,
                            cx,
                        );
                        this.upsert_toast(
                            "summarize",
                            ToastKind::Info,
                            "Already up to date",
                            Some("Summaries are current for every repo".into()),
                            cx,
                        );
                    });
                }
                SummarizeOutcome::Unavailable(msg) => {
                    let _ = this.update(cx, |this, cx| {
                        this.log_action_result("✗", "31", "Summarize failed", Some(msg), false, cx);
                        this.upsert_toast(
                            "summarize",
                            ToastKind::Error,
                            "Summarize failed",
                            Some(msg.into()),
                            cx,
                        );
                    });
                }
            }
        })
        .detach();
    }

    /// Focus the Mission Control grid on a parent repo (+ its submodules), or
    /// clear focus when `None` / toggling the same id.
    pub fn set_tree_focus(&mut self, id: Option<SharedString>, cx: &mut Context<Self>) {
        let next = match (&self.grid.tree_focus, id) {
            (Some(cur), Some(next)) if cur == &next => None,
            (_, next) => next,
        };
        if self.grid.tree_focus == next {
            return;
        }
        self.grid.tree_focus = next;
        self.clear_selection_quiet();
        cx.notify();
    }

    /// Compact Mission Control banner shown when `tree_focus` scopes the list to
    /// a single parent repository and its submodule children.
    fn tree_focus_banner(&self, t: &Theme, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let focus_id = self.grid.tree_focus.as_ref()?;
        let parent_name = self
            .rows
            .iter()
            .find(|r| &r.id == focus_id)
            .map(|r| r.name.clone())
            .unwrap_or_else(|| focus_id.clone());
        let sub_count = self
            .rows
            .iter()
            .filter(|r| r.parent_id.as_ref() == Some(focus_id))
            .count();
        let (hb, hf) = (t.border_strong, t.fg0);
        Some(
            div()
                .id("tree-focus-banner")
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(10.))
                .px(px(16.))
                .py(px(7.))
                .bg(rgb(t.accent_wash))
                .border_b_1()
                .border_color(rgb(t.border_accent))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.))
                        .min_w(px(0.))
                        .child(lucide("git-fork", 14., t.accent_bright))
                        .child(
                            div()
                                .text_size(px(t.text_small))
                                .text_color(rgb(t.fg0))
                                .font_weight(FontWeight::MEDIUM)
                                .truncate()
                                .child(SharedString::from(format!(
                                    "Showing submodules of {parent_name} ({sub_count} submodule(s))"
                                ))),
                        ),
                )
                .child(
                    div()
                        .id("tree-clear-focus")
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(5.))
                        .px(px(9.))
                        .py(px(3.))
                        .rounded(px(t.r_xs))
                        .bg(rgb(t.button_bg))
                        .border_1()
                        .border_color(rgb(t.border))
                        .cursor_pointer()
                        .hover(move |s| s.border_color(rgb(hb)).text_color(rgb(hf)))
                        .text_size(px(t.text_data_sm))
                        .text_color(rgb(t.fg1))
                        .child(lucide("x", 12., t.fg2))
                        .child("Show all repos")
                        .on_click(cx.listener(|this, _e, _w, cx| this.set_tree_focus(None, cx))),
                )
                .into_any_element(),
        )
    }

    /// Absolute row indices passing every active filter (chip AND root AND
    /// language AND name query), in the active sort order.
    pub(crate) fn visible_rows(&self) -> Vec<usize> {
        let q_filter = SearchQueryFilter::parse(&self.grid.query);
        let mut v: Vec<usize> = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| self.row_matches_chips(r))
            .filter(|(_, r)| self.grid.root.as_ref().is_none_or(|root| &r.root == root))
            .filter(|(_, r)| {
                self.grid
                    .language
                    .as_ref()
                    .is_none_or(|lang| &r.language == lang)
            })
            .filter(|(_, r)| {
                let Some(name) = self.config.active_workspace_group.as_deref() else {
                    return true;
                };
                self.config
                    .workspace_groups
                    .iter()
                    .find(|g| g.name == name)
                    .is_some_and(|g| Self::group_matches_path(g, r.id.as_ref()))
            })
            .filter(|(_, r)| {
                if q_filter.is_empty() {
                    return true;
                }
                q_filter.matches_row(r, &self.attention_by_repo)
            })
            .filter(|(_, r)| {
                // Default: hide submodule children. Focusing a parent in TREE
                // shows that parent + its children only. Needs me reveals
                // attention-index children so chip/badge never outrun the list.
                let reveal = self.grid.filter == RepoFilter::Attention
                    && self.attention_by_repo.contains_key(&r.id);
                tree_allows_row(
                    self.grid.tree_focus.as_ref(),
                    r.parent_id.as_ref(),
                    &r.id,
                    reveal,
                )
            })
            .map(|(i, _)| i)
            .collect();
        match self.grid.sort {
            SortMode::Activity => v.sort_by(|&a, &b| {
                self.rows[b]
                    .last_commit_unix
                    .cmp(&self.rows[a].last_commit_unix)
            }),
            SortMode::Name => v.sort_by(|&a, &b| {
                self.rows[a]
                    .name
                    .to_lowercase()
                    .cmp(&self.rows[b].name.to_lowercase())
            }),
        }
        // The Attention filter ranks the most urgent repos first; the sort is
        // stable, so the active sort still orders repos within each severity
        // tier.
        if self.grid.filter == RepoFilter::Attention {
            v.sort_by_key(|&i| {
                self.attention_by_repo
                    .get(&self.rows[i].id)
                    .copied()
                    .unwrap_or(Severity::Info)
            });
        }
        v
    }

    /// Create the Mission Control name-filter input once (needs a `Window`).
    fn ensure_repo_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.repo_search.is_some() {
            return;
        }
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Filter repos (e.g. is:dirty lang:rust)…")
        });
        let sub = cx.observe(&input, |this, input, cx| {
            this.grid.query = input.read(cx).value().to_string();
            // Name filter is orthogonal to chips — prune (don't clear) so
            // still-visible picks stay selected and hidden ones can't be fleeted.
            this.prune_selection_to_visible();
            cx.notify();
        });
        self.repo_search = Some(input);
        self._repo_search_sub = Some(sub);
    }

    /// Focus the Mission Control name/filter input (shortcut `/` or `Ctrl+F`).
    pub fn focus_repo_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ensure_repo_search(window, cx);
        if let Some(input) = &self.repo_search {
            let fh = input.read(cx).focus_handle(cx);
            window.focus(&fh, cx);
        }
    }

    /// Clear the Mission Control name filter (Esc when nothing else is open).
    fn clear_repo_query(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.grid.query.clear();
        if let Some(input) = &self.repo_search {
            input.update(cx, |st, cx| st.set_value("", window, cx));
        }
        cx.notify();
    }

    /// Refresh host enrichment (stars/topics/issues/release/visibility) from
    /// GitHub/GitLab on the tokio runtime, then reload the grid from the freshly
    /// written cache. A no-op when every repo's cache is still within the TTL
    /// (so repeated rescans cost nothing) or when offline. Network failures are
    /// silent by design — stale enrichment simply persists.
    pub fn enrich_hosts(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let now = crate::data::now_unix();
            let updated =
                crate::task::run(async move { repoharbor_core::enrich::refresh_cached(now).await })
                    .await;
            if updated == 0 {
                return;
            }
            // Rebuild rows from the enriched cache, off the UI thread.
            let snap = cx
                .background_executor()
                .spawn(async { crate::data::load(crate::data::now_unix()) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.apply_snapshot(snap);
                cx.notify();
            });
        })
        .detach();
    }

    /// Run the central CI-status pass (`repoharbor_core::ci`, #183) on the tokio
    /// runtime, then reload `ci_states` from the cache and recompute
    /// attention — failing default-branch CI flows to badges, tray, and
    /// notifications through the model. TTL-paced in core (~5 min), so
    /// calling this on every rescan and attention poll is cheap.
    ///
    /// A pass where *everything* failed (expired token, rate limit, outage)
    /// surfaces one soft Info toast per distinct error — the cached states are
    /// deliberately left untouched (see `repoharbor_core::ci`), so this toast is
    /// still how an auth/permission problem becomes visible, but as a quiet
    /// auto-dismissing notice rather than a sticky red Error that stacks on
    /// top of unrelated fleet successes (e.g. Pull ok + CI 403).
    pub fn refresh_ci(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let now = crate::data::now_unix();
            let outcome =
                crate::task::run(async move { repoharbor_core::ci::refresh_cached(now).await })
                    .await;
            if outcome.updated == 0 && outcome.errors == 0 {
                return; // Nothing attempted (all fresh / no token / no repos).
            }
            let states = cx
                .background_executor()
                .spawn(async { repoharbor_core::cache::all_ci_states() })
                .await;
            let _ = this.update(cx, |this, cx| {
                if outcome.all_failed() {
                    if this.ci_last_error != outcome.last_error
                        && let Some(err) = &outcome.last_error
                    {
                        // Soft Info (not sticky Error). When the hint points at
                        // org SSO, make the toast a one-click link to authorize.
                        let sso = err.contains("SSO") || err.contains("authorize org");
                        if sso {
                            this.upsert_toast_link(
                                "ci-pass",
                                ToastKind::Info,
                                "Couldn't refresh CI status",
                                Some(err.clone().into()),
                                repoharbor_core::oauth::github_oauth_app_settings_url().into(),
                                cx,
                            );
                        } else {
                            this.upsert_toast(
                                "ci-pass",
                                ToastKind::Info,
                                "Couldn't refresh CI status",
                                Some(err.clone().into()),
                                cx,
                            );
                        }
                    }
                    this.ci_last_error = outcome.last_error;
                } else if outcome.errors == 0 {
                    this.ci_last_error = None;
                }
                this.ci_states = states;
                this.recompute_attention();
                cx.notify();
            });
        })
        .detach();
    }
}

impl RepoHarborApp {
    /// Mission Control's nav badge: listable Needs-me work only (Urgent +
    /// Attention). Counts local repos that pass the same facets as the Needs
    /// me list (including submodule reveal) plus host-only actionable facts —
    /// never host-only / hidden-submodule inflation against an empty grid.
    fn grid_badge(&self) -> (usize, bool) {
        let mut n = 0;
        let mut urgent = false;
        for r in &self.rows {
            let Some(sev) = self.attention_by_repo.get(&r.id).copied() else {
                continue;
            };
            if !self.row_passes_needs_me_facets(r) {
                continue;
            }
            match sev {
                Severity::Urgent => {
                    n += 1;
                    urgent = true;
                }
                Severity::Attention => n += 1,
                Severity::Info => {}
            }
        }
        for item in host_only_needs_me(&self.attention_items) {
            n += 1;
            if item.severity == Severity::Urgent {
                urgent = true;
            }
        }
        (n, urgent)
    }

    /// The Inbox nav badge: (count, any-urgent) over the inbox-derived
    /// attention items (review requests + your open PRs) once the inbox has
    /// loaded. Until then it falls back to the platform notifier's glance
    /// count — the poll already running from launch — so the badge is live
    /// before the Inbox view is ever opened.
    fn inbox_badge(&self) -> (usize, bool) {
        if !matches!(self.inbox, crate::views::inbox::InboxState::Ready(_)) {
            return (self.attention.len(), false);
        }
        let mut n = 0;
        let mut urgent = false;
        for item in &self.attention_items {
            match item.kind {
                AttentionKind::ReviewRequested => {
                    n += 1;
                    urgent = true;
                }
                AttentionKind::PrAssigned => n += 1,
                _ => {}
            }
        }
        (n, urgent)
    }

    /// The card state flags for `rows[idx]`: live agent session, urgent
    /// attention, reason chips, and fleet selection. Cheap map lookups — fine
    /// inside the `uniform_list` closures.
    fn card_state(&self, idx: usize, selecting: bool) -> crate::card::CardState {
        let id = &self.rows[idx].id;
        let (reason_chips, reasons_more, reason_subtitle) =
            attention_reason_chips(&self.attention_items, id);
        crate::card::CardState {
            active: self.active_agents.contains(id),
            urgent: self.attention_by_repo.get(id).copied() == Some(Severity::Urgent),
            selected: selecting && self.selected.contains(id),
            selecting,
            reason_chips,
            reasons_more,
            reason_subtitle,
            tree_focused: self.grid.tree_focus.as_ref() == Some(id),
        }
    }

    /// Single chrome row: sidebar toggle + primary nav tabs + search + actions.
    /// Branding/count live only in the TitleBar above — no second RepoHarbor strip.
    fn chrome_bar(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let mut tabs = div()
            .id("header-nav-tabs")
            .flex()
            .flex_row()
            .items_center()
            .gap(px(2.))
            .flex_1()
            .min_w(px(0.))
            .overflow_x_scroll();
        for (idx, (view, icon_name, label)) in NAV.iter().copied().enumerate() {
            // Subtle separator between workspace views (Mission Control..Agents) and utility/system tabs.
            if idx == 5 {
                tabs = tabs.child(
                    div()
                        .w(px(1.))
                        .h(px(18.))
                        .mx(px(4.))
                        .bg(rgb(t.border_strong)),
                );
            }
            let active = self.view == view;
            let fg = if active { t.accent_bright } else { t.fg2 };
            let mut item = div()
                .id(SharedString::from(format!("tab-{label}")))
                .flex()
                .flex_row()
                .flex_shrink_0()
                .items_center()
                .gap(px(5.))
                .px(px(8.))
                .py(px(5.))
                .rounded(px(t.r_sm))
                .border_1()
                .border_color(rgb(if active { t.border_accent } else { t.page }))
                .text_size(px(t.text_small))
                .text_color(rgb(fg))
                .cursor_pointer()
                .hover(|s| s.text_color(rgb(t.fg0)).bg(rgb(t.surface_hover)))
                .on_click(cx.listener(move |this, _ev, window, cx| {
                    this.view = view;
                    this.view_filter = if view == View::Settings {
                        Some("roots".into())
                    } else {
                        None
                    };
                    this.maybe_load_view(view, window, cx);
                    cx.notify();
                }))
                .child(lucide(icon_name, 14., fg))
                .child(SharedString::from(label.to_string()));
            if active {
                item = item.font_weight(FontWeight::MEDIUM).bg(rgb(t.accent_wash));
            }
            let (n, urgent) = match view {
                View::Grid => self.grid_badge(),
                View::Inbox => self.inbox_badge(),
                _ => (0, false),
            };
            if n > 0 {
                item = item.child(badge(n, urgent, t));
            }
            tabs = tabs.child(item);
        }

        let search = div()
            .id("header-search")
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .w(px(160.))
            .flex_shrink(1.)
            .h(px(30.))
            .px(px(10.))
            .rounded(px(t.r_sm))
            .bg(rgb(t.button_bg))
            .border_1()
            .border_color(rgb(t.border))
            .text_color(rgb(t.fg2))
            .cursor_pointer()
            .hover(|s| {
                s.border_color(rgb(t.border_strong))
                    .bg(rgb(t.surface_hover))
            })
            .on_click(cx.listener(|this, _ev, window, cx| this.open_palette(window, cx)))
            .child(lucide("search", 14., t.fg2))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .truncate()
                    .text_size(px(t.text_small))
                    .child("Jump to…"),
            )
            .child(
                div()
                    .px(px(5.))
                    .rounded(px(t.r_xs))
                    .border_1()
                    .border_color(rgb(t.border))
                    .font_family("monospace")
                    .text_size(px(t.text_data_sm))
                    .text_color(rgb(t.fg3))
                    .child("⌘K"),
            );

        let actions = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .flex_shrink_0()
            .child(search)
            .child(
                div()
                    .id("header-new")
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(30.))
                    .h(px(30.))
                    .rounded(px(t.r_sm))
                    .bg(rgb(t.button_bg))
                    .border_1()
                    .border_color(rgb(t.border))
                    .cursor_pointer()
                    .hover(|s| {
                        s.bg(rgb(t.surface_hover))
                            .border_color(rgb(t.border_strong))
                    })
                    .child(lucide("plus", 15., t.fg0))
                    .on_click(cx.listener(|this, _ev, window, cx| {
                        this.open_add_dialog(
                            crate::views::newproject::NewMode::AddRoot,
                            window,
                            cx,
                        );
                    })),
            )
            .child(
                div()
                    .id("header-rescan")
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_center()
                    .gap(px(6.))
                    .h(px(30.))
                    .px(px(10.))
                    .rounded(px(t.r_sm))
                    .bg(rgb(t.accent_wash))
                    .border_1()
                    .border_color(rgb(t.border_accent))
                    .text_size(px(t.text_small))
                    .text_color(rgb(t.fg0))
                    .cursor_pointer()
                    .hover(|s| {
                        s.bg(rgb(t.surface_hover))
                            .border_color(rgb(t.border_strong))
                    })
                    .child(lucide("refresh-cw", 15., t.accent_bright))
                    .child("Refresh")
                    .on_click(cx.listener(|this, _ev, _window, cx| this.rescan_user(cx))),
            )
            .children(match &self.update_state {
                AppUpdateState::Available(rel) => {
                    let tag = rel.tag_name.clone();
                    Some(
                        div()
                            .id("header-update")
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_center()
                            .gap(px(6.))
                            .h(px(30.))
                            .px(px(10.))
                            .rounded(px(t.r_sm))
                            .bg(rgb(t.accent_wash))
                            .border_1()
                            .border_color(rgb(t.clean))
                            .text_size(px(t.text_small))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(t.clean))
                            .cursor_pointer()
                            .hover(|s| {
                                s.bg(rgb(t.surface_hover))
                                    .border_color(rgb(t.border_strong))
                            })
                            .tooltip(move |window, cx| {
                                gpui_component::tooltip::Tooltip::new(format!(
                                    "New release {tag} available — click to update & restart"
                                ))
                                .build(window, cx)
                            })
                            .child(lucide("cloud-download", 15., t.clean))
                            .child(SharedString::from(format!("Update {}", rel.tag_name)))
                            .on_click(cx.listener(|this, _ev, window, cx| {
                                this.apply_app_update(window, cx);
                            }))
                            .into_any_element(),
                    )
                }
                AppUpdateState::Installing => Some(
                    div()
                        .id("header-updating")
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_center()
                        .gap(px(6.))
                        .h(px(30.))
                        .px(px(10.))
                        .rounded(px(t.r_sm))
                        .bg(rgb(t.accent_wash))
                        .border_1()
                        .border_color(rgb(t.border_accent))
                        .text_size(px(t.text_small))
                        .text_color(rgb(t.primary))
                        .child(lucide("cloud-download", 15., t.primary))
                        .child("Updating…")
                        .into_any_element(),
                ),
                _ => None,
            })
            .child(
                div()
                    .id("header-terminal")
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_center()
                    .gap(px(6.))
                    .h(px(30.))
                    .px(px(10.))
                    .rounded(px(t.r_sm))
                    .bg(rgb(if self.terminal.open {
                        t.accent_wash
                    } else {
                        t.button_bg
                    }))
                    .border_1()
                    .border_color(rgb(if self.terminal.open {
                        t.border_accent
                    } else {
                        t.border
                    }))
                    .text_size(px(t.text_small))
                    .text_color(rgb(if self.terminal.open {
                        t.accent_bright
                    } else {
                        t.fg1
                    }))
                    .cursor_pointer()
                    .hover(|s| {
                        s.bg(rgb(t.surface_hover))
                            .border_color(rgb(t.border_strong))
                    })
                    .child(lucide(
                        "terminal",
                        15.,
                        if self.terminal.open {
                            t.accent_bright
                        } else {
                            t.fg2
                        },
                    ))
                    .child("Terminal")
                    .when(!self.terminal.tabs.is_empty(), |d| {
                        d.child(
                            div()
                                .px(px(5.))
                                .rounded(px(t.r_xs))
                                .bg(rgb(t.surface))
                                .border_1()
                                .border_color(rgb(t.border))
                                .font_family("monospace")
                                .text_size(px(t.text_data_sm))
                                .text_color(rgb(t.fg2))
                                .child(SharedString::from(format!("{}", self.terminal.tabs.len()))),
                        )
                    })
                    .on_click(cx.listener(|this, _ev, window, cx| {
                        this.toggle_terminal(Some(window), cx);
                    })),
            );

        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .w_full()
            .min_w(px(0.))
            .overflow_hidden()
            .h(px(44.))
            .px(px(10.))
            .border_b_1()
            .border_color(rgb(t.border))
            .bg(rgb(t.page))
            .child(
                div()
                    .id("header-sidebar-toggle")
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .justify_center()
                    .w(px(30.))
                    .h(px(30.))
                    .rounded(px(t.r_sm))
                    .bg(rgb(t.button_bg))
                    .border_1()
                    .border_color(rgb(t.border))
                    .cursor_pointer()
                    .hover(|s| {
                        s.bg(rgb(t.surface_hover))
                            .border_color(rgb(t.border_strong))
                    })
                    .child(lucide(
                        if self.sidebar_collapsed {
                            "panel-left-open"
                        } else {
                            "panel-left-close"
                        },
                        15.,
                        t.fg0,
                    ))
                    .on_click(cx.listener(|this, _ev, _window, cx| this.toggle_sidebar(cx))),
            )
            .child(tabs)
            .child(actions)
    }

    fn sidebar(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let collapsed = self.sidebar_collapsed;
        let width = self.sidebar_w();
        // Primary nav lives in top tabs; the rail is context-only (GROUPS/TREE/…).
        let mut rail = div()
            .flex()
            .flex_col()
            .w(px(width))
            .h_full()
            .px(px(if collapsed { 8. } else { 12. }))
            .py(px(16.))
            .gap(px(16.))
            .border_r_1()
            .border_color(rgb(t.border))
            .bg(rgb(t.page));

        if !collapsed {
            rail = rail
                .child(
                    div()
                        .id("sidebar-context")
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_h(px(0.))
                        .overflow_y_scroll()
                        .children(self.contextual_sidebar(t, cx)),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.))
                        .pt(px(10.))
                        .border_t_1()
                        .border_color(rgb(t.border))
                        .font_family("monospace")
                        .text_size(px(t.text_data_sm))
                        .text_color(rgb(t.fg3))
                        .child(lucide("hard-drive", 13., t.fg3))
                        .child("Scanned just now"),
                );
        } else {
            rail = rail.child(div().flex_1());
        }

        // Rail + drag handle (expanded only). Handle drag updates `sidebar_width`.
        div()
            .flex()
            .flex_row()
            .h_full()
            .flex_shrink_0()
            .child(rail)
            .when(!collapsed, |this| {
                this.child(
                    div()
                        .id("sidebar-resize")
                        .w(px(5.))
                        .ml(px(-2.))
                        .h_full()
                        .cursor(CursorStyle::ResizeLeftRight)
                        .hover(|s| s.bg(rgb(t.border_accent)))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _ev, _window, cx| {
                                this.sidebar_dragging = true;
                                cx.notify();
                            }),
                        )
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(|this, _ev, _window, cx| {
                                // Right-click resets to the default width.
                                this.sidebar_width = SIDEBAR_DEFAULT;
                                this.sidebar_dragging = false;
                                this.persist_sidebar_chrome();
                                cx.notify();
                            }),
                        ),
                )
            })
    }

    /// The view-specific sidebar content shown below the fixed nav. `None` for
    /// views that have no contextual panel yet (just the nav above).
    fn contextual_sidebar(&self, t: &Theme, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        match self.view {
            // Mission Control: the ROOTS / LANGUAGES quick-filters.
            View::Grid => Some(self.filter_sections(t, cx).into_any_element()),
            View::Feed => Some(self.feed_panel(t, cx)),
            View::Inbox => Some(self.inbox_panel(t, cx)),
            View::Tools => Some(self.devtools_panel(t, cx)),
            View::Settings => Some(self.settings_panel(t, cx)),
            View::Janitor => Some(self.cleanup_panel(t, cx)),
            View::Explore => Some(self.explore_panel(t, cx)),
            View::Agents => Some(self.agents_panel(t, cx)),
            View::Log => None,
        }
    }

    /// A contextual filter list: a titled section of single-select category rows
    /// that drive `view_filter`. `cats` is `(key, icon, label, count)`; a `None`
    /// key is the "All" row.
    fn category_panel(
        &self,
        t: &Theme,
        cx: &mut Context<Self>,
        title: &'static str,
        cats: Vec<(
            Option<SharedString>,
            &'static str,
            SharedString,
            Option<usize>,
        )>,
    ) -> gpui::AnyElement {
        let mut sec = div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(section_header(title, t));
        for (key, icon, label, count) in cats {
            let active = key == self.view_filter;
            let icon_fg = if active { t.accent_bright } else { t.fg2 };
            let pick = key.clone();
            sec = sec.child(sidebar_filter_item(
                SharedString::from(format!("cat-{title}-{label}")),
                lucide(icon, 14., icon_fg).into_any_element(),
                label,
                count,
                active,
                t,
                cx.listener(move |this, _e, _w, cx| this.set_view_filter(pick.clone(), cx)),
            ));
        }
        div().flex().flex_col().child(sec).into_any_element()
    }

    /// Feed: filter by activity type.
    fn feed_panel(&self, t: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        use crate::views::feed::FeedState;
        let rows = match &self.feed {
            FeedState::Ready(rows) => rows.as_slice(),
            _ => &[],
        };
        let total = rows.len();
        let count = |kind: &str| rows.iter().filter(|r| r.kind.as_ref() == kind).count();
        let new = rows.iter().filter(|r| r.is_new).count();
        self.category_panel(
            t,
            cx,
            "FILTER",
            vec![
                (None, "rss", "All".into(), Some(total)),
                (Some("new".into()), "sparkles", "New".into(), Some(new)),
                (
                    Some("release".into()),
                    "tag",
                    "Releases".into(),
                    Some(count("release")),
                ),
                (
                    Some("starred".into()),
                    "star",
                    "Stars".into(),
                    Some(count("starred")),
                ),
                (
                    Some("created".into()),
                    "box",
                    "New repos".into(),
                    Some(count("created")),
                ),
                (
                    Some("forked".into()),
                    "git-branch",
                    "Forks".into(),
                    Some(count("forked")),
                ),
                (
                    Some("public".into()),
                    "globe",
                    "Open-sourced".into(),
                    Some(count("public")),
                ),
            ],
        )
    }

    /// Inbox: filter by item category.
    fn inbox_panel(&self, t: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        use crate::views::inbox::InboxState;
        let count = |kind: &str| match &self.inbox {
            InboxState::Ready(d) => d.items.iter().filter(|i| i.kind.as_ref() == kind).count(),
            _ => 0,
        };
        let notifications = match &self.inbox {
            InboxState::Ready(d) => d.notifications.len(),
            _ => 0,
        };
        let total = match &self.inbox {
            InboxState::Ready(d) => d.items.len() + d.notifications.len(),
            _ => 0,
        };
        self.category_panel(
            t,
            cx,
            "FILTER",
            vec![
                (None, "inbox", "All".into(), Some(total)),
                (
                    Some("pr".into()),
                    "git-pull-request",
                    "Pull requests".into(),
                    Some(count("pr")),
                ),
                (
                    Some("review".into()),
                    "eye",
                    "Reviews".into(),
                    Some(count("review")),
                ),
                (
                    Some("issue".into()),
                    "circle-dot",
                    "Issues".into(),
                    Some(count("issue")),
                ),
                (
                    Some("notification".into()),
                    "bell",
                    "Notifications".into(),
                    Some(notifications),
                ),
            ],
        )
    }

    /// Dev Tools: filter the utility belt by category (composes with the search
    /// box). Counts are the number of tools in each category.
    fn devtools_panel(&self, t: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        self.category_panel(
            t,
            cx,
            "CATEGORY",
            vec![
                (None, "wrench", "All tools".into(), Some(11)),
                (
                    Some("generators".into()),
                    "box",
                    "Generators".into(),
                    Some(1),
                ),
                (
                    Some("encoding".into()),
                    "binary",
                    "Encoding".into(),
                    Some(2),
                ),
                (Some("hashing".into()), "hash", "Hashing".into(), Some(1)),
                (Some("data".into()), "braces", "Data".into(), Some(2)),
                (
                    Some("convert".into()),
                    "arrow-up-down",
                    "Convert".into(),
                    Some(3),
                ),
                (Some("text".into()), "type", "Text".into(), Some(2)),
            ],
        )
    }

    /// Settings: jump to a section (gates which section the view renders).
    fn settings_panel(&self, t: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        self.category_panel(
            t,
            cx,
            "SECTIONS",
            vec![
                (
                    Some("roots".into()),
                    "folder",
                    "Workspace & Roots".into(),
                    None,
                ),
                (
                    Some("account".into()),
                    "user",
                    "GitHub Account".into(),
                    None,
                ),
                (
                    Some("launchers".into()),
                    "rocket",
                    "Launchers & Terminal".into(),
                    None,
                ),
                (Some("ai".into()), "sparkles", "AI & Providers".into(), None),
                (
                    Some("notifications".into()),
                    "bell",
                    "Notifications & System".into(),
                    None,
                ),
                (
                    Some("about".into()),
                    "harbor",
                    "About & License".into(),
                    None,
                ),
            ],
        )
    }

    /// Cleanup: filter prunable branches by why they're prunable.
    fn cleanup_panel(&self, t: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        use crate::views::cleanup::CleanupState;
        let (mut merged, mut gone) = (0usize, 0usize);
        if let CleanupState::Ready(repos) = &self.cleanup {
            for repo in repos {
                for b in &repo.branches {
                    if b.why == "merged" {
                        merged += 1;
                    } else {
                        gone += 1;
                    }
                }
            }
        }
        self.category_panel(
            t,
            cx,
            "FILTER",
            vec![
                (None, "scissors", "All".into(), Some(merged + gone)),
                (
                    Some("merged".into()),
                    "git-merge",
                    "Merged".into(),
                    Some(merged),
                ),
                (
                    Some("gone".into()),
                    "circle-alert",
                    "Gone".into(),
                    Some(gone),
                ),
            ],
        )
    }

    /// Explore: filter starred results by language.
    fn explore_panel(&self, t: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        use crate::views::explore::ExploreState;
        let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        let total = if let ExploreState::Ready(rows) = &self.explore {
            for r in rows {
                let l: &str = r.language.as_ref();
                if !l.is_empty() {
                    *counts.entry(l).or_default() += 1;
                }
            }
            rows.len()
        } else {
            0
        };
        let mut langs: Vec<(SharedString, usize)> = counts
            .into_iter()
            .map(|(k, n)| (SharedString::from(k.to_string()), n))
            .collect();
        langs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let mut cats: Vec<(
            Option<SharedString>,
            &'static str,
            SharedString,
            Option<usize>,
        )> = vec![(None, "compass", "All".into(), Some(total))];
        for (lang, n) in langs {
            cats.push((Some(lang.clone()), "box", lang, Some(n)));
        }
        self.category_panel(t, cx, "LANGUAGE", cats)
    }

    /// Agents: filter running sessions by repo.
    fn agents_panel(&self, t: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        use crate::views::agents::AgentsState;
        let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        let total = if let AgentsState::Ready(data) = &self.agents {
            for r in &data.sessions {
                *counts.entry(r.name.as_ref()).or_default() += 1;
            }
            // Dispatched worktrees file under their origin repo's name.
            for d in &data.dispatched {
                *counts.entry(d.origin_name.as_ref()).or_default() += 1;
            }
            data.sessions.len() + data.dispatched.len()
        } else {
            0
        };
        let mut repos: Vec<(SharedString, usize)> = counts
            .into_iter()
            .map(|(k, n)| (SharedString::from(k.to_string()), n))
            .collect();
        repos.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let mut cats: Vec<(
            Option<SharedString>,
            &'static str,
            SharedString,
            Option<usize>,
        )> = vec![(None, "square-terminal", "All".into(), Some(total))];
        for (name, n) in repos {
            cats.push((Some(name.clone()), "folder", name, Some(n)));
        }
        self.category_panel(t, cx, "REPO", cats)
    }

    /// The ROOTS and LANGUAGES filter lists, derived from the current rows.
    fn filter_sections(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        use std::collections::HashMap;

        // Aggregate counts per root and per language.
        let mut root_counts: HashMap<&str, usize> = HashMap::new();
        let mut lang_counts: HashMap<&str, usize> = HashMap::new();
        for r in &self.rows {
            *root_counts.entry(r.root.as_ref()).or_default() += 1;
            let lang: &str = r.language.as_ref();
            if !lang.is_empty() {
                *lang_counts.entry(lang).or_default() += 1;
            }
        }
        // Sort by descending count, then name, for a stable order.
        let sorted = |m: HashMap<&str, usize>| {
            let mut v: Vec<(SharedString, usize)> = m
                .into_iter()
                .map(|(k, n)| (SharedString::from(k.to_string()), n))
                .collect();
            v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            v
        };
        let roots = sorted(root_counts);
        let langs = sorted(lang_counts);

        // ── GROUPS (workspace path-prefix sets + fleet Pull/Fetch) ─────────
        let hov = t.surface_hover;
        let active_group = self.config.active_workspace_group.clone();
        let mut groups_sec = div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(section_header_action(
                "GROUPS",
                "plus",
                t,
                cx.listener(|this, _e, _w, cx| this.seed_workspace_groups(cx)),
            ));
        if self.config.workspace_groups.is_empty() {
            groups_sec = groups_sec.child(
                div()
                    .px(px(9.))
                    .py(px(4.))
                    .text_size(px(t.text_data_sm))
                    .text_color(rgb(t.fg3))
                    .child("Press + to seed Core / Digits / Custom groups."),
            );
        } else {
            groups_sec = groups_sec.child(sidebar_filter_item(
                "group-all".into(),
                lucide("folder", 14., t.fg2).into_any_element(),
                "All groups".into(),
                None,
                active_group.is_none(),
                t,
                cx.listener(|this, _e, _w, cx| this.set_workspace_group(None, cx)),
            ));
            for g in &self.config.workspace_groups {
                let name = g.name.clone();
                let active = active_group.as_deref() == Some(name.as_str());
                let n = self
                    .rows
                    .iter()
                    .filter(|r| Self::group_matches_path(g, r.id.as_ref()))
                    .count();
                let fg = if active { t.accent_bright } else { t.fg1 };
                let pick = name.clone();
                let mut row = div()
                    .id(SharedString::from(format!("group-{name}")))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .px(px(9.))
                    .py(px(6.))
                    .rounded(px(t.r_sm))
                    .text_size(px(t.text_small))
                    .text_color(rgb(fg))
                    .cursor_pointer()
                    .hover(move |s| s.bg(rgb(hov)))
                    .on_click(cx.listener(move |this, _e, _w, cx| {
                        this.set_workspace_group(Some(pick.clone()), cx);
                    }))
                    .child(lucide("folder-tree", 14., fg))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .child(SharedString::from(name.clone())),
                    )
                    .child(
                        div()
                            .font_family("monospace")
                            .text_size(px(t.text_data_sm))
                            .text_color(rgb(t.fg3))
                            .child(SharedString::from(n.to_string())),
                    );
                if active {
                    row = row
                        .bg(rgb(t.accent_wash))
                        .child(
                            div()
                                .id(SharedString::from(format!("group-fetch-{name}")))
                                .flex()
                                .items_center()
                                .justify_center()
                                .w(px(22.))
                                .h(px(22.))
                                .rounded(px(t.r_xs))
                                .hover(move |s| s.bg(rgb(hov)))
                                .child(lucide("cloud-download", 13., t.fg2))
                                .on_click(cx.listener(|this, _e, _w, cx| {
                                    cx.stop_propagation();
                                    this.run_active_group_fleet(crate::fleet::FleetOp::Fetch, cx);
                                })),
                        )
                        .child(
                            div()
                                .id(SharedString::from(format!("group-pull-{name}")))
                                .flex()
                                .items_center()
                                .justify_center()
                                .w(px(22.))
                                .h(px(22.))
                                .rounded(px(t.r_xs))
                                .hover(move |s| s.bg(rgb(hov)))
                                .child(lucide("arrow-down", 13., t.primary))
                                .on_click(cx.listener(|this, _e, _w, cx| {
                                    cx.stop_propagation();
                                    this.run_active_group_fleet(crate::fleet::FleetOp::Pull, cx);
                                })),
                        );
                }
                groups_sec = groups_sec.child(row);
            }
        }

        // ── VIEWS (saved quick filters) ────────────────────────────────────
        let mut views_sec = div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(section_header_action(
                "VIEWS",
                "plus",
                t,
                cx.listener(|this, _e, _w, cx| this.save_current_view(cx)),
            ));
        if self.grid.saved_views.is_empty() {
            views_sec = views_sec.child(
                div()
                    .px(px(9.))
                    .py(px(4.))
                    .text_size(px(t.text_data_sm))
                    .text_color(rgb(t.fg3))
                    .child("Save the current filters as a quick view."),
            );
        } else {
            let hov = t.surface_hover;
            for (i, v) in self.grid.saved_views.iter().enumerate() {
                let active = self.view_is_active(v);
                let fg = if active { t.accent_bright } else { t.fg1 };
                let mut row = div()
                    .id(SharedString::from(format!("view-{i}")))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .px(px(9.))
                    .py(px(6.))
                    .rounded(px(t.r_sm))
                    .text_size(px(t.text_small))
                    .text_color(rgb(fg))
                    .cursor_pointer()
                    .hover(move |s| s.bg(rgb(hov)))
                    .on_click(cx.listener(move |this, _e, _w, cx| this.apply_view(i, cx)))
                    .child(lucide("bookmark", 14., fg))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .child(SharedString::from(v.name.clone())),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("view-del-{i}")))
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(18.))
                            .h(px(18.))
                            .rounded(px(t.r_xs))
                            .hover(move |s| s.bg(rgb(hov)))
                            .child(lucide("trash-2", 13., t.fg3))
                            .on_click(cx.listener(move |this, _e, _w, cx| {
                                // Don't let delete also apply the view.
                                cx.stop_propagation();
                                this.delete_view(i, cx);
                            })),
                    );
                if active {
                    row = row.bg(rgb(t.accent_wash));
                }
                views_sec = views_sec.child(row);
            }
        }

        // ── ROOTS ──────────────────────────────────────────────────────────
        let mut roots_sec = div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(section_header("ROOTS", t));
        roots_sec = roots_sec.child(sidebar_filter_item(
            "root-all".into(),
            lucide("folder", 14., t.fg2).into_any_element(),
            "All repos".into(),
            Some(self.rows.len()),
            self.grid.root.is_none(),
            t,
            cx.listener(|this, _e, _w, cx| this.set_root(None, cx)),
        ));
        for (root, n) in roots {
            let active = self.grid.root.as_ref() == Some(&root);
            let pick = root.clone();
            roots_sec = roots_sec.child(sidebar_filter_item(
                SharedString::from(format!("root-{root}")),
                lucide("folder", 14., t.fg2).into_any_element(),
                root,
                Some(n),
                active,
                t,
                cx.listener(move |this, _e, _w, cx| this.set_root(Some(pick.clone()), cx)),
            ));
        }

        // ── LANGUAGES ──────────────────────────────────────────────────────
        let mut langs_sec = div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(section_header("LANGUAGES", t));
        for (lang, n) in langs {
            let active = self.grid.language.as_ref() == Some(&lang);
            let pick = lang.clone();
            langs_sec = langs_sec.child(sidebar_filter_item(
                SharedString::from(format!("lang-{lang}")),
                crate::card::lang_mark(&lang, t),
                lang,
                Some(n),
                active,
                t,
                cx.listener(move |this, _e, _w, cx| this.toggle_language(pick.clone(), cx)),
            ));
        }

        // ── PULLED (files from the latest fast-forward Pull) ───────────────
        let mut sections = div().flex().flex_col().gap(px(14.));
        if let Some(report) = &self.last_pull {
            let more = report.files.len().saturating_sub(40);
            let shown = report.files.iter().take(40);
            let mut pulled = div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(section_header_action(
                    "PULLED",
                    "x",
                    t,
                    cx.listener(|this, _e, _w, cx| this.clear_last_pull(cx)),
                ))
                .child(
                    div()
                        .px(px(9.))
                        .pb(px(4.))
                        .text_size(px(t.text_data_sm))
                        .text_color(rgb(t.fg3))
                        .child(SharedString::from(format!(
                            "{} file(s) · {} repo(s)",
                            report.files.len(),
                            report.repo_count
                        ))),
                );
            let mut last_repo = SharedString::from("");
            for (i, file) in shown.enumerate() {
                if file.repo_name != last_repo {
                    last_repo = file.repo_name.clone();
                    pulled = pulled.child(
                        div()
                            .px(px(9.))
                            .pt(px(6.))
                            .pb(px(2.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_size(px(t.text_data_sm))
                            .text_color(rgb(t.fg2))
                            .truncate()
                            .child(file.repo_name.clone()),
                    );
                }
                let repo = file.repo_id.clone();
                let path = file.path.clone();
                pulled = pulled.child(
                    div()
                        .id(SharedString::from(format!("pulled-{i}")))
                        .px(px(9.))
                        .py(px(3.))
                        .rounded(px(t.r_xs))
                        .cursor_pointer()
                        .hover(move |s| s.bg(rgb(hov)))
                        .on_click(cx.listener(move |this, _e, window, cx| {
                            this.open_drawer_changes(repo.clone(), window, cx);
                        }))
                        .child(
                            div()
                                .font_family("monospace")
                                .text_size(px(t.text_data_sm))
                                .text_color(rgb(t.fg1))
                                .truncate()
                                .child(path),
                        ),
                );
            }
            if more > 0 {
                pulled = pulled.child(
                    div()
                        .px(px(9.))
                        .py(px(4.))
                        .text_size(px(t.text_data_sm))
                        .text_color(rgb(t.fg3))
                        .child(SharedString::from(format!("+{more} more"))),
                );
            }
            sections = sections.child(pulled);
        }

        sections
            .child(groups_sec)
            .child(views_sec)
            .child(roots_sec)
            .child(langs_sec)
    }

    fn main_view(&self, t: &Theme, cx: &mut Context<Self>, cols: usize) -> gpui::AnyElement {
        match self.view {
            View::Grid => self.grid(t, cx, cols).into_any_element(),
            View::Inbox => crate::views::inbox::render(
                &self.inbox,
                self.view_filter.as_deref(),
                t,
                &cx.entity(),
            )
            .into_any_element(),
            View::Feed => {
                crate::views::feed::render(&self.feed, self.view_filter.as_deref(), t, &cx.entity())
                    .into_any_element()
            }
            View::Explore => {
                let cloned: std::collections::HashSet<SharedString> =
                    self.rows.iter().map(|r| r.slug.clone()).collect();
                crate::views::explore::render(
                    &self.explore,
                    crate::views::explore::CloneStatus {
                        cloned: &cloned,
                        cloning: &self.explore_cloning,
                        errors: &self.explore_errors,
                    },
                    self.view_filter.as_deref(),
                    self.config.roots.first().map(|s| s.as_str()),
                    t,
                    &cx.entity(),
                )
                .into_any_element()
            }
            View::Janitor => crate::views::cleanup::render(
                &self.cleanup,
                self.view_filter.as_deref(),
                self.cleanup_confirm.as_deref(),
                t,
                &cx.entity(),
            )
            .into_any_element(),
            View::Agents => crate::views::agents::render(
                &self.agents,
                self.view_filter.as_deref(),
                self.agents_confirm.as_deref(),
                self.agents_review
                    .as_deref()
                    .map(|p| (p, self.agents_review_diff.as_deref())),
                t,
                &cx.entity(),
            )
            .into_any_element(),
            View::Log => {
                let entries: Vec<_> = self.activity_log.entries().cloned().collect();
                crate::views::log::render(&entries, t, &cx.entity()).into_any_element()
            }
            View::Tools => match &self.devtools {
                Some(d) => crate::views::devtools::render(
                    d,
                    self.view_filter.as_deref(),
                    t,
                    &cx.entity(),
                    cx,
                )
                .into_any_element(),
                None => placeholder(View::Tools, t).into_any_element(),
            },
            View::Settings => match &self.settings {
                Some(s) => crate::views::settings::render(
                    s,
                    self.view_filter.as_deref(),
                    self.services.github_authed,
                    self.services.github_token_source,
                    self.services.github_signed_out,
                    &self.services.github_device,
                    &self.services.ai_status,
                    self.services.ai_ready,
                    &self.install_kind,
                    &self.update_state,
                    self.config.auto_check_updates,
                    self.autostart_enabled,
                    t,
                    &cx.entity(),
                )
                .into_any_element(),
                None => placeholder(View::Settings, t).into_any_element(),
            },
        }
    }

    fn grid(&self, t: &Theme, cx: &mut Context<Self>, cols: usize) -> impl IntoElement {
        // The contribution graph sits pinned above the toolbar + scrolling cards.
        let band = match (self.grid.activity_open, &self.grid.activity) {
            (true, Some(activity)) => {
                Some(crate::heatmap::render(activity, t, &cx.entity()).into_any_element())
            }
            _ => None,
        };
        let visible = self.visible_rows();
        let q = self.grid.query.trim().to_lowercase();
        let host_only: Vec<&AttentionItem> = if self.grid.filter == RepoFilter::Attention {
            host_only_needs_me(&self.attention_items)
                .into_iter()
                .filter(|i| {
                    if q.is_empty() {
                        return true;
                    }
                    i.repo.name.to_lowercase().contains(&q)
                        || i.repo
                            .slug
                            .as_deref()
                            .is_some_and(|s| s.to_lowercase().contains(&q))
                        || i.summary.to_lowercase().contains(&q)
                })
                .collect()
        } else {
            Vec::new()
        };
        let header_count = visible.len() + host_only.len();
        // Select-all + global/selection ops sit above the list; Actions appears
        // only when something is selected.
        let fleet_bar = self.fleet_bar(t, cx);
        let list_area: gpui::AnyElement = if self.grid.filter == RepoFilter::Attention
            && visible.is_empty()
            && host_only.is_empty()
            && self.grid.query.trim().is_empty()
        {
            div()
                .flex()
                .flex_1()
                .min_h(px(0.))
                .items_center()
                .justify_center()
                .px(px(24.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(8.))
                        .max_w(px(420.))
                        .child(lucide("circle-check", 28., t.clean))
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_size(px(t.text_h3))
                                .text_color(rgb(t.fg0))
                                .child("All clear"),
                        )
                        .child(
                            div()
                                .text_size(px(t.text_small))
                                .text_color(rgb(t.fg2))
                                .child(
                                    "No repos need attention right now. Pull behind updates upstream checkouts; digits work stays under Working → Pushable.",
                                ),
                        ),
                )
                .into_any_element()
        } else if visible.is_empty() && !host_only.is_empty() {
            self.host_only_needs_me_list(t, &host_only, cx)
                .into_any_element()
        } else if !host_only.is_empty() {
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.))
                .child(self.host_only_needs_me_list(t, &host_only, cx))
                .child(self.card_list(t, cx, cols, visible.clone()))
                .into_any_element()
        } else {
            self.card_list(t, cx, cols, visible.clone())
                .into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(t.page))
            .children(band)
            .child(self.toolbar(t, cx, header_count))
            .child(self.ops_row(t, cx))
            .children(self.onboarding_banner(t, cx))
            .children(self.tree_focus_banner(t, cx))
            .child(list_area)
            .children(fleet_bar)
    }

    /// Host-only Needs me facts (review / CI with no local checkout) as a
    /// compact list so the badge never points at an empty Mission Control.
    fn host_only_needs_me_list(
        &self,
        t: &Theme,
        items: &[&AttentionItem],
        _cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut col = div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .px(px(16.))
            .py(px(10.))
            .flex_none();
        col = col.child(
            div()
                .text_size(px(t.text_data_sm))
                .text_color(rgb(t.fg2))
                .child("On host (no local checkout)"),
        );
        for (i, item) in items.iter().enumerate() {
            let url = item
                .detail
                .as_ref()
                .filter(|d| d.starts_with("http://") || d.starts_with("https://"))
                .cloned();
            let name = item.repo.name.clone();
            let summary = item.summary.clone();
            let sev_color = match item.severity {
                Severity::Urgent => t.behind,
                Severity::Attention => t.dirty,
                Severity::Info => t.fg2,
            };
            let mut row = div()
                .id(SharedString::from(format!("host-needs-me-{i}")))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.))
                .px(px(12.))
                .py(px(8.))
                .rounded(px(t.r_sm))
                .border_1()
                .border_color(rgb(t.border))
                .bg(rgb(t.surface))
                .child(
                    div()
                        .w(px(8.))
                        .h(px(8.))
                        .rounded(px(4.))
                        .bg(rgb(sev_color))
                        .flex_none(),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .flex_1()
                        .min_w(px(0.))
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .text_size(px(t.text_small))
                                .text_color(rgb(t.fg0))
                                .child(SharedString::from(name)),
                        )
                        .child(
                            div()
                                .text_size(px(t.text_data_sm))
                                .text_color(rgb(t.fg2))
                                .child(SharedString::from(summary)),
                        ),
                );
            if let Some(url) = url {
                row = row
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(t.surface_hover)))
                    .on_click(move |_ev, _w, _cx| {
                        let _ = repoharbor_core::launch::open(&url);
                    });
            }
            col = col.child(row);
        }
        col
    }

    /// Top Mission Control bar — filters only: title · count, search, work
    /// modes, visibility chips, mode-scoped git chips, then sort/layout.
    /// Execute ops live on [`Self::ops_row`].
    fn toolbar(&self, t: &Theme, cx: &mut Context<Self>, count: usize) -> impl IntoElement {
        let title = format!("{} · {count}", self.grid.mode.label());
        let sep = || {
            div()
                .w(px(1.))
                .h(px(18.))
                .mx(px(2.))
                .bg(rgb(t.border_strong))
        };
        let mut bar = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .px(px(16.))
            .py(px(10.))
            .border_b_1()
            .border_color(rgb(t.border))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(t.text_h3))
                    .text_color(rgb(t.fg0))
                    .child(SharedString::from(title)),
            );
        if let Some(search) = &self.repo_search {
            bar = bar.child(div().w(px(290.)).child(Input::new(search).cleanable(true)));
        }
        bar = bar
            .child(div().flex_1())
            .child(self.mode_segments(t, cx))
            .child(sep());
        // Always-visible filter chips in the upper toolbar (every work mode).
        for f in VISIBILITY_CHIPS {
            bar = bar.child(self.filter_chip(f, t, cx));
        }
        // Working-mode git chips (Dirty / Stageable / Pushable) — filters, not ops.
        let mode_chips = self.grid.mode.chips();
        if !mode_chips.is_empty() {
            bar = bar.child(sep());
            for f in mode_chips {
                bar = bar.child(self.filter_chip(*f, t, cx));
            }
        }
        // Unified segmented List | Grid control next to Sort.
        let list_active = self.grid.layout == Layout::List;
        let grid_active = self.grid.layout == Layout::Grid;
        let layout_switch = div()
            .flex()
            .flex_row()
            .items_center()
            .rounded(px(t.r_sm))
            .border_1()
            .border_color(rgb(t.border))
            .bg(rgb(t.button_bg))
            .overflow_hidden()
            .child(
                div()
                    .id("tb-list")
                    .flex()
                    .items_center()
                    .justify_center()
                    .px(px(8.))
                    .py(px(6.))
                    .bg(rgb(if list_active {
                        t.accent_wash
                    } else {
                        t.button_bg
                    }))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(t.surface_hover)))
                    .child(lucide(
                        "list",
                        14.,
                        if list_active { t.accent_bright } else { t.fg1 },
                    ))
                    .on_click(cx.listener(|this, _ev, _w, cx| this.set_layout(Layout::List, cx))),
            )
            .child(
                div()
                    .id("tb-grid")
                    .flex()
                    .items_center()
                    .justify_center()
                    .px(px(8.))
                    .py(px(6.))
                    .border_l_1()
                    .border_color(rgb(t.border))
                    .bg(rgb(if grid_active {
                        t.accent_wash
                    } else {
                        t.button_bg
                    }))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(t.surface_hover)))
                    .child(lucide(
                        "layout-grid",
                        14.,
                        if grid_active { t.accent_bright } else { t.fg1 },
                    ))
                    .on_click(cx.listener(|this, _ev, _w, cx| this.set_layout(Layout::Grid, cx))),
            );

        bar.child(sep())
            .child(tool_btn(
                "tb-sort",
                "arrow-up-down",
                Some(self.grid.sort.label()),
                None,
                false,
                t,
                cx.listener(|this, _ev, _w, cx| this.cycle_sort(cx)),
            ))
            .child(layout_switch)
    }

    /// Segmented work-mode control: Needs me | Behind | Working | All.
    fn mode_segments(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let attention_n = self.attention_count();
        let mut group = div()
            .id("mc-work-modes")
            .flex()
            .flex_row()
            .items_center()
            .rounded(px(t.r_sm))
            .border_1()
            .border_color(rgb(t.border))
            .bg(rgb(t.button_bg))
            .overflow_hidden();
        for (i, mode) in WorkMode::ORDER.iter().copied().enumerate() {
            let active = self.grid.mode == mode;
            let (bg, fg) = if active {
                (t.accent_wash, t.accent_bright)
            } else {
                (t.button_bg, t.fg1)
            };
            let label = if mode == WorkMode::NeedsMe && attention_n > 0 {
                format!("{} {attention_n}", mode.label())
            } else {
                mode.label().to_string()
            };
            // Emphasize Needs me when there is outstanding attention.
            let primary_hint = mode == WorkMode::NeedsMe && attention_n > 0 && !active;
            let fg = if primary_hint { t.accent_bright } else { fg };
            let mut seg = div()
                .id(SharedString::from(format!("mode-{}", mode.label())))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(5.))
                .px(px(10.))
                .py(px(6.))
                .bg(rgb(bg))
                .text_size(px(t.text_small))
                .text_color(rgb(fg))
                .cursor_pointer()
                .hover(move |s| s.bg(rgb(t.accent_wash)))
                .on_click(cx.listener(move |this, _ev, _w, cx| this.set_mode(mode, cx)))
                .child(SharedString::from(label));
            if i > 0 {
                seg = seg.border_l_1().border_color(rgb(t.border));
            }
            group = group.child(seg);
        }
        group
    }

    /// Bottom Mission Control strip — execute ops only, cleanly grouped into
    /// Selection, Maintenance/Discovery, and Fleet Sync/AI Commit verbs.
    fn ops_row(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let sep = || {
            div()
                .w(px(1.))
                .h(px(18.))
                .mx(px(2.))
                .bg(rgb(t.border_strong))
        };
        let mut row = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(7.))
            .px(px(16.))
            .py(px(9.))
            .bg(rgb(t.surface))
            .border_b_1()
            .border_color(rgb(t.border));
        row = row.child(self.select_all_checkbox(t, cx));
        // Disk/git rescan — labeled + accent so it isn’t a ghost icon.
        row = row
            .child(tool_btn_primary(
                "tb-refresh",
                "refresh-cw",
                "Refresh",
                t,
                cx.listener(|this, _ev, _w, cx| this.rescan_user(cx)),
            ))
            .child(sep());

        // One-click Pull for every repo behind its upstream (vendor trees).
        row = row.child(tool_btn(
            "tb-pull-behind",
            "download",
            Some("Pull behind"),
            Some("Pull only the repositories that are behind upstream"),
            false,
            t,
            cx.listener(|this, _ev, _w, cx| this.pull_behind_repos(cx)),
        ));
        // Host enrichment refresh (ignores TTL) — not a git fetch of the selection.
        row = row.child(tool_btn(
            "tb-fetch-all",
            "globe",
            Some("Enrich hosts"),
            Some("Refresh GitHub / GitLab host enrichment data (pull requests, CI status, issues)"),
            false,
            t,
            cx.listener(|this, _ev, _w, cx| this.fetch_all_hosts(cx)),
        ));
        // Always-on Submodules: empty *visible* selection → all visible parents
        // with nested checkouts; with a visible selection → selected ∩ visible.
        let sub_label = if self.selected_repos_ordered().is_empty() {
            let visible = self.visible_rows();
            let n =
                crate::fleet::submodule_update_targets(&self.rows, &self.selected, &visible).len();
            if n > 0 {
                format!("Submodules ({n})")
            } else {
                "Submodules".into()
            }
        } else {
            "Submodules".into()
        };
        row = row.child(tool_btn(
            "tb-submodules",
            "box",
            Some(sub_label.as_str()),
            Some(
                "Update nested checkouts — selection when checked, else every visible parent with submodules",
            ),
            false,
            t,
            cx.listener(|this, _ev, _w, cx| this.run_submodule_update(cx)),
        ));
        if self.services.ai_ready {
            row = row.child(tool_btn(
                "tb-summarize",
                "sparkles",
                Some("Summarize"),
                Some("One-line AI summaries for each repo (cached by commit)"),
                false,
                t,
                cx.listener(|this, _ev, _w, cx| this.summarize_all(cx)),
            ));
        }
        // Fleet sync primaries (Fetch, Pull, and selection actions).
        row = row
            .child(sep())
            .child(self.fleet_primary_sync_buttons(t, cx));

        let selected_n = self.selected_repos_ordered().len();
        if selected_n > 0 {
            row = row.child(self.fleet_actions_button(t, cx)).child(
                div()
                    .px(px(7.))
                    .py(px(2.))
                    .rounded(px(t.r_xs))
                    .bg(rgb(t.accent_wash))
                    .border_1()
                    .border_color(rgb(t.border_accent))
                    .font_family("monospace")
                    .text_size(px(t.text_data_sm))
                    .text_color(rgb(t.accent_bright))
                    .child(SharedString::from(format!("{selected_n} selected"))),
            );
        }
        row
    }

    /// One rounded filter chip (icon + label) matching Mission Control styling.
    fn filter_chip(&self, f: RepoFilter, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let active = if is_visibility_filter(f) {
            self.grid.visibility == Some(f)
        } else {
            self.grid.filter == f
        };
        let (bg, border, fg) = if active {
            (t.accent_wash, t.border_accent, t.accent_bright)
        } else {
            (t.button_bg, t.border, t.fg1)
        };
        let hov = t.border_strong;
        let mut chip = div()
            .id(SharedString::from(format!("chip-{}", f.label())))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(5.))
            .px(px(11.))
            .py(px(5.))
            .rounded_full()
            .bg(rgb(bg))
            .border_1()
            .border_color(rgb(border))
            .text_size(px(t.text_small))
            .text_color(rgb(fg))
            .cursor_pointer()
            .hover(move |s| s.border_color(rgb(hov)))
            .on_click(cx.listener(move |this, _ev, _w, cx| this.set_filter(f, cx)));
        if let Some(icon) = f.icon() {
            chip = chip.child(lucide(icon, 13., fg));
        }
        chip.child(SharedString::from(f.label()))
    }

    /// Fleet select-all checkbox at the top of the repo list (next to All).
    /// Unique id `mc-select-all` — must not collide with card checkboxes.
    fn select_all_checkbox(&self, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.all_visible_selected();
        // Selection edits stay available while a fleet run is active (next
        // op can target a different set and enqueue). Confirm strips still lock.
        let idle = self.fleet_actions_idle();
        let (border_c, bg_c) = if selected {
            (t.primary, t.primary)
        } else {
            (t.border_strong, t.button_bg)
        };
        let hov = t.primary;
        let mut b = div()
            .id("mc-select-all")
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .w(px(16.))
            .h(px(16.))
            .rounded(px(t.r_xs))
            .border_1()
            .border_color(rgb(border_c))
            .bg(rgb(bg_c));
        if idle {
            b = b
                .cursor_pointer()
                .hover(move |s| s.border_color(rgb(hov)))
                .on_click(cx.listener(|this, _ev, _w, cx| {
                    this.toggle_select_all_visible(cx);
                }));
        }
        if selected {
            b = b.child(lucide("check", 12., t.page));
        }
        b
    }

    fn card_list(
        &self,
        t: &Theme,
        cx: &mut Context<Self>,
        cols: usize,
        visible: Vec<usize>,
    ) -> impl IntoElement {
        let entity = cx.entity();
        let theme = self.theme.clone();
        let ide = self.config.ide_command.clone();
        let agent = self.config.agent_command.clone();

        let list = match self.grid.layout {
            // Compact single-column list: one repo per row.
            Layout::List => {
                // Same all-or-nothing height as Grid: the AI sparkles line is
                // uniform across the list so rows don't clip mid-viewport.
                let has_ai = visible.iter().any(|&i| !self.rows[i].ai_summary.is_empty());
                let row_h = if has_ai { LIST_H_AI } else { LIST_H };
                gpui::uniform_list("repo-list", visible.len(), move |range, _win, cx| {
                    let app = entity.read(cx);
                    let selecting = !app.selected.is_empty();
                    range
                        .map(|i| {
                            let abs = visible[i];
                            crate::card::list_item(
                                &app.rows[abs],
                                abs,
                                &theme,
                                &entity,
                                &ide,
                                &agent,
                                app.card_state(abs, selecting),
                                row_h,
                            )
                            .into_any_element()
                        })
                        .collect()
                })
            }
            // Multi-column card grid (one uniform_list row = `cols` cards).
            Layout::Grid => {
                let grid_rows = visible.len().div_ceil(cols);
                // uniform_list needs one row height, so size it to the tallest
                // card. The AI-summary line is all-or-nothing per user (gated on
                // aiReady), so pick the taller height only when summaries are
                // present — keeping cards snug either way rather than clipping
                // the launcher row at the bottom.
                let has_ai = visible.iter().any(|&i| !self.rows[i].ai_summary.is_empty());
                let row_h = if has_ai { ROW_H_AI } else { ROW_H };
                gpui::uniform_list("repo-grid", grid_rows, move |range, _win, cx| {
                    let app = entity.read(cx);
                    let selecting = !app.selected.is_empty();
                    range
                        .map(|gi| {
                            let start = gi * cols;
                            let end = (start + cols).min(visible.len());
                            // Map each grid slot to its absolute row index (so the
                            // card's favorite toggle keeps editing the right row).
                            let mut cells: Vec<gpui::AnyElement> = visible[start..end]
                                .iter()
                                .map(|&i| {
                                    let state = app.card_state(i, selecting);
                                    card(&app.rows[i], i, &theme, &entity, &ide, &agent, state)
                                        .into_any_element()
                                })
                                .collect();
                            while cells.len() < cols {
                                cells.push(div().flex_1().min_w(px(0.)).into_any_element());
                            }
                            // w_full so the row fills the list width and the flex_1
                            // cells divide it equally — otherwise the row shrink-
                            // wraps to content width and overflows horizontally.
                            div()
                                .w_full()
                                .flex()
                                .flex_row()
                                .items_stretch()
                                .h(px(row_h))
                                .gap(px(12.))
                                .px(px(16.))
                                .py(px(8.))
                                .children(cells)
                                .into_any_element()
                        })
                        .collect()
                })
            }
        };
        list.flex_1().size_full().bg(rgb(t.page))
    }
}

/// Toast copy after a successful summarize pass that wrote `n` new summaries.
fn summarize_finish_toast(n: usize) -> (ToastKind, SharedString, Option<SharedString>) {
    let detail = if n == 1 {
        "1 new summary".into()
    } else {
        format!("{n} new summaries").into()
    };
    (ToastKind::Success, "Summaries ready".into(), Some(detail))
}

/// A toolbar action button: a lucide icon with an optional label, highlighted
/// when `active`. `on` fires on click. Optional `tooltip` shows on hover.
fn tool_btn(
    id: &'static str,
    icon: &'static str,
    label: Option<&str>,
    tooltip: Option<&'static str>,
    active: bool,
    t: &Theme,
    on: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let (bg, border, fg) = if active {
        (t.accent_wash, t.border_accent, t.accent_bright)
    } else {
        (t.button_bg, t.border, t.fg1)
    };
    let hov = t.border_strong;
    let mut b = div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.))
        .px(px(10.))
        .py(px(6.))
        .rounded(px(t.r_sm))
        .bg(rgb(bg))
        .border_1()
        .border_color(rgb(border))
        .text_size(px(t.text_small))
        .text_color(rgb(fg))
        .cursor_pointer()
        .hover(move |s| s.border_color(rgb(hov)))
        .on_click(on)
        .child(lucide(icon, 15., fg));
    if let Some(label) = label {
        b = b.child(SharedString::from(label.to_string()));
    }
    if let Some(tip) = tooltip {
        let tip = SharedString::from(tip);
        b = b.tooltip(move |window, cx| {
            gpui_component::tooltip::Tooltip::new(tip.clone()).build(window, cx)
        });
    }
    b
}

/// Always-on primary toolbar control (icon + label, accent wash, `fg0` text).
fn tool_btn_primary(
    id: &'static str,
    icon: &'static str,
    label: &'static str,
    t: &Theme,
    on: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let hov = t.border_strong;
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.))
        .px(px(12.))
        .py(px(6.))
        .rounded(px(t.r_sm))
        .bg(rgb(t.accent_wash))
        .border_1()
        .border_color(rgb(t.border_accent))
        .text_size(px(t.text_small))
        .text_color(rgb(t.fg0))
        .cursor_pointer()
        .hover(move |s| s.border_color(rgb(hov)))
        .on_click(on)
        .child(lucide(icon, 15., t.accent_bright))
        .child(SharedString::from(label))
}

/// Responsive column count from the window width: aim for ~340px-wide cards
/// (after the sidebar), clamped to a sensible range.
fn columns(viewport_width: f32, sidebar_w: f32) -> usize {
    (((viewport_width - sidebar_w) / 340.).floor() as usize).clamp(1, 6)
}

impl Render for RepoHarborApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_repo_search(window, cx);
        if self.terminal.open {
            self.ensure_terminal_cmd_input(window, cx);
            if self.terminal.pending_focus {
                self.terminal.pending_focus = false;
                if let Some(tab) = self.terminal.active() {
                    window.focus(&tab.focus, cx);
                }
            }
        }
        let t = self.theme.clone();
        let sidebar_w = self.sidebar_w();
        // Responsive grid columns from the current window width.
        let cols = columns(f32::from(window.viewport_size().width), sidebar_w);

        // Client-side window chrome (min / max / close). On Wayland GPUI defaults
        // to CSD, so without this bar the window has no control buttons.
        // Default TitleBar close calls `remove_window` / quits — do not remap
        // to minimize (that made ✕ feel broken).
        // CSD drag strip + window controls only. Brand/count once here —
        // the chrome bar below is tabs + search (no second RepoHarbor row).
        let update_pill = match &self.update_state {
            AppUpdateState::Available(rel) => Some(
                div()
                    .id("tb-update-pill")
                    .px(px(7.))
                    .py(px(1.))
                    .rounded(px(t.r_xs))
                    .bg(rgb(t.accent_wash))
                    .border_1()
                    .border_color(rgb(t.border_accent))
                    .font_family("monospace")
                    .text_size(px(t.text_data_sm))
                    .text_color(rgb(t.clean))
                    .cursor_pointer()
                    .child(SharedString::from(format!("↑ {}", rel.tag_name)))
                    .on_click(cx.listener(|this, _e, window, cx| {
                        this.open_about_updates(window, cx);
                    })),
            ),
            _ => None,
        };

        let title_bar = TitleBar::new()
            .bg(rgb(t.button_bg))
            .border_color(rgb(t.border_strong))
            .text_color(rgb(t.fg0))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .pl(px(4.))
                    .child(lucide("harbor", 13., t.primary))
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .text_size(px(12.))
                            .text_color(rgb(t.fg1))
                            .child("RepoHarbor"),
                    )
                    .child(
                        div()
                            .px(px(5.))
                            .py(px(1.))
                            .rounded(px(t.r_xs))
                            .bg(rgb(t.surface))
                            .border_1()
                            .border_color(rgb(t.border))
                            .font_family("monospace")
                            .text_size(px(t.text_data_sm))
                            .text_color(rgb(t.fg2))
                            .child(SharedString::from(format!(
                                "v{}",
                                crate::identity::APP_VERSION
                            ))),
                    )
                    .children(update_pill)
                    .child(
                        div()
                            .font_family("monospace")
                            .text_size(px(t.text_data_sm))
                            .text_color(rgb(t.fg3))
                            .child(SharedString::from(format!(
                                "{} roots · {} repos",
                                self.roots,
                                self.rows.len()
                            ))),
                    ),
            );

        let measured_dims = self.default_terminal_dims(Some(window));
        if self.terminal.last_dims != measured_dims {
            self.terminal.last_dims = measured_dims;
            for tab in &self.terminal.tabs {
                tab.backend.resize(measured_dims.0, measured_dims.1);
            }
        }

        let shell = div()
            .flex()
            .flex_col()
            .size_full()
            .overflow_hidden()
            .bg(rgb(t.page))
            .text_color(rgb(t.fg1))
            .font_family("sans-serif")
            .child(title_bar)
            .child(self.chrome_bar(&t, cx))
            .child({
                let mut workspace_col = div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.))
                    .min_h(px(0.))
                    .overflow_hidden()
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_h(px(0.))
                            .min_w(px(0.))
                            .child(self.main_view(&t, cx, cols)),
                    );
                if self.terminal.open {
                    workspace_col = workspace_col.child(crate::terminal_view::render_dock(
                        &self.terminal,
                        self.config.terminal_auto_show_log,
                        sidebar_w,
                        &t,
                        &cx.entity(),
                        window,
                        cx,
                    ));
                }
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_w(px(0.))
                    .min_h(px(0.))
                    .overflow_hidden()
                    .child(self.sidebar(&t, cx))
                    .child(workspace_col)
            });

        // The shell, with overlays (drawer/palette/dialog) then toasts on top
        // so AI/fleet errors stay visible above the drawer (toasts are
        // content-sized bottom-right — they don't block UI).
        let mut root = div()
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &crate::ToggleTerminal, window, cx| {
                this.toggle_terminal(Some(window), cx);
            }))
            .on_action(cx.listener(|this, _: &crate::CloseOverlay, window, cx| {
                if this.notice_detail.is_some() {
                    this.close_notice(cx);
                    window.focus(&this.focus, cx);
                    cx.notify();
                } else if this.fleet_gen_push.is_some() {
                    // Centered confirm modal — dismiss before drawer/palette.
                    this.cancel_fleet_gen_push(cx);
                } else if this.overlay.is_some() {
                    this.close_overlay();
                    window.focus(&this.focus, cx);
                    cx.notify();
                } else if this.fleet_discard.is_some() {
                    this.cancel_fleet_discard(cx);
                } else if this.fleet_reset.is_some() {
                    this.cancel_fleet_reset(cx);
                } else if this.fleet_commit.is_some() {
                    this.cancel_fleet_commit(cx);
                } else if this.fleet_merge.is_some() {
                    this.cancel_fleet_merge(cx);
                } else if this.fleet_prune.is_some() {
                    // A pending prune confirm dismisses first, keeping the
                    // selection — a second Esc clears that.
                    this.cancel_fleet_prune(cx);
                } else if !this.grid.query.is_empty() {
                    this.clear_repo_query(window, cx);
                } else if !this.selected.is_empty() {
                    // No overlay to dismiss — Esc clears the fleet selection.
                    this.clear_selection(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &crate::FocusSearch, window, cx| {
                this.focus_repo_search(window, cx);
            }))
            .on_action(cx.listener(|this, _: &crate::OpenPalette, window, cx| {
                this.open_palette(window, cx);
            }))
            .on_action(cx.listener(|this, _: &crate::PaletteDown, _window, cx| {
                this.move_palette(1, cx);
            }))
            .on_action(cx.listener(|this, _: &crate::PaletteUp, _window, cx| {
                this.move_palette(-1, cx);
            }))
            .on_action(cx.listener(|this, _: &crate::PaletteConfirm, window, cx| {
                this.confirm_palette(window, cx);
            }))
            .on_action(
                cx.listener(|this, _: &crate::FleetFetchSelected, _window, cx| {
                    if !this.ensure_fleet_actions_idle(cx) {
                        return;
                    }
                    this.run_fleet(crate::fleet::FleetOp::Fetch, cx);
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::FleetPullSelected, _window, cx| {
                    if !this.ensure_fleet_actions_idle(cx) {
                        return;
                    }
                    this.run_fleet(crate::fleet::FleetOp::Pull, cx);
                }),
            )
            // Sidebar + terminal dock resize: track mouse while a handle is held.
            .on_mouse_move(cx.listener(|this, _ev, window, cx| {
                if this.terminal.dragging {
                    let vh = f32::from(window.viewport_size().height);
                    let y = f32::from(window.mouse_position().y);
                    let next = (vh - y).clamp(
                        crate::terminal_view::DOCK_MIN_H,
                        crate::terminal_view::DOCK_MAX_H,
                    );
                    if (next - this.terminal.height).abs() >= 1. {
                        this.terminal.height = next;
                        this.terminal.maximized = false;
                        cx.notify();
                    }
                    return;
                }
                if !this.sidebar_dragging || this.sidebar_collapsed {
                    return;
                }
                let x = f32::from(window.mouse_position().x);
                let next = x.clamp(SIDEBAR_MIN, SIDEBAR_MAX);
                if (next - this.sidebar_width).abs() >= 1. {
                    this.sidebar_width = next;
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _ev, _window, cx| {
                    if this.terminal.dragging {
                        this.terminal.dragging = false;
                        this.config.terminal_height = this.terminal.height;
                        let _ = repoharbor_core::config::save(&this.config);
                        cx.notify();
                    }
                    if let Some(sel) = &mut this.terminal.selection
                        && sel.dragging
                    {
                        sel.dragging = false;
                        if !sel.has_range() {
                            this.terminal.selection = None;
                        }
                        cx.notify();
                    }
                    if this.sidebar_dragging {
                        this.sidebar_dragging = false;
                        this.persist_sidebar_chrome();
                        cx.notify();
                    }
                }),
            )
            .relative()
            .size_full()
            .child(shell);
        if let Some(overlay) = self.overlay_element(&t, cx) {
            root = root.child(overlay);
        }
        if let Some(toasts) = self.toast_layer(&t, cx) {
            root = root.child(toasts);
        }
        if let Some(entry) = &self.notice_detail {
            root = root
                .child(crate::views::notice::render(entry, &t, &cx.entity()).into_any_element());
        }
        if let Some(repos) = &self.fleet_gen_push {
            let names = crate::views::fleet_gen_push::repo_names(self, repos);
            root = root.child(
                crate::views::fleet_gen_push::render(repos, &t, &cx.entity(), &names)
                    .into_any_element(),
            );
        }
        root
    }
}

impl RepoHarborApp {
    /// Build the active overlay's element, if one is open. Returns `None` when
    /// the drawer's repo has vanished (e.g. a rescan dropped it) — which also
    /// leaves the stale overlay to be cleared on the next interaction.
    fn overlay_element(&self, t: &Theme, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        match &self.overlay {
            Some(Overlay::Drawer { repo, tab }) => {
                let row = self.rows.iter().find(|r| &r.id == repo)?;
                let cmds = (
                    self.config.ide_command.clone(),
                    self.config.agent_command.clone(),
                );
                Some(
                    crate::drawer::drawer(
                        row,
                        *tab,
                        t,
                        &cx.entity(),
                        &self.drawer,
                        &cmds.0,
                        &cmds.1,
                        self.services.ai_ready,
                        self.services.github_authed,
                    )
                    .into_any_element(),
                )
            }
            Some(Overlay::Palette(data)) => {
                let query = data.query.read(cx).value();
                let items = crate::palette::items(
                    &self.rows,
                    &data.code,
                    &data.semantic,
                    &query,
                    !self.selected_repos_ordered().is_empty(),
                );
                Some(
                    crate::palette::render(data, &items, &self.rows, &query, t, &cx.entity())
                        .into_any_element(),
                )
            }
            Some(Overlay::NewProject(data)) => Some(
                crate::views::newproject::render(data, &self.config.roots, t, &cx.entity())
                    .into_any_element(),
            ),
            None => None,
        }
    }
}

/// An uppercase sidebar section header (ROOTS / LANGUAGES).
fn section_header(label: &'static str, t: &Theme) -> impl IntoElement {
    div()
        .px(px(9.))
        .pb(px(2.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(px(t.text_data_sm))
        .text_color(rgb(t.fg3))
        .child(label)
}

/// A section header with a trailing icon action (e.g. VIEWS + to save a view).
fn section_header_action(
    label: &'static str,
    icon: &'static str,
    t: &Theme,
    on: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let hov = t.surface_hover;
    div()
        .flex()
        .flex_row()
        .items_center()
        .pl(px(9.))
        .pb(px(2.))
        .child(
            div()
                .flex_1()
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(t.text_data_sm))
                .text_color(rgb(t.fg3))
                .child(label),
        )
        .child(
            div()
                .id(SharedString::from(format!("hdr-{label}")))
                .flex()
                .items_center()
                .justify_center()
                .w(px(18.))
                .h(px(18.))
                .rounded(px(t.r_xs))
                .cursor_pointer()
                .hover(move |s| s.bg(rgb(hov)))
                .child(lucide(icon, 13., t.fg3))
                .on_click(on),
        )
}

/// One clickable sidebar filter row: leading mark, label, and a right-aligned
/// count. Highlighted when `active`. `on` fires on click.
fn sidebar_filter_item(
    id: SharedString,
    leading: gpui::AnyElement,
    label: SharedString,
    count: Option<usize>,
    active: bool,
    t: &Theme,
    on: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let fg = if active { t.accent_bright } else { t.fg1 };
    let hov = t.surface_hover;
    let mut item = div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .gap(px(9.))
        .px(px(9.))
        .py(px(6.))
        .rounded(px(t.r_sm))
        .text_size(px(t.text_small))
        .text_color(rgb(fg))
        .cursor_pointer()
        .hover(move |s| s.bg(rgb(hov)))
        .on_click(on)
        .child(leading)
        .child(div().flex_1().min_w(px(0.)).truncate().child(label))
        // Count is right-aligned and optional (section selectors omit it).
        .children(count.map(|n| {
            div()
                .font_family("monospace")
                .text_size(px(t.text_data_sm))
                .text_color(rgb(t.fg3))
                .child(SharedString::from(n.to_string()))
        }));
    if active {
        item = item.bg(rgb(t.accent_wash));
    }
    item
}

/// A small count pill for the sidebar nav. Urgent counts get the danger
/// tint; everything else stays a neutral chip.
fn badge(n: usize, urgent: bool, t: &Theme) -> impl IntoElement {
    let (bg, fg) = if urgent {
        (t.danger_badge, t.behind)
    } else {
        (t.button_bg, t.fg2)
    };
    div()
        .flex()
        .items_center()
        .justify_center()
        .min_w(px(18.))
        .px(px(5.))
        .py(px(1.))
        .rounded(px(t.r_xs))
        .bg(rgb(bg))
        .font_family("monospace")
        .text_size(px(t.text_data_sm))
        .text_color(rgb(fg))
        .child(SharedString::from(n.to_string()))
}

/// Scaffold for a not-yet-ported view: centered title + note.
fn placeholder(view: View, t: &Theme) -> impl IntoElement {
    let (title, sub): (&str, &str) = match view {
        View::Inbox => ("Inbox", "Review queue — PRs & notifications awaiting you"),
        View::Feed => ("Feed", "Activity stream across your repos"),
        View::Explore => ("Explore", "Discover & search across hosts"),
        View::Agents => ("Agents", "Running terminal coding-agent sessions"),
        View::Tools => ("Dev Tools", "Utilities & quick actions"),
        View::Janitor => ("Cleanup", "Prunable branches & worktrees"),
        View::Log => ("Log", "Recent app events — scans, fleet ops, push/pull"),
        View::Settings => ("Settings", "Roots, AI, launchers, about"),
        View::Grid => ("Mission Control", ""),
    };
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .size_full()
        .gap(px(8.))
        .bg(rgb(t.page))
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(22.))
                .text_color(rgb(t.fg0))
                .child(SharedString::from(title.to_string())),
        )
        .child(
            div()
                .text_size(px(t.text_small))
                .text_color(rgb(t.fg2))
                .child(SharedString::from(sub.to_string())),
        )
        .child(
            div()
                .mt(px(6.))
                .px(px(10.))
                .py(px(4.))
                .rounded(px(t.r_xs))
                .border_1()
                .border_color(rgb(t.border))
                .font_family("monospace")
                .text_size(px(t.text_data_sm))
                .text_color(rgb(t.fg3))
                .child("Phase 2 scaffold"),
        )
}

/// Stable tab ID for the built-in Action Log stream inside the bottom dock.
pub const ACTION_LOG_TAB_ID: &str = "term-action-log";

impl RepoHarborApp {
    /// Grid dimensions estimated for a newly spawned tab before first dock layout.
    fn default_terminal_dims(&self, window: Option<&Window>) -> (u16, u16) {
        if let Some(win) = window {
            let vp = win.viewport_size();
            let cell_w = crate::terminal_view::measure_cell_width(win);
            let body_w = (f32::from(vp.width) - self.sidebar_w()).max(320.0);
            let body_h = (self.terminal.height - 82.0).max(80.0);
            crate::terminal_view::grid_dimensions_with(body_w, body_h, cell_w)
        } else {
            self.terminal.last_dims
        }
    }

    /// Ensure the read-only `Action Log` VT100 stream tab exists inside the dock.
    pub fn ensure_action_log_tab(
        &mut self,
        reveal: bool,
        cx: &mut Context<Self>,
    ) -> std::sync::Arc<repoharbor_core::terminal::TerminalLogStream> {
        self.ensure_terminal_drain(cx);
        if let Some(existing) = self
            .terminal
            .tabs
            .iter()
            .find_map(|t| t.backend.as_log().cloned())
        {
            if reveal && self.config.terminal_auto_show_log {
                self.terminal.open = true;
                self.terminal.active_tab = Some(ACTION_LOG_TAB_ID.to_string());
                self.terminal.pending_confirm = None;
            }
            return existing;
        }

        let (rows, cols) = self.default_terminal_dims(None);
        let stream = std::sync::Arc::new(repoharbor_core::terminal::TerminalLogStream::new(
            ACTION_LOG_TAB_ID,
            rows,
            cols,
            self.terminal.tx.clone(),
        ));
        let snapshot = stream.snapshot();
        let tab = crate::terminal_view::TerminalTab {
            id: ACTION_LOG_TAB_ID.to_string(),
            title: SharedString::from("Action Log"),
            cwd: SharedString::default(),
            backend: crate::terminal_view::TerminalBackend::Log(std::sync::Arc::clone(&stream)),
            focus: cx.focus_handle(),
            snapshot,
            exited: None,
        };
        self.terminal.tabs.insert(0, tab);
        if reveal && self.config.terminal_auto_show_log {
            self.terminal.open = true;
            self.terminal.active_tab = Some(ACTION_LOG_TAB_ID.to_string());
            self.terminal.pending_confirm = None;
        }
        stream
    }

    /// Toggle whether toolbar/fleet/git actions auto-open the bottom Action Log tab.
    pub fn toggle_terminal_auto_log(&mut self, cx: &mut Context<Self>) {
        self.config.terminal_auto_show_log = !self.config.terminal_auto_show_log;
        if let Some(s) = &mut self.settings {
            s.draft.terminal_auto_show_log = self.config.terminal_auto_show_log;
        }
        let _ = repoharbor_core::config::save(&self.config);
        cx.notify();
    }

    fn append_action_ansi(&mut self, reveal: bool, ansi: &str, cx: &mut Context<Self>) {
        let stream = self.ensure_action_log_tab(reveal, cx);
        stream.append_ansi(ansi);
        if let Some(tab) = self
            .terminal
            .tabs
            .iter_mut()
            .find(|t| t.id == ACTION_LOG_TAB_ID)
        {
            tab.snapshot = tab.backend.snapshot();
        }
        cx.notify();
    }

    /// Append a command header (`[HH:MM:SS] $ <cmd>  (<scope>)`) to the bottom Action Log.
    pub fn log_action_cmd(
        &mut self,
        command: &str,
        scope: Option<&str>,
        reveal: bool,
        cx: &mut Context<Self>,
    ) {
        let now = chrono::Local::now().format("%H:%M:%S").to_string();
        let line = repoharbor_core::terminal::format_log_command(&now, command, scope);
        self.append_action_ansi(reveal, &line, cx);
    }

    /// Append a structured outcome line (`✓`/`↷`/`✗`) to the bottom Action Log.
    pub fn log_action_result(
        &mut self,
        tag: &str,
        sgr_color: &str,
        subject: &str,
        detail: Option<&str>,
        reveal: bool,
        cx: &mut Context<Self>,
    ) {
        let now = chrono::Local::now().format("%H:%M:%S").to_string();
        let subject = crate::data::oneline(subject.to_string());
        let detail = detail.map(|d| crate::data::oneline(d.to_string()));
        let line = repoharbor_core::terminal::format_log_entry(
            &now,
            tag,
            sgr_color,
            &subject,
            detail.as_deref(),
        );
        self.append_action_ansi(reveal, &line, cx);
    }

    /// Mirror a non-fleet toast event cleanly into the bottom Action Log stream.
    pub fn log_action_toast(
        &mut self,
        kind: ToastKind,
        title: &str,
        detail: Option<&str>,
        reveal: bool,
        cx: &mut Context<Self>,
    ) {
        let (tag, sgr) = match kind {
            ToastKind::Success => ("✓", "32"),
            ToastKind::Error => ("✗", "31"),
            ToastKind::Info => ("•", "33"),
            ToastKind::Progress => ("…", "36"),
        };
        self.log_action_result(tag, sgr, title, detail, reveal, cx);
    }

    /// Ensure the dock's quick-command `InputState` exists and submits on Enter.
    pub fn ensure_terminal_cmd_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.terminal.cmd_input.is_some() {
            return;
        }
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Run any command in active terminal… (Enter)")
        });
        let sub = cx.subscribe_in(
            &input,
            window,
            |this, _input, ev: &gpui_component::input::InputEvent, window, cx| {
                if let gpui_component::input::InputEvent::PressEnter { .. } = ev {
                    this.terminal_run_input(window, cx, true);
                }
            },
        );
        self.terminal.cmd_input = Some(input);
        self.terminal._cmd_sub = Some(sub);
    }

    /// Start the single background PTY event drainer onto the GPUI foreground.
    pub fn ensure_terminal_drain(&mut self, cx: &mut Context<Self>) {
        if self.terminal.draining {
            return;
        }
        self.terminal.draining = true;
        let rx = self.terminal.rx.clone();
        cx.spawn(async move |this, cx| {
            while let Ok(first) = rx.recv().await {
                let mut batch = vec![first];
                while let Ok(more) = rx.try_recv() {
                    batch.push(more);
                    if batch.len() >= 64 {
                        break;
                    }
                }
                let updated = this.update(cx, |this, cx| {
                    let mut dirty = std::collections::HashSet::new();
                    let mut exits = Vec::new();
                    for ev in batch {
                        match ev {
                            repoharbor_core::terminal::TerminalEvent::Output(id) => {
                                dirty.insert(id);
                            }
                            repoharbor_core::terminal::TerminalEvent::Exited { id, code } => {
                                dirty.insert(id.clone());
                                exits.push((id, code));
                            }
                        }
                    }
                    for tab in &mut this.terminal.tabs {
                        if dirty.contains(&tab.id) {
                            tab.snapshot = tab.backend.snapshot();
                            if let Some((_, code)) = exits.iter().find(|(eid, _)| eid == &tab.id) {
                                tab.exited = Some(*code);
                            }
                        }
                    }
                    cx.notify();
                });
                if updated.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// Pick a sensible default directory and tab label for a new shell session.
    fn pick_default_terminal_target(&self) -> (String, String) {
        if let Some(Overlay::Drawer { repo, .. }) = &self.overlay
            && std::path::Path::new(repo.as_ref()).is_dir()
        {
            return (repo.to_string(), repo_tab_title(repo.as_ref()));
        }
        if let Some(repo) = self.selected.iter().next()
            && std::path::Path::new(repo.as_ref()).is_dir()
        {
            return (repo.to_string(), repo_tab_title(repo.as_ref()));
        }
        if let Some(tab) = self.terminal.active()
            && !tab.backend.is_log()
            && !tab.cwd.is_empty()
        {
            return (tab.cwd.to_string(), tab.title.to_string());
        }
        if let Some(row) = self.rows.first() {
            return (row.id.to_string(), row.name.to_string());
        }
        let home = std::env::var("HOME")
            .ok()
            .filter(|h| !h.is_empty())
            .unwrap_or_else(|| "/".to_string());
        (home, "Shell".to_string())
    }

    /// Toggle the built-in bottom terminal dock (`Ctrl+`` or Header button).
    pub fn toggle_terminal(&mut self, window: Option<&mut Window>, cx: &mut Context<Self>) {
        if self.terminal.open {
            self.terminal.open = false;
            if let Some(win) = window {
                win.focus(&self.focus, cx);
            }
            cx.notify();
            return;
        }
        self.terminal.open = true;
        if self.terminal.tabs.is_empty() {
            let (cwd, title) = self.pick_default_terminal_target();
            self.open_terminal_tab(&cwd, &title, None, false, false, window, cx);
            return;
        }
        if let Some(tab) = self.terminal.active() {
            if let Some(win) = window {
                win.focus(&tab.focus, cx);
            } else {
                self.terminal.pending_focus = true;
            }
        }
        cx.notify();
    }

    /// Spawn a brand-new tab (`+ New` button or `Ctrl+Shift+T`).
    pub fn open_default_terminal_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (cwd, title) = self.pick_default_terminal_target();
        self.open_terminal_tab(&cwd, &title, None, false, false, Some(window), cx);
    }

    /// Open a terminal tab at `cwd`, optionally running or pasting `cmd`.
    ///
    /// When `reuse_tab` is true, an existing live (non-exited, non-alternate-screen)
    /// shell tab in `cwd` is focused and reused instead of opening a duplicate shell.
    /// Potentially destructive commands stay unexecuted on the prompt behind a
    /// safety confirm strip when `config.terminal_confirm_destructive` is set.
    pub fn open_terminal_in(
        &mut self,
        cwd: &str,
        title: &str,
        cmd: Option<&str>,
        execute: bool,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        self.open_terminal_tab(cwd, title, cmd, execute, true, window, cx);
    }

    #[allow(clippy::too_many_arguments)]
    fn open_terminal_tab(
        &mut self,
        cwd: &str,
        title: &str,
        cmd: Option<&str>,
        execute: bool,
        reuse_tab: bool,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        self.terminal.open = true;
        self.terminal.pending_confirm = None;
        self.ensure_terminal_drain(cx);

        let existing_id = if reuse_tab {
            self.terminal
                .tabs
                .iter()
                .find(|t| {
                    !t.backend.is_log()
                        && t.cwd.as_ref() == cwd
                        && t.exited.is_none()
                        && !t.snapshot.alternate_screen
                })
                .map(|t| t.id.clone())
        } else {
            None
        };

        let target_id = if let Some(id) = existing_id {
            id
        } else {
            let (rows, cols) = self.default_terminal_dims(window.as_deref());
            let id = self.terminal.next_id();
            let tx = self.terminal.tx.clone();
            match repoharbor_core::terminal::TerminalSession::spawn(
                id.clone(),
                cwd,
                &self.config.terminal_shell,
                rows,
                cols,
                tx,
            ) {
                Ok(session) => {
                    let session = std::sync::Arc::new(session);
                    let snapshot = session.snapshot();
                    let tab = crate::terminal_view::TerminalTab {
                        id: id.clone(),
                        title: SharedString::from(title.to_string()),
                        cwd: SharedString::from(cwd.to_string()),
                        backend: crate::terminal_view::TerminalBackend::Pty(session),
                        focus: cx.focus_handle(),
                        snapshot,
                        exited: None,
                    };
                    self.terminal.tabs.push(tab);
                    id
                }
                Err(e) => {
                    self.push_toast(
                        ToastKind::Error,
                        "Failed to open embedded terminal",
                        Some(e.into()),
                        cx,
                    );
                    cx.notify();
                    return;
                }
            }
        };

        self.terminal.active_tab = Some(target_id);
        self.send_terminal_command_if_any(cwd, title, cmd, execute, cx);

        if let Some(tab) = self.terminal.active() {
            if let Some(win) = window {
                win.focus(&tab.focus, cx);
            } else {
                self.terminal.pending_focus = true;
            }
        }
        cx.notify();
    }

    fn send_terminal_command_if_any(
        &mut self,
        cwd: &str,
        title: &str,
        cmd: Option<&str>,
        execute: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(raw) = cmd.map(str::trim).filter(|c| !c.is_empty()) else {
            return;
        };
        let dangerous = execute
            && self.config.terminal_confirm_destructive
            && repoharbor_core::terminal::is_potentially_destructive(raw);

        if let Some(tab) = self.terminal.active_mut()
            && let Some(pty) = tab.backend.as_pty()
        {
            if dangerous {
                let _ = pty.run_command(raw, false);
                tab.snapshot = tab.backend.snapshot();
                self.terminal.pending_confirm = Some(crate::terminal_view::PendingCommand {
                    cwd: SharedString::from(cwd.to_string()),
                    title: SharedString::from(title.to_string()),
                    command: SharedString::from(raw.to_string()),
                });
            } else if let Err(e) = pty.run_command(raw, execute) {
                self.push_toast(
                    ToastKind::Error,
                    "Terminal command write failed",
                    Some(e.into()),
                    cx,
                );
            } else {
                tab.snapshot = tab.backend.snapshot();
            }
        }
    }

    /// Execute the pending destructive command pasted onto the active PTY prompt.
    pub fn terminal_confirm_pending(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(_pending) = self.terminal.pending_confirm.take() else {
            return;
        };
        if let Some(tab) = self.terminal.active_mut()
            && let Some(pty) = tab.backend.as_pty()
        {
            let _ = pty.write_bytes(b"\r");
            tab.snapshot = tab.backend.snapshot();
            window.focus(&tab.focus, cx);
        }
        cx.notify();
    }

    pub fn select_terminal_tab(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.terminal.active_tab.as_deref() != Some(id) {
            self.terminal.pending_confirm = None;
            self.terminal.selection = None;
        }
        self.terminal.active_tab = Some(id.to_string());
        if let Some(tab) = self.terminal.active() {
            window.focus(&tab.focus, cx);
        }
        cx.notify();
    }

    pub fn close_terminal_tab(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(idx) = self.terminal.tabs.iter().position(|t| t.id == id) else {
            return;
        };
        self.terminal.tabs.remove(idx);
        if self.terminal.active_tab.as_deref() == Some(id) {
            self.terminal.pending_confirm = None;
            let next_idx = idx.min(self.terminal.tabs.len().saturating_sub(1));
            self.terminal.active_tab = self.terminal.tabs.get(next_idx).map(|t| t.id.clone());
        }
        cx.notify();
    }

    pub fn terminal_send_interrupt(&mut self, cx: &mut Context<Self>) {
        self.terminal.pending_confirm = None;
        if let Some(tab) = self.terminal.active_mut()
            && let Some(pty) = tab.backend.as_pty()
        {
            let _ = pty.send_interrupt();
            tab.snapshot = tab.backend.snapshot();
        }
        cx.notify();
    }

    pub fn terminal_clear(&mut self, cx: &mut Context<Self>) {
        self.terminal.selection = None;
        if let Some(tab) = self.terminal.active_mut() {
            match &tab.backend {
                crate::terminal_view::TerminalBackend::Pty(pty) => {
                    let _ = pty.clear_screen();
                }
                crate::terminal_view::TerminalBackend::Log(log) => {
                    log.clear();
                }
            }
            tab.snapshot = tab.backend.snapshot();
        }
        cx.notify();
    }

    pub fn terminal_copy_visible(&mut self, cx: &mut Context<Self>) {
        let Some(tab) = self.terminal.active() else {
            return;
        };
        if let Some(sel) = self.terminal.selection.filter(|s| s.has_range()) {
            let selected = crate::terminal_view::extract_selected_text(&tab.snapshot, sel);
            if !selected.is_empty() {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(selected));
                self.push_toast(ToastKind::Info, "Copied terminal selection", None, cx);
                return;
            }
        }
        let text = tab.backend.plain_text();
        if text.is_empty() {
            self.push_toast(
                ToastKind::Info,
                "Nothing to copy — terminal viewport is empty",
                None,
                cx,
            );
            return;
        }
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        self.push_toast(ToastKind::Info, "Copied terminal output", None, cx);
    }

    pub fn terminal_paste_clipboard(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        if text.is_empty() {
            return;
        }
        if let Some(tab) = self.terminal.active_mut()
            && let Some(pty) = tab.backend.as_pty()
        {
            if text.contains('\n') || text.contains('\r') {
                let mut payload = Vec::with_capacity(text.len() + 12);
                payload.extend_from_slice(b"\x1b[200~");
                payload.extend_from_slice(text.as_bytes());
                payload.extend_from_slice(b"\x1b[201~");
                let _ = pty.write_bytes(&payload);
            } else {
                let _ = pty.write_bytes(text.as_bytes());
            }
            tab.snapshot = tab.backend.snapshot();
        }
        cx.notify();
    }

    /// Execute or paste the quick-command bar text into an interactive shell tab.
    pub fn terminal_run_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        execute: bool,
    ) {
        let Some(input) = self.terminal.cmd_input.clone() else {
            return;
        };
        let raw = input.read(cx).value().trim().to_string();
        if raw.is_empty() {
            return;
        }
        input.update(cx, |st, cx| st.set_value("", window, cx));

        let needs_shell = self
            .terminal
            .active()
            .is_none_or(|t| t.backend.is_log() || t.exited.is_some());
        if needs_shell {
            let (cwd, title) = self.pick_default_terminal_target();
            self.open_terminal_tab(&cwd, &title, Some(&raw), execute, true, Some(window), cx);
            return;
        }
        let (cwd, title) = self
            .terminal
            .active()
            .map(|t| (t.cwd.to_string(), t.title.to_string()))
            .unwrap_or_else(|| self.pick_default_terminal_target());
        self.send_terminal_command_if_any(&cwd, &title, Some(&raw), execute, cx);
        if let Some(tab) = self.terminal.active() {
            window.focus(&tab.focus, cx);
        }
        cx.notify();
    }

    /// Background check on launch (+ 12h periodic interval) for newer GitHub releases.
    pub fn updater_startup(&mut self, cx: &mut Context<Self>) {
        if !self.config.auto_check_updates {
            return;
        }
        cx.spawn(async move |this, cx| {
            crate::task::run(async {
                tokio::time::sleep(std::time::Duration::from_secs(4)).await;
            })
            .await;
            loop {
                let outcome = crate::task::run(async {
                    repoharbor_core::updater::check_latest_release(crate::identity::APP_VERSION)
                        .await
                })
                .await;
                let ok_still_alive = this.update(cx, |this, cx| {
                    if !this.config.auto_check_updates {
                        return;
                    }
                    match outcome {
                        Ok(repoharbor_core::updater::UpdateCheck::Available {
                            release, ..
                        }) => {
                            let tag = release.tag_name.clone();
                            this.update_state = AppUpdateState::Available(release);
                            this.upsert_toast(
                                "app-update",
                                ToastKind::Info,
                                "RepoHarbor update available",
                                Some(SharedString::from(format!(
                                    "{tag} is ready — open Settings → About to update"
                                ))),
                                cx,
                            );
                        }
                        Ok(repoharbor_core::updater::UpdateCheck::UpToDate {
                            latest_tag, ..
                        }) => {
                            this.update_state = AppUpdateState::UpToDate(latest_tag);
                        }
                        Err(_) => {
                            // Keep background checks silent when offline / rate-limited.
                        }
                    }
                    cx.notify();
                });
                if ok_still_alive.is_err() {
                    break;
                }
                crate::task::run(async {
                    tokio::time::sleep(std::time::Duration::from_secs(12 * 3600)).await;
                })
                .await;
            }
        })
        .detach();
    }

    /// Trigger an immediate update check from Settings → About.
    pub fn check_for_updates(&mut self, cx: &mut Context<Self>) {
        if matches!(
            self.update_state,
            AppUpdateState::Checking | AppUpdateState::Installing
        ) {
            return;
        }
        self.update_state = AppUpdateState::Checking;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let outcome = crate::task::run(async {
                repoharbor_core::updater::check_latest_release(crate::identity::APP_VERSION).await
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                match outcome {
                    Ok(repoharbor_core::updater::UpdateCheck::Available { release, .. }) => {
                        this.update_state = AppUpdateState::Available(release);
                    }
                    Ok(repoharbor_core::updater::UpdateCheck::UpToDate { latest_tag, .. }) => {
                        this.update_state = AppUpdateState::UpToDate(latest_tag);
                    }
                    Err(e) => {
                        this.update_state = AppUpdateState::Error(e);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Download, verify SHA-256, atomically replace binary, and automatically restart RepoHarbor.
    pub fn apply_app_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let release = match &self.update_state {
            AppUpdateState::Available(r) => r.clone(),
            _ => return,
        };
        if !self.install_kind.can_self_update_in_place() {
            let cmd = self.install_kind.external_update_command();
            let dir = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
            self.open_terminal_in(&dir, "Update", Some(&cmd), true, Some(window), cx);
            return;
        }

        let kind = self.install_kind.clone();
        self.update_state = AppUpdateState::Installing;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let res = crate::task::run(async move {
                repoharbor_core::updater::download_and_apply_release_update(&release, &kind).await
            })
            .await;
            let _ = this.update(cx, |this, cx| match res {
                Ok(new_exe) => {
                    match repoharbor_core::updater::spawn_detached_relaunch(&new_exe) {
                        Ok(()) => {
                            std::thread::spawn(|| {
                                std::thread::sleep(std::time::Duration::from_millis(200));
                                std::process::exit(0);
                            });
                            cx.quit();
                        }
                        Err(e) => {
                            this.update_state = AppUpdateState::Error(format!(
                                "Updated on disk, but relaunch failed: {e}"
                            ));
                            this.push_toast(
                                ToastKind::Success,
                                "Update installed",
                                Some("Relaunch RepoHarbor manually to load the new build.".into()),
                                cx,
                            );
                        }
                    }
                    cx.notify();
                }
                Err(e) => {
                    this.update_state = AppUpdateState::Error(e.clone());
                    this.push_toast(ToastKind::Error, "Update failed", Some(e.into()), cx);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Navigate straight to Settings → About for release notes & one-click update.
    pub fn open_about_updates(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.view = View::Settings;
        self.view_filter = Some("about".into());
        self.open_settings(window, cx);
        cx.notify();
    }

    /// Toggle automatic startup on login (`~/.config/autostart`).
    pub fn toggle_autostart(&mut self, cx: &mut Context<Self>) {
        let next = !self.autostart_enabled;
        let exec = crate::desktop::preferred_exec_path();
        match repoharbor_core::config::set_autostart_enabled(next, &exec) {
            Ok(()) => {
                self.autostart_enabled = next;
                self.push_toast(
                    ToastKind::Success,
                    if next {
                        "Autostart enabled"
                    } else {
                        "Autostart disabled"
                    },
                    None,
                    cx,
                );
            }
            Err(e) => {
                self.push_toast(ToastKind::Error, "Autostart failed", Some(e.into()), cx);
            }
        }
        cx.notify();
    }

    /// Toggle background periodic update checks cleanly.
    pub fn toggle_auto_check_updates(&mut self, cx: &mut Context<Self>) {
        self.config.auto_check_updates = !self.config.auto_check_updates;
        if let Some(s) = &mut self.settings {
            s.draft.auto_check_updates = self.config.auto_check_updates;
        }
        let _ = repoharbor_core::config::save(&self.config);
        cx.notify();
    }

    /// Install or repair user-local `.desktop` shortcut + hicolor icons.
    pub fn repair_desktop_shortcut(&mut self, cx: &mut Context<Self>) {
        match crate::desktop::install_desktop_entry(self.autostart_enabled) {
            Ok(path) => {
                self.push_toast(
                    ToastKind::Success,
                    "Desktop shortcut installed",
                    Some(SharedString::from(path.to_string_lossy().into_owned())),
                    cx,
                );
            }
            Err(e) => {
                self.push_toast(
                    ToastKind::Error,
                    "Desktop install failed",
                    Some(e.into()),
                    cx,
                );
            }
        }
        cx.notify();
    }

    /// Open native folder picker directly from Mission Control onboarding and append roots.
    pub fn pick_and_add_roots(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async {
                    repoharbor_platform::folder_picker::pick_folders("Add workspace roots", "Add")
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if let Ok(paths) = result
                    && !paths.is_empty()
                {
                    let mut added = 0usize;
                    for p in paths {
                        let s = p.to_string_lossy().into_owned();
                        if !this.config.roots.contains(&s) {
                            this.config.roots.push(s);
                            added += 1;
                        }
                    }
                    if added > 0 {
                        repoharbor_core::config::seed_odoo_groups_if_empty(&mut this.config);
                        repoharbor_core::config::seed_pull_only_if_empty(&mut this.config);
                        let _ = repoharbor_core::config::save(&this.config);
                        this.rescan_user(cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Discover common developer folders on `$HOME`, add missing ones, save & rescan.
    pub fn auto_detect_and_add_roots(&mut self, cx: &mut Context<Self>) {
        let candidates = repoharbor_core::config::detect_candidate_roots();
        let mut added = 0usize;
        for c in candidates {
            if !self.config.roots.contains(&c) {
                self.config.roots.push(c);
                added += 1;
            }
        }
        if added > 0 {
            repoharbor_core::config::seed_odoo_groups_if_empty(&mut self.config);
            repoharbor_core::config::seed_pull_only_if_empty(&mut self.config);
            let _ = repoharbor_core::config::save(&self.config);
            self.rescan_user(cx);
            self.push_toast(
                ToastKind::Success,
                "Workspace folders added",
                Some(SharedString::from(format!(
                    "Added {added} root(s) and scanning…"
                ))),
                cx,
            );
        } else {
            self.push_toast(
                ToastKind::Info,
                "No new workspace folders found",
                Some("Use Pick folders to choose custom project paths.".into()),
                cx,
            );
        }
        cx.notify();
    }

    /// Persist dismissal of the first-run onboarding setup checklist.
    pub fn dismiss_onboarding(&mut self, cx: &mut Context<Self>) {
        self.config.onboarding_dismissed = true;
        let _ = repoharbor_core::config::save(&self.config);
        cx.notify();
    }

    /// Compact flat onboarding strip shown on fresh installs or when 0 roots are configured.
    fn onboarding_banner(&self, t: &Theme, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if self.config.onboarding_dismissed && self.roots > 0 {
            return None;
        }
        let (hb, hf) = (t.border_strong, t.fg0);
        let folder_label = if self.roots > 0 {
            SharedString::from(format!("{} roots ({} repos)", self.roots, self.rows.len()))
        } else {
            SharedString::from("Pick workspace folders")
        };
        let gh_label = if self.services.github_authed {
            SharedString::from("GitHub connected")
        } else {
            SharedString::from("Connect GitHub")
        };
        Some(
            div()
                .id("first-run-onboarding")
                .flex()
                .flex_row()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap(px(10.))
                .px(px(16.))
                .py(px(9.))
                .bg(rgb(t.surface))
                .border_b_1()
                .border_color(rgb(t.border_accent))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .gap(px(8.))
                        .child(lucide("sparkles", 14., t.primary))
                        .child(
                            div()
                                .text_size(px(t.text_small))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(t.fg0))
                                .child("First-run setup"),
                        )
                        .child(onboarding_pill_btn(
                            "onb-pick-roots",
                            "folder-plus",
                            folder_label,
                            self.roots > 0,
                            t.primary,
                            t,
                            cx,
                            |this, _, _, cx| this.pick_and_add_roots(cx),
                        ))
                        .child(onboarding_pill_btn(
                            "onb-detect-roots",
                            "search",
                            "Auto-detect folders".into(),
                            false,
                            t.fg2,
                            t,
                            cx,
                            |this, _, _, cx| this.auto_detect_and_add_roots(cx),
                        ))
                        .child(onboarding_pill_btn(
                            "onb-github",
                            "git-branch",
                            gh_label,
                            self.services.github_authed,
                            if self.services.github_authed {
                                t.clean
                            } else {
                                t.fg2
                            },
                            t,
                            cx,
                            |this, _, win, cx| {
                                if !this.services.github_authed {
                                    this.view = View::Settings;
                                    this.view_filter = Some("account".into());
                                    self::RepoHarborApp::open_settings(this, win, cx);
                                }
                            },
                        ))
                        .child(onboarding_pill_btn(
                            "onb-desktop",
                            "harbor",
                            "Pin desktop shortcut".into(),
                            false,
                            t.fg2,
                            t,
                            cx,
                            |this, _, _, cx| this.repair_desktop_shortcut(cx),
                        ))
                        .child(onboarding_pill_btn(
                            "onb-autostart",
                            "bell",
                            SharedString::from(if self.autostart_enabled {
                                "Autostart: On"
                            } else {
                                "Autostart: Off"
                            }),
                            self.autostart_enabled,
                            if self.autostart_enabled {
                                t.clean
                            } else {
                                t.fg2
                            },
                            t,
                            cx,
                            |this, _, _, cx| this.toggle_autostart(cx),
                        )),
                )
                .child(
                    div()
                        .id("onb-dismiss")
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(4.))
                        .px(px(8.))
                        .py(px(3.))
                        .rounded(px(t.r_xs))
                        .bg(rgb(t.button_bg))
                        .border_1()
                        .border_color(rgb(t.border))
                        .cursor_pointer()
                        .hover(move |s| s.border_color(rgb(hb)).text_color(rgb(hf)))
                        .text_size(px(t.text_data_sm))
                        .text_color(rgb(t.fg2))
                        .child(lucide("x", 12., t.fg3))
                        .child("Hide")
                        .on_click(cx.listener(|this, _, _, cx| this.dismiss_onboarding(cx))),
                )
                .into_any_element(),
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn onboarding_pill_btn(
    id: &'static str,
    icon: &'static str,
    label: SharedString,
    active: bool,
    color: u32,
    t: &Theme,
    cx: &mut Context<RepoHarborApp>,
    handler: impl Fn(&mut RepoHarborApp, &gpui::ClickEvent, &mut Window, &mut Context<RepoHarborApp>)
    + 'static,
) -> impl IntoElement {
    let (hb, hf) = (t.border_strong, t.fg0);
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .gap(px(5.))
        .px(px(9.))
        .py(px(4.))
        .rounded(px(t.r_xs))
        .bg(rgb(if active { t.accent_wash } else { t.button_bg }))
        .border_1()
        .border_color(rgb(if active { t.border_accent } else { t.border }))
        .cursor_pointer()
        .hover(move |s| s.border_color(rgb(hb)).text_color(rgb(hf)))
        .text_size(px(t.text_data_sm))
        .text_color(rgb(if active { t.accent_bright } else { t.fg1 }))
        .child(lucide(icon, 12., color))
        .child(label)
        .on_click(cx.listener(move |this, ev, win, cx| {
            handler(this, ev, win, cx);
        }))
}

/// Short display title for a repository or worktree tab.
pub(crate) fn repo_tab_title(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("Shell")
        .to_string()
}

/// POSIX shell single-quote escape for embedded terminal command construction.
fn posix_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

/// Unwrap external terminal emulator wrappers (`ptyxis`, `kitty`, etc.) to extract
/// an interactive agent CLI invocation suitable for running directly in our PTY.
/// Returns `None` when `agent_cmd` only names a terminal emulator with no CLI agent.
pub(crate) fn build_embedded_agent_command(
    agent_cmd: &str,
    dispatch_args: &str,
    prompt: &str,
) -> Option<String> {
    const WRAPPERS: &[&str] = &[
        "kitty",
        "wezterm",
        "alacritty",
        "gnome-terminal",
        "konsole",
        "xterm",
        "foot",
        "st",
        "terminator",
        "tilix",
        "xfce4-terminal",
        "urxvt",
        "ghostty",
        "ptyxis",
        "xdg-terminal-exec",
        "start",
    ];
    let tokens: Vec<&str> = agent_cmd.split_whitespace().collect();
    if tokens.is_empty() {
        return None;
    }

    let first_bin = tokens[0].rsplit('/').next().unwrap_or(tokens[0]);
    let cli_tokens: &[&str] = if WRAPPERS.contains(&first_bin) {
        if let Some(pos) = tokens.iter().position(|t| *t == "--" || *t == "-e") {
            tokens.get(pos + 1..).unwrap_or(&[])
        } else {
            // Look for first non-flag token after wrapper + its arguments (ignoring `{path}`).
            let mut idx = 1;
            let mut found = None;
            while idx < tokens.len() {
                let tok = tokens[idx];
                if tok.starts_with('-') {
                    if let Some(next) = tokens.get(idx + 1)
                        && !next.starts_with('-')
                    {
                        let next_bin = next.rsplit('/').next().unwrap_or(next);
                        if matches!(
                            tok,
                            "-d" | "--directory"
                                | "--working-directory"
                                | "--cwd"
                                | "-w"
                                | "--title"
                        ) || next.contains('{')
                            || WRAPPERS.contains(&next_bin)
                        {
                            idx += 2;
                            continue;
                        }
                    }
                    idx += 1;
                    continue;
                }
                if !tok.contains('{') {
                    found = Some(idx);
                    break;
                }
                idx += 1;
            }
            found.map(|i| &tokens[i..]).unwrap_or(&[])
        }
    } else {
        &tokens
    };

    let filtered: Vec<&str> = cli_tokens
        .iter()
        .copied()
        .filter(|t| !t.contains("{path}"))
        .collect();
    if filtered.is_empty() {
        return None;
    }

    let mut out = filtered.join(" ");
    let trimmed_prompt = prompt.trim();
    if !trimmed_prompt.is_empty() {
        let quoted_prompt = posix_quote(trimmed_prompt);
        let template = dispatch_args.trim();
        if template.is_empty() {
            out.push(' ');
            out.push_str(&quoted_prompt);
        } else if template.contains("{prompt}") {
            out.push(' ');
            out.push_str(&template.replace("{prompt}", &quoted_prompt));
        } else {
            out.push(' ');
            out.push_str(template);
            out.push(' ');
            out.push_str(&quoted_prompt);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use repoharbor_core::attention::RepoRef;

    fn item(kind: AttentionKind, name: &str, id: Option<&str>) -> AttentionItem {
        AttentionItem {
            repo: RepoRef {
                id: id.map(str::to_string),
                remote_host: Some("github.com".into()),
                slug: Some(format!("o/{name}")),
                name: name.into(),
            },
            kind,
            severity: kind.severity(),
            summary: format!("{kind:?} in {name}"),
            detail: None,
        }
    }

    #[test]
    fn tree_hides_submodules_unless_needs_me_reveals() {
        let parent = SharedString::from("/p");
        let child = SharedString::from("/p/child");
        assert!(
            !tree_allows_row(None, Some(&parent), &child, false),
            "flat grid hides submodule children by default"
        );
        assert!(
            tree_allows_row(None, Some(&parent), &child, true),
            "Needs me reveals attention-index children"
        );
        assert!(tree_allows_row(None, None, &parent, false));
        assert!(tree_allows_row(Some(&parent), Some(&parent), &child, false));
        assert!(!tree_allows_row(
            Some(&SharedString::from("/other")),
            Some(&parent),
            &child,
            true
        ));
    }

    #[test]
    fn host_only_needs_me_skips_linked_and_info() {
        let items = vec![
            item(AttentionKind::ReviewRequested, "orphan", None),
            item(AttentionKind::CiFailing, "linked", Some("/a")),
            item(AttentionKind::PrAssigned, "open-pr", None), // Info
            item(AttentionKind::AgentRunning, "run", None),   // not needs_me
        ];
        let host = host_only_needs_me(&items);
        assert_eq!(host.len(), 1);
        assert_eq!(host[0].repo.name, "orphan");
    }

    #[test]
    fn needs_me_count_matches_listable_repos_not_hidden_children() {
        // Two Ahead items on submodule children — without reveal the flat grid
        // hides them (the reported bug); with Needs me reveal, listable == index.
        let child_a = SharedString::from("/p/a");
        let child_b = SharedString::from("/q/b");
        let parent_a = SharedString::from("/p");
        let parent_b = SharedString::from("/q");
        let items = vec![
            item(AttentionKind::Ahead, "a", Some(child_a.as_ref())),
            item(AttentionKind::Ahead, "b", Some(child_b.as_ref())),
        ];
        let map = index_needs_me(&items);
        assert_eq!(map.len(), 2);

        let pairs = [(&child_a, &parent_a), (&child_b, &parent_b)];
        let hidden = pairs
            .iter()
            .filter(|(id, parent)| {
                tree_allows_row(None, Some(*parent), id, false) && map.contains_key(*id)
            })
            .count();
        assert_eq!(
            hidden, 0,
            "submodule children stay off the flat grid by default"
        );

        let listed = pairs
            .iter()
            .filter(|(id, parent)| {
                let reveal = map.contains_key(*id);
                tree_allows_row(None, Some(*parent), id, reveal) && map.contains_key(*id)
            })
            .count();
        assert_eq!(listed, map.len(), "Needs me count must equal listable rows");

        // Host-only facts add to the surface without a local row.
        let with_host = vec![
            item(AttentionKind::Ahead, "a", Some(child_a.as_ref())),
            item(AttentionKind::ReviewRequested, "ghost", None),
        ];
        assert_eq!(
            index_needs_me(&with_host).len() + host_only_needs_me(&with_host).len(),
            2
        );
    }

    #[test]
    fn attention_reason_chips_dedupes_and_caps() {
        let items = vec![
            item(AttentionKind::CiFailing, "a", Some("/a")),
            item(AttentionKind::ReviewRequested, "a", Some("/a")),
            item(AttentionKind::DirtyWorktree, "a", Some("/a")),
            item(AttentionKind::Ahead, "a", Some("/a")),
            item(AttentionKind::CiFailing, "b", Some("/b")),
        ];
        let id = SharedString::from("/a");
        let (chips, more, subtitle) = attention_reason_chips(&items, &id);
        assert_eq!(
            chips.iter().map(|(l, _)| l.as_ref()).collect::<Vec<_>>(),
            vec!["CI failing", "Review requested"]
        );
        assert_eq!(more, 2, "DirtyWorktree + Ahead overflow");
        assert!(
            subtitle
                .as_ref()
                .is_some_and(|s| s.as_ref().contains("CiFailing") && s.as_ref().contains("Open CI")),
            "subtitle uses top item summary + action hint"
        );

        let quiet = SharedString::from("/missing");
        let (chips, more, subtitle) = attention_reason_chips(&items, &quiet);
        assert!(chips.is_empty() && more == 0 && subtitle.is_none());
    }

    #[test]
    fn live_agent_readout_skips_needs_me_index_and_subtitle() {
        let running = item(AttentionKind::AgentRunning, "a", Some("/a"));
        let dirty = item(AttentionKind::DirtyWorktree, "a", Some("/a"));
        let only_running = vec![running.clone()];
        let mixed = vec![running, dirty];

        let map = index_needs_me(&only_running);
        assert!(
            map.is_empty(),
            "AgentRunning alone must not land a repo on Needs me"
        );
        let map = index_needs_me(&mixed);
        assert_eq!(
            map.get(&SharedString::from("/a")).copied(),
            Some(Severity::Attention)
        );

        let id = SharedString::from("/a");
        let (chips, more, subtitle) = attention_reason_chips(&only_running, &id);
        assert!(
            chips.is_empty() && more == 0 && subtitle.is_none(),
            "Agent running must not replace the git/commit line"
        );
        let (chips, more, subtitle) = attention_reason_chips(&mixed, &id);
        assert_eq!(
            chips.iter().map(|(l, _)| l.as_ref()).collect::<Vec<_>>(),
            vec!["Uncommitted changes"]
        );
        assert_eq!(more, 0);
        assert!(
            subtitle
                .as_ref()
                .is_some_and(|s| s.as_ref().contains("DirtyWorktree")),
            "next actionable kind becomes the subtitle"
        );
    }

    #[test]
    fn tray_summary_counts_actionable_and_caps_top_lines() {
        // 2 urgent + 2 attention + 1 info → total 4, urgent 2, top capped at 3
        // (severity-ordered, like the model's output).
        let items = vec![
            item(AttentionKind::CiFailing, "a", Some("/a")),
            item(AttentionKind::ReviewRequested, "b", None),
            item(AttentionKind::DirtyWorktree, "c", Some("/c")),
            item(AttentionKind::Ahead, "d", Some("/d")),
            item(AttentionKind::PrunableBranches, "e", Some("/e")),
        ];
        let s = tray_summary(&items);
        assert_eq!((s.total, s.urgent), (4, 2));
        assert_eq!(
            s.top,
            vec![
                "a · CiFailing in a",
                "b · ReviewRequested in b",
                "c · DirtyWorktree in c",
            ]
        );

        let quiet = tray_summary(&[item(AttentionKind::AgentRunning, "x", Some("/x"))]);
        assert_eq!(quiet, repoharbor_platform::tray::TrayAttention::default());
    }

    #[test]
    fn attention_key_is_stable_and_repo_specific() {
        let a = item(AttentionKind::ReviewRequested, "a", Some("/a"));
        assert_eq!(attention_key(&a), attention_key(&a.clone()));
        // Same fact, different repo → different key. Local id wins; host+slug
        // is the fallback so unlinked host facts stay distinct across hosts.
        assert_ne!(
            attention_key(&a),
            attention_key(&item(AttentionKind::ReviewRequested, "a", Some("/b")))
        );
        let unlinked = item(AttentionKind::ReviewRequested, "a", None);
        assert!(attention_key(&unlinked).contains("github.com/o/a"));
    }

    #[test]
    fn visibility_filters_are_orthogonal_helpers() {
        assert!(is_visibility_filter(RepoFilter::Public));
        assert!(is_visibility_filter(RepoFilter::Private));
        assert!(is_visibility_filter(RepoFilter::Starred));
        assert!(is_visibility_filter(RepoFilter::Stale));
        assert!(!is_visibility_filter(RepoFilter::Dirty));
        assert!(!is_visibility_filter(RepoFilter::Attention));
        assert!(!is_visibility_filter(RepoFilter::All));
        assert!(matches!(
            WorkMode::from_filter(RepoFilter::Public),
            WorkMode::All
        ));
        assert!(matches!(
            WorkMode::from_filter(RepoFilter::Dirty),
            WorkMode::Working
        ));
    }

    #[test]
    fn urgent_kind_toggles_keep_their_meaning() {
        let mut cfg = AppConfig::default();
        assert!(urgent_kind_enabled(&cfg, AttentionKind::ReviewRequested));
        cfg.notify_review_requested = false;
        assert!(!urgent_kind_enabled(&cfg, AttentionKind::ReviewRequested));
        cfg.notify_ci_failure = false;
        assert!(!urgent_kind_enabled(&cfg, AttentionKind::CiFailing));
    }

    #[test]
    fn mission_control_layout_defaults_to_list() {
        assert_eq!(Layout::default(), Layout::List);
        assert_eq!(AppConfig::default().layout, Layout::List);
    }

    #[test]
    fn summarize_finish_toast_copy() {
        let (kind, title, detail) = summarize_finish_toast(1);
        assert_eq!(kind, ToastKind::Success);
        assert_eq!(title.as_ref(), "Summaries ready");
        assert_eq!(detail.as_deref(), Some("1 new summary"));

        let (kind, title, detail) = summarize_finish_toast(3);
        assert_eq!(kind, ToastKind::Success);
        assert_eq!(title.as_ref(), "Summaries ready");
        assert_eq!(detail.as_deref(), Some("3 new summaries"));
    }

    #[test]
    fn search_query_filter_parsing() {
        let q = SearchQueryFilter::parse("is:dirty is:behind lang:rust host:github my-repo");
        assert_eq!(q.terms, vec!["my-repo"]);
        assert_eq!(q.is_filters, vec!["dirty", "behind"]);
        assert_eq!(q.languages, vec!["rust"]);
        assert_eq!(q.hosts, vec!["github"]);
        assert!(!q.is_empty());
    }

    #[test]
    fn search_query_filter_matching() {
        let q = SearchQueryFilter::parse("is:dirty lang:rust");
        let attention = std::collections::HashMap::new();

        let mut row = crate::data::Row {
            id: "/path/to/my-repo".into(),
            url: "".into(),
            name: "my-repo".into(),
            slug: "user/my-repo".into(),
            root: "/path".into(),
            path: "/path/to/my-repo".into(),
            description: "".into(),
            language: "Rust".into(),
            branch: "main".into(),
            age: "1d ago".into(),
            release: "".into(),
            ai_summary: "".into(),
            ahead: 0,
            behind: 0,
            dirty: 2,
            staged: 0,
            unstaged: 2,
            stars: "".into(),
            host: "github".into(),
            private: false,
            favorite: false,
            activity: repoharbor_core::model::Activity::Active,
            last_commit_unix: 1000,
            parent_id: None,
            submodule_path: None,
            child_count: 0,
        };

        assert!(q.matches_row(&row, &attention));

        // When clean, it should fail is:dirty
        row.dirty = 0;
        assert!(!q.matches_row(&row, &attention));

        // When not rust, it should fail
        row.dirty = 1;
        row.language = "Python".into();
        assert!(!q.matches_row(&row, &attention));
    }

    #[test]
    fn repo_tab_title_uses_directory_basename() {
        assert_eq!(
            repo_tab_title("/home/user/projects/RepoHarbor"),
            "RepoHarbor"
        );
        assert_eq!(repo_tab_title("/"), "Shell");
        assert_eq!(repo_tab_title(""), "Shell");
    }

    #[test]
    fn build_embedded_agent_command_unwraps_wrappers_and_escapes_prompt() {
        assert_eq!(
            build_embedded_agent_command("ptyxis --new-window -d {path}", "{prompt}", "fix tests"),
            None
        );
        assert_eq!(
            build_embedded_agent_command(
                "ptyxis --new-window -d {path} -- claude",
                "-p {prompt}",
                "fix 'quoted' bug"
            ),
            Some("claude -p 'fix '\\''quoted'\\'' bug'".into())
        );
        assert_eq!(
            build_embedded_agent_command("kitty --directory {path} aider --yes", "{prompt}", "hi"),
            Some("aider --yes 'hi'".into())
        );
        assert_eq!(
            build_embedded_agent_command("claude", "{prompt}", "refactor terminal"),
            Some("claude 'refactor terminal'".into())
        );
    }
}
