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

use crate::state::engine;
use crate::{
    bridge,
    pages::body,
    state::{
        Category, Picked, Route, Selection, desktop_platform, picked, platform_label, push,
        reset_to, selection, toast,
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

/// A second launch of the app (Windows and Linux, `nsh.json`
/// `singleInstance`) hands the running one its command line: the files on
/// it, resolved against where it was started, become the selection, as
/// files opened with the app do.
pub fn take_second_launches() {
    #[cfg(not(target_arch = "wasm32"))]
    detached(async {
        loop {
            let launch = next_second_instance().await;
            let cwd = PathBuf::from(&launch.cwd);
            let paths: Vec<PathBuf> = launch
                .args
                .iter()
                .filter(|a| !a.starts_with('-'))
                .map(|a| {
                    let p = PathBuf::from(a);
                    if p.is_absolute() { p } else { cwd.join(p) }
                })
                .filter(|p| p.exists())
                .collect();
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
                .map(|p| (p.clone(), crate::files::stat_of(p)))
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

/// Android: adds apps under their names (one APK: sent where it is; split
/// APKs: copied into staging), then to the selection; a card says so
/// meanwhile.
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
                Ok(Err(err)) => toast(NoticeKind::Error, format!("Could not add the app: {err}")),
                Err(_) => {}
            }
        });
    }
}

/// Phones: the device's media browsed in the app when it has a provider
/// (`pages::library`), else the system's picker.
fn browse_or(category: Category, kind: Kind) {
    if crate::pages::library::offered() {
        push(Route::Library(category));
    } else {
        choose_staged(kind);
    }
}

/// Phones' files: the system's document picker, the documents sent from
/// where they are, nothing copied (Android: `dexdrop_net::assets`; iPhone:
/// `dexdrop_photos`' `ios-doc`); elsewhere the picker that copies.
fn choose_documents() {
    #[cfg(target_os = "android")]
    if crate::pages::library::offered() {
        set_loading(Some("Getting your files…".into()));
        let picking = blocking(|| dexdrop_net::assets::pick_documents(&[]));
        detached(async move {
            let picked = picking.await;
            set_loading(None);
            match picked {
                Ok(Ok(items)) => select(
                    items
                        .iter()
                        .map(dexdrop_core::asset::MediaItem::path)
                        .collect(),
                ),
                Ok(Err(err)) => toast(
                    NoticeKind::Error,
                    format!("Could not open the files: {err}"),
                ),
                Err(_) => {}
            }
        });
        return;
    }
    // iPhone: the document picker in open mode, the files read where they
    // are (dexdrop-photos' `ios-doc`, nothing copied).
    #[cfg(target_os = "ios")]
    if dexdrop_core::asset::provider(dexdrop_photos::DOC_SCHEME).is_some() {
        set_loading(Some("Getting your files…".into()));
        let picking = blocking(dexdrop_photos::pick_documents);
        detached(async move {
            let picked = picking.await;
            set_loading(None);
            match picked {
                Ok(Ok(items)) => select(
                    items
                        .iter()
                        .map(dexdrop_core::asset::MediaItem::path)
                        .collect(),
                ),
                Ok(Err(err)) => toast(
                    NoticeKind::Error,
                    format!("Could not open the files: {err}"),
                ),
                Err(_) => {}
            }
        });
        return;
    }
    choose_staged(Kind::Files);
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
    engine::peers().with(|engine::Peers(peers)| peers.iter().find(|p| p.peer_id == id).cloned())
}

// ---------------------------------------------------------------- send page

pub fn page() -> impl IntoElement {
    let has = move || selection().with(|s| !s.paths.is_empty());
    let busy = move || loading().with(|l| l.0.clone());
    let dropping = signal(false);
    let phone = !desktop_platform();
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
                // Computers: the choices at the top, where the pointer
                // starts; phones keep them at the foot (`dock`).
                .child(when(move || !phone, kinds))
                .child(when(move || busy().is_some(), move || loading_card(busy)))
                .child(when(has, selected))
                .child(when(
                    move || phone && !has() && busy().is_none(),
                    empty_hero,
                ))
                .child(when(move || !phone && !has(), move || drop_zone(dropping)))
                // The newest files, a tap away (computers).
                .child(crate::pages::recent::section())
                // Room for what floats over the bottom.
                .child(dynamic(move || {
                    let room = if phone {
                        dock_height(has())
                    } else if has() {
                        72.0
                    } else {
                        0.0
                    };
                    view().h(px(room))
                })),
        ))
        .child(when(move || phone, dock))
        .child(when(move || !phone && has(), next_bar))
}

