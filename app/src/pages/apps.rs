//! Android: choosing apps to send. Each installed app is listed with its
//! icon, name, version, size and when it was updated; the chosen ones are
//! copied as APKs under their names ("Maps 11.2.apk") into the selection.
//! APK files already on the phone are picked from the system's files.

use std::path::PathBuf;

use dexdrop_core::format;
use nsh_base::theme;
use nsh_core::prelude::*;

use crate::{
    files::{AppCard, app_cards, app_icon},
    pages::{body, send},
    state::pop,
    ui::{self, Icon, Variant, muted, space},
};

/// The apps, once listed: each with its APKs.
#[derive(Clone, PartialEq)]
struct Listed {
    card: AppCard,
    apks: Vec<PathBuf>,
}

#[derive(Clone, PartialEq)]
enum Apps {
    Loading,
    Ready(Vec<Listed>),
    Failed(String),
}

pub fn page() -> impl IntoElement {
    let apps = signal(Apps::Loading);
    let query = signal(String::new());
    let chosen = signal(Vec::<String>::new());
    let show_system = signal(false);
    load(apps);

    let shown = move |system: bool| {
        let q = query.get().to_lowercase();
        apps.with(|a| match a {
            Apps::Ready(list) => list
                .iter()
                .filter(|l| l.card.system == system)
                .filter(|l| {
                    q.is_empty()
                        || l.card.label.to_lowercase().contains(&q)
                        || l.card.package.to_lowercase().contains(&q)
                })
                .cloned()
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
    };
    let count = move || chosen.with(|c| c.len());

    view()
        .flex_1()
        .child(body(
            view()
                .gap(px(space::L))
                .child(apk_files())
                .child(search(query))
                .child(dynamic(move || match apps.get() {
                    Apps::Loading => view()
                        .items_center()
                        .gap(px(space::M))
                        .py(px(space::XXL))
                        .child(ui::spinner(28.0))
                        .child(muted("Finding your apps…"))
                        .into_element(),
                    Apps::Failed(why) => ui::empty(Icon::Android, why).into_element(),
                    Apps::Ready(_) => view()
                        .gap(px(space::L))
                        .child(section("Your apps", move || shown(false), chosen))
                        .child(ui::group().child(ui::switch_row(
                            ui::glyph(Icon::Android, (0x22c55e, 0x16a34a), 30.0),
                            "Show system apps",
                            move || show_system.get(),
                            move |on| show_system.set(on),
                        )))
                        .child(when(
                            move || show_system.get(),
                            move || section("System apps", move || shown(true), chosen),
                        ))
                        .into_element(),
                })),
        ))
        .child(when(move || count() > 0, move || add_bar(apps, chosen)))
}

/// Lists the apps in the background.
fn load(apps: Signal<Apps>) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let icons = crate::bridge::state_dir("app-icons");
        let listing = blocking(move || dexdrop_net::apps::installed(&icons));
        spawn(async move {
            apps.set(match listing.await {
                Ok(Ok(list)) => Apps::Ready(
                    list.into_iter()
                        .map(|a| Listed {
                            apks: a.apks.clone(),
                            card: a.into(),
                        })
                        .collect(),
                ),
                Ok(Err(err)) => Apps::Failed(err),
                Err(_) => Apps::Failed("The apps could not be listed.".into()),
            });
        });
    }
    #[cfg(target_arch = "wasm32")]
    apps.set(Apps::Failed("Sending apps works on Android.".into()));
}

/// APK files already on the phone, from the system's files.
fn apk_files() -> impl IntoElement {
    ui::group().child(ui::nav_row(
        ui::file_badge("app.apk", 30.0, 8.0),
        "APK files",
        "From your files",
        || {
            send::choose_apks();
            pop();
        },
    ))
}

fn search(query: Signal<String>) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap(px(space::S))
        .px(px(space::M))
        .rounded(px(12.0))
        .bg(ui::surface)
        .child(ui::icon(Icon::Search, 18.0, move || {
            theme().muted_foreground
        }))
        .child(
            nsh_base::input()
                .value(move || query.get())
                .on_change(move |v: &str| query.set(v.to_owned()))
                .placeholder("Search apps")
                .label("Search apps")
                .flex_1()
                .min_h(px(ui::hit()))
                .text_size(px(ui::size::BODY))
                .text_color(move || theme().foreground),
        )
}

