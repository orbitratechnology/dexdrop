//! History: transfers running now, then what was sent and received.

use dexdrop_core::{
    api::{Command, TransferSnapshot},
    store::{Direction, HistoryEntry, Outcome},
};
use nsh_base::theme;
use nsh_core::prelude::*;

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
                .take(60)
                .cloned()
                .collect::<Vec<_>>()
        })
    };
    let nothing = move || active().is_empty() && past().is_empty();
    body(
        view()
            .gap(px(space::XL))
            .child(filter_bar(filter))
            .child(when(
                move || !active().is_empty(),
                move || {
                    view()
                        .gap(px(space::M))
                        .child(ui::section("This session"))
                        .child(ui::group().child(each(
                            active,
                            |t: &TransferSnapshot| t.id,
                            |t, i| {
                                view()
                                    .child(when(move || i.get() > 0, || ui::separator(68.0)))
                                    .child(session_row(t))
                            },
                        )))
                },
            ))
            .child(when(
                move || !past().is_empty(),
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
                        .child(ui::group().child(each(
                            past,
                            |h: &HistoryEntry| h.transfer_id.clone(),
                            |h, i| {
                                view()
                                    .child(when(move || i.get() > 0, || ui::separator(68.0)))
                                    .child(row(h))
                            },
                        )))
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
fn session_row(t: ReadSignal<TransferSnapshot>) -> impl IntoElement {
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

/// A past transfer.
pub fn row(h: ReadSignal<HistoryEntry>) -> impl IntoElement {
    let e = move || h.get();
    let (mark, tone) = match h.with_untracked(|e| e.outcome.clone()) {
        Outcome::Completed => (Icon::Check, Tone::Success),
        Outcome::Partial { .. } => (Icon::Alert, Tone::Warning),
        Outcome::Declined | Outcome::Cancelled => (Icon::X, Tone::Neutral),
        Outcome::Failed(_) => (Icon::Alert, Tone::Danger),
    };
    let outgoing = h.with_untracked(|e| e.direction == Direction::Outgoing);
    let kind = Icon::for_file(&h.with_untracked(|e| e.label.clone()));
    view()
        .flex_row()
        .items_center()
        .gap(px(space::M))
        .px(px(space::L))
        .py(px(space::M))
        .child(
            view().child(ui::kind_glyph(kind, 40.0)).child(
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
            ),
        )
        .child(
            view()
                .flex_1()
                .gap(px(2.0))
                .child(ui::body(move || e().label).number_of_lines(1))
                .child(
                    muted(move || format!("{} · {}", home::summary(&e()), ago(e().finished)))
                        .number_of_lines(1),
                ),
        )
        .child(ui::icon(mark, 18.0, move || ui::tone_color(tone)))
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
