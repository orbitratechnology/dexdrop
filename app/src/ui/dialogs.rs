//! Questions to the user and surfaces over the page: the system's alerts,
//! sheets and dialogs.

use nsh_base::theme;
use nsh_core::{
    element::{ModalAnimation, PointerEvents},
    prelude::*,
};

use super::*;

/// Asks before something that costs the user (data on a metered link),
/// with the system's alert: `title`, `description`, Cancel and `action`;
/// runs `then` when the user goes ahead.
pub fn ask(
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
    action: impl Into<SharedString>,
    then: impl FnOnce() + 'static,
) {
    let answer = nsh_base::alert_dialog()
        .title(title)
        .description(description)
        .cancel("cancel", "Cancel")
        .action("go", action)
        .show();
    spawn(async move {
        if answer
            .await
            .is_ok_and(|a| a.action.as_deref() == Some("go"))
        {
            then();
        }
    });
}

/// Asks before something is removed, with the system's alert: `title`,
/// what happens (`description`), Cancel and a destructive `action`; runs
/// `then` when the user confirms.
pub fn confirm(
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
    action: impl Into<SharedString>,
    then: impl FnOnce() + 'static,
) {
    let answer = nsh_base::alert_dialog()
        .title(title)
        .description(description)
        .cancel("cancel", "Cancel")
        .destructive("go", action)
        .show();
    spawn(async move {
        if answer
            .await
            .is_ok_and(|a| a.action.as_deref() == Some("go"))
        {
            then();
        }
    });
}

/// A bottom sheet (`nsh_base::sheet`): it slides up over the scrim while
/// `open`, takes the keyboard focus and the screen reader to itself, and
/// gives them back as it leaves. Escape and the Android back button run
/// `on_dismiss`; a press on the scrim does nothing, so a stray tap never
/// answers for the user. `content` is its card.
pub fn sheet(
    label: &'static str,
    open: impl Fn() -> bool + 'static,
    on_dismiss: impl Fn() + 'static,
    content: impl IntoElement,
) -> nsh_base::SheetElement {
    nsh_base::sheet()
        .open(move || Some(open()))
        .close_on_backdrop_press(false)
        .accessibility_label(label)
        .on_open_change(move |e: &nsh_base::SheetEvent| {
            if let nsh_base::SheetEvent::OpenChange(false, _) = e {
                on_dismiss();
            }
        })
        .backdrop_style(|s| s.bg(scrim))
        .popup_style(|s| s.items_center())
        .child(content)
}

/// A dialog (`nsh_base::dialog`): a card centered over the scrim, rising
/// into place as it opens (`motion::SLOW`, ease-out; at once with reduced
/// motion), holding the keyboard focus and the screen reader until it
/// closes. Escape, the Android back button and a press on the scrim run
/// `on_dismiss` (none: it stays until its own buttons answer). Put it in
/// the view to show it, leave it out to close it.
pub fn dialog(
    label: impl Into<SharedString>,
    on_dismiss: Option<fn()>,
    content: impl IntoElement,
) -> impl IntoElement {
    let rise = motion().entering(MotionPreset::FadeInUp);
    nsh_base::dialog()
        .label(label.into())
        .animation(ModalAnimation::Fade)
        .close_on_escape(on_dismiss.is_some())
        .on_cancel(move || {
            if let Some(f) = on_dismiss {
                f()
            }
        })
        .child(
            view()
                .absolute()
                .inset_0()
                .bg(scrim)
                .accessible(false)
                .on_press(move || {
                    if let Some(f) = on_dismiss {
                        f()
                    }
                }),
        )
        .child(
            view()
                .absolute()
                .inset_0()
                .items_center()
                .justify_center()
                .p(px(space::L))
                .pointer_events(PointerEvents::BoxNone)
                .child(
                    view()
                        .bind_motion(&rise)
                        .w_full()
                        .max_w(px(440.0))
                        .p(px(space::XL))
                        .gap(px(space::L))
                        .rounded(px(radius::xxl()))
                        .bg(move || theme().card)
                        .shadow_lg()
                        .child(content),
                ),
        )
}

