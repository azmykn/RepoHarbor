//! Release update checker, install metadata, and atomic binary self-updater.
//!
//! Keeps first-time installs and periodic updates painless across four Linux
//! installation shapes:
//! - User binary (`~/.local/bin/repoharbor`): one-click in-place self-update.
//! - Portable AppImage (`$APPIMAGE`): atomic AppImage swap.
//! - Source checkout (`scripts/install.sh --from-source`): git pull + release rebuild.
//! - Distro package (`.deb`/`.rpm` under `/usr/bin`): release notice + exact command.

use std::fs;
use std::io::Read as _;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::paths;

const GITHUB_LATEST_RELEASE_API: &str =
    "https://api.github.com/repos/azmykn/RepoHarbor/releases/latest";

/// Metadata stored in `~/.local/share/repoharbor/install-meta.toml` by
/// `scripts/install.sh` and self-updates so RepoHarbor knows how it was installed.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InstallMeta {
    /// `"binary"`, `"source"`, `"appimage"`, or `"package"`.
    #[serde(default)]
    pub kind: String,
    /// Path to source checkout if installed via `--from-source`.
    #[serde(default)]
    pub source_dir: Option<String>,
    /// Installed calendar version (`YYYY.M.P`).
    #[serde(default)]
    pub version: String,
    /// RFC3339 / unix timestamp string when installed/updated.
    #[serde(default)]
    pub installed_at: String,
}

/// Detected runtime installation strategy for the running executable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallKind {
    UserBinary(PathBuf),
    AppImage(PathBuf),
    SourceCheckout {
        repo_dir: PathBuf,
        bin_path: PathBuf,
    },
    SystemPackage(PathBuf),
}

impl InstallKind {
    /// Human-readable label shown in Settings → About and `repoharbor update`.
    pub fn label(&self) -> String {
        match self {
            Self::UserBinary(p) => format!("User binary ({})", crate::scan::abbreviate(p)),
            Self::AppImage(p) => format!("AppImage ({})", crate::scan::abbreviate(p)),
            Self::SourceCheckout { repo_dir, .. } => {
                format!("Source checkout ({})", crate::scan::abbreviate(repo_dir))
            }
            Self::SystemPackage(p) => format!("System package ({})", crate::scan::abbreviate(p)),
        }
    }

    /// True when the current executable can be updated cleanly without root privileges.
    pub fn can_self_update_in_place(&self) -> bool {
        !matches!(self, Self::SystemPackage(_))
    }

    /// Recommended shell command for updating externally (fallback for system packages).
    pub fn external_update_command(&self) -> String {
        match self {
            Self::SourceCheckout { repo_dir, .. } => format!(
                "cd {} && git pull --ff-only && bash scripts/install.sh --from-source",
                shell_quote_path(repo_dir)
            ),
            Self::SystemPackage(_) => {
                "curl -fsSL https://raw.githubusercontent.com/azmykn/RepoHarbor/main/scripts/install.sh | bash"
                    .to_string()
            }
            Self::UserBinary(_) | Self::AppImage(_) => "repoharbor update".to_string(),
        }
    }
}

fn shell_quote_path(p: &Path) -> String {
    let raw = p.to_string_lossy();
    format!("'{}'", raw.replace('\'', "'\"'\"'"))
}

/// Metadata parsed from a GitHub Release response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseInfo {
    pub tag_name: String,
    pub version: String,
    pub notes: String,
    pub html_url: String,
    pub tarball_url: Option<String>,
    pub tarball_name: Option<String>,
    pub appimage_url: Option<String>,
    pub appimage_name: Option<String>,
    pub checksums_url: Option<String>,
}

/// Result of checking for a newer version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateCheck {
    UpToDate {
        current: String,
        latest_tag: Option<String>,
    },
    Available {
        current: String,
        release: ReleaseInfo,
    },
}

#[derive(Debug, Deserialize)]
struct GithubReleaseJson {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    assets: Vec<GithubAssetJson>,
}

