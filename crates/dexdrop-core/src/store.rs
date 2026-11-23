//! Local-only app state: settings, trusted devices and history (FR-015,
//! FR-060, FR-070 – FR-074). Everything stays on the device as small JSON
//! files in the app's private data directory.

use std::{
    io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{PeerId, fsutil, journal::RecordDir};

/// The files of the app's state directory.
#[derive(Clone, Debug)]
pub struct StateDir {
    root: PathBuf,
}

impl StateDir {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn identity(&self) -> PathBuf {
        self.root.join("identity.json")
    }

    pub fn settings(&self) -> PathBuf {
        self.root.join("settings.json")
    }

    pub fn trusted(&self) -> PathBuf {
        self.root.join("trusted-devices.json")
    }

    pub fn known(&self) -> PathBuf {
        self.root.join("known-devices.json")
    }

    pub fn history(&self) -> PathBuf {
        self.root.join("history.json")
    }

    pub fn incoming(&self) -> RecordDir {
        RecordDir::new(self.root.join("incoming"))
    }

    pub fn outgoing(&self) -> RecordDir {
        RecordDir::new(self.root.join("outgoing"))
    }

    /// Where picked files are copied when the platform gives bytes, not paths.
    pub fn staging(&self) -> PathBuf {
        self.root.join("staging")
    }
}

/// The user's settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// The name other devices see.
    pub device_name: String,
    /// Whether this device advertises itself to nearby devices (FR-071).
    pub visible: bool,
    /// Where received files go; `None`: the platform default.
    pub download_dir: Option<PathBuf>,
    /// Accept transfers from trusted devices that allow it without asking.
    pub auto_accept_trusted: bool,
    /// Keep a local history of transfers.
    pub keep_history: bool,
    /// Allow transfers over cellular / metered connections (FR-072).
    pub allow_metered: bool,
    /// The TCP port to listen on; 0: any free port.
    pub port: u16,
    /// The user has seen why DexDrop uses the local network (FR-004). On
    /// platforms that ask permission for it (iOS, macOS), discovery waits
    /// for this, so the system's prompt follows the explanation.
    pub local_network_explained: bool,
    /// Send and receive over the internet too (Phase 6): devices this one
    /// exchanged files with, and invitations, from any network.
    pub internet: bool,
    /// This device's avatar ([`crate::avatar`]); `None`: its initial.
    pub avatar: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            device_name: String::new(),
            visible: true,
            download_dir: None,
            auto_accept_trusted: false,
            keep_history: true,
            allow_metered: false,
            port: 0,
            local_network_explained: false,
            internet: true,
            avatar: None,
        }
    }
}

impl Settings {
    pub fn load(path: &Path) -> Self {
        fsutil::read_json(path).ok().flatten().unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        fsutil::write_json(path, self)
    }
}

/// A device this one exchanged files with, with its public key, so it can
/// be reached over the internet later (Phase 6). Sending to it still asks
/// the other side, unless it trusts this device.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownPeer {
    pub peer_id: PeerId,
    pub name: String,
    pub platform: String,
    /// Its raw Ed25519 public key: its internet address.
    pub key: [u8; 32],
    /// Its home relay when last seen, as a hint.
    #[serde(default)]
    pub relay: Option<String>,
    /// Unix seconds.
    pub last_seen: u64,
    /// Its avatar when last seen.
    #[serde(default)]
    pub avatar: Option<String>,
}

/// The known devices, most recent first.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownPeers {
    pub peers: Vec<KnownPeer>,
}

impl KnownPeers {
    /// At most this many are kept.
    pub const CAP: usize = 32;

    pub fn load(path: &Path) -> Self {
        let mut known: Self = fsutil::read_json(path).ok().flatten().unwrap_or_default();
        known.peers.truncate(Self::CAP);
        known
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        fsutil::write_json(path, self)
    }

    pub fn get(&self, id: &PeerId) -> Option<&KnownPeer> {
        self.peers.iter().find(|p| p.peer_id == *id)
    }

    /// Adds or refreshes a device, first in the list; the key must name the
    /// same peer ID (it is checked, so a stored key always matches).
    pub fn remember(&mut self, peer: KnownPeer) -> bool {
        if PeerId::from_ed25519_key(&peer.key) != peer.peer_id {
            return false;
        }
        self.peers.retain(|p| p.peer_id != peer.peer_id);
        self.peers.insert(0, peer);
        self.peers.truncate(Self::CAP);
        true
    }

    pub fn forget(&mut self, id: &PeerId) -> bool {
        let before = self.peers.len();
        self.peers.retain(|p| p.peer_id != *id);
        self.peers.len() != before
    }
}

/// A device the user chose to trust (explicit consent only).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustedPeer {
    pub peer_id: PeerId,
    pub name: String,
    pub platform: String,
    /// Accept its transfers without asking (only with the global setting).
    pub auto_accept: bool,
    pub added: u64,
    /// Its avatar when last seen.
    #[serde(default)]
    pub avatar: Option<String>,
}

