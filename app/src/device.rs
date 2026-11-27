//! What the device and its environment tell DexDrop (NSH's "Device and
//! environment"): coming back from sleep or from the background, the
//! taskbar or Dock, the app's badge, haptics on phones.

use std::cell::Cell;

use dexdrop_core::api::{Command, TransferSnapshot, TransferState};
use nsh_core::{
    Haptic, HapticImpact, HapticNotification,
    desktop::{ProgressState, WindowAttributes, WindowProgress},
    element::{
        close_notification, desktop::current_window, haptic, set_badge, show_notification,
        wake_count,
    },
    geometry::Percentage,
    platform::Lifecycle,
    prelude::*,
};

use crate::bridge;
use crate::state::engine;

/// Starts watching; called once by the shell.
pub fn watch() {
    reconnect_on_return();
    taskbar_progress();
    transfer_service();
    badge();
    keep_picked();
}

/// Tells the engine what is selected to send, each time that changes: the
/// copies phones make of picked files that nothing selects or sends any
/// more are deleted then, not left in the app's storage.
fn keep_picked() {
    let last = std::cell::RefCell::new(None::<Vec<std::path::PathBuf>>);
    effect(move || {
        let paths = crate::state::selection().with(|s| s.paths.clone());
        if last.borrow().as_ref() != Some(&paths) {
            *last.borrow_mut() = Some(paths.clone());
            crate::bridge::send(dexdrop_core::api::Command::KeepPicked(paths));
        }
    });
}

/// Back from the background (a phone unlocked) or from sleep (a laptop's
/// lid opened: the app stays "active", but its sockets and timers did not
/// survive): the network may have changed, so look for devices again
/// (FR-013) and retry at once the transfers that gave up while away,
/// rather than waiting for the user. Going to the background in the middle
/// of a transfer on a phone, say that the system may pause it there.
fn reconnect_on_return() {
    let was_away = Cell::new(false);
    let wakes = Cell::new(None::<u32>);
    effect(move || {
        let now = lifecycle();
        let woke = wake_count();
        let slept = wakes
            .replace(Some(woke))
            .is_some_and(|before| woke > before);
        if now == Lifecycle::Background {
            if !was_away.replace(true) {
                warn_backgrounded();
            }
            return;
        }
        let returned = now == Lifecycle::Active && was_away.replace(false);
        if returned {
            close_notification(BACKGROUND_NOTICE);
        }
        if returned || slept {
            reconnect();
        }
    });
}

/// Looks for devices again and retries the transfers that stopped retrying.
fn reconnect() {
    bridge::send(Command::RefreshDiscovery);
    let stalled: Vec<_> = engine::transfers().with_untracked(|engine::Transfers(transfers)| {
        transfers
            .iter()
            .filter(|t| {
                matches!(
                    t.state,
                    TransferState::Interrupted {
                        retrying: false,
                        ..
                    }
                )
            })
            .map(|t| t.id)
            .collect()
    });
    for id in stalled {
        bridge::send(Command::Resume(id));
    }
}

const BACKGROUND_NOTICE: &str = "dexdrop-background";

/// What the foreground service's notification says while transfers move:
/// a title, a line of progress, the whole percent done (`None` while no
/// size is known) and whether anything is being sent; `None` when nothing
/// moves.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceStatus {
    pub title: String,
    pub text: String,
    pub percent: Option<u8>,
    pub upload: bool,
}

/// [`ServiceStatus`] of `transfers`: those moving (preparing, connecting,
/// waiting for an answer, transferring, reconnecting).
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub fn service_status(transfers: &[TransferSnapshot]) -> Option<ServiceStatus> {
    let active: Vec<_> = transfers.iter().filter(|t| moving(&t.state)).collect();
    let upload = active
        .iter()
        .any(|t| t.direction == dexdrop_core::store::Direction::Outgoing);
    let download = active
        .iter()
        .any(|t| t.direction == dexdrop_core::store::Direction::Incoming);
    let title = match active.as_slice() {
        [] => return None,
        [one] if upload => format!("Sending {} to {}", one.label, one.peer_name),
        [one] => format!("Receiving {} from {}", one.label, one.peer_name),
        many if upload && download => format!("{} transfers", many.len()),
        many if upload => format!("Sending to {} devices", many.len()),
        many => format!("Receiving {} transfers", many.len()),
    };
    let total: u64 = active.iter().map(|t| t.total_bytes).sum();
    let done: u64 = active.iter().map(|t| t.bytes_done.min(t.total_bytes)).sum();
    let percent = (total > 0).then(|| (done as f64 / total as f64 * 100.0).floor() as u8);
    let text = match percent {
        Some(p) => format!(
            "{p}% · {} of {}",
            dexdrop_core::format::bytes(done),
            dexdrop_core::format::bytes(total)
        ),
        None => "Getting ready…".to_owned(),
    };
    Some(ServiceStatus {
        title,
        text,
        percent,
        upload,
    })
}

