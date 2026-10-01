//! The time range picker in the stream toolbar.

use std::time::SystemTime;

use chrono::Local;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{BUFFER, Workspace};
use crate::theme::{MONO, Tokens, tokens};
use crate::time_range::{PRESETS, TimeRange, format_local, parse_local};
use crate::ui::widgets::primary_button;
use crate::ui::{Icon, icon};

pub struct RangeInputs {
    from: Entity<InputState>,
    to: Entity<InputState>,
}

impl RangeInputs {
    pub fn new(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        RangeInputs {
            from: cx.new(|cx| InputState::new(window, cx).placeholder("2026-10-01 09:00")),
            to: cx.new(|cx| InputState::new(window, cx).placeholder("now")),
        }
    }
}

impl Workspace {
    pub(super) fn set_range(&mut self, range: TimeRange, cx: &mut Context<Self>) {
        self.range = range;
        self.range_open = false;
        self.range_error = None;
        // A window that ended in the past gets no new lines, so stop jumping to the end.
        if !range.includes_now() {
            self.follow = false;
        }
        self.refilter(cx);
    }

    fn open_range(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.range_open = !self.range_open;
        self.range_error = None;
        if self.range_open {
            // Prefill the custom fields with the current window so it can be nudged.
            let (from, to) = self.range.bounds(SystemTime::now());
            let (from, to) = (from.map(format_local).unwrap_or_default(), to.map(format_local).unwrap_or_default());
            self.range_inputs.from.update(cx, |input, cx| input.set_value(from, window, cx));
            self.range_inputs.to.update(cx, |input, cx| input.set_value(to, window, cx));
        }
        cx.notify();
    }

    fn apply_custom_range(&mut self, cx: &mut Context<Self>) {
        let today = Local::now().date_naive();
        let from = parse_local(&self.range_inputs.from.read(cx).value(), today);
        let to = parse_local(&self.range_inputs.to.read(cx).value(), today);
        match (from, to) {
            (Ok(from), Ok(to)) if from.zip(to).is_some_and(|(f, t)| f > t) => {
                self.range_error = Some("\"From\" is after \"To\".".into());
            }
            (Ok(from), Ok(to)) => {
                let range = if from.is_none() && to.is_none() { TimeRange::All } else { TimeRange::Between { from, to } };
                self.set_range(range, cx);
                return;
            }
            (Err(e), _) | (_, Err(e)) => self.range_error = Some(e),
        }
        cx.notify();
    }

    pub(super) fn close_range(&mut self, cx: &mut Context<Self>) {
        if self.range_open {
            self.range_open = false;
            cx.notify();
        }
    }

    pub(super) fn render_range_picker(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = tokens(cx);
        let active = self.range != TimeRange::All;
        let button = div()
            .id("time-range")
            .h(px(32.))
            .px(px(10.))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(7.))
            .rounded(px(8.))
            .border_1()
            .border_color(if active { t.fg3 } else { t.line2 })
            .bg(if active { t.bg3 } else { t.bg })
            .text_size(px(12.5))
            .font_weight(FontWeight::MEDIUM)
            .text_color(if active { t.fg } else { t.fg2 })
            .cursor_pointer()
            .hover(|s| s.bg(t.bg3))
            .on_click(cx.listener(|this, _, window, cx| this.open_range(window, cx)))
            .child(icon(Icon::ClockCounterClockwise, 14., if active { t.fg } else { t.fg2 }))
            .child(self.range.label())
            .child(icon(Icon::CaretDown, 11., t.fg3));

        div()
            .relative()
            .flex_none()
            .child(button)
            .when(self.range_open, |el| {
                el.child(deferred(div().absolute().top(px(38.)).left_0().child(self.render_range_panel(t, cx))).with_priority(1))
            })
            .into_any_element()
    }

    fn render_range_panel(&self, t: Tokens, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let option = |id: &'static str, label: &'static str, range: TimeRange, current: TimeRange| {
            let selected = range == current;
            div()
                .id(id)
                .h(px(32.))
                .px(px(10.))
                .flex()
                .items_center()
                .gap(px(8.))
                .rounded(px(6.))
                .text_size(px(13.))
                .text_color(if selected { t.fg } else { t.fg2 })
                .cursor_pointer()
                .hover(|s| s.bg(t.bg3).text_color(t.fg))
                .child(div().flex_1().child(label))
                .when(selected, |el| el.child(icon(Icon::Check, 13., t.fg)))
                .on_click(cx.listener(move |this, _, _, cx| this.set_range(range, cx)))
        };

        let field = |label: &'static str, input: &Entity<InputState>| {
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(div().w(px(36.)).flex_none().text_size(px(12.)).text_color(t.fg3).child(label))
                .child(
                    div()
                        .flex_1()
                        .h(px(32.))
                        .px(px(10.))
                        .flex()
                        .items_center()
                        .rounded(px(7.))
                        .border_1()
                        .border_color(t.line2)
                        .bg(t.bg)
                        .font_family(MONO)
                        .text_size(px(12.))
                        .child(div().flex_1().child(Input::new(input).appearance(false))),
                )
        };

        let section = |label: &'static str| {
            div().px(px(10.)).pt(px(8.)).pb(px(4.)).text_size(px(11.5)).font_weight(FontWeight::MEDIUM).text_color(t.fg3).child(label)
        };

        div()
            .w(px(300.))
            .p(px(6.))
            .flex()
            .flex_col()
            .rounded(px(10.))
            .border_1()
            .border_color(t.line2)
            .bg(t.bg)
            .shadow_2xl()
            .occlude()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_range(cx)))
            .child(section("Quick ranges"))
            .children(PRESETS.iter().map(|(window, label)| {
                let id: &'static str = label;
                option(id, label, TimeRange::Last(*window), self.range)
            }))
            .child(option("All time", "All time", TimeRange::All, self.range))
            .child(div().my(px(6.)).h(px(1.)).bg(t.line))
            .child(section("Custom range"))
            .child(
                div()
                    .px(px(10.))
                    .pt(px(2.))
                    .pb(px(8.))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(field("From", &self.range_inputs.from))
                    .child(field("To", &self.range_inputs.to))
                    .children(self.range_error.clone().map(|e| div().text_size(px(12.)).text_color(t.err).child(e)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(px(11.5))
                                    .text_color(t.fg3)
                                    .child("Local time. Leave “To” empty to keep following."),
                            )
                            .child(
                                primary_button(t, "apply-range", "Apply", None)
                                    .h(px(30.))
                                    .px(px(12.))
                                    .text_size(px(12.5))
                                    .on_click(cx.listener(|this, _, _, cx| this.apply_custom_range(cx))),
                            ),
                    ),
            )
            .child(
                div()
                    .mx(px(-6.))
                    .mb(px(-6.))
                    .px(px(16.))
                    .py(px(10.))
                    .border_t_1()
                    .border_color(t.line)
                    .bg(t.bg2)
                    .text_size(px(11.5))
                    .text_color(t.fg3)
                    .child(format!(
                        "Searches the {}k-line buffer in memory. Older logs arrive with buckets.",
                        BUFFER / 1000
                    )),
            )
    }
}
