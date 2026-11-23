//! Sending: what to send (photos, videos, files, folders), then who to send
//! it to (a nearby device on the radar, or a code / invitation).

use std::path::PathBuf;

use dexdrop_core::{
    api::{Command, NoticeKind, PeerInfo, Target},
    format,
};
use nsh_base::theme;
use nsh_core::{file_dialog::OpenDialog, prelude::*};

use crate::{
    bridge,
    pages::body,
    state::{
        Picked, Route, Selection, desktop_platform, model, picked, platform_label, push, reset_to,
        selection, toast,
    },
    ui::{self, Icon, Variant, muted, space},
};

// ---------------------------------------------------------------- selection

pub fn has_selection() -> bool {
    selection().with_untracked(|s| !s.paths.is_empty())
}

/// Sets the selection (added to what is there) and counts it.
pub fn select(paths: Vec<PathBuf>) {
    if paths.is_empty() {
        return;
    }
    let mut all = selection().with_untracked(|s| s.paths.clone());
    for p in paths {
        if !all.contains(&p) {
            all.push(p);
        }
    }
    selection().set(Selection {
        paths: all.clone(),
        summary: None,
    });
    #[cfg(not(target_arch = "wasm32"))]
    {
        let counting = blocking(move || {
            dexdrop_core::manifest::Manifest::build(&all)
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

fn remove(path: PathBuf) {
    let rest: Vec<PathBuf> =
        selection().with_untracked(|s| s.paths.iter().filter(|p| **p != path).cloned().collect());
    selection().set(Selection::default());
    select(rest);
}

/// Desktops: the system's open dialog.
pub fn choose(dialog: OpenDialog) {
    let picked = pick_paths(dialog.multiple(true).button_label("Choose"));
    spawn(async move {
        match picked.await {
            Ok(Some(paths)) => select(paths),
            Ok(None) => {}
            Err(_) => choose_staged(Kind::Files),
        }
    });
}

/// What the user asked to pick.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Photos,
    Videos,
    Files,
}

/// Phones give the bytes of picked files, not paths: they are copied to
/// app-private staging first (PRD 15, "OS file providers").
fn choose_staged(kind: Kind) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use nsh_core::{
            file_dialog::FileFilter,
            picker::{FilePickRequest, PickFormat, PickRequest},
        };
        let picked = match kind {
            Kind::Photos => {
                let mut request = PickRequest::new(true, 0);
                // Originals, as they are stored (HEIC stays HEIC).
                request.format = PickFormat::Current;
                pick_images(request)
            }
            Kind::Videos => {
                let mut request = FilePickRequest::new(true, 0);
                request.filters = vec![FileFilter::new(
                    "Videos",
                    ["mp4", "mov", "m4v", "mkv", "webm", "avi", "3gp"],
                )];
                pick_files(request)
            }
            Kind::Files => pick_files(FilePickRequest::new(true, 0)),
        };
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
    #[cfg(target_arch = "wasm32")]
    let _ = kind;
}

pub fn file_paths(uris: &[String]) -> Vec<PathBuf> {
    uris.iter()
        .filter_map(|uri| dexdrop_core::fsutil::file_uri_to_path(uri))
        .collect()
}

/// Sends the selection to `target` and shows its progress.
pub fn send_to(target: Target) {
    let paths = selection().with_untracked(|s| s.paths.clone());
    if paths.is_empty() {
        toast(NoticeKind::Info, "Choose what to send first.");
        return;
    }
    bridge::send(Command::SendFiles { target, paths });
    selection().set(Selection::default());
    picked().set(Picked(None));
    // The transfer's progress, with home behind it: the flow is over.
    reset_to(Route::Transfer(None));
}

fn summary() -> String {
    selection().with(|s| match (s.paths.len(), s.summary) {
        (0, _) => String::new(),
        (_, Some((files, folders, bytes))) => {
            format!(
                "{} · {}",
                format::item_count(files, folders),
                format::bytes(bytes)
            )
        }
        (n, None) => format!("{n} selected"),
    })
}

/// The device picked before the files, if it is still nearby.
fn picked_peer() -> Option<PeerInfo> {
    let id = picked().get().0?;
    model().with(|m| m.peers.iter().find(|p| p.peer_id == id).cloned())
}

// ---------------------------------------------------------------- send page

pub fn page() -> impl IntoElement {
    let has = move || selection().with(|s| !s.paths.is_empty());
    view()
        .flex_1()
        .child(body(
            view()
                .gap(px(space::XL))
                .child(kinds())
                .child(when(has, selected)),
        ))
        .child(when(has, next_bar))
}

/// The kinds of things to send, as big tiles.
fn kinds() -> impl IntoElement {
    let tile = |i: Icon, label: &'static str, colors: (u32, u32), pick: fn()| {
        ui::pressable()
            .accessibility_label(label)
            .on_press(pick)
            .flex_1()
            // Three across on a phone, two (or more) on wider windows.
            .min_w(px(if ui::mobile() { 96.0 } else { 140.0 }))
            .items_center()
            .gap(px(space::M))
            .py(px(space::XL))
            .rounded(px(20.0))
            .bg(ui::surface)
            .active(|s| s.scale(0.97).opacity(0.85))
            .hover(move |s| s.bg(move || theme().muted))
            .child(ui::glyph(i, colors, 52.0))
            .child(ui::strong(label))
    };
    let mut grid = view().flex_row().flex_wrap().gap(px(space::M));
    if desktop_platform() {
        grid = grid
            .child(tile(
                Icon::File,
                "Choose files",
                (0x3b82f6, 0x6366f1),
                || choose(OpenDialog::files()),
            ))
            .child(tile(
                Icon::Folder,
                "Choose folder",
                (0xf59e0b, 0xf97316),
                || choose(OpenDialog::folders()),
            ));
    } else {
        grid = grid
            .child(tile(Icon::Photo, "Photos", (0xec4899, 0xf43f5e), || {
                choose_staged(Kind::Photos)
            }))
            .child(tile(Icon::Video, "Videos", (0x8b5cf6, 0xd946ef), || {
                choose_staged(Kind::Videos)
            }))
            .child(tile(Icon::File, "Files", (0x3b82f6, 0x6366f1), || {
                choose_staged(Kind::Files)
            }));
    }
    grid
}

/// What is selected, each removable.
fn selected() -> impl IntoElement {
    let items = move || selection().with(|s| s.paths.clone());
    view()
        .gap(px(space::M))
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .child(ui::section("Selected"))
                .child(muted(summary)),
        )
        .child(ui::group().child(each(
            items,
            |p: &PathBuf| p.clone(),
            |p, i| {
                let path = p.get_untracked();
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let folder = path.is_dir();
                let glyph = if folder {
                    Icon::Folder
                } else {
                    Icon::for_file(&name)
                };
                view()
                    .child(when(move || i.get() > 0, || ui::separator(64.0)))
                    .child(ui::row(
                        ui::kind_glyph(glyph, 36.0),
                        name.clone(),
                        String::new,
                        ui::round_btn(Icon::X, format!("Remove {name}"), Variant::Secondary, 28.0)
                            .on_press(move || remove(path.clone())),
                    ))
            },
        )))
}

