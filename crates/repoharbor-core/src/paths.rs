//! XDG config/data paths for RepoHarbor.
//!
//! New installs use `~/.config/repoharbor` and `~/.local/share/repoharbor`.
//! For one transition release, **reads** also fall back to the legacy Orrery
//! paths (`~/.config/orrery`, `~/.local/share/orrery`) so DigitsCode users are
//! not reset. **Writes always go to the new paths only.**

use std::path::PathBuf;

const APP: &str = "repoharbor";
/// Previous product data dir name (Orrery). Read-only fallback.
const LEGACY_APP: &str = "orrery";

/// Writable XDG data directory: `~/.local/share/repoharbor`.
pub fn data_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join(APP))
}

/// Legacy Orrery data directory (read fallback only).
pub fn legacy_data_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join(LEGACY_APP))
}

/// Writable config file path: `~/.config/repoharbor/config.toml`.
pub fn config_write_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(APP).join("config.toml"))
}

/// Legacy Orrery config path (read fallback only).
pub fn legacy_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(LEGACY_APP).join("config.toml"))
}

/// Config path for reading: prefer the new file when present, else the legacy
/// Orrery file. Returns the new path when neither exists (caller may seed defaults).
pub fn config_read_path() -> Option<PathBuf> {
    let write = config_write_path()?;
    if write.exists() {
        return Some(write);
    }
    if let Some(legacy) = legacy_config_path().filter(|p| p.exists()) {
        return Some(legacy);
    }
    Some(write)
}

/// Resolve a file under the app data dir for **reading**: new path if it exists,
/// otherwise the same relative path under the legacy Orrery data dir, otherwise
/// the (not-yet-created) new path.
pub fn resolve_data_file(file_name: &str) -> Option<PathBuf> {
    let new = data_dir()?.join(file_name);
    if new.exists() {
        return Some(new);
    }
    if let Some(legacy) = legacy_data_dir()
        .map(|d| d.join(file_name))
        .filter(|p| p.exists())
    {
        return Some(legacy);
    }
    Some(new)
}
