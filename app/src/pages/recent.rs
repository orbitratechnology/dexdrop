//! Send's "Recent": what was added to the device lately, a tap away from
//! the selection. On Android the newest photos and videos (the system's
//! media library, once the user allows it), sent from their own files; on
//! iOS those of the photo library (PhotoKit, `library.rs`), whose original
//! is exported when picked; on computers the newest files in Downloads,
//! Pictures, Movies (and Desktop and Documents on Windows and Linux).

use std::{collections::HashMap, path::PathBuf};

use dexdrop_net::media::{self, Access, Kind, Recent};
use nsh_base::theme;
use nsh_core::{platform::Lifecycle, prelude::*};

use crate::{
    pages::{library, send},
    state::selection,
    ui::{self, Icon, Variant, muted, space},
};

/// What Recent shows, loaded off the UI thread.
#[derive(Clone, Default, PartialEq)]
pub struct State {
    pub loaded: bool,
    pub access: Option<Access>,
    pub items: Vec<Recent>,
}

fn state() -> Signal<State> {
    global::<State>()
}

/// iOS: the files exported for library items picked ([`library`]), by the
/// item's path.
#[derive(Clone, Default, PartialEq)]
struct Exported(HashMap<PathBuf, PathBuf>);

/// Records that the library item `item` was exported to `file`.
pub fn remember_export(item: PathBuf, file: PathBuf) {
    global::<Exported>().update(|e| {
        e.0.insert(item, file);
    });
}

fn exported(item: &std::path::Path) -> Option<PathBuf> {
    global::<Exported>().with(|e| e.0.get(item).cloned())
}

/// Whether this platform has a Recent section (every one but the web,
/// which has its own module).
pub fn offered() -> bool {
    true
}

fn phone() -> bool {
    matches!(
        os(),
        nsh_core::platform::Os::Android | nsh_core::platform::Os::Ios
    )
}

fn ios() -> bool {
    os() == nsh_core::platform::Os::Ios
}

/// The folders whose newest files a computer shows (`DEXDROP_RECENT_DIRS`,
/// separated like `PATH`, in their place for tests).
fn folders() -> Vec<PathBuf> {
    if let Some(dirs) = std::env::var_os("DEXDROP_RECENT_DIRS") {
        return std::env::split_paths(&dirs).collect();
    }
    let Some(home) = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
    else {
        return Vec::new();
    };
    let names: &[&str] = match os() {
        // macOS asks before an app reads Desktop or Documents: left out.
        nsh_core::platform::Os::MacOs => &["Downloads", "Pictures", "Movies"],
        nsh_core::platform::Os::Windows | nsh_core::platform::Os::Linux => {
            &["Downloads", "Desktop", "Documents", "Pictures", "Videos"]
        }
        _ => &[],
    };
    names.iter().map(|n| home.join(n)).collect()
}

