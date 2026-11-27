//! The design tokens (`app/DESIGN.md`): spacing, type, radius and motion
//! scales, and the semantic colors `theme.json` defines beyond shadcn's.
//! Presentation code reads colors only through these and `theme()`.

use nsh_base::theme;
use nsh_core::prelude::*;

/// Spacing scale, in points.
pub mod space {
    pub const XS: f32 = 4.0;
    pub const S: f32 = 8.0;
    pub const M: f32 = 12.0;
    pub const L: f32 = 16.0;
    pub const XL: f32 = 24.0;
    pub const XXL: f32 = 32.0;
}

/// Type scale, in points. Nothing reads smaller than `CAPTION`.
pub mod size {
    /// The big number of a transfer's progress.
    pub const FIGURE: f32 = 44.0;
    pub const DISPLAY: f32 = 34.0;
    /// A page's own large heading (a code to type, a device's name).
    pub const HEADLINE: f32 = 28.0;
    pub const TITLE: f32 = 22.0;
    /// A section's title over a group.
    pub const SECTION: f32 = 18.0;
    pub const HEADING: f32 = 17.0;
    pub const BODY: f32 = 16.0;
    pub const SMALL: f32 = 14.0;
    pub const CAPTION: f32 = 12.0;
}

/// Radius scale, from the theme (`cssVars.theme`, `radius-*`), in points.
// A scale: a step no screen uses yet stays part of it.
#[allow(dead_code)]
pub mod radius {
    use nsh_base::theme;

    fn of(name: &str, fallback: f32) -> f32 {
        theme().number(name).unwrap_or(fallback)
    }

    /// Small marks: a progress track, a tick.
    pub fn xs() -> f32 {
        of("radius-xs", 4.0)
    }
    /// Thumbnails in lists, chips' insides.
    pub fn sm() -> f32 {
        of("radius-sm", 8.0)
    }
    /// Controls: fields, small tiles, file badges.
    pub fn md() -> f32 {
        of("radius-md", 12.0)
    }
    /// Groups and cards.
    pub fn lg() -> f32 {
        of("radius-lg", 16.0)
    }
    /// Large tiles and sheets.
    pub fn xl() -> f32 {
        of("radius-xl", 20.0)
    }
    /// Hero surfaces: the QR code, a big preview.
    pub fn xxl() -> f32 {
        of("radius-2xl", 24.0)
    }
    /// Capsules and circles.
    pub const FULL: f32 = 9999.0;
}

/// Motion tokens (Design Guides, "Define motion tokens once"): durations
/// in milliseconds, distances in points, press scales. Every transition,
/// motion and animation of the app takes its numbers from here; with
/// reduced motion NSH makes transitions instant and skips motions.
#[allow(dead_code)]
pub mod motion {
    /// A hover wash, a color change.
    pub const INSTANT: f32 = 120.0;
    /// A press, a control's state.
    pub const FAST: f32 = 180.0;
    /// Moving into place: a selection sliding, a glyph swapping, a sheet
    /// leaving.
    pub const BASE: f32 = 240.0;
    /// Reveals: a dialog or sheet opening, a result popping in.
    pub const SLOW: f32 = 360.0;
    /// Unhurried entrances.
    pub const SLOWER: f32 = 500.0;
    /// Looping indicators: a spinner's turn, a skeleton's pulse, the
    /// radar's ping.
    pub const SPIN: f32 = 800.0;
    pub const PULSE: f32 = 1400.0;
    pub const PING: f32 = 2400.0;
    /// The soft settle of presses: `cubic-bezier(0.22, 1, 0.36, 1)`.
    pub const SETTLE: [f32; 4] = [0.22, 1.0, 0.36, 1.0];
    /// Distances: a label rising, a popup sliding, items entering.
    pub const NUDGE: f32 = 4.0;
    pub const SLIDE: f32 = 8.0;
    pub const ENTER: f32 = 16.0;
    /// Press scales: a button, an icon button or tile.
    pub const PRESS: f32 = 0.97;
    pub const PRESS_ICON: f32 = 0.95;
}

// ---------------------------------------------------------------- color

/// A color the theme defines by `name` (`theme.json`), else `fallback`.
pub fn token(name: &str, fallback: AdaptiveColor) -> AdaptiveColor {
    theme().color(name).unwrap_or(fallback)
}

pub fn success() -> AdaptiveColor {
    token("success", theme().primary)
}

pub fn warning() -> AdaptiveColor {
    token("warning", theme().destructive)
}

/// Text and glyphs on a colored fill (an avatar, a source's badge, a
/// count on the tab bar).
pub fn on_color() -> AdaptiveColor {
    token("on-color", theme().primary_foreground)
}

/// The dimming behind a dialog or sheet.
pub fn scrim() -> AdaptiveColor {
    token("scrim", theme().foreground.opacity(0.4))
}

/// Over pictures and video: the backdrop, a chip's fill (dark, or light on
/// a selected picture), its text and its quieter text.
pub fn media_backdrop() -> AdaptiveColor {
    token("media-backdrop", theme().foreground)
}
pub fn media_chip() -> AdaptiveColor {
    token("media-chip", theme().foreground.opacity(0.6))
}
pub fn media_chip_light() -> AdaptiveColor {
    token("media-chip-light", theme().background.opacity(0.2))
}
pub fn on_media() -> AdaptiveColor {
    token("on-media", theme().background)
}
pub fn on_media_muted() -> AdaptiveColor {
    token("on-media-muted", theme().background.opacity(0.67))
}

/// The main action's fill, the app icon's blue from light to deep.
pub fn primary_from() -> AdaptiveColor {
    token("primary-from", theme().primary)
}
pub fn primary_to() -> AdaptiveColor {
    token("primary-to", theme().primary)
}

/// Where something to send comes from: each source keeps its hue (Apple's
/// system colors, `source-*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Photos,
    Videos,
    Files,
    Apps,
    Folder,
}

impl Source {
    pub fn color(self) -> AdaptiveColor {
        let name = match self {
            Source::Photos => "source-photos",
            Source::Videos => "source-videos",
            Source::Files => "source-files",
            Source::Apps => "source-apps",
            Source::Folder => "source-folder",
        };
        token(name, theme().primary)
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
