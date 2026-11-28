//! Devices' faces: avatars, their badges, how a transfer travels.

use nsh_base::theme;
use nsh_core::prelude::*;

use super::*;

/// Avatar colors, (from, to) of a gradient: a device without a chosen
/// avatar gets one by its name; a chosen avatar names its own by index,
/// which devices tell each other ([`dexdrop_core::avatar`]). These are the
/// avatars' data, not the app's colors, so they live here rather than in
/// the theme (`app/DESIGN.md`, "Color"); each keeps white text above 3:1.
pub const AVATAR: [(u32, u32); dexdrop_core::avatar::COLORS as usize] = [
    (0x6366f1, 0x8b5cf6),
    (0x0ea5e9, 0x3b82f6),
    (0x10b981, 0x14b8a6),
    (0xd97706, 0xea580c),
    (0xec4899, 0xf43f5e),
    (0x8b5cf6, 0xd946ef),
    (0x64748b, 0x334155),
    (0x65a30d, 0x15803d),
];

/// The avatar colors' names, for screen readers (the swatches).
pub const AVATAR_NAMES: [&str; dexdrop_core::avatar::COLORS as usize] = [
    "Indigo", "Blue", "Green", "Orange", "Pink", "Purple", "Gray", "Lime",
];

fn avatar_colors(name: &str) -> (u32, u32) {
    // FNV-1a: similar names still spread over the colors.
    let hash = name.bytes().fold(0x811c_9dc5u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    });
    AVATAR[(hash % AVATAR.len() as u32) as usize]
}

/// An avatar's gradient.
pub fn avatar_fill(colors: (u32, u32)) -> View {
    view()
        .bg(rgb(colors.0))
        .bg_linear(deg(135.0))
        .gradient_from(rgb(colors.0))
        .gradient_to(rgb(colors.1))
}

/// An avatar's face: the chosen emoji on its color, or else the name's
/// initial on the name's color.
pub fn face(name: &str, chosen: Option<&str>, diameter: f32) -> View {
    let chosen = chosen.and_then(dexdrop_core::avatar::Avatar::parse);
    let colors = chosen
        .as_ref()
        .map_or_else(|| avatar_colors(name), |a| AVATAR[a.color as usize]);
    let inner = match chosen {
        Some(a) => text(a.emoji).text_size(px(diameter * 0.5)),
        None => text(
            name.chars()
                .find(|c| c.is_alphanumeric())
                .unwrap_or('?')
                .to_uppercase()
                .to_string(),
        )
        .text_size(px(diameter * 0.4))
        .font_weight(FontWeight::BOLD)
        .text_color(on_color),
    };
    avatar_fill(colors)
        .size(px(diameter))
        .rounded(px(diameter / 2.0))
        .items_center()
        .justify_center()
        .child(inner)
}

/// A small icon set on another element (an avatar's corner, a picture):
/// on a disc of the card's color with a soft shadow, so it stands apart
/// from what is under it. The icon fills most of the disc.
pub fn badge_disc(side: f32, child: impl IntoElement) -> View {
    view()
        .size(px(side))
        .rounded(px(side / 2.0))
        .bg(surface)
        .shadow_md()
        .items_center()
        .justify_center()
        .child(child)
}

/// A device's avatar: its chosen emoji or its initial on a gradient circle,
/// with its kind (phone, laptop, desktop) in a small badge.
pub fn avatar(
    name: impl Fn() -> String + Copy + 'static,
    platform: impl Fn() -> String + Copy + 'static,
    chosen: impl Fn() -> Option<String> + Copy + 'static,
    diameter: f32,
) -> View {
    let badge = (diameter * 0.38).max(16.0);
    view()
        .size(px(diameter))
        .child(dynamic(move || {
            face(&name(), chosen().as_deref(), diameter)
        }))
        .child(
            view()
                .absolute()
                .right(px(-2.0))
                .bottom(px(-2.0))
                .child(dynamic(move || {
                    badge_disc(badge + 4.0, os_badge(&platform(), badge * 0.62))
                })),
        )
}

/// How a transfer's data goes, in words and an icon (PRD 9.5: a relay is
/// always shown).
pub fn route_label(route: dexdrop_core::api::Route) -> (&'static str, Icon) {
    use dexdrop_core::api::Route;
    match route {
        Route::Nearby => ("On this network", Icon::Wifi),
        Route::PeerToPeer => ("Directly over Wi-Fi", Icon::Wifi),
        Route::Direct => ("Over the internet", Icon::Globe),
        Route::Relayed => ("Over the internet, through a relay", Icon::Globe),
    }
}

/// An avatar with a globe: a device reached over the internet.
pub fn far_avatar(
    name: impl Fn() -> String + Copy + 'static,
    platform: impl Fn() -> String + Copy + 'static,
    chosen: impl Fn() -> Option<String> + Copy + 'static,
    diameter: f32,
    far: impl Fn() -> bool + 'static,
) -> View {
    let badge = (diameter * 0.38).max(16.0);
    view()
        .size(px(diameter))
        .child(avatar(name, platform, chosen, diameter))
        .child(when(far, move || {
            view()
                .absolute()
                .left(px(-2.0))
                .bottom(px(-2.0))
                .child(badge_disc(
                    badge + 4.0,
                    icon(Icon::Globe, badge * 0.62, move || theme().foreground),
                ))
        }))
}

#[cfg(test)]
mod tests {
    use dexdrop_core::api::Route;

    /// Each route says how the data goes in words of its own; a
    /// device-to-device link is not "this network", which there may not be.
    #[test]
    fn every_route_says_how_the_data_goes() {
        let routes = [
            Route::Nearby,
            Route::PeerToPeer,
            Route::Direct,
            Route::Relayed,
        ];
        let words: Vec<_> = routes.iter().map(|r| super::route_label(*r).0).collect();
        assert_eq!(
            super::route_label(Route::PeerToPeer).0,
            "Directly over Wi-Fi"
        );
        for (i, w) in words.iter().enumerate() {
            assert!(!words[i + 1..].contains(w), "{w} is said twice");
        }
    }
}
