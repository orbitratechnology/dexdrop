//! The app's pages, one module per area.

pub mod apps;
pub mod history;
pub mod home;
pub mod profile;
pub mod receive;
pub mod send;
pub mod settings;
pub mod transfer;
pub mod viewer;

use nsh_core::prelude::*;

use crate::{
    state::{Route, Tab},
    ui::{self, space},
};

/// A page's title, which can follow what the page shows ("Sending", then
/// "Sent").
pub type Title = std::rc::Rc<dyn Fn() -> SharedString>;

/// A route's title and content.
pub fn page(tab: Tab, route: Route) -> (Title, AnyElement) {
    let fixed = |s: &'static str| -> Title { std::rc::Rc::new(move || s.into()) };
    match (tab, route) {
        (Tab::Share, Route::Root) => (fixed("DexDrop"), home::page().into_element()),
        (Tab::History, Route::Root) => (fixed("History"), history::page().into_element()),
        (Tab::Settings, Route::Root) => (fixed("Settings"), settings::page().into_element()),
        (_, Route::Send) => (fixed("Send"), send::page().into_element()),
        (_, Route::Devices) => (fixed("Send to"), send::devices_page().into_element()),
        (_, Route::Code) => (fixed("Enter a code"), send::code_page().into_element()),
        (_, Route::Receive) => (fixed("Receive"), receive::page().into_element()),
        (_, Route::Transfer(id)) => (
            std::rc::Rc::new(move || transfer::title(id)),
            transfer::page(id).into_element(),
        ),
        (_, Route::Past(id)) => (fixed("Details"), history::past_page(id).into_element()),
        (_, Route::Viewer(path)) => {
            let name = viewer::title(&path);
            (
                std::rc::Rc::new(move || name.clone()),
                viewer::page(path).into_element(),
            )
        }
        (_, Route::Apps) => (fixed("Apps"), apps::page().into_element()),
        (_, Route::Avatar) => (fixed("Avatar"), profile::page().into_element()),
        (_, Route::Trusted) => (
            fixed("Trusted devices"),
            settings::trusted_page().into_element(),
        ),
    }
}

/// A page's scrolling body: centered, padded, clear of the tab bar.
pub fn body(content: impl IntoElement) -> impl IntoElement {
    // The page paints the theme's background itself, so light and dark are
    // the same on every platform (the system's page colour differs).
    scroll()
        .flex_1()
        .bg(move || nsh_base::theme().background)
        .child(
            view()
                .w_full()
                .max_w(px(ui::content_width()))
                .self_center()
                .px(px(if ui::mobile() { space::L } else { space::XL }))
                .pt(px(space::S))
                // The tab bar floats over the bottom of the page (phones,
                // narrow windows).
                .pb(px(
                    ui::above_tab_bar() + if ui::mobile() { 24.0 } else { space::XXL }
                ))
                .gap(px(space::XL))
                .child(content),
        )
}
