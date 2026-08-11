//! Generate-commit choice dialog — opened from every "Generate…" entry point
//! (drawer Changes, card/TREE context menu, fleet Actions). Two outcomes:
//! message only, or generate + commit + push.

use gpui::{
    Entity, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, px, rgb, rgba,
};

use crate::icon::lucide;
use crate::shell::RepoHarborApp;
use crate::theme::Theme;

/// What the modal should do once the user picks an option.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GenerateCommitChoice {
    /// Fill the commit composer (or report the message) without committing.
    MessageOnly,
    /// AI message → `commit_all` → `push` (push skipped on pull-only).
    CommitAndPush,
}

/// Pending generate-commit dialog (layered above the drawer when present).
#[derive(Clone)]
pub struct GenerateCommitPrompt {
    pub repos: Vec<String>,
    /// True when opened from the drawer Changes tab (message-only stays there).
    pub from_drawer: bool,
}

pub fn render(
    prompt: &GenerateCommitPrompt,
    t: &Theme,
    app: &Entity<RepoHarborApp>,
) -> impl IntoElement {
    let n = prompt.repos.len();
    let scope = if n == 1 {
        prompt
            .repos
            .first()
            .and_then(|p| p.rsplit('/').next())
            .unwrap_or("repo")
            .to_string()
    } else {
        format!("{n} repos")
    };

    let app_bg = app.clone();
    let panel = div()
        .id("gen-commit-panel")
        .occlude()
        .w(px(440.))
        .flex()
        .flex_col()
        .gap(px(14.))
        .p(px(20.))
        .rounded(px(t.r_lg))
        .bg(rgb(t.surface))
        .border_1()
        .border_color(rgb(t.border))
        .on_click(|_ev, _win, _cx| {})
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.))
                .child(lucide("sparkles", 18., t.accent_bright))
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(t.text_h3))
                        .text_color(rgb(t.fg0))
                        .child("Generate commit"),
                ),
        )
        .child(
            div()
                .text_size(px(t.text_small))
                .text_color(rgb(t.fg2))
                .child(SharedString::from(format!(
                    "AI will draft a message for {scope}. Choose how far to go:"
                ))),
        )
        .child(choice_btn(
            "gen-only",
            "Generate only",
            "Fill the commit message — you review and commit yourself.",
            false,
            t,
            app,
            GenerateCommitChoice::MessageOnly,
        ))
        .child(choice_btn(
            "gen-push",
            "Generate, commit & push",
            "Commit all changes with the AI message, then push (skipped on pull-only).",
            true,
            t,
            app,
            GenerateCommitChoice::CommitAndPush,
        ))
        .child(
            div()
                .flex()
                .flex_row()
                .justify_end()
                .child(cancel_btn(t, app)),
        );

    div()
        .id("gen-commit-backdrop")
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgba(0x00000088))
        .on_click(move |_ev, _win, cx| {
            app_bg.update(cx, |this, cx| this.cancel_generate_commit_prompt(cx));
        })
        .child(panel)
}

fn choice_btn(
    id: &'static str,
    title: &str,
    detail: &str,
    primary: bool,
    t: &Theme,
    app: &Entity<RepoHarborApp>,
    choice: GenerateCommitChoice,
) -> impl IntoElement {
    let app = app.clone();
    let title = SharedString::from(title.to_string());
    let detail = SharedString::from(detail.to_string());
    let bg = if primary { t.primary } else { t.button_bg };
    let fg = if primary { t.page } else { t.fg1 };
    let border = if primary { t.primary } else { t.border };
    let hov = t.border_strong;
    div()
        .id(SharedString::from(id))
        .flex()
        .flex_col()
        .gap(px(4.))
        .px(px(14.))
        .py(px(12.))
        .rounded(px(t.r_sm))
        .bg(rgb(bg))
        .border_1()
        .border_color(rgb(border))
        .cursor_pointer()
        .hover(move |s| if primary { s } else { s.border_color(rgb(hov)) })
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(t.text_small))
                .text_color(rgb(fg))
                .child(title),
        )
        .child(
            div()
                .text_size(px(t.text_data_sm))
                .text_color(rgb(if primary { t.page } else { t.fg3 }))
                .opacity(if primary { 0.85 } else { 1. })
                .child(detail),
        )
        .on_click(move |_ev, window, cx| {
            app.update(cx, |this, cx| {
                this.confirm_generate_commit(choice, window, cx);
            });
        })
}

fn cancel_btn(t: &Theme, app: &Entity<RepoHarborApp>) -> impl IntoElement {
    let app = app.clone();
    let (hb, hf) = (t.border_strong, t.fg0);
    div()
        .id("gen-commit-cancel")
        .px(px(14.))
        .py(px(7.))
        .rounded(px(t.r_sm))
        .bg(rgb(t.button_bg))
        .border_1()
        .border_color(rgb(t.border))
        .text_size(px(t.text_data_sm))
        .text_color(rgb(t.fg1))
        .cursor_pointer()
        .hover(move |s| s.border_color(rgb(hb)).text_color(rgb(hf)))
        .child("Cancel")
        .on_click(move |_ev, _win, cx| {
            app.update(cx, |this, cx| this.cancel_generate_commit_prompt(cx));
        })
}
