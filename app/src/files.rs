//! Files as the app shows them: a picture of each (the photo itself when it
//! is one on this device, else its kind), its name and size, and a tap to
//! open it.

use std::path::{Path, PathBuf};

use dexdrop_core::{api::NoticeKind, format, store::FileItem};
use nsh_base::theme;
use nsh_core::{ResizeMode, prelude::*, share::ShareRequest};

use crate::{
    state::{desktop_platform, toast},
    ui::{self, Icon, muted, space},
};

/// Photos larger than this show their kind instead (decoding is costly).
const MAX_PREVIEW_BYTES: u64 = 40 * 1024 * 1024;
/// At most this many files of a list show a preview.
pub const PREVIEWED: usize = 24;

/// Whether the platform's image decoder reads this file.
pub fn is_image(name: &str) -> bool {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" => true,
        "heic" | "heif" => matches!(
            os(),
            nsh_core::platform::Os::Ios | nsh_core::platform::Os::MacOs
        ),
        _ => false,
    }
}

/// Whether the app plays this file itself (the platform decides in the
/// end: a format it cannot decode says so in the player).
pub fn is_video(name: &str) -> bool {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "mp4" | "m4v" | "mov" | "webm" | "mkv" | "3gp" | "avi" | "mpg" | "mpeg" | "ts" | "wmv"
    )
}

/// A local file as a `file://` URL, for the platform's player: iOS, macOS
/// and Android read URLs, and paths have spaces and other characters a URL
/// must escape.
pub fn file_url(path: &Path) -> String {
    let mut s = path.to_string_lossy().replace('\\', "/");
    if !s.starts_with('/') {
        // C:/Videos → /C:/Videos
        s.insert(0, '/');
    }
    let mut url = String::from("file://");
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~:".contains(&b) {
            url.push(b as char);
        } else {
            url.push_str(&format!("%{b:02X}"));
        }
    }
    url
}

/// The local file a preview can be drawn from, if any.
pub fn preview_of(item: &FileItem) -> Option<PathBuf> {
    let local = item.local.as_ref()?;
    (is_image(item.name()) && item.size <= MAX_PREVIEW_BYTES && local.exists())
        .then(|| local.clone())
}

/// A square picture of a file: the photo itself, cropped, or its kind.
pub fn thumb(name: &str, preview: Option<&Path>, side: f32, radius: f32) -> View {
    let kind = ui::file_badge(name, side, radius);
    match preview {
        // The kind shows through until the photo is drawn, and stays if it
        // cannot be (a damaged file).
        Some(path) => view()
            .size(px(side))
            .rounded(px(radius))
            .overflow_hidden()
            .child(kind)
            .child(
                image(path.to_string_lossy().into_owned())
                    .absolute()
                    .top_0()
                    .left_0()
                    .size(px(side))
                    .object_fit(ResizeMode::Cover),
            ),
        None => kind,
    }
}

/// Opens a file: in its app on computers, through the share sheet on phones.
pub fn open(path: PathBuf) {
    // Videos play, and photos show, in the app.
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if is_video(&name) || is_image(&name) {
        crate::state::push(crate::state::Route::Viewer(path));
        return;
    }
    // An APK on Android: the system's installer, which asks first.
    let apk = path
        .to_string_lossy()
        .to_ascii_lowercase()
        .ends_with(".apk");
    if desktop_platform() || (apk && os() == nsh_core::platform::Os::Android) {
        let opened = open_path(path);
        spawn(async move {
            if let Err(err) = opened.await {
                toast(NoticeKind::Error, format!("Could not open it: {err}"));
            }
        });
    } else {
        let shared = share(ShareRequest::new().file(path));
        spawn(async move {
            let _ = shared.await;
        });
    }
}

/// One file in a list: its picture, name, folder and size; a tap opens it
/// when it is on this device.
pub fn row(item: FileItem, previewed: bool) -> impl IntoElement {
    let preview = previewed.then(|| preview_of(&item)).flatten();
    let local = item.local.clone().filter(|p| p.exists());
    let folder = item
        .path
        .rsplit_once('/')
        .map(|(dir, _)| dir.to_owned())
        .unwrap_or_default();
    let detail = if folder.is_empty() {
        format::bytes(item.size)
    } else {
        format!("{} · {folder}", format::bytes(item.size))
    };
    let name = item.name().to_owned();
    // An APK on this device shows the app it installs, once read.
    let card = {
        let local = local.clone();
        move || local.as_deref().and_then(app_card)
    };
    let content = view()
        .flex_row()
        .items_center()
        .gap(px(space::M))
        .px(px(space::L))
        .py(px(space::S))
        .min_h(px(ui::hit().max(56.0)))
        .child(dynamic({
            let card = card.clone();
            let name = name.clone();
            move || match card() {
                Some(app) => app_icon(app.icon.as_deref(), 44.0),
                None => thumb(&name, preview.as_deref(), 44.0, 10.0),
            }
        }))
        .child(dynamic(move || {
            let (title, detail) = match card() {
                Some(app) => (app.label.clone(), app_detail(&app, item.size)),
                None => (name.clone(), detail.clone()),
            };
            view()
                .flex_1()
                .gap(px(2.0))
                .child(ui::body(title).number_of_lines(1))
                .child(
                    muted(detail)
                        .text_size(px(ui::size::CAPTION))
                        .number_of_lines(1),
                )
        }));
    match local {
        Some(path) => ui::pressable()
            .accessibility_label(format!("Open {}", item.name()))
            .on_press(move || open(path.clone()))
            .active(move |s| s.bg(move || theme().muted))
            .child(content.child(ui::icon(Icon::ChevronRight, 16.0, move || {
                theme().muted_foreground
            })))
            .into_element(),
        None => content.into_element(),
    }
}

