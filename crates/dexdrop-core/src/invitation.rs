//! QR/link invitations and short pairing codes (FR-020 – FR-023).
//!
//! A receiver can show an [`Invitation`]: its peer ID, the addresses it
//! listens on, a random single-use token and an expiry. Encoded as text it is
//! a QR code and a copyable link; the sender pins the peer ID from it, so a
//! device on the network cannot impersonate the receiver.
//!
//! A short code (`K7P2-QX9M`) is for typing: the receiver advertises that it
//! has an open code, the sender connects to such devices and presents the code
//! inside the encrypted session. Both sides then compare the 6-digit
//! verification number. Codes expire, are single-use, and five wrong attempts
//! close them all.

use std::net::SocketAddr;

use data_encoding::BASE32_NOPAD;
use serde::{Deserialize, Serialize};

use crate::PeerId;
#[cfg(not(target_arch = "wasm32"))]
use crate::protocol::Pairing;
#[cfg(not(target_arch = "wasm32"))]
use crate::{random_bytes, unix_now};

/// How long a new invitation or code stays valid, in seconds.
pub const DEFAULT_TTL_SECS: u64 = 10 * 60;
/// Wrong code attempts before every open code closes.
pub const MAX_CODE_FAILURES: u32 = 5;
/// The text prefix of encoded invitations (QR alphanumeric-friendly).
pub const PREFIX: &str = "DEXDROP1:";

/// Crockford base32 alphabet: no I, L, O or U.
const CODE_ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
/// Characters in a short code.
pub const CODE_LEN: usize = 8;

/// An invitation a receiver shows as a QR code or link.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invitation {
    pub peer_id: PeerId,
    pub name: String,
    pub addrs: Vec<SocketAddr>,
    pub token: [u8; 16],
    /// Unix seconds.
    pub expires: u64,
}

/// Why an invitation or code was not accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum InviteError {
    #[error("This is not a DexDrop invitation.")]
    Malformed,
    #[error("This invitation has expired. Ask for a new one.")]
    Expired,
    #[error("This invitation is not valid or was already used.")]
    Unknown,
    #[error("Too many wrong codes. Show a new code to try again.")]
    TooManyAttempts,
    #[error("No QR code was found in this picture.")]
    NoQrCode,
}

impl Invitation {
    /// Encodes the invitation as text for a QR code or link.
    pub fn encode(&self) -> String {
        let bytes = postcard::to_stdvec(self).expect("invitations serialize");
        format!("{PREFIX}{}", BASE32_NOPAD.encode(&bytes))
    }

    /// Decodes invitation text, case- and whitespace-insensitively, and
    /// checks that it has not expired.
    pub fn decode(text: &str, now: u64) -> Result<Self, InviteError> {
        let cleaned: String = text
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .to_ascii_uppercase();
        let body = cleaned
            .strip_prefix(PREFIX)
            .or_else(|| cleaned.strip_prefix("DEXDROP://1/"))
            .ok_or(InviteError::Malformed)?;
        if body.len() > 2048 {
            return Err(InviteError::Malformed);
        }
        let bytes = BASE32_NOPAD
            .decode(body.as_bytes())
            .map_err(|_| InviteError::Malformed)?;
        let invitation: Invitation =
            postcard::from_bytes(&bytes).map_err(|_| InviteError::Malformed)?;
        if invitation.addrs.is_empty() || invitation.addrs.len() > 16 {
            return Err(InviteError::Malformed);
        }
        if invitation.expires <= now {
            return Err(InviteError::Expired);
        }
        Ok(invitation)
    }
}

/// Normalizes a typed code: upper case, separators removed, look-alike
/// letters mapped (`O`→`0`, `I`/`L`→`1`).
pub fn normalize_code(input: &str) -> Option<String> {
    let code: String = input
        .chars()
        .filter(|c| !matches!(c, '-' | ' ' | '_'))
        .map(|c| match c.to_ascii_uppercase() {
            'O' => '0',
            'I' | 'L' => '1',
            c => c,
        })
        .collect();
    (code.len() == CODE_LEN && code.bytes().all(|b| CODE_ALPHABET.contains(&b))).then_some(code)
}

/// A code shown with a separator: `K7P2-QX9M`.
pub fn display_code(code: &str) -> String {
    if code.len() == CODE_LEN {
        format!("{}-{}", &code[..4], &code[4..])
    } else {
        code.to_owned()
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn new_code() -> String {
    random_bytes::<CODE_LEN>()
        .iter()
        .map(|b| CODE_ALPHABET[(b & 31) as usize] as char)
        .collect()
}

#[cfg(not(target_arch = "wasm32"))]
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug)]
struct Open {
    token: [u8; 16],
    code: Option<String>,
    expires: u64,
}

/// The receiver's open invitations and codes.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Default)]
pub struct InvitationBook {
    open: Vec<Open>,
    code_failures: u32,
}

