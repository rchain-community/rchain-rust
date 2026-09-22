//! The `SignaturesAlg` abstraction.
//!
//! Mirrors `crypto/src/main/scala/coop/rchain/crypto/signatures/SignaturesAlg.scala`.

use crate::errors::CryptoError;
use crate::private_key::PrivateKey;
use crate::public_key::PublicKey;

/// A digital signature algorithm.
pub trait SignaturesAlg {
    /// Verify `signature` over `data` against the public key `pub_key`.
    fn verify(&self, data: &[u8], signature: &[u8], pub_key: &[u8]) -> bool;

    /// Sign `data` with the secret key `sec`.
    fn sign(&self, data: &[u8], sec: &[u8]) -> Result<Vec<u8>, CryptoError>;

    /// Compute the public key corresponding to `sec`.
    fn to_public(&self, sec: &PrivateKey) -> Result<PublicKey, CryptoError>;

    /// Generate a fresh (private, public) key pair.
    fn new_key_pair(&self) -> (PrivateKey, PublicKey);

    /// The algorithm name.
    fn name(&self) -> &'static str;

    /// The signature length in bytes.
    fn sig_length(&self) -> usize;
}

/// Resolve an algorithm by (case-insensitive) name.
///
/// Ed25519 is deliberately disabled (RCHAIN-3560); only `"secp256k1"` and `"secp256k1:eth"` are
/// available, matching the Scala `SignaturesAlg.apply`.
pub fn from_algorithm(algorithm: &str) -> Option<&'static dyn SignaturesAlg> {
    match algorithm.to_ascii_lowercase().as_str() {
        // case Ed25519.name => Some(Ed25519) — disabled
        "secp256k1" => Some(&super::secp256k1::Secp256k1),
        "secp256k1:eth" => Some(&super::secp256k1_eth::Secp256k1Eth),
        _ => None,
    }
}

/// The secp256k1 curve order `n` (the scalar field modulus), as a 32-byte big-endian integer.
///
/// Hardcoded: `n = FFFFFFFF FFFFFFFF FFFFFFFF FFFFFFFE BAAEDCE6 AF48A03B BFD25E8C D0364141`.
/// `k256` does not expose the order as a clean `const` surface (only through `Scalar`'s internal
/// `BigUint` helper), so the fixed value is inlined here.
const SECP256K1_ORDER: [u8; 32] = [
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFE,
    0xBA, 0xAE, 0xDC, 0xE6, 0xAF, 0x48, 0xA0, 0x3B, 0xBF, 0xD2, 0x5E, 0x8C, 0xD0, 0x36, 0x41, 0x41,
];

/// `n / 2` (floor), the low-S threshold.
const SECP256K1_ORDER_HALF: [u8; 32] = [
    0x7F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0x5D, 0x57, 0x6E, 0x73, 0x57, 0xA4, 0x50, 0x1D, 0xDF, 0xE9, 0x2F, 0x46, 0x68, 0x1B, 0x20, 0xA0,
];

/// True when the 32-byte big-endian scalar `s` is in the high half of the curve order
/// (i.e. `s > n/2`), the signature-malleability condition normalized away by
/// [`normalize_signature_low_s`].
fn is_high_s(s: &[u8]) -> bool {
    for (b, half) in s.iter().zip(SECP256K1_ORDER_HALF.iter()) {
        match b.cmp(half) {
            std::cmp::Ordering::Less => return false,
            std::cmp::Ordering::Greater => return true,
            std::cmp::Ordering::Equal => {}
        }
    }
    false
}

/// Compute `n - s` (mod n) as a 32-byte big-endian integer, for `s <= n`.
fn negate_mod_order(s: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let mut borrow = 0i16;
    for i in (0..32).rev() {
        let mut diff = SECP256K1_ORDER[i] as i16 - s[i] as i16 - borrow;
        if diff < 0 {
            diff += 256;
            borrow = 1;
        } else {
            borrow = 0;
        }
        out[i] = diff as u8;
    }
    out
}

/// Replace the `s` half of a 64-byte RS signature with its low-S twin, leaving `r` unchanged.
fn normalize_rs_low_s(rs: &[u8]) -> Vec<u8> {
    if rs.len() != 64 {
        return rs.to_vec();
    }
    let (r, s) = rs.split_at(32);
    let mut out = Vec::with_capacity(64);
    out.extend_from_slice(r);
    if is_high_s(s) {
        out.extend_from_slice(&negate_mod_order(s));
    } else {
        out.extend_from_slice(s);
    }
    out
}

