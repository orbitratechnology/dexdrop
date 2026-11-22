//! The versioned DexDrop wire protocol (PRD 9.7).
//!
//! Every control message is a frame: a 4-byte big-endian length followed by a
//! [postcard](https://docs.rs/postcard) encoding of [`Message`]. Frames are
//! bounded by [`MAX_FRAME`] and decoded only after the length is checked, so a
//! peer cannot make the receiver allocate more than that. File bytes are not
//! inside messages: a [`Message::Chunk`] header is followed on the stream by
//! exactly `len` raw bytes.
//!
//! Session flow (sender connects, receiver listens; both over mutual TLS):
//!
//! 1. Both send [`Hello`]; the highest common version is used.
//! 2. Sender sends [`Offer`] and the manifest in [`ManifestPage`]s.
//! 3. Receiver asks its user, then answers [`Message::Accept`] or
//!    [`Message::Reject`].
//! 4. Receiver sends, in manifest order, [`Message::Want`] (with the offset to
//!    resume from) or [`Message::Skip`] per file. At most a bounded number of
//!    requests are outstanding at once.
//! 5. Sender answers each `Want` with [`Message::FileStart`], chunks and
//!    [`Message::FileEnd`] (carrying the whole-file BLAKE3).
//! 6. Receiver verifies, commits and reports [`Message::FileResult`]. A file
//!    that fails verification is requested again once. When every file is
//!    resolved the receiver sends [`Message::Complete`].
//!
//! Either side may send [`Message::Cancel`] or [`Message::Pause`] at any time.

use serde::{Deserialize, Serialize};

/// The newest protocol version this build speaks.
pub const PROTOCOL_VERSION: u16 = 1;
/// The oldest protocol version this build still speaks.
pub const MIN_PROTOCOL_VERSION: u16 = 1;
/// The TLS ALPN identifier.
pub const ALPN: &[u8] = b"dexdrop/1";

/// The largest control frame accepted, in bytes.
pub const MAX_FRAME: usize = 256 * 1024;
/// Entries per manifest page.
pub const MANIFEST_PAGE_ENTRIES: usize = 256;
/// The most entries (files and folders) one transfer may hold.
pub const MAX_ENTRIES: u64 = 250_000;
/// The longest relative path accepted, in bytes.
pub const MAX_PATH_BYTES: usize = 4096;
/// The most manifest bytes (paths and metadata) one offer may carry, read
/// before the user is asked: bounds what an unknown peer can make the
/// receiver hold.
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024 * 1024;
/// The longest offer label accepted, in bytes.
pub const MAX_LABEL_BYTES: usize = 256;
/// The longest device name accepted, in bytes.
pub const MAX_NAME_BYTES: usize = 128;

/// Default chunk size: each chunk is hashed and acknowledged on its own.
pub const DEFAULT_CHUNK_SIZE: u32 = 4 * 1024 * 1024;
/// Smallest chunk size a peer may ask for.
pub const MIN_CHUNK_SIZE: u32 = 64 * 1024;
/// Largest chunk size a peer may ask for.
pub const MAX_CHUNK_SIZE: u32 = 16 * 1024 * 1024;

/// A 128-bit transfer identifier.
pub type TransferId = [u8; 16];

/// How a connecting sender proves it was invited (QR/link token or code).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Pairing {
    /// The token of a QR/link invitation.
    Token([u8; 16]),
    /// A short code typed by the user.
    Code(String),
}

/// The first message each side sends.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub protocol: u16,
    pub min_protocol: u16,
    pub app_version: String,
    pub device_name: String,
    /// `"android"`, `"ios"`, `"macos"`, `"windows"`, `"linux"`, `"web"`.
    pub platform: String,
    pub pairing: Option<Pairing>,
}

impl Hello {
    /// This build's hello.
    pub fn new(device_name: &str, platform: &str, pairing: Option<Pairing>) -> Self {
        Self {
            protocol: PROTOCOL_VERSION,
            min_protocol: MIN_PROTOCOL_VERSION,
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            device_name: truncate(device_name, MAX_NAME_BYTES),
            platform: truncate(platform, 16),
            pairing,
        }
    }

    /// The version both sides speak, if any.
    pub fn negotiate(&self, theirs: &Hello) -> Option<u16> {
        let high = self.protocol.min(theirs.protocol);
        let low = self.min_protocol.max(theirs.min_protocol);
        (high >= low).then_some(high)
    }
}

/// What a sender proposes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Offer {
    pub transfer_id: TransferId,
    /// Files and folders in the manifest.
    pub entries: u64,
    pub files: u64,
    pub folders: u64,
    pub total_bytes: u64,
    pub chunk_size: u32,
    /// A short label: the single item's name, or the top-level names.
    pub label: String,
    /// Whether the sender is resuming a transfer the receiver accepted before.
    pub resume: bool,
}

/// What kind of entry a manifest line is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryKind {
    File,
    Folder,
}

/// One manifest line: a relative path, never an absolute source path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireEntry {
    /// `/`-separated relative path.
    pub path: String,
    pub kind: EntryKind,
    pub size: u64,
    /// Modification time, Unix seconds, when known.
    pub mtime: Option<u64>,
}

