use gpui_kit::component::input::Input;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::sidebar::kind_icon;
use super::{Screen, Status, View, Workspace, modal};
use crate::theme::{MONO, tokens};
use crate::time_range::{PRESETS, TimeRange};
use crate::ui::widgets::kbd;
use crate::ui::{Icon, icon};

#[derive(Clone)]
pub enum Action {
    Go(Screen),
    ToggleView,
    ToggleFollow,
    AddSource,
    ToggleTheme,
    ClearStream,
    Reconnect,
    Solo(String),
    Range(TimeRange),
}

#[derive(Clone)]
pub struct Command {
    group: &'static str,
    label: String,
    glyph: Icon,
    shortcut: &'static str,
    pub action: Action,
}

impl Workspace {
    /// Commands matching the palette query, in display order.
    pub(super) fn palette_commands(&self, cx: &App) -> Vec<Command> {
        let command = |group, label: &str, glyph, shortcut, action| Command {
            group,
            label: label.to_string(),
            glyph,
            shortcut,
            action,
        };
        let mut all = vec![
            command("Navigate", "Live stream", Icon::Pulse, "", Action::Go(Screen::Stream)),
            command("Navigate", "Sources", Icon::Plugs, "", Action::Go(Screen::Sources)),
            command("Navigate", "Storage", Icon::Database, "", Action::Go(Screen::Storage)),
            command("Navigate", "Team", Icon::UsersThree, "", Action::Go(Screen::Team)),
            command("Navigate", "Agents", Icon::Broadcast, "", Action::Go(Screen::Agents)),
            command(
                "Actions",
                if self.view == View::Rows {
                    "Switch to JSON view"
                } else {
                    "Switch to rows view"
                },
                Icon::BracketsCurly,
                "⌘J",
                Action::ToggleView,
            ),
            command(
                "Actions",
                if self.follow { "Pause following" } else { "Follow tail" },
                Icon::ArrowLineDown,
                "",
                Action::ToggleFollow,
            ),
            command("Actions", "Add source…", Icon::Plus, "", Action::AddSource),
            command(
                "Actions",
                if tokens(cx).dark {
                    "Switch to light theme"
                } else {
                    "Switch to dark theme"
                },
                Icon::CircleHalf,
                "",
                Action::ToggleTheme,
            ),
            command("Actions", "Clear stream", Icon::Broom, "", Action::ClearStream),
        ];
        all.extend(PRESETS.iter().map(|(window, label)| Command {
            group: "Time range",
            label: label.to_string(),
            glyph: Icon::ClockCounterClockwise,
            shortcut: "",
            action: Action::Range(TimeRange::Last(*window)),
        }));
        all.push(command(
            "Time range",
            "All time",
            Icon::ClockCounterClockwise,
            "",
            Action::Range(TimeRange::All),
        ));
        if matches!(self.status, Status::Disconnected { .. }) {
            all.push(command(
                "Actions",
                "Reconnect server",
                Icon::PlugsConnected,
                "",
                Action::Reconnect,
            ));
        }
        all.extend(
            self.targets
                .iter()
                .filter(|target| target.state == "running")
                .map(|target| Command {
                    group: "Tail only…",
                    label: super::display_name(target.source, &target.name, &target.labels),
                    glyph: kind_icon(target.source),
                    shortcut: "",
                    action: Action::Solo(target.name.clone()),
                }),
        );

        let query = self.palette_input.read(cx).value().to_lowercase();
        all.into_iter()
            .filter(|c| {
                query.is_empty() || c.label.to_lowercase().contains(&query) || c.group.to_lowercase().contains(&query)
            })
            .collect()
    }

    pub(super) fn run(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            Action::Go(screen) => self.go(screen, cx),
            Action::ToggleView => {
                let view = if self.view == View::Rows {
                    View::Json
                } else {
                    View::Rows
                };
                self.set_view(view, cx);
            }
            Action::ToggleFollow => self.set_follow(!self.follow, cx),
            Action::AddSource => self.open_add_source(cx),
            Action::ToggleTheme => self.toggle_theme(cx),
            Action::ClearStream => self.clear_stream(cx),
            Action::Reconnect => {
                self.client.retry_now();
                self.status = Status::Connecting;
                self.go(Screen::Stream, cx);
            }
            Action::Solo(origin) => self.solo(&origin, cx),
            Action::Range(range) => {
                self.screen = Screen::Stream;
                self.set_range(range, cx);
            }
        }
        self.focus.focus(window, cx);
    }

    pub(super) fn render_palette(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = tokens(cx);
        let commands = self.palette_commands(cx);
        let mut items: Vec<AnyElement> = Vec::new();
        let mut last_group = "";
        for (i, command) in commands.iter().enumerate() {
            if command.group != last_group {
                last_group = command.group;
                items.push(
                    div()
                        .px(px(12.))
                        .pt(px(10.))
                        .pb(px(6.))
                        .text_size(px(11.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(t.fg3)
                        .child(command.group)
                        .into_any_element(),
                );
            }
            let action = command.action.clone();
            items.push(
                div()
                    .id(("command", i))
                    .h(px(40.))
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .rounded(px(8.))
                    .cursor_pointer()
                    .when(i == self.palette_index, |el| el.bg(t.bg3))
                    .on_mouse_move(cx.listener(move |this, _, _, cx| {
                        if this.palette_index != i {
                            this.palette_index = i;
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.close_overlays(window, cx);
                        this.run(action.clone(), window, cx);
                    }))
                    .child(icon(command.glyph, 16., t.fg2))
                    .child(div().flex_1().text_size(px(13.5)).child(command.label.clone()))
                    .child(
                        div()
                            .font_family(MONO)
                            .text_size(px(11.))
                            .text_color(t.fg3)
                            .child(command.shortcut),
                    )
                    .into_any_element(),
            );
        }
        if commands.is_empty() {
            items.push(
                div()
                    .p(px(28.))
                    .flex()
                    .justify_center()
                    .text_size(px(13.))
                    .text_color(t.fg3)
                    .child("No results")
                    .into_any_element(),
            );
        }

        let panel = div()
            .w(px(600.))
            .rounded(px(12.))
            .border_1()
            .border_color(t.line2)
            .bg(t.bg)
            .shadow_2xl()
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .h(px(54.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px(px(18.))
                    .border_b_1()
                    .border_color(t.line)
                    .text_size(px(15.))
                    .child(icon(Icon::MagnifyingGlass, 17., t.fg3))
                    .child(div().flex_1().child(Input::new(&self.palette_input).appearance(false)))
                    .child(kbd(t, "esc")),
            )
            .child(
                div()
                    .id("commands")
                    .max_h(px(380.))
                    .overflow_y_scroll()
                    .p(px(6.))
                    .children(items),
            )
            .child(
                div()
                    .h(px(38.))
                    .flex()
                    .items_center()
                    .gap(px(16.))
                    .px(px(16.))
                    .border_t_1()
                    .border_color(t.line)
                    .bg(t.bg2)
                    .text_size(px(11.5))
                    .text_color(t.fg3)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(icon(Icon::ArrowsDownUp, 12., t.fg3))
                            .child("Navigate"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(icon(Icon::ArrowElbowDownLeft, 12., t.fg3))
                            .child("Run"),
                    )
                    .child(div().flex_1())
                    .child(div().font_family(MONO).child("⌘J toggle JSON")),
            );
        modal(t, panel, Some(110.), cx)
    }
}
