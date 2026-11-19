//! Multi-repo Gen & push confirm — centered occlude modal (same chrome as the
//! notice panel). Armed only when N>1 dirty targets; Confirm runs
//! [`crate::fleet::FleetOp::GenerateCommitAndPush`]. Esc / backdrop / Cancel
//! dismiss without running.

use gpui::{
    Entity, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, px, rgb, rgba,
};

use crate::icon::lucide;
use crate::shell::RepoHarborApp;
use crate::theme::Theme;

/// Soft cap on named rows in the scroll body; beyond this, a "+N more" footer.
const MAX_NAMES_SHOWN: usize = 24;

pub fn render(
    repos: &[String],
    t: &Theme,
    app: &Entity<RepoHarborApp>,
    names: &[SharedString],
) -> impl IntoElement {
    let n = repos.len();
    let app_bg = app.clone();
    let app_cancel = app.clone();
    let app_confirm = app.clone();

    let mut list = div()
        .id("fleet-gen-push-list")
        .flex()
        .flex_col()
        .gap(px(4.))
        .w_full()
        .max_h(px(280.))
        .overflow_y_scroll()
        .px(px(2.));
    let shown = names.len().min(MAX_NAMES_SHOWN);
    for name in names.iter().take(shown) {
        list = list.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.))
                .px(px(8.))
                .py(px(4.))
                .rounded(px(t.r_xs))
                .bg(rgb(t.page))
                .child(lucide("folder-git-2", 13., t.fg3))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .font_family("monospace")
                        .text_size(px(t.text_data_sm))
                        .text_color(rgb(t.fg1))
                        .child(name.clone()),
                ),
        );
    }
    if names.len() > MAX_NAMES_SHOWN {
        list = list.child(
            div()
                .px(px(8.))
                .py(px(4.))
                .font_family("monospace")
                .text_size(px(t.text_data_sm))
                .text_color(rgb(t.fg3))
                .child(SharedString::from(format!(
                    "+{} more",
                    names.len() - MAX_NAMES_SHOWN
                ))),
        );
    }

    let panel = div()
        .id("fleet-gen-push-panel")
        .flex()
        .flex_col()
        .gap(px(14.))
        .w(px(440.))
        .p(px(18.))
        .rounded(px(t.r_md))
        .bg(rgb(t.surface))
        .border_1()
        .border_color(rgb(t.border_strong))
        .occlude()
        .on_click(|_ev, _win, _cx| {})
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.))
                .child(lucide("sparkles", 16., t.accent_bright))
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(t.text_h3))
                        .text_color(rgb(t.fg0))
                        .child(SharedString::from(format!("Gen & push {n} repos"))),
                ),
        )
        .child(
            div()
                .text_size(px(t.text_small))
                .text_color(rgb(t.fg1))
                .child(
                    "AI drafts a commit message per repo, commits all changes, then pushes. There is no per-repo message review for multi-repo runs.",
                ),
        )
        .child(list)
        .child(
            div()
                .flex()
                .flex_row()
                .gap(px(8.))
                .justify_end()
                .child(action_btn("Cancel", false, t, move |_w, cx| {
                    app_cancel.update(cx, |this, cx| this.cancel_fleet_gen_push(cx));
                }))
                .child(action_btn("Confirm Gen & push", true, t, move |_w, cx| {
                    app_confirm.update(cx, |this, cx| this.confirm_fleet_gen_push(cx));
                })),
        );

    div()
        .id("fleet-gen-push-backdrop")
        .absolute()
        .inset_0()
        .occlude()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgba(0x00000088))
        .on_click(move |_ev, _win, cx| {
            app_bg.update(cx, |this, cx| this.cancel_fleet_gen_push(cx));
        })
        .child(panel)
}

fn action_btn(
    label: &str,
    primary: bool,
    t: &Theme,
    on: impl Fn(&mut gpui::Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let (bg, border, fg, hov_bg) = if primary {
        (t.accent_wash, t.border_accent, t.fg0, t.surface_hover)
    } else {
        (t.button_bg, t.border, t.fg1, t.surface_hover)
    };
    div()
        .id(SharedString::from(format!("fleet-gen-push-{label}")))
        .px(px(12.))
        .py(px(7.))
        .rounded(px(t.r_sm))
        .bg(rgb(bg))
        .border_1()
        .border_color(rgb(border))
        .text_size(px(t.text_data_sm))
        .text_color(rgb(fg))
        .cursor_pointer()
        .hover(move |s| s.bg(rgb(hov_bg)).text_color(rgb(t.fg0)))
        .child(SharedString::from(label.to_string()))
        .on_click(move |_ev, window, cx| on(window, cx))
}

/// Display names for the confirm list (grid `name`, else path tail).
pub fn repo_names(app: &RepoHarborApp, repos: &[String]) -> Vec<SharedString> {
    repos
        .iter()
        .map(|id| {
            app.rows
                .iter()
                .find(|r| r.id.as_ref() == id.as_str())
                .map(|r| r.name.clone())
                .unwrap_or_else(|| {
                    SharedString::from(id.rsplit('/').next().unwrap_or(id).to_string())
                })
        })
        .collect()
}
