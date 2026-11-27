//! Settings: this device, receiving, trusted devices, privacy (FR-070 –
//! FR-075), in grouped rows with the platform's own switches.

use dexdrop_core::{
    api::{Command, NoticeKind},
    store::TrustedPeer,
};
use nsh_base::theme;
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

fn g(i: Icon) -> View {
    ui::glyph(i, 30.0)
}

pub fn page() -> impl IntoElement {
    body(
        view()
            .gap(px(space::XL))
            .child(device())
            .child(receiving())
            .child(internet())
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
        .child(
            ui::pressable()
                .accessibility_label("Change avatar")
                .test_id("change-avatar")
                .on_press(|| push(Route::Avatar))
                .active(|s| s.scale(ui::motion::PRESS_ICON))
                .child(ui::avatar(name, platform, crate::state::my_avatar, 80.0))
                .child(
                    view()
                        .absolute()
                        .top(px(-4.0))
                        .right(px(-4.0))
                        .child(ui::badge_disc(
                            28.0,
                            ui::icon(Icon::Pencil, 16.0, move || theme().foreground),
                        )),
                ),
        )
        .child(
            when(
                move || editing.get(),
                move || {
                    view()
                        .flex_row()
                        .items_center()
                        .gap(px(space::S))
                        .child(
                            ui::field("Device name", ui::FieldSize::Regular)
                                .value(move || draft.get())
                                .on_change(move |v: &str| draft.set(v.to_owned()))
                                .on_submit(move |_: &str| save())
                                .placeholder("Device name")
                                .w(px(220.0)),
                        )
                        .child(ui::icon_btn(Icon::Check, "Save", Variant::Primary).on_press(save))
                },
            )
            .otherwise(move || {
                ui::pressable()
                    .accessibility_label("Rename")
                    .test_id("rename")
                    .on_press(move || {
                        draft.set(settings().device_name);
                        editing.set(true);
                    })
                    .self_stretch()
                    .flex_row()
                    .items_center()
                    .gap(px(space::S))
                    .min_h(px(ui::hit()))
                    .hover(|s| s.opacity(0.92))
                    .active(|s| s.scale(ui::motion::PRESS))
                    // The name centered with the room of the row (not cut
                    // short where the system font is wider than measured),
                    // the pencil after it balanced by a space before.
                    .child(view().size(px(18.0)))
                    .child(ui::title(name).flex_1().text_center().number_of_lines(2))
                    .child(ui::icon(Icon::Pencil, 18.0, move || theme().primary))
            }),
        )
        .child(
            muted(move || {
                model().with(|m| {
                    m.status
                        .as_ref()
                        .map(|s| format!("{} · {}", platform_label(&s.platform), s.fingerprint))
                        .unwrap_or_default()
                })
            })
            .w_full()
            .text_center(),
        )
}

/// Where received files go now: the folder chosen in Settings, else the
/// platform's default (the system's Downloads).
fn save_dir() -> Option<std::path::PathBuf> {
    model()
        .with(|m| m.settings.download_dir.clone())
        .or_else(default_dir)
}

/// Where received files go, as the user knows the place: "~/Downloads",
/// "Downloads" on Android, "Files › On My iPhone › DexDrop" on iOS
/// (FR-041, FR-042).
pub fn save_place() -> String {
    let custom = model().with(|m| m.settings.download_dir.is_some());
    let dir = save_dir();
    match os().name() {
        "ios" if !custom => {
            let device = if idiom() == nsh_core::platform::Idiom::Pad {
                "iPad"
            } else {
                "iPhone"
            };
            format!("Files › On My {device} › DexDrop")
        }
        "android" if !custom => {
            if dir.as_deref() == Some(std::path::Path::new("/storage/emulated/0/Download")) {
                "Downloads".to_owned()
            } else {
                "DexDrop (app storage)".to_owned()
            }
        }
        _ => dir.map(|d| pretty_path(&d)).unwrap_or_default(),
    }
}

/// A folder as people read it: the home folder as "~" (macOS, Linux).
fn pretty_path(dir: &std::path::Path) -> String {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    match home.as_deref().and_then(|h| dir.strip_prefix(h).ok()) {
        Some(rest) if os().name() != "windows" => {
            if rest.as_os_str().is_empty() {
                "~".to_owned()
            } else {
                format!("~/{}", rest.display())
            }
        }
        _ => dir.display().to_string(),
    }
}

/// Where received files go, in a few words (FR-041, FR-073).
pub fn saves_to() -> String {
    format!("Saves to {}", save_place())
}

fn receiving() -> impl IntoElement {
    let custom = move || settings().download_dir.is_some();
    view()
        .gap(px(space::S))
        .child(ui::section("Receiving"))
        .child(
            ui::group()
                .child(ui::switch_row(
                    g(Icon::Eye),
                    "Visible to nearby devices",
                    move || settings().visible,
                    |on| bridge::send(Command::SetVisible(on)),
                ))
                .child(ui::separator(58.0))
                .child(ui::switch_row(
                    g(Icon::UserCheck),
                    "Auto-accept trusted devices",
                    move || settings().auto_accept_trusted,
                    |on| bridge::send(Command::SetAutoAcceptTrusted(on)),
                ))
                .child(ui::separator(58.0))
                // Where received files go: shown everywhere; chosen with
                // the system's folder dialog on computers (phones' pickers
                // choose files, not folders).
                .child(
                    when(desktop_platform, move || {
                        ui::nav_row(g(Icon::Folder), "Save to", save_place, choose_folder)
                            .test_id("save-to")
                    })
                    .otherwise(move || {
                        ui::row(g(Icon::Folder), "Save to", save_place, view()).test_id("save-to")
                    }),
                )
                .child(when(
                    move || desktop_platform() && custom(),
                    move || {
                        view().child(ui::separator(58.0)).child(
                            ui::nav_row(g(Icon::Refresh), "Use the Downloads folder", "", || {
                                bridge::send(Command::SetDownloadDir(None))
                            })
                            .test_id("save-to-downloads"),
                        )
                    },
                )),
        )
        .child(when(
            move || !desktop_platform(),
            || {
                muted(match os().name() {
                    "ios" => "Find them in the Files app, or share them from DexDrop.",
                    _ => "Find them in your phone's Downloads, or share them from DexDrop.",
                })
                .px(px(space::L))
                .text_size(px(ui::size::CAPTION))
            },
        ))
}

/// Computers: the system's folder dialog chooses where files go.
fn choose_folder() {
    let picked = pick_paths(OpenDialog::folders().button_label("Use this folder"));
    spawn(async move {
        if let Ok(Some(paths)) = picked.await
            && let Some(dir) = paths.into_iter().next()
        {
            bridge::send(Command::SetDownloadDir(Some(dir)));
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn default_dir() -> Option<std::path::PathBuf> {
    Some(bridge::default_download_dir())
}

#[cfg(target_arch = "wasm32")]
fn default_dir() -> Option<std::path::PathBuf> {
    None
}

/// Transfers over the internet (Phase 6): on by default; a relay is said
/// when one carries a transfer.
fn internet() -> impl IntoElement {
    view()
        .gap(px(space::S))
        .child(ui::section("Internet"))
        .child(ui::group().child(ui::switch_row(
            g(Icon::Globe),
            "Send over the internet",
            move || settings().internet,
            |on| {
                #[cfg(not(target_arch = "wasm32"))]
                bridge::set_internet(on);
                #[cfg(target_arch = "wasm32")]
                let _ = on;
            },
        )))
        .child(
            muted(
                "Devices you exchanged files with, and links you share, work from any network. \
                 Without a direct path, encrypted data passes through a relay that cannot read it.",
            )
            .px(px(space::L))
            .text_size(px(ui::size::CAPTION)),
        )
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
            g(Icon::Shield),
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
                    g(Icon::Clock),
                    "Keep transfer history",
                    move || settings().keep_history,
                    |on| bridge::send(Command::SetKeepHistory(on)),
                ))
                .child(ui::separator(58.0))
                .child(ui::nav_row(
                    g(Icon::Info),
                    "Export diagnostics",
                    "",
                    || bridge::send(Command::ExportDiagnostics),
                ))
                .child(ui::separator(58.0))
                .child(ui::nav_row(
                    g(Icon::Refresh),
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
                        "Files go between your devices encrypted end to end; no server stores them. \
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
    ui::confirm(
        "Reset this device's identity?",
        "Other devices will see it as a new device and ask before trusting it again. Your name, trusted devices and files stay.",
        "Reset identity",
        || {
            #[cfg(not(target_arch = "wasm32"))]
            bridge::reset_identity();
        },
    );
}

// ---------------------------------------------------------------- trusted

pub fn trusted_page() -> impl IntoElement {
    let empty = move || model().with(|m| m.trusted.is_empty());
    body(
        when(empty, || {
            ui::empty(
                ui::Art::Trusted,
                "Devices you trust when accepting a transfer show up here.",
            )
        })
        .otherwise(|| {
            view()
                .gap(px(space::S))
                .child(ui::group().child({
                    let trusted = computed(move || model().with(|m| m.trusted.clone()));
                    // Virtualized (NSH lists), in the page's scroll view.
                    virtual_list(
                        move || trusted.with(Vec::len),
                        move |at| {
                            dynamic(move || {
                                trusted
                                    .with(|t| t.get(at.index()).cloned())
                                    .map(|p| trusted_row(signal(p).read_only()))
                            })
                        },
                    )
                    .keys(move || trusted.with(|t| t.iter().map(|p| p.peer_id).collect::<Vec<_>>()))
                    .separator(|| ui::separator(16.0))
                    .estimated_item_size(px(120.0))
                    .viewport(ListViewport::Parent)
                }))
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
            ui::avatar(name, platform, move || p().avatar, 40.0),
            name,
            move || platform_label(&p().platform).to_owned(),
            ui::round_btn(
                Icon::Trash,
                format!("Forget {}", peer.with_untracked(|p| p.name.clone())),
                Variant::Destructive,
                34.0,
            )
            .on_press(move || {
                let peer = p();
                ui::confirm(
                    format!("Forget {}?", peer.name),
                    "It is no longer trusted: it asks before sending again.",
                    "Forget",
                    move || bridge::send(Command::ForgetPeer(peer.peer_id)),
                )
            }),
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
