//! Sending: what to send (photos, videos, files, folders), then who to send
//! it to (a nearby device on the radar, or a code / invitation).

use std::path::PathBuf;

use dexdrop_core::{
    api::{Command, NoticeKind, PeerInfo, Target, TransferSnapshot, TransferState},
    format,
    protocol::TransferId,
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

/// Files the system hands DexDrop to send (`nsh.json` `fileTypes`): shared
/// to it from another app on Android (the share sheet), opened with it or
/// dropped on its icon on computers, "Open in" on iOS. They join the
/// selection, and Share shows them on Send, ready to pick a device. Files
/// that came before this listens (those that launched the app) come at
/// once.
pub fn take_opened_files() {
    detached(async {
        loop {
            let paths = next_open_files().await;
            if paths.is_empty() {
                continue;
            }
            select(paths);
            crate::state::open(crate::state::Tab::Share, Route::Send);
        }
    });
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
pub fn remove(path: PathBuf) {
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
        use nsh_core::file_dialog::FileFilter;
        set_loading(Some(match kind {
            Kind::Photos => "Getting your photos…".into(),
            Kind::Videos => "Getting your videos…".into(),
            Kind::Files => "Getting your files…".into(),
            Kind::Apks => "Getting your APK files…".into(),
        }));
        // Videos, files and APKs: the system's document picker through
        // `pick_paths`, which copies each file to app storage without
        // holding it in memory and with no size limit, so a 5 GB video
        // can be sent. Photos keep the photo library's picker.
        if kind != Kind::Photos {
            let filter = match kind {
                Kind::Videos => Some(FileFilter::new(
                    "Videos",
                    ["mp4", "mov", "m4v", "mkv", "webm", "avi", "3gp"],
                )),
                Kind::Apks => Some(FileFilter::new("Android apps", ["apk"])),
                _ => None,
            };
            let mut dialog = OpenDialog::files().multiple(true).button_label("Choose");
            if let Some(filter) = filter {
                dialog = dialog.filter(filter);
            }
            let picked = pick_paths(dialog);
            detached(async move {
                match picked.await {
                    Ok(Some(paths)) => {
                        let staging = bridge::state_dir("staging");
                        let moved = blocking(move || adopt(paths, &staging)).await;
                        set_loading(None);
                        if let Ok(paths) = moved {
                            select(paths);
                        }
                    }
                    Ok(None) => set_loading(None),
                    // No path picker here: the byte picker, with its limit.
                    Err(_) => choose_bytes(kind),
                }
            });
            return;
        }
        choose_bytes(kind);
    }
    #[cfg(target_arch = "wasm32")]
    let _ = kind;
}

/// Moves picked copies into DexDrop's staging folder (the engine cleans it
/// up), where a rename is free; a copy that cannot move stays where the
/// picker put it.
#[cfg(not(target_arch = "wasm32"))]
fn adopt(paths: Vec<PathBuf>, staging: &std::path::Path) -> Vec<PathBuf> {
    let dir = staging.join(dexdrop_core::hex(&dexdrop_core::random_bytes::<6>()));
    if std::fs::create_dir_all(&dir).is_err() {
        return paths;
    }
    paths
        .into_iter()
        .map(|path| {
            let Some(name) = path.file_name() else {
                return path;
            };
            let to = dexdrop_core::naming::first_free(&dir, &name.to_string_lossy());
            if std::fs::rename(&path, &to).is_ok() {
                to
            } else {
                path
            }
        })
        .collect()
}

/// The pickers that hand over bytes (photos; others where paths cannot
/// be picked), copied into staging.
fn choose_bytes(kind: Kind) {
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
                ))
                // The newest photos, videos or files, a tap away.
                .child(crate::pages::recent::section())
                // Room for the bar floating over the bottom.
                .child(when(has, || view().h(px(72.0)))),
        ))
        .child(when(has, next_bar))
}

