//! Self-contained Linux `.desktop` + hicolor icon installer and XDG autostart helper.
//!
//! Icons are embedded directly via `include_bytes!`, so standalone release tarballs,
//! AppImages, or GUI repair buttons work with zero external scripts.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const APP_ID: &str = "com.digitscode.repoharbor";
const SHORT_NAME: &str = "repoharbor";

#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
const ICON_32: &[u8] = include_bytes!("../../../packaging/icons/32x32.png");
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
const ICON_64: &[u8] = include_bytes!("../../../packaging/icons/64x64.png");
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
const ICON_128: &[u8] = include_bytes!("../../../packaging/icons/128x128.png");
#[cfg_attr(
    not(any(target_os = "linux", target_os = "macos", test)),
    allow(dead_code)
)]
const ICON_512: &[u8] = include_bytes!("../../../packaging/icons/icon.png");
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
const ICON_SVG: &[u8] = include_bytes!("../../../packaging/icons/repoharbor.svg");
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
const DESKTOP_TEMPLATE: &str = include_str!("../../../packaging/repoharbor.desktop");

/// Resolve the most stable executable path for desktop shortcuts and autostart entries.
pub fn preferred_exec_path() -> String {
    if let Some(ai) = std::env::var_os("APPIMAGE").filter(|v| !v.is_empty()) {
        return PathBuf::from(ai).to_string_lossy().into_owned();
    }
    let bin_name = format!("{SHORT_NAME}{}", std::env::consts::EXE_SUFFIX);
    if let Some(bin_dir) = repoharbor_core::paths::user_bin_dir() {
        let user_bin = bin_dir.join(&bin_name);
        if user_bin.is_file() {
            return user_bin.to_string_lossy().into_owned();
        }
    }
    std::env::current_exe()
        .unwrap_or_else(|_| PathBuf::from(bin_name))
        .to_string_lossy()
        .into_owned()
}

/// Write `.desktop` entries and hicolor icons into user-local XDG data dirs (`data_home`).
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
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

/// Build a self-contained macOS `RepoHarbor.app` bundle inside `apps_dir` (e.g. `~/Applications`)
/// that delegates execution to `exec_path`.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub fn install_macos_app_bundle_at(apps_dir: &Path, exec_path: &str) -> Result<PathBuf, String> {
    let bundle = apps_dir.join("RepoHarbor.app");
    let contents = bundle.join("Contents");
    let macos_dir = contents.join("MacOS");
    let resources_dir = contents.join("Resources");
    fs::create_dir_all(&macos_dir).map_err(|e| e.to_string())?;
    fs::create_dir_all(&resources_dir).map_err(|e| e.to_string())?;

    let plist = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n\
         <dict>\n\
           <key>CFBundleName</key>\n\
           <string>RepoHarbor</string>\n\
           <key>CFBundleDisplayName</key>\n\
           <string>RepoHarbor</string>\n\
           <key>CFBundleIdentifier</key>\n\
           <string>{APP_ID}</string>\n\
           <key>CFBundleVersion</key>\n\
           <string>{ver}</string>\n\
           <key>CFBundleShortVersionString</key>\n\
           <string>{ver}</string>\n\
           <key>CFBundlePackageType</key>\n\
           <string>APPL</string>\n\
           <key>CFBundleExecutable</key>\n\
           <string>repoharbor</string>\n\
           <key>CFBundleIconFile</key>\n\
           <string>icon.png</string>\n\
           <key>NSHighResolutionCapable</key>\n\
           <true/>\n\
         </dict>\n\
         </plist>\n",
        ver = crate::identity::APP_VERSION
    );
    fs::write(contents.join("Info.plist"), plist).map_err(|e| e.to_string())?;
    fs::write(resources_dir.join("icon.png"), ICON_512).map_err(|e| e.to_string())?;

    let launcher = macos_dir.join("repoharbor");
    let script = format!("#!/bin/sh\nexec \"{exec_path}\" \"$@\"\n");
    fs::write(&launcher, script).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = fs::set_permissions(&launcher, fs::Permissions::from_mode(0o755));
    }
    Ok(bundle)
}

#[cfg(target_os = "windows")]
fn install_windows_start_menu_shortcut(exec_path: &str) -> Result<PathBuf, String> {
    let appdata = repoharbor_core::paths::xdg_data_home().ok_or("no AppData directory")?;
    let programs_dir = appdata
        .join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs");
    fs::create_dir_all(&programs_dir).map_err(|e| e.to_string())?;
    let lnk_path = programs_dir.join("RepoHarbor.lnk");
    let lnk_str = lnk_path.to_string_lossy().replace('\'', "''");
    let target_str = exec_path.replace('\'', "''");
    let ps = format!(
        "$s = (New-Object -ComObject WScript.Shell).CreateShortcut('{lnk_str}'); \
         $s.TargetPath = '{target_str}'; \
         $s.Description = 'RepoHarbor — Git Fleet Command Center'; \
         $s.Save()"
    );
    let ok = Command::new("powershell")
        .args(["-NoProfile", "-Command", &ps])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok && lnk_path.exists() {
        return Ok(lnk_path);
    }
    let cmd_path = programs_dir.join("RepoHarbor.cmd");
    fs::write(
        &cmd_path,
        format!("@echo off\r\nstart \"\" \"{exec_path}\"\r\n"),
    )
    .map_err(|e| e.to_string())?;
    Ok(cmd_path)
}

/// Install the user-local desktop shortcut / application bundle for RepoHarbor (and
/// optionally enable autostart on login).
pub fn install_desktop_entry(enable_autostart: bool) -> Result<PathBuf, String> {
    let exec_path = preferred_exec_path();

    #[cfg(target_os = "macos")]
    let desktop_path = {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or("no HOME directory")?;
        install_macos_app_bundle_at(&home.join("Applications"), &exec_path)?
    };

    #[cfg(target_os = "windows")]
    let desktop_path = install_windows_start_menu_shortcut(&exec_path)?;

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let desktop_path = {
        let data_home = repoharbor_core::paths::xdg_data_home().ok_or("no XDG data directory")?;
        install_desktop_entry_at(&data_home, &exec_path)?
    };

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

        let bundle = install_macos_app_bundle_at(
            &root.join("Applications"),
            "/Users/test/.local/bin/repoharbor",
        )
        .unwrap();
        assert!(bundle.join("Contents/Info.plist").is_file());
        assert!(bundle.join("Contents/MacOS/repoharbor").is_file());
        assert!(bundle.join("Contents/Resources/icon.png").is_file());

        let _ = fs::remove_dir_all(&root);
    }
}
