//! Sending to several devices, headless: a computer with two devices it
//! knows, files dropped on Send, then each device tapped in the list.

use std::time::Duration;

use dexdrop_core::{
    identity::PeerId,
    store::{KnownPeer, KnownPeers},
};
use nsh_core::{element::Os, geometry::size, prelude::px};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;

fn known(name: &str, n: u8) -> KnownPeer {
    KnownPeer {
        peer_id: PeerId([n; 32]),
        name: name.into(),
        platform: "linux".into(),
        key: [n; 32],
        relay: None,
        last_seen: 1_700_000_000,
        avatar: None,
    }
}

/// The app on a Mac-sized window, knowing Desk and Laptop (listed as
/// devices over the internet).
fn start() -> (Headless, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    for state in [dir.path().to_path_buf(), dir.path().join("state")] {
        std::fs::create_dir_all(&state).unwrap();
        std::fs::write(
            state.join("settings.json"),
            r#"{"local_network_explained":true,"device_name":"Test","internet":true}"#,
        )
        .unwrap();
        KnownPeers {
            peers: vec![known("Desk", 1), known("Laptop", 2)],
        }
        .save(&state.join("known-devices.json"))
        .unwrap();
    }
    // SAFETY: every test sets the same values before starting the app.
    unsafe {
        std::env::set_var("DEXDROP_NO_DISCOVERY", "1");
        std::env::set_var("DEXDROP_KEY_STORE", "file");
    }
    let mut config = EngineConfig::new(Root::of::<dexdrop_app::App>());
    config.data_dir = Some(dir.path().to_path_buf());
    config.viewport = size(px(1100.0), px(800.0));
    config.environment.window = config.viewport;
    config.environment.os = Os::MacOs;
    config.environment.platform = Some("macos");
    let mut app = Headless::start(config).unwrap();
    app.wait_for("Nearby").unwrap();
    (app, dir)
}

/// Tapping a device sends to it and the list stays: another tap sends to
/// another device, each row says how its transfer goes, and the progress
/// shows them all.
#[test]
fn tapping_devices_one_after_another_sends_to_each() {
    let (mut app, dir) = start();
    let file = dir.path().join("talk.pdf");
    std::fs::write(&file, vec![7u8; 4096]).unwrap();
    app.press("id:send").unwrap();
    app.wait_for("id:pick-files").unwrap();
    app.drop_files("id:pick-files", vec![format!("file://{}", file.display())])
        .unwrap();
    app.wait_for_within("id:next", Duration::from_secs(5))
        .unwrap();
    app.press("id:next").unwrap();
    app.wait_for_within("label:Send to Desk", Duration::from_secs(5))
        .unwrap();

    app.press("label:Send to Desk").unwrap();
    // Sent (it says how it goes); the list is still there.
    app.wait_for_within("label:/^Desk, /", Duration::from_secs(5))
        .unwrap();
    assert!(app.screen().id("label:Send to Laptop").is_ok());
    assert!(app.screen().id("id:see-progress").is_ok());

    app.press("label:Send to Laptop").unwrap();
    app.wait_for_within("label:/^Laptop, /", Duration::from_secs(5))
        .unwrap();

    // Both, together.
    app.press("id:see-progress").unwrap();
    app.wait_for_within("text:/^Sent to \\d of 2$/", Duration::from_secs(5))
        .unwrap();
    assert!(app.screen().id("label:/^Desk, /").is_ok());
    assert!(app.screen().id("label:/^Laptop, /").is_ok());
}

/// On a small phone (Android, 320 × 640) the device list starts high
/// enough that a device's row is clear of the tab bar floating over the
/// bottom 96 pt: CI's emulator once tapped a row there and hit History.
#[test]
fn devices_clear_the_tab_bar_on_a_small_phone() {
    let dir = tempfile::tempdir().unwrap();
    for state in [dir.path().to_path_buf(), dir.path().join("state")] {
        std::fs::create_dir_all(&state).unwrap();
        std::fs::write(
            state.join("settings.json"),
            r#"{"local_network_explained":true,"device_name":"Test","internet":true}"#,
        )
        .unwrap();
        KnownPeers {
            peers: vec![known("Desk", 1)],
        }
        .save(&state.join("known-devices.json"))
        .unwrap();
    }
    // SAFETY: every test sets the same values before starting the app.
    unsafe {
        std::env::set_var("DEXDROP_NO_DISCOVERY", "1");
        std::env::set_var("DEXDROP_KEY_STORE", "file");
    }
    let mut config = EngineConfig::new(Root::of::<dexdrop_app::App>());
    config.data_dir = Some(dir.path().to_path_buf());
    config.viewport = size(px(320.0), px(640.0));
    config.environment.window = config.viewport;
    config.environment.os = Os::Android;
    config.environment.platform = Some("android");
    let mut app = Headless::start(config).unwrap();
    app.wait_for("Nearby").unwrap();

    let file = dir.path().join("talk.pdf");
    std::fs::write(&file, vec![7u8; 4096]).unwrap();
    app.on_pick_paths(vec![file]).unwrap();
    app.press("id:send").unwrap();
    app.wait_for("id:pick-files").unwrap();
    app.press("id:pick-files").unwrap();
    app.wait_for_within("id:next", Duration::from_secs(5))
        .unwrap();
    app.press("id:next").unwrap();
    app.wait_for_within("label:Send to Desk", Duration::from_secs(5))
        .unwrap();
    let id = app.screen().id("label:Send to Desk").unwrap();
    let row = app
        .screen()
        .layout_nodes()
        .into_iter()
        .find(|n| n.id == id)
        .expect("laid out")
        .bounds;
    let bottom = row.origin.y.0 + row.size.height.0;
    assert!(
        bottom <= 640.0 - 96.0,
        "the row ends at {bottom} pt, under the tab bar"
    );
}
