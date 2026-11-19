//! Notice detail overlay — opened by clicking a toast or a Log row. Shows the
//! full message plus repo / branch / commit / fleet facts so a clipped toast
//! never hides the useful data.

use chrono::{DateTime, Local};
use gpui::{
    Entity, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px, rgb, rgba,
};

use crate::activity_log::{LogEntry, LogLevel};
use crate::icon::lucide;
use crate::shell::RepoHarborApp;
use crate::theme::Theme;

pub fn render(entry: &LogEntry, t: &Theme, app: &Entity<RepoHarborApp>) -> impl IntoElement {
    let (icon, color) = match entry.level {
        LogLevel::Info => ("bell", t.fg2),
        LogLevel::Warn => ("circle-alert", t.dirty),
        LogLevel::Error => ("circle-alert", t.behind),
    };
    let ctx = &entry.context;
    let app_bg = app.clone();
    let mut body = div()
        .id("notice-body")
        .flex()
        .flex_col()
        .gap(px(10.))
        .w(px(520.))
        .max_h(px(390.))
        .overflow_y_scroll()
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.))
                .child(lucide(icon, 16., color))
                .child(super::tag(entry.level.label(), color, t))
                .child(
                    div()
                        .font_family("monospace")
                        .text_size(px(t.text_data_sm))
                        .text_color(rgb(t.fg3))
                        .child(SharedString::from(format_datetime(entry.at))),
                ),
        )
        .child(
            div()
                .text_size(px(t.text_small))
                .text_color(rgb(t.fg0))
                .child(SharedString::from(crate::data::oneline(
                    entry.message.to_string(),
                ))),
        );

    body = body.child(kv("Repository", ctx.repo_name.as_ref(), t));
    body = body.child(kv("Slug", ctx.slug.as_ref(), t));
    body = body.child(kv("Path", ctx.path.as_ref(), t));
    body = body.child(kv("Branch", ctx.branch.as_ref(), t));
    let commit = match (&ctx.commit, &ctx.commit_subject) {
        (Some(h), Some(s)) if !s.is_empty() => Some(SharedString::from(format!("{h}  {s}"))),
        (Some(h), _) => Some(h.clone()),
        _ => None,
    };
    body = body.child(kv("Commit", commit.as_ref(), t));
    let git = git_line(ctx.ahead, ctx.behind, ctx.dirty);
    body = body.child(kv("Git", git.as_ref(), t));
    body = body.child(kv("Host", ctx.host.as_ref(), t));
    body = body.child(kv("URL", ctx.url.as_ref(), t));

    if !ctx.facts.is_empty() {
        let mut facts = div().flex().flex_col().gap(px(4.)).mt(px(4.));
        facts = facts.child(
            div()
                .font_weight(FontWeight::MEDIUM)
                .text_size(px(t.text_data_sm))
                .text_color(rgb(t.fg2))
                .child("Details"),
        );
        for (k, v) in &ctx.facts {
            facts = facts.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .child(
                        div()
                            .font_family("monospace")
                            .text_size(px(t.text_data_sm))
                            .text_color(rgb(t.fg3))
                            .child(k.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(t.text_small))
                            .text_color(rgb(t.fg1))
                            .child(SharedString::from(crate::data::oneline(v.to_string()))),
                    ),
            );
        }
        body = body.child(facts);
    }

    let term_cwd = ctx.path.as_ref().or(ctx.repo_id.as_ref()).cloned();
    let mut combined = entry.message.to_string();
    for (k, v) in &ctx.facts {
        combined.push('\n');
        combined.push_str(k.as_ref());
        combined.push(' ');
        combined.push_str(v.as_ref());
    }
    let cmds = repoharbor_core::terminal::extract_commands_from_text(&combined);
    let mut cmd_box_opt = None;
    if !cmds.is_empty() {
        let fallback_cwd = std::env::var("HOME")
            .ok()
            .filter(|h| !h.is_empty())
            .unwrap_or_else(|| "/".to_string());
        let cwd_base = term_cwd
            .as_ref()
            .map(|s| s.to_string())
            .unwrap_or(fallback_cwd);
        let title_base = ctx
            .repo_name
            .as_ref()
            .map(|s| s.to_string())
            .unwrap_or_else(|| crate::shell::repo_tab_title(&cwd_base));
        let mut cmd_box = div()
            .w(px(520.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .pt(px(4.))
            .border_t_1()
            .border_color(rgb(t.border))
            .child(
                div()
                    .font_weight(FontWeight::MEDIUM)
                    .text_size(px(t.text_data_sm))
                    .text_color(rgb(t.fg2))
                    .child("Run in built-in terminal"),
            );
        for (i, cmd) in cmds.into_iter().take(4).enumerate() {
            let (app_r, app_p) = (app.clone(), app.clone());
            let (cwd_r, title_r, raw_r) =
                (cwd_base.clone(), title_base.clone(), cmd.command.clone());
            let (cwd_p, title_p, raw_p) =
                (cwd_base.clone(), title_base.clone(), cmd.command.clone());
            let border_c = if cmd.dangerous { t.dirty } else { t.border };
            let fg_c = if cmd.dangerous { t.dirty } else { t.fg1 };
            let hover_c = t.border_strong;
            cmd_box = cmd_box.child(
                div()
                    .w_full()
                    .overflow_hidden()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap(px(8.))
                    .px(px(8.))
                    .py(px(5.))
                    .rounded(px(t.r_xs))
                    .bg(rgb(t.page))
                    .border_1()
                    .border_color(rgb(border_c))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .font_family("monospace")
                            .text_size(px(t.text_data_sm))
                            .text_color(rgb(fg_c))
                            .child(SharedString::from(cmd.command)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .flex_shrink_0()
                            .gap(px(4.))
                            .child(
                                div()
                                    .id(SharedString::from(format!("notice-run-{i}")))
                                    .px(px(7.))
                                    .py(px(2.))
                                    .rounded(px(t.r_xs))
                                    .bg(rgb(t.button_bg))
                                    .border_1()
                                    .border_color(rgb(border_c))
                                    .cursor_pointer()
                                    .hover(move |s| s.border_color(rgb(hover_c)))
                                    .text_size(px(t.text_data_sm))
                                    .text_color(rgb(if cmd.dangerous {
                                        t.dirty
                                    } else {
                                        t.accent_bright
                                    }))
                                    .child("▶ Run")
                                    .on_click(move |_ev, win, cx| {
                                        app_r.update(cx, |this, cx| {
                                            this.close_notice(cx);
                                            this.open_terminal_in(
                                                &cwd_r,
                                                &title_r,
                                                Some(&raw_r),
                                                true,
                                                Some(win),
                                                cx,
                                            );
                                        });
                                    }),
                            )
                            .child(
                                div()
                                    .id(SharedString::from(format!("notice-paste-{i}")))
                                    .px(px(6.))
                                    .py(px(2.))
                                    .rounded(px(t.r_xs))
                                    .bg(rgb(t.button_bg))
                                    .border_1()
                                    .border_color(rgb(t.border))
                                    .cursor_pointer()
                                    .hover(move |s| s.border_color(rgb(hover_c)))
                                    .text_size(px(t.text_data_sm))
                                    .text_color(rgb(t.fg2))
                                    .child("Paste")
                                    .on_click(move |_ev, win, cx| {
                                        app_p.update(cx, |this, cx| {
                                            this.close_notice(cx);
                                            this.open_terminal_in(
                                                &cwd_p,
                                                &title_p,
                                                Some(&raw_p),
                                                false,
                                                Some(win),
                                                cx,
                                            );
                                        });
                                    }),
                            ),
                    ),
            );
        }
        cmd_box_opt = Some(cmd_box);
    }

    let mut actions = div()
        .flex()
        .flex_row()
        .flex_wrap()
        .gap(px(8.))
        .justify_end();
    if let Some(id) = ctx.repo_id.clone() {
        let app_drawer = app.clone();
        let id_drawer = id.clone();
        actions = actions.child(action_btn("Open drawer", t, move |window, cx| {
            app_drawer.update(cx, |this, cx| {
                this.close_notice(cx);
                this.open_drawer(id_drawer.clone(), window, cx);
            });
        }));
        let app_ide = app.clone();
        let path = id;
        actions = actions.child(action_btn("Open in IDE", t, move |_w, cx| {
            app_ide.update(cx, |this, _cx| {
                let _ = repoharbor_core::launch::launch(&this.config.ide_command, path.as_ref());
            });
        }));
    }
    if let Some(term_path) = term_cwd {
        let app_term = app.clone();
        let term_title = ctx
            .repo_name
            .clone()
            .unwrap_or_else(|| crate::shell::repo_tab_title(term_path.as_ref()).into());
        actions = actions.child(action_btn("Open terminal", t, move |win, cx| {
            app_term.update(cx, |this, cx| {
                this.close_notice(cx);
                this.open_terminal_in(
                    term_path.as_ref(),
                    term_title.as_ref(),
                    None,
                    false,
                    Some(win),
                    cx,
                );
            });
        }));
    }
    if let Some(url) = ctx.url.clone().filter(|u| !u.is_empty()) {
        actions = actions.child(action_btn("Open link", t, move |_w, _cx| {
            let _ = repoharbor_core::launch::open(url.as_ref());
        }));
    }
    let app_close = app.clone();
    actions = actions.child(action_btn("Close", t, move |_w, cx| {
        app_close.update(cx, |this, cx| this.close_notice(cx));
    }));

    let mut panel = div()
        .id("notice-panel")
        .flex()
        .flex_col()
        .gap(px(14.))
        .p(px(18.))
        .rounded(px(t.r_md))
        .bg(rgb(t.surface))
        .border_1()
        .border_color(rgb(t.border_strong))
        .occlude()
        .on_click(|_ev, _win, _cx| {})
        .child(body);
    if let Some(cmd_box) = cmd_box_opt {
        panel = panel.child(cmd_box);
    }
    let panel = panel.child(actions);

    div()
        .id("notice-backdrop")
        .absolute()
        .inset_0()
        .occlude()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgba(0x00000088))
        .on_click(move |_ev, _win, cx| {
            app_bg.update(cx, |this, cx| this.close_notice(cx));
        })
        .child(panel)
}

