use std::ops::Range;

use futures::StreamExt;
use futures::channel::mpsc::UnboundedReceiver;
use gpui_kit::component::button::*;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::*;
use gpui_kit::*;
use telelog_core::{Filter, Level, LogRecord, Stream};

use crate::client::ClientEvent;
use crate::time::clock;

/// Records kept in memory. Older ones are dropped; long retention lives in buckets.
const MAX_RECORDS: usize = 200_000;
const ROW_HEIGHT: f32 = 22.;

enum Status {
    Connecting,
    Connected { targets: usize },
    Disconnected { reason: String },
}

pub struct LogView {
    server: String,
    status: Status,
    records: Vec<LogRecord>,
    /// Indices into `records` that pass `filter`.
    visible: Vec<usize>,
    filter: Filter,
    filter_input: Entity<InputState>,
    follow: bool,
    scroll: UniformListScrollHandle,
    _subscriptions: Vec<Subscription>,
    _pump: Task<()>,
}

impl LogView {
    pub fn new(
        server: String,
        mut events: UnboundedReceiver<ClientEvent>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let filter_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Filter by text or container…"));
        let subscription = cx.subscribe(&filter_input, |this, input, event, cx| {
            if let InputEvent::Change = event {
                this.filter = Filter::new(&input.read(cx).value());
                this.refilter();
                cx.notify();
            }
        });

        // Drain events in batches so a burst of lines costs one re-render.
        let pump = cx.spawn(async move |this, cx| {
            while let Some(first) = events.next().await {
                let mut batch = vec![first];
                while let Ok(event) = events.try_recv() {
                    batch.push(event);
                }
                if this.update(cx, |this, cx| this.apply(batch, cx)).is_err() {
                    break;
                }
            }
        });

        LogView {
            server,
            status: Status::Connecting,
            records: Vec::new(),
            visible: Vec::new(),
            filter: Filter::default(),
            filter_input,
            follow: true,
            scroll: UniformListScrollHandle::new(),
            _subscriptions: vec![subscription],
            _pump: pump,
        }
    }

    fn apply(&mut self, batch: Vec<ClientEvent>, cx: &mut Context<Self>) {
        for event in batch {
            match event {
                ClientEvent::Connected { targets } => {
                    // The server resends backlog on reconnect, so start fresh.
                    self.records.clear();
                    self.visible.clear();
                    self.status = Status::Connected { targets: targets.len() };
                }
                ClientEvent::Disconnected { reason } => self.status = Status::Disconnected { reason },
                ClientEvent::Record(record) => {
                    if self.filter.matches(&record) {
                        self.visible.push(self.records.len());
                    }
                    self.records.push(record);
                }
            }
        }

        if self.records.len() > MAX_RECORDS {
            let excess = self.records.len() - MAX_RECORDS;
            self.records.drain(..excess);
            self.refilter();
        }
        if self.follow && !self.visible.is_empty() {
            self.scroll.scroll_to_item(self.visible.len() - 1, ScrollStrategy::Bottom);
        }
        cx.notify();
    }

    fn refilter(&mut self) {
        self.visible = if self.filter.is_empty() {
            (0..self.records.len()).collect()
        } else {
            (0..self.records.len()).filter(|&i| self.filter.matches(&self.records[i])).collect()
        };
    }

    fn render_rows(&mut self, range: Range<usize>, _: &mut Window, cx: &mut Context<Self>) -> Vec<Div> {
        let theme = cx.theme();
        range
            .filter_map(|ix| self.visible.get(ix).map(|&i| &self.records[i]))
            .map(|record| {
                let (level, level_color) = match record.level {
                    Level::Error => ("ERR", theme.danger),
                    Level::Warn => ("WRN", theme.warning),
                    Level::Info => ("INF", theme.info),
                    Level::Debug => ("DBG", theme.muted_foreground),
                    Level::Trace => ("TRC", theme.muted_foreground),
                    Level::Unknown if record.stream == Stream::Stderr => ("ERR", theme.danger),
                    Level::Unknown => ("   ", theme.muted_foreground),
                };
                div()
                    .h(px(ROW_HEIGHT))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_3()
                    .whitespace_nowrap()
                    .child(div().text_color(theme.muted_foreground).child(clock(record.timestamp)))
                    .child(div().w(px(28.)).text_color(level_color).child(level))
                    .child(
                        div()
                            .w(px(160.))
                            .overflow_hidden()
                            .text_ellipsis()
                            .text_color(theme.primary)
                            .child(record.origin.clone()),
                    )
                    .child(div().text_color(theme.foreground).child(record.body.clone()))
            })
            .collect()
    }

    fn status_text(&self) -> String {
        match &self.status {
            Status::Connecting => format!("Connecting to {}…", self.server),
            Status::Connected { targets } => format!("{} · {targets} containers", self.server),
            Status::Disconnected { reason } => format!("Disconnected: {reason} (retrying)"),
        }
    }
}

impl Render for LogView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let status_color = match self.status {
            Status::Connected { .. } => theme.success,
            Status::Connecting => theme.muted_foreground,
            Status::Disconnected { .. } => theme.danger,
        };

        let follow = Button::new("follow").label(if self.follow { "Following" } else { "Follow" });
        let follow = if self.follow { follow.primary() } else { follow.outline() };

        let toolbar = div()
            .flex()
            .items_center()
            .gap_3()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(theme.border)
            .child(div().w(px(420.)).child(Input::new(&self.filter_input).cleanable(true)))
            .child(follow.on_click(cx.listener(|this, _, _, cx| {
                this.follow = !this.follow;
                if this.follow && !this.visible.is_empty() {
                    this.scroll.scroll_to_item(this.visible.len() - 1, ScrollStrategy::Bottom);
                }
                cx.notify();
            })))
            .child(div().flex_1())
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(format!("{} / {} lines", self.visible.len(), self.records.len())),
            );

        let list = uniform_list("logs", self.visible.len(), cx.processor(Self::render_rows))
            .track_scroll(&self.scroll)
            .flex_1()
            .font_family("Menlo")
            .text_size(px(12.));

        let status_bar = div()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_1()
            .border_t_1()
            .border_color(theme.border)
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(div().size(px(8.)).rounded_full().bg(status_color))
            .child(self.status_text());

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(toolbar)
            .child(list)
            .child(status_bar)
    }
}
