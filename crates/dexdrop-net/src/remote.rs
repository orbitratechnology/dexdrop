//! Transfers over the internet (PRD 6.8, Phase 6), through Iroh.
//!
//! Iroh dials a device by its public key: it finds the device's addresses
//! (DNS address lookup), punches through home routers for a direct QUIC
//! path, and falls back to a relay server when no direct path exists. The
//! relay forwards encrypted packets it cannot read.
//!
//! Iroh's key is the device's own Ed25519 key, so a device's internet
//! address is the public key in its certificate: a trusted device or an
//! invitation is enough to reach it from anywhere. DexDrop's own mutual TLS
//! (certificate pinning, the verification number, code proofs) runs inside
//! an Iroh stream exactly as over TCP, so every guarantee of local transfers
//! holds over the internet too; Iroh's encryption is a second layer.

use std::{
    net::SocketAddr,
    pin::Pin,
    task::{Context, Poll},
};

use dexdrop_core::Identity;
pub use dexdrop_core::api::Route;
use iroh::{
    Endpoint, EndpointAddr, EndpointId, RelayMode, RelayUrl, SecretKey,
    endpoint::{Connection, RecvStream, SendStream, presets},
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use crate::tls::BoxLink;

/// The application protocol on Iroh connections.
pub const ALPN: &[u8] = b"dexdrop/1";

/// Which relay servers carry traffic when no direct path exists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Relay {
    /// n0's public relays: free, rate-limited, without an SLA; for
    /// development (PRD 9.5 asks for the project's own in production).
    Public,
    /// The project's own relay (`iroh-relay`), by URL.
    Own(String),
    /// No relay: direct paths only (tests, and networks that forbid relays).
    Off,
}

/// How the internet transport is set up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteConfig {
    pub relay: Relay,
    /// Publish this device's addresses and look others up by key (n0's DNS
    /// address lookup). Off in tests, which give addresses directly.
    pub address_lookup: bool,
    /// Trust any certificate of the relay. Only for tests against a local
    /// relay with a self-signed certificate (feature `test-relay`); without
    /// that feature it does nothing.
    pub insecure_relay_for_tests: bool,
}

impl Default for RemoteConfig {
    fn default() -> Self {
        Self {
            relay: Relay::Public,
            address_lookup: true,
            insecure_relay_for_tests: false,
        }
    }
}

/// The Iroh key of this device: its Ed25519 key, checked against the
/// certificate so both name the same device.
pub fn secret_key(identity: &Identity) -> Result<SecretKey, String> {
    let seed = identity
        .ed25519_seed()
        .ok_or("the device key is not an Ed25519 key")?;
    let secret = SecretKey::from_bytes(&seed);
    if Some(*secret.public().as_bytes()) != identity.ed25519_key() {
        return Err("the device key does not match its certificate".into());
    }
    Ok(secret)
}

/// Starts this device's internet endpoint.
pub async fn bind(identity: &Identity, config: &RemoteConfig) -> Result<Endpoint, String> {
    #[cfg(target_os = "android")]
    if !crate::jvm::ensure_ndk_context() {
        return Err("no Android context in this process (run from a shell)".into());
    }
    let secret = secret_key(identity)?;
    let builder = if config.address_lookup {
        Endpoint::builder(presets::N0)
    } else {
        Endpoint::builder(presets::Minimal)
    };
    let relay = match &config.relay {
        Relay::Public => iroh::endpoint::default_relay_mode(),
        Relay::Own(url) => {
            let url: RelayUrl = url
                .parse()
                .map_err(|e| format!("the relay address {url} is not valid: {e}"))?;
            RelayMode::custom([url])
        }
        Relay::Off => RelayMode::Disabled,
    };
    #[cfg(feature = "test-relay")]
    let builder = if config.insecure_relay_for_tests {
        builder.ca_tls_config(iroh::tls::CaTlsConfig::insecure_skip_verify())
    } else {
        builder
    };
    builder
        .secret_key(secret)
        .alpns(vec![ALPN.to_vec()])
        .relay_mode(relay)
        .bind()
        .await
        .map_err(|e| format!("the internet transport could not start: {e}"))
}

/// The internet address of a device with this raw Ed25519 key, with any
/// hints known (a relay, direct addresses).
pub fn addr_of(key: &[u8; 32], relay: Option<&str>, direct: &[SocketAddr]) -> Option<EndpointAddr> {
    let id = EndpointId::from_bytes(key).ok()?;
    let mut addr = EndpointAddr::new(id);
    if let Some(url) = relay.and_then(|u| u.parse::<RelayUrl>().ok()) {
        addr = addr.with_relay_url(url);
    }
    for a in direct {
        addr = addr.with_ip_addr(*a);
    }
    Some(addr)
}

/// This endpoint's home relay, for invitations.
pub fn home_relay(endpoint: &Endpoint) -> Option<String> {
    endpoint.addr().relay_urls().next().map(|u| u.to_string())
}

/// Opens a stream to `addr` for a session.
pub async fn open(
    endpoint: &Endpoint,
    addr: EndpointAddr,
) -> Result<(BoxLink, Connection), String> {
    let conn = endpoint
        .connect(addr, ALPN)
        .await
        .map_err(|e| e.to_string())?;
    let (send, recv) = conn.open_bi().await.map_err(|e| e.to_string())?;
    let link = IrohLink {
        send,
        recv,
        _conn: conn.clone(),
    };
    Ok((Box::new(link), conn))
}

/// Accepts the session stream of an incoming connection.
pub async fn accept(conn: Connection) -> Result<BoxLink, String> {
    let (send, recv) = conn.accept_bi().await.map_err(|e| e.to_string())?;
    Ok(Box::new(IrohLink {
        send,
        recv,
        _conn: conn,
    }))
}

/// Whether the connection's packets go directly or through a relay now
/// (Iroh moves to a direct path when hole punching succeeds).
pub fn route(conn: &Connection) -> Route {
    let paths = conn.paths();
    let selected = paths.iter().find(|p| p.is_selected());
    match selected {
        Some(p) if p.is_ip() => Route::Direct,
        _ => Route::Relayed,
    }
}

/// Reports the route of a transfer's connection whenever it changes (Iroh
/// moves from a relay to a direct path when hole punching succeeds), until
/// the connection closes or the engine stops.
pub(crate) fn follow_route(
    ctx: std::sync::Arc<crate::engine::Ctx>,
    id: dexdrop_core::protocol::TransferId,
    conn: Connection,
) {
    tokio::spawn(async move {
        let mut last = None;
        while conn.close_reason().is_none() {
            let now = route(&conn);
            if last != Some(now) {
                last = Some(now);
                let message = crate::engine::Internal::Route { id, route: now };
                if ctx.internal.send(message).is_err() {
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    });
}

/// A QUIC stream pair as one byte stream. It holds the connection so the
/// stream lives as long as the session.
struct IrohLink {
    send: SendStream,
    recv: RecvStream,
    _conn: Connection,
}

impl AsyncRead for IrohLink {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.recv).poll_read(cx, buf)
    }
}

impl AsyncWrite for IrohLink {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.send)
            .poll_write(cx, buf)
            .map_err(std::io::Error::other)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.send).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.send).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_iroh_key_is_the_device_key() {
        let id = Identity::generate().unwrap();
        let secret = secret_key(&id).unwrap();
        assert_eq!(Some(*secret.public().as_bytes()), id.ed25519_key());
        assert_eq!(
            dexdrop_core::PeerId::from_ed25519_key(secret.public().as_bytes()),
            id.peer_id()
        );
    }
}
