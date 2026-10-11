use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::collections::BTreeMap;

use telelog_core::{SourceKind, Target};

use super::{Screen, Status, Workspace, namespace_of};
use crate::servers;
use crate::theme::{MONO, tokens};
use crate::ui::canvas::brand_mark;
use crate::ui::widgets::{checkbox, checkbox_mixed, dot, halo_dot, kbd, pill};
use crate::ui::{Icon, icon};

pub fn kind_label(kind: SourceKind) -> &'static str {
    match kind {
        SourceKind::Docker => "Docker",
        SourceKind::Kubernetes => "Kubernetes",
        SourceKind::Vm => "Virtual machines",
    }
}

pub fn kind_icon(kind: SourceKind) -> Icon {
    match kind {
        SourceKind::Docker => Icon::Cube,
        SourceKind::Kubernetes => Icon::SteeringWheel,
        SourceKind::Vm => Icon::HardDrives,
    }
}

pub const KINDS: [SourceKind; 3] = [SourceKind::Docker, SourceKind::Kubernetes, SourceKind::Vm];

/// The local account name, shown where the design has the signed-in user.
pub fn user_name() -> String {
    std::env::var("USER")
        .ok()
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| "you".into())
}

impl Workspace {
    pub(super) fn status_color(&self, cx: &App) -> Hsla {
        let t = tokens(cx);
        match self.status {
            Status::Connected => t.ok,
            Status::Connecting => t.fg3,
            Status::Disconnected { .. } => t.err,
        }
    }

    fn nav_item(
        &self,
        screen: Screen,
        label: &'static str,
        glyph: Icon,
        badge: Option<&'static str>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = tokens(cx);
        let active = self.screen == screen;
        let group: SharedString = format!("nav-{label}").into();
        div()
            .id(label)
            .group(group.clone())
            .h(px(32.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(10.))
            .rounded(px(7.))
            .text_size(px(13.5))
            .cursor_pointer()
            .when(active, |el| {
                el.bg(t.bg4).text_color(t.fg).font_weight(FontWeight::MEDIUM)
            })
            .when(!active, |el| {
                el.text_color(t.fg2).hover(|s| s.bg(t.bg3).text_color(t.fg))
            })
            .on_click(cx.listener(move |this, _, _, cx| this.go(screen, cx)))
            .child(icon(glyph, 16., if active { t.fg } else { t.fg2 }).group_hover(group, |s| s.text_color(t.fg)))
            .child(div().flex_1().child(label))
            .children(badge.map(|b| pill(t, b)))
    }

    fn render_source_groups(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let t = tokens(cx);
        let groups: Vec<_> = KINDS
            .into_iter()
            .filter(|kind| self.targets.iter().any(|target| target.source == *kind))
            .collect();
        if groups.is_empty() {
            let note = match self.status {
                Status::Connected => "No targets on this server yet.",
                _ => "Targets appear once the server connects.",
            };
            return vec![
                div()
                    .px(px(10.))
                    .py(px(6.))
                    .text_size(px(12.))
                    .text_color(t.fg3)
                    .child(note)
                    .into_any_element(),
            ];
        }

        groups
            .into_iter()
            .map(|kind| {
                let targets: Vec<_> = self.targets.iter().filter(|target| target.source == kind).collect();
                let on = targets.iter().filter(|target| self.is_enabled(&target.name)).count();
                let open = self.open_groups.contains(&kind);
                let header = div()
                    .id(SharedString::from(format!("group-{}", kind.as_str())))
                    .h(px(30.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .rounded(px(7.))
                    .text_size(px(13.))
                    .cursor_pointer()
                    .hover(|s| s.bg(t.bg3))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.open_groups.remove(&kind) {
                            this.open_groups.insert(kind);
                        }
                        cx.notify();
                    }))
                    .child(
                        icon(Icon::CaretRight, 11., t.fg3).with_transformation(Transformation::rotate(if open {
                            percentage(0.25)
                        } else {
                            percentage(0.)
                        })),
                    )
                    .child(icon(kind_icon(kind), 15., t.fg2))
                    .child(div().flex_1().child(kind_label(kind)))
                    .child(
                        div()
                            .font_family(MONO)
                            .text_size(px(11.))
                            .text_color(t.fg3)
                            .child(format!("{on}/{}", targets.len())),
                    );

                let rows = open.then(|| {
                    let rows: Vec<AnyElement> = if kind == SourceKind::Kubernetes {
                        self.render_namespaces(targets, cx)
                    } else {
                        targets
                            .into_iter()
                            .map(|target| self.render_target(target, 30., cx).into_any_element())
                            .collect()
                    };
                    div().flex().flex_col().pt(px(2.)).pb(px(6.)).children(rows)
                });
                div().flex().flex_col().child(header).children(rows).into_any_element()
            })
            .collect()
    }

