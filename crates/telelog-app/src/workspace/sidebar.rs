use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use telelog_core::SourceKind;

use super::{Screen, Status, Workspace};
use crate::theme::{MONO, tokens};
use crate::ui::canvas::brand_mark;
use crate::ui::widgets::{checkbox, halo_dot, kbd, pill};
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
                    div()
                        .flex()
                        .flex_col()
                        .pt(px(2.))
                        .pb(px(6.))
                        .children(targets.into_iter().map(|target| {
                            let on = self.is_enabled(&target.name);
                            let name = target.name.clone();
                            div()
                                .id(SharedString::from(format!("target-{}", target.id)))
                                .h(px(27.))
                                .pl(px(30.))
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
                        }))
                });
                div().flex().flex_col().child(header).children(rows).into_any_element()
            })
            .collect()
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
            .w_full()
            .px(px(10.))
            .py(px(8.))
            .flex()
            .items_center()
            .gap(px(10.))
            .rounded(px(8.))
            .border_1()
            .border_color(t.line2)
            .bg(t.bg)
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
                            .text_size(px(12.5))
                            .font_weight(FontWeight::MEDIUM)
                            .child("local server"),
                    )
                    .child(
                        div()
                            .font_family(MONO)
                            .text_size(px(11.))
                            .text_color(t.fg3)
                            .child(self.server_addr().to_string()),
                    ),
            )
            .child(icon(Icon::CaretUpDown, 13., t.fg3));

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
