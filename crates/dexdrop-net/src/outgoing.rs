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
/// Dialing over the internet: address lookup, a relay, hole punching.
const CONNECT_REMOTE: Duration = Duration::from_secs(20);
const HANDSHAKE: Duration = Duration::from_secs(10);
/// Happy Eyeballs: how long one address has before the next starts too.
const STAGGER: Duration = Duration::from_millis(250);
/// How long the network has before the internet is tried beside it.
const LAN_HEAD_START: Duration = Duration::from_millis(1500);

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
    /// The local address that answered; `None` over the internet.
    addr: Option<SocketAddr>,
    /// The internet connection, to follow its route.
    internet: Option<iroh::endpoint::Connection>,
    /// It goes over a network the other device hosts (the direct link).
    direct: bool,
}

impl Connected {
    fn route(&self) -> dexdrop_core::api::Route {
        match (&self.internet, self.addr) {
            _ if self.direct => dexdrop_core::api::Route::PeerToPeer,
            (Some(conn), _) => crate::remote::route(conn),
            (None, Some(addr)) => crate::link::local_route(addr),
            (None, None) => dexdrop_core::api::Route::Nearby,
        }
    }
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
    /// The device's internet address (raw Ed25519 key) and relay hint, from
    /// an invitation; for a known device it is looked up when dialing.
    remote: Option<([u8; 32], Option<String>)>,
    /// A network the device hosts for this transfer, from an invitation.
    direct: Option<dexdrop_core::invitation::DirectHint>,
    /// That network, once joined: kept for the transfer, left when it ends.
    joined: Option<Arc<dyn crate::direct::Joined>>,
}