#[derive(Debug, Deserialize)]
struct GithubAssetJson {
    name: String,
    browser_download_url: String,
}

/// Parse a calendar version tag like `"v2026.11.1"` or `"2026.10.14"` into `(year, month, patch)`.
pub fn parse_calendar_version(raw: &str) -> Option<(u32, u32, u32)> {
    let clean = raw.trim().strip_prefix('v').unwrap_or(raw.trim());
    let clean = clean.split(['-', '+']).next()?.trim();
    let mut parts = clean.split('.');
    let y = parts.next()?.parse::<u32>().ok()?;
    let m = parts.next()?.parse::<u32>().ok()?;
    let p = parts.next()?.parse::<u32>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((y, m, p))
}

/// True when `candidate` is strictly newer than `current` under calendar semver (`YYYY.M.P`).
pub fn is_newer_version(candidate: &str, current: &str) -> bool {
    match (
        parse_calendar_version(candidate),
        parse_calendar_version(current),
    ) {
        (Some(cand), Some(cur)) => cand > cur,
        _ => false,
    }
}

/// Load install metadata from `~/.local/share/repoharbor/install-meta.toml` if present.
pub fn load_install_meta() -> Option<InstallMeta> {
    let path = paths::install_meta_path()?;
    let text = fs::read_to_string(path).ok()?;
    toml::from_str(&text).ok()
}

/// Save install metadata into `~/.local/share/repoharbor/install-meta.toml`.
pub fn save_install_meta(meta: &InstallMeta) -> Result<(), String> {
    let path = paths::install_meta_path().ok_or("no data directory for install-meta.toml")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let text = toml::to_string_pretty(meta).map_err(|e| e.to_string())?;
    fs::write(path, text).map_err(|e| e.to_string())
}

fn dir_is_user_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".repoharbor-write-probe-{}", std::process::id()));
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(_) => {
            let _ = fs::remove_file(probe);
            true
        }
        Err(_) => false,
    }
}

fn find_git_ancestor(start: &Path) -> Option<PathBuf> {
    for anc in start.ancestors() {
        if anc.join(".git").exists() && anc.join("Cargo.toml").exists() {
            return Some(anc.to_path_buf());
        }
    }
    None
}

/// Pure, testable install-kind detector given executable path, optional `$APPIMAGE`, and metadata.
pub fn detect_install_kind_for(
    exe: &Path,
    appimage_env: Option<&Path>,
    meta: Option<&InstallMeta>,
) -> InstallKind {
    if let Some(ai) = appimage_env {
        if !ai.as_os_str().is_empty() {
            return InstallKind::AppImage(ai.to_path_buf());
        }
    }
    if let Some(m) = meta {
        if m.kind == "source" {
            if let Some(src) = &m.source_dir {
                let repo_dir = PathBuf::from(src);
                if repo_dir.join(".git").exists() {
                    return InstallKind::SourceCheckout {
                        repo_dir,
                        bin_path: exe.to_path_buf(),
                    };
                }
            }
        }
    }
    if let Some(parent) = exe.parent() {
        let s = parent.to_string_lossy();
        if s.ends_with("/target/release") || s.ends_with("/target/debug") {
            if let Some(repo_dir) = find_git_ancestor(parent) {
                return InstallKind::SourceCheckout {
                    repo_dir,
                    bin_path: exe.to_path_buf(),
                };
            }
        }
        if dir_is_user_writable(parent) {
            return InstallKind::UserBinary(exe.to_path_buf());
        }
    }
    InstallKind::SystemPackage(exe.to_path_buf())
}

/// Inspect the running process environment to classify how RepoHarbor was installed.
pub fn detect_install_kind() -> InstallKind {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("repoharbor"));
    let ai = std::env::var_os("APPIMAGE").map(PathBuf::from);
    let meta = load_install_meta();
    detect_install_kind_for(&exe, ai.as_deref(), meta.as_ref())
}