/// A cover: a full screen over everything (an introduction the user must
/// get through), fading in, with its content rising into place. It holds
/// the keyboard focus and the screen reader; nothing dismisses it but its
/// own buttons.
pub fn cover(label: impl Into<SharedString>, content: impl IntoElement) -> impl IntoElement {
    let rise = motion().entering(MotionPreset::FadeInUp);
    nsh_base::dialog()
        .label(label.into())
        .animation(ModalAnimation::Fade)
        .close_on_escape(false)
        .child(
            view()
                .absolute()
                .inset_0()
                .bg(move || theme().background)
                .items_center()
                .justify_center()
                .px(px(space::XL))
                .child(
                    view()
                        .bind_motion(&rise)
                        .w_full()
                        .max_w(px(420.0))
                        .child(content),
                ),
        )
}

/// An action of a [`context_menu`].
pub struct Action {
    label: &'static str,
    destructive: bool,
    run: std::rc::Rc<dyn Fn()>,
}

impl Action {
    pub fn new(label: &'static str, run: impl Fn() + 'static) -> Action {
        Action {
            label,
            destructive: false,
            run: std::rc::Rc::new(run),
        }
    }

    /// Shown in the destructive color (it asks before it removes).
    pub fn destructive(mut self) -> Action {
        self.destructive = true;
        self
    }
}

/// A context menu about `trigger` (`nsh_base::context_menu`): a right click
/// on the desktops, a held finger on touch screens, and Shift+F10 or the
/// Menu key on the focused trigger open it; the arrows move between its
/// actions and Return chooses. It grows from where it opens (`motion::SLOW`
/// in, `motion::BASE` out; at once with reduced motion), as a sheet at the
/// bottom on phones.
pub fn context_menu(trigger: impl IntoElement, actions: Vec<Action>) -> impl IntoElement {
    nsh_base::context_menu()
        .trigger(trigger)
        .popup_style(|s| {
            s.min_w(px(200.0))
                .p(px(space::XS))
                .rounded(px(radius::lg()))
                .bg(move || theme().popover)
                .shadow_lg()
                .transition_all()
                .duration(motion::SLOW)
                .ease_out()
        })
        .popup_starting_style(|s| s.opacity(0.0).scale(0.97))
        .popup_ending_style(|s| s.opacity(0.0).scale(0.97).duration(motion::BASE))
        .backdrop_style(|s| s.bg(scrim))
        .children(actions.into_iter().map(|a| {
            let run = a.run.clone();
            let color = move || {
                if a.destructive {
                    theme().destructive
                } else {
                    theme().popover_foreground
                }
            };
            nsh_base::menu_item()
                .accessibility_label(a.label)
                .on_select(move || run())
                .min_h(px(hit()))
                .px(px(space::M))
                .flex_row()
                .items_center()
                .rounded(px(radius::sm()))
                .transition_colors()
                .duration(motion::INSTANT)
                .ease_out()
                .hover(|s| s.bg(move || theme().accent))
                .focus(|s| s.bg(move || theme().accent))
                .child(text(a.label).text_size(px(size::BODY)).text_color(color))
        }))
}

/// A tooltip naming a control that shows only an icon: on the desktops
/// while the pointer rests on it, and with the keyboard's focus
/// (`nsh_base::tooltip`, drawn in the foreground's color, fading in
/// `motion::FAST`). Set it with `.tooltip(ui::tooltip(".."))`.
pub fn tooltip(label: &'static str) -> nsh_base::TooltipElement {
    nsh_base::tooltip()
        .starting_style(|s| s.opacity(0.0))
        .ending_style(|s| s.opacity(0.0))
        .child(
            view()
                .px(px(space::S))
                .py(px(space::XS))
                .rounded(px(radius::sm()))
                .bg(move || theme().foreground)
                .transition_opacity()
                .duration(motion::FAST)
                .ease_out()
                .child(
                    text(label)
                        .text_size(px(size::CAPTION))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || theme().background),
                ),
        )
}
