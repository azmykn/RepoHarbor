//! Work tab: the existing Changes composer, plus stash, conflict paths, and
//! blame for the selected file.

use gpui::{App, Entity, IntoElement, ParentElement, SharedString, Styled, div, px, rgb};

use crate::icon::lucide;
use crate::shell::RepoHarborApp;
use crate::theme::Theme;

use super::{
    BlameState, DrawerData, MONO, load_blame, placeholder, pr_btn, section, stash_op, stash_save,
};

pub(super) fn view(
    row: &crate::data::Row,
    data: &DrawerData,
    t: &Theme,
    app: &Entity<RepoHarborApp>,
    ai_ready: bool,
    github_authed: bool,
) -> impl IntoElement {
    let mut col = div().flex().flex_col().gap(px(16.));
    if data
        .conflicts
        .as_ref()
        .is_some_and(|paths| !paths.is_empty())
    {
        col = col.child(conflict_banner(data, t, app));
    }
    col = col.child(super::changes_view(
        row,
        data,
        t,
        app,
        ai_ready,
        github_authed,
    ));
    col = col.child(stash_section(row, data, t, app));
    if let Some((path, _)) = &data.change_sel {
        let (app, repo, file) = (app.clone(), data.repo.clone(), path.clone());
        let (app_diff, repo_diff, file_diff) = (app.clone(), repo.clone(), file.clone());
        col = col.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.))
                .child(pr_btn(
                    SharedString::from("blame-file"),
                    "Blame file",
                    t,
                    move |_win, cx: &mut App| {
                        let (repo, file) = (repo.clone(), file.clone());
                        app.update(cx, |this, cx| {
                            this.drawer.blame_file = Some(file.clone());
                            this.drawer.blame = BlameState::Loading;
                            load_blame(repo, file, None, cx);
                            cx.notify();
                        });
                    },
                ))
                .child(pr_btn(
                    SharedString::from("diff-file"),
                    "Open in Diff tool",
                    t,
                    move |_win, cx: &mut App| {
                        let (repo, file) = (repo_diff.clone(), file_diff.clone());
                        app_diff.update(cx, |this, cx| {
                            this.open_external_diff(&repo, Some(&file), cx);
                        });
                    },
                )),
        );
    }
    if data.blame_file.is_some() {
        col = col.child(blame_pane(data, t));
    }
    col
}

fn conflict_banner(data: &DrawerData, t: &Theme, app: &Entity<RepoHarborApp>) -> impl IntoElement {
    let Some(paths) = &data.conflicts else {
        return div();
    };
    if paths.is_empty() {
        return div();
    }
    let repo = data.repo.clone();
    let first = paths.first().map(|p| p.to_string()).unwrap_or_default();
    let app_b = app.clone();
    let app_ide = app.clone();
    let app_abort = app.clone();
    let repo_ide = repo.clone();
    let repo_abort = repo.clone();
    let mut body = div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .p(px(11.))
        .rounded(px(t.r_sm))
        .bg(rgb(t.danger_badge))
        .border_1()
        .border_color(rgb(t.behind))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.))
                .text_size(px(t.text_small))
                .text_color(rgb(t.behind))
                .child(lucide("circle-alert", 14., t.behind))
                .child(SharedString::from(format!(
                    "{} merge conflict{}",
                    paths.len(),
                    if paths.len() == 1 { "" } else { "s" }
                ))),
        );
    for path in paths {
        body = body.child(
            div()
                .font_family(MONO)
                .text_size(px(t.text_data_sm))
                .text_color(rgb(t.fg1))
                .child(path.clone()),
        );
    }
    body.child(
        div()
            .flex()
            .flex_row()
            .gap(px(6.))
            .child(pr_btn(
                SharedString::from("conflict-diff"),
                "Open in diff tool",
                t,
                move |_win, cx: &mut App| {
                    let (repo, file) = (repo.clone(), first.clone());
                    app_b.update(cx, |this, cx| {
                        this.open_external_diff(
                            &repo,
                            if file.is_empty() {
                                None
                            } else {
                                Some(file.as_str())
                            },
                            cx,
                        );
                    });
                },
            ))
            .child(pr_btn(
                SharedString::from("conflict-ide"),
                "Open in IDE",
                t,
                move |_win, cx: &mut App| {
                    let repo = repo_ide.clone();
                    app_ide.update(cx, |this, cx| this.open_in_ide(&repo, cx));
                },
            ))
            .child(pr_btn(
                SharedString::from("conflict-abort"),
                "Abort merge",
                t,
                move |_win, cx: &mut App| {
                    let repo = repo_abort.clone();
                    app_abort.update(cx, |_this, cx| {
                        super::abort_merge(repo, cx);
                    });
                },
            )),
    )
}

