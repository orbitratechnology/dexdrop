//! Transfers over the internet (Iroh) between two engines in one process,
//! through a local relay: local discovery is off and the engines' TCP
//! listeners are on loopback, so only the internet path can carry them.
//! Run with `--features test-relay` (a relay with a self-signed certificate).
#![cfg(feature = "test-relay")]

mod common;

use common::{Node, write_file};
use dexdrop_core::{naming::ConflictPolicy, store::Direction};
use dexdrop_net::{
    Command, EngineConfig, Event, Target, TransferState,
    remote::{Relay, RemoteConfig, Route},
};

fn accept(id: [u8; 16]) -> Command {
    Command::Respond {
        id,
        accept: true,
        trust: false,
        conflict: ConflictPolicy::KeepBoth,
    }
}

fn over_the_internet(relay: &str) -> impl FnOnce(&mut EngineConfig) {
    let relay = relay.to_owned();
    move |c: &mut EngineConfig| {
        c.remote = Some(RemoteConfig {
            relay: Relay::Own(relay),
            address_lookup: false,
            insecure_relay_for_tests: true,
        });
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sends_over_the_internet_then_to_a_known_device() {
    let (_map, relay_url, _server) = iroh::test_utils::run_relay_server().await.unwrap();
    let relay = relay_url.to_string();
    let mut a = Node::start_with("Alice", over_the_internet(&relay)).await;
    let mut b = Node::start_with("Bob", over_the_internet(&relay)).await;

    // 1. With an invitation (a link sent through a messenger): it carries
    // Bob's internet address and relay.
    b.send(Command::CreateInvitation);
    let invitation = b
        .wait("an invitation", |e| match e {
            Event::Invitation(v) => Some(v.text.clone()),
            _ => None,
        })
        .await;
    let src = tempfile::tempdir().unwrap();
    write_file(&src.path().join("far.bin"), 700_000, 7);
    a.send(Command::SendFiles {
        target: Target::Invitation(invitation),
        paths: vec![src.path().join("far.bin")],
    });
    let request = b.wait_request().await;
    assert_ne!(request.route, Route::Nearby, "came over the local network");
    b.send(accept(request.id));
    let done = b
        .wait_state("received over the internet", |s| {
            *s == TransferState::Completed
        })
        .await;
    assert_eq!(done.direction, Direction::Incoming);
    assert!(matches!(done.route, Some(Route::Direct | Route::Relayed)));
    assert_eq!(
        std::fs::read(b.downloads().join("far.bin")).unwrap(),
        std::fs::read(src.path().join("far.bin")).unwrap()
    );
    let sent = a
        .wait_state("sent over the internet", |s| *s == TransferState::Completed)
        .await;
    assert!(matches!(sent.route, Some(Route::Direct | Route::Relayed)));

    // 2. Each now knows the other: listed as reachable over the internet
    // (not nearby), and Alice sends to Bob by his ID alone.
    let bob = b.status.peer_id;
    let listed = a
        .wait("Bob among the devices", |e| match e {
            Event::Peers(peers) => peers.iter().find(|p| p.peer_id == bob).cloned(),
            _ => None,
        })
        .await;
    assert!(!listed.nearby);
    write_file(&src.path().join("again.bin"), 50_000, 9);
    a.send(Command::SendFiles {
        target: Target::Peer(bob),
        paths: vec![src.path().join("again.bin")],
    });
    let request = b.wait_request().await;
    assert_ne!(request.route, Route::Nearby);
    b.send(accept(request.id));
    b.wait_state("received from a known device", |s| {
        *s == TransferState::Completed
    })
    .await;
    assert_eq!(
        std::fs::read(b.downloads().join("again.bin")).unwrap(),
        std::fs::read(src.path().join("again.bin")).unwrap()
    );
}
