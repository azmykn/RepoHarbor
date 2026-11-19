//! In-memory activity log — every toast, desktop notification, scan, and fleet
//! outcome for the Log view. Ring buffer; not persisted.

use std::collections::VecDeque;

use gpui::SharedString;

use crate::toast::ToastKind;

/// How many entries the ring keeps (newest at the front for the Log view).
pub const CAPACITY: usize = 400;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    /// Soft problems that aren't hard failures (e.g. skipped ops).
    Warn,
    Error,
}

impl LogLevel {
    pub fn label(self) -> &'static str {
        match self {
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        }
    }
}

/// Structured facts attached to a log line / toast so a click can open a
/// detail panel (repo, branch, commit, per-repo fleet outcomes, …).
#[derive(Clone, Default)]
pub struct LogContext {
    pub repo_id: Option<SharedString>,
    pub repo_name: Option<SharedString>,
    pub slug: Option<SharedString>,
    pub path: Option<SharedString>,
    pub branch: Option<SharedString>,
    pub commit: Option<SharedString>,
    pub commit_subject: Option<SharedString>,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
    pub dirty: Option<u32>,
    pub host: Option<SharedString>,
    pub url: Option<SharedString>,
    /// Extra labeled rows (fleet per-repo outcomes, attention kind, …).
    pub facts: Vec<(SharedString, SharedString)>,
}

impl LogContext {
    pub fn with_fact(
        mut self,
        key: impl Into<SharedString>,
        value: impl Into<SharedString>,
    ) -> Self {
        self.facts.push((key.into(), value.into()));
        self
    }

    /// Fill empty repo fields from `other`; append its facts.
    pub fn merge_repo_from(&mut self, other: LogContext) {
        if self.repo_id.is_none() {
            self.repo_id = other.repo_id;
            if self.repo_name.is_none() {
                self.repo_name = other.repo_name;
            }
            if self.slug.is_none() {
                self.slug = other.slug;
            }
            if self.path.is_none() {
                self.path = other.path;
            }
            if self.branch.is_none() {
                self.branch = other.branch;
            }
            if self.commit.is_none() {
                self.commit = other.commit;
                self.commit_subject = other.commit_subject;
            }
            if self.ahead.is_none() {
                self.ahead = other.ahead;
            }
            if self.behind.is_none() {
                self.behind = other.behind;
            }
            if self.dirty.is_none() {
                self.dirty = other.dirty;
            }
            if self.host.is_none() {
                self.host = other.host;
            }
            if self.url.is_none() {
                self.url = other.url;
            }
        }
        self.facts.extend(other.facts);
    }
}

#[derive(Clone)]
pub struct LogEntry {
    pub id: u64,
    /// Unix seconds when the event was recorded.
    pub at: i64,
    pub level: LogLevel,
    pub message: SharedString,
    pub context: LogContext,
}

#[derive(Default)]
pub struct ActivityLog {
    entries: VecDeque<LogEntry>,
    seq: u64,
}

impl ActivityLog {
    /// Newest-first snapshot for the Log view.
    pub fn entries(&self) -> impl Iterator<Item = &LogEntry> {
        self.entries.iter()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Look up a line by id (newest-first ring; `None` if it has rotated out).
    pub fn get(&self, id: u64) -> Option<&LogEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// Append one line. `at` is unix seconds (caller supplies so tests stay
    /// deterministic and toast paths can share a clock).
    pub fn push(&mut self, at: i64, level: LogLevel, message: impl Into<SharedString>) -> u64 {
        self.push_ctx(at, level, message, LogContext::default())
    }

    /// Append one line with structured click-through facts.
    pub fn push_ctx(
        &mut self,
        at: i64,
        level: LogLevel,
        message: impl Into<SharedString>,
        context: LogContext,
    ) -> u64 {
        let message = repoharbor_core::privacy::redact_user_paths(message.into().as_ref());
        self.seq += 1;
        self.entries.push_front(LogEntry {
            id: self.seq,
            at,
            level,
            message: SharedString::from(message),
            context,
        });
        while self.entries.len() > CAPACITY {
            self.entries.pop_back();
        }
        self.seq
    }

    /// Map a toast into a log line. The caller skips in-flight Progress *ticks*
    /// (same-key updates like "Pushing 3/10…"); the first Progress for a key is
    /// an event and should be recorded.
    pub fn record_toast(
        &mut self,
        at: i64,
        kind: ToastKind,
        title: &str,
        detail: Option<&str>,
        context: LogContext,
    ) -> u64 {
        let level = match kind {
            ToastKind::Success | ToastKind::Progress => LogLevel::Info,
            ToastKind::Info => LogLevel::Warn,
            ToastKind::Error => LogLevel::Error,
        };
        let message = match detail.filter(|d| !d.is_empty()) {
            Some(d) => format!("{title} — {d}"),
            None => title.to_string(),
        };
        self.push_ctx(at, level, message, context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_drops_oldest() {
        let mut log = ActivityLog::default();
        for i in 0..(CAPACITY + 5) {
            log.push(i as i64, LogLevel::Info, format!("e{i}"));
        }
        assert_eq!(log.entries().count(), CAPACITY);
        let newest = log.entries().next().unwrap();
        assert_eq!(newest.message.as_ref(), format!("e{}", CAPACITY + 4));
        let oldest = log.entries().last().unwrap();
        assert_eq!(oldest.message.as_ref(), "e5");
    }

    #[test]
    fn records_every_toast_kind() {
        let mut log = ActivityLog::default();
        assert_eq!(
            log.record_toast(
                1,
                ToastKind::Progress,
                "Pushing…",
                None,
                LogContext::default()
            ),
            1
        );
        log.record_toast(
            1,
            ToastKind::Success,
            "Pushed",
            Some("origin/main"),
            LogContext::default(),
        );
        log.record_toast(
            1,
            ToastKind::Info,
            "Skipped",
            Some("up to date"),
            LogContext::default(),
        );
        log.record_toast(
            1,
            ToastKind::Error,
            "Push failed",
            Some("denied"),
            LogContext::default(),
        );
        let levels: Vec<_> = log.entries().map(|e| e.level).collect();
        // Newest-first.
        assert_eq!(
            levels,
            vec![
                LogLevel::Error,
                LogLevel::Warn,
                LogLevel::Info,
                LogLevel::Info
            ]
        );
        let newest = log.entries().next().unwrap();
        assert_eq!(newest.message.as_ref(), "Push failed — denied");
    }

    #[test]
    fn get_returns_entry_with_context() {
        let mut log = ActivityLog::default();
        let ctx = LogContext {
            repo_name: Some("ISO".into()),
            branch: Some("19.0".into()),
            ..LogContext::default()
        };
        let id = log.push_ctx(1, LogLevel::Error, "failed", ctx);
        let e = log.get(id).expect("stored");
        assert_eq!(e.context.repo_name.as_deref(), Some("ISO"));
        assert_eq!(e.context.branch.as_deref(), Some("19.0"));
        assert!(log.get(id + 1).is_none());
    }

    #[test]
    fn merge_repo_from_fills_empty_fields_and_appends_facts() {
        let mut a = LogContext {
            facts: vec![("a".into(), "1".into())],
            ..LogContext::default()
        };
        let b = LogContext {
            repo_name: Some("ISO".into()),
            branch: Some("19.0".into()),
            facts: vec![("b".into(), "2".into())],
            ..LogContext::default()
        };
        a.merge_repo_from(b);
        assert_eq!(a.repo_name.as_deref(), Some("ISO"));
        assert_eq!(a.branch.as_deref(), Some("19.0"));
        assert_eq!(a.facts.len(), 2);
    }
}
