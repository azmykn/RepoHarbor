//! Settings view — edit the config in-app instead of hand-editing TOML.
//!
//! A draft `AppConfig` (toggles / scan depth / roots) lives on `RepoHarborApp`
//! alongside text-input entities for the string fields; "Save & rescan" reads
//! them back, persists via `config::save`, and re-scans. This first cut covers
//! roots, launchers, AI backend/endpoints, and notifications, plus the live
//! panels: GitHub device-flow login and AI status — Ollama (host + model pull)
//! or llama.cpp (server path + GGUF download), switchable in-app.

use gpui::{
    AppContext, ClipboardItem, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, Window, div, px, rgb,
};
use gpui_component::input::{Input, InputState};
use repoharbor_core::model::AppConfig;

use crate::icon::lucide;
use crate::shell::RepoHarborApp;
use crate::theme::Theme;

/// A GitHub device-flow login in progress: the code the user types at the
/// verification URL, plus a live status line.
pub struct GithubDevice {
    pub user_code: SharedString,
    pub verification_uri: SharedString,
    pub status: SharedString,
}

/// Live reachability of the AI backend, refreshed when Settings opens and on
/// demand. Drives the AI-status panel.
#[derive(Default, Clone)]
pub enum AiStatus {
    #[default]
    Unknown,
    Checking,
    Offline,
    /// Reachable — the installed models as (name, human size).
    Ready(Vec<(SharedString, SharedString)>),
    /// A model download is in flight.
    Pulling(SharedString),
}

/// The editable settings session: a draft config + the string-field inputs.
pub struct SettingsState {
    pub draft: AppConfig,
    pub ide: Entity<InputState>,
    pub agent: Entity<InputState>,
    /// Argv appended to the agent command when dispatching with a task;
    /// `{prompt}` is substituted as one argument.
    pub agent_dispatch: Entity<InputState>,
    pub ollama_host: Entity<InputState>,
    pub ai_model: Entity<InputState>,
    pub embed_model: Entity<InputState>,
    /// llama.cpp: path override for the `llama-server` binary.
    pub llama_server: Entity<InputState>,
    /// llama.cpp: a GGUF model URL to download.
    pub llama_url: Entity<InputState>,
    /// Cloud: base URL of the OpenAI-compatible endpoint.
    pub openai_base: Entity<InputState>,
    /// Cloud: API key entry. Starts empty (the stored key is never displayed);
    /// a blank value on Save leaves the existing key alone.
    pub openai_key: Entity<InputState>,
    /// Provider pool form: display name / provider label.
    pub pool_name: Entity<InputState>,
    /// Provider pool form: OpenAI-compatible base URL (optional for known providers).
    pub pool_base: Entity<InputState>,
    /// Provider pool form: model id.
    pub pool_model: Entity<InputState>,
    /// Provider pool form: masked per-row API key (written owner-only to pool_keys.toml).
    pub pool_key: Entity<InputState>,
    /// Deployment id currently loaded into the add/edit form, if editing.
    pub pool_edit: Option<String>,
    /// Inline status/test feedback for the provider list view.
    pub pool_note: SharedString,
    pub client_id: Entity<InputState>,
    pub ignore: Entity<InputState>,
    pub add_root: Entity<InputState>,
    /// Typed path to append to `draft.pull_only_prefixes`.
    pub add_pull_only: Entity<InputState>,
    /// Typed path to append to `draft.mute_attention_prefixes`.
    pub add_mute_attention: Entity<InputState>,
    /// External diff tool template (`{path}` / `{file}`).
    pub diff_command: Entity<InputState>,
    /// Embedded terminal shell override (`blank` = `$SHELL`).
    pub terminal_shell: Entity<InputState>,
    /// Scan depth as a NumberInput (integer 1–8).
    pub scan_depth: Entity<InputState>,
    /// Flash a "Saved" confirmation after a successful save.
    pub saved: bool,
    /// Result line for the AI Test / Clear-cache actions.
    pub ai_note: SharedString,
    /// Semantic-index size (chunks/repos/bytes); `None` until loaded.
    pub index_stats: Option<repoharbor_core::semantic::IndexStats>,
}

impl SettingsState {
    /// Seed a session from the live config.
    pub fn new(cfg: &AppConfig, window: &mut Window, cx: &mut gpui::App) -> Self {
        let field = |window: &mut Window, cx: &mut gpui::App, ph: &'static str, val: &str| {
            let val = val.to_string();
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(ph)
                    .default_value(val)
            })
        };
        SettingsState {
            ide: field(window, cx, "code {path}", &cfg.ide_command),
            agent: field(
                window,
                cx,
                "ptyxis --new-window -d {path}  (or … -- aider)",
                &cfg.agent_command,
            ),
            agent_dispatch: field(window, cx, "{prompt}", &cfg.agent_dispatch_args),
            ollama_host: field(window, cx, "http://localhost:11434", &cfg.ollama_host),
            ai_model: field(window, cx, "model name", &cfg.ai_model),
            embed_model: field(window, cx, "embed model", &cfg.embed_model),
            llama_server: field(
                window,
                cx,
                "llama-server (auto-detected)",
                &cfg.llama_server_path,
            ),
            llama_url: field(window, cx, "https://…/model.gguf", ""),
            openai_base: field(
                window,
                cx,
                if backend_is_openrouter(&cfg.ai_backend) {
                    "https://openrouter.ai/api/v1"
                } else if backend_is_groq(&cfg.ai_backend) {
                    "https://api.groq.com/openai/v1"
                } else if backend_is_mistral(&cfg.ai_backend) {
                    "https://api.mistral.ai/v1"
                } else {
                    "https://api.openai.com/v1"
                },
                &cfg.openai_base,
            ),
            openai_key: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(if repoharbor_core::cloud::has_api_key() {
                        "•••••• (stored — type to replace)"
                    } else if backend_is_groq(&cfg.ai_backend) {
                        "gsk_… (Groq key or keys pool)"
                    } else if backend_is_mistral(&cfg.ai_backend) {
                        "mstrl_… (Mistral key or keys pool)"
                    } else if backend_is_openrouter(&cfg.ai_backend) {
                        "sk-or-v1-… (OpenRouter key)"
                    } else {
                        "API key"
                    })
                    .masked(true)
            }),
            pool_name: field(window, cx, "Name / provider (e.g. Groq #2, DeepSeek)", ""),
            pool_base: field(
                window,
                cx,
                "Base URL (blank = auto for known providers)",
                "",
            ),
            pool_model: field(window, cx, "Model id (e.g. codestral-latest)", ""),
            pool_key: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("API key (saved 0600 — blank = provider key / local)")
                    .masked(true)
            }),
            pool_edit: None,
            pool_note: SharedString::default(),
            client_id: field(window, cx, "GitHub OAuth client id", &cfg.github_client_id),
            ignore: field(window, cx, "node_modules, .cache", &cfg.ignore.join(", ")),
            add_root: field(window, cx, "~/Projects or ~/code/my-app", ""),
            add_pull_only: field(window, cx, "~/Projects/upstream/core", ""),
            add_mute_attention: field(window, cx, "~/odoo/…/noisy-repo", ""),
            diff_command: field(window, cx, "meld {path}", &cfg.diff_command),
            terminal_shell: field(window, cx, "$SHELL (default)", &cfg.terminal_shell),
            scan_depth: cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(cfg.scan_depth.to_string())
                    .validate(|s, _| s.chars().all(|c| c.is_ascii_digit()))
                    .min(1.)
                    .max(8.)
                    .step(1.)
            }),
            draft: cfg.clone(),
            saved: false,
            ai_note: SharedString::default(),
            index_stats: None,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn render(
    s: &SettingsState,
    filter: Option<&str>,
    authed: bool,
    device: &Option<GithubDevice>,
    ai: &AiStatus,
    ai_ready: bool,
    install_kind: &repoharbor_core::updater::InstallKind,
    update_state: &crate::shell::AppUpdateState,
    auto_check_updates: bool,
    autostart_enabled: bool,
    t: &Theme,
    app: &Entity<RepoHarborApp>,
) -> impl IntoElement {
    // The sidebar SECTIONS panel gates which section shows (None = all).
    let show = |section: &str| filter.is_none() || filter == Some(section);
    let mut sections: Vec<gpui::AnyElement> = Vec::new();
    if show("account") {
        sections.push(
            account_section(authed, device, s.draft.github_allow_cli_token, t, app)
                .into_any_element(),
        );
    }
    if show("roots") {
        sections.push(roots_section(s, t, app).into_any_element());
    }
    if show("launchers") {
        sections.push(launchers_section(s, t, app).into_any_element());
    }
    if show("ai") {
        sections.push(ai_section(s, ai, ai_ready, t, app).into_any_element());
    }
    if show("notifications") {
        sections.push(notifications_section(s, autostart_enabled, t, app).into_any_element());
    }
    if show("about") {
        sections.push(
            about_section(install_kind, update_state, auto_check_updates, t, app)
                .into_any_element(),
        );
    }

    let header_title = if filter == Some("about") {
        "About & license"
    } else {
        "Settings"
    };
    let mut page = div()
        .flex()
        .flex_col()
        .size_full()
        .bg(rgb(t.page))
        // Header
        .child(
            div()
                .flex()
                .items_center()
                .h(px(52.))
                .px(px(20.))
                .border_b_1()
                .border_color(rgb(t.border))
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(t.text_h3))
                        .text_color(rgb(t.fg0))
                        .child(header_title),
                ),
        )
        // Scrolling sections
        .child(
            div()
                .id("settings-scroll")
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .p(px(20.))
                .gap(px(16.))
                .children(sections),
        );
    if filter != Some("about") {
        page = page.child(save_footer(s, t, app));
    }
    page
}

