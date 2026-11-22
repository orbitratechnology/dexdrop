//! The device's cryptographic identity (FR-002) and peer IDs.
//!
//! Each install generates an Ed25519 key pair on first run and a self-signed
//! certificate for it. TLS presents the certificate; peers identify each
//! other by the BLAKE3 hash of its SubjectPublicKeyInfo, never by name or IP.

#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
use std::{fmt, io, str::FromStr};

use data_encoding::BASE32_NOPAD;
#[cfg(not(target_arch = "wasm32"))]
use data_encoding::BASE64;
use serde::{Deserialize, Serialize};

#[cfg(not(target_arch = "wasm32"))]
use crate::fsutil;

/// A peer's stable identifier: BLAKE3 of its certificate's public key info.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PeerId(pub [u8; 32]);

impl PeerId {
    /// The peer ID of the certificate `cert_der`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn from_cert_der(cert_der: &[u8]) -> Result<Self, IdentityError> {
        let (_, cert) = x509_parser::parse_x509_certificate(cert_der)
            .map_err(|e| IdentityError::Certificate(e.to_string()))?;
        Ok(Self::from_spki(cert.tbs_certificate.subject_pki.raw))
    }

    /// The peer ID of a DER SubjectPublicKeyInfo.
    pub fn from_spki(spki_der: &[u8]) -> Self {
        let mut hasher = blake3::Hasher::new_derive_key("dexdrop 2026 peer id v1");
        hasher.update(spki_der);
        PeerId(*hasher.finalize().as_bytes())
    }

    /// A short, human-comparable fingerprint: `"k3xq-7m2d-p9ra"`.
    pub fn fingerprint(&self) -> String {
        let full = self.to_string();
        format!("{}-{}-{}", &full[0..4], &full[4..8], &full[8..12])
    }
}

impl fmt::Display for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&BASE32_NOPAD.encode(&self.0).to_ascii_lowercase())
    }
}

impl fmt::Debug for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PeerId({})", self.fingerprint())
    }
}

impl FromStr for PeerId {
    type Err = IdentityError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = BASE32_NOPAD
            .decode(s.trim().to_ascii_uppercase().as_bytes())
            .map_err(|_| IdentityError::InvalidPeerId)?;
        let bytes: [u8; 32] = bytes.try_into().map_err(|_| IdentityError::InvalidPeerId)?;
        Ok(PeerId(bytes))
    }
}

/// Errors loading, creating or reading identities.
#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    #[error("identity storage: {0}")]
    Io(#[from] io::Error),
    #[error("could not create the identity: {0}")]
    Generate(String),
    #[error("certificate: {0}")]
    Certificate(String),
    #[error("not a valid peer ID")]
    InvalidPeerId,
}

/// This device's key pair and certificate.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
pub struct Identity {
    key_pkcs8: Vec<u8>,
    cert_der: Vec<u8>,
    peer_id: PeerId,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Serialize, Deserialize)]
struct StoredIdentity {
    version: u32,
    key_pkcs8: String,
    cert_der: String,
}

#[cfg(not(target_arch = "wasm32"))]
impl Identity {
    /// A new random identity.
    pub fn generate() -> Result<Self, IdentityError> {
        let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519)
            .map_err(|e| IdentityError::Generate(e.to_string()))?;
        let mut params = rcgen::CertificateParams::new(vec!["dexdrop.invalid".to_owned()])
            .map_err(|e| IdentityError::Generate(e.to_string()))?;
        params.distinguished_name = rcgen::DistinguishedName::new();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "DexDrop device");
        let cert = params
            .self_signed(&key)
            .map_err(|e| IdentityError::Generate(e.to_string()))?;
        Self::from_parts(key.serialize_der(), cert.der().to_vec())
    }

    fn from_parts(key_pkcs8: Vec<u8>, cert_der: Vec<u8>) -> Result<Self, IdentityError> {
        let peer_id = PeerId::from_cert_der(&cert_der)?;
        Ok(Self {
            key_pkcs8,
            cert_der,
            peer_id,
        })
    }

    /// Loads the identity stored at `path`, creating and storing one on first
    /// run. The file is private to the app (mode 0600 on Unix).
    pub fn load_or_create(path: &Path) -> Result<Self, IdentityError> {
        if let Some(stored) = fsutil::read_json::<StoredIdentity>(path)? {
            let key = BASE64
                .decode(stored.key_pkcs8.as_bytes())
                .map_err(|e| IdentityError::Certificate(e.to_string()))?;
            let cert = BASE64
                .decode(stored.cert_der.as_bytes())
                .map_err(|e| IdentityError::Certificate(e.to_string()))?;
            return Self::from_parts(key, cert);
        }
        let identity = Self::generate()?;
        identity.save(path)?;
        Ok(identity)
    }

    /// Stores the identity at `path`.
    pub fn save(&self, path: &Path) -> Result<(), IdentityError> {
        let stored = StoredIdentity {
            version: 1,
            key_pkcs8: BASE64.encode(&self.key_pkcs8),
            cert_der: BASE64.encode(&self.cert_der),
        };
        fsutil::write_json(path, &stored)?;
        Ok(())
    }

    /// The PKCS#8 private key, DER.
    pub fn key_pkcs8(&self) -> &[u8] {
        &self.key_pkcs8
    }

    /// The self-signed certificate, DER.
    pub fn cert_der(&self) -> &[u8] {
        &self.cert_der
    }

    /// This device's peer ID.
    pub fn peer_id(&self) -> PeerId {
        self.peer_id
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl fmt::Debug for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never print the private key.
        f.debug_struct("Identity")
            .field("peer_id", &self.peer_id)
            .finish_non_exhaustive()
    }
}

/// The 6-digit short authentication string both sides show (FR-023),
/// derived from TLS keying material bound to the session.
pub fn sas_digits(keying_material: &[u8]) -> String {
    let hash = blake3::derive_key("dexdrop 2026 sas v1", keying_material);
    let n = u32::from_be_bytes([hash[0], hash[1], hash[2], hash[3]]) % 1_000_000;
    format!("{:03} {:03}", n / 1000, n % 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_round_trips_and_is_stable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity.json");
        let a = Identity::load_or_create(&path).unwrap();
        let b = Identity::load_or_create(&path).unwrap();
        assert_eq!(a.peer_id(), b.peer_id());
        let c = Identity::generate().unwrap();
        assert_ne!(a.peer_id(), c.peer_id());
    }

    #[test]
    fn peer_ids_parse_back() {
        let id = Identity::generate().unwrap().peer_id();
        let text = id.to_string();
        assert_eq!(text.len(), 52);
        assert_eq!(text.parse::<PeerId>().unwrap(), id);
        assert!("not-an-id".parse::<PeerId>().is_err());
        assert_eq!(id.fingerprint().len(), 14);
    }

    #[test]
    fn sas_is_six_digits() {
        let sas = sas_digits(b"material");
        assert_eq!(sas.len(), 7);
        assert_eq!(sas, sas_digits(b"material"));
        assert_ne!(sas, sas_digits(b"other"));
    }
}
