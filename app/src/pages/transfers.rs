//! Transfers: requests waiting for an answer, active and recent transfers
//! (FR-040 – FR-060, PRD 7.4).

use dexdrop_core::{
    api::{Command, IncomingRequest, TransferSnapshot, TransferState},
    format,
    naming::ConflictPolicy,
    store::{Direction, HistoryEntry, Outcome},
};
use nsh_base::{alert_dialog, theme};
use nsh_core::prelude::*;

use crate::{
    bridge,
    state::{desktop_platform, model, platform_label},
    ui::{self, Tone, Variant, badge, btn, card, muted, section_label, space},
};

pub fn page() -> impl IntoElement {
    let has_requests = move || model().with(|m| !m.requests.is_empty());
    let has_transfers = move || model().with(|m| !m.transfers.is_empty());
    let has_history = move || model().with(|m| !m.history.is_empty());
    let has_finished = move || model().with(|m| m.transfers.iter().any(|t| t.state.is_finished()));
    view()
        .gap(px(space::XL))
        .child(ui::page_header("Transfers", subtitle))
        .child(when(has_requests, || {
            view()
                .gap(px(space::M))
                .child(section_label("Waiting for you"))
                .child(each(
                    move || model().with(|m| m.requests.clone()),
                    |r: &IncomingRequest| r.id,
                    |r, _| request_card(r),
                ))
        }))
        .child(
            view()
                .gap(px(space::M))
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .child(section_label("This session"))
                        .child(when(has_finished, || {
                            btn("Clear finished", Variant::Ghost)
                                .on_press(|| bridge::send(Command::ClearFinished))
                        })),
                )
                .child(
                    when(has_transfers, || {
                        view().gap(px(space::S)).child(each(
                            move || model().with(|m| m.transfers.clone()),
                            |t: &TransferSnapshot| t.id,
                            |t, _| transfer_card(t),
                        ))
                    })
                    .otherwise(|| {
                        card().items_center().py(px(space::XL)).child(muted(
                            "Nothing yet. Transfers you send or receive show up here.",
                        ))
                    }),
                ),
        )
        .child(when(has_history, history))
}

fn subtitle() -> String {
    model().with(|m| {
        let active = m
            .transfers
            .iter()
            .filter(|t| !t.state.is_finished() && t.state != TransferState::Requested)
            .count();
        match (m.requests.len(), active) {
            (0, 0) => "Files go directly between your devices, encrypted and verified.".into(),
            (0, n) => format!("{n} in progress"),
            (r, n) => format!("{r} waiting for your answer · {n} in progress"),
        }
    })
}

// ---------------------------------------------------------------- requests

fn request_card(request: ReadSignal<IncomingRequest>) -> impl IntoElement {
    let r = move || request.get();
    let trust = signal(false);
    let conflict = signal(ConflictPolicy::KeepBoth);
    let respond = move |accept: bool| {
        let r = request.get_untracked();
        bridge::send(Command::Respond {
            id: r.id,
            accept,
            trust: trust.get_untracked(),
            conflict: conflict.get_untracked(),
        });
    };
    let accept = move || {
        if conflict.get_untracked() == ConflictPolicy::Replace {
            // Replacing files is destructive: confirm explicitly (FR-043).
            let answer = alert_dialog()
                .title("Replace existing files?")
                .description(
                    "Files with the same names in the destination folder will be overwritten.",
                )
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
    card()
        .border_color(move || theme().primary)
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(space::M))
                .child(ui::avatar(move || r().peer_name, 40.0))
                .child(
                    view()
                        .flex_1()
                        .gap(px(2.0))
                        .child(
                            ui::body(move || {
                                format!("{} wants to send {}", r().peer_name, r().label)
                            })
                            .font_weight(FontWeight::SEMIBOLD),
                        )
                        .child(muted(move || {
                            let r = r();
                            format!(
                                "{} · {} · {}",
                                platform_label(&r.peer_platform),
                                format::item_count(r.files, r.folders),
                                format::bytes(r.total_bytes)
                            )
                        })),
                ),
        )
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap(px(space::S))
                .child(when(
                    move || r().trusted,
                    || badge("Trusted device", Tone::Success),
                ))
                .child(when(
                    move || r().via_invitation,
                    || badge("Used your code", Tone::Accent),
                ))
                .child(when(
                    move || !r().enough_space(),
                    || badge("Not enough free space", Tone::Danger),
                )),
        )
        .child(
            view()
                .p(px(space::M))
                .gap(px(2.0))
                .rounded(px(8.0))
                .bg(move || theme().muted)
                .child(muted(
                    "Verification number — it must match the other screen",
                ))
                .child(
                    text(move || r().sas)
                        .text_size(px(22.0))
                        .font_weight(FontWeight::BOLD)
                        .letter_spacing(px(1.5))
                        .text_color(move || theme().foreground),
                )
                .child(muted(move || format!("Device ID {}", r().fingerprint))),
        )
        .child(muted(move || format!("Saves to {}", r().destination.display())).number_of_lines(2))
        .child(conflict_picker(conflict))
        .child(when(
            move || !r().trusted,
            move || {
                ui::switch_row(
                    "Trust this device",
                    "Remember it. You can allow it to send without asking in Settings.",
                    move || trust.get(),
                    move |on| trust.set(on),
                )
            },
        ))
        .child(
            view()
                .flex_row()
                .gap(px(space::S))
                .justify_end()
                .child(btn("Decline", Variant::Secondary).on_press(move || respond(false)))
                .child(
                    btn("Accept", Variant::Primary)
                        .disabled(move || !r().enough_space())
                        .on_press(accept),
                ),
        )
}

