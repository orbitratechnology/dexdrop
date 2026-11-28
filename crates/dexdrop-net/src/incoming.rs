//! The receiving side of a connection: handshake, offer, the user's decision
//! and the session.

use std::{sync::Arc, time::Duration};

use dexdrop_core::protocol::{Pairing, clean_name};
use dexdrop_core::{
    protocol::{Hello, Message, RejectReason},
    unix_now,
};
use futures_util::FutureExt;
use tokio::{io::AsyncReadExt, sync::oneshot};

use crate::{
    engine::{Ctx, Decision, Internal, OfferArrived, lock},
    pairing::{self, Role},
    session::{self, ReceivePlan, SessionError},
    tls,
    wire::{self, WireError},
};

const HANDSHAKE: Duration = Duration::from_secs(10);
const DECISION: Duration = Duration::from_secs(10 * 60);

/// How a connection arrived.
pub(crate) enum Via {
    /// The local network or a device-to-device link (TCP), and the other
    /// end's address, which says which.
    Nearby(std::net::SocketAddr),
    /// The internet (Iroh); the connection, to check the sender's key and
    /// follow the route.
    Internet(iroh::endpoint::Connection),
}

pub(crate) async fn handle(ctx: Arc<Ctx>, link: tls::BoxLink, via: Via) {
    if let Err(err) = run(ctx, link, via).await {
        tracing::debug!("incoming connection ended: {err}");
    }
}

