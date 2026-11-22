//! How long a receive journal save takes: `journal_cost [files]`.
use dexdrop_core::{PeerId, journal::*, naming::ConflictPolicy};
use std::{collections::BTreeMap, time::Instant};

fn main() {
    let n: u32 = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(200_000);
    let dir = tempfile::tempdir().unwrap();
    let mut journal = ReceiveJournal {
        version: 1,
        transfer_id: [1; 16],
        peer_id: PeerId([2; 32]),
        peer_name: "x".into(),
        manifest_digest: [3; 32],
        destination: dir.path().into(),
        conflict: ConflictPolicy::KeepBoth,
        label: "x".into(),
        total_bytes: 0,
        created: 0,
        renames: BTreeMap::new(),
        files: BTreeMap::new(),
        log_generation: 0,
    };
    for i in 0..n {
        journal.files.insert(
            i,
            FileProgress {
                verified: 12345,
                state: FileState::Done,
                temp: None,
                saved: Some(dir.path().join(format!("Photos/2024/Holiday/IMG_{i:06}.jpg"))),
            },
        );
    }
    let records = RecordDir::new(dir.path().join("incoming"));
    let mut writer = JournalWriter::new(records.clone(), journal);
    let t = Instant::now();
    writer.save().unwrap();
    println!("{n} files: snapshot {:?}", t.elapsed());
    // A typical 2-second interval: a few thousand small files finished.
    let t = Instant::now();
    for i in 0..3000 {
        writer.file_mut(i).verified = 1;
    }
    writer.save().unwrap();
    println!("{n} files: save of 3,000 changes {:?}", t.elapsed());
    let t = Instant::now();
    let back = records.load_journal(&[1; 16]).unwrap().unwrap();
    println!("{n} files: load {:?}", t.elapsed());
    assert_eq!(back, *writer);
}
