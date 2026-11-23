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

/// Android sends apps too: four kinds of things, two by two even on a
/// 320 pt phone, so what is picked shows below them (CI's emulator once
/// found them stacked one per row, the selection out of view).
#[test]
fn send_offers_four_kinds_two_by_two_on_a_small_phone() {
    let (mut app, _dir) = android(320.0, 640.0);
    app.press("role:button[Send]").unwrap();
    app.wait_for_within("role:button[Apps]", Duration::from_secs(5))
        .unwrap();
    let photos = frame(&mut app, "role:button[Photos]");
    let videos = frame(&mut app, "role:button[Videos]");
    let files = frame(&mut app, "role:button[Files]");
    let apps = frame(&mut app, "role:button[Apps]");
    assert_eq!(photos.1, videos.1, "Photos and Videos share a row");
    assert_eq!(files.1, apps.1, "Files and Apps share a row");
    assert!(files.1 > photos.1, "two rows");
    // Both rows end well above the middle of the screen.
    assert!(
        files.1 + files.3 < 400.0,
        "the tiles take {} pt of a 640 pt screen",
        files.1 + files.3
    );
}
