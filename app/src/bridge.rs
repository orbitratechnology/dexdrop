//! The bridge between the UI and the transfer engine.
//!
//! The engine runs on its own thread and Tokio runtime (`dexdrop-net`); the
//! UI sends it commands and a task on the UI's engine thread applies the
//! events it sends back to [`crate::state::model`]. No socket or file stream
//! is ever owned by UI code (PRD 9.1).

use dexdrop_core::api::{Command, Event, NoticeKind};
use nsh_core::prelude::*;

use crate::state::{engine, toast};

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
    /// default: the system's Downloads folder (computers: `~/Downloads`;
    /// Android: the shared `Download` folder; iOS: the app's Documents,
    /// which the Files app shows under On My iPhone › DexDrop). App
    /// storage only where none of those can be written.
    pub fn dirs(platform: &str) -> (PathBuf, PathBuf) {
        let base = std::env::var_os("DEXDROP_STATE_DIR")
            .map(PathBuf::from)
            .or_else(data_dir)
            .unwrap_or_else(|| std::env::temp_dir().join("dexdrop"));
        let fallback = base.join("Received");
        let downloads = match (platform, home()) {
            ("macos" | "windows" | "linux", Some(home)) => home.join("Downloads"),
            // Only on the device itself: headless tests run as a phone on
            // a computer, and must not write into its home.
            ("android", _) if cfg!(target_os = "android") => {
                android_downloads().unwrap_or(fallback)
            }
            ("ios", Some(home)) if cfg!(target_os = "ios") => home.join("Documents"),
            _ => fallback,
        };
        let downloads = std::env::var_os("DEXDROP_DOWNLOAD_DIR")
            .map(PathBuf::from)
            .unwrap_or(downloads);
        (base.join("state"), downloads)
    }

    /// Android's shared Download folder, when this app can write there:
    /// from Android 11 an app may create files in it without a permission
    /// (they are its own); older versions need one DexDrop does not ask
    /// for, so received files stay in app storage there. Tried once.
    fn android_downloads() -> Option<PathBuf> {
        static WRITABLE: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
        WRITABLE
            .get_or_init(|| {
                let dir = PathBuf::from("/storage/emulated/0/Download");
                let probe = dir.join(".dexdrop-write-test");
                let ok = std::fs::create_dir_all(&dir).is_ok()
                    && std::fs::write(&probe, b"").is_ok()
                    && std::fs::remove_file(&probe).is_ok();
                ok.then_some(dir)
            })
            .clone()
    }

    /// The engine's configuration on this platform, and the providers of
    /// its media registered (on the UI thread, where `os()` is known).
    pub fn config() -> EngineConfig {
        // NSH has put the Java VM and the application Context in
        // ndk-context before this code runs: discovery, internet transfers,
        // the apps list and the foreground service reach Java through it.
        #[cfg(target_os = "android")]
        dexdrop_net::use_android_context();
        let platform = os().name();
        let (state, downloads) = dirs(platform);
        // The device's media and documents, sent from where they are
        // (docs/FILE_SELECTION.md); their thumbnails in the state's cache.
        dexdrop_net::assets::register_platform(&state.join("cache"));
        #[cfg(target_os = "ios")]
        dexdrop_photos::register(&state.join("cache"));
        let mut config = EngineConfig::new(state, downloads, platform);
        config.discovery = std::env::var_os("DEXDROP_NO_DISCOVERY").is_none();
        // iOS and macOS ask before an app uses the local network: say why
        // first (FR-004).
        config.explain_local_network_first = matches!(platform, "ios" | "macos");
        // Transfers over the internet (Phase 6), when Settings allow them:
        // n0's public relays unless DEXDROP_RELAY_URL names the project's own.
        config.remote = Some(dexdrop_net::remote::RemoteConfig {
            relay: match std::env::var("DEXDROP_RELAY_URL") {
                Ok(url) if !url.trim().is_empty() => {
                    dexdrop_net::remote::Relay::Own(url.trim().to_owned())
                }
                _ => dexdrop_net::remote::Relay::Public,
            },
            ..Default::default()
        });
        // The direct link (docs/CONNECTIVITY.md 4.3): the platform's where it
        // has one (Android), or the loopback stand-in for headless tests.
        config.direct_link = if std::env::var("DEXDROP_DIRECT_LINK").as_deref() == Ok("loopback") {
            Some(std::sync::Arc::new(dexdrop_net::direct::Loopback::default()))
        } else {
            dexdrop_net::direct::platform()
        };
        if let Some(port) = std::env::var("DEXDROP_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
        {
            config.port = Some(port);
        }
        config
    }

    /// Starts the engine off the UI thread (it reads its identity from the
    /// platform's key store and its state from disk) and keeps it.
    pub async fn start() -> Result<dexdrop_net::Events, String> {
        let platform = os().name();
        let config = config();
        let started = crate::bridge::off_thread(move || Engine::start(config))
            .await
            .ok_or_else(|| "cancelled".to_owned())?;
        let (engine, events) = started.map_err(|e| e.to_string())?;
        global::<Slot>().set(Slot(Some(Rc::new(engine))));
        if matches!(platform, "android" | "ios") {
            move_old_received(platform);
        }
        Ok(events)
    }

    /// Stops the running engine, off the UI thread: stopping waits for its
    /// thread to finish.
    pub async fn stop() {
        let engine = global::<Slot>().with_untracked(|s| s.0.clone());
        global::<Slot>().set(Slot(None));
        if let Some(engine) = engine.and_then(|e| Rc::try_unwrap(e).ok()) {
            crate::bridge::off_thread(move || drop(engine)).await;
        }
    }

    /// Earlier versions kept received files in the app's own storage on
    /// phones (counted as the app's data). Now they go to the system's
    /// Downloads: move what is still there, once, in the background.
    fn move_old_received(platform: &'static str) {
        let base = std::env::var_os("DEXDROP_STATE_DIR")
            .map(PathBuf::from)
            .or_else(data_dir)
            .unwrap_or_else(|| std::env::temp_dir().join("dexdrop"));
        let old = base.join("Received");
        let new = dirs(platform).1;
        if new == old || !old.is_dir() {
            return;
        }
        let moving = blocking(move || dexdrop_core::fsutil::move_tree(&old, &new));
        spawn(async move {
            if let Ok(Ok(n)) = moving.await
                && n > 0
            {
                let place = if platform == "ios" {
                    "the Files app"
                } else {
                    "Downloads"
                };
                crate::state::toast(
                    dexdrop_core::api::NoticeKind::Info,
                    if n == 1 {
                        format!("Moved 1 received file to {place}.")
                    } else {
                        format!("Moved {n} received files to {place}.")
                    },
                );
            }
        });
    }
}

