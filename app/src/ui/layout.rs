//! Platform fit: hit targets, the tab bars, content widths.

use nsh_core::prelude::*;

use crate::state::desktop_platform;

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

/// The height of the tab bar a narrow window draws (desktops, the web).
pub const DRAWN_TAB_BAR: f32 = 56.0;

/// The height of the tab bar the app draws on Android, above the system's
/// navigation (gesture handle or buttons).
pub const ANDROID_TAB_BAR: f32 = 68.0;

/// Whether the app draws its own tab bar: everywhere but iOS, whose
/// system bar takes the app's colors. Android's Material bar takes the
/// phone's wallpaper colors (Material You); NSH 0.7.0 sets only the chosen
/// tab's color.
pub fn drawn_tabs() -> bool {
    os() != nsh_core::platform::Os::Ios
}

/// The height of the drawn tab bar, the system's navigation below it
/// included.
pub fn drawn_tab_bar_height() -> f32 {
    if os() == nsh_core::platform::Os::Android {
        ANDROID_TAB_BAR + nsh_core::element::safe_area().bottom.0
    } else {
        DRAWN_TAB_BAR
    }
}

/// How far above the window's bottom edge a bar floating over a page sits:
/// clear of the tab bar (the system's on iOS, about 96 pt with the home
/// indicator; the drawn one elsewhere), or at the edge when a sidebar
/// holds the tabs.
pub fn above_tab_bar() -> f32 {
    if crate::state::wide() {
        0.0
    } else if !drawn_tabs() {
        96.0
    } else {
        drawn_tab_bar_height()
    }
}

/// Page content width on desktops.
pub fn content_width() -> f32 {
    if desktop_platform() { 720.0 } else { 640.0 }
}
