//! The app's controls, each on its `nsh_base` counterpart: buttons,
//! fields, switches, segmented choices, progress and ticks. Pages compose
//! these; they take variants, sizes and states, never colors or radii.
//!
//! Every pressable answers in layers (Design Guides, "Layered feedback on
//! controls"): a wash on hover (`motion::INSTANT`), a press scale on a
//! soft settle (`motion::FAST`, none with reduced motion), the focus ring
//! on keyboard focus, and a dimmed look with no hover or press while
//! disabled.

use nsh_base::{ButtonElement, theme};
use nsh_core::{element::PointerEvents, prelude::*};

use super::*;

/// A button's look.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    /// The main action: filled.
    Primary,
    /// A quieter action: tinted.
    Secondary,
    /// Text only.
    Plain,
    /// Destructive: red, tinted.
    Destructive,
}

/// The keyboard's focus ring (PRD 7.5, 7.7): on every focusable control.
pub fn ring(s: nsh_core::element::StyleRefinement) -> nsh_core::element::StyleRefinement {
    s.outline(px(2.0)).outline_color(move || theme().ring)
}

/// A press target: the focus ring, and its state changes eased on the
/// motion tokens. Every control of the app is one, so Tab always shows
/// where it is.
pub fn pressable() -> ButtonElement {
    let [x1, y1, x2, y2] = motion::SETTLE;
    nsh_base::button()
        .focus_visible(ring)
        .transition_all()
        .duration(motion::FAST)
        .ease_bezier(x1, y1, x2, y2)
}

/// The press scale of a control, none with reduced motion.
fn pressed(scale: f32) -> f32 {
    if reduce_motion() { 1.0 } else { scale }
}

pub fn btn(label: impl Into<SharedString>, variant: Variant) -> ButtonElement {
    labelled(label, None, variant)
}

/// [`btn`] with a leading icon.
pub fn icon_btn(i: Icon, label: impl Into<SharedString>, variant: Variant) -> ButtonElement {
    labelled(label, Some(i), variant)
}

type Color = fn() -> AdaptiveColor;

/// A variant's colors: (fill, hover fill, pressed fill, foreground). The
/// primary fill is the gradient, its hover and press a deeper shade.
fn colors(variant: Variant) -> (Color, Color, Color, Color) {
    match variant {
        Variant::Primary => (
            || theme().primary,
            || token("primary-hover", theme().primary),
            || primary_to(),
            || theme().primary_foreground,
        ),
        Variant::Secondary => (
            || theme().primary.opacity(0.12),
            || theme().primary.opacity(0.18),
            || theme().primary.opacity(0.24),
            || theme().primary,
        ),
        Variant::Plain => (
            || theme().primary.opacity(0.0),
            || theme().primary.opacity(0.08),
            || theme().primary.opacity(0.14),
            || theme().primary,
        ),
        Variant::Destructive => (
            || theme().destructive.opacity(0.12),
            || theme().destructive.opacity(0.18),
            || theme().destructive.opacity(0.24),
            || theme().destructive,
        ),
    }
}

fn labelled(label: impl Into<SharedString>, i: Option<Icon>, variant: Variant) -> ButtonElement {
    let label: SharedString = label.into();
    let (bg, hover, active, fg) = colors(variant);
    let primary = variant == Variant::Primary;
    let mut b = pressable()
        .min_h(px(hit() + if mobile() { 6.0 } else { 0.0 }))
        .px(px(if variant == Variant::Plain {
            space::S
        } else {
            20.0
        }))
        .flex_row()
        .items_center()
        .justify_center()
        .gap(px(space::S))
        .rounded(px(radius::FULL))
        .bg(bg)
        .style_when(
            move || primary,
            |s| {
                s.bg_linear(deg(135.0))
                    .gradient_from(primary_from)
                    .gradient_to(primary_to)
            },
        )
        .hover(move |s| {
            s.bg(hover).style_when(
                move || primary,
                |s| s.gradient_from(hover).gradient_to(primary_to),
            )
        })
        .active(move |s| {
            s.bg(active).scale(pressed(motion::PRESS)).style_when(
                move || primary,
                |s| s.gradient_from(active).gradient_to(active),
            )
        })
        .disabled_style(|s| s.opacity(0.4))
        .accessibility_label(label.clone());
    if let Some(i) = i {
        b = b.child(icon(i, 18.0, fg));
    }
    b.child(
        text(label)
            .text_size(px(size::BODY))
            .font_weight(FontWeight::SEMIBOLD)
            .number_of_lines(1)
            .text_color(fg),
    )
}

