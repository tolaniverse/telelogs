//! Design tokens from the Telelogs design (dark and light), Geist fonts, and gpui-kit theming.

use std::borrow::Cow;

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;

pub const SANS: &str = "Geist";
pub const MONO: &str = "Geist Mono";

#[derive(Clone, Copy)]
pub struct Tokens {
    pub dark: bool,
    pub bg: Hsla,
    pub bg2: Hsla,
    pub bg3: Hsla,
    pub bg4: Hsla,
    pub line: Hsla,
    pub line2: Hsla,
    pub fg: Hsla,
    pub fg2: Hsla,
    pub fg3: Hsla,
    pub err: Hsla,
    pub errbg: Hsla,
    pub warn: Hsla,
    pub ok: Hsla,
    pub ok_ring: Hsla,
    pub inv: Hsla,
    pub invfg: Hsla,
    pub dot: Hsla,
    pub scrim: Hsla,
}

impl Global for Tokens {}

pub fn hex(rgb_hex: u32) -> Hsla {
    rgb(rgb_hex).into()
}

pub fn hexa(rgb_hex: u32, alpha: f32) -> Hsla {
    let mut color: Hsla = rgb(rgb_hex).into();
    color.a = alpha;
    color
}

impl Tokens {
    pub fn dark() -> Self {
        Tokens {
            dark: true,
            bg: hex(0x000000),
            bg2: hex(0x0a0a0a),
            bg3: hex(0x151515),
            bg4: hex(0x1f1f1f),
            line: hex(0x1c1c1c),
            line2: hex(0x2b2b2b),
            fg: hex(0xededed),
            fg2: hex(0xa1a1a1),
            fg3: hex(0x7d7d7d),
            err: hex(0xff6166),
            errbg: hexa(0xff6166, 0.07),
            warn: hex(0xf2a33a),
            ok: hex(0x3ecf8e),
            ok_ring: hexa(0x3ecf8e, 0.15),
            inv: hex(0xededed),
            invfg: hex(0x0a0a0a),
            dot: hexa(0xffffff, 0.075),
            scrim: hexa(0x000000, 0.45),
        }
    }

    pub fn light() -> Self {
        Tokens {
            dark: false,
            bg: hex(0xffffff),
            bg2: hex(0xfafafa),
            bg3: hex(0xf2f2f2),
            bg4: hex(0xe9e9e9),
            line: hex(0xececec),
            line2: hex(0xdedede),
            fg: hex(0x171717),
            fg2: hex(0x5c5c5c),
            fg3: hex(0x757575),
            err: hex(0xd93036),
            errbg: hexa(0xd93036, 0.06),
            warn: hex(0xa35200),
            ok: hex(0x18794e),
            ok_ring: hexa(0x18794e, 0.15),
            inv: hex(0x171717),
            invfg: hex(0xffffff),
            dot: hexa(0x000000, 0.09),
            scrim: hexa(0x000000, 0.25),
        }
    }

    /// Full-strength ink for particles and dots: white on dark, black on light.
    pub fn ink(&self, alpha: f32) -> Hsla {
        hsla(0., 0., if self.dark { 1. } else { 0. }, alpha)
    }
}

pub fn tokens(cx: &App) -> Tokens {
    *cx.global::<Tokens>()
}

pub fn load_fonts(cx: &mut App) -> anyhow::Result<()> {
    cx.text_system().add_fonts(vec![
        Cow::Borrowed(include_bytes!("../assets/fonts/Geist-Regular.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/Geist-Medium.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/Geist-SemiBold.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/GeistMono-Regular.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/GeistMono-Medium.ttf")),
    ])
}

/// Installs the tokens and points gpui-kit's components (inputs, scrollbars) at them.
pub fn apply(dark: bool, cx: &mut App) {
    let t = if dark { Tokens::dark() } else { Tokens::light() };
    cx.set_global(t);
    Theme::change(if dark { ThemeMode::Dark } else { ThemeMode::Light }, None, cx);
    Theme::update(cx, |theme| {
        theme.font_family = SANS.into();
        theme.mono_font_family = MONO.into();
        theme.font_size = px(13.);
        theme.radius = px(8.);
        theme.radius_lg = px(10.);
        theme.background = t.bg;
        theme.foreground = t.fg;
        theme.border = t.line2;
        theme.input = t.line2;
        theme.ring = t.fg3;
        theme.caret = t.fg;
        theme.selection = t.ink(0.2);
        theme.muted = t.bg3;
        theme.muted_foreground = t.fg3;
        theme.primary = t.inv;
        theme.primary_hover = t.inv;
        theme.primary_active = t.inv;
        theme.primary_foreground = t.invfg;
        theme.secondary = t.bg3;
        theme.secondary_hover = t.bg4;
        theme.secondary_active = t.bg4;
        theme.secondary_foreground = t.fg;
        theme.accent = t.bg3;
        theme.accent_foreground = t.fg;
        theme.popover = t.bg;
        theme.popover_foreground = t.fg;
        theme.scrollbar = t.ink(0.);
        theme.scrollbar_thumb = t.line2;
        theme.scrollbar_thumb_hover = t.fg3;
        theme.danger = t.err;
        theme.warning = t.warn;
        theme.success = t.ok;
        theme.info = t.fg2;
    });
    cx.refresh_windows();
}
