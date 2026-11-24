//! Home: send or receive, nearby devices, what is moving now, and recent
//! transfers.

use dexdrop_core::{
    api::{Command, PeerInfo, TransferSnapshot},
    format,
    store::{Direction, HistoryEntry},
};
use nsh_base::theme;
use nsh_core::prelude::*;

use crate::{
    bridge,
    pages::{body, history, send, transfer},
    state::{Picked, Route, Tab, model, open, picked, push, set_tab},
    ui::{self, Icon, Tone, Variant, muted, space},
};

pub fn page() -> impl IntoElement {
    let dropping = signal(false);
    view()
        .flex_1()
        .on_file_drag_enter(move |_: &FileDragEvent| dropping.set(true))
        .on_file_drag_leave(move || dropping.set(false))
        .on_file_drop(move |event: &FileDropEvent| {
            dropping.set(false);
            send::select(send::file_paths(&event.uris));
            push(Route::Send);
        })
        .child(body(
            view()
                .gap(px(space::XXL))
                .child(status())
                .child(actions())
                .child(when(
                    move || model().with(|m| m.transfers.iter().any(|t| !t.state.is_finished())),
                    active,
                ))
                .child(nearby())
                .child(far_devices())
                .child(recent()),
        ))
        .child(when(move || dropping.get(), drop_hint))
}

/// "Visible as Pixel 8": this device, and whether others can see it.
fn status() -> impl IntoElement {
    let line = move || {
        model().with(|m| match (&m.status, &m.offline) {
            (_, Some(_)) => "Preview — open the app to share".to_owned(),
            (Some(s), _) if s.visible => format!("Visible as {}", s.device_name),
            (Some(s), _) => format!("{} · hidden", s.device_name),
            (None, _) => "Starting…".to_owned(),
        })
    };
    let tone = move || {
        model().with(|m| match &m.status {
            Some(s) if s.visible && m.offline.is_none() => Tone::Success,
            _ => Tone::Neutral,
        })
    };
    ui::pressable()
        .accessibility_label("This device")
        .on_press(|| set_tab(Tab::Settings))
        .self_center()
        .flex_row()
        .items_center()
        .gap(px(space::S))
        .px(px(space::M))
        .py(px(6.0))
        .rounded(px(999.0))
        .bg(ui::surface)
        .active(|s| s.opacity(0.7))
        .child(ui::dot(tone))
        .child(muted(line).font_weight(FontWeight::MEDIUM))
}

fn actions() -> impl IntoElement {
    view()
        .flex_row()
        .justify_center()
        .gap(px(if ui::mobile() { 40.0 } else { 64.0 }))
        // Large system text: one above the other, so the labels fit.
        .style_when(ui::large_text, |s| s.flex_col().items_center())
        .pt(px(space::S))
        .child(ui::hero_action(Icon::Send, "Send", ui::SEND, || {
            push(Route::Send)
        }))
        .child(ui::hero_action(
            Icon::Receive,
            "Receive",
            ui::RECEIVE,
            || push(Route::Receive),
        ))
}

/// Transfers running now, each a tap away from its progress.
fn active() -> impl IntoElement {
    let running = move || {
        model().with(|m| {
            m.transfers
                .iter()
                .filter(|t| !t.state.is_finished())
                .cloned()
                .collect::<Vec<_>>()
        })
    };
    view()
        .gap(px(space::M))
        .child(ui::section("Now"))
        .child(ui::group().child(each(
            running,
            |t: &TransferSnapshot| t.id,
            |t, _| active_row(t),
        )))
}

fn active_row(t: ReadSignal<TransferSnapshot>) -> impl IntoElement {
    let s = move || t.get();
    ui::pressable()
        .accessibility_label(t.with_untracked(|t| t.label.clone()))
        .on_press(move || push(Route::Transfer(Some(t.with_untracked(|t| t.id)))))
        .flex_row()
        .items_center()
        .gap(px(space::M))
        .px(px(space::L))
        .py(px(space::M))
        .active(move |st| st.bg(move || theme().muted))
        .child(ui::progress_ring(
            move || s().progress(),
            move || transfer::tone_of(&s().state),
            40.0,
        ))
        .child(
            view()
                .flex_1()
                .gap(px(2.0))
                .child(ui::strong(move || s().label).number_of_lines(1))
                .child(muted(move || transfer::one_line(&s())).number_of_lines(1)),
        )
        .child(ui::icon(Icon::ChevronRight, 18.0, move || {
            theme().muted_foreground
        }))
}

// ---------------------------------------------------------------- nearby

fn nearby() -> impl IntoElement {
    let peers = move || {
        model().with(|m| {
            m.peers
                .iter()
                .filter(|p| p.nearby)
                .cloned()
                .collect::<Vec<_>>()
        })
    };
    let empty = move || model().with(|m| !m.peers.iter().any(|p| p.nearby));
    view()
        .gap(px(space::M))
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .child(ui::section("Nearby"))
                .child(when(
                    move || !ui::mobile(),
                    || {
                        ui::round_btn(Icon::Refresh, "Look again", Variant::Secondary, 32.0)
                            .on_press(|| bridge::send(Command::RefreshDiscovery))
                    },
                )),
        )
        .child(when(empty, looking).otherwise(move || {
            scroll().horizontal().child(
                view()
                    .flex_row()
                    .gap(px(space::L))
                    .py(px(space::XS))
                    .child(each(peers, |p: &PeerInfo| p.peer_id, |p, _| bubble(p))),
            )
        }))
}

