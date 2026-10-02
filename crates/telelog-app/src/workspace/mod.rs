//! The window's root view: owns all app state and lays out the title bar, sidebar and
//! the active screen. Each screen renders from its own module.

mod add_source;
mod pages;
mod palette;
mod range;
mod sidebar;
mod stream;

use std::collections::{HashMap, HashSet};
use std::time::{Duration, SystemTime};

use futures::StreamExt;
use futures::channel::mpsc::UnboundedReceiver;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use telelog_core::{Filter, Level, LogRecord, SourceKind, Target};

use crate::client::{Client, ClientEvent};
use crate::theme::{self, MONO, tokens};
use crate::time_range::TimeRange;
use crate::ui::Icon;

actions!(telelogs, [ToggleCommandPalette, ToggleJsonView]);

/// Records kept in memory. Older ones are dropped; long retention lives in buckets.
pub const BUFFER: usize = 200_000;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Stream,
    Sources,
    Storage,
    Team,
    Agents,
}

pub enum Status {
    Connecting,
    Connected,
    Disconnected { reason: String },
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum View {
    Rows,
    Json,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LevelFilter {
    All,
    Error,
    Warn,
    Info,
    Debug,
}

impl LevelFilter {
    fn min_level(self) -> Option<Level> {
        match self {
            LevelFilter::All => None,
            LevelFilter::Error => Some(Level::Error),
            LevelFilter::Warn => Some(Level::Warn),
            LevelFilter::Info => Some(Level::Info),
            LevelFilter::Debug => Some(Level::Debug),
        }
    }
}

pub struct Entry {
    pub id: u64,
    pub record: LogRecord,
}

/// Lines fetched from the server's archive bucket for the current time range.
pub enum ArchiveState {
    /// No bucket, or the range doesn't reach past the live buffer.
    Off,
    Loading,
    Loaded {
        lines: usize,
        truncated: bool,
    },
    Failed(String),
}

/// Archived entries get ids from their own range so they never collide with live ones.
const ARCHIVED_IDS: u64 = 1 << 62;

pub struct Workspace {
    server: String,
    client: Client,
    screen: Screen,
    status: Status,
    targets: Vec<Target>,
    /// Origins (target names) included in the stream. Missing means included.
    enabled: HashMap<String, bool>,
    open_groups: HashSet<SourceKind>,

    entries: Vec<Entry>,
    next_id: u64,
    /// Oldest timestamp in `entries` per origin; that origin's archived lines from then on are
    /// already live.
    live_starts: HashMap<String, SystemTime>,
    /// Lines from the archive bucket that aren't in the live buffer, merged in by time.
    archived: Vec<Entry>,
    next_archived_id: u64,
    archive: ArchiveState,
    /// Number of the newest archive request; older answers are ignored.
    archive_request: u64,
    storage: Option<Result<telelog_proto::v1::StorageInfo, String>>,
    /// Indices into `archived` followed by `entries` (see [`Workspace::entry`]) that pass the filters.
    visible: Vec<usize>,
    filter: Filter,
    level: LevelFilter,
    range: TimeRange,
    range_open: bool,
    range_error: Option<String>,
    range_inputs: range::RangeInputs,
    view: View,
    follow: bool,
    selected: Option<u64>,
    copied: bool,

    filter_input: Entity<InputState>,
    rows_scroll: UniformListScrollHandle,
    json_scroll: ScrollHandle,

    palette_open: bool,
    palette_input: Entity<InputState>,
    palette_index: usize,

    add_open: bool,
    add_kind: SourceKind,
    add_inputs: add_source::Inputs,

    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
    _pump: Task<()>,
    _range_tick: Task<()>,
}

impl Workspace {
    pub fn new(
        server: String,
        client: Client,
        mut events: UnboundedReceiver<ClientEvent>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let filter_input = cx.new(|cx| InputState::new(window, cx).placeholder("Filter by text or container…"));
        let palette_input = cx.new(|cx| InputState::new(window, cx).placeholder("Type a command or search sources…"));
        let subscriptions = vec![
            cx.subscribe(&filter_input, |this, _, event, cx| {
                if let InputEvent::Change = event {
                    this.refilter(cx);
                }
            }),
            cx.subscribe(&palette_input, |this, _, event, cx| {
                if let InputEvent::Change = event {
                    this.palette_index = 0;
                    cx.notify();
                }
            }),
        ];

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

        // Relative ranges ("last 15 minutes") move with the clock, so old lines age out.
        let range_tick = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(5)).await;
                let alive = this.update(cx, |this, cx| {
                    if this.range.is_relative() {
                        this.refilter(cx);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        });

        let focus = cx.focus_handle();
        focus.focus(window, cx);

        Workspace {
            server,
            client,
            screen: Screen::Stream,
            status: Status::Connecting,
            targets: Vec::new(),
            enabled: HashMap::new(),
            open_groups: HashSet::from([SourceKind::Docker, SourceKind::Kubernetes]),
            entries: Vec::new(),
            next_id: 1,
            live_starts: HashMap::new(),
            archived: Vec::new(),
            next_archived_id: ARCHIVED_IDS,
            archive: ArchiveState::Off,
            archive_request: 0,
            storage: None,
            visible: Vec::new(),
            filter: Filter::default(),
            level: LevelFilter::All,
            range: TimeRange::All,
            range_open: false,
            range_error: None,
            range_inputs: range::RangeInputs::new(window, cx),
            view: View::Rows,
            follow: true,
            selected: None,
            copied: false,
            filter_input,
            rows_scroll: UniformListScrollHandle::new(),
            json_scroll: ScrollHandle::new(),
            palette_open: false,
            palette_input,
            palette_index: 0,
            add_open: false,
            add_kind: SourceKind::Docker,
            add_inputs: add_source::Inputs::new(window, cx),
            focus,
            _subscriptions: subscriptions,
            _pump: pump,
            _range_tick: range_tick,
        }
    }

    fn server_addr(&self) -> &str {
        self.server.trim_start_matches("http://").trim_start_matches("https://")
    }

    fn is_enabled(&self, origin: &str) -> bool {
        self.enabled.get(origin).copied().unwrap_or(true)
    }

    fn passes(&self, entry: &Entry) -> bool {
        self.is_enabled(&entry.record.origin) && self.filter.matches(&entry.record)
    }

    fn apply(&mut self, batch: Vec<ClientEvent>, cx: &mut Context<Self>) {
        let mut rebuild = false;
        for event in batch {
            match event {
                ClientEvent::Connecting => {
                    if !matches!(self.status, Status::Disconnected { .. }) {
                        self.status = Status::Connecting;
                    }
                }
                ClientEvent::Connected { targets } => {
                    // The server resends backlog on reconnect, so start fresh.
                    self.entries.clear();
                    self.live_starts.clear();
                    self.selected = None;
                    rebuild = true;
                    self.client.fetch_storage();
                    for target in &targets {
                        self.enabled
                            .entry(target.name.clone())
                            .or_insert(target.state == "running");
                    }
                    self.targets = targets;
                    self.status = Status::Connected;
                }
                ClientEvent::Disconnected { reason } => self.status = Status::Disconnected { reason },
                ClientEvent::TargetChanged(target) => self.update_target(target),
                ClientEvent::Record(record) => {
                    self.note_target(&record);
                    let entry = Entry {
                        id: self.next_id,
                        record,
                    };
                    self.next_id += 1;
                    let timestamp = entry.record.timestamp;
                    let start = self.live_starts.entry(entry.record.origin.clone()).or_insert(timestamp);
                    if timestamp <= *start {
                        *start = timestamp;
                        // The boundary hiding this origin's archived duplicates moved.
                        rebuild |= !self.archived.is_empty();
                    }
                    if self.passes(&entry) {
                        self.visible.push(self.archived.len() + self.entries.len());
                    }
                    self.entries.push(entry);
                }
                ClientEvent::Archive { request, result } if request == self.archive_request => {
                    match result {
                        Ok(records) => {
                            let lines = records.len();
                            self.archived = records
                                .into_iter()
                                .map(|record| {
                                    self.next_archived_id += 1;
                                    Entry {
                                        id: self.next_archived_id,
                                        record,
                                    }
                                })
                                .collect();
                            self.archive = ArchiveState::Loaded {
                                lines,
                                truncated: lines >= crate::client::ARCHIVE_LIMIT as usize,
                            };
                        }
                        Err(e) => self.archive = ArchiveState::Failed(e),
                    }
                    rebuild = true;
                }
                ClientEvent::Archive { .. } => {}
                ClientEvent::Storage(result) => {
                    let enabled = matches!(&result, Ok(info) if info.enabled);
                    self.storage = Some(result);
                    if enabled && matches!(self.archive, ArchiveState::Off) {
                        self.load_archive();
                    }
                }
            }
        }

        if self.entries.len() > BUFFER {
            let excess = self.entries.len() - BUFFER;
            self.entries.drain(..excess);
            self.live_starts.clear();
            for e in &self.entries {
                let start = self
                    .live_starts
                    .entry(e.record.origin.clone())
                    .or_insert(e.record.timestamp);
                *start = (*start).min(e.record.timestamp);
            }
            rebuild = true;
        }
        if rebuild {
            self.rebuild_visible();
        }
        self.scroll_to_end_if_following();
        cx.notify();
    }

    /// The server follows containers as they start; the first line from one adds it to the sidebar.
    fn note_target(&mut self, record: &LogRecord) {
        if self.targets.iter().any(|t| t.name == record.origin) {
            return;
        }
        let id = record
            .labels
            .get("container_id")
            .cloned()
            .unwrap_or_else(|| record.origin.clone());
        self.targets.push(Target {
            id,
            name: record.origin.clone(),
            source: record.source,
            state: "running".into(),
            labels: record.labels.clone(),
        });
        self.enabled.entry(record.origin.clone()).or_insert(true);
    }

    /// Applies a lifecycle change from the server. Targets are matched by name, which is also how
    /// lines and toggles refer to them; a recreated container with the same name is the same target.
    fn update_target(&mut self, target: Target) {
        let position = self.targets.iter().position(|t| t.name == target.name);
        if target.state == "removed" {
            // Keep a removed container while its lines are still in the buffer, so they stay
            // filterable; otherwise drop it from the sidebar.
            let has_lines = self.entries.iter().any(|e| e.record.origin == target.name);
            match position {
                Some(ix) if has_lines => self.targets[ix].state = target.state,
                Some(ix) => {
                    self.targets.remove(ix);
                }
                None => {}
            }
            return;
        }
        self.enabled.entry(target.name.clone()).or_insert(true);
        match position {
            Some(ix) => self.targets[ix] = target,
            None => self.targets.push(target),
        }
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        let text = self.filter_input.read(cx).value();
        self.filter = Filter::new(&text);
        self.filter.min_level = self.level.min_level();
        (self.filter.from, self.filter.to) = self.range.bounds(SystemTime::now());
        self.rebuild_visible();
        self.scroll_to_end_if_following();
        cx.notify();
    }

    /// The `i`th line of the combined list: archived lines first, then the live buffer.
    pub(super) fn entry(&self, i: usize) -> &Entry {
        match i.checked_sub(self.archived.len()) {
            Some(live) => &self.entries[live],
            None => &self.archived[i],
        }
    }

    fn rebuild_visible(&mut self) {
        let archived: Vec<usize> = (0..self.archived.len())
            .filter(|&i| {
                let e = &self.archived[i];
                let duplicate = self
                    .live_starts
                    .get(&e.record.origin)
                    .is_some_and(|&start| e.record.timestamp >= start);
                !duplicate && self.passes(e)
            })
            .collect();
        let live: Vec<usize> = (0..self.entries.len())
            .filter(|&i| self.passes(&self.entries[i]))
            .map(|i| self.archived.len() + i)
            .collect();
        // Both are in time order; merge them so a removed container's archived lines sit
        // among the live lines of the same moment.
        let mut visible = Vec::with_capacity(archived.len() + live.len());
        let (mut a, mut l) = (archived.into_iter().peekable(), live.into_iter().peekable());
        while let (Some(&ai), Some(&li)) = (a.peek(), l.peek()) {
            if self.entry(ai).record.timestamp <= self.entry(li).record.timestamp {
                visible.push(ai);
                a.next();
            } else {
                visible.push(li);
                l.next();
            }
        }
        visible.extend(a);
        visible.extend(l);
        self.visible = visible;
    }

    /// Fetches the time range from the archive bucket, minus what the live buffer already holds
    /// for each origin.
    pub(super) fn load_archive(&mut self) {
        self.archive_request += 1;
        self.archived.clear();
        self.archive = ArchiveState::Off;
        let bucket = matches!(&self.storage, Some(Ok(info)) if info.enabled);
        if !bucket || self.range == TimeRange::All {
            return;
        }
        let now = SystemTime::now();
        let (from, to) = self.range.bounds(now);
        let to = to.unwrap_or(now);
        if from.is_some_and(|from| from > to) {
            return;
        }
        self.archive = ArchiveState::Loading;
        self.client
            .query_archive(self.archive_request, from, to, self.live_starts.clone());
    }

    fn scroll_to_end_if_following(&self) {
        if !self.follow || self.visible.is_empty() {
            return;
        }
        match self.view {
            View::Rows => self
                .rows_scroll
                .scroll_to_item(self.visible.len() - 1, ScrollStrategy::Bottom),
            View::Json => self.json_scroll.scroll_to_bottom(),
        }
    }

    fn set_follow(&mut self, follow: bool, cx: &mut Context<Self>) {
        if self.follow != follow {
            self.follow = follow;
            self.scroll_to_end_if_following();
            cx.notify();
        }
    }

    fn set_view(&mut self, view: View, cx: &mut Context<Self>) {
        self.view = view;
        self.screen = Screen::Stream;
        self.scroll_to_end_if_following();
        cx.notify();
    }

    fn toggle_target(&mut self, origin: &str, cx: &mut Context<Self>) {
        let on = self.is_enabled(origin);
        self.enabled.insert(origin.to_string(), !on);
        self.rebuild_visible();
        cx.notify();
    }

    /// "Tail only this": include one origin, exclude the rest.
    fn solo(&mut self, origin: &str, cx: &mut Context<Self>) {
        for target in &self.targets {
            self.enabled.insert(target.name.clone(), target.name == origin);
        }
        self.enabled.insert(origin.to_string(), true);
        self.screen = Screen::Stream;
        self.rebuild_visible();
        self.scroll_to_end_if_following();
        cx.notify();
    }

    fn set_filter_text(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.filter_input
            .update(cx, |input, cx| input.set_value(text, window, cx));
        self.refilter(cx);
    }

    fn reset_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.level = LevelFilter::All;
        self.range = TimeRange::All;
        for target in &self.targets {
            self.enabled.insert(target.name.clone(), target.state == "running");
        }
        self.set_filter_text(String::new(), window, cx);
    }

    fn go(&mut self, screen: Screen, cx: &mut Context<Self>) {
        if screen == Screen::Storage {
            self.client.fetch_storage();
        }
        self.screen = screen;
        self.selected = None;
        cx.notify();
    }

    fn toggle_theme(&mut self, cx: &mut Context<Self>) {
        let dark = tokens(cx).dark;
        theme::apply(!dark, cx);
        cx.notify();
    }

    fn clear_stream(&mut self, cx: &mut Context<Self>) {
        self.entries.clear();
        self.live_starts.clear();
        self.archived.clear();
        self.archive = ArchiveState::Off;
        self.archive_request += 1;
        self.visible.clear();
        self.selected = None;
        cx.notify();
    }

    fn selected_entry(&self) -> Option<&Entry> {
        let id = self.selected?;
        // Ids are increasing within each list, so the entry can be found by binary search.
        let list = if id > ARCHIVED_IDS {
            &self.archived
        } else {
            &self.entries
        };
        let ix = list.binary_search_by_key(&id, |e| e.id).ok()?;
        list.get(ix)
    }

    /// Lines `origin` wrote during the last minute (by log timestamp, so backlog doesn't count).
    fn lines_per_minute(&self, origin: &str) -> usize {
        let cutoff = SystemTime::now() - std::time::Duration::from_secs(60);
        self.entries
            .iter()
            .rev()
            .take_while(|e| e.record.timestamp >= cutoff)
            .filter(|e| e.record.origin == origin)
            .count()
    }

    fn oldest_record(&self) -> Option<SystemTime> {
        self.entries.first().map(|e| e.record.timestamp)
    }

    fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette_open = true;
        self.add_open = false;
        self.palette_index = 0;
        self.palette_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn close_overlays(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette_open = false;
        self.add_open = false;
        self.focus.focus(window, cx);
        cx.notify();
    }

    fn open_add_source(&mut self, cx: &mut Context<Self>) {
        self.add_open = true;
        self.palette_open = false;
        cx.notify();
    }

    /// Escape, arrows and Enter are handled here, before inputs see them, so they work
    /// while typing in the command palette.
    fn capture_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        if self.palette_open {
            let count = self.palette_commands(cx).len();
            match key {
                "escape" => self.close_overlays(window, cx),
                "down" if count > 0 => self.palette_index = (self.palette_index + 1) % count,
                "up" if count > 0 => self.palette_index = (self.palette_index + count - 1) % count,
                "enter" => {
                    if let Some(command) = self.palette_commands(cx).into_iter().nth(self.palette_index) {
                        self.close_overlays(window, cx);
                        self.run(command.action, window, cx);
                    }
                }
                _ => return,
            }
            cx.stop_propagation();
            cx.notify();
        } else if key == "escape" {
            // Close the innermost thing that is open: range picker, dialog, then selection.
            if self.range_open {
                self.close_range(cx);
            } else if self.add_open {
                self.close_overlays(window, cx);
            } else if self.selected.is_some() {
                self.selected = None;
                cx.notify();
            } else {
                return;
            }
            cx.stop_propagation();
        }
    }

