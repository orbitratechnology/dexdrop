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

/// A device the user chose to trust (explicit consent only).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustedPeer {
    pub peer_id: PeerId,
    pub name: String,
    pub platform: String,
    /// Accept its transfers without asking (only with the global setting).
    pub auto_accept: bool,
    pub added: u64,
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
            });
        }
        assert_eq!(history.entries.len(), History::CAP);
        assert_eq!(
            history.entries[0].transfer_id,
            (History::CAP + 4).to_string()
        );
    }
}
