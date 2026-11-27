//! Progress, waiting and the QR code, drawn.

use nsh_base::theme;
use nsh_core::{element::AccessibilityRole, prelude::*};

use super::*;

/// A theme color as the current appearance shows it (for drawing).
fn pick(color: AdaptiveColor) -> Hsla {
    if dark() { color.dark } else { color.light }
}

/// A circular progress ring of `side` points, the fraction in `value`.
pub fn progress_ring(
    value: impl Fn() -> f64 + Copy + 'static,
    tone: impl Fn() -> Tone + Copy + 'static,
    side: f32,
) -> View {
    let stroke = (side * 0.075).max(6.0);
    let r = (side - stroke) / 2.0;
    view()
        .size(px(side))
        .role(AccessibilityRole::Progressbar)
        .label(move || format!("{}%", (value() * 100.0).round()))
        .child(
            canvas(move || {
                let fraction = value().clamp(0.0, 1.0) as f32;
                let start = -std::f32::consts::FRAC_PI_2;
                let mut painter = Painter::new()
                    .stroke_color(pick(theme().muted))
                    .line_width(stroke)
                    .begin_path()
                    .arc(side / 2.0, side / 2.0, r, 0.0, std::f32::consts::TAU)
                    .stroke();
                if fraction > 0.0 {
                    painter = painter
                        .stroke_color(pick(tone_color(tone())))
                        .line_cap(nsh_core::canvas::LineCap::Round)
                        .begin_path()
                        .arc(
                            side / 2.0,
                            side / 2.0,
                            r,
                            start,
                            start + fraction * std::f32::consts::TAU,
                        )
                        .stroke();
                }
                painter
            })
            .size_full(),
        )
}

/// Rings that pulse out from the center, as a radar looking for devices.
pub fn radar(side: f32, center: impl IntoElement) -> View {
    let ring = move |delay: f32| {
        view()
            .absolute()
            .size(px(side / 2.0))
            .rounded(px(side / 4.0))
            .border_2()
            .border_color(move || theme().foreground.opacity(0.18))
            .animation(
                KeyframeAnimation::new(Keyframes::Ping)
                    .duration(motion::PING)
                    .delay(delay),
            )
    };
    view()
        .size(px(side))
        .test_id("radar")
        .items_center()
        .justify_center()
        // The pings say "looking"; with reduced motion the still circles do.
        .child(
            when(
                move || !reduce_motion(),
                move || {
                    view()
                        .absolute()
                        .size_full()
                        .items_center()
                        .justify_center()
                        .child(ring(0.0))
                        .child(ring(motion::PING / 3.0))
                        .child(ring(motion::PING * 2.0 / 3.0))
                },
            )
            .otherwise(move || {
                // Still: three circles, fainter outwards, where the pings go.
                let still = move |fraction: f32, alpha: f32| {
                    view()
                        .absolute()
                        .size(px(side * fraction))
                        .rounded(px(side * fraction / 2.0))
                        .border_2()
                        .border_color(move || theme().foreground.opacity(alpha))
                };
                view()
                    .absolute()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .child(still(1.0, 0.06))
                    .child(still(0.76, 0.1))
                    .child(still(0.52, 0.16))
            }),
        )
        .child(center)
}

/// A small turning indicator: work in progress without a known end. A
/// faint ring with a bright arc going round it (pulsing instead with
/// reduced motion).
pub fn spinner(side: f32) -> View {
    let animation = if reduce_motion() {
        KeyframeAnimation::new(Keyframes::Pulse).duration(motion::PULSE)
    } else {
        KeyframeAnimation::new(Keyframes::Spin).duration(motion::SPIN)
    };
    let stroke = (side * 0.13).max(2.0);
    let r = (side - stroke) / 2.0;
    view().size(px(side)).animation(animation).child(
        canvas(move || {
            let c = side / 2.0;
            let top = -std::f32::consts::FRAC_PI_2;
            Painter::new()
                .stroke_color(pick(theme().primary.opacity(0.16)))
                .line_width(stroke)
                .begin_path()
                .arc(c, c, r, 0.0, std::f32::consts::TAU)
                .stroke()
                .stroke_color(pick(theme().primary))
                .line_cap(nsh_core::canvas::LineCap::Round)
                .begin_path()
                .arc(c, c, r, top, top + std::f32::consts::PI * 0.62)
                .stroke()
        })
        .size_full(),
    )
}

/// A placeholder in the shape of what is loading, gently pulsing (still
/// with reduced motion).
pub fn skeleton(width: Option<f32>, height: f32, radius: f32) -> View {
    let v = view()
        .h(px(height))
        .rounded(px(radius))
        .bg(move || theme().muted);
    let v = match width {
        Some(w) => v.w(px(w)),
        None => v.flex_1(),
    };
    if reduce_motion() {
        v
    } else {
        v.animation(KeyframeAnimation::new(Keyframes::Pulse).duration(motion::PULSE))
    }
}

/// `n` rows of skeletons: a rounded square and two lines, as a list of
/// apps or files shows while it loads.
pub fn skeleton_rows(n: usize) -> View {
    group().children((0..n).map(|_| {
        view()
            .flex_row()
            .items_center()
            .gap(px(space::M))
            .px(px(space::L))
            .py(px(space::M))
            .child(skeleton(Some(40.0), 40.0, 10.0))
            .child(
                view()
                    .flex_1()
                    .gap(px(6.0))
                    .child(
                        view()
                            .flex_row()
                            .child(skeleton(None, 12.0, 6.0))
                            .child(view().w(px(60.0))),
                    )
                    .child(skeleton(Some(90.0), 10.0, 5.0)),
            )
    }))
}

/// A QR code drawn dark on light whatever the appearance (`qr-dark` on
/// `qr-light`), as scanners read best.
pub fn qr(width: usize, modules: Vec<bool>, side: f32) -> impl IntoElement {
    let quiet = 2.0;
    let total = width as f32 + quiet * 2.0;
    let mut path = String::with_capacity(modules.len() * 8);
    for (i, dark) in modules.iter().enumerate() {
        if *dark {
            let x = (i % width) as f32 + quiet;
            let y = (i / width) as f32 + quiet;
            path.push_str(&format!("M{x} {y}h1v1h-1z"));
        }
    }
    let scale = side / total;
    view()
        .size(px(side))
        .bg(|| token("qr-light", theme().background))
        .rounded(px(radius::lg()))
        .label("QR code invitation")
        .child(
            canvas(move || {
                Painter::new()
                    .scale(scale, scale)
                    .fill_color(pick(token("qr-dark", theme().foreground)))
                    .fill_svg(&path)
            })
            .size_full(),
        )
}