/// The height of a hero button.
pub const HERO: f32 = 64.0;

/// A hero button: one of a page's two or three big actions (Home's Send
/// and Receive), a capsule `HERO` tall on a shadow, filled with the main
/// action's gradient (`filled`) or the card's color.
pub fn hero_btn(i: Icon, label: &'static str, filled: bool) -> ButtonElement {
    let fg = move || {
        if filled {
            theme().primary_foreground
        } else {
            theme().foreground
        }
    };
    pressable()
        .accessibility_label(label)
        .min_h(px(HERO))
        .rounded(px(radius::FULL))
        .flex_row()
        .items_center()
        .justify_center()
        .gap(px(space::S))
        .bg(move || {
            if filled {
                theme().primary
            } else {
                theme().card
            }
        })
        .style_when(
            move || filled,
            |s| {
                s.bg_linear(deg(135.0))
                    .gradient_from(primary_from)
                    .gradient_to(primary_to)
            },
        )
        .shadow_lg()
        .hover(move |s| {
            s.style_when(
                move || filled,
                |s| s.gradient_from(|| token("primary-hover", theme().primary)),
            )
            .style_when(move || !filled, |s| s.bg(move || theme().accent))
        })
        .active(move |s| s.scale(pressed(motion::PRESS)))
        .disabled_style(|s| s.opacity(0.4))
        .child(icon(i, 24.0, fg))
        .child(
            text(label)
                .text_size(px(size::SECTION))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(fg),
        )
}

/// A round icon-only button, labelled for screen readers: a circle
/// `side` across whose hit area reaches at least [`hit`] (44 pt on iOS,
/// 48 dp on Android) however small the circle is drawn.
pub fn round_btn(
    i: Icon,
    label: impl Into<SharedString>,
    variant: Variant,
    side: f32,
) -> ButtonElement {
    let (bg, hover, active, fg) = match variant {
        Variant::Secondary | Variant::Plain => (
            (|| theme().muted) as Color,
            (|| theme().accent) as Color,
            (|| theme().border) as Color,
            (|| theme().foreground) as Color,
        ),
        other => colors(other),
    };
    let primary = variant == Variant::Primary;
    let slop = ((hit() - side) / 2.0).max(0.0);
    pressable()
        .size(px(side))
        .hit_slop(EdgeInsets::all(px(slop)))
        .rounded(px(radius::FULL))
        .items_center()
        .justify_center()
        .bg(bg)
        .style_when(
            move || primary,
            |s| {
                s.bg_linear(deg(135.0))
                    .gradient_from(primary_from)
                    .gradient_to(primary_to)
            },
        )
        .hover(move |s| {
            s.bg(hover).style_when(
                move || primary,
                |s| s.gradient_from(hover).gradient_to(primary_to),
            )
        })
        .active(move |s| {
            s.bg(active).scale(pressed(motion::PRESS_ICON)).style_when(
                move || primary,
                |s| s.gradient_from(active).gradient_to(active),
            )
        })
        .disabled_style(|s| s.opacity(0.4))
        .accessibility_label(label.into())
        .child(icon(i, side * 0.42, fg))
}

/// A big round control with its caption under it (a transfer's Pause,
/// Resume, Stop): one button to screen readers, named by the caption.
pub fn captioned_btn(i: Icon, label: &'static str, variant: Variant) -> ButtonElement {
    let (bg, hover, active, fg) = match variant {
        Variant::Secondary | Variant::Plain => (
            (|| theme().muted) as Color,
            (|| theme().accent) as Color,
            (|| theme().border) as Color,
            (|| theme().foreground) as Color,
        ),
        other => colors(other),
    };
    pressable()
        .accessibility_label(label)
        .items_center()
        .gap(px(space::S))
        .p(px(space::XS))
        .rounded(px(radius::md()))
        .hover(move |s| s.bg(hover))
        .active(move |s| s.bg(active).scale(pressed(motion::PRESS_ICON)))
        .disabled_style(|s| s.opacity(0.4))
        .child(
            view()
                .size(px(64.0))
                .rounded(px(radius::FULL))
                .items_center()
                .justify_center()
                .bg(bg)
                .child(icon(i, 26.0, fg)),
        )
        .child(muted(label).font_weight(FontWeight::MEDIUM))
}

