//! The app's screens, driven headless on NSH's real engine (`cargo test`).
//! The transfer engine runs too, on loopback, with discovery off.

use std::time::Duration;

use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;

fn start() -> (Headless, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    // SAFETY: every test sets the same values before starting the app.
    unsafe {
        std::env::set_var("DEXDROP_NO_DISCOVERY", "1");
    }
    let state = dir.path().to_path_buf();
    let mut config = EngineConfig::new(Root::of::<dexdrop_app::App>());
    config.data_dir = Some(state);
    let mut app = Headless::start(config).unwrap();
    app.wait_for("Nearby").unwrap();
    (app, dir)
}

/// Waits until some text on screen contains `part`.
fn wait_for_text(app: &mut Headless, part: &str) {
    for _ in 0..100 {
        if app.screen().texts().iter().any(|t| t.contains(part)) {
            return;
        }
        app.wait(Duration::from_millis(100)).unwrap();
    }
    panic!(
        "no text containing {part:?}; on screen: {:?}",
        app.screen().texts()
    );
}

#[test]
fn shows_the_nearby_screen_with_fallbacks() {
    let (mut app, _dir) = start();
    app.wait_for("WHAT TO SEND").unwrap();
    app.wait_for("DEVICE NOT LISTED?").unwrap();
    app.wait_for("Choose files").unwrap();
    // The engine starts and names this device.
    wait_for_text(&mut app, "is visible to nearby devices");
}

#[test]
fn navigates_between_sections() {
    let (mut app, _dir) = start();
    app.press("role:tab[Transfers]").unwrap();
    app.wait_for("Nothing yet. Transfers you send or receive show up here.")
        .unwrap();
    app.press("role:tab[Settings]").unwrap();
    app.wait_for("Visible to nearby devices").unwrap();
    app.wait_for("PRIVACY AND STORAGE").unwrap();
    app.press("role:tab[Nearby]").unwrap();
    app.wait_for("DEVICES NEARBY").unwrap();
}

#[test]
fn shows_a_pairing_code_and_closes_it() {
    let (mut app, _dir) = start();
    wait_for_text(&mut app, "is visible to nearby devices");
    app.press("role:button[Receive with a code]").unwrap();
    app.wait_for("You'll still be asked before anything is saved.")
        .unwrap();
    app.wait_for_within("label:QR code invitation", Duration::from_secs(10))
        .unwrap();
    app.press("role:button[Done]").unwrap();
    app.wait_for_gone(
        "You'll still be asked before anything is saved.",
        Duration::from_secs(5),
    )
    .unwrap();
}

#[test]
fn send_with_a_code_needs_files_first() {
    let (mut app, _dir) = start();
    app.press("role:button[Send with a code]").unwrap();
    app.wait_for("Choose what to send first").unwrap();
    assert!(!app.screen().enabled("role:button[Send]").unwrap());
    app.press("role:button[Cancel]").unwrap();
}