/// Normalize a secp256k1 signature to its low-S form (idempotent, best-effort).
///
/// For `"secp256k1"` (DER) and `"secp256k1:eth"` (raw 64-byte RS) the `s` half is replaced with
/// `n - s` whenever `s > n/2`, removing the high-S malleability. Any other algorithm — or a
/// signature that fails to decode — is returned unchanged. This only canonicalizes the
/// representation; it does not change verification semantics (both the high-S and low-S forms
/// verify against the same public key).
pub fn normalize_signature_low_s(algorithm: &str, signature: &[u8]) -> Vec<u8> {
    match algorithm.to_ascii_lowercase().as_str() {
        "secp256k1" => {
            let rs = match crate::util::certificate_helper::decode_signature_der_to_rs(signature) {
                Ok(rs) => rs,
                Err(_) => return signature.to_vec(),
            };
            let normalized = normalize_rs_low_s(&rs);
            crate::util::certificate_helper::encode_signature_rs_to_der(&normalized)
                .unwrap_or_else(|_| signature.to_vec())
        }
        "secp256k1:eth" => {
            if signature.len() != 64 {
                return signature.to_vec();
            }
            normalize_rs_low_s(signature)
        }
        _ => signature.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::certificate_helper::{decode_signature_der_to_rs, encode_signature_rs_to_der};
    use rchain_shared::base16;

    /// A low-S `secp256k1` signature from `secp256k1.rs`'s own vectors, so this module's tests and
    /// the algorithm's stay pinned to the same bytes.
    const LOW_S_DER: &str = "3044022079BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F817980220294F14E883B3F525B5367756C2A11EF6CF84B730B36C17CB0C56F0AAB2C98589";

    fn rs_of(der: &[u8]) -> Vec<u8> {
        decode_signature_der_to_rs(der).expect("a well-formed DER signature")
    }

    /// The registry: names resolve case-insensitively to the algorithm that reports that name back,
    /// and **`ed25519` is deliberately absent** (RCHAIN-3560). A lookup that started resolving it
    /// would put a disabled algorithm back on the signature-verification path.
    #[test]
    fn the_registry_resolves_the_two_enabled_algorithms_and_refuses_ed25519() {
        for name in ["secp256k1", "SECP256K1", "Secp256k1"] {
            assert_eq!(
                from_algorithm(name).map(|a| a.name()),
                Some("secp256k1"),
                "{name}"
            );
        }
        for name in ["secp256k1:eth", "SECP256K1:ETH", "Secp256k1:Eth"] {
            assert_eq!(
                from_algorithm(name).map(|a| a.name()),
                Some("secp256k1:eth"),
                "{name}"
            );
        }
        assert!(
            from_algorithm("ed25519").is_none(),
            "disabled by RCHAIN-3560"
        );
        assert!(from_algorithm("").is_none());
        assert!(from_algorithm("secp256r1").is_none());
        assert!(from_algorithm("secp256k1 ").is_none(), "no trimming");
    }

    /// The declared signature lengths: 32 for secp256k1's `s` half (the DER wrapper is carried
    /// separately) and the same for the eth variant, which shares the curve.
    #[test]
    fn the_declared_signature_lengths_are_the_curve_element_size() {
        assert_eq!(from_algorithm("secp256k1").expect("alg").sig_length(), 32);
        assert_eq!(
            from_algorithm("secp256k1:eth").expect("alg").sig_length(),
            32
        );
    }

    /// `n/2` is the low-S threshold and the comparison is **strict**: exactly `n/2` is already low-S
    /// (`n` is odd, so `n/2` has no twin), one above it is high.
    #[test]
    fn is_high_s_is_strict_at_the_half_order_boundary() {
        assert!(!is_high_s(&SECP256K1_ORDER_HALF), "n/2 is not above n/2");
        assert!(!is_high_s(&[0u8; 32]), "zero is low");
        assert!(is_high_s(&SECP256K1_ORDER), "n exceeds n/2");

        let mut above = SECP256K1_ORDER_HALF;
        above[31] += 1;
        assert!(is_high_s(&above), "n/2 + 1 is the first high value");

        let mut below = SECP256K1_ORDER_HALF;
        below[31] -= 1;
        assert!(!is_high_s(&below));

        // The comparison is over the whole 32-byte scalar, not just its tail: the decisive byte is
        // the first that differs, so a high head wins even with a low tail.
        let mut high_head = SECP256K1_ORDER_HALF;
        high_head[0] += 1;
        high_head[31] = 0;
        assert!(is_high_s(&high_head));
    }

    /// `negate_mod_order` is `n − s`: it maps the order to zero, is an involution, and preserves the
    /// 32-byte big-endian width.
    #[test]
    fn negate_mod_order_is_n_minus_s() {
        assert_eq!(negate_mod_order(&SECP256K1_ORDER), [0u8; 32], "n − n = 0");
        assert_eq!(negate_mod_order(&[0u8; 32]), SECP256K1_ORDER, "n − 0 = n");

        let mut one = [0u8; 32];
        one[31] = 1;
        let mut n_minus_one = SECP256K1_ORDER;
        n_minus_one[31] -= 1;
        assert_eq!(negate_mod_order(&one), n_minus_one);

        // Twice is the identity, for a value that borrows down every byte.
        let s = [
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x7F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            0xFF, 0xFF, 0xFF, 0xFF,
        ];
        assert_eq!(negate_mod_order(&negate_mod_order(&s)), s);
    }

    /// Normalizing a low-S signature is the identity, and normalizing is **idempotent**: the
    /// malleability fix must not move an already-canonical signature, which is what makes the deploy
    /// dedup key stable.
    #[test]
    fn a_low_s_signature_is_left_alone() {
        let der = base16::unsafe_decode(LOW_S_DER);
        assert_eq!(normalize_signature_low_s("secp256k1", &der), der);
        assert_eq!(
            normalize_signature_low_s("secp256k1:eth", &rs_of(&der)),
            rs_of(&der)
        );
    }

    /// The high-S form of that same signature — `s` replaced by `n − s` — is rewritten back to the
    /// low-S one, in both encodings. This is the malleability fix (R28): without it the same
    /// signature could be re-encoded and produce a second dedup key.
    #[test]
    fn the_high_s_twin_is_normalized_back_to_the_low_s_form() {
        let der = base16::unsafe_decode(LOW_S_DER);
        let low = rs_of(&der);

        let mut high = low.clone();
        high[32..].copy_from_slice(&negate_mod_order(&low[32..]));
        assert!(is_high_s(&high[32..]), "the twin is high-S by construction");

        let high_der = encode_signature_rs_to_der(&high).expect("64 bytes");
        assert_ne!(high_der, der, "the twin is a different encoding");
        assert_eq!(
            normalize_signature_low_s("secp256k1", &high_der),
            der,
            "the DER form is rewritten to the canonical low-S encoding"
        );
        assert_eq!(
            normalize_signature_low_s("secp256k1:eth", &high),
            low,
            "the raw RS form is rewritten in place"
        );
        // Idempotent in both directions.
        assert_eq!(
            normalize_signature_low_s(
                "secp256k1",
                &normalize_signature_low_s("secp256k1", &high_der)
            ),
            der
        );
    }

    /// Everything that is **not** a signature this function understands is returned unchanged: a
    /// 64-byte check for the eth form, a DER decode failure for the secp256k1 form, and any
    /// unregistered algorithm name. Best-effort, never lossy.
    #[test]
    fn anything_it_cannot_decode_is_returned_unchanged() {
        // The eth form is raw RS: any other length cannot be split into r and s.
        let short = vec![0xFFu8; 63];
        assert_eq!(normalize_signature_low_s("secp256k1:eth", &short), short);

        // Not a DER SEQUENCE.
        let garbage = vec![0x01, 0x02, 0x03];
        assert_eq!(normalize_signature_low_s("secp256k1", &garbage), garbage);
        assert_eq!(
            normalize_signature_low_s("secp256k1", &[]),
            Vec::<u8>::new()
        );

        // An unregistered name (or one that is merely registered *elsewhere*) is a passthrough.
        let rs = vec![0xFFu8; 64];
        assert_eq!(normalize_signature_low_s("ed25519", &rs), rs);
        assert_eq!(normalize_signature_low_s("", &rs), rs);
    }
}
