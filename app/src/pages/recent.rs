//! Send's "Recent" on computers: the newest files in Downloads, Pictures,
//! Movies (and Desktop and Documents on Windows and Linux), a tap away from
//! the selection, sent from their own files. Reading those folders asks
//! for no permission.
//!
//! Phones have none: their newest photos and videos take a broad photo
//! permission (Android's `READ_MEDIA_IMAGES`/`READ_MEDIA_VIDEO`, which
//! Google Play restricts to apps whose core use needs them; iOS's full
//! photo library access), and the system's pickers, which need none, take
//! their place (docs/ARCHITECTURE.md ADR-008).

use std::path::PathBuf;

use dexdrop_net::media::{self, Recent};
use nsh_base::theme;
use nsh_core::{platform::Lifecycle, prelude::*};

use crate::{
    pages::send,
    state::{desktop_platform, selection},
    ui::{self, Icon, muted, space},
};

/// What Recent shows, loaded off the UI thread.
#[derive(Clone, Default, PartialEq)]
pub struct State {
    pub loaded: bool,
    pub items: Vec<Recent>,
}

fn state() -> Signal<State> {
    global::<State>()
}

/// Whether this platform has a Recent section: the computers (the web has
/// its own module).
pub fn offered() -> bool {
    desktop_platform()
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
/// one that ends later cannot put back what was there before.
static RELOADS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Reads what to show, off the UI thread (a few folders).
pub fn reload() {
    if !offered() {
        return;
    }
    use std::sync::atomic::Ordering;
    let this = RELOADS.fetch_add(1, Ordering::Relaxed) + 1;
    let dirs = folders();
    let reading = blocking(move || media::newest_files(&dirs, 8));
    Owner::detached().run(|| {
        spawn(async move {
            if let Ok(items) = reading.await
                && RELOADS.load(Ordering::Relaxed) == this
            {
                state().set(State {
                    loaded: true,
                    items,
                });
            }
        });
    });
}

fn selected(path: &std::path::Path) -> bool {
    selection().with(|s| s.paths.iter().any(|p| p == path))
}

fn toggle(path: PathBuf) {
    if selection().with_untracked(|s| s.paths.contains(&path)) {
        send::remove(path);
    } else {
        send::select(vec![path]);
    }
}

/// The section: a heading, then one row a file.
pub fn section() -> impl IntoElement {
    if !offered() {
        return view().into_element();
    }
    reload();
    // Back in the app (files may have arrived meanwhile): read again.
    let away = std::cell::Cell::new(false);
    effect(move || {
        let now = lifecycle();
        if now != Lifecycle::Active {
            away.set(true);
        } else if away.replace(false) {
            reload();
        }
    });
    view()
        .gap(px(space::M))
        .test_id("recent")
        .child(ui::section("Recent files"))
        .child(dynamic(move || {
            let st = state().get();
            // Rows in the shape of what is coming, while it is read.
            if !st.loaded {
                return ui::skeleton_rows(3).into_element();
            }
            if st.items.is_empty() {
                return muted("Nothing new in Downloads, Pictures or Movies.").into_element();
            }
            rows(st.items).into_element()
        }))
        .into_element()
}

/// One row a file, its picture or kind, name, size and when.
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
