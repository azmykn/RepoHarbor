//! Lightweight redaction for text that may leave the machine (Log screenshots,
//! shared toasts, pasted diagnostics). Never a substitute for keeping
//! `~/.config/repoharbor` and `~/.local/share/repoharbor` out of git.

/// Replace the current user's home directory prefix with `~` so absolute
/// workspace paths are less identifying when the Log view or toasts are
/// captured. No-op when home cannot be resolved.
pub fn redact_user_paths(text: &str) -> String {
    let Some(home) = dirs::home_dir() else {
        return text.to_string();
    };
    let home = home.to_string_lossy();
    if home.is_empty() || !text.contains(home.as_ref()) {
        return text.to_string();
    }
    text.replace(home.as_ref(), "~")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_home_prefix_when_present() {
        let Some(home) = dirs::home_dir() else {
            return;
        };
        let home = home.to_string_lossy();
        let raw = format!("{home}/odoo/project/foo");
        assert_eq!(redact_user_paths(&raw), "~/odoo/project/foo");
        assert_eq!(
            redact_user_paths("already ~ relative"),
            "already ~ relative"
        );
    }
}
