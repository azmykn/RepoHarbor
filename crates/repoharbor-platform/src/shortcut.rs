//! Global shortcut to summon the app (XDG **GlobalShortcuts** portal).
//!
//! **Registration is disabled.** `bind_shortcuts` pops a desktop permission
//! dialog on first launch (Wayland portals) — "allow add shortcut" — which is
//! disruptive every cold start. Desktop pin / `.desktop` install stays
//! opt-in via `scripts/install-desktop.sh`; users can also bind a raise
//! hotkey in their desktop's global-shortcuts settings if they want one.
//!
//! [`spawn`] remains as a stable hook for a future Settings opt-in; until
//! then it is a documented no-op so `live.rs` does not need a compile gate.

/// Would-be stable id if portal registration is re-enabled.
#[allow(dead_code)]
const SHORTCUT_ID: &str = "activate";
/// Preferred accelerator hint (portal syntax) if registration is re-enabled.
#[allow(dead_code)]
const PREFERRED_TRIGGER: &str = "CTRL+ALT+o";

/// No-op: do not call the GlobalShortcuts portal (avoids the add-shortcut
/// dialog). `on_activate` is unused while registration is off.
pub fn spawn(_on_activate: impl Fn() + Send + 'static) {
    // Intentionally empty — see module docs.
}