fn stash_section(
    row: &crate::data::Row,
    data: &DrawerData,
    t: &Theme,
    app: &Entity<RepoHarborApp>,
) -> impl IntoElement {
    let mut s = section(t, "Stash", data.stash.as_ref().map(|rows| rows.len()));
    if row.dirty > 0 {
        let (app_s, repo_s) = (app.clone(), data.repo.clone());
        s = s.child(pr_btn(
            SharedString::from("stash-save"),
            "Stash changes",
            t,
            move |_win, cx: &mut App| {
                let repo = repo_s.clone();
                app_s.update(cx, |_this, cx| stash_save(repo, cx));
            },
        ));
    }
    match &data.stash {
        None => s = s.child(placeholder("Loading…", t)),
        Some(list) if list.is_empty() => s = s.child(placeholder("Nothing stashed.", t)),
        Some(list) => {
            for entry in list {
                let (app_a, app_d) = (app.clone(), app.clone());
                let repo_a = data.repo.clone();
                let repo_d = data.repo.clone();
                let index = entry.index;
                s = s.child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.))
                        .py(px(4.))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(px(t.text_small))
                                        .text_color(rgb(t.fg1))
                                        .child(entry.message.clone()),
                                )
                                .child(
                                    div()
                                        .font_family(MONO)
                                        .text_size(px(t.text_data_sm))
                                        .text_color(rgb(t.fg3))
                                        .child(entry.id.clone()),
                                ),
                        )
                        .child(pr_btn(
                            SharedString::from(format!("stash-apply-{index}")),
                            "Apply",
                            t,
                            move |_win, cx: &mut App| {
                                let repo = repo_a.clone();
                                app_a.update(cx, |_this, cx| stash_op(repo, index, true, cx));
                            },
                        ))
                        .child(pr_btn(
                            SharedString::from(format!("stash-drop-{index}")),
                            "Drop",
                            t,
                            move |_win, cx: &mut App| {
                                let repo = repo_d.clone();
                                app_d.update(cx, |_this, cx| stash_op(repo, index, false, cx));
                            },
                        )),
                );
            }
        }
    }
    s
}

pub(super) fn blame_pane(data: &DrawerData, t: &Theme) -> impl IntoElement {
    let Some(file) = &data.blame_file else {
        return div();
    };
    let mut s = section(t, "Blame", None);
    s = s.child(
        div()
            .font_family(MONO)
            .text_size(px(t.text_data_sm))
            .text_color(rgb(t.fg2))
            .child(file.clone()),
    );
    match &data.blame {
        BlameState::Idle | BlameState::Loading => s = s.child(placeholder("Loading…", t)),
        BlameState::Failed(err) => s = s.child(placeholder(err.clone(), t)),
        BlameState::Ready {
            lines,
            truncated: _,
        } if lines.is_empty() => {
            s = s.child(placeholder("No blame lines.", t));
        }
        BlameState::Ready { lines, truncated } => {
            if *truncated {
                s = s.child(placeholder(
                    format!("Showing the first {} lines.", lines.len()),
                    t,
                ));
            }
            for line in lines {
                let who = if line.id.is_empty() {
                    line.author.to_string()
                } else {
                    format!("{} {}", line.id, line.author)
                };
                s = s.child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.))
                        .py(px(1.))
                        .child(
                            div()
                                .w(px(108.))
                                .flex_shrink_0()
                                .truncate()
                                .font_family(MONO)
                                .text_size(px(t.text_data_sm))
                                .text_color(rgb(t.fg3))
                                .child(SharedString::from(format!("L{} {who}", line.number))),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .truncate()
                                .font_family(MONO)
                                .text_size(px(t.text_data_sm))
                                .text_color(rgb(t.fg1))
                                .child(line.text.clone()),
                        ),
                );
            }
        }
    }
    s
}
