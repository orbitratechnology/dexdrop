//! The app's icons: one family of one-color SVGs (`assets/icons/`), and
//! the marks of file kinds and operating systems.

use nsh_base::theme;
use nsh_core::prelude::*;

use super::*;

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
    Camera,
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
    Share,
    UserCheck,
    Lock,
    ArrowRight,
    Sliders,
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
    /// Four rounded squares: apps.
    Apps,
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
            Icon::Camera => asset!("icons/camera.svg"),
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
            Icon::Share => asset!("icons/share.svg"),
            Icon::UserCheck => asset!("icons/user-check.svg"),
            Icon::Lock => asset!("icons/lock.svg"),
            Icon::ArrowRight => asset!("icons/arrow-right.svg"),
            Icon::Sliders => asset!("icons/sliders.svg"),
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
            Icon::Apps => asset!("icons/apps.svg"),
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
/// mark of a row or a heading.
pub fn glyph(i: Icon, side: f32) -> View {
    view()
        .size(px(side))
        .items_center()
        .justify_center()
        .child(icon(i, side, move || theme().foreground))
}

/// The theme's token for the color of a kind of file (`kind-*`).
fn kind_token(i: Icon) -> &'static str {
    match i {
        Icon::Folder => "kind-folder",
        Icon::Photo => "kind-photo",
        Icon::Video => "kind-video",
        Icon::Music => "kind-music",
        Icon::Archive => "kind-archive",
        Icon::Pdf => "kind-pdf",
        Icon::Doc => "kind-doc",
        Icon::Sheet => "kind-sheet",
        Icon::Slides => "kind-slides",
        Icon::Code => "kind-code",
        Icon::Markdown => "kind-markdown",
        Icon::Font => "kind-font",
        Icon::Book => "kind-book",
        Icon::Disk => "kind-disk",
        Icon::Vector => "kind-vector",
        Icon::Database => "kind-database",
        Icon::Calendar => "kind-calendar",
        Icon::Person => "kind-contact",
        Icon::Android => "kind-android",
        Icon::Apple => "kind-apple",
        Icon::Windows => "kind-windows",
        Icon::Linux => "kind-linux",
        Icon::Package => "kind-package",
        _ => "kind-file",
    }
}

/// A theme color by its token, the primary color when the theme lacks it.
fn color_of(token: &'static str) -> impl Fn() -> AdaptiveColor + Copy {
    move || super::token(token, theme().primary)
}

/// A file's kind by its extension: its icon, its color's token and the short tag
/// shown on it ("PDF", "XLSX"; none for files without a known one).
pub fn file_kind(name: &str) -> (Icon, &'static str, Option<String>) {
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
        "gif" => "kind-gif",
        "csv" | "tsv" => "kind-table",
        "html" | "htm" => "kind-web",
        "svg" => "kind-vector",
        "txt" | "log" => "kind-text",
        "doc" | "docx" | "odt" | "rtf" | "pages" => "kind-doc",
        _ => kind_token(icon),
    };
    let tag = (!ext.is_empty() && ext.len() <= 5 && ext.chars().all(|c| c.is_ascii_alphanumeric()))
        .then(|| ext.to_ascii_uppercase());
    (icon, color, tag)
}

/// A file's picture when there is no preview: its kind's icon in its
/// kind's color, filling `side` (no tile behind it).
pub fn file_badge(name: &str, side: f32) -> View {
    let (i, c, _tag) = file_kind(name);
    view()
        .size(px(side))
        .items_center()
        .justify_center()
        .child(icon(i, side, color_of(c)))
}

/// A device's operating system: its mark and the mark's color token
/// (`None`: the page's foreground, for black marks such as Apple's).
fn os_mark(platform: &str) -> (Icon, Option<&'static str>) {
    match platform {
        "ios" | "macos" => (Icon::Apple, None),
        "windows" => (Icon::Windows, Some("os-windows")),
        "android" => (Icon::Android, Some("os-android")),
        "linux" => (Icon::Linux, Some("os-linux")),
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
            Some(c) => icon(i, side, color_of(c)),
            None => icon(i, side, move || theme().foreground),
        })
}

/// A file's kind: its icon in its color, filling `side`.
pub fn kind_glyph(i: Icon, side: f32) -> View {
    view()
        .size(px(side))
        .items_center()
        .justify_center()
        .child(icon(i, side, color_of(kind_token(i))))
}

/// An icon in a status's color, filling `side` (no circle behind it).
pub fn soft_glyph(i: Icon, tone: Tone, side: f32) -> View {
    view()
        .size(px(side))
        .items_center()
        .justify_center()
        .child(icon(i, side, move || tone_color(tone)))
}

/// A source's mark, as iOS's Settings and share sheets draw them: its
/// symbol on a rounded square of its color.
pub fn source_badge(i: Icon, source: Source, side: f32) -> View {
    view()
        .size(px(side))
        .rounded(px(side * 0.26))
        .items_center()
        .justify_center()
        .bg(move || source.color())
        .child(icon(i, side * 0.56, on_color))
}