/// Which [`reload`] started last: only its answer is shown, so an older
/// one that ends later (read before the user allowed access, say) cannot
/// put back what was there before.
static RELOADS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Reads what to show, off the UI thread (a media library query and its
/// thumbnails, or a few folders).
pub fn reload() {
    if !offered() {
        return;
    }
    use std::sync::atomic::Ordering;
    let this = RELOADS.fetch_add(1, Ordering::Relaxed) + 1;
    let android = phone() && cfg!(target_os = "android");
    let ios = ios();
    let thumbs = crate::bridge::state_dir("recent-thumbs");
    let dirs = folders();
    let limit = if phone() { 24 } else { 8 };
    let reading = blocking(move || {
        if ios {
            let access = library::access();
            let items = if matches!(access, Access::All | Access::Some) {
                library::recent(limit, &thumbs)
            } else {
                Vec::new()
            };
            media::prune_thumbs(&thumbs, &items);
            (Some(access), items)
        } else if android {
            #[cfg(target_os = "android")]
            let access = crate::android::photo_access();
            #[cfg(not(target_os = "android"))]
            let access = Access::None;
            let items = if access == Access::None {
                Vec::new()
            } else {
                media::recent_media(limit, &thumbs)
            };
            (Some(access), items)
        } else {
            (None, media::newest_files(&dirs, limit))
        }
    });
    Owner::detached().run(|| {
        spawn(async move {
            if let Ok((access, items)) = reading.await
                && RELOADS.load(Ordering::Relaxed) == this
            {
                state().set(State {
                    loaded: true,
                    access,
                    items,
                });
            }
        });
    });
}

fn selected(path: &std::path::Path) -> bool {
    // A library item: its exported file.
    let path = if library::item_id(path).is_some() {
        match exported(path) {
            Some(file) => file,
            None => return false,
        }
    } else {
        path.to_path_buf()
    };
    selection().with(|s| s.paths.contains(&path))
}

fn toggle(path: PathBuf) {
    if let Some(id) = library::item_id(&path) {
        match exported(&path) {
            Some(file) if selection().with_untracked(|s| s.paths.contains(&file)) => {
                send::remove(file)
            }
            _ => send::add_from_library(path, id),
        }
        return;
    }
    if selection().with_untracked(|s| s.paths.contains(&path)) {
        send::remove(path);
    } else {
        send::select(vec![path]);
    }
}

/// The section: a heading, then the items (a grid of pictures on phones,
/// rows on computers), or the way to allow access.
pub fn section() -> impl IntoElement {
    if !offered() {
        return view().into_element();
    }
    reload();
    // Back from the system's permission dialog (or from another app that
    // added photos): read again.
    let away = std::cell::Cell::new(false);
    effect(move || {
        let now = lifecycle();
        if now != Lifecycle::Active {
            away.set(true);
        } else if away.replace(false) {
            reload();
        }
    });
    let s = move || state().get();
    view()
        .gap(px(space::M))
        .test_id("recent")
        .child(ui::section(if phone() {
            "Recent photos and videos"
        } else {
            "Recent files"
        }))
        .child(dynamic(move || {
            let st = s();
            if st.access == Some(Access::None) {
                return allow_card().into_element();
            }
            // Squares (phones) or rows (computers) in the shape of what
            // is coming, while it is read.
            if !st.loaded {
                return if phone() {
                    let side = cell_side();
                    view()
                        .flex_row()
                        .flex_wrap()
                        .gap(px(4.0))
                        .children((0..8).map(move |_| ui::skeleton(Some(side), side, 10.0)))
                        .into_element()
                } else {
                    ui::skeleton_rows(3).into_element()
                };
            }
            if st.items.is_empty() {
                return muted(if phone() {
                    "No photos or videos yet."
                } else {
                    "Nothing new in Downloads, Pictures or Movies."
                })
                .into_element();
            }
            let more = (st.access == Some(Access::Some)).then(|| {
                ui::icon_btn(Icon::Plus, "Choose more photos", Variant::Plain)
                    .on_press(ask)
                    .into_element()
            });
            let list = if phone() {
                grid(st.items).into_element()
            } else {
                rows(st.items).into_element()
            };
            view()
                .gap(px(space::S))
                .child(list)
                .children(more)
                .into_element()
        }))
        .into_element()
}

/// Asks for the photos and videos with the system's prompt (NSH's
/// `nsh::permissions`), then shows them; a user who refused before is
/// sent to the system's settings, the only place that changes it.
fn ask() {
    if ios() {
        ask_library();
    } else {
        ask_android();
    }
}

/// Android: NSH's `nsh::permissions`, asked from Rust.
fn ask_android() {
    // Detached, as `reload`: the card that asked goes once access changes,
    // and must not take the answer with it.
    #[cfg(target_os = "android")]
    Owner::detached().run(|| {
        spawn(async {
        use dexdrop_core::api::NoticeKind;
        if !crate::android::can_ask_photos() {
            crate::state::toast(
                NoticeKind::Info,
                "Allow photos and videos for DexDrop in the system's settings to see them here.",
            );
            drop(nsh_core::mobile::open_app_settings());
            return;
        }
        match crate::android::ask_photos().await {
            Ok(Access::None) => {}
            Ok(_) => reload(),
            Err(why) => crate::state::toast(
                NoticeKind::Error,
                format!("Could not ask for your photos: {why}"),
            ),
        }
        });
    });
}

/// iOS: the photo library's prompt the first time; after a refusal only
/// the system's settings allow it. With Limited Access, the system's sheet
/// to select more.
fn ask_library() {
    use dexdrop_core::api::NoticeKind;
    if state().with_untracked(|s| s.access == Some(Access::Some)) {
        library::choose_more();
        return;
    }
    if !library::can_ask() && library::access() == Access::None {
        crate::state::toast(
            NoticeKind::Info,
            "Allow Photos for DexDrop in Settings to see your photos and videos here.",
        );
        drop(nsh_core::mobile::open_app_settings());
        return;
    }
    let asking = blocking(library::ask);
    // Detached, as `reload`: the card that asked goes once access changes,
    // and must not take the answer with it.
    Owner::detached().run(|| {
        spawn(async move {
            if let Ok(access) = asking.await
                && access != Access::None
            {
                reload();
            }
        });
    });
}

/// Phones without access yet: what it is for, and the button.
fn allow_card() -> impl IntoElement {
    ui::tile()
        .items_center()
        .gap(px(space::M))
        .py(px(space::XL))
        .child(ui::art(ui::Art::Photos, 160.0))
        .child(
            muted("See your latest photos and videos here and send them in one tap.")
                .w_full()
                .text_center(),
        )
        .child(
            ui::icon_btn(Icon::Photo, "Show recent photos", Variant::Primary)
                .test_id("allow-recent")
                .on_press(ask),
        )
}

/// Phones: square pictures, four across; a tap adds or takes out.
fn grid(items: Vec<Recent>) -> impl IntoElement {
    view()
        .flex_row()
        .flex_wrap()
        .gap(px(4.0))
        .children(items.into_iter().map(cell))
}

fn cell(item: Recent) -> impl IntoElement {
    let path = item.path.clone();
    let on = {
        let path = item.path.clone();
        move || selected(&path)
    };
    let name = item
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let label = match item.kind {
        Kind::Video => format!("Video {name}"),
        _ => format!("Photo {name}"),
    };
    let on2 = on.clone();
    ui::pressable()
        .accessibility_label(label)
        .selected(on.clone())
        .on_press(move || toggle(path.clone()))
        // Four across, square, whatever the width.
        .size(px(cell_side()))
        .rounded(px(10.0))
        .overflow_hidden()
        .bg(move || theme().muted)
        .active(|s| s.opacity(0.8))
        .child(match &item.thumb {
            Some(t) => image(t.to_string_lossy().into_owned())
                .size_full()
                .object_fit(ResizeMode::Cover)
                .into_element(),
            None => view()
                .size_full()
                .items_center()
                .justify_center()
                .child(ui::kind_glyph(
                    if item.kind == Kind::Video {
                        Icon::Video
                    } else {
                        Icon::Photo
                    },
                    28.0,
                ))
                .into_element(),
        })
        .child(when(
            move || item.kind == Kind::Video,
            move || {
                view()
                    .absolute()
                    .right(px(4.0))
                    .bottom(px(4.0))
                    .px(px(6.0))
                    .py(px(2.0))
                    .rounded(px(999.0))
                    .bg(rgba(0x0000_00a6))
                    .child(
                        text(duration(item.duration_ms))
                            .text_size(px(11.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(white()),
                    )
            },
        ))
        .child(when(on2.clone(), || {
            view().absolute().size_full().bg(rgba(0x0000_0040)).child(
                view()
                    .absolute()
                    .top(px(6.0))
                    .right(px(6.0))
                    .size(px(24.0))
                    .rounded(px(12.0))
                    .items_center()
                    .justify_center()
                    .bg(move || theme().primary)
                    .shadow_md()
                    .child(ui::icon(Icon::Check, 14.0, move || {
                        theme().primary_foreground
                    })),
            )
        }))
}

/// The side of a picture: four across the page's width, 4 pt apart.
fn cell_side() -> f32 {
    let page = window().width.0.min(ui::content_width()) - 2.0 * space::L;
    ((page - 3.0 * 4.0) / 4.0).floor().max(56.0)
}

/// "1:05", "12:40", "1:02:03".
fn duration(ms: u64) -> String {
    let s = ms / 1000;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// Computers: one row a file, its picture or kind, name, size and when.
fn rows(items: Vec<Recent>) -> impl IntoElement {
    ui::group().children(items.into_iter().map(row))
}

fn row(item: Recent) -> impl IntoElement {
    let path = item.path.clone();
    let on = {
        let path = item.path.clone();
        move || selected(&path)
    };
    let name = item
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let detail = format!(
        "{} · {}",
        dexdrop_core::format::bytes(item.size),
        crate::pages::history::ago(item.added)
    );
    let on2 = on.clone();
    ui::pressable()
        .accessibility_label(format!("Add {name}"))
        .selected(on.clone())
        .on_press(move || toggle(path.clone()))
        .flex_row()
        .items_center()
        .gap(px(space::M))
        .min_h(px(ui::hit() + 12.0))
        .px(px(space::L))
        .py(px(space::S))
        .hover(move |s| s.bg(move || theme().muted.opacity(0.5)))
        .child(crate::files::thumb(
            &name,
            item.thumb.as_deref(),
            40.0,
            10.0,
        ))
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .child(ui::body(name).number_of_lines(1))
                .child(muted(detail)),
        )
        .child(
            view()
                .size(px(24.0))
                .rounded(px(12.0))
                .items_center()
                .justify_center()
                .bg(move || {
                    if on2() {
                        theme().primary
                    } else {
                        theme().muted
                    }
                })
                .child(when(on, || {
                    ui::icon(Icon::Check, 14.0, move || theme().primary_foreground)
                })),
        )
}

#[cfg(test)]
mod tests {
    #[test]
    fn durations_read_like_a_clock() {
        assert_eq!(super::duration(65_000), "1:05");
        assert_eq!(super::duration(760_000), "12:40");
        assert_eq!(super::duration(3_723_000), "1:02:03");
    }
}
