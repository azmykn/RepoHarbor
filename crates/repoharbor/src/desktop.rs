//! Self-contained Linux `.desktop` + hicolor icon installer and XDG autostart helper.
//!
//! Icons are embedded directly via `include_bytes!`, so standalone release tarballs,
//! AppImages, or GUI repair buttons work with zero external scripts.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const APP_ID: &str = "com.digitscode.repoharbor";
const SHORT_NAME: &str = "repoharbor";

const ICON_32: &[u8] = include_bytes!("../../../packaging/icons/32x32.png");
const ICON_64: &[u8] = include_bytes!("../../../packaging/icons/64x64.png");
const ICON_128: &[u8] = include_bytes!("../../../packaging/icons/128x128.png");
const ICON_512: &[u8] = include_bytes!("../../../packaging/icons/icon.png");
const ICON_SVG: &[u8] = include_bytes!("../../../packaging/icons/repoharbor.svg");
const DESKTOP_TEMPLATE: &str = include_str!("../../../packaging/repoharbor.desktop");

/// Resolve the most stable executable path for `.desktop` and autostart entries.
pub fn preferred_exec_path() -> String {
    if let Some(ai) = std::env::var_os("APPIMAGE").filter(|v| !v.is_empty()) {
        return PathBuf::from(ai).to_string_lossy().into_owned();
    }
    if let Some(bin_dir) = repoharbor_core::paths::user_bin_dir() {
        let user_bin = bin_dir.join(SHORT_NAME);
        if user_bin.is_file() {
            return user_bin.to_string_lossy().into_owned();
        }
    }
    std::env::current_exe()
        .unwrap_or_else(|_| PathBuf::from(SHORT_NAME))
        .to_string_lossy()
        .into_owned()
}

/// Write `.desktop` entries and hicolor icons into user-local XDG data dirs (`data_home`).
pub fn install_desktop_entry_at(data_home: &Path, exec_path: &str) -> Result<PathBuf, String> {
    let app_dir = data_home.join("applications");
    let icon_base = data_home.join("icons/hicolor");
    fs::create_dir_all(&app_dir).map_err(|e| e.to_string())?;

    for legacy in ["com.orrery.app", "orrery"] {
        let _ = fs::remove_file(app_dir.join(format!("{legacy}.desktop")));
        for size in [
            "32x32", "64x64", "128x128", "256x256", "512x512", "scalable",
        ] {
            let _ = fs::remove_file(
                icon_base
                    .join(size)
                    .join("apps")
                    .join(format!("{legacy}.png")),
            );
            let _ = fs::remove_file(
                icon_base
                    .join(size)
                    .join("apps")
                    .join(format!("{legacy}.svg")),
            );
        }
    }

    let pngs: &[(&str, &[u8])] = &[
        ("32x32", ICON_32),
        ("64x64", ICON_64),
        ("128x128", ICON_128),
        ("512x512", ICON_512),
    ];
    for (size, bytes) in pngs {
        let dir = icon_base.join(size).join("apps");
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        fs::write(dir.join(format!("{SHORT_NAME}.png")), bytes).map_err(|e| e.to_string())?;
        fs::write(dir.join(format!("{APP_ID}.png")), bytes).map_err(|e| e.to_string())?;
    }

    let svg_dir = icon_base.join("scalable/apps");
    fs::create_dir_all(&svg_dir).map_err(|e| e.to_string())?;
    fs::write(svg_dir.join(format!("{SHORT_NAME}.svg")), ICON_SVG).map_err(|e| e.to_string())?;
    fs::write(svg_dir.join(format!("{APP_ID}.svg")), ICON_SVG).map_err(|e| e.to_string())?;

    let escaped_exec = exec_path.replace(' ', "\\ ");
    let mut rendered = String::new();
    for line in DESKTOP_TEMPLATE.lines() {
        if line.starts_with("Exec=") {
            rendered.push_str(&format!("Exec={escaped_exec}\n"));
        } else if line.starts_with("Icon=") {
            rendered.push_str(&format!("Icon={SHORT_NAME}\n"));
        } else {
            rendered.push_str(line);
            rendered.push('\n');
        }
    }

    let primary = app_dir.join(format!("{APP_ID}.desktop"));
    let alias = app_dir.join(format!("{SHORT_NAME}.desktop"));
    fs::write(&primary, &rendered).map_err(|e| e.to_string())?;
    fs::write(&alias, &rendered).map_err(|e| e.to_string())?;

    let _ = Command::new("update-desktop-database")
        .arg(&app_dir)
        .output();
    let _ = Command::new("gtk-update-icon-cache")
        .args(["-f", "-t"])
        .arg(&icon_base)
        .output();

    Ok(primary)
}

/// Install the user-local desktop shortcut + hicolor icons for RepoHarbor (and
/// optionally enable XDG autostart on login).
pub fn install_desktop_entry(enable_autostart: bool) -> Result<PathBuf, String> {
    let data_home = repoharbor_core::paths::xdg_data_home().ok_or("no XDG data directory")?;
    let exec_path = preferred_exec_path();
    let desktop_path = install_desktop_entry_at(&data_home, &exec_path)?;
    if enable_autostart {
        repoharbor_core::config::set_autostart_enabled(true, &exec_path)?;
    }
    Ok(desktop_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_desktop_entry_at_writes_desktop_and_icons() {
        let root = std::env::temp_dir().join(format!(
            "repoharbor-desktop-test-{}-{}",
            std::process::id(),
            crate::data::now_unix()
        ));
        let desktop = install_desktop_entry_at(&root, "/home/test/.local/bin/repoharbor").unwrap();
        assert!(desktop.is_file());
        let text = fs::read_to_string(&desktop).unwrap();
        assert!(text.contains("Exec=/home/test/.local/bin/repoharbor"));
        assert!(text.contains("StartupWMClass=com.digitscode.repoharbor"));
        assert!(
            root.join("icons/hicolor/128x128/apps/repoharbor.png")
                .is_file()
        );
        assert!(
            root.join("icons/hicolor/scalable/apps/com.digitscode.repoharbor.svg")
                .is_file()
        );
        let _ = fs::remove_dir_all(&root);
    }
}
