//! Receiving: this device on a radar, ready for nearby devices, with a
//! code and a QR code for devices that cannot see it (FR-020).

use dexdrop_core::{
    api::{Command, DirectState},
    invitation::Band,
};
use nsh_base::theme;
use nsh_core::prelude::*;

use crate::state::engine;
use crate::{
    bridge,
    pages::body,
    ui::{self, Icon, Variant, muted, space},
};

/// What a screen reader says for the code: "Pairing code: 9 S V Y, W F X Y".
pub fn spoken_code(shown: &str) -> String {
    if !shown.chars().any(|c| c.is_ascii_alphanumeric()) {
        return "Pairing code: not ready yet".into();
    }
    let halves: Vec<String> = shown
        .split('-')
        .map(|half| half.chars().map(String::from).collect::<Vec<_>>().join(" "))
        .collect();
    format!("Pairing code: {}", halves.join(", "))
}

pub fn page() -> impl IntoElement {
    // A fresh code and invitation whenever the page shows (coming back to
    // it from another tab too); they end when the user leaves it. Asked for
    // after this update, so a page shown again in place gets its new code
    // after the old one closed.
    spawn(async {
        bridge::send(Command::CreateCode);
        bridge::send(Command::CreateInvitation);
    });
    cleanup(|| bridge::send(Command::CloseInvitations));
    let me = move || {
        engine::status().with(|engine::EngineStatus(status)| {
            status
                .as_ref()
                .map(|s| s.device_name.clone())
                .unwrap_or_default()
        })
    };
    let platform = move || {
        engine::status().with(|engine::EngineStatus(status)| {
            status
                .as_ref()
                .map(|s| s.platform.clone())
                .unwrap_or_default()
        })
    };
    let visible = move || {
        engine::status()
            .with(|engine::EngineStatus(status)| status.as_ref().is_some_and(|s| s.visible))
    };
    body(
        view()
            .gap(px(space::XL))
            .items_center()
            .child(ui::radar(
                if nsh_core::element::window().height.0 < 760.0 {
                    180.0
                } else {
                    240.0
                },
                ui::avatar(me, platform, crate::state::my_avatar, 84.0),
            ))
            .child(
                view()
                    .items_center()
                    .gap(px(space::XS))
                    .child(
                        ui::title(move || {
                            if visible() {
                                "Ready to receive".to_owned()
                            } else {
                                "Hidden".to_owned()
                            }
                        })
                        .w_full()
                        .text_center(),
                    )
                    .child(
                        muted(move || {
                            if visible() {
                                format!("Nearby devices see “{}”", me())
                            } else {
                                "Others can still send with the code".to_owned()
                            }
                        })
                        .w_full()
                        .text_center(),
                    ),
            )
            .child(code_card()),
    )
}

/// The code to type and the QR to scan.
fn code_card() -> impl IntoElement {
    let code = move || {
        engine::code().with(|engine::Code(code)| {
            code.as_ref()
                .map(|(c, _)| dexdrop_core::invitation::display_code(c))
                .unwrap_or_else(|| "····-····".into())
        })
    };
    let invitation =
        move || engine::invitation().with(|engine::Invitation(invitation)| invitation.clone());
    ui::tile()
        .self_stretch()
        .items_center()
        .gap(px(space::L))
        .py(px(space::XL))
        .child(
            muted("Or enter this code on the other device")
                .w_full()
                .text_center(),
        )
        .child(
            text(code)
                .w_full()
                .text_center()
                .text_size(px(ui::size::DISPLAY))
                .font_weight(FontWeight::BOLD)
                .letter_spacing(px(3.0))
                .text_color(move || theme().foreground)
                .selectable(true)
                // Spelled out, so screen readers read the code itself.
                .label(move || spoken_code(&code())),
        )
        // A code works once: after it is used, offer a fresh one.
        .child(when(
            move || engine::code().with(|engine::Code(code)| code.is_none()),
            || {
                ui::icon_btn(Icon::Refresh, "New code", Variant::Secondary)
                    .on_press(|| bridge::send(Command::CreateCode))
            },
        ))
        // Before the QR code, which it then explains, and above the fold.
        .child(direct_link())
        .child(dynamic(move || {
            invitation().map(|inv| {
                // The link first: on a short phone the QR code may need a
                // scroll, the button should not.
                let text = inv.text.clone();
                view()
                    .items_center()
                    .gap(px(space::M))
                    .child(
                        ui::icon_btn(Icon::Link, "Copy invitation", Variant::Plain)
                            .test_id("copy-invitation")
                            .on_press(move || {
                                crate::state::copy(text.clone(), "Invitation copied.");
                            }),
                    )
                    .child(ui::qr(inv.qr_width, inv.qr_modules.clone(), 188.0))
            })
        }))
}

