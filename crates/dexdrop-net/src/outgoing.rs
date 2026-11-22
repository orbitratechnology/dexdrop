//! The sending side: connect (with retries), offer, stream, resume.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use dexdrop_core::{
    PeerId,
    invitation::{Invitation, normalize_code},
    journal::OutgoingRecord,
    manifest::Manifest,
    protocol::{Hello, Message, Offer, Pairing, RejectReason, TransferId, WireEntry},
    unix_now,
};
use tokio::{net::TcpStream, sync::watch};

use crate::{
    engine::{Ctx, Internal, lock},
    pairing::{self, Role},
    session::{self, Answer, Control, LiveCell, SessionEnd},
    tls::{self, ClientStream, PeerSession},
    wire,
};
use dexdrop_core::api::*;

/// Reconnection attempts before waiting for the user.
const MAX_RETRIES: u32 = 8;
const CONNECT: Duration = Duration::from_secs(4);
const HANDSHAKE: Duration = Duration::from_secs(10);

pub(crate) struct Job {
    pub id: TransferId,
    pub target: Target,
    pub manifest: Arc<Manifest>,
    pub resume: bool,
    pub live: LiveCell,
    pub control: watch::Receiver<Control>,
}

pub(crate) async fn run(ctx: Arc<Ctx>, mut job: Job) {
    let state = drive(&ctx, &mut job).await;
    if state.is_finished() && !matches!(state, TransferState::Failed(_)) {
        let _ = ctx.state.outgoing().remove(&job.id);
    }
    let _ = ctx
        .internal
        .send(Internal::OutgoingEnded { id: job.id, state });
}

enum ConnectError {
    Rejected(RejectReason),
    Unreachable(String),
    WrongPeer,
}

struct Connected {
    stream: ClientStream,
    session: PeerSession,
    hello: Hello,
    addr: SocketAddr,
}

/// How a sender proves it was invited.
#[derive(Clone)]
enum Proof {
    Token([u8; 16]),
    /// A typed code, proven with SPAKE2 (never sent).
    Code(String),
}

struct Route {
    /// The peer ID the other side must have.
    pinned: Option<PeerId>,
    /// Addresses to try when discovery has none.
    known: Vec<SocketAddr>,
    /// Proof of invitation, sent on the first successful connection only.
    pairing: Option<Proof>,
    /// For codes: try every nearby device that shows one.
    code: Option<String>,
}

fn route_of(target: &Target) -> Result<Route, String> {
    Ok(match target {
        Target::Peer(id) => Route {
            pinned: Some(*id),
            known: Vec::new(),
            pairing: None,
            code: None,
        },
        Target::Invitation(text) => {
            let inv = Invitation::decode(text, unix_now()).map_err(|e| e.to_string())?;
            Route {
                pinned: Some(inv.peer_id),
                known: inv.addrs,
                pairing: Some(Proof::Token(inv.token)),
                code: None,
            }
        }
        Target::Code(code) => Route {
            pinned: None,
            known: Vec::new(),
            pairing: None,
            code: Some(normalize_code(code).ok_or("Codes have 8 letters and digits.")?),
        },
        Target::Address { addr, peer_id } => Route {
            pinned: *peer_id,
            known: vec![*addr],
            pairing: None,
            code: None,
        },
    })
}

fn report(ctx: &Ctx, id: TransferId, state: TransferState) {
    let _ = ctx.internal.send(Internal::OutgoingState { id, state });
}

