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

    // Diagnostics (PRD 9.9, 12): JSON with where the key is, and never the
    // key itself, a path or a file name.
    b.send(Command::ExportDiagnostics);
    let report = b
        .wait("diagnostics", |e| match e {
            Event::Diagnostics(report) => Some(report.clone()),
            _ => None,
        })
        .await;
    let json: serde_json::Value = serde_json::from_str(&report).expect("diagnostics are JSON");
    assert_eq!(json["identity_key"], "app-private file");
    assert_eq!(json["recent"][0]["outcome"], "completed");
    let stored: serde_json::Value =
        serde_json::from_slice(&std::fs::read(b.state().join("identity.json")).unwrap()).unwrap();
    let key = stored["key_pkcs8"].as_str().unwrap();
    for secret in [
        key,
        &key[..16],
        &b.dir.path().to_string_lossy(),
        "photo.jpg",
        "main.rs",
    ] {
        assert!(!report.contains(secret), "diagnostics contain {secret:?}");
    }
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
        .load_journal(&id)
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
        lowest != u64::MAX && lowest >= verified,
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
            .load_journal(&id)
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
    // Both settings are applied (in order) before the second offer arrives.
    b.wait("auto-accept on", |e| match e {
        Event::Settings(s) if s.auto_accept_trusted => Some(()),
        _ => None,
    })
    .await;
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
    use dexdrop_core::protocol::{Hello, Message, Pairing, RejectReason};
    use dexdrop_net::{pairing, tls, wire};
    let mut b = Node::start("Bob").await;
    b.send(Command::CreateCode);
    let code = b
        .wait("a code", |e| match e {
            Event::Code { code, .. } => Some(code.clone()),
            _ => None,
        })
        .await;
    let identity = dexdrop_core::Identity::generate().unwrap();
    let ctx = tls::TlsContext::new(&identity).unwrap();

    // Runs the sender's half of code pairing with `guess`; returns what the
    // receiver answered after the sender's proof.
    let addr = b.addr();
    let attempt = |guess: String| {
        let ctx = ctx.clone();
        async move {
            let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
            let (mut stream, session) = tls::connect(&ctx, tcp).await.unwrap();
            let (exchange, msg) = pairing::start(&guess);
            let hello = Hello::new("Mallory", "linux", Some(Pairing::Pake(msg)));
            wire::send(&mut stream, &Message::Hello(hello))
                .await
                .unwrap();
            let reply = wire::read_msg(&mut stream).await.unwrap();
            let Message::Hello(Hello {
                pairing: Some(Pairing::PakeReply { msg, confirm }),
                ..
            }) = reply
            else {
                return (false, reply);
            };
            let key = exchange.finish(&msg).unwrap();
            // The receiver's proof only checks out with the right code.
            let receiver_proven = pairing::matches(
                &confirm,
                &pairing::proof(&key, pairing::Role::Receiver, &session.binding),
            );
            let ours = pairing::proof(&key, pairing::Role::Sender, &session.binding);
            wire::send(&mut stream, &Message::PairingConfirm(ours))
                .await
                .unwrap();
            // Proven: the receiver waits for an offer; otherwise it refuses.
            let next = tokio::time::timeout(
                std::time::Duration::from_millis(500),
                wire::read_msg(&mut stream),
            )
            .await;
            (
                receiver_proven,
                next.ok().and_then(|r| r.ok()).unwrap_or(Message::Accept),
            )
        }
    };

    // A wrong guess: the receiver's proof does not verify and it refuses.
    let (proven, answer) = attempt("00000000".into()).await;
    assert!(!proven);
    assert_eq!(answer, Message::Reject(RejectReason::InvitationInvalid));

    // The right code: both proofs verify; the code is used up.
    let (proven, _) = attempt(code.clone()).await;
    assert!(proven);
    let (code_used, by) = b
        .wait("the code to be reported used", |e| match e {
            Event::PairingUsed { code, by } => Some((*code, by.clone())),
            _ => None,
        })
        .await;
    assert!(code_used);
    assert_eq!(by, "Mallory");
    // The same code does not work twice.
    let (_, answer) = attempt(code.clone()).await;
    assert_eq!(answer, Message::Reject(RejectReason::InvitationInvalid));

    // A code sent in the clear, as before, is refused outright.
    let tcp = tokio::net::TcpStream::connect(b.addr()).await.unwrap();
    let (mut stream, _) = tls::connect(&ctx, tcp).await.unwrap();
    let hello = Hello::new("Mallory", "linux", Some(Pairing::Code(code)));
    wire::send(&mut stream, &Message::Hello(hello))
        .await
        .unwrap();
    assert_eq!(
        wire::read_msg(&mut stream).await.unwrap(),
        Message::Reject(RejectReason::InvitationInvalid)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn code_guesses_are_limited_even_when_the_guesser_hangs_up() {
    use dexdrop_core::protocol::{Hello, Message, Pairing, RejectReason};
    use dexdrop_net::{pairing, tls, wire};
    let mut b = Node::start("Bob").await;
    b.send(Command::CreateCode);
    let code = b
        .wait("a code", |e| match e {
            Event::Code { code, .. } => Some(code.clone()),
            _ => None,
        })
        .await;
    let ctx = tls::TlsContext::new(&dexdrop_core::Identity::generate().unwrap()).unwrap();
    // Five guessers check the receiver's proof and hang up without
    // answering: each still used up an attempt.
    for _ in 0..dexdrop_core::invitation::MAX_CODE_FAILURES {
        let tcp = tokio::net::TcpStream::connect(b.addr()).await.unwrap();
        let (mut stream, _) = tls::connect(&ctx, tcp).await.unwrap();
        let (_, msg) = pairing::start("00000000");
        wire::send(
            &mut stream,
            &Message::Hello(Hello::new("M", "linux", Some(Pairing::Pake(msg)))),
        )
        .await
        .unwrap();
        let _ = wire::read_msg(&mut stream).await;
        drop(stream);
    }
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    // Now even the right code is refused: the code closed.
    let tcp = tokio::net::TcpStream::connect(b.addr()).await.unwrap();
    let (mut stream, _) = tls::connect(&ctx, tcp).await.unwrap();
    let (_, msg) = pairing::start(&code);
    wire::send(
        &mut stream,
        &Message::Hello(Hello::new("M", "linux", Some(Pairing::Pake(msg)))),
    )
    .await
    .unwrap();
    assert_eq!(
        wire::read_msg(&mut stream).await.unwrap(),
        Message::Reject(RejectReason::InvitationInvalid)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn many_small_files_arrive_intact() {
    let mut a = Node::start("Alice").await;
    let mut b = Node::start("Bob").await;
    // 500 flushes to disk: slow on CI's Windows disks. This test is about
    // integrity; throughput is measured by `dexdrop bench`.
    a.timeout = std::time::Duration::from_secs(120);
    b.timeout = std::time::Duration::from_secs(120);
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
    // Saved before the drop; the resumed session must not count it twice.
    write_file(&src.path().join("a-note.txt"), 1000, 3);
    a.send(Command::SendFiles {
        target: Target::Address {
            addr: proxy.addr,
            peer_id: Some(b.status.peer_id),
        },
        paths: vec![src.path().join("a-note.txt"), src.path().join("video.mp4")],
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
        .load_journal(&id)
        .unwrap()
        .expect("a journal after the drop");
    let verified = journal.verified_bytes();
    assert!(verified > 0, "nothing was journaled before the drop");

    let sent = a.wait_state("sender finished", |s| s.is_finished()).await;
    assert_eq!(sent.state, TransferState::Completed, "{sent:?}");
    let got = b.wait_state("receiver finished", |s| s.is_finished()).await;
    assert_eq!(got.state, TransferState::Completed, "{got:?}");
    for t in [&sent, &got] {
        assert_eq!(t.files_done, 2, "{t:?}");
        assert_eq!(t.bytes_done, t.total_bytes, "{t:?}");
    }
    assert_eq!(
        std::fs::read(b.downloads().join("video.mp4")).unwrap(),
        std::fs::read(src.path().join("video.mp4")).unwrap()
    );
    // The resumed session did not start over: the link carried the file
    // about once, not the part before the drop plus the whole file again.
    let size = 40 * 1024 * 1024u64 + 1000;
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_unanswered_request_per_device_and_withdrawn_requests_leave_no_trace() {
    let mut a = Node::start("Alice").await;
    let mut b = Node::start("Bob").await;
    let src = tempfile::tempdir().unwrap();
    write_file(&src.path().join("1.txt"), 10, 1);
    write_file(&src.path().join("2.txt"), 10, 2);
    a.send(Command::SendFiles {
        target: b.target(),
        paths: vec![src.path().join("1.txt")],
    });
    let first = b.wait_request().await;
    a.send(Command::SendFiles {
        target: b.target(),
        paths: vec![src.path().join("2.txt")],
    });
    // The second offer from the same device waits its turn.
    let second = a
        .wait("the second transfer told to wait", |e| match e {
            Event::Transfer(t)
                if t.label == "2.txt"
                    && matches!(t.state, TransferState::Interrupted { retrying: true, .. }) =>
            {
                Some(t.id)
            }
            _ => None,
        })
        .await;
    // The sender withdraws it; then the first is withdrawn too.
    a.send(Command::Cancel(second));
    a.send(Command::Cancel(first.id));
    b.wait("the request withdrawn", |e| match e {
        Event::TransferRemoved(id) if *id == first.id => Some(()),
        _ => None,
    })
    .await;
    // Nothing was kept on the receiving side.
    let history = dexdrop_core::store::History::load(&b.state().join("history.json"));
    assert!(history.entries.is_empty(), "{:?}", history.entries);
}

/// A fake device that advertises an open code nearby and records
/// whether anyone offered it files.
async fn impostor() -> (
    dexdrop_core::PeerId,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
    dexdrop_net::discovery::Discovery,
) {
    use dexdrop_core::protocol::{Hello, Message, Pairing};
    use dexdrop_net::{tls, wire};
    let identity = dexdrop_core::Identity::generate().unwrap();
    let id = identity.peer_id();
    let ctx = tls::TlsContext::new(&identity).unwrap();
    let listener = tokio::net::TcpListener::bind("0.0.0.0:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let offered = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let seen = offered.clone();
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let (ctx, seen) = (ctx.clone(), seen.clone());
            tokio::spawn(async move {
                let Ok((mut stream, _)) = tls::accept(&ctx, tcp).await else {
                    return;
                };
                let _ = wire::read_msg(&mut stream).await; // the sender's hello
                // Pretends to accept the code without knowing it.
                let fake = Pairing::PakeReply {
                    msg: vec![0x42; 33],
                    confirm: [7; 32],
                };
                let _ = wire::send(
                    &mut stream,
                    &Message::Hello(Hello::new("Bob", "linux", Some(fake))),
                )
                .await;
                while let Ok(msg) = wire::read_msg(&mut stream).await {
                    if matches!(msg, Message::Offer(_)) {
                        seen.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                }
            });
        }
    });
    // Advertised like a real device, through this platform's discovery.
    let (events, _) = tokio::sync::mpsc::unbounded_channel();
    let mut discovery = dexdrop_net::discovery::Discovery::start(id, events).unwrap();
    discovery
        .advertise(&dexdrop_net::discovery::Advertisement {
            peer_id: id,
            name: "Bob".into(),
            platform: "linux".into(),
            port,
            code_open: true,
        })
        .unwrap();
    (id, offered, discovery)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_device_faking_an_open_code_gets_nothing() {
    let lan = |c: &mut dexdrop_net::EngineConfig| {
        c.discovery = true;
        c.listen = std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED);
    };
    let mut a = Node::start_with("Alice", lan).await;
    let mut b = Node::start_with("Bob", lan).await;
    let (fake_id, offered, _daemon) = impostor().await;
    b.send(Command::CreateCode);
    let code = b
        .wait("a code", |e| match e {
            Event::Code { code, .. } => Some(code.clone()),
            _ => None,
        })
        .await;
    // Both the real receiver and the impostor show an open code.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    let mut both = false;
    let mut last = String::new();
    while let Ok(Some(event)) = tokio::time::timeout_at(deadline, a.events.recv()).await {
        if let Event::Peers(peers) = event {
            let open = |id| peers.iter().any(|p| p.peer_id == id && p.code_open);
            if open(b.status.peer_id) && open(fake_id) {
                both = true;
                break;
            }
            last = format!(
                "receiver open: {}, impostor open: {}, peers: {}",
                open(b.status.peer_id),
                open(fake_id),
                peers.len()
            );
        }
    }
    if !both {
        // The system's Bonjour service is always there on a Mac.
        #[cfg(target_os = "macos")]
        {
            // What the Mac's own Bonjour knows, to tell a missing
            // registration from a missed one.
            let sh = |cmd: String| {
                std::process::Command::new("sh")
                    .arg("-c")
                    .arg(format!("({cmd} & p=$!; sleep 4; kill $p) 2>&1"))
                    .output()
                    .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                    .unwrap_or_default()
            };
            let instance = |id: dexdrop_core::PeerId| format!("dexdrop-{}", &id.to_string()[..16]);
            panic!(
                "Bonjour discovery did not show both open codes ({last})\n\
                 dns-sd -B:\n{}\nimpostor {}:\n{}\nreceiver {}:\n{}",
                sh("dns-sd -B _dexdrop._tcp local.".into()),
                instance(fake_id),
                sh(format!(
                    "dns-sd -L {} _dexdrop._tcp local.",
                    instance(fake_id)
                )),
                instance(b.status.peer_id),
                sh(format!(
                    "dns-sd -L {} _dexdrop._tcp local.",
                    instance(b.status.peer_id)
                )),
            );
        }
        #[cfg(not(target_os = "macos"))]
        {
            eprintln!("skipped: mDNS discovery is not available here ({last})");
            return;
        }
    }
    let src = tempfile::tempdir().unwrap();
    write_file(&src.path().join("secret.pdf"), 50_000, 4);
    // First a code nobody real shows: the impostor is the only candidate.
    // It cannot prove the code, so nothing is sent and the transfer stops.
    a.send(Command::SendFiles {
        target: Target::Code("ZZZZ9999".into()),
        paths: vec![src.path().join("secret.pdf")],
    });
    let refused = a
        .wait_state("the impostor refused", |s| {
            s.is_finished() || matches!(s, TransferState::Interrupted { .. })
        })
        .await;
    assert!(
        matches!(refused.state, TransferState::Declined(_)),
        "{:?}",
        refused.state
    );
    assert!(
        !offered.load(std::sync::atomic::Ordering::SeqCst),
        "the impostor was offered the files"
    );
    // Then the real code: only the real receiver can prove it.
    a.send(Command::SendFiles {
        target: Target::Code(code),
        paths: vec![src.path().join("secret.pdf")],
    });
    // Only the device that proves the code is offered the files.
    let request = b.wait_request().await;
    assert_eq!(request.peer_id, a.status.peer_id);
    assert!(request.via_invitation);
    b.send(accept(request.id));
    b.wait_state("received", |s| *s == TransferState::Completed)
        .await;
    assert!(
        !offered.load(std::sync::atomic::Ordering::SeqCst),
        "the impostor was offered the files"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn keep_both_never_merges_into_a_folder_of_the_same_transfer() {
    let a = Node::start("Alice").await;
    let mut b = Node::start("Bob").await;
    let src = tempfile::tempdir().unwrap();
    write_file(&src.path().join("A/one.txt"), 10, 1);
    write_file(&src.path().join("A (1)/two.txt"), 10, 2);
    std::fs::create_dir_all(b.downloads().join("A")).unwrap();
    a.send(Command::SendFiles {
        target: b.target(),
        paths: vec![src.path().join("A"), src.path().join("A (1)")],
    });
    let request = b.wait_request().await;
    b.send(accept(request.id));
    b.wait_state("completed", |s| *s == TransferState::Completed)
        .await;
    let dl = b.downloads();
    // "A" exists, so it becomes "A (2)", not "A (1)" which this transfer
    // brings itself.
    assert!(dl.join("A (2)/one.txt").exists());
    assert!(dl.join("A (1)/two.txt").exists());
    assert!(!dl.join("A (1)/one.txt").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn identical_files_already_there_are_not_sent_again() {
    let mut a = Node::start("Alice").await;
    let mut b = Node::start("Bob").await;
    let src = tempfile::tempdir().unwrap();
    // Small (one chunk) and large files, identical, different, and new.
    write_file(&src.path().join("same.txt"), 3000, 1);
    write_file(&src.path().join("same-big.bin"), 3 * 1024 * 1024, 2);
    write_file(&src.path().join("differs.txt"), 3000, 3);
    write_file(&src.path().join("new.txt"), 10, 4);
    let dl = b.downloads();
    write_file(&dl.join("same.txt"), 3000, 1);
    write_file(&dl.join("same-big.bin"), 3 * 1024 * 1024, 2);
    write_file(&dl.join("differs.txt"), 3000, 9);
    let untouched = std::fs::metadata(dl.join("same-big.bin"))
        .unwrap()
        .modified()
        .unwrap();

    a.send(Command::SendFiles {
        target: b.target(),
        paths: ["same.txt", "same-big.bin", "differs.txt", "new.txt"]
            .iter()
            .map(|n| src.path().join(n))
            .collect(),
    });
    let request = b.wait_request().await;
    b.send(accept(request.id));
    let end = b
        .wait_state("receiver completed", |s| *s == TransferState::Completed)
        .await;
    let sent = a
        .wait_state("sender completed", |s| *s == TransferState::Completed)
        .await;
    for t in [&end, &sent] {
        assert_eq!(t.files_done, 4, "{t:?}");
        assert_eq!(t.files_failed + t.files_skipped, 0, "{t:?}");
        assert_eq!(t.bytes_done, t.total_bytes, "{t:?}");
    }
    let mut names: Vec<String> = walk(&dl)
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    names.sort();
    // Only the file that differs was kept twice.
    assert_eq!(
        names,
        [
            "differs (1).txt",
            "differs.txt",
            "new.txt",
            "same-big.bin",
            "same.txt"
        ]
    );
    assert_eq!(
        std::fs::read(dl.join("differs (1).txt")).unwrap(),
        std::fs::read(src.path().join("differs.txt")).unwrap()
    );
    assert_eq!(
        std::fs::metadata(dl.join("same-big.bin"))
            .unwrap()
            .modified()
            .unwrap(),
        untouched
    );
}

/// A receiver's rate cap slows the whole transfer (the sender waits on TCP's
/// flow control), and the file still arrives intact.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_receive_rate_cap_slows_the_transfer() {
    let a = Node::start("Alice").await;
    let mut b = Node::start_with("Bob", |c| c.max_receive_rate = Some(2_000_000)).await;
    let src = tempfile::tempdir().unwrap();
    write_file(&src.path().join("capped.bin"), 4_000_000, 9);
    a.send(Command::SendFiles {
        target: b.target(),
        paths: vec![src.path().join("capped.bin")],
    });
    let request = b.wait_request().await;
    let started = std::time::Instant::now();
    b.send(accept(request.id));
    b.wait_state("receiver completed", |s| *s == TransferState::Completed)
        .await;
    let took = started.elapsed();
    // 4 MB at 2 MB/s: about 2 s (the first chunks fill buffers at once).
    assert!(
        took >= std::time::Duration::from_millis(1200),
        "took {took:?}"
    );
    assert_eq!(
        std::fs::read(b.downloads().join("capped.bin")).unwrap(),
        std::fs::read(src.path().join("capped.bin")).unwrap()
    );
}

/// By default the engine listens on IPv4 and IPv6 alike: discovery
/// advertises IPv6 addresses too, and a peer may resolve only those
/// (Android's NsdManager often does). Sends to 127.0.0.1 and to ::1 both
/// arrive.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_default_listener_takes_ipv4_and_ipv6() {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
    let ipv6 = std::net::TcpListener::bind("[::1]:0").is_ok();
    // GitHub's runners have IPv6 loopback: there, a skip would hide a bug.
    assert!(
        ipv6 || std::env::var("CI").is_err(),
        "no IPv6 loopback on CI"
    );
    let a = Node::start("Alice").await;
    let mut b = Node::start_with("Bob", |c| c.listen = IpAddr::V4(Ipv4Addr::UNSPECIFIED)).await;
    let src = tempfile::tempdir().unwrap();
    let mut targets = vec![("over-v4.bin", IpAddr::V4(Ipv4Addr::LOCALHOST))];
    if ipv6 {
        targets.push(("over-v6.bin", IpAddr::V6(Ipv6Addr::LOCALHOST)));
    } else {
        eprintln!("no IPv6 loopback here: IPv4 only");
    }
    for (name, ip) in targets {
        write_file(&src.path().join(name), 100_000, 4);
        a.send(Command::SendFiles {
            target: Target::Address {
                addr: SocketAddr::new(ip, b.status.port),
                peer_id: Some(b.status.peer_id),
            },
            paths: vec![src.path().join(name)],
        });
        let request = b.wait_request().await;
        b.send(accept(request.id));
        b.wait_state(name, |s| *s == TransferState::Completed).await;
        assert_eq!(
            std::fs::read(b.downloads().join(name)).unwrap(),
            std::fs::read(src.path().join(name)).unwrap()
        );
    }
}