/// A button whose icon follows its words ("Continue →").
pub fn btn_then(label: impl Into<SharedString>, i: Icon, variant: Variant) -> ButtonElement {
    let (_, _, _, fg) = colors(variant);
    btn(label, variant).child(icon(i, 18.0, fg))
}

/// A quiet button that leads somewhere: its words, then a chevron.
pub fn link_btn(label: impl Into<SharedString>) -> ButtonElement {
    let label: SharedString = label.into();
    pressable()
        .accessibility_label(label.clone())
        .min_h(px(hit()))
        .px(px(space::S))
        .flex_row()
        .items_center()
        .gap(px(2.0))
        .rounded(px(radius::FULL))
        .hover(|s| s.bg(move || theme().primary.opacity(0.08)))
        .active(|s| {
            s.bg(move || theme().primary.opacity(0.14))
                .scale(pressed(motion::PRESS))
        })
        .disabled_style(|s| s.opacity(0.4))
        .child(
            text(label)
                .text_size(px(size::BODY))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().primary),
        )
        .child(icon(Icon::ChevronRight, 16.0, move || theme().primary))
}

/// A filter chip: a capsule that is on (filled in the primary color) or
/// off.
pub fn chip(
    label: impl Fn() -> String + Copy + 'static,
    on: impl Fn() -> bool + Clone + 'static,
) -> ButtonElement {
    let (on2, on3) = (on.clone(), on.clone());
    pressable()
        .accessibility_label(move || Some(SharedString::from(label())))
        .selected(on3)
        .min_h(px(hit().min(40.0)))
        .px(px(space::M))
        .items_center()
        .justify_center()
        .rounded(px(radius::FULL))
        .bg(move || if on() { theme().primary } else { surface() })
        .hover(|s| s.opacity(0.88))
        .active(|s| s.scale(pressed(motion::PRESS)))
        .child(
            text(label)
                .text_size(px(size::SMALL))
                .font_weight(FontWeight::SEMIBOLD)
                .number_of_lines(1)
                .text_color(move || {
                    if on2() {
                        theme().primary_foreground
                    } else {
                        theme().foreground
                    }
                }),
        )
}

// ---------------------------------------------------------------- fields

/// A field's look.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldSize {
    /// A line of a form: body text.
    Regular,
    /// One big value to type (a code): large, spaced and centered.
    Large,
}

/// A text field: the native input on a muted fill, its border showing the
/// focus (the ring color) and a problem (`invalid`, destructive).
pub fn field(label: impl Into<SharedString>, size_: FieldSize) -> nsh_base::Input {
    let label: SharedString = label.into();
    let input = nsh_base::input()
        .label(label)
        .min_h(px(match size_ {
            FieldSize::Regular => hit(),
            FieldSize::Large => 64.0,
        }))
        .px(px(match size_ {
            FieldSize::Regular => space::M,
            FieldSize::Large => space::L,
        }))
        .rounded(px(match size_ {
            FieldSize::Regular => radius::md(),
            FieldSize::Large => radius::xl(),
        }))
        .bg(move || theme().muted)
        .border_2()
        .border_color(move || theme().muted)
        .transition_colors()
        .duration(motion::INSTANT)
        .ease_out()
        .focus(|s| s.border_color(move || theme().ring))
        .text_color(move || theme().foreground);
    match size_ {
        FieldSize::Regular => input.text_size(px(size::BODY)),
        FieldSize::Large => input
            .text_size(px(size::HEADLINE))
            .font_weight(FontWeight::SEMIBOLD)
            .letter_spacing(px(2.0))
            .text_center(),
    }
}

/// A search field: a capsule with the magnifier and the input of `query`,
/// labelled and hinted `label`, ringed while it has the focus.
pub fn search_field(label: &'static str, query: Signal<String>) -> nsh_base::Input {
    nsh_base::input()
        .label(label)
        .placeholder(label)
        .value(move || query.get())
        .on_change(move |v: &str| query.set(v.to_owned()))
        .flex_1()
        .min_h(px(hit()))
        .text_size(px(size::BODY))
        .text_color(move || theme().foreground)
}

