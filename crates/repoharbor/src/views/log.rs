//! Activity log view — recent in-app events (fleet ops, push/pull, scans,
//! soft CI notices). Backed by the in-memory ring in `activity_log`.

use chrono::{DateTime, Local};
use gpui::{
    Entity, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, px, rgb,
};

use crate::activity_log::{LogEntry, LogLevel};
use crate::icon::lucide;
use crate::shell::RepoHarborApp;
use crate::theme::Theme;

/// Render the Log / Activity view.
pub fn render(entries: &[LogEntry], t: &Theme, app: &Entity<RepoHarborApp>) -> impl IntoElement {
    let body = if entries.is_empty() {
        super::note(
            "No activity yet — toasts, desktop notifications, scans, and fleet ops will show up here. Click a row for full details.",
            t,
        )
        .into_any_element()
    } else {
        list(entries, t, app).into_any_element()
    };

    // Custom chrome: Clear instead of Refresh (nothing to reload).
    let app_clear = app.clone();
    div()
        .flex()
        .flex_col()
        .size_full()
        .bg(rgb(t.page))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.))
                .h(px(52.))
                .px(px(20.))
                .border_b_1()
                .border_color(rgb(t.border))
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(t.text_h3))
                        .text_color(rgb(t.fg0))
                        .child("Log"),
                )
                .child(super::muted_mono(format!("{} events", entries.len()), t))
                .child(div().flex_1())
                .child(
                    div()
                        .id("log-clear")
                        .px(px(10.))
                        .py(px(5.))
                        .rounded(px(t.r_sm))
                        .border_1()
                        .border_color(rgb(t.border))
                        .text_size(px(t.text_data_sm))
                        .text_color(rgb(t.fg2))
                        .cursor_pointer()
                        .hover(|s| s.bg(rgb(t.surface_hover)).text_color(rgb(t.fg0)))
                        .child("Clear")
                        .on_click(move |_ev, _win, cx| {
                            app_clear.update(cx, |this, cx| {
                                this.activity_log.clear();
                                cx.notify();
                            });
                        }),
                ),
        )
        .child(
            div()
                .id("log-scroll")
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .p(px(20.))
                .child(body),
        )
}

fn list(entries: &[LogEntry], t: &Theme, app: &Entity<RepoHarborApp>) -> impl IntoElement {
    let mut col = div().flex().flex_col().gap(px(4.));
    for e in entries {
        col = col.child(row(e, t, app));
    }
    col
}

