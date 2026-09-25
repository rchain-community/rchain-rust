//! Ethereum personal signatures over secp256k1.
//!
//! Mirrors `crypto/src/main/scala/coop/rchain/crypto/signatures/Secp256k1Eth.scala`. Identical to
//! `Secp256k1` except the signature format is raw 64-byte RS rather than DER.

use super::secp256k1::Secp256k1;
use super::signatures_alg::SignaturesAlg;
use crate::errors::CryptoError;
use crate::private_key::PrivateKey;
use crate::public_key::PublicKey;
use crate::util::certificate_helper;

/// The secp256k1 "eth" algorithm.
pub struct Secp256k1Eth;

/// Decode a DER signature into the raw 64-byte RS form, as a *value*.
///
/// A decode failure here was flattened by `unwrap_or_default()` in the sign path, and the measurement
/// is sharper than the shape suggests: `Vec::default()` is the **empty** vector, so `sign` returned
/// `Ok(vec![])` — zero bytes, where `sig_length()` promises 64, and not even signature-shaped. That is
/// the register's §6 class, "a failure read as a value", invisible to the `silent` scan because its
/// call shape is `unwrap_or_default`. The refusal is a named error now, so a malformed frame can only
/// be a refusal.
fn signature_rs_from_der(der: &[u8]) -> Result<Vec<u8>, CryptoError> {
    certificate_helper::decode_signature_der_to_rs(der).map_err(CryptoError::InvalidSignatureFormat)
}

impl SignaturesAlg for Secp256k1Eth {
    fn verify(&self, data: &[u8], signature_rs: &[u8], pub_key: &[u8]) -> bool {
        match certificate_helper::encode_signature_rs_to_der(signature_rs) {
            Ok(der) => Secp256k1::verify_bytes(data, &der, pub_key),
            // DER conversion error silently returns false (only for empty input, per the Scala).
            Err(_) => false,
        }
    }

    fn sign(&self, data: &[u8], sec: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let der = Secp256k1::sign_bytes(data, sec)?;
        signature_rs_from_der(&der)
    }

    fn to_public(&self, sec: &PrivateKey) -> Result<PublicKey, CryptoError> {
        Ok(PublicKey::new(Secp256k1::to_public_bytes(sec.bytes())?))
    }

    fn new_key_pair(&self) -> (PrivateKey, PublicKey) {
        Secp256k1.new_key_pair()
    }

    fn name(&self) -> &'static str {
        "secp256k1:eth"
    }

    fn sig_length(&self) -> usize {
        Secp256k1.sig_length()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_short_eth_signature_returns_false_without_panicking() {
        let data = b"data";
        let pub_key = [0u8; 33];
        // A 1-byte and a 31-byte RS signature must be rejected by the DER encoder without
        // panicking (previously `split_at` on a short slice reached `der_integer(&[])`).
        assert!(!Secp256k1Eth.verify(data, &[0u8], &pub_key));
        assert!(!Secp256k1Eth.verify(data, &[0u8; 31], &pub_key));
    }

    /// **H2c's measurement, kept as a guard.** It ran first as a probe over 64 key pairs: no result was
    /// ever the *empty* signature `unwrap_or_default()` would have produced — which is the
    /// *measurement* that the flattening was not reached by well-formed signatures, not an assumption.
    /// The reason is structural: the signer emits minimal DER and the decoder strips DER's sign
    /// padding (`left_pad_32`), so the decode of a signature this code just produced cannot fail. The
    /// guard keeps that true, and pins the shape (raw 64-byte RS that verifies, never zeros).
    #[test]
    fn sign_returns_a_verifying_64_byte_signature_and_never_zeros() {
        for i in 0..32u8 {
            let (sec, pub_key) = Secp256k1Eth.new_key_pair();
            let data = [i; 32];
            let sig = Secp256k1Eth.sign(&data, sec.bytes()).expect("sign");
            assert_eq!(sig.len(), 64, "raw RS is 64 bytes at {i}");
            assert_ne!(
                sig,
                vec![0u8; 64],
                "a signature, not the flattened failure ({i})"
            );
            assert!(
                Secp256k1Eth.verify(&data, &sig, pub_key.bytes()),
                "and it verifies at {i}"
            );
        }
    }

    /// **H2c's fence, at the seam the fix created.** A malformed DER is refused rather than flattened:
    /// the decoder refuses it, the old expression made an **empty** signature of that refusal (measured
    /// — `Vec::default()`, not 64 zeros), and the sign path's own helper now returns the refusal as
    /// `CryptoError::InvalidSignatureFormat`. The well-formed case is the control, so this test can
    /// fail.
    #[test]
    fn a_malformed_der_is_refused_rather_than_becoming_an_empty_signature() {
        let malformed: &[u8] = &[0xFF, 0x01, 0x02];
        let refusal = certificate_helper::decode_signature_der_to_rs(malformed);
        assert!(refusal.is_err(), "the decoder refuses a non-DER frame");
        assert_eq!(
            refusal.unwrap_or_default(),
            Vec::<u8>::new(),
            "and that refusal, flattened, was an *empty* signature where 64 bytes are promised"
        );

        let err = signature_rs_from_der(malformed).expect_err("refused as a value");
        assert!(
            matches!(err, CryptoError::InvalidSignatureFormat(_)),
            "{err:?}"
        );

        // The control: the signer's own DER is accepted and decoded to 64 bytes.
        let (sec, _pub_key) = Secp256k1Eth.new_key_pair();
        let der = Secp256k1::sign_bytes(&[9u8; 32], sec.bytes()).expect("sign");
        assert_eq!(
            signature_rs_from_der(&der)
                .expect("a well-formed DER")
                .len(),
            64
        );
    }
}
