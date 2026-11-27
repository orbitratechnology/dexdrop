//! Choosing this device's avatar: an emoji on a color, which other devices
//! see next to its name.

use dexdrop_core::{api::Command, avatar::Avatar};
use nsh_base::theme;
use nsh_core::prelude::*;

use crate::{
    bridge,
    pages::body,
    state::model,
    ui::{self, Icon, muted, space},
};

/// The emoji offered.
const EMOJI: &[&str] = &[
    "🦊", "🐼", "🐨", "🦁", "🐯", "🐸", "🐙", "🦄", "🐳", "🦉", "🐝", "🦋", "🌵", "🌻", "🍀", "🌙",
    "⭐", "🔥", "⚡", "🌈", "🍕", "🍩", "☕", "🎧", "🎮", "🚀", "🛸", "⚽", "🎨", "📷", "💎", "👾",
];

fn name() -> String {
    model().with(|m| m.settings.device_name.clone())
}

fn chosen() -> Option<Avatar> {
    crate::state::my_avatar().as_deref().and_then(Avatar::parse)
}

fn set(avatar: Option<Avatar>) {
    bridge::send(Command::SetAvatar(avatar.map(|a| a.to_string())));
}

pub fn page() -> impl IntoElement {
    // The color stays chosen while no emoji is (the initial shows then).
    let color = signal(chosen().map_or(0, |a| a.color));
    body(
        view()
            .gap(px(space::XL))
            .child(
                ui::tile()
                    .items_center()
                    .gap(px(space::M))
                    .py(px(space::XL))
                    .child(dynamic(move || {
                        ui::face(&name(), crate::state::my_avatar().as_deref(), 112.0)
                    }))
                    .child(ui::title(name))
                    .child(muted("Nearby devices and those you send to see this.")),
            )
            .child(
                view().gap(px(space::S)).child(ui::section("Color")).child(
                    ui::tile()
                        .flex_row()
                        .flex_wrap()
                        .justify_center()
                        .gap(px(space::M))
                        .children((0..dexdrop_core::avatar::COLORS).map(|c| swatch(c, color))),
                ),
            )
            .child(
                view().gap(px(space::S)).child(ui::section("Emoji")).child(
                    ui::tile()
                        .flex_row()
                        .flex_wrap()
                        .justify_center()
                        .gap(px(space::S))
                        .child(initial_choice())
                        .children(EMOJI.iter().map(|e| emoji_choice(e, color))),
                ),
            ),
    )
}

/// A color: picks it for the avatar.
fn swatch(c: u8, color: Signal<u8>) -> impl IntoElement {
    let selected = move || color.get() == c;
    ui::pressable()
        .accessibility_label(format!("Color {}", c + 1))
        .selected(selected)
        .on_press(move || {
            color.set(c);
            if let Some(a) = chosen() {
                set(Avatar::new(&a.emoji, c));
            }
        })
        .size(px(ui::hit().max(44.0)))
        .rounded(px(ui::hit().max(44.0) / 2.0))
        .items_center()
        .justify_center()
        .bg(move || theme().card)
        .style_when(selected, |s| s.shadow_lg())
        .child(
            ui::avatar_fill(ui::AVATAR[c as usize])
                .size(px(34.0))
                .rounded(px(ui::radius::lg()))
                .items_center()
                .justify_center()
                .child(when(selected, || ui::icon(Icon::Check, 16.0, ui::on_color))),
        )
}

/// The device's initial instead of an emoji.
fn initial_choice() -> impl IntoElement {
    let selected = move || crate::state::my_avatar().is_none();
    choice(selected)
        .accessibility_label("Initial")
        .on_press(|| set(None))
        .child(dynamic(|| ui::face(&name(), None, 40.0)))
}

fn emoji_choice(emoji: &'static str, color: Signal<u8>) -> impl IntoElement {
    let selected = move || chosen().is_some_and(|a| a.emoji == emoji);
    choice(selected)
        .accessibility_label(emoji)
        .on_press(move || set(Avatar::new(emoji, color.get_untracked())))
        .child(text(emoji).text_size(px(ui::size::HEADLINE)))
}

fn choice(selected: impl Fn() -> bool + Copy + 'static) -> nsh_base::ButtonElement {
    ui::pressable()
        .selected(selected)
        .size(px(52.0))
        .rounded(px(ui::radius::lg()))
        .items_center()
        .justify_center()
        .bg(move || {
            if selected() {
                theme().primary.opacity(0.16)
            } else {
                theme().muted.opacity(0.5)
            }
        })
        .style_when(selected, |s| s.shadow_md())
        .active(|s| s.scale(ui::motion::PRESS_ICON))
}