async fn run(ctx: Arc<Ctx>, link: tls::BoxLink, via: Via) -> Result<(), SessionError> {
    let (mut stream, peer) = tokio::time::timeout(HANDSHAKE, tls::accept(&ctx.tls, link))
        .await
        .map_err(|_| WireError::Timeout)?
        .map_err(WireError::from)?;
    // Over the internet, the device that dialed (its Iroh key) must be the
    // one in the TLS certificate.
    if let Via::Internet(conn) = &via
        && dexdrop_core::PeerId::from_ed25519_key(conn.remote_id().as_bytes()) != peer.peer_id
    {
        return Err(SessionError::Protocol(
            "the internet key and the certificate name different devices".into(),
        ));
    }
    let route = match &via {
        Via::Nearby(peer) => crate::link::local_route(*peer),
        Via::Internet(conn) => crate::remote::route(conn),
    };

    let hello = match wire::read_msg_within(&mut stream, HANDSHAKE).await? {
        Message::Hello(hello) => hello,
        other => {
            return Err(SessionError::Protocol(format!(
                "expected Hello, got {}",
                other.name()
            )));
        }
    };
    let ours =
        Hello::new(&ctx.device_name(), &ctx.platform, None).with_avatar(ctx.avatar().as_deref());
    if ours.negotiate(&hello).is_none() {
        wire::send(&mut stream, &Message::Reject(RejectReason::Incompatible)).await?;
        return Ok(());
    }
    let mut ours = ours;
    let mut via_invitation = false;
    // A code attempt in progress: the code and the SPAKE2 key.
    let mut attempt: Option<(String, [u8; 32])> = None;
    match &hello.pairing {
        None => {}
        Some(Pairing::Token(_)) => {
            let redeemed =
                lock(&ctx.invitations).redeem(hello.pairing.as_ref().expect("some"), unix_now());
            if redeemed.is_err() {
                wire::send(
                    &mut stream,
                    &Message::Reject(RejectReason::InvitationInvalid),
                )
                .await?;
                return Ok(());
            }
            via_invitation = true;
            let _ = ctx.internal.send(Internal::PairingUsed {
                code: false,
                by: clean_name(&hello.device_name),
            });
        }
        Some(Pairing::Pake(their_msg)) => {
            let Some(code) = lock(&ctx.invitations).begin_code_attempt(unix_now()) else {
                wire::send(
                    &mut stream,
                    &Message::Reject(RejectReason::InvitationInvalid),
                )
                .await?;
                return Ok(());
            };
            let (exchange, our_msg) = pairing::start(&code);
            let Some(key) = exchange.finish(their_msg) else {
                lock(&ctx.invitations).end_code_attempt(&code, false);
                wire::send(
                    &mut stream,
                    &Message::Reject(RejectReason::InvitationInvalid),
                )
                .await?;
                return Ok(());
            };
            ours.pairing = Some(Pairing::PakeReply {
                msg: our_msg,
                confirm: pairing::proof(&key, Role::Receiver, &peer.binding),
            });
            attempt = Some((code, key));
        }
        // Codes are never accepted in the clear, nor replies from a sender.
        Some(_) => {
            wire::send(
                &mut stream,
                &Message::Reject(RejectReason::InvitationInvalid),
            )
            .await?;
            return Ok(());
        }
    }
    wire::send(&mut stream, &Message::Hello(ours)).await?;
    if let Some((code, key)) = attempt {
        // Only a correct proof from the sender uses the code; anything else
        // (a wrong proof, silence, a disconnect) was a failed attempt.
        let expected = pairing::proof(&key, Role::Sender, &peer.binding);
        let proven = matches!(
            wire::read_msg_within(&mut stream, HANDSHAKE).await,
            Ok(Message::PairingConfirm(theirs)) if pairing::matches(&theirs, &expected)
        );
        lock(&ctx.invitations).end_code_attempt(&code, proven);
        if !proven {
            let _ = wire::send(
                &mut stream,
                &Message::Reject(RejectReason::InvitationInvalid),
            )
            .await;
            return Ok(());
        }
        via_invitation = true;
        let _ = ctx.internal.send(Internal::PairingUsed {
            code: true,
            by: clean_name(&hello.device_name),
        });
    }

    let offer = match session::read_offer(&mut stream).await? {
        Ok(offer) => offer,
        Err(reason) => {
            wire::send(&mut stream, &Message::Reject(reason)).await?;
            return Ok(());
        }
    };
    let id = offer.offer.transfer_id;
    let (decide, decision) = oneshot::channel();
    let arrived = OfferArrived {
        peer_id: peer.peer_id,
        peer_name: clean_name(&hello.device_name),
        peer_platform: hello.platform.clone(),
        sas: peer.sas.clone(),
        via_invitation,
        offer: offer.clone(),
        decide,
        route,
        key: peer.key,
        avatar: dexdrop_core::avatar::Avatar::clean(hello.avatar.as_deref()),
    };
    if ctx
        .internal
        .send(Internal::Offer(Box::new(arrived)))
        .is_err()
    {
        return Ok(());
    }
    if let Via::Internet(conn) = &via {
        crate::remote::follow_route(ctx.clone(), id, conn.clone());
    }

    // The sender sends nothing until it gets an answer: anything readable
    // means it cancelled or went away.
    let mut decision = decision;
    let decided = {
        let mut probe = [0u8; 1];
        tokio::select! {
            decided = &mut decision => decided.ok(),
            _ = stream.read(&mut probe) => None,
            _ = tokio::time::sleep(DECISION) => None,
        }
    };
    // The user may have answered at the very moment the sender left or the
    // wait ran out: an acceptance must still end the transfer it started.
    let decided = decided.or_else(|| decision.try_recv().ok());
    match decided {
        None => {
            let _ = ctx.internal.send(Internal::RequestGone(id));
            Ok(())
        }
        Some(Decision::Accept { journal, .. }) if gone_while_deciding(&mut stream) => {
            let _ = ctx.internal.send(Internal::IncomingEnded {
                id,
                result: Err("The other device left".into()),
                journal,
            });
            Ok(())
        }
        Some(Decision::Reject(reason)) => {
            wire::send(&mut stream, &Message::Reject(reason)).await?;
            Ok(())
        }
        Some(Decision::Accept {
            journal,
            live,
            control,
        }) => {
            // From here the engine shows the transfer as running: whatever
            // happens, it must hear how the session ended.
            let fallback = (*journal).clone();
            let ended = async {
                wire::send(&mut stream, &Message::Accept).await?;
                let plan = ReceivePlan {
                    offer,
                    journal: *journal,
                    records: ctx.state.incoming(),
                    max_rate: ctx.max_receive_rate,
                };
                let (result, journal) = session::run_receiver(stream, plan, live, control).await;
                Ok::<_, SessionError>((result, journal))
            };
            let (result, journal) = match std::panic::AssertUnwindSafe(ended).catch_unwind().await {
                Ok(Ok((result, journal))) => (result.map_err(|e| e.to_string()), journal),
                Ok(Err(err)) => (Err(err.to_string()), fallback),
                Err(_) => (
                    Err("an internal error stopped the transfer".to_owned()),
                    fallback,
                ),
            };
            let _ = ctx.internal.send(Internal::IncomingEnded {
                id,
                result,
                journal: Box::new(journal),
            });
            Ok(())
        }
    }
}

/// Whether the sender went away while the user was deciding: the stream
/// already ended. (Checked without waiting.)
fn gone_while_deciding(stream: &mut crate::tls::ServerStream) -> bool {
    use std::task::{Context, Poll};
    use tokio::io::{AsyncRead, ReadBuf};
    let waker = futures_util::task::noop_waker();
    let mut cx = Context::from_waker(&waker);
    let mut byte = [0u8; 1];
    let mut buf = ReadBuf::new(&mut byte);
    // Ready with no bytes (end of stream), data (a Cancel) or an error all
    // mean the sender is not waiting for an answer any more.
    matches!(
        std::pin::Pin::new(stream).poll_read(&mut cx, &mut buf),
        Poll::Ready(_)
    )
}