async fn drive(ctx: &Arc<Ctx>, job: &mut Job) -> TransferState {
    let mut route = match route_of(&job.target) {
        Ok(route) => route,
        Err(reason) => return TransferState::Failed(reason),
    };
    let entries: Vec<WireEntry> = job.manifest.wire_entries();
    let mut failures = 0u32;
    loop {
        if *job.control.borrow_and_update() == Control::Pause {
            report(ctx, job.id, TransferState::Paused { by_peer: false });
            if wait_for_run(&mut job.control).await == Control::Cancel {
                return TransferState::Cancelled { by_peer: false };
            }
        }
        if *job.control.borrow() == Control::Cancel {
            return TransferState::Cancelled { by_peer: false };
        }
        report(ctx, job.id, TransferState::Connecting);
        let connected = connect(ctx, &route).await;
        let mut c = match connected {
            Ok(c) => c,
            Err(ConnectError::Rejected(RejectReason::Busy)) => {
                match retry(ctx, job, &mut failures, "The other device is busy.").await {
                    Some(end) => return end,
                    None => continue,
                }
            }
            Err(ConnectError::Rejected(reason)) => {
                return TransferState::Declined(reason.explain().into());
            }
            Err(ConnectError::WrongPeer) => {
                return TransferState::Failed(
                    "The device that answered is not the one expected.".into(),
                );
            }
            // A code that no nearby device shows will not appear by
            // waiting: codes are single-use and expire.
            Err(ConnectError::Unreachable(reason)) if route.code.is_some() => {
                return TransferState::Failed(format!(
                    "{reason} Check the code, or ask for a new one."
                ));
            }
            Err(ConnectError::Unreachable(reason)) => {
                match retry(ctx, job, &mut failures, &reason).await {
                    Some(end) => return end,
                    None => continue,
                }
            }
        };

        // From now on, always talk to this exact device.
        route.pinned = Some(c.session.peer_id);
        route.pairing = None;
        route.code = None;
        if !route.known.contains(&c.addr) {
            route.known.insert(0, c.addr);
        }
        let _ = ctx.internal.send(Internal::OutgoingPeer {
            id: job.id,
            peer_id: c.session.peer_id,
            name: dexdrop_core::protocol::clean_name(&c.hello.device_name),
            platform: c.hello.platform.clone(),
            sas: c.session.sas.clone(),
        });
        let record = OutgoingRecord {
            version: 1,
            transfer_id: job.id,
            peer_id: c.session.peer_id,
            peer_name: dexdrop_core::protocol::clean_name(&c.hello.device_name),
            peer_platform: c.hello.platform.clone(),
            addrs: route.known.clone(),
            chunk_size: ctx.chunk_size,
            manifest: (*job.manifest).clone(),
            created: unix_now(),
        };
        let _ = ctx.state.outgoing().save(&job.id, &record);

        let offer = Offer {
            transfer_id: job.id,
            entries: entries.len() as u64,
            files: job.manifest.files,
            folders: job.manifest.folders,
            total_bytes: job.manifest.total_bytes,
            chunk_size: ctx.chunk_size,
            label: job.manifest.label.clone(),
            resume: job.resume,
        };
        if let Err(err) = session::send_offer(&mut c.stream, &offer, &entries).await {
            match retry(ctx, job, &mut failures, &err.to_string()).await {
                Some(end) => return end,
                None => continue,
            }
        }
        report(ctx, job.id, TransferState::Waiting);
        match session::await_answer(&mut c.stream, &mut job.control).await {
            Ok(Answer::Accepted) => {}
            Ok(Answer::Rejected(RejectReason::Busy)) => {
                match retry(ctx, job, &mut failures, "The other device is busy.").await {
                    Some(end) => return end,
                    None => continue,
                }
            }
            Ok(Answer::Rejected(reason)) => {
                return TransferState::Declined(reason.explain().into());
            }
            Ok(Answer::Cancelled) => return TransferState::Cancelled { by_peer: false },
            Err(err) => match retry(ctx, job, &mut failures, &err.to_string()).await {
                Some(end) => return end,
                None => continue,
            },
        }

        job.resume = true;
        failures = 0;
        report(ctx, job.id, TransferState::Transferring);
        job.control.borrow_and_update();
        let ended = session::run_sender(
            c.stream,
            job.manifest.clone(),
            ctx.chunk_size,
            job.live.clone(),
            job.control.clone(),
            ctx.max_send_rate,
        )
        .await;
        match ended {
            Ok(SessionEnd::Completed) => {
                let live = lock(&job.live);
                return if live.files_failed > 0 || live.files_skipped > 0 {
                    TransferState::CompletedWithIssues
                } else {
                    TransferState::Completed
                };
            }
            Ok(SessionEnd::Paused { by_peer }) => {
                report(ctx, job.id, TransferState::Paused { by_peer });
                if wait_for_run(&mut job.control).await == Control::Cancel {
                    return TransferState::Cancelled { by_peer: false };
                }
            }
            Ok(SessionEnd::OutOfSpace { .. }) => {
                report(
                    ctx,
                    job.id,
                    TransferState::Interrupted {
                        reason:
                            "The other device ran out of storage space. Resume once it has room."
                                .into(),
                        retrying: false,
                    },
                );
                if wait_for_run(&mut job.control).await == Control::Cancel {
                    return TransferState::Cancelled { by_peer: false };
                }
            }
            Ok(SessionEnd::Cancelled { by_peer }) => return TransferState::Cancelled { by_peer },
            Ok(SessionEnd::Rejected(reason)) => {
                return TransferState::Declined(reason.explain().into());
            }
            Err(err) if err.is_retryable() => {
                match retry(ctx, job, &mut failures, "The connection was interrupted.").await {
                    Some(end) => return end,
                    None => continue,
                }
            }
            Err(err) => return TransferState::Failed(err.to_string()),
        }
    }
}

