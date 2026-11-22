//! DexDrop's platform-independent core.
//!
//! Nothing in this crate touches sockets, async runtimes or UI. It holds the
//! pieces every platform shares and that must be testable on their own:
//!
//! - [`identity`]: the device's cryptographic identity and peer IDs.
//! - [`protocol`]: the versioned, bounded wire messages.
//! - [`pathsafe`]: validation of peer-supplied relative paths.
//! - [`manifest`]: building the list of files and folders to send.
//! - [`naming`]: "keep both" names for destination conflicts.
//! - [`journal`]: the persisted resume state of incoming transfers.
//! - [`invitation`]: QR/link invitations and short pairing codes.
//! - [`store`]: settings, trusted devices and history, stored locally.
//! - [`format`]: human-readable sizes, rates and durations.

pub mod format;
pub mod fsutil;
pub mod identity;
pub mod invitation;
pub mod journal;
pub mod manifest;
pub mod naming;
pub mod pathsafe;
pub mod protocol;
pub mod store;

pub use identity::{Identity, PeerId};

/// The application's display name.
pub const APP_NAME: &str = "DexDrop";

/// The DNS-SD service type every DexDrop instance advertises and browses.
///
/// iOS requires this exact string in `NSBonjourServices`.
pub const SERVICE_TYPE: &str = "_dexdrop._tcp.local.";

/// Random bytes from the operating system.
pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    getrandom::fill(&mut out).expect("the operating system's random source is available");
    out
}

/// A random 128-bit identifier (transfer IDs, invitation tokens).
pub fn random_id() -> [u8; 16] {
    random_bytes::<16>()
}

/// Lower-case hex of `bytes`.
pub fn hex(bytes: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(bytes)
}

/// The current time as Unix seconds.
pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