/// Query GitHub Releases (`/releases/latest`) and compare against `current_version`.
pub async fn check_latest_release(current_version: &str) -> Result<UpdateCheck, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .user_agent(format!("RepoHarbor/{current_version}"))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(GITHUB_LATEST_RELEASE_API)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("release check failed: {e}"))?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(UpdateCheck::UpToDate {
            current: current_version.to_string(),
            latest_tag: None,
        });
    }
    if !resp.status().is_success() {
        return Err(format!("GitHub Releases returned HTTP {}", resp.status()));
    }

    let gh: GithubReleaseJson = resp
        .json()
        .await
        .map_err(|e| format!("invalid release payload: {e}"))?;
    if gh.draft {
        return Ok(UpdateCheck::UpToDate {
            current: current_version.to_string(),
            latest_tag: None,
        });
    }

    let version = gh
        .tag_name
        .trim()
        .strip_prefix('v')
        .unwrap_or(gh.tag_name.trim())
        .to_string();
    let tarball = gh
        .assets
        .iter()
        .find(|a| a.name.ends_with(".tar.gz") && a.name.contains("linux"));
    let appimage = gh.assets.iter().find(|a| a.name.ends_with(".AppImage"));
    let checksums = gh
        .assets
        .iter()
        .find(|a| a.name.eq_ignore_ascii_case("SHA256SUMS.txt"));

    let release = ReleaseInfo {
        tag_name: gh.tag_name.clone(),
        version: version.clone(),
        notes: gh.body.unwrap_or_default().trim().to_string(),
        html_url: gh.html_url,
        tarball_url: tarball.map(|a| a.browser_download_url.clone()),
        tarball_name: tarball.map(|a| a.name.clone()),
        appimage_url: appimage.map(|a| a.browser_download_url.clone()),
        appimage_name: appimage.map(|a| a.name.clone()),
        checksums_url: checksums.map(|a| a.browser_download_url.clone()),
    };

    if is_newer_version(&release.tag_name, current_version) {
        Ok(UpdateCheck::Available {
            current: current_version.to_string(),
            release,
        })
    } else {
        Ok(UpdateCheck::UpToDate {
            current: current_version.to_string(),
            latest_tag: Some(gh.tag_name),
        })
    }
}

/// Extract expected hex SHA-256 checksum for `asset_name` from a `SHA256SUMS.txt` file.
pub fn parse_sha256_for_asset(sums_txt: &str, asset_name: &str) -> Option<String> {
    for line in sums_txt.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let digest = parts.next()?;
        let file = parts.next()?.trim_start_matches('*');
        if (file == asset_name || file.ends_with(&format!("/{asset_name}")))
            && digest.len() == 64
            && digest.chars().all(|c| c.is_ascii_hexdigit())
        {
            return Some(digest.to_ascii_lowercase());
        }
    }
    None
}

/// Compute lowercase hex SHA-256 for `bytes` and verify it equals `expected_hex`.
pub fn verify_sha256(bytes: &[u8], expected_hex: &str) -> Result<(), String> {
    let actual = format!("{:x}", Sha256::digest(bytes));
    if actual.eq_ignore_ascii_case(expected_hex.trim()) {
        Ok(())
    } else {
        Err(format!(
            "checksum mismatch (expected {}, got {actual})",
            expected_hex.trim()
        ))
    }
}

/// Atomically replace `target` executable with `new_binary_bytes` via a sibling
/// staging file + `0755` chmod + POSIX `rename()`.
pub fn atomic_replace_executable(target: &Path, new_binary_bytes: &[u8]) -> Result<(), String> {
    if new_binary_bytes.is_empty() {
        return Err("downloaded update binary is empty".to_string());
    }
    let parent = target
        .parent()
        .ok_or_else(|| format!("target path has no parent: {}", target.display()))?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let file_name = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("repoharbor");
    let staging = parent.join(format!(".{file_name}.new.{}", std::process::id()));
    fs::write(&staging, new_binary_bytes).map_err(|e| format!("write staging failed: {e}"))?;
    #[cfg(unix)]
    fs::set_permissions(&staging, fs::Permissions::from_mode(0o755))
        .map_err(|e| format!("chmod staging failed: {e}"))?;
    fs::rename(&staging, target).map_err(|e| {
        let _ = fs::remove_file(&staging);
        format!("atomic swap failed: {e}")
    })
}