/// Waits before the next attempt; `Some` when the transfer should end.
async fn retry(
    ctx: &Ctx,
    job: &mut Job,
    failures: &mut u32,
    reason: &str,
) -> Option<TransferState> {
    *failures += 1;
    if *failures > MAX_RETRIES {
        report(
            ctx,
            job.id,
            TransferState::Interrupted {
                reason: format!("{reason} Check that both devices are on the same network."),
                retrying: false,
            },
        );
        *failures = 0;
        return match wait_for_run(&mut job.control).await {
            Control::Cancel => Some(TransferState::Cancelled { by_peer: false }),
            _ => None,
        };
    }
    report(
        ctx,
        job.id,
        TransferState::Interrupted {
            reason: reason.to_owned(),
            retrying: true,
        },
    );
    let delay = Duration::from_secs((1u64 << (*failures - 1).min(4)).min(15));
    tokio::select! {
        _ = tokio::time::sleep(delay) => None,
        changed = job.control.changed() => {
            match if changed.is_err() { Control::Cancel } else { *job.control.borrow_and_update() } {
                Control::Cancel => Some(TransferState::Cancelled { by_peer: false }),
                Control::Pause => {
                    report(ctx, job.id, TransferState::Paused { by_peer: false });
                    match wait_for_run(&mut job.control).await {
                        Control::Cancel => Some(TransferState::Cancelled { by_peer: false }),
                        _ => None,
                    }
                }
                Control::Run => None,
            }
        }
    }
}

async fn wait_for_run(control: &mut watch::Receiver<Control>) -> Control {
    loop {
        if control.changed().await.is_err() {
            return Control::Cancel;
        }
        let wanted = *control.borrow_and_update();
        if wanted != Control::Pause {
            return wanted;
        }
    }
}

async fn connect(ctx: &Ctx, route: &Route) -> Result<Connected, ConnectError> {
    if let Some(code) = &route.code {
        let candidates: Vec<_> = lock(&ctx.peers)
            .values()
            .filter(|p| p.code_open)
            .cloned()
            .collect();
        if candidates.is_empty() {
            return Err(ConnectError::Unreachable(
                "No nearby device is showing a code.".into(),
            ));
        }
        let mut last = ConnectError::Rejected(RejectReason::InvitationInvalid);
        for peer in candidates {
            for addr in &peer.addrs {
                match connect_one(
                    ctx,
                    *addr,
                    Some(peer.peer_id),
                    Some(Proof::Code(code.clone())),
                )
                .await
                {
                    Ok(c) => return Ok(c),
                    Err(ConnectError::Rejected(RejectReason::InvitationInvalid)) => break,
                    Err(err) => last = err,
                }
            }
        }
        return Err(match last {
            ConnectError::Rejected(RejectReason::InvitationInvalid) => {
                ConnectError::Rejected(RejectReason::InvitationInvalid)
            }
            other => other,
        });
    }

    let mut addrs: Vec<SocketAddr> = route
        .pinned
        .and_then(|id| lock(&ctx.peers).get(&id).map(|p| p.addrs.clone()))
        .unwrap_or_default();
    for addr in &route.known {
        if !addrs.contains(addr) {
            addrs.push(*addr);
        }
    }
    if addrs.is_empty() {
        return Err(ConnectError::Unreachable(
            "The device is not visible nearby.".into(),
        ));
    }
    let mut last = ConnectError::Unreachable("The device could not be reached.".into());
    for addr in addrs {
        match connect_one(ctx, addr, route.pinned, route.pairing.clone()).await {
            Ok(c) => return Ok(c),
            Err(err @ (ConnectError::Rejected(_) | ConnectError::WrongPeer)) => return Err(err),
            Err(err) => last = err,
        }
    }
    Err(last)
}