/// GitHub account panel: connection status + Connect (device flow) / Sign out.
fn account_section(
    authed: bool,
    device: &Option<GithubDevice>,
    allow_cli: bool,
    t: &Theme,
    app: &Entity<RepoHarborApp>,
) -> impl IntoElement {
    use repoharbor_core::oauth::{self, GithubTokenSource};

    let mut col = section(t, "GitHub account");
    let source = oauth::github_token_source().map(|(_, s)| s);
    let signed_out = oauth::is_signed_out();

    if let Some(d) = device {
        // A device-flow login is in progress: show the code + verification URL.
        let uri = d.verification_uri.clone();
        col = col
            .child(field_label(
                "Enter this code at the verification page, then authorize:",
                t,
            ))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.))
                    .child({
                        let code = d.user_code.clone();
                        let app = app.clone();
                        let (hb, hc) = (t.surface_hover, t.border_strong);
                        div()
                            // Unique id — duplicate interactive ids break click routing in GPUI.
                            .id("github-device-user-code")
                            .px(px(10.))
                            .py(px(6.))
                            .rounded(px(t.r_sm))
                            .bg(rgb(t.button_bg))
                            .border_1()
                            .border_color(rgb(t.border))
                            .font_family("monospace")
                            .text_size(px(t.text_h3))
                            .text_color(rgb(t.fg0))
                            .cursor_pointer()
                            .hover(move |s| s.bg(rgb(hb)).border_color(rgb(hc)))
                            .child(code.clone())
                            .on_click(move |_ev, _win, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(code.to_string()));
                                app.update(cx, |this, cx| {
                                    this.push_toast(
                                        crate::toast::ToastKind::Success,
                                        "Code copied",
                                        None,
                                        cx,
                                    );
                                });
                            })
                    })
                    .child(button_id(
                        "btn-github-open-verify",
                        "Open page",
                        t,
                        move |_cx| {
                            let _ = repoharbor_core::launch::open(&uri);
                        },
                    )),
            )
            .child(note_line(d.status.clone(), t.fg3, t));
    } else if authed {
        let source = source.unwrap_or(GithubTokenSource::AppOAuth);
        col = col
            .child(status_row(
                "circle-check",
                format!("Connected to GitHub · via {}", source.label()),
                t.clean,
                t,
            ))
            .child(note_line(
                SharedString::from(
                    "Powers Inbox, Explore, PR actions, and CI badges. Pull/Push still use your git credentials (SSH or credential helper), not this link.",
                ),
                t.fg3,
                t,
            ));
        col = match source {
            GithubTokenSource::AppOAuth => {
                let sso_url = oauth::github_oauth_app_settings_url();
                let open_url = sso_url.clone();
                col.child(note_line(
                    SharedString::from(
                        "Org remotes: open the RepoHarbor OAuth app and click Authorize for each org (SSO). CI uses the classic `repo` scope from Connect.",
                    ),
                    t.fg3,
                    t,
                ))
                .child(
                    div()
                        .font_family("monospace")
                        .text_size(px(t.text_data_sm))
                        .text_color(rgb(t.fg2))
                        .child(SharedString::from(sso_url)),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .gap(px(8.))
                        .child(button_id("btn-github-sso", "Authorize org SSO", t, move |_cx| {
                            let _ = repoharbor_core::launch::open(&open_url);
                        }))
                        .child(button_id("btn-github-sign-out", "Sign out", t, {
                            let app = app.clone();
                            move |cx| {
                                app.update(cx, |this, cx| this.github_sign_out(cx));
                            }
                        })),
                )
            }
            GithubTokenSource::EnvVar | GithubTokenSource::GhCli => col
                .child(note_line(
                    SharedString::from(
                        "Connected via machine token — GitHub → Settings → Applications may not list RepoHarbor. CI needs Actions:read (fine-grained) or classic `repo`. Prefer Connect with RepoHarbor for Inbox/CI without changing `gh`.",
                    ),
                    t.fg3,
                    t,
                ))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .gap(px(8.))
                        .child(button_id(
                            "btn-github-prefer-oauth",
                            "Use RepoHarbor login instead",
                            t,
                            {
                                let app = app.clone();
                                move |cx| {
                                    app.update(cx, |this, cx| this.github_prefer_oauth(cx));
                                }
                            },
                        ))
                        .child(button_id("btn-github-sign-out", "Sign out", t, {
                            let app = app.clone();
                            move |cx| {
                                app.update(cx, |this, cx| this.github_sign_out(cx));
                            }
                        })),
                ),
        };
    } else {
        col = col
            .child(status_row(
                "circle-alert",
                if signed_out {
                    "Not connected — signed out of RepoHarbor (machine `gh` / env left alone)."
                } else {
                    "Not connected — sign in to use Inbox, Feed, Explore, PR actions, and CI."
                },
                t.fg3,
                t,
            ))
            .child(note_line(
                SharedString::from(
                    "Connect requests read:user + repo (private repos and Actions/CI). Sign out only affects RepoHarbor — it never runs `gh auth logout`.",
                ),
                t.fg3,
                t,
            ))
            .child(
                div().child(button_id("btn-github-connect", "Connect GitHub", t, {
                    let app = app.clone();
                    move |cx| {
                        app.update(cx, |this, cx| this.github_connect(cx));
                    }
                })),
            );
        if signed_out {
            col = col.child(note_line(
                SharedString::from(
                    "To use `gh` / REPOHARBOR_GITHUB_TOKEN again without RepoHarbor OAuth, turn on “Also use `gh` / env token” below.",
                ),
                t.fg3,
                t,
            ));
        }
    }

    col.child(toggle(
        "Also use `gh` / env token",
        allow_cli,
        t,
        app,
        |this| {
            let on = !this
                .settings
                .as_ref()
                .map(|s| s.draft.github_allow_cli_token)
                .unwrap_or(this.config.github_allow_cli_token);
            this.github_set_allow_cli_token(on);
        },
    ))
    .child(note_line(
        SharedString::from(
            "When off, only an RepoHarbor Connect token is used. Sign out always stops API calls until Connect or you turn this back on.",
        ),
        t.fg3,
        t,
    ))
}

// ── sections ────────────────────────────────────────────────────────────────

fn roots_section(s: &SettingsState, t: &Theme, app: &Entity<RepoHarborApp>) -> impl IntoElement {
    let mut col = section(t, "Workspace roots");
    col = col.child(
        div()
            .text_size(px(t.text_data_sm))
            .text_color(rgb(t.fg3))
            .child("A single git repo or a folder of repos."),
    );
    for (i, root) in s.draft.roots.iter().enumerate() {
        // Element ids must be unique — every row used to share `ib-x`, so GPUI
        // dropped/misrouted clicks and × appeared dead.
        let remove = {
            let app = app.clone();
            let (hb, _) = (t.surface_hover, t.fg3);
            div()
                .id(SharedString::from(format!("ib-x-root-{i}")))
                .flex()
                .items_center()
                .justify_center()
                .w(px(24.))
                .h(px(24.))
                .rounded(px(t.r_xs))
                .cursor_pointer()
                .hover(move |s| s.bg(rgb(hb)))
                .child(lucide("x", 13., t.fg3))
                .on_click(move |_ev, _win, cx| {
                    app.update(cx, |this, cx| this.settings_remove_root(i, cx));
                })
        };
        col = col.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.))
                .child(lucide("folder", 13., t.fg3))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .font_family("monospace")
                        .text_size(px(t.text_data_sm))
                        .text_color(rgb(t.fg1))
                        .child(SharedString::from(root.clone())),
                )
                .child(remove),
        );
    }
    // Browse (native multi-folder picker) + typed path Add (fallback / paste).
    let browse = {
        let app = app.clone();
        let (hb, hf) = (t.border_strong, t.fg0);
        div()
            .id("btn-Browse-roots")
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
            .child("Browse…")
            .on_click(move |_ev, _window, cx| {
                app.update(cx, |this, cx| this.settings_browse_roots(cx));
            })
    };
    let add = {
        let app = app.clone();
        let (hb, hf) = (t.border_strong, t.fg0);
        div()
            .id("btn-Add-root")
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
            .child("Add")
            .on_click(move |_ev, window, cx| {
                app.update(cx, |this, cx| this.settings_add_root(window, cx));
            })
    };
    col = col.child(
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .child(div().flex_1().min_w(px(0.)).child(Input::new(&s.add_root)))
            .child(browse)
            .child(add),
    );

    // Pull-only upstream prefixes (editable — not only config.toml).
    col = col.child(
        div()
            .mt(px(14.))
            .mb(px(4.))
            .text_size(px(t.text_data_sm))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(rgb(t.fg1))
            .child("Pull-only paths (upstream)"),
    );
    col = col.child(
        div()
            .text_size(px(t.text_data_sm))
            .text_color(rgb(t.fg3))
            .child(
                "Repos under these prefixes: Pull to update; hide Push / Gen & push / Empty commit; demote upstream CI. Removing a path is remembered across restarts (new Odoo core/custom trees still auto-add). Your digits modules stay Pushable when outside this list.",
            ),
    );
    for (i, prefix) in s.draft.pull_only_prefixes.iter().enumerate() {
        let remove = {
            let app = app.clone();
            let (hb, _) = (t.surface_hover, t.fg3);
            div()
                .id(SharedString::from(format!("ib-x-pullonly-{i}")))
                .flex()
                .items_center()
                .justify_center()
                .w(px(24.))
                .h(px(24.))
                .rounded(px(t.r_xs))
                .cursor_pointer()
                .hover(move |s| s.bg(rgb(hb)))
                .child(lucide("x", 13., t.fg3))
                .on_click(move |_ev, _win, cx| {
                    app.update(cx, |this, cx| this.settings_remove_pull_only(i, cx));
                })
        };
        col = col.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.))
                .child(lucide("arrow-down", 13., t.fg3))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .font_family("monospace")
                        .text_size(px(t.text_data_sm))
                        .text_color(rgb(t.fg1))
                        .child(SharedString::from(prefix.clone())),
                )
                .child(remove),
        );
    }
    let add_po = {
        let app = app.clone();
        let (hb, hf) = (t.border_strong, t.fg0);
        div()
            .id("btn-Add-pull-only")
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
            .child("Add")
            .on_click(move |_ev, window, cx| {
                app.update(cx, |this, cx| this.settings_add_pull_only(window, cx));
            })
    };
    col = col.child(
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .mt(px(4.))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .child(Input::new(&s.add_pull_only)),
            )
            .child(add_po),
    );

    // Mute-attention prefixes — silence Needs me for selected trees.
    col = col.child(
        div()
            .mt(px(14.))
            .mb(px(4.))
            .text_size(px(t.text_data_sm))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(rgb(t.fg1))
            .child("Mute attention paths"),
    );
    col = col.child(
        div()
            .text_size(px(t.text_data_sm))
            .text_color(rgb(t.fg3))
            .child(
                "Repos under these prefixes never raise Needs me, attention chips, or tray alerts. Does not edit files inside the repo — local to RepoHarbor only.",
            ),
    );
    for (i, prefix) in s.draft.mute_attention_prefixes.iter().enumerate() {
        let remove = {
            let app = app.clone();
            let (hb, _) = (t.surface_hover, t.fg3);
            div()
                .id(SharedString::from(format!("ib-x-mute-attn-{i}")))
                .flex()
                .items_center()
                .justify_center()
                .w(px(24.))
                .h(px(24.))
                .rounded(px(t.r_xs))
                .cursor_pointer()
                .hover(move |s| s.bg(rgb(hb)))
                .child(lucide("x", 13., t.fg3))
                .on_click(move |_ev, _win, cx| {
                    app.update(cx, |this, cx| this.settings_remove_mute_attention(i, cx));
                })
        };
        col = col.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.))
                .child(lucide("bell", 13., t.fg3))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .font_family("monospace")
                        .text_size(px(t.text_data_sm))
                        .text_color(rgb(t.fg1))
                        .child(SharedString::from(prefix.clone())),
                )
                .child(remove),
        );
    }
    let add_mute = {
        let app = app.clone();
        let (hb, hf) = (t.border_strong, t.fg0);
        div()
            .id("btn-Add-mute-attention")
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
            .child("Add")
            .on_click(move |_ev, window, cx| {
                app.update(cx, |this, cx| this.settings_add_mute_attention(window, cx));
            })
    };
    col = col.child(
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .mt(px(4.))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .child(Input::new(&s.add_mute_attention)),
            )
            .child(add_mute),
    );

    // Scan depth stepper + ignore field.
    col = col.child(number_row("Scan depth", &s.scan_depth, t));
    col.child(labeled("Ignore (comma-separated)", s.ignore.clone(), t))
}

