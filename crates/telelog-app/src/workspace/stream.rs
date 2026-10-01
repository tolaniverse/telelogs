use std::ops::Range;

use chrono::{DateTime, Local, SecondsFormat, Utc};
use gpui_kit::component::input::Input;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use telelog_core::{Level, LogRecord};

use super::sidebar::{kind_icon, kind_label};
use super::{BUFFER, Entry, LevelFilter, Status, View, Workspace};
use crate::json;
use crate::theme::{MONO, SANS, Tokens, tokens};
use crate::ui::canvas::{SphereMode, dot_grid, sphere};
use crate::ui::widgets::{dot, icon_button, outline_button, primary_button, segment, segmented};
use crate::ui::{Icon, icon};

const ROW_HEIGHT: f32 = 24.;
/// Longest message rendered in a row; the detail panel shows the full line.
const ROW_TEXT_LIMIT: usize = 400;
/// JSON view renders whole records, so it shows only the most recent ones.
const JSON_RECORDS: usize = 60;

pub fn level_color(t: Tokens, level: Level) -> Hsla {
    match level {
        Level::Error => t.err,
        Level::Warn => t.warn,
        Level::Info => t.fg,
        Level::Debug | Level::Trace | Level::Unknown => t.fg3,
    }
}

pub fn level_short(level: Level) -> &'static str {
    match level {
        Level::Error => "ERR",
        Level::Warn => "WRN",
        Level::Info => "INF",
        Level::Debug => "DBG",
        Level::Trace => "TRC",
        Level::Unknown => "···",
    }
}

