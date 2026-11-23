//! Settings: device name, visibility, receiving, trusted devices, privacy
//! and diagnostics (FR-070 – FR-075).

use dexdrop_core::{
    api::{Command, DiscoveryStatus, NoticeKind},
    store::TrustedPeer,
};
use nsh_base::theme;
use nsh_core::{file_dialog::OpenDialog, prelude::*};

use crate::{
    bridge,
    state::{Sheet, desktop_platform, model, platform_label, sheet, toast},
    ui::{self, Variant, btn, card, divider, muted, section_label, space, switch_row},
};

pub fn page() -> impl IntoElement {
    view()
        .gap(px(space::XL))
        .child(ui::page_header("Settings", || {
            "Everything stays on this device.".to_owned()
        }))
        .child(device())
        .child(receiving())
        .child(trusted())
        .child(privacy())
        .child(about())
}

fn settings() -> dexdrop_core::store::Settings {
    model().with(|m| m.settings.clone())
}

fn device() -> impl IntoElement {
    let draft = signal(String::new());
    let editing = signal(false);
    let save = move || {
        let name = draft.get_untracked();
        if name.trim().is_empty() {
            toast(NoticeKind::Error, "The device name cannot be empty.");
            return;
        }
        bridge::send(Command::SetDeviceName(name));
        editing.set(false);
    };
    card()
        .child(section_label("This device"))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(space::M))
                .child(ui::avatar(move || settings().device_name, 44.0))
                .child(
                    view()
                        .flex_1()
                        .gap(px(2.0))
                        .child(
                            when(
                                move || editing.get(),
                                move || {
                                    nsh_base::input()
                                        .value(move || draft.get())
                                        .on_change(move |v: &str| draft.set(v.to_owned()))
                                        .on_submit(move |_: &str| save())
                                        .placeholder("Device name")
                                        .min_h(px(ui::hit()))
                                        .px(px(space::M))
                                        .border_1()
                                        .border_color(move || theme().input)
                                        .rounded(move || theme().radius)
                                        .text_color(move || theme().foreground)
                                },
                            )
                            .otherwise(move || {
                                ui::body(move || settings().device_name)
                                    .font_weight(FontWeight::SEMIBOLD)
                            }),
                        )
                        .child(muted(move || {
                            model().with(|m| match &m.status {
                                Some(s) => format!(
                                    "{} · ID {}",
                                    platform_label(&s.platform),
                                    s.fingerprint
                                ),
                                None => "Starting…".into(),
                            })
                        })),
                )
                .child(
                    when(
                        move || editing.get(),
                        move || btn("Save", Variant::Primary).on_press(save),
                    )
                    .otherwise(move || {
                        btn("Rename", Variant::Secondary).on_press(move || {
                            draft.set(settings().device_name);
                            editing.set(true);
                        })
                    }),
                ),
        )
        .child(divider())
        .child(switch_row(
            "Visible to nearby devices",
            "Other devices on this network can see this one and offer files. You can always send.",
            move || settings().visible,
            |on| bridge::send(Command::SetVisible(on)),
        ))
        .child(muted(move || {
            model().with(|m| match m.status.as_ref().map(|s| &s.discovery) {
                Some(DiscoveryStatus::Active) => "Nearby discovery is on.".to_owned(),
                Some(DiscoveryStatus::Unavailable(why)) => {
                    format!("Nearby discovery is unavailable: {why}")
                }
                Some(DiscoveryStatus::Disabled) => "Nearby discovery is turned off.".to_owned(),
                None => m.offline.clone().unwrap_or_default(),
            })
        }))
}

fn receiving() -> impl IntoElement {
    let folder = move || {
        model().with(|m| {
            m.settings
                .download_dir
                .clone()
                .or_else(default_dir)
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        })
    };
    card()
        .child(section_label("Receiving"))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(space::M))
                .child(
                    view()
                        .flex_1()
                        .gap(px(2.0))
                        .child(ui::body("Save received files to"))
                        .child(muted(folder).number_of_lines(2)),
                )
                .child(when(desktop_platform, || {
                    btn("Change…", Variant::Secondary).on_press(|| {
                        let picked =
                            pick_paths(OpenDialog::folders().button_label("Use this folder"));
                        spawn(async move {
                            if let Ok(Some(paths)) = picked.await
                                && let Some(dir) = paths.into_iter().next()
                            {
                                bridge::send(Command::SetDownloadDir(Some(dir)));
                            }
                        });
                    })
                })),
        )
        .child(when(
            move || settings().download_dir.is_some(),
            || {
                btn("Use the default folder", Variant::Ghost)
                    .self_start()
                    .on_press(|| bridge::send(Command::SetDownloadDir(None)))
            },
        ))
        .child(divider())
        .child(switch_row(
            "Accept trusted devices automatically",
            "Only devices you trusted and allowed below. Everything else always asks first.",
            move || settings().auto_accept_trusted,
            |on| bridge::send(Command::SetAutoAcceptTrusted(on)),
        ))
        .child(when(
            move || !desktop_platform(),
            move || {
                switch_row(
                    "Use mobile data",
                    "Off: transfers stay on Wi‑Fi and local networks.",
                    move || settings().allow_metered,
                    |on| bridge::send(Command::SetAllowMetered(on)),
                )
            },
        ))
}