fn launchers_section(
    s: &SettingsState,
    t: &Theme,
    app: &Entity<RepoHarborApp>,
) -> impl IntoElement {
    section(t, "Launchers")
        .child(labeled("IDE command", s.ide.clone(), t))
        .child(labeled(
            "Terminal / agent command (opens shell or local agent at {path})",
            s.agent.clone(),
            t,
        ))
        .child(labeled(
            "Agent dispatch args ({prompt} = task)",
            s.agent_dispatch.clone(),
            t,
        ))
        .child(labeled(
            "External diff command ({path} = repo, {file} = selected path)",
            s.diff_command.clone(),
            t,
        ))
        .child(note_line(
            SharedString::from(
                "Changes → Open external diff launches this tool (default: meld, or code / xdg-open).",
            ),
            t.fg3,
            t,
        ))
        .child(toggle(
            "Open built-in terminal on repo card / drawer clicks",
            s.draft.terminal_embedded_default,
            t,
            app,
            |this| {
                if let Some(st) = &mut this.settings {
                    st.draft.terminal_embedded_default = !st.draft.terminal_embedded_default;
                }
            },
        ))
        .child(toggle(
            "Confirm destructive commands before executing in terminal",
            s.draft.terminal_confirm_destructive,
            t,
            app,
            |this| {
                if let Some(st) = &mut this.settings {
                    st.draft.terminal_confirm_destructive = !st.draft.terminal_confirm_destructive;
                }
            },
        ))
        .child(toggle(
            "Dispatch coding agents inside built-in terminal tabs",
            s.draft.terminal_dispatch_embedded,
            t,
            app,
            |this| {
                if let Some(st) = &mut this.settings {
                    st.draft.terminal_dispatch_embedded = !st.draft.terminal_dispatch_embedded;
                }
            },
        ))
        .child(toggle(
            "Show live command log in bottom terminal on toolbar actions",
            s.draft.terminal_auto_show_log,
            t,
            app,
            |this| {
                if let Some(st) = &mut this.settings {
                    st.draft.terminal_auto_show_log = !st.draft.terminal_auto_show_log;
                }
            },
        ))
        .child(labeled(
            "Built-in terminal shell (blank = $SHELL)",
            s.terminal_shell.clone(),
            t,
        ))
}

/// True when the configured backend string selects the llama.cpp sidecar.
fn backend_is_llama(backend: &str) -> bool {
    matches!(backend, "llamaCpp" | "llama_cpp" | "llamacpp")
}

/// True when the configured backend string selects OpenRouter.
fn backend_is_openrouter(backend: &str) -> bool {
    matches!(backend, "openrouter" | "open_router" | "OpenRouter")
}

/// True when the configured backend string selects Groq.
fn backend_is_groq(backend: &str) -> bool {
    matches!(backend, "groq" | "Groq")
}

/// True when the configured backend string selects Mistral.
fn backend_is_mistral(backend: &str) -> bool {
    matches!(backend, "mistral" | "Mistral")
}

/// True when the configured backend string selects the generic OpenAI-compatible
/// endpoint. Mirrors `repoharbor_core::ai`'s accepted spellings.
fn backend_is_cloud(backend: &str) -> bool {
    matches!(
        backend,
        "cloud" | "openai" | "openaiCompat" | "openai_compat"
    )
}

fn ai_section(
    s: &SettingsState,
    ai: &AiStatus,
    ai_ready: bool,
    t: &Theme,
    app: &Entity<RepoHarborApp>,
) -> impl IntoElement {
    let is_llama = backend_is_llama(&s.draft.ai_backend);
    let is_openrouter = backend_is_openrouter(&s.draft.ai_backend);
    let is_groq = backend_is_groq(&s.draft.ai_backend);
    let is_mistral = backend_is_mistral(&s.draft.ai_backend);
    let is_cloud = backend_is_cloud(&s.draft.ai_backend);
    let mut sec = section(t, "AI & search")
        .child(toggle(
            "Enable AI features",
            s.draft.ai_enabled,
            t,
            app,
            |a| {
                if let Some(s) = &mut a.settings {
                    s.draft.ai_enabled = !s.draft.ai_enabled;
                }
            },
        ))
        .child(backend_picker(&s.draft.ai_backend, t, app));
    if is_groq {
        sec = sec
            .child(labeled("Base URL", s.openai_base.clone(), t))
            .child(labeled("Chat model", s.ai_model.clone(), t))
            .child(groq_model_presets(s, t, app))
            .child(cloud_key_row(s, t, app, "groq"))
            .child(note_line(
                SharedString::from(
                    "Groq backend: Ultra-fast LPU inference (15-60ms) with generous free tier. Supports openai/gpt-oss-120b, openai/gpt-oss-20b, qwen3.8-27b, and allam-2-7b. Prompts go to Groq; embeddings stay local (Ollama).",
                ),
                t.clean,
                t,
            ));
    } else if is_mistral {
        sec = sec
            .child(labeled("Base URL", s.openai_base.clone(), t))
            .child(labeled("Chat model", s.ai_model.clone(), t))
            .child(mistral_model_presets(s, t, app))
            .child(cloud_key_row(s, t, app, "mistral"))
            .child(note_line(
                SharedString::from(
                    "Mistral AI backend: Codestral optimized for code and Conventional Commits, plus Mistral Large/Small. Prompts go to Mistral; embeddings stay local (Ollama).",
                ),
                t.clean,
                t,
            ));
    } else if is_openrouter {
        sec = sec
            .child(labeled("Base URL", s.openai_base.clone(), t))
            .child(labeled("Chat model", s.ai_model.clone(), t))
            .child(openrouter_model_presets(s, t, app))
            .child(cloud_key_row(s, t, app, "openrouter"))
            .child(note_line(
                SharedString::from(
                    "OpenRouter backend: Access 200+ hosted models (DeepSeek V3, Llama 3.3 70B, Claude 3.5, GPT-4o) using your OpenRouter API key. Prompts are routed to OpenRouter; embeddings and semantic search stay local (Ollama).",
                ),
                t.clean,
                t,
            ));
    } else if is_cloud {
        sec = sec
            .child(labeled("Base URL", s.openai_base.clone(), t))
            .child(labeled("Chat model", s.ai_model.clone(), t))
            .child(cloud_key_row(s, t, app, "cloud"))
            // Amber, not red: this is a standing caution about where the data
            // goes, not a failure. Red (`behind`) reads as "something broke".
            .child(note_line(
                SharedString::from(
                    "Cloud backend: prompts — including your staged diffs and commit history — are sent to this endpoint. Pick Ollama or llama.cpp to keep everything on-device. Embeddings and semantic search always stay local.",
                ),
                t.dirty,
                t,
            ));
    } else if is_llama {
        sec = sec
            .child(labeled("llama-server path", s.llama_server.clone(), t))
            .child(llama_download_row(s, t, app))
            .child(llama_model_line(s, t));
    } else {
        sec = sec
            .child(labeled("Ollama host", s.ollama_host.clone(), t))
            .child(labeled("Chat model", s.ai_model.clone(), t))
            .child(labeled("Embedding model", s.embed_model.clone(), t));
    }
    sec = sec
        .child(toggle(
            "Auto-failover to next model/provider on rate limit or error (429 / 402 / timeout)",
            s.draft.ai_failover_enabled,
            t,
            app,
            |a| {
                if let Some(s) = &mut a.settings {
                    s.draft.ai_failover_enabled = !s.draft.ai_failover_enabled;
                }
            },
        ))
        .child(model_pool_block(s, t, app))
        .child(labeled("GitHub OAuth client id", s.client_id.clone(), t))
        .child(ai_status_block(s, ai, t, app));
    // Semantic recall index — an aiReady affordance (hidden, not broken, when
    // AI is off/unreachable or the backend can't embed).
    if ai_ready && repoharbor_core::ai::embeddings_supported() {
        sec = sec.child(semantic_index_block(s, t, app));
    }
    sec
}