/// The capsule around a [`search_field`]: the magnifier, then the field.
pub fn search_bar(field: nsh_base::Input) -> View {
    view()
        .flex_row()
        .items_center()
        .gap(px(space::S))
        .px(px(space::M))
        .rounded(px(radius::FULL))
        .bg(surface)
        .border_2()
        .border_color(surface)
        .transition_colors()
        .duration(motion::INSTANT)
        .ease_out()
        .focus_within(|s| s.border_color(move || theme().ring))
        .child(icon(Icon::Search, 18.0, move || theme().muted_foreground))
        .child(field)
}

// ---------------------------------------------------------------- progress

/// A linear progress bar (`nsh_base::progress`, which has no look of its
/// own): a muted track, 4 or 8 points tall (`thick`), filled in the primary
/// color, the fill easing to each new value (`motion::BASE`).
pub fn progress(value: impl Fn() -> f64 + 'static, thick: bool) -> nsh_base::ProgressElement {
    let track = if thick {
        "h-2 w-full rounded-full bg-muted overflow-hidden"
    } else {
        "h-1 w-full rounded-full bg-muted overflow-hidden"
    };
    nsh_base::progress()
        .value(move || (value().clamp(0.0, 1.0) * 100.0) as f32)
        .track_class(track)
        .indicator_class(format!(
            "h-full rounded-full bg-primary transition-all ease-out duration-{}",
            motion::BASE as u32
        ))
}

// ---------------------------------------------------------------- ticks

/// Where a tick sits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TickOn {
    /// A row of a list: a ring on the page's colors.
    Row,
    /// A picture: a white ring over a dark wash, so it shows on any image.
    Media,
}

/// A round tick, filled with a check when chosen: the mark of a row or
/// picture that is chosen (the row or picture is the checkbox to screen
/// readers; this draws its state).
pub fn tick(on: impl Fn() -> bool + Clone + 'static, over: TickOn) -> View {
    let side = if over == TickOn::Row { 26.0 } else { 24.0 };
    view()
        .size(px(side))
        .rounded(px(radius::FULL))
        .items_center()
        .justify_center()
        .border_2()
        .accessible(false)
        .transition_colors()
        .duration(motion::FAST)
        .ease_out()
        .border_color({
            let on = on.clone();
            move || match (on(), over) {
                (true, _) => theme().primary,
                (false, TickOn::Row) => theme().muted_foreground.opacity(0.4),
                (false, TickOn::Media) => on_media(),
            }
        })
        .bg({
            let on = on.clone();
            move || match (on(), over) {
                (true, _) => theme().primary,
                (false, TickOn::Row) => theme().card,
                (false, TickOn::Media) => media_chip().opacity(0.35),
            }
        })
        .child(when(on, move || {
            icon(Icon::Check, side * 0.56, move || theme().primary_foreground)
        }))
}

// ---------------------------------------------------------------- switches

/// A row with a switch: the system's on phones, the app's on desktops.
pub fn switch_row(
    leading: impl IntoElement,
    title: &'static str,
    checked: impl Fn() -> bool + 'static,
    on_change: impl Fn(bool) + 'static,
) -> View {
    view()
        .flex_row()
        .items_center()
        .gap(px(space::M))
        .min_h(px(hit() + 12.0))
        .px(px(space::L))
        .py(px(space::S))
        .child(leading)
        .child(body(title).flex_1())
        .child(toggle(title, checked, on_change))
}

/// An on/off control: `UISwitch` on iOS, Material 3's switch on Android,
/// the app's own elsewhere.
pub fn toggle(
    label: &'static str,
    checked: impl Fn() -> bool + 'static,
    on_change: impl Fn(bool) + 'static,
) -> AnyElement {
    if mobile() {
        nsh_core::mobile::switch()
            .checked(checked)
            .on_change(move |on: &bool| on_change(*on))
            .accessibility_label(label)
            .into_element()
    } else {
        nsh_base::switch()
            .checked(checked)
            .on_change(move |on: &bool| on_change(*on))
            .accessibility_label(label)
            .min_h(px(hit()))
            .justify_center()
            .rounded(px(radius::FULL))
            .focus_visible(ring)
            .track_class(
                "w-11 rounded-full bg-muted p-0.5 transition-colors duration-150 hover:bg-accent checked:bg-primary",
            )
            .thumb_class(
                "size-5 rounded-full bg-background shadow transition-transform duration-150 checked:translate-x-5",
            )
            .into_element()
    }
}

