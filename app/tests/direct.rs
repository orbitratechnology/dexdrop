//! The direct link on Receive (docs/CONNECTIVITY.md 4.3), headless, with
//! the engine's loopback link standing in for the radio
//! (`DEXDROP_DIRECT_LINK=loopback`; its own test binary, since the
//! variable is the process's).

#[path = "common/shot.rs"]
mod shot;

use std::{path::PathBuf, time::Duration};

use nsh_core::{element::Os, geometry::size, prelude::px};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;

/// An Android phone, the first platform to host one.
fn start() -> (Headless, tempfile::TempDir, shot::Camera) {
    let dir = tempfile::tempdir().unwrap();
    for state in [dir.path().to_path_buf(), dir.path().join("state")] {
        std::fs::create_dir_all(&state).unwrap();
        std::fs::write(
            state.join("settings.json"),
            r#"{"local_network_explained":true,"device_name":"Test"}"#,
        )
        .unwrap();
    }
    // SAFETY: the one test here sets these before starting the app.
    unsafe {
        std::env::set_var("DEXDROP_NO_DISCOVERY", "1");
        std::env::set_var("DEXDROP_KEY_STORE", "file");
        std::env::set_var("DEXDROP_DIRECT_LINK", "loopback");
    }
    let mut config = EngineConfig::new(Root::of::<dexdrop_app::App>());
    config.data_dir = Some(dir.path().to_path_buf());
    config.viewport = size(px(390.0), px(844.0));
    config.environment.window = config.viewport;
    config.environment.os = Os::Android;
    config.environment.platform = Some("android");
    let mut app = Headless::start(config).unwrap();
    let camera = shot::Camera::attach(&mut app);
    app.wait_for("Nearby").unwrap();
    (app, dir, camera)
}

fn shows(app: &mut Headless, part: &str) -> bool {
    app.screen().texts().iter().any(|t| t.contains(part))
}

/// Receive offers a direct link where the device can host one: it starts,
/// says it is on and on which band, and stops when asked. Pictures of it
/// off and on go to `DEXDROP_SHOTS` (default `target/shots/direct`).
#[test]
fn receive_starts_and_stops_a_direct_link() {
    let out = std::env::var_os("DEXDROP_SHOTS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/shots/direct")
        });
    let (mut app, _dir, camera) = start();
    app.wait_for_within("id:receive", Duration::from_secs(10))
        .unwrap();
    app.press("id:receive").unwrap();
    app.wait_for_within("label:QR code invitation", Duration::from_secs(10))
        .unwrap();
    app.wait_for_within("id:direct-link", Duration::from_secs(10))
        .unwrap();
    camera.shoot(&mut app, &out, "receive-direct-off");

    app.press("id:direct-link").unwrap();
    app.wait_for_within("id:direct-link-on", Duration::from_secs(10))
        .unwrap();
    assert!(shows(&mut app, "Direct link on (5 GHz)"));
    // The QR code now carries the network: a new invitation replaced it.
    app.wait_for("label:QR code invitation").unwrap();
    camera.shoot(&mut app, &out, "receive-direct-on");

    app.press("id:direct-link-stop").unwrap();
    app.wait_for_within("id:direct-link", Duration::from_secs(10))
        .unwrap();
    assert!(app.screen().id("id:direct-link-on").is_err());
}