/// Semantic-index size line + "Rebuild index" (drop + re-embed the corpus).
fn semantic_index_block(
    s: &SettingsState,
    t: &Theme,
    app: &Entity<RepoHarborApp>,
) -> impl IntoElement {
    let line: SharedString = match &s.index_stats {
        Some(st) if st.chunks > 0 => format!(
            "Semantic index: {} chunk{} across {} repo{} · {}",
            st.chunks,
            if st.chunks == 1 { "" } else { "s" },
            st.repos,
            if st.repos == 1 { "" } else { "s" },
            crate::data::human_bytes(st.bytes),
        )
        .into(),
        Some(_) => "Semantic index: empty — repos embed in the background.".into(),
        None => "Semantic index: …".into(),
    };
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(10.))
        .child(div().flex_1().child(status_row("sparkles", line, t.fg2, t)))
        .child(button("Rebuild index", t, {
            let app = app.clone();
            move |cx| {
                app.update(cx, |this, cx| this.rebuild_semantic_index(cx));
            }
        }))
}

/// Small clickable pill button inside the provider list view.
fn pool_chip(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    color: u32,
    active_border: Option<u32>,
    t: &Theme,
    on_click: impl Fn(&mut gpui::Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let border = active_border.unwrap_or(t.border);
    let (hb, hf) = (t.border_strong, t.fg0);
    div()
        .id(id.into())
        .px(px(7.))
        .py(px(3.))
        .rounded(px(t.r_xs))
        .bg(rgb(t.button_bg))
        .border_1()
        .border_color(rgb(border))
        .hover(move |st| st.border_color(rgb(hb)).text_color(rgb(hf)))
        .cursor_pointer()
        .text_size(px(t.text_data_sm))
        .text_color(rgb(color))
        .child(label.into())
        .on_click(move |_ev, window, cx| on_click(window, cx))
}

fn pool_status_badge(
    label: impl Into<SharedString>,
    color: u32,
    bg: u32,
    t: &Theme,
) -> impl IntoElement {
    div()
        .px(px(6.))
        .py(px(1.))
        .rounded(px(t.r_xs))
        .bg(rgb(bg))
        .text_size(px(t.text_data_sm))
        .text_color(rgb(color))
        .child(label.into())
}

fn pool_deployment_row(
    idx: usize,
    total: usize,
    dep: &repoharbor_core::model::ModelDeployment,
    pool_keys_map: &std::collections::HashMap<String, String>,
    editing: bool,
    t: &Theme,
    app: &Entity<RepoHarborApp>,
) -> impl IntoElement {
    let is_enabled = dep.enabled;
    let (resolved_base, resolved_model) =
        repoharbor_core::cloud::resolve_deployment_base_and_model(dep);
    let prov = if dep.provider.trim().is_empty() {
        repoharbor_core::cloud::infer_provider_id(&dep.name, &resolved_base)
    } else {
        dep.provider.clone()
    };
    let display_name = if dep.name.trim().is_empty() {
        prov.clone()
    } else {
        dep.name.clone()
    };

    let has_own_key = pool_keys_map
        .get(&dep.id)
        .is_some_and(|v| !v.trim().is_empty())
        || !dep.api_key.trim().is_empty();
    let is_local = repoharbor_core::cloud::url_is_loopback(&resolved_base);
    let has_prov_key = !repoharbor_core::cloud::keys_for_target(&prov).is_empty();

    let (key_label, key_color) = if has_own_key {
        ("own key", t.clean)
    } else if is_local {
        ("local (no key)", t.primary)
    } else if has_prov_key {
        ("provider key", t.fg2)
    } else {
        ("no key", t.behind)
    };

    let (state_label, state_color) = if !is_enabled {
        ("disabled".to_string(), t.fg3)
    } else if let Some(rem) =
        repoharbor_core::cloud::cooldown_remaining_for_deployment(dep, pool_keys_map)
    {
        (format!("cooling {}s", rem.as_secs().max(1)), t.dirty)
    } else {
        ("ready".to_string(), t.clean)
    };

    let mut actions = div().flex().flex_row().items_center().gap(px(4.));
    if idx > 0 {
        let (app, id) = (app.clone(), dep.id.clone());
        actions = actions.child(pool_chip(
            format!("pool-up-{idx}-{}", dep.id),
            "↑",
            t.fg1,
            None,
            t,
            move |_, cx| {
                let id = id.clone();
                app.update(cx, |this, cx| this.pool_move(&id, -1, cx));
            },
        ));
    }
    if idx + 1 < total {
        let (app, id) = (app.clone(), dep.id.clone());
        actions = actions.child(pool_chip(
            format!("pool-dn-{idx}-{}", dep.id),
            "↓",
            t.fg1,
            None,
            t,
            move |_, cx| {
                let id = id.clone();
                app.update(cx, |this, cx| this.pool_move(&id, 1, cx));
            },
        ));
    }
    {
        let (app, id) = (app.clone(), dep.id.clone());
        actions = actions.child(pool_chip(
            format!("pool-tg-{idx}-{}", dep.id),
            if is_enabled { "Enabled" } else { "Disabled" },
            if is_enabled { t.clean } else { t.fg3 },
            None,
            t,
            move |_, cx| {
                let id = id.clone();
                app.update(cx, |this, cx| this.pool_toggle(&id, cx));
            },
        ));
    }
    {
        let (app, id) = (app.clone(), dep.id.clone());
        actions = actions.child(pool_chip(
            format!("pool-ed-{idx}-{}", dep.id),
            "Edit",
            if editing { t.primary } else { t.fg1 },
            editing.then_some(t.primary),
            t,
            move |window, cx| {
                let id = id.clone();
                app.update(cx, |this, cx| this.pool_edit(&id, window, cx));
            },
        ));
    }
    {
        let (app, id) = (app.clone(), dep.id.clone());
        actions = actions.child(pool_chip(
            format!("pool-ts-{idx}-{}", dep.id),
            "Test",
            t.fg1,
            None,
            t,
            move |_, cx| {
                let id = id.clone();
                app.update(cx, |this, cx| this.pool_test(&id, cx));
            },
        ));
    }
    {
        let (app, id) = (app.clone(), dep.id.clone());
        actions = actions.child(pool_chip(
            format!("pool-rm-{idx}-{}", dep.id),
            "✕",
            t.behind,
            None,
            t,
            move |_, cx| {
                let id = id.clone();
                app.update(cx, |this, cx| this.pool_remove(&id, cx));
            },
        ));
    }

    div()
        .flex()
        .flex_row()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap(px(8.))
        .py(px(6.))
        .px(px(8.))
        .rounded(px(t.r_xs))
        .bg(rgb(if is_enabled { t.page } else { t.surface_hover }))
        .border_1()
        .border_color(rgb(if editing {
            t.primary
        } else if is_enabled {
            t.border
        } else {
            t.button_bg
        }))
        .child(
            div()
                .flex()
                .flex_1()
                .min_w(px(220.))
                .flex_row()
                .items_center()
                .gap(px(8.))
                .child(pool_status_badge(
                    format!("#{}", idx + 1),
                    t.primary,
                    t.accent_wash,
                    t,
                ))
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_w(px(0.))
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .flex_wrap()
                                .items_center()
                                .gap(px(6.))
                                .child(
                                    div()
                                        .text_size(px(t.text_data_sm))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(rgb(if is_enabled { t.fg0 } else { t.fg3 }))
                                        .child(format!("{display_name} · {resolved_model}")),
                                )
                                .child(pool_status_badge(key_label, key_color, t.button_bg, t))
                                .child(pool_status_badge(state_label, state_color, t.button_bg, t)),
                        )
                        .child(
                            div()
                                .truncate()
                                .font_family("monospace")
                                .text_size(px(t.text_data_sm))
                                .text_color(rgb(t.fg3))
                                .child(resolved_base),
                        ),
                ),
        )
        .child(actions)
}