/// A thing Send offers to pick from: its mark, its name, what it holds,
/// and what a tap does.
#[derive(Clone, Copy)]
struct Source {
    icon: Icon,
    label: &'static str,
    detail: &'static str,
    hue: ui::Source,
    pick: fn(),
}

impl Source {
    /// The same id everywhere ("Choose files" on computers is "Files" on
    /// phones), for tests that run on every platform.
    fn test_id(&self) -> String {
        format!(
            "pick-{}",
            self.label
                .trim_start_matches("Choose ")
                .to_ascii_lowercase()
        )
    }
}

/// What this platform picks from: files and folders on computers; photos,
/// videos and files on phones, and apps on Android.
fn sources() -> Vec<Source> {
    if desktop_platform() {
        return vec![
            Source {
                icon: Icon::File,
                label: "Choose files",
                detail: "Any kind, any size",
                hue: ui::Source::Files,
                pick: || choose(OpenDialog::files()),
            },
            Source {
                icon: Icon::Folder,
                label: "Choose folder",
                detail: "Everything in it, as it is",
                hue: ui::Source::Folder,
                pick: || choose(OpenDialog::folders()),
            },
        ];
    }
    let mut list = vec![
        Source {
            icon: Icon::Photo,
            label: "Photos",
            detail: "From your photo library",
            hue: ui::Source::Photos,
            pick: || browse_or(Category::Photos, Kind::Photos),
        },
        Source {
            icon: Icon::Video,
            label: "Videos",
            detail: "Any length, as they are",
            hue: ui::Source::Videos,
            pick: || browse_or(Category::Videos, Kind::Videos),
        },
        Source {
            icon: Icon::File,
            label: "Files",
            detail: "Documents and anything else",
            hue: ui::Source::Files,
            pick: choose_documents,
        },
    ];
    if os() == nsh_core::platform::Os::Android {
        list.push(Source {
            icon: Icon::Apps,
            label: "Apps",
            detail: "Installed apps and APK files",
            hue: ui::Source::Apps,
            pick: || push(Route::Apps),
        });
    }
    list
}

/// A source's row on phones: its badge, name and what it holds, and a
/// chevron; as tall as a thumb wants (Apple's HIG: 44 pt at least).
const SOURCE_ROW: f32 = 64.0;
/// The phones' strip of sources while something is chosen.
const SOURCE_STRIP: f32 = 72.0;
/// The main button's height, with the room around it.
const NEXT_BUTTON: f32 = 56.0;

/// How tall the phones' dock is: the room the page keeps under its content.
fn dock_height(has: bool) -> f32 {
    if has {
        space::S + SOURCE_STRIP + space::M + NEXT_BUTTON + space::L
    } else {
        space::S + sources().len() as f32 * SOURCE_ROW + space::L
    }
}

