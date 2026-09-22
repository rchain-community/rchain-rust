//! Ed25519 signatures.
//!
//! Mirrors `crypto/src/main/scala/coop/rchain/crypto/signatures/Ed25519.scala`. The Scala wraps
//! kalium (libsodium); the Rust port wraps `ed25519-dalek`, using the non-strict `verify` to match
//! libsodium's malleable-signature acceptance.

use super::signatures_alg::SignaturesAlg;
use crate::errors::CryptoError;
use crate::private_key::PrivateKey;
use crate::public_key::PublicKey;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;

/// The Ed25519 algorithm.
pub struct Ed25519;

impl Ed25519 {
    /// Compute the 32-byte public key from a 32-byte secret (seed) key.
    pub fn to_public_bytes(sec: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let arr: [u8; 32] = sec.try_into().map_err(|_| CryptoError::InvalidLength {
            expected: 32,
            actual: sec.len(),
        })?;
        Ok(SigningKey::from_bytes(&arr)
            .verifying_key()
            .to_bytes()
            .to_vec())
    }

    /// Sign `data` with a 32-byte secret (seed) key, returning a 64-byte signature.
    pub fn sign_bytes(data: &[u8], sec: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let arr: [u8; 32] = sec.try_into().map_err(|_| CryptoError::InvalidLength {
            expected: 32,
            actual: sec.len(),
        })?;
        Ok(SigningKey::from_bytes(&arr).sign(data).to_bytes().to_vec())
    }

    /// Verify a 64-byte signature against a 32-byte public key.
    pub fn verify_bytes(data: &[u8], signature: &[u8], pub_key: &[u8]) -> bool {
        let Ok(pk) = <[u8; 32]>::try_from(pub_key) else {
            return false;
        };
        let Ok(vk) = VerifyingKey::from_bytes(&pk) else {
            return false;
        };
        let Ok(sig) = Signature::from_slice(signature) else {
            return false;
        };
        vk.verify(data, &sig).is_ok()
    }
}

impl SignaturesAlg for Ed25519 {
    fn verify(&self, data: &[u8], signature: &[u8], pub_key: &[u8]) -> bool {
        Ed25519::verify_bytes(data, signature, pub_key)
    }

    fn sign(&self, data: &[u8], sec: &[u8]) -> Result<Vec<u8>, CryptoError> {
        Ed25519::sign_bytes(data, sec)
    }

    fn to_public(&self, sec: &PrivateKey) -> Result<PublicKey, CryptoError> {
        Ok(PublicKey::new(Ed25519::to_public_bytes(sec.bytes())?))
    }

    fn new_key_pair(&self) -> (PrivateKey, PublicKey) {
        let signing = SigningKey::generate(&mut OsRng);
        let sec = signing.to_bytes().to_vec();
        let pub_key = signing.verifying_key().to_bytes().to_vec();
        (PrivateKey::new(sec), PublicKey::new(pub_key))
    }

    fn name(&self) -> &'static str {
        "ed25519"
    }

    fn sig_length(&self) -> usize {
        64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rchain_shared::base16;

    #[test]
    fn computes_public_key_from_secret_key() {
        let sec = base16::unsafe_decode(
            "b18e1d0045995ec3d010c387ccfeb984d783af8fbb0f40fa7db126d889f6dadd",
        );
        assert_eq!(
            base16::encode(&Ed25519::to_public_bytes(&sec).expect("compute ed25519 public key")),
            "77f48b59caeda77751ed138b0ec667ff50f8768c25d48309a8f386a2bad187fb"
        );
    }

    #[test]
    fn verifies_the_given_signature() {
        let data = base16::unsafe_decode(
            "916c7d1d268fc0e77c1bef238432573c39be577bbea0998936add2b50a653171ce18a542b0b7f96c1691a3be6031522894a8634183eda38798a0c5d5d79fbd01dd04a8646d71873b77b221998a81922d8105f892316369d5224c9983372d2313c6b1f4556ea26ba49d46e8b561e0fc76633ac9766e68e21fba7edca93c4c7460376d7f3ac22ff372c18f613f2ae2e856af40",
        );
        let sig = base16::unsafe_decode(
            "6bd710a368c1249923fc7a1610747403040f0cc30815a00f9ff548a896bbda0b4eb2ca19ebcf917f0f34200a9edbad3901b64ab09cc5ef7b9bcc3c40c0ff7509",
        );
        let pub_key = base16::unsafe_decode(
            "77f48b59caeda77751ed138b0ec667ff50f8768c25d48309a8f386a2bad187fb",
        );
        assert!(Ed25519::verify_bytes(&data, &sig, &pub_key));
    }

    #[test]
    fn creates_a_signature() {
        let data = base16::unsafe_decode(
            "916c7d1d268fc0e77c1bef238432573c39be577bbea0998936add2b50a653171ce18a542b0b7f96c1691a3be6031522894a8634183eda38798a0c5d5d79fbd01dd04a8646d71873b77b221998a81922d8105f892316369d5224c9983372d2313c6b1f4556ea26ba49d46e8b561e0fc76633ac9766e68e21fba7edca93c4c7460376d7f3ac22ff372c18f613f2ae2e856af40",
        );
        let expected = base16::unsafe_decode(
            "6bd710a368c1249923fc7a1610747403040f0cc30815a00f9ff548a896bbda0b4eb2ca19ebcf917f0f34200a9edbad3901b64ab09cc5ef7b9bcc3c40c0ff7509",
        );
        let sec = base16::unsafe_decode(
            "b18e1d0045995ec3d010c387ccfeb984d783af8fbb0f40fa7db126d889f6dadd",
        );
        assert_eq!(
            Ed25519::sign_bytes(&data, &sec).expect("ed25519 sign"),
            expected
        );
    }
}