async fn connect_one(
    ctx: &Ctx,
    addr: SocketAddr,
    pinned: Option<PeerId>,
    proof: Option<Proof>,
) -> Result<Connected, ConnectError> {
    let unreachable = |what: &str| {
        ConnectError::Unreachable(format!("The device could not be reached ({what})."))
    };
    let tcp = tokio::time::timeout(CONNECT, TcpStream::connect(addr))
        .await
        .map_err(|_| unreachable("timed out"))?
        .map_err(|e| unreachable(&e.kind().to_string()))?;
    let (mut stream, session) = tokio::time::timeout(HANDSHAKE, tls::connect(&ctx.tls, tcp))
        .await
        .map_err(|_| unreachable("handshake timed out"))?
        .map_err(|_| unreachable("secure handshake failed"))?;
    if pinned.is_some_and(|expected| expected != session.peer_id) {
        return Err(ConnectError::WrongPeer);
    }
    // A code is never sent: a SPAKE2 message stands in for it, and the
    // answer must prove the device knows the same code.
    let (pairing, exchange) = match proof {
        None => (None, None),
        Some(Proof::Token(token)) => (Some(Pairing::Token(token)), None),
        Some(Proof::Code(code)) => {
            let (exchange, msg) = pairing::start(&code);
            (Some(Pairing::Pake(msg)), Some(exchange))
        }
    };
    let ours = Hello::new(&ctx.device_name(), &ctx.platform, pairing);
    wire::send(&mut stream, &Message::Hello(ours.clone()))
        .await
        .map_err(|e| ConnectError::Unreachable(e.to_string()))?;
    match wire::read_msg_within(&mut stream, HANDSHAKE).await {
        Ok(Message::Hello(hello)) => {
            if ours.negotiate(&hello).is_none() {
                return Err(ConnectError::Rejected(RejectReason::Incompatible));
            }
            if let Some(exchange) = exchange {
                // A device that cannot prove the code is not the one meant:
                // nothing is offered to it (review #1).
                let refused = ConnectError::Rejected(RejectReason::InvitationInvalid);
                let Some(Pairing::PakeReply { msg, confirm }) = &hello.pairing else {
                    return Err(refused);
                };
                let key = exchange.finish(msg).ok_or(refused)?;
                if !pairing::matches(
                    confirm,
                    &pairing::proof(&key, Role::Receiver, &session.binding),
                ) {
                    return Err(ConnectError::Rejected(RejectReason::InvitationInvalid));
                }
                let ours = pairing::proof(&key, Role::Sender, &session.binding);
                wire::send(&mut stream, &Message::PairingConfirm(ours))
                    .await
                    .map_err(|e| ConnectError::Unreachable(e.to_string()))?;
            }
            Ok(Connected {
                stream,
                session,
                hello,
                addr,
            })
        }
        Ok(Message::Reject(reason)) => Err(ConnectError::Rejected(reason)),
        Ok(other) => Err(ConnectError::Unreachable(format!(
            "unexpected {}",
            other.name()
        ))),
        Err(err) => Err(ConnectError::Unreachable(err.to_string())),
    }
}
