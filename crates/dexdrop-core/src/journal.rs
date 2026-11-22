//! Persisted transfer state, so interrupted transfers resume (FR-053, 9.10).
//!
//! An incoming transfer keeps a [`ReceiveJournal`]: the manifest digest it was
//! accepted for, where it writes, and per file the verified prefix of its
//! temporary file. Data is flushed to disk before the journal records it, so
//! a crash can lose progress but never marks unverified bytes as verified.
//!
//! An outgoing transfer keeps an [`OutgoingRecord`] so it can be resumed after
//! the app restarts.

use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{
    PeerId, fsutil, hex, manifest::Manifest, naming::ConflictPolicy, protocol::TransferId,
};

/// Suffix of temporary files written while receiving.
pub const PARTIAL_SUFFIX: &str = ".dexdrop-part";

/// One file's progress.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileProgress {
    /// Bytes of the temporary file that are written, flushed and verified.
    pub verified: u64,
    pub state: FileState,
    /// The temporary file, while partial.
    pub temp: Option<PathBuf>,
    /// Where the file was saved, once done.
    pub saved: Option<PathBuf>,
}

/// Where one file stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileState {
    #[default]
    Pending,
    Partial,
    Done,
    Skipped,
    Failed,
}

/// An incoming transfer's resume state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiveJournal {
    pub version: u32,
    pub transfer_id: TransferId,
    pub peer_id: PeerId,
    pub peer_name: String,
    pub manifest_digest: [u8; 32],
    pub destination: PathBuf,
    pub conflict: ConflictPolicy,
    pub label: String,
    pub total_bytes: u64,
    pub created: u64,
    /// Top-level folder names given a "keep both" name, kept for resume.
    #[serde(default)]
    pub renames: BTreeMap<String, String>,
    pub files: BTreeMap<u32, FileProgress>,
}

impl ReceiveJournal {
    /// Bytes verified so far, over all files.
    pub fn verified_bytes(&self) -> u64 {
        self.files.values().map(|f| f.verified).sum()
    }
}

/// An outgoing transfer, kept so it can resume after a restart.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutgoingRecord {
    pub version: u32,
    pub transfer_id: TransferId,
    pub peer_id: PeerId,
    pub peer_name: String,
    pub peer_platform: String,
    /// Addresses the peer was last reached at.
    pub addrs: Vec<std::net::SocketAddr>,
    pub chunk_size: u32,
    pub manifest: Manifest,
    pub created: u64,
}

/// A directory of JSON records keyed by transfer ID.
#[derive(Clone, Debug)]
pub struct RecordDir {
    dir: PathBuf,
}

impl RecordDir {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn path(&self, id: &TransferId) -> PathBuf {
        self.dir.join(format!("{}.json", hex(id)))
    }

    pub fn load<T: DeserializeOwned>(&self, id: &TransferId) -> io::Result<Option<T>> {
        fsutil::read_json(&self.path(id))
    }

    pub fn save<T: Serialize>(&self, id: &TransferId, value: &T) -> io::Result<()> {
        fsutil::write_json(&self.path(id), value)
    }

    pub fn remove(&self, id: &TransferId) -> io::Result<()> {
        match fs::remove_file(self.path(id)) {
            Err(err) if err.kind() != io::ErrorKind::NotFound => Err(err),
            _ => Ok(()),
        }
    }

    /// Every readable record; unreadable ones are skipped.
    pub fn list<T: DeserializeOwned>(&self) -> Vec<T> {
        let Ok(read) = fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        read.filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .filter_map(|e| fsutil::read_json(&e.path()).ok().flatten())
            .collect()
    }
}

/// Removes temporary files of a journal (after cancel or when abandoned).
pub fn remove_partials(journal: &ReceiveJournal) {
    for file in journal.files.values() {
        if let Some(temp) = &file.temp
            && temp.to_string_lossy().ends_with(PARTIAL_SUFFIX)
        {
            let _ = fs::remove_file(temp);
        }
    }
}

/// The temporary file for `final_path`: a hidden sibling in the same folder,
/// so the final rename stays on one filesystem.
pub fn partial_path(final_path: &Path, transfer_id: &TransferId, index: u32) -> PathBuf {
    let name = final_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    // Only the name part is shortened, never the transfer ID and index that
    // make the temporary file unique (the sender picks the ID, so all of it
    // is kept). Stays well under the common 255-byte name limit.
    const NAME_BYTES: usize = 150;
    let mut name = name;
    if name.len() > NAME_BYTES {
        let mut end = NAME_BYTES;
        while !name.is_char_boundary(end) {
            end -= 1;
        }
        name.truncate(end);
    }
    let mut temp = final_path.to_path_buf();
    temp.set_file_name(format!(
        ".{name}.{}-{index}{PARTIAL_SUFFIX}",
        hex(transfer_id)
    ));
    temp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let records = RecordDir::new(dir.path().join("incoming"));
        let id = [9u8; 16];
        let mut journal = ReceiveJournal {
            version: 1,
            transfer_id: id,
            peer_id: PeerId([1; 32]),
            peer_name: "Laptop".into(),
            manifest_digest: [2; 32],
            destination: dir.path().to_path_buf(),
            conflict: ConflictPolicy::KeepBoth,
            label: "x".into(),
            total_bytes: 10,
            created: 0,
            renames: BTreeMap::new(),
            files: BTreeMap::new(),
        };
        journal.files.insert(
            0,
            FileProgress {
                verified: 4,
                state: FileState::Partial,
                temp: None,
                saved: None,
            },
        );
        records.save(&id, &journal).unwrap();
        let back: ReceiveJournal = records.load(&id).unwrap().unwrap();
        assert_eq!(back, journal);
        assert_eq!(back.verified_bytes(), 4);
        assert_eq!(records.list::<ReceiveJournal>().len(), 1);
        records.remove(&id).unwrap();
        assert!(records.load::<ReceiveJournal>(&id).unwrap().is_none());
    }

    #[test]
    fn partial_paths_are_hidden_siblings() {
        let p = partial_path(Path::new("/d/photo.jpg"), &[0xab; 16], 3);
        assert_eq!(
            p,
            Path::new("/d/.photo.jpg.abababababababababababababababab-3.dexdrop-part")
        );
        // Long names that share a prefix never share a temporary file.
        let long_a = format!("{}A.txt", "日本".repeat(60));
        let long_b = format!("{}B.txt", "日本".repeat(60));
        let a = partial_path(&Path::new("/d").join(&long_a), &[1; 16], 0);
        let b = partial_path(&Path::new("/d").join(&long_b), &[1; 16], 1);
        let c = partial_path(&Path::new("/d").join(&long_a), &[2; 16], 0);
        assert_ne!(a, b);
        assert_ne!(a, c);
        for p in [a, b, c] {
            assert!(p.file_name().unwrap().len() < 255);
        }
    }
}
