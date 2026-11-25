//! Send's Recent, headless: the newest files on a computer as rows (from
//! the folders in `DEXDROP_RECENT_DIRS`); a tap adds one to the selection,
//! another takes it out. Phones have none: their photos and videos would
//! take a broad photo permission (ADR-008).

use std::time::Duration;

use nsh_core::{element::Os, geometry::size, prelude::px};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;

fn start(env: impl FnOnce(&mut EngineConfig)) -> (Headless, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    for state in [dir.path().to_path_buf(), dir.path().join("state")] {
        std::fs::create_dir_all(&state).unwrap();
        std::fs::write(
            state.join("settings.json"),
            r#"{"local_network_explained":true,"device_name":"Test"}"#,
        )
        .unwrap();
    }
    let recent = dir.path().join("recent");
    std::fs::create_dir_all(&recent).unwrap();
    std::fs::write(recent.join("report.pdf"), vec![1u8; 2048]).unwrap();
    std::fs::write(recent.join("beach.jpg"), vec![2u8; 4096]).unwrap();
    std::fs::write(recent.join("clip.mp4"), vec![3u8; 8192]).unwrap();
    // SAFETY: every test sets the same values before starting the app
    // (the folder differs, and each test only reads its own).
    unsafe {
        std::env::set_var("DEXDROP_NO_DISCOVERY", "1");
        std::env::set_var("DEXDROP_KEY_STORE", "file");
        std::env::set_var("DEXDROP_RECENT_DIRS", &recent);
    }
    let mut config = EngineConfig::new(Root::of::<dexdrop_app::App>());
    config.data_dir = Some(dir.path().to_path_buf());
    env(&mut config);
    let mut app = Headless::start(config).unwrap();
    app.wait_for("Nearby").unwrap();
    (app, dir)
}

/// The tests share a process: one at a time, since each points
/// `DEXDROP_RECENT_DIRS` at its own folder.
static ONE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A computer lists its newest files under "Recent files"; a tap adds one
/// to what is sent (Next shows), another takes it out.
#[test]
fn a_computer_offers_its_newest_files() {
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let (mut app, _dir) = start(|c| {
        c.viewport = size(px(1100.0), px(800.0));
        c.environment.window = c.viewport;
        c.environment.os = Os::MacOs;
        c.environment.platform = Some("macos");
    });
    app.press("id:send").unwrap();
    app.wait_for_within("text:Recent files", Duration::from_secs(5))
        .unwrap();
    app.wait_for_within("label:Add report.pdf", Duration::from_secs(5))
        .unwrap();
    assert!(app.screen().id("label:Add clip.mp4").is_ok());
    app.press("label:Add report.pdf").unwrap();
    app.wait_for_within("id:next", Duration::from_secs(5))
        .unwrap();
    app.press("label:Add report.pdf").unwrap();
    app.wait_for_gone("id:next", Duration::from_secs(5))
        .unwrap();
}

/// Phones show no Recent and ask for no photos: their newest photos and
/// videos take a broad photo permission (Android's READ_MEDIA_*, which
/// Google Play restricts; iOS's full photo library), so Send offers the
/// system's pickers only (ADR-008).
#[test]
fn phones_show_no_recent_and_ask_for_no_photos() {
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    for (os, platform) in [(Os::Android, "android"), (Os::Ios, "ios")] {
        let (mut app, _dir) = start(|c| {
            c.viewport = size(px(360.0), px(760.0));
            c.environment.window = c.viewport;
            c.environment.os = os;
            c.environment.platform = Some(platform);
        });
        app.press("id:send").unwrap();
        app.wait_for_within("id:pick-photos", Duration::from_secs(5))
            .unwrap();
        let screen = app.screen();
        assert!(
            screen.id("id:recent").is_err(),
            "{platform}: a Recent section"
        );
        assert!(
            screen.id("id:allow-recent").is_err(),
            "{platform}: a photos prompt"
        );
        assert!(!screen.texts().contains(&"Recent photos and videos"));
    }
}