/// A page of the manifest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestPage {
    pub entries: Vec<WireEntry>,
    pub last: bool,
}

/// Why a receiver turned an offer down.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RejectReason {
    Declined,
    Busy,
    InvalidManifest,
    NotEnoughSpace,
    InvitationInvalid,
    Incompatible,
}

impl RejectReason {
    /// A user-facing explanation.
    pub fn explain(self) -> &'static str {
        match self {
            RejectReason::Declined => "The other device declined the transfer.",
            RejectReason::Busy => "The other device is busy with too many requests.",
            RejectReason::InvalidManifest => "The other device could not accept these file names.",
            RejectReason::NotEnoughSpace => "The other device does not have enough free space.",
            RejectReason::InvitationInvalid => {
                "The invitation or code was not valid or has expired."
            }
            RejectReason::Incompatible => "The other device runs an incompatible version.",
        }
    }
}

/// Why one file failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileFailure {
    /// The source changed or disappeared while sending.
    SourceChanged,
    /// The source could not be read.
    SourceUnreadable,
    /// Data did not match its hash after a retry.
    IntegrityMismatch,
    /// The destination could not be written.
    WriteFailed,
    /// The destination ran out of space.
    NoSpace,
}

impl FileFailure {
    pub fn explain(self) -> &'static str {
        match self {
            FileFailure::SourceChanged => "The file changed or was removed on the sending device.",
            FileFailure::SourceUnreadable => "The file could not be read on the sending device.",
            FileFailure::IntegrityMismatch => "The received data did not pass verification.",
            FileFailure::WriteFailed => "The file could not be saved on the receiving device.",
            FileFailure::NoSpace => "The receiving device ran out of storage space.",
        }
    }
}

/// Why a session ended early.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CancelReason {
    /// A user cancelled.
    User,
    /// The peer hit an error it cannot recover from.
    Error,
    /// The app is closing; the transfer can resume later.
    Shutdown,
}

/// The header of one chunk; `len` raw bytes follow it on the stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkHeader {
    pub index: u32,
    pub offset: u64,
    pub len: u32,
    pub hash: [u8; 32],
}

/// Every message of the protocol.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Message {
    Hello(Hello),
    Offer(Offer),
    ManifestPage(ManifestPage),
    Accept,
    Reject(RejectReason),
    /// Receiver: send file `index` from byte `offset`.
    Want {
        index: u32,
        offset: u64,
    },
    /// Receiver: do not send file `index`; `already_received` when an
    /// earlier session of this transfer saved it.
    Skip {
        index: u32,
        already_received: bool,
    },
    /// Sender: file `index` follows from `offset`.
    FileStart {
        index: u32,
        offset: u64,
    },
    Chunk(ChunkHeader),
    /// Sender: file `index` is complete; its whole-file hash.
    FileEnd {
        index: u32,
        hash: [u8; 32],
    },
    /// Sender: file `index` cannot be sent.
    FileError {
        index: u32,
        failure: FileFailure,
    },
    /// Receiver: file `index` was verified and saved, or failed.
    FileResult {
        index: u32,
        failure: Option<FileFailure>,
    },
    /// Receiver: every request was answered.
    Complete,
    /// Either side: stop now, keep resume state.
    Pause,
    /// Either side: stop now.
    Cancel(CancelReason),
    /// Receiver: stop, this device ran out of storage space; the verified
    /// part is kept and the transfer can resume once there is room.
    /// (Appended last: earlier variants keep their wire numbers.)
    OutOfSpace,
}

/// Errors decoding a frame.
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("frame of {0} bytes exceeds the limit")]
    FrameTooLarge(usize),
    #[error("malformed message: {0}")]
    Malformed(String),
    #[error("unexpected message: {0}")]
    Unexpected(&'static str),
}

impl Message {
    /// Encodes the message as a frame: length prefix and body.
    pub fn encode_frame(&self) -> Result<Vec<u8>, ProtocolError> {
        let body =
            postcard::to_stdvec(self).map_err(|e| ProtocolError::Malformed(e.to_string()))?;
        if body.len() > MAX_FRAME {
            return Err(ProtocolError::FrameTooLarge(body.len()));
        }
        let mut frame = Vec::with_capacity(4 + body.len());
        frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
        frame.extend_from_slice(&body);
        Ok(frame)
    }

    /// Checks a frame length read from the stream before allocating.
    pub fn check_frame_len(len: u32) -> Result<usize, ProtocolError> {
        let len = len as usize;
        if len > MAX_FRAME {
            Err(ProtocolError::FrameTooLarge(len))
        } else {
            Ok(len)
        }
    }

    /// Decodes a frame body.
    pub fn decode(body: &[u8]) -> Result<Self, ProtocolError> {
        postcard::from_bytes(body).map_err(|e| ProtocolError::Malformed(e.to_string()))
    }