/// The trusted devices.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustList {
    pub peers: Vec<TrustedPeer>,
}

impl TrustList {
    pub fn load(path: &Path) -> Self {
        fsutil::read_json(path).ok().flatten().unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        fsutil::write_json(path, self)
    }

    pub fn get(&self, id: &PeerId) -> Option<&TrustedPeer> {
        self.peers.iter().find(|p| &p.peer_id == id)
    }

    /// Adds or updates a trusted device.
    pub fn trust(&mut self, peer: TrustedPeer) {
        self.peers.retain(|p| p.peer_id != peer.peer_id);
        self.peers.push(peer);
    }

    /// Forgets a device; whether it was trusted.
    pub fn forget(&mut self, id: &PeerId) -> bool {
        let before = self.peers.len();
        self.peers.retain(|p| &p.peer_id != id);
        before != self.peers.len()
    }
}

/// Which way a transfer went.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    Outgoing,
    Incoming,
}

/// How a transfer ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    Completed,
    /// Some files failed or were skipped.
    Partial {
        failed: u64,
        skipped: u64,
    },
    Declined,
    Cancelled,
    Failed(String),
}

/// A finished transfer. Holds no file names unless the user keeps history,
/// and never file contents or full paths of the other device.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub transfer_id: String,
    pub direction: Direction,
    pub peer_name: String,
    pub peer_platform: String,
    pub label: String,
    pub files: u64,
    pub bytes: u64,
    pub outcome: Outcome,
    pub finished: u64,
    /// The local folder the files went to (incoming only).
    pub saved_to: Option<PathBuf>,
    /// The other device, when known.
    #[serde(default)]
    pub peer_id: Option<PeerId>,
    /// Measured average speed in bytes per second, for larger transfers.
    #[serde(default)]
    pub average_rate: Option<f64>,
    /// The other device's avatar.
    #[serde(default)]
    pub peer_avatar: Option<String>,
    /// When the transfer started, Unix seconds.
    #[serde(default)]
    pub started: Option<u64>,
    /// How the data went.
    #[serde(default)]
    pub route: Option<crate::api::Route>,
    /// The first files (up to [`FileItem::LISTED`]), with where each is on
    /// this device.
    #[serde(default)]
    pub items: Vec<FileItem>,
}

/// A file of a transfer, as the app lists it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileItem {
    /// Its `/`-separated path within the transfer.
    pub path: String,
    pub size: u64,
    /// Where it is on this device: the file sent, or the file saved.
    pub local: Option<PathBuf>,
}

impl FileItem {
    /// The most files a transfer lists.
    pub const LISTED: usize = 100;

    /// Its name, without folders.
    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }
}

/// The transfer history, newest first, capped.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct History {
    pub entries: Vec<HistoryEntry>,
}

impl History {
    pub const CAP: usize = 200;

    pub fn load(path: &Path) -> Self {
        fsutil::read_json(path).ok().flatten().unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        fsutil::write_json(path, self)
    }

    pub fn push(&mut self, entry: HistoryEntry) {
        self.entries.retain(|e| e.transfer_id != entry.transfer_id);
        self.entries.insert(0, entry);
        self.entries.truncate(Self::CAP);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_default_and_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let state = StateDir::new(dir.path());
        let mut settings = Settings::load(&state.settings());
        assert!(settings.visible);
        assert!(!settings.auto_accept_trusted);
        settings.device_name = "Desk".into();
        settings.save(&state.settings()).unwrap();
        assert_eq!(Settings::load(&state.settings()).device_name, "Desk");
    }

    #[test]
    fn trust_list_adds_and_forgets() {
        let mut list = TrustList::default();
        let peer = TrustedPeer {
            peer_id: PeerId([1; 32]),
            name: "A".into(),
            platform: "linux".into(),
            auto_accept: false,
            added: 0,
            avatar: None,
        };
        list.trust(peer.clone());
        list.trust(TrustedPeer {
            name: "B".into(),
            ..peer.clone()
        });
        assert_eq!(list.peers.len(), 1);
        assert_eq!(list.get(&peer.peer_id).unwrap().name, "B");
        assert!(list.forget(&peer.peer_id));
        assert!(!list.forget(&peer.peer_id));
    }

    #[test]
    fn history_is_capped_newest_first() {
        let mut history = History::default();
        for i in 0..(History::CAP + 5) {
            history.push(HistoryEntry {
                transfer_id: i.to_string(),
                direction: Direction::Outgoing,
                peer_name: "p".into(),
                peer_platform: "linux".into(),
                label: "l".into(),
                files: 1,
                bytes: 1,
                outcome: Outcome::Completed,
                finished: i as u64,
                saved_to: None,
                peer_id: None,
                average_rate: None,
                peer_avatar: None,
                started: None,
                route: None,
                items: Vec::new(),
            });
        }
        assert_eq!(history.entries.len(), History::CAP);
        assert_eq!(
            history.entries[0].transfer_id,
            (History::CAP + 4).to_string()
        );
    }
}
