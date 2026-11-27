//! The layout the app promises, headless: Send and Receive within a
//! thumb's reach, where received files go, segmented choices.

use std::time::Duration;

use nsh_core::{element::Os, geometry::size, prelude::px};
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

fn as_mac(config: &mut EngineConfig) {
    config.viewport = size(px(1100.0), px(800.0));
    config.environment.window = config.viewport;
    config.environment.os = Os::MacOs;
    config.environment.platform = Some("macos");
}

fn as_android(config: &mut EngineConfig) {
    config.viewport = size(px(360.0), px(760.0));
    config.environment.window = config.viewport;
    config.environment.os = Os::Android;
    config.environment.platform = Some("android");
}

/// (x, y, width, height) of the one view `selector` matches.
fn frame(app: &mut Headless, selector: &str) -> (f32, f32, f32, f32) {
    let id = app
        .screen()
        .id(selector)
        .unwrap_or_else(|e| panic!("{selector}: {e}"));
    let b = app
        .screen()
        .layout_nodes()
        .into_iter()
        .find(|n| n.id == id)
        .expect("laid out")
        .bounds;
    (b.origin.x.0, b.origin.y.0, b.size.width.0, b.size.height.0)
}

/// On a phone (the default 390 × 844 iPhone), Send and Receive float at
/// the bottom, side by side, big enough for a thumb, clear of the tab bar
/// that floats over the last 96 pt.
#[test]
fn send_and_receive_are_within_a_thumbs_reach() {
    let (mut app, _dir) = start(|_| {});
    let send = frame(&mut app, "id:send");
    let receive = frame(&mut app, "id:receive");
    assert_eq!(send.1, receive.1, "side by side");
    for (name, f) in [("Send", send), ("Receive", receive)] {
        assert!(f.3 >= 56.0, "{name} is {} pt tall", f.3);
        assert!(f.2 >= 120.0, "{name} is {} pt wide", f.2);
        let bottom = f.1 + f.3;
        assert!(
            bottom > 844.0 - 200.0 && bottom <= 844.0 - 96.0,
            "{name} ends at {bottom} pt: not in the thumb's zone above the tab bar"
        );
    }
}

/// In a narrow window on a computer (Windows, 600 × 800) the tabs are drawn
/// along the bottom: Send and Receive sit above them, not under them (CI's
/// Windows run clicked Receive there and got Settings).
#[test]
fn send_and_receive_clear_the_drawn_tab_bar() {
    let (mut app, _dir) = start(|c| {
        c.viewport = size(px(600.0), px(800.0));
        c.environment.window = c.viewport;
        c.environment.os = Os::Windows;
        c.environment.platform = Some("windows");
    });
    let tabs = frame(&mut app, "label:History");
    for name in ["id:send", "id:receive"] {
        let f = frame(&mut app, name);
        assert!(
            f.1 + f.3 <= tabs.1,
            "{name} ends at {} pt, under the tabs at {} pt",
            f.1 + f.3,
            tabs.1
        );
    }
    app.press("id:receive").unwrap();
    app.wait_for_within("text:Ready to receive", Duration::from_secs(5))
        .unwrap();
}

/// Settings says where received files go: the Downloads folder by default
/// on a computer; on an iPhone, where the Files app shows them.
#[test]
fn settings_shows_where_files_go() {
    let (mut app, _dir) = start(as_mac);
    app.press_key("mod+3").unwrap();
    app.wait_for_within("id:save-to", Duration::from_secs(5))
        .unwrap();
    assert!(
        app.screen()
            .texts()
            .iter()
            .any(|t| t.ends_with("Downloads")),
        "no Downloads folder in {:?}",
        app.screen().texts()
    );

    let (mut app, _dir) = start(|_| {});
    app.press_key("mod+3").unwrap();
    app.wait_for_within(
        "text:Files › On My iPhone › DexDrop",
        Duration::from_secs(5),
    )
    .unwrap();
}

/// Outside iOS (no system control there), History's All / Sent / Received
/// is NSH's toggle group: one toolbar, the choice pressed.
#[test]
fn history_filter_is_a_toggle_group_outside_ios() {
    let (mut app, _dir) = start(as_android);
    app.press_key("mod+2").unwrap();
    app.wait_for_within("role:toolbar", Duration::from_secs(5))
        .unwrap();
    app.press("text:Sent").unwrap();
    app.wait_for("text:Sent").unwrap();
}

