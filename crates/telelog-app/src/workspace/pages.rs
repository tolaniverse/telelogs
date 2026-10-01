//! Sources, Storage, Team and Agents screens.
//!
//! Storage, Team and Agents describe features that are not built yet, so they show the
//! design's layout with real local values and clearly marked "coming soon" sections
//! instead of sample data.

use chrono::{DateTime, Local};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::sidebar::{KINDS, kind_icon, kind_label, user_name};
use super::stream::group_digits;
use super::{BUFFER, Status, Workspace};
use crate::client::BACKLOG;
use crate::theme::{MONO, Tokens, tokens};
use crate::ui::canvas::{SphereMode, dot_grid, sphere};
use crate::ui::widgets::{card, dot, page_header, pill, primary_button, segment, segmented, switch};
use crate::ui::{Icon, icon};

const REPO_URL: &str = "https://github.com/tolaniverse/telelogs";

fn stat(t: Tokens, label: &'static str, glyph: Icon, value: String, unit: impl Into<SharedString>) -> Div {
    div()
        .flex_1()
        .px(px(20.))
        .py(px(18.))
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(12.))
                .text_color(t.fg3)
                .child(icon(glyph, 14., t.fg3))
                .child(label),
        )
        .child(
            div()
                .flex()
                .items_baseline()
                .child(div().text_size(px(26.)).font_weight(FontWeight::MEDIUM).child(value))
                .child(div().text_size(px(14.)).text_color(t.fg3).child(unit.into())),
        )
}

fn labeled(t: Tokens, label: &'static str, value: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(3.))
        .child(div().text_size(px(11.5)).text_color(t.fg3).child(label))
        .child(value)
}

