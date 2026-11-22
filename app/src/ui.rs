//! The app's own component layer (PRD 7.6, NSH design guides): finite
//! spacing and type scales, semantic variants, and colors only from the
//! theme's tokens. Pages compose these instead of styling raw views.

use nsh_base::{ButtonElement, button, switch, theme};
use nsh_core::{element::AccessibilityRole, prelude::*};

use crate::state::desktop_platform;

/// Spacing scale, in points.
pub mod space {
    pub const XS: f32 = 4.0;
    pub const S: f32 = 8.0;
    pub const M: f32 = 12.0;
    pub const L: f32 = 16.0;
    pub const XL: f32 = 24.0;
}

/// Type scale, in points.
pub mod size {
    pub const TITLE: f32 = 24.0;
    pub const HEADING: f32 = 17.0;
    pub const BODY: f32 = 15.0;
    pub const SMALL: f32 = 13.0;
    pub const CAPTION: f32 = 12.0;
}

/// The smallest hit target: 44 pt on iOS, 48 dp on Android, compact on
/// pointer-driven desktops (PRD 7.7).
pub fn hit() -> f32 {
    match os().name() {
        "android" => 48.0,
        "ios" => 44.0,
        _ => 34.0,
    }
}

pub fn success() -> AdaptiveColor {
    theme().color("success").unwrap_or(theme().primary)
}

pub fn warning() -> AdaptiveColor {
    theme().color("warning").unwrap_or(theme().destructive)
}

/// A button's look.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    Primary,
    Secondary,
    Ghost,
    Danger,
}

/// A labelled button.
pub fn btn(label: impl Into<SharedString>, variant: Variant) -> ButtonElement {
    let label: SharedString = label.into();
    let (bg, fg): (fn() -> AdaptiveColor, fn() -> AdaptiveColor) = match variant {
        Variant::Primary => (|| theme().primary, || theme().primary_foreground),
        Variant::Secondary => (|| theme().secondary, || theme().secondary_foreground),
        Variant::Ghost => (|| theme().background.opacity(0.0), || theme().foreground),
        Variant::Danger => (|| theme().destructive.opacity(0.12), || theme().destructive),
    };
    let mut b = button()
        .min_h(px(hit()))
        .px(px(if desktop_platform() { 14.0 } else { 18.0 }))
        .flex_row()
        .items_center()
        .justify_center()
        .gap(px(space::S))
        .rounded(move || theme().radius)
        .bg(move || bg())
        .disabled_style(|s| s.opacity(0.45))
        .active(|s| s.opacity(0.75));
    if variant == Variant::Ghost {
        b = b.hover(|s| s.bg(move || theme().accent));
    } else {
        b = b.hover(|s| s.opacity(0.9));
    }
    if variant == Variant::Secondary {
        b = b.border_1().border_color(move || theme().border);
    }
    b.accessibility_label(label.clone()).child(
        text(label)
            .text_size(px(size::BODY))
            .font_weight(FontWeight::MEDIUM)
            .number_of_lines(1)
            .text_color(move || fg()),
    )
}

/// A surface that groups related content.
pub fn card() -> View {
    view()
        .p(px(space::L))
        .gap(px(space::M))
        .rounded(move || theme().radius)
        .bg(move || theme().card)
        .border_1()
        .border_color(move || theme().border)
}

/// A page title with an optional subtitle.
pub fn page_header(
    title: &'static str,
    subtitle: impl Fn() -> String + 'static,
) -> impl IntoElement {
    view()
        .gap(px(2.0))
        .child(
            text(title)
                .text_size(px(size::TITLE))
                .font_weight(FontWeight::BOLD)
                .text_color(move || theme().foreground)
                .role(AccessibilityRole::Header),
        )
        .child(
            text(subtitle)
                .text_size(px(size::SMALL))
                .text_color(move || theme().muted_foreground),
        )
}

/// A small uppercase section label.
pub fn section_label(label: impl Into<SharedString>) -> Text {
    let label: SharedString = label.into();
    text(label.to_uppercase())
        .text_size(px(size::CAPTION))
        .font_weight(FontWeight::SEMIBOLD)
        .letter_spacing(px(0.5))
        .text_color(move || theme().muted_foreground)
        .role(AccessibilityRole::Header)
}

