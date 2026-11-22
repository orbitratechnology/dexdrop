//! Persisted transfer state, so interrupted transfers resume (FR-053, 9.10).
//!
//! An incoming transfer keeps a [`ReceiveJournal`]: the manifest digest it was
//! accepted for, where it writes, and per file the verified prefix of its
//! temporary file. Data is flushed to disk before the journal records it, so
//! a crash can lose progress but never marks unverified bytes as verified.
//!
//! On disk a journal is a snapshot (`<id>.json`) plus a log of the files
//! that changed since (`<id>.log`, one JSON line per change), so a save
//! costs the size of what changed, not of the whole manifest: a 200,000-file
//! journal is about 30 MB. The log is folded into a new snapshot once it
//! holds as many lines as the journal has files. Log lines carry the
//! snapshot's generation, so lines left from before a newer snapshot are
//! ignored, as is a line cut short by a crash.
//!
//! An outgoing transfer keeps an [`OutgoingRecord`] so it can be resumed after
//! the app restarts.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, BufRead, Write},
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
    /// Which log lines belong to this snapshot (see the module docs).
    #[serde(default)]
    pub log_generation: u64,
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

    fn log_path(&self, id: &TransferId) -> PathBuf {
        self.dir.join(format!("{}.log", hex(id)))
    }

    /// Removes a record, and a journal's log with it.
    pub fn remove(&self, id: &TransferId) -> io::Result<()> {
        remove_if_present(&self.log_path(id))?;
        remove_if_present(&self.path(id))
    }

    /// A receive journal: its snapshot with its log applied.
    pub fn load_journal(&self, id: &TransferId) -> io::Result<Option<ReceiveJournal>> {
        let Some(mut journal) = self.load::<ReceiveJournal>(id)? else {
            return Ok(None);
        };
        self.apply_log(&mut journal);
        Ok(Some(journal))
    }

    /// Every readable receive journal, logs applied.
    pub fn list_journals(&self) -> Vec<ReceiveJournal> {
        let mut journals = self.list::<ReceiveJournal>();
        for journal in &mut journals {
            self.apply_log(journal);
        }
        journals
    }

    /// Writes a new snapshot of `journal` and drops its log.
    pub fn save_journal(&self, journal: &mut ReceiveJournal) -> io::Result<()> {
        journal.log_generation += 1;
        let bytes = serde_json::to_vec(journal).map_err(io::Error::other)?;
        fsutil::write_atomic(&self.path(&journal.transfer_id), &bytes)?;
        // Lines left in the log are of the older generation and ignored, so
        // failing to remove it costs only space.
        let _ = remove_if_present(&self.log_path(&journal.transfer_id));
        Ok(())
    }

    /// Appends changed files to a journal's log and flushes it to disk.
    fn append_log(
        &self,
        journal: &ReceiveJournal,
        changed: impl Iterator<Item = u32>,
    ) -> io::Result<usize> {
        let mut buf = Vec::new();
        let mut lines = 0;
        for index in changed {
            if let Some(progress) = journal.files.get(&index) {
                let line = LogLine {
                    generation: journal.log_generation,
                    index,
                    progress: progress.clone(),
                };
                serde_json::to_writer(&mut buf, &line).map_err(io::Error::other)?;
                buf.push(b'\n');
                lines += 1;
            }
        }
        if lines == 0 {
            return Ok(0);
        }
        let mut options = fs::OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(self.log_path(&journal.transfer_id))?;
        file.write_all(&buf)?;
        file.sync_data()?;
        Ok(lines)
    }

    fn apply_log(&self, journal: &mut ReceiveJournal) {
        let Ok(file) = fs::File::open(self.log_path(&journal.transfer_id)) else {
            return;
        };
        for line in io::BufReader::new(file).split(b'\n') {
            // A line cut short by a crash ends the log.
            let Some(line) = line
                .ok()
                .and_then(|l| serde_json::from_slice::<LogLine>(&l).ok())
            else {
                break;
            };
            if line.generation == journal.log_generation {
                journal.files.insert(line.index, line.progress);
            }
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

fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(err) if err.kind() != io::ErrorKind::NotFound => Err(err),
        _ => Ok(()),
    }
}

#[derive(Serialize, Deserialize)]
struct LogLine {
    #[serde(rename = "g")]
    generation: u64,
    #[serde(rename = "i")]
    index: u32,
    #[serde(rename = "p")]
    progress: FileProgress,
}

/// A receive journal being updated: remembers which files changed so a
/// save writes only those.
#[derive(Debug)]
pub struct JournalWriter {
    records: RecordDir,
    journal: ReceiveJournal,
    changed: BTreeSet<u32>,
    /// Lines in the log since the last snapshot; `None` until this writer
    /// has written a snapshot of its own.
    logged: Option<usize>,
}

impl JournalWriter {
    pub fn new(records: RecordDir, journal: ReceiveJournal) -> Self {
        Self {
            records,
            journal,
            changed: BTreeSet::new(),
            logged: None,
        }
    }

    /// One file's progress, created if missing, marked as changed.
    pub fn file_mut(&mut self, index: u32) -> &mut FileProgress {
        self.changed.insert(index);
        self.journal.files.entry(index).or_default()
    }

    /// Replaces one file's progress.
    pub fn set_file(&mut self, index: u32, progress: FileProgress) {
        *self.file_mut(index) = progress;
    }

