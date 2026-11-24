//! The device's settings, headless (NSH's "Device and environment"): the
//! user's clock and text size, set in `EngineConfig::environment` as a
//! platform reports them.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use nsh_core::platform::HourCycle;
use nsh_engine::{EngineConfig, Environment, Root};
use nsh_testing::Headless;

/// The app with one transfer finished a minute ago, past its first
/// screen, its environment set by `env`.
fn start(env: impl FnOnce(&mut Environment)) -> (Headless, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let finished = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        - 60;
    let history = format!(
        r#"{{"entries":[{{"transfer_id":"{:032x}","direction":"Incoming","peer_name":"Desk","peer_platform":"linux","label":"Report.pdf","files":1,"bytes":1000,"outcome":"Completed","finished":{finished},"saved_to":null,"items":[{{"path":"Report.pdf","size":1000,"local":null}}]}}]}}"#,
        1
    );
    for state in [dir.path().to_path_buf(), dir.path().join("state")] {
        std::fs::create_dir_all(&state).unwrap();
        std::fs::write(state.join("history.json"), &history).unwrap();
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
    env(&mut config.environment);
    let mut app = Headless::start(config).unwrap();
    app.wait_for("Nearby").unwrap();
    (app, dir)
}

/// The time a transfer finished, on its detail page.
fn finished_time(app: &mut Headless) -> String {
    app.press_key("mod+2").unwrap();
    app.wait_for("Report.pdf").unwrap();
    app.press("label:Report.pdf").unwrap();
    app.wait_for_within("text:/^Today, /", Duration::from_secs(5))
        .unwrap();
    let screen = app.screen();
    screen
        .texts()
        .into_iter()
        .find(|t| t.starts_with("Today, "))
        .expect("a time today")
        .to_owned()
}

/// History tells the time on the user's clock: 2:05 PM where the device
/// uses 12 hours, 14:05 where it uses 24.
#[test]
fn times_follow_the_users_clock() {
    let (mut app, _dir) = start(|e| e.locale_settings.hour_cycle = HourCycle::H12);
    let twelve = finished_time(&mut app);
    assert!(
        twelve.ends_with(" AM") || twelve.ends_with(" PM"),
        "12-hour clock: {twelve}"
    );
    let (mut app, _dir) = start(|e| e.locale_settings.hour_cycle = HourCycle::H23);
    let twenty_four = finished_time(&mut app);
    assert!(!twenty_four.contains('M'), "24-hour clock: {twenty_four}");
}

/// The frame of the one view `selector` matches: (x, y).
fn origin(app: &mut Headless, selector: &str) -> (f32, f32) {
    let id = app.screen().id(selector).unwrap();
    let node = app
        .screen()
        .layout_nodes()
        .into_iter()
        .find(|n| n.id == id)
        .expect("laid out");
    (node.bounds.origin.x.0, node.bounds.origin.y.0)
}

/// With large system text, Home's Send and Receive go one above the
/// other, so their labels fit; at the default size they sit side by side.
#[test]
fn large_text_stacks_home_actions() {
    let (mut app, _dir) = start(|_| {});
    let (send, receive) = (origin(&mut app, "id:send"), origin(&mut app, "id:receive"));
    assert_eq!(send.1, receive.1, "side by side at the default size");

    app.set_environment("larger text", |e| e.font_scale = 1.6)
        .unwrap();
    app.wait(Duration::from_millis(300)).unwrap();
    let (send, receive) = (origin(&mut app, "id:send"), origin(&mut app, "id:receive"));
    assert!(
        receive.1 > send.1,
        "stacked with large text: {send:?} {receive:?}"
    );
}
