//! Playing a video, or showing a photo, in the app: what was sent,
//! received or picked, with the platform's own playback controls
//! (AVPlayer, MediaPlayer, GStreamer, Media Foundation), its length,
//! size and resolution, and the ways to take it elsewhere.

use std::path::{Path, PathBuf};

use dexdrop_core::{api::NoticeKind, format};
use nsh_base::theme;
use nsh_core::{ResizeMode, prelude::*, share::ShareRequest};

use crate::{
    files,
    state::{desktop_platform, toast},
    ui::{self, Icon, Variant, muted, space},
};

/// What the player learned once the video loaded.
#[derive(Clone, Copy, PartialEq)]
struct Loaded {
    seconds: f32,
    width: i32,
    height: i32,
}

/// The page's title: the file's name.
pub fn title(path: &Path) -> SharedString {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Video".into())
        .into()
}

pub fn page(path: PathBuf) -> impl IntoElement {
    let name = title(&path).to_string();
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let is_video = files::is_video(&name);
    let loaded = signal(None::<Loaded>);
    let failed = signal(None::<String>);
    let stage = if is_video && !can_play_videos() {
        failed.set(Some(
            "Install GStreamer's plugins (gstreamer1.0-plugins-base, -good and -libav) to play videos in DexDrop."
                .into(),
        ));
        view().into_element()
    } else if is_video {
        player(&path, loaded, failed).into_element()
    } else {
        image(path.to_string_lossy().into_owned())
            .size_full()
            .object_fit(ResizeMode::Contain)
            .into_element()
    };
    let detail = move || {
        let mut parts = Vec::new();
        if let Some(l) = loaded.get() {
            if l.seconds > 0.0 {
                parts.push(length(l.seconds));
            }
            if l.width > 0 && l.height > 0 {
                parts.push(format!("{} × {}", l.width, l.height));
            }
        }
        parts.push(format::bytes(size));
        parts.join(" · ")
    };
    view()
        .flex_1()
        .bg(move || theme().background)
        .px(px(if ui::mobile() { 0.0 } else { space::XL }))
        .pt(px(space::S))
        // The tab bar floats over the bottom of the page (phones, narrow
        // windows).
        .pb(move || -> Length {
            px(ui::above_tab_bar() + if ui::mobile() { 8.0 } else { space::XL }).into()
        })
        .gap(px(space::L))
        .child(
            view()
                .flex_1()
                .min_h(px(220.0))
                .w_full()
                .rounded(px(if ui::mobile() { 0.0 } else { 18.0 }))
                .overflow_hidden()
                .bg(ui::media_backdrop)
                .child(stage)
                .child(when(
                    move || failed.get().is_some(),
                    move || cannot_play(failed),
                )),
        )
        .child(
            view()
                .w_full()
                .max_w(px(ui::content_width()))
                .self_center()
                .px(px(space::L))
                .flex_row()
                .items_center()
                .gap(px(space::M))
                .child(ui::file_badge(&name, 44.0))
                .child(
                    view()
                        .flex_1()
                        .gap(px(2.0))
                        .child(ui::strong(name.clone()).number_of_lines(1))
                        .child(muted(detail).text_size(px(ui::size::CAPTION))),
                )
                .child(actions(path)),
        )
}

/// Whether the platform can play videos at all. On Linux GTK plays them
/// with GStreamer, and without its `playbin` element GTK ends the app
/// instead of failing: look for the plugin first.
fn can_play_videos() -> bool {
    if os() != nsh_core::platform::Os::Linux {
        return true;
    }
    let mut dirs: Vec<PathBuf> = [
        "GST_PLUGIN_PATH",
        "GST_PLUGIN_PATH_1_0",
        "GST_PLUGIN_SYSTEM_PATH",
        "GST_PLUGIN_SYSTEM_PATH_1_0",
    ]
    .iter()
    .filter_map(std::env::var_os)
    .flat_map(|v| std::env::split_paths(&v).collect::<Vec<_>>())
    .collect();
    for lib in ["/usr/lib", "/usr/lib64", "/usr/local/lib"] {
        dirs.push(Path::new(lib).join("gstreamer-1.0"));
        for arch in [
            "x86_64-linux-gnu",
            "aarch64-linux-gnu",
            "arm-linux-gnueabihf",
            "i386-linux-gnu",
        ] {
            dirs.push(Path::new(lib).join(arch).join("gstreamer-1.0"));
        }
    }
    dirs.iter().any(|d| d.join("libgstplayback.so").is_file())
}

