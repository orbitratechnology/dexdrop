//! Send's Recent, headless: the newest files on a computer as rows, the
//! newest photos and videos on Android and iPhone as a grid of squares; a
//! tap adds an item to the selection, another takes it out. (On a real
//! phone the items come from the system's media or photo library;
//! headless, from the folders in `DEXDROP_RECENT_DIRS`.)

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

fn frame(app: &mut Headless, selector: &str) -> (f32, f32, f32, f32) {
    let id = app
        .screen()
        .id(selector)
        .unwrap_or_else(|e| panic!("{selector}: {e}"));
    let b = app
        .screen()
        .layout_nodes()
        .into_iter()
        .find(|n| n.id == id)
        .expect("laid out")
        .bounds;
    (b.origin.x.0, b.origin.y.0, b.size.width.0, b.size.height.0)
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

/// Android shows its newest photos and videos as squares, four across;
/// a video says how long it is.
#[test]
fn android_offers_its_newest_photos_and_videos() {
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let (mut app, _dir) = start(|c| {
        c.viewport = size(px(360.0), px(760.0));
        c.environment.window = c.viewport;
        c.environment.os = Os::Android;
        c.environment.platform = Some("android");
    });
    app.press("id:send").unwrap();
    app.wait_for_within("text:Recent photos and videos", Duration::from_secs(5))
        .unwrap();
    app.wait_for_within("label:Photo beach.jpg", Duration::from_secs(5))
        .unwrap();
    let photo = frame(&mut app, "label:Photo beach.jpg");
    assert!((photo.2 - photo.3).abs() < 0.5, "not square: {photo:?}");
    assert!(
        photo.2 >= 70.0 && photo.2 <= 90.0,
        "{} pt: not four across",
        photo.2
    );
    assert!(app.screen().id("label:Video clip.mp4").is_ok());
    app.press("label:Photo beach.jpg").unwrap();
    app.wait_for_within("id:next", Duration::from_secs(5))
        .unwrap();
}

/// An iPhone shows the photo library's newest photos and videos (PhotoKit;
/// headless, the folder standing in for it): a tap exports the original
/// into staging and selects that copy; another tap takes it out.
#[test]
fn an_iphone_offers_its_photo_library() {
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let (mut app, dir) = start(|_| {});
    app.press("id:send").unwrap();
    app.wait_for_within("text:Recent photos and videos", Duration::from_secs(5))
        .unwrap();
    app.wait_for_within("label:Photo beach.jpg", Duration::from_secs(5))
        .unwrap();
    assert!(app.screen().id("label:Video clip.mp4").is_ok());
    // Only photos and videos come from a photo library.
    assert!(app.screen().id("label:Photo report.pdf").is_err());
    app.press("label:Photo beach.jpg").unwrap();
    app.wait_for_within("id:next", Duration::from_secs(5))
        .unwrap();
    // The selection holds the exported copy, in staging.
    let staged: Vec<_> = walk(&dir.path().join("state").join("staging"))
        .into_iter()
        .filter(|p| p.file_name().is_some_and(|n| n == "beach.jpg"))
        .collect();
    assert_eq!(staged.len(), 1, "exported once: {staged:?}");
    assert_eq!(std::fs::read(&staged[0]).unwrap(), vec![2u8; 4096]);
    app.press("label:Photo beach.jpg").unwrap();
    app.wait_for_gone("id:next", Duration::from_secs(5))
        .unwrap();
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else {
            out.push(path);
        }
    }
    out
}
