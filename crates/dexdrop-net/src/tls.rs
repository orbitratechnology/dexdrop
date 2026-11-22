//! Mutual TLS 1.3 with self-signed device certificates.
//!
//! There is no certificate authority: each side presents its own self-signed
//! certificate, and the TLS handshake proves it holds the matching private
//! key (signatures are verified as usual). Who the peer *is* is decided by
//! its [`PeerId`], which the caller compares with the ID it expected (from
//! discovery, an invitation or the trust list) or shows to the user together
//! with the session's verification number.

use std::sync::Arc;

use dexdrop_core::{Identity, PeerId, identity::sas_digits, protocol::ALPN};
use rustls::{
    DigitallySignedStruct, DistinguishedName, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::{
        CryptoProvider, WebPkiSupportedAlgorithms, verify_tls12_signature, verify_tls13_signature,
    },
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime},
    server::danger::{ClientCertVerified, ClientCertVerifier},
};
use tokio::net::TcpStream;
use tokio_rustls::{TlsAcceptor, TlsConnector};

/// The SNI name used for every connection (identity comes from the cert).
pub const SERVER_NAME: &str = "dexdrop.invalid";

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// TLS configuration for both directions, built once per identity.
#[derive(Clone)]
pub struct TlsContext {
    pub acceptor: TlsAcceptor,
    pub connector: TlsConnector,
    /// The same configurations, for other transports over TLS 1.3 (QUIC).
    pub server_config: Arc<rustls::ServerConfig>,
    pub client_config: Arc<rustls::ClientConfig>,
}

impl TlsContext {
    pub fn new(identity: &Identity) -> Result<Self, rustls::Error> {
        let provider = provider();
        let algorithms = provider.signature_verification_algorithms;
        let cert = CertificateDer::from(identity.cert_der().to_vec());
        let key = || PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(identity.key_pkcs8().to_vec()));

        let mut server = rustls::ServerConfig::builder_with_provider(provider.clone())
            .with_protocol_versions(&[&rustls::version::TLS13])?
            .with_client_cert_verifier(Arc::new(AnyPeer { algorithms }))
            .with_single_cert(vec![cert.clone()], key())?;
        server.alpn_protocols = vec![ALPN.to_vec()];

        let mut client = rustls::ClientConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AnyPeer { algorithms }))
            .with_client_auth_cert(vec![cert], key())?;
        client.alpn_protocols = vec![ALPN.to_vec()];

        let (server, client) = (Arc::new(server), Arc::new(client));
        Ok(Self {
            acceptor: TlsAcceptor::from(server.clone()),
            connector: TlsConnector::from(client.clone()),
            server_config: server,
            client_config: client,
        })
    }
}

/// What the handshake established about the other side.
#[derive(Clone, Debug)]
pub struct PeerSession {
    pub peer_id: PeerId,
    /// The 6-digit number both sides can compare (FR-023).
    pub sas: String,
    /// Keying material unique to this TLS session, to bind proofs (code
    /// pairing) to it.
    pub binding: [u8; 32],
}

/// The client half of a TLS stream.
pub type ClientStream = tokio_rustls::client::TlsStream<TcpStream>;
/// The server half of a TLS stream.
pub type ServerStream = tokio_rustls::server::TlsStream<TcpStream>;

const SAS_LABEL: &[u8] = b"EXPORTER-dexdrop-sas";

fn session_of<D>(conn: &rustls::ConnectionCommon<D>) -> Result<PeerSession, std::io::Error> {
    let cert = conn
        .peer_certificates()
        .and_then(|c| c.first())
        .ok_or_else(|| std::io::Error::other("the peer presented no certificate"))?;
    let peer_id = PeerId::from_cert_der(cert).map_err(std::io::Error::other)?;
    let mut material = [0u8; 32];
    conn.export_keying_material(&mut material, SAS_LABEL, None)
        .map_err(std::io::Error::other)?;
    let mut binding = [0u8; 32];
    conn.export_keying_material(&mut binding, b"EXPORTER-dexdrop-pairing", None)
        .map_err(std::io::Error::other)?;
    Ok(PeerSession {
        peer_id,
        sas: sas_digits(&material),
        binding,
    })
}

/// Connects over TLS and returns the stream and who answered.
pub async fn connect(
    ctx: &TlsContext,
    tcp: TcpStream,
) -> std::io::Result<(ClientStream, PeerSession)> {
    tcp.set_nodelay(true).ok();
    let name = ServerName::try_from(SERVER_NAME).expect("valid server name");
    let stream = ctx.connector.connect(name, tcp).await?;
    let session = session_of(stream.get_ref().1)?;
    Ok((stream, session))
}

/// Accepts a TLS connection and returns the stream and who connected.
pub async fn accept(
    ctx: &TlsContext,
    tcp: TcpStream,
) -> std::io::Result<(ServerStream, PeerSession)> {
    tcp.set_nodelay(true).ok();
    let stream = ctx.acceptor.accept(tcp).await?;
    let session = session_of(stream.get_ref().1)?;
    Ok((stream, session))
}

/// Accepts any well-formed certificate whose key signs the handshake.
#[derive(Debug)]
struct AnyPeer {
    algorithms: WebPkiSupportedAlgorithms,
}

fn check_cert(cert: &CertificateDer<'_>) -> Result<(), rustls::Error> {
    PeerId::from_cert_der(cert)
        .map(|_| ())
        .map_err(|_| rustls::Error::InvalidCertificate(rustls::CertificateError::BadEncoding))
}

impl ServerCertVerifier for AnyPeer {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        check_cert(end_entity)?;
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

impl ClientCertVerifier for AnyPeer {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        check_cert(end_entity)?;
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn handshake_identifies_both_sides_and_agrees_on_sas() {
        let a = Identity::generate().unwrap();
        let b = Identity::generate().unwrap();
        let ctx_a = TlsContext::new(&a).unwrap();
        let ctx_b = TlsContext::new(&b).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            accept(&ctx_b, tcp).await.unwrap().1
        });
        let tcp = TcpStream::connect(addr).await.unwrap();
        let (_stream, client_view) = connect(&ctx_a, tcp).await.unwrap();
        let server_view = server.await.unwrap();
        assert_eq!(client_view.peer_id, b.peer_id());
        assert_eq!(server_view.peer_id, a.peer_id());
        assert_eq!(client_view.sas, server_view.sas);
    }
}
