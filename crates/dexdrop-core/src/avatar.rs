//! A device's avatar: an emoji on one of a few colors, chosen by its user
//! and shown to other devices next to its name.
//!
//! On the wire (the handshake, the mDNS TXT record) and on disk it is a short
//! string, `"<color>:<emoji>"` (`"3:🦊"`). Anything else from a peer is
//! ignored and the device shows its initial instead.

use std::fmt;

/// How many background colors there are; the app maps each index to a
/// gradient.
pub const COLORS: u8 = 8;
/// The longest emoji accepted, in bytes (a family or flag sequence fits).
pub const MAX_EMOJI_BYTES: usize = 32;

/// An emoji on a background color.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Avatar {
    pub emoji: String,
    /// Below [`COLORS`].
    pub color: u8,
}

impl Avatar {
    /// An avatar, if `emoji` is short and printable and `color` in range.
    pub fn new(emoji: &str, color: u8) -> Option<Self> {
        let emoji = emoji.trim();
        let ok = !emoji.is_empty()
            && emoji.len() <= MAX_EMOJI_BYTES
            && color < COLORS
            && !emoji
                .chars()
                .any(|c| c.is_control() || c.is_whitespace() || c.is_ascii_alphanumeric());
        ok.then(|| Self {
            emoji: emoji.to_owned(),
            color,
        })
    }

    /// Reads the wire form; `None` for anything malformed.
    pub fn parse(s: &str) -> Option<Self> {
        let (color, emoji) = s.split_once(':')?;
        Self::new(emoji, color.parse().ok()?)
    }

    /// The wire form of an optional avatar, cleaned: what a peer sent, kept
    /// only if it is valid.
    pub fn clean(s: Option<&str>) -> Option<String> {
        s.and_then(Self::parse).map(|a| a.to_string())
    }
}

impl fmt::Display for Avatar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.color, self.emoji)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_rejects_junk() {
        let a = Avatar::new("🦊", 3).unwrap();
        assert_eq!(a.to_string(), "3:🦊");
        assert_eq!(Avatar::parse("3:🦊"), Some(a));
        assert_eq!(Avatar::parse("1:👨‍👩‍👧").unwrap().emoji, "👨‍👩‍👧");
        for junk in ["", "🦊", "9:🦊", "x:🦊", "1:", "1:ab", "1:🦊 🦊", "1:\u{7}"] {
            assert_eq!(Avatar::parse(junk), None, "{junk:?}");
        }
        assert_eq!(Avatar::parse(&format!("1:{}", "🦊".repeat(20))), None);
        assert_eq!(Avatar::clean(Some("2:🌵")), Some("2:🌵".into()));
        assert_eq!(Avatar::clean(Some("nope")), None);
    }
}