/// Home's header is the app's icon and name, centered, and so are
/// "Nearby" and the radar looking for devices: on an iPhone, an Android
/// phone and a Mac.
#[test]
fn home_header_and_nearby_are_centered() {
    type Setup = (&'static str, fn(&mut EngineConfig), f32);
    let setups: [Setup; 3] = [
        ("iPhone", |_| {}, 390.0),
        ("Android", as_android, 360.0),
        ("Mac", as_mac, 1100.0),
    ];
    for (name, env, width) in setups {
        let (mut app, _dir) = start(env);
        // The Mac's sidebar takes the left; the page is centered right of it.
        let page = if name == "Mac" {
            let p = frame(&mut app, "id:brand");
            (p.0, p.2)
        } else {
            (0.0, width)
        };
        let middle = page.0 + page.1 / 2.0;
        for selector in ["id:brand", "text:Nearby", "id:radar"] {
            let f = frame(&mut app, selector);
            let centre = f.0 + f.2 / 2.0;
            assert!(
                (centre - middle).abs() <= 2.0,
                "{name}: {selector} is centered at {centre} pt, the page at {middle} pt"
            );
        }
    }
}

/// With less motion asked for (CI's Android emulator turns animations off),
/// the radar keeps still circles around the Wi‑Fi mark instead of none.
#[test]
fn the_radar_keeps_still_circles_with_less_motion() {
    let (mut app, _dir) = start(|c| {
        as_android(c);
        c.environment.reduce_motion = true;
    });
    let radar = frame(&mut app, "id:radar");
    let centre = (radar.0 + radar.2 / 2.0, radar.1 + radar.3 / 2.0);
    let circles = app
        .screen()
        .layout_nodes()
        .into_iter()
        .filter(|n| {
            let b = n.bounds;
            let (w, h) = (b.size.width.0, b.size.height.0);
            let c = (b.origin.x.0 + w / 2.0, b.origin.y.0 + h / 2.0);
            (w - h).abs() < 0.5
                && w > 44.0
                && w < radar.2 - 0.5
                && (c.0 - centre.0).abs() < 1.0
                && (c.1 - centre.1).abs() < 1.0
        })
        .count();
    assert!(circles >= 2, "{circles} circles inside the radar");
}

/// On Android the tabs are the app's own, drawn in its colors: white in
/// light (Material's bar took the phone's wallpaper colors, brown on a
/// user's phone), with Send and Receive above them; a tab opens its page.
#[test]
fn android_tabs_are_drawn_white() {
    let (mut app, _dir) = start(as_android);
    let style = app
        .screen()
        .view_style("id:tab-bar")
        .unwrap()
        .expect("a drawn tab bar");
    assert_eq!(style.background_color, Some(0xffff_ffff), "not white");
    let bar = frame(&mut app, "id:tab-bar");
    for name in ["id:send", "id:receive"] {
        let f = frame(&mut app, name);
        assert!(f.1 + f.3 <= bar.1, "{name} is under the tabs");
    }
    app.press("label:Settings").unwrap();
    app.wait_for_within("id:save-to", Duration::from_secs(5))
        .unwrap();
}

/// Words that are centered or head a row take the room around them, not
/// only the width measured for them: on a phone whose system font is wider
/// than the one measured, "Nearby" showed as "Nearb" and the status pill
/// lost the device's name.
#[test]
fn headings_and_the_status_take_their_rows_width() {
    let (mut app, _dir) = start(as_android);
    // The status dot (drawn, not typed) and the words beside it.
    app.wait_for_within("text:/^Visible as /", Duration::from_secs(5))
        .unwrap();
    let nearby = frame(&mut app, "text:Nearby");
    let status = frame(&mut app, "text:/^Visible as /");
    for (name, f, least) in [("Nearby", nearby, 200.0), ("status", status, 200.0)] {
        assert!(f.2 >= least, "{name} is only {} pt wide", f.2);
    }
}

/// A phone's picker hands over copies of picked files (NSH's `nsh-picker`
/// in the app's cache): one taken out of the selection is deleted at
/// once, not left to fill the app's storage.
#[test]
fn a_picked_copy_goes_when_taken_out_of_the_selection() {
    let (mut app, dir) = start(as_android);
    let picker = dir.path().join("cache/nsh-picker/pick1");
    std::fs::create_dir_all(&picker).unwrap();
    let copy = picker.join("clip.mp4");
    std::fs::write(&copy, vec![5u8; 64 * 1024]).unwrap();
    app.on_pick_paths(vec![copy.clone()]).unwrap();
    app.press("id:send").unwrap();
    app.wait_for("id:pick-files").unwrap();
    app.press("id:pick-files").unwrap();
    app.wait_for_within("label:Remove clip.mp4", Duration::from_secs(5))
        .unwrap();
    // The app moves the picker's copy into its staging folder.
    let staged = find(dir.path(), "clip.mp4").expect("the copy, somewhere");
    app.press("label:Remove clip.mp4").unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while staged.exists() && std::time::Instant::now() < deadline {
        app.wait_for_within("id:pick-files", Duration::from_millis(100))
            .unwrap();
    }
    assert!(!staged.exists(), "the copy stayed after it was taken out");
}

/// The file named `name` under `dir`, if any.
fn find(dir: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        if p.is_dir() {
            if let Some(found) = find(&p, name) {
                return Some(found);
            }
        } else if e.file_name() == name {
            return Some(p);
        }
    }
    None
}

