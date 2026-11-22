//! Nearby: which devices can I send to, how do I connect if mine is not
//! listed, and is this device ready to receive (PRD 7.2)?

use std::path::PathBuf;

use dexdrop_core::{
    api::{Command, DiscoveryStatus, NoticeKind, PeerInfo, Target},
    format,
};
use nsh_base::theme;
use nsh_core::{file_dialog::OpenDialog, prelude::*};

use crate::{
    bridge,
    state::{
        Selection, Sheet, desktop_platform, model, picked, platform_label, selection, sheet, toast,
    },
    ui::{self, Tone, Variant, btn, card, muted, section_label, space},
};

pub fn page() -> impl IntoElement {
    view()
        .gap(px(space::XL))
        .child(ui::page_header("Nearby", subtitle))
        .child(files_card())
        .child(devices())
        .child(fallback())
}

fn subtitle() -> String {
    model().with(|m| match &m.status {
        Some(s) if s.visible => format!("{} is visible to nearby devices", s.device_name),
        Some(s) => format!("{} is hidden — you can still send", s.device_name),
        None if m.offline.is_some() => "Browser preview".into(),
        None => "Starting…".into(),
    })
}

// ---------------------------------------------------------------- files

/// Sets the selection and counts it in the background.
pub fn select(paths: Vec<PathBuf>) {
    if paths.is_empty() {
        return;
    }
    selection().set(Selection {
        paths: paths.clone(),
        summary: None,
    });
    #[cfg(not(target_arch = "wasm32"))]
    {
        let counting = blocking(move || {
            dexdrop_core::manifest::Manifest::build(&paths)
                .map(|m| (m.files, m.folders, m.total_bytes))
        });
        spawn(async move {
            match counting.await {
                Ok(Ok(summary)) => selection().update(|s| s.summary = Some(summary)),
                Ok(Err(err)) => {
                    toast(
                        NoticeKind::Error,
                        format!("Could not read the selection: {err}"),
                    );
                    selection().set(Selection::default());
                }
                Err(_) => {}
            }
        });
    }
}

pub fn choose(dialog: OpenDialog) {
    let picked = pick_paths(dialog.multiple(true).button_label("Choose"));
    spawn(async move {
        match picked.await {
            Ok(Some(paths)) => select(paths),
            Ok(None) => {}
            Err(_) => choose_staged(),
        }
    });
}

/// Phones give the bytes of picked files, not paths: they are copied to
/// app-private staging first (PRD 15, "OS file providers").
fn choose_staged() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let picked = pick_files(nsh_core::picker::FilePickRequest::new(true, 0));
        spawn(async move {
            let files = picked.await;
            if files.is_empty() {
                return;
            }
            let staging = bridge::default_download_dir()
                .parent()
                .map(|p| p.join("state").join("staging"))
                .unwrap_or_else(|| std::env::temp_dir().join("dexdrop-staging"));
            let staged = blocking(move || {
                let dir = staging.join(dexdrop_core::hex(&dexdrop_core::random_bytes::<6>()));
                std::fs::create_dir_all(&dir)?;
                let mut paths = Vec::new();
                for file in files {
                    let name = dexdrop_core::pathsafe::sanitize(&file.name)
                        .ok()
                        .and_then(|c| c.into_iter().last())
                        .unwrap_or_else(|| "file".into());
                    let path = dexdrop_core::naming::first_free(&dir, &name);
                    std::fs::write(&path, file.data.bytes())?;
                    paths.push(path);
                }
                Ok::<_, std::io::Error>(paths)
            });
            match staged.await {
                Ok(Ok(paths)) => select(paths),
                Ok(Err(err)) => toast(
                    NoticeKind::Error,
                    format!("Could not read the files: {err}"),
                ),
                Err(_) => {}
            }
        });
    }
}

fn file_paths(uris: &[String]) -> Vec<PathBuf> {
    uris.iter()
        .filter_map(|uri| uri.strip_prefix("file://"))
        .map(|path| PathBuf::from(percent_decode(path)))
        .collect()
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    // Windows: `file:///C:/x` → `C:/x`.
    let decoded = String::from_utf8_lossy(&out).into_owned();
    if decoded.len() > 2 && decoded.starts_with('/') && decoded.as_bytes()[2] == b':' {
        decoded[1..].to_owned()
    } else {
        decoded
    }
}

