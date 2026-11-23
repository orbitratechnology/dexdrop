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
    registry.register::<App>();
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
