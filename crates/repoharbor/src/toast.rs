//! Toast surface — the shared "something succeeded / failed / is running"
//! feedback channel, rendered as a stacked column in the shell's bottom-right
//! corner. Bulk ops, clone, the commit flow, and the attention engine all
//! report through here so long-running operations never fail silently.
//!
//! Lifecycle: Success/Info toasts auto-dismiss after a few seconds via a
//! background-executor timer; ids are unique per insert, so a stale timer from
//! a replaced toast is a no-op (the generation-guard pattern from the cleanup
//! confirm, with the id as the generation). Error toasts persist until
//! clicked. Progress toasts persist until the owning operation replaces them
//! through its stable caller-supplied key ([`RepoHarborApp::upsert_toast`]),
//! flipping "Cloning…" to done/failed in place.

use std::time::Duration;

use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, px, rgb,
};

use crate::icon::lucide;
use crate::shell::RepoHarborApp;
use crate::theme::Theme;

/// How many toasts render at once; the rest queue (oldest first) behind a
/// "+N more" line and surface as earlier ones dismiss.
const MAX_VISIBLE: usize = 5;

/// How long Success/Info toasts linger before auto-dismissing.
const AUTO_DISMISS: Duration = Duration::from_secs(4);

/// Toast column width (px) — comfortably narrower than the drawer so it never
/// competes with an open overlay.
const TOAST_W: f32 = 340.;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Success,
    Error,
    Info,
    Progress,
}

impl ToastKind {
    /// Success/Info are transient; Error waits for a click, Progress for its
    /// owning operation to resolve it.
    fn auto_dismisses(self) -> bool {
        matches!(self, ToastKind::Success | ToastKind::Info)
    }

    /// (lucide icon, semantic color) for the leading mark: ok/danger tokens
    /// for Success/Error, the accent for Info/Progress.
    fn style(self, t: &Theme) -> (&'static str, u32) {
        match self {
            ToastKind::Success => ("circle-check", t.clean),
            ToastKind::Error => ("circle-alert", t.behind),
            ToastKind::Info => ("bell", t.primary),
            ToastKind::Progress => ("refresh-cw", t.primary),
        }
    }
}

/// One toast. `id` is unique per insert (so a pending auto-dismiss timer armed
/// for a replaced toast can't kill its successor); `key` is the caller's
/// stable handle for updating an in-flight operation's toast (e.g.
/// `"clone:owner/repo"`).
pub struct Toast {
    pub id: u64,
    pub kind: ToastKind,
    pub title: SharedString,
    pub detail: Option<SharedString>,
    pub key: Option<SharedString>,
    /// A link the toast opens from the detail panel (e.g. a freshly created PR).
    pub url: Option<SharedString>,
    /// Activity-log id for the click-through detail panel (`None` on Progress ticks).
    pub log_id: Option<u64>,
}

impl RepoHarborApp {
    /// Show a toast. Success/Info auto-dismiss after [`AUTO_DISMISS`]; Error
    /// stays until clicked; Progress stays until the caller resolves it (use
    /// [`Self::upsert_toast`] for that). Returns the toast's id.
    pub fn push_toast(
        &mut self,
        kind: ToastKind,
        title: impl Into<SharedString>,
        detail: Option<SharedString>,
        cx: &mut Context<Self>,
    ) -> u64 {
        self.insert_toast(None, kind, title.into(), detail, None, None, cx)
    }

    /// Show or update the toast owned by `key` — the in-flight-operation
    /// channel: push a Progress toast when the op starts, then upsert the same
    /// key to Success/Error when it resolves. An update replaces the keyed
    /// toast in place (keeping its slot in the stack); if the toast was
    /// dismissed meanwhile, the resolution appears as a fresh toast.
    pub fn upsert_toast(
        &mut self,
        key: impl Into<SharedString>,
        kind: ToastKind,
        title: impl Into<SharedString>,
        detail: Option<SharedString>,
        cx: &mut Context<Self>,
    ) -> u64 {
        self.insert_toast(Some(key.into()), kind, title.into(), detail, None, None, cx)
    }

