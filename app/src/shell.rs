//! The app shell: on phones the system's tab bar (Liquid Glass on iOS 26,
//! Material 3 on Android) over one native navigation stack per tab; on
//! desktops a sidebar beside the same stacks, whose pages draw their own
//! header. Overlays every page shares: an incoming request, dialogs and
//! toasts.

use dexdrop_core::api::{Command, NoticeKind, TransferState};
use nsh_base::{button, nav_page, nav_stack, theme};
use nsh_core::{
    element::{AccessibilityRole, Screen},
    mobile::{TabBarItem, tab_bar},
    prelude::*,
};

use crate::{
    bridge, pages,
    state::{Route, Sheet, Tab, model, pop, set_tab, sheet, stack, tab, toasts, truncate, wide},
    ui::{self, Icon, Tone, Variant, btn, muted, space},
};

/// Desktop keyboard shortcuts (PRD 7.3); `mod` is Cmd on Apple platforms
/// and Ctrl elsewhere.
fn shortcuts() {
    use nsh_core::file_dialog::OpenDialog;
    on_key("mod+o", |_| pages::send::choose(OpenDialog::files()));
    on_key(
        "mod+shift+o",
        |_| pages::send::choose(OpenDialog::folders()),
    );
    on_key("mod+1", |_| set_tab(Tab::Share));
    on_key("mod+2", |_| set_tab(Tab::History));
    on_key("mod+3", |_| set_tab(Tab::Settings));
    on_key("mod+,", |_| set_tab(Tab::Settings));
    on_key("escape", |_| {
        if sheet().with_untracked(|s| *s != Sheet::None) {
            sheet().set(Sheet::None);
        } else {
            pop();
        }
    });
}

pub fn shell() -> impl IntoElement {
    shortcuts();
    view()
        .flex_1()
        .bg(move || theme().background)
        .child(when(wide, desktop).otherwise(phone))
        .child(pages::transfer::request_overlay())
        .child(overlay_sheet())
        .child(when(needs_local_network_intro, local_network_intro))
        .child(toast_stack())
}

/// Whether to say why DexDrop uses the local network before the system asks
/// (FR-004): on iOS and macOS, once, after the engine has started.
fn needs_local_network_intro() -> bool {
    matches!(nsh_core::element::os().name(), "ios" | "macos")
        && model().with(|m| {
            m.status.is_some() && m.offline.is_none() && !m.settings.local_network_explained
        })
}

/// One screen: what DexDrop does on the network, and that the system will
/// ask. Continue starts discovery, which brings the system's prompt.
fn local_network_intro() -> impl IntoElement {
    let enter = motion().entering(MotionPreset::FadeInUp);
    view()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .bottom_0()
        .bg(move || theme().background)
        .items_center()
        .justify_center()
        .px(px(space::XL))
        .role(AccessibilityRole::Dialog)
        .child(
            view()
                .bind_motion(&enter)
                .w_full()
                .max_w(px(420.0))
                .items_center()
                .gap(px(space::L))
                .child(ui::radar(
                    ui::SEND,
                    200.0,
                    ui::glyph(Icon::Wifi, ui::SEND, 84.0),
                ))
                .child(ui::title("Find your devices").text_center())
                .child(
                    ui::muted(
                        "DexDrop finds your devices on this Wi‑Fi and sends files straight to them. \
                         Allow access to your local network when asked.",
                    )
                    .text_center(),
                )
                .child(
                    ui::btn("Continue", Variant::Primary)
                        .self_stretch()
                        .on_press(|| bridge::send(Command::LocalNetworkExplained)),
                ),
        )
}

/// Requests waiting for an answer and transfers running.
fn activity() -> usize {
    model().with(|m| {
        m.requests.len()
            + m.transfers
                .iter()
                .filter(|t| !t.state.is_finished() && t.state != TransferState::Requested)
                .count()
    })
}

// ---------------------------------------------------------------- pages

/// One tab's native navigation stack.
fn tab_stack(t: Tab) -> impl IntoElement {
    nav_stack().flex_1().child(each(
        move || stack(t),
        |route: &Route| format!("{route:?}"),
        move |route, index| {
            let route = route.get_untracked();
            let root = route == Route::Root;
            let (title, content) = pages::page(t, route);
            page(title, root, content)
                // The user went back natively (swipe, back button).
                .on_dismiss(move || truncate(t, index.get_untracked()))
        },
    ))
}

/// A page: the system's header on phones (a large title at a tab's root
/// on iOS), the app's own on desktops and the web.
fn page(title: pages::Title, root: bool, content: AnyElement) -> Screen {
    let t = title.clone();
    nav_page()
        .title(move || t())
        .header_large_title(root)
        .header(desktop_header(title, root))
        .child(content)
}

