//! The app's screens, driven headless on NSH's real engine (`cargo test`).
//! The transfer engine runs too, on loopback, with discovery off.

use std::time::Duration;

use nsh_engine::{EngineConfig, Root};
use nsh_testing::{DialogAnswer, Headless};

/// The headless engine is an iPhone: its tab bar is the system's (a native
/// view tests cannot press), so tests switch tabs with the app's shortcuts
/// (mod+1 Share, mod+2 History, mod+3 Settings). The first launch explains
/// the local network (FR-004); `start` reads it and goes on.
fn start() -> (Headless, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let mut app = open(&dir);
    app.wait_for_within("role:button[Continue]", Duration::from_secs(10))
        .unwrap();
    app.press("role:button[Continue]").unwrap();
    app.wait_for_gone("Find your devices", Duration::from_secs(5))
        .unwrap();
    (app, dir)
}

/// The app on the state in `dir`, at its first screen.
fn open(dir: &tempfile::TempDir) -> Headless {
    // SAFETY: every test sets the same values before starting the app.
    unsafe {
        std::env::set_var("DEXDROP_NO_DISCOVERY", "1");
        // Keys stay in the test's files, not the machine's keyring.
        std::env::set_var("DEXDROP_KEY_STORE", "file");
    }
    let mut config = EngineConfig::new(Root::of::<dexdrop_app::App>());
    config.data_dir = Some(dir.path().to_path_buf());
    let mut app = Headless::start(config).unwrap();
    app.wait_for("Nearby").unwrap();
    app
}

/// iOS asks before an app uses the local network: the first launch says
/// why, once (FR-004).
#[test]
fn explains_the_local_network_once() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = open(&dir);
    app.wait_for_within("Find your devices", Duration::from_secs(10))
        .unwrap();
    wait_for_text(&mut app, "Allow access to your local network");
    app.press("role:button[Continue]").unwrap();
    app.wait_for_gone("Find your devices", Duration::from_secs(5))
        .unwrap();
    drop(app);

    let mut again = open(&dir);
    wait_for_text(&mut again, "Visible as ");
    again.wait(Duration::from_millis(500)).unwrap();
    assert!(
        !again.screen().texts().contains(&"Find your devices"),
        "explained again: {:?}",
        again.screen().texts()
    );
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

/// The "Linux · abcd-efgh-ijkl" line under the device name in Settings.
fn device_id(app: &mut Headless) -> String {
    let is_id = |s: &str| s.len() == 14 && s.chars().filter(|c| *c == '-').count() == 2;
    for _ in 0..100 {
        if let Some(id) = app.screen().texts().iter().find_map(|t| {
            t.rsplit_once(" · ")
                .map(|(_, id)| id.to_owned())
                .filter(|id| is_id(id))
        }) {
            return id;
        }
        app.wait(Duration::from_millis(100)).unwrap();
    }
    panic!("no device ID on screen: {:?}", app.screen().texts());
}

#[test]
fn home_offers_send_and_receive() {
    let (mut app, _dir) = start();
    app.wait_for("id:send").unwrap();
    app.wait_for("id:receive").unwrap();
    // The engine starts and names this device.
    wait_for_text(&mut app, "Visible as ");
    // No device yet: the app says it is looking.
    wait_for_text(&mut app, "Looking for devices");
}

#[test]
fn switches_tabs() {
    let (mut app, _dir) = start();
    app.press_key("mod+2").unwrap();
    app.wait_for("Files you send and receive show up here.")
        .unwrap();
    app.press_key("mod+3").unwrap();
    app.wait_for("Visible to nearby devices").unwrap();
    app.wait_for("Privacy").unwrap();
    app.press_key("mod+1").unwrap();
    app.wait_for("id:receive").unwrap();
}

#[test]
fn receive_shows_a_code_and_closes_it() {
    let (mut app, _dir) = start();
    wait_for_text(&mut app, "Visible as ");
    app.press("id:receive").unwrap();
    app.wait_for("Ready to receive").unwrap();
    wait_for_text(&mut app, "Or enter this code");
    app.wait_for_within("label:QR code invitation", Duration::from_secs(10))
        .unwrap();
    app.wait_for("id:copy-invitation").unwrap();
    // No direct link where the platform has none (tests/direct.rs has one).
    assert!(app.screen().id("id:direct-link").is_err());
    app.press_key("escape").unwrap();
    app.wait_for_gone("Ready to receive", Duration::from_secs(5))
        .unwrap();
}

/// Choosing the tab already shown goes back to its root; the Receive page
/// asks for its own code, so it has one again after a trip to another tab.
#[test]
fn reselecting_a_tab_goes_home_and_receive_gets_a_fresh_code() {
    let (mut app, _dir) = start();
    wait_for_text(&mut app, "Visible as ");
    app.press("id:receive").unwrap();
    app.wait_for_within("label:QR code invitation", Duration::from_secs(10))
        .unwrap();
    app.press_key("mod+1").unwrap();
    app.wait_for_gone("Ready to receive", Duration::from_secs(5))
        .unwrap();
    app.wait_for("id:receive").unwrap();

    app.press("id:receive").unwrap();
    app.wait_for_within("label:QR code invitation", Duration::from_secs(10))
        .unwrap();
    app.press_key("mod+2").unwrap();
    app.wait_for("Files you send and receive show up here.")
        .unwrap();
    app.press_key("mod+1").unwrap();
    app.wait_for("Ready to receive").unwrap();
    app.wait_for_within("label:QR code invitation", Duration::from_secs(10))
        .unwrap();
    for _ in 0..100 {
        if !app.screen().texts().iter().any(|t| t.contains("····")) {
            break;
        }
        app.wait(Duration::from_millis(100)).unwrap();
    }
    assert!(
        !app.screen().texts().iter().any(|t| t.contains("····")),
        "no code: {:?}",
        app.screen().texts()
    );
}