/// Phones: the choices and the way on, at the foot of the screen, where the
/// thumb is. Nothing chosen: every source as a row of a grouped list.
/// Something chosen: the sources as a compact strip to add more, and the
/// button on to a device.
fn dock() -> impl IntoElement {
    let has = move || selection().with(|s| !s.paths.is_empty());
    let mine = crate::state::tab_untracked();
    on_key("enter", move |_| {
        if !crate::state::showing(mine, |r| *r == Route::Send) || !has_selection() {
            return;
        }
        next();
    });
    view()
        .test_id("kinds")
        .absolute()
        .left_0()
        .right_0()
        .bottom(move || -> Length { px(ui::above_tab_bar()).into() })
        .px(px(space::L))
        .pt(px(space::S))
        .pb(px(space::L))
        .gap(px(space::M))
        .items_center()
        // The page's own color: what scrolls under it stays out of sight.
        .bg(move || theme().background)
        .child(when(move || !has(), source_list))
        .child(when(has, source_strip))
        .child(when(has, || {
            dynamic(move || {
                ui::icon_btn(Icon::Paperplane, next_label(), Variant::Primary)
                    .test_id("next")
                    .w_full()
                    .max_w(px(420.0))
                    .on_press(next)
            })
        }))
}

/// Straight to the device picked first, else to the device list.
fn next() {
    match picked_peer() {
        Some(p) => send_to(Target::Peer(p.peer_id)),
        None => push(Route::Devices),
    }
}

fn next_label() -> String {
    match picked_peer() {
        Some(p) => format!("Send to {}", p.name),
        None => "Choose device".to_owned(),
    }
}

/// Phones, nothing chosen yet: the sources as a grouped list, one row each.
fn source_list() -> impl IntoElement {
    let list = sources();
    let last = list.len().saturating_sub(1);
    ui::flat_group()
        .w_full()
        .max_w(px(560.0))
        .children(list.into_iter().enumerate().map(move |(i, s)| {
            view().child(source_row(s)).child(when(
                move || i < last,
                || ui::separator(space::L + 36.0 + space::M),
            ))
        }))
}

fn source_row(s: Source) -> impl IntoElement {
    ui::pressable()
        .accessibility_label(s.label)
        .test_id(&s.test_id())
        .on_press(s.pick)
        .flex_row()
        .items_center()
        .gap(px(space::M))
        .min_h(px(SOURCE_ROW))
        .px(px(space::L))
        .hover(move |st| st.bg(move || theme().muted.opacity(0.5)))
        .active(move |st| st.bg(move || theme().muted))
        .child(ui::source_badge(s.icon, s.hue, 36.0))
        .child(
            view()
                .flex_1()
                .gap(px(1.0))
                .child(
                    text(s.label)
                        .number_of_lines(1)
                        .text_size(px(ui::size::HEADING))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(
                    muted(s.detail)
                        .text_size(px(ui::size::SMALL))
                        .number_of_lines(1),
                ),
        )
        .child(ui::icon(Icon::ChevronRight, 18.0, move || {
            theme().muted_foreground
        }))
}

/// Phones, something chosen: the sources side by side, to add more.
fn source_strip() -> impl IntoElement {
    view()
        .w_full()
        .max_w(px(560.0))
        .flex_row()
        .gap(px(space::S))
        .children(sources().into_iter().map(|s| {
            ui::pressable()
                .accessibility_label(s.label)
                .test_id(&s.test_id())
                .on_press(s.pick)
                .flex_1()
                .h(px(SOURCE_STRIP))
                .items_center()
                .justify_center()
                .gap(px(6.0))
                .rounded(px(ui::radius::lg()))
                .bg(ui::surface)
                .hover(|st| st.opacity(0.92))
                .active(|st| st.scale(ui::motion::PRESS))
                .child(ui::source_badge(s.icon, s.hue, 30.0))
                .child(
                    text(s.label)
                        .number_of_lines(1)
                        .text_size(px(ui::size::CAPTION))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || theme().foreground),
                )
        }))
}