/// Interactive Provider List View: ordered list + add/edit form + quick presets.
fn model_pool_block(s: &SettingsState, t: &Theme, app: &Entity<RepoHarborApp>) -> impl IntoElement {
    let pool_keys_map = repoharbor_core::pool_keys::load();
    let total = s.draft.model_pool.len();
    let editing_id = s.pool_edit.as_deref();

    let mut block = div()
        .flex()
        .flex_col()
        .gap(px(10.))
        .p(px(10.))
        .rounded(px(t.r_sm))
        .bg(rgb(t.surface))
        .border_1()
        .border_color(rgb(t.border));

    let header = div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(8.))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .text_size(px(t.text_small))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgb(t.fg0))
                        .child("Provider List View (Failover Cascade)"),
                )
                .child(pool_status_badge(
                    format!("{total} configured"),
                    t.fg2,
                    t.button_bg,
                    t,
                )),
        )
        .child({
            let app = app.clone();
            button("Reset Cooldowns", t, move |cx| {
                repoharbor_core::cloud::clear_cooldowns();
                app.update(cx, |this, cx| {
                    if let Some(st) = &mut this.settings {
                        st.pool_note = "Cooldown timers cleared.".into();
                    }
                    cx.notify();
                });
            })
        });

    block = block.child(header).child(note_line(
        SharedString::from(
            "Priority cascade (#1 tried first). On 429 rate-limit, 402 quota, auth failure, or timeout, RepoHarbor moves to the next enabled row (with a 60s cooldown on 429/402/503).",
        ),
        t.fg3,
        t,
    ));

    if s.draft.model_pool.is_empty() {
        block = block.child(
            div()
                .p(px(8.))
                .rounded(px(t.r_xs))
                .bg(rgb(t.page))
                .border_1()
                .border_color(rgb(t.border))
                .text_size(px(t.text_data_sm))
                .text_color(rgb(t.fg3))
                .child("List is empty — add custom providers below or click a preset. When empty, RepoHarbor uses the active backend above plus auto-failover."),
        );
    } else {
        let mut list = div().flex().flex_col().gap(px(6.));
        for (idx, dep) in s.draft.model_pool.iter().enumerate() {
            let is_editing = editing_id == Some(dep.id.as_str());
            list = list.child(pool_deployment_row(
                idx,
                total,
                dep,
                &pool_keys_map,
                is_editing,
                t,
                app,
            ));
        }
        block = block.child(list);
    }

    // Add / Edit custom provider form.
    let is_editing = editing_id.is_some();
    let app_submit = app.clone();
    let submit_btn = div()
        .id("btn-pool-submit")
        .px(px(14.))
        .py(px(7.))
        .rounded(px(t.r_sm))
        .bg(rgb(t.accent_wash))
        .border_1()
        .border_color(rgb(t.primary))
        .text_size(px(t.text_data_sm))
        .text_color(rgb(t.fg0))
        .cursor_pointer()
        .child(if is_editing {
            "Save row changes"
        } else {
            "+ Add provider to list"
        })
        .on_click(move |_ev, window, cx| {
            app_submit.update(cx, |this, cx| this.pool_add_or_update(window, cx));
        });

    let mut form_actions = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.))
        .child(submit_btn);
    if is_editing {
        let app_cancel = app.clone();
        form_actions = form_actions.child(pool_chip(
            "btn-pool-cancel-edit",
            "Cancel edit",
            t.fg2,
            None,
            t,
            move |window, cx| {
                app_cancel.update(cx, |this, cx| this.pool_cancel_edit(window, cx));
            },
        ));
    }

    let form = div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .p(px(8.))
        .rounded(px(t.r_xs))
        .bg(rgb(t.page))
        .border_1()
        .border_color(rgb(t.border))
        .child(field_label(
            if is_editing {
                "Edit provider deployment"
            } else {
                "Add custom provider / model / key"
            },
            t,
        ))
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap(px(8.))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(200.))
                        .child(Input::new(&s.pool_name)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(200.))
                        .child(Input::new(&s.pool_model)),
                ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap(px(8.))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(200.))
                        .child(Input::new(&s.pool_base)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(200.))
                        .child(Input::new(&s.pool_key)),
                ),
        )
        .child(form_actions);

    block = block.child(form);

    if !s.pool_note.is_empty() {
        let is_err = s.pool_note.contains("failed")
            || s.pool_note.contains("required")
            || s.pool_note.contains("HTTP")
            || s.pool_note.contains("https");
        block = block.child(note_line(
            s.pool_note.clone(),
            if is_err { t.dirty } else { t.clean },
            t,
        ));
    }

    // Quick-add presets (extended).
    let presets = [
        (
            "Groq Fast",
            "groq",
            repoharbor_core::cloud::GROQ_BASE_URL,
            "openai/gpt-oss-120b",
        ),
        (
            "Mistral Codestral",
            "mistral",
            repoharbor_core::cloud::MISTRAL_BASE_URL,
            "codestral-latest",
        ),
        (
            "OpenRouter DeepSeek",
            "openrouter",
            repoharbor_core::cloud::OPENROUTER_BASE_URL,
            "deepseek/deepseek-chat",
        ),
        (
            "Cerebras",
            "cerebras",
            repoharbor_core::cloud::CEREBRAS_BASE_URL,
            "llama-3.3-70b",
        ),
        (
            "DeepSeek",
            "deepseek",
            repoharbor_core::cloud::DEEPSEEK_BASE_URL,
            "deepseek-chat",
        ),
        (
            "Together",
            "together",
            repoharbor_core::cloud::TOGETHER_BASE_URL,
            "meta-llama/Llama-3.3-70B-Instruct-Turbo",
        ),
        (
            "Local Ollama",
            "ollama",
            repoharbor_core::cloud::OLLAMA_OPENAI_BASE_URL,
            "qwen3:0.6b",
        ),
    ];

    let mut preset_row = div()
        .flex()
        .flex_row()
        .flex_wrap()
        .gap(px(6.))
        .items_center();
    preset_row = preset_row.child(field_label("Quick presets:", t));
    for (label, prov, base, model) in presets {
        let app = app.clone();
        preset_row = preset_row.child(pool_chip(
            format!("add-preset-{label}"),
            format!("+ {label}"),
            t.fg1,
            None,
            t,
            move |_, cx| {
                app.update(cx, |this, cx| {
                    this.pool_add_preset(label, prov, base, model, cx);
                });
            },
        ));
    }
    block.child(preset_row)
}

/// API-key row for Cloud / OpenRouter / Groq: a masked field plus a Clear button. The
/// stored key is never rendered back, so an empty field means "keep as is".
fn cloud_key_row(
    s: &SettingsState,
    t: &Theme,
    app: &Entity<RepoHarborApp>,
    target: &str,
) -> impl IntoElement {
    let app_clear = app.clone();
    let label = match target {
        "groq" => "Groq API key(s) (supports multiple comma/newline-separated keys)",
        "mistral" => "Mistral API key(s) (supports multiple comma/newline-separated keys)",
        "openrouter" => "OpenRouter API key(s)",
        _ => "API key(s) (stored outside config.toml)",
    };
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(field_label(label, t))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.))
                .child(div().flex_1().child(Input::new(&s.openai_key)))
                .child(button("Clear key", t, move |cx| {
                    app_clear.update(cx, |this, cx| this.ai_clear_cloud_key(cx));
                })),
        )
}

/// Quick 1-click model presets for Groq.
fn groq_model_presets(
    s: &SettingsState,
    t: &Theme,
    app: &Entity<RepoHarborApp>,
) -> impl IntoElement {
    let presets: [(&'static str, &'static str); 4] = [
        ("gpt-oss 120B (Smart)", "openai/gpt-oss-120b"),
        ("gpt-oss 20B (Fast)", "openai/gpt-oss-20b"),
        ("Qwen 27B (Instant)", "qwen/qwen3.8-27b"),
        ("Allam 7B (Bilingual)", "allam-2-7b"),
    ];
    let mut row = div()
        .flex()
        .flex_row()
        .flex_wrap()
        .gap(px(6.))
        .items_center();
    for (label, model_id) in presets {
        let (app, ai_model) = (app.clone(), s.ai_model.clone());
        row = row.child(
            div()
                .id(SharedString::from(format!("groq-preset-{label}")))
                .px(px(8.))
                .py(px(3.))
                .rounded(px(t.r_xs))
                .bg(rgb(t.button_bg))
                .border_1()
                .border_color(rgb(t.border))
                .hover(|st| st.bg(rgb(t.surface_hover)).border_color(rgb(t.primary)))
                .cursor_pointer()
                .text_size(px(t.text_data_sm))
                .text_color(rgb(t.fg1))
                .child(SharedString::from(label))
                .on_click(move |_ev, window, cx| {
                    ai_model.update(cx, |st, cx| st.set_value(model_id.to_string(), window, cx));
                    app.update(cx, |this, _cx| {
                        if let Some(s) = &mut this.settings {
                            s.draft.ai_model = model_id.to_string();
                        }
                    });
                }),
        );
    }
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(field_label("Recommended models", t))
        .child(row)
}

/// Quick 1-click model presets for OpenRouter.
fn openrouter_model_presets(
    s: &SettingsState,
    t: &Theme,
    app: &Entity<RepoHarborApp>,
) -> impl IntoElement {
    let presets: [(&'static str, &'static str); 5] = [
        ("DeepSeek V3", "deepseek/deepseek-chat"),
        ("Llama 3.3 70B", "meta-llama/llama-3.3-70b-instruct"),
        ("GPT-4o Mini", "openai/gpt-4o-mini"),
        ("Claude 3.5 Haiku", "anthropic/claude-3.5-haiku"),
        ("Gemini 2.0 Flash", "google/gemini-2.0-flash-exp:free"),
    ];
    let mut row = div()
        .flex()
        .flex_row()
        .flex_wrap()
        .gap(px(6.))
        .items_center();
    for (label, model_id) in presets {
        let (app, ai_model) = (app.clone(), s.ai_model.clone());
        row = row.child(
            div()
                .id(SharedString::from(format!("openrouter-preset-{label}")))
                .px(px(8.))
                .py(px(3.))
                .rounded(px(t.r_xs))
                .bg(rgb(t.button_bg))
                .border_1()
                .border_color(rgb(t.border))
                .hover(|st| st.bg(rgb(t.surface_hover)).border_color(rgb(t.primary)))
                .cursor_pointer()
                .text_size(px(t.text_data_sm))
                .text_color(rgb(t.fg1))
                .child(SharedString::from(label))
                .on_click(move |_ev, window, cx| {
                    ai_model.update(cx, |st, cx| st.set_value(model_id.to_string(), window, cx));
                    app.update(cx, |this, _cx| {
                        if let Some(s) = &mut this.settings {
                            s.draft.ai_model = model_id.to_string();
                        }
                    });
                }),
        );
    }
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(field_label("Recommended models", t))
        .child(row)
}

