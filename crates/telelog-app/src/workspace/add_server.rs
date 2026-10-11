//! The Add server dialog (name, URL, token and CA certificate) and the Telelog Cloud sign-in
//! (gateway, token and CA certificate), each with a test before saving.

use std::path::PathBuf;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{Workspace, modal};
use crate::client::{self, Connection};
use crate::servers;
use crate::theme::{MONO, Tokens, tokens};
use crate::ui::widgets::{icon_button, outline_button, primary_button};
use crate::ui::{Icon, icon};

/// Which of the two the dialog is.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Dialog {
    Server,
    Cloud,
}

/// Where the dialog's connection test stands.
pub enum Check {
    Idle,
    Testing,
    Passed(usize),
    Failed(String),
}

pub struct Inputs {
    name: Entity<InputState>,
    url: Entity<InputState>,
    token: Entity<InputState>,
    ca_cert: Entity<InputState>,
    /// A test result describes the fields as they were, so editing one clears it.
    _subscriptions: Vec<Subscription>,
}

impl Inputs {
    pub fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("prod cluster"));
        let url = cx.new(|cx| InputState::new(window, cx).placeholder("https://logs.example.com:7070"));
        let token = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("from telelog-server gen-token")
                .masked(true)
        });
        let ca_cert = cx.new(|cx| InputState::new(window, cx).placeholder("/path/to/ca.pem"));
        let _subscriptions = [&url, &token, &ca_cert]
            .into_iter()
            .map(|input| {
                cx.subscribe(input, |this: &mut Workspace, _, event, cx| {
                    if let InputEvent::Change = event {
                        this.server_check = Check::Idle;
                        this._server_check = None;
                        cx.notify();
                    }
                })
            })
            .collect();
        Inputs {
            name,
            url,
            token,
            ca_cert,
            _subscriptions,
        }
    }
}

/// What the dialog's fields describe, checked.
struct Fields {
    name: String,
    url: String,
    token: Option<String>,
    ca_cert: Option<PathBuf>,
}

impl Fields {
    fn connection(&self) -> Connection {
        Connection {
            server: self.url.clone(),
            token: client::Token::given(self.token.clone()),
            ca_cert: self.ca_cert.clone(),
            collector: None,
        }
    }
}

fn field(t: Tokens, label: &'static str, hint: Option<&'static str>, input: Input) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(
            div()
                .flex()
                .gap(px(4.))
                .text_size(px(12.5))
                .text_color(t.fg2)
                .child(label)
                .children(hint.map(|h| div().text_color(t.fg3).child(h))),
        )
        .child(
            div()
                .h(px(36.))
                .px(px(12.))
                .flex()
                .items_center()
                .rounded(px(8.))
                .border_1()
                .border_color(t.line2)
                .bg(t.bg)
                .font_family(MONO)
                .text_size(px(12.5))
                .child(div().flex_1().child(input.appearance(false))),
        )
}

impl Workspace {
    /// Opens the dialog empty, or filled in from `from` to save the server that's open.
    pub(super) fn open_add_server(&mut self, from: Option<Connection>, window: &mut Window, cx: &mut Context<Self>) {
        let (url, token, ca_cert) = match from {
            Some(c) => (
                c.server,
                c.token.as_given().unwrap_or_default().to_string(),
                c.ca_cert.map(|p| p.display().to_string()).unwrap_or_default(),
            ),
            None => Default::default(),
        };
        let inputs = &self.server_inputs;
        for (input, value) in [
            (&inputs.name, String::new()),
            (&inputs.url, url),
            (&inputs.token, token),
            (&inputs.ca_cert, ca_cert),
        ] {
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        inputs.name.update(cx, |input, cx| input.focus(window, cx));
        self.server_dialog = Dialog::Server;
        self.server_check = Check::Idle;
        self._server_check = None;
        self.add_server_open = true;
        self.add_open = false;
        self.palette_open = false;
        self.server_menu_open = false;
        cx.notify();
    }

    pub(super) fn open_cloud_sign_in(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_add_server(None, window, cx);
        self.server_dialog = Dialog::Cloud;
        self.server_inputs.url.update(cx, |input, cx| input.focus(window, cx));
    }

    fn server_fields(&self, cx: &App) -> Result<Fields, String> {
        let value = |input: &Entity<InputState>| input.read(cx).value().trim().to_string();
        let url = servers::check_url(&value(&self.server_inputs.url))?;
        let ca_cert = Some(value(&self.server_inputs.ca_cert))
            .filter(|path| !path.is_empty())
            .map(PathBuf::from);
        if let Some(path) = &ca_cert
            && !path.is_file()
        {
            return Err(format!("no file at {}", path.display()));
        }
        let name = Some(value(&self.server_inputs.name))
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| servers::address(&url).to_string());
        let token = Some(value(&self.server_inputs.token)).filter(|token| !token.is_empty());
        if self.server_dialog == Dialog::Cloud && token.is_none() {
            return Err("paste your Telelog Cloud token".into());
        }
        Ok(Fields {
            name,
            url,
            token,
            ca_cert,
        })
    }