/// Extract the `repoharbor` executable bytes from a release `.tar.gz` archive.
pub fn extract_binary_from_tar_gz(archive_bytes: &[u8]) -> Result<Vec<u8>, String> {
    let gz = GzDecoder::new(archive_bytes);
    let mut tar = tar::Archive::new(gz);
    let entries = tar
        .entries()
        .map_err(|e| format!("corrupt tar.gz archive: {e}"))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| format!("tar entry error: {e}"))?;
        let path = entry.path().map_err(|e| e.to_string())?;
        if entry.header().entry_type().is_file()
            && path.file_name().and_then(|n| n.to_str()) == Some("repoharbor")
        {
            let mut out = Vec::new();
            entry
                .read_to_end(&mut out)
                .map_err(|e| format!("failed reading binary from archive: {e}"))?;
            if !out.is_empty() {
                return Ok(out);
            }
        }
    }
    Err("release archive did not contain a `repoharbor` binary".to_string())
}

/// Download the appropriate release artifact for `kind`, verify SHA-256 when published,
/// and atomically swap the executable in place.
pub async fn download_and_apply_release_update(
    release: &ReleaseInfo,
    kind: &InstallKind,
) -> Result<PathBuf, String> {
    let (target_path, asset_url, asset_name, is_tarball) = match kind {
        InstallKind::AppImage(path) => (
            path.clone(),
            release
                .appimage_url
                .clone()
                .ok_or("release has no AppImage asset")?,
            release
                .appimage_name
                .clone()
                .unwrap_or_else(|| "RepoHarbor-x86_64.AppImage".to_string()),
            false,
        ),
        InstallKind::UserBinary(path) => (
            path.clone(),
            release
                .tarball_url
                .clone()
                .ok_or("release has no Linux tarball asset")?,
            release
                .tarball_name
                .clone()
                .unwrap_or_else(|| "repoharbor-linux-x86_64.tar.gz".to_string()),
            true,
        ),
        InstallKind::SourceCheckout { bin_path, .. } => {
            let user_bin = paths::user_bin_dir()
                .map(|d| d.join("repoharbor"))
                .unwrap_or_else(|| bin_path.clone());
            let target = if bin_path.parent().is_some_and(dir_is_user_writable)
                && !bin_path.to_string_lossy().contains("/target/")
            {
                bin_path.clone()
            } else {
                user_bin
            };
            (
                target,
                release
                    .tarball_url
                    .clone()
                    .ok_or("release has no Linux tarball asset")?,
                release
                    .tarball_name
                    .clone()
                    .unwrap_or_else(|| "repoharbor-linux-x86_64.tar.gz".to_string()),
                true,
            )
        }
        InstallKind::SystemPackage(_) => {
            return Err(
                "system package installs (/usr/bin) require root — update with apt/dnf or install user binary"
                    .to_string(),
            );
        }
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .user_agent("RepoHarbor-Updater")
        .build()
        .map_err(|e| e.to_string())?;

    let artifact_bytes = client
        .get(&asset_url)
        .send()
        .await
        .map_err(|e| format!("failed downloading update: {e}"))?
        .error_for_status()
        .map_err(|e| format!("update download error: {e}"))?
        .bytes()
        .await
        .map_err(|e| format!("failed reading update stream: {e}"))?;

    if let Some(sums_url) = &release.checksums_url {
        if let Ok(resp) = client.get(sums_url).send().await {
            if resp.status().is_success() {
                if let Ok(sums_txt) = resp.text().await {
                    if let Some(expected) = parse_sha256_for_asset(&sums_txt, &asset_name) {
                        verify_sha256(&artifact_bytes, &expected)?;
                    }
                }
            }
        }
    }

    let final_bytes = if is_tarball {
        extract_binary_from_tar_gz(&artifact_bytes)?
    } else {
        artifact_bytes.to_vec()
    };

    atomic_replace_executable(&target_path, &final_bytes)?;

    if let Some(cache_dir) = paths::update_cache_dir() {
        let _ = fs::remove_dir_all(cache_dir);
    }

    let meta_kind = match kind {
        InstallKind::AppImage(_) => "appimage",
        InstallKind::SourceCheckout { .. } => "binary",
        _ => "binary",
    };
    let _ = save_install_meta(&InstallMeta {
        kind: meta_kind.to_string(),
        source_dir: None,
        version: release.version.clone(),
        installed_at: chrono::Utc::now().to_rfc3339(),
    });

    Ok(target_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::GzEncoder;
    use flate2::Compression;

    #[test]
    fn calendar_version_ordering_and_tag_stripping() {
        assert_eq!(parse_calendar_version("v2026.10.14"), Some((2026, 10, 14)));
        assert_eq!(parse_calendar_version("2026.11.1"), Some((2026, 11, 1)));
        assert!(is_newer_version("v2026.11.1", "2026.10.14"));
        assert!(is_newer_version("2027.1.0", "2026.12.99"));
        assert!(!is_newer_version("v2026.10.14", "2026.10.14"));
        assert!(!is_newer_version("2026.9.1", "2026.10.14"));
    }

    #[test]
    fn sha256sums_parser_and_verifier() {
        let payload = b"hello repoharbor release";
        let good_hex = format!("{:x}", Sha256::digest(payload));
        let sums = format!(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  other.deb\n\
             {good_hex}  *repoharbor-v2026.11.1-linux-x86_64.tar.gz\n"
        );
        let parsed =
            parse_sha256_for_asset(&sums, "repoharbor-v2026.11.1-linux-x86_64.tar.gz").unwrap();
        assert_eq!(parsed, good_hex);
        assert!(verify_sha256(payload, &parsed).is_ok());
        assert!(verify_sha256(b"tampered", &parsed).is_err());
    }

    #[test]
    fn tar_gz_extraction_and_atomic_replace_roundtrip() {
        let mut gz_buf = Vec::new();
        {
            let encoder = GzEncoder::new(&mut gz_buf, Compression::default());
            let mut builder = tar::Builder::new(encoder);
            let data = b"#!/bin/sh\necho v2026.11.1\n";
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder
                .append_data(&mut header, "repoharbor/repoharbor", &data[..])
                .unwrap();
            builder.into_inner().unwrap().finish().unwrap();
        }
        let extracted = extract_binary_from_tar_gz(&gz_buf).unwrap();
        assert_eq!(extracted, b"#!/bin/sh\necho v2026.11.1\n");

        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("bin/repoharbor");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, b"old binary").unwrap();

        atomic_replace_executable(&target, &extracted).unwrap();
        assert_eq!(fs::read(&target).unwrap(), extracted);
        #[cfg(unix)]
        {
            let mode = fs::metadata(&target).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o755);
        }
    }

    #[test]
    fn install_kind_prefers_appimage_and_meta_source_then_writable_bin() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("repoharbor");
        fs::write(&exe, b"bin").unwrap();

        assert_eq!(
            detect_install_kind_for(&exe, Some(Path::new("/tmp/RepoHarbor.AppImage")), None),
            InstallKind::AppImage(PathBuf::from("/tmp/RepoHarbor.AppImage"))
        );

        assert_eq!(
            detect_install_kind_for(&exe, None, None),
            InstallKind::UserBinary(exe.clone())
        );

        let src_dir = dir.path().join("repo");
        fs::create_dir_all(src_dir.join(".git")).unwrap();
        let meta = InstallMeta {
            kind: "source".into(),
            source_dir: Some(src_dir.to_string_lossy().into_owned()),
            version: "2026.10.14".into(),
            installed_at: String::new(),
        };
        assert_eq!(
            detect_install_kind_for(&exe, None, Some(&meta)),
            InstallKind::SourceCheckout {
                repo_dir: src_dir,
                bin_path: exe,
            }
        );
    }
}
