//! Send › Photos, Videos, Audio and Large files on phones: the device's
//! media browsed in the app (docs/FILE_SELECTION.md), from the platform's
//! asset provider (`dexdrop_core::asset`: Android's MediaStore, iOS's
//! PhotoKit). Items are listed a page at a time as the grid scrolls
//! (NSH's virtualized list: only the tiles near the screen exist), their
//! thumbnails made by the system a few at a time for the tiles in view, and
//! what is chosen is added to the selection as references: nothing is read
//! or copied until a transfer starts.
//!
//! Without a provider (computers, the web, a phone before the user allows
//! access) Send keeps the system's pickers.

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
};

use dexdrop_core::{
    asset::{self, AssetId, AssetProvider, MediaItem, MediaType, Query, Sort},
    format,
};
use dexdrop_net::assets::Access;
use nsh_base::theme;
use nsh_core::{ResizeMode, prelude::*};

pub use crate::state::Category;
use crate::{
    pages::send,
    state::pop,
    ui::{self, Icon, Variant, muted, space},
};

/// What "Large files" starts at.
pub const LARGE: u64 = 100 * 1024 * 1024;

/// Items asked for at a time.
const PAGE: usize = 96;

/// The side thumbnails are made at, in pixels.
const THUMB_PX: u32 = 256;

/// Thumbnails made at once (the newest asked first: the tiles in view).
const THUMB_WORKERS: usize = 4;

const CATEGORIES: &[(&str, Category)] = &[
    ("Photos", Category::Photos),
    ("Videos", Category::Videos),
    ("Audio", Category::Audio),
    ("Large", Category::Large),
];

const SORTS: &[(Sort, &str)] = &[
    (Sort::Newest, "Newest"),
    (Sort::Oldest, "Oldest"),
    (Sort::Largest, "Largest"),
    (Sort::Name, "Name"),
];

impl Category {
    fn kinds(self) -> &'static [MediaType] {
        match self {
            Category::Photos => &[MediaType::Image],
            Category::Videos => &[MediaType::Video],
            Category::Audio => &[MediaType::Audio],
            Category::Large => &[MediaType::Image, MediaType::Video, MediaType::Audio],
        }
    }

    /// Shown as a grid of pictures (else as rows).
    fn grid(self) -> bool {
        matches!(self, Category::Photos | Category::Videos)
    }

    pub fn title(self) -> &'static str {
        match self {
            Category::Photos => "Photos",
            Category::Videos => "Videos",
            Category::Audio => "Audio",
            Category::Large => "Large files",
        }
    }
}

/// The scheme of this platform's media provider.
fn scheme() -> Option<&'static str> {
    match os() {
        nsh_core::platform::Os::Android => Some(dexdrop_net::assets::ANDROID_MEDIA),
        nsh_core::platform::Os::Ios => Some("ios-photos"),
        _ => None,
    }
}

/// This platform's media provider, once registered.
pub fn provider() -> Option<Arc<dyn AssetProvider>> {
    scheme().and_then(asset::provider)
}

/// Whether Send browses media in the app (else it uses the system's
/// pickers).
pub fn offered() -> bool {
    provider().is_some()
}

/// The access granted in this run, once asked (tests and platforms whose
/// answer comes back from the prompt).
#[derive(Clone, Copy, Default, PartialEq)]
struct Asked(Option<Access>);

/// What the app may read of the media now, without asking.
fn read_access() -> Access {
    if let Some(a) = global::<Asked>().get().0 {
        return a;
    }
    // Tests stand in for a phone's answer.
    match std::env::var("DEXDROP_MEDIA_ACCESS").as_deref() {
        Ok("none") => return Access::None,
        Ok("some") => return Access::Some,
        _ => {}
    }
    #[cfg(target_os = "android")]
    {
        crate::android::media_access()
    }
    #[cfg(target_os = "ios")]
    {
        ios_access(dexdrop_photos::access())
    }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    Access::NotNeeded
}