/// History's and Settings' titles are centered in their headers, bold
/// (as Home's), on an iPhone, an Android phone and a Mac.
#[test]
fn history_and_settings_titles_are_centered() {
    type Setup = (&'static str, fn(&mut EngineConfig), f32);
    let setups: [Setup; 3] = [
        ("iPhone", |_| {}, 390.0),
        ("Android", as_android, 360.0),
        ("Mac", as_mac, 1100.0),
    ];
    for (name, env, width) in setups {
        let (mut app, _dir) = start(env);
        // The page's middle: where Home's brand is centered (right of the
        // Mac's sidebar).
        let brand = frame(&mut app, "id:brand");
        let page_middle = brand.0 + brand.2 / 2.0;
        for (key, title) in [("mod+2", "History"), ("mod+3", "Settings")] {
            app.press_key(key).unwrap();
            app.wait_for_within("id:header-title", Duration::from_secs(5))
                .unwrap();
            let f = frame(&mut app, "id:header-title");
            let middle = if name == "Mac" {
                page_middle
            } else {
                width / 2.0
            };
            let centre = f.0 + f.2 / 2.0;
            assert!(
                (centre - middle).abs() <= 2.0,
                "{name}: {title} is centered at {centre} pt, not {middle} pt"
            );
        }
    }
}

/// Empty states and the first launch show the app's illustrations: an
/// empty History, no trusted devices, a computer's drop area on Send, and
/// on first launch the picture of devices finding each other.
#[test]
fn empty_states_show_illustrations() {
    let (mut app, _dir) = start(as_mac);
    app.press_key("mod+2").unwrap();
    app.wait_for_within("id:art-history", Duration::from_secs(5))
        .unwrap();
    app.press_key("mod+3").unwrap();
    app.wait_for_within("text:Trusted devices", Duration::from_secs(5))
        .unwrap();
    app.press("text:Trusted devices").unwrap();
    app.wait_for_within("id:art-trusted", Duration::from_secs(5))
        .unwrap();
    app.press_key("mod+1").unwrap();
    app.press("id:send").unwrap();
    app.wait_for_within("id:art-drop", Duration::from_secs(5))
        .unwrap();

    // First launch on a Mac: the local network explained, with its picture.
    let dir = tempfile::tempdir().unwrap();
    let mut config = EngineConfig::new(Root::of::<dexdrop_app::App>());
    config.data_dir = Some(dir.path().to_path_buf());
    as_mac(&mut config);
    let mut first = Headless::start(config).unwrap();
    first
        .wait_for_within("id:art-devices", Duration::from_secs(10))
        .unwrap();
    assert!(first.screen().id("id:local-network-continue").is_ok());
}