/// Secondary text.
pub fn muted(content: impl IntoReactive<SharedString> + 'static) -> Text {
    text(content)
        .text_size(px(size::SMALL))
        .text_color(move || theme().muted_foreground)
}

/// Body text.
pub fn body(content: impl IntoReactive<SharedString> + 'static) -> Text {
    text(content)
        .text_size(px(size::BODY))
        .text_color(move || theme().foreground)
}

/// The tone of a badge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Accent,
    Success,
    Warning,
    Danger,
}

fn tone_color(tone: Tone) -> AdaptiveColor {
    match tone {
        Tone::Neutral => theme().muted_foreground,
        Tone::Accent => theme().primary,
        Tone::Success => success(),
        Tone::Warning => warning(),
        Tone::Danger => theme().destructive,
    }
}

/// A short status label.
pub fn badge(label: impl Into<SharedString>, tone: Tone) -> View {
    view()
        .flex_row()
        .self_start()
        .px(px(8.0))
        .py(px(2.0))
        .rounded(px(999.0))
        .bg(move || tone_color(tone).opacity(0.14))
        .child(
            text(label.into())
                .text_size(px(size::CAPTION))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || tone_color(tone)),
        )
}

/// A small status dot.
pub fn dot(tone: impl Fn() -> Tone + 'static) -> View {
    view()
        .size(px(8.0))
        .rounded(px(4.0))
        .bg(move || tone_color(tone()))
}

/// A device's avatar: its initial on a tinted circle.
pub fn avatar(name: impl Fn() -> String + 'static, diameter: f32) -> View {
    view()
        .size(px(diameter))
        .rounded(px(diameter / 2.0))
        .items_center()
        .justify_center()
        .bg(move || theme().primary.opacity(0.14))
        .child(
            text(move || {
                name()
                    .chars()
                    .next()
                    .unwrap_or('?')
                    .to_uppercase()
                    .to_string()
            })
            .text_size(px(diameter * 0.42))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(move || theme().primary),
        )
}

/// A thin determinate progress bar.
pub fn progress_bar(value: impl Fn() -> f64 + 'static, tone: impl Fn() -> Tone + 'static) -> View {
    view()
        .h(px(6.0))
        .rounded(px(3.0))
        .bg(move || theme().muted)
        .overflow_hidden()
        .role(AccessibilityRole::Progressbar)
        .child(
            view()
                .h_full()
                .rounded(px(3.0))
                .w(move || relative(value().clamp(0.0, 1.0) as f32))
                .bg(move || tone_color(tone())),
        )
}

/// A labelled switch row (FR-071 and friends).
pub fn switch_row(
    title: &'static str,
    detail: &'static str,
    checked: impl Fn() -> bool + 'static,
    on_change: impl Fn(bool) + 'static,
) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap(px(space::M))
        .min_h(px(hit()))
        .child(
            view()
                .flex_1()
                .gap(px(2.0))
                .child(body(title))
                .child(muted(detail)),
        )
        .child(
            switch()
                .checked(checked)
                .on_change(move |on: &bool| on_change(*on))
                .accessibility_label(title)
                .track_class("w-11 rounded-full bg-muted p-0.5 transition-colors checked:bg-primary")
                .thumb_class("size-5 rounded-full bg-background shadow transition-transform checked:translate-x-5"),
        )
}

/// A horizontal rule.
pub fn divider() -> View {
    view().h(px(1.0)).bg(move || theme().border)
}

/// A QR code drawn black on white, whatever the appearance.
pub fn qr(width: usize, modules: Vec<bool>, side: f32) -> impl IntoElement {
    let quiet = 4.0;
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
        .bg(white())
        .rounded(px(8.0))
        .label("QR code invitation")
        .child(
            canvas(move || {
                Painter::new()
                    .scale(scale, scale)
                    .fill_color(black())
                    .fill_svg(&path)
            })
            .size_full(),
        )
}