#[cfg(target_os = "ios")]
fn ios_access(a: dexdrop_photos::Access) -> Access {
    match a {
        dexdrop_photos::Access::All | dexdrop_photos::Access::NotNeeded => Access::All,
        dexdrop_photos::Access::Some => Access::Some,
        dexdrop_photos::Access::NotAsked | dexdrop_photos::Access::Refused => Access::None,
    }
}

/// Asks with the system's prompt, then shows what was allowed.
fn ask(reload: impl Fn() + 'static) {
    Owner::detached().run(|| {
        spawn(async move {
            #[cfg(target_os = "android")]
            let answer = crate::android::ask_media().await.unwrap_or(Access::None);
            // iOS: the prompt the first time; with limited access, the
            // sheet to choose more; refused, only Settings change it.
            #[cfg(target_os = "ios")]
            let answer = {
                let now = dexdrop_photos::access();
                if now == dexdrop_photos::Access::Some {
                    dexdrop_photos::choose_more();
                }
                let asked = blocking(dexdrop_photos::request_access).await;
                ios_access(asked.unwrap_or(now))
            };
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            let answer = Access::All;
            global::<Asked>().set(Asked(Some(answer)));
            reload();
        });
    });
}

// ------------------------------------------------------------ thumbnails

/// Thumbnails made so far, by item id (`None`: the provider has none).
#[derive(Clone, Default, PartialEq)]
struct Thumbs(HashMap<AssetId, Option<PathBuf>>);

thread_local! {
    /// Asked for and not made yet, the newest last; and how many are being
    /// made.
    static WANTED: RefCell<(Vec<AssetId>, HashSet<AssetId>, usize)> =
        RefCell::new((Vec::new(), HashSet::new(), 0));
}

/// The thumbnail of `id`, when made; asks for it otherwise.
fn thumb(id: &AssetId) -> Option<PathBuf> {
    if let Some(made) = global::<Thumbs>().with(|t| t.0.get(id).cloned()) {
        return made;
    }
    want(id.clone());
    None
}

/// The thumbnail made for the item `path` references, if any (Send's list
/// of what is chosen).
pub fn thumb_of(path: &std::path::Path) -> Option<PathBuf> {
    let asset = asset::AssetPath::parse(path)?;
    global::<Thumbs>().with(|t| t.0.get(&asset.id).cloned().flatten())
}

fn want(id: AssetId) {
    WANTED.with(|w| {
        let mut w = w.borrow_mut();
        if w.1.insert(id.clone()) {
            w.0.push(id);
        }
    });
    pump();
}

/// Starts making thumbnails, the newest asked first, a few at a time.
fn pump() {
    loop {
        let next = WANTED.with(|w| {
            let mut w = w.borrow_mut();
            if w.2 >= THUMB_WORKERS {
                return None;
            }
            let id = w.0.pop()?;
            w.2 += 1;
            Some(id)
        });
        let Some(id) = next else { return };
        let Some(provider) = asset::provider(id.scheme()) else {
            WANTED.with(|w| w.borrow_mut().2 -= 1);
            continue;
        };
        let asked = id.clone();
        let making = blocking(move || provider.thumbnail(&asked, THUMB_PX).ok().flatten());
        Owner::detached().run(|| {
            spawn(async move {
                let made = making.await.ok().flatten();
                global::<Thumbs>().update(|t| {
                    t.0.insert(id.clone(), made);
                });
                WANTED.with(|w| {
                    let mut w = w.borrow_mut();
                    w.1.remove(&id);
                    w.2 -= 1;
                });
                pump();
            });
        });
    }
}

// ------------------------------------------------------------ the page

/// The browser's state: what is asked for, what came back, what is chosen.
#[derive(Clone, Copy)]
struct Browser {
    category: Signal<Category>,
    sort: Signal<Sort>,
    search: Signal<String>,
    album: Signal<Option<String>>,
    albums: Signal<Vec<(String, usize)>>,
    items: Signal<Vec<MediaItem>>,
    more: Signal<bool>,
    loading: Signal<bool>,
    failed: Signal<Option<String>>,
    /// Bumped by each new query: only the newest one's pages are shown.
    generation: Signal<u64>,
    chosen: Signal<Vec<MediaItem>>,
    access: Signal<Access>,
}

