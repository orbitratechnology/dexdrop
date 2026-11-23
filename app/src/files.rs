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

/// The local file a preview can be drawn from, if any.
pub fn preview_of(item: &FileItem) -> Option<PathBuf> {
    let local = item.local.as_ref()?;
    (is_image(item.name()) && item.size <= MAX_PREVIEW_BYTES && local.exists())
        .then(|| local.clone())
}

/// A square picture of a file: the photo itself, cropped, or its kind.
pub fn thumb(name: &str, preview: Option<&Path>, side: f32, radius: f32) -> View {
    let i = Icon::for_file(name);
    let c = ui::kind_color(i);
    let kind = view()
        .size(px(side))
        .rounded(px(radius))
        .items_center()
        .justify_center()
        .bg(rgba((c << 8) | 0x24))
        .child(ui::icon(i, side * 0.5, rgb(c)));
    match preview {
        Some(path) => view()
            .size(px(side))
            .rounded(px(radius))
            .overflow_hidden()
            .bg(move || theme().muted)
            .child(
                image(path.to_string_lossy().into_owned())
                    .size(px(side))
                    .object_fit(ResizeMode::Cover),
            ),
        None => kind,
    }
}

/// Opens a file: in its app on computers, through the share sheet on phones.
pub fn open(path: PathBuf) {
    if desktop_platform() {
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
    let content = view()
        .flex_row()
        .items_center()
        .gap(px(space::M))
        .px(px(space::L))
        .py(px(space::S))
        .min_h(px(ui::hit().max(56.0)))
        .child(thumb(item.name(), preview.as_deref(), 44.0, 10.0))
        .child(
            view()
                .flex_1()
                .gap(px(2.0))
                .child(ui::body(item.name().to_owned()).number_of_lines(1))
                .child(
                    muted(detail)
                        .text_size(px(ui::size::CAPTION))
                        .number_of_lines(1),
                ),
        );
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