fn selection_text() -> String {
    selection().with(|s| match (s.paths.len(), s.summary) {
        (0, _) => "Choose files or a folder, then pick a device.".into(),
        (_, Some((files, folders, bytes))) => dexdrop_net_summary(files, folders, bytes),
        (n, None) => format!("{n} selected · counting…"),
    })
}

fn dexdrop_net_summary(files: u64, folders: u64, bytes: u64) -> String {
    format!(
        "{} · {}",
        format::item_count(files, folders),
        format::bytes(bytes)
    )
}

fn files_card() -> impl IntoElement {
    let hovering = signal(false);
    let has_selection = move || selection().with(|s| !s.paths.is_empty());
    let first_name = move || {
        selection().with(|s| {
            let names: Vec<String> = s
                .paths
                .iter()
                .take(3)
                .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .collect();
            let more = s.paths.len().saturating_sub(3);
            if more > 0 {
                format!("{} and {more} more", names.join(", "))
            } else {
                names.join(", ")
            }
        })
    };
    card()
        .border_color(move || {
            if hovering.get() {
                theme().primary
            } else {
                theme().border
            }
        })
        .bg(move || {
            if hovering.get() {
                theme().accent
            } else {
                theme().card
            }
        })
        .on_file_drag_enter(move |_: &FileDragEvent| hovering.set(true))
        .on_file_drag_leave(move || hovering.set(false))
        .on_file_drop(move |event: &FileDropEvent| {
            hovering.set(false);
            select(file_paths(&event.uris));
        })
        .child(section_label("What to send"))
        .child(when(has_selection, move || {
            ui::body(first_name).number_of_lines(2)
        }))
        .child(muted(selection_text))
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap(px(space::S))
                .child(btn("Choose files", Variant::Secondary).on_press(|| {
                    if desktop_platform() {
                        choose(OpenDialog::files());
                    } else {
                        choose_staged();
                    }
                }))
                .child(when(desktop_platform, || {
                    btn("Choose folder", Variant::Secondary)
                        .on_press(|| choose(OpenDialog::folders()))
                }))
                .child(when(has_selection, || {
                    btn("Clear", Variant::Ghost).on_press(|| selection().set(Selection::default()))
                })),
        )
        .child(when(desktop_platform, move || {
            muted(move || {
                if hovering.get() {
                    "Release to add".to_owned()
                } else {
                    "Or drop files and folders here.".to_owned()
                }
            })
        }))
}

// ---------------------------------------------------------------- devices

/// Sends the selection to `target`, or asks for files first.
pub fn send_to(target: Target) {
    let paths = selection().with_untracked(|s| s.paths.clone());
    if paths.is_empty() {
        toast(NoticeKind::Info, "Choose what to send first.");
        return;
    }
    bridge::send(Command::SendFiles { target, paths });
    selection().set(Selection::default());
    picked().set(Default::default());
    crate::state::set_tab(crate::state::Tab::Transfers);
}

fn devices() -> impl IntoElement {
    let peers = move || model().with(|m| m.peers.clone());
    let empty = move || model().with(|m| m.peers.is_empty());
    view()
        .gap(px(space::M))
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .child(section_label("Devices nearby"))
                .child(when(desktop_platform, || {
                    btn("Refresh", Variant::Ghost)
                        .on_press(|| bridge::send(Command::RefreshDiscovery))
                })),
        )
        .child(when(empty, empty_devices).otherwise(move || {
            view().gap(px(space::S)).child(each(
                peers,
                |p: &PeerInfo| p.peer_id,
                |peer, _| device_row(peer),
            ))
        }))
}

fn device_row(peer: ReadSignal<PeerInfo>) -> impl IntoElement {
    let name = move || peer.with(|p| p.name.clone());
    let detail = move || {
        peer.with(|p| {
            let mut parts = vec![
                platform_label(&p.platform).to_owned(),
                p.fingerprint.clone(),
            ];
            if p.trusted {
                parts.push("Trusted".into());
            }
            parts.join(" · ")
        })
    };
    let ready = move || selection().with(|s| !s.paths.is_empty());
    card()
        .flex_row()
        .items_center()
        .gap(px(space::M))
        .p(px(space::M))
        .hover(|s| s.border_color(move || theme().ring))
        .child(ui::avatar(name, 40.0))
        .child(
            view()
                .flex_1()
                .gap(px(2.0))
                .child(
                    ui::body(name)
                        .font_weight(FontWeight::SEMIBOLD)
                        .number_of_lines(1),
                )
                .child(muted(detail).number_of_lines(1)),
        )
        .child(
            btn("Send", Variant::Primary)
                .disabled(move || !ready())
                .accessibility_label(format!(
                    "Send to {}",
                    peer.with_untracked(|p| p.name.clone())
                ))
                .on_press(move || send_to(Target::Peer(peer.with_untracked(|p| p.peer_id)))),
        )
}

