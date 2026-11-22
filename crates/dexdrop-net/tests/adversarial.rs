//! A misbehaving peer, played by hand over an in-memory stream (PRD 13.4).

use std::{collections::BTreeMap, path::Path, sync::Arc, time::Duration};

use dexdrop_core::{
    PeerId,
    journal::{ReceiveJournal, RecordDir},
    naming::ConflictPolicy,
    protocol::{
        self, ChunkHeader, EntryKind, FileFailure, ManifestPage, Message, Offer, RejectReason,
        WireEntry,
    },
};
use dexdrop_net::{
    session::{self, Control, Live, ReceivePlan, SessionEnd},
    wire,
};
use tokio::{io::AsyncWriteExt, sync::watch};

fn offer(entries: &[WireEntry]) -> Offer {
    Offer {
        transfer_id: [7; 16],
        entries: entries.len() as u64,
        files: entries.iter().filter(|e| e.kind == EntryKind::File).count() as u64,
        folders: entries
            .iter()
            .filter(|e| e.kind == EntryKind::Folder)
            .count() as u64,
        total_bytes: entries.iter().map(|e| e.size).sum(),
        chunk_size: protocol::MIN_CHUNK_SIZE,
        label: "x".into(),
        resume: false,
    }
}

fn file(path: &str, size: u64) -> WireEntry {
    WireEntry {
        path: path.into(),
        kind: EntryKind::File,
        size,
        mtime: None,
    }
}

async fn offered(
    entries: Vec<WireEntry>,
    tweak: impl FnOnce(&mut Offer),
) -> Result<session::ReceivedOffer, RejectReason> {
    let (mut a, mut b) = tokio::io::duplex(1 << 20);
    let mut o = offer(&entries);
    tweak(&mut o);
    session::send_offer(&mut a, &o, &entries).await.unwrap();
    session::read_offer(&mut b).await.unwrap()
}

#[tokio::test]
async fn rejects_traversal_absolute_and_inconsistent_manifests() {
    for bad in [
        "../escape.txt",
        "a/../../b",
        "/etc/passwd",
        "C:/Windows/x",
        "a//b",
        "",
    ] {
        let result = offered(vec![file(bad, 1)], |_| {}).await;
        assert_eq!(result.err(), Some(RejectReason::InvalidManifest), "{bad:?}");
    }
    // Totals that do not match the entries.
    let result = offered(vec![file("a", 10)], |o| o.total_bytes = 5).await;
    assert_eq!(result.err(), Some(RejectReason::InvalidManifest));
    let result = offered(vec![file("a", 10)], |o| o.files = 2).await;
    assert_eq!(result.err(), Some(RejectReason::InvalidManifest));
    // Chunk sizes out of range.
    let result = offered(vec![file("a", 10)], |o| o.chunk_size = 1).await;
    assert_eq!(result.err(), Some(RejectReason::InvalidManifest));
    // A folder claiming a size.
    let folder = WireEntry {
        path: "d".into(),
        kind: EntryKind::Folder,
        size: 9,
        mtime: None,
    };
    let result = offered(vec![folder], |o| o.files = 0).await;
    assert_eq!(result.err(), Some(RejectReason::InvalidManifest));
    // More entries than announced.
    let (mut a, mut b) = tokio::io::duplex(1 << 20);
    let o = offer(&[file("a", 1)]);
    wire::write_msg(&mut a, &Message::Offer(o)).await.unwrap();
    wire::write_msg(
        &mut a,
        &Message::ManifestPage(ManifestPage {
            entries: vec![file("a", 1), file("b", 1)],
            last: true,
        }),
    )
    .await
    .unwrap();
    a.flush().await.unwrap();
    assert_eq!(
        session::read_offer(&mut b).await.unwrap().err(),
        Some(RejectReason::InvalidManifest)
    );
}

#[tokio::test]
async fn refuses_oversized_frames_before_allocating() {
    let (mut a, mut b) = tokio::io::duplex(1024);
    a.write_all(&u32::MAX.to_be_bytes()).await.unwrap();
    let err = wire::read_msg(&mut b).await.unwrap_err();
    assert!(err.to_string().contains("exceeds the limit"), "{err}");
}