/// Android: a foreground service keeps DexDrop running while files move
/// (PRD 8.5), its notification showing their progress; it stops once none
/// moves. Android starts one only while the app is in front, so a transfer
/// started in the background keeps the older notice instead.
fn transfer_service() {
    #[cfg(target_os = "android")]
    {
        let shown = computed(|| {
            engine::transfers().with(|engine::Transfers(transfers)| service_status(&transfers))
        });
        effect(move || match shown.get() {
            Some(status) => {
                let status = dexdrop_net::android_service::Status {
                    title: status.title,
                    text: status.text,
                    percent: status.percent,
                    upload: status.upload,
                };
                match dexdrop_net::android_service::show(&status) {
                    Ok(()) => SERVICE_RUNNING.set(true),
                    Err(why) => log::info!("transfer service: {why}"),
                }
            }
            None => {
                if SERVICE_RUNNING.replace(false) {
                    dexdrop_net::android_service::hide();
                }
            }
        });
    }
}

thread_local! {
    /// Whether the transfer service runs (Android), set on the UI thread.
    static SERVICE_RUNNING: Cell<bool> = const { Cell::new(false) };
}

/// Phones suspend apps in the background within seconds: when a transfer
/// is moving, a notification says so (and goes when the app is back).
/// Android's foreground service keeps it running instead, when it started.
fn warn_backgrounded() {
    if !crate::ui::mobile() || SERVICE_RUNNING.get() {
        return;
    }
    let moving: Vec<String> = engine::transfers().with_untracked(|engine::Transfers(transfers)| {
        transfers
            .iter()
            .filter(|t| moving(&t.state))
            .map(|t| t.label.clone())
            .collect()
    });
    let body = match moving.as_slice() {
        [] => return,
        [one] => format!(
            "Return to DexDrop to make sure {one} finishes: your phone may pause it while DexDrop is in the background."
        ),
        many => format!(
            "Return to DexDrop to make sure your {} transfers finish: your phone may pause them while DexDrop is in the background.",
            many.len()
        ),
    };
    drop(show_notification(
        nsh_core::notifications::Notification::new(BACKGROUND_NOTICE, "Transfer in progress", body),
    ));
}

fn moving(state: &TransferState) -> bool {
    matches!(
        state,
        TransferState::Preparing
            | TransferState::Connecting
            | TransferState::Waiting
            | TransferState::Transferring
            | TransferState::Interrupted { retrying: true, .. }
    )
}

/// What the taskbar button or the Dock's icon shows for the transfers in
/// progress: `None` when there are none; the share done, or no share while
/// none is known yet; paused when all are, failed when one gave up.
pub fn progress_of(transfers: &[TransferSnapshot]) -> Option<(Option<f32>, ProgressState)> {
    let active: Vec<_> = transfers
        .iter()
        .filter(|t| !t.state.is_finished() && t.state != TransferState::Requested)
        .collect();
    if active.is_empty() {
        return None;
    }
    let state = if active.iter().any(|t| {
        matches!(
            t.state,
            TransferState::Interrupted {
                retrying: false,
                ..
            }
        )
    }) {
        ProgressState::Error
    } else if active
        .iter()
        .all(|t| matches!(t.state, TransferState::Paused { .. }))
    {
        ProgressState::Paused
    } else {
        ProgressState::Normal
    };
    let total: u64 = active.iter().map(|t| t.total_bytes).sum();
    let done: u64 = active.iter().map(|t| t.bytes_done).sum();
    if total == 0 || (done == 0 && state == ProgressState::Normal) {
        return Some((None, ProgressState::Indeterminate));
    }
    // Whole percents: the window is told only when the shown share changes.
    let share = (done.min(total) as f64 / total as f64 * 100.0).floor() as f32 / 100.0;
    Some((Some(share), state))
}