    /// A short name for logs (never contents or paths).
    pub fn name(&self) -> &'static str {
        match self {
            Message::Hello(_) => "Hello",
            Message::Offer(_) => "Offer",
            Message::ManifestPage(_) => "ManifestPage",
            Message::Accept => "Accept",
            Message::Reject(_) => "Reject",
            Message::Want { .. } => "Want",
            Message::Skip { .. } => "Skip",
            Message::FileStart { .. } => "FileStart",
            Message::Chunk(_) => "Chunk",
            Message::FileEnd { .. } => "FileEnd",
            Message::FileError { .. } => "FileError",
            Message::FileResult { .. } => "FileResult",
            Message::Complete => "Complete",
            Message::Pause => "Pause",
            Message::Cancel(_) => "Cancel",
            Message::OutOfSpace => "OutOfSpace",
        }
    }
}

/// The digest of a manifest: binds resume state to exactly these entries.
pub fn manifest_digest(
    transfer_id: &TransferId,
    chunk_size: u32,
    entries: &[WireEntry],
) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new_derive_key("dexdrop 2026 manifest v1");
    hasher.update(transfer_id);
    hasher.update(&chunk_size.to_be_bytes());
    for entry in entries {
        let bytes = postcard::to_stdvec(entry).unwrap_or_default();
        hasher.update(&(bytes.len() as u32).to_be_bytes());
        hasher.update(&bytes);
    }
    *hasher.finalize().as_bytes()
}

/// The BLAKE3 hash of a chunk.
pub fn chunk_hash(data: &[u8]) -> [u8; 32] {
    *blake3::hash(data).as_bytes()
}

/// A peer's device name made safe to show and store: control characters
/// removed, at most [`MAX_NAME_BYTES`], never empty.
pub fn clean_name(name: &str) -> String {
    let cleaned: String = name.chars().filter(|c| !c.is_control()).collect();
    let cleaned = truncate(cleaned.trim(), MAX_NAME_BYTES);
    if cleaned.is_empty() {
        "Unnamed device".to_owned()
    } else {
        cleaned
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_owned();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip() {
        let messages = vec![
            Message::Hello(Hello::new(
                "Desk",
                "linux",
                Some(Pairing::Code("ABCD1234".into())),
            )),
            Message::Want {
                index: 3,
                offset: 4096,
            },
            Message::Chunk(ChunkHeader {
                index: 1,
                offset: 0,
                len: 10,
                hash: [7; 32],
            }),
            Message::ManifestPage(ManifestPage {
                entries: vec![WireEntry {
                    path: "a/b.txt".into(),
                    kind: EntryKind::File,
                    size: 5,
                    mtime: None,
                }],
                last: true,
            }),
        ];
        for message in messages {
            let frame = message.encode_frame().unwrap();
            let len = u32::from_be_bytes(frame[..4].try_into().unwrap());
            assert_eq!(Message::check_frame_len(len).unwrap(), frame.len() - 4);
            assert_eq!(Message::decode(&frame[4..]).unwrap(), message);
        }
    }

    #[test]
    fn rejects_oversized_and_malformed_frames() {
        assert!(Message::check_frame_len(MAX_FRAME as u32 + 1).is_err());
        assert!(Message::decode(&[0xff, 0xff, 0xff]).is_err());
        assert!(Message::decode(&[]).is_err());
        // A huge claimed vector length inside a small frame cannot allocate.
        assert!(Message::decode(&[2, 0xff, 0xff, 0xff, 0xff, 0x0f]).is_err());
    }

    #[test]
    fn negotiates_versions() {
        let mine = Hello::new("a", "linux", None);
        let mut theirs = mine.clone();
        assert_eq!(mine.negotiate(&theirs), Some(PROTOCOL_VERSION));
        theirs.min_protocol = PROTOCOL_VERSION + 1;
        theirs.protocol = PROTOCOL_VERSION + 2;
        assert_eq!(mine.negotiate(&theirs), None);
    }

    #[test]
    fn digests_depend_on_every_field() {
        let entries = vec![WireEntry {
            path: "a".into(),
            kind: EntryKind::File,
            size: 1,
            mtime: Some(2),
        }];
        let a = manifest_digest(&[1; 16], DEFAULT_CHUNK_SIZE, &entries);
        assert_ne!(a, manifest_digest(&[2; 16], DEFAULT_CHUNK_SIZE, &entries));
        assert_ne!(a, manifest_digest(&[1; 16], MIN_CHUNK_SIZE, &entries));
        let mut other = entries.clone();
        other[0].size = 3;
        assert_ne!(a, manifest_digest(&[1; 16], DEFAULT_CHUNK_SIZE, &other));
    }

    #[test]
    fn peer_names_are_cleaned() {
        assert_eq!(clean_name("  Desk\u{0}\n PC "), "Desk PC");
        assert_eq!(clean_name("\u{7}"), "Unnamed device");
        assert!(clean_name(&"名".repeat(500)).len() <= MAX_NAME_BYTES);
    }

    #[test]
    fn names_are_truncated_on_char_boundaries() {
        let long = "é".repeat(200);
        let hello = Hello::new(&long, "linux", None);
        assert!(hello.device_name.len() <= MAX_NAME_BYTES);
    }
}
