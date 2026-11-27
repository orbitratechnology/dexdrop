//! The keyboard and long presses, headless: shortcuts for the common
//! actions, removing a past transfer and forgetting a device.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dexdrop_core::{
    identity::PeerId,
    store::{KnownPeer, KnownPeers},
};
use nsh_core::{element::Os, geometry::size, prelude::px};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::{DialogAnswer, Headless};

/// The app on a Mac-sized window, with one past transfer and one device
/// known from before (over the internet).
fn start() -> (Headless, tempfile::TempDir) {
    start_on(Os::MacOs, "macos")
}

/// The app on `os` (`platform`), in a computer-sized window.
fn start_on(os: Os, platform: &'static str) -> (Headless, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let finished = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        - 60;
    let history = format!(
        r#"{{"entries":[{{"transfer_id":"{:032x}","direction":"Incoming","peer_name":"Desk","peer_platform":"linux","label":"Report.pdf","files":1,"bytes":1000,"outcome":"Completed","finished":{finished},"saved_to":null,"items":[]}}]}}"#,
        7
    );
    for state in [dir.path().to_path_buf(), dir.path().join("state")] {
        std::fs::create_dir_all(&state).unwrap();
        std::fs::write(state.join("history.json"), &history).unwrap();
        std::fs::write(
            state.join("settings.json"),
            r#"{"local_network_explained":true,"device_name":"Test","internet":true}"#,
        )
        .unwrap();
        KnownPeers {
            peers: vec![KnownPeer {
                peer_id: PeerId([3; 32]),
                name: "Laptop".into(),
                platform: "windows".into(),
                key: [3; 32],
                relay: None,
                last_seen: 1_700_000_000,
                avatar: None,
            }],
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
    config.environment.os = os;
    config.environment.platform = Some(platform);
    let mut app = Headless::start(config).unwrap();
    app.wait_for("Nearby").unwrap();
    (app, dir)
}

/// On Home, S (or ←) opens Send and R (or →) opens Receive; Escape goes
/// back.
#[test]
fn keys_open_send_and_receive() {
    let (mut app, _dir) = start();
    app.press_key("s").unwrap();
    app.wait_for_within("id:pick-files", Duration::from_secs(5))
        .unwrap();
    app.press_key("escape").unwrap();
    app.wait_for("id:receive").unwrap();
    app.press_key("right").unwrap();
    app.wait_for_within("text:Ready to receive", Duration::from_secs(5))
        .unwrap();
    app.press_key("escape").unwrap();
    app.press_key("left").unwrap();
    app.wait_for_within("id:pick-files", Duration::from_secs(5))
        .unwrap();
}

/// A past transfer's context menu (right-clicked here; held on a phone)
/// removes it after asking; its files are not touched.
#[test]
fn long_press_removes_a_past_transfer() {
    let (mut app, _dir) = start();
    app.press_key("mod+2").unwrap();
    app.wait_for("label:Report.pdf").unwrap();
    app.answer_dialogs_myself();
    app.right_click("label:Report.pdf").unwrap();
    app.wait_for("label:Remove from History").unwrap();
    app.press("label:Remove from History").unwrap();
    let dialog = app.pending_dialog().expect("an alert").clone();
    assert_eq!(
        dialog.title.as_deref(),
        Some("Remove Report.pdf from History?")
    );
    app.answer_dialog(DialogAnswer::Press("Remove".into()))
        .unwrap();
    app.wait_for_gone("label:Report.pdf", Duration::from_secs(5))
        .unwrap();
}

/// A device in Your devices held opens its menu; Forget asks, Cancel keeps
/// it.
#[test]
fn long_press_forgets_a_device_after_asking() {
    let (mut app, _dir) = start();
    app.wait_for_within("label:Send to Laptop", Duration::from_secs(5))
        .unwrap();
    app.answer_dialogs_myself();
    app.long_press("label:Send to Laptop").unwrap();
    app.wait_for("label:Forget").unwrap();
    app.press("label:Forget").unwrap();
    assert_eq!(
        app.pending_dialog().expect("an alert").title.as_deref(),
        Some("Forget Laptop?")
    );
    app.answer_dialog(DialogAnswer::Press("Cancel".into()))
        .unwrap();
    assert!(app.screen().id("label:Send to Laptop").is_ok());

    app.long_press("label:Send to Laptop").unwrap();
    app.wait_for("label:Forget").unwrap();
    app.press("label:Forget").unwrap();
    app.answer_dialog(DialogAnswer::Press("Forget".into()))
        .unwrap();
    app.wait_for_gone("label:Send to Laptop", Duration::from_secs(5))
        .unwrap();
}

/// The keyboard reaches what a long press does: Tab to a known device,
/// Shift+F10 opens its menu (Linux and the web; macOS's is VoiceOver's
/// "show menu"), Return on Forget asks (WAI-ARIA's context menu).
#[test]
fn shift_f10_opens_a_devices_menu() {
    let (mut app, _dir) = start_on(Os::Linux, "linux");
    app.wait_for_within("label:Send to Laptop", Duration::from_secs(5))
        .unwrap();
    app.answer_dialogs_myself();
    let mut opened = false;
    for _ in 0..40 {
        app.press_key("tab").unwrap();
        app.press_key("shift+f10").unwrap();
        if app.screen().id("label:Forget").is_ok() {
            opened = true;
            break;
        }
    }
    assert!(opened, "no device took the focus and opened its menu");
    app.press_key("enter").unwrap();
    assert_eq!(
        app.pending_dialog().expect("an alert").title.as_deref(),
        Some("Forget Laptop?")
    );
}

/// Clearing History asks first (it removes every entry, with no undo);
/// Cancel keeps them.
#[test]
fn clearing_history_asks_first() {
    let (mut app, _dir) = start();
    app.press_key("mod+2").unwrap();
    app.wait_for("label:Report.pdf").unwrap();
    app.answer_dialogs_myself();
    app.press("id:clear-history").unwrap();
    assert_eq!(
        app.pending_dialog().expect("an alert").title.as_deref(),
        Some("Clear History?")
    );
    app.answer_dialog(DialogAnswer::Press("Cancel".into()))
        .unwrap();
    app.settle();
    assert!(
        app.screen().id("label:Report.pdf").is_ok(),
        "cleared after Cancel"
    );
    app.press("id:clear-history").unwrap();
    app.answer_dialog(DialogAnswer::Press("Clear".into()))
        .unwrap();
    app.wait_for_gone("label:Report.pdf", Duration::from_secs(5))
        .unwrap();
}
