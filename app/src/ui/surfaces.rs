//! Cards, groups, rows, status marks and empty states.

use nsh_base::{ButtonElement, theme};
use nsh_core::prelude::*;

use super::*;

// ---------------------------------------------------------------- surfaces

/// The fill of cards and groups: white in light, on the off-white page
/// with a soft shadow (`edged`); one step above the page's background in
/// dark.
pub fn surface() -> AdaptiveColor {
    theme().card
}

/// A rounded group of rows on the page's grouped background.
pub fn group() -> View {
    edged(
        view()
            .rounded(px(radius::lg()))
            .bg(surface)
            .overflow_hidden(),
    )
}

/// A soft shadow under a card: in light, cards and the page are both
/// white; no borders anywhere.
pub fn edged(v: View) -> View {
    v.shadow_md()
}

/// A tile: a rounded card.
pub fn tile() -> View {
    view()
        .p(px(space::L))
        .gap(px(space::M))
        .rounded(px(radius::xl()))
        .bg(surface)
        .shadow_md()
}

/// A row of a [`group`]: leading view, a title with a detail line under it
/// (left out when empty), and a trailing view.
pub fn row(
    leading: impl IntoElement,
    title: impl IntoReactive<SharedString> + 'static,
    detail: impl Fn() -> String + Copy + 'static,
    trailing: impl IntoElement,
) -> View {
    view()
        .flex_row()
        .items_center()
        .gap(px(space::M))
        .min_h(px(hit() + 12.0))
        .px(px(space::L))
        .py(px(space::S))
        .child(leading)
        .child(
            view()
                .flex_1()
                .gap(px(2.0))
                .child(body(title).number_of_lines(1))
                .child(when(
                    move || !detail().is_empty(),
                    move || muted(detail).number_of_lines(1),
                )),
        )
        .child(trailing)
}

/// A pressable row with a chevron.
pub fn nav_row(
    leading: impl IntoElement,
    title: impl IntoReactive<SharedString> + 'static,
    value: impl IntoReactive<SharedString> + 'static,
    on_press: impl Fn() + 'static,
) -> ButtonElement {
    pressable()
        .on_press(on_press)
        .flex_row()
        .items_center()
        .gap(px(space::M))
        .min_h(px(hit() + 12.0))
        .px(px(space::L))
        .py(px(space::S))
        .active(move |s| s.bg(move || theme().muted))
        .hover(move |s| s.bg(move || theme().muted.opacity(0.6)))
        .child(leading)
        .child(body(title).flex_1().number_of_lines(1))
        .child(muted(value).number_of_lines(1).max_w(px(180.0)))
        .child(icon(Icon::ChevronRight, 18.0, move || {
            theme().muted_foreground
        }))
}

/// A thin separator inside a [`group`], inset past the leading glyph.
pub fn separator(inset: f32) -> View {
    view()
        .h(px(1.0))
        .ml(px(inset))
        .bg(move || theme().border.opacity(0.6))
}

/// A rounded group of rows with no shadow: the grouped style of iOS's
/// lists, white cells on the page's grouped background (one step above it
/// in dark).
pub fn flat_group() -> View {
    view()
        .rounded(px(radius::lg()))
        .bg(surface)
        .overflow_hidden()
}

/// A short status label on a tinted capsule.
pub fn pill(
    label: impl IntoReactive<SharedString> + 'static,
    tone: impl Fn() -> Tone + Copy + 'static,
) -> View {
    view()
        .flex_row()
        .self_start()
        .px(px(10.0))
        .py(px(3.0))
        .rounded(px(radius::FULL))
        .bg(move || tone_color(tone()).opacity(0.14))
        .child(
            text(label)
                .text_size(px(size::CAPTION))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || tone_color(tone())),
        )
}

/// A small status dot.
pub fn dot(tone: impl Fn() -> Tone + 'static) -> View {
    view()
        .size(px(8.0))
        .rounded(px(radius::xs()))
        .bg(move || tone_color(tone()))
}

/// The app's illustrations: clean vector pictures in the icon's blues for
/// empty states, waits and outcomes (`app/assets/art`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Art {
    History,
    Devices,
    Done,
    Failed,
    Waiting,
    Trusted,
    Apps,
    Drop,
}

impl Art {
    fn asset(self) -> nsh_core::element::ImageAsset {
        match self {
            Art::History => asset!("art/history.svg"),
            Art::Devices => asset!("art/devices.svg"),
            Art::Done => asset!("art/done.svg"),
            Art::Failed => asset!("art/failed.svg"),
            Art::Waiting => asset!("art/waiting.svg"),
            Art::Trusted => asset!("art/trusted.svg"),
            Art::Apps => asset!("art/apps.svg"),
            Art::Drop => asset!("art/drop.svg"),
        }
    }
}

/// An illustration `width` wide (4:3), decorative: the words beside it
/// say what it means.
pub fn art(a: Art, width: f32) -> nsh_core::element::Image {
    image(a.asset())
        .test_id(&format!("art-{}", format!("{a:?}").to_ascii_lowercase()))
        .w(px(width))
        .h(px(width * 0.75))
        .accessible(false)
}

/// An illustration and one line for an empty state.
pub fn empty(a: Art, line: impl IntoReactive<SharedString> + 'static) -> View {
    view()
        .items_center()
        .gap(px(space::M))
        .py(px(space::XL))
        .child(art(a, 200.0))
        .child(
            muted(line)
                .text_center()
                .w_full()
                .max_w(px(280.0))
                .self_center(),
        )
}
