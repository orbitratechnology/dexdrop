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
    config.environment.os = Os::MacOs;
    config.environment.platform = Some("macos");
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

/// A past transfer held (or right-clicked) is removed after asking; its
/// files are not touched.
#[test]
fn long_press_removes_a_past_transfer() {
    let (mut app, _dir) = start();
    app.press_key("mod+2").unwrap();
    app.wait_for("label:Report.pdf").unwrap();
    app.answer_dialogs_myself();
    app.long_press("label:Report.pdf").unwrap();
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

/// A device in Your devices held is forgotten after asking; Cancel keeps
/// it.
#[test]
fn long_press_forgets_a_device_after_asking() {
    let (mut app, _dir) = start();
    app.wait_for_within("label:Send to Laptop", Duration::from_secs(5))
        .unwrap();
    app.answer_dialogs_myself();
    app.long_press("label:Send to Laptop").unwrap();
    assert_eq!(
        app.pending_dialog().expect("an alert").title.as_deref(),
        Some("Forget Laptop?")
    );
    app.answer_dialog(DialogAnswer::Press("Cancel".into()))
        .unwrap();
    assert!(app.screen().id("label:Send to Laptop").is_ok());

    app.long_press("label:Send to Laptop").unwrap();
    app.answer_dialog(DialogAnswer::Press("Forget".into()))
        .unwrap();
    app.wait_for_gone("label:Send to Laptop", Duration::from_secs(5))
        .unwrap();
}
