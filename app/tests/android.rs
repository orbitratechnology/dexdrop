//! Android's screens, headless: NSH's engine told it runs on Android (its
//! `Environment`), at the size of a small phone. What CI's emulator checks
//! in half an hour, checked here in seconds (layouts that depend on the
//! platform, Android-only features).

use std::time::Duration;

use nsh_core::{geometry::size, platform::Os, prelude::px};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;

/// The app on Android, `width` × `height` points, past its first screen.
fn android(width: f32, height: f32) -> (Headless, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    // SAFETY: every test sets the same values before starting the app.
    unsafe {
        std::env::set_var("DEXDROP_NO_DISCOVERY", "1");
        std::env::set_var("DEXDROP_KEY_STORE", "file");
    }
    let mut config = EngineConfig::new(Root::of::<dexdrop_app::App>());
    config.data_dir = Some(dir.path().to_path_buf());
    config.viewport = size(px(width), px(height));
    config.environment.window = config.viewport;
    config.environment.os = Os::Android;
    config.environment.platform = Some("android");
    let mut app = Headless::start(config).unwrap();
    app.wait_for("Nearby").unwrap();
    (app, dir)
}

/// The frame of the one view `selector` matches: (x, y, width, height).
fn frame(app: &mut Headless, selector: &str) -> (f32, f32, f32, f32) {
    let id = app
        .screen()
        .id(selector)
        .unwrap_or_else(|e| panic!("{selector}: {e}"));
    let node = app
        .screen()
        .layout_nodes()
        .into_iter()
        .find(|n| n.id == id)
        .expect("laid out");
    let b = node.bounds;
    (b.origin.x.0, b.origin.y.0, b.size.width.0, b.size.height.0)
}

/// Send's choices sit at the foot of the screen, in thumb reach, clear of
/// the tab bar. Nothing chosen: one row each (photos, videos, files and,
/// on Android, apps), the screen's width, a thumb's height, the first in
/// the lower part of the screen. Something chosen: the four side by side,
/// to add more, above "Choose device".
#[test]
fn send_offers_its_choices_in_thumb_reach() {
    let (mut app, dir) = android(320.0, 640.0);
    let bottom = 640.0 - 68.0; // Android's drawn tab bar.
    app.press("id:send").unwrap();
    app.wait_for_within("id:pick-apps", Duration::from_secs(5))
        .unwrap();
    let ids = [
        "id:pick-photos",
        "id:pick-videos",
        "id:pick-files",
        "id:pick-apps",
    ];
    let rows = ids.map(|t| frame(&mut app, t));
    for (i, r) in rows.iter().enumerate() {
        assert!(r.3 >= 48.0, "row {i} is {} pt tall", r.3);
        assert!(r.2 >= 280.0, "row {i} is {} pt wide", r.2);
        assert!(
            r.0 >= 0.0 && r.0 + r.2 <= 320.0,
            "row {i} is off the screen"
        );
        if i > 0 {
            assert!(
                r.1 >= rows[i - 1].1 + rows[i - 1].3,
                "row {i} is not below row {}",
                i - 1
            );
        }
    }
    assert!(
        rows[0].1 > 640.0 * 0.4,
        "the choices start at {} pt of a 640 pt screen: out of thumb reach",
        rows[0].1
    );
    let last = rows[3];
    assert!(
        last.1 + last.3 <= bottom,
        "the last row runs under the tab bar"
    );
    // What the page is for shows whole above them.
    let hero = frame(&mut app, "text:Choose what to send");
    assert!(
        hero.1 + hero.3 <= rows[0].1,
        "the heading ends at {} pt, under the choices at {} pt",
        hero.1 + hero.3,
        rows[0].1
    );

    // Something chosen: the choices in one row, above the way on.
    let file = dir.path().join("notes.txt");
    std::fs::write(&file, b"hello").unwrap();
    app.on_pick_paths(vec![file]).unwrap();
    app.press("id:pick-files").unwrap();
    app.wait_for_within("id:next", Duration::from_secs(5))
        .unwrap();
    let strip = ids.map(|t| frame(&mut app, t));
    let next = frame(&mut app, "id:next");
    for (i, t) in strip.iter().enumerate() {
        assert_eq!(t.1, strip[0].1, "choice {i} is not in the row");
        assert!(
            t.3 >= 48.0 && t.2 >= 48.0,
            "choice {i} is {} × {}",
            t.2,
            t.3
        );
        assert!(t.1 + t.3 <= next.1, "choice {i} is not above Choose device");
    }
    assert!(
        next.1 + next.3 <= bottom,
        "Choose device runs under the tab bar"
    );
    assert!(next.1 > 640.0 * 0.6, "Choose device is at {} pt", next.1);
}

/// A long selection lies in the page itself, every shown row laid out (a
/// list virtualized against the page took the swipe on a phone and the
/// page did not scroll), the rest counted, with room under it for the
/// "Choose device" bar.
#[test]
fn a_long_selection_scrolls_with_the_page() {
    let (mut app, dir) = android(360.0, 760.0);
    let files: Vec<_> = (0..130)
        .map(|i| {
            let p = dir.path().join(format!("file{i:03}.bin"));
            std::fs::write(&p, [1u8; 100]).unwrap();
            p
        })
        .collect();
    app.on_pick_paths(files).unwrap();
    app.press("id:send").unwrap();
    app.wait_for("id:pick-files").unwrap();
    app.press("id:pick-files").unwrap();
    app.wait_for_within("text:And 10 more files", Duration::from_secs(10))
        .unwrap();
    // The first and the last row shown are both laid out, far below the
    // screen: the page is that long, and scrolls.
    let first = frame(&mut app, "text:file000.bin");
    let last = frame(&mut app, "text:file119.bin");
    assert!(
        last.1 > first.1 + 119.0 * 50.0,
        "rows at {first:?} … {last:?}"
    );
    let more = frame(&mut app, "text:And 10 more files");
    let next = frame(&mut app, "id:next");
    assert!(
        more.1 > last.1 && more.1 > next.1,
        "the count is not below the rows"
    );
}

/// Send is in the app icon's blue, not black.
#[test]
fn send_is_blue() {
    let (app, _dir) = android(360.0, 760.0);
    let color = app
        .screen()
        .view_style("id:send")
        .unwrap()
        .expect("Send")
        .background_color
        .expect("a fill");
    let (r, g, b) = (
        (color >> 24) & 0xff,
        (color >> 16) & 0xff,
        (color >> 8) & 0xff,
    );
    assert!(b > 180 && b > r + 100 && b > g, "Send is #{color:08x}");
}
