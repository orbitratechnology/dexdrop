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
use crate::{
    fsutil,
    keystore::{self, KeyStorage},
};

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
    #[error("the system's credential store: {0}")]
    KeyStore(String),
}

/// This device's key pair and certificate.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
pub struct Identity {
    key_pkcs8: Vec<u8>,
    cert_der: Vec<u8>,
    storage: KeyStorage,
    peer_id: PeerId,
}

#[cfg(not(target_arch = "wasm32"))]
/// The identity file. Version 1 always held the key; version 2 holds it
/// only when it is not in the system's credential store.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Serialize, Deserialize)]
struct StoredIdentity {
    version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    key_pkcs8: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    key_in_system_store: bool,
    cert_der: String,
}

#[cfg(not(target_arch = "wasm32"))]
fn decode(text: &str) -> Result<Vec<u8>, IdentityError> {
    BASE64
        .decode(text.as_bytes())
        .map_err(|e| IdentityError::Certificate(e.to_string()))
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
        Self::from_parts(key.serialize_der(), cert.der().to_vec(), KeyStorage::File)
    }

    /// An identity from its parts, after checking that the key is the
    /// certificate's: a key from a store must not pair with another cert.
    fn from_parts(
        key_pkcs8: Vec<u8>,
        cert_der: Vec<u8>,
        storage: KeyStorage,
    ) -> Result<Self, IdentityError> {
        use rcgen::PublicKeyData;
        let peer_id = PeerId::from_cert_der(&cert_der)?;
        let key = rcgen::KeyPair::try_from(key_pkcs8.as_slice())
            .map_err(|e| IdentityError::Certificate(format!("private key: {e}")))?;
        if PeerId::from_spki(&key.subject_public_key_info()) != peer_id {
            return Err(IdentityError::Certificate(
                "the private key does not belong to the certificate".into(),
            ));
        }
        Ok(Self {
            key_pkcs8,
            cert_der,
            storage,
            peer_id,
        })
    }

    /// A new identity whose key is kept as `want` says, or in the file when
    /// the system's store does not work.
    fn generate_in(want: KeyStorage) -> Result<Self, IdentityError> {
        let mut identity = Self::generate()?;
        if want == KeyStorage::System
            && keystore::put_checked(&identity.peer_id.to_string(), &identity.key_pkcs8).is_ok()
        {
            identity.storage = KeyStorage::System;
        }
        Ok(identity)
    }

    /// Loads the identity stored at `path`, creating and storing one on first
    /// run, with the key where this platform keeps it
    /// ([`KeyStorage::platform_default`]).
    pub fn load_or_create(path: &Path) -> Result<Self, IdentityError> {
        Self::load_or_create_with(path, KeyStorage::platform_default())
    }

    /// [`Identity::load_or_create`], keeping a new key as `want` says. A key
    /// found in the file moves to the system's store when `want` is
    /// [`KeyStorage::System`] and the store works; until then it stays put.
    pub fn load_or_create_with(path: &Path, want: KeyStorage) -> Result<Self, IdentityError> {
        if let Some(stored) = fsutil::read_json::<StoredIdentity>(path)? {
            let cert = decode(&stored.cert_der)?;
            let account = PeerId::from_cert_der(&cert)?.to_string();
            if let Some(key) = &stored.key_pkcs8 {
                let mut identity = Self::from_parts(decode(key)?, cert, KeyStorage::File)?;
                if want == KeyStorage::System
                    && keystore::put_checked(&account, &identity.key_pkcs8).is_ok()
                {
                    // The file loses its copy only once the store holds it.
                    identity.storage = KeyStorage::System;
                    identity.save(path)?;
                }
                return Ok(identity);
            }
            if stored.key_in_system_store {
                // A store that does not answer is an error, not a reason
                // to replace the device's identity.
                if let Some(key) = keystore::get(&account).map_err(IdentityError::KeyStore)? {
                    return Self::from_parts(key, cert, KeyStorage::System);
                }
                // The key was removed from the store: start over.
            }
        }
        let identity = Self::generate_in(want)?;
        identity.save(path)?;
        Ok(identity)
    }

    /// Replaces the identity at `path` with a new one (PRD 9.9): a new peer
    /// ID, so devices that trusted the old one see a new device. The old key
    /// is removed from the system's store.
    pub fn rotate(path: &Path, want: KeyStorage) -> Result<Self, IdentityError> {
        let old = fsutil::read_json::<StoredIdentity>(path)?;
        let identity = Self::generate_in(want)?;
        identity.save(path)?;
        if let Some(old) = old
            && old.key_in_system_store
            && let Ok(cert) = decode(&old.cert_der)
            && let Ok(id) = PeerId::from_cert_der(&cert)
        {
            let _ = keystore::delete(&id.to_string());
        }
        Ok(identity)
    }

    /// Stores the identity at `path`: the certificate, and the key unless it
    /// is in the system's credential store.
    pub fn save(&self, path: &Path) -> Result<(), IdentityError> {
        let in_file = self.storage == KeyStorage::File;
        let stored = StoredIdentity {
            version: 2,
            key_pkcs8: in_file.then(|| BASE64.encode(&self.key_pkcs8)),
            key_in_system_store: !in_file,
            cert_der: BASE64.encode(&self.cert_der),
        };
        fsutil::write_json(path, &stored)?;
        Ok(())
    }

    /// Where the private key is kept.
    pub fn key_storage(&self) -> KeyStorage {
        self.storage
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
        let a = Identity::load_or_create_with(&path, KeyStorage::File).unwrap();
        let b = Identity::load_or_create_with(&path, KeyStorage::File).unwrap();
        assert_eq!(a.peer_id(), b.peer_id());
        assert_eq!(b.key_storage(), KeyStorage::File);
        assert_eq!(a.key_pkcs8(), b.key_pkcs8());
        let c = Identity::generate().unwrap();
        assert_ne!(a.peer_id(), c.peer_id());
    }

    #[test]
    fn a_version_1_file_still_loads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity.json");
        let id = Identity::generate().unwrap();
        let v1 = serde_json::json!({
            "version": 1,
            "key_pkcs8": BASE64.encode(id.key_pkcs8()),
            "cert_der": BASE64.encode(id.cert_der()),
        });
        std::fs::write(&path, v1.to_string()).unwrap();
        let loaded = Identity::load_or_create_with(&path, KeyStorage::File).unwrap();
        assert_eq!(loaded.peer_id(), id.peer_id());
    }

    #[test]
    fn a_key_that_is_not_the_certificates_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity.json");
        let (a, b) = (Identity::generate().unwrap(), Identity::generate().unwrap());
        let mixed = serde_json::json!({
            "version": 2,
            "key_pkcs8": BASE64.encode(a.key_pkcs8()),
            "cert_der": BASE64.encode(b.cert_der()),
        });
        std::fs::write(&path, mixed.to_string()).unwrap();
        let err = Identity::load_or_create_with(&path, KeyStorage::File).unwrap_err();
        assert!(err.to_string().contains("does not belong"), "{err}");
    }

    #[test]
    fn rotation_gives_a_new_peer_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity.json");
        let old = Identity::load_or_create_with(&path, KeyStorage::File).unwrap();
        let new = Identity::rotate(&path, KeyStorage::File).unwrap();
        assert_ne!(old.peer_id(), new.peer_id());
        let loaded = Identity::load_or_create_with(&path, KeyStorage::File).unwrap();
        assert_eq!(loaded.peer_id(), new.peer_id());
    }

    /// The system's store where it works (Windows in CI; Linux with a
    /// Secret Service), the file where it does not: either way the identity
    /// survives a reload, and a key in the store is not also in the file.
    #[test]
    fn the_system_store_keeps_the_key_or_the_file_does() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity.json");
        // An identity from before, key in the file: it moves when it can.
        let old = Identity::load_or_create_with(&path, KeyStorage::File).unwrap();
        let moved = Identity::load_or_create_with(&path, KeyStorage::System).unwrap();
        assert_eq!(moved.peer_id(), old.peer_id());
        // CI sets this where the store must work (Windows), so a silent
        // fall back to the file fails there.
        if std::env::var("DEXDROP_EXPECT_SYSTEM_KEYSTORE").is_ok_and(|v| v == "1") {
            assert_eq!(moved.key_storage(), KeyStorage::System);
        }
        let text = std::fs::read_to_string(&path).unwrap();
        match moved.key_storage() {
            KeyStorage::System => assert!(!text.contains("key_pkcs8"), "{text}"),
            KeyStorage::File => assert!(text.contains("key_pkcs8"), "{text}"),
        }
        let again = Identity::load_or_create_with(&path, KeyStorage::System).unwrap();
        assert_eq!(again.peer_id(), old.peer_id());
        assert_eq!(again.key_pkcs8(), old.key_pkcs8());
        assert_eq!(again.key_storage(), moved.key_storage());
        // Rotating removes the old key from the store.
        let new = Identity::rotate(&path, KeyStorage::System).unwrap();
        if moved.key_storage() == KeyStorage::System {
            assert_eq!(keystore::get(&old.peer_id().to_string()).unwrap(), None);
        }
        let _ = keystore::delete(&new.peer_id().to_string());
        eprintln!("key storage here: {}", moved.key_storage().describe());
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
