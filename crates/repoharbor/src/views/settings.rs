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
    pub client_id: Entity<InputState>,
    pub ignore: Entity<InputState>,
    pub add_root: Entity<InputState>,
    /// Typed path to append to `draft.pull_only_prefixes`.
    pub add_pull_only: Entity<InputState>,
    /// External diff tool template (`{path}` / `{file}`).
    pub diff_command: Entity<InputState>,
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
                "https://api.groq.com/openai/v1",
                &cfg.openai_base,
            ),
            openai_key: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(if repoharbor_core::cloud::has_api_key() {
                        "•••••• (stored — type to replace)"
                    } else {
                        "API key"
                    })
                    .masked(true)
            }),
            client_id: field(window, cx, "GitHub OAuth client id", &cfg.github_client_id),
            ignore: field(window, cx, "node_modules, .cache", &cfg.ignore.join(", ")),
            add_root: field(window, cx, "~/Projects or ~/code/my-app", ""),
            add_pull_only: field(window, cx, "~/Projects/upstream/core", ""),
            diff_command: field(window, cx, "meld {path}", &cfg.diff_command),
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
        sections.push(launchers_section(s, t).into_any_element());
    }
    if show("ai") {
        sections.push(ai_section(s, ai, ai_ready, t, app).into_any_element());
    }
    if show("notifications") {
        sections.push(notifications_section(s, t, app).into_any_element());
    }
    if show("about") {
        sections.push(about_section(t).into_any_element());
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
                "Repos under these prefixes: Pull to update, hide Push, demote upstream CI. Your digits modules stay Pushable when outside this list.",
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

    // Scan depth stepper + ignore field.
    col = col.child(number_row("Scan depth", &s.scan_depth, t));
    col.child(labeled("Ignore (comma-separated)", s.ignore.clone(), t))
}

fn launchers_section(s: &SettingsState, t: &Theme) -> impl IntoElement {
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
}

/// True when the configured backend string selects the llama.cpp sidecar.
fn backend_is_llama(backend: &str) -> bool {
    matches!(backend, "llamaCpp" | "llama_cpp" | "llamacpp")
}

/// True when the configured backend string selects the OpenAI-compatible
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
    if is_cloud {
        sec = sec
            .child(labeled("Base URL", s.openai_base.clone(), t))
            .child(labeled("Chat model", s.ai_model.clone(), t))
            .child(cloud_key_row(s, t, app))
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
        .child(labeled("GitHub OAuth client id", s.client_id.clone(), t))
        .child(ai_status_block(s, is_llama, is_cloud, ai, t, app));
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

/// API-key row for the cloud backend: a masked field plus a Clear button. The
/// stored key is never rendered back, so an empty field means "keep as is".
fn cloud_key_row(s: &SettingsState, t: &Theme, app: &Entity<RepoHarborApp>) -> impl IntoElement {
    let app_clear = app.clone();
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(field_label("API key (stored outside config.toml)", t))
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

/// Three-way Ollama / llama.cpp / Cloud backend picker. Takes effect on Save.
fn backend_picker(backend: &str, t: &Theme, app: &Entity<RepoHarborApp>) -> impl IntoElement {
    let current: SharedString = if backend_is_llama(backend) {
        "llamaCpp".into()
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
            .on_click(move |_ev, _win, cx| {
                app.update(cx, |this, cx| {
                    let changed = this
                        .settings
                        .as_ref()
                        .is_some_and(|s| s.draft.ai_backend != value);
                    if let Some(s) = &mut this.settings {
                        s.draft.ai_backend = value.into();
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
    is_llama: bool,
    is_cloud: bool,
    ai: &AiStatus,
    t: &Theme,
    app: &Entity<RepoHarborApp>,
) -> impl IntoElement {
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
        SharedString::from(if is_cloud {
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
                if is_cloud {
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
                // chat-model name for Ollama (a lightweight picker over the list).
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
                        .children((!is_cloud).then(|| super::muted_mono(size.clone(), t)))
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
        if !is_llama && !is_cloud && !model.trim().is_empty() {
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
    t: &Theme,
    app: &Entity<RepoHarborApp>,
) -> impl IntoElement {
    section(t, "Notifications")
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
}

fn about_section(t: &Theme) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(16.))
        .child(about_product(t))
        .child(about_contact(t))
        .child(about_license(t))
        .child(about_attribution(t))
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