fn plan(dir: &std::path::Path, received: session::ReceivedOffer) -> ReceivePlan {
    let journal = ReceiveJournal {
        version: 1,
        transfer_id: received.offer.transfer_id,
        peer_id: PeerId([1; 32]),
        peer_name: "Mallory".into(),
        manifest_digest: [0; 32],
        destination: dir.join("dl"),
        conflict: ConflictPolicy::KeepBoth,
        label: "x".into(),
        total_bytes: received.offer.total_bytes,
        created: 0,
        renames: BTreeMap::new(),
        files: BTreeMap::new(),
    };
    ReceivePlan {
        offer: received,
        journal,
        records: RecordDir::new(dir.join("journal")),
    }
}

/// Plays a sender that sends `data` for file 0 with a wrong whole-file hash
/// `bad_rounds` times, then the right one.
async fn corrupt_sender(bad_rounds: usize) -> (SessionEnd, Live, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let data = vec![42u8; 100_000];
    let entries = vec![file("f.bin", data.len() as u64)];
    let received = offered(entries, |_| {}).await.unwrap();
    let (mut sender, receiver) = tokio::io::duplex(1 << 20);
    let live = Arc::new(std::sync::Mutex::new(Live::default()));
    let (_control_tx, control) = watch::channel(Control::Run);
    let session = tokio::spawn(session::run_receiver(
        receiver,
        plan(dir.path(), received),
        live.clone(),
        control,
    ));

    let mut rounds = 0;
    loop {
        match wire::read_msg_within(&mut sender, Duration::from_secs(5))
            .await
            .unwrap()
        {
            Message::Want {
                index: 0,
                offset: 0,
            } => {
                wire::write_msg(
                    &mut sender,
                    &Message::FileStart {
                        index: 0,
                        offset: 0,
                    },
                )
                .await
                .unwrap();
                for (i, piece) in data.chunks(protocol::MIN_CHUNK_SIZE as usize).enumerate() {
                    let header = ChunkHeader {
                        index: 0,
                        offset: (i * protocol::MIN_CHUNK_SIZE as usize) as u64,
                        len: piece.len() as u32,
                        hash: protocol::chunk_hash(piece),
                    };
                    wire::write_msg(&mut sender, &Message::Chunk(header))
                        .await
                        .unwrap();
                    sender.write_all(piece).await.unwrap();
                }
                let hash = if rounds < bad_rounds {
                    [0u8; 32]
                } else {
                    *blake3::hash(&data).as_bytes()
                };
                rounds += 1;
                wire::send(&mut sender, &Message::FileEnd { index: 0, hash })
                    .await
                    .unwrap();
            }
            Message::FileResult { .. } => {}
            Message::Complete => break,
            other => panic!("unexpected {other:?}"),
        }
    }
    drop(sender);
    let (end, _journal) = session.await.unwrap();
    let live = live.lock().unwrap().clone();
    (end.unwrap(), live, dir)
}

#[tokio::test]
async fn retries_a_file_that_fails_verification_once() {
    let (end, live, dir) = corrupt_sender(1).await;
    assert_eq!(end, SessionEnd::Completed);
    assert_eq!(live.files_done, 1);
    assert_eq!(
        std::fs::read(dir.path().join("dl/f.bin")).unwrap(),
        vec![42u8; 100_000]
    );
}

#[tokio::test]
async fn never_saves_data_that_keeps_failing_verification() {
    let (end, live, dir) = corrupt_sender(2).await;
    assert_eq!(end, SessionEnd::Completed);
    assert_eq!(live.files_failed, 1);
    assert_eq!(live.failures[0].1, FileFailure::IntegrityMismatch);
    assert!(!dir.path().join("dl/f.bin").exists());
    // No temporary file is left either.
    let left: Vec<_> = std::fs::read_dir(dir.path().join("dl"))
        .unwrap()
        .flatten()
        .collect();
    assert!(left.is_empty(), "{left:?}");
}

#[tokio::test]
async fn rejects_chunks_out_of_place() {
    let dir = tempfile::tempdir().unwrap();
    let received = offered(vec![file("f.bin", 10)], |_| {}).await.unwrap();
    let (mut sender, receiver) = tokio::io::duplex(1 << 20);
    let live = Arc::new(std::sync::Mutex::new(Live::default()));
    let (_control_tx, control) = watch::channel(Control::Run);
    let session = tokio::spawn(session::run_receiver(
        receiver,
        plan(dir.path(), received),
        live,
        control,
    ));
    assert!(matches!(
        wire::read_msg(&mut sender).await.unwrap(),
        Message::Want { .. }
    ));
    wire::write_msg(
        &mut sender,
        &Message::FileStart {
            index: 0,
            offset: 0,
        },
    )
    .await
    .unwrap();
    // Claims more bytes than the file has.
    let header = ChunkHeader {
        index: 0,
        offset: 0,
        len: 11,
        hash: [0; 32],
    };
    wire::send(&mut sender, &Message::Chunk(header))
        .await
        .unwrap();
    let (end, _) = session.await.unwrap();
    assert!(end.unwrap_err().to_string().contains("out of place"));
}

