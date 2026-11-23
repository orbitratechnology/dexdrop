//! Settings: this device, receiving, trusted devices, privacy (FR-070 –
//! FR-075), in grouped rows with the platform's own switches.

use dexdrop_core::{
    api::{Command, NoticeKind},
    store::TrustedPeer,
};
use nsh_base::{alert_dialog, theme};
use nsh_core::{file_dialog::OpenDialog, prelude::*};

use crate::{
    bridge,
    pages::body,
    state::{Route, desktop_platform, model, platform_label, push, toast},
    ui::{self, Icon, Variant, muted, space},
};

fn settings() -> dexdrop_core::store::Settings {
    model().with(|m| m.settings.clone())
}

/// Glyph colors, one per kind of setting.
const BLUE: (u32, u32) = (0x3b82f6, 0x2563eb);
const GREEN: (u32, u32) = (0x22c55e, 0x16a34a);
const ORANGE: (u32, u32) = (0xf59e0b, 0xea580c);
const PURPLE: (u32, u32) = (0x8b5cf6, 0x7c3aed);
const GRAY: (u32, u32) = (0x94a3b8, 0x64748b);
const RED: (u32, u32) = (0xf43f5e, 0xe11d48);

fn g(i: Icon, colors: (u32, u32)) -> View {
    ui::glyph(i, colors, 30.0)
}

pub fn page() -> impl IntoElement {
    body(
        view()
            .gap(px(space::XL))
            .child(device())
            .child(receiving())
            .child(trust())
            .child(privacy())
            .child(
                muted(format!("DexDrop {}", env!("CARGO_PKG_VERSION")))
                    .self_center()
                    .text_size(px(ui::size::CAPTION)),
            ),
    )
}

/// This device: its avatar and name, renamed in place.
fn device() -> impl IntoElement {
    let draft = signal(String::new());
    let editing = signal(false);
    let save = move || {
        let name = draft.get_untracked();
        if name.trim().is_empty() {
            toast(NoticeKind::Error, "The name can't be empty.");
            return;
        }
        bridge::send(Command::SetDeviceName(name));
        editing.set(false);
    };
    let name = move || settings().device_name;
    let platform = move || {
        model().with(|m| {
            m.status
                .as_ref()
                .map(|s| s.platform.clone())
                .unwrap_or_default()
        })
    };
    ui::tile()
        .items_center()
        .gap(px(space::M))
        .py(px(space::XL))
        .child(ui::avatar(name, platform, 80.0))
        .child(
            when(
                move || editing.get(),
                move || {
                    view()
                        .flex_row()
                        .items_center()
                        .gap(px(space::S))
                        .child(
                            nsh_base::input()
                                .value(move || draft.get())
                                .on_change(move |v: &str| draft.set(v.to_owned()))
                                .on_submit(move |_: &str| save())
                                .placeholder("Device name")
                                .label("Device name")
                                .w(px(220.0))
                                .min_h(px(ui::hit()))
                                .px(px(space::M))
                                .rounded(px(12.0))
                                .bg(move || theme().muted)
                                .text_size(px(ui::size::BODY))
                                .text_color(move || theme().foreground),
                        )
                        .child(ui::btn("Save", Variant::Primary).on_press(save))
                },
            )
            .otherwise(move || {
                ui::pressable()
                    .accessibility_label("Rename")
                    .on_press(move || {
                        draft.set(settings().device_name);
                        editing.set(true);
                    })
                    .flex_row()
                    .items_center()
                    .gap(px(space::S))
                    .min_h(px(ui::hit()))
                    .active(|s| s.opacity(0.6))
                    .child(ui::title(name))
                    .child(ui::icon(Icon::Pencil, 18.0, move || theme().primary))
            }),
        )
        .child(muted(move || {
            model().with(|m| {
                m.status
                    .as_ref()
                    .map(|s| format!("{} · {}", platform_label(&s.platform), s.fingerprint))
                    .unwrap_or_default()
            })
        }))
}

/// The name of the folder received files go to (FR-042).
pub fn save_folder() -> String {
    model().with(|m| {
        m.settings
            .download_dir
            .clone()
            .or_else(default_dir)
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default()
    })
}

/// Where received files go, in a few words (FR-041, FR-073).
pub fn saves_to() -> String {
    if desktop_platform() {
        format!("Saves to {}", save_folder())
    } else {
        "Kept in DexDrop until you share or save them".to_owned()
    }
}

