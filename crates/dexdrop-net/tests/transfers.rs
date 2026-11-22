//! End-to-end transfers between two engines over real TLS on loopback.

mod common;

use common::{Node, write_file};
use dexdrop_core::{journal::ReceiveJournal, naming::ConflictPolicy, store::Direction};
use dexdrop_net::{Command, Event, Target, TransferState};

fn accept(id: [u8; 16]) -> Command {
    Command::Respond {
        id,
        accept: true,
        trust: false,
        conflict: ConflictPolicy::KeepBoth,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sends_files_and_folders_with_consent() {
    let mut a = Node::start("Alice").await;
    let mut b = Node::start("Bob").await;
    let src = tempfile::tempdir().unwrap();
    write_file(&src.path().join("photo.jpg"), 300_000, 1);
    write_file(&src.path().join("Project/src/main.rs"), 1234, 2);
    write_file(&src.path().join("Project/README.md"), 0, 3);
    std::fs::create_dir_all(src.path().join("Project/empty")).unwrap();

    a.send(Command::SendFiles {
        target: b.target(),
        paths: vec![src.path().join("photo.jpg"), src.path().join("Project")],
    });

    let request = b.wait_request().await;
    assert_eq!(request.peer_name, "Alice");
    assert_eq!(request.peer_id, a.status.peer_id);
    assert_eq!(request.files, 3);
    assert_eq!(request.folders, 3);
    assert_eq!(request.total_bytes, 301_234);
    assert_eq!(request.sample, vec!["photo.jpg", "Project"]);
    // Both screens show the same verification number.
    let sender_view = a
        .wait("the sender's SAS", |e| match e {
            Event::Transfer(t) if t.sas.is_some() => t.sas.clone(),
            _ => None,
        })
        .await;
    assert_eq!(sender_view, request.sas);

    b.send(accept(request.id));
    let done = b
        .wait_state("receiver completed", |s| *s == TransferState::Completed)
        .await;
    assert_eq!(done.direction, Direction::Incoming);
    assert_eq!(done.bytes_done, 301_234);
    // The saved files are listed for sharing/exporting (FR-042).
    let mut saved: Vec<_> = done
        .saved_files
        .iter()
        .map(|p| p.strip_prefix(b.downloads()).unwrap().to_owned())
        .collect();
    saved.sort();
    assert_eq!(
        saved,
        vec![
            std::path::PathBuf::from("Project/README.md"),
            std::path::PathBuf::from("Project/src/main.rs"),
            std::path::PathBuf::from("photo.jpg"),
        ]
    );
    let sent = a
        .wait_state("sender completed", |s| *s == TransferState::Completed)
        .await;
    assert_eq!(sent.files_done, 3);

    let dl = b.downloads();
    assert_eq!(
        std::fs::read(dl.join("photo.jpg")).unwrap(),
        std::fs::read(src.path().join("photo.jpg")).unwrap()
    );
    assert_eq!(
        std::fs::read(dl.join("Project/src/main.rs")).unwrap(),
        std::fs::read(src.path().join("Project/src/main.rs")).unwrap()
    );
    assert_eq!(
        std::fs::read(dl.join("Project/README.md")).unwrap().len(),
        0
    );
    assert!(dl.join("Project/empty").is_dir());
    // No temporary files are left behind.
    let leftovers: Vec<_> = walk(&dl)
        .into_iter()
        .filter(|p| p.to_string_lossy().ends_with(".dexdrop-part"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn declining_tells_the_sender() {
    let mut a = Node::start("Alice").await;
    let mut b = Node::start("Bob").await;
    let src = tempfile::tempdir().unwrap();
    write_file(&src.path().join("a.txt"), 10, 0);
    a.send(Command::SendFiles {
        target: b.target(),
        paths: vec![src.path().join("a.txt")],
    });
    let request = b.wait_request().await;
    b.send(Command::Respond {
        id: request.id,
        accept: false,
        trust: false,
        conflict: ConflictPolicy::KeepBoth,
    });
    let t = a
        .wait_state("declined", |s| matches!(s, TransferState::Declined(_)))
        .await;
    assert_eq!(t.state.label(), "The other device declined the transfer.");
    assert!(!b.downloads().join("a.txt").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn never_overwrites_silently() {
    let a = Node::start("Alice").await;
    let mut b = Node::start("Bob").await;
    let src = tempfile::tempdir().unwrap();
    write_file(&src.path().join("report.pdf"), 1000, 7);
    std::fs::create_dir_all(b.downloads()).unwrap();
    std::fs::write(b.downloads().join("report.pdf"), b"existing").unwrap();
    std::fs::create_dir_all(b.downloads().join("Folder")).unwrap();
    std::fs::write(b.downloads().join("Folder/keep.txt"), b"mine").unwrap();
    write_file(&src.path().join("Folder/keep.txt"), 50, 9);

    a.send(Command::SendFiles {
        target: b.target(),
        paths: vec![src.path().join("report.pdf"), src.path().join("Folder")],
    });
    let request = b.wait_request().await;
    b.send(accept(request.id));
    b.wait_state("completed", |s| *s == TransferState::Completed)
        .await;
    let dl = b.downloads();
    assert_eq!(std::fs::read(dl.join("report.pdf")).unwrap(), b"existing");
    assert_eq!(
        std::fs::read(dl.join("report (1).pdf")).unwrap().len(),
        1000
    );
    assert_eq!(std::fs::read(dl.join("Folder/keep.txt")).unwrap(), b"mine");
    assert_eq!(
        std::fs::read(dl.join("Folder (1)/keep.txt")).unwrap().len(),
        50
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn skip_policy_keeps_existing_files() {
    let a = Node::start("Alice").await;
    let mut b = Node::start("Bob").await;
    let src = tempfile::tempdir().unwrap();
    write_file(&src.path().join("a.txt"), 100, 1);
    write_file(&src.path().join("b.txt"), 100, 2);
    std::fs::create_dir_all(b.downloads()).unwrap();
    std::fs::write(b.downloads().join("a.txt"), b"old").unwrap();
    a.send(Command::SendFiles {
        target: b.target(),
        paths: vec![src.path().join("a.txt"), src.path().join("b.txt")],
    });
    let request = b.wait_request().await;
    b.send(Command::Respond {
        id: request.id,
        accept: true,
        trust: false,
        conflict: ConflictPolicy::Skip,
    });
    let t = b
        .wait_state("completed", |s| *s == TransferState::CompletedWithIssues)
        .await;
    assert_eq!(t.files_skipped, 1);
    assert_eq!(std::fs::read(b.downloads().join("a.txt")).unwrap(), b"old");
    assert_eq!(
        std::fs::read(b.downloads().join("b.txt")).unwrap().len(),
        100
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pauses_and_resumes_without_starting_over() {
    // Small chunks so there are many chunks to pause between.
    let mut a = Node::start_with("Alice", |c| {
        c.chunk_size = 64 * 1024;
        c.max_send_rate = Some(24_000_000);
    })
    .await;
    let mut b = Node::start("Bob").await;
    let src = tempfile::tempdir().unwrap();
    let size = 48 * 1024 * 1024;
    write_file(&src.path().join("big.bin"), size, 5);
    a.send(Command::SendFiles {
        target: b.target(),
        paths: vec![src.path().join("big.bin")],
    });
    let request = b.wait_request().await;
    let id = request.id;
    b.send(accept(id));
    a.wait("some progress", |e| match e {
        Event::Transfer(t) if t.bytes_done > 4 * 1024 * 1024 => Some(()),
        _ => None,
    })
    .await;
    a.send(Command::Pause(id));
    a.wait_state("sender paused", |s| {
        *s == TransferState::Paused { by_peer: false }
    })
    .await;
    let paused = b
        .wait_state("receiver paused", |s| {
            matches!(s, TransferState::Paused { by_peer: true })
        })
        .await;
    assert!(paused.bytes_done > 0);

    // The receiver journaled verified progress.
    let journal: ReceiveJournal = dexdrop_core::journal::RecordDir::new(b.state().join("incoming"))
        .load(&id)
        .unwrap()
        .expect("a journal while paused");
    let verified = journal.verified_bytes();
    assert!(
        verified > 0 && verified < size as u64,
        "verified {verified}"
    );

    a.send(Command::Resume(id));
    // The resumed session starts from the verified offset, not from zero:
    // no progress report after resuming is below it.
    b.wait_state("receiver transferring again", |s| {
        *s == TransferState::Transferring
    })
    .await;
    let mut lowest = u64::MAX;
    let end = b
        .wait("receiver finished", |e| match e {
            Event::Transfer(t) if t.id == id => {
                if t.state == TransferState::Transferring && t.bytes_done > 0 {
                    lowest = lowest.min(t.bytes_done);
                }
                (t.state.is_finished() || matches!(t.state, TransferState::Interrupted { .. }))
                    .then(|| t.clone())
            }
            _ => None,
        })
        .await;
    assert_eq!(end.state, TransferState::Completed, "{end:?}");
    // The measured average reflects the 24 MB/s cap, not the paused time.
    let average = end.average_rate.expect("a measured average");
    assert!((10e6..30e6).contains(&average), "average {average}");
    assert!(
        lowest == u64::MAX || lowest >= verified,
        "resumed at {lowest}, verified {verified}"
    );
    a.wait_state("sender completed", |s| *s == TransferState::Completed)
        .await;
    assert_eq!(
        std::fs::read(b.downloads().join("big.bin")).unwrap(),
        std::fs::read(src.path().join("big.bin")).unwrap()
    );
    assert!(
        dexdrop_core::journal::RecordDir::new(b.state().join("incoming"))
            .load::<ReceiveJournal>(&id)
            .unwrap()
            .is_none()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelling_removes_partial_files() {
    let mut a = Node::start_with("Alice", |c| {
        c.chunk_size = 64 * 1024;
        c.max_send_rate = Some(24_000_000);
    })
    .await;
    let mut b = Node::start("Bob").await;
    let src = tempfile::tempdir().unwrap();
    write_file(&src.path().join("big.bin"), 32 * 1024 * 1024, 5);
    a.send(Command::SendFiles {
        target: b.target(),
        paths: vec![src.path().join("big.bin")],
    });
    let request = b.wait_request().await;
    b.send(accept(request.id));
    b.wait("some progress", |e| match e {
        Event::Transfer(t) if t.bytes_done > 1024 * 1024 => Some(()),
        _ => None,
    })
    .await;
    b.send(Command::Cancel(request.id));
    b.wait_state("receiver cancelled", |s| {
        *s == TransferState::Cancelled { by_peer: false }
    })
    .await;
    a.wait_state("sender told", |s| {
        *s == TransferState::Cancelled { by_peer: true }
    })
    .await;
    let leftovers = walk(&b.downloads());
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn wrong_peer_is_refused() {
    let mut a = Node::start("Alice").await;
    let b = Node::start("Bob").await;
    let c = Node::start("Carol").await;
    let src = tempfile::tempdir().unwrap();
    write_file(&src.path().join("a.txt"), 10, 0);
    // Bob's address, but Carol's identity: someone else answered.
    a.send(Command::SendFiles {
        target: Target::Address {
            addr: b.addr(),
            peer_id: Some(c.status.peer_id),
        },
        paths: vec![src.path().join("a.txt")],
    });
    let t = a
        .wait_state("failed", |s| matches!(s, TransferState::Failed(_)))
        .await;
    assert!(t.state.label().contains("not the one expected"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn trusted_devices_can_auto_accept() {
    let mut a = Node::start("Alice").await;
    let mut b = Node::start("Bob").await;
    let src = tempfile::tempdir().unwrap();
    write_file(&src.path().join("1.txt"), 10, 0);
    write_file(&src.path().join("2.txt"), 10, 1);
    a.send(Command::SendFiles {
        target: b.target(),
        paths: vec![src.path().join("1.txt")],
    });
    let request = b.wait_request().await;
    // Accept and trust, then allow auto-accept for this device.
    b.send(Command::Respond {
        id: request.id,
        accept: true,
        trust: true,
        conflict: ConflictPolicy::KeepBoth,
    });
    b.wait_state("first completed", |s| *s == TransferState::Completed)
        .await;
    b.send(Command::SetPeerAutoAccept {
        peer_id: a.status.peer_id,
        auto_accept: true,
    });
    b.send(Command::SetAutoAcceptTrusted(true));
    a.send(Command::SendFiles {
        target: b.target(),
        paths: vec![src.path().join("2.txt")],
    });
    a.wait("second sent without a prompt", |e| match e {
        Event::Transfer(t) if t.label == "2.txt" && t.state == TransferState::Completed => Some(()),
        Event::Transfer(t) if t.label == "2.txt" && t.state.is_finished() => panic!("{t:?}"),
        _ => None,
    })
    .await;
    assert!(b.downloads().join("2.txt").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn invitations_are_single_use() {
    let mut a = Node::start("Alice").await;
    let mut b = Node::start("Bob").await;
    let src = tempfile::tempdir().unwrap();
    write_file(&src.path().join("a.txt"), 10, 0);
    // An invitation pointing at loopback (the engine would list LAN addresses).
    let invitation = {
        let mut book = dexdrop_core::invitation::InvitationBook::default();
        book.create(b.status.peer_id, "Bob", vec![b.addr()], 60)
            .encode()
    };
    // Bob never issued this token: the invitation is refused.
    a.send(Command::SendFiles {
        target: Target::Invitation(invitation),
        paths: vec![src.path().join("a.txt")],
    });
    let t = a
        .wait_state("refused", |s| matches!(s, TransferState::Declined(_)))
        .await;
    assert!(t.state.label().contains("not valid"));
    // An expired or garbled invitation is rejected before connecting.
    a.send(Command::SendFiles {
        target: Target::Invitation("DEXDROP1:ZZZZ".into()),
        paths: vec![src.path().join("a.txt")],
    });
    a.wait("an error notice", |e| {
        matches!(e, Event::Notice { .. }).then_some(())
    })
    .await;
    let _ = b.events.try_recv();
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if let Ok(read) = std::fs::read_dir(dir) {
        for entry in read.flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(walk(&path));
            } else {
                out.push(path);
            }
        }
    }
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_used_code_is_reported_and_closed() {
    use dexdrop_core::protocol::{Hello, Message, Pairing};
    let mut b = Node::start("Bob").await;
    b.send(Command::CreateCode);
    let code = b
        .wait("a code", |e| match e {
            Event::Code { code, .. } => Some(code.clone()),
            _ => None,
        })
        .await;
    // Connect as a sender presenting the code (the handshake only).
    let identity = dexdrop_core::Identity::generate().unwrap();
    let tls = dexdrop_net::tls::TlsContext::new(&identity).unwrap();
    let tcp = tokio::net::TcpStream::connect(b.addr()).await.unwrap();
    let (mut stream, _) = dexdrop_net::tls::connect(&tls, tcp).await.unwrap();
    let hello = Hello::new("Mallory", "linux", Some(Pairing::Code(code.clone())));
    dexdrop_net::wire::send(&mut stream, &Message::Hello(hello.clone()))
        .await
        .unwrap();
    assert!(matches!(
        dexdrop_net::wire::read_msg(&mut stream).await.unwrap(),
        Message::Hello(_)
    ));
    let (code_used, by) = b
        .wait("the code to be reported used", |e| match e {
            Event::PairingUsed { code, by } => Some((*code, by.clone())),
            _ => None,
        })
        .await;
    assert!(code_used);
    assert_eq!(by, "Mallory");
    // The same code does not work twice.
    let tcp = tokio::net::TcpStream::connect(b.addr()).await.unwrap();
    let (mut again, _) = dexdrop_net::tls::connect(&tls, tcp).await.unwrap();
    dexdrop_net::wire::send(&mut again, &Message::Hello(hello))
        .await
        .unwrap();
    assert_eq!(
        dexdrop_net::wire::read_msg(&mut again).await.unwrap(),
        Message::Reject(dexdrop_core::protocol::RejectReason::InvitationInvalid)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn many_small_files_arrive_intact() {
    let mut a = Node::start("Alice").await;
    let mut b = Node::start("Bob").await;
    let src = tempfile::tempdir().unwrap();
    for i in 0..500usize {
        write_file(
            &src.path()
                .join(format!("Photos/{:02}/img{i:04}.jpg", i % 10)),
            1000 + i * 37,
            i as u8,
        );
    }
    a.send(Command::SendFiles {
        target: b.target(),
        paths: vec![src.path().join("Photos")],
    });
    let request = b.wait_request().await;
    assert_eq!(request.files, 500);
    b.send(accept(request.id));
    let done = b.wait_state("completed", |s| s.is_finished()).await;
    assert_eq!(done.state, TransferState::Completed, "{done:?}");
    assert_eq!(done.files_done, 500);
    let sent = a.wait_state("sender completed", |s| s.is_finished()).await;
    assert_eq!(sent.state, TransferState::Completed);
    assert_eq!(sent.files_done, 500);
    for i in 0..500usize {
        let rel = format!("Photos/{:02}/img{i:04}.jpg", i % 10);
        assert_eq!(
            std::fs::read(b.downloads().join(&rel)).unwrap(),
            std::fs::read(src.path().join(&rel)).unwrap(),
            "{rel}"
        );
    }
    let leftovers: Vec<_> = walk(&b.downloads())
        .into_iter()
        .filter(|p| p.to_string_lossy().ends_with(".dexdrop-part"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_source_changed_mid_transfer_is_never_saved() {
    let mut a = Node::start_with("Alice", |c| {
        c.chunk_size = 64 * 1024;
        c.max_send_rate = Some(4_000_000);
    })
    .await;
    let mut b = Node::start("Bob").await;
    let src = tempfile::tempdir().unwrap();
    write_file(&src.path().join("changing.bin"), 24 * 1024 * 1024, 1);
    write_file(&src.path().join("steady.txt"), 5_000, 2);
    a.send(Command::SendFiles {
        target: b.target(),
        paths: vec![
            src.path().join("changing.bin"),
            src.path().join("steady.txt"),
        ],
    });
    let request = b.wait_request().await;
    b.send(accept(request.id));
    b.wait("some progress", |e| match e {
        // Early, with seconds of sending left at 4 MB/s.
        Event::Transfer(t) if t.bytes_done > 1024 * 1024 => Some(()),
        _ => None,
    })
    .await;
    // Someone edits the file while it is being sent.
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(src.path().join("changing.bin"))
            .unwrap();
        f.write_all(b"edited").unwrap();
    }
    let done = b.wait_state("receiver finished", |s| s.is_finished()).await;
    assert_eq!(done.state, TransferState::CompletedWithIssues, "{done:?}");
    assert_eq!(done.files_failed, 1);
    assert!(
        done.failures[0].1.contains("changed"),
        "{:?}",
        done.failures
    );
    // The changed file is not saved, half-written or otherwise; the other is.
    assert!(!b.downloads().join("changing.bin").exists());
    assert_eq!(
        std::fs::read(b.downloads().join("steady.txt"))
            .unwrap()
            .len(),
        5_000
    );
    let leftovers: Vec<_> = walk(&b.downloads())
        .into_iter()
        .filter(|p| p.to_string_lossy().ends_with(".dexdrop-part"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    let sent = a.wait_state("sender finished", |s| s.is_finished()).await;
    assert_eq!(sent.state, TransferState::CompletedWithIssues);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_source_deleted_before_sending_fails_only_that_file() {
    let mut a = Node::start("Alice").await;
    let mut b = Node::start("Bob").await;
    let src = tempfile::tempdir().unwrap();
    write_file(&src.path().join("gone.txt"), 100, 1);
    write_file(&src.path().join("kept.txt"), 100, 2);
    a.send(Command::SendFiles {
        target: b.target(),
        paths: vec![src.path().join("gone.txt"), src.path().join("kept.txt")],
    });
    let request = b.wait_request().await;
    // Deleted after offering, before the receiver accepted.
    std::fs::remove_file(src.path().join("gone.txt")).unwrap();
    b.send(accept(request.id));
    let done = b.wait_state("receiver finished", |s| s.is_finished()).await;
    assert_eq!(done.state, TransferState::CompletedWithIssues, "{done:?}");
    assert_eq!((done.files_done, done.files_failed), (1, 1));
    assert!(!b.downloads().join("gone.txt").exists());
    assert!(b.downloads().join("kept.txt").exists());
    // The sender reports the same outcome.
    let sent = a.wait_state("sender finished", |s| s.is_finished()).await;
    assert_eq!(sent.state, TransferState::CompletedWithIssues);
    assert_eq!((sent.files_done, sent.files_failed), (1, 1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn resumes_on_its_own_after_the_connection_drops() {
    let mut a = Node::start_with("Alice", |c| {
        c.chunk_size = 64 * 1024;
        c.max_send_rate = Some(16_000_000);
    })
    .await;
    let mut b = Node::start("Bob").await;
    // Everything goes through a link that drops once, 12 MB in.
    let proxy = common::CuttingProxy::start(b.addr(), 12 * 1024 * 1024).await;
    let src = tempfile::tempdir().unwrap();
    write_file(&src.path().join("video.mp4"), 40 * 1024 * 1024, 9);
    a.send(Command::SendFiles {
        target: Target::Address {
            addr: proxy.addr,
            peer_id: Some(b.status.peer_id),
        },
        paths: vec![src.path().join("video.mp4")],
    });
    let request = b.wait_request().await;
    let id = request.id;
    b.send(accept(id));

    // The sender notices and retries without anyone pressing anything.
    a.wait_state("sender interrupted and retrying", |s| {
        matches!(s, TransferState::Interrupted { retrying: true, .. })
    })
    .await;
    assert_eq!(proxy.cuts.load(std::sync::atomic::Ordering::SeqCst), 1);
    // The receiver keeps what it verified and waits for the sender.
    b.wait_state("receiver interrupted", |s| {
        matches!(s, TransferState::Interrupted { .. })
    })
    .await;
    let journal: ReceiveJournal = dexdrop_core::journal::RecordDir::new(b.state().join("incoming"))
        .load(&id)
        .unwrap()
        .expect("a journal after the drop");
    let verified = journal.verified_bytes();
    assert!(verified > 0, "nothing was journaled before the drop");

    let sent = a.wait_state("sender finished", |s| s.is_finished()).await;
    assert_eq!(sent.state, TransferState::Completed, "{sent:?}");
    let got = b.wait_state("receiver finished", |s| s.is_finished()).await;
    assert_eq!(got.state, TransferState::Completed, "{got:?}");
    assert_eq!(
        std::fs::read(b.downloads().join("video.mp4")).unwrap(),
        std::fs::read(src.path().join("video.mp4")).unwrap()
    );
    // The resumed session did not start over: the link carried the file
    // about once, not the part before the drop plus the whole file again.
    let size = 40 * 1024 * 1024u64;
    let moved = proxy.moved.load(std::sync::atomic::Ordering::SeqCst);
    assert!(
        moved < size + size / 8,
        "{moved} bytes crossed the link for a {size}-byte file"
    );
}

/// A small RAM disk for storage-full tests; `None` where mounting is not
/// allowed (not root, containers without the capability), and the test
/// is then skipped.
struct TinyDisk(std::path::PathBuf, #[allow(dead_code)] tempfile::TempDir);

impl TinyDisk {
    fn mount(megabytes: u32) -> Option<TinyDisk> {
        let dir = tempfile::tempdir().ok()?;
        let ok = std::process::Command::new("mount")
            .args(["-t", "tmpfs", "-o", &format!("size={megabytes}m"), "tmpfs"])
            .arg(dir.path())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        ok.then(|| TinyDisk(dir.path().to_owned(), dir))
    }
}

impl Drop for TinyDisk {
    fn drop(&mut self) {
        let _ = std::process::Command::new("umount").arg(&self.0).status();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn running_out_of_space_stops_cleanly_and_keeps_progress() {
    let Some(disk) = TinyDisk::mount(16) else {
        eprintln!("skipped: cannot mount a tmpfs here");
        return;
    };
    let downloads = disk.0.join("Downloads");
    let mut a = Node::start_with("Alice", |c| {
        c.chunk_size = 64 * 1024;
        c.max_send_rate = Some(6_000_000);
    })
    .await;
    let mut b = Node::start_with("Bob", |c| c.default_download_dir = downloads.clone()).await;
    let src = tempfile::tempdir().unwrap();
    write_file(&src.path().join("big.bin"), 12 * 1024 * 1024, 3);
    a.send(Command::SendFiles {
        target: b.target(),
        paths: vec![src.path().join("big.bin")],
    });
    let request = b.wait_request().await;
    assert!(request.enough_space(), "12 MB fits a 16 MB disk at first");
    b.send(accept(request.id));
    b.wait("some progress", |e| match e {
        Event::Transfer(t) if t.bytes_done > 1024 * 1024 => Some(()),
        _ => None,
    })
    .await;
    // Something else fills the disk meanwhile.
    let _ = std::fs::write(disk.0.join("filler"), vec![0u8; 12 * 1024 * 1024]);

    let stopped = b
        .wait_state("receiver stopped", |s| {
            s.is_finished()
                || matches!(
                    s,
                    TransferState::Interrupted { .. } | TransferState::Paused { .. }
                )
        })
        .await;
    assert!(
        matches!(&stopped.state, TransferState::Interrupted { reason, retrying: false } if reason.contains("ran out of storage space")),
        "{:?}",
        stopped.state
    );
    assert!(
        !downloads.join("big.bin").exists(),
        "a partial file must never look complete"
    );
    // The sender stops too, instead of reconnecting again and again.
    let sender = a
        .wait_state("sender stopped", |s| {
            s.is_finished()
                || matches!(
                    s,
                    TransferState::Interrupted {
                        retrying: false,
                        ..
                    }
                )
        })
        .await;
    assert!(
        matches!(&sender.state, TransferState::Interrupted { reason, .. } if reason.contains("other device ran out of storage")),
        "{:?}",
        sender.state
    );
    assert!(sender.can_resume());
    // Verified progress is kept for when there is room again.
    let journal: ReceiveJournal = dexdrop_core::journal::RecordDir::new(b.state().join("incoming"))
        .load(&request.id)
        .unwrap()
        .expect("the journal is kept");
    assert!(journal.verified_bytes() > 0);

    // Free the space and resume: the file completes, intact.
    std::fs::remove_file(disk.0.join("filler")).unwrap();
    a.send(Command::Resume(request.id));
    let done = b
        .wait_state("receiver completed", |s| s.is_finished())
        .await;
    assert_eq!(done.state, TransferState::Completed, "{done:?}");
    assert_eq!(
        std::fs::read(downloads.join("big.bin")).unwrap(),
        std::fs::read(src.path().join("big.bin")).unwrap()
    );
}
