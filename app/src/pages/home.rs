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
                .child(when(
                    move || model().with(|m| m.transfers.iter().any(|t| !t.state.is_finished())),
                    active,
                ))
                .child(nearby())
                .child(far_devices())
                .child(recent())
                // Room for the actions floating over the bottom.
                .child(view().h(px(actions_height()))),
        ))
        .child(actions())
        .child(when(move || dropping.get(), drop_hint))
}

/// The height the floating actions take: both side by side, or one above
/// the other with large text.
fn actions_height() -> f32 {
    if ui::large_text() {
        2.0 * ACTION + space::M
    } else {
        ACTION
    }
}

/// The height of Send and Receive: a big target for a thumb.
const ACTION: f32 = ui::HERO;

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
        .accessibility_label(move || Some(SharedString::from(format!("This device: {}", line()))))
        .on_press(|| set_tab(Tab::Settings))
        .self_center()
        // As wide as it may be, the words taking the room beside the dot: a
        // pill hugging its words was cut short on phones whose system font
        // is wider than the one measured.
        .w_full()
        .max_w(px(360.0))
        .min_h(px(ui::hit()))
        .flex_row()
        .items_center()
        .gap(px(space::S))
        .px(px(space::L))
        .rounded(px(ui::radius::FULL))
        .bg(ui::surface)
        .hover(|s| s.bg(move || theme().accent))
        .active(|s| s.bg(move || theme().accent).scale(ui::motion::PRESS))
        .child(ui::dot(tone))
        .child(
            text(line)
                .flex_1()
                .number_of_lines(1)
                .text_size(px(ui::size::SMALL))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || theme().muted_foreground),
        )
}

/// Send and Receive, floating at the bottom where a thumb rests: above
/// the tab bar (phones, narrow windows), else at the window's foot. Two big
/// buttons sharing the width (one above the other with large text).
fn actions() -> impl IntoElement {
    view()
        .absolute()
        .left_0()
        .right_0()
        .bottom(px(ui::above_tab_bar()))
        .px(px(space::L))
        .pb(px(space::L))
        .items_center()
        .child(
            view()
                .w_full()
                .max_w(px(520.0))
                .flex_row()
                .gap(px(space::M))
                .style_when(ui::large_text, |s| s.flex_col())
                .child(action(Icon::Send, "Send", true, || push(Route::Send)))
                .child(action(Icon::Receive, "Receive", false, || {
                    push(Route::Receive)
                })),
        )
}

/// One of the two actions: a hero button, filled (Send) or on the card
/// color (Receive).
fn action(
    i: Icon,
    label: &'static str,
    filled: bool,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    ui::hero_btn(i, label, filled)
        .test_id(&label.to_ascii_lowercase())
        .on_press(on_press)
        .flex_1()
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
        .hover(move |st| st.bg(move || theme().muted.opacity(0.5)))
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
        // "Nearby" centered, over the centered radar; Look again (computers)
        // at the end, out of the way of the centering.
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_center()
                .min_h(px(32.0))
                .child(ui::section("Nearby").flex_1().text_center())
                .child(when(
                    move || !ui::mobile(),
                    || {
                        view().absolute().right_0().child(
                            ui::round_btn(Icon::Refresh, "Look again", Variant::Secondary, 32.0)
                                .on_press(|| bridge::send(Command::RefreshDiscovery)),
                        )
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
    // A view, not a button: views take long presses (NSH's buttons do not).
    view()
        .role(nsh_core::element::AccessibilityRole::Button)
        .label(format!(
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
        // A device known from before (Your devices): held or right-clicked,
        // forget it, after asking.
        .on_long_press(move || ask_forget(peer.get_untracked()))
        .on_context_menu(move |_: &ContextMenuEvent| ask_forget(peer.get_untracked()))
        .w(px(84.0))
        .items_center()
        .gap(px(space::S))
        .active(|s| s.scale(ui::motion::PRESS_ICON))
        .child(ui::far_avatar(
            name,
            platform,
            move || peer.with(|p| p.avatar.clone()),
            60.0,
            move || !peer.with(|p| p.nearby),
        ))
        .child(
            text(name)
                .text_size(px(ui::size::CAPTION))
                .font_weight(FontWeight::MEDIUM)
                .number_of_lines(2)
                .text_center()
                .w_full()
                .text_color(move || theme().foreground),
        )
}

/// Asks, then forgets a device known from before: it leaves Your devices
/// and the trusted ones. Nearby devices show while they are there, so
/// there is nothing to forget.
fn ask_forget(peer: PeerInfo) {
    if peer.nearby {
        return;
    }
    crate::device::buzz(crate::device::Buzz::Tap);
    ui::confirm(
        format!("Forget {}?", peer.name),
        "It leaves Your devices and is no longer trusted. You can send to it again when it is nearby, or with its code or invitation.",
        "Forget",
        move || bridge::send(Command::ForgetPeer(peer.peer_id)),
    );
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
    // The radar centered, the Wi‑Fi mark large in it, the words below; a
    // plain white card, no shadow.
    ui::tile()
        .shadow_none()
        .items_center()
        .gap(px(space::M))
        .py(px(space::XL))
        .child(ui::radar(
            120.0,
            ui::icon(Icon::Wifi, 44.0, move || theme().foreground),
        ))
        .child(
            muted(line)
                .text_center()
                .w_full()
                .max_w(px(320.0))
                .self_center(),
        )
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
                        .child(ui::section("Recent").flex_1())
                        .child(
                            ui::link_btn("See all").on_press(|| open(Tab::History, Route::Root)),
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
        .rounded(px(ui::radius::xxl()))
        .bg(move || theme().card.opacity(0.92))
        .shadow_xl()
        .items_center()
        .justify_center()
        .gap(px(space::M))
        .pointer_events(PointerEvents::None)
        .child(ui::art(ui::Art::Drop, 180.0))
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