fn empty_devices() -> impl IntoElement {
    let (title, detail) = (
        move || {
            model().with(|m| match m.status.as_ref().map(|s| &s.discovery) {
                _ if m.offline.is_some() => "Not available here".to_owned(),
                Some(DiscoveryStatus::Unavailable(_)) => "Can't look for devices".to_owned(),
                _ => "Looking for devices…".to_owned(),
            })
        },
        move || {
            model().with(|m| match (&m.offline, m.status.as_ref().map(|s| &s.discovery)) {
                (Some(why), _) => why.clone(),
                (_, Some(DiscoveryStatus::Unavailable(why))) => format!(
                    "This network or a permission blocks finding devices ({why}). Use a code or a QR invitation below."
                ),
                _ => "Open DexDrop on the other device and keep both on the same Wi‑Fi or network. \
                      Guest networks and VPNs can hide devices — use a code below if yours doesn't appear."
                    .to_owned(),
            })
        },
    );
    card()
        .items_center()
        .py(px(space::XL))
        .child(ui::body(title).font_weight(FontWeight::SEMIBOLD))
        .child(muted(detail).text_center().max_w(px(460.0)))
}

// ---------------------------------------------------------------- fallback

fn fallback() -> impl IntoElement {
    card()
        .child(section_label("Device not listed?"))
        .child(muted(
            "Show a code or QR on the receiving device, then enter it here. Codes expire after 10 minutes and work once.",
        ))
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap(px(space::S))
                .child(btn("Receive with a code", Variant::Secondary).on_press(|| {
                    bridge::send(Command::CreateCode);
                    bridge::send(Command::CreateInvitation);
                    sheet().set(Sheet::Receive);
                }))
                .child(btn("Send with a code", Variant::Secondary).on_press(|| sheet().set(Sheet::Connect))),
        )
}

/// The receive sheet: this device's code and QR invitation (FR-020).
pub fn receive_sheet() -> impl IntoElement {
    let code = move || {
        model().with(|m| {
            m.code
                .as_ref()
                .map(|(c, _)| dexdrop_core::invitation::display_code(c))
                .unwrap_or_else(|| "········".into())
        })
    };
    let invitation = move || model().with(|m| m.invitation.clone());
    view()
        .gap(px(space::L))
        .items_center()
        .child(
            ui::body("On the sending device, choose “Send with a code” and enter:").text_center(),
        )
        .child(
            text(code)
                .text_size(px(34.0))
                .font_weight(FontWeight::BOLD)
                .letter_spacing(px(2.0))
                .text_color(move || theme().foreground)
                .selectable(true)
                .label("Pairing code"),
        )
        // A code works once: after it is used, offer a fresh one.
        .child(when(
            move || model().with(|m| m.code.is_none()),
            || {
                btn("Show a new code", Variant::Secondary)
                    .on_press(|| bridge::send(Command::CreateCode))
            },
        ))
        .child(dynamic(move || {
            invitation().map(|inv| {
                view()
                    .items_center()
                    .gap(px(space::S))
                    .child(ui::qr(inv.qr_width, inv.qr_modules.clone(), 196.0))
                    .child(
                        muted("Or scan this QR code, or copy the invitation link.").text_center(),
                    )
                    .child(btn("Copy invitation", Variant::Ghost).on_press(move || {
                        set_clipboard(inv.text.clone());
                        toast(NoticeKind::Success, "Invitation copied.");
                    }))
            })
        }))
        .child(muted("You'll still be asked before anything is saved.").text_center())
        .child(btn("Done", Variant::Primary).self_stretch().on_press(|| {
            bridge::send(Command::CloseInvitations);
            sheet().set(Sheet::None);
        }))
}

