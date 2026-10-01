use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{Icon, icon};
use crate::theme::{MONO, Tokens};

/// Container for a segmented control.
pub fn segmented(t: Tokens, children: impl IntoIterator<Item = AnyElement>) -> Div {
    div()
        .flex()
        .items_center()
        .p(px(2.))
        .gap(px(2.))
        .rounded(px(8.))
        .bg(t.bg3)
        .border_1()
        .border_color(t.line)
        .children(children)
}

/// One option in a segmented control. The caller attaches `on_click`.
pub fn segment(
    t: Tokens,
    id: impl Into<ElementId>,
    label: SharedString,
    leading: Option<AnyElement>,
    active: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(26.))
        .px(px(10.))
        .flex()
        .items_center()
        .gap(px(6.))
        .rounded(px(6.))
        .border_1()
        .text_size(px(12.5))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .when(active, |el| el.bg(t.bg).border_color(t.line2).text_color(t.fg))
        .when(!active, |el| el.border_color(t.ink(0.)).text_color(t.fg2).hover(|s| s.text_color(t.fg)))
        .children(leading)
        .child(label)
}

/// Filled, inverted button: white on dark, black on light.
pub fn primary_button(t: Tokens, id: impl Into<ElementId>, label: impl Into<SharedString>, glyph: Option<Icon>) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(34.))
        .px(px(14.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(px(7.))
        .rounded(px(8.))
        .bg(t.inv)
        .text_color(t.invfg)
        .text_size(px(13.5))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .hover(|s| s.opacity(0.9))
        .children(glyph.map(|g| icon(g, 14., t.invfg)))
        .child(label.into())
}

pub fn outline_button(t: Tokens, id: impl Into<ElementId>, label: impl Into<SharedString>, glyph: Option<Icon>) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(32.))
        .px(px(12.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(px(7.))
        .rounded(px(7.))
        .border_1()
        .border_color(t.line2)
        .bg(t.bg)
        .text_color(t.fg)
        .text_size(px(13.))
        .cursor_pointer()
        .hover(|s| s.bg(t.bg3))
        .children(glyph.map(|g| icon(g, 14., t.fg)))
        .child(label.into())
}

/// Square ghost button holding one icon.
pub fn icon_button(t: Tokens, id: impl Into<ElementId>, glyph: Icon, size: f32) -> Stateful<Div> {
    let id = id.into();
    let group: SharedString = format!("icon-button-{id}").into();
    div()
        .id(id)
        .group(group.clone())
        .size(px(28.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .cursor_pointer()
        .hover(|s| s.bg(t.bg3))
        .child(icon(glyph, size, t.fg2).group_hover(group, |s| s.text_color(t.fg)))
}

pub fn pill(t: Tokens, label: impl Into<SharedString>) -> Div {
    div()
        .px(px(7.))
        .py(px(1.))
        .rounded_full()
        .border_1()
        .border_color(t.line2)
        .text_size(px(10.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(t.fg2)
        .child(label.into())
}

pub fn kbd(t: Tokens, label: impl Into<SharedString>) -> Div {
    div()
        .px(px(5.))
        .h(px(18.))
        .flex()
        .items_center()
        .rounded(px(4.))
        .border_1()
        .border_color(t.line2)
        .font_family(MONO)
        .text_size(px(10.5))
        .text_color(t.fg3)
        .child(label.into())
}

pub fn dot(color: Hsla, size: f32) -> Div {
    div().size(px(size)).flex_none().rounded_full().bg(color)
}

/// A status dot with a soft halo, as on the server card.
pub fn halo_dot(t: Tokens, color: Hsla, halo: bool) -> Div {
    div()
        .size(px(13.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .when(halo, |el| el.bg(t.ok_ring))
        .child(dot(color, 7.))
}

pub fn switch(t: Tokens, on: bool) -> Div {
    div()
        .relative()
        .w(px(30.))
        .h(px(18.))
        .flex_none()
        .rounded_full()
        .bg(if on { t.inv } else { t.line2 })
        .child(
            div()
                .absolute()
                .top(px(2.))
                .left(px(if on { 14. } else { 2. }))
                .size(px(14.))
                .rounded_full()
                .bg(if on { t.invfg } else { t.fg2 }),
        )
}

/// The design's 14px checkbox with a hand-drawn 10px check.
pub fn checkbox(t: Tokens, on: bool) -> Div {
    div()
        .size(px(14.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .border_1()
        .border_color(if on { t.inv } else { t.line2 })
        .when(on, |el| el.bg(t.inv).child(icon(Icon::CheckboxCheck, 10., t.invfg)))
}

/// Page header used by Sources, Storage and Team.
pub fn page_header(t: Tokens, title: &'static str, subtitle: impl Into<SharedString>, action: Option<AnyElement>) -> Div {
    div()
        .px(px(36.))
        .pt(px(32.))
        .pb(px(20.))
        .flex()
        .items_end()
        .gap(px(20.))
        .border_b_1()
        .border_color(t.line)
        .child(
            div()
                .flex_1()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(div().text_size(px(24.)).font_weight(FontWeight::SEMIBOLD).text_color(t.fg).child(title))
                .child(div().text_size(px(14.)).text_color(t.fg2).child(subtitle.into())),
        )
        .children(action)
}

/// A bordered, rounded table container.
pub fn card(t: Tokens) -> Div {
    div().rounded(px(10.)).border_1().border_color(t.line2).overflow_hidden()
}
