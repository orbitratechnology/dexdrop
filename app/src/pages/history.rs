//! History: transfers running now, then what was sent and received.

use dexdrop_core::{
    api::{Command, TransferSnapshot},
    store::{Direction, HistoryEntry, Outcome},
};
use nsh_base::theme;
use nsh_core::prelude::*;

use crate::state::desktop_platform;

use crate::{
    bridge,
    pages::{body, home, transfer},
    state::{Route, model, push},
    ui::{self, Icon, Tone, Variant, muted, space},
};

/// Which transfers to list.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter {
    All,
    Sent,
    Received,
}

impl Filter {
    const ALL: [(Filter, &'static str); 3] = [
        (Filter::All, "All"),
        (Filter::Sent, "Sent"),
        (Filter::Received, "Received"),
    ];

    fn keeps(self, direction: Direction) -> bool {
        match self {
            Filter::All => true,
            Filter::Sent => direction == Direction::Outgoing,
            Filter::Received => direction == Direction::Incoming,
        }
    }
}

pub fn page() -> impl IntoElement {
    let filter = signal(Filter::All);
    let active = move || {
        model().with(|m| {
            m.transfers
                .iter()
                .filter(|t| filter.get().keeps(t.direction))
                .cloned()
                .collect::<Vec<_>>()
        })
    };
    let active = computed(active);
    let past = move || {
        model().with(|m| {
            let live: Vec<String> = m
                .transfers
                .iter()
                .map(|t| dexdrop_core::hex(&t.id))
                .collect();
            m.history
                .iter()
                .filter(|h| filter.get().keeps(h.direction) && !live.contains(&h.transfer_id))
                .cloned()
                .collect::<Vec<_>>()
        })
    };
    let past = computed(past);
    let nothing = move || active.with(Vec::is_empty) && past.with(Vec::is_empty);
    body(
        view()
            .gap(px(space::XL))
            .child(filter_bar(filter))
            .child(when(
                move || !active.with(Vec::is_empty),
                move || {
                    view()
                        .gap(px(space::M))
                        .child(ui::section("This session"))
                        .child(
                            ui::group().child(
                                // Virtualized (NSH lists): only the rows near the
                                // screen exist, in the page's scroll view.
                                virtual_list(
                                    move || active.with(Vec::len),
                                    move |at| {
                                        session_row(move || {
                                            active.with(|a| a.get(at.index()).cloned())
                                        })
                                    },
                                )
                                .keys(move || {
                                    active.with(|a| a.iter().map(|t| t.id).collect::<Vec<_>>())
                                })
                                .separator(|| ui::separator(68.0))
                                .estimated_item_size(px(68.0))
                                .viewport(ListViewport::Parent),
                            ),
                        )
                },
            ))
            .child(when(
                move || !past.with(Vec::is_empty),
                move || {
                    view()
                        .gap(px(space::M))
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .child(ui::section("Earlier"))
                                .child(ui::btn("Clear", Variant::Plain).on_press(|| {
                                    bridge::send(Command::ClearHistory);
                                    bridge::send(Command::ClearFinished);
                                })),
                        )
                        .child(
                            ui::group().child(
                                virtual_list(
                                    move || past.with(Vec::len),
                                    move |at| {
                                        row(move || past.with(|p| p.get(at.index()).cloned()))
                                    },
                                )
                                .keys(move || {
                                    past.with(|p| {
                                        p.iter().map(|h| h.transfer_id.clone()).collect::<Vec<_>>()
                                    })
                                })
                                .separator(|| ui::separator(68.0))
                                .estimated_item_size(px(68.0))
                                .viewport(ListViewport::Parent),
                            ),
                        )
                },
            ))
            .child(when(nothing, || {
                ui::empty(Icon::Clock, "Files you send and receive show up here.")
            })),
    )
}

/// All / Sent / Received: the system's segmented control on iOS, the app's
/// own elsewhere.
fn filter_bar(filter: Signal<Filter>) -> impl IntoElement {
    let index = move || {
        Filter::ALL
            .iter()
            .position(|(f, _)| *f == filter.get())
            .unwrap_or(0) as u32
    };
    if os().name() == "ios" {
        nsh_core::mobile::segmented_control()
            .items(
                Filter::ALL
                    .map(|(_, l)| nsh_core::mobile::SegmentedControlItem::new(l))
                    .to_vec(),
            )
            .selected_index(index)
            .on_change(move |i: &u32| {
                if let Some((f, _)) = Filter::ALL.get(*i as usize) {
                    filter.set(*f);
                }
            })
            .self_stretch()
            .into_element()
    } else {
        view()
            .flex_row()
            .self_start()
            .p(px(3.0))
            .rounded(px(12.0))
            .bg(ui::surface)
            .children(Filter::ALL.map(|(f, label)| {
                let selected = move || filter.get() == f;
                ui::pressable()
                    .selected(selected)
                    .accessibility_label(label)
                    .on_press(move || filter.set(f))
                    .px(px(space::L))
                    .min_h(px(ui::hit() - 4.0))
                    .justify_center()
                    .rounded(px(9.0))
                    .bg(move || {
                        if selected() {
                            theme().primary
                        } else {
                            theme().card.opacity(0.0)
                        }
                    })
                    .child(
                        text(label)
                            .text_size(px(ui::size::SMALL))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(move || {
                                if selected() {
                                    theme().primary_foreground
                                } else {
                                    theme().muted_foreground
                                }
                            }),
                    )
            }))
            .into_element()
    }
}