/// Reads a QR invitation from a screenshot or photo into `entry` (FR-025):
/// for phones without a camera scanner yet, and desktops shown a QR.
fn read_qr_picture(entry: Signal<String>) {
    let decode = |bytes: Vec<u8>| {
        bridge::off_thread(move || dexdrop_core::invitation::read_qr_image(&bytes))
    };
    if desktop_platform() {
        let picked = pick_paths(
            OpenDialog::files()
                .filter(nsh_core::file_dialog::FileFilter::new(
                    "Pictures",
                    ["png", "jpg", "jpeg", "webp"],
                ))
                .button_label("Read"),
        );
        spawn(async move {
            let Ok(Some(paths)) = picked.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let read = bridge::off_thread(move || std::fs::read(path)).await;
            let Some(Ok(bytes)) = read else {
                toast(NoticeKind::Error, "Could not open the picture.");
                return;
            };
            apply_qr(entry, decode(bytes).await);
        });
    } else {
        let picked = pick_images(nsh_core::picker::PickRequest::new(false, 1));
        spawn(async move {
            let Some(file) = picked.await.into_iter().next() else {
                return;
            };
            let bytes = file.data.bytes().to_vec();
            apply_qr(entry, decode(bytes).await);
        });
    }
}

fn apply_qr(
    entry: Signal<String>,
    read: Option<Result<String, dexdrop_core::invitation::InviteError>>,
) {
    match read {
        Some(Ok(text)) => {
            entry.set(text);
            toast(NoticeKind::Success, "Invitation read from the picture.");
        }
        Some(Err(err)) => toast(NoticeKind::Error, err.to_string()),
        None => {}
    }
}

/// The connect sheet: send with a typed code or a pasted invitation (FR-021).
pub fn connect_sheet() -> impl IntoElement {
    let entry = signal(String::new());
    let submit = move || {
        let value = entry.get_untracked();
        let value = value.trim();
        if value.is_empty() {
            return;
        }
        let target = if value
            .to_ascii_uppercase()
            .starts_with(dexdrop_core::invitation::PREFIX)
        {
            Target::Invitation(value.to_owned())
        } else if dexdrop_core::invitation::normalize_code(value).is_some() {
            Target::Code(value.to_owned())
        } else {
            toast(
                NoticeKind::Error,
                "Enter an 8-character code like K7P2-QX9M, or paste an invitation.",
            );
            return;
        };
        sheet().set(Sheet::None);
        send_to(target);
    };
    let has_files = move || selection().with(|s| !s.paths.is_empty());
    view()
        .gap(px(space::L))
        .child(ui::body(
            "Enter the code shown on the receiving device, or paste its invitation.",
        ))
        .child(
            nsh_base::input()
                .value(move || entry.get())
                .on_change(move |v: &str| entry.set(v.to_owned()))
                .on_submit(move |_: &str| submit())
                .placeholder("K7P2-QX9M or DEXDROP1:…")
                .auto_capitalize(AutoCapitalize::Characters)
                .min_h(px(ui::hit()))
                .px(px(space::M))
                .border_1()
                .border_color(move || theme().input)
                .rounded(move || theme().radius)
                .text_size(px(ui::size::BODY))
                .text_color(move || theme().foreground),
        )
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap(px(space::S))
                .child(btn("Paste", Variant::Secondary).on_press(move || {
                    let pasted = clipboard_string();
                    spawn(async move {
                        let text = pasted.await;
                        if text.trim().is_empty() {
                            toast(NoticeKind::Info, "The clipboard is empty.");
                        } else {
                            entry.set(text.trim().to_owned());
                        }
                    });
                }))
                .child(
                    btn("Read QR from picture", Variant::Secondary)
                        .on_press(move || read_qr_picture(entry)),
                ),
        )
        .child(when(
            move || !has_files(),
            || ui::badge("Choose what to send first", Tone::Warning),
        ))
        .child(
            view()
                .flex_row()
                .gap(px(space::S))
                .justify_end()
                .child(btn("Cancel", Variant::Ghost).on_press(|| sheet().set(Sheet::None)))
                .child(
                    btn("Send", Variant::Primary)
                        .disabled(move || !has_files() || entry.with(|e| e.trim().is_empty()))
                        .on_press(submit),
                ),
        )
}