fn clock(record: &LogRecord) -> String {
    let local: DateTime<Local> = record.timestamp.into();
    local.format("%H:%M:%S%.3f").to_string()
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

pub fn group_digits(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

impl Workspace {
    fn row_style(&self, t: Tokens, entry: &Entry) -> (Hsla, Hsla) {
        if self.selected == Some(entry.id) {
            (t.bg4, t.fg)
        } else if entry.record.level == Level::Error {
            (t.errbg, t.err)
        } else {
            (t.ink(0.), t.ink(0.))
        }
    }

    fn select(&mut self, id: u64, cx: &mut Context<Self>) {
        self.selected = if self.selected == Some(id) { None } else { Some(id) };
        self.copied = false;
        cx.notify();
    }

    fn render_rows(&mut self, range: Range<usize>, _: &mut Window, cx: &mut Context<Self>) -> Vec<Stateful<Div>> {
        let t = tokens(cx);
        range
            .filter_map(|ix| self.visible.get(ix).map(|&i| (ix, &self.entries[i])))
            .map(|(ix, entry)| {
                let record = &entry.record;
                let (bg, edge) = self.row_style(t, entry);
                let id = entry.id;
                div()
                    .id(("row", ix))
                    .h(px(ROW_HEIGHT))
                    .pl(px(14.))
                    .pr(px(16.))
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .whitespace_nowrap()
                    .bg(bg)
                    .border_l_2()
                    .border_color(edge)
                    .hover(|s| s.bg(t.bg3))
                    .on_click(cx.listener(move |this, _, _, cx| this.select(id, cx)))
                    .child(div().flex_none().text_color(t.fg3).child(clock(record)))
                    .child(
                        div()
                            .w(px(26.))
                            .flex_none()
                            .text_color(level_color(t, record.level))
                            .font_weight(FontWeight::MEDIUM)
                            .child(level_short(record.level)),
                    )
                    .child(
                        div()
                            .w(px(200.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(px(7.))
                            .overflow_hidden()
                            .child(icon(kind_icon(record.source), 13., t.fg3))
                            .child(
                                div()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .text_color(t.fg2)
                                    .child(record.origin.clone()),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .text_color(t.fg)
                            .child(truncate(&record.body, ROW_TEXT_LIMIT).to_string()),
                    )
            })
            .collect()
    }

    fn render_json_records(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = tokens(cx);
        let start = self.visible.len().saturating_sub(JSON_RECORDS);
        div()
            .id("json-records")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.json_scroll)
            .font_family(MONO)
            .text_size(px(12.))
            .line_height(px(19.))
            .py(px(6.))
            .children(self.visible[start..].iter().map(|&i| {
                let entry = &self.entries[i];
                let (bg, edge) = self.row_style(t, entry);
                let id = entry.id;
                let color = level_color(t, entry.record.level);
                div()
                    .id(("json", id))
                    .px(px(16.))
                    .py(px(10.))
                    .border_b_1()
                    .border_color(t.line)
                    .bg(bg)
                    .border_l_2()
                    .when(edge.a > 0., |el| el.border_color(edge))
                    .hover(|s| s.bg(t.bg2))
                    .on_click(cx.listener(move |this, _, _, cx| this.select(id, cx)))
                    .children(json::lines(&entry.record).iter().map(|line| {
                        div()
                            .pl(px(line.depth as f32 * json::INDENT))
                            .child(line.styled(t, color))
                    }))
            }))
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = tokens(cx);
        let has_filter = !self.filter_input.read(cx).value().is_empty();

        let search = div()
            .w(px(340.))
            .h(px(32.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(10.))
            .rounded(px(8.))
            .border_1()
            .border_color(t.line2)
            .bg(t.bg)
            .text_size(px(13.))
            .child(icon(Icon::MagnifyingGlass, 14., t.fg3))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(&self.filter_input).appearance(false)),
            )
            .when(has_filter, |el| {
                el.child(
                    div()
                        .id("clear-filter")
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, window, cx| this.set_filter_text(String::new(), window, cx)))
                        .child(icon(Icon::XCircle, 15., t.fg3)),
                )
            });

        let levels = [
            (LevelFilter::All, "All", None),
            (LevelFilter::Error, "Error", Some(t.err)),
            (LevelFilter::Warn, "Warn", Some(t.warn)),
            (LevelFilter::Info, "Info", Some(t.fg)),
            (LevelFilter::Debug, "Debug", Some(t.fg3)),
        ];
        let level_control = segmented(
            t,
            levels.into_iter().map(|(level, label, color)| {
                let leading = color.map(|color| dot(color, 6.).into_any_element());
                segment(
                    t,
                    SharedString::from(format!("level-{label}")),
                    label.into(),
                    leading,
                    self.level == level,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.level = level;
                    this.refilter(cx);
                }))
                .into_any_element()
            }),
        );

        let views = [
            (View::Rows, "Rows", Icon::Rows),
            (View::Json, "JSON", Icon::BracketsCurly),
        ];
        let view_control = segmented(
            t,
            views.into_iter().map(|(view, label, glyph)| {
                let active = self.view == view;
                let leading = icon(glyph, 14., if active { t.fg } else { t.fg2 }).into_any_element();
                segment(
                    t,
                    SharedString::from(format!("view-{label}")),
                    label.into(),
                    Some(leading),
                    active,
                )
                .on_click(cx.listener(move |this, _, _, cx| this.set_view(view, cx)))
                .into_any_element()
            }),
        );

        let follow = div()
            .id("follow")
            .h(px(32.))
            .px(px(12.))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(7.))
            .rounded(px(8.))
            .border_1()
            .text_size(px(13.))
            .font_weight(FontWeight::MEDIUM)
            .cursor_pointer()
            .on_click(cx.listener(|this, _, _, cx| this.set_follow(!this.follow, cx)))
            .map(|el| {
                if self.follow {
                    el.bg(t.inv)
                        .border_color(t.inv)
                        .text_color(t.invfg)
                        .child(icon(Icon::ArrowLineDown, 14., t.invfg))
                        .child("Following")
                } else {
                    el.bg(t.bg)
                        .border_color(t.line2)
                        .text_color(t.fg)
                        .child(icon(Icon::Pause, 14., t.fg))
                        .child("Follow")
                }
            });

        div()
            .h(px(52.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(14.))
            .border_b_1()
            .border_color(t.line)
            .child(search)
            .child(level_control)
            .child(self.render_range_picker(cx))
            .child(div().flex_1())
            .child(view_control)
            .child(follow)
    }

    fn render_empty(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = tokens(cx);
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(12.))
            .font_family(SANS)
            .child(icon(Icon::FunnelSimpleX, 34., t.fg3))
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(FontWeight::MEDIUM)
                    .child("No lines match"),
            )
            .child(
                div()
                    .max_w(px(320.))
                    .text_center()
                    .text_size(px(13.))
                    .text_color(t.fg2)
                    .child("Nothing in the buffer matches your filter and selected sources."),
            )
            .child(
                div().mt(px(4.)).child(
                    outline_button(t, "reset-filters", "Reset filters", None)
                        .on_click(cx.listener(|this, _, window, cx| this.reset_filters(window, cx))),
                ),
            )
    }

    fn render_overlay(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let t = tokens(cx);
        let addr = self.server_addr().to_string();
        let (mode, title, sub) = match &self.status {
            Status::Connected => return None,
            Status::Connecting => (
                SphereMode::Connecting,
                format!("Connecting to {addr}"),
                "telelog-server · LogService.Tail".to_string(),
            ),
            Status::Disconnected { reason } => (
                SphereMode::Disconnected,
                "Server unreachable".to_string(),
                format!(
                    "{reason} · retrying every {}s",
                    crate::client::RECONNECT_DELAY.as_secs()
                ),
            ),
        };
        Some(
            div().absolute().inset_0().bg(t.bg).occlude().child(dot_grid(t)).child(
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(6.))
                    .child(sphere(t, mode, 260.))
                    .child(div().text_size(px(16.)).font_weight(FontWeight::MEDIUM).child(title))
                    .child(div().font_family(MONO).text_size(px(12.)).text_color(t.fg3).child(sub))
                    .when(mode == SphereMode::Disconnected, |el| {
                        el.child(
                            div().mt(px(14.)).child(
                                primary_button(t, "retry", "Retry now", Some(Icon::ArrowClockwise))
                                    .h(px(32.))
                                    .text_size(px(13.))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.client.retry_now();
                                        this.status = Status::Connecting;
                                        cx.notify();
                                    })),
                            ),
                        )
                    }),
            ),
        )
    }

    fn render_detail(&self, entry: &Entry, cx: &mut Context<Self>) -> AnyElement {
        let t = tokens(cx);
        let record = &entry.record;
        let color = level_color(t, record.level);
        let time: DateTime<Utc> = record.timestamp.into();
        let fields = [
            ("Timestamp", time.to_rfc3339_opts(SecondsFormat::Millis, true)),
            ("Source", kind_label(record.source).to_string()),
            ("Origin", record.origin.clone()),
            ("Stream", record.stream.as_str().to_string()),
            ("Level", record.level.as_str().to_string()),
        ];
        let section_label = |label: &'static str| {
            div()
                .text_size(px(11.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(t.fg3)
                .child(label)
        };
        let pretty = json::pretty(record);
        let origin = record.origin.clone();
        let solo_origin = record.origin.clone();

        div()
            .w(px(400.))
            .flex_none()
            .flex()
            .flex_col()
            .min_h_0()
            .bg(t.bg)
            .border_l_1()
            .border_color(t.line)
            .child(
                div()
                    .h(px(48.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .pl(px(18.))
                    .pr(px(14.))
                    .border_b_1()
                    .border_color(t.line)
                    .child(
                        div()
                            .px(px(5.))
                            .py(px(1.))
                            .rounded(px(4.))
                            .border_1()
                            .border_color(color)
                            .font_family(MONO)
                            .text_size(px(11.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(color)
                            .child(level_short(record.level)),
                    )
                    .child(
                        div()
                            .text_size(px(13.5))
                            .font_weight(FontWeight::MEDIUM)
                            .child("Log line"),
                    )
                    .child(
                        div()
                            .font_family(MONO)
                            .text_size(px(11.5))
                            .text_color(t.fg3)
                            .child(format!("#{}", entry.id)),
                    )
                    .child(div().flex_1())
                    .child(
                        icon_button(t, "close-detail", Icon::X, 15.).on_click(cx.listener(|this, _, _, cx| {
                            this.selected = None;
                            cx.notify();
                        })),
                    ),
            )
            .child(
                div()
                    .id("detail-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p(px(18.))
                    .flex()
                    .flex_col()
                    .gap(px(20.))
                    .child(
                        div()
                            .px(px(14.))
                            .py(px(12.))
                            .rounded(px(8.))
                            .border_1()
                            .border_color(t.line)
                            .bg(t.bg2)
                            .font_family(MONO)
                            .text_size(px(12.5))
                            .line_height(px(20.))
                            .child(record.body.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(10.))
                            .text_size(px(12.5))
                            .children(fields.into_iter().map(|(k, v)| {
                                div()
                                    .flex()
                                    .gap(px(12.))
                                    .child(div().w(px(96.)).flex_none().text_color(t.fg3).child(k))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .overflow_hidden()
                                            .text_ellipsis()
                                            .whitespace_nowrap()
                                            .font_family(MONO)
                                            .text_size(px(12.))
                                            .child(v),
                                    )
                            })),
                    )
                    .when(!record.labels.is_empty(), |el| {
                        el.child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(8.))
                                .child(section_label("Labels"))
                                .child(div().flex().flex_wrap().gap(px(6.)).children(record.labels.iter().map(
                                    |(k, v)| {
                                        div()
                                            .flex()
                                            .gap(px(4.))
                                            .px(px(7.))
                                            .py(px(3.))
                                            .rounded(px(6.))
                                            .border_1()
                                            .border_color(t.line2)
                                            .font_family(MONO)
                                            .text_size(px(11.5))
                                            .child(div().text_color(t.fg3).child(k.clone()))
                                            .child(div().text_color(t.fg).child(v.clone()))
                                    },
                                ))),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(section_label("JSON"))
                                    .child(
                                        div()
                                            .id("copy-json")
                                            .h(px(24.))
                                            .px(px(8.))
                                            .flex()
                                            .items_center()
                                            .gap(px(5.))
                                            .rounded(px(6.))
                                            .border_1()
                                            .border_color(t.line2)
                                            .bg(t.bg)
                                            .text_size(px(12.))
                                            .text_color(t.fg2)
                                            .cursor_pointer()
                                            .hover(|s| s.text_color(t.fg))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                cx.write_to_clipboard(ClipboardItem::new_string(pretty.clone()));
                                                this.copied = true;
                                                cx.notify();
                                            }))
                                            .child(icon(if self.copied { Icon::Check } else { Icon::Copy }, 13., t.fg2))
                                            .child(if self.copied { "Copied" } else { "Copy" }),
                                    ),
                            )
                            .child(
                                div()
                                    .px(px(14.))
                                    .py(px(12.))
                                    .rounded(px(8.))
                                    .border_1()
                                    .border_color(t.line)
                                    .bg(t.bg2)
                                    .overflow_hidden()
                                    .font_family(MONO)
                                    .text_size(px(11.5))
                                    .line_height(px(18.))
                                    .whitespace_nowrap()
                                    .children(json::lines(record).iter().map(|line| {
                                        div()
                                            .pl(px(line.depth as f32 * json::INDENT))
                                            .child(line.styled(t, color))
                                    })),
                            ),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .gap(px(8.))
                    .px(px(18.))
                    .py(px(12.))
                    .border_t_1()
                    .border_color(t.line)
                    .child(
                        outline_button(t, "filter-origin", "Filter by origin", Some(Icon::FunnelSimple))
                            .flex_1()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.set_filter_text(origin.clone(), window, cx);
                            })),
                    )
                    .child(
                        outline_button(t, "solo-origin", "Tail only this", Some(Icon::CrosshairSimple))
                            .flex_1()
                            .on_click(cx.listener(move |this, _, _, cx| this.solo(&solo_origin, cx))),
                    ),
            )
            .into_any_element()
    }

    fn render_status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = tokens(cx);
        let addr = self.server_addr();
        let enabled = self
            .targets
            .iter()
            .filter(|target| self.is_enabled(&target.name))
            .count();
        let text = match &self.status {
            Status::Connected => format!("{addr} · {enabled} targets"),
            Status::Connecting => format!("Connecting to {addr}…"),
            Status::Disconnected { reason } => format!("Disconnected: {reason} (retrying)"),
        };
        div()
            .h(px(30.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(14.))
            .border_t_1()
            .border_color(t.line)
            .bg(t.bg2)
            .text_size(px(11.5))
            .text_color(t.fg2)
            .font_family(MONO)
            .child(dot(self.status_color(cx), 7.))
            .child(div().overflow_hidden().text_ellipsis().whitespace_nowrap().child(text))
            .child(div().flex_1())
            .child(div().flex_none().text_color(t.fg3).child(format!(
                "{} / {} lines",
                group_digits(self.visible.len()),
                group_digits(self.entries.len())
            )))
            .child(div().text_color(t.line2).child("|"))
            .child(
                div()
                    .flex_none()
                    .text_color(t.fg3)
                    .child(format!("buffer {}k", BUFFER / 1000)),
            )
    }

    pub(super) fn render_stream(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let list = if self.visible.is_empty() {
            match self.status {
                Status::Connected => self.render_empty(cx).into_any_element(),
                _ => div().flex_1().into_any_element(),
            }
        } else {
            match self.view {
                View::Rows => uniform_list("rows", self.visible.len(), cx.processor(Self::render_rows))
                    .track_scroll(&self.rows_scroll)
                    .flex_1()
                    .py(px(6.))
                    .font_family(MONO)
                    .text_size(px(12.))
                    .into_any_element(),
                View::Json => self.render_json_records(cx).into_any_element(),
            }
        };

        let detail = self.selected_entry().map(|entry| self.render_detail(entry, cx));
        let overlay = self.render_overlay(cx);

        div()
            .size_full()
            .flex()
            .flex_col()
            .child(self.render_toolbar(cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .flex()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(list)
                            // Scrolling up pauses follow; scrolling back to the end resumes it.
                            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, window, cx| {
                                let dy = event.delta.pixel_delta(window.line_height()).y;
                                if dy > px(0.) {
                                    this.set_follow(false, cx);
                                } else if dy < px(0.) && !this.follow {
                                    let at_end = match this.view {
                                        View::Rows => this.rows_scroll.is_scrolled_to_end().unwrap_or(false),
                                        View::Json => {
                                            let handle = &this.json_scroll;
                                            handle.offset().y <= -handle.max_offset().y + px(24.)
                                        }
                                    };
                                    if at_end {
                                        this.set_follow(true, cx);
                                    }
                                }
                            })),
                    )
                    .children(detail)
                    .children(overlay),
            )
            .child(self.render_status_bar(cx))
    }
}

#[cfg(test)]
mod tests {
    use super::{group_digits, truncate};

    #[test]
    fn groups_digits() {
        assert_eq!(group_digits(0), "0");
        assert_eq!(group_digits(1_234_567), "1,234,567");
    }

    #[test]
    fn truncates_on_char_boundary() {
        assert_eq!(truncate("héllo", 2), "h");
        assert_eq!(truncate("abc", 10), "abc");
    }
}
