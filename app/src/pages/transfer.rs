//! A transfer's progress (FR-050 – FR-056), and the sheet that asks before
//! anything is received (FR-040 – FR-043).

use dexdrop_core::{
    api::{Command, IncomingRequest, TransferSnapshot, TransferState},
    format,
    naming::ConflictPolicy,
    protocol::TransferId,
    store::Direction,
};
use nsh_base::{alert_dialog, button, theme};
use nsh_core::{prelude::*, share::ShareRequest};

use crate::{
    bridge,
    pages::body,
    state::{Route, desktop_platform, model, reset_to, toast},
    ui::{self, Icon, Tone, Variant, muted, space},
};

pub fn tone_of(state: &TransferState) -> Tone {
    match state {
        TransferState::Completed => Tone::Success,
        TransferState::CompletedWithIssues
        | TransferState::Interrupted { .. }
        | TransferState::Paused { .. } => Tone::Warning,
        TransferState::Failed(_) | TransferState::Declined(_) => Tone::Danger,
        TransferState::Cancelled { .. } => Tone::Neutral,
        _ => Tone::Accent,
    }
}

/// The icon and tone of how a transfer ended.
pub fn mark(state: &TransferState) -> (Icon, Tone) {
    match state {
        TransferState::Completed => (Icon::Check, Tone::Success),
        TransferState::CompletedWithIssues => (Icon::Alert, Tone::Warning),
        TransferState::Failed(_) => (Icon::Alert, Tone::Danger),
        _ => (Icon::X, Tone::Neutral),
    }
}

pub fn short_state(state: &TransferState) -> &'static str {
    match state {
        TransferState::Preparing => "Preparing",
        TransferState::Connecting => "Connecting",
        TransferState::Waiting => "Waiting for an answer",
        TransferState::Requested => "Needs your answer",
        TransferState::Transferring => "Transferring",
        TransferState::Paused { .. } => "Paused",
        TransferState::Interrupted { .. } => "Reconnecting",
        TransferState::Completed => "Done",
        TransferState::CompletedWithIssues => "Done with issues",
        TransferState::Declined(_) => "Declined",
        TransferState::Cancelled { .. } => "Cancelled",
        TransferState::Failed(_) => "Failed",
    }
}

/// One line about a transfer: progress and speed, or how it ended.
pub fn one_line(t: &TransferSnapshot) -> String {
    match &t.state {
        TransferState::Transferring => {
            let mut line = format!(
                "{} of {}",
                format::bytes(t.bytes_done),
                format::bytes(t.total_bytes)
            );
            if let Some(eta) = t.eta {
                line += &format!(" · {} left", format::duration(eta));
            }
            line
        }
        TransferState::Interrupted {
            retrying: false, ..
        } => "Interrupted".to_owned(),
        TransferState::Completed => {
            let mut line = format!(
                "{} · {}",
                format::item_count(t.files, t.folders),
                format::bytes(t.total_bytes)
            );
            if let Some(rate) = t.average_rate {
                line += &format!(" · {}", format::rate(rate));
            }
            line
        }
        TransferState::Failed(why) | TransferState::Declined(why) => why.clone(),
        TransferState::Interrupted { reason, .. } => reason.clone(),
        state => short_state(state).to_owned(),
    }
}

/// The transfer a route shows: by id, or the newest outgoing one.
fn find(id: Option<TransferId>) -> Option<TransferSnapshot> {
    model().with(|m| match id {
        Some(id) => m.transfers.iter().find(|t| t.id == id).cloned(),
        None => m
            .transfers
            .iter()
            .find(|t| t.direction == Direction::Outgoing)
            .cloned(),
    })
}

pub fn title(id: Option<TransferId>) -> SharedString {
    let incoming = find(id).is_some_and(|t| t.direction == Direction::Incoming);
    let done = find(id).is_some_and(|t| t.state == TransferState::Completed);
    match (incoming, done) {
        (true, true) => "Received".into(),
        (true, false) => "Receiving".into(),
        (false, true) => "Sent".into(),
        (false, false) => "Sending".into(),
    }
}

pub fn page(id: Option<TransferId>) -> impl IntoElement {
    let snap = move || find(id);
    body(dynamic(move || match snap() {
        None => ui::empty(Icon::Paperplane, "Getting ready…").into_element(),
        Some(first) => progress(first.id).into_element(),
    }))
}