/// The video, playing at once, with the platform's controls.
fn player(path: &Path, loaded: Signal<Option<Loaded>>, failed: Signal<Option<String>>) -> Video {
    // Tabs stay mounted while another shows: the video pauses while its tab
    // is away and plays on when it comes back.
    let mine = crate::state::tab_untracked();
    video(files::file_url(path))
        .paused(move || crate::state::tab() != mine)
        .controls(true)
        .size_full()
        .object_fit(ResizeMode::Contain)
        .on_load(move |info: &VideoInfo| {
            loaded.set(Some(Loaded {
                seconds: info.duration,
                width: info.size.width.0,
                height: info.size.height.0,
            }))
        })
        .on_error(move |message: &str| failed.set(Some(message.to_owned())))
}

/// When the platform cannot play it: say so, and offer another app.
fn cannot_play(failed: Signal<Option<String>>) -> impl IntoElement {
    view()
        .absolute()
        .size_full()
        .items_center()
        .justify_center()
        .gap(px(space::M))
        .p(px(space::XL))
        .bg(move || ui::media_backdrop().opacity(0.9))
        .child(ui::icon(Icon::Video, 44.0, ui::on_media))
        .child(
            text("This video can't be played in DexDrop.")
                .text_color(ui::on_media)
                .font_weight(FontWeight::SEMIBOLD),
        )
        .child(
            text(move || failed.get().unwrap_or_default())
                .text_color(ui::on_media_muted)
                .text_size(px(ui::size::CAPTION))
                .text_center()
                .w_full()
                .number_of_lines(3),
        )
}

/// Computers: open it in its app, or show it in its folder. Phones: the
/// share sheet (save it to Photos or Files, or send it on).
fn actions(path: PathBuf) -> impl IntoElement {
    let row = view().flex_row().gap(px(space::S));
    if desktop_platform() {
        let shown = path.clone();
        row.child(
            ui::round_btn(Icon::Folder, "Show in folder", Variant::Secondary, 40.0).on_press(
                move || {
                    let shown = reveal_path(shown.clone());
                    spawn(async move {
                        let _ = shown.await;
                    });
                },
            ),
        )
        .child(
            ui::round_btn(Icon::Share, "Open in another app", Variant::Primary, 40.0).on_press(
                move || {
                    let opened = open_path(path.clone());
                    spawn(async move {
                        if let Err(err) = opened.await {
                            toast(NoticeKind::Error, format!("Could not open it: {err}"));
                        }
                    });
                },
            ),
        )
    } else {
        row.child(
            ui::round_btn(Icon::Share, "Share or save", Variant::Primary, 40.0).on_press(
                move || {
                    let shared = share(ShareRequest::new().file(path.clone()));
                    spawn(async move {
                        let _ = shared.await;
                    });
                },
            ),
        )
    }
}

/// "1:05", "12:40", "1:02:03".
fn length(seconds: f32) -> String {
    let s = seconds.round().max(0.0) as u64;
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::length;

    #[test]
    fn lengths_read_like_players_show_them() {
        assert_eq!(length(0.4), "0:00");
        assert_eq!(length(65.0), "1:05");
        assert_eq!(length(760.2), "12:40");
        assert_eq!(length(3723.0), "1:02:03");
    }
}
