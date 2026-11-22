//! The app shell: a persistent sidebar in wide windows, a bottom tab bar on
//! phones (PRD 7.1 – 7.3), plus the overlays every page shares: the incoming
//! request banner, sheets and toasts.

use dexdrop_core::api::NoticeKind;
use nsh_base::{button, theme};
use nsh_core::{element::AccessibilityRole, prelude::*};

use crate::{
    pages,
    state::{Sheet, Tab, model, set_tab, sheet, tab, toasts, wide},
    ui::{self, Tone, Variant, btn, muted, space},
};

/// Desktop keyboard shortcuts (PRD 7.3); `mod` is Cmd on Apple platforms
/// and Ctrl elsewhere.
fn shortcuts() {
    use nsh_core::file_dialog::OpenDialog;
    on_key("mod+o", |_| pages::nearby::choose(OpenDialog::files()));
    on_key("mod+shift+o", |_| {
        pages::nearby::choose(OpenDialog::folders())
    });
    on_key("mod+1", |_| set_tab(Tab::Nearby));
    on_key("mod+2", |_| set_tab(Tab::Transfers));
    on_key("mod+3", |_| set_tab(Tab::Settings));
    on_key("mod+,", |_| set_tab(Tab::Settings));
    on_key("escape", |_| {
        let open = sheet().with_untracked(|s| s.clone());
        if open == Sheet::Receive {
            crate::bridge::send(dexdrop_core::api::Command::CloseInvitations);
        }
        if open != Sheet::None {
            sheet().set(Sheet::None);
        }
    });
}

pub fn shell() -> impl IntoElement {
    shortcuts();
    view()
        .flex_1()
        .bg(move || theme().background)
        .child(when(wide, desktop).otherwise(phone))
        .child(overlay_sheet())
        .child(toast_stack())
}

fn page_body() -> impl IntoElement {
    dynamic(move || match tab() {
        Tab::Nearby => pages::nearby::page().into_element(),
        Tab::Transfers => pages::transfers::page().into_element(),
        Tab::Settings => pages::settings::page().into_element(),
    })
}

fn content(padding: f32) -> impl IntoElement {
    scroll().flex_1().child(
        view()
            .p(px(padding))
            .pb(px(padding + 24.0))
            .gap(px(space::L))
            .w_full()
            .max_w(px(860.0))
            .self_center()
            .child(request_banner())
            .child(offline_banner())
            .child(page_body()),
    )
}

// ---------------------------------------------------------------- desktop

fn desktop() -> impl IntoElement {
    view()
        .flex_1()
        .flex_row()
        .child(sidebar())
        .child(view().flex_1().child(content(space::XL + 8.0)))
}

fn sidebar() -> impl IntoElement {
    view()
        .w(px(232.0))
        .bg(move || theme().sidebar)
        .border_r_1()
        .border_r_color(move || theme().border)
        .p(px(space::M))
        .gap(px(space::XS))
        .child(
            view()
                .px(px(space::S))
                .pt(px(space::S))
                .pb(px(space::L))
                .gap(px(2.0))
                .child(
                    text("DexDrop")
                        .text_size(px(20.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(move || theme().foreground),
                )
                .child(muted("Local, private, direct.")),
        )
        .children(Tab::ALL.map(nav_item))
        .child(view().flex_1())
        .child(device_status())
}

fn badge_count(t: Tab) -> usize {
    match t {
        Tab::Transfers => model().with(|m| {
            m.requests.len()
                + m.transfers
                    .iter()
                    .filter(|t| {
                        !t.state.is_finished()
                            && t.state != dexdrop_core::api::TransferState::Requested
                    })
                    .count()
        }),
        _ => 0,
    }
}

fn nav_item(t: Tab) -> impl IntoElement {
    let active = move || tab() == t;
    button()
        .selected(active)
        .role(AccessibilityRole::Tab)
        .accessibility_label(t.title())
        .on_press(move || set_tab(t))
        .flex_row()
        .items_center()
        .gap(px(space::S))
        .h(px(36.0))
        .px(px(space::M))
        .rounded(px(8.0))
        .bg(move || {
            if active() {
                theme().sidebar_accent
            } else {
                theme().sidebar.opacity(0.0)
            }
        })
        .hover(move |s| s.bg(move || theme().sidebar_accent))
        .child(
            text(t.title())
                .flex_1()
                .text_size(px(ui::size::BODY))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if active() {
                        theme().foreground
                    } else {
                        theme().muted_foreground
                    }
                }),
        )
        .child(when(
            move || badge_count(t) > 0,
            move || ui::badge(SharedString::from(badge_count(t).to_string()), Tone::Accent),
        ))
}

