//! DexDrop's design system: a small set of components built on the theme's
//! tokens, icons drawn as tinted SVGs, and the platform's own controls
//! where they exist (switches, segmented controls, the tab bar, headers).
//! Pages compose these instead of styling raw views.

mod controls;
mod dialogs;
mod icons;
mod indicators;
mod layout;
mod people;
mod surfaces;
mod text;
mod tokens;

pub use controls::*;
pub use dialogs::*;
pub use icons::*;
pub use indicators::*;
pub use layout::*;
pub use people::*;
pub use surfaces::*;
pub use text::*;
pub use tokens::*;
