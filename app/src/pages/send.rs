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

/// What is being read while picked files are handed over and copied (on
/// phones the system hands over every byte first, which takes a while for
/// videos): shown instead of silence.
#[derive(Clone, Default, PartialEq)]
pub struct Loading(pub Option<String>);

pub fn loading() -> Signal<Loading> {
    global::<Loading>()
}

fn set_loading(what: Option<String>) {
    loading().set(Loading(what));
}

/// Runs `work` to its end whatever the screen does meanwhile: a task
/// started from a row (removing a file) or a page ends with them
/// otherwise, leaving the selection uncounted or the loading card up.
fn detached(work: impl std::future::Future<Output = ()> + 'static) {
    Owner::detached().run(|| {
        spawn(work);
    });
}

pub fn has_selection() -> bool {
    selection().with_untracked(|s| !s.paths.is_empty())
}

/// Sets the selection (added to what is there) and counts it.
pub fn select(paths: Vec<PathBuf>) {
    if paths.is_empty() {
        return;
    }
    selection().update(|s| {
        for p in paths {
            if !s.paths.contains(&p) {
                s.paths.push(p);
            }
        }
        s.summary = None;
    });
    recount();
}

/// Counts the selection and reads each new path's size, off the engine
/// thread (a selection of thousands is thousands of disk reads), then
/// updates it if it has not changed meanwhile.
fn recount() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let (all, known) = selection().with_untracked(|s| (s.paths.clone(), s.stats.clone()));
        let counting = blocking(move || {
            let stats: Vec<(PathBuf, (bool, u64))> = all
                .iter()
                .filter(|p| !known.contains_key(*p))
                .map(|p| {
                    let meta = std::fs::metadata(p).ok();
                    let stat = (
                        meta.as_ref().is_some_and(|m| m.is_dir()),
                        meta.map_or(0, |m| m.len()),
                    );
                    (p.clone(), stat)
                })
                .collect();
            let summary = dexdrop_core::manifest::Manifest::build(&all)
                .map(|m| (m.files, m.folders, m.total_bytes));
            (all, stats, summary)
        });
        detached(async move {
            let Ok((counted, stats, summary)) = counting.await else {
                return;
            };
            if selection().with_untracked(|s| s.paths != counted) {
                return; // Changed meanwhile: a newer count follows.
            }
            match summary {
                Ok(summary) => selection().update(|s| {
                    s.summary = Some(summary);
                    s.stats.extend(stats);
                }),
                Err(err) => {
                    toast(
                        NoticeKind::Error,
                        format!("Could not read the selection: {err}"),
                    );
                    selection().set(Selection::default());
                }
            }
        });
    }
}

/// Takes one path out: the rest stays as it is on screen (nothing is
/// built again) and is counted anew.
fn remove(path: PathBuf) {
    selection().update(|s| {
        s.paths.retain(|p| *p != path);
        s.stats.remove(&path);
        s.summary = None;
    });
    if selection().with_untracked(|s| s.paths.is_empty()) {
        selection().set(Selection::default());
    } else {
        recount();
    }
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
    Apks,
}

/// Android: APK files from the system's files.
pub fn choose_apks() {
    choose_staged(Kind::Apks);
}

