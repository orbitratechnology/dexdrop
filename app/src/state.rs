//! The app's state: what the engine last reported, and what the user is
//! doing. Pages read it through these functions; only the bridge writes the
//! engine's part.

use std::path::PathBuf;

use dexdrop_core::{
    PeerId,
    api::{IncomingRequest, InvitationView, NoticeKind, PeerInfo, Status, TransferSnapshot},
    store::{HistoryEntry, Settings, TrustedPeer},
};
use nsh_core::prelude::*;

/// The engine's view of the world.
#[derive(Clone, Default, PartialEq)]
pub struct Model {
    pub status: Option<Status>,
    pub settings: Settings,
    pub peers: Vec<PeerInfo>,
    /// Newest first.
    pub transfers: Vec<TransferSnapshot>,
    pub requests: Vec<IncomingRequest>,
    pub invitation: Option<InvitationView>,
    pub code: Option<(String, u64)>,
    pub trusted: Vec<TrustedPeer>,
    /// Why the engine is not running (the web preview, a startup error).
    pub offline: Option<String>,
}

pub fn model() -> Signal<Model> {
    global::<Model>()
}

/// The transfer history, newest first: a signal of its own, so what reads
/// it does not run again on every change of the model (a transfer's
/// progress, several times a second).
#[derive(Clone, Default, PartialEq)]
pub struct HistoryList(pub Vec<HistoryEntry>);

pub fn history() -> Signal<HistoryList> {
    global::<HistoryList>()
}

/// A section of the app (PRD 7.1): the system's tab bar on phones, the
/// sidebar on desktops.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Tab {
    #[default]
    Share,
    History,
    Settings,
}

impl Tab {
    pub const ALL: [Tab; 3] = [Tab::Share, Tab::History, Tab::Settings];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Share => "Share",
            Tab::History => "History",
            Tab::Settings => "Settings",
        }
    }

    fn index(self) -> usize {
        match self {
            Tab::Share => 0,
            Tab::History => 1,
            Tab::Settings => 2,
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq)]
struct CurrentTab(Tab);

pub fn tab() -> Tab {
    global::<CurrentTab>().get().0
}

/// The tab shown now, without subscribing.
/// Whether `route` is the page showing on `tab` and `tab` is the one
/// shown: keyboard shortcuts of a page act only then (visited tabs stay
/// mounted behind the one shown).
pub fn showing(tab: Tab, route: impl Fn(&Route) -> bool) -> bool {
    tab_untracked() == tab
        && global::<Stacks>().with_untracked(|s| s.0[tab.index()].last().is_some_and(&route))
        && model().with_untracked(|m| m.requests.is_empty())
}

pub fn tab_untracked() -> Tab {
    global::<CurrentTab>().get_untracked().0
}

/// Shows `tab`. Choosing the tab already shown goes back to its root, as
/// the platforms' tab bars do, and rebuilds nothing else.
pub fn set_tab(tab: Tab) {
    let current = global::<CurrentTab>();
    if current.get_untracked().0 == tab {
        truncate(tab, 1);
    } else {
        current.set(CurrentTab(tab));
    }
}

/// A page of a tab's navigation stack.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Route {
    /// The root of a tab.
    Root,
    /// Choosing what to send.
    Send,
    /// Choosing who to send it to.
    Devices,
    /// Sending with a code or an invitation.
    Code,
    /// Showing this device to others: radar, code and QR.
    Receive,
    /// A transfer's progress; `None`: the newest outgoing one.
    Transfer(Option<dexdrop_core::protocol::TransferId>),
    /// Settings: trusted devices.
    Trusted,
    /// Settings: this device's avatar.
    Avatar,
    /// Send: apps installed on this phone (Android).
    Apps,
    /// A video playing, or a photo shown, in the app.
    Viewer(std::path::PathBuf),
    /// History: a past transfer, by its hex ID.
    Past(String),
}

/// Each tab's pages, root first.
#[derive(Clone, Debug, PartialEq)]
struct Stacks([Vec<Route>; 3]);

impl Default for Stacks {
    fn default() -> Self {
        Stacks([vec![Route::Root], vec![Route::Root], vec![Route::Root]])
    }
}

/// The pages of `tab`, root first.
pub fn stack(tab: Tab) -> Vec<Route> {
    global::<Stacks>().with(|s| s.0[tab.index()].clone())
}

/// Opens `route` on top of the current tab.
pub fn push(route: Route) {
    let t = global::<CurrentTab>().get_untracked().0;
    global::<Stacks>().update(|s| s.0[t.index()].push(route));
}

