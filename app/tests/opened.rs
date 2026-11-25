//! Files the system hands DexDrop (`nsh.json` `fileTypes`), headless:
//! shared to it from another app on Android, opened with it or dropped on
//! its icon on a computer. NSH reports them as `file://` links, which the
//! app takes once it declares file types (the `file` scheme here, as `nsh`
//! sets it from `fileTypes`).

use std::time::Duration;

use nsh_core::{geometry::size, platform::Os, prelude::px};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;

/// The app's config on Android, with two files in `dir` to share.
fn config(dir: &tempfile::TempDir) -> EngineConfig {
    std::fs::write(dir.path().join("Report.pdf"), vec![1u8; 2048]).unwrap();
    std::fs::write(dir.path().join("Notes.txt"), b"hello").unwrap();
    // SAFETY: every test sets the same values before starting the app.
    unsafe {
        std::env::set_var("DEXDROP_NO_DISCOVERY", "1");
        std::env::set_var("DEXDROP_KEY_STORE", "file");
    }
    let root = Root::of::<dexdrop_app::App>().url_schemes(["file"]);
    let mut config = EngineConfig::new(root);
    config.data_dir = Some(dir.path().join("data"));
    config.viewport = size(px(390.0), px(844.0));
    config.environment.window = config.viewport;
    config.environment.os = Os::Android;
    config.environment.platform = Some("android");
    config
}

fn file_url(dir: &tempfile::TempDir, name: &str) -> String {
    format!("file://{}", dir.path().join(name).display())
}

/// Waits until the Send page shows `name` in its selection, ready to go
/// on to a device.
fn shows_selected(app: &mut Headless, name: &str) {
    app.wait_for_within(&format!("text:{name}"), Duration::from_secs(5))
        .unwrap();
    app.wait_for_within("id:next", Duration::from_secs(5))
        .unwrap();
}

/// Shared to DexDrop while it runs: the files join the selection and Send
/// shows them, wherever the user was.
#[test]
fn shared_files_open_send_with_them_selected() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = Headless::start(config(&dir)).unwrap();
    app.wait_for("Nearby").unwrap();
    // Somewhere else first: History.
    app.press_key("mod+2").unwrap();
    app.open_with_url(&file_url(&dir, "Report.pdf")).unwrap();
    shows_selected(&mut app, "Report.pdf");
    // A second share adds to what is picked.
    app.open_with_url(&file_url(&dir, "Notes.txt")).unwrap();
    shows_selected(&mut app, "Notes.txt");
    assert!(
        app.screen().texts().contains(&"Report.pdf"),
        "the first share stays picked: {:?}",
        app.screen().texts()
    );
}

/// Launched by a share: the file is waiting on Send at the first screen.
#[test]
fn a_share_that_launches_the_app_lands_on_send() {
    let dir = tempfile::tempdir().unwrap();
    let url = file_url(&dir, "Report.pdf");
    let mut app = Headless::start_with_link(config(&dir), &url).unwrap();
    shows_selected(&mut app, "Report.pdf");
}
