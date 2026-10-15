//! About tab: repo facts, branches, worktrees, pull requests, notes, and the
//! README on one scroll.

use gpui::{Entity, IntoElement, ParentElement, Styled, div, px};

use crate::data::Row;
use crate::shell::RepoHarborApp;
use crate::theme::Theme;

use super::DrawerData;

pub(super) fn view(
    row: &Row,
    data: &DrawerData,
    t: &Theme,
    app: &Entity<RepoHarborApp>,
    ai_ready: bool,
) -> impl IntoElement {
    let mut col = div()
        .flex()
        .flex_col()
        .gap(px(22.))
        .child(super::overview(row, t, data, app));
    if row.host.as_ref() == "github" {
        col = col.child(super::pr_view(row, data, t, app));
    }
    col.child(super::notes_view(row, data, t, app, ai_ready))
        .child(super::readme_view(data, t))
}
