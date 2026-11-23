//! DexDrop: fast, private, local file transfer between your devices.
//!
//! A Rust-only Native Shell app (no QuickJS). `nsh.json` names [`App`] as
//! the root; `nsh run` / `nsh package` build it for each platform, and
//! `cargo run -p dexdrop-app` starts it on this machine's desktop backend.
//!
//! The UI never owns sockets or files: it sends commands to the transfer
//! engine (`dexdrop-net`) and renders the events it sends back.

mod bridge;
mod files;
mod pages;
mod shell;
mod state;
mod ui;

use nsh_base::theme;
use nsh_core::prelude::*;

/// The app's UI set (`nsh.json` `"rust"`).
#[native_module("dexdrop/native")]
fn native(registry: &mut Registry) {
    // The app's icons (assets/), shipped with it.
    registry.assets(assets!());
    registry.register::<App>();
}

/// For `cargo run` on a computer: the app's assets laid out as packages
/// ship them (`@dexdrop-app/icons/…`), copied from the sources, for the
/// runner's asset folder. `nsh run` and `nsh package` do this themselves.
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
pub fn dev_assets() -> Option<std::path::PathBuf> {
    let dir = std::env::temp_dir().join("dexdrop-dev-assets");
    for file in assets!() {
        let to = dir.join(file.path());
        std::fs::create_dir_all(to.parent()?).ok()?;
        std::fs::copy(file.source, &to).ok()?;
    }
    Some(dir)
}

/// The app's root.
#[component]
pub fn App() -> impl IntoElement {
    // The theme of nsh.json, applied by the app itself so every runner
    // (`cargo run`, embedding hosts, tests) draws the same tokens.
    match nsh_base::tokens_of(Some(include_str!("../theme.json"))) {
        Ok(tokens) => set_tokens(tokens),
        Err(err) => log::warn!("theme: {err}"),
    }
    bridge::start();
    view()
        .flex_1()
        .bg(move || theme().background)
        .text_color(move || theme().foreground)
        .child(shell::shell())
}