/// Phones, nothing chosen: what the page is for, above the sources.
fn empty_hero() -> impl IntoElement {
    let art = if short_screen() { 110.0 } else { 150.0 };
    // Centered in the room between the header and the choices: the window
    // less the header (about 100 pt), the choices and the tab bar, less
    // the art and two lines of text.
    let room = window().height.0 - 100.0 - dock_height(false) - ui::above_tab_bar();
    let top = ((room - (art + 90.0)) / 2.0).clamp(space::M, 160.0);
    view()
        .items_center()
        .gap(px(space::S))
        .pt(px(top))
        .child(ui::art(ui::Art::Devices, art))
        .child(
            text("Choose what to send")
                .text_size(px(ui::size::TITLE))
                .font_weight(FontWeight::BOLD)
                .text_center()
                .text_color(move || theme().foreground),
        )
        .child(
            muted("It goes straight to your other device, encrypted.")
                .text_center()
                .max_w(px(300.0)),
        )
}

/// Computers: the two choices side by side at the top, flat (no shadow),
/// each its badge, name and what it holds.
fn kinds() -> impl IntoElement {
    view()
        .test_id("kinds")
        .flex_row()
        .gap(px(space::M))
        .children(sources().into_iter().map(|s| {
            ui::pressable()
                .accessibility_label(s.label)
                .test_id(&s.test_id())
                .on_press(s.pick)
                .flex_1()
                .flex_row()
                .items_center()
                .gap(px(space::M))
                .p(px(space::L))
                .min_h(px(72.0))
                .rounded(px(ui::radius::lg()))
                .bg(ui::surface)
                .hover(move |st| st.bg(move || theme().muted))
                .hover(|st| st.opacity(0.92))
                .active(|st| st.scale(ui::motion::PRESS))
                .child(ui::source_badge(s.icon, s.hue, 40.0))
                .child(
                    view()
                        .flex_1()
                        .gap(px(2.0))
                        .child(
                            text(s.label)
                                .number_of_lines(1)
                                .text_size(px(ui::size::BODY))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || theme().foreground),
                        )
                        .child(
                            muted(s.detail)
                                .text_size(px(ui::size::SMALL))
                                .number_of_lines(1),
                        ),
                )
        }))
}