impl Browser {
    fn query(&self, offset: usize) -> Query {
        let category = self.category.get_untracked();
        let mut q = Query::of(category.kinds());
        q.offset = offset;
        q.limit = PAGE;
        q.sort = self.sort.get_untracked();
        let search = self.search.get_untracked();
        q.search = (!search.trim().is_empty()).then(|| search.trim().to_owned());
        if category == Category::Large {
            q.min_size = Some(LARGE);
        }
        if category.grid() {
            q.album = self.album.get_untracked();
        }
        q
    }

    /// Lists from the start (`reset`) or the page after what is shown.
    fn load(self, reset: bool) {
        let Some(provider) = provider() else {
            return;
        };
        if matches!(self.access.get_untracked(), Access::None) {
            self.items.set(Vec::new());
            return;
        }
        if !reset && (self.loading.get_untracked() || !self.more.get_untracked()) {
            return;
        }
        let generation = if reset {
            self.generation.update(|g| *g += 1);
            self.generation.get_untracked()
        } else {
            self.generation.get_untracked()
        };
        let offset = if reset {
            0
        } else {
            self.items.with_untracked(Vec::len)
        };
        let query = self.query(offset);
        self.loading.set(true);
        let listing = blocking(move || provider.list(&query));
        let this = self;
        Owner::detached().run(|| {
            spawn(async move {
                let page = listing.await;
                if this.generation.get_untracked() != generation {
                    return; // A newer query is showing.
                }
                this.loading.set(false);
                match page {
                    Ok(Ok(page)) => {
                        this.failed.set(None);
                        this.more.set(page.more);
                        if reset {
                            this.items.set(page.items);
                        } else {
                            this.items.update(|i| i.extend(page.items));
                        }
                    }
                    Ok(Err(err)) => {
                        this.more.set(false);
                        this.failed.set(Some(err.to_string()));
                    }
                    Err(_) => this.more.set(false),
                }
            });
        });
    }

    fn load_albums(self) {
        let Some(provider) = provider() else {
            return;
        };
        let category = self.category.get_untracked();
        if !category.grid() || matches!(self.access.get_untracked(), Access::None) {
            self.albums.set(Vec::new());
            return;
        }
        let listing = blocking(move || provider.albums(category.kinds()));
        let albums = self.albums;
        Owner::detached().run(|| {
            spawn(async move {
                albums.set(listing.await.ok().and_then(Result::ok).unwrap_or_default());
            });
        });
    }

    fn is_chosen(&self, id: &AssetId) -> bool {
        self.chosen.with(|c| c.iter().any(|m| &m.id == id))
    }

    fn toggle(&self, item: MediaItem) {
        self.chosen.update(|c| {
            if let Some(i) = c.iter().position(|m| m.id == item.id) {
                c.remove(i);
            } else {
                c.push(item);
            }
        });
    }

    /// Every item listed so far chosen, or none of them if all were.
    fn toggle_all(&self) {
        let shown = self.items.get_untracked();
        let all = shown.iter().all(|m| self.is_chosen(&m.id));
        self.chosen.update(|c| {
            if all {
                c.retain(|m| !shown.iter().any(|s| s.id == m.id));
            } else {
                for m in shown {
                    if !c.iter().any(|x| x.id == m.id) {
                        c.push(m);
                    }
                }
            }
        });
    }
}

