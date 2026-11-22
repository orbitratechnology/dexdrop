//! The engine's public contract: commands in, events and snapshots out.
//!
//! It lives in the core crate so a UI can be compiled where the engine cannot
//! run (the web preview), and is re-exported by `dexdrop-net`.

use std::{
    net::{IpAddr, SocketAddr},
    path::PathBuf,
};

use crate::{
    PeerId,
    naming::ConflictPolicy,
    protocol::TransferId,
    store::{Direction, HistoryEntry, Settings, TrustedPeer},
};

/// Where to send files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A device found nearby.
    Peer(PeerId),
    /// A QR/link invitation's text.
    Invitation(String),
    /// A short code shown by a nearby device.
    Code(String),
    /// An address, optionally pinned to the peer ID expected there.
    Address {
        addr: SocketAddr,
        peer_id: Option<PeerId>,
    },
}

/// What the UI asks of the engine.
#[derive(Clone, Debug)]
pub enum Command {
    SendFiles {
        target: Target,
        paths: Vec<PathBuf>,
    },
    /// Answer an incoming request.
    Respond {
        id: TransferId,
        accept: bool,
        trust: bool,
        conflict: ConflictPolicy,
    },
    Pause(TransferId),
    Resume(TransferId),
    Cancel(TransferId),
    /// Remove a finished transfer from the list.
    Dismiss(TransferId),
    ClearFinished,
    CreateInvitation,
    CreateCode,
    CloseInvitations,
    SetDeviceName(String),
    SetVisible(bool),
    SetDownloadDir(Option<PathBuf>),
    SetAutoAcceptTrusted(bool),
    SetKeepHistory(bool),
    SetAllowMetered(bool),
    SetPeerAutoAccept {
        peer_id: PeerId,
        auto_accept: bool,
    },
    ForgetPeer(PeerId),
    ClearHistory,
    RefreshDiscovery,
    ExportDiagnostics,
    Shutdown,
}

/// Whether nearby discovery works.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiscoveryStatus {
    Active,
    /// Turned off in the engine's configuration (tests, CLI).
    Disabled,
    Unavailable(String),
}

/// This device as others see it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub peer_id: PeerId,
    pub fingerprint: String,
    pub device_name: String,
    pub platform: String,
    pub port: u16,
    pub addrs: Vec<IpAddr>,
    pub visible: bool,
    pub discovery: DiscoveryStatus,
}

/// A device the user can send to.
#[derive(Clone, Debug, PartialEq)]
pub struct PeerInfo {
    pub peer_id: PeerId,
    pub name: String,
    pub platform: String,
    pub fingerprint: String,
    pub trusted: bool,
    pub code_open: bool,
    /// The average speed of the last measured transfer with this device
    /// (bytes per second); only measured data, never estimated (FR-016).
    pub last_rate: Option<f64>,
}

/// Where a transfer stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransferState {
    /// Reading the files to send.
    Preparing,
    Connecting,
    /// Waiting for the other device to accept.
    Waiting,
    /// Incoming: waiting for this device's user.
    Requested,
    Transferring,
    Paused {
        by_peer: bool,
    },
    /// The connection dropped; resumes automatically or on request.
    Interrupted {
        reason: String,
        retrying: bool,
    },
    Completed,
    /// Finished, but some files failed or were skipped.
    CompletedWithIssues,
    Declined(String),
    Cancelled {
        by_peer: bool,
    },
    Failed(String),
}

impl TransferState {
    /// Whether the transfer has ended for good.
    pub fn is_finished(&self) -> bool {
        matches!(
            self,
            TransferState::Completed
                | TransferState::CompletedWithIssues
                | TransferState::Declined(_)
                | TransferState::Cancelled { .. }
                | TransferState::Failed(_)
        )
    }

    /// A short status line.
    pub fn label(&self) -> String {
        match self {
            TransferState::Preparing => "Preparing…".into(),
            TransferState::Connecting => "Connecting…".into(),
            TransferState::Waiting => "Waiting for the other device to accept…".into(),
            TransferState::Requested => "Waiting for your answer".into(),
            TransferState::Transferring => "Transferring".into(),
            TransferState::Paused { by_peer: false } => "Paused".into(),
            TransferState::Paused { by_peer: true } => "Paused by the other device".into(),
            TransferState::Interrupted {
                reason,
                retrying: true,
            } => format!("{reason} Retrying…"),
            TransferState::Interrupted { reason, .. } => reason.clone(),
            TransferState::Completed => "Completed".into(),
            TransferState::CompletedWithIssues => "Completed with issues".into(),
            TransferState::Declined(reason) => reason.clone(),
            TransferState::Cancelled { by_peer: false } => "Cancelled".into(),
            TransferState::Cancelled { by_peer: true } => "Cancelled by the other device".into(),
            TransferState::Failed(reason) => reason.clone(),
        }
    }
}