/// The action at the foot of the send page: straight to the device picked
/// first, else to the device list.
fn next_bar() -> impl IntoElement {
    view()
        .absolute()
        .left_0()
        .right_0()
        .bottom(px(if ui::mobile() { 96.0 } else { 0.0 }))
        .px(px(space::L))
        .pb(px(space::L))
        .items_center()
        .child(dynamic(move || {
            let peer = picked_peer();
            let label = match &peer {
                Some(p) => format!("Send to {}", p.name),
                None => "Choose device".to_owned(),
            };
            ui::icon_btn(Icon::Paperplane, label, Variant::Primary)
                .w_full()
                .max_w(px(420.0))
                .shadow_lg()
                .on_press(move || match &peer {
                    Some(p) => send_to(Target::Peer(p.peer_id)),
                    None => push(Route::Devices),
                })
        }))
}

// ---------------------------------------------------------------- devices

pub fn devices_page() -> impl IntoElement {
    let peers = move || model().with(|m| m.peers.clone());
    let me = move || {
        model().with(|m| {
            m.status
                .as_ref()
                .map(|s| s.device_name.clone())
                .unwrap_or_default()
        })
    };
    let my_platform = move || {
        model().with(|m| {
            m.status
                .as_ref()
                .map(|s| s.platform.clone())
                .unwrap_or_default()
        })
    };
    body(
        view()
            .gap(px(space::XL))
            .child(
                view()
                    .items_center()
                    .gap(px(space::M))
                    .child(ui::radar(
                        ui::SEND,
                        220.0,
                        ui::avatar(me, my_platform, ui::my_avatar, 72.0),
                    ))
                    .child(muted(move || {
                        let n = model().with(|m| m.peers.iter().filter(|p| p.nearby).count());
                        match n {
                            0 => "Looking for devices…".to_owned(),
                            1 => "1 device nearby".to_owned(),
                            n => format!("{n} devices nearby"),
                        }
                    })),
            )
            .child(when(
                move || model().with(|m| !m.peers.is_empty()),
                move || {
                    ui::group().child(each(
                        peers,
                        |p: &PeerInfo| p.peer_id,
                        |p, i| {
                            view()
                                .child(when(move || i.get() > 0, || ui::separator(76.0)))
                                .child(device_row(p))
                        },
                    ))
                },
            ))
            .child(ui::group().child(ui::nav_row(
                ui::glyph(Icon::Keypad, (0x64748b, 0x475569), 30.0),
                "Use a code",
                "",
                || push(Route::Code),
            ))),
    )
}