    /// Like [`Self::upsert_toast`] with structured click-through facts (fleet
    /// per-repo outcomes, a matched local checkout, …).
    pub fn upsert_toast_ctx(
        &mut self,
        key: impl Into<SharedString>,
        kind: ToastKind,
        title: impl Into<SharedString>,
        detail: Option<SharedString>,
        context: crate::activity_log::LogContext,
        cx: &mut Context<Self>,
    ) -> u64 {
        self.insert_toast(
            Some(key.into()),
            kind,
            title.into(),
            detail,
            None,
            Some(context),
            cx,
        )
    }

    /// Like [`Self::upsert_toast`] but the resolved toast is a link: clicking
    /// it opens `url` (then dismisses). Used by "PR opened" to jump to the PR.
    pub fn upsert_toast_link(
        &mut self,
        key: impl Into<SharedString>,
        kind: ToastKind,
        title: impl Into<SharedString>,
        detail: Option<SharedString>,
        url: SharedString,
        cx: &mut Context<Self>,
    ) -> u64 {
        self.insert_toast(
            Some(key.into()),
            kind,
            title.into(),
            detail,
            Some(url),
            None,
            cx,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_toast(
        &mut self,
        key: Option<SharedString>,
        kind: ToastKind,
        title: SharedString,
        detail: Option<SharedString>,
        url: Option<SharedString>,
        context: Option<crate::activity_log::LogContext>,
        cx: &mut Context<Self>,
    ) -> u64 {
        let progress_tick = is_progress_tick(kind, key.as_ref(), &self.toasts);
        let mut ctx = context.unwrap_or_default();
        if let Some(u) = url.as_ref()
            && ctx.url.is_none()
        {
            ctx.url = Some(u.clone());
        }
        if ctx.repo_id.is_none() {
            ctx.merge_repo_from(
                self.infer_log_context(title.as_ref(), detail.as_ref().map(|d| d.as_ref())),
            );
        }
        // Mirror every popup into the Log. Progress *ticks* (same-key updates
        // like "Pushing 3/10…") stay out so a 400-repo fleet doesn't fill the
        // ring; the first Progress for a key still records that the op started.
        let log_id = if progress_tick {
            None
        } else {
            Some(self.activity_log.record_toast(
                crate::data::now_unix(),
                kind,
                title.as_ref(),
                detail.as_ref().map(|d| d.as_ref()),
                ctx,
            ))
        };
        self.toast_seq += 1;
        let id = self.toast_seq;
        let toast = Toast {
            id,
            kind,
            title,
            detail,
            key: key.clone(),
            url,
            log_id,
        };
        let slot = key.and_then(|k| self.toasts.iter().position(|x| x.key.as_ref() == Some(&k)));
        match slot {
            Some(i) => self.toasts[i] = toast,
            None => self.toasts.push(toast),
        }
        if kind.auto_dismisses() {
            // Arm the auto-dismiss. Dismissing by this insert's unique id makes
            // the timer inherently stale-safe: if the toast was clicked away or
            // replaced (new id) first, the wake-up finds nothing to remove.
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(AUTO_DISMISS).await;
                let _ = this.update(cx, |this, cx| this.dismiss_toast(id, cx));
            })
            .detach();
        }
        cx.notify();
        id
    }

    /// Remove a toast by id (click, or the auto-dismiss timer). A no-op when
    /// it's already gone.
    pub fn dismiss_toast(&mut self, id: u64, cx: &mut Context<Self>) {
        let before = self.toasts.len();
        self.toasts.retain(|x| x.id != id);
        if self.toasts.len() != before {
            cx.notify();
        }
    }