/// Computers: a place to drop files and folders, lit while they hover.
fn drop_zone(dropping: Signal<bool>) -> impl IntoElement {
    view()
        .items_center()
        .justify_center()
        .gap(px(space::M))
        .py(px(space::XXL))
        .rounded(px(ui::radius::xl()))
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
                .rounded(px(ui::radius::md()))
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
                    // Flat, as the choices under it (no shadow).
                    .child(ui::flat_group().child(each(
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
    let name = pick.name.clone();
    let path = pick.path.clone();
    // An APK shows the app it installs, once read.
    let at = signal(pick.path.clone());
    let label = signal(pick.name.clone());
    // Its kind and size as the selection reads them: the row is made at
    // once, before the size is known (it showed "0 B" beside a summary
    // that had it).
    let stat = move || {
        selection().with(|s| {
            at.with(|p| s.stats.get(p).copied())
                .unwrap_or((pick.folder, pick.size))
        })
    };
    let card = move || crate::files::app_card(&at.get());
    let glyph = dynamic(move || {
        if stat().0 {
            ui::kind_glyph(Icon::Folder, 44.0)
        } else if let Some(app) = card() {
            crate::files::app_icon(app.icon.as_deref(), 44.0)
        } else if crate::files::is_video(&label.get()) {
            play_thumb(at.get(), label.get())
        } else {
            crate::files::thumb(&label.get(), None, 44.0, ui::radius::md())
        }
    });
    ui::row(
        glyph,
        move || card().map(|a| a.label).unwrap_or_else(|| label.get()),
        move || {
            let (folder, size) = stat();
            match card() {
                Some(a) => crate::files::app_detail(&a, size),
                None if folder => "Folder".to_owned(),
                None => format::bytes(size),
            }
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
            .hover(|s| s.opacity(0.92))
            .active(|s| s.scale(ui::motion::PRESS))
            .child(crate::files::thumb(&name, None, 44.0, ui::radius::md()))
            .child(
                view()
                    .absolute()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .child(
                        view()
                            .size(px(22.0))
                            .rounded(px(ui::radius::md()))
                            .items_center()
                            .justify_center()
                            .bg(ui::media_chip)
                            .child(ui::icon(Icon::Play, 11.0, ui::on_media)),
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
                .hover(|s| s.opacity(0.92))
                .active(|s| s.scale(ui::motion::PRESS))
                .child(crate::files::thumb(
                    &pick.name,
                    crate::files::preview(&pick.path).as_deref(),
                    side,
                    ui::radius::lg(),
                )),
        )
        .child(
            ui::pressable()
                .accessibility_label(format!("Remove {}", pick.name))
                .on_press(move || remove(path.clone()))
                .absolute()
                .top(px(2.0))
                .right(px(2.0))
                .size(px(ui::hit()))
                .items_center()
                .justify_center()
                .child(
                    view()
                        .size(px(24.0))
                        .rounded(px(ui::radius::md()))
                        .items_center()
                        .justify_center()
                        .bg(ui::media_chip)
                        .child(ui::icon(Icon::X, 12.0, ui::on_media)),
                ),
        )
}

/// Computers: the action floating at the foot of the send page: straight
/// to the device picked first, else to the device list. Return does the
/// same.
fn next_bar() -> impl IntoElement {
    let mine = crate::state::tab_untracked();
    on_key("enter", move |_| {
        if !crate::state::showing(mine, |r| *r == Route::Send) || !has_selection() {
            return;
        }
        next();
    });
    view()
        .absolute()
        .left_0()
        .right_0()
        .bottom(move || -> Length { px(ui::above_tab_bar()).into() })
        .px(px(space::L))
        .pb(px(space::L))
        .items_center()
        .child(dynamic(move || {
            ui::icon_btn(Icon::Paperplane, next_label(), Variant::Primary)
                .test_id("next")
                .w_full()
                .max_w(px(420.0))
                .shadow_lg()
                .on_press(next)
        }))
}

// ---------------------------------------------------------------- devices

pub fn devices_page() -> impl IntoElement {
    // Everything sent from this page is one group: tap a device and it is
    // sent to at once while the list stays, tap another and it is sent to
    // as well (the engine reads the selection once for all of them).
    let group = new_group();
    let peers = move || engine::peers().with(|engine::Peers(peers)| peers.clone());
    let me = move || {
        engine::status().with(|engine::EngineStatus(status)| {
            status
                .as_ref()
                .map(|s| s.device_name.clone())
                .unwrap_or_default()
        })
    };
    let my_platform = move || {
        engine::status().with(|engine::EngineStatus(status)| {
            status
                .as_ref()
                .map(|s| s.platform.clone())
                .unwrap_or_default()
        })
    };
    // Nearby devices not sent to yet: "Send to all" sends to these.
    let unsent = move || {
        engine::peers().with(|engine::Peers(peers)| {
            engine::transfers().with(|engine::Transfers(transfers)| {
                peers
                    .iter()
                    .filter(|p| p.nearby)
                    .filter(|p| {
                        !transfers
                            .iter()
                            .any(|t| t.group == Some(group) && t.peer_id == Some(p.peer_id))
                    })
                    .map(|p| p.peer_id)
                    .collect::<Vec<_>>()
            })
        })
    };
    let sent_any = move || {
        engine::transfers()
            .with(|engine::Transfers(transfers)| transfers.iter().any(|t| t.group == Some(group)))
    };
    // The keyboard: Up and Down move a highlight through the devices,
    // Return sends to the highlighted one (else the first), A to all nearby.
    // The highlight is a device, not a place in the list: discovery
    // reorders the list, and Return must send where the user pointed.
    let focus = signal(None::<dexdrop_core::identity::PeerId>);
    let mine = crate::state::tab_untracked();
    let here = move || crate::state::showing(mine, |r| *r == Route::Devices);
    let ids = move || {
        engine::peers().with_untracked(|engine::Peers(peers)| {
            peers.iter().map(|p| p.peer_id).collect::<Vec<_>>()
        })
    };
    // The device `step` places from the highlighted one, held to the list.
    let moved = move |step: isize| {
        let ids = ids();
        if ids.is_empty() {
            return;
        }
        let at = focus
            .get_untracked()
            .and_then(|id| ids.iter().position(|p| *p == id));
        let next = match at {
            None => 0,
            Some(i) => (i as isize + step).clamp(0, ids.len() as isize - 1) as usize,
        };
        focus.set(Some(ids[next]));
    };
    let repeat = || KeyOptions {
        repeat: true,
        ..Default::default()
    };
    on_key_with("down", repeat(), move |_| {
        if here() {
            moved(1)
        }
    });
    on_key_with("up", repeat(), move |_| {
        if here() {
            moved(-1)
        }
    });
    on_key("enter", move |_| {
        if !here() {
            return;
        }
        let ids = ids();
        let target = focus
            .get_untracked()
            .filter(|id| ids.contains(id))
            .or_else(|| ids.first().copied());
        if let Some(id) = target {
            tap_device(group, id);
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
                            if short_screen() { 140.0 } else { 220.0 },
                            ui::avatar(
                                me,
                                my_platform,
                                crate::state::my_avatar,
                                if short_screen() { 56.0 } else { 72.0 },
                            ),
                        ))
                        .child(muted(move || {
                            let n = engine::peers().with(|engine::Peers(peers)| {
                                peers.iter().filter(|p| p.nearby).count()
                            });
                            match n {
                                0 => "Looking for devices…".to_owned(),
                                1 => "1 device nearby".to_owned(),
                                n => format!("{n} devices nearby · tap each to send"),
                            }
                        })),
                )
                .child(when(
                    move || engine::peers().with(|engine::Peers(peers)| !peers.is_empty()),
                    move || {
                        ui::group().child(each(
                            peers,
                            |p: &PeerInfo| p.peer_id,
                            move |p, i| {
                                view()
                                    .child(when(move || i.get() > 0, || ui::separator(76.0)))
                                    .child(device_row(p, group, move || {
                                        focus.get() == Some(p.with(|p| p.peer_id))
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
                                ui::glyph(Icon::Paperplane, 30.0),
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
                    ui::glyph(Icon::Keypad, 30.0),
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
    engine::peers().with_untracked(|engine::Peers(peers)| {
        targets.iter().any(|t| match t {
            Target::Peer(id) => peers.iter().any(|p| p.peer_id == *id && !p.nearby),
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
    engine::transfers().with(|engine::Transfers(transfers)| {
        transfers
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
        .bottom(move || -> Length { px(ui::above_tab_bar()).into() })
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
                    let ids: Vec<TransferId> =
                        engine::transfers().with_untracked(|engine::Transfers(transfers)| {
                            transfers
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
        .hover(move |s| s.bg(move || theme().muted.opacity(0.5)))
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
                    ui::progress(move || sent.get().map_or(0.0, |(_, _, p)| p), false)
                        .accessibility_label("Progress")
                })),
        )
        .child(dynamic(move || {
            if done() {
                ui::soft_glyph(Icon::Check, ui::Tone::Success, 36.0).into_element()
            } else {
                ui::soft_glyph(Icon::Paperplane, ui::Tone::Accent, 36.0).into_element()
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
            .child(ui::glyph(Icon::Keypad, 64.0))
            .child(
                muted("The code is on the other device's Receive screen.")
                    .text_center()
                    .w_full(),
            )
            .child(
                ui::field("Code or invitation", ui::FieldSize::Large)
                    .value(move || entry.get())
                    .on_change(move |v: &str| entry.set(v.to_owned()))
                    .on_submit(move |_: &str| submit())
                    .placeholder("K7P2-QX9M")
                    .auto_capitalize(AutoCapitalize::Characters)
                    .self_stretch(),
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