fn progress(id: TransferId) -> impl IntoElement {
    let s = move || find(Some(id)).unwrap_or_default();
    let done = move || s().state == TransferState::Completed;
    let finished = move || s().state.is_finished();
    view()
        .items_center()
        .gap(px(space::XL))
        .pt(px(space::L))
        .child(
            view()
                .size(px(220.0))
                .items_center()
                .justify_center()
                .child(
                    ui::progress_ring(move || s().progress(), move || tone_of(&s().state), 220.0)
                        .absolute(),
                )
                .child(dynamic(move || {
                    if done() {
                        let pop_in = motion().entering(MotionPreset::ZoomIn);
                        ui::gradient(ui::RECEIVE)
                            .bind_motion(&pop_in)
                            .size(px(96.0))
                            .rounded(px(48.0))
                            .items_center()
                            .justify_center()
                            .child(ui::icon(Icon::Check, 48.0, white()))
                            .into_element()
                    } else {
                        view()
                            .items_center()
                            .child(
                                text(move || format!("{}%", (s().progress() * 100.0).floor()))
                                    .text_size(px(44.0))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(move || theme().foreground),
                            )
                            .child(muted(move || {
                                s().rate.map(format::rate).unwrap_or_default()
                            }))
                            .into_element()
                    }
                })),
        )
        .child(
            view()
                .items_center()
                .gap(px(space::XS))
                .child(
                    ui::title(move || s().label)
                        .number_of_lines(2)
                        .text_center(),
                )
                .child(
                    ui::pill(move || short_state(&s().state), move || tone_of(&s().state))
                        .self_center(),
                )
                // Detail the pill does not say: bytes and time, or why it ended.
                .child(when(
                    move || one_line(&s()) != short_state(&s().state),
                    move || muted(move || one_line(&s())).text_center(),
                )),
        )
        .child(peer_card(s))
        .child(dynamic(move || {
            let failures = s().failures;
            (!failures.is_empty()).then(|| {
                ui::tile().self_stretch().gap(px(space::XS)).children(
                    failures.into_iter().take(5).map(|(path, why)| {
                        muted(format!("{path}: {why}")).text_color(move || theme().destructive)
                    }),
                )
            })
        }))
        .child(controls(id, s))
        .child(when(finished, move || after(s)))
}

/// Who is on the other end, and the number both screens show.
fn peer_card(s: impl Fn() -> TransferSnapshot + Copy + 'static) -> impl IntoElement {
    let name = move || {
        let n = s().peer_name;
        if n.is_empty() { "…".to_owned() } else { n }
    };
    let platform = move || s().peer_platform;
    ui::tile()
        .self_stretch()
        .flex_row()
        .items_center()
        .gap(px(space::M))
        .child(ui::avatar(name, platform, 44.0))
        .child(
            view()
                .flex_1()
                .gap(px(2.0))
                .child(muted(move || {
                    if s().direction == Direction::Outgoing {
                        "To".to_owned()
                    } else {
                        "From".to_owned()
                    }
                }))
                .child(ui::strong(name).number_of_lines(1)),
        )
        .child(when(
            move || s().sas.is_some() && !s().state.is_finished(),
            move || {
                view().items_end().child(muted("Check")).child(
                    text(move || s().sas.unwrap_or_default())
                        .text_size(px(ui::size::HEADING))
                        .font_weight(FontWeight::BOLD)
                        .letter_spacing(px(1.0))
                        .text_color(move || theme().foreground)
                        .label(move || {
                            format!("Verification number {}", s().sas.unwrap_or_default())
                        }),
                )
            },
        ))
}

/// Pause, resume, cancel.
fn controls(id: TransferId, s: impl Fn() -> TransferSnapshot + Copy + 'static) -> impl IntoElement {
    view()
        .flex_row()
        .justify_center()
        .gap(px(space::XL))
        .child(when(
            move || s().can_pause(),
            move || {
                control(Icon::Pause, "Pause", Variant::Secondary, move || {
                    bridge::send(Command::Pause(id))
                })
            },
        ))
        .child(when(
            move || s().can_resume(),
            move || {
                let label = match s().state {
                    TransferState::Failed(_) | TransferState::Interrupted { .. } => "Retry",
                    _ => "Resume",
                };
                control(Icon::Play, label, Variant::Primary, move || {
                    bridge::send(Command::Resume(id))
                })
            },
        ))
        .child(when(
            move || s().can_cancel(),
            move || {
                control(Icon::X, "Cancel", Variant::Danger, move || {
                    let answer = alert_dialog()
                        .title("Stop this transfer?")
                        .cancel("keep", "Keep going")
                        .destructive("stop", "Stop")
                        .show();
                    spawn(async move {
                        if answer
                            .await
                            .is_ok_and(|a| a.action.as_deref() == Some("stop"))
                        {
                            bridge::send(Command::Cancel(id));
                        }
                    });
                })
            },
        ))
}

