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
        // Keys stay in the test's files, not the machine's keyring.
        std::env::set_var("DEXDROP_KEY_STORE", "file");
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
    app.wait_for("role:button[Paste]").unwrap();
    app.wait_for("role:button[Read QR from picture]").unwrap();
    app.press("role:button[Cancel]").unwrap();
}

#[test]
fn keyboard_shortcuts_switch_sections_and_close_sheets() {
    let (mut app, _dir) = start();
    app.press_key("mod+2").unwrap();
    app.wait_for("Nothing yet. Transfers you send or receive show up here.")
        .unwrap();
    app.press_key("mod+3").unwrap();
    app.wait_for("PRIVACY AND STORAGE").unwrap();
    app.press_key("mod+1").unwrap();
    app.wait_for("DEVICES NEARBY").unwrap();
    app.press("role:button[Send with a code]").unwrap();
    app.wait_for("role:button[Paste]").unwrap();
    app.press_key("escape").unwrap();
    app.wait_for_gone("role:button[Paste]", Duration::from_secs(5))
        .unwrap();
}

/// Every control on the phone layout is at least 44 × 44 pt (PRD 7.7; the
/// headless engine is an iPhone, where 44 pt is the minimum).
#[test]
fn phone_controls_meet_the_minimum_hit_target() {
    let (mut app, _dir) = start();
    wait_for_text(&mut app, "is visible to nearby devices");
    let mut too_small = Vec::new();
    let mut check = |app: &mut Headless, selectors: &[&str]| {
        let nodes = app.screen().layout_nodes();
        for selector in selectors {
            let id = app
                .screen()
                .id(selector)
                .unwrap_or_else(|e| panic!("{selector}: {e}"));
            let node = nodes.iter().find(|n| n.id == id).expect("laid out");
            let (w, h) = (node.bounds.size.width.0, node.bounds.size.height.0);
            if w < 44.0 || h < 44.0 {
                too_small.push(format!("{selector}: {w:.0} × {h:.0} pt"));
            }
        }
    };
    check(
        &mut app,
        &[
            "role:tab[Nearby]",
            "role:tab[Transfers]",
            "role:tab[Settings]",
            "role:button[Choose files]",
            "role:button[Receive with a code]",
            "role:button[Send with a code]",
        ],
    );
    app.press("role:tab[Settings]").unwrap();
    app.wait_for("PRIVACY AND STORAGE").unwrap();
    check(
        &mut app,
        &[
            "role:button[Rename]",
            "role:button[Export diagnostics]",
            "role:switch[Visible to nearby devices]",
            "role:switch[Keep transfer history]",
        ],
    );
    assert!(too_small.is_empty(), "controls under 44 pt: {too_small:#?}");
}

/// The " · ID xxxx-xxxx-xxxx" line under the device name in Settings.
fn device_id(app: &mut Headless) -> String {
    for _ in 0..100 {
        if let Some(id) = app
            .screen()
            .texts()
            .iter()
            .find_map(|t| t.split_once(" · ID ").map(|(_, id)| id.to_owned()))
        {
            return id;
        }
        app.wait(Duration::from_millis(100)).unwrap();
    }
    panic!("no device ID on screen: {:?}", app.screen().texts());
}

#[test]
fn resetting_the_identity_gives_a_new_device_id() {
    let (mut app, _dir) = start();
    wait_for_text(&mut app, "is visible to nearby devices");
    app.press("role:tab[Settings]").unwrap();
    let before = device_id(&mut app);
    app.press("role:button[Reset device identity…]").unwrap();
    app.wait_for("Reset this device's identity?").unwrap();
    // Cancel changes nothing.
    app.press("role:button[Cancel]").unwrap();
    app.wait_for_gone("Reset this device's identity?", Duration::from_secs(5))
        .unwrap();
    assert_eq!(device_id(&mut app), before);

    app.press("role:button[Reset device identity…]").unwrap();
    app.press("role:button[Reset identity]").unwrap();
    wait_for_text(&mut app, "This device has a new identity.");
    let mut after = device_id(&mut app);
    for _ in 0..100 {
        if after != before {
            break;
        }
        app.wait(Duration::from_millis(100)).unwrap();
        after = device_id(&mut app);
    }
    assert_ne!(after, before, "the device ID did not change");
    // The restarted engine is running: no "stopped" banner.
    assert!(
        !app.screen()
            .texts()
            .iter()
            .any(|t| t.contains("The transfer service stopped")),
        "{:?}",
        app.screen().texts()
    );
}