/// The browser, opened on `category`.
pub fn page(category: Category) -> impl IntoElement {
    let b = Browser {
        category: signal(category),
        sort: signal(if category == Category::Large {
            Sort::Largest
        } else {
            Sort::Newest
        }),
        search: signal(String::new()),
        album: signal(None),
        albums: signal(Vec::new()),
        items: signal(Vec::new()),
        more: signal(false),
        loading: signal(true),
        failed: signal(None),
        generation: signal(0),
        chosen: signal(Vec::new()),
        access: signal(read_access()),
    };
    // A new category, order, search or album lists from the start.
    effect(move || {
        let _ = (
            b.category.get(),
            b.sort.get(),
            b.search.get(),
            b.album.get(),
            b.access.get(),
        );
        untracked(|| b.load(true));
    });
    effect(move || {
        let _ = (b.category.get(), b.access.get());
        untracked(|| {
            b.album.set(None);
            b.load_albums();
        });
    });
    // Switching to Large files sorts by size; back, newest first.
    let first = std::cell::Cell::new(true);
    effect(move || {
        let c = b.category.get();
        if first.replace(false) {
            return;
        }
        untracked(|| {
            b.sort.set(if c == Category::Large {
                Sort::Largest
            } else {
                Sort::Newest
            })
        });
    });

    view()
        .flex_1()
        .test_id("library")
        .bg(move || theme().background)
        .child(dynamic(move || match b.access.get() {
            Access::None => allow(b).into_element(),
            _ => grid(b).into_element(),
        }))
        .child(when(
            move || b.chosen.with(|c| !c.is_empty()),
            move || add_bar(b),
        ))
}

/// The columns of the grid: four on a phone held upright, more wider.
fn columns() -> u32 {
    let w = window().width.0;
    if w >= 900.0 {
        7
    } else if w >= 600.0 {
        5
    } else {
        4
    }
}

/// The page's width for its content.
fn inner_width() -> f32 {
    window().width.0.min(ui::content_width() + 2.0 * space::L) - 2.0 * space::L
}

const GAP: f32 = 3.0;

fn grid(b: Browser) -> impl IntoElement {
    dynamic(move || {
        let category = b.category.get();
        let cols = if category.grid() { columns() } else { 1 };
        let side = ((inner_width() - (cols - 1) as f32 * GAP) / cols as f32).floor();
        let list = virtual_list(
            move || b.items.with(Vec::len),
            move |at| {
                let item = move || b.items.with(|i| i.get(at.index()).cloned());
                if category.grid() {
                    tile(b, item, side).into_element()
                } else {
                    item_row(b, item).into_element()
                }
            },
        )
        .keys(move || {
            b.items
                .with(|i| i.iter().map(|m| m.id.clone()).collect::<Vec<_>>())
        })
        .columns(cols)
        .estimated_item_size(px(if category.grid() { side } else { 64.0 }))
        .row_gap(px(if category.grid() { GAP } else { 0.0 }))
        .column_gap(px(GAP))
        .content_padding_x(px(space::L))
        .on_load_more(move || b.load(false))
        .load_more_threshold(1.5)
        .header(controls(b))
        .footer(footer(b))
        .empty(empty(b));
        let list = if category.grid() {
            list
        } else {
            list.separator(|| ui::separator(72.0))
        };
        list.flex_1().into_element()
    })
}

/// Above the grid: the categories, search and order, the albums, and
/// "Select all".
fn controls(b: Browser) -> impl IntoElement {
    view()
        .gap(px(space::M))
        .pt(px(space::S))
        .pb(px(space::M))
        .child(ui::segmented(CATEGORIES, b.category))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(space::S))
                .child(search_field(b.search).flex_1())
                .child(sort_button(b.sort)),
        )
        .child(when(
            move || b.category.get().grid() && b.albums.with(|a| a.len() > 1),
            move || albums(b),
        ))
        .child(when(
            move || b.access.get() == Access::Some,
            move || limited(b),
        ))
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .child(muted(move || shown_count(b)).text_size(px(ui::size::SMALL)))
                .child(when(
                    move || b.items.with(|i| !i.is_empty()),
                    move || {
                        ui::link_btn("Select all")
                            .test_id("select-all")
                            .accessibility_label(move || {
                                let all = b.items.with(|i| i.iter().all(|m| b.is_chosen(&m.id)));
                                Some(SharedString::from(if all {
                                    "Deselect all"
                                } else {
                                    "Select all"
                                }))
                            })
                            .on_press(move || b.toggle_all())
                    },
                )),
        )
}

