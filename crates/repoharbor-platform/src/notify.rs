//! Native desktop notifications across Linux (`org.freedesktop.Notifications` over D-Bus),
//! macOS (`osascript`), and Windows (PowerShell toast/balloon notification).

#[cfg(target_os = "linux")]
use std::collections::HashMap;

#[cfg(target_os = "linux")]
use zbus::zvariant::Value;
#[cfg(target_os = "linux")]
use zbus::Connection;

#[cfg(target_os = "linux")]
#[zbus::proxy(
    interface = "org.freedesktop.Notifications",
    default_service = "org.freedesktop.Notifications",
    default_path = "/org/freedesktop/Notifications"
)]
trait Notifications {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: &[&str],
        hints: HashMap<&str, &Value<'_>>,
        expire_timeout: i32,
    ) -> zbus::Result<u32>;
}

/// Show a desktop notification; returns the server-assigned id. Best-effort —
/// the caller can ignore the result on a desktop without a notification daemon.
#[cfg(target_os = "linux")]
pub async fn send(summary: &str, body: &str) -> Result<u32, String> {
    let conn = Connection::session().await.map_err(|e| e.to_string())?;
    let proxy = NotificationsProxy::new(&conn)
        .await
        .map_err(|e| e.to_string())?;
    proxy
        .notify(
            "RepoHarbor",
            0,
            "repoharbor",
            summary,
            body,
            &[],
            HashMap::new(),
            -1,
        )
        .await
        .map_err(|e| e.to_string())
}

#[cfg(target_os = "macos")]
pub async fn send(summary: &str, body: &str) -> Result<u32, String> {
    let s = summary.replace('\\', "\\\\").replace('"', "\\\"");
    let b = body.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!("display notification \"{b}\" with title \"RepoHarbor\" subtitle \"{s}\"");
    let _ = std::process::Command::new("osascript")
        .args(["-e", &script])
        .output();
    Ok(0)
}

#[cfg(target_os = "windows")]
pub async fn send(summary: &str, body: &str) -> Result<u32, String> {
    let s = summary.replace('\'', "''");
    let b = body.replace('\'', "''");
    let script = format!(
        "Add-Type -AssemblyName System.Windows.Forms; \
         $n = New-Object System.Windows.Forms.NotifyIcon; \
         $n.Icon = [System.Drawing.SystemIcons]::Information; \
         $n.Visible = $true; \
         $n.ShowBalloonTip(4000, 'RepoHarbor - {s}', '{b}', [System.Windows.Forms.ToolTipIcon]::Info); \
         Start-Sleep -Milliseconds 500; \
         $n.Dispose()"
    );
    let _ = std::process::Command::new("powershell")
        .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &script])
        .spawn();
    Ok(0)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub async fn send(_summary: &str, _body: &str) -> Result<u32, String> {
    Ok(0)
}
