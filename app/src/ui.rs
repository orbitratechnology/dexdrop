//! DexDrop's design system: a small set of components built on the theme's
//! tokens, icons drawn as tinted SVGs, and the platform's own controls
//! where they exist (switches, segmented controls, the tab bar, headers).
//! Pages compose these instead of styling raw views.

use nsh_base::{ButtonElement, theme};
use nsh_core::{element::AccessibilityRole, prelude::*};

use crate::state::desktop_platform;

/// Spacing scale, in points.
pub mod space {
    pub const XS: f32 = 4.0;
    pub const S: f32 = 8.0;
    pub const M: f32 = 12.0;
    pub const L: f32 = 16.0;
    pub const XL: f32 = 24.0;
    pub const XXL: f32 = 32.0;
}

/// Type scale, in points.
pub mod size {
    pub const DISPLAY: f32 = 34.0;
    pub const TITLE: f32 = 22.0;
    pub const HEADING: f32 = 17.0;
    pub const BODY: f32 = 16.0;
    pub const SMALL: f32 = 14.0;
    pub const CAPTION: f32 = 12.0;
}

/// The smallest hit target: 44 pt on iOS, 48 dp on Android, compact on
/// pointer-driven desktops (PRD 7.7).
pub fn hit() -> f32 {
    match os().name() {
        "android" => 48.0,
        "ios" => 44.0,
        _ => 36.0,
    }
}

/// The user chose large system text (NSH's `font_scale`): rows of
/// labelled controls go one above the other so their labels fit.
pub fn large_text() -> bool {
    font_scale() > 1.3
}

/// Whether this is a phone or tablet (iOS, Android).
pub fn mobile() -> bool {
    matches!(os().name(), "ios" | "android")
}

// ---------------------------------------------------------------- color

pub fn success() -> AdaptiveColor {
    theme().color("success").unwrap_or(theme().primary)
}

pub fn warning() -> AdaptiveColor {
    theme().color("warning").unwrap_or(theme().destructive)
}

/// The two actions' marks: sending and receiving. The app is black and
/// white; these stay for the few places a pair of colors still tells the
/// two apart (avatars of the past, file kinds keep their own colors).
pub const SEND: (u32, u32) = (0x6366f1, 0x8b5cf6);
pub const RECEIVE: (u32, u32) = (0x10b981, 0x0d9488);

/// A gradient-filled view.
pub fn gradient(colors: (u32, u32)) -> View {
    view()
        .bg(rgb(colors.0))
        .bg_linear(deg(135.0))
        .gradient_from(rgb(colors.0))
        .gradient_to(rgb(colors.1))
}

/// Avatar colors: a device without a chosen avatar gets one by its name;
/// a chosen avatar names its own ([`dexdrop_core::avatar`]).
pub const AVATAR: [(u32, u32); dexdrop_core::avatar::COLORS as usize] = [
    (0x6366f1, 0x8b5cf6),
    (0x0ea5e9, 0x3b82f6),
    (0x10b981, 0x14b8a6),
    (0xf59e0b, 0xf97316),
    (0xec4899, 0xf43f5e),
    (0x8b5cf6, 0xd946ef),
    (0x64748b, 0x334155),
    (0x84cc16, 0x16a34a),
];

fn avatar_colors(name: &str) -> (u32, u32) {
    // FNV-1a: similar names still spread over the colors.
    let hash = name.bytes().fold(0x811c_9dc5u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    });
    AVATAR[(hash % AVATAR.len() as u32) as usize]
}

// ---------------------------------------------------------------- icons

/// The app's icons: one-color SVGs tinted to match the text around them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Send,
    Receive,
    Paperplane,
    Clock,
    Gear,
    Photo,
    Video,
    File,
    Folder,
    Music,
    Archive,
    Monitor,
    Pause,
    Play,
    X,
    Check,
    ChevronRight,
    Plus,
    Refresh,
    Qr,
    Keypad,
    Link,
    Copy,
    Shield,
    Eye,
    Wifi,
    Globe,
    Pencil,
    Trash,
    Info,
    Alert,
    Devices,
    Share,
    UserCheck,
    Lock,
    Apple,
    Windows,
    Android,
    Linux,
    Doc,
    Sheet,
    Slides,
    Code,
    Font,
    Book,
    Disk,
    Vector,
    Database,
    Calendar,
    Person,
    Package,
    Pdf,
    Markdown,
    Search,
}

