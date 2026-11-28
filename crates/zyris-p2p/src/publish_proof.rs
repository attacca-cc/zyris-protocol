//! Proof that a node holds the iroh key it publishes: it signs a server-issued nonce, so the
//! rendezvous server never records (or relays for) a key the publisher cannot use.

use crate::fingerprint;

/// Prefixed to the nonce before signing, so a signature made here cannot be replayed as one for
/// any other protocol that signs a server-chosen string with the same key.
pub const PUBLISH_PROOF_DOMAIN: &[u8] = b"zyris-peer-publish-v1:";

fn message(nonce: &str) -> Vec<u8> {
    [PUBLISH_PROOF_DOMAIN, nonce.as_bytes()].concat()
}

/// Signs `DOMAIN || nonce` and returns the 64-byte ed25519 signature as lowercase hex.
pub fn sign_publish_challenge(secret: &iroh::SecretKey, nonce: &str) -> String {
    secret
        .sign(&message(nonce))
        .to_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Whether `signature` (hex) is `endpoint_id`'s signature over `DOMAIN || nonce`. Any parse
/// failure — of the id or the signature — is `false`, never a panic.
pub fn verify_publish_challenge(endpoint_id: &str, nonce: &str, signature: &str) -> bool {
    let Ok(key) = fingerprint::parse(endpoint_id) else {
        return false;
    };
    let Some(bytes) = decode_hex(signature) else {
        return false;
    };
    let Ok(bytes) = <[u8; 64]>::try_from(bytes) else {
        return false;
    };
    key.verify(&message(nonce), &iroh::Signature::from_bytes(&bytes))
        .is_ok()
}

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if !s.is_ascii() || s.len() % 2 != 0 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> iroh::SecretKey {
        iroh::SecretKey::generate()
    }

    #[test]
    fn signature_verifies_for_own_key() {
        let k = key();
        let sig = sign_publish_challenge(&k, "abc123");
        assert!(verify_publish_challenge(
            &k.public().to_string(),
            "abc123",
            &sig
        ));
    }

    #[test]
    fn signature_fails_for_other_key() {
        let (a, b) = (key(), key());
        let sig = sign_publish_challenge(&a, "abc123");
        assert!(!verify_publish_challenge(
            &b.public().to_string(),
            "abc123",
            &sig
        ));
    }

    #[test]
    fn signature_fails_for_other_nonce() {
        let k = key();
        let sig = sign_publish_challenge(&k, "abc123");
        assert!(!verify_publish_challenge(
            &k.public().to_string(),
            "abc124",
            &sig
        ));
    }

    #[test]
    fn garbage_signature_is_false_not_panic() {
        let id = key().public().to_string();
        for bad in ["zz", "", "é"] {
            assert!(!verify_publish_challenge(&id, "n", bad));
        }
        assert!(!verify_publish_challenge(
            "not-a-key",
            "n",
            &"00".repeat(64)
        ));
    }
}