fn shown_count(b: Browser) -> String {
    let n = b.items.with(Vec::len);
    let more = if b.more.get() { "+" } else { "" };
    match (n, b.category.get()) {
        (0, _) if b.loading.get() => String::new(),
        (1, _) => "1 item".to_owned(),
        (n, _) => format!("{n}{more} items"),
    }
}

fn search_field(search: Signal<String>) -> View {
    view()
        .flex_row()
        .items_center()
        .gap(px(space::S))
        .px(px(space::M))
        .rounded(px(999.0))
        .bg(ui::surface)
        .child(ui::icon(Icon::Search, 18.0, move || {
            theme().muted_foreground
        }))
        .child(
            nsh_base::input()
                .test_id("library-search")
                .value(move || search.get())
                .on_change(move |v: &str| search.set(v.to_owned()))
                .placeholder("Search by name")
                .label("Search by name")
                .flex_1()
                .min_h(px(ui::hit()))
                .text_size(px(ui::size::BODY))
                .text_color(move || theme().foreground),
        )
}

/// The order, a tap to the next one.
fn sort_button(sort: Signal<Sort>) -> impl IntoElement {
    let label = move || {
        SORTS
            .iter()
            .find(|(s, _)| *s == sort.get())
            .map_or("Newest", |(_, l)| *l)
    };
    ui::pressable()
        .test_id("library-sort")
        .accessibility_label(move || Some(SharedString::from(format!("Sort: {}", label()))))
        .on_press(move || {
            let at = SORTS
                .iter()
                .position(|(s, _)| *s == sort.get_untracked())
                .unwrap_or(0);
            sort.set(SORTS[(at + 1) % SORTS.len()].0);
        })
        .flex_row()
        .items_center()
        .gap(px(space::XS))
        .min_h(px(ui::hit()))
        .px(px(space::M))
        .rounded(px(999.0))
        .bg(ui::surface)
        .active(|s| s.opacity(0.7))
        .child(ui::icon(Icon::Sliders, 16.0, move || theme().primary))
        .child(
            text(label)
                .text_size(px(ui::size::SMALL))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().primary),
        )
}

/// The albums as chips, "All" first.
fn albums(b: Browser) -> impl IntoElement {
    scroll().horizontal().child(
        view()
            .flex_row()
            .gap(px(space::S))
            .child(chip(b, None, "All".to_owned()))
            .children(
                b.albums
                    .get_untracked()
                    .into_iter()
                    .map(move |(name, n)| chip(b, Some(name.clone()), format!("{name} {n}"))),
            ),
    )
}

fn chip(b: Browser, album: Option<String>, label: String) -> impl IntoElement {
    let on = {
        let album = album.clone();
        move || b.album.with(|a| *a == album)
    };
    let on2 = on.clone();
    ui::pressable()
        .accessibility_label(label.clone())
        .selected(on.clone())
        .on_press(move || b.album.set(album.clone()))
        .min_h(px(36.0))
        .px(px(space::M))
        .items_center()
        .justify_center()
        .rounded(px(999.0))
        .bg(move || if on() { theme().primary } else { ui::surface() })
        .child(
            text(label)
                .number_of_lines(1)
                .text_size(px(ui::size::SMALL))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if on2() {
                        theme().primary_foreground
                    } else {
                        theme().foreground
                    }
                }),
        )
}

/// Android 14's and iOS's limited access: only what the user chose shows.
fn limited(b: Browser) -> impl IntoElement {
    ui::flat_group().child(ui::nav_row(
        ui::source_badge(Icon::Photo, ui::source::PHOTOS, 30.0),
        "Showing the items you chose",
        "Choose more, or allow all",
        move || ask(move || b.access.set(read_access())),
    ))
}