impl Icon {
    /// The icon: an SVG of the app's assets (`assets/icons/`), shipped with
    /// the app and checked at build time (NSH's images and assets).
    fn asset(self) -> ImageAsset {
        match self {
            Icon::Send => asset!("icons/send.svg"),
            Icon::Receive => asset!("icons/receive.svg"),
            Icon::Paperplane => asset!("icons/paperplane.svg"),
            Icon::Clock => asset!("icons/clock.svg"),
            Icon::Gear => asset!("icons/gear.svg"),
            Icon::Photo => asset!("icons/photo.svg"),
            Icon::Video => asset!("icons/video.svg"),
            Icon::File => asset!("icons/file.svg"),
            Icon::Folder => asset!("icons/folder.svg"),
            Icon::Music => asset!("icons/music.svg"),
            Icon::Archive => asset!("icons/archive.svg"),
            Icon::Monitor => asset!("icons/monitor.svg"),
            Icon::Pause => asset!("icons/pause.svg"),
            Icon::Play => asset!("icons/play.svg"),
            Icon::X => asset!("icons/x.svg"),
            Icon::Check => asset!("icons/check.svg"),
            Icon::ChevronRight => asset!("icons/chevron-right.svg"),
            Icon::Plus => asset!("icons/plus.svg"),
            Icon::Refresh => asset!("icons/refresh.svg"),
            Icon::Qr => asset!("icons/qr.svg"),
            Icon::Keypad => asset!("icons/keypad.svg"),
            Icon::Link => asset!("icons/link.svg"),
            Icon::Copy => asset!("icons/copy.svg"),
            Icon::Shield => asset!("icons/shield.svg"),
            Icon::Eye => asset!("icons/eye.svg"),
            Icon::Wifi => asset!("icons/wifi.svg"),
            Icon::Globe => asset!("icons/globe.svg"),
            Icon::Pencil => asset!("icons/pencil.svg"),
            Icon::Trash => asset!("icons/trash.svg"),
            Icon::Info => asset!("icons/info.svg"),
            Icon::Alert => asset!("icons/alert.svg"),
            Icon::Devices => asset!("icons/devices.svg"),
            Icon::Share => asset!("icons/share.svg"),
            Icon::UserCheck => asset!("icons/user-check.svg"),
            Icon::Lock => asset!("icons/lock.svg"),
            Icon::Apple => asset!("icons/os-apple.svg"),
            Icon::Windows => asset!("icons/os-windows.svg"),
            Icon::Android => asset!("icons/os-android.svg"),
            Icon::Linux => asset!("icons/os-linux.svg"),
            Icon::Doc => asset!("icons/doc.svg"),
            Icon::Sheet => asset!("icons/sheet.svg"),
            Icon::Slides => asset!("icons/slides.svg"),
            Icon::Code => asset!("icons/code.svg"),
            Icon::Font => asset!("icons/font.svg"),
            Icon::Book => asset!("icons/book.svg"),
            Icon::Disk => asset!("icons/disk.svg"),
            Icon::Vector => asset!("icons/vector.svg"),
            Icon::Database => asset!("icons/database.svg"),
            Icon::Calendar => asset!("icons/calendar.svg"),
            Icon::Person => asset!("icons/person.svg"),
            Icon::Package => asset!("icons/package.svg"),
            Icon::Pdf => asset!("icons/pdf.svg"),
            Icon::Markdown => asset!("icons/markdown.svg"),
            Icon::Search => asset!("icons/search.svg"),
        }
    }
}

/// An icon at `size` points in `color`.
pub fn icon(
    icon: Icon,
    size: f32,
    color: impl IntoReactive<AdaptiveColor>,
) -> nsh_core::element::Image {
    image(icon.asset()).size(px(size)).tint_color(color)
}

/// An icon filling `side`, in the page's foreground: the black-and-white
/// mark of a row or a heading. No tile behind it (`_colors` stays for the
/// callers that once had one).
pub fn glyph(i: Icon, _colors: (u32, u32), side: f32) -> View {
    view()
        .size(px(side))
        .items_center()
        .justify_center()
        .child(icon(i, side, move || theme().foreground))
}

/// The color of a kind of file.
pub fn kind_color(i: Icon) -> u32 {
    match i {
        Icon::Folder => 0xf59e0b,
        other => file_kind_of_icon(other),
    }
}

