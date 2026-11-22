//! Code pairing as a PAKE (SPAKE2).
//!
//! A typed code is short (40 bits), so it must never be sent to a device
//! that merely claims to show one, nor be checkable offline. Both sides run
//! SPAKE2 with the code; each then proves it derived the same key with a
//! MAC over keying material of this TLS session. A device that does not
//! know the code cannot produce the proof, learns nothing it could test
//! offline, and every failed attempt counts towards the receiver's limit.

use spake2::{Ed25519Group, Identity, Password, Spake2};

const IDENTITY: &[u8] = b"dexdrop pairing v1";

/// A SPAKE2 exchange in progress.
pub struct Exchange(Spake2<Ed25519Group>);

/// Starts an exchange for `code` (normalized); returns the message to send.
pub fn start(code: &str) -> (Exchange, Vec<u8>) {
    let (state, msg) = Spake2::<Ed25519Group>::start_symmetric(
        &Password::new(code.as_bytes()),
        &Identity::new(IDENTITY),
    );
    (Exchange(state), msg)
}

impl Exchange {
    /// The shared key, if `their_msg` is well-formed. (A wrong code still
    /// yields a key; only the proofs tell.)
    pub fn finish(self, their_msg: &[u8]) -> Option<[u8; 32]> {
        let key = self.0.finish(their_msg).ok()?;
        Some(blake3::derive_key("dexdrop 2026 pake key v1", &key))
    }
}

/// Which side a proof is from: the two proofs must differ.
#[derive(Clone, Copy)]
pub enum Role {
    Sender,
    Receiver,
}

/// Proof of knowing the code, bound to this TLS session.
pub fn proof(key: &[u8; 32], role: Role, binding: &[u8; 32]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new_keyed(key);
    hasher.update(match role {
        Role::Sender => b"sender",
        Role::Receiver => b"receiver",
    });
    hasher.update(binding);
    *hasher.finalize().as_bytes()
}

/// Compares proofs in constant time.
pub fn matches(a: &[u8; 32], b: &[u8; 32]) -> bool {
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_code_same_key_different_code_different_key() {
        let binding = [9u8; 32];
        let (a, msg_a) = start("K7P2QX9M");
        let (b, msg_b) = start("K7P2QX9M");
        let (ka, kb) = (a.finish(&msg_b).unwrap(), b.finish(&msg_a).unwrap());
        assert_eq!(ka, kb);
        assert!(matches(
            &proof(&ka, Role::Sender, &binding),
            &proof(&kb, Role::Sender, &binding)
        ));
        assert!(!matches(
            &proof(&ka, Role::Sender, &binding),
            &proof(&kb, Role::Receiver, &binding)
        ));
        assert!(!matches(
            &proof(&ka, Role::Sender, &binding),
            &proof(&kb, Role::Sender, &[8; 32])
        ));

        let (c, msg_c) = start("K7P2QX9M");
        let (d, msg_d) = start("00000000");
        assert_ne!(c.finish(&msg_d).unwrap(), d.finish(&msg_c).unwrap());
        let (e, _) = start("K7P2QX9M");
        assert!(e.finish(b"garbage").is_none());
    }
}