fn receiving() -> impl IntoElement {
    let folder = save_folder;
    view()
        .gap(px(space::S))
        .child(ui::section("Receiving"))
        .child(
            ui::group()
                .child(ui::switch_row(
                    g(Icon::Eye, BLUE),
                    "Visible to nearby devices",
                    move || settings().visible,
                    |on| bridge::send(Command::SetVisible(on)),
                ))
                .child(ui::separator(58.0))
                .child(ui::switch_row(
                    g(Icon::UserCheck, GREEN),
                    "Auto-accept trusted devices",
                    move || settings().auto_accept_trusted,
                    |on| bridge::send(Command::SetAutoAcceptTrusted(on)),
                ))
                // No mobile-data switch: transfers stay on the local network
                // (P0), so they never use mobile data. It returns with remote
                // transfers.
                .child(when(desktop_platform, move || {
                    view().child(ui::separator(58.0)).child(ui::nav_row(
                        g(Icon::Folder, ORANGE),
                        "Save to",
                        folder,
                        || {
                            let picked =
                                pick_paths(OpenDialog::folders().button_label("Use this folder"));
                            spawn(async move {
                                if let Ok(Some(paths)) = picked.await
                                    && let Some(dir) = paths.into_iter().next()
                                {
                                    bridge::send(Command::SetDownloadDir(Some(dir)));
                                }
                            });
                        },
                    ))
                })),
        )
        .child(when(
            move || !desktop_platform(),
            || {
                muted("Received files are kept in DexDrop until you share or save them.")
                    .px(px(space::L))
                    .text_size(px(ui::size::CAPTION))
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

fn trust() -> impl IntoElement {
    let count = move || {
        let n = model().with(|m| m.trusted.len());
        if n == 0 {
            "None".to_owned()
        } else {
            n.to_string()
        }
    };
    view()
        .gap(px(space::S))
        .child(ui::section("Devices"))
        .child(ui::group().child(ui::nav_row(
            g(Icon::Shield, PURPLE),
            "Trusted devices",
            count,
            || push(Route::Trusted),
        )))
}

fn privacy() -> impl IntoElement {
    view()
        .gap(px(space::S))
        .child(ui::section("Privacy"))
        .child(
            ui::group()
                .child(ui::switch_row(
                    g(Icon::Clock, GRAY),
                    "Keep transfer history",
                    move || settings().keep_history,
                    |on| bridge::send(Command::SetKeepHistory(on)),
                ))
                .child(ui::separator(58.0))
                .child(ui::nav_row(
                    g(Icon::Info, GRAY),
                    "Export diagnostics",
                    "",
                    || bridge::send(Command::ExportDiagnostics),
                ))
                .child(ui::separator(58.0))
                .child(ui::nav_row(
                    g(Icon::Refresh, RED),
                    "Reset device identity…",
                    "",
                    reset_identity,
                )),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(space::S))
                .px(px(space::L))
                .child(ui::icon(Icon::Lock, 14.0, move || theme().muted_foreground))
                .child(
                    muted(
                        "Files go straight between your devices, encrypted; nothing is uploaded. \
                         Your device name, trusted devices and history stay on this device.",
                    )
                    .flex_1()
                    .text_size(px(ui::size::CAPTION)),
                ),
        )
}

/// Asks, with the system's alert, before giving this device a new
/// identity (PRD 9.9).
fn reset_identity() {
    let busy = model().with_untracked(|m| {
        m.transfers.iter().any(|t| !t.state.is_finished()) || !m.requests.is_empty()
    });
    if busy {
        toast(
            NoticeKind::Error,
            "Finish or cancel the transfers in progress first.",
        );
        return;
    }
    let answer = alert_dialog()
        .title("Reset this device's identity?")
        .description("Other devices will see it as a new device and ask before trusting it again. Your name, trusted devices and files stay.")
        .cancel("cancel", "Cancel")
        .destructive("reset", "Reset identity")
        .show();
    spawn(async move {
        if answer
            .await
            .is_ok_and(|a| a.action.as_deref() == Some("reset"))
        {
            #[cfg(not(target_arch = "wasm32"))]
            bridge::reset_identity();
        }
    });
}

// ---------------------------------------------------------------- trusted

pub fn trusted_page() -> impl IntoElement {
    let empty = move || model().with(|m| m.trusted.is_empty());
    body(
        when(empty, || {
            ui::empty(
                Icon::Devices,
                "Devices you trust when accepting a transfer show up here.",
            )
        })
        .otherwise(|| {
            view()
                .gap(px(space::S))
                .child(ui::group().child(each(
                    move || model().with(|m| m.trusted.clone()),
                    |p: &TrustedPeer| p.peer_id,
                    |p, i| {
                        view()
                            .child(when(move || i.get() > 0, || ui::separator(16.0)))
                            .child(trusted_row(p))
                    },
                )))
                .child(
                    muted(
                        "“Accept without asking” works while “Auto-accept trusted devices” is on.",
                    )
                    .px(px(space::L))
                    .text_size(px(ui::size::CAPTION)),
                )
        }),
    )
}

fn trusted_row(peer: ReadSignal<TrustedPeer>) -> impl IntoElement {
    let p = move || peer.get();
    let name = move || p().name;
    let platform = move || p().platform;
    view()
        .child(ui::row(
            ui::avatar(name, platform, 40.0),
            name,
            move || platform_label(&p().platform).to_owned(),
            ui::round_btn(
                Icon::Trash,
                format!("Forget {}", peer.with_untracked(|p| p.name.clone())),
                Variant::Danger,
                34.0,
            )
            .on_press(move || bridge::send(Command::ForgetPeer(p().peer_id))),
        ))
        .child(ui::switch_row(
            view().w(px(40.0)),
            "Accept without asking",
            move || p().auto_accept,
            move |on| {
                bridge::send(Command::SetPeerAutoAccept {
                    peer_id: peer.with_untracked(|p| p.peer_id),
                    auto_accept: on,
                })
            },
        ))
}