fn file_kind_of_icon(i: Icon) -> u32 {
    match i {
        Icon::Photo => 0xec4899,
        Icon::Video => 0x8b5cf6,
        Icon::Music => 0xf59e0b,
        Icon::Archive => 0x78716c,
        Icon::Pdf => 0xef4444,
        Icon::Sheet => 0x16a34a,
        Icon::Slides => 0xea580c,
        Icon::Code => 0x6366f1,
        Icon::Markdown => 0x475569,
        Icon::Font => 0x0891b2,
        Icon::Book => 0xa16207,
        Icon::Disk => 0x475569,
        Icon::Vector => 0xd97706,
        Icon::Database => 0x0ea5e9,
        Icon::Calendar => 0xdc2626,
        Icon::Person => 0x0d9488,
        Icon::Android => 0x16a34a,
        Icon::Apple => 0x4b5563,
        Icon::Windows => 0x0078d4,
        Icon::Linux => 0xb45309,
        Icon::Package => 0x7c3aed,
        _ => 0x3b82f6,
    }
}

/// A file's kind by its extension: its icon, its color and the short tag
/// shown on it ("PDF", "XLSX"; none for files without a known one).
pub fn file_kind(name: &str) -> (Icon, u32, Option<String>) {
    // A transfer's label ("Holiday.jpg and 2 more") goes by its first file.
    let name = name.split_once(" and ").map_or(name, |(first, _)| first);
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    let icon = match ext.as_str() {
        "jpg" | "jpeg" | "png" | "heic" | "heif" | "gif" | "webp" | "bmp" | "tif" | "tiff"
        | "raw" | "dng" | "cr2" | "nef" | "arw" | "avif" | "ico" => Icon::Photo,
        "mp4" | "mov" | "m4v" | "mkv" | "avi" | "webm" | "3gp" | "wmv" | "flv" | "mpg" | "mpeg"
        | "ts" => Icon::Video,
        "mp3" | "m4a" | "aac" | "wav" | "flac" | "ogg" | "opus" | "wma" | "aiff" | "mid"
        | "midi" | "amr" => Icon::Music,
        "zip" | "rar" | "7z" | "tar" | "gz" | "tgz" | "xz" | "bz2" | "zst" | "lz" | "cab" => {
            Icon::Archive
        }
        "pdf" => Icon::Pdf,
        "doc" | "docx" | "odt" | "rtf" | "pages" | "txt" | "log" | "wpd" => Icon::Doc,
        "xls" | "xlsx" | "xlsm" | "ods" | "numbers" | "csv" | "tsv" => Icon::Sheet,
        "ppt" | "pptx" | "odp" | "key" => Icon::Slides,
        "md" | "markdown" | "mdx" => Icon::Markdown,
        "html" | "htm" | "css" | "js" | "mjs" | "tsx" | "jsx" | "json" | "xml" | "yml" | "yaml"
        | "toml" | "ini" | "rs" | "py" | "java" | "kt" | "swift" | "c" | "h" | "cpp" | "hpp"
        | "cs" | "go" | "rb" | "php" | "sh" | "bat" | "ps1" | "sql" | "dart" | "lua" | "vue"
        | "svelte" => Icon::Code,
        "svg" | "ai" | "eps" | "psd" | "fig" | "sketch" | "xd" | "afdesign" => Icon::Vector,
        "ttf" | "otf" | "woff" | "woff2" => Icon::Font,
        "epub" | "mobi" | "azw" | "azw3" | "fb2" | "djvu" | "cbz" | "cbr" => Icon::Book,
        "iso" | "img" | "vhd" | "vhdx" | "vmdk" | "qcow2" => Icon::Disk,
        "db" | "sqlite" | "sqlite3" | "mdb" | "accdb" => Icon::Database,
        "ics" | "ical" => Icon::Calendar,
        "vcf" | "vcard" => Icon::Person,
        "apk" | "aab" | "apks" | "xapk" | "apkm" => Icon::Android,
        "dmg" | "pkg" | "ipa" | "app" => Icon::Apple,
        "exe" | "msi" | "msix" | "appx" | "msixbundle" => Icon::Windows,
        "deb" | "rpm" | "appimage" | "flatpak" | "snap" => Icon::Linux,
        "jar" | "crx" | "xpi" | "vsix" => Icon::Package,
        _ => Icon::File,
    };
    // A few kinds get their own shade.
    let color = match ext.as_str() {
        "gif" => 0xd946ef,
        "csv" | "tsv" => 0x0d9488,
        "html" | "htm" => 0xe34f26,
        "svg" => 0xf59e0b,
        "txt" | "log" => 0x64748b,
        "doc" | "docx" | "odt" | "rtf" | "pages" => 0x2563eb,
        _ => file_kind_of_icon(icon),
    };
    let tag = (!ext.is_empty() && ext.len() <= 5 && ext.chars().all(|c| c.is_ascii_alphanumeric()))
        .then(|| ext.to_ascii_uppercase());
    (icon, color, tag)
}