// ---------------------------------------------------------------- selection

/// One selection that slides between equal places `pitch` apart (Design
/// Guides, "One selection, sliding"): an absolutely placed view at place 0
/// of its container, moved to place `index()` along x (or y, `vertical`)
/// in `motion::BASE`, ease-in-out; it jumps with reduced motion and before
/// the places are measured. The caller sizes, places and colors it.
pub fn sliding_selection(
    index: impl Fn() -> usize + 'static,
    pitch: impl Fn() -> f32 + 'static,
    vertical: bool,
) -> View {
    let at = animated_value(0.0);
    let measured = std::cell::Cell::new(false);
    effect(move || {
        let step = pitch();
        let to = index() as f32 * step;
        if reduce_motion() || step <= 0.0 || !measured.replace(true) {
            at.set(to);
        } else {
            let _ = at.timing(to, motion::BASE, Easing::EaseInOut);
        }
    });
    let v = view()
        .absolute()
        .accessible(false)
        .pointer_events(PointerEvents::None);
    if vertical {
        v.bind_translate_y(&at)
    } else {
        v.bind_translate_x(&at)
    }
}

// ---------------------------------------------------------------- segmented

/// A choice of one among a few, as a segmented control: the system's own
/// on iOS (`UISegmentedControl`); elsewhere, where the platform has none,
/// NSH's `ToggleGroup` of `Toggle`s (a toolbar to screen readers, one
/// keyboard stop with arrow keys), drawn as joined segments under one
/// selection that slides to the chosen one (`motion::BASE`, ease-in-out;
/// it jumps with reduced motion).
pub fn segmented<T: Copy + PartialEq + 'static>(
    items: &'static [(&'static str, T)],
    value: impl Fn() -> T + Copy + 'static,
    on_change: impl Fn(T) + Copy + 'static,
) -> AnyElement {
    let index = move || items.iter().position(|(_, v)| *v == value()).unwrap_or(0) as u32;
    if os().name() == "ios" {
        return nsh_core::mobile::segmented_control()
            .items(
                items
                    .iter()
                    .map(|(l, _)| nsh_core::mobile::SegmentedControlItem::new(*l))
                    .collect::<Vec<_>>(),
            )
            .selected_index(index)
            .on_change(move |i: &u32| {
                if let Some((_, v)) = items.get(*i as usize) {
                    on_change(*v);
                }
            })
            .self_stretch()
            .into_element();
    }
    const INSET: f32 = 3.0;
    let n = items.len().max(1) as f32;
    // The width of one segment, measured; the selection's offset.
    let width = signal(0.0_f32);
    let x = animated_value(0.0);
    effect(move || {
        let to = index() as f32 * width.get();
        if reduce_motion() || width.get_untracked() == 0.0 {
            x.set(to);
        } else {
            let _ = x.timing(to, motion::BASE, Easing::EaseInOut);
        }
    });
    nsh_base::toggle_group()
        .child(
            view()
                .flex_row()
                .p(px(INSET))
                .rounded(px(radius::FULL))
                .bg(move || theme().muted)
                .on_layout(move |b: &Bounds<Points>| {
                    width.set((b.size.width.0 - INSET * 2.0) / n);
                })
                .child(
                    view()
                        .absolute()
                        .top(px(INSET))
                        .bottom(px(INSET))
                        .left(px(INSET))
                        .w(move || -> Length { px(width.get()).into() })
                        .rounded(px(radius::FULL))
                        .bg(move || theme().card)
                        .shadow_sm()
                        .accessible(false)
                        .bind_translate_x(&x),
                )
                .children(items.iter().map(|(label, v)| {
                    let v = *v;
                    let on = move || value() == v;
                    nsh_base::toggle()
                        .pressed(on)
                        .on_change(move |_: &bool| on_change(v))
                        .flex_1()
                        .min_h(px(if mobile() { hit() } else { hit() - INSET * 2.0 }))
                        .px(px(space::M))
                        .items_center()
                        .justify_center()
                        .rounded(px(radius::FULL))
                        .focus_visible(ring)
                        .child(
                            text(*label)
                                .text_size(px(size::SMALL))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || {
                                    if on() {
                                        theme().foreground
                                    } else {
                                        theme().muted_foreground
                                    }
                                }),
                        )
                })),
        )
        .self_stretch()
        .into_element()
}