    fn render_titlebar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = tokens(cx);
        div()
            .h(px(40.))
            .flex_none()
            .relative()
            .flex()
            .items_center()
            .px(px(14.))
            .bg(t.bg2)
            .border_b_1()
            .border_color(t.line)
            .on_mouse_down(MouseButton::Left, |event, window, _| {
                if event.click_count == 2 {
                    window.titlebar_double_click();
                } else {
                    window.start_window_move();
                }
            })
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(8.))
                    .text_size(px(12.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(t.fg2)
                    .child("telelogs")
                    .child(div().text_color(t.fg3).child("—"))
                    .child(
                        div()
                            .font_family(MONO)
                            .text_size(px(12.))
                            .child(self.server_addr().to_string()),
                    ),
            )
            .child(div().flex_1())
            .child(
                div()
                    .id("theme-toggle")
                    .w(px(28.))
                    .h(px(26.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(t.line2)
                    .bg(t.bg)
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_theme(cx)))
                    .child(crate::ui::icon(if t.dark { Icon::Sun } else { Icon::Moon }, 14., t.fg2)),
            )
    }

    fn render_screen(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        match self.screen {
            Screen::Stream => self.render_stream(window, cx).into_any_element(),
            Screen::Sources => self.render_sources(cx).into_any_element(),
            Screen::Storage => self.render_storage(cx).into_any_element(),
            Screen::Team => self.render_team(cx).into_any_element(),
            Screen::Agents => self.render_agents(cx).into_any_element(),
        }
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = tokens(cx);
        let screen = self.render_screen(window, cx);
        let palette = self.palette_open.then(|| self.render_palette(cx));
        let add = self.add_open.then(|| self.render_add_source(cx));

        div()
            .key_context("Workspace")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &ToggleCommandPalette, window, cx| {
                if this.palette_open {
                    this.close_overlays(window, cx);
                } else {
                    this.open_palette(window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleJsonView, _, cx| {
                let view = if this.view == View::Rows {
                    View::Json
                } else {
                    View::Rows
                };
                this.set_view(view, cx);
            }))
            .capture_key_down(cx.listener(Self::capture_key))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(t.bg)
            .font_family(theme::SANS)
            .text_color(t.fg)
            .child(self.render_titlebar(cx))
            .child(
                div().flex_1().min_h_0().flex().child(self.render_sidebar(cx)).child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .flex()
                        .flex_col()
                        .bg(t.bg)
                        .child(screen),
                ),
            )
            .children(palette)
            .children(add)
    }
}

/// A full-window scrim that closes overlays when clicked outside `panel`.
fn modal(t: crate::theme::Tokens, panel: impl IntoElement, top: Option<f32>, cx: &mut Context<Workspace>) -> Div {
    div()
        .absolute()
        .inset_0()
        .occlude()
        .bg(t.scrim)
        .flex()
        .justify_center()
        .map(|el| match top {
            Some(top) => el.items_start().pt(px(top)),
            None => el.items_center(),
        })
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, window, cx| this.close_overlays(window, cx)),
        )
        .child(panel)
}