    fn render_target(&self, target: &Target, indent: f32, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = tokens(cx);
        let on = self.is_enabled(&target.name);
        let name = target.name.clone();
        div()
            .id(SharedString::from(format!("target-{}", target.id)))
            .h(px(27.))
            .pl(px(indent))
            .pr(px(10.))
            .flex()
            .items_center()
            .gap(px(9.))
            .rounded(px(6.))
            .cursor_pointer()
            .hover(|s| s.bg(t.bg3))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_target(&name, cx)))
            .child(checkbox(t, on))
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .font_family(MONO)
                    .text_size(px(11.5))
                    .text_color(if on { t.fg } else { t.fg3 })
                    .child(super::display_name(target.source, &target.name, &target.labels)),
            )
    }

    /// Kubernetes pods under one collapsible row per namespace, whose checkbox selects them all.
    fn render_namespaces(&self, targets: Vec<&Target>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let t = tokens(cx);
        let mut namespaces: BTreeMap<&str, Vec<&Target>> = BTreeMap::new();
        for target in targets {
            namespaces.entry(namespace_of(&target.name)).or_default().push(target);
        }
        namespaces
            .into_iter()
            .map(|(namespace, targets)| {
                let on = targets.iter().filter(|target| self.is_enabled(&target.name)).count();
                let open = self.open_namespaces.contains(namespace);
                let (toggle, select) = (namespace.to_string(), namespace.to_string());
                let header = div()
                    .id(SharedString::from(format!("namespace-{namespace}")))
                    .h(px(27.))
                    .pl(px(14.))
                    .pr(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .hover(|s| s.bg(t.bg3))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.open_namespaces.remove(&toggle) {
                            this.open_namespaces.insert(toggle.clone());
                        }
                        cx.notify();
                    }))
                    .child(
                        icon(Icon::CaretRight, 10., t.fg3).with_transformation(Transformation::rotate(if open {
                            percentage(0.25)
                        } else {
                            percentage(0.)
                        })),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("namespace-all-{namespace}")))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.toggle_namespace(&select, cx);
                            }))
                            .child(match on {
                                0 => checkbox(t, false),
                                n if n == targets.len() => checkbox(t, true),
                                _ => checkbox_mixed(t),
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .text_size(px(12.5))
                            .text_color(if on > 0 { t.fg } else { t.fg3 })
                            .child(namespace.to_string()),
                    )
                    .child(
                        div()
                            .font_family(MONO)
                            .text_size(px(11.))
                            .text_color(t.fg3)
                            .child(format!("{on}/{}", targets.len())),
                    );
                let pods = open.then(|| {
                    div()
                        .flex()
                        .flex_col()
                        .children(targets.into_iter().map(|target| self.render_target(target, 40., cx)))
                });
                div().flex().flex_col().child(header).children(pods).into_any_element()
            })
            .collect()
    }

    /// Saved servers and Telelog Cloud collectors to switch to, and ways to add more.
    fn render_server_menu(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = tokens(cx);
        let status_color = self.status_color(cx);
        let open_id = self.open.saved.as_ref().map(|server| server.id.clone());

        let row = |id: SharedString| {
            div()
                .id(id.clone())
                .group(id)
                .h(px(40.))
                .px(px(10.))
                .flex()
                .items_center()
                .gap(px(10.))
                .rounded(px(6.))
                .cursor_pointer()
                .hover(|s| s.bg(t.bg3))
        };
        let label = |name: String, address: String| {
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .text_size(px(12.5))
                        .child(name),
                )
                .child(
                    div()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .font_family(MONO)
                        .text_size(px(11.))
                        .text_color(t.fg3)
                        .child(address),
                )
        };

        let heading = |title: &'static str| {
            div()
                .px(px(10.))
                .pt(px(6.))
                .pb(px(4.))
                .text_size(px(11.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(t.fg3)
                .child(title)
        };
        let note = |text: String| {
            div()
                .px(px(10.))
                .py(px(6.))
                .text_size(px(12.))
                .text_color(t.fg3)
                .child(text)
                .into_any_element()
        };

        let mut items: Vec<AnyElement> = Vec::new();
        if self.open.is_unsaved() {
            // The server given on the command line, until it's saved.
            let connection = self.open.connection.clone();
            items.push(
                row("server-unsaved".into())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_add_server(Some(connection.clone()), window, cx)
                    }))
                    .child(dot(status_color, 7.))
                    .child(label(self.open.name(), self.server_addr().to_string()))
                    .child(div().text_size(px(12.)).text_color(t.fg2).child("Save…"))
                    .into_any_element(),
            );
        }
        for server in &self.servers.list {
            let current = open_id.as_deref() == Some(server.id.as_str());
            let group: SharedString = format!("server-{}", server.id).into();
            let (switch, remove) = (server.clone(), server.id.clone());
            items.push(
                row(group.clone())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.open.saved.as_ref() == Some(&switch) {
                            this.server_menu_open = false;
                            cx.notify();
                        } else {
                            this.switch_server(switch.open(), cx);
                        }
                    }))
                    .child(dot(if current { status_color } else { t.line2 }, 7.))
                    .child(label(server.name.clone(), servers::address(&server.url).to_string()))
                    .when(current, |el| el.child(icon(Icon::Check, 13., t.fg)))
                    .child(
                        div()
                            .id(SharedString::from(format!("remove-server-{}", server.id)))
                            .invisible()
                            .group_hover(group, |s| s.visible())
                            .size(px(22.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(5.))
                            .hover(|s| s.bg(t.bg4))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.remove_server(&remove, cx);
                            }))
                            .child(icon(Icon::X, 12., t.fg3)),
                    )
                    .into_any_element(),
            );
        }

        let mut cloud: Vec<AnyElement> = Vec::new();
        if let Some(account) = &self.servers.cloud {
            cloud.push(
                div()
                    .mt(px(4.))
                    .pr(px(10.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(heading("Telelog Cloud"))
                    .child(
                        div()
                            .id("cloud-sign-out")
                            .text_size(px(11.5))
                            .text_color(t.fg3)
                            .cursor_pointer()
                            .hover(|s| s.text_color(t.fg))
                            .on_click(cx.listener(|this, _, _, cx| this.sign_out_cloud(cx)))
                            .child("Sign out"),
                    )
                    .into_any_element(),
            );
            match &self.cloud_collectors {
                None => cloud.push(note("Loading collectors…".into())),
                Some(Err(e)) => cloud.push(note(format!("Couldn’t list collectors: {e}"))),
                Some(Ok(list)) if list.is_empty() => cloud.push(note(
                    "No collectors yet. Enroll one with telelog-collector --enroll.".into(),
                )),
                Some(Ok(list)) => {
                    for collector in list {
                        let current = self.open.cloud.as_ref().is_some_and(|open| open.id == collector.id);
                        let pick = servers::CloudCollector {
                            id: collector.id.clone(),
                            name: collector.name.clone(),
                            host: collector.host.clone(),
                        };
                        let account = account.clone();
                        let place = if collector.online {
                            collector.host.clone()
                        } else {
                            format!("{} · offline", collector.host)
                        };
                        let color = match (current, collector.online) {
                            (true, _) => status_color,
                            (false, true) => t.ok,
                            (false, false) => t.line2,
                        };
                        cloud.push(
                            row(format!("collector-{}", collector.id).into())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if current {
                                        this.server_menu_open = false;
                                        cx.notify();
                                    } else {
                                        this.switch_server(account.open(pick.clone()), cx);
                                    }
                                }))
                                .child(dot(color, 7.))
                                .child(label(collector.name.clone(), place))
                                .when(current, |el| el.child(icon(Icon::Check, 13., t.fg)))
                                .into_any_element(),
                        );
                    }
                }
            }
        }

        div()
            .p(px(6.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .rounded(px(10.))
            .border_1()
            .border_color(t.line2)
            .bg(t.bg)
            .shadow_lg()
            .occlude()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.server_menu_open = false;
                cx.notify();
            }))
            .child(heading("Servers"))
            .children(items)
            .children(cloud)
            .child(div().my(px(4.)).h(px(1.)).bg(t.line))
            .child(
                row("add-server".into())
                    .h(px(34.))
                    .text_size(px(12.5))
                    .text_color(t.fg2)
                    .on_click(cx.listener(|this, _, window, cx| this.open_add_server(None, window, cx)))
                    .child(icon(Icon::Plus, 13., t.fg2))
                    .child("Add server…"),
            )
            .when(self.servers.cloud.is_none(), |el| {
                el.child(
                    row("cloud-sign-in".into())
                        .h(px(34.))
                        .text_size(px(12.5))
                        .text_color(t.fg2)
                        .on_click(cx.listener(|this, _, window, cx| this.open_cloud_sign_in(window, cx)))
                        .child(icon(Icon::Broadcast, 13., t.fg2))
                        .child("Sign in to Telelog Cloud…"),
                )
            })
    }

    pub(super) fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = tokens(cx);
        let status_color = self.status_color(cx);
        let user = user_name();
        let initial = user.chars().next().map(|c| c.to_ascii_uppercase()).unwrap_or('?');
        let version = env!("CARGO_PKG_VERSION")
            .rsplit_once('.')
            .map(|(v, _)| v)
            .unwrap_or("0");

        let brand = div()
            .px(px(16.))
            .pt(px(16.))
            .pb(px(12.))
            .flex()
            .items_center()
            .gap(px(10.))
            .child(brand_mark(t, matches!(self.status, Status::Connecting)))
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("telelogs"),
            )
            .child(
                div()
                    .px(px(6.))
                    .py(px(1.))
                    .rounded_full()
                    .border_1()
                    .border_color(t.line2)
                    .font_family(MONO)
                    .text_size(px(10.5))
                    .text_color(t.fg3)
                    .child(version.to_string()),
            );

        let server_card = div()
            .id("server-card")
            .w_full()
            .px(px(10.))
            .py(px(8.))
            .flex()
            .items_center()
            .gap(px(10.))
            .rounded(px(8.))
            .border_1()
            .border_color(if self.server_menu_open { t.fg3 } else { t.line2 })
            .bg(t.bg)
            .cursor_pointer()
            .hover(|s| s.bg(t.bg3))
            // The menu closes itself on any click outside it, this card included.
            .on_click(cx.listener(|this, _, _, cx| {
                this.server_menu_open = true;
                this.fetch_collectors(cx);
                cx.notify();
            }))
            .child(halo_dot(t, status_color, matches!(self.status, Status::Connected)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .child(
                        div()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .text_size(px(12.5))
                            .font_weight(FontWeight::MEDIUM)
                            .child(self.open.name()),
                    )
                    .child(
                        div()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .font_family(MONO)
                            .text_size(px(11.))
                            .text_color(t.fg3)
                            .child(self.server_addr().to_string()),
                    ),
            )
            .child(icon(Icon::CaretUpDown, 13., t.fg3));
        let server_card = div().relative().child(server_card).when(self.server_menu_open, |el| {
            el.child(
                deferred(
                    div()
                        .absolute()
                        .top(px(54.))
                        .left_0()
                        .right_0()
                        .child(self.render_server_menu(cx)),
                )
                .with_priority(1),
            )
        });

        let search = div()
            .id("open-search")
            .group("open-search")
            .w_full()
            .h(px(32.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded(px(8.))
            .text_size(px(13.))
            .text_color(t.fg2)
            .cursor_pointer()
            .hover(|s| s.bg(t.bg3).text_color(t.fg))
            .on_click(cx.listener(|this, _, window, cx| this.open_palette(window, cx)))
            .child(icon(Icon::MagnifyingGlass, 15., t.fg2).group_hover("open-search", |s| s.text_color(t.fg)))
            .child(div().flex_1().child("Search"))
            .child(div().flex().gap(px(3.)).child(kbd(t, "⌘")).child(kbd(t, "K")));

        let nav = div()
            .px(px(10.))
            .pt(px(10.))
            .pb(px(4.))
            .flex()
            .flex_col()
            .gap(px(1.))
            .child(self.nav_item(Screen::Stream, "Live stream", Icon::Pulse, None, cx))
            .child(self.nav_item(Screen::Sources, "Sources", Icon::Plugs, None, cx))
            .child(self.nav_item(Screen::Storage, "Storage", Icon::Database, None, cx))
            .child(self.nav_item(Screen::Team, "Team", Icon::UsersThree, None, cx))
            .child(self.nav_item(Screen::Agents, "Agents", Icon::Broadcast, Some("Soon"), cx));

        let sources_header = div()
            .px(px(20.))
            .pt(px(14.))
            .pb(px(6.))
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .text_size(px(11.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(t.fg3)
                    .child("Log sources"),
            )
            .child(
                div()
                    .id("add-source")
                    .group("add-source")
                    .size(px(20.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(5.))
                    .cursor_pointer()
                    .hover(|s| s.bg(t.bg3))
                    .on_click(cx.listener(|this, _, _, cx| this.open_add_source(cx)))
                    .child(icon(Icon::Plus, 13., t.fg3).group_hover("add-source", |s| s.text_color(t.fg))),
            );

        let footer = div()
            .px(px(12.))
            .py(px(10.))
            .flex()
            .items_center()
            .gap(px(10.))
            .border_t_1()
            .border_color(t.line)
            .child(
                div()
                    .size(px(28.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(t.fg)
                    .text_color(t.bg)
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(initial.to_string()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).child(user))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(t.fg3)
                            .child("Local workspace · Owner"),
                    ),
            )
            .child(icon(Icon::GearSix, 16., t.fg3));

        div()
            .w(px(244.))
            .flex_none()
            .flex()
            .flex_col()
            .min_h_0()
            .bg(t.bg2)
            .border_r_1()
            .border_color(t.line)
            .child(brand)
            .child(
                div()
                    .px(px(10.))
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(server_card)
                    .child(search),
            )
            .child(nav)
            .child(sources_header)
            .child(
                div()
                    .id("source-groups")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(10.))
                    .pb(px(10.))
                    .children(self.render_source_groups(cx)),
            )
            .child(footer)
    }
}