fn conflict_picker(conflict: Signal<ConflictPolicy>) -> impl IntoElement {
    let option = move |label: &'static str, policy: ConflictPolicy| {
        let selected = move || conflict.get() == policy;
        nsh_base::button()
            .selected(selected)
            .accessibility_label(label)
            .on_press(move || conflict.set(policy))
            .px(px(space::M))
            .min_h(px(ui::hit() - 6.0))
            .justify_center()
            .rounded(px(8.0))
            .bg(move || {
                if selected() {
                    theme().card
                } else {
                    theme().muted
                }
            })
            .border_1()
            .border_color(move || {
                if selected() {
                    theme().primary
                } else {
                    theme().muted
                }
            })
            .child(
                text(label)
                    .text_size(px(ui::size::SMALL))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(move || {
                        if selected() {
                            theme().primary
                        } else {
                            theme().muted_foreground
                        }
                    }),
            )
    };
    view()
        .gap(px(space::XS))
        .child(muted("If a file already exists"))
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap(px(space::XS))
                .child(option("Keep both", ConflictPolicy::KeepBoth))
                .child(option("Skip", ConflictPolicy::Skip))
                .child(option("Replace", ConflictPolicy::Replace)),
        )
}

// ---------------------------------------------------------------- transfers

fn tone_of(state: &TransferState) -> Tone {
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

fn status_line(t: &TransferSnapshot) -> String {
    match &t.state {
        TransferState::Transferring => {
            let mut line = format!(
                "{} of {}",
                format::bytes(t.bytes_done),
                format::bytes(t.total_bytes)
            );
            if let Some(rate) = t.rate {
                line += &format!(" · {}", format::rate(rate));
            }
            if let Some(eta) = t.eta {
                line += &format!(" · {} left", format::duration(eta));
            }
            line
        }
        TransferState::Completed => match t.direction {
            Direction::Incoming => format!(
                "{} received and verified",
                format::item_count(t.files, t.folders)
            ),
            Direction::Outgoing => format!(
                "{} sent and verified",
                format::item_count(t.files, t.folders)
            ),
        },
        TransferState::CompletedWithIssues => format!(
            "{} done · {} failed · {} skipped",
            t.files_done, t.files_failed, t.files_skipped
        ),
        state => state.label(),
    }
}

fn transfer_card(t: ReadSignal<TransferSnapshot>) -> impl IntoElement {
    let s = move || t.get();
    let id = move || t.with_untracked(|t| t.id);
    let arrow = move || {
        if s().direction == Direction::Outgoing {
            "To"
        } else {
            "From"
        }
    };
    card()
        .p(px(space::M))
        .gap(px(space::S))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(space::M))
                .child(
                    view()
                        .flex_1()
                        .gap(px(2.0))
                        .child(
                            ui::body(move || s().label)
                                .font_weight(FontWeight::SEMIBOLD)
                                .number_of_lines(1),
                        )
                        .child(muted(move || {
                            let s = s();
                            let peer = if s.peer_name.is_empty() {
                                "…".to_owned()
                            } else {
                                s.peer_name.clone()
                            };
                            format!("{} {peer} · {}", arrow(), format::bytes(s.total_bytes))
                        })),
                )
                .child(dynamic(move || {
                    let state = s().state;
                    badge(short_state(&state), tone_of(&state))
                })),
        )
        .child(ui::progress_bar(
            move || s().progress(),
            move || tone_of(&s().state),
        ))
        .child(muted(move || status_line(&s())).number_of_lines(2))
        .child(when(
            move || s().sas.is_some() && !s().state.is_finished(),
            move || muted(move || format!("Verification number {}", s().sas.unwrap_or_default())),
        ))
        .child(dynamic(move || {
            let failures = s().failures;
            (!failures.is_empty()).then(|| {
                view()
                    .gap(px(2.0))
                    .children(failures.into_iter().take(5).map(|(path, why)| {
                        muted(format!("{path}: {why}")).text_color(move || theme().destructive)
                    }))
            })
        }))
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap(px(space::S))
                .justify_end()
                .child(when(
                    move || s().can_pause(),
                    move || {
                        btn("Pause", Variant::Secondary)
                            .on_press(move || bridge::send(Command::Pause(id())))
                    },
                ))
                .child(when(
                    move || s().can_resume(),
                    move || {
                        let label = if matches!(
                            s().state,
                            TransferState::Failed(_) | TransferState::Interrupted { .. }
                        ) {
                            "Retry"
                        } else {
                            "Resume"
                        };
                        btn(label, Variant::Primary)
                            .on_press(move || bridge::send(Command::Resume(id())))
                    },
                ))
                .child(when(
                    move || s().can_cancel(),
                    move || {
                        btn("Cancel", Variant::Danger)
                            .on_press(move || bridge::send(Command::Cancel(id())))
                    },
                ))
                .child(when(
                    move || {
                        desktop_platform()
                            && s().direction == Direction::Incoming
                            && s().state == TransferState::Completed
                    },
                    move || {
                        btn("Show in folder", Variant::Secondary).on_press(move || {
                            if let Some(dir) = s().saved_to {
                                let shown = reveal_path(dir);
                                spawn(async move {
                                    let _ = shown.await;
                                });
                            }
                        })
                    },
                ))
                .child(when(
                    move || s().can_dismiss(),
                    move || {
                        btn("Dismiss", Variant::Ghost)
                            .on_press(move || bridge::send(Command::Dismiss(id())))
                    },
                )),
        )
}