/// Android: copies apps' APKs into staging under their names, then adds
/// them to the selection; a card says so meanwhile.
pub fn add_apps(apps: Vec<(crate::files::AppCard, Vec<PathBuf>)>) {
    if apps.is_empty() {
        return;
    }
    set_loading(Some(match apps.as_slice() {
        [(one, _)] => format!("Preparing {}…", one.label),
        many => format!("Preparing {} apps…", many.len()),
    }));
    #[cfg(not(target_arch = "wasm32"))]
    {
        let staging = bridge::state_dir("staging")
            .join(dexdrop_core::hex(&dexdrop_core::random_bytes::<6>()));
        let cards: Vec<crate::files::AppCard> = apps.iter().map(|(c, _)| c.clone()).collect();
        let copying = blocking(move || {
            apps.into_iter()
                .map(|(card, apks)| {
                    let info = dexdrop_net::apps::AppInfo {
                        package: card.package,
                        label: card.label,
                        version: card.version,
                        version_code: 0,
                        system: card.system,
                        size: card.size,
                        apks,
                        icon: card.icon,
                        installed: 0,
                        updated: card.updated,
                        min_sdk: 0,
                        target_sdk: 0,
                    };
                    dexdrop_net::apps::stage(&info, &staging)
                })
                .collect::<Result<Vec<_>, _>>()
        });
        detached(async move {
            let copied = copying.await;
            set_loading(None);
            match copied {
                Ok(Ok(paths)) => {
                    for (path, card) in paths.iter().zip(cards) {
                        crate::pages::apps::remember(path.clone(), card);
                    }
                    select(paths);
                }
                Ok(Err(err)) => toast(NoticeKind::Error, format!("Could not copy the app: {err}")),
                Err(_) => {}
            }
        });
    }
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
        set_loading(Some(match kind {
            Kind::Photos => "Getting your photos…".into(),
            Kind::Videos => "Getting your videos…".into(),
            Kind::Files => "Getting your files…".into(),
            Kind::Apks => "Getting your APK files…".into(),
        }));
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
            Kind::Apks => {
                let mut request = FilePickRequest::new(true, 0);
                request.filters = vec![FileFilter::new("Android apps", ["apk"])];
                pick_files(request)
            }
        };
        detached(async move {
            let picked = picked.await;
            // The system hands the app each file's bytes, at most 512 MB
            // of them (NSH's picker): a larger one comes with its name and
            // size only. Say so, rather than send it empty.
            let (files, too_large): (Vec<_>, Vec<_>) = picked
                .into_iter()
                .partition(|f| f.status != nsh_core::picker::PickedStatus::TooLarge);
            if !too_large.is_empty() {
                let names: Vec<String> = too_large
                    .iter()
                    .map(|f| format!("{} ({})", f.name, format::bytes(f.len)))
                    .collect();
                toast(
                    NoticeKind::Error,
                    format!(
                        "Too large to send from this phone yet (512 MB at most per file): {}",
                        names.join(", ")
                    ),
                );
            }
            if files.is_empty() {
                set_loading(None);
                return;
            }
            set_loading(Some(if files.len() == 1 {
                "Preparing 1 file…".to_owned()
            } else {
                format!("Preparing {} files…", files.len())
            }));
            let staging = bridge::state_dir("staging");
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
            let staged = staged.await;
            set_loading(None);
            match staged {
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
    let busy = move || loading().with(|l| l.0.clone());
    let dropping = signal(false);
    view()
        .flex_1()
        .on_file_drag_enter(move |_: &FileDragEvent| dropping.set(true))
        .on_file_drag_leave(move || dropping.set(false))
        .on_file_drop(move |event: &FileDropEvent| {
            dropping.set(false);
            select(file_paths(&event.uris));
        })
        .child(body(
            view()
                .gap(px(space::XL))
                .child(kinds())
                .child(when(move || busy().is_some(), move || loading_card(busy)))
                .child(when(has, selected))
                .child(when(
                    move || desktop_platform() && !has(),
                    move || drop_zone(dropping),
                )),
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
            // Three across on an iPhone; two by two on Android, with Apps,
            // down to 320 pt wide screens; two (or more) on wider windows.
            // Compact on phones, so what is selected shows below them.
            .min_w(px(match os() {
                nsh_core::platform::Os::Ios => 96.0,
                nsh_core::platform::Os::Android => 120.0,
                _ => 140.0,
            }))
            .items_center()
            .gap(px(if ui::mobile() { space::S } else { space::M }))
            .py(px(if ui::mobile() { space::L } else { space::XL }))
            .rounded(px(20.0))
            .bg(ui::surface)
            .active(|s| s.scale(0.97).opacity(0.85))
            .hover(move |s| s.bg(move || theme().muted))
            .child(ui::glyph(i, colors, if ui::mobile() { 44.0 } else { 52.0 }))
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
        if os() == nsh_core::platform::Os::Android {
            grid = grid.child(tile(Icon::Android, "Apps", (0x22c55e, 0x059669), || {
                push(Route::Apps)
            }));
        }
    }
    grid
}

/// Computers: a place to drop files and folders, lit while they hover.
fn drop_zone(dropping: Signal<bool>) -> impl IntoElement {
    view()
        .items_center()
        .justify_center()
        .gap(px(space::M))
        .py(px(space::XXL))
        .rounded(px(20.0))
        .border_2()
        .border_dashed()
        .border_color(move || {
            if dropping.get() {
                theme().primary
            } else {
                theme().border
            }
        })
        .bg(move || {
            if dropping.get() {
                theme().primary.opacity(0.08)
            } else {
                theme().card.opacity(0.0)
            }
        })
        .child(ui::soft_glyph(Icon::Plus, move || theme().primary, 56.0))
        .child(ui::strong("Drop files or folders here"))
        .child(muted("Or choose them above. Photos show as previews."))
}

/// Picked files being handed over: a spinner and what is happening.
fn loading_card(what: impl Fn() -> Option<String> + Copy + 'static) -> impl IntoElement {
    ui::tile()
        .flex_row()
        .items_center()
        .gap(px(space::M))
        .child(
            view()
                .size(px(44.0))
                .rounded(px(12.0))
                .items_center()
                .justify_center()
                .bg(move || theme().primary.opacity(0.12))
                .child(ui::spinner(22.0)),
        )
        .child(
            view()
                .flex_1()
                .gap(px(2.0))
                .child(ui::strong(move || what().unwrap_or_default()))
                .child(
                    muted("Large videos take a moment to copy.").text_size(px(ui::size::CAPTION)),
                ),
        )
}

/// A picked path as the selection shows it.
#[derive(Clone, PartialEq)]
struct Pick {
    path: PathBuf,
    name: String,
    folder: bool,
    size: u64,
}

impl Pick {
    /// What the selection knows of `path`: its size once read (0 until
    /// then); nothing here touches the disk.
    fn of(path: &PathBuf, stats: &std::collections::HashMap<PathBuf, (bool, u64)>) -> Self {
        let (folder, size) = stats.get(path).copied().unwrap_or((false, 0));
        Self {
            path: path.clone(),
            name: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            folder,
            size,
        }
    }

    /// Shown as a photo tile.
    fn photo(&self) -> bool {
        !self.folder && crate::files::is_image(&self.name) && self.size <= 40 * 1024 * 1024
    }
}

/// What is selected, each removable: photos as a grid of previews, the
/// rest as a list with their kind and size.
fn selected() -> impl IntoElement {
    let picks = computed(move || {
        selection().with(|s| {
            s.paths
                .iter()
                .map(|p| Pick::of(p, &s.stats))
                .collect::<Vec<_>>()
        })
    });
    // Photos as a grid of previews, the rest as a list; both virtualized
    // (NSH lists): only what is near the screen exists, so a selection of
    // thousands costs what one of thirty does.
    let photos = computed(move || {
        picks.with(|p| p.iter().filter(|p| p.photo()).cloned().collect::<Vec<_>>())
    });
    let others = computed(move || {
        picks.with(|p| p.iter().filter(|p| !p.photo()).cloned().collect::<Vec<_>>())
    });
    let counting = move || selection().with(|s| s.summary.is_none());
    // Four across on a phone, six on wider windows.
    let columns: u32 = if ui::mobile() { 4 } else { 6 };
    let inner = if ui::mobile() {
        window().width.0.min(ui::content_width()) - 2.0 * space::L
    } else {
        ui::content_width() - 2.0 * space::XL
    };
    let side = ((inner - (columns - 1) as f32 * space::S) / columns as f32).floor();
    let paths = |list: Memo<Vec<Pick>>| {
        move || list.with(|l| l.iter().map(|p| p.path.clone()).collect::<Vec<_>>())
    };
    view()
        .gap(px(space::M))
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .child(ui::section("Selected"))
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap(px(space::XS))
                        .child(when(counting, || ui::spinner(14.0)))
                        .child(muted(summary)),
                ),
        )
        .child(when(
            move || !photos.with(Vec::is_empty),
            move || {
                virtual_list(
                    move || photos.with(Vec::len),
                    move |at| {
                        dynamic(move || {
                            photos
                                .with(|p| p.get(at.index()).cloned())
                                .map(|p| photo_tile(p, side))
                        })
                    },
                )
                .keys(paths(photos))
                .columns(columns)
                .row_gap(px(space::S))
                .column_gap(px(space::S))
                .estimated_item_size(px(side))
                .viewport(ListViewport::Parent)
            },
        ))
        .child(when(
            move || !others.with(Vec::is_empty),
            move || {
                ui::group().child(
                    virtual_list(
                        move || others.with(Vec::len),
                        move |at| {
                            dynamic(move || {
                                others.with(|o| o.get(at.index()).cloned()).map(pick_row)
                            })
                        },
                    )
                    .keys(paths(others))
                    .separator(|| ui::separator(72.0))
                    .estimated_item_size(px(64.0))
                    .viewport(ListViewport::Parent),
                )
            },
        ))
}

/// A picked file or folder that is not a photo: its kind (or the app an
/// APK installs, or a play button for a video), name, size, and a button
/// to remove it.
fn pick_row(pick: Pick) -> View {
    let fallback = if pick.folder {
        "Folder".to_owned()
    } else {
        format::bytes(pick.size)
    };
    let (size, folder) = (pick.size, pick.folder);
    let name = pick.name.clone();
    let path = pick.path.clone();
    // An APK shows the app it installs, once read.
    let at = signal(pick.path.clone());
    let label = signal(pick.name.clone());
    let fallback = signal(fallback);
    let card = move || crate::files::app_card(&at.get());
    let glyph = dynamic(move || {
        if folder {
            ui::kind_glyph(Icon::Folder, 44.0)
        } else if let Some(app) = card() {
            crate::files::app_icon(app.icon.as_deref(), 44.0)
        } else if crate::files::is_video(&label.get()) {
            play_thumb(at.get(), label.get())
        } else {
            crate::files::thumb(&label.get(), None, 44.0, 10.0)
        }
    });
    ui::row(
        glyph,
        move || card().map(|a| a.label).unwrap_or_else(|| label.get()),
        move || {
            card()
                .map(|a| crate::files::app_detail(&a, size))
                .unwrap_or_else(|| fallback.get())
        },
        ui::round_btn(Icon::X, format!("Remove {name}"), Variant::Secondary, 28.0)
            .on_press(move || remove(path.clone())),
    )
}

/// A picked video: its kind with a play button, which plays it.
fn play_thumb(path: PathBuf, name: String) -> View {
    view().child(
        ui::pressable()
            .accessibility_label(format!("Play {name}"))
            .on_press(move || crate::files::open(path.clone()))
            .active(|s| s.opacity(0.8))
            .child(crate::files::thumb(&name, None, 44.0, 10.0))
            .child(
                view()
                    .absolute()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .child(
                        view()
                            .size(px(22.0))
                            .rounded(px(11.0))
                            .items_center()
                            .justify_center()
                            .bg(rgba(0x0000_0099))
                            .child(ui::icon(Icon::Play, 11.0, white())),
                    ),
            ),
    )
}

/// A picked photo: its preview, with a button to remove it.
fn photo_tile(pick: Pick, side: f32) -> impl IntoElement {
    let path = pick.path.clone();
    let shown = pick.path.clone();
    view()
        .size(px(side))
        .child(
            ui::pressable()
                .accessibility_label(format!("View {}", pick.name))
                .on_press(move || crate::files::open(shown.clone()))
                .active(|s| s.opacity(0.8))
                .child(crate::files::thumb(
                    &pick.name,
                    Some(&pick.path),
                    side,
                    14.0,
                )),
        )
        .child(
            ui::pressable()
                .accessibility_label(format!("Remove {}", pick.name))
                .on_press(move || remove(path.clone()))
                .absolute()
                .top(px(2.0))
                .right(px(2.0))
                .size(px(ui::hit().min(36.0)))
                .items_center()
                .justify_center()
                .child(
                    view()
                        .size(px(24.0))
                        .rounded(px(12.0))
                        .items_center()
                        .justify_center()
                        .bg(rgba(0x0000_0099))
                        .child(ui::icon(Icon::X, 12.0, white())),
                ),
        )
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