/// A file's picture when there is no preview: its kind's icon in its
/// kind's color, filling `side` (no tile behind it). `_radius` stays for
/// the callers that once rounded a tile.
pub fn file_badge(name: &str, side: f32, _radius: f32) -> View {
    let (i, c, _tag) = file_kind(name);
    view()
        .size(px(side))
        .items_center()
        .justify_center()
        .child(icon(i, side, rgb(c)))
}

/// A device's operating system: its mark and the mark's color (`None`:
/// the page's foreground, for black marks such as Apple's).
pub fn os_mark(platform: &str) -> (Icon, Option<u32>) {
    match platform {
        "ios" | "macos" => (Icon::Apple, None),
        "windows" => (Icon::Windows, Some(0x0078d4)),
        "android" => (Icon::Android, Some(0x3ddc84)),
        "linux" => (Icon::Linux, Some(0xf59e0b)),
        "web" => (Icon::Globe, None),
        _ => (Icon::Monitor, None),
    }
}

/// A device's operating system: its mark, in its color, filling `side`.
pub fn os_badge(platform: &str, side: f32) -> View {
    let (i, color) = os_mark(platform);
    view()
        .size(px(side))
        .items_center()
        .justify_center()
        .child(match color {
            Some(c) => icon(i, side, rgb(c)),
            None => icon(i, side, move || theme().foreground),
        })
}

/// A file's kind: its icon in its color, filling `side`.
pub fn kind_glyph(i: Icon, side: f32) -> View {
    let c = kind_color(i);
    view()
        .size(px(side))
        .items_center()
        .justify_center()
        .child(icon(i, side, rgb(c)))
}

/// An icon in its own color, filling `side` (no circle behind it).
pub fn soft_glyph(i: Icon, color: impl Fn() -> AdaptiveColor + Copy + 'static, side: f32) -> View {
    view()
        .size(px(side))
        .items_center()
        .justify_center()
        .child(icon(i, side, color))
}

// ---------------------------------------------------------------- text

pub fn title(content: impl IntoReactive<SharedString> + 'static) -> Text {
    text(content)
        .text_size(px(size::TITLE))
        .font_weight(FontWeight::BOLD)
        .text_color(move || theme().foreground)
}

pub fn body(content: impl IntoReactive<SharedString> + 'static) -> Text {
    text(content)
        .text_size(px(size::BODY))
        .text_color(move || theme().foreground)
}

pub fn strong(content: impl IntoReactive<SharedString> + 'static) -> Text {
    body(content).font_weight(FontWeight::SEMIBOLD)
}

/// Secondary text.
pub fn muted(content: impl IntoReactive<SharedString> + 'static) -> Text {
    text(content)
        .text_size(px(size::SMALL))
        .text_color(move || theme().muted_foreground)
}

/// A section's title over a group.
pub fn section(label: impl Into<SharedString>) -> Text {
    text(label.into())
        .text_size(px(size::HEADING + 1.0))
        .font_weight(FontWeight::BOLD)
        .text_color(move || theme().foreground)
        .role(AccessibilityRole::Header)
}

// ---------------------------------------------------------------- buttons

/// A button's look.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    /// The main action: filled.
    Primary,
    /// A quieter action: tinted.
    Secondary,
    /// Text only.
    Plain,
    /// Destructive: red, tinted.
    Danger,
}

/// A capsule button with a label and an optional icon.
/// A press target with the keyboard's focus ring (PRD 7.5, 7.7): every
/// control of the app is one, so Tab always shows where it is.
pub fn pressable() -> ButtonElement {
    nsh_base::button().focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
}