fn short_state(state: &TransferState) -> &'static str {
    match state {
        TransferState::Preparing => "Preparing",
        TransferState::Connecting => "Connecting",
        TransferState::Waiting => "Waiting",
        TransferState::Requested => "Needs answer",
        TransferState::Transferring => "Transferring",
        TransferState::Paused { .. } => "Paused",
        TransferState::Interrupted { .. } => "Interrupted",
        TransferState::Completed => "Done",
        TransferState::CompletedWithIssues => "Done with issues",
        TransferState::Declined(_) => "Declined",
        TransferState::Cancelled { .. } => "Cancelled",
        TransferState::Failed(_) => "Failed",
    }
}

// ---------------------------------------------------------------- history

fn history() -> impl IntoElement {
    view()
        .gap(px(space::M))
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .child(section_label("Recent"))
                .child(
                    btn("Clear history", Variant::Ghost)
                        .on_press(|| bridge::send(Command::ClearHistory)),
                ),
        )
        .child(card().p(px(space::S)).gap(px(0.0)).child(each(
            move || model().with(|m| m.history.iter().take(30).cloned().collect::<Vec<_>>()),
            |h: &HistoryEntry| h.transfer_id.clone(),
            |h, _| history_row(h),
        )))
}

fn history_row(h: ReadSignal<HistoryEntry>) -> impl IntoElement {
    let e = move || h.get();
    view()
        .flex_row()
        .items_center()
        .gap(px(space::M))
        .px(px(space::S))
        .py(px(space::S))
        .child(
            view()
                .flex_1()
                .gap(px(2.0))
                .child(ui::body(move || e().label).number_of_lines(1))
                .child(muted(move || {
                    let e = e();
                    let dir = if e.direction == Direction::Outgoing {
                        "To"
                    } else {
                        "From"
                    };
                    format!("{dir} {} · {}", e.peer_name, format::bytes(e.bytes))
                })),
        )
        .child(dynamic(move || {
            let (label, tone) = match e().outcome {
                Outcome::Completed => ("Done", Tone::Success),
                Outcome::Partial { .. } => ("Partial", Tone::Warning),
                Outcome::Declined => ("Declined", Tone::Neutral),
                Outcome::Cancelled => ("Cancelled", Tone::Neutral),
                Outcome::Failed(_) => ("Failed", Tone::Danger),
            };
            badge(label, tone)
        }))
}
