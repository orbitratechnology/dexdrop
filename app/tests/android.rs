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

/// Android sends apps too: four kinds of things, square, side by side in
/// one row even on a 320 pt phone, so what is picked shows below them.
#[test]
fn send_offers_four_square_kinds_in_a_row_on_a_small_phone() {
    let (mut app, _dir) = android(320.0, 640.0);
    app.press("id:send").unwrap();
    app.wait_for_within("id:pick-apps", Duration::from_secs(5))
        .unwrap();
    let tiles = [
        "id:pick-photos",
        "id:pick-videos",
        "id:pick-files",
        "id:pick-apps",
    ]
    .map(|t| frame(&mut app, t));
    for (i, t) in tiles.iter().enumerate() {
        assert_eq!(t.1, tiles[0].1, "tile {i} is not in the row");
        assert!((t.2 - t.3).abs() < 0.5, "tile {i} is {} × {}", t.2, t.3);
        assert!(
            t.0 >= 0.0 && t.0 + t.2 <= 320.0,
            "tile {i} is off the screen"
        );
    }
    assert!(
        tiles[0].1 + tiles[0].3 < 250.0,
        "the tiles end at {} pt of a 640 pt screen",
        tiles[0].1 + tiles[0].3
    );
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