pub fn btn(label: impl Into<SharedString>, variant: Variant) -> ButtonElement {
    labelled(label, None, variant)
}

/// [`btn`] with a leading icon.
pub fn icon_btn(i: Icon, label: impl Into<SharedString>, variant: Variant) -> ButtonElement {
    labelled(label, Some(i), variant)
}

fn labelled(label: impl Into<SharedString>, i: Option<Icon>, variant: Variant) -> ButtonElement {
    let label: SharedString = label.into();
    type Color = fn() -> AdaptiveColor;
    let (bg, fg): (Color, Color) = match variant {
        Variant::Primary => (|| theme().primary, || theme().primary_foreground),
        Variant::Secondary => (|| theme().primary.opacity(0.12), || theme().primary),
        Variant::Plain => (|| theme().background.opacity(0.0), || theme().primary),
        Variant::Danger => (|| theme().destructive.opacity(0.12), || theme().destructive),
    };
    let mut b = pressable()
        .min_h(px(hit() + if mobile() { 6.0 } else { 0.0 }))
        .px(px(if variant == Variant::Plain { 8.0 } else { 20.0 }))
        .flex_row()
        .items_center()
        .justify_center()
        .gap(px(space::S))
        .rounded(px(999.0))
        .bg(bg)
        .disabled_style(|s| s.opacity(0.4))
        .active(|s| s.opacity(0.7).scale(0.98))
        .hover(|s| s.opacity(0.88))
        .accessibility_label(label.clone());
    if let Some(i) = i {
        b = b.child(icon(i, 18.0, fg));
    }
    b.child(
        text(label)
            .text_size(px(size::BODY))
            .font_weight(FontWeight::SEMIBOLD)
            .number_of_lines(1)
            .text_color(fg),
    )
}

/// A round icon-only button, labelled for screen readers.
pub fn round_btn(
    i: Icon,
    label: impl Into<SharedString>,
    variant: Variant,
    side: f32,
) -> ButtonElement {
    type Color = fn() -> AdaptiveColor;
    let (bg, fg): (Color, Color) = match variant {
        Variant::Primary => (|| theme().primary, || theme().primary_foreground),
        Variant::Secondary | Variant::Plain => (|| theme().muted, || theme().foreground),
        Variant::Danger => (|| theme().destructive.opacity(0.12), || theme().destructive),
    };
    pressable()
        .size(px(side))
        .rounded(px(side / 2.0))
        .items_center()
        .justify_center()
        .bg(bg)
        .active(|s| s.opacity(0.7).scale(0.94))
        .hover(|s| s.opacity(0.88))
        .accessibility_label(label.into())
        .child(icon(i, side * 0.42, fg))
}

// ---------------------------------------------------------------- surfaces

/// The fill of cards and groups: one step above the page's background, in
/// light and dark alike (every page paints the theme's background).
pub fn surface() -> AdaptiveColor {
    theme().card
}

/// A rounded group of rows on the page's grouped background.
pub fn group() -> View {
    view().rounded(px(16.0)).bg(surface).overflow_hidden()
}

/// A tile: a rounded card.
pub fn tile() -> View {
    view()
        .p(px(space::L))
        .gap(px(space::M))
        .rounded(px(18.0))
        .bg(surface)
}

/// A row of a [`group`]: leading view, a title with a detail line under it
/// (left out when empty), and a trailing view.
pub fn row(
    leading: impl IntoElement,
    title: impl IntoReactive<SharedString> + 'static,
    detail: impl Fn() -> String + Copy + 'static,
    trailing: impl IntoElement,
) -> View {
    view()
        .flex_row()
        .items_center()
        .gap(px(space::M))
        .min_h(px(hit() + 12.0))
        .px(px(space::L))
        .py(px(space::S))
        .child(leading)
        .child(
            view()
                .flex_1()
                .gap(px(2.0))
                .child(body(title).number_of_lines(1))
                .child(when(
                    move || !detail().is_empty(),
                    move || muted(detail).number_of_lines(1),
                )),
        )
        .child(trailing)
}