fn device_status() -> impl IntoElement {
    let name = move || {
        model().with(|m| {
            m.status
                .as_ref()
                .map(|s| s.device_name.clone())
                .unwrap_or_default()
        })
    };
    let visible = move || {
        model().with(|m| m.status.as_ref().is_some_and(|s| s.visible) && m.offline.is_none())
    };
    view()
        .flex_row()
        .items_center()
        .gap(px(space::S))
        .p(px(space::S))
        .rounded(px(8.0))
        .bg(move || theme().card)
        .border_1()
        .border_color(move || theme().border)
        .child(ui::avatar(name, 30.0))
        .child(
            view()
                .flex_1()
                .child(
                    ui::body(name)
                        .text_size(px(ui::size::SMALL))
                        .font_weight(FontWeight::SEMIBOLD)
                        .number_of_lines(1),
                )
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .child(ui::dot(move || {
                            if visible() {
                                Tone::Success
                            } else {
                                Tone::Neutral
                            }
                        }))
                        .child(
                            muted(move || {
                                if visible() {
                                    "Ready to receive".to_owned()
                                } else if model().with(|m| m.offline.is_some()) {
                                    "Not receiving".to_owned()
                                } else {
                                    "Hidden".to_owned()
                                }
                            })
                            .text_size(px(ui::size::CAPTION)),
                        ),
                ),
        )
}

// ---------------------------------------------------------------- phone

fn phone() -> impl IntoElement {
    view()
        .flex_1()
        .pt_safe_area()
        .child(content(space::L))
        .child(tab_bar())
}

fn tab_bar() -> impl IntoElement {
    view()
        .flex_row()
        .border_t_1()
        .border_t_color(move || theme().border)
        .bg(move || theme().card)
        .pb_safe_area()
        .children(Tab::ALL.map(|t| {
            let active = move || tab() == t;
            button()
                .selected(active)
                .role(AccessibilityRole::Tab)
                .accessibility_label(t.title())
                .on_press(move || set_tab(t))
                .flex_1()
                .min_h(px(ui::hit() + 8.0))
                .items_center()
                .justify_center()
                .gap(px(2.0))
                .child(view().h(px(3.0)).w(px(24.0)).rounded(px(2.0)).bg(move || {
                    if active() {
                        theme().primary
                    } else {
                        theme().card.opacity(0.0)
                    }
                }))
                .child(
                    text(move || {
                        let n = badge_count(t);
                        if n > 0 {
                            format!("{} ({n})", t.title())
                        } else {
                            t.title().to_owned()
                        }
                    })
                    .text_size(px(ui::size::SMALL))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(move || {
                        if active() {
                            theme().primary
                        } else {
                            theme().muted_foreground
                        }
                    }),
                )
        }))
}

// ---------------------------------------------------------------- overlays

fn request_banner() -> impl IntoElement {
    let pending = move || model().with(|m| m.requests.len());
    when(
        move || pending() > 0 && tab() != Tab::Transfers,
        move || {
            view()
                .flex_row()
                .items_center()
                .gap(px(space::M))
                .p(px(space::M))
                .rounded(move || theme().radius)
                .bg(move || theme().color("info-bg").unwrap_or(theme().accent))
                .border_1()
                .border_color(move || theme().primary)
                .child(
                    ui::body(move || {
                        model().with(|m| match m.requests.as_slice() {
                            [one] => format!("{} wants to send you {}.", one.peer_name, one.label),
                            many => format!("{} devices want to send you files.", many.len()),
                        })
                    })
                    .flex_1(),
                )
                .child(btn("Review", Variant::Primary).on_press(|| set_tab(Tab::Transfers)))
        },
    )
}