fn row(e: &LogEntry, t: &Theme, app: &Entity<RepoHarborApp>) -> impl IntoElement {
    let (icon, color) = match e.level {
        LogLevel::Info => ("bell", t.fg2),
        LogLevel::Warn => ("circle-alert", t.dirty),
        LogLevel::Error => ("circle-alert", t.behind),
    };
    let app_row = app.clone();
    let entry = e.clone();
    let hov = t.surface_hover;
    div()
        .id(SharedString::from(format!("log-{}", e.id)))
        .flex()
        .flex_row()
        .items_start()
        .gap(px(10.))
        .px(px(10.))
        .py(px(8.))
        .rounded(px(t.r_sm))
        .border_1()
        .border_color(rgb(t.border))
        .bg(rgb(t.surface))
        .cursor_pointer()
        .hover(move |s| s.bg(rgb(hov)))
        .on_click(move |_ev, _win, cx| {
            let entry = entry.clone();
            app_row.update(cx, |this, cx| this.open_notice(entry, cx));
        })
        .child(lucide(icon, 14., color))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.))
                .gap(px(2.))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            div()
                                .font_family("monospace")
                                .text_size(px(t.text_data_sm))
                                .text_color(rgb(t.fg3))
                                .child(SharedString::from(format_time(e.at))),
                        )
                        .child(super::tag(e.level.label(), color, t)),
                )
                .child(
                    div()
                        .text_size(px(t.text_small))
                        .text_color(rgb(t.fg1))
                        .child(e.message.clone()),
                )
                .children({
                    let term_cwd = e
                        .context
                        .path
                        .as_ref()
                        .or(e.context.repo_id.as_ref())
                        .cloned();
                    let mut out = Vec::new();
                    if let Some(cwd) = term_cwd {
                        let mut combined = e.message.to_string();
                        for (k, v) in &e.context.facts {
                            combined.push('\n');
                            combined.push_str(k.as_ref());
                            combined.push(' ');
                            combined.push_str(v.as_ref());
                        }
                        let cmds = repoharbor_core::terminal::extract_commands_from_text(&combined);
                        if !cmds.is_empty() {
                            let mut bar =
                                div().flex().flex_row().flex_wrap().gap(px(6.)).mt(px(4.));
                            let title = e.context.repo_name.clone().unwrap_or_else(|| {
                                crate::shell::repo_tab_title(cwd.as_ref()).into()
                            });
                            for (i, c) in cmds.into_iter().take(2).enumerate() {
                                let app_cmd = app.clone();
                                let cwd_cmd = cwd.clone();
                                let title_cmd = title.clone();
                                let raw = c.command.clone();
                                let border_c = if c.dangerous { t.dirty } else { t.border };
                                let fg_c = if c.dangerous {
                                    t.dirty
                                } else {
                                    t.accent_bright
                                };
                                let hover_c = t.border_strong;
                                bar = bar.child(
                                    div()
                                        .id(SharedString::from(format!("log-run-{}-{i}", e.id)))
                                        .px(px(7.))
                                        .py(px(2.))
                                        .rounded(px(t.r_xs))
                                        .bg(rgb(t.button_bg))
                                        .border_1()
                                        .border_color(rgb(border_c))
                                        .font_family("monospace")
                                        .text_size(px(t.text_data_sm))
                                        .text_color(rgb(fg_c))
                                        .cursor_pointer()
                                        .hover(move |s| s.border_color(rgb(hover_c)))
                                        .child(SharedString::from(format!("▶ {}", c.label)))
                                        .on_click(move |_ev, win, cx| {
                                            cx.stop_propagation();
                                            let (cwd_cmd, title_cmd, raw) =
                                                (cwd_cmd.clone(), title_cmd.clone(), raw.clone());
                                            app_cmd.update(cx, |this, cx| {
                                                this.open_terminal_in(
                                                    cwd_cmd.as_ref(),
                                                    title_cmd.as_ref(),
                                                    Some(&raw),
                                                    true,
                                                    Some(win),
                                                    cx,
                                                );
                                            });
                                        }),
                                );
                            }
                            out.push(bar);
                        }
                    }
                    out
                }),
        )
}

/// Compact local clock from unix seconds (HH:MM:SS).
fn format_time(at: i64) -> String {
    DateTime::from_timestamp(at, 0)
        .map(|dt| dt.with_timezone(&Local).format("%H:%M:%S").to_string())
        .unwrap_or_else(|| "--:--:--".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn format_time_matches_local_clock() {
        let at = 1_704_067_200; // 2023-12-31 16:00:00 UTC
        let expected = DateTime::from_timestamp(at, 0)
            .expect("valid unix")
            .with_timezone(&Local)
            .format("%H:%M:%S")
            .to_string();
        assert_eq!(format_time(at), expected);
    }

    #[test]
    fn format_time_is_not_utc_when_offset_nonzero() {
        let at = 1_704_067_200;
        let utc = DateTime::from_timestamp(at, 0)
            .expect("valid unix")
            .with_timezone(&Utc)
            .format("%H:%M:%S")
            .to_string();
        if Local::now().offset().local_minus_utc() != 0 {
            assert_ne!(format_time(at), utc);
        }
    }

    #[test]
    fn format_time_handles_invalid_timestamp() {
        assert_eq!(format_time(i64::MAX), "--:--:--");
    }
}
