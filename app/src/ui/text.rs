//! The type styles.

use nsh_base::theme;
use nsh_core::{element::AccessibilityRole, prelude::*};

use super::*;

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
        .text_size(px(size::SECTION))
        .font_weight(FontWeight::BOLD)
        .text_color(move || theme().foreground)
        .role(AccessibilityRole::Header)
}