    /// The stacked bottom-right toast column, layered over the active view but
    /// under the drawer/palette overlay (an open modal keeps the front).
    /// `None` when there's nothing to show, so the idle path costs nothing.
    pub fn toast_layer(&self, t: &Theme, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if self.toasts.is_empty() {
            return None;
        }
        let queued = self.toasts.len().saturating_sub(MAX_VISIBLE);
        let mut col = div()
            .absolute()
            .bottom(px(16.))
            .right(px(16.))
            .w(px(TOAST_W))
            .flex()
            .flex_col()
            .gap(px(8.))
            // Sized to content (not a full-screen layer), so only the toasts
            // themselves swallow clicks — the view stays interactive around them.
            .occlude();
        for toast in self.toasts.iter().take(MAX_VISIBLE) {
            col = col.child(toast_card(toast, t, cx));
        }
        if queued > 0 {
            col = col.child(
                div()
                    .px(px(12.))
                    .font_family("monospace")
                    .text_size(px(t.text_data_sm))
                    .text_color(rgb(t.fg3))
                    .child(SharedString::from(format!("+{queued} more"))),
            );
        }
        Some(col.into_any_element())
    }
}

/// One flat toast card: leading semantic icon, title + optional detail, on the
/// surface token with an elevation border. Clicking opens the notice detail
/// panel (full repo / branch / commit facts) and dismisses the toast.
fn toast_card(toast: &Toast, t: &Theme, cx: &mut Context<RepoHarborApp>) -> impl IntoElement {
    let (icon, color) = toast.kind.style(t);
    let id = toast.id;
    let hov = t.surface_hover;
    let mut text = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.))
        .gap(px(2.))
        .child(
            div()
                .font_weight(FontWeight::MEDIUM)
                .text_size(px(t.text_small))
                .text_color(rgb(t.fg0))
                .child(toast.title.clone()),
        );
    if let Some(detail) = &toast.detail {
        text = text.child(
            div()
                .text_size(px(t.text_data_sm))
                .text_color(rgb(t.fg2))
                .child(detail.clone()),
        );
    }
    let mut card = div()
        .id(SharedString::from(format!("toast-{id}")))
        .flex()
        .flex_row()
        .items_start()
        .gap(px(10.))
        .p(px(12.))
        .rounded(px(t.r_md))
        .bg(rgb(t.surface))
        .border_1()
        .border_color(rgb(t.border_strong))
        .cursor_pointer()
        .hover(move |s| s.bg(rgb(hov)))
        .on_click(cx.listener(move |this, _ev, _w, cx| {
            this.open_notice_from_toast(id, cx);
            this.dismiss_toast(id, cx);
        }))
        .child(lucide(icon, 16., color))
        .child(text);
    if toast.url.is_some() {
        card = card.child(lucide("external-link", 13., t.fg3));
    }
    card
}

/// True when this insert is a Progress update of an already-visible keyed
/// toast (e.g. "Pushing 3/10…") rather than the first Progress for that key.
fn is_progress_tick(kind: ToastKind, key: Option<&SharedString>, toasts: &[Toast]) -> bool {
    kind == ToastKind::Progress
        && key.is_some_and(|k| {
            toasts
                .iter()
                .any(|t| t.kind == ToastKind::Progress && t.key.as_ref() == Some(k))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toast(kind: ToastKind, key: &str) -> Toast {
        Toast {
            id: 1,
            kind,
            title: "t".into(),
            detail: None,
            key: Some(key.into()),
            url: None,
            log_id: None,
        }
    }

    #[test]
    fn first_progress_is_not_a_tick() {
        assert!(!is_progress_tick(
            ToastKind::Progress,
            Some(&SharedString::from("fleet:1")),
            &[]
        ));
    }

    #[test]
    fn later_progress_for_same_key_is_a_tick() {
        let existing = [toast(ToastKind::Progress, "fleet:1")];
        assert!(is_progress_tick(
            ToastKind::Progress,
            Some(&SharedString::from("fleet:1")),
            &existing
        ));
        assert!(!is_progress_tick(
            ToastKind::Progress,
            Some(&SharedString::from("fleet:2")),
            &existing
        ));
        assert!(!is_progress_tick(
            ToastKind::Success,
            Some(&SharedString::from("fleet:1")),
            &existing
        ));
    }
}