/// A transfer of this session: its progress, a tap from its details.
fn session_row(t: impl Fn() -> Option<TransferSnapshot> + Copy + 'static) -> impl IntoElement {
    // A recycled cell shows another row: everything reads the row anew.
    let s = move || t().unwrap_or_default();
    ui::pressable()
        .accessibility_label(move || Some(SharedString::from(s().label)))
        .on_press(move || {
            if let Some(t) = t() {
                push(Route::Transfer(Some(t.id)));
            }
        })
        .flex_row()
        .items_center()
        .gap(px(space::M))
        .px(px(space::L))
        .py(px(space::M))
        .active(move |st| st.bg(move || theme().muted))
        .child(dynamic(move || {
            let state = s().state;
            if state.is_finished() {
                let (mark, tone) = transfer::mark(&state);
                ui::soft_glyph(mark, move || ui::tone_color(tone), 40.0).into_element()
            } else {
                ui::progress_ring(
                    move || s().progress(),
                    move || transfer::tone_of(&s().state),
                    40.0,
                )
                .into_element()
            }
        }))
        .child(
            view()
                .flex_1()
                .gap(px(2.0))
                .child(ui::strong(move || s().label).number_of_lines(1))
                .child(
                    muted(move || {
                        let s = s();
                        let dir = if s.direction == Direction::Outgoing {
                            "To"
                        } else {
                            "From"
                        };
                        format!("{dir} {} · {}", s.peer_name, transfer::one_line(&s))
                    })
                    .number_of_lines(1),
                ),
        )
        .child(ui::icon(Icon::ChevronRight, 18.0, move || {
            theme().muted_foreground
        }))
}

/// A past transfer: a tap shows its details. Reads `entry` in every
/// binding, so a recycled list cell shows its new row.
pub fn row(entry: impl Fn() -> Option<HistoryEntry> + Copy + 'static) -> impl IntoElement {
    let field =
        move |f: fn(&HistoryEntry) -> String| move || entry().map(|e| f(&e)).unwrap_or_default();
    ui::pressable()
        .accessibility_label(move || entry().map(|e| SharedString::from(e.label)))
        .on_press(move || {
            if let Some(e) = entry() {
                push(Route::Past(e.transfer_id));
            }
        })
        .active(move |s| s.bg(move || theme().muted))
        .hover(move |s| s.bg(move || theme().muted.opacity(0.6)))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(space::M))
                .px(px(space::L))
                .py(px(space::M))
                .child(dynamic(move || entry().map(row_picture)))
                .child(
                    view()
                        .flex_1()
                        .gap(px(2.0))
                        .child(ui::body(field(|e| e.label.clone())).number_of_lines(1))
                        .child(
                            muted(field(|e| {
                                format!("{} · {}", home::summary(e), ago(e.finished))
                            }))
                            .number_of_lines(1),
                        ),
                )
                .child(dynamic(move || {
                    entry().map(|e| {
                        let (mark, tone) = match e.outcome {
                            Outcome::Completed => (Icon::Check, Tone::Success),
                            Outcome::Partial { .. } => (Icon::Alert, Tone::Warning),
                            Outcome::Declined | Outcome::Cancelled => (Icon::X, Tone::Neutral),
                            Outcome::Failed(_) => (Icon::Alert, Tone::Danger),
                        };
                        ui::icon(mark, 18.0, move || ui::tone_color(tone))
                    })
                }))
                .child(ui::icon(Icon::ChevronRight, 16.0, move || {
                    theme().muted_foreground
                })),
        )
}

