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
    pub history: Vec<HistoryEntry>,
    /// Why the engine is not running (the web preview, a startup error).
    pub offline: Option<String>,
}

pub fn model() -> Signal<Model> {
    global::<Model>()
}

/// A section of the app (PRD 7.1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Nearby,
    Transfers,
    Settings,
}

impl Tab {
    pub const ALL: [Tab; 3] = [Tab::Nearby, Tab::Transfers, Tab::Settings];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Nearby => "Nearby",
            Tab::Transfers => "Transfers",
            Tab::Settings => "Settings",
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq)]
struct CurrentTab(Tab);

pub fn tab() -> Tab {
    global::<CurrentTab>().get().0
}

pub fn set_tab(tab: Tab) {
    global::<CurrentTab>().set(CurrentTab(tab));
}

/// Files the user chose to send, before choosing a device.
#[derive(Clone, Default, PartialEq)]
pub struct Selection {
    pub paths: Vec<PathBuf>,
    /// `(files, folders, bytes)` once counted.
    pub summary: Option<(u64, u64, u64)>,
}

pub fn selection() -> Signal<Selection> {
    global::<Selection>()
}

/// The overlay sheet open, if any.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Sheet {
    #[default]
    None,
    /// Show this device's QR invitation and code.
    Receive,
    /// Send with a code or an invitation.
    Connect,
    /// The diagnostics report.
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

/// The device picked on the Nearby screen.
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
