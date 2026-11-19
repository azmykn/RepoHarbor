//! Owner-only storage for per-deployment API keys in the Provider List View.
//!
//! Keys entered for individual pool rows live under
//! `~/.local/share/repoharbor/pool_keys.toml` with mode `0600` rather than
//! inside `config.toml`, mirroring the security boundary in `crate::cloud`.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

const POOL_KEYS_FILE: &str = "pool_keys.toml";

/// Serializes read-modify-write operations on `pool_keys.toml`.
static WRITE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

fn write_path() -> Option<PathBuf> {
    crate::paths::data_dir().map(|d| d.join(POOL_KEYS_FILE))
}

fn read_path() -> Option<PathBuf> {
    crate::paths::resolve_data_file(POOL_KEYS_FILE)
}

/// Parse TOML key map (`id = "secret"`), skipping blank entries.
pub fn parse(text: &str) -> HashMap<String, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return HashMap::new();
    }
    toml::from_str::<BTreeMap<String, String>>(trimmed)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(k, v)| {
            let id = k.trim().to_string();
            let val = v.trim().to_string();
            if id.is_empty() || val.is_empty() {
                None
            } else {
                Some((id, val))
            }
        })
        .collect()
}

/// Render sorted TOML for deterministic writes, omitting blank entries.
pub fn render(map: &HashMap<String, String>) -> String {
    let sorted: BTreeMap<&str, &str> = map
        .iter()
        .filter_map(|(k, v)| {
            let id = k.trim();
            let val = v.trim();
            if id.is_empty() || val.is_empty() {
                None
            } else {
                Some((id, val))
            }
        })
        .collect();
    if sorted.is_empty() {
        return String::new();
    }
    toml::to_string(&sorted).unwrap_or_default()
}

fn load_from(path: &Path) -> HashMap<String, String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| parse(&s))
        .unwrap_or_default()
}

fn write_to(path: &Path, map: &HashMap<String, String>) -> Result<(), String> {
    let body = render(map);
    if body.is_empty() {
        if path.exists() {
            std::fs::remove_file(path).map_err(|e| e.to_string())?;
        }
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    // Create owner-only from the start (no umask race) — keys are secrets.
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .map_err(|e| e.to_string())?;
        file.write_all(body.as_bytes()).map_err(|e| e.to_string())?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, body).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Load all stored per-deployment keys.
pub fn load() -> HashMap<String, String> {
    read_path().map(|p| load_from(&p)).unwrap_or_default()
}

/// Look up the key stored specifically for deployment `id`.
pub fn get(id: &str) -> Option<String> {
    let id = id.trim();
    if id.is_empty() {
        return None;
    }
    load().remove(id).filter(|v| !v.is_empty())
}

/// True when deployment `id` has a dedicated stored key.
pub fn has(id: &str) -> bool {
    get(id).is_some()
}

/// Store or remove the key for deployment `id`. Empty/whitespace removes it.
pub fn set(id: &str, key: &str) -> Result<(), String> {
    let path = write_path().ok_or("no data directory")?;
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    set_at(&path, id, key)
}

fn set_at(path: &Path, id: &str, key: &str) -> Result<(), String> {
    let id = id.trim();
    if id.is_empty() {
        return Err("deployment id is empty".into());
    }
    let mut map = load_from(path);
    let key = key.trim();
    if key.is_empty() {
        map.remove(id);
    } else {
        map.insert(id.to_string(), key.to_string());
    }
    write_to(path, &map)
}

/// Keep only keys whose deployment id appears in `ids`, removing orphaned entries.
pub fn retain(ids: &[String]) -> Result<(), String> {
    let path = write_path().ok_or("no data directory")?;
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    retain_at(&path, ids)
}

fn retain_at(path: &Path, ids: &[String]) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let mut map = load_from(path);
    let before = map.len();
    map.retain(|k, _| ids.iter().any(|keep| keep == k));
    if map.len() != before {
        write_to(path, &map)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_render_roundtrips_and_drops_blanks() {
        let mut map = HashMap::new();
        map.insert("p2".into(), "secret-two".into());
        map.insert("p1".into(), "  secret-one  ".into());
        map.insert("empty".into(), "   ".into());

        let text = render(&map);
        assert!(!text.contains("empty"));
        let parsed = parse(&text);
        assert_eq!(parsed.get("p1").map(String::as_str), Some("secret-one"));
        assert_eq!(parsed.get("p2").map(String::as_str), Some("secret-two"));
        assert!(!parsed.contains_key("empty"));
    }

    #[test]
    fn set_and_retain_manage_keys_and_cleanup_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pool_keys.toml");

        set_at(&path, "dep-1", "k-alpha").unwrap();
        set_at(&path, "dep-2", "k-beta").unwrap();
        assert_eq!(load_from(&path).len(), 2);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }

        // Empty value removes the entry.
        set_at(&path, "dep-1", "   ").unwrap();
        let loaded = load_from(&path);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded.get("dep-2").map(String::as_str), Some("k-beta"));

        // Retain with non-matching ids drops the last key and deletes the file.
        retain_at(&path, &["other".into()]).unwrap();
        assert!(!path.exists());
    }
}
