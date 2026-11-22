//! Best-effort filesystem live-watch. Calls `on_change` (debounced) whenever
//! watched repos change on disk, so the UI can rescan.
//!
//! Rather than watching every repo *fully* recursively (hundreds of thousands
//! of inotify watches across `node_modules` / Odoo trees), we watch a bounded,
//! targeted set:
//!
//! - each configured **root**, non-recursively → new top-level repos;
//! - each discovered **repo root**, non-recursively → top-level file changes;
//! - a **shallow BFS** of directories under each repo (capped, ignore-filtered)
//!   → nested edits/deletes (the common case for module trees) still notify;
//! - each repo's **`.git`** dir, non-recursively → `index`/`HEAD` cover the
//!   high-value signals (staging, commits, branch switches, ahead/behind).
//!
//! Degrades silently if watches can't be established.
//!
//! The watch set is computed from the config at spawn and again on every
//! [`WatcherHandle::rearm`], so repos/roots added while the app runs (Settings
//! save, New Project, Explore clone) get live events without a restart.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::Duration;

use notify_debouncer_mini::new_debouncer;
use notify_debouncer_mini::notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_mini::{DebounceEventResult, DebouncedEvent, Debouncer};
use repoharbor_core::config;
use repoharbor_core::model::AppConfig;
use repoharbor_core::scan::{self, expand};

/// How deep under each repo root to register non-recursive directory watches.
/// Depth 0 is the repo root itself (already watched); 1 = immediate children,
/// 2 = one nested level (`src/nested`, `models/`, `views/`). Keeps inotify
/// usage lean on large multi-hundred-repo fleets.
const NESTED_WATCH_DEPTH: usize = 2;

/// Hard cap on nested directory watches *per repo* (in addition to the repo
/// root + `.git`). Keeps inotify usage bounded on giant checkouts (Odoo core).
const NESTED_WATCH_CAP: usize = 32;

/// Debounce window for coalescing bursts of filesystem writes before signalling.
const WATCH_DEBOUNCE: Duration = Duration::from_millis(1_500);

/// Built-in noisy directory names that are always excluded from nested watches
/// and event triggers, even when not listed in the user's `config.toml` ignore list.
const BUILTIN_WATCH_IGNORE: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".tox",
    ".nox",
    "venv",
    ".venv",
    ".idea",
    ".vscode",
    ".cursor",
    ".windsurf",
    ".theia",
    ".next",
    ".nuxt",
    ".svelte-kit",
    ".turbo",
    ".parcel-cache",
    "coverage",
    "htmlcov",
    "build",
    "out",
    "logs",
    "tmp",
    ".gradle",
    ".dart_tool",
];

/// What the watcher thread reacts to: a debounced fs change batch from the
/// debouncer, or a re-arm request from the app.
enum Msg {
    Fs(DebounceEventResult),
    Rearm,
}

/// Handle to the running watcher thread. Cheap to clone.
#[derive(Clone)]
pub struct WatcherHandle {
    tx: std::sync::mpsc::Sender<Msg>,
}

impl WatcherHandle {
    /// Ask the watcher thread to reload the config, recompute its targets and
    /// re-establish its watches. Call after anything that adds repos or roots
    /// at runtime. No-op if the thread is gone.
    pub fn rearm(&self) {
        let _ = self.tx.send(Msg::Rearm);
    }
}

fn watch_one(debouncer: &mut Debouncer<RecommendedWatcher>, path: &std::path::Path) -> bool {
    debouncer
        .watcher()
        .watch(path, RecursiveMode::NonRecursive)
        .is_ok()
}

fn ignored_dir(name: &str, ignore: &[String]) -> bool {
    // VCS metadata, IDE/cache/runtime output dirs, plus configured scan ignores.
    BUILTIN_WATCH_IGNORE.contains(&name) || ignore.iter().any(|p| p == name)
}

/// High-signal files inside `.git/` that represent real repo state transitions
/// (commits, staging, branch switches, merges, rebases, ref updates). Transient
/// files like `FETCH_HEAD`, `ORIG_HEAD`, `COMMIT_EDITMSG`, `*.lock`, and `logs/`
/// are ignored so no-op fetches and external git housekeeping don't storm rescans.
fn is_meaningful_dotgit_event(after_dotgit: &[&str]) -> bool {
    match after_dotgit {
        [] => false,
        [first, ..] => {
            if first.ends_with(".lock") {
                return false;
            }
            matches!(
                *first,
                "HEAD"
                    | "index"
                    | "MERGE_HEAD"
                    | "REBASE_HEAD"
                    | "CHERRY_PICK_HEAD"
                    | "packed-refs"
                    | "config"
                    | "refs"
            )
        }
    }
}