/// Which engine's events count: each start gets the next number, so a
/// stopped engine's last events cannot overwrite its successor's state.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
#[derive(Clone, Copy, Default, PartialEq)]
struct Generation(u64);

/// Commands sent while the engine starts, sent to it once it runs.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Default)]
struct Starting(Option<Vec<Command>>);

/// Starts the engine (off the UI thread) and applies its events to the
/// model. Until it runs, the model has no status ("Starting…") and
/// commands wait for it.
pub fn start() {
    #[cfg(not(target_arch = "wasm32"))]
    global::<Starting>().set(Starting(Some(Vec::new())));
    #[cfg(not(target_arch = "wasm32"))]
    spawn(async {
        let started = native::start().await;
        let waiting = global::<Starting>().with_untracked(|s| s.0.clone().unwrap_or_default());
        global::<Starting>().set(Starting(None));
        match started {
            Ok(mut events) => {
                for command in waiting {
                    send(command);
                }
                let generation = global::<Generation>().with_untracked(|g| g.0 + 1);
                global::<Generation>().set(Generation(generation));
                engine::offline().set(engine::Offline(None));
                while let Some(event) = events.recv().await {
                    if global::<Generation>().with_untracked(|g| g.0) != generation {
                        break;
                    }
                    apply(event);
                }
            }
            Err(err) => {
                engine::offline().set(engine::Offline(Some(format!(
                    "The transfer service could not start: {err}"
                ))));
            }
        }
    });
    #[cfg(target_arch = "wasm32")]
    engine::offline().set(engine::Offline(Some(
        "This is the browser preview. Browsers cannot discover nearby devices or open direct \
         connections, so transfers need the DexDrop app on Android, Windows, macOS or Linux."
            .into(),
    )));
}

/// Gives this device a new identity (PRD 9.9): stops the engine, replaces
/// the identity (the old key leaves the credential store) and starts the
/// engine again with the new peer ID. Callers check that nothing is
/// transferring.
#[cfg(not(target_arch = "wasm32"))]
pub fn reset_identity() {
    let (state, _) = native::dirs(os().name());
    let path = dexdrop_core::store::StateDir::new(state).identity();
    restart(move || {
        let rotated = dexdrop_core::identity::Identity::rotate(
            &path,
            dexdrop_core::keystore::KeyStorage::platform_default(),
        )
        .map(|_| ())
        .map_err(|e| e.to_string());
        match rotated {
            Ok(()) => Some((
                NoticeKind::Success,
                "This device has a new identity. Other devices see it as a new device.".to_owned(),
            )),
            Err(err) => Some((
                NoticeKind::Error,
                format!("The identity could not be reset: {err}"),
            )),
        }
    });
}