fn device_row(peer: ReadSignal<PeerInfo>) -> impl IntoElement {
    let name = move || peer.with(|p| p.name.clone());
    let platform = move || peer.with(|p| p.platform.clone());
    let detail = move || {
        peer.with(|p| {
            let mut parts = vec![platform_label(&p.platform).to_owned()];
            if !p.nearby {
                parts.push("Over the internet".into());
            }
            if p.trusted {
                parts.push("Trusted".into());
            }
            if let Some(rate) = p.last_rate {
                parts.push(format::rate(rate));
            }
            parts.join(" · ")
        })
    };
    ui::pressable()
        .accessibility_label(format!(
            "Send to {}",
            peer.with_untracked(|p| p.name.clone())
        ))
        .on_press(move || send_to(Target::Peer(peer.with_untracked(|p| p.peer_id))))
        .flex_row()
        .items_center()
        .gap(px(space::M))
        .px(px(space::L))
        .py(px(space::M))
        .active(move |s| s.bg(move || theme().muted))
        .hover(move |s| s.bg(move || theme().muted.opacity(0.6)))
        .child(ui::far_avatar(
            name,
            platform,
            move || peer.with(|p| p.avatar.clone()),
            48.0,
            move || !peer.with(|p| p.nearby),
        ))
        .child(
            view()
                .flex_1()
                .gap(px(2.0))
                .child(ui::strong(name).number_of_lines(1))
                .child(muted(detail).number_of_lines(1)),
        )
        .child(ui::soft_glyph(
            Icon::Paperplane,
            move || theme().primary,
            36.0,
        ))
}

// ---------------------------------------------------------------- code

/// Send with a typed code or a pasted invitation (FR-021).
pub fn code_page() -> impl IntoElement {
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
                "Enter the 8-character code, like K7P2-QX9M.",
            );
            return;
        };
        send_to(target);
    };
    body(
        view()
            .gap(px(space::XL))
            .items_center()
            .child(ui::glyph(Icon::Keypad, (0x64748b, 0x475569), 64.0))
            .child(muted("The code is on the other device's Receive screen.").text_center())
            .child(
                nsh_base::input()
                    .value(move || entry.get())
                    .on_change(move |v: &str| entry.set(v.to_owned()))
                    .on_submit(move |_: &str| submit())
                    .placeholder("K7P2-QX9M")
                    .label("Code or invitation")
                    .auto_capitalize(AutoCapitalize::Characters)
                    .self_stretch()
                    .min_h(px(64.0))
                    .px(px(space::L))
                    .rounded(px(18.0))
                    .bg(ui::surface)
                    .text_size(px(26.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .letter_spacing(px(2.0))
                    .text_center()
                    .text_color(move || theme().foreground),
            )
            .child(
                view()
                    .flex_row()
                    .gap(px(space::S))
                    .child(
                        ui::icon_btn(Icon::Copy, "Paste", Variant::Secondary).on_press(move || {
                            let pasted = clipboard_string();
                            spawn(async move {
                                let text = pasted.await;
                                if text.trim().is_empty() {
                                    toast(NoticeKind::Info, "The clipboard is empty.");
                                } else {
                                    entry.set(text.trim().to_owned());
                                }
                            });
                        }),
                    )
                    .child(
                        ui::icon_btn(Icon::Qr, "QR picture", Variant::Secondary)
                            .on_press(move || read_qr_picture(entry)),
                    ),
            )
            .child(
                ui::icon_btn(Icon::Paperplane, "Send", Variant::Primary)
                    .self_stretch()
                    .disabled(move || {
                        !selection().with(|s| !s.paths.is_empty())
                            || entry.with(|e| e.trim().is_empty())
                    })
                    .on_press(submit),
            ),
    )
}

/// Reads a QR invitation from a screenshot or photo (FR-025).
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
            toast(NoticeKind::Success, "Invitation read.");
        }
        Some(Err(err)) => toast(NoticeKind::Error, err.to_string()),
        None => {}
    }
}