/// A pressable row with a chevron.
pub fn nav_row(
    leading: impl IntoElement,
    title: impl IntoReactive<SharedString> + 'static,
    value: impl IntoReactive<SharedString> + 'static,
    on_press: impl Fn() + 'static,
) -> ButtonElement {
    pressable()
        .on_press(on_press)
        .flex_row()
        .items_center()
        .gap(px(space::M))
        .min_h(px(hit() + 12.0))
        .px(px(space::L))
        .py(px(space::S))
        .active(move |s| s.bg(move || theme().muted))
        .hover(move |s| s.bg(move || theme().muted.opacity(0.6)))
        .child(leading)
        .child(body(title).flex_1().number_of_lines(1))
        .child(muted(value).number_of_lines(1).max_w(px(180.0)))
        .child(icon(Icon::ChevronRight, 18.0, move || {
            theme().muted_foreground
        }))
}

/// A thin separator inside a [`group`], inset past the leading glyph.
pub fn separator(inset: f32) -> View {
    view()
        .h(px(1.0))
        .ml(px(inset))
        .bg(move || theme().border.opacity(0.6))
}

/// A row with a switch: the system's on phones, the app's on desktops.
pub fn switch_row(
    leading: impl IntoElement,
    title: &'static str,
    checked: impl Fn() -> bool + 'static,
    on_change: impl Fn(bool) + 'static,
) -> View {
    view()
        .flex_row()
        .items_center()
        .gap(px(space::M))
        .min_h(px(hit() + 12.0))
        .px(px(space::L))
        .py(px(space::S))
        .child(leading)
        .child(body(title).flex_1())
        .child(toggle(title, checked, on_change))
}

/// An on/off control: `UISwitch` on iOS, Material 3's switch on Android,
/// the app's own elsewhere.
pub fn toggle(
    label: &'static str,
    checked: impl Fn() -> bool + 'static,
    on_change: impl Fn(bool) + 'static,
) -> AnyElement {
    if mobile() {
        nsh_core::mobile::switch()
            .checked(checked)
            .on_change(move |on: &bool| on_change(*on))
            .accessibility_label(label)
            .into_element()
    } else {
        nsh_base::switch()
            .checked(checked)
            .on_change(move |on: &bool| on_change(*on))
            .accessibility_label(label)
            .min_h(px(hit()))
            .justify_center()
            .track_class(
                "w-11 rounded-full bg-muted p-0.5 transition-colors checked:bg-primary",
            )
            .thumb_class(
                "size-5 rounded-full bg-background shadow transition-transform checked:translate-x-5",
            )
            .into_element()
    }
}

// ---------------------------------------------------------------- status

/// The tone of a status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Accent,
    Success,
    Warning,
    Danger,
}

pub fn tone_color(tone: Tone) -> AdaptiveColor {
    match tone {
        Tone::Neutral => theme().muted_foreground,
        Tone::Accent => theme().primary,
        Tone::Success => success(),
        Tone::Warning => warning(),
        Tone::Danger => theme().destructive,
    }
}

/// A short status label on a tinted capsule.
pub fn pill(
    label: impl IntoReactive<SharedString> + 'static,
    tone: impl Fn() -> Tone + Copy + 'static,
) -> View {
    view()
        .flex_row()
        .self_start()
        .px(px(10.0))
        .py(px(3.0))
        .rounded(px(999.0))
        .bg(move || tone_color(tone()).opacity(0.14))
        .child(
            text(label)
                .text_size(px(size::CAPTION))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || tone_color(tone())),
        )
}

/// A small status dot.
pub fn dot(tone: impl Fn() -> Tone + 'static) -> View {
    view()
        .size(px(8.0))
        .rounded(px(4.0))
        .bg(move || tone_color(tone()))
}

/// This device's chosen avatar.
pub fn my_avatar() -> Option<String> {
    crate::state::model().with(|m| m.settings.avatar.clone())
}

/// An avatar's face: the chosen emoji on its color, or else the name's
/// initial on the name's color.
pub fn face(name: &str, chosen: Option<&str>, diameter: f32) -> View {
    let chosen = chosen.and_then(dexdrop_core::avatar::Avatar::parse);
    let colors = chosen
        .as_ref()
        .map_or_else(|| avatar_colors(name), |a| AVATAR[a.color as usize]);
    let inner = match chosen {
        Some(a) => text(a.emoji).text_size(px(diameter * 0.5)),
        None => text(
            name.chars()
                .find(|c| c.is_alphanumeric())
                .unwrap_or('?')
                .to_uppercase()
                .to_string(),
        )
        .text_size(px(diameter * 0.4))
        .font_weight(FontWeight::BOLD)
        .text_color(white()),
    };
    gradient(colors)
        .size(px(diameter))
        .rounded(px(diameter / 2.0))
        .items_center()
        .justify_center()
        .child(inner)
}