#[cfg(not(target_arch = "wasm32"))]
impl InvitationBook {
    /// Opens a QR/link invitation for `ttl` seconds.
    pub fn create(
        &mut self,
        peer_id: PeerId,
        name: &str,
        addrs: Vec<SocketAddr>,
        ttl: u64,
    ) -> Invitation {
        self.prune(unix_now());
        let invitation = Invitation {
            peer_id,
            name: name.to_owned(),
            addrs,
            token: random_bytes::<16>(),
            expires: unix_now() + ttl,
        };
        self.open.push(Open {
            token: invitation.token,
            code: None,
            expires: invitation.expires,
        });
        invitation
    }

    /// Opens a short code for `ttl` seconds; returns it and its expiry.
    pub fn create_code(&mut self, ttl: u64) -> (String, u64) {
        self.prune(unix_now());
        self.code_failures = 0;
        let code = new_code();
        let expires = unix_now() + ttl;
        self.open.push(Open {
            token: random_bytes::<16>(),
            code: Some(code.clone()),
            expires,
        });
        (code, expires)
    }

    /// Whether a short code is open (advertised to nearby devices).
    pub fn has_open_code(&self, now: u64) -> bool {
        self.open
            .iter()
            .any(|o| o.code.is_some() && o.expires > now)
    }

    /// Closes every invitation and code.
    pub fn clear(&mut self) {
        self.open.clear();
    }

    /// Checks a presented token or code and uses it up.
    pub fn redeem(&mut self, pairing: &Pairing, now: u64) -> Result<(), InviteError> {
        self.prune(now);
        match pairing {
            Pairing::Token(token) => {
                let at = self
                    .open
                    .iter()
                    .position(|o| o.code.is_none() && ct_eq(&o.token, token))
                    .ok_or(InviteError::Unknown)?;
                self.open.remove(at);
                Ok(())
            }
            Pairing::Code(typed) => {
                if self.code_failures >= MAX_CODE_FAILURES {
                    return Err(InviteError::TooManyAttempts);
                }
                let typed = normalize_code(typed).unwrap_or_default();
                let found = self.open.iter().position(|o| {
                    o.code
                        .as_deref()
                        .is_some_and(|c| ct_eq(c.as_bytes(), typed.as_bytes()))
                });
                match found {
                    Some(at) => {
                        self.open.remove(at);
                        Ok(())
                    }
                    None => {
                        self.code_failures += 1;
                        if self.code_failures >= MAX_CODE_FAILURES {
                            self.open.retain(|o| o.code.is_none());
                            Err(InviteError::TooManyAttempts)
                        } else {
                            Err(InviteError::Unknown)
                        }
                    }
                }
            }
        }
    }

    fn prune(&mut self, now: u64) {
        self.open.retain(|o| o.expires > now);
    }
}

/// The QR code of `text`: its width and dark modules, row by row.
pub fn qr_matrix(text: &str) -> Option<(usize, Vec<bool>)> {
    let code =
        qrcode::QrCode::with_error_correction_level(text.as_bytes(), qrcode::EcLevel::M).ok()?;
    let width = code.width();
    let modules = code
        .to_colors()
        .into_iter()
        .map(|c| c == qrcode::Color::Dark)
        .collect();
    Some((width, modules))
}