/// The kinds of things to send: squares in one row, on the page's own
/// color with a soft shadow (no fill, no border), each kind's icon in its
/// color over its name.
fn kinds() -> impl IntoElement {
    let android = os() == nsh_core::platform::Os::Android;
    let count = if desktop_platform() {
        2.0
    } else if android {
        4.0
    } else {
        3.0
    };
    // As wide as the row allows, square, at most 150 pt.
    let page = window().width.0.min(ui::content_width())
        - 2.0 * if ui::mobile() { space::L } else { space::XL };
    let side = ((page - (count - 1.0) * space::M) / count).clamp(64.0, 150.0);
    let small = side < 90.0;
    let tile = move |i: Icon, label: &'static str, color: u32, pick: fn()| {
        ui::pressable()
            .accessibility_label(label)
            // The same id everywhere ("Choose files" on computers is
            // "Files" on phones), for tests that run on every platform.
            .test_id(&format!(
                "pick-{}",
                label.trim_start_matches("Choose ").to_ascii_lowercase()
            ))
            .on_press(pick)
            .size(px(side))
            .items_center()
            .justify_center()
            .gap(px(if small { space::XS } else { space::S }))
            .rounded(px(if small { 18.0 } else { 22.0 }))
            .bg(move || theme().background)
            .shadow_md()
            .active(|s| s.scale(0.96).shadow_sm())
            .hover(|s| s.shadow_lg())
            .child(ui::soft_glyph(
                i,
                move || AdaptiveColor::from(rgb(color)),
                (side * 0.36).clamp(26.0, 48.0),
            ))
            .child(
                text(label)
                    .w_full()
                    .text_center()
                    .number_of_lines(1)
                    .text_size(px(if small {
                        ui::size::CAPTION + 1.0
                    } else {
                        ui::size::BODY
                    }))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(move || theme().foreground),
            )
    };
    let mut row = view()
        .test_id("kinds")
        .flex_row()
        .justify_center()
        .gap(px(space::M))
        // Room for the shadows.
        .py(px(space::XS));
    if desktop_platform() {
        row = row
            .child(tile(Icon::File, "Choose files", 0x3b82f6, || {
                choose(OpenDialog::files())
            }))
            .child(tile(Icon::Folder, "Choose folder", 0xf59e0b, || {
                choose(OpenDialog::folders())
            }));
    } else {
        row = row
            .child(tile(Icon::Photo, "Photos", 0xec4899, || {
                choose_staged(Kind::Photos)
            }))
            .child(tile(Icon::Video, "Videos", 0x8b5cf6, || {
                choose_staged(Kind::Videos)
            }))
            .child(tile(Icon::File, "Files", 0x3b82f6, || {
                choose_staged(Kind::Files)
            }));
        if android {
            row = row.child(tile(Icon::Android, "Apps", 0x22c55e, || push(Route::Apps)));
        }
    }
    row
}

/// Computers: a place to drop files and folders, lit while they hover.
fn drop_zone(dropping: Signal<bool>) -> impl IntoElement {
    view()
        .items_center()
        .justify_center()
        .gap(px(space::M))
        .py(px(space::XXL))
        .rounded(px(20.0))
        .bg(move || {
            if dropping.get() {
                theme().primary.opacity(0.08)
            } else {
                theme().muted.opacity(0.5)
            }
        })
        .style_when(move || dropping.get(), |s| s.shadow_lg())
        .child(ui::art(ui::Art::Drop, 150.0))
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
    // Photos as a grid of previews, the rest as a list.
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
    view()
        .gap(px(space::M))
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .child(ui::section("Selected").flex_1())
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap(px(space::XS))
                        .child(when(counting, || ui::spinner(14.0)))
                        .child(muted(summary)),
                ),
        )
        // Plain rows in the page's own scroll: a list virtualized against
        // the page (NSH's `ListViewport::Parent`) took the swipe on Android
        // and the page did not scroll. The first ones show; the rest are
        // counted (a selection of thousands stays light).
        .child(when(
            move || !photos.with(Vec::is_empty),
            move || {
                view()
                    .gap(px(space::S))
                    .child(view().flex_row().flex_wrap().gap(px(space::S)).child(each(
                        move || {
                            photos
                                .with(|p| p.iter().take(SHOWN_PHOTOS).cloned().collect::<Vec<_>>())
                        },
                        |p: &Pick| p.path.clone(),
                        move |p, _| photo_tile(p.get_untracked(), side),
                    )))
                    .child(more(move || photos.with(Vec::len), SHOWN_PHOTOS, "photo"))
            },
        ))
        .child(when(
            move || !others.with(Vec::is_empty),
            move || {
                view()
                    .gap(px(space::S))
                    .child(ui::group().child(each(
                        move || {
                            others.with(|o| o.iter().take(SHOWN_ROWS).cloned().collect::<Vec<_>>())
                        },
                        |p: &Pick| p.path.clone(),
                        |p, index| {
                            view()
                                .child(when(move || index.get() > 0, || ui::separator(72.0)))
                                .child(pick_row(p.get_untracked()))
                        },
                    )))
                    .child(more(move || others.with(Vec::len), SHOWN_ROWS, "file"))
            },
        ))
}