/// Goes back one page on the current tab.
pub fn pop() {
    let t = global::<CurrentTab>().get_untracked().0;
    global::<Stacks>().update(|s| {
        let stack = &mut s.0[t.index()];
        if stack.len() > 1 {
            stack.pop();
        }
    });
}

/// Keeps the first `len` pages of `tab` (the system went back).
pub fn truncate(tab: Tab, len: usize) {
    global::<Stacks>().update(|s| s.0[tab.index()].truncate(len.max(1)));
}

/// Shows `route` right above the current tab's root (a flow that ends).
pub fn reset_to(route: Route) {
    let t = global::<CurrentTab>().get_untracked().0;
    global::<Stacks>().update(|s| {
        let stack = &mut s.0[t.index()];
        stack.truncate(1);
        if route != Route::Root {
            stack.push(route);
        }
    });
}

/// Switches to `tab` and shows `route` on top of its root.
pub fn open(tab: Tab, route: Route) {
    global::<Stacks>().update(|s| {
        let stack = &mut s.0[tab.index()];
        stack.truncate(1);
        if route != Route::Root {
            stack.push(route);
        }
    });
    set_tab(tab);
}

/// Files the user chose to send, before choosing a device.
#[derive(Clone, Default, PartialEq)]
pub struct Selection {
    pub paths: Vec<PathBuf>,
    /// `(files, folders, bytes)` once counted.
    pub summary: Option<(u64, u64, u64)>,
    /// Each path's `(is a folder, size)`, read off the engine thread.
    pub stats: std::collections::HashMap<PathBuf, (bool, u64)>,
}

pub fn selection() -> Signal<Selection> {
    global::<Selection>()
}

/// The overlay sheet open, if any.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Sheet {
    #[default]
    None,
    /// The diagnostics report.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Diagnostics(String),
}

pub fn sheet() -> Signal<Sheet> {
    global::<Sheet>()
}

/// Short-lived messages (PRD 7.5).
#[derive(Clone, Default, PartialEq)]
pub struct Toasts {
    pub next: u64,
    pub items: Vec<(u64, NoticeKind, String)>,
}

pub fn toasts() -> Signal<Toasts> {
    global::<Toasts>()
}

pub fn toast(kind: NoticeKind, text: impl Into<String>) {
    let text = text.into();
    let id = {
        let mut id = 0;
        toasts().update(|t| {
            id = t.next;
            t.next += 1;
            t.items.retain(|(_, _, old)| !supersedes(&text, old));
            t.items.push((id, kind, text));
            if t.items.len() > 3 {
                t.items.remove(0);
            }
        });
        id
    };
    set_timeout(std::time::Duration::from_secs(4), move || {
        toasts().update(|t| t.items.retain(|(i, ..)| *i != id));
    });
}

/// "Received X from Y." replaces a "Receiving X from Y." still on screen.
fn supersedes(new: &str, old: &str) -> bool {
    new.strip_prefix("Received ")
        .is_some_and(|rest| old.strip_prefix("Receiving ") == Some(rest))
}

#[cfg(test)]
mod toast_tests {
    use super::supersedes;

    #[test]
    fn received_replaces_receiving() {
        let old = "Receiving photo.bin and 1 more from Linux PC 5JTV.";
        assert!(supersedes(
            "Received photo.bin and 1 more from Linux PC 5JTV.",
            old
        ));
        assert!(!supersedes("Received notes.txt from Linux PC 5JTV.", old));
        assert!(!supersedes("Invitation copied.", old));
    }
}

/// The device picked before choosing files (a tap on a nearby device).
#[derive(Clone, Copy, Default, PartialEq)]
pub struct Picked(pub Option<PeerId>);

pub fn picked() -> Signal<Picked> {
    global::<Picked>()
}

/// Whether the window is wide enough for the desktop layout (PRD 7.3).
pub fn wide() -> bool {
    window().width >= if is_desktop() { px(720.0) } else { px(900.0) }
}

/// Whether this platform uses pointer-and-keyboard conventions.
pub fn desktop_platform() -> bool {
    matches!(os().name(), "macos" | "windows" | "linux")
}

/// The user-facing name of a platform key.
pub fn platform_label(key: &str) -> &'static str {
    match key {
        "android" => "Android",
        "ios" => "iOS",
        "macos" => "macOS",
        "windows" => "Windows",
        "linux" => "Linux",
        "web" => "Browser",
        _ => "Device",
    }
}
