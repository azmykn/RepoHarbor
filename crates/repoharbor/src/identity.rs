//! Product identity shown in Settings → About and used for public attribution.
//!
//! `APP_VERSION` is compiled from `crates/repoharbor/Cargo.toml`, so the in-app
//! About page always matches the crate that built the binary. Bump all three
//! workspace crate versions together (calendar `YYYY.M.P`) when shipping a
//! user-facing change.

/// Display name.
pub const APP_NAME: &str = "RepoHarbor";

/// Crate version (`YYYY.M.P`), from `Cargo.toml`.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Canonical source repository.
pub const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

/// GitHub Pages documentation.
pub const DOCS_URL: &str = "https://azmykn.github.io/RepoHarbor/";

/// Changelog on GitHub.
pub const CHANGELOG_URL: &str = "https://github.com/azmykn/RepoHarbor/blob/main/CHANGELOG.md";

pub const COPYRIGHT_YEAR: &str = "2026";
pub const PUBLISHER: &str = "DigitsCode";
pub const AUTHOR: &str = "Azmy Karam";
pub const CONTACT_EMAIL: &str = "azmykn@gmail.com";
pub const CONTACT_PHONE: &str = "+966559622034";
pub const LICENSE_SPDX: &str = "MIT";

/// One-line MIT credit required for the included Orrery portions.
pub const UPSTREAM_CREDIT: &str = "Includes software originally published as Orrery by Seb Burrell (MIT). This is an independent DigitsCode product, not an official fork continuation.";

/// Short public-use summary (not a substitute for LICENSE).
pub const PUBLIC_USE: &str = "Released under the MIT License for public use: you may use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies, provided the copyright and permission notices in LICENSE are retained.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_calendar() {
        let parts: Vec<&str> = APP_VERSION.split('.').collect();
        assert_eq!(parts.len(), 3, "expected YYYY.M.P, got {APP_VERSION}");
        let year: u32 = parts[0].parse().expect("year");
        assert!(year >= 2026, "year {year}");
        let month: u32 = parts[1].parse().expect("month");
        assert!((1..=12).contains(&month), "month {month}");
        let patch: u32 = parts[2].parse().expect("patch");
        let _ = patch;
    }

    #[test]
    fn public_identity_matches_published_contact() {
        assert_eq!(CONTACT_EMAIL, "azmykn@gmail.com");
        assert_eq!(CONTACT_PHONE, "+966559622034");
        assert!(
            REPOSITORY.contains("github.com/azmykn/RepoHarbor"),
            "repository {REPOSITORY}"
        );
        assert_eq!(LICENSE_SPDX, "MIT");
        assert!(!AUTHOR.is_empty());
        assert!(!PUBLISHER.is_empty());
    }
}
