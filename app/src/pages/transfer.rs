//! A transfer's progress (FR-050 – FR-056), and the sheet that asks before
//! anything is received (FR-040 – FR-043).

use dexdrop_core::{
    api::{Command, IncomingRequest, TransferSnapshot, TransferState},
    format,
    naming::ConflictPolicy,
    protocol::TransferId,
    store::Direction,
};
use nsh_base::theme;
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

/// The keyboard on a transfer's page, while it is the page showing:
/// Space pauses or resumes, O opens what came in (the file, or the folder
/// of several on a computer).
fn keys(id: TransferId, s: impl Fn() -> TransferSnapshot + Copy + 'static) {
    let mine = crate::state::tab_untracked();
    let here = move || {
        crate::state::showing(mine, |r| match r {
            Route::Transfer(Some(x)) => *x == id,
            Route::Transfer(None) => find(None).is_some_and(|t| t.id == id),
            _ => false,
        })
    };
    on_key("space", move |_| {
        if !here() {
            return;
        }
        let t = s();
        if t.can_pause() {
            bridge::send(Command::Pause(id));
        } else if t.can_resume() && !t.state.is_finished() {
            bridge::send(Command::Resume(id));
        }
    });
    on_key("o", move |_| {
        if !here() {
            return;
        }
        let t = s();
        if t.direction != Direction::Incoming || !t.state.is_finished() {
            return;
        }
        match (t.saved_files.as_slice(), t.saved_to) {
            ([one], _) => crate::files::open(one.clone()),
            (_, Some(dir)) if desktop_platform() => {
                let shown = reveal_path(dir);
                spawn(async move {
                    let _ = shown.await;
                });
            }
            ([first, ..], _) => crate::files::open(first.clone()),
            _ => {}
        }
    });
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
    // What to show, rebuilt only when that changes (not on every byte): the
    // transfer, or, for the latest send to several devices, all of them.
    let shown = computed(move || {
        let first = find(id)?;
        let group = first.group.filter(|g| id.is_none() && group_size(*g) > 1);
        Some(match group {
            Some(g) => Shown::Group(g),
            None => Shown::One(first.id),
        })
    });
    body(dynamic(move || match shown.get() {
        None => ui::empty(ui::Art::Waiting, "Getting ready…").into_element(),
        Some(Shown::One(id)) => progress(id).into_element(),
        Some(Shown::Group(g)) => group_progress(g).into_element(),
    }))
}

#[derive(Clone, Copy, PartialEq)]
enum Shown {
    One(TransferId),
    Group(TransferId),
}

fn group_size(group: TransferId) -> usize {
    model().with(|m| {
        m.transfers
            .iter()
            .filter(|t| t.group == Some(group))
            .count()
    })
}

fn in_group(group: TransferId) -> Vec<TransferSnapshot> {
    model().with(|m| {
        m.transfers
            .iter()
            .filter(|t| t.group == Some(group))
            .cloned()
            .collect()
    })
}