/// A device's avatar: its chosen emoji or its initial on a gradient circle,
/// with its kind (phone, laptop, desktop) in a small badge.
pub fn avatar(
    name: impl Fn() -> String + Copy + 'static,
    platform: impl Fn() -> String + Copy + 'static,
    chosen: impl Fn() -> Option<String> + Copy + 'static,
    diameter: f32,
) -> View {
    let badge = (diameter * 0.38).max(16.0);
    view()
        .size(px(diameter))
        .child(dynamic(move || {
            face(&name(), chosen().as_deref(), diameter)
        }))
        .child(
            view()
                .absolute()
                .right(px(-2.0))
                .bottom(px(-2.0))
                .size(px(badge + 4.0))
                .items_center()
                .justify_center()
                .child(dynamic(move || os_badge(&platform(), badge))),
        )
}

/// How a transfer's data goes, in words and an icon (PRD 9.5: a relay is
/// always shown).
pub fn route_label(route: dexdrop_core::api::Route) -> (&'static str, Icon) {
    use dexdrop_core::api::Route;
    match route {
        Route::Nearby => ("On this network", Icon::Wifi),
        Route::Direct => ("Over the internet", Icon::Globe),
        Route::Relayed => ("Over the internet, through a relay", Icon::Globe),
    }
}

/// An avatar with a globe: a device reached over the internet.
pub fn far_avatar(
    name: impl Fn() -> String + Copy + 'static,
    platform: impl Fn() -> String + Copy + 'static,
    chosen: impl Fn() -> Option<String> + Copy + 'static,
    diameter: f32,
    far: impl Fn() -> bool + 'static,
) -> View {
    let badge = (diameter * 0.38).max(16.0);
    view()
        .size(px(diameter))
        .child(avatar(name, platform, chosen, diameter))
        .child(when(far, move || {
            view()
                .absolute()
                .left(px(-2.0))
                .bottom(px(-2.0))
                .size(px(badge))
                .items_center()
                .justify_center()
                .child(icon(Icon::Globe, badge, move || theme().foreground))
        }))
}

/// A theme color as the current appearance shows it (for drawing).
fn pick(color: AdaptiveColor) -> Hsla {
    if dark() { color.dark } else { color.light }
}

/// A circular progress ring of `side` points, the fraction in `value`.
pub fn progress_ring(
    value: impl Fn() -> f64 + Copy + 'static,
    tone: impl Fn() -> Tone + Copy + 'static,
    side: f32,
) -> View {
    let stroke = (side * 0.075).max(6.0);
    let r = (side - stroke) / 2.0;
    view()
        .size(px(side))
        .role(AccessibilityRole::Progressbar)
        .label(move || format!("{}%", (value() * 100.0).round()))
        .child(
            canvas(move || {
                let fraction = value().clamp(0.0, 1.0) as f32;
                let start = -std::f32::consts::FRAC_PI_2;
                let mut painter = Painter::new()
                    .stroke_color(pick(theme().muted))
                    .line_width(stroke)
                    .begin_path()
                    .arc(side / 2.0, side / 2.0, r, 0.0, std::f32::consts::TAU)
                    .stroke();
                if fraction > 0.0 {
                    painter = painter
                        .stroke_color(pick(tone_color(tone())))
                        .line_cap(nsh_core::canvas::LineCap::Round)
                        .begin_path()
                        .arc(
                            side / 2.0,
                            side / 2.0,
                            r,
                            start,
                            start + fraction * std::f32::consts::TAU,
                        )
                        .stroke();
                }
                painter
            })
            .size_full(),
        )
}

/// Rings that pulse out from the center, as a radar looking for devices.
pub fn radar(_colors: (u32, u32), side: f32, center: impl IntoElement) -> View {
    let ring = move |delay: f32| {
        view()
            .absolute()
            .size(px(side / 2.0))
            .rounded(px(side / 4.0))
            .border_2()
            .border_color(move || theme().foreground.opacity(0.18))
            .animation(
                KeyframeAnimation::new(Keyframes::Ping)
                    .duration(2400.0)
                    .delay(delay),
            )
    };
    view()
        .size(px(side))
        .items_center()
        .justify_center()
        // The pings say "looking"; with reduced motion the still circles do.
        .child(when(
            move || !reduce_motion(),
            move || {
                view()
                    .absolute()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .child(ring(0.0))
                    .child(ring(800.0))
                    .child(ring(1600.0))
            },
        ))
        .child(center)
}