fn route_of(target: &Target) -> Result<Route, String> {
    Ok(match target {
        Target::Peer(id) => Route {
            pinned: Some(*id),
            known: Vec::new(),
            pairing: None,
            code: None,
            remote: None,
            direct: None,
            joined: None,
        },
        Target::Invitation(text) => {
            let inv = Invitation::decode(text, unix_now()).map_err(|e| e.to_string())?;
            Route {
                pinned: Some(inv.peer_id),
                known: inv.addrs,
                pairing: Some(Proof::Token(inv.token)),
                code: None,
                remote: inv.remote.map(|r| (r.key, r.relay)),
                direct: inv.direct,
                joined: None,
            }
        }
        Target::Code(code) => Route {
            pinned: None,
            known: Vec::new(),
            pairing: None,
            code: Some(normalize_code(code).ok_or("Codes have 8 letters and digits.")?),
            remote: None,
            direct: None,
            joined: None,
        },
        Target::Address { addr, peer_id } => Route {
            pinned: *peer_id,
            known: vec![*addr],
            pairing: None,
            code: None,
            remote: None,
            direct: None,
            joined: None,
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
        let connected = connect(ctx, &mut route).await;
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
        if let Some(addr) = c.addr
            && !route.known.contains(&addr)
        {
            route.known.insert(0, addr);
        }
        if route.remote.is_none()
            && let Some(key) = c.session.key
        {
            route.remote = Some((key, None));
        }
        let _ = ctx.internal.send(Internal::OutgoingPeer {
            id: job.id,
            peer_id: c.session.peer_id,
            name: dexdrop_core::protocol::clean_name(&c.hello.device_name),
            platform: c.hello.platform.clone(),
            sas: c.session.sas.clone(),
            route: c.route(),
            key: c.session.key,
            avatar: dexdrop_core::avatar::Avatar::clean(c.hello.avatar.as_deref()),
        });
        if let Some(conn) = c.internet.clone() {
            crate::remote::follow_route(ctx.clone(), job.id, conn);
        }
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

async fn connect(ctx: &Ctx, route: &mut Route) -> Result<Connected, ConnectError> {
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
            match connect_raced(
                ctx,
                peer.addrs.clone(),
                Some(peer.peer_id),
                Some(Proof::Code(code.clone())),
            )
            .await
            {
                Ok(c) => return Ok(c),
                // Not this device's code: the next one showing a code.
                Err(ConnectError::Rejected(RejectReason::InvitationInvalid)) => {}
                Err(err) => last = err,
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
    // The internet, when this device can and the other's internet address
    // is known (an invitation, or a device it exchanged files with before).
    let remote = route.remote.clone().or_else(|| {
        let id = route.pinned?;
        lock(&ctx.known).get(&id).map(|k| (k.key, k.relay.clone()))
    });
    let internet = match (ctx.remote.as_ref(), remote) {
        (Some(endpoint), Some((key, relay))) => Some((endpoint, key, relay)),
        _ => None,
    };
    // A network the device hosts for this transfer (the direct link): the
    // network in common first, as there may be one after all, then the
    // direct link, then the internet. One at a time, so an invitation's
    // proof goes on one connection only.
    if let Some(hint) = route
        .direct
        .clone()
        .filter(|_| ctx.direct.as_ref().is_some_and(|d| d.can_join()))
    {
        if !addrs.is_empty() {
            match connect_raced(ctx, addrs, route.pinned, route.pairing.clone()).await {
                Ok(c) => return Ok(c),
                Err(err @ ConnectError::Rejected(_)) => return Err(err),
                // The direct link's error says more than this network's.
                Err(_) => {}
            }
        }
        let last = match connect_direct(ctx, route, &hint).await {
            Ok(c) => return Ok(c),
            Err(err @ ConnectError::Rejected(_)) => return Err(err),
            Err(err) => err,
        };
        return match internet {
            Some((endpoint, key, relay)) => pick(
                last,
                connect_remote(ctx, endpoint, &key, relay.as_deref(), route).await,
            ),
            None => Err(last),
        };
    }
    let lan = connect_raced(ctx, addrs.clone(), route.pinned, route.pairing.clone());
    let Some((endpoint, key, relay)) = internet else {
        return lan.await;
    };
    let internet = connect_remote(ctx, endpoint, &key, relay.as_deref(), route);
    // A pairing proof goes on one connection only: the internet is tried
    // after the network has failed, as before. Without one, the internet
    // starts once the network has had a head start, so a device that
    // cannot be reached here costs LAN_HEAD_START, not every address's
    // timeout, and the first connection made wins.
    if route.pairing.is_some() || addrs.is_empty() {
        let last = if addrs.is_empty() {
            ConnectError::Unreachable("The device is not visible nearby.".into())
        } else {
            match lan.await {
                Ok(c) => return Ok(c),
                Err(err @ ConnectError::Rejected(_)) => return Err(err),
                Err(err) => err,
            }
        };
        return pick(last, internet.await);
    }
    tokio::pin!(lan);
    let internet = async {
        tokio::time::sleep(LAN_HEAD_START).await;
        internet.await
    };
    tokio::pin!(internet);
    tokio::select! {
        biased;
        lan_result = &mut lan => match lan_result {
            Ok(c) => Ok(c),
            Err(err @ ConnectError::Rejected(_)) => Err(err),
            Err(err) => pick(err, internet.await),
        },
        internet_result = &mut internet => match internet_result {
            Ok(c) => Ok(c),
            Err(err @ (ConnectError::Rejected(_) | ConnectError::WrongPeer)) => Err(err),
            // The internet failed: the network still has its chance.
            Err(err) => match lan.await {
                Ok(c) => Ok(c),
                Err(lan_err @ (ConnectError::Rejected(_) | ConnectError::WrongPeer)) => Err(lan_err),
                Err(_) => Err(err),
            },
        },
    }
}

/// Joins the network the device hosts (once for the transfer: later
/// connections reuse it) and connects to the device there.
async fn connect_direct(
    ctx: &Ctx,
    route: &mut Route,
    hint: &dexdrop_core::invitation::DirectHint,
) -> Result<Connected, ConnectError> {
    let joined = match &route.joined {
        Some(joined) => joined.clone(),
        None => {
            let link = ctx.direct.clone().ok_or_else(|| {
                ConnectError::Unreachable("This device cannot join a direct link.".into())
            })?;
            let wanted = hint.clone();
            let joined = tokio::task::spawn_blocking(move || link.join(&wanted))
                .await
                .map_err(|e| ConnectError::Unreachable(e.to_string()))?
                .map_err(|why| {
                    ConnectError::Unreachable(format!("The direct link could not be joined: {why}"))
                })?;
            let joined: Arc<dyn crate::direct::Joined> = Arc::from(joined);
            route.joined = Some(joined.clone());
            joined
        }
    };
    let tcp = tcp_through(hint.host, joined.as_ref()).await.map_err(|e| {
        ConnectError::Unreachable(format!(
            "The device could not be reached on the direct link ({e})."
        ))
    })?;
    let mut c = handshake(ctx, tls::tcp_link(tcp), route.pinned, route.pairing.clone()).await?;
    c.direct = true;
    Ok(c)
}

/// A TCP connection to `addr` from a socket the joined network prepared
/// (bound to it where the system keeps it apart).
async fn tcp_through(
    addr: SocketAddr,
    joined: &dyn crate::direct::Joined,
) -> std::io::Result<TcpStream> {
    use socket2::{Domain, Protocol, Socket, Type};
    let socket = Socket::new(Domain::for_address(addr), Type::STREAM, Some(Protocol::TCP))?;
    joined.prepare(&socket)?;
    socket.set_nonblocking(true)?;
    let socket = tokio::net::TcpSocket::from_std_stream(socket.into());
    tokio::time::timeout(CONNECT, socket.connect(addr))
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "timed out"))?
}

/// The error to report when the network failed with `lan` and the internet
/// answered `internet`: a refusal or a wrong device is said as it is; a
/// wrong device seen here beats an internet path that did not answer.
fn pick(
    lan: ConnectError,
    internet: Result<Connected, ConnectError>,
) -> Result<Connected, ConnectError> {
    match internet {
        Ok(c) => Ok(c),
        Err(err @ (ConnectError::Rejected(_) | ConnectError::WrongPeer)) => Err(err),
        Err(ConnectError::Unreachable(_)) if matches!(lan, ConnectError::WrongPeer) => Err(lan),
        Err(err) => Err(err),
    }
}

/// Connects to one of a device's addresses on this network: their TCP
/// connections raced ([`race_tcp`]), then the TLS handshake on the one that
/// answered first. A handshake that finds another device, or fails, goes on
/// with the addresses left; a refusal ends it. The handshake (which carries
/// a pairing proof) runs on one connection at a time, so a proof is never
/// spent on a connection that is then dropped.
async fn connect_raced(
    ctx: &Ctx,
    mut addrs: Vec<SocketAddr>,
    pinned: Option<PeerId>,
    proof: Option<Proof>,
) -> Result<Connected, ConnectError> {
    order(&mut addrs, &crate::discovery::local_addrs());
    let mut last = ConnectError::Unreachable("The device is not visible nearby.".into());
    while !addrs.is_empty() {
        let (addr, tcp) = match race_tcp(addrs.clone(), STAGGER).await {
            Ok(won) => won,
            Err(why) => {
                // Every address failed to connect: a handshake failure seen
                // before says more than the last timeout.
                return Err(match last {
                    ConnectError::Unreachable(_) => ConnectError::Unreachable(format!(
                        "The device could not be reached ({why})."
                    )),
                    other => other,
                });
            }
        };
        addrs.retain(|a| *a != addr);
        match handshake(ctx, tls::tcp_link(tcp), pinned, proof.clone()).await {
            Ok(mut c) => {
                c.addr = Some(addr);
                return Ok(c);
            }
            Err(err @ ConnectError::Rejected(_)) => return Err(err),
            // Another device answered at this address (a stale or spoofed
            // announcement): the expected one may be at the next address.
            Err(err) => last = err,
        }
    }
    Err(last)
}

/// Puts the addresses most likely to answer first: those on one of this
/// device's own networks (the same IPv4 /24 or IPv6 /64), then IPv4, then
/// IPv6, link-local last. Stable, so discovery's order breaks ties.
fn order(addrs: &mut [SocketAddr], local: &[std::net::IpAddr]) {
    use std::net::IpAddr;
    let same_network = |ip: &IpAddr| {
        local.iter().any(|l| match (ip, l) {
            (IpAddr::V4(a), IpAddr::V4(b)) => a.octets()[..3] == b.octets()[..3],
            (IpAddr::V6(a), IpAddr::V6(b)) => a.segments()[..4] == b.segments()[..4],
            _ => false,
        })
    };
    let link_local = |ip: &IpAddr| match ip {
        IpAddr::V4(v4) => v4.is_link_local(),
        IpAddr::V6(v6) => (v6.segments()[0] & 0xffc0) == 0xfe80,
    };
    addrs.sort_by_key(|a| {
        let ip = a.ip();
        (!same_network(&ip), link_local(&ip), ip.is_ipv6())
    });
}

/// Dials `addrs` the Happy Eyeballs way (RFC 8305): the first at once, each
/// next one `stagger` later or as soon as one before it fails, all running
/// together; the first TCP connection made wins and the others are dropped.
/// A dead address (a VPN's, a container bridge's, a stale IPv6 one) no
/// longer costs its whole timeout before the next is tried. Each attempt
/// gives up after `CONNECT`.
async fn race_tcp(
    addrs: Vec<SocketAddr>,
    stagger: Duration,
) -> Result<(SocketAddr, TcpStream), String> {
    let mut pending = addrs.into_iter();
    let mut running = tokio::task::JoinSet::new();
    let mut last = "no address".to_owned();
    let mut start_next = true;
    loop {
        if start_next && let Some(addr) = pending.next() {
            running.spawn(async move {
                (
                    addr,
                    tokio::time::timeout(CONNECT, TcpStream::connect(addr)).await,
                )
            });
        }
        if running.is_empty() {
            return Err(last);
        }
        let finished = if pending.len() > 0 {
            match tokio::time::timeout(stagger, running.join_next()).await {
                Ok(done) => done,
                // The stagger passed with no answer: start the next one.
                Err(_) => {
                    start_next = true;
                    continue;
                }
            }
        } else {
            running.join_next().await
        };
        start_next = true;
        match finished {
            Some(Ok((addr, Ok(Ok(tcp))))) => {
                running.abort_all();
                return Ok((addr, tcp));
            }
            Some(Ok((_, Ok(Err(e))))) => last = e.kind().to_string(),
            Some(Ok((_, Err(_)))) => last = "timed out".to_owned(),
            Some(Err(_)) | None => {}
        }
    }
}

/// Dials the device over the internet (Iroh) and runs the same TLS
/// handshake inside the connection's stream.
async fn connect_remote(
    ctx: &Ctx,
    endpoint: &iroh::Endpoint,
    key: &[u8; 32],
    relay: Option<&str>,
    route: &Route,
) -> Result<Connected, ConnectError> {
    let unreachable = |what: &str| {
        ConnectError::Unreachable(format!(
            "The device could not be reached over the internet ({what})."
        ))
    };
    // The key must be the device meant: the TLS identity is checked again
    // inside, but a wrong key is refused before dialing anything.
    if route
        .pinned
        .is_some_and(|expected| PeerId::from_ed25519_key(key) != expected)
    {
        return Err(ConnectError::WrongPeer);
    }
    // Without a relay hint, this device's own home relay: the other device
    // often uses the same, and address lookup finds it otherwise.
    let home = crate::remote::home_relay(endpoint);
    let relay = relay.or(home.as_deref());
    let addr = crate::remote::addr_of(key, relay, &[]).ok_or_else(|| unreachable("bad address"))?;
    let (link, conn) = tokio::time::timeout(CONNECT_REMOTE, crate::remote::open(endpoint, addr))
        .await
        .map_err(|_| unreachable("timed out"))?
        .map_err(|e| unreachable(&e))?;
    let mut c = handshake(ctx, link, route.pinned, route.pairing.clone()).await?;
    c.internet = Some(conn);
    Ok(c)
}

/// TLS, the peer check, Hello and any pairing proof, over any link.
async fn handshake(
    ctx: &Ctx,
    link: tls::BoxLink,
    pinned: Option<PeerId>,
    proof: Option<Proof>,
) -> Result<Connected, ConnectError> {
    let unreachable = |what: &str| {
        ConnectError::Unreachable(format!("The device could not be reached ({what})."))
    };
    let (mut stream, session) = tokio::time::timeout(HANDSHAKE, tls::connect(&ctx.tls, link))
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
    let ours =
        Hello::new(&ctx.device_name(), &ctx.platform, pairing).with_avatar(ctx.avatar().as_deref());
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
                addr: None,
                internet: None,
                direct: false,
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

#[cfg(test)]
mod racing {
    use std::{
        net::{IpAddr, Ipv4Addr, SocketAddr},
        time::{Duration, Instant},
    };

    use super::{order, race_tcp};

    fn addr(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    #[test]
    fn addresses_on_this_devices_networks_go_first_link_local_last() {
        let local = [IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20))];
        let mut addrs = vec![
            addr("[fe80::1]:7000"),
            addr("10.8.0.5:7000"),
            addr("[2001:db8::5]:7000"),
            addr("192.168.1.31:7000"),
        ];
        order(&mut addrs, &local);
        assert_eq!(
            addrs,
            vec![
                addr("192.168.1.31:7000"),
                addr("10.8.0.5:7000"),
                addr("[2001:db8::5]:7000"),
                addr("[fe80::1]:7000"),
            ]
        );
    }

    /// A dead address before the live one costs the stagger, not its
    /// whole timeout (it used to cost 4 s an address).
    #[tokio::test]
    async fn a_dead_address_does_not_hold_up_the_live_one() {
        let live = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let live_addr = live.local_addr().unwrap();
        // TEST-NET-1: never answers (dropped or unreachable).
        let dead = addr("192.0.2.1:7000");
        let started = Instant::now();
        let (won, _tcp) = race_tcp(vec![dead, live_addr], Duration::from_millis(250))
            .await
            .unwrap();
        assert_eq!(won, live_addr);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "took {:?}",
            started.elapsed()
        );
    }

    /// Nothing answers: an error, after the attempts' timeout at most.
    #[tokio::test]
    async fn no_address_answers() {
        let closed = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap()
        };
        assert!(
            race_tcp(vec![closed], Duration::from_millis(250))
                .await
                .is_err()
        );
        assert!(
            race_tcp(Vec::new(), Duration::from_millis(250))
                .await
                .is_err()
        );
    }
}