fn control(
    i: Icon,
    label: &'static str,
    variant: Variant,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    view()
        .items_center()
        .gap(px(space::S))
        .child(ui::round_btn(i, label, variant, 64.0).on_press(on_press))
        .child(muted(label).font_weight(FontWeight::MEDIUM))
}

/// After a transfer: share or show what came in, and leave.
fn after(s: impl Fn() -> TransferSnapshot + Copy + 'static) -> impl IntoElement {
    view()
        .self_stretch()
        .gap(px(space::S))
        .child(when(
            move || {
                !desktop_platform()
                    && s().direction == Direction::Incoming
                    && !s().saved_files.is_empty()
            },
            move || {
                ui::icon_btn(Icon::Share, "Share or save", Variant::Primary).on_press(move || {
                    let snap = s();
                    let mut request = ShareRequest::new().title(snap.label.clone());
                    for file in snap.saved_files {
                        request = request.file(file);
                    }
                    let shared = share(request);
                    spawn(async move {
                        if let Err(err) = shared.await {
                            toast(
                                dexdrop_core::api::NoticeKind::Error,
                                format!("Could not share: {err:?}"),
                            );
                        }
                    });
                })
            },
        ))
        .child(when(
            move || {
                desktop_platform()
                    && s().direction == Direction::Incoming
                    && s().state == TransferState::Completed
            },
            move || {
                ui::icon_btn(Icon::Folder, "Show in folder", Variant::Primary).on_press(move || {
                    if let Some(dir) = s().saved_to {
                        let shown = reveal_path(dir);
                        spawn(async move {
                            let _ = shown.await;
                        });
                    }
                })
            },
        ))
        .child(ui::btn("Done", Variant::Secondary).on_press(|| crate::state::reset_to(Route::Root)))
}

// ---------------------------------------------------------------- requests

/// The first request waiting for an answer, over whatever is on screen.
pub fn request_overlay() -> impl IntoElement {
    let first = move || model().with(|m| m.requests.first().cloned());
    dynamic(move || {
        first().map(|r| {
            view()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .bottom_0()
                .bg(rgba(0x0000_0059))
                .justify_end()
                .items_center()
                .child(request_sheet(r))
        })
    })
}