/// A small turning indicator: work in progress without a known end
/// (pulsing instead with reduced motion).
pub fn spinner(side: f32) -> View {
    let animation = if reduce_motion() {
        KeyframeAnimation::new(Keyframes::Pulse).duration(1200.0)
    } else {
        KeyframeAnimation::new(Keyframes::Spin).duration(900.0)
    };
    view()
        .size(px(side))
        .items_center()
        .justify_center()
        .animation(animation)
        .child(icon(Icon::Refresh, side, move || theme().primary))
}

/// An illustration and one line for an empty state.
pub fn empty(i: Icon, line: impl IntoReactive<SharedString> + 'static) -> View {
    view()
        .items_center()
        .gap(px(space::M))
        .py(px(space::XXL))
        .child(soft_glyph(i, || theme().muted_foreground, 64.0))
        .child(muted(line).text_center().max_w(px(280.0)))
}

/// A QR code drawn black on white, whatever the appearance.
pub fn qr(width: usize, modules: Vec<bool>, side: f32) -> impl IntoElement {
    let quiet = 2.0;
    let total = width as f32 + quiet * 2.0;
    let mut path = String::with_capacity(modules.len() * 8);
    for (i, dark) in modules.iter().enumerate() {
        if *dark {
            let x = (i % width) as f32 + quiet;
            let y = (i / width) as f32 + quiet;
            path.push_str(&format!("M{x} {y}h1v1h-1z"));
        }
    }
    let scale = side / total;
    view()
        .size(px(side))
        .bg(white())
        .rounded(px(16.0))
        .label("QR code invitation")
        .child(
            canvas(move || {
                Painter::new()
                    .scale(scale, scale)
                    .fill_color(black())
                    .fill_svg(&path)
            })
            .size_full(),
        )
}

/// Page content width on desktops.
pub fn content_width() -> f32 {
    if desktop_platform() { 720.0 } else { 640.0 }
}

/// A choice of one among a few, as a segmented control: the system's own
/// on iOS (`UISegmentedControl`); elsewhere, where the platform has none,
/// NSH's `ToggleGroup` of `Toggle`s (a toolbar to screen readers, one
/// keyboard stop with arrow keys), drawn as joined segments.
pub fn segmented<T: Copy + PartialEq + 'static>(
    items: &'static [(&'static str, T)],
    value: Signal<T>,
) -> AnyElement {
    let index = move || {
        items
            .iter()
            .position(|(_, v)| *v == value.get())
            .unwrap_or(0) as u32
    };
    if os().name() == "ios" {
        return nsh_core::mobile::segmented_control()
            .items(
                items
                    .iter()
                    .map(|(l, _)| nsh_core::mobile::SegmentedControlItem::new(*l))
                    .collect::<Vec<_>>(),
            )
            .selected_index(index)
            .on_change(move |i: &u32| {
                if let Some((_, v)) = items.get(*i as usize) {
                    value.set(*v);
                }
            })
            .self_stretch()
            .into_element();
    }
    nsh_base::toggle_group()
        .child(
            view()
                .flex_row()
                .p(px(3.0))
                .rounded(px(12.0))
                .bg(move || theme().muted)
                .children(items.iter().map(|(label, v)| {
                    let v = *v;
                    let on = move || value.get() == v;
                    nsh_base::toggle()
                        .pressed(on)
                        .on_change(move |_: &bool| value.set(v))
                        .flex_1()
                        .min_h(px(hit() - 6.0))
                        .px(px(space::M))
                        .items_center()
                        .justify_center()
                        .rounded(px(9.0))
                        .bg(move || {
                            if on() {
                                theme().card
                            } else {
                                theme().card.opacity(0.0)
                            }
                        })
                        .child(
                            text(*label)
                                .text_size(px(size::SMALL))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || {
                                    if on() {
                                        theme().foreground
                                    } else {
                                        theme().muted_foreground
                                    }
                                }),
                        )
                })),
        )
        .self_stretch()
        .into_element()
}