fn git_line(ahead: Option<u32>, behind: Option<u32>, dirty: Option<u32>) -> Option<SharedString> {
    if ahead.is_none() && behind.is_none() && dirty.is_none() {
        return None;
    }
    Some(SharedString::from(format!(
        "{} ahead · {} behind · {} dirty",
        ahead.unwrap_or(0),
        behind.unwrap_or(0),
        dirty.unwrap_or(0)
    )))
}

fn kv(label: &str, value: Option<&SharedString>, t: &Theme) -> impl IntoElement {
    match value.filter(|v| !v.is_empty()) {
        None => div(),
        Some(v) => div()
            .flex()
            .flex_row()
            .gap(px(10.))
            .child(
                div()
                    .w(px(92.))
                    .flex_shrink_0()
                    .text_size(px(t.text_data_sm))
                    .text_color(rgb(t.fg3))
                    .child(SharedString::from(label.to_string())),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .text_size(px(t.text_small))
                    .text_color(rgb(t.fg1))
                    .child(v.clone()),
            ),
    }
}

fn action_btn(
    label: &str,
    t: &Theme,
    on: impl Fn(&mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let hov = t.surface_hover;
    div()
        .id(SharedString::from(format!("notice-{label}")))
        .px(px(10.))
        .py(px(6.))
        .rounded(px(t.r_sm))
        .border_1()
        .border_color(rgb(t.border))
        .text_size(px(t.text_data_sm))
        .text_color(rgb(t.fg1))
        .cursor_pointer()
        .hover(move |s| s.bg(rgb(hov)).text_color(rgb(t.fg0)))
        .child(SharedString::from(label.to_string()))
        .on_click(move |_ev, window, cx| on(window, cx))
}

fn format_datetime(at: i64) -> String {
    DateTime::from_timestamp(at, 0)
        .map(|dt| {
            dt.with_timezone(&Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|| "--".into())
}
