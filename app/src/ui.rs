//! DexDrop's design system: a small set of components built on the theme's
//! tokens, icons drawn as tinted SVGs, and the platform's own controls
//! where they exist (switches, segmented controls, the tab bar, headers).
//! Pages compose these instead of styling raw views.

use nsh_base::{ButtonElement, button, theme};
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

/// The two brand gradients: sending (blue to indigo), receiving (green to
/// teal). Each is `(from, to)`.
pub const SEND: (u32, u32) = (0x3b82f6, 0x6366f1);
pub const RECEIVE: (u32, u32) = (0x10b981, 0x14b8a6);

/// A gradient-filled view.
pub fn gradient(colors: (u32, u32)) -> View {
    view()
        .bg(rgb(colors.0))
        .bg_linear(deg(135.0))
        .gradient_from(rgb(colors.0))
        .gradient_to(rgb(colors.1))
}

/// Colors for avatars, picked by name so a device keeps its color.
const AVATAR: [(u32, u32); 6] = [
    (0x6366f1, 0x8b5cf6),
    (0x0ea5e9, 0x3b82f6),
    (0x10b981, 0x14b8a6),
    (0xf59e0b, 0xf97316),
    (0xec4899, 0xf43f5e),
    (0x8b5cf6, 0xd946ef),
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
    Phone,
    Laptop,
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
    Pencil,
    Trash,
    Info,
    Alert,
    Devices,
    Share,
    UserCheck,
    Lock,
    Cellular,
}

impl Icon {
    /// The icon's SVG, built into the app.
    fn svg(self) -> &'static str {
        match self {
            Icon::Send => include_str!("../assets/icons/send.svg"),
            Icon::Receive => include_str!("../assets/icons/receive.svg"),
            Icon::Paperplane => include_str!("../assets/icons/paperplane.svg"),
            Icon::Clock => include_str!("../assets/icons/clock.svg"),
            Icon::Gear => include_str!("../assets/icons/gear.svg"),
            Icon::Photo => include_str!("../assets/icons/photo.svg"),
            Icon::Video => include_str!("../assets/icons/video.svg"),
            Icon::File => include_str!("../assets/icons/file.svg"),
            Icon::Folder => include_str!("../assets/icons/folder.svg"),
            Icon::Music => include_str!("../assets/icons/music.svg"),
            Icon::Archive => include_str!("../assets/icons/archive.svg"),
            Icon::Phone => include_str!("../assets/icons/phone.svg"),
            Icon::Laptop => include_str!("../assets/icons/laptop.svg"),
            Icon::Monitor => include_str!("../assets/icons/monitor.svg"),
            Icon::Pause => include_str!("../assets/icons/pause.svg"),
            Icon::Play => include_str!("../assets/icons/play.svg"),
            Icon::X => include_str!("../assets/icons/x.svg"),
            Icon::Check => include_str!("../assets/icons/check.svg"),
            Icon::ChevronRight => include_str!("../assets/icons/chevron-right.svg"),
            Icon::Plus => include_str!("../assets/icons/plus.svg"),
            Icon::Refresh => include_str!("../assets/icons/refresh.svg"),
            Icon::Qr => include_str!("../assets/icons/qr.svg"),
            Icon::Keypad => include_str!("../assets/icons/keypad.svg"),
            Icon::Link => include_str!("../assets/icons/link.svg"),
            Icon::Copy => include_str!("../assets/icons/copy.svg"),
            Icon::Shield => include_str!("../assets/icons/shield.svg"),
            Icon::Eye => include_str!("../assets/icons/eye.svg"),
            Icon::Wifi => include_str!("../assets/icons/wifi.svg"),
            Icon::Pencil => include_str!("../assets/icons/pencil.svg"),
            Icon::Trash => include_str!("../assets/icons/trash.svg"),
            Icon::Info => include_str!("../assets/icons/info.svg"),
            Icon::Alert => include_str!("../assets/icons/alert.svg"),
            Icon::Devices => include_str!("../assets/icons/devices.svg"),
            Icon::Share => include_str!("../assets/icons/share.svg"),
            Icon::UserCheck => include_str!("../assets/icons/user-check.svg"),
            Icon::Lock => include_str!("../assets/icons/lock.svg"),
            Icon::Cellular => include_str!("../assets/icons/cellular.svg"),
        }
    }

    /// The icon as a `data:` URI: the same on every platform and runner,
    /// with no asset files to ship.
    fn uri(self) -> String {
        let mut uri = String::from("data:image/svg+xml,");
        for b in self.svg().trim().bytes() {
            if b.is_ascii_alphanumeric() || b" -._~/:=,;()'".contains(&b) {
                uri.push(b as char);
            } else {
                uri.push_str(&format!("%{b:02X}"));
            }
        }
        uri
    }

    /// The icon for a file by its name.
    pub fn for_file(name: &str) -> Icon {
        // A transfer's label ("Holiday.jpg and 2 more") goes by its first file.
        let name = name.split_once(" and ").map_or(name, |(first, _)| first);
        let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        match ext.as_str() {
            "jpg" | "jpeg" | "png" | "heic" | "heif" | "gif" | "webp" | "bmp" | "tif" | "tiff"
            | "raw" | "dng" => Icon::Photo,
            "mp4" | "mov" | "m4v" | "mkv" | "avi" | "webm" | "3gp" => Icon::Video,
            "mp3" | "m4a" | "aac" | "wav" | "flac" | "ogg" | "opus" => Icon::Music,
            "zip" | "rar" | "7z" | "tar" | "gz" | "tgz" | "xz" => Icon::Archive,
            _ => Icon::File,
        }
    }

    /// The icon for a device's platform.
    pub fn for_platform(platform: &str) -> Icon {
        match platform {
            "android" | "ios" => Icon::Phone,
            "macos" => Icon::Laptop,
            _ => Icon::Monitor,
        }
    }
}