    /// Saves what changed: a log append, or a new snapshot when the log has
    /// grown as large as the journal (or this writer has none yet).
    pub fn save(&mut self) -> io::Result<()> {
        let fold_at = self.journal.files.len().max(1024);
        match self.logged {
            Some(logged) if logged + self.changed.len() <= fold_at => {
                let changed = std::mem::take(&mut self.changed);
                match self
                    .records
                    .append_log(&self.journal, changed.iter().copied())
                {
                    Ok(lines) => {
                        self.logged = Some(logged + lines);
                        Ok(())
                    }
                    Err(err) => {
                        // Keep them for the next try, as a snapshot: a
                        // partly written line ends the log anyway.
                        self.changed = changed;
                        self.logged = None;
                        Err(err)
                    }
                }
            }
            _ => {
                self.records.save_journal(&mut self.journal)?;
                self.changed.clear();
                self.logged = Some(0);
                Ok(())
            }
        }
    }

    pub fn into_journal(self) -> ReceiveJournal {
        self.journal
    }
}

impl std::ops::Deref for JournalWriter {
    type Target = ReceiveJournal;

    fn deref(&self) -> &ReceiveJournal {
        &self.journal
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
            log_generation: 0,
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

    fn journal_with(dir: &Path, files: u32) -> ReceiveJournal {
        ReceiveJournal {
            version: 1,
            transfer_id: [7; 16],
            peer_id: PeerId([1; 32]),
            peer_name: "Laptop".into(),
            manifest_digest: [2; 32],
            destination: dir.to_path_buf(),
            conflict: ConflictPolicy::KeepBoth,
            label: "x".into(),
            total_bytes: 0,
            created: 0,
            renames: BTreeMap::new(),
            files: (0..files).map(|i| (i, FileProgress::default())).collect(),
            log_generation: 0,
        }
    }

    #[test]
    fn saves_append_changes_and_fold_them_into_snapshots() {
        let dir = tempfile::tempdir().unwrap();
        let records = RecordDir::new(dir.path().join("incoming"));
        let mut journal = journal_with(dir.path(), 5000);
        records.save_journal(&mut journal).unwrap();
        let id = journal.transfer_id;
        let snapshot = dir
            .path()
            .join("incoming")
            .join(format!("{}.json", hex(&id)));
        let log = dir
            .path()
            .join("incoming")
            .join(format!("{}.log", hex(&id)));

        let mut writer = JournalWriter::new(records.clone(), journal);
        writer.file_mut(1).verified = 10;
        writer.save().unwrap(); // the writer's first save is a snapshot
        assert!(!log.exists());
        let before = fs::metadata(&snapshot).unwrap().modified().unwrap();

        // Later saves append only what changed.
        for round in 0..3u64 {
            writer.file_mut(2).verified = round;
            writer.file_mut(3).state = FileState::Partial;
            writer.save().unwrap();
        }
        assert_eq!(fs::read_to_string(&log).unwrap().lines().count(), 6);
        assert_eq!(fs::metadata(&snapshot).unwrap().modified().unwrap(), before);
        let back = records.load_journal(&id).unwrap().unwrap();
        assert_eq!(back, *writer);
        assert_eq!(back.files[&2].verified, 2);
        assert_eq!(records.list_journals(), vec![back]);

        // Once the log is as long as the journal, it becomes a snapshot.
        for i in 0..5000 {
            writer.file_mut(i).verified = 1;
        }
        writer.save().unwrap();
        assert!(!log.exists());
        assert_eq!(records.load_journal(&id).unwrap().unwrap(), *writer);

        records.remove(&id).unwrap();
        assert!(records.load_journal(&id).unwrap().is_none());
    }

    #[test]
    fn stale_and_torn_log_lines_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let records = RecordDir::new(dir.path().join("incoming"));
        let mut journal = journal_with(dir.path(), 4);
        records.save_journal(&mut journal).unwrap();
        let id = journal.transfer_id;
        let log = dir
            .path()
            .join("incoming")
            .join(format!("{}.log", hex(&id)));
        let mut writer = JournalWriter::new(records.clone(), journal);
        writer.save().unwrap();
        writer.file_mut(0).verified = 5;
        writer.save().unwrap();
        let old_log = fs::read(&log).unwrap();

        // A newer snapshot with the old log still beside it (a crash
        // between writing the one and removing the other).
        let mut newer = records.load_journal(&id).unwrap().unwrap();
        newer.files.get_mut(&0).unwrap().verified = 9;
        records.save_journal(&mut newer).unwrap();
        fs::write(&log, &old_log).unwrap();
        assert_eq!(
            records.load_journal(&id).unwrap().unwrap().files[&0].verified,
            9
        );

        // A line cut short ends the log; the lines before it count.
        let mut writer = JournalWriter::new(records.clone(), newer);
        writer.save().unwrap();
        writer.file_mut(1).verified = 3;
        writer.save().unwrap();
        let mut bytes = fs::read(&log).unwrap();
        bytes.extend_from_slice(br#"{"g":2,"i":2,"p":{"verif"#);
        fs::write(&log, &bytes).unwrap();
        let back = records.load_journal(&id).unwrap().unwrap();
        assert_eq!(back.files[&1].verified, 3);
        assert_eq!(back.files[&2].verified, 0);
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