/// The same files going to several devices: all of them together, then
/// each with its own progress (a tap for its details, Pause or Cancel).
fn group_progress(group: TransferId) -> impl IntoElement {
    let all = move || in_group(group);
    let overall = move || {
        let all = all();
        let total: u64 = all.iter().map(|t| t.total_bytes).sum();
        let done: u64 = all.iter().map(|t| t.bytes_done).sum();
        if total == 0 {
            0.0
        } else {
            done as f64 / total as f64
        }
    };
    let summary = move || {
        let all = all();
        let sent = all
            .iter()
            .filter(|t| t.state == TransferState::Completed)
            .count();
        format!("Sent to {sent} of {}", all.len())
    };
    let can_pause = move || all().iter().any(|t| t.can_pause());
    let can_resume = move || {
        all()
            .iter()
            .any(|t| t.can_resume() && !t.state.is_finished())
    };
    view()
        .gap(px(space::L))
        .pt(px(space::L))
        .child(
            view()
                .gap(px(space::S))
                .child(ui::title(summary))
                .child(ui::progress(overall, true).accessibility_label("All devices")),
        )
        .child(ui::group().child(each(
            all,
            |t: &TransferSnapshot| t.id,
            |t, i| {
                view()
                    .child(when(move || i.get() > 0, || ui::separator(16.0)))
                    .child(group_row(t))
            },
        )))
        .child(
            view()
                .flex_row()
                .justify_center()
                .gap(px(space::M))
                .child(when(can_pause, move || {
                    ui::icon_btn(Icon::Pause, "Pause all", Variant::Secondary)
                        .test_id("pause-all")
                        .on_press(move || {
                            for t in all().into_iter().filter(|t| t.can_pause()) {
                                bridge::send(Command::Pause(t.id));
                            }
                        })
                }))
                .child(when(can_resume, move || {
                    ui::icon_btn(Icon::Play, "Resume all", Variant::Primary)
                        .test_id("resume-all")
                        .on_press(move || {
                            for t in all()
                                .into_iter()
                                .filter(|t| t.can_resume() && !t.state.is_finished())
                            {
                                bridge::send(Command::Resume(t.id));
                            }
                        })
                })),
        )
}

/// One device of a group: its name, state and progress.
fn group_row(t: ReadSignal<TransferSnapshot>) -> impl IntoElement {
    let line = move || {
        t.with(|t| match t.state {
            TransferState::Transferring => format!(
                "{}% · {}",
                (t.progress() * 100.0).floor(),
                t.rate.map(dexdrop_core::format::rate).unwrap_or_default()
            ),
            ref other => short_state(other).to_owned(),
        })
    };
    ui::pressable()
        .accessibility_label(move || {
            Some(SharedString::from(t.with(|t| {
                format!("{}, {}", t.peer_name, short_state(&t.state))
            })))
        })
        .on_press(move || crate::state::push(Route::Transfer(Some(t.with_untracked(|t| t.id)))))
        .gap(px(4.0))
        .px(px(space::L))
        .py(px(space::M))
        .hover(move |s| s.bg(move || theme().muted.opacity(0.5)))
        .active(move |s| s.bg(move || theme().muted))
        .child(
            view()
                .flex_row()
                .justify_between()
                .gap(px(space::S))
                .child(ui::strong(move || t.with(|t| t.peer_name.clone())).number_of_lines(1))
                .child(muted(line).number_of_lines(1)),
        )
        .child(
            ui::progress(move || t.with(|t| t.progress()), false).accessibility_label(move || {
                Some(SharedString::from(t.with(|t| t.peer_name.clone())))
            }),
        )
}