/// Before the user allows reading their media: what it is for, and the
/// system's prompt a tap away.
fn allow(b: Browser) -> impl IntoElement {
    view()
        .flex_1()
        .items_center()
        .justify_center()
        .gap(px(space::L))
        .px(px(space::XL))
        .child(ui::source_badge(Icon::Photo, ui::source::PHOTOS, 64.0))
        .child(ui::title("Send your photos and videos").text_center())
        .child(
            muted(
                "DexDrop lists them here so you can choose several at once. \
                 They are read only when you send them.",
            )
            .text_center(),
        )
        .child(
            ui::btn("Allow access", Variant::Primary)
                .test_id("allow-media")
                .w_full()
                .max_w(px(360.0))
                .on_press(move || ask(move || b.access.set(read_access()))),
        )
}

fn empty(b: Browser) -> impl IntoElement {
    view()
        .py(px(space::XXL))
        .items_center()
        .child(dynamic(move || {
            if b.loading.get() {
                return ui::spinner(20.0).into_element();
            }
            if let Some(why) = b.failed.get() {
                return muted(why).text_center().into_element();
            }
            let what = if !b.search.with(|s| s.trim().is_empty()) {
                "Nothing by that name."
            } else {
                match b.category.get() {
                    Category::Photos => "No photos yet.",
                    Category::Videos => "No videos yet.",
                    Category::Audio => "No audio yet.",
                    Category::Large => "Nothing of 100 MB or more.",
                }
            };
            muted(what).text_center().into_element()
        }))
}

/// Below the grid: a spinner while the next page comes, and room for the
/// bar at the foot.
fn footer(b: Browser) -> impl IntoElement {
    view()
        .items_center()
        .pt(px(space::M))
        .pb(px(ui::above_tab_bar() + 96.0))
        .child(when(
            move || b.loading.get() && b.items.with(|i| !i.is_empty()),
            || ui::spinner(18.0),
        ))
}