/// Turns transfers over the internet on or off: the engine saves the
/// setting, then starts again with or without its internet endpoint.
#[cfg(not(target_arch = "wasm32"))]
pub fn set_internet(on: bool) {
    send(Command::SetInternet(on));
    restart(|| None);
}

/// Stops the engine, runs `between` off the UI thread (with the engine
/// stopped), shows the notice it returns, and starts the engine again.
#[cfg(not(target_arch = "wasm32"))]
fn restart(between: impl FnOnce() -> Option<(NoticeKind, String)> + Send + 'static) {
    // The old engine's last events ("stopped") no longer count.
    global::<Generation>().update(|g| g.0 += 1);
    batch(|| {
        engine::status().set(engine::EngineStatus(None));
        engine::peers().set(engine::Peers(Vec::new()));
        engine::invitation().set(engine::Invitation(None));
        engine::code().set(engine::Code(None));
    });
    // Not owned by the page that asked, which may close now: the restarted
    // engine's event loop must outlive it.
    Owner::detached().run(move || {
        spawn(async move {
            native::stop().await;
            if let Some(Some((kind, text))) = off_thread(between).await {
                toast(kind, text);
            }
            start();
        })
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
        None if global::<Starting>().with_untracked(|s| s.0.is_some()) => {
            global::<Starting>().update(|s| s.0.get_or_insert_with(Vec::new).push(command));
        }
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

/// A folder of the app's own state (`staging`, `app-icons`), in the
/// engine's state folder: the engine cleans `staging` up.
#[cfg(not(target_arch = "wasm32"))]
pub fn state_dir(name: &str) -> std::path::PathBuf {
    native::dirs(os().name()).0.join(name)
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn apply(event: Event) {
    match event {
        Event::Status(status) => engine::status().set(engine::EngineStatus(Some(status))),
        Event::Settings(settings) => engine::settings().set(engine::SettingsState(settings)),
        Event::Peers(peers) => engine::peers().set(engine::Peers(peers)),
        Event::Transfer(t) => {
            let was_finished =
                engine::transfers().with_untracked(|engine::Transfers(transfers)| {
                    transfers
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
            engine::transfers().update(|engine::Transfers(all)| {
                match all.iter_mut().find(|x| x.id == t.id) {
                    Some(slot) => *slot = t,
                    None => all.insert(0, t),
                }
            })
        }
        Event::TransferRemoved(id) => {
            engine::transfers().update(|engine::Transfers(all)| all.retain(|t| t.id != id))
        }
        Event::Request(request) => {
            let text = format!("{} wants to send you {}.", request.peer_name, request.label);
            engine::requests().update(|engine::Requests(all)| {
                all.retain(|r| r.id != request.id);
                all.push(request);
            });
            crate::device::buzz(crate::device::Buzz::Request);
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
        Event::RequestClosed(id) => {
            engine::requests().update(|engine::Requests(all)| all.retain(|r| r.id != id))
        }
        Event::Invitation(view) => engine::invitation().set(engine::Invitation(Some(view))),
        Event::Code { code, expires } => engine::code().set(engine::Code(Some((code, expires)))),
        Event::PairingUsed { code, by } => {
            if code {
                engine::code().set(engine::Code(None));
            } else {
                engine::invitation().set(engine::Invitation(None));
            }
            let what = if code { "code" } else { "invitation" };
            toast(
                NoticeKind::Info,
                format!("{by} used your {what}. You'll be asked before anything is saved."),
            );
        }
        Event::InvitationsClosed => batch(|| {
            engine::invitation().set(engine::Invitation(None));
            engine::code().set(engine::Code(None));
        }),
        Event::Trusted(trusted) => engine::trusted().set(engine::Trusted(trusted)),
        Event::History(entries) => crate::state::history().set(crate::state::HistoryList(entries)),
        Event::Notice { kind, text } => toast(kind, text),
        Event::Diagnostics(report) => {
            crate::state::sheet().set(crate::state::Sheet::Diagnostics(report))
        }
        Event::Stopped => engine::offline().set(engine::Offline(Some(
            "The transfer service stopped.".into(),
        ))),
    }
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn notify_finished(t: &dexdrop_core::api::TransferSnapshot) {
    use dexdrop_core::{api::TransferState, store::Direction};
    let (verb, prep) = match t.direction {
        Direction::Outgoing => ("Sent", "to"),
        Direction::Incoming => ("Received", "from"),
    };
    use crate::device::{Buzz, buzz};
    buzz(match &t.state {
        TransferState::Completed => Buzz::Done,
        TransferState::CompletedWithIssues => Buzz::Issues,
        TransferState::Cancelled { .. } => return,
        _ => Buzz::Failed,
    });
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
