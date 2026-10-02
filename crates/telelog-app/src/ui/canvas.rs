//! Canvas animations from the design: the brand mark, the rotating dot sphere,
//! and the static dot grid backdrop.

use std::sync::OnceLock;
use std::time::Instant;

use gpui_kit::*;

use crate::theme::Tokens;

/// Seconds since the first animation frame; shared so every animation stays in phase.
fn now() -> f32 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f32()
}

fn circle(window: &mut Window, center: Point<Pixels>, radius: f32, color: Hsla) {
    let bounds = Bounds::new(
        point(center.x - px(radius), center.y - px(radius)),
        size(px(radius * 2.), px(radius * 2.)),
    );
    window.paint_quad(fill(bounds, color).corner_radii(px(radius)));
}

fn dims(bounds: Bounds<Pixels>) -> (f32, f32) {
    (bounds.size.width / px(1.), bounds.size.height / px(1.))
}

/// The brand mark (`assets/brand/mark.svg`) on a 32×32 grid: a row of dots, then a stem
/// that fades out like a stream of lines. `(x, y, radius, alpha)`.
const MARK: [(f32, f32, f32, f32); 9] = [
    (4., 5., 2.4, 1.),
    (10., 5., 2.4, 1.),
    (16., 5., 2.4, 1.),
    (22., 5., 2.4, 1.),
    (28., 5., 2.4, 1.),
    (16., 11., 2.4, 1.),
    (16., 16.6, 2.1, 0.75),
    (16., 21.7, 1.75, 0.5),
    (16., 26.3, 1.4, 0.28),
];

/// The Telelogs mark. While `animate` is set a pulse runs down the stem; otherwise it is
/// drawn once so the window can sit idle.
pub fn brand_mark(t: Tokens, animate: bool) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let time = now();
            let (w, _) = dims(bounds);
            let scale = w / 32.;
            for (x, y, radius, alpha) in MARK {
                let alpha = if animate {
                    // A bright band falling from the bar to the end of the stem, once a second.
                    let band = (time * 1.2).fract() * 34. - 2.;
                    let lift = (-((y - band) / 4.).powi(2)).exp();
                    (alpha * 0.55 + lift * 0.6).min(1.)
                } else {
                    alpha
                };
                let center = point(bounds.origin.x + px(x * scale), bounds.origin.y + px(y * scale));
                circle(window, center, radius * scale, t.ink(alpha));
            }
            if animate {
                window.request_animation_frame();
            }
        },
    )
    .size(px(22.))
    .flex_none()
}

/// A static grid of faint dots, used behind overlays and empty pages.
pub fn dot_grid(t: Tokens) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let (w, h) = dims(bounds);
            let step = 18.;
            let mut y = step / 2.;
            while y < h {
                let mut x = step / 2.;
                while x < w {
                    let dot = Bounds::new(
                        point(bounds.origin.x + px(x - 0.75), bounds.origin.y + px(y - 0.75)),
                        size(px(1.5), px(1.5)),
                    );
                    window.paint_quad(fill(dot, t.dot).corner_radii(px(0.75)));
                    x += step;
                }
                y += step;
            }
        },
    )
    .absolute()
    .inset_0()
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SphereMode {
    Connecting,
    Disconnected,
    Idle,
}

fn sphere_points() -> &'static [[f32; 3]] {
    static POINTS: OnceLock<Vec<[f32; 3]>> = OnceLock::new();
    POINTS.get_or_init(|| {
        // Fibonacci sphere: evenly spread points.
        const N: usize = 520;
        (0..N)
            .map(|i| {
                let y = 1. - (i as f32 / (N - 1) as f32) * 2.;
                let r = (1. - y * y).sqrt();
                let theta = i as f32 * 2.39996;
                [theta.cos() * r, y, theta.sin() * r]
            })
            .collect()
    })
}

/// A rotating sphere of dots with a scan line sweeping through it. When disconnected it
/// slows down, wobbles and the scan line turns red.
pub fn sphere(t: Tokens, mode: SphereMode, diameter: f32) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let time = now();
            let (w, h) = dims(bounds);
            let disconnected = mode == SphereMode::Disconnected;
            let connecting = mode != SphereMode::Idle;
            let radius = w.min(h) * 0.36;
            let (cx, cy) = (w / 2., h / 2.);
            let a = time * if disconnected { 0.08 } else { 0.32 };
            let b: f32 = 0.38;
            let (ca, sa, cb, sb) = (a.cos(), a.sin(), b.cos(), b.sin());
            let scan_y = (time * if connecting { 1.6 } else { 0.9 }).sin();
            let breathe = if connecting && !disconnected {
                1. + 0.035 * (time * 3.).sin()
            } else {
                1.
            };

            for (i, p) in sphere_points().iter().enumerate() {
                let x1 = p[0] * ca + p[2] * sa;
                let z1 = -p[0] * sa + p[2] * ca;
                let y2 = p[1] * cb - z1 * sb;
                let z2 = p[1] * sb + z1 * cb;
                let depth = (z2 + 1.) / 2.;
                let wobble = if disconnected {
                    1. + 0.22 * (time * 0.9 + i as f32 * 1.7).sin()
                } else {
                    1.
                };
                let m = breathe * wobble;
                let scan = (-(y2 - scan_y).powi(2) * 36.).exp();
                let alpha = (0.06 + 0.45 * depth + 0.6 * scan * depth).min(1.);
                let color = if disconnected && scan > 0.4 {
                    let mut err = t.err;
                    err.a = alpha;
                    err
                } else {
                    t.ink(alpha)
                };
                let center = point(
                    bounds.origin.x + px(cx + x1 * radius * m),
                    bounds.origin.y + px(cy + y2 * radius * m),
                );
                circle(window, center, 0.5 + depth * 1.1 + scan * 0.6, color);
            }
            window.request_animation_frame();
        },
    )
    .size(px(diameter))
    .flex_none()
}