#[cfg(not(target_arch = "wasm32"))]
fn default_dir() -> Option<std::path::PathBuf> {
    Some(bridge::default_download_dir())
}

#[cfg(target_arch = "wasm32")]
fn default_dir() -> Option<std::path::PathBuf> {
    None
}

fn trusted() -> impl IntoElement {
    let empty = move || model().with(|m| m.trusted.is_empty());
    card().child(section_label("Trusted devices")).child(
        when(empty, || {
            muted("None yet. When you accept a transfer you can choose to trust the sender.")
        })
        .otherwise(|| {
            view().gap(px(space::S)).child(each(
                move || model().with(|m| m.trusted.clone()),
                |p: &TrustedPeer| p.peer_id,
                |p, _| trusted_row(p),
            ))
        }),
    )
}

fn trusted_row(peer: ReadSignal<TrustedPeer>) -> impl IntoElement {
    let p = move || peer.get();
    view()
        .gap(px(space::XS))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(space::M))
                .child(ui::avatar(move || p().name, 32.0))
                .child(
                    view()
                        .flex_1()
                        .child(ui::body(move || p().name))
                        .child(muted(move || {
                            format!(
                                "{} · {}",
                                platform_label(&p().platform),
                                p().peer_id.fingerprint()
                            )
                        })),
                )
                .child(
                    btn("Forget", Variant::Danger)
                        .on_press(move || bridge::send(Command::ForgetPeer(p().peer_id))),
                ),
        )
        .child(switch_row(
            "Accept without asking",
            "Requires “Accept trusted devices automatically”.",
            move || p().auto_accept,
            move |on| {
                bridge::send(Command::SetPeerAutoAccept {
                    peer_id: peer.with_untracked(|p| p.peer_id),
                    auto_accept: on,
                })
            },
        ))
}

fn privacy() -> impl IntoElement {
    card()
        .child(section_label("Privacy and storage"))
        .child(muted(
            "Files go directly between your devices over an encrypted connection and are never uploaded to a server. \
             Every file is verified before it is saved, and nothing is saved without your answer unless you allowed a trusted device.",
        ))
        .child(muted(
            "Stored on this device only: its name and key, your settings, trusted devices, the state needed to resume \
             interrupted transfers and, if kept, a short history.",
        ))
        .child(switch_row(
            "Keep transfer history",
            "A local list of recent transfers.",
            move || settings().keep_history,
            |on| bridge::send(Command::SetKeepHistory(on)),
        ))
}

fn about() -> impl IntoElement {
    card()
        .child(section_label("About"))
        .child(muted(format!(
            "DexDrop {} · Native Shell",
            env!("CARGO_PKG_VERSION")
        )))
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap(px(space::S))
                .child(
                    btn("Export diagnostics", Variant::Secondary)
                        .on_press(|| bridge::send(Command::ExportDiagnostics)),
                )
                .child(
                    btn("Reset device identity…", Variant::Secondary)
                        .on_press(|| sheet().set(Sheet::ResetIdentity)),
                ),
        )
        .child(muted(
            "Diagnostics contain no file names, paths, contents or keys.",
        ))
}

/// Asks before giving this device a new identity (PRD 9.9).
pub fn reset_identity_sheet() -> impl IntoElement {
    let busy = move || {
        model()
            .with(|m| m.transfers.iter().any(|t| !t.state.is_finished()) || !m.requests.is_empty())
    };
    view()
        .gap(px(space::M))
        .child(ui::body(
            "Other devices will see this one as a new device. Devices that trusted it will \
             ask again before accepting its files, and its old ID stops working.",
        ))
        .child(muted(
            "Your device name, trusted devices, history and received files stay.",
        ))
        .child(when(busy, || {
            muted("Finish or cancel the transfers in progress first.")
        }))
        .child(
            view()
                .flex_row()
                .justify_end()
                .gap(px(space::S))
                .child(btn("Cancel", Variant::Secondary).on_press(|| sheet().set(Sheet::None)))
                .child(btn("Reset identity", Variant::Danger).on_press(move || {
                    if busy() {
                        toast(
                            NoticeKind::Error,
                            "Finish or cancel the transfers in progress first.",
                        );
                        return;
                    }
                    sheet().set(Sheet::None);
                    #[cfg(not(target_arch = "wasm32"))]
                    bridge::reset_identity();
                })),
        )
}