/// Quick 1-click model presets for Mistral AI.
fn mistral_model_presets(
    s: &SettingsState,
    t: &Theme,
    app: &Entity<RepoHarborApp>,
) -> impl IntoElement {
    let presets: [(&'static str, &'static str); 4] = [
        ("Codestral (Code & Commits)", "codestral-latest"),
        ("Mistral Large (Smart)", "mistral-large-latest"),
        ("Mistral Small (Fast)", "mistral-small-latest"),
        ("Ministral 8B (Compact)", "ministral-8b-latest"),
    ];
    let mut row = div()
        .flex()
        .flex_row()
        .flex_wrap()
        .gap(px(6.))
        .items_center();
    for (label, model_id) in presets {
        let (app, ai_model) = (app.clone(), s.ai_model.clone());
        row = row.child(
            div()
                .id(SharedString::from(format!("mistral-preset-{label}")))
                .px(px(8.))
                .py(px(3.))
                .rounded(px(t.r_xs))
                .bg(rgb(t.button_bg))
                .border_1()
                .border_color(rgb(t.border))
                .hover(|st| st.bg(rgb(t.surface_hover)).border_color(rgb(t.primary)))
                .cursor_pointer()
                .text_size(px(t.text_data_sm))
                .text_color(rgb(t.fg1))
                .child(SharedString::from(label))
                .on_click(move |_ev, window, cx| {
                    ai_model.update(cx, |st, cx| st.set_value(model_id.to_string(), window, cx));
                    app.update(cx, |this, _cx| {
                        if let Some(s) = &mut this.settings {
                            s.draft.ai_model = model_id.to_string();
                        }
                    });
                }),
        );
    }
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(field_label("Recommended models", t))
        .child(row)
}

/// Six-way Ollama / llama.cpp / Groq / Mistral / OpenRouter / Cloud backend picker. Takes effect on Save.
fn backend_picker(backend: &str, t: &Theme, app: &Entity<RepoHarborApp>) -> impl IntoElement {
    let current: SharedString = if backend_is_llama(backend) {
        "llamaCpp".into()
    } else if backend_is_groq(backend) {
        "groq".into()
    } else if backend_is_mistral(backend) {
        "mistral".into()
    } else if backend_is_openrouter(backend) {
        "openrouter".into()
    } else if backend_is_cloud(backend) {
        "cloud".into()
    } else {
        "ollama".into()
    };
    let opt = |label: &'static str, value: &'static str| {
        let app = app.clone();
        let on = current.as_ref() == value;
        div()
            .id(label)
            .px(px(12.))
            .py(px(6.))
            .rounded(px(t.r_sm))
            .bg(rgb(if on { t.accent_wash } else { t.button_bg }))
            .border_1()
            .border_color(rgb(if on { t.primary } else { t.border }))
            .text_size(px(t.text_data_sm))
            .text_color(rgb(if on { t.fg0 } else { t.fg2 }))
            .cursor_pointer()
            .child(label)
            .on_click(move |_ev, window, cx| {
                app.update(cx, |this, cx| {
                    let changed = this
                        .settings
                        .as_ref()
                        .is_some_and(|s| s.draft.ai_backend != value);
                    if let Some(s) = &mut this.settings {
                        s.draft.ai_backend = value.into();
                        if value == "groq" {
                            let cur_base = s.openai_base.read(cx).value();
                            if cur_base.is_empty()
                                || cur_base == "https://openrouter.ai/api/v1"
                                || cur_base == "https://ollama.com/v1"
                            {
                                s.openai_base.update(cx, |st, cx| {
                                    st.set_value(
                                        repoharbor_core::cloud::GROQ_BASE_URL.to_string(),
                                        window,
                                        cx,
                                    );
                                });
                            }
                            let cur_model = s.ai_model.read(cx).value();
                            if cur_model.is_empty()
                                || cur_model.starts_with("deepseek/")
                                || cur_model.starts_with("meta-llama/")
                            {
                                s.ai_model.update(cx, |st, cx| {
                                    st.set_value("openai/gpt-oss-120b".to_string(), window, cx);
                                });
                            }
                        } else if value == "openrouter" {
                            let cur_base = s.openai_base.read(cx).value();
                            if cur_base.is_empty() || cur_base == "https://api.groq.com/openai/v1" {
                                s.openai_base.update(cx, |st, cx| {
                                    st.set_value(
                                        repoharbor_core::cloud::OPENROUTER_BASE_URL.to_string(),
                                        window,
                                        cx,
                                    );
                                });
                            }
                            let cur_model = s.ai_model.read(cx).value();
                            if cur_model.is_empty() || cur_model.starts_with("openai/gpt-oss") {
                                s.ai_model.update(cx, |st, cx| {
                                    st.set_value("deepseek/deepseek-chat".to_string(), window, cx);
                                });
                            }
                        } else if value == "mistral" {
                            let cur_base = s.openai_base.read(cx).value();
                            if cur_base.is_empty()
                                || cur_base == "https://api.groq.com/openai/v1"
                                || cur_base == "https://openrouter.ai/api/v1"
                                || cur_base == "https://ollama.com/v1"
                            {
                                s.openai_base.update(cx, |st, cx| {
                                    st.set_value(
                                        repoharbor_core::cloud::MISTRAL_BASE_URL.to_string(),
                                        window,
                                        cx,
                                    );
                                });
                            }
                            let cur_model = s.ai_model.read(cx).value();
                            if cur_model.is_empty()
                                || cur_model.starts_with("openai/gpt-oss")
                                || cur_model.starts_with("deepseek/")
                                || cur_model.starts_with("meta-llama/")
                            {
                                s.ai_model.update(cx, |st, cx| {
                                    st.set_value("codestral-latest".to_string(), window, cx);
                                });
                            }
                        }
                    }
                    // The live status describes the *saved* backend, so keeping
                    // it would advertise the old engine's models under the newly
                    // picked one. Drop it until Save applies the change.
                    if changed {
                        this.invalidate_ai_status(cx);
                    }
                    cx.notify();
                });
            })
    };
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(field_label("Backend", t))
        .child(
            div()
                .flex()
                .flex_row()
                .gap(px(6.))
                .child(opt("Ollama", "ollama"))
                .child(opt("llama.cpp", "llamaCpp"))
                .child(opt("Groq", "groq"))
                .child(opt("Mistral", "mistral"))
                .child(opt("OpenRouter", "openrouter"))
                .child(opt("Cloud", "cloud")),
        )
}

/// GGUF download row: a URL field + a Download button (progress shows in the AI
/// note line below). Only shown for the llama.cpp backend.
fn llama_download_row(
    s: &SettingsState,
    t: &Theme,
    app: &Entity<RepoHarborApp>,
) -> impl IntoElement {
    let (app, url_input) = (app.clone(), s.llama_url.clone());
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(field_label("Download GGUF model", t))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.))
                .child(div().flex_1().child(Input::new(&s.llama_url)))
                .child(button("Download", t, move |cx| {
                    let url = url_input.read(cx).value().to_string();
                    app.update(cx, |this, cx| this.llama_download(url, cx));
                })),
        )
}

/// Read-only display of the currently-selected GGUF model.
fn llama_model_line(s: &SettingsState, t: &Theme) -> impl IntoElement {
    let sel = s
        .draft
        .llama_model_path
        .rsplit('/')
        .next()
        .filter(|x| !x.is_empty())
        .unwrap_or("none — download or pick a model below");
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.))
        .child(field_label("Selected model", t))
        .child(
            div()
                .font_family("monospace")
                .text_size(px(t.text_data_sm))
                .text_color(rgb(t.fg1))
                .child(SharedString::from(sel.to_string())),
        )
}