/// A past transfer's picture: its first photo when still on this device,
/// else its kind, with which way it went in the corner.
fn row_picture(e: HistoryEntry) -> View {
    let outgoing = e.direction == Direction::Outgoing;
    let preview = e
        .items
        .iter()
        .find_map(|i| crate::files::preview_of(i).map(|p| (i.name().to_owned(), p)));
    view()
        .child(match &preview {
            Some((name, p)) => crate::files::thumb(name, Some(p), 40.0, 10.0),
            None => ui::file_badge(&e.label, 40.0, 10.0),
        })
        .child(
            view()
                .absolute()
                .right(px(-4.0))
                .bottom(px(-4.0))
                .size(px(20.0))
                .rounded(px(10.0))
                .items_center()
                .justify_center()
                .bg(rgb(if outgoing { ui::SEND.0 } else { ui::RECEIVE.0 }))
                .border_2()
                .border_color(ui::surface)
                .child(ui::icon(
                    if outgoing { Icon::Send } else { Icon::Receive },
                    11.0,
                    white(),
                )),
        )
}

// ---------------------------------------------------------------- details

fn outcome_of(o: &Outcome) -> (&'static str, Tone) {
    match o {
        Outcome::Completed => ("Completed", Tone::Success),
        Outcome::Partial { .. } => ("Completed with issues", Tone::Warning),
        Outcome::Declined => ("Declined", Tone::Neutral),
        Outcome::Cancelled => ("Cancelled", Tone::Neutral),
        Outcome::Failed(_) => ("Failed", Tone::Danger),
    }
}

/// A past transfer in full: what, with whom, when, how fast, which way,
/// and its files.
pub fn past_page(id: String) -> impl IntoElement {
    let entry = model().with_untracked(|m| m.history.iter().find(|h| h.transfer_id == id).cloned());
    body(match entry {
        None => ui::empty(Icon::Clock, "This transfer is no longer in the history.").into_element(),
        Some(e) => details(e).into_element(),
    })
}

fn details(e: HistoryEntry) -> impl IntoElement {
    let outgoing = e.direction == Direction::Outgoing;
    let (status, tone) = outcome_of(&e.outcome);
    let photos = crate::files::photo_strip(&e.items, 6);
    let video = e
        .items
        .iter()
        .filter(|i| crate::files::is_video(i.name()))
        .find_map(|i| i.local.clone().filter(|p| p.exists()));
    let size_line = {
        let mut parts = vec![dexdrop_core::format::bytes(e.bytes)];
        parts.push(if e.files == 1 {
            "1 file".to_owned()
        } else {
            format!("{} files", e.files)
        });
        parts.join(" · ")
    };
    let hero = ui::tile()
        .items_center()
        .gap(px(space::M))
        .py(px(space::XL))
        .child(match photos {
            Some(strip) => strip.into_element(),
            None => view()
                .child(ui::file_badge(&e.label, 88.0, 24.0))
                .child(
                    view()
                        .absolute()
                        .right(px(-4.0))
                        .bottom(px(-4.0))
                        .size(px(32.0))
                        .rounded(px(16.0))
                        .items_center()
                        .justify_center()
                        .bg(rgb(if outgoing { ui::SEND.0 } else { ui::RECEIVE.0 }))
                        .border_2()
                        .border_color(ui::surface)
                        .child(ui::icon(
                            if outgoing { Icon::Send } else { Icon::Receive },
                            16.0,
                            white(),
                        )),
                )
                .into_element(),
        })
        .child(ui::title(e.label.clone()).text_center().number_of_lines(2))
        .child(ui::pill(status, move || tone).self_center())
        .child(muted(size_line))
        .children(video.map(|path| {
            ui::icon_btn(Icon::Play, "Play video", Variant::Primary)
                .on_press(move || crate::files::open(path.clone()))
        }));

    let (name, platform, avatar) = (
        e.peer_name.clone(),
        e.peer_platform.clone(),
        e.peer_avatar.clone(),
    );
    let peer = ui::tile()
        .flex_row()
        .items_center()
        .gap(px(space::M))
        .child({
            let who = signal((name.clone(), platform.clone(), avatar.clone()));
            ui::avatar(
                move || who.with(|w| w.0.clone()),
                move || who.with(|w| w.1.clone()),
                move || who.with(|w| w.2.clone()),
                48.0,
            )
        })
        .child(
            view()
                .flex_1()
                .gap(px(2.0))
                .child(
                    muted(if outgoing { "Sent to" } else { "Received from" })
                        .text_size(px(ui::size::CAPTION)),
                )
                .child(ui::strong(name.clone()).number_of_lines(1))
                .child(
                    muted(crate::state::platform_label(&platform).to_owned())
                        .text_size(px(ui::size::CAPTION)),
                ),
        );

    let mut facts: Vec<(Icon, (u32, u32), &'static str, String)> = vec![(
        Icon::Clock,
        (0x6366f1, 0x8b5cf6),
        if outgoing { "Sent" } else { "Received" },
        when_text(e.finished),
    )];
    if let Some(started) = e.started.filter(|s| *s <= e.finished) {
        facts.push((
            Icon::Refresh,
            (0x0ea5e9, 0x3b82f6),
            "Took",
            dexdrop_core::format::duration((e.finished - started).max(1)),
        ));
    }
    if let Some(rate) = e.average_rate {
        facts.push((
            Icon::Send,
            (0x10b981, 0x14b8a6),
            "Average speed",
            dexdrop_core::format::rate(rate),
        ));
    }
    if let Some(route) = e.route {
        let (label, icon) = ui::route_label(route);
        facts.push((icon, (0xf59e0b, 0xf97316), "Connection", label.to_owned()));
    }
    match &e.outcome {
        Outcome::Partial { failed, skipped } => {
            let mut parts = Vec::new();
            if *failed > 0 {
                parts.push(format!("{failed} failed"));
            }
            if *skipped > 0 {
                parts.push(format!("{skipped} skipped"));
            }
            facts.push((
                Icon::Alert,
                (0xf59e0b, 0xea580c),
                "Issues",
                parts.join(", "),
            ));
        }
        Outcome::Failed(reason) => {
            facts.push((Icon::Alert, (0xf43f5e, 0xe11d48), "Reason", reason.clone()));
        }
        _ => {}
    }
    facts.push((
        Icon::Shield,
        (0x8b5cf6, 0x7c3aed),
        "Encryption",
        "End to end".to_owned(),
    ));
    let info = ui::group().children(facts.into_iter().enumerate().map(
        |(i, (icon, colors, label, value))| {
            view()
                .child(when(move || i > 0, || ui::separator(58.0)))
                .child(fact(icon, colors, label, value))
        },
    ));

    let saved_to = e.saved_to.clone().filter(|_| !outgoing);
    let folder = saved_to.clone().map(|dir| {
        let label = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| dir.to_string_lossy().into_owned());
        ui::group().child(ui::nav_row(
            ui::glyph(Icon::Folder, (0xf59e0b, 0xea580c), 30.0),
            "Saved in",
            label,
            move || {
                if desktop_platform() {
                    let shown = reveal_path(dir.clone());
                    spawn(async move {
                        let _ = shown.await;
                    });
                }
            },
        ))
    });

    let items = e.items.clone();
    let files = (!items.is_empty()).then(|| {
        view()
            .gap(px(space::S))
            .child(ui::section(if e.files == 1 { "File" } else { "Files" }))
            .child(crate::files::list(items, e.files))
    });

    view()
        .gap(px(space::XL))
        .child(hero)
        .child(peer)
        .child(
            view()
                .gap(px(space::S))
                .child(ui::section("Details"))
                .child(info),
        )
        .children(folder)
        .children(files)
}