#[tokio::test]
async fn bounds_manifests_before_consent() {
    // A path over the limit is refused on the page it arrives in.
    let long = "a/".repeat(protocol::MAX_PATH_BYTES / 2) + "x";
    let (mut a, mut b) = tokio::io::duplex(1 << 20);
    let entries = vec![file(&long, 1)];
    let o = offer(&entries);
    wire::write_msg(&mut a, &Message::Offer(o)).await.unwrap();
    wire::write_msg(
        &mut a,
        &Message::ManifestPage(ManifestPage {
            entries,
            last: false,
        }),
    )
    .await
    .unwrap();
    a.flush().await.unwrap();
    assert_eq!(
        session::read_offer(&mut b).await.unwrap().err(),
        Some(RejectReason::InvalidManifest)
    );

    // Empty pages that are not the last cannot hold the connection open.
    let (mut a, mut b) = tokio::io::duplex(1 << 20);
    let o = offer(&[file("a", 1)]);
    wire::write_msg(&mut a, &Message::Offer(o)).await.unwrap();
    for _ in 0..3 {
        wire::write_msg(
            &mut a,
            &Message::ManifestPage(ManifestPage {
                entries: vec![],
                last: false,
            }),
        )
        .await
        .unwrap();
    }
    a.flush().await.unwrap();
    assert_eq!(
        session::read_offer(&mut b).await.unwrap().err(),
        Some(RejectReason::InvalidManifest)
    );

    // An oversized label is refused.
    let (mut a, mut b) = tokio::io::duplex(1 << 20);
    let mut o = offer(&[file("a", 1)]);
    o.label = "x".repeat(protocol::MAX_LABEL_BYTES + 1);
    wire::write_msg(&mut a, &Message::Offer(o)).await.unwrap();
    a.flush().await.unwrap();
    assert_eq!(
        session::read_offer(&mut b).await.unwrap().err(),
        Some(RejectReason::InvalidManifest)
    );
}

/// A receiving session fed by hand, after the planner's first request.
async fn receiving(
    entries: Vec<WireEntry>,
) -> (
    tokio::io::DuplexStream,
    tokio::task::JoinHandle<(Result<SessionEnd, session::SessionError>, ReceiveJournal)>,
    tempfile::TempDir,
) {
    let dir = tempfile::tempdir().unwrap();
    let received = offered(entries, |_| {}).await.unwrap();
    let (mut sender, receiver) = tokio::io::duplex(1 << 20);
    let live = Arc::new(std::sync::Mutex::new(Live::default()));
    let (control_tx, control) = watch::channel(Control::Run);
    std::mem::forget(control_tx);
    let session = tokio::spawn(session::run_receiver(
        receiver,
        plan(dir.path(), received),
        live,
        control,
    ));
    assert!(matches!(
        wire::read_msg(&mut sender).await.unwrap(),
        Message::Want { .. }
    ));
    (sender, session, dir)
}

#[tokio::test]
async fn file_errors_for_files_not_requested_are_refused_without_panicking() {
    for bad in [u32::MAX, 1] {
        // Index 1 exists but is a folder, which is never requested.
        let folder = WireEntry {
            path: "d".into(),
            kind: EntryKind::Folder,
            size: 0,
            mtime: None,
        };
        let (mut sender, session, _dir) = receiving(vec![file("a", 10), folder]).await;
        wire::send(
            &mut sender,
            &Message::FileError {
                index: bad,
                failure: FileFailure::SourceChanged,
            },
        )
        .await
        .unwrap();
        let (end, _) = session.await.expect("no panic");
        assert!(
            end.unwrap_err().to_string().contains("not requested"),
            "index {bad}"
        );
    }
}