/// The live AI-backend status row + installed models + refresh/pull actions.
fn ai_status_block(
    s: &SettingsState,
    ai: &AiStatus,
    t: &Theme,
    app: &Entity<RepoHarborApp>,
) -> impl IntoElement {
    let is_llama = backend_is_llama(&s.draft.ai_backend);
    let is_openrouter = backend_is_openrouter(&s.draft.ai_backend);
    let is_groq = backend_is_groq(&s.draft.ai_backend);
    let is_mistral = backend_is_mistral(&s.draft.ai_backend);
    let is_cloud = backend_is_cloud(&s.draft.ai_backend);
    let is_remote = is_openrouter || is_groq || is_mistral || is_cloud;
    let mut block = div().flex().flex_col().gap(px(8.)).pt(px(4.));

    let head = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(10.))
        .child(div().flex_1().child(match ai {
            AiStatus::Unknown => status_row("clock", "Status unknown".to_string(), t.fg3, t),
            AiStatus::Checking => status_row("clock", "Checking…".to_string(), t.fg3, t),
            AiStatus::Offline => status_row(
                "circle-alert",
                "Backend unreachable".to_string(),
                t.behind,
                t,
            ),
            AiStatus::Ready(m) => status_row(
                "circle-check",
                format!(
                    "Reachable — {} model{}",
                    m.len(),
                    if m.len() == 1 { "" } else { "s" }
                ),
                t.clean,
                t,
            ),
            AiStatus::Pulling(name) => status_row("clock", format!("Pulling {name}…"), t.fg2, t),
        }))
        .child(button("Test", t, {
            let app = app.clone();
            move |cx| {
                app.update(cx, |this, cx| this.ai_test(cx));
            }
        }))
        .child(button("Clear cache", t, {
            let app = app.clone();
            move |cx| {
                app.update(cx, |this, cx| this.ai_clear_cache(cx));
            }
        }))
        .child(button("Refresh", t, {
            let app = app.clone();
            move |cx| {
                app.update(cx, |this, cx| this.ai_refresh(cx));
            }
        }));
    block = block.child(head);
    block = block.child(note_line(
        SharedString::from(if is_groq {
            "Test checks Groq LPU connectivity and your key. Fast hosted models like openai/gpt-oss-120b, openai/gpt-oss-20b, and qwen3.8-27b draft commit messages in 15-60 milliseconds."
        } else if is_openrouter {
            "Test checks OpenRouter connectivity and your key. Fast hosted models like deepseek/deepseek-chat or meta-llama/llama-3.3-70b-instruct draft commit messages in ~1 second."
        } else if is_remote {
            "Test checks the endpoint and key. Small hosted models (e.g. llama-3.1-8b-instant on Groq) draft a commit message in well under a second."
        } else {
            "If Generate commit fails with a runner / 500 error, free GPU VRAM (or wait — RepoHarbor retries on CPU) and restart Ollama if needed. Prefer qwen2.5:3b when VRAM allows; qwen3:0.6b is a lighter fallback."
        }),
        t.fg3,
        t,
    ));
    if !s.ai_note.is_empty() {
        block = block.child(note_line(s.ai_note.clone(), t.fg2, t));
    }

    if let AiStatus::Ready(models) = ai {
        if models.is_empty() {
            block = block.child(note_line(
                if is_remote {
                    "No models offered by this endpoint.".into()
                } else {
                    "No models installed.".into()
                },
                t.fg3,
                t,
            ));
        } else {
            block = block.child(note_line("Click a model to use it.".into(), t.fg3, t));
            for (name, size) in models {
                // Clicking a model selects it: the GGUF path for llama.cpp, or the
                // chat-model name for Ollama/Cloud (a lightweight picker over the list).
                let (app, ai_model, pick) = (app.clone(), s.ai_model.clone(), name.clone());
                block = block.child(
                    div()
                        .id(SharedString::from(format!("ai-model-{name}")))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.))
                        .px(px(6.))
                        .py(px(3.))
                        .rounded(px(t.r_sm))
                        .font_family("monospace")
                        .text_size(px(t.text_data_sm))
                        .cursor_pointer()
                        .hover(|s| s.bg(rgb(t.surface_hover)))
                        .child(lucide("box", 12., t.fg3))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .truncate()
                                .text_color(rgb(t.fg1))
                                .child(name.clone()),
                        )
                        .children((!is_remote).then(|| super::muted_mono(size.clone(), t)))
                        .on_click(move |_ev, window, cx| {
                            let pick = pick.clone();
                            if is_llama {
                                let path = repoharbor_core::llama::models_dir()
                                    .map(|d| d.join(pick.as_ref()).to_string_lossy().into_owned());
                                app.update(cx, |this, cx| {
                                    if let (Some(s), Some(path)) = (&mut this.settings, path) {
                                        s.draft.llama_model_path = path;
                                    }
                                    cx.notify();
                                });
                            } else {
                                ai_model
                                    .update(cx, |st, cx| st.set_value(pick.clone(), window, cx));
                                app.update(cx, |this, _cx| {
                                    if let Some(s) = &mut this.settings {
                                        s.draft.ai_model = pick.to_string();
                                    }
                                });
                            }
                        }),
                );
            }
        }
        // Ollama can pull the configured chat model from the registry; the
        // llama.cpp backend downloads GGUFs via the field above instead.
        let model = s.draft.ai_model.clone();
        if !is_llama && !is_remote && !model.trim().is_empty() {
            block = block.child(div().child(button(&format!("Pull \"{model}\""), t, {
                let app = app.clone();
                move |cx| {
                    let model = model.clone();
                    app.update(cx, |this, cx| this.ai_pull(model.clone(), cx));
                }
            })));
        }
    }
    block
}

fn notifications_section(
    s: &SettingsState,
    autostart_enabled: bool,
    t: &Theme,
    app: &Entity<RepoHarborApp>,
) -> impl IntoElement {
    let app_auto = app.clone();
    let app_desk = app.clone();
    section(t, "Notifications & Startup")
        .child(toggle(
            "Background notifications",
            s.draft.notify_enabled,
            t,
            app,
            |a| {
                if let Some(s) = &mut a.settings {
                    s.draft.notify_enabled = !s.draft.notify_enabled;
                }
            },
        ))
        .child(toggle(
            "Urgent attention items",
            s.draft.notify_attention,
            t,
            app,
            |a| {
                if let Some(s) = &mut a.settings {
                    s.draft.notify_attention = !s.draft.notify_attention;
                }
            },
        ))
        .child(toggle(
            "New pull request",
            s.draft.notify_new_pr,
            t,
            app,
            |a| {
                if let Some(s) = &mut a.settings {
                    s.draft.notify_new_pr = !s.draft.notify_new_pr;
                }
            },
        ))
        .child(toggle(
            "Review requested",
            s.draft.notify_review_requested,
            t,
            app,
            |a| {
                if let Some(s) = &mut a.settings {
                    s.draft.notify_review_requested = !s.draft.notify_review_requested;
                }
            },
        ))
        .child(toggle(
            "CI failure",
            s.draft.notify_ci_failure,
            t,
            app,
            |a| {
                if let Some(s) = &mut a.settings {
                    s.draft.notify_ci_failure = !s.draft.notify_ci_failure;
                }
            },
        ))
        .child(toggle(
            "Agent finished",
            s.draft.notify_agent_finished,
            t,
            app,
            |a| {
                if let Some(s) = &mut a.settings {
                    s.draft.notify_agent_finished = !s.draft.notify_agent_finished;
                }
            },
        ))
        .child({
            use gpui_component::switch::Switch;
            div()
                .pt(px(8.))
                .border_t_1()
                .border_color(rgb(t.border))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(t.text_small))
                        .text_color(rgb(t.fg1))
                        .child("Launch automatically on desktop login (~/.config/autostart)"),
                )
                .child(
                    Switch::new("tg-autostart-login")
                        .checked(autostart_enabled)
                        .color(rgb(t.primary))
                        .on_click(move |_checked, _win, cx| {
                            app_auto.update(cx, |this, cx| this.toggle_autostart(cx));
                        }),
                )
        })
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap(px(8.))
                .child(button_id(
                    "btn-repair-desktop",
                    "Install / repair desktop shortcut & icon",
                    t,
                    move |cx| {
                        app_desk.update(cx, |this, cx| this.repair_desktop_shortcut(cx));
                    },
                )),
        )
}

fn about_section(
    install_kind: &repoharbor_core::updater::InstallKind,
    update_state: &crate::shell::AppUpdateState,
    auto_check_updates: bool,
    t: &Theme,
    app: &Entity<RepoHarborApp>,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(16.))
        .child(about_product(t))
        .child(about_updates(
            install_kind,
            update_state,
            auto_check_updates,
            t,
            app,
        ))
        .child(about_contact(t))
        .child(about_license(t))
        .child(about_attribution(t))
}

fn about_updates(
    install_kind: &repoharbor_core::updater::InstallKind,
    update_state: &crate::shell::AppUpdateState,
    auto_check_updates: bool,
    t: &Theme,
    app: &Entity<RepoHarborApp>,
) -> impl IntoElement {
    use crate::shell::AppUpdateState;
    use gpui_component::switch::Switch;

    let mut box_el = section(t, "Updates & installation")
        .child(about_value_row("Install mode", &install_kind.label(), t))
        .child({
            let app_sw = app.clone();
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(t.text_small))
                        .text_color(rgb(t.fg1))
                        .child("Check for periodic updates automatically"),
                )
                .child(
                    Switch::new("tg-auto-check-updates")
                        .checked(auto_check_updates)
                        .color(rgb(t.primary))
                        .on_click(move |_checked, _win, cx| {
                            app_sw.update(cx, |this, cx| this.toggle_auto_check_updates(cx));
                        }),
                )
        });

    let (status_text, status_color) = match update_state {
        AppUpdateState::Idle => (
            SharedString::from("Not checked yet in this session."),
            t.fg3,
        ),
        AppUpdateState::Checking => (SharedString::from("Checking GitHub Releases…"), t.primary),
        AppUpdateState::UpToDate(tag) => {
            let suffix = tag
                .as_deref()
                .map(|t| format!(" (latest release: {t})"))
                .unwrap_or_default();
            (
                SharedString::from(format!(
                    "RepoHarbor v{} is up to date{suffix}.",
                    crate::identity::APP_VERSION
                )),
                t.clean,
            )
        }
        AppUpdateState::Available(rel) => (
            SharedString::from(format!(
                "Update available: {} (current: v{}).",
                rel.tag_name,
                crate::identity::APP_VERSION
            )),
            t.clean,
        ),
        AppUpdateState::Installing => (
            SharedString::from("Downloading, verifying SHA-256, and installing update…"),
            t.primary,
        ),
        AppUpdateState::Error(err) => (
            SharedString::from(format!("Update check failed: {err}")),
            t.behind,
        ),
    };
    box_el = box_el.child(note_line(status_text, status_color, t));

    if let AppUpdateState::Available(rel) = update_state
        && !rel.notes.is_empty()
    {
        box_el = box_el.child(
            div()
                .p(px(10.))
                .rounded(px(t.r_sm))
                .bg(rgb(t.page))
                .border_1()
                .border_color(rgb(t.border))
                .font_family("monospace")
                .text_size(px(t.text_data_sm))
                .text_color(rgb(t.fg2))
                .child(SharedString::from(
                    rel.notes.chars().take(600).collect::<String>(),
                )),
        );
    }

    let app_chk = app.clone();
    let app_apply = app.clone();
    let mut actions_row = div()
        .flex()
        .flex_row()
        .flex_wrap()
        .gap(px(8.))
        .child(button_id(
            "btn-check-updates",
            "Check for updates",
            t,
            move |cx| {
                app_chk.update(cx, |this, cx| this.check_for_updates(cx));
            },
        ))
        .child(button_id(
            "btn-open-releases",
            "Open Releases page",
            t,
            |_cx| {
                let _ =
                    repoharbor_core::launch::open("https://github.com/azmykn/RepoHarbor/releases");
            },
        ));

    if let AppUpdateState::Available(_) = update_state {
        let label = match install_kind {
            repoharbor_core::updater::InstallKind::SystemPackage(_) => {
                "Run user installer in terminal"
            }
            _ => "Update & Restart",
        };
        actions_row =
            actions_row.child(button_win("btn-apply-update", label, t, move |win, cx| {
                app_apply.update(cx, |this, cx| this.apply_app_update(win, cx));
            }));
    }

    let ext_cmd = install_kind.external_update_command();
    let ext_cmd_copy = ext_cmd.clone();
    box_el.child(actions_row).child(
        div()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .child(note_line(
                SharedString::from(format!("CLI update: `{ext_cmd}`")),
                t.fg3,
                t,
            ))
            .child(button_id(
                "btn-copy-update-cmd",
                "Copy CLI command",
                t,
                move |cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(ext_cmd_copy.clone()));
                },
            )),
    )
}

