//! The receiving side of a connection: handshake, offer, the user's decision
//! and the session.

use std::{sync::Arc, time::Duration};

use dexdrop_core::{
    protocol::{Hello, Message, RejectReason},
    unix_now,
};
use futures_util::FutureExt;
use tokio::{io::AsyncReadExt, net::TcpStream, sync::oneshot};

use crate::{
    engine::{Ctx, Decision, Internal, OfferArrived, lock},
    session::{self, ReceivePlan, SessionError},
    tls,
    wire::{self, WireError},
};

const HANDSHAKE: Duration = Duration::from_secs(10);
const DECISION: Duration = Duration::from_secs(10 * 60);

pub(crate) async fn handle(ctx: Arc<Ctx>, tcp: TcpStream) {
    if let Err(err) = run(ctx, tcp).await {
        tracing::debug!("incoming connection ended: {err}");
    }
}

async fn run(ctx: Arc<Ctx>, tcp: TcpStream) -> Result<(), SessionError> {
    let (mut stream, peer) = tokio::time::timeout(HANDSHAKE, tls::accept(&ctx.tls, tcp))
        .await
        .map_err(|_| WireError::Timeout)?
        .map_err(WireError::from)?;

    let hello = match wire::read_msg_within(&mut stream, HANDSHAKE).await? {
        Message::Hello(hello) => hello,
        other => {
            return Err(SessionError::Protocol(format!(
                "expected Hello, got {}",
                other.name()
            )));
        }
    };
    let ours = Hello::new(&ctx.device_name(), &ctx.platform, None);
    if ours.negotiate(&hello).is_none() {
        wire::send(&mut stream, &Message::Reject(RejectReason::Incompatible)).await?;
        return Ok(());
    }
    let via_invitation = match &hello.pairing {
        Some(pairing) => {
            let redeemed = lock(&ctx.invitations).redeem(pairing, unix_now());
            if redeemed.is_err() {
                wire::send(
                    &mut stream,
                    &Message::Reject(RejectReason::InvitationInvalid),
                )
                .await?;
                return Ok(());
            }
            let _ = ctx.internal.send(Internal::PairingUsed {
                code: matches!(pairing, dexdrop_core::protocol::Pairing::Code(_)),
                by: hello.device_name.clone(),
            });
            true
        }
        None => false,
    };
    wire::send(&mut stream, &Message::Hello(ours)).await?;

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
        peer_name: hello.device_name.clone(),
        peer_platform: hello.platform.clone(),
        sas: peer.sas.clone(),
        via_invitation,
        offer: offer.clone(),
        decide,
    };
    if ctx
        .internal
        .send(Internal::Offer(Box::new(arrived)))
        .is_err()
    {
        return Ok(());
    }

    // The sender sends nothing until it gets an answer: anything readable
    // means it cancelled or went away.
    let decision = {
        let mut probe = [0u8; 1];
        tokio::select! {
            decision = decision => decision.ok(),
            _ = stream.read(&mut probe) => None,
            _ = tokio::time::sleep(DECISION) => None,
        }
    };
    match decision {
        None => {
            let _ = ctx.internal.send(Internal::RequestGone(id));
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
