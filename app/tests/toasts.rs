//! Toasts as NSH builds them (`ToastManager`, `ToastStack`, `Toast`),
//! headless: shown with the alert role, timed out by the manager on the
//! clock, and the first Escape dismisses the newest before Escape goes
//! back.

use std::time::{Duration, SystemTime};

use nsh_core::{element::Os, geometry::size, prelude::px, time::Clock};
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
    // SAFETY: every test sets the same values before starting the app.
    unsafe {
        std::env::set_var("DEXDROP_NO_DISCOVERY", "1");
        std::env::set_var("DEXDROP_KEY_STORE", "file");
    }
    let mut config = EngineConfig::new(Root::of::<dexdrop_app::App>());
    config.data_dir = Some(dir.path().to_path_buf());
    env(&mut config);
    let mut app = Headless::start(config).unwrap();
    app.wait_for("Nearby").unwrap();
    (app, dir)
}

/// Copying the invitation says so in a toast: an alert, gone by itself
/// after its 4 s (the manager's timeout, on the manual clock).
#[test]
fn a_toast_shows_then_times_out() {
    let (mut app, _dir) = start(|c| c.clock = Clock::manual(SystemTime::now()));
    app.press("id:receive").unwrap();
    app.wait_for_within("id:copy-invitation", Duration::from_secs(10))
        .unwrap();
    app.press("id:copy-invitation").unwrap();
    app.wait_for("text:Invitation copied.").unwrap();
    assert!(app.screen().id("role:alert").is_ok(), "not an alert");
    app.fast_forward(Duration::from_secs(5)).unwrap();
    app.wait_for_gone("text:Invitation copied.", Duration::from_secs(5))
        .unwrap();
}

/// On a computer, Escape first dismisses the newest toast (its stack hears
/// it), and only then goes back.
#[test]
fn escape_dismisses_a_toast_before_going_back() {
    let (mut app, _dir) = start(|c| {
        c.viewport = size(px(1100.0), px(800.0));
        c.environment.window = c.viewport;
        c.environment.os = Os::MacOs;
        c.environment.platform = Some("macos");
    });
    app.press("id:receive").unwrap();
    app.wait_for_within("id:copy-invitation", Duration::from_secs(10))
        .unwrap();
    app.press("id:copy-invitation").unwrap();
    app.wait_for("text:Invitation copied.").unwrap();
    app.press_key("escape").unwrap();
    app.wait_for_gone("text:Invitation copied.", Duration::from_secs(5))
        .unwrap();
    // Still on Receive: the Escape went to the toast.
    assert!(app.screen().id("id:copy-invitation").is_ok());
    app.press_key("escape").unwrap();
    app.wait_for_within("id:receive", Duration::from_secs(5))
        .unwrap();
}