    fn test_server(&mut self, cx: &mut Context<Self>) {
        let fields = match self.server_fields(cx) {
            Ok(fields) => fields,
            Err(e) => {
                self.server_check = Check::Failed(e);
                cx.notify();
                return;
            }
        };
        self.server_check = Check::Testing;
        let answer = match self.server_dialog {
            Dialog::Server => client::probe(&self.runtime, fields.connection()),
            Dialog::Cloud => client::probe_cloud(&self.runtime, fields.connection()),
        };
        self._server_check = Some(cx.spawn(async move |this, cx| {
            let result = answer.await.unwrap_or_else(|_| Err("the test stopped".into()));
            let _ = this.update(cx, |this, cx| {
                this.server_check = match result {
                    Ok(targets) => Check::Passed(targets),
                    Err(e) => Check::Failed(e),
                };
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// Saves the server, its token in the Keychain, and switches to it; or signs in to Telelog
    /// Cloud and shows its collectors.
    fn save_server(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let fields = match self.server_fields(cx) {
            Ok(fields) => fields,
            Err(e) => {
                self.server_check = Check::Failed(e);
                cx.notify();
                return;
            }
        };
        if self.server_dialog == Dialog::Cloud {
            if let Err(e) = servers::set_token(servers::CLOUD_TOKEN, fields.token.as_deref()) {
                self.server_check = Check::Failed(e);
                cx.notify();
                return;
            }
            self.servers.cloud = Some(servers::CloudAccount {
                gateway: fields.url,
                ca_cert: fields.ca_cert,
            });
            self.servers.save();
            self.close_overlays(window, cx);
            self.server_menu_open = true;
            self.fetch_collectors(cx);
            return;
        }
        let server = self.servers.add(fields.name, fields.url, fields.ca_cert);
        if let Err(e) = servers::set_token(&server.id, fields.token.as_deref()) {
            self.servers.remove(&server.id);
            self.server_check = Check::Failed(e);
            cx.notify();
            return;
        }
        self.servers.save();
        self.close_overlays(window, cx);
        let saving_open = self.open.is_unsaved() && self.open.connection.server == server.url;
        if saving_open
            && self.open.connection.token.as_given() == fields.token.as_deref()
            && self.open.connection.ca_cert == server.ca_cert
        {
            // The server that's open was saved as it is: keep the connection and the stream.
            self.selections.save(&server.id);
            self.open.saved = Some(server);
            if self.servers.remember(&self.open) {
                self.servers.save();
            }
            cx.notify();
        } else {
            self.switch_server(server.open(), cx);
        }
    }

    pub(super) fn render_add_server(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = tokens(cx);
        let inputs = &self.server_inputs;
        let cloud = self.server_dialog == Dialog::Cloud;

        let form = div()
            .px(px(22.))
            .py(px(18.))
            .flex()
            .flex_col()
            .gap(px(14.))
            .when(!cloud, |el| {
                el.child(field(t, "Name", Some("(optional)"), Input::new(&inputs.name)))
            })
            .child(field(
                t,
                if cloud { "Gateway" } else { "URL" },
                None,
                Input::new(&inputs.url),
            ))
            .child(field(
                t,
                "Token",
                Some(if cloud {
                    "(from your workspace)"
                } else {
                    "(if the server has one)"
                }),
                Input::new(&inputs.token).mask_toggle(),
            ))
            .child(field(
                t,
                "CA certificate",
                Some("(optional, for a self-signed certificate)"),
                Input::new(&inputs.ca_cert),
            ));

        let (glyph, color, message) = match &self.server_check {
            Check::Idle => (
                Icon::LockSimple,
                t.fg3,
                "The token is kept in your Keychain.".to_string(),
            ),
            Check::Testing => (Icon::ArrowsClockwise, t.fg3, "Connecting…".to_string()),
            Check::Passed(n) => {
                let (one, many) = if cloud {
                    ("collector", "collectors")
                } else {
                    ("source", "sources")
                };
                let noun = if *n == 1 { one } else { many };
                (Icon::Check, t.ok, format!("Connected: {n} {noun}."))
            }
            Check::Failed(e) => (Icon::XCircle, t.err, e.clone()),
        };

        let panel = div()
            .w(px(520.))
            .rounded(px(12.))
            .border_1()
            .border_color(t.line2)
            .bg(t.bg)
            .shadow_2xl()
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .px(px(22.))
                    .pt(px(20.))
                    .flex()
                    .items_start()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(
                                div()
                                    .text_size(px(17.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(if cloud {
                                        "Sign in to Telelog Cloud"
                                    } else {
                                        "Add server"
                                    }),
                            )
                            .child(div().text_size(px(13.)).text_color(t.fg2).child(if cloud {
                                "Your workspace's collectors appear in the servers menu."
                            } else {
                                "A telelog-server this app can switch to."
                            })),
                    )
                    .child(
                        icon_button(t, "close-add-server", Icon::X, 15.)
                            .on_click(cx.listener(|this, _, window, cx| this.close_overlays(window, cx))),
                    ),
            )
            .child(form)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(22.))
                    .py(px(14.))
                    .border_t_1()
                    .border_color(t.line)
                    .bg(t.bg2)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .text_size(px(12.5))
                            .text_color(color)
                            .child(icon(glyph, 14., color))
                            .child(div().flex_1().min_w_0().child(message)),
                    )
                    .child(
                        outline_button(t, "test-server", "Test", None)
                            .h(px(34.))
                            .px(px(14.))
                            .text_size(px(13.5))
                            .rounded(px(8.))
                            .on_click(cx.listener(|this, _, _, cx| this.test_server(cx))),
                    )
                    .child(
                        primary_button(t, "save-server", if cloud { "Sign in" } else { "Save" }, None)
                            .on_click(cx.listener(|this, _, window, cx| this.save_server(window, cx))),
                    ),
            );
        modal(t, panel, None, cx)
    }
}