#[cfg(test)]
mod error_paths {
    use super::*;

    use rchain_shared::base16;

    /// A secret key of the wrong length is an error, not a panic: the key material comes from a
    /// config file (`--validator-private-key`), so a truncated hex string must be reported.
    #[test]
    fn a_wrong_length_secret_key_is_an_error() {
        for bad in [vec![], vec![0u8; 31], vec![0u8; 33], vec![0u8; 64]] {
            assert!(
                Ed25519::to_public_bytes(&bad).is_err(),
                "{} bytes must be refused",
                bad.len()
            );
            assert!(Ed25519::sign_bytes(&[1, 2, 3], &bad).is_err());
        }
    }

    /// Verification is **total**: a malformed signature, a malformed key or a plain mismatch is
    /// `false`, never an error or a panic — a peer's byte string must not be able to make the
    /// verifier fail loudly.
    #[test]
    fn verification_answers_false_for_every_malformed_input() {
        let sec = base16::unsafe_decode(
            "b18e1d0045995ec3d010c387ccfeb984d783af8fbb0f40fa7db126d889f6dadd",
        );
        let pub_key = Ed25519::to_public_bytes(&sec).expect("public key");
        let data = b"the message";
        let sig = Ed25519::sign_bytes(data, &sec).expect("sign");

        assert!(
            Ed25519::verify_bytes(data, &sig, &pub_key),
            "the happy path"
        );
        // A different message.
        assert!(!Ed25519::verify_bytes(b"another message", &sig, &pub_key));
        // A truncated/oversized signature.
        assert!(!Ed25519::verify_bytes(data, &sig[..63], &pub_key));
        assert!(!Ed25519::verify_bytes(data, &[1u8; 65], &pub_key));
        assert!(!Ed25519::verify_bytes(data, &[], &pub_key));
        // A malformed public key.
        assert!(!Ed25519::verify_bytes(data, &sig, &pub_key[..31]));
        // A flipped signature bit (malleability of the *message*, not the encoding: Ed25519 is
        // canonical, so any change must fail).
        let mut tampered = sig.clone();
        tampered[0] ^= 0x01;
        assert!(!Ed25519::verify_bytes(data, &tampered, &pub_key));
    }

    /// An empty message is signable and verifiable — the edge case a "no data" signature hits.
    #[test]
    fn an_empty_message_round_trips() {
        let sec = base16::unsafe_decode(
            "b18e1d0045995ec3d010c387ccfeb984d783af8fbb0f40fa7db126d889f6dadd",
        );
        let pub_key = Ed25519::to_public_bytes(&sec).expect("public key");
        let sig = Ed25519::sign_bytes(&[], &sec).expect("sign the empty message");
        assert!(Ed25519::verify_bytes(&[], &sig, &pub_key));
        assert!(!Ed25519::verify_bytes(&[0], &sig, &pub_key));
    }

    /// **Ed25519 is deterministic** (RFC 8032): signing the same message with the same key twice
    /// gives the same bytes, unlike the ECDSA path (which needs its own nonce handling). A caller
    /// relying on a stable deploy signature depends on this.
    #[test]
    fn signing_is_deterministic() {
        let sec = base16::unsafe_decode(
            "b18e1d0045995ec3d010c387ccfeb984d783af8fbb0f40fa7db126d889f6dadd",
        );
        let a = Ed25519::sign_bytes(b"payload", &sec).expect("sign");
        let b = Ed25519::sign_bytes(b"payload", &sec).expect("sign");
        assert_eq!(a, b);
        assert_eq!(a.len(), Ed25519.sig_length());
    }

    /// The algorithm's own metadata is what the deploy wire format records: the name is the string
    /// peers compare and `sig_length` is the fixed 64 bytes.
    #[test]
    fn the_algorithm_reports_its_name_and_signature_length() {
        use crate::signatures::signatures_alg::SignaturesAlg;
        assert_eq!(Ed25519.name(), "ed25519");
        assert_eq!(Ed25519.sig_length(), 64);
        assert!(
            crate::signatures::signatures_alg::from_algorithm("ed25519").is_none(),
            "ed25519 is **deliberately not registered**: the Scala registry has the case commented \
             out (RCHAIN-3560, `SignaturesAlg.scala:21-24`) so ed25519 cannot sign deploys, and the \
             port reproduces that. `to_public_bytes`/`sign_bytes` remain callable for the tests and \
             for the Curve25519 key exchange, which is a different use"
        );
        assert_eq!(
            crate::signatures::signatures_alg::from_algorithm("SECP256K1").map(|a| a.name()),
            Some("secp256k1"),
            "the registry compares case-insensitively for the algorithms it does carry"
        );
        // A generated key pair verifies its own signatures.
        let (sec, pub_key) = Ed25519.new_key_pair();
        let sig = Ed25519.sign(&[7u8; 32], sec.bytes()).expect("sign");
        assert!(Ed25519.verify(&[7u8; 32], &sig, pub_key.bytes()));
    }
}