#[tokio::test]
async fn senders_refuse_results_for_files_never_sent() {
    let src = tempfile::tempdir().unwrap();
    std::fs::write(src.path().join("a.txt"), b"hello").unwrap();
    let manifest =
        Arc::new(dexdrop_core::manifest::Manifest::build(&[src.path().join("a.txt")]).unwrap());
    let (sender_side, mut receiver_side) = tokio::io::duplex(1 << 20);
    let live = Arc::new(std::sync::Mutex::new(Live::default()));
    let (_control_tx, control) = watch::channel(Control::Run);
    let session = tokio::spawn(session::run_sender(
        sender_side,
        manifest,
        protocol::MIN_CHUNK_SIZE,
        live,
        control,
        None,
    ));
    // A result for a file the receiver never asked for, then a flood.
    for _ in 0..10_000 {
        if wire::write_msg(
            &mut receiver_side,
            &Message::FileResult {
                index: 0,
                failure: None,
            },
        )
        .await
        .is_err()
        {
            break;
        }
    }
    let _ = receiver_side.flush().await;
    let end = tokio::time::timeout(Duration::from_secs(5), session)
        .await
        .expect("ends")
        .unwrap();
    assert!(end.unwrap_err().to_string().contains("not sent"));
}

/// Sends one small file through a hand-driven session.
async fn send_one(sender: &mut tokio::io::DuplexStream, data: &[u8]) {
    wire::write_msg(
        sender,
        &Message::FileStart {
            index: 0,
            offset: 0,
        },
    )
    .await
    .unwrap();
    for (i, piece) in data.chunks(protocol::MIN_CHUNK_SIZE as usize).enumerate() {
        let header = ChunkHeader {
            index: 0,
            offset: (i * protocol::MIN_CHUNK_SIZE as usize) as u64,
            len: piece.len() as u32,
            hash: protocol::chunk_hash(piece),
        };
        wire::write_msg(sender, &Message::Chunk(header))
            .await
            .unwrap();
        sender.write_all(piece).await.unwrap();
    }
    wire::send(
        sender,
        &Message::FileEnd {
            index: 0,
            hash: *blake3::hash(data).as_bytes(),
        },
    )
    .await
    .unwrap();
}
#[cfg(unix)]
#[tokio::test]
async fn a_planted_link_at_the_temp_path_is_never_followed() {
    for size in [16usize, 300_000] {
        planted_link(size).await;
    }
}

#[cfg(unix)]
async fn planted_link(size: usize) {
    let outside = tempfile::tempdir().unwrap();
    let victim = outside.path().join("victim.txt");
    std::fs::write(&victim, b"precious").unwrap();

    let data: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
    let entries = vec![file("a.txt", data.len() as u64)];
    let received = offered(entries, |_| {}).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("dl");
    std::fs::create_dir_all(&dest).unwrap();
    // A local attacker predicts the temporary name and plants a link there.
    let temp =
        dexdrop_core::journal::partial_path(&dest.join("a.txt"), &received.offer.transfer_id, 0);
    std::os::unix::fs::symlink(&victim, &temp).unwrap();

    let (mut sender, receiver) = tokio::io::duplex(1 << 20);
    let live = Arc::new(std::sync::Mutex::new(Live::default()));
    let (_control_tx, control) = watch::channel(Control::Run);
    let session = tokio::spawn(session::run_receiver(
        receiver,
        plan(dir.path(), received),
        live,
        control,
    ));
    assert!(matches!(
        wire::read_msg(&mut sender).await.unwrap(),
        Message::Want { .. }
    ));
    send_one(&mut sender, &data).await;
    loop {
        match wire::read_msg_within(&mut sender, Duration::from_secs(5))
            .await
            .unwrap()
        {
            Message::Complete => break,
            Message::FileResult { failure, .. } => assert_eq!(failure, None),
            other => panic!("unexpected {other:?}"),
        }
    }
    drop(sender);
    let (end, _) = session.await.unwrap();
    assert_eq!(end.unwrap(), SessionEnd::Completed);
    assert_eq!(
        std::fs::read(&victim).unwrap(),
        b"precious",
        "the link target was written"
    );
    let saved = dest.join("a.txt");
    assert!(
        !std::fs::symlink_metadata(&saved)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(std::fs::read(&saved).unwrap(), data);
}

#[cfg(unix)]
#[test]
fn symlinks_are_found_again_right_before_writing() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("Project")).unwrap();
    let target = dir.path().join("Project/src/.main.rs.part");
    assert!(dexdrop_core::pathsafe::ensure_no_symlinks(dir.path(), &target).is_ok());
    // A folder replaced by a link after planning is caught.
    std::os::unix::fs::symlink(outside.path(), dir.path().join("Project/src")).unwrap();
    assert!(dexdrop_core::pathsafe::ensure_no_symlinks(dir.path(), &target).is_err());
    assert!(
        dexdrop_core::pathsafe::ensure_no_symlinks(dir.path(), Path::new("/etc/passwd")).is_err()
    );
}