/// How many picked photos and other files show; the rest are counted.
const SHOWN_PHOTOS: usize = 60;
const SHOWN_ROWS: usize = 120;

/// "And 380 more files", under a list cut at `shown`.
fn more(
    total: impl Fn() -> usize + Copy + 'static,
    shown: usize,
    what: &'static str,
) -> impl IntoElement {
    when(
        move || total() > shown,
        move || {
            muted(move || {
                let n = total() - shown;
                format!("And {n} more {what}{}", if n == 1 { "" } else { "s" })
            })
            .w_full()
            .text_center()
        },
    )
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
/// first, else to the device list. Return does the same.
fn next_bar() -> impl IntoElement {
    let mine = crate::state::tab_untracked();
    on_key("enter", move |_| {
        if !crate::state::showing(mine, |r| *r == Route::Send) || !has_selection() {
            return;
        }
        match picked_peer() {
            Some(p) => send_to(Target::Peer(p.peer_id)),
            None => push(Route::Devices),
        }
    });
    view()
        .absolute()
        .left_0()
        .right_0()
        .bottom(px(ui::above_tab_bar()))
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
                .test_id("next")
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
    // Everything sent from this page is one group: tap a device and it is
    // sent to at once while the list stays, tap another and it is sent to
    // as well (the engine reads the selection once for all of them).
    let group = new_group();
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
    // Nearby devices not sent to yet: "Send to all" sends to these.
    let unsent = move || {
        model().with(|m| {
            m.peers
                .iter()
                .filter(|p| p.nearby)
                .filter(|p| {
                    !m.transfers
                        .iter()
                        .any(|t| t.group == Some(group) && t.peer_id == Some(p.peer_id))
                })
                .map(|p| p.peer_id)
                .collect::<Vec<_>>()
        })
    };
    let sent_any = move || model().with(|m| m.transfers.iter().any(|t| t.group == Some(group)));
    // The keyboard: Up and Down move a highlight through the devices,
    // Return sends to the highlighted one (else the first), A to all nearby.
    let focus = signal(None::<usize>);
    let mine = crate::state::tab_untracked();
    let here = move || crate::state::showing(mine, |r| *r == Route::Devices);
    let count = move || model().with_untracked(|m| m.peers.len());
    on_key_with(
        "down",
        KeyOptions {
            repeat: true,
            ..Default::default()
        },
        move |_| {
            if here() && count() > 0 {
                focus.set(Some(
                    focus
                        .get_untracked()
                        .map_or(0, |i| (i + 1).min(count() - 1)),
                ));
            }
        },
    );
    on_key_with(
        "up",
        KeyOptions {
            repeat: true,
            ..Default::default()
        },
        move |_| {
            if here() && count() > 0 {
                focus.set(Some(
                    focus.get_untracked().map_or(0, |i| i.saturating_sub(1)),
                ));
            }
        },
    );
    on_key("enter", move |_| {
        if !here() {
            return;
        }
        let i = focus.get_untracked().unwrap_or(0);
        if let Some(p) = model().with_untracked(|m| m.peers.get(i).cloned()) {
            tap_device(group, p.peer_id);
        }
    });
    on_key("a", move |_| {
        if here() {
            let targets: Vec<Target> = unsent().into_iter().map(Target::Peer).collect();
            send_group(group, targets);
        }
    });
    view()
        .flex_1()
        .child(body(
            view()
                .gap(px(space::XL))
                .child(
                    view()
                        .items_center()
                        .gap(px(space::M))
                        // Smaller on short screens: the devices are what this
                        // page is for, and they should not start under the
                        // tab bar.
                        .child(ui::radar(
                            ui::SEND,
                            if short_screen() { 140.0 } else { 220.0 },
                            ui::avatar(
                                me,
                                my_platform,
                                ui::my_avatar,
                                if short_screen() { 56.0 } else { 72.0 },
                            ),
                        ))
                        .child(muted(move || {
                            let n = model().with(|m| m.peers.iter().filter(|p| p.nearby).count());
                            match n {
                                0 => "Looking for devices…".to_owned(),
                                1 => "1 device nearby".to_owned(),
                                n => format!("{n} devices nearby · tap each to send"),
                            }
                        })),
                )
                .child(when(
                    move || model().with(|m| !m.peers.is_empty()),
                    move || {
                        ui::group().child(each(
                            peers,
                            |p: &PeerInfo| p.peer_id,
                            move |p, i| {
                                view()
                                    .child(when(move || i.get() > 0, || ui::separator(76.0)))
                                    .child(device_row(p, group, move || {
                                        focus.get() == Some(i.get())
                                    }))
                            },
                        ))
                    },
                ))
                // Below the devices, which stay high on small screens.
                .child(when(
                    move || unsent().len() >= 2,
                    move || {
                        ui::group()
                            .child(ui::nav_row(
                                ui::glyph(Icon::Paperplane, ui::SEND, 30.0),
                                move || format!("Send to all {} nearby", unsent().len()),
                                "",
                                move || {
                                    let targets = unsent().into_iter().map(Target::Peer).collect();
                                    send_group(group, targets);
                                },
                            ))
                            .test_id("send-to-all")
                    },
                ))
                .child(ui::group().child(ui::nav_row(
                    ui::glyph(Icon::Keypad, (0x64748b, 0x475569), 30.0),
                    "Use a code",
                    "",
                    || push(Route::Code),
                ))),
        ))
        .child(when(sent_any, move || progress_bar(group)))
}

fn short_screen() -> bool {
    nsh_core::element::window().height.0 < 720.0
}

/// An id for what is sent from one visit to the device list.
fn new_group() -> TransferId {
    #[cfg(not(target_arch = "wasm32"))]
    {
        dexdrop_core::random_id()
    }
    // The web preview sends nothing; any id unique to the page will do.
    #[cfg(target_arch = "wasm32")]
    {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let mut id = [0u8; 16];
        id[..8].copy_from_slice(&NEXT.fetch_add(1, Ordering::Relaxed).to_le_bytes());
        id
    }
}

/// Whether sending to `targets` would use a link that costs the user
/// (FR-072): one of them is reached over the internet while the system
/// calls the network expensive (mobile data, a phone's hotspot, a metered
/// Wi-Fi) or constrained (Low Data Mode, Data Saver). Nearby devices are
/// reached over the local network, which costs nothing. Platforms that
/// cannot tell (Windows, Linux) never ask.
fn costs_data(targets: &[Target]) -> bool {
    let net = network();
    if !(net.expensive || net.constrained) {
        return false;
    }
    model().with_untracked(|m| {
        targets.iter().any(|t| match t {
            Target::Peer(id) => m.peers.iter().any(|p| p.peer_id == *id && !p.nearby),
            _ => false,
        })
    })
}

/// Sends the selection to `targets` as part of `group`, staying on the
/// device list; over the internet on a metered link, once the user agrees.
fn send_group(group: TransferId, targets: Vec<Target>) {
    if selection().with_untracked(|s| s.paths.is_empty()) {
        toast(NoticeKind::Info, "Choose what to send first.");
        return;
    }
    if targets.is_empty() {
        return;
    }
    if costs_data(&targets) {
        let size = selection()
            .with_untracked(|s| s.summary)
            .map(|(_, _, bytes)| format!(" ({})", format::bytes(bytes)))
            .unwrap_or_default();
        let (title, link) = if network().expensive {
            (
                "Send over mobile data?",
                "You are on mobile data or a metered network",
            )
        } else {
            ("Send with Data Saver on?", "Data Saver is on")
        };
        ui::ask(
            title,
            format!(
                "{link}. This device is not nearby, so the files{size} go over the internet and may use your data plan."
            ),
            "Send",
            move || start_group(group, targets),
        );
        return;
    }
    start_group(group, targets);
}

fn start_group(group: TransferId, targets: Vec<Target>) {
    let paths = selection().with_untracked(|s| s.paths.clone());
    if paths.is_empty() {
        return;
    }
    crate::device::buzz(crate::device::Buzz::Tap);
    bridge::send(Command::Send {
        targets,
        paths,
        group: Some(group),
    });
}

/// The latest transfer of `group` to `peer`.
fn sent_to(group: TransferId, peer: dexdrop_core::identity::PeerId) -> Option<TransferSnapshot> {
    model().with(|m| {
        m.transfers
            .iter()
            .find(|t| t.group == Some(group) && t.peer_id == Some(peer))
            .cloned()
    })
}

/// Once something is sent: on to the progress (of the one transfer, or
/// all of them), the selection done with.
fn progress_bar(group: TransferId) -> impl IntoElement {
    view()
        .absolute()
        .left_0()
        .right_0()
        .bottom(px(ui::above_tab_bar()))
        .px(px(space::L))
        .pb(px(space::L))
        .items_center()
        .child(
            ui::icon_btn(Icon::Paperplane, "See progress", Variant::Primary)
                .test_id("see-progress")
                .w_full()
                .max_w(px(420.0))
                .shadow_lg()
                .on_press(move || {
                    let ids: Vec<TransferId> = model().with_untracked(|m| {
                        m.transfers
                            .iter()
                            .filter(|t| t.group == Some(group))
                            .map(|t| t.id)
                            .collect()
                    });
                    selection().set(Selection::default());
                    picked().set(Picked(None));
                    reset_to(Route::Transfer(match ids.as_slice() {
                        [one] => Some(*one),
                        _ => None,
                    }));
                }),
        )
}

/// A device tapped (or chosen with Return): sends to it, or opens its
/// transfer while that one is still going.
fn tap_device(group: TransferId, peer: dexdrop_core::identity::PeerId) {
    match sent_to(group, peer) {
        Some(t) if !t.state.is_finished() => push(Route::Transfer(Some(t.id))),
        _ => send_group(group, vec![Target::Peer(peer)]),
    }
}

fn device_row(
    peer: ReadSignal<PeerInfo>,
    group: TransferId,
    highlighted: impl Fn() -> bool + Copy + 'static,
) -> impl IntoElement {
    let name = move || peer.with(|p| p.name.clone());
    let platform = move || peer.with(|p| p.platform.clone());
    let id = move || peer.with_untracked(|p| p.peer_id);
    // What was sent to this device from this page, as it goes.
    let sent =
        computed(move || sent_to(group, id()).map(|t| (t.id, t.state.clone(), t.progress())));
    let detail = move || {
        if let Some((_, state, progress)) = sent.get() {
            return match state {
                TransferState::Transferring => format!("Sending · {}%", (progress * 100.0).floor()),
                TransferState::Waiting => "Waiting for them to accept".to_owned(),
                TransferState::Completed => "Sent".to_owned(),
                other => crate::pages::transfer::short_state(&other).to_owned(),
            };
        }
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
    let moving = move || {
        sent.get()
            .is_some_and(|(_, s, _)| !s.is_finished() && !matches!(s, TransferState::Paused { .. }))
    };
    let done = move || {
        sent.get()
            .is_some_and(|(_, s, _)| s == TransferState::Completed)
    };
    ui::pressable()
        .accessibility_label(move || {
            let name = peer.with(|p| p.name.clone());
            Some(SharedString::from(match sent.get() {
                Some((_, state, _)) => {
                    format!("{name}, {}", crate::pages::transfer::short_state(&state))
                }
                None => format!("Send to {name}"),
            }))
        })
        // Sent to already and still going: its progress, to pause or
        // cancel; once over, a tap sends again.
        .on_press(move || tap_device(group, id()))
        .bg(move || {
            if highlighted() {
                theme().muted
            } else {
                theme().muted.opacity(0.0)
            }
        })
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
                .gap(px(4.0))
                .child(ui::strong(name).number_of_lines(1))
                .child(muted(detail).number_of_lines(1))
                .child(when(moving, move || {
                    nsh_base::progress()
                        .value(move || {
                            sent.get()
                                .map(|(_, _, p)| (p * 100.0) as f32)
                                .unwrap_or(0.0)
                        })
                        .accessibility_label("Progress")
                        .h(px(4.0))
                })),
        )
        .child(dynamic(move || {
            if done() {
                ui::soft_glyph(Icon::Check, move || ui::tone_color(ui::Tone::Success), 36.0)
                    .into_element()
            } else {
                ui::soft_glyph(Icon::Paperplane, move || theme().primary, 36.0).into_element()
            }
        }))
}

// ---------------------------------------------------------------- code

/// Send with a typed code or a pasted invitation (FR-021).
pub fn code_page() -> impl IntoElement {
    let entry = signal(String::new());
    // Phones with a camera scan the other device's QR code (FR-024).
    let camera = signal(false);
    spawn(async move { camera.set(nsh_extra::camera::available().await) });
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
            .child(
                muted("The code is on the other device's Receive screen.")
                    .text_center()
                    .w_full(),
            )
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
                    .shadow_md()
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
                    .child(when(
                        move || camera.get(),
                        move || {
                            ui::icon_btn(Icon::Camera, "Scan QR", Variant::Secondary)
                                .test_id("scan-qr")
                                .on_press(move || scan_qr(entry))
                        },
                    ))
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

/// The invitation in a picture's QR code, read off the UI thread.
async fn decode_qr(
    bytes: Vec<u8>,
) -> Option<Result<String, dexdrop_core::invitation::InviteError>> {
    bridge::off_thread(move || dexdrop_core::invitation::read_qr_image(&bytes)).await
}

/// Reads the other device's QR code with the system's camera (FR-024,
/// `nsh-extra`'s `camera`): the user takes a photo of it, which is read as
/// a picture is (FR-025) and not kept. The photo comes scaled to 2048 px,
/// where QR modules stay several pixels wide.
fn scan_qr(entry: Signal<String>) {
    let mut request = nsh_extra::camera::PhotoRequest::default();
    request.max_dimension = Some(nsh_core::geometry::DevicePixels(2048));
    spawn(async move {
        match nsh_extra::camera::take_photo(request).await {
            Ok(Some(photo)) => apply_qr(entry, decode_qr(photo.data.bytes().to_vec()).await),
            Ok(None) => {}
            Err(err) if err.code.as_deref() == Some(nsh_extra::camera::DENIED) => toast(
                NoticeKind::Error,
                "DexDrop may not use the camera: allow it in the system's settings.",
            ),
            Err(err) => toast(
                NoticeKind::Error,
                format!("Could not take a photo: {}", err.message),
            ),
        }
    });
}

/// Reads a QR invitation from a screenshot or photo (FR-025).
fn read_qr_picture(entry: Signal<String>) {
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
            apply_qr(entry, decode_qr(bytes).await);
        });
    } else {
        let picked = pick_images(nsh_core::picker::PickRequest::new(false, 1));
        spawn(async move {
            let Some(file) = picked.await.into_iter().next() else {
                return;
            };
            let bytes = file.data.bytes().to_vec();
            apply_qr(entry, decode_qr(bytes).await);
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
