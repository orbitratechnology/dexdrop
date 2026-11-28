//! The direct link (docs/CONNECTIVITY.md 4.3) between two engines, with
//! the loopback link standing in for the radio: one hosts, its invitation
//! carries the network, the other joins it and sends.

mod common;

use std::sync::{Arc, atomic::Ordering};

use common::{Node, write_file};
use dexdrop_core::{
    api::{DirectState, Route},
    invitation::Invitation,
    naming::ConflictPolicy,
};
use dexdrop_net::{Command, Event, Target, TransferState, direct::Loopback};

async fn node(
    name: &str,
    link: &Arc<Loopback>,
    tweak: impl FnOnce(&mut dexdrop_net::EngineConfig),
) -> Node {
    let link = link.clone();
    Node::start_with(name, move |c| {
        c.direct_link = Some(link);
        tweak(c);
    })
    .await
}

async fn direct_state(
    node: &mut Node,
    what: &str,
    want: impl Fn(&DirectState) -> bool,
) -> DirectState {
    node.wait(what, |e| match e {
        Event::Status(s) if want(&s.direct) => Some(s.direct.clone()),
        _ => None,
    })
    .await
}

/// Hosts and returns the invitation the receiver shows, with only the
/// direct link in it: no address on a network in common, no internet.
async fn host(bob: &mut Node) -> Invitation {
    bob.send(Command::HostDirectLink);
    direct_state(bob, "hosting", |d| matches!(d, DirectState::Hosting { .. })).await;
    let text = bob
        .wait("the invitation", |e| match e {
            Event::Invitation(view) => Some(view.text.clone()),
            _ => None,
        })
        .await;
    let mut inv = Invitation::decode(&text, 0).unwrap();
    let hint = inv
        .direct
        .clone()
        .expect("the invitation carries the direct link");
    assert!(hint.is_valid());
    assert_eq!(hint.host.port(), bob.status.port);
    inv.addrs.clear();
    inv.remote = None;
    inv
}

fn accept(id: [u8; 16]) -> Command {
    Command::Respond {
        id,
        accept: true,
        trust: false,
        conflict: ConflictPolicy::KeepBoth,
    }
}

/// With no network in common, the sender joins the network the receiver
/// hosts and the files go over it; both say so ("directly over Wi-Fi").
/// Hosting stops when the receiver asks once the transfer is over.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn files_go_over_the_network_the_receiver_hosts() {
    let (alice_link, bob_link) = (Arc::new(Loopback::default()), Arc::new(Loopback::default()));
    let mut alice = node("Alice", &alice_link, |_| {}).await;
    let mut bob = node("Bob", &bob_link, |_| {}).await;
    assert_eq!(bob.status.direct, DirectState::Off);
    assert!(alice.status.direct_join);

    let inv = host(&mut bob).await;
    let src = tempfile::tempdir().unwrap();
    write_file(&src.path().join("clip.mov"), 400_000, 7);
    alice.send(Command::SendFiles {
        target: Target::Invitation(inv.encode()),
        paths: vec![src.path().join("clip.mov")],
    });
    let request = bob.wait_request().await;
    assert!(request.via_invitation);
    bob.send(accept(request.id));
    let received = bob
        .wait_state("received", |s| *s == TransferState::Completed)
        .await;
    let sent = alice
        .wait_state("sent", |s| *s == TransferState::Completed)
        .await;
    assert_eq!(received.route, Some(Route::PeerToPeer));
    assert_eq!(sent.route, Some(Route::PeerToPeer));
    assert_eq!(alice_link.joins.load(Ordering::SeqCst), 1);
    assert_eq!(bob_link.hosts.load(Ordering::SeqCst), 1);

    bob.send(Command::StopDirectLink);
    direct_state(&mut bob, "stopped", |d| *d == DirectState::Off).await;
    assert_eq!(bob_link.stops.load(Ordering::SeqCst), 1);
}

/// Leaving the Receive page (its invitations closed) while a transfer runs
/// over the network keeps it up until the transfer ends.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hosting_outlasts_the_invitation_while_a_transfer_uses_it() {
    let (alice_link, bob_link) = (Arc::new(Loopback::default()), Arc::new(Loopback::default()));
    let mut alice = node("Alice", &alice_link, |_| {}).await;
    // 1.5 MB at 300 KB/s: about five seconds of transfer to close it in.
    let mut bob = node("Bob", &bob_link, |c| c.max_receive_rate = Some(300_000)).await;
    let inv = host(&mut bob).await;
    let src = tempfile::tempdir().unwrap();
    write_file(&src.path().join("big.bin"), 1_500_000, 3);
    alice.send(Command::SendFiles {
        target: Target::Invitation(inv.encode()),
        paths: vec![src.path().join("big.bin")],
    });
    let request = bob.wait_request().await;
    bob.send(accept(request.id));
    bob.wait_state("receiving", |s| *s == TransferState::Transferring)
        .await;

    bob.send(Command::CloseInvitations);
    bob.wait("invitations closed", |e| {
        matches!(e, Event::InvitationsClosed).then_some(())
    })
    .await;
    assert_eq!(
        bob_link.stops.load(Ordering::SeqCst),
        0,
        "stopped under a transfer"
    );

    bob.wait_state("received", |s| *s == TransferState::Completed)
        .await;
    direct_state(&mut bob, "stopped after the transfer", |d| {
        *d == DirectState::Off
    })
    .await;
    assert_eq!(bob_link.stops.load(Ordering::SeqCst), 1);
    alice
        .wait_state("sent", |s| *s == TransferState::Completed)
        .await;
}

/// When the platform cannot start the network, the receiver hears why, and
/// asking again later is possible.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_network_that_cannot_start_says_why() {
    let link = Arc::new(Loopback::default());
    *link.refuse_host.lock().unwrap() = Some("Wi-Fi is off.".into());
    let mut bob = node("Bob", &link, |_| {}).await;
    bob.send(Command::HostDirectLink);
    let failed = direct_state(&mut bob, "failed", |d| matches!(d, DirectState::Failed(_))).await;
    assert_eq!(failed, DirectState::Failed("Wi-Fi is off.".into()));

    *link.refuse_host.lock().unwrap() = None;
    bob.send(Command::HostDirectLink);
    direct_state(&mut bob, "hosting", |d| {
        matches!(d, DirectState::Hosting { .. })
    })
    .await;
}

/// Without a direct link, nothing changes: the state says it is not
/// available, and an invitation carries no network.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_a_direct_link_nothing_is_hosted() {
    let mut bob = Node::start("Bob").await;
    assert_eq!(bob.status.direct, DirectState::Unavailable);
    assert!(!bob.status.direct_join);
    bob.send(Command::HostDirectLink);
    bob.wait("refused", |e| match e {
        Event::Notice { text, .. } if text.contains("direct link") => Some(()),
        _ => None,
    })
    .await;
}