fn offline_banner() -> impl IntoElement {
    when(
        move || model().with(|m| m.offline.is_some()),
        move || {
            view()
                .p(px(space::M))
                .rounded(move || theme().radius)
                .bg(move || theme().muted)
                .child(muted(move || {
                    model().with(|m| m.offline.clone().unwrap_or_default())
                }))
        },
    )
}

fn sheet_title(s: &Sheet) -> &'static str {
    match s {
        Sheet::None => "",
        Sheet::Receive => "Receive with a code",
        Sheet::Connect => "Send with a code",
        Sheet::Diagnostics(_) => "Diagnostics",
    }
}

fn overlay_sheet() -> impl IntoElement {
    let open = move || sheet().with(|s| *s != Sheet::None);
    when(open, move || {
        view()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .bottom_0()
            .bg(move || theme().foreground.opacity(0.35))
            .items_center()
            .justify_center()
            .p(px(space::L))
            .child(
                view()
                    .w_full()
                    .max_w(px(460.0))
                    .p(px(space::XL))
                    .gap(px(space::L))
                    .rounded(px(16.0))
                    .bg(move || theme().card)
                    .border_1()
                    .border_color(move || theme().border)
                    .role(AccessibilityRole::Dialog)
                    .child(
                        text(move || sheet().with(sheet_title))
                            .text_size(px(ui::size::HEADING + 2.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(move || theme().foreground),
                    )
                    .child(dynamic(move || match sheet().get() {
                        Sheet::Receive => pages::nearby::receive_sheet().into_element(),
                        Sheet::Connect => pages::nearby::connect_sheet().into_element(),
                        Sheet::Diagnostics(report) => diagnostics(report).into_element(),
                        _ => view().into_element(),
                    })),
            )
    })
}

fn diagnostics(report: String) -> impl IntoElement {
    let copy = report.clone();
    view()
        .gap(px(space::M))
        .child(muted("Anonymized: no file names, paths, contents or keys."))
        .child(
            scroll().max_h(px(280.0)).child(
                text(report)
                    .text_size(px(ui::size::CAPTION))
                    .font_family(move || {
                        theme()
                            .font_mono
                            .clone()
                            .unwrap_or_else(|| "monospace".into())
                    })
                    .selectable(true)
                    .text_color(move || theme().foreground),
            ),
        )
        .child(
            view()
                .flex_row()
                .justify_end()
                .gap(px(space::S))
                .child(btn("Copy", Variant::Secondary).on_press(move || {
                    set_clipboard(copy.clone());
                    crate::state::toast(NoticeKind::Success, "Diagnostics copied.");
                }))
                .child(btn("Close", Variant::Primary).on_press(|| sheet().set(Sheet::None))),
        )
}

fn toast_stack() -> impl IntoElement {
    view()
        .absolute()
        .bottom(px(if crate::state::desktop_platform() {
            20.0
        } else {
            84.0
        }))
        .right(px(16.0))
        .left(px(if crate::state::desktop_platform() {
            260.0
        } else {
            16.0
        }))
        .items_end()
        .gap(px(space::S))
        .pointer_events(PointerEvents::BoxNone)
        .child(each(
            move || toasts().with(|t| t.items.clone()),
            |(id, ..): &(u64, NoticeKind, String)| *id,
            |item, _| {
                let tone = move || match item.with(|i| i.1) {
                    NoticeKind::Error => Tone::Danger,
                    NoticeKind::Success => Tone::Success,
                    NoticeKind::Info => Tone::Accent,
                };
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(space::S))
                    .max_w(px(420.0))
                    .px(px(space::M))
                    .py(px(space::S + 2.0))
                    .rounded(px(10.0))
                    .bg(move || theme().popover)
                    .border_1()
                    .border_color(move || theme().border)
                    .role(AccessibilityRole::Alert)
                    .child(ui::dot(tone))
                    .child(
                        ui::body(move || item.with(|i| i.2.clone())).text_size(px(ui::size::SMALL)),
                    )
            },
        ))
}