fn progress(id: TransferId) -> impl IntoElement {
    let s = move || find(Some(id)).unwrap_or_default();
    keys(id, s);
    let done = move || s().state == TransferState::Completed;
    let finished = move || s().state.is_finished();
    // Screen readers hear each change of state (paused, reconnecting, done,
    // failed), never every byte (PRD 7.7).
    let last = std::cell::Cell::new(None::<&'static str>);
    effect(move || {
        let now = short_state(&s().state);
        if let Some(before) = last.replace(Some(now))
            && before != now
        {
            announce_for_accessibility(format!("{}: {now}", s().label));
        }
    });
    let outcome = computed(move || (done(), matches!(s().state, TransferState::Failed(_))));
    // Smaller on short screens, so the actions stay in view.
    let ring = if nsh_core::element::window().height.0 < 720.0 {
        168.0
    } else {
        220.0
    };
    view()
        .items_center()
        .gap(px(space::XL))
        .pt(px(space::L))
        .child(
            view()
                .size(px(ring))
                .items_center()
                .justify_center()
                .child(
                    ui::progress_ring(move || s().progress(), move || tone_of(&s().state), ring)
                        .absolute(),
                )
                // Only the outcome selects what shows (progress ticks do
                // not), so the result pops in once.
                .child(dynamic(move || {
                    let (finished, failed) = outcome.get();
                    if finished || failed {
                        let pop_in = motion().entering(MotionPreset::ZoomIn);
                        view()
                            .bind_motion(&pop_in)
                            .items_center()
                            .justify_center()
                            // The outcome's illustration, filling the ring.
                            .child(ui::art(
                                if failed {
                                    ui::Art::Failed
                                } else {
                                    ui::Art::Done
                                },
                                ring * 0.92,
                            ))
                            .into_element()
                    } else {
                        view()
                            .items_center()
                            .child(
                                text(move || format!("{}%", (s().progress() * 100.0).floor()))
                                    .text_size(px(ui::size::FIGURE))
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
                        .text_center()
                        .w_full(),
                )
                .child(
                    ui::pill(move || short_state(&s().state), move || tone_of(&s().state))
                        .self_center(),
                )
                // Detail the pill does not say: bytes and time, or why it ended.
                .child(when(
                    move || one_line(&s()) != short_state(&s().state),
                    move || muted(move || one_line(&s())).text_center().w_full(),
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
        .child(file_section(s))
}

/// The transfer's files, photos previewed: outgoing from the start,
/// incoming once saved. Rebuilt only when the list changes, not on every
/// progress update.
fn file_section(s: impl Fn() -> TransferSnapshot + Copy + 'static) -> impl IntoElement {
    let items = computed(move || s().items);
    dynamic(move || {
        let items = items.get();
        (!items.is_empty()).then(|| {
            let total = s().files.max(items.len() as u64);
            view()
                .self_stretch()
                .gap(px(space::S))
                .child(ui::section(if total == 1 { "File" } else { "Files" }))
                .child(crate::files::list(items, total))
        })
    })
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
        .child(ui::avatar(name, platform, move || s().peer_avatar, 44.0))
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
                .child(ui::strong(name).number_of_lines(1))
                // How the data goes; a relay is always said (PRD 9.5).
                .child(dynamic(move || {
                    s().route.map(|route| {
                        let (words, i) = ui::route_label(route);
                        view()
                            .flex_row()
                            .items_center()
                            .gap(px(space::XS))
                            .child(ui::icon(i, 13.0, move || theme().muted_foreground))
                            .child(muted(words).text_size(px(ui::size::CAPTION)))
                    })
                })),
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
        .style_when(ui::large_text, |s| {
            s.flex_col().items_center().gap(px(space::M))
        })
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
                control(Icon::X, "Stop", Variant::Destructive, move || {
                    ui::confirm(
                        "Stop this transfer?",
                        "What has arrived so far stays; the rest is not sent.",
                        "Stop",
                        move || bridge::send(Command::Cancel(id)),
                    )
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
    ui::captioned_btn(i, label, variant)
        .test_id(&label.to_ascii_lowercase())
        .on_press(on_press)
}

/// After a transfer: share or show what came in, and leave.
fn after(s: impl Fn() -> TransferSnapshot + Copy + 'static) -> impl IntoElement {
    let first_video = move || {
        s().items
            .iter()
            .filter(|i| crate::files::is_video(i.name()))
            .find_map(|i| i.local.clone().filter(|p| p.exists()))
    };
    view()
        .self_stretch()
        .gap(px(space::S))
        .child(when(
            move || first_video().is_some(),
            move || {
                ui::icon_btn(Icon::Play, "Play video", Variant::Primary).on_press(move || {
                    if let Some(path) = first_video() {
                        crate::files::open(path);
                    }
                })
            },
        ))
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
        .child(
            ui::icon_btn(Icon::Check, "Done", Variant::Secondary)
                .test_id("done")
                .on_press(|| crate::state::reset_to(Route::Root)),
        )
}

// ---------------------------------------------------------------- requests

/// The first request waiting for an answer, in a sheet over whatever is on
/// screen. Only which request is first selects the sheet's content, so the
/// model's other changes (progress, peers) leave it and the user's choices
/// in it (trust, conflicts, options) alone. Escape and back decline it.
pub fn request_overlay() -> impl IntoElement {
    let shown = computed(move || model().with(|m| m.requests.first().map(|r| r.id)));
    ui::sheet(
        "Incoming files",
        move || shown.get().is_some(),
        move || {
            if let Some(id) = shown.get_untracked() {
                decline(id);
            }
        },
        dynamic(move || {
            let id = shown.get()?;
            model()
                .with_untracked(|m| m.requests.iter().find(|r| r.id == id).cloned())
                .map(request_sheet)
        }),
    )
}

/// Says no to request `id`.
fn decline(id: TransferId) {
    bridge::send(Command::Respond {
        id,
        accept: false,
        trust: false,
        conflict: ConflictPolicy::KeepBoth,
    });
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
            ui::confirm(
                "Replace existing files?",
                "Files with the same names will be overwritten.",
                "Replace",
                move || respond(true),
            );
        } else {
            respond(true);
        }
    };
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
    let (n1, p1, a1) = (
        field(|r| r.peer_name.clone()),
        field(|r| r.peer_platform.clone()),
        move || {
            model().with(|m| {
                m.requests
                    .iter()
                    .find(|x| x.id == id)
                    .and_then(|r| r.peer_avatar.clone())
            })
        },
    );
    let enough = r.enough_space();
    let trusted = r.trusted;
    view()
        .w_full()
        .max_w(px(520.0))
        .p(px(space::XL))
        .pb(px(if ui::mobile() { 40.0 } else { space::XL }))
        .gap(px(space::L))
        .rounded_t(px(ui::radius::xxl()))
        .rounded_b(px(if ui::mobile() { 0.0 } else { ui::radius::xxl() }))
        .mb(px(if ui::mobile() { 0.0 } else { space::XL }))
        .bg(move || theme().card)
        .shadow_lg()
        .child(
            view()
                .items_center()
                .gap(px(space::S))
                .child(ui::avatar(n1, p1, a1, 72.0))
                .child(ui::title(name.clone()).text_center().w_full())
                .child(
                    muted(format!(
                        "wants to send {} · {}",
                        r.label,
                        format::bytes(r.total_bytes)
                    ))
                    .text_center()
                    .w_full(),
                )
                // Where it goes, before accepting (FR-041).
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap(px(space::XS))
                        .child(ui::icon(Icon::Folder, 14.0, move || {
                            theme().muted_foreground
                        }))
                        .child(
                            muted(crate::pages::settings::saves_to())
                                .text_size(px(ui::size::CAPTION)),
                        ),
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
                    move || r.route != dexdrop_core::api::Route::Nearby,
                    move || ui::pill(ui::route_label(r.route).0, || Tone::Accent),
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
                .rounded(px(ui::radius::lg()))
                .bg(move || theme().muted)
                .child(muted("Same number on both screens?"))
                .child(
                    text(r.sas.clone())
                        .text_size(px(ui::size::HEADLINE))
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
                                ui::glyph(Icon::UserCheck, 28.0),
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
                ui::icon_btn(Icon::Sliders, "Options", Variant::Plain)
                    .self_center()
                    .on_press(move || options.set(true))
            },
        ))
        .child(
            view()
                .flex_row()
                .gap(px(space::M))
                .child(
                    ui::icon_btn(Icon::X, "Decline", Variant::Secondary)
                        .test_id("decline")
                        .flex_1()
                        .on_press(move || respond(false)),
                )
                .child(
                    ui::icon_btn(Icon::Receive, "Accept", Variant::Primary)
                        .test_id("accept")
                        .flex_1()
                        .disabled(move || !enough)
                        .on_press(accept),
                ),
        )
}

/// What to do with a file that already exists: the system's segmented
/// control on iOS, NSH's toggle group elsewhere ([`ui::segmented`]).
fn conflict_picker(conflict: Signal<ConflictPolicy>) -> impl IntoElement {
    const POLICIES: [(&str, ConflictPolicy); 3] = [
        ("Keep both", ConflictPolicy::KeepBoth),
        ("Skip", ConflictPolicy::Skip),
        ("Replace", ConflictPolicy::Replace),
    ];
    view()
        .px(px(space::L))
        .py(px(space::M))
        .gap(px(space::S))
        .child(muted("If a file is already there"))
        .child(ui::segmented(&POLICIES, conflict))
}