fn table_head(t: Tokens, columns: &[(&'static str, f32)]) -> Div {
    div()
        .h(px(36.))
        .px(px(18.))
        .flex()
        .items_center()
        .gap(px(12.))
        .bg(t.bg2)
        .text_size(px(12.))
        .text_color(t.fg3)
        .children(columns.iter().map(|(label, width)| {
            let col = div().child(*label);
            if *width > 0. {
                col.w(px(*width)).flex_none()
            } else {
                col.flex_1()
            }
        }))
}

/// Section title with an optional "Soon" marker.
fn section(t: Tokens, title: &'static str, soon: bool) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .text_size(px(14.))
        .font_weight(FontWeight::MEDIUM)
        .child(title)
        .when(soon, |el| el.child(pill(t, "Soon")))
}

impl Workspace {
    pub(super) fn render_sources(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = tokens(cx);
        let enabled = self
            .targets
            .iter()
            .filter(|target| self.is_enabled(&target.name))
            .count();
        let (status_word, status_color) = match self.status {
            Status::Connected => ("Connected", t.ok),
            Status::Connecting => ("Connecting", t.fg3),
            Status::Disconnected { .. } => ("Disconnected", t.err),
        };

        let server = div()
            .flex()
            .items_center()
            .gap(px(28.))
            .px(px(20.))
            .py(px(16.))
            .rounded(px(10.))
            .border_1()
            .border_color(t.line2)
            .bg(t.bg2)
            .child(
                div()
                    .w(px(200.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        div()
                            .size(px(36.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(8.))
                            .border_1()
                            .border_color(t.line2)
                            .bg(t.bg)
                            .child(icon(Icon::HardDrive, 18., t.fg)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                div()
                                    .text_size(px(14.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child("telelog-server"),
                            )
                            .child(
                                div()
                                    .font_family(MONO)
                                    .text_size(px(12.))
                                    .text_color(t.fg3)
                                    .child("gRPC · LogService"),
                            ),
                    ),
            )
            .child(
                labeled(
                    t,
                    "Endpoint",
                    div()
                        .flex_1()
                        .font_family(MONO)
                        .text_size(px(12.5))
                        .child(self.server_addr().to_string()),
                )
                .flex_1(),
            )
            .child(
                labeled(
                    t,
                    "Status",
                    div()
                        .flex()
                        .items_center()
                        .gap(px(7.))
                        .text_size(px(13.))
                        .child(dot(status_color, 7.))
                        .child(status_word),
                )
                .flex_1(),
            )
            .child(
                labeled(
                    t,
                    "Tailing",
                    div()
                        .font_family(MONO)
                        .text_size(px(12.5))
                        .child(format!("{enabled} of {} targets", self.targets.len())),
                )
                .flex_1(),
            )
            .child(
                labeled(
                    t,
                    "Backlog",
                    div()
                        .font_family(MONO)
                        .text_size(px(12.5))
                        .child(format!("{BACKLOG} lines / target")),
                )
                .flex_1(),
            );

        let groups = KINDS.into_iter().map(|kind| {
            let targets: Vec<_> = self.targets.iter().filter(|target| target.source == kind).collect();
            let header = div()
                .flex()
                .items_center()
                .gap(px(10.))
                .px(px(16.))
                .py(px(12.))
                .bg(t.bg2)
                .child(icon(kind_icon(kind), 17., t.fg))
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(kind_label(kind)),
                )
                .child(
                    div()
                        .font_family(MONO)
                        .text_size(px(12.))
                        .text_color(t.fg3)
                        .child(match kind {
                            telelog_core::SourceKind::Docker => "via telelog-server",
                            _ => "not connected",
                        }),
                )
                .child(div().flex_1())
                .map(|el| {
                    if targets.is_empty() && kind != telelog_core::SourceKind::Docker {
                        el.child(pill(t, "Soon"))
                    } else {
                        el.child(
                            div()
                                .text_size(px(12.))
                                .text_color(t.fg3)
                                .child(format!("{} targets", targets.len())),
                        )
                    }
                });

            let rows = targets.into_iter().map(|target| {
                let on = self.is_enabled(&target.name);
                let running = target.state == "running";
                let name = target.name.clone();
                let image = target.labels.get("image").cloned().unwrap_or_default();
                div()
                    .h(px(46.))
                    .px(px(16.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .border_t_1()
                    .border_color(t.line)
                    .hover(|s| s.bg(t.bg2))
                    .child(
                        div()
                            .id(SharedString::from(format!("switch-{}", target.id)))
                            .w(px(44.))
                            .flex_none()
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| this.toggle_target(&name, cx)))
                            .child(switch(t, on)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .font_family(MONO)
                            .text_size(px(12.5))
                            .child(target.name.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .font_family(MONO)
                            .text_size(px(12.))
                            .text_color(t.fg3)
                            .child(image),
                    )
                    .child(
                        div()
                            .w(px(110.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .text_size(px(12.))
                            .text_color(t.fg2)
                            .child(dot(if running { t.ok } else { t.fg3 }, 6.))
                            .child(target.state.clone()),
                    )
                    .child(
                        div()
                            .w(px(90.))
                            .flex_none()
                            .flex()
                            .justify_end()
                            .font_family(MONO)
                            .text_size(px(12.))
                            .text_color(t.fg2)
                            .child(format!("{}/min", group_digits(self.lines_per_minute(&target.name)))),
                    )
            });
            card(t).child(header).children(rows)
        });

        div()
            .id("sources")
            .size_full()
            .overflow_y_scroll()
            .child(page_header(
                t,
                "Sources",
                "Targets your telelog-server can tail. Toggle a target to include it in the stream.",
                Some(
                    primary_button(t, "add-source-page", "Add source", Some(Icon::Plus))
                        .on_click(cx.listener(|this, _, _, cx| this.open_add_source(cx)))
                        .into_any_element(),
                ),
            ))
            .child(
                div()
                    .px(px(36.))
                    .pt(px(24.))
                    .pb(px(40.))
                    .flex()
                    .flex_col()
                    .gap(px(24.))
                    .child(server)
                    .children(groups),
            )
    }

    pub(super) fn render_storage(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = tokens(cx);
        let oldest = self
            .oldest_record()
            .map(|time| {
                let local: DateTime<Local> = time.into();
                (local.format("%b %-d").to_string(), local.format(" %Y").to_string())
            })
            .unwrap_or_else(|| ("—".into(), String::new()));

        let stats = card(t)
            .flex()
            .child(
                stat(t, "In-memory buffer", Icon::Memory, group_digits(BUFFER), " lines")
                    .border_r_1()
                    .border_color(t.line),
            )
            .child(
                stat(t, "Archived", Icon::Archive, "—".into(), "")
                    .border_r_1()
                    .border_color(t.line),
            )
            .child(stat(
                t,
                "Oldest in memory",
                Icon::ClockCounterClockwise,
                oldest.0,
                oldest.1,
            ));

        let buckets = card(t)
            .child(table_head(
                t,
                &[
                    ("Bucket", 0.),
                    ("Provider", 0.),
                    ("Region", 0.),
                    ("Size", 90.),
                    ("Status", 110.),
                ],
            ))
            .child(
                div()
                    .px(px(18.))
                    .py(px(28.))
                    .border_t_1()
                    .border_color(t.line)
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(6.))
                    .child(icon(Icon::Cylinder, 22., t.fg3))
                    .child(
                        div()
                            .text_size(px(13.5))
                            .font_weight(FontWeight::MEDIUM)
                            .child("No buckets connected"),
                    )
                    .child(
                        div()
                            .text_size(px(12.5))
                            .text_color(t.fg3)
                            .child("Archiving to S3, Google Cloud Storage, R2 and MinIO is on the roadmap."),
                    ),
            );

        let retention_row =
            |title: &'static str, desc: &'static str, options: &[&'static str], current: &'static str, first: bool| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(20.))
                    .px(px(18.))
                    .py(px(16.))
                    .when(!first, |el| el.border_t_1().border_color(t.line))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .child(div().text_size(px(13.5)).child(title))
                            .child(div().text_size(px(12.5)).text_color(t.fg3).child(desc)),
                    )
                    .child(segmented(
                        t,
                        options.iter().map(|label| {
                            segment(
                                t,
                                SharedString::from(format!("{title}-{label}")),
                                (*label).into(),
                                None,
                                *label == current,
                            )
                            .font_family(MONO)
                            .text_size(px(12.))
                            .cursor_default()
                            .into_any_element()
                        }),
                    ))
            };
        let retention = div()
            .rounded(px(10.))
            .border_1()
            .border_color(t.line2)
            .opacity(0.55)
            .child(retention_row(
                "Archive to bucket after",
                "Lines older than this leave memory and are written as compressed chunks.",
                &["15m", "1h", "6h", "24h"],
                "1h",
                true,
            ))
            .child(retention_row(
                "Delete from bucket after",
                "Applied as a lifecycle rule on every connected bucket.",
                &["30d", "90d", "1y", "Never"],
                "90d",
                false,
            ));

        div()
            .id("storage")
            .size_full()
            .overflow_y_scroll()
            .child(page_header(
                t,
                "Storage",
                "Live lines stay in memory. Everything older will land in buckets you own.",
                Some(
                    primary_button(t, "connect-bucket", "Connect bucket", Some(Icon::Plus))
                        .opacity(0.5)
                        .cursor_default()
                        .into_any_element(),
                ),
            ))
            .child(
                div()
                    .px(px(36.))
                    .pt(px(24.))
                    .pb(px(40.))
                    .flex()
                    .flex_col()
                    .gap(px(28.))
                    .child(stats)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(12.))
                            .child(section(t, "Buckets", true))
                            .child(buckets),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(12.))
                            .child(section(t, "Retention", true))
                            .child(retention),
                    ),
            )
    }

    pub(super) fn render_team(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = tokens(cx);
        let user = user_name();
        let initials: String = user.chars().take(2).collect::<String>().to_uppercase();

        let invite = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .items_center()
                    .opacity(0.55)
                    .child(
                        div()
                            .flex_1()
                            .h(px(36.))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .px(px(12.))
                            .rounded(px(8.))
                            .border_1()
                            .border_color(t.line2)
                            .text_size(px(13.5))
                            .text_color(t.fg3)
                            .child(icon(Icon::EnvelopeSimple, 15., t.fg3))
                            .child("name@company.com"),
                    )
                    .child(
                        div()
                            .h(px(36.))
                            .px(px(12.))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .rounded(px(8.))
                            .border_1()
                            .border_color(t.line2)
                            .text_size(px(13.5))
                            .child("Member")
                            .child(icon(Icon::CaretDown, 12., t.fg3)),
                    )
                    .child(primary_button(t, "invite", "Invite", None).h(px(36.)).cursor_default()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(12.5))
                    .text_color(t.fg3)
                    .child(icon(Icon::LockSimple, 13., t.fg3))
                    .child("Teams, invites and per-source access arrive with Telelog Cloud and multi-user servers."),
            );

        let members = card(t)
            .child(table_head(
                t,
                &[
                    ("Member", 0.),
                    ("Role", 110.),
                    ("Source access", 0.),
                    ("Last active", 100.),
                    ("", 32.),
                ],
            ))
            .child(
                div()
                    .h(px(58.))
                    .px(px(18.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .border_t_1()
                    .border_color(t.line)
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .child(
                                div()
                                    .size(px(32.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_full()
                                    .border_1()
                                    .border_color(t.line2)
                                    .bg(t.bg3)
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(initials),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(2.))
                                    .child(div().text_size(px(13.5)).font_weight(FontWeight::MEDIUM).child(user))
                                    .child(
                                        div()
                                            .font_family(MONO)
                                            .text_size(px(12.))
                                            .text_color(t.fg3)
                                            .child("this device"),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .w(px(110.))
                            .flex_none()
                            .flex()
                            .child(pill(t, "Owner").text_size(px(12.)).py(px(2.)).px(px(9.))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .font_family(MONO)
                            .text_size(px(12.))
                            .text_color(t.fg2)
                            .child("All sources"),
                    )
                    .child(
                        div()
                            .w(px(100.))
                            .flex_none()
                            .text_size(px(12.5))
                            .text_color(t.fg3)
                            .child("Now"),
                    )
                    .child(div().w(px(32.)).flex_none().child(icon(Icon::DotsThree, 18., t.fg3))),
            );

        div()
            .id("team")
            .size_full()
            .overflow_y_scroll()
            .child(page_header(t, "Team", "Local workspace · 1 member", None))
            .child(
                div()
                    .px(px(36.))
                    .pt(px(24.))
                    .pb(px(40.))
                    .flex()
                    .flex_col()
                    .gap(px(28.))
                    .child(invite)
                    .child(members),
            )
    }

    pub(super) fn render_agents(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = tokens(cx);
        let feature = |glyph: Icon, label: &'static str| {
            div()
                .flex()
                .items_center()
                .gap(px(7.))
                .child(icon(glyph, 15., t.fg))
                .child(label)
        };
        div()
            .relative()
            .size_full()
            .overflow_hidden()
            .child(dot_grid(t))
            .child(
                div()
                    .size_full()
                    .p(px(40.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(22.))
                    .child(div().mb(px(-18.)).child(sphere(t, SphereMode::Idle, 300.)))
                    .child(
                        div()
                            .px(px(10.))
                            .py(px(3.))
                            .rounded_full()
                            .border_1()
                            .border_color(t.line2)
                            .bg(t.bg)
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(t.fg2)
                            .child("Coming soon"),
                    )
                    .child(div().text_size(px(44.)).line_height(px(44.)).font_weight(FontWeight::SEMIBOLD).child("telelog-agents"))
                    .child(
                        div()
                            .max_w(px(460.))
                            .text_center()
                            .text_size(px(15.))
                            .line_height(px(23.))
                            .text_color(t.fg2)
                            .child("A small collector that runs next to your workloads and ships logs to your server — so you can tail hosts you can't reach directly."),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(24.))
                            .text_size(px(13.))
                            .text_color(t.fg2)
                            .child(feature(Icon::Feather, "Single static binary"))
                            .child(feature(Icon::ArrowUpRight, "Outbound-only"))
                            .child(feature(Icon::ArrowsClockwise, "Buffers while offline")),
                    )
                    .child(
                        primary_button(t, "watch-repo", "Watch on GitHub", Some(Icon::GithubLogo))
                            .mt(px(6.))
                            .h(px(36.))
                            .on_click(|_, _, cx| cx.open_url(REPO_URL)),
                    ),
            )
    }
}
