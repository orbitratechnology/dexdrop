//! Receiving: this device on a radar, ready for nearby devices, with a
//! code and a QR code for devices that cannot see it (FR-020).

use dexdrop_core::api::Command;
use nsh_base::theme;
use nsh_core::prelude::*;

use crate::{
    bridge,
    pages::body,
    state::model,
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
        model().with(|m| {
            m.status
                .as_ref()
                .map(|s| s.device_name.clone())
                .unwrap_or_default()
        })
    };
    let platform = move || {
        model().with(|m| {
            m.status
                .as_ref()
                .map(|s| s.platform.clone())
                .unwrap_or_default()
        })
    };
    let visible = move || model().with(|m| m.status.as_ref().is_some_and(|s| s.visible));
    body(
        view()
            .gap(px(space::XL))
            .items_center()
            .child(ui::radar(
                ui::RECEIVE,
                if nsh_core::element::window().height.0 < 760.0 {
                    180.0
                } else {
                    240.0
                },
                ui::avatar(me, platform, ui::my_avatar, 84.0),
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
        model().with(|m| {
            m.code
                .as_ref()
                .map(|(c, _)| dexdrop_core::invitation::display_code(c))
                .unwrap_or_else(|| "····-····".into())
        })
    };
    let invitation = move || model().with(|m| m.invitation.clone());
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
            move || model().with(|m| m.code.is_none()),
            || {
                ui::icon_btn(Icon::Refresh, "New code", Variant::Secondary)
                    .on_press(|| bridge::send(Command::CreateCode))
            },
        ))
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

#[cfg(test)]
mod tests {
    use super::spoken_code;

    #[test]
    fn screen_readers_get_the_code_itself() {
        assert_eq!(spoken_code("9SVY-WFXY"), "Pairing code: 9 S V Y, W F X Y");
        assert_eq!(spoken_code("········"), "Pairing code: not ready yet");
    }
}