/// True when a filename outside `.git/` is a noisy transient/log/IDE/cache file
/// that should never trigger a full-workspace git rescan on its own.
fn is_noisy_filename(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".log")
        || lower.ends_with(".tmp")
        || lower.ends_with(".swp")
        || lower.ends_with(".swo")
        || lower.ends_with('~')
        || lower.ends_with(".pyc")
        || lower.ends_with(".pyo")
        || lower.ends_with(".sqlite-wal")
        || lower.ends_with(".sqlite-shm")
        || lower.ends_with(".sqlite-journal")
        || lower.ends_with(".stats.json")
        || matches!(lower.as_str(), "workspace.xml" | "tasks.xml")
}

/// Decide whether a debounced event batch contains at least one meaningful
/// repository change worth rescanning. Pure helper — unit-tested below.
fn should_trigger_rescan(events: &[DebouncedEvent], ignore: &[String]) -> bool {
    events.iter().any(|ev| {
        let components: Vec<&str> = ev
            .path
            .components()
            .filter_map(|c| c.as_os_str().to_str())
            .collect();
        if let Some(git_pos) = components.iter().position(|&c| c == ".git") {
            return is_meaningful_dotgit_event(&components[git_pos + 1..]);
        }
        if let Some((file_name, parent_dirs)) = components.split_last() {
            if parent_dirs.iter().any(|dir| ignored_dir(dir, ignore)) {
                return false;
            }
            if is_noisy_filename(file_name) || ignored_dir(file_name, ignore) {
                return false;
            }
        }
        true
    })
}

/// Shallow directory list under `repo` for nested watches. Skips configured
/// ignore names and caps the count so large trees stay cheap.
fn nested_watch_dirs(repo: &Path, ignore: &[String]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut q: VecDeque<(PathBuf, usize)> = VecDeque::new();
    q.push_back((repo.to_path_buf(), 0));
    while let Some((dir, depth)) = q.pop_front() {
        if depth >= NESTED_WATCH_DEPTH || out.len() >= NESTED_WATCH_CAP {
            break;
        }
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            if out.len() >= NESTED_WATCH_CAP {
                break;
            }
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if !ft.is_dir() {
                continue;
            }
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if ignored_dir(name, ignore) {
                continue;
            }
            let child = entry.path();
            out.push(child.clone());
            if depth + 1 < NESTED_WATCH_DEPTH {
                q.push_back((child, depth + 1));
            }
        }
    }
    out
}

/// The paths the watcher registers for a config: each configured root, each
/// discovered repo root + shallow nested dirs, and each repo's `.git` dir
/// (all non-recursive).
fn target_paths(cfg: &AppConfig) -> Vec<PathBuf> {
    // Configured roots → detect new top-level repos.
    let mut paths: Vec<PathBuf> = cfg.roots.iter().map(|root| expand(root)).collect();
    // Each repo's working root + shallow tree + .git → file changes and git ops.
    for repo in scan::repo_paths(&cfg.roots, cfg.scan_depth, &cfg.ignore) {
        let dotgit = repo.join(".git");
        paths.push(repo.clone());
        paths.extend(nested_watch_dirs(&repo, &cfg.ignore));
        if dotgit.is_dir() {
            paths.push(dotgit);
        }
    }
    paths
}

fn watch_targets(debouncer: &mut Debouncer<RecommendedWatcher>) -> Vec<String> {
    let cfg = config::load();
    for path in target_paths(&cfg) {
        let _ = watch_one(debouncer, &path);
    }
    cfg.ignore
}