/// Finds a DexDrop invitation in a picture of a QR code: a screenshot or a
/// photo (FR-025). Returns the invitation text, not yet decoded or checked.
pub fn read_qr_image(bytes: &[u8]) -> Result<String, InviteError> {
    let image = image::load_from_memory(bytes).map_err(|_| InviteError::NoQrCode)?;
    // Large photos are scaled down: QR modules stay several pixels wide and
    // detection is much faster.
    let image = if image.width().max(image.height()) > 1600 {
        image.resize(1600, 1600, image::imageops::FilterType::Triangle)
    } else {
        image
    };
    let gray = image.to_luma8();
    let (w, h) = (gray.width() as usize, gray.height() as usize);
    let mut prepared = rqrr::PreparedImage::prepare_from_greyscale(w, h, |x, y| {
        gray.get_pixel(x as u32, y as u32).0[0]
    });
    let mut found_other = false;
    for grid in prepared.detect_grids() {
        if let Ok((_, text)) = grid.decode() {
            if text.trim().to_ascii_uppercase().starts_with(PREFIX) {
                return Ok(text.trim().to_owned());
            }
            found_other = true;
        }
    }
    Err(if found_other {
        InviteError::Malformed
    } else {
        InviteError::NoQrCode
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(expires: u64) -> Invitation {
        Invitation {
            peer_id: PeerId([3; 32]),
            name: "Studio Mac".into(),
            addrs: vec!["192.168.1.20:47820".parse().unwrap()],
            token: [5; 16],
            expires,
        }
    }

    #[test]
    fn invitations_round_trip_and_expire() {
        let inv = sample(1_000);
        let text = inv.encode();
        assert!(text.starts_with(PREFIX));
        assert!(
            text.bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b':')
        );
        assert_eq!(Invitation::decode(&text, 999).unwrap(), inv);
        assert_eq!(Invitation::decode(&text.to_lowercase(), 999).unwrap(), inv);
        assert_eq!(Invitation::decode(&text, 1_000), Err(InviteError::Expired));
        assert_eq!(Invitation::decode("hello", 0), Err(InviteError::Malformed));
        assert_eq!(
            Invitation::decode("DEXDROP1:AAAA", 0),
            Err(InviteError::Malformed)
        );
        assert!(qr_matrix(&text).is_some());
    }

    #[test]
    fn tokens_are_single_use() {
        let mut book = InvitationBook::default();
        let inv = book.create(
            PeerId([1; 32]),
            "x",
            vec!["10.0.0.1:1".parse().unwrap()],
            60,
        );
        let now = unix_now();
        assert_eq!(
            book.redeem(&Pairing::Token([0; 16]), now),
            Err(InviteError::Unknown)
        );
        assert_eq!(book.redeem(&Pairing::Token(inv.token), now), Ok(()));
        assert_eq!(
            book.redeem(&Pairing::Token(inv.token), now),
            Err(InviteError::Unknown)
        );
    }

    #[test]
    fn tokens_expire() {
        let mut book = InvitationBook::default();
        let inv = book.create(
            PeerId([1; 32]),
            "x",
            vec!["10.0.0.1:1".parse().unwrap()],
            60,
        );
        assert_eq!(
            book.redeem(&Pairing::Token(inv.token), unix_now() + 61),
            Err(InviteError::Unknown)
        );
    }

    #[test]
    fn codes_normalize_and_lock_after_failures() {
        let mut book = InvitationBook::default();
        let (code, _) = book.create_code(60);
        assert!(book.has_open_code(unix_now()));
        let typed = display_code(&code).to_lowercase();
        assert_eq!(normalize_code(&typed).as_deref(), Some(code.as_str()));
        let now = unix_now();
        for _ in 0..MAX_CODE_FAILURES - 1 {
            assert_eq!(
                book.redeem(&Pairing::Code("00000000".into()), now),
                Err(InviteError::Unknown)
            );
        }
        assert_eq!(
            book.redeem(&Pairing::Code("00000000".into()), now),
            Err(InviteError::TooManyAttempts)
        );
        assert_eq!(
            book.redeem(&Pairing::Code(code), now),
            Err(InviteError::TooManyAttempts)
        );
        assert!(!book.has_open_code(now));
    }

    #[test]
    fn correct_codes_open_once() {
        let mut book = InvitationBook::default();
        let (code, _) = book.create_code(60);
        let now = unix_now();
        assert_eq!(
            book.redeem(&Pairing::Code(display_code(&code)), now),
            Ok(())
        );
        assert_eq!(
            book.redeem(&Pairing::Code(code), now),
            Err(InviteError::Unknown)
        );
    }

    /// Renders `text` as a QR code picture, as a screenshot would show it.
    fn qr_png(text: &str, scale: u32) -> Vec<u8> {
        let (width, modules) = qr_matrix(text).unwrap();
        let side = (width as u32 + 8) * scale;
        let img = image::GrayImage::from_fn(side, side, |x, y| {
            let (mx, my) = ((x / scale) as i64 - 4, (y / scale) as i64 - 4);
            let dark = mx >= 0
                && my >= 0
                && (mx as usize) < width
                && (my as usize) < width
                && modules[my as usize * width + mx as usize];
            image::Luma([if dark { 20 } else { 235 }])
        });
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    #[test]
    fn reads_invitations_from_pictures() {
        let text = sample(u64::MAX).encode();
        assert_eq!(read_qr_image(&qr_png(&text, 6)).unwrap(), text);
        assert_eq!(
            Invitation::decode(&read_qr_image(&qr_png(&text, 3)).unwrap(), 0).unwrap(),
            sample(u64::MAX)
        );
        // A QR code of something else, and a picture without one.
        assert_eq!(
            read_qr_image(&qr_png("https://example.com", 6)),
            Err(InviteError::Malformed)
        );
        let blank = {
            let mut out = std::io::Cursor::new(Vec::new());
            image::GrayImage::from_pixel(64, 64, image::Luma([255]))
                .write_to(&mut out, image::ImageFormat::Png)
                .unwrap();
            out.into_inner()
        };
        assert_eq!(read_qr_image(&blank), Err(InviteError::NoQrCode));
        assert_eq!(read_qr_image(b"not an image"), Err(InviteError::NoQrCode));
    }

    #[test]
    fn normalizes_look_alikes() {
        assert_eq!(normalize_code("abcd-efgo").as_deref(), Some("ABCDEFG0"));
        assert_eq!(normalize_code("il11-2345").as_deref(), Some("11112345"));
        assert_eq!(normalize_code("short"), None);
        assert_eq!(normalize_code("ABCDEFGU"), None);
    }
}