/// Devices this one exchanged files with, not on this network: reached over
/// the internet.
fn far_devices() -> impl IntoElement {
    let far = move || {
        model().with(|m| {
            m.peers
                .iter()
                .filter(|p| !p.nearby)
                .cloned()
                .collect::<Vec<_>>()
        })
    };
    when(
        move || model().with(|m| m.peers.iter().any(|p| !p.nearby)),
        move || {
            view()
                .gap(px(space::M))
                .child(ui::section("Your devices"))
                .child(
                    scroll().horizontal().child(
                        view()
                            .flex_row()
                            .gap(px(space::L))
                            .py(px(space::XS))
                            .child(each(far, |p: &PeerInfo| p.peer_id, |p, _| bubble(p))),
                    ),
                )
        },
    )
}

/// A nearby device: tap to send to it.
fn bubble(peer: ReadSignal<PeerInfo>) -> impl IntoElement {
    let name = move || peer.with(|p| p.name.clone());
    let platform = move || peer.with(|p| p.platform.clone());
    ui::pressable()
        .accessibility_label(format!(
            "Send to {}",
            peer.with_untracked(|p| p.name.clone())
        ))
        .on_press(move || {
            let id = peer.with_untracked(|p| p.peer_id);
            if send::has_selection() {
                send::send_to(dexdrop_core::api::Target::Peer(id));
            } else {
                picked().set(Picked(Some(id)));
                push(Route::Send);
            }
        })
        .w(px(84.0))
        .items_center()
        .gap(px(space::S))
        .active(|s| s.scale(0.94))
        .child(ui::far_avatar(
            name,
            platform,
            move || peer.with(|p| p.avatar.clone()),
            60.0,
            move || !peer.with(|p| p.nearby),
        ))
        .child(
            text(name)
                .text_size(px(ui::size::CAPTION + 1.0))
                .font_weight(FontWeight::MEDIUM)
                .number_of_lines(2)
                .text_center()
                .text_color(move || theme().foreground),
        )
}

/// No device yet: a small radar while discovery looks.
fn looking() -> impl IntoElement {
    // After a while with nobody found, say what to check (FR-005).
    let waited = signal(false);
    set_timeout(std::time::Duration::from_secs(8), move || waited.set(true));
    let line = move || {
        model().with(|m| match m.status.as_ref().map(|s| &s.discovery) {
            _ if m.offline.is_some() => "Nearby devices show up in the app".to_owned(),
            Some(dexdrop_core::api::DiscoveryStatus::Unavailable(_)) => {
                "This network doesn't let devices find each other. Tap Receive on the other device and use its code.".to_owned()
            }
            _ if waited.get() => {
                "Not here yet? Open DexDrop on the other device, on the same Wi‑Fi. Or tap Receive there and use its code.".to_owned()
            }
            _ => "Looking for devices on this Wi‑Fi…".to_owned(),
        })
    };
    ui::tile()
        .flex_row()
        .items_center()
        .gap(px(space::L))
        .child(ui::radar(
            ui::SEND,
            56.0,
            ui::icon(Icon::Wifi, 22.0, rgb(ui::SEND.0)),
        ))
        .child(muted(line).flex_1())
}

// ---------------------------------------------------------------- recent

fn recent() -> impl IntoElement {
    let entries =
        move || crate::state::history().with(|h| h.0.iter().take(4).cloned().collect::<Vec<_>>());
    when(
        move || crate::state::history().with(|h| !h.0.is_empty()),
        move || {
            view()
                .gap(px(space::M))
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .child(ui::section("Recent"))
                        .child(
                            ui::btn("See all", Variant::Plain)
                                .on_press(|| open(Tab::History, Route::Root)),
                        ),
                )
                .child(ui::group().child(each(
                    entries,
                    |h: &HistoryEntry| h.transfer_id.clone(),
                    |h, i| {
                        view()
                            .child(when(move || i.get() > 0, || ui::separator(68.0)))
                            .child(history::row(move || Some(h.get())))
                    },
                )))
        },
    )
}

/// The overlay while files are dragged over the window.
fn drop_hint() -> impl IntoElement {
    view()
        .absolute()
        .top(px(space::L))
        .left(px(space::L))
        .right(px(space::L))
        .bottom(px(space::L))
        .rounded(px(24.0))
        .border_2()
        .border_dashed()
        .border_color(rgb(ui::SEND.0))
        .bg(rgba((ui::SEND.0 << 8) | 0x14))
        .items_center()
        .justify_center()
        .gap(px(space::M))
        .pointer_events(PointerEvents::None)
        .child(ui::glyph(Icon::Plus, ui::SEND, 64.0))
        .child(ui::strong("Drop to send"))
}

/// What a past transfer says in one line.
pub fn summary(e: &HistoryEntry) -> String {
    let dir = if e.direction == Direction::Outgoing {
        "To"
    } else {
        "From"
    };
    format!("{dir} {} · {}", e.peer_name, format::bytes(e.bytes))
}