/// A transfer as the UI shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct TransferSnapshot {
    pub id: TransferId,
    pub direction: Direction,
    pub peer_id: Option<PeerId>,
    pub peer_name: String,
    pub peer_platform: String,
    pub label: String,
    pub files: u64,
    pub folders: u64,
    pub total_bytes: u64,
    pub bytes_done: u64,
    pub files_done: u64,
    pub files_failed: u64,
    pub files_skipped: u64,
    /// Bytes per second, once measured.
    pub rate: Option<f64>,
    /// Seconds left, once the rate is stable.
    pub eta: Option<u64>,
    pub state: TransferState,
    /// The verification number of the current session.
    pub sas: Option<String>,
    /// Incoming: the folder the files are saved in.
    pub saved_to: Option<PathBuf>,
    /// The first few failures: (relative path, explanation).
    pub failures: Vec<(String, String)>,
    pub current: Option<String>,
    pub started: u64,
    /// The measured average speed while transferring, once finished.
    pub average_rate: Option<f64>,
    /// Incoming, completed: the files saved (up to 100), to share or
    /// export them from app-private storage on phones (FR-042).
    pub saved_files: Vec<std::path::PathBuf>,
}

impl TransferSnapshot {
    pub fn progress(&self) -> f64 {
        if self.total_bytes == 0 {
            if self.state.is_finished() { 1.0 } else { 0.0 }
        } else {
            (self.bytes_done as f64 / self.total_bytes as f64).clamp(0.0, 1.0)
        }
    }

    pub fn can_pause(&self) -> bool {
        self.direction == Direction::Outgoing
            && matches!(
                self.state,
                TransferState::Transferring
                    | TransferState::Connecting
                    | TransferState::Interrupted { .. }
            )
    }

    pub fn can_resume(&self) -> bool {
        self.direction == Direction::Outgoing
            && matches!(
                self.state,
                TransferState::Paused { .. }
                    | TransferState::Interrupted {
                        retrying: false,
                        ..
                    }
                    | TransferState::Failed(_)
            )
    }

    pub fn can_cancel(&self) -> bool {
        !self.state.is_finished() && self.state != TransferState::Requested
    }

    pub fn can_dismiss(&self) -> bool {
        self.state.is_finished()
    }
}

/// An incoming offer waiting for the user (FR-040, FR-041).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IncomingRequest {
    pub id: TransferId,
    pub peer_id: PeerId,
    pub peer_name: String,
    pub peer_platform: String,
    pub fingerprint: String,
    /// The 6-digit number the sender's screen shows too.
    pub sas: String,
    pub label: String,
    pub files: u64,
    pub folders: u64,
    pub total_bytes: u64,
    /// Up to five top-level names.
    pub sample: Vec<String>,
    pub trusted: bool,
    pub via_invitation: bool,
    pub destination: PathBuf,
    pub free_space: Option<u64>,
}

impl IncomingRequest {
    pub fn enough_space(&self) -> bool {
        self.free_space.is_none_or(|free| free >= self.total_bytes)
    }
}

/// A QR/link invitation to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvitationView {
    pub text: String,
    pub qr_width: usize,
    pub qr_modules: Vec<bool>,
    pub expires: u64,
}

/// How serious a notice is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeKind {
    Info,
    Success,
    Error,
}

/// What the engine reports.
#[derive(Clone, Debug)]
pub enum Event {
    Status(Status),
    Settings(Settings),
    Peers(Vec<PeerInfo>),
    Transfer(TransferSnapshot),
    TransferRemoved(TransferId),
    Request(IncomingRequest),
    RequestClosed(TransferId),
    Invitation(InvitationView),
    Code {
        code: String,
        expires: u64,
    },
    InvitationsClosed,
    /// A device connected with this device's code (`code`) or its QR/link
    /// invitation; that code or invitation no longer works.
    PairingUsed {
        code: bool,
        by: String,
    },
    Trusted(Vec<TrustedPeer>),
    History(Vec<HistoryEntry>),
    Notice {
        kind: NoticeKind,
        text: String,
    },
    Diagnostics(String),
    Stopped,
}
