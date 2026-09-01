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