fn request_sheet(r: IncomingRequest) -> impl IntoElement {
    let id = r.id;
    let trust = signal(false);
    let conflict = signal(ConflictPolicy::KeepBoth);
    let options = signal(false);
    let respond = move |accept: bool| {
        bridge::send(Command::Respond {
            id,
            accept,
            trust: trust.get_untracked(),
            conflict: conflict.get_untracked(),
        });
        if accept {
            // The progress takes over this tab: whatever led here (the
            // Receive page, with its code) is done, and "Done" goes home.
            reset_to(Route::Transfer(Some(id)));
        }
    };
    let accept = move || {
        if conflict.get_untracked() == ConflictPolicy::Replace {
            // Replacing files is destructive: confirm explicitly (FR-043).
            let answer = alert_dialog()
                .title("Replace existing files?")
                .description("Files with the same names will be overwritten.")
                .cancel("cancel", "Cancel")
                .destructive("replace", "Replace")
                .show();
            spawn(async move {
                if answer
                    .await
                    .is_ok_and(|a| a.action.as_deref() == Some("replace"))
                {
                    respond(true);
                }
            });
        } else {
            respond(true);
        }
    };
    let rise = motion().entering(MotionPreset::SlideInUp);
    let name = r.peer_name.clone();
    let field = move |f: fn(&IncomingRequest) -> String| {
        move || {
            model().with(|m| {
                m.requests
                    .iter()
                    .find(|x| x.id == id)
                    .map(f)
                    .unwrap_or_default()
            })
        }
    };
    let (n1, p1) = (
        field(|r| r.peer_name.clone()),
        field(|r| r.peer_platform.clone()),
    );
    let enough = r.enough_space();
    let trusted = r.trusted;
    view()
        .bind_motion(&rise)
        .w_full()
        .max_w(px(520.0))
        .p(px(space::XL))
        .pb(px(if ui::mobile() { 40.0 } else { space::XL }))
        .gap(px(space::L))
        .rounded_t(px(28.0))
        .rounded_b(px(if ui::mobile() { 0.0 } else { 28.0 }))
        .mb(px(if ui::mobile() { 0.0 } else { space::XL }))
        .bg(move || theme().card)
        .shadow_lg()
        .role(nsh_core::element::AccessibilityRole::Dialog)
        .child(
            view()
                .items_center()
                .gap(px(space::S))
                .child(ui::avatar(n1, p1, 72.0))
                .child(ui::title(name.clone()).text_center())
                .child(
                    muted(format!(
                        "wants to send {} · {}",
                        r.label,
                        format::bytes(r.total_bytes)
                    ))
                    .text_center(),
                ),
        )
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .justify_center()
                .gap(px(space::S))
                .child(when(
                    move || trusted,
                    || ui::pill("Trusted", || Tone::Success),
                ))
                .child(when(
                    move || r.via_invitation,
                    || ui::pill("Used your code", || Tone::Accent),
                ))
                .child(when(
                    move || !enough,
                    || ui::pill("Not enough space", || Tone::Danger),
                )),
        )
        .child(
            view()
                .items_center()
                .gap(px(2.0))
                .py(px(space::M))
                .rounded(px(16.0))
                .bg(move || theme().muted)
                .child(muted("Same number on both screens?"))
                .child(
                    text(r.sas.clone())
                        .text_size(px(28.0))
                        .font_weight(FontWeight::BOLD)
                        .letter_spacing(px(3.0))
                        .text_color(move || theme().foreground)
                        .label(format!("Verification number {}", r.sas)),
                ),
        )
        .child(when(
            move || options.get(),
            move || {
                ui::group()
                    .bg(move || theme().background)
                    .child(when(
                        move || !trusted,
                        move || {
                            ui::switch_row(
                                ui::glyph(Icon::UserCheck, ui::RECEIVE, 28.0),
                                "Trust this device",
                                move || trust.get(),
                                move |on| trust.set(on),
                            )
                        },
                    ))
                    .child(conflict_picker(conflict))
            },
        ))
        .child(when(
            move || !options.get(),
            move || {
                ui::btn("Options", Variant::Plain)
                    .self_center()
                    .on_press(move || options.set(true))
            },
        ))
        .child(
            view()
                .flex_row()
                .gap(px(space::M))
                .child(
                    ui::btn("Decline", Variant::Secondary)
                        .flex_1()
                        .on_press(move || respond(false)),
                )
                .child(
                    ui::btn("Accept", Variant::Primary)
                        .flex_1()
                        .disabled(move || !enough)
                        .on_press(accept),
                ),
        )
}

/// What to do with a file that already exists: the system's segmented
/// control on iOS, the app's own elsewhere.
fn conflict_picker(conflict: Signal<ConflictPolicy>) -> impl IntoElement {
    const POLICIES: [(&str, ConflictPolicy); 3] = [
        ("Keep both", ConflictPolicy::KeepBoth),
        ("Skip", ConflictPolicy::Skip),
        ("Replace", ConflictPolicy::Replace),
    ];
    let index = move || {
        POLICIES
            .iter()
            .position(|(_, p)| *p == conflict.get())
            .unwrap_or(0) as u32
    };
    view()
        .px(px(space::L))
        .py(px(space::M))
        .gap(px(space::S))
        .child(muted("If a file is already there"))
        .child(if os().name() == "ios" {
            nsh_core::mobile::segmented_control()
                .items(
                    POLICIES
                        .map(|(l, _)| nsh_core::mobile::SegmentedControlItem::new(l))
                        .to_vec(),
                )
                .selected_index(index)
                .on_change(move |i: &u32| {
                    if let Some((_, p)) = POLICIES.get(*i as usize) {
                        conflict.set(*p);
                    }
                })
                .self_stretch()
                .into_element()
        } else {
            view()
                .flex_row()
                .p(px(3.0))
                .rounded(px(12.0))
                .bg(move || theme().muted)
                .children(POLICIES.map(|(label, policy)| {
                    let selected = move || conflict.get() == policy;
                    button()
                        .selected(selected)
                        .accessibility_label(label)
                        .on_press(move || conflict.set(policy))
                        .flex_1()
                        .min_h(px(ui::hit() - 6.0))
                        .items_center()
                        .justify_center()
                        .rounded(px(9.0))
                        .bg(move || {
                            if selected() {
                                theme().card
                            } else {
                                theme().muted.opacity(0.0)
                            }
                        })
                        .child(
                            text(label)
                                .text_size(px(ui::size::SMALL))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || {
                                    if selected() {
                                        theme().foreground
                                    } else {
                                        theme().muted_foreground
                                    }
                                }),
                        )
                }))
                .into_element()
        })
}