/// A tile of the grid: the picture, a video's length, the tick. Reads
/// `item` in each binding, so a recycled cell rebinds to its new item.
fn tile(
    b: Browser,
    item: impl Fn() -> Option<MediaItem> + Copy + 'static,
    side: f32,
) -> impl IntoElement {
    let id = move || item().map(|i| i.id);
    let on = move || id().is_some_and(|id| b.is_chosen(&id));
    let name = move || item().map(|i| i.name).unwrap_or_default();
    let picture = move || {
        id().and_then(|id| thumb(&id))
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    let duration = move || {
        item()
            .and_then(|i| i.duration)
            .map(clock)
            .unwrap_or_default()
    };
    ui::pressable()
        .accessibility_label(move || Some(SharedString::from(name())))
        .selected(on)
        .on_press(move || {
            if let Some(i) = item() {
                b.toggle(i);
            }
        })
        .size(px(side))
        .overflow_hidden()
        .bg(move || theme().muted)
        .child(
            image(picture)
                .absolute()
                .top_0()
                .left_0()
                .size(px(side))
                .object_fit(ResizeMode::Cover),
        )
        // Chosen: dimmed, with a ring.
        .child(when(on, move || {
            view()
                .absolute()
                .size_full()
                .border(px(3.0))
                .border_color(move || theme().primary)
                .bg(rgba(0xffff_ff33))
        }))
        .child(when(
            move || !duration().is_empty(),
            move || {
                text(duration)
                    .absolute()
                    .bottom(px(4.0))
                    .right(px(6.0))
                    .text_size(px(ui::size::CAPTION))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(white())
            },
        ))
        .child(
            view()
                .absolute()
                .top(px(6.0))
                .right(px(6.0))
                .child(tick(on)),
        )
}

/// A video's or a song's length: `1:05`, `1:02:09`.
pub fn clock(ms: u64) -> String {
    let s = ms / 1000;
    let (h, m, s) = (s / 3600, (s / 60) % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// A round tick: an empty ring, filled when chosen.
fn tick(on: impl Fn() -> bool + Copy + 'static) -> impl IntoElement {
    view()
        .size(px(24.0))
        .rounded(px(12.0))
        .items_center()
        .justify_center()
        .border_2()
        .border_color(move || {
            if on() {
                theme().primary
            } else {
                white().opacity(0.9).into()
            }
        })
        .bg(move || {
            if on() {
                theme().primary
            } else {
                rgba(0x0000_0033).into()
            }
        })
        .child(when(on, || {
            ui::icon(Icon::Check, 14.0, move || theme().primary_foreground)
        }))
}

/// A row of a list (audio, large files): its picture or kind, name, size,
/// length and date, the tick.
fn item_row(b: Browser, item: impl Fn() -> Option<MediaItem> + Copy + 'static) -> impl IntoElement {
    let id = move || item().map(|i| i.id);
    let on = move || id().is_some_and(|id| b.is_chosen(&id));
    let name = move || item().map(|i| i.name).unwrap_or_default();
    let detail = move || {
        let Some(i) = item() else {
            return String::new();
        };
        let mut d = vec![i.size.map(format::bytes).unwrap_or_default()];
        if let Some(ms) = i.duration {
            d.push(clock(ms));
        }
        if let Some(t) = i.modified_at.or(i.created_at) {
            d.push(crate::pages::history::ago(t.max(0) as u64));
        }
        d.retain(|s| !s.is_empty());
        d.join(" · ")
    };
    let picture = move || {
        item()
            .filter(|i| i.media_type != MediaType::Audio)
            .and_then(|i| thumb(&i.id))
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    ui::pressable()
        .accessibility_label(move || Some(SharedString::from(name())))
        .selected(on)
        .on_press(move || {
            if let Some(i) = item() {
                b.toggle(i);
            }
        })
        .flex_row()
        .items_center()
        .gap(px(space::M))
        .min_h(px(ui::hit() + 20.0))
        .active(move |s| s.bg(move || theme().muted))
        .child(
            view()
                .size(px(44.0))
                .rounded(px(10.0))
                .overflow_hidden()
                .child(dynamic(move || ui::file_badge(&name(), 44.0, 10.0)))
                .child(
                    image(picture)
                        .absolute()
                        .top_0()
                        .left_0()
                        .size(px(44.0))
                        .object_fit(ResizeMode::Cover),
                ),
        )
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .gap(px(2.0))
                .child(ui::body(name).number_of_lines(1))
                .child(
                    muted(detail)
                        .text_size(px(ui::size::CAPTION))
                        .number_of_lines(1),
                ),
        )
        .child(
            view()
                .rounded(px(12.0))
                .bg(move || theme().muted_foreground.opacity(0.35))
                .child(tick(on)),
        )
}

/// At the foot, in thumb reach: how many are chosen and their size, and
/// "Add".
fn add_bar(b: Browser) -> impl IntoElement {
    let summary = move || {
        b.chosen.with(|c| {
            let bytes: u64 = c.iter().filter_map(|m| m.size).sum();
            let n = c.len();
            format!("{n} selected · {}", format::bytes(bytes))
        })
    };
    view()
        .absolute()
        .left_0()
        .right_0()
        .bottom(px(ui::above_tab_bar()))
        .px(px(space::L))
        .pt(px(space::S))
        .pb(px(space::L))
        .gap(px(space::S))
        .items_center()
        .bg(move || theme().background)
        .child(
            muted(summary)
                .test_id("library-summary")
                .text_size(px(ui::size::SMALL)),
        )
        .child(
            ui::icon_btn(Icon::Plus, "Add", Variant::Primary)
                .test_id("library-add")
                .accessibility_label(move || {
                    let n = b.chosen.with(Vec::len);
                    Some(SharedString::from(if n == 1 {
                        "Add 1 item".to_owned()
                    } else {
                        format!("Add {n} items")
                    }))
                })
                .w_full()
                .max_w(px(420.0))
                .on_press(move || {
                    let chosen = b.chosen.get_untracked();
                    send::select(chosen.iter().map(MediaItem::path).collect());
                    pop();
                }),
        )
}