fn about_product(t: &Theme) -> impl IntoElement {
    use crate::identity as id;
    section(t, "Product")
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.))
                .child(
                    div()
                        .text_color(rgb(t.fg0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(t.text_h3))
                        .child(id::APP_NAME),
                )
                .child(
                    div()
                        .px(px(8.))
                        .py(px(2.))
                        .rounded(px(t.r_xs))
                        .bg(rgb(t.button_bg))
                        .border_1()
                        .border_color(rgb(t.border))
                        .font_family("monospace")
                        .text_size(px(t.text_data_sm))
                        .text_color(rgb(t.primary))
                        .child(SharedString::from(format!("v{}", id::APP_VERSION))),
                ),
        )
        .child(note_line(
            SharedString::from(
                "every repo at harbor — a Linux-native command center for your git fleet",
            ),
            t.fg2,
            t,
        ))
        .child(note_line(
            SharedString::from(format!(
                "Publisher: {} · Author: {}",
                id::PUBLISHER,
                id::AUTHOR
            )),
            t.fg1,
            t,
        ))
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap(px(8.))
                .child(button_id("btn-about-repo", "Open repository", t, |_cx| {
                    let _ = repoharbor_core::launch::open(id::REPOSITORY);
                }))
                .child(button_id(
                    "btn-about-docs",
                    "Open documentation",
                    t,
                    |_cx| {
                        let _ = repoharbor_core::launch::open(id::DOCS_URL);
                    },
                ))
                .child(button_id(
                    "btn-about-changelog",
                    "Open changelog",
                    t,
                    |_cx| {
                        let _ = repoharbor_core::launch::open(id::CHANGELOG_URL);
                    },
                )),
        )
}

fn about_contact(t: &Theme) -> impl IntoElement {
    use crate::identity as id;
    section(t, "Contact")
        .child(about_value_row("Email", id::CONTACT_EMAIL, t))
        .child(about_value_row("Phone", id::CONTACT_PHONE, t))
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap(px(8.))
                .child(button_id("btn-about-email", "Email", t, |_cx| {
                    let _ = repoharbor_core::launch::open(&format!("mailto:{}", id::CONTACT_EMAIL));
                }))
                .child(button_id("btn-about-copy-email", "Copy email", t, |cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(id::CONTACT_EMAIL.to_string()));
                }))
                .child(button_id("btn-about-copy-phone", "Copy phone", t, |cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(id::CONTACT_PHONE.to_string()));
                })),
        )
}

fn about_license(t: &Theme) -> impl IntoElement {
    use crate::identity as id;
    section(t, "License & public use")
        .child(note_line(
            SharedString::from(format!(
                "© {} {} / {} — {}.",
                id::COPYRIGHT_YEAR,
                id::AUTHOR,
                id::PUBLISHER,
                id::LICENSE_SPDX
            )),
            t.fg1,
            t,
        ))
        .child(note_line(SharedString::from(id::PUBLIC_USE), t.fg2, t))
        .child(note_line(
            SharedString::from(
                "Full terms: LICENSE, NOTICE, and COPYRIGHT.md in the repository root.",
            ),
            t.fg3,
            t,
        ))
}

fn about_attribution(t: &Theme) -> impl IntoElement {
    use crate::identity as id;
    section(t, "Attribution").child(note_line(SharedString::from(id::UPSTREAM_CREDIT), t.fg2, t))
}

fn about_value_row(label: &str, value: &str, t: &Theme) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_baseline()
        .gap(px(8.))
        .child(field_label(label, t))
        .child(
            div()
                .font_family("monospace")
                .text_size(px(t.text_data_sm))
                .text_color(rgb(t.fg0))
                .child(SharedString::from(value.to_string())),
        )
}

fn save_footer(s: &SettingsState, t: &Theme, app: &Entity<RepoHarborApp>) -> impl IntoElement {
    let app2 = app.clone();
    let mut bar = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(12.))
        .h(px(56.))
        .px(px(20.))
        .border_t_1()
        .border_color(rgb(t.border))
        .child(div().flex_1());
    if s.saved {
        bar = bar.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(5.))
                .text_size(px(t.text_data_sm))
                .text_color(rgb(t.clean))
                .child(lucide("check", 14., t.clean))
                .child("Saved"),
        );
    }
    bar.child(button("Save & rescan", t, move |cx| {
        app2.update(cx, |this, cx| this.settings_save(cx));
    }))
}

// ── building blocks ─────────────────────────────────────────────────────────

fn section(t: &Theme, title: &str) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(px(10.))
        .p(px(16.))
        .rounded(px(t.r_md))
        .bg(rgb(t.surface))
        .border_1()
        .border_color(rgb(t.border))
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(t.text_small))
                .text_color(rgb(t.fg0))
                .child(SharedString::from(title.to_string())),
        )
}

/// A labelled text-input row.
fn labeled(label: &str, input: Entity<InputState>, t: &Theme) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(field_label(label, t))
        .child(Input::new(&input))
}

fn field_label(label: &str, t: &Theme) -> impl IntoElement {
    div()
        .text_size(px(t.text_data_sm))
        .text_color(rgb(t.fg3))
        .child(SharedString::from(label.to_string()))
}

/// A status icon + label row (connection / reachability).
fn status_row(
    icon: &str,
    label: impl Into<SharedString>,
    color: u32,
    t: &Theme,
) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.))
        .child(lucide(icon, 14., color))
        .child(
            div()
                .text_size(px(t.text_small))
                .text_color(rgb(t.fg1))
                .child(label.into()),
        )
}

/// A small coloured note line (status / hint).
fn note_line(text: SharedString, color: u32, t: &Theme) -> impl IntoElement {
    div()
        .text_size(px(t.text_data_sm))
        .text_color(rgb(color))
        .child(text)
}

/// A label + gpui-component on/off [`Switch`] row. `flip` toggles the bound
/// draft field; the switch reflects `on`.
fn toggle(
    label: &str,
    on: bool,
    t: &Theme,
    app: &Entity<RepoHarborApp>,
    flip: impl Fn(&mut RepoHarborApp) + 'static,
) -> impl IntoElement {
    use gpui_component::switch::Switch;
    let app = app.clone();
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(10.))
        .child(
            div()
                .flex_1()
                .text_size(px(t.text_small))
                .text_color(rgb(t.fg1))
                .child(SharedString::from(label.to_string())),
        )
        .child(
            Switch::new(SharedString::from(format!("tg-{label}")))
                .checked(on)
                .color(rgb(t.primary))
                .on_click(move |_checked, _window, cx| {
                    app.update(cx, |this, cx| {
                        flip(this);
                        cx.notify();
                    });
                }),
        )
}

/// A label + gpui-component [`NumberInput`] row (used for scan depth). Stepping +
/// validation live in the bound `InputState`; the value is read back on save.
fn number_row(label: &str, input: &Entity<InputState>, t: &Theme) -> impl IntoElement {
    use gpui_component::input::NumberInput;
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(10.))
        .child(
            div()
                .flex_1()
                .text_size(px(t.text_small))
                .text_color(rgb(t.fg1))
                .child(SharedString::from(label.to_string())),
        )
        .child(div().w(px(120.)).child(NumberInput::new(input)))
}

fn button(label: &str, t: &Theme, on: impl Fn(&mut gpui::App) + 'static) -> impl IntoElement {
    button_id(format!("btn-{label}"), label, t, on)
}

fn button_id(
    id: impl Into<SharedString>,
    label: &str,
    t: &Theme,
    on: impl Fn(&mut gpui::App) + 'static,
) -> impl IntoElement {
    let (hb, hf) = (t.border_strong, t.fg0);
    div()
        .id(id.into())
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
        .child(SharedString::from(label.to_string()))
        .on_click(move |_ev, _win, cx| on(cx))
}

fn button_win(
    id: impl Into<SharedString>,
    label: &str,
    t: &Theme,
    on: impl Fn(&mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let (hb, hf) = (t.border_strong, t.fg0);
    div()
        .id(id.into())
        .px(px(14.))
        .py(px(7.))
        .rounded(px(t.r_sm))
        .bg(rgb(t.accent_wash))
        .border_1()
        .border_color(rgb(t.border_accent))
        .text_size(px(t.text_data_sm))
        .text_color(rgb(t.accent_bright))
        .cursor_pointer()
        .hover(move |s| s.border_color(rgb(hb)).text_color(rgb(hf)))
        .child(SharedString::from(label.to_string()))
        .on_click(move |_ev, win, cx| on(win, cx))
}
