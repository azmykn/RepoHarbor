//! Conventional commit changelog generator (inspired by `git-cliff`).
//!
//! Groups conventional commit messages into Features, Fixes, Performance,
//! Refactoring, Documentation, and Maintenance sections.

use git2::Repository;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ChangelogSections {
    pub features: Vec<String>,
    pub fixes: Vec<String>,
    pub performance: Vec<String>,
    pub refactors: Vec<String>,
    pub docs: Vec<String>,
    pub maintenance: Vec<String>,
    pub other: Vec<String>,
}

impl ChangelogSections {
    pub fn is_empty(&self) -> bool {
        self.features.is_empty()
            && self.fixes.is_empty()
            && self.performance.is_empty()
            && self.refactors.is_empty()
            && self.docs.is_empty()
            && self.maintenance.is_empty()
            && self.other.is_empty()
    }

    pub fn to_markdown(&self, title: &str) -> String {
        let mut out = String::new();
        out.push_str(&format!("## {title}\n\n"));

        let sections = [
            ("Features", &self.features),
            ("Bug Fixes", &self.fixes),
            ("Performance Improvements", &self.performance),
            ("Refactoring", &self.refactors),
            ("Documentation", &self.docs),
            ("Maintenance & CI", &self.maintenance),
            ("Other Changes", &self.other),
        ];

        for (section_name, items) in sections {
            if !items.is_empty() {
                out.push_str(&format!("### {section_name}\n\n"));
                for item in items {
                    out.push_str(&format!("* {item}\n"));
                }
                out.push('\n');
            }
        }

        out.trim_end().to_string()
    }
}

/// Categorize a single commit subject into appropriate changelog category.
pub fn categorize_commit(subject: &str, short_hash: &str) -> (ChangelogKind, String) {
    let sub = subject.trim();
    let formatted = if !short_hash.is_empty() {
        format!("{sub} (`{short_hash}`)")
    } else {
        sub.to_string()
    };

    let lower = sub.to_ascii_lowercase();
    if lower.starts_with("feat") {
        (ChangelogKind::Feature, formatted)
    } else if lower.starts_with("fix") {
        (ChangelogKind::Fix, formatted)
    } else if lower.starts_with("perf") {
        (ChangelogKind::Perf, formatted)
    } else if lower.starts_with("refactor") || lower.starts_with("clean") {
        (ChangelogKind::Refactor, formatted)
    } else if lower.starts_with("doc") {
        (ChangelogKind::Docs, formatted)
    } else if lower.starts_with("chore")
        || lower.starts_with("ci")
        || lower.starts_with("test")
        || lower.starts_with("build")
    {
        (ChangelogKind::Maintenance, formatted)
    } else {
        (ChangelogKind::Other, formatted)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ChangelogKind {
    Feature,
    Fix,
    Perf,
    Refactor,
    Docs,
    Maintenance,
    Other,
}

/// Read recent commits from the repository and generate a formatted Changelog.
pub fn generate_changelog(path: &str, max_commits: usize) -> Result<String, String> {
    let repo = Repository::open(path).map_err(|e| e.to_string())?;
    let mut revwalk = repo.revwalk().map_err(|e| e.to_string())?;
    revwalk
        .push_head()
        .map_err(|_| "repository has no HEAD commit".to_string())?;

    let mut sections = ChangelogSections::default();
    let branch = repo
        .head()
        .ok()
        .and_then(|h| h.shorthand().map(str::to_string))
        .unwrap_or_else(|| "HEAD".to_string());

    let mut count = 0;
    for oid in revwalk {
        if count >= max_commits {
            break;
        }
        let oid = oid.map_err(|e| e.to_string())?;
        let commit = repo.find_commit(oid).map_err(|e| e.to_string())?;
        let summary = commit.summary().unwrap_or_default().to_string();
        if summary.is_empty() {
            continue;
        }

        let hash_str = oid.to_string();
        let short_hash = &hash_str[..7.min(hash_str.len())];
        let (kind, entry) = categorize_commit(&summary, short_hash);

        match kind {
            ChangelogKind::Feature => sections.features.push(entry),
            ChangelogKind::Fix => sections.fixes.push(entry),
            ChangelogKind::Perf => sections.performance.push(entry),
            ChangelogKind::Refactor => sections.refactors.push(entry),
            ChangelogKind::Docs => sections.docs.push(entry),
            ChangelogKind::Maintenance => sections.maintenance.push(entry),
            ChangelogKind::Other => sections.other.push(entry),
        }
        count += 1;
    }

    if sections.is_empty() {
        return Ok(format!("## Changelog ({branch})\n\nNo commits found."));
    }

    Ok(sections.to_markdown(&format!("Changelog ({branch})")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categorizes_conventional_commits() {
        assert_eq!(
            categorize_commit("feat(ai): add mistral support", "abc1234").0,
            ChangelogKind::Feature
        );
        assert_eq!(
            categorize_commit("fix: handle empty diff", "def5678").0,
            ChangelogKind::Fix
        );
        assert_eq!(
            categorize_commit("docs: update README", "ghi9012").0,
            ChangelogKind::Docs
        );
        assert_eq!(
            categorize_commit("chore: bump crate version", "jkl3456").0,
            ChangelogKind::Maintenance
        );
    }

    #[test]
    fn sections_render_to_markdown() {
        let mut sec = ChangelogSections::default();
        sec.features
            .push("feat(ai): add native Mistral support (`abc1234`)".into());
        sec.fixes
            .push("fix(diff): stage untracked folders (`def5678`)".into());

        let md = sec.to_markdown("Changelog v2026.10.12");
        assert!(md.contains("## Changelog v2026.10.12"));
        assert!(md.contains("### Features"));
        assert!(md.contains("* feat(ai): add native Mistral support (`abc1234`)"));
        assert!(md.contains("### Bug Fixes"));
        assert!(md.contains("* fix(diff): stage untracked folders (`def5678`)"));
    }
}