#[test]
fn send_offers_photos_videos_and_files() {
    let (mut app, _dir) = start();
    app.press("id:send").unwrap();
    app.wait_for("id:pick-photos").unwrap();
    app.wait_for("id:pick-videos").unwrap();
    app.wait_for("id:pick-files").unwrap();
    // Nothing chosen yet: no way on to a device.
    assert!(app.screen().id("id:next").is_err());
    app.press_key("escape").unwrap();
    app.wait_for_gone("id:pick-photos", Duration::from_secs(5))
        .unwrap();
}

#[test]
fn keyboard_shortcuts_switch_tabs_and_go_back() {
    let (mut app, _dir) = start();
    app.press_key("mod+2").unwrap();
    app.wait_for("Files you send and receive show up here.")
        .unwrap();
    app.press_key("mod+3").unwrap();
    app.wait_for("Visible to nearby devices").unwrap();
    app.press_key("mod+1").unwrap();
    app.press("id:send").unwrap();
    app.wait_for("id:pick-photos").unwrap();
    app.press_key("escape").unwrap();
    app.wait_for_gone("id:pick-photos", Duration::from_secs(5))
        .unwrap();
}

/// Every control the app draws on the phone layout is at least 44 × 44 pt
/// (PRD 7.7; the headless engine is an iPhone, where 44 pt is the minimum).
/// The tab bar and switches are the system's own, sized by the system.
#[test]
fn phone_controls_meet_the_minimum_hit_target() {
    let (mut app, _dir) = start();
    wait_for_text(&mut app, "Visible as ");
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
    check(&mut app, &["id:send", "id:receive"]);
    app.press("id:send").unwrap();
    app.wait_for("id:pick-photos").unwrap();
    check(
        &mut app,
        &["id:pick-photos", "id:pick-videos", "id:pick-files"],
    );
    app.press_key("mod+3").unwrap();
    app.wait_for("Visible to nearby devices").unwrap();
    check(
        &mut app,
        &[
            "id:rename",
            "role:button[Export diagnostics]",
            "role:button[Reset device identity…]",
        ],
    );
    assert!(too_small.is_empty(), "controls under 44 pt: {too_small:#?}");
}

#[test]
fn resetting_the_identity_gives_a_new_device_id() {
    let (mut app, _dir) = start();
    wait_for_text(&mut app, "Visible as ");
    app.press_key("mod+3").unwrap();
    let before = device_id(&mut app);
    // The system's alert asks first.
    app.answer_dialogs_myself();
    app.press("role:button[Reset device identity…]").unwrap();
    let dialog = app.pending_dialog().expect("an alert").clone();
    assert_eq!(
        dialog.title.as_deref(),
        Some("Reset this device's identity?")
    );
    // Cancel changes nothing.
    app.answer_dialog(DialogAnswer::Press("Cancel".into()))
        .unwrap();
    assert_eq!(device_id(&mut app), before);

    app.press("role:button[Reset device identity…]").unwrap();
    app.answer_dialog(DialogAnswer::Press("Reset identity".into()))
        .unwrap();
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

/// Settings: the avatar opens the picker; an emoji chosen shows at once and
/// is kept.
#[test]
fn chooses_an_avatar() {
    let (mut app, dir) = start();
    wait_for_text(&mut app, "Visible as ");
    app.press_key("mod+3").unwrap();
    app.press("id:change-avatar").unwrap();
    app.wait_for("Emoji").unwrap();
    let foxes = |app: &mut Headless| app.screen().texts().iter().filter(|t| **t == "🦊").count();
    assert_eq!(foxes(&mut app), 1, "only the choice shows a fox");
    app.press("role:button[🦊]").unwrap();
    app.wait_until("the fox on the avatar", Duration::from_secs(5), |s| {
        s.texts().iter().filter(|t| **t == "🦊").count() > 1
    })
    .unwrap();
    assert!(foxes(&mut app) > 1, "the avatar does not show the fox");
    drop(app);
    let settings = std::fs::read_to_string(dir.path().join("state").join("settings.json"))
        .or_else(|_| std::fs::read_to_string(dir.path().join("settings.json")))
        .unwrap_or_default();
    assert!(settings.contains("🦊"), "not kept: {settings}");
}

/// Diagnostics open in a dialog (`ui::dialog`, NSH's `Dialog`): screen
/// readers hear a dialog named by its title, Escape closes it, and Escape
/// is the dialog's alone (Settings stays, nothing goes back under it).
#[test]
fn diagnostics_open_in_a_dialog_that_escape_closes() {
    let (mut app, _dir) = start();
    app.press_key("mod+3").unwrap();
    app.wait_for("Export diagnostics").unwrap();
    app.press("role:button[Export diagnostics]").unwrap();
    app.wait_for_within("role:dialog[Diagnostics]", Duration::from_secs(5))
        .unwrap();
    app.wait_for("role:button[Copy]").unwrap();
    app.press_key("escape").unwrap();
    app.wait_for_gone("role:dialog[Diagnostics]", Duration::from_secs(5))
        .unwrap();
    app.wait_for("Export diagnostics").unwrap();
}
