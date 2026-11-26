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
//! - [`avatar`]: a device's chosen emoji and color.
//! - [`format`]: human-readable sizes, rates and durations.
//! - [`asset`], [`source`]: selected items as references, and reading any
//!   source (a file or a referenced item) to send.

pub mod api;
pub mod asset;
pub mod avatar;
pub mod format;
pub mod fsutil;
pub mod identity;
pub mod invitation;
pub mod journal;
#[cfg(not(target_arch = "wasm32"))]
pub mod keystore;
pub mod manifest;
pub mod naming;
pub mod pathsafe;
pub mod protocol;
pub mod source;
pub mod store;

#[cfg(not(target_arch = "wasm32"))]
pub use identity::Identity;
pub use identity::PeerId;

/// The application's display name.
pub const APP_NAME: &str = "DexDrop";

/// The DNS-SD service type every DexDrop instance advertises and browses.
///
/// iOS requires this exact string in `NSBonjourServices`.
pub const SERVICE_TYPE: &str = "_dexdrop._tcp.local.";

/// Random bytes from the operating system.
#[cfg(not(target_arch = "wasm32"))]
pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    getrandom::fill(&mut out).expect("the operating system's random source is available");
    out
}

/// A random 128-bit identifier (transfer IDs, invitation tokens).
#[cfg(not(target_arch = "wasm32"))]
pub fn random_id() -> [u8; 16] {
    random_bytes::<16>()
}

/// Lower-case hex of `bytes`.
pub fn hex(bytes: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(bytes)
}

/// The current time as Unix seconds.
#[cfg(not(target_arch = "wasm32"))]
pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The current time as Unix seconds (the browser has no `SystemTime`).
#[cfg(target_arch = "wasm32")]
pub fn unix_now() -> u64 {
    0
}