/// A titled group of apps, each a row to tick.
fn section(
    title: &'static str,
    list: impl Fn() -> Vec<Listed> + Copy + 'static,
    chosen: Signal<Vec<String>>,
) -> impl IntoElement {
    view()
        .gap(px(space::S))
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .child(ui::section(title))
                .child(muted(move || {
                    let n = list().len();
                    if n == 1 {
                        "1 app".to_owned()
                    } else {
                        format!("{n} apps")
                    }
                })),
        )
        .child(when(
            move || list().is_empty(),
            || muted("None").px(px(space::L)),
        ))
        .child(when(
            move || !list().is_empty(),
            move || {
                let rows = computed(list);
                // Virtualized (NSH lists): phones have hundreds of apps; only
                // the rows near the screen exist.
                ui::group().child(
                    virtual_list(
                        move || rows.with(Vec::len),
                        move |at| {
                            dynamic(move || {
                                rows.with(|r| r.get(at.index()).cloned())
                                    .map(|l| app_row(l, chosen))
                            })
                        },
                    )
                    .keys(move || {
                        rows.with(|r| r.iter().map(|l| l.card.package.clone()).collect::<Vec<_>>())
                    })
                    .separator(|| ui::separator(72.0))
                    .estimated_item_size(px(68.0))
                    .viewport(ListViewport::Parent),
                )
            },
        ))
}

fn app_row(app: Listed, chosen: Signal<Vec<String>>) -> impl IntoElement {
    let package = app.card.package.clone();
    let selected = {
        let package = package.clone();
        move || chosen.with(|c| c.contains(&package))
    };
    let mut detail = vec![];
    if !app.card.version.is_empty() {
        detail.push(app.card.version.clone());
    }
    detail.push(format::bytes(app.card.size));
    if app.card.split {
        detail.push("Split APKs".into());
    }
    if app.card.updated > 0 {
        detail.push(format!(
            "Updated {}",
            crate::pages::history::ago(app.card.updated / 1000).to_lowercase()
        ));
    }
    let label = app.card.label.clone();
    ui::pressable()
        .accessibility_label(label.clone())
        .selected(selected.clone())
        .on_press(move || {
            chosen.update(|c| {
                if let Some(i) = c.iter().position(|p| *p == package) {
                    c.remove(i);
                } else {
                    c.push(package.clone());
                }
            })
        })
        .flex_row()
        .items_center()
        .gap(px(space::M))
        .px(px(space::L))
        .py(px(space::S))
        .min_h(px(ui::hit() + 16.0))
        .active(move |s| s.bg(move || theme().muted))
        .child(app_icon(app.card.icon.as_deref(), 44.0))
        .child(
            view()
                .flex_1()
                .gap(px(2.0))
                .child(ui::body(label).number_of_lines(1))
                .child(
                    muted(detail.join(" · "))
                        .text_size(px(ui::size::CAPTION))
                        .number_of_lines(1),
                ),
        )
        .child(check(selected))
}

/// A round tick, filled when chosen.
fn check(selected: impl Fn() -> bool + Clone + 'static) -> impl IntoElement {
    let on = selected.clone();
    let on2 = selected.clone();
    view()
        .size(px(26.0))
        .rounded(px(13.0))
        .items_center()
        .justify_center()
        .border_2()
        .border_color(move || {
            if on() {
                theme().primary
            } else {
                theme().border
            }
        })
        .bg(move || {
            if on2() {
                theme().primary
            } else {
                theme().card.opacity(0.0)
            }
        })
        .child(when(selected, || {
            ui::icon(Icon::Check, 15.0, move || theme().primary_foreground)
        }))
}

/// "Add 3 apps": copies them as APKs into the selection, then back to
/// Send, where a card says so while they copy.
fn add_bar(apps: Signal<Apps>, chosen: Signal<Vec<String>>) -> impl IntoElement {
    view()
        .absolute()
        .left_0()
        .right_0()
        .bottom(px(if ui::mobile() { 96.0 } else { 0.0 }))
        .px(px(space::L))
        .pb(px(space::L))
        .items_center()
        .child(dynamic(move || {
            let n = chosen.with(|c| c.len());
            let label = if n == 1 {
                "Add 1 app".to_owned()
            } else {
                format!("Add {n} apps")
            };
            ui::icon_btn(Icon::Plus, label, Variant::Primary)
                .w_full()
                .max_w(px(420.0))
                .shadow_lg()
                .on_press(move || {
                    let picked: Vec<Listed> = apps.with_untracked(|a| match a {
                        Apps::Ready(list) => {
                            let chosen = chosen.get_untracked();
                            list.iter()
                                .filter(|l| chosen.contains(&l.card.package))
                                .cloned()
                                .collect()
                        }
                        _ => Vec::new(),
                    });
                    send::add_apps(picked.into_iter().map(|l| (l.card, l.apks)).collect());
                    pop();
                })
        }))
}

/// Remembers what a staged APK installs, so lists show the app.
pub fn remember(path: PathBuf, card: AppCard) {
    app_cards().update(|c| {
        c.0.insert(path, Some(card));
    });
}