/// A list of files in a group, with the first [`PREVIEWED`] previewed and a
/// line for those not listed.
pub fn list(items: Vec<FileItem>, total: u64) -> impl IntoElement {
    let listed = items.len() as u64;
    ui::group()
        .children(items.into_iter().enumerate().map(|(i, item)| {
            view()
                .child(when(move || i > 0, || ui::separator(72.0)))
                .child(row(item, i < PREVIEWED))
        }))
        .child(when(
            move || total > listed,
            move || {
                muted(format!("and {} more", total - listed))
                    .px(px(space::L))
                    .py(px(space::M))
                    .text_size(px(ui::size::SMALL))
            },
        ))
}

/// The photos among `items`, as a row of large previews (at most `max`).
pub fn photo_strip(items: &[FileItem], max: usize) -> Option<View> {
    let photos: Vec<PathBuf> = items.iter().filter_map(preview_of).take(max).collect();
    if photos.is_empty() {
        return None;
    }
    Some(
        view()
            .flex_row()
            .flex_wrap()
            .gap(px(space::S))
            .justify_center()
            .children(photos.into_iter().map(|p| {
                let path = p.clone();
                ui::pressable()
                    .accessibility_label("Open photo")
                    .on_press(move || open(path.clone()))
                    .active(|s| s.opacity(0.8))
                    .child(thumb(&p.to_string_lossy(), Some(&p), 96.0, 14.0))
            })),
    )
}

// ---------------------------------------------------------------- apps

/// An app as the lists show it: what an APK installs, or an installed app.
#[derive(Clone, Debug, PartialEq)]
pub struct AppCard {
    pub label: String,
    pub package: String,
    pub version: String,
    pub size: u64,
    pub icon: Option<PathBuf>,
    /// Installed from a bundle, as several APKs.
    pub split: bool,
    pub system: bool,
    /// Unix milliseconds; 0 for a file.
    pub updated: u64,
}

#[cfg(not(target_arch = "wasm32"))]
impl From<dexdrop_net::apps::AppInfo> for AppCard {
    fn from(a: dexdrop_net::apps::AppInfo) -> Self {
        Self {
            split: a.split(),
            label: a.label,
            package: a.package,
            version: a.version,
            size: a.size,
            icon: a.icon,
            system: a.system,
            updated: a.updated,
        }
    }
}

/// What is known about APK files on this device (Android): their app's
/// name, version and icon, by path. Filled when apps are picked to send
/// and, for other APKs, read in the background on first sight.
#[derive(Clone, Default, PartialEq)]
pub struct AppCards(pub std::collections::HashMap<PathBuf, Option<AppCard>>);

pub fn app_cards() -> Signal<AppCards> {
    global::<AppCards>()
}

/// The app an APK file at `path` installs, once read; asks for it the
/// first time (Android only).
pub fn app_card(path: &Path) -> Option<AppCard> {
    if let Some(known) = app_cards().with(|c| c.0.get(path).cloned()) {
        return known;
    }
    #[cfg(not(target_arch = "wasm32"))]
    if dexdrop_net::apps::AVAILABLE
        && path
            .to_string_lossy()
            .to_ascii_lowercase()
            .ends_with(".apk")
    {
        app_cards().update(|c| {
            c.0.insert(path.to_owned(), None);
        });
        let file = path.to_owned();
        let icons = crate::bridge::state_dir("app-icons");
        let reading = blocking({
            let file = file.clone();
            move || dexdrop_net::apps::archive(&file, &icons)
        });
        Owner::detached().run(|| {
            spawn(async move {
                if let Ok(Some(info)) = reading.await {
                    app_cards().update(|c| {
                        c.0.insert(file, Some(info.into()));
                    });
                }
            });
        });
    }
    None
}

/// "App · 11.2 · 58 MB": what an app row says under its name.
pub fn app_detail(app: &AppCard, size: u64) -> String {
    let mut parts = vec!["App".to_owned()];
    if !app.version.is_empty() {
        parts.push(app.version.clone());
    }
    parts.push(format::bytes(if size > 0 { size } else { app.size }));
    if app.split {
        parts.push("Split APKs".into());
    }
    parts.join(" · ")
}

/// An app's icon, rounded as launchers draw them, or its APK badge.
pub fn app_icon(icon: Option<&Path>, side: f32) -> View {
    match icon {
        Some(path) => view()
            .size(px(side))
            .rounded(px(side * 0.24))
            .overflow_hidden()
            .child(
                image(path.to_string_lossy().into_owned())
                    .size(px(side))
                    .object_fit(ResizeMode::Contain),
            ),
        None => ui::file_badge("app.apk", side, side * 0.24),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{file_url, is_video};

    #[test]
    fn local_files_become_escaped_urls() {
        assert_eq!(
            file_url(Path::new(
                "/data/user/0/dev.dexdrop.app/files/My Trip #1.mp4"
            )),
            "file:///data/user/0/dev.dexdrop.app/files/My%20Trip%20%231.mp4"
        );
        assert_eq!(
            file_url(Path::new(r"C:\Videos\été.mov")),
            "file:///C:/Videos/%C3%A9t%C3%A9.mov"
        );
        assert!(is_video("clip.MOV"));
        assert!(!is_video("clip.gif"));
    }
}