/// Computers: progress on the taskbar button (Windows, Linux docks that
/// support it) or the Dock's icon (macOS), seen while the window is hidden.
fn taskbar_progress() {
    if !is_desktop() {
        return;
    }
    let shown = computed(|| {
        engine::transfers().with(|engine::Transfers(transfers)| progress_of(transfers))
    });
    // The window this renders in (the app's "main"): the app host
    // (`main_window`) has no taskbar button or Dock tile of its own.
    let window = current_window();
    effect(move || {
        let progress = shown.get().map(|(value, state)| {
            let mut p = WindowProgress::default();
            p.value = value.map(Percentage);
            p.state = state;
            p
        });
        let mut attributes = WindowAttributes::default();
        attributes.progress = Some(progress);
        window.set(attributes);
    });
}

/// The app's icon counts the requests waiting for an answer (the Dock, the
/// taskbar, iOS's home screen; Android launchers only mark the app).
fn badge() {
    let waiting =
        computed(|| engine::requests().with(|engine::Requests(requests)| requests.len() as u32));
    effect(move || set_badge(waiting.get()));
}

/// A short vibration on phones (NSH's haptics; nothing elsewhere).
pub fn buzz(what: Buzz) {
    haptic(match what {
        Buzz::Tap => Haptic::Selection,
        Buzz::Request => Haptic::Impact(HapticImpact::Medium),
        Buzz::Done => Haptic::Notification(HapticNotification::Success),
        Buzz::Issues => Haptic::Notification(HapticNotification::Warning),
        Buzz::Failed => Haptic::Notification(HapticNotification::Error),
    });
}

/// What a vibration says.
#[derive(Clone, Copy)]
pub enum Buzz {
    /// A tap that did something (sent to a device).
    Tap,
    /// Another device wants to send something.
    Request,
    /// A transfer finished, verified.
    Done,
    /// A transfer finished with files failed or skipped.
    Issues,
    /// A transfer stopped.
    Failed,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(state: TransferState, done: u64, total: u64) -> TransferSnapshot {
        TransferSnapshot {
            state,
            bytes_done: done,
            total_bytes: total,
            ..Default::default()
        }
    }

    #[test]
    fn the_service_says_what_moves() {
        assert_eq!(service_status(&[]), None);
        // Paused and finished transfers keep nothing running.
        assert_eq!(
            service_status(&[
                snap(TransferState::Paused { by_peer: false }, 1, 2),
                snap(TransferState::Completed, 2, 2),
            ]),
            None
        );
        let mut send = snap(TransferState::Transferring, 512 * 1024, 2 * 1024 * 1024);
        send.direction = dexdrop_core::store::Direction::Outgoing;
        send.label = "Talk.pdf".into();
        send.peer_name = "Desk".into();
        let one = service_status(std::slice::from_ref(&send)).unwrap();
        assert_eq!(one.title, "Sending Talk.pdf to Desk");
        assert_eq!(one.percent, Some(25));
        assert!(one.upload);
        assert!(one.text.starts_with("25% · "), "{}", one.text);

        let mut receive = snap(TransferState::Connecting, 0, 0);
        receive.direction = dexdrop_core::store::Direction::Incoming;
        let both = service_status(&[send, receive.clone()]).unwrap();
        assert_eq!(both.title, "2 transfers");
        let waiting = service_status(&[receive]).unwrap();
        assert_eq!(waiting.percent, None);
        assert_eq!(waiting.text, "Getting ready…");
        assert!(!waiting.upload);
    }

    #[test]
    fn the_taskbar_shows_what_is_moving() {
        assert_eq!(progress_of(&[]), None);
        assert_eq!(progress_of(&[snap(TransferState::Completed, 10, 10)]), None);
        assert_eq!(
            progress_of(&[snap(TransferState::Transferring, 25, 100)]),
            Some((Some(0.25), ProgressState::Normal))
        );
        // Two at once: their bytes together.
        assert_eq!(
            progress_of(&[
                snap(TransferState::Transferring, 50, 100),
                snap(TransferState::Transferring, 0, 100),
            ]),
            Some((Some(0.25), ProgressState::Normal))
        );
        assert_eq!(
            progress_of(&[snap(TransferState::Connecting, 0, 100)]),
            Some((None, ProgressState::Indeterminate))
        );
        assert_eq!(
            progress_of(&[snap(TransferState::Paused { by_peer: false }, 40, 100)]),
            Some((Some(0.4), ProgressState::Paused))
        );
        let gave_up = TransferState::Interrupted {
            reason: String::new(),
            retrying: false,
        };
        assert_eq!(
            progress_of(&[
                snap(gave_up, 40, 100),
                snap(TransferState::Transferring, 10, 100)
            ]),
            Some((Some(0.25), ProgressState::Error))
        );
    }
}