/// Spawn the watcher thread. `on_change` fires on each debounced change batch
/// that passes path filtering (the app rescans). The returned handle re-arms
/// the watch set on demand; the thread stays alive even with zero established
/// watches so a later re-arm (e.g. the first root added in Settings) can bring
/// the watch up.
pub fn spawn(on_change: impl Fn() + Send + 'static) -> WatcherHandle {
    let (tx, rx) = std::sync::mpsc::channel::<Msg>();
    let handle = WatcherHandle { tx: tx.clone() };
    std::thread::spawn(move || {
        // The debouncer forwards fs batches onto the same channel re-arm
        // requests arrive on, so one recv loop serves both.
        let make_debouncer = || {
            let tx = tx.clone();
            new_debouncer(WATCH_DEBOUNCE, move |res: DebounceEventResult| {
                let _ = tx.send(Msg::Fs(res));
            })
        };
        let Ok(mut debouncer) = make_debouncer() else {
            return;
        };
        let mut ignore = watch_targets(&mut debouncer);

        // Keep `debouncer` alive for the life of the thread; fire only on real,
        // non-noisy change batches (`Msg::Fs` also carries notify errors, which
        // we ignore so a degraded watch can't spam rescans). On re-arm, drop
        // the debouncer — and with it every existing watch — and register a
        // fresh set from the current config. Exits on disconnect.
        while let Ok(msg) = rx.recv() {
            match msg {
                Msg::Fs(Ok(events)) => {
                    if should_trigger_rescan(&events, &ignore) {
                        on_change();
                    }
                }
                Msg::Fs(Err(_)) => {}
                Msg::Rearm => {
                    if let Ok(fresh) = make_debouncer() {
                        debouncer = fresh;
                        ignore = watch_targets(&mut debouncer);
                    }
                }
            }
        }
    });
    handle
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_paths_cover_roots_repos_and_dotgit() {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("proj");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        // A plain directory (no .git) is not a repo, so it gets no watch.
        std::fs::create_dir_all(root.path().join("notes")).unwrap();

        let cfg = AppConfig {
            roots: vec![root.path().to_string_lossy().into_owned()],
            ..AppConfig::default()
        };
        let paths = target_paths(&cfg);
        assert!(paths.contains(&root.path().to_path_buf()));
        assert!(paths.contains(&repo));
        assert!(paths.contains(&repo.join(".git")));
        assert!(!paths.contains(&root.path().join("notes")));
    }

    #[test]
    fn target_paths_include_shallow_nested_dirs() {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("proj");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(repo.join("src").join("nested")).unwrap();
        std::fs::create_dir_all(repo.join("node_modules").join("pkg")).unwrap();

        let cfg = AppConfig {
            roots: vec![root.path().to_string_lossy().into_owned()],
            ignore: vec!["node_modules".into()],
            ..AppConfig::default()
        };
        let paths = target_paths(&cfg);
        assert!(paths.contains(&repo.join("src")));
        assert!(paths.contains(&repo.join("src").join("nested")));
        assert!(
            !paths
                .iter()
                .any(|p| p.ends_with("node_modules") || p.ends_with("pkg")),
            "ignored dirs must not be watched"
        );
    }

    #[test]
    fn target_paths_empty_config_yields_no_targets() {
        let cfg = AppConfig {
            roots: Vec::new(),
            ..AppConfig::default()
        };
        assert!(target_paths(&cfg).is_empty());
    }

    #[test]
    fn nested_watch_dirs_respects_cap() {
        let root = tempfile::tempdir().unwrap();
        // More children than the cap — BFS should stop at the limit.
        for i in 0..(NESTED_WATCH_CAP + 20) {
            std::fs::create_dir_all(root.path().join(format!("d{i}"))).unwrap();
        }
        let dirs = nested_watch_dirs(root.path(), &[]);
        assert!(dirs.len() <= NESTED_WATCH_CAP);
    }

    #[test]
    fn ignored_dir_skips_builtin_noisy_dirs() {
        for noisy in [
            "__pycache__",
            ".idea",
            ".cursor",
            ".ruff_cache",
            ".venv",
            "out",
            "logs",
        ] {
            assert!(ignored_dir(noisy, &[]), "{noisy} must be ignored");
        }
        assert!(!ignored_dir("src", &[]));
        assert!(!ignored_dir(".github", &[]));
    }

    #[test]
    fn should_trigger_rescan_filters_noisy_paths() {
        use notify_debouncer_mini::DebouncedEventKind;
        let ev = |p: &str| DebouncedEvent {
            path: PathBuf::from(p),
            kind: DebouncedEventKind::Any,
        };

        // Transient git internals & runtime logs/IDE files must not trigger.
        assert!(!should_trigger_rescan(&[ev("/repo/.git/FETCH_HEAD")], &[]));
        assert!(!should_trigger_rescan(&[ev("/repo/.git/index.lock")], &[]));
        assert!(!should_trigger_rescan(
            &[ev("/repo/tools/out/ns_2024_ship.log")],
            &[]
        ));
        assert!(!should_trigger_rescan(
            &[ev("/repo/tools/out/ns_archive.sqlite.stats.json")],
            &[]
        ));
        assert!(!should_trigger_rescan(
            &[ev("/repo/.idea/workspace.xml")],
            &[]
        ));
        assert!(!should_trigger_rescan(
            &[ev("/repo/pkg/__pycache__/mod.cpython-312.pyc")],
            &[]
        ));

        // High-signal git files and real source edits must trigger.
        assert!(should_trigger_rescan(&[ev("/repo/.git/index")], &[]));
        assert!(should_trigger_rescan(&[ev("/repo/.git/HEAD")], &[]));
        assert!(should_trigger_rescan(
            &[ev("/repo/.git/refs/remotes/origin/main")],
            &[]
        ));
        assert!(should_trigger_rescan(&[ev("/repo/src/main.rs")], &[]));
    }
}
