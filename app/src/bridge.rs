//! The bridge between the UI and the transfer engine.
//!
//! The engine runs on its own thread and Tokio runtime (`dexdrop-net`); the
//! UI sends it commands and a task on the UI's engine thread applies the
//! events it sends back to [`crate::state::model`]. No socket or file stream
//! is ever owned by UI code (PRD 9.1).

use dexdrop_core::api::{Command, Event, NoticeKind};
use nsh_core::prelude::*;

use crate::state::{model, toast};

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::{path::PathBuf, rc::Rc};

    use dexdrop_net::{Engine, EngineConfig, EngineHandle};
    use nsh_core::prelude::*;

    /// The running engine, kept for the life of the surface.
    #[derive(Clone, Default)]
    pub struct Slot(pub Option<Rc<Engine>>);

    pub fn handle() -> Option<EngineHandle> {
        global::<Slot>().with_untracked(|s| s.0.as_ref().map(|e| e.handle()))
    }

    fn home() -> Option<PathBuf> {
        std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
    }

    /// Where the engine keeps its state, and where files are received by
    /// default: the user's Downloads folder on desktops, app-private storage
    /// on phones (exported from there with the share sheet).
    pub fn dirs(platform: &str) -> (PathBuf, PathBuf) {
        let base = std::env::var_os("DEXDROP_STATE_DIR")
            .map(PathBuf::from)
            .or_else(data_dir)
            .unwrap_or_else(|| std::env::temp_dir().join("dexdrop"));
        let downloads = match (platform, home()) {
            ("macos" | "windows" | "linux", Some(home)) => home.join("Downloads").join("DexDrop"),
            _ => base.join("Received"),
        };
        let downloads = std::env::var_os("DEXDROP_DOWNLOAD_DIR")
            .map(PathBuf::from)
            .unwrap_or(downloads);
        (base.join("state"), downloads)
    }

    pub fn start() -> Result<dexdrop_net::Events, String> {
        let platform = os().name();
        let (state, downloads) = dirs(platform);
        let mut config = EngineConfig::new(state, downloads, platform);
        config.discovery = std::env::var_os("DEXDROP_NO_DISCOVERY").is_none();
        if let Some(port) = std::env::var("DEXDROP_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
        {
            config.port = Some(port);
        }
        let (engine, events) = Engine::start(config).map_err(|e| e.to_string())?;
        global::<Slot>().set(Slot(Some(Rc::new(engine))));
        Ok(events)
    }
}

/// Starts the engine and applies its events to the model.
pub fn start() {
    #[cfg(not(target_arch = "wasm32"))]
    match native::start() {
        Ok(mut events) => {
            spawn(async move {
                while let Some(event) = events.recv().await {
                    apply(event);
                }
            });
        }
        Err(err) => {
            model().update(|m| {
                m.offline = Some(format!("The transfer service could not start: {err}"))
            });
        }
    }
    #[cfg(target_arch = "wasm32")]
    model().update(|m| {
        m.offline = Some(
            "This is the browser preview. Browsers cannot discover nearby devices or open direct \
             connections, so transfers need the DexDrop app on Android, Windows, macOS or Linux."
                .into(),
        )
    });
}

/// Runs `f` off the UI's engine thread where there are threads, inline in
/// the browser. `None` if the work was cancelled.
pub async fn off_thread<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        blocking(f).await.ok()
    }
    #[cfg(target_arch = "wasm32")]
    {
        Some(f())
    }
}

/// Sends a command to the engine.
pub fn send(command: Command) {
    #[cfg(not(target_arch = "wasm32"))]
    match native::handle() {
        Some(engine) => engine.send(command),
        None => toast(NoticeKind::Error, "The transfer service is not running."),
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = command;
        toast(NoticeKind::Info, "Transfers need the DexDrop app.");
    }
}

/// Where received files go by default on this platform.
#[cfg(not(target_arch = "wasm32"))]
pub fn default_download_dir() -> std::path::PathBuf {
    native::dirs(os().name()).1
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn apply(event: Event) {
    match event {
        Event::Status(status) => model().update(|m| m.status = Some(status)),
        Event::Settings(settings) => model().update(|m| m.settings = settings),
        Event::Peers(peers) => model().update(|m| m.peers = peers),
        Event::Transfer(t) => {
            let was_finished = model().with_untracked(|m| {
                m.transfers
                    .iter()
                    .find(|x| x.id == t.id)
                    .map(|x| x.state.is_finished())
            });
            // A system notification when a transfer ends (P4-006), useful
            // while the window is in the background; not for transfers
            // already finished when the app started.
            if was_finished == Some(false) && t.state.is_finished() {
                notify_finished(&t);
            }
            model().update(|m| match m.transfers.iter_mut().find(|x| x.id == t.id) {
                Some(slot) => *slot = t,
                None => m.transfers.insert(0, t),
            })
        }
        Event::TransferRemoved(id) => model().update(|m| m.transfers.retain(|t| t.id != id)),
        Event::Request(request) => {
            let text = format!("{} wants to send you {}.", request.peer_name, request.label);
            model().update(|m| {
                m.requests.retain(|r| r.id != request.id);
                m.requests.push(request);
            });
            // A system notification, for when the window is in the background.
            // The notification is posted now; the answer (whether the system
            // showed it) is not needed.
            drop(show_notification(
                nsh_core::notifications::Notification::new(
                    "dexdrop-request",
                    "Incoming transfer",
                    text,
                ),
            ));
        }
        Event::RequestClosed(id) => model().update(|m| m.requests.retain(|r| r.id != id)),
        Event::Invitation(view) => model().update(|m| m.invitation = Some(view)),
        Event::Code { code, expires } => model().update(|m| m.code = Some((code, expires))),
        Event::PairingUsed { code, by } => {
            model().update(|m| {
                if code {
                    m.code = None;
                } else {
                    m.invitation = None;
                }
            });
            let what = if code { "code" } else { "invitation" };
            toast(
                NoticeKind::Info,
                format!("{by} used your {what}. You'll be asked before anything is saved."),
            );
        }
        Event::InvitationsClosed => model().update(|m| {
            m.invitation = None;
            m.code = None;
        }),
        Event::Trusted(trusted) => model().update(|m| m.trusted = trusted),
        Event::History(history) => model().update(|m| m.history = history),
        Event::Notice { kind, text } => toast(kind, text),
        Event::Diagnostics(report) => {
            crate::state::sheet().set(crate::state::Sheet::Diagnostics(report))
        }
        Event::Stopped => {
            model().update(|m| m.offline = Some("The transfer service stopped.".into()))
        }
    }
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn notify_finished(t: &dexdrop_core::api::TransferSnapshot) {
    use dexdrop_core::{api::TransferState, store::Direction};
    let (verb, prep) = match t.direction {
        Direction::Outgoing => ("Sent", "to"),
        Direction::Incoming => ("Received", "from"),
    };
    let (title, body) = match &t.state {
        TransferState::Completed => (
            format!("{verb} {}", t.label),
            format!("{prep} {}, verified.", t.peer_name),
        ),
        TransferState::CompletedWithIssues => (
            format!("{verb} {} with issues", t.label),
            format!("{} failed, {} skipped.", t.files_failed, t.files_skipped),
        ),
        TransferState::Cancelled { .. } => return,
        other => (format!("Transfer of {} stopped", t.label), other.label()),
    };
    // The notification is posted now; the answer (whether the system
    // showed it) is not needed.
    drop(show_notification(
        nsh_core::notifications::Notification::new(
            format!("dexdrop-{}", dexdrop_core::hex(&t.id)),
            title,
            body,
        ),
    ));
}