/// The band's name, as the hosted network says it.
fn band_words(band: Band) -> &'static str {
    match band {
        Band::Ghz5 => " (5 GHz)",
        Band::Ghz2_4 => " (2.4 GHz)",
        Band::Auto => "",
    }
}

/// A network this device hosts for one transfer (docs/CONNECTIVITY.md
/// 4.3), for a device with no network in common: its QR code then carries
/// the network, and the other device joins it when it scans the code. Only
/// where this device can host one; nothing changes elsewhere.
fn direct_link() -> impl IntoElement {
    let state = computed(move || {
        engine::status().with(|engine::EngineStatus(status)| {
            status
                .as_ref()
                .map(|s| s.direct.clone())
                .unwrap_or_default()
        })
    });
    dynamic(move || match state.get() {
        DirectState::Unavailable => None,
        DirectState::Off => Some(
            view()
                .items_center()
                .gap(px(space::XS))
                .child(muted("Not on the same Wi-Fi?"))
                .child(
                    ui::icon_btn(Icon::Wifi, "Start a direct link", Variant::Plain)
                        .test_id("direct-link")
                        .on_press(|| bridge::send(Command::HostDirectLink)),
                )
                .into_element(),
        ),
        DirectState::Starting => Some(
            view()
                .flex_row()
                .items_center()
                .gap(px(space::S))
                .child(ui::spinner(16.0))
                .child(muted("Starting a direct link…"))
                .into_element(),
        ),
        DirectState::Hosting { band, .. } => Some(
            view()
                .self_stretch()
                .items_center()
                .gap(px(space::S))
                .child(
                    view()
                        .test_id("direct-link-on")
                        .flex_row()
                        .items_center()
                        .gap(px(space::S))
                        .child(ui::icon(Icon::Wifi, 16.0, move || {
                            ui::tone_color(ui::Tone::Success)
                        }))
                        .child(
                            muted(format!(
                                "Direct link on{}. The other device joins it when it scans \
                                 this QR code.",
                                band_words(band)
                            ))
                            .flex_1(),
                        ),
                )
                .child(
                    ui::icon_btn(Icon::X, "Stop the direct link", Variant::Plain)
                        .test_id("direct-link-stop")
                        .on_press(|| bridge::send(Command::StopDirectLink)),
                )
                .into_element(),
        ),
        DirectState::Failed(why) => Some(
            view()
                .self_stretch()
                .items_center()
                .gap(px(space::S))
                .child(
                    muted(format!("The direct link could not start. {why}"))
                        .text_color(move || theme().destructive)
                        .w_full()
                        .text_center(),
                )
                .child(
                    ui::icon_btn(Icon::Refresh, "Try again", Variant::Plain)
                        .test_id("direct-link")
                        .on_press(|| bridge::send(Command::HostDirectLink)),
                )
                .into_element(),
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::spoken_code;

    #[test]
    fn screen_readers_get_the_code_itself() {
        assert_eq!(spoken_code("9SVY-WFXY"), "Pairing code: 9 S V Y, W F X Y");
        assert_eq!(spoken_code("········"), "Pairing code: not ready yet");
    }
}