/// A fact about a transfer: its glyph, what it is, and its value.
fn fact(icon: Icon, colors: (u32, u32), label: &'static str, value: String) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap(px(space::M))
        .min_h(px(ui::hit() + 8.0))
        .px(px(space::L))
        .py(px(space::S))
        .child(ui::glyph(icon, colors, 30.0))
        .child(ui::body(label).flex_1())
        .child(
            muted(value)
                .number_of_lines(2)
                .max_w(px(220.0))
                .text_right(),
        )
}

/// "Today, 14:05", "Yesterday, 09:12", "Mon 3 Nov 2026, 18:40" in local time.
pub fn when_text(unix: u64) -> String {
    use chrono::{Local, TimeZone};
    let Some(t) = Local.timestamp_opt(unix as i64, 0).single() else {
        return String::new();
    };
    let today = Local::now().date_naive();
    let day = t.date_naive();
    let time = t.format("%H:%M");
    if day == today {
        format!("Today, {time}")
    } else if today.pred_opt() == Some(day) {
        format!("Yesterday, {time}")
    } else {
        format!("{}, {time}", t.format("%a %-d %b %Y"))
    }
}

/// "Just now", "5 min ago", "Yesterday", "3 days ago".
pub fn ago(when: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(when);
    let s = now.saturating_sub(when);
    match s {
        0..60 => "Just now".into(),
        60..3600 => format!("{} min ago", s / 60),
        3600..86_400 => format!("{} h ago", s / 3600),
        86_400..172_800 => "Yesterday".into(),
        _ => format!("{} days ago", s / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::ago;

    #[test]
    fn times_read_naturally() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert_eq!(ago(now), "Just now");
        assert_eq!(ago(now - 300), "5 min ago");
        assert_eq!(ago(now - 90_000), "Yesterday");
        assert_eq!(ago(now - 3 * 86_400), "3 days ago");
    }
}