/// An icon at `size` points in `color`.
pub fn icon(
    icon: Icon,
    size: f32,
    color: impl IntoReactive<AdaptiveColor>,
) -> nsh_core::element::Image {
    image(icon.uri()).size(px(size)).tint_color(color)
}

/// An icon on a tinted rounded square, as the system's settings show them.
pub fn glyph(i: Icon, colors: (u32, u32), side: f32) -> View {
    gradient(colors)
        .size(px(side))
        .rounded(px(side * 0.28))
        .items_center()
        .justify_center()
        .child(icon(i, side * 0.56, white()))
}

/// The color of a kind of file: photos pink, videos purple, music
/// orange, archives slate, documents blue.
pub fn kind_color(i: Icon) -> u32 {
    match i {
        Icon::Photo => 0xec4899,
        Icon::Video => 0x8b5cf6,
        Icon::Music => 0xf59e0b,
        Icon::Archive => 0x64748b,
        Icon::Folder => 0xf59e0b,
        _ => 0x3b82f6,
    }
}

/// A file's kind on a soft circle of its color.
pub fn kind_glyph(i: Icon, side: f32) -> View {
    let c = kind_color(i);
    view()
        .size(px(side))
        .rounded(px(side / 2.0))
        .items_center()
        .justify_center()
        .bg(rgba((c << 8) | 0x24))
        .child(icon(i, side * 0.5, rgb(c)))
}

/// An icon on a soft circle of its own color.
pub fn soft_glyph(i: Icon, color: impl Fn() -> AdaptiveColor + Copy + 'static, side: f32) -> View {
    view()
        .size(px(side))
        .rounded(px(side / 2.0))
        .items_center()
        .justify_center()
        .bg(move || color().opacity(0.14))
        .child(icon(i, side * 0.5, color))
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
    let mut b = button()
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
    button()
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

/// One of the two big actions of the home screen: a gradient circle with
/// an icon, its label below.
pub fn hero_action(
    i: Icon,
    label: &'static str,
    colors: (u32, u32),
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    let side = if mobile() { 112.0 } else { 120.0 };
    button()
        .accessibility_label(label)
        .on_press(on_press)
        .items_center()
        .gap(px(space::M))
        .active(|s| s.scale(0.95))
        .child(
            gradient(colors)
                .size(px(side))
                .rounded(px(side / 2.0))
                .items_center()
                .justify_center()
                .shadow_lg()
                .shadow_color(rgba((colors.0 << 8) | 0x66))
                .border(px(4.0))
                .border_color(rgba((colors.1 << 8) | 0x33))
                .child(icon(i, side * 0.4, white())),
        )
        .child(
            text(label)
                .text_size(px(size::HEADING + 1.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(colors.0)),
        )
}

// ---------------------------------------------------------------- surfaces

/// A rounded group of rows on the page's grouped background.
pub fn group() -> View {
    view()
        .rounded(px(16.0))
        .bg(move || theme().card)
        .overflow_hidden()
}

/// A tile: a rounded card.
pub fn tile() -> View {
    view()
        .p(px(space::L))
        .gap(px(space::M))
        .rounded(px(18.0))
        .bg(move || theme().card)
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
    button()
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

/// A device's avatar: its initial on a gradient circle of its own color,
/// with its kind (phone, laptop, desktop) in a small badge.
pub fn avatar(
    name: impl Fn() -> String + Copy + 'static,
    platform: impl Fn() -> String + Copy + 'static,
    diameter: f32,
) -> View {
    let badge = (diameter * 0.38).max(16.0);
    view()
        .size(px(diameter))
        .child(dynamic(move || {
            let n = name();
            gradient(avatar_colors(&n))
                .size(px(diameter))
                .rounded(px(diameter / 2.0))
                .items_center()
                .justify_center()
                .child(
                    text(
                        n.chars()
                            .find(|c| c.is_alphanumeric())
                            .unwrap_or('?')
                            .to_uppercase()
                            .to_string(),
                    )
                    .text_size(px(diameter * 0.4))
                    .font_weight(FontWeight::BOLD)
                    .text_color(white()),
                )
        }))
        .child(
            view()
                .absolute()
                .right(px(-2.0))
                .bottom(px(-2.0))
                .size(px(badge))
                .rounded(px(badge / 2.0))
                .items_center()
                .justify_center()
                .bg(move || theme().card)
                .shadow_sm()
                .child(dynamic(move || {
                    icon(Icon::for_platform(&platform()), badge * 0.6, move || {
                        theme().muted_foreground
                    })
                })),
        )
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
pub fn radar(colors: (u32, u32), side: f32, center: impl IntoElement) -> View {
    let ring = move |delay: f32| {
        view()
            .absolute()
            .size(px(side / 2.0))
            .rounded(px(side / 4.0))
            .border_2()
            .border_color(rgba((colors.0 << 8) | 0x55))
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
        .child(
            view()
                .absolute()
                .size(px(side * 0.72))
                .rounded(px(side * 0.36))
                .bg(rgba((colors.0 << 8) | 0x10)),
        )
        .child(
            view()
                .absolute()
                .size(px(side * 0.46))
                .rounded(px(side * 0.23))
                .bg(rgba((colors.0 << 8) | 0x18)),
        )
        .child(ring(0.0))
        .child(ring(800.0))
        .child(ring(1600.0))
        .child(center)
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