/// The header desktops and the web draw: a back button and the title.
fn desktop_header(title: pages::Title, root: bool) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap(px(space::S))
        .px(px(space::XL))
        .pt(px(space::XL))
        .pb(px(space::S))
        .child(when(
            move || !root,
            || {
                ui::round_btn(Icon::ChevronRight, "Back", Variant::Secondary, 34.0)
                    .rotate(deg(180.0))
                    .on_press(pop)
            },
        ))
        .child(
            text(move || title())
                .text_size(px(if root { 28.0 } else { 22.0 }))
                .font_weight(FontWeight::BOLD)
                .text_color(move || theme().foreground)
                .role(AccessibilityRole::Header),
        )
}

// ---------------------------------------------------------------- phone

fn phone() -> impl IntoElement {
    view()
        .flex_1()
        .child(dynamic(move || tab_stack(tab()).into_element()))
        .child(if ui::mobile() {
            native_tab_bar().into_element()
        } else {
            drawn_tab_bar().into_element()
        })
}

/// A narrow window on a desktop or the web: the same tabs, drawn.
fn drawn_tab_bar() -> impl IntoElement {
    view()
        .absolute()
        .left_0()
        .right_0()
        .bottom_0()
        .flex_row()
        .bg(move || theme().card)
        .border_t_1()
        .border_t_color(move || theme().border)
        .children(Tab::ALL.map(|t| {
            let active = move || tab() == t;
            let color = move || {
                if active() {
                    theme().primary
                } else {
                    theme().muted_foreground
                }
            };
            button()
                .selected(active)
                .role(AccessibilityRole::Tab)
                .accessibility_label(t.title())
                .on_press(move || set_tab(t))
                .flex_1()
                .items_center()
                .gap(px(2.0))
                .py(px(space::S))
                .child(ui::icon(sidebar_icon(t), 22.0, color))
                .child(
                    text(t.title())
                        .text_size(px(ui::size::CAPTION))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(color),
                )
        }))
}

/// The system's tab bar: `UITabBar` (Liquid Glass on iOS 26), Material 3's
/// navigation bar on Android.
fn native_tab_bar() -> impl IntoElement {
    tab_bar()
        .items(move || {
            let n = activity();
            Tab::ALL
                .map(|t| {
                    let item = TabBarItem::new(t.title()).icon(tab_icon(t));
                    if t == Tab::History && n > 0 {
                        item.badge(n.to_string())
                    } else {
                        item
                    }
                })
                .to_vec()
        })
        .selected_index(move || Tab::ALL.iter().position(|t| *t == tab()).unwrap_or(0) as u32)
        .on_change(|index: &u32| {
            if let Some(t) = Tab::ALL.get(*index as usize) {
                set_tab(*t);
            }
        })
        .absolute()
        .left_0()
        .right_0()
        .bottom_0()
}

/// The tab's icon by the platform's name: an SF Symbol on iOS, one of
/// Android's own drawables there.
fn tab_icon(t: Tab) -> &'static str {
    let ios = os().name() == "ios";
    match (t, ios) {
        (Tab::Share, true) => "paperplane",
        (Tab::History, true) => "clock",
        (Tab::Settings, true) => "gearshape",
        (Tab::Share, false) => "android:ic_menu_send",
        (Tab::History, false) => "android:ic_menu_recent_history",
        (Tab::Settings, false) => "android:ic_menu_preferences",
    }
}

// ---------------------------------------------------------------- desktop

fn desktop() -> impl IntoElement {
    view().flex_1().flex_row().child(sidebar()).child(
        view()
            .flex_1()
            .child(dynamic(move || tab_stack(tab()).into_element())),
    )
}

fn sidebar() -> impl IntoElement {
    view()
        .w(px(220.0))
        .bg(move || theme().sidebar)
        .p(px(space::M))
        .pt(px(space::XL))
        .gap(px(space::XS))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(space::S))
                .px(px(space::S))
                .pb(px(space::L))
                .child(ui::glyph(Icon::Paperplane, ui::SEND, 30.0))
                .child(
                    text("DexDrop")
                        .text_size(px(19.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(move || theme().foreground),
                ),
        )
        .children(Tab::ALL.map(nav_item))
        .child(view().flex_1())
        .child(device_chip())
}

fn sidebar_icon(t: Tab) -> Icon {
    match t {
        Tab::Share => Icon::Paperplane,
        Tab::History => Icon::Clock,
        Tab::Settings => Icon::Gear,
    }
}

fn nav_item(t: Tab) -> impl IntoElement {
    let active = move || tab() == t;
    let color = move || {
        if active() {
            theme().primary
        } else {
            theme().muted_foreground
        }
    };
    button()
        .selected(active)
        .role(AccessibilityRole::Tab)
        .accessibility_label(t.title())
        .on_press(move || set_tab(t))
        .flex_row()
        .items_center()
        .gap(px(space::M))
        .h(px(40.0))
        .px(px(space::M))
        .rounded(px(10.0))
        .bg(move || {
            if active() {
                theme().primary.opacity(0.12)
            } else {
                theme().sidebar.opacity(0.0)
            }
        })
        .hover(move |s| s.bg(move || theme().primary.opacity(0.08)))
        .child(ui::icon(sidebar_icon(t), 20.0, color))
        .child(
            text(t.title())
                .flex_1()
                .text_size(px(ui::size::BODY))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if active() {
                        theme().primary
                    } else {
                        theme().foreground
                    }
                }),
        )
        .child(when(
            move || t == Tab::History && activity() > 0,
            move || ui::pill(move || activity().to_string(), || Tone::Accent),
        ))
}

