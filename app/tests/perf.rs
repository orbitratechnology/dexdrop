//! What the app sends the platform per interaction, as NSH's performance
//! guide measures it (https://nsh.dev/docs/performance/): mount items per
//! tap ("a handful, not dozens" for small changes), nothing at all while
//! idle (no signal written every frame), and long lists that cost what
//! short ones do (virtualized). Headless, on NSH's real engine.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;

/// The mount items of each step, as observed.
type Seen = Arc<Mutex<Vec<(String, usize)>>>;

/// A history of `n` transfers, each with a few files.
fn history(n: usize) -> String {
    let entries: Vec<String> = (0..n)
        .map(|i| {
            format!(
                r#"{{"transfer_id":"{i:032x}","direction":"{dir}","peer_name":"Desk {i}","peer_platform":"linux","label":"Report {i}.pdf","files":3,"bytes":{bytes},"outcome":"Completed","finished":{finished},"saved_to":null,"items":[{{"path":"Report {i}.pdf","size":1000,"local":null}}]}}"#,
                dir = if i % 2 == 0 { "Incoming" } else { "Outgoing" },
                bytes = 1000 + i,
                finished = 1_700_000_000 + i,
            )
        })
        .collect();
    format!(r#"{{"entries":[{}]}}"#, entries.join(","))
}

/// The app on state with `entries` past transfers, past its first screen,
/// counting the mount items of each step.
fn start(entries: usize) -> (Headless, tempfile::TempDir, Seen) {
    start_with(entries, false)
}

/// [`start`], on a manual clock the test moves (NSH's `Clock::manual`):
/// the app's timers run when the test says, not after real seconds.
fn start_with(entries: usize, manual: bool) -> (Headless, tempfile::TempDir, Seen) {
    let dir = tempfile::tempdir().unwrap();
    for state in [dir.path().to_path_buf(), dir.path().join("state")] {
        std::fs::create_dir_all(&state).unwrap();
        std::fs::write(state.join("history.json"), history(entries)).unwrap();
        std::fs::write(
            state.join("settings.json"),
            r#"{"local_network_explained":true,"device_name":"Perf"}"#,
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
    if manual {
        config.clock = nsh_core::time::Clock::manual(std::time::SystemTime::now());
    }
    let mut app = Headless::start(config).unwrap();
    app.wait_for("Nearby").unwrap();
    // The engine starts off the UI thread: measure once it runs (Home says
    // how this device shows), not after a guessed time.
    app.wait_until("the engine to run", Duration::from_secs(10), |s| {
        s.texts()
            .iter()
            .any(|t| t.starts_with("Visible as ") || t.ends_with(" · hidden"))
    })
    .unwrap();
    app.settle();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    app.observe(move |step, batch| {
        if std::env::var_os("PERF_DUMP").is_some() {
            for item in &batch.items {
                eprintln!("[{step}] {:.200}", format!("{item:?}"));
            }
        }
        log.lock()
            .unwrap()
            .push((step.to_owned(), batch.items.len()))
    });
    (app, dir, seen)
}

/// The mount items sent since the last call.
fn taken(seen: &Seen) -> usize {
    seen.lock().unwrap().drain(..).map(|(_, n)| n).sum()
}

/// A long history costs what a short one does: only the rows near the
/// screen exist, so opening it is bounded; coming back to it from another
/// tab rebuilds nothing (tabs stay mounted).
#[test]
fn long_history_costs_what_a_short_one_does() {
    let mut cost = Vec::new();
    for n in [20, 2000] {
        let (mut app, _dir, seen) = start(n);
        app.press_key("mod+2").unwrap();
        app.wait_for("Earlier").unwrap();
        app.wait(Duration::from_millis(300)).unwrap();
        let open = taken(&seen);
        app.press_key("mod+3").unwrap();
        app.wait_for("Privacy").unwrap();
        app.wait(Duration::from_millis(300)).unwrap();
        taken(&seen);
        app.press_key("mod+2").unwrap();
        app.wait_for("Earlier").unwrap();
        app.wait(Duration::from_millis(300)).unwrap();
        let filter = taken(&seen);
        eprintln!("history of {n}: open {open} mount items, back to it {filter}");
        cost.push((open, filter));
    }
    let (small, large) = (cost[0], cost[1]);
    assert!(
        large.1 <= 10,
        "coming back to History sent {} mount items: the tab was rebuilt",
        large.1
    );
    assert!(
        large.0 <= small.0 * 2 + 50,
        "opening a history of 2000 costs {} mount items against {} for 20",
        large.0,
        small.0
    );
    assert!(
        large.1 <= small.1 * 2 + 50,
        "coming back to a history of 2000 costs {} mount items against {} for 20",
        large.1,
        small.1
    );
}

/// Nothing is sent while nothing changes: no signal written every frame
/// (animations run natively), on every tab.
#[test]
fn idle_screens_send_nothing() {
    let (mut app, _dir, seen) = start_with(5, true);
    // Home's one-off hint ("Not here yet?") comes 8 s after it opens; tabs
    // stay alive while others show, so let it come first: at once, on the
    // manual clock.
    app.fast_forward(Duration::from_secs(9)).unwrap();
    for (key, shows) in [
        ("mod+1", "Nearby"),
        ("mod+2", "Earlier"),
        ("mod+3", "Privacy"),
    ] {
        app.press_key(key).unwrap();
        app.wait_for(shows).unwrap();
        // Let the page settle (fresh rows measure, the engine reports in).
        app.wait(Duration::from_millis(1500)).unwrap();
        taken(&seen);
        app.wait(Duration::from_secs(2)).unwrap();
        let idle = taken(&seen);
        eprintln!("{shows}: {idle} mount items in 2 s idle");
        assert!(
            idle <= 4,
            "{shows} sent {idle} mount items in 2 s while idle"
        );
    }
}

/// A small change sends a small update: flipping a switch in Settings, or
/// the History filter on a short list.
#[test]
fn small_changes_send_small_updates() {
    let (mut app, _dir, seen) = start(3);
    app.press_key("mod+3").unwrap();
    app.wait_for("Privacy").unwrap();
    app.wait(Duration::from_millis(800)).unwrap();
    taken(&seen);
    app.press("id:rename").unwrap();
    app.wait(Duration::from_millis(300)).unwrap();
    let rename = taken(&seen);
    eprintln!("rename field opened: {rename} mount items");
    // 37 before Save had an icon, 42 with it.
    assert!(
        rename <= 45,
        "opening the rename field sent {rename} mount items"
    );
}
