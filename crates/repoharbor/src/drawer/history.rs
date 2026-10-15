//! History tab: a flat lane graph of recent commits, with blame for a file
//! touched by the selected commit.

use gpui::{
    Entity, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, px, rgb,
};

use crate::shell::RepoHarborApp;
use crate::theme::Theme;

use super::{BlameState, DrawerData, MONO, load_blame, load_history_files, placeholder, section};

const LANE_COLORS: [fn(&Theme) -> u32; 6] = [
    |t| t.primary,
    |t| t.act_ide,
    |t| t.ai,
    |t| t.dirty,
    |t| t.star,
    |t| t.clean,
];

pub(super) fn view(data: &DrawerData, t: &Theme, app: &Entity<RepoHarborApp>) -> impl IntoElement {
    let mut col = div().flex().flex_col().gap(px(12.));
    col = col.child(graph(data, t, app));
    if data.blame_file.is_some() {
        col = col.child(super::work::blame_pane(data, t));
    }
    col
}

fn graph(data: &DrawerData, t: &Theme, app: &Entity<RepoHarborApp>) -> impl IntoElement {
    let mut s = section(t, "History", data.history.as_ref().map(|rows| rows.len()));
    match &data.history {
        None => s = s.child(placeholder("Loading…", t)),
        Some(list) if list.is_empty() => s = s.child(placeholder("No commits.", t)),
        Some(list) => {
            for (ix, commit) in list.iter().enumerate() {
                let selected = data.history_sel == Some(ix);
                let color = LANE_COLORS[commit.lane as usize % LANE_COLORS.len()](t);
                let (app, repo, id) = (app.clone(), data.repo.clone(), commit.id.clone());
                let mut row = div()
                    .id(SharedString::from(format!("hist-{ix}")))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .px(px(6.))
                    .py(px(4.))
                    .rounded(px(t.r_sm))
                    .ml(px(commit.lane as f32 * 14.))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(t.surface_hover)))
                    .on_click(move |_ev, _win, cx| {
                        let (repo, id) = (repo.clone(), id.clone());
                        app.update(cx, |this, cx| {
                            this.drawer.history_sel = Some(ix);
                            this.drawer.history_files = None;
                            this.drawer.blame = BlameState::Idle;
                            this.drawer.blame_file = None;
                            load_history_files(repo, id, cx);
                            cx.notify();
                        });
                    })
                    .child(
                        div()
                            .w(px(8.))
                            .h(px(8.))
                            .rounded(px(4.))
                            .bg(rgb(color))
                            .flex_shrink_0(),
                    )
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
                                    .text_color(rgb(if selected { t.fg0 } else { t.fg1 }))
                                    .child(commit.summary.clone()),
                            )
                            .child(
                                div()
                                    .font_family(MONO)
                                    .text_size(px(t.text_data_sm))
                                    .text_color(rgb(t.fg3))
                                    .child(SharedString::from(format!(
                                        "{} · {} · {}",
                                        commit.id, commit.author, commit.age
                                    ))),
                            ),
                    );
                if commit.parents.len() > 1 {
                    row = row.child(
                        div()
                            .font_family(MONO)
                            .text_size(px(t.text_data_sm))
                            .text_color(rgb(t.fg3))
                            .child(SharedString::from(format!(
                                "merge {}",
                                commit
                                    .parents
                                    .iter()
                                    .map(|p| p.as_ref())
                                    .collect::<Vec<_>>()
                                    .join(" ")
                            ))),
                    );
                }
                s = s.child(row);
            }
        }
    }
    if data.history_sel.is_some() {
        s = s.child(files(data, t, app));
    }
    s
}

fn files(data: &DrawerData, t: &Theme, app: &Entity<RepoHarborApp>) -> impl IntoElement {
    let mut block = div().flex().flex_col().gap(px(4.)).mt(px(8.));
    match &data.history_files {
        None => block = block.child(placeholder("Loading files…", t)),
        Some(list) if list.is_empty() => {
            block = block.child(placeholder("No files in this commit.", t));
        }
        Some(list) => {
            let commit_id = data
                .history_sel
                .and_then(|ix| data.history.as_ref().and_then(|rows| rows.get(ix)))
                .map(|c| c.id.to_string())
                .unwrap_or_default();
            for file in list {
                let (app, repo, path, newest) = (
                    app.clone(),
                    data.repo.clone(),
                    file.clone(),
                    commit_id.clone(),
                );
                block = block.child(
                    div()
                        .id(SharedString::from(format!("hist-file-{file}")))
                        .font_family(MONO)
                        .text_size(px(t.text_data_sm))
                        .text_color(rgb(t.fg1))
                        .px(px(6.))
                        .py(px(3.))
                        .rounded(px(t.r_xs))
                        .cursor_pointer()
                        .hover(|style| style.bg(rgb(t.surface_hover)))
                        .child(file.clone())
                        .on_click(move |_ev, _win, cx| {
                            let (repo, path, newest) = (repo.clone(), path.clone(), newest.clone());
                            app.update(cx, |this, cx| {
                                this.drawer.blame_file = Some(path.clone());
                                this.drawer.blame = BlameState::Loading;
                                load_blame(repo, path, Some(newest), cx);
                                cx.notify();
                            });
                        }),
                );
            }
        }
    }
    block
}