/// This device, visible or not, at the foot of the sidebar.
fn device_chip() -> impl IntoElement {
    let name = move || {
        model().with(|m| {
            m.status
                .as_ref()
                .map(|s| s.device_name.clone())
                .unwrap_or_default()
        })
    };
    let platform = move || {
        model().with(|m| {
            m.status
                .as_ref()
                .map(|s| s.platform.clone())
                .unwrap_or_default()
        })
    };
    let visible = move || {
        model().with(|m| m.status.as_ref().is_some_and(|s| s.visible) && m.offline.is_none())
    };
    button()
        .accessibility_label("This device")
        .on_press(|| set_tab(Tab::Settings))
        .flex_row()
        .items_center()
        .gap(px(space::S))
        .p(px(space::S))
        .rounded(px(12.0))
        .hover(move |s| s.bg(move || theme().primary.opacity(0.08)))
        .child(ui::avatar(name, platform, 34.0))
        .child(
            view()
                .flex_1()
                .child(
                    ui::strong(name)
                        .text_size(px(ui::size::SMALL))
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
                                    "Visible".to_owned()
                                } else {
                                    "Hidden".to_owned()
                                }
                            })
                            .text_size(px(ui::size::CAPTION)),
                        ),
                ),
        )
}

// ---------------------------------------------------------------- overlays

fn sheet_title(s: &Sheet) -> &'static str {
    match s {
        Sheet::None => "",
        Sheet::Diagnostics(_) => "Diagnostics",
    }
}

/// A centered dialog for the rare things that need one.
fn overlay_sheet() -> impl IntoElement {
    let open = move || sheet().with(|s| *s != Sheet::None);
    when(open, move || {
        view()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .bottom_0()
            .bg(rgba(0x0000_0066))
            .items_center()
            .justify_center()
            .p(px(space::L))
            .child(
                view()
                    .w_full()
                    .max_w(px(440.0))
                    .p(px(space::XL))
                    .gap(px(space::L))
                    .rounded(px(24.0))
                    .bg(move || theme().card)
                    .shadow_lg()
                    .role(AccessibilityRole::Dialog)
                    .child(ui::title(move || sheet().with(sheet_title)))
                    .child(dynamic(move || match sheet().get() {
                        Sheet::Diagnostics(report) => diagnostics(report).into_element(),
                        Sheet::None => view().into_element(),
                    })),
            )
    })
}

fn diagnostics(report: String) -> impl IntoElement {
    let copy = report.clone();
    view()
        .gap(px(space::M))
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
                .child(
                    ui::icon_btn(Icon::Copy, "Copy", Variant::Secondary).on_press(move || {
                        set_clipboard(copy.clone());
                        crate::state::toast(NoticeKind::Success, "Diagnostics copied.");
                    }),
                )
                .child(btn("Close", Variant::Primary).on_press(|| sheet().set(Sheet::None))),
        )
}

/// Notices drop in at the top, where they never cover a sheet's buttons,
/// the tab bar or the controls at the bottom of a page.
fn toast_stack() -> impl IntoElement {
    view()
        .absolute()
        .top(move || {
            px(nsh_core::element::safe_area().top.0 + if ui::mobile() { 8.0 } else { 20.0 })
        })
        .left(px(16.0))
        .right(px(16.0))
        .items_center()
        .gap(px(space::S))
        .pointer_events(PointerEvents::BoxNone)
        .child(each(
            move || toasts().with(|t| t.items.clone()),
            |(id, ..): &(u64, NoticeKind, String)| *id,
            |item, _| {
                let kind = item.with_untracked(|i| i.1);
                let (i, tone) = match kind {
                    NoticeKind::Error => (Icon::Alert, Tone::Danger),
                    NoticeKind::Success => (Icon::Check, Tone::Success),
                    NoticeKind::Info => (Icon::Info, Tone::Accent),
                };
                let enter = motion().entering(MotionPreset::FadeInDown);
                view()
                    .bind_motion(&enter)
                    .flex_row()
                    .items_center()
                    .gap(px(space::S))
                    .max_w(px(440.0))
                    .px(px(space::L))
                    .py(px(space::M))
                    .rounded(px(999.0))
                    .bg(move || theme().popover)
                    .shadow_lg()
                    .role(AccessibilityRole::Alert)
                    .child(ui::icon(i, 18.0, move || ui::tone_color(tone)))
                    .child(
                        ui::body(move || item.with(|i| i.2.clone()))
                            .text_size(px(ui::size::SMALL))
                            .font_weight(FontWeight::MEDIUM),
                    )
            },
        ))
}
