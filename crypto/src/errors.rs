//! Cryptographic errors.
//!
//! Hard error type for the crypto crate's declared partiality boundaries (fixed-width coercion,
//! key-material validation, box encryption/decryption, hex decoding). Mirrors the typed-fix column
//! of `spec/TYPE-SYSTEM.md` §3.2.

use std::fmt;

/// A cryptographic error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CryptoError {
    /// Secret/public key material is invalid for the algorithm.
    InvalidKey,
    /// A fixed-width coercion received the wrong length.
    InvalidLength { expected: usize, actual: usize },
    /// Box (XSalsa20-Poly1305) encryption/decryption failed.
    EncryptionFailed,
    /// A hex string was malformed or not the expected length.
    InvalidHex,
}

impl fmt::Display for CryptoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CryptoError::InvalidKey => write!(f, "invalid key material"),
            CryptoError::InvalidLength { expected, actual } => {
                write!(f, "invalid length: expected {expected} bytes, got {actual}")
            }
            CryptoError::EncryptionFailed => write!(f, "box encryption failed"),
            CryptoError::InvalidHex => write!(f, "invalid hex string"),
        }
    }
}

impl std::error::Error for CryptoError {}

/// Convenience result alias.
pub type Result<A> = std::result::Result<A, CryptoError>;

#[cfg(test)]
mod tests {
    use super::*;

    /// Every variant renders, and `InvalidLength` keeps `expected` and `actual` the right way round
    /// — swapped arguments in a length error send a reader looking in the wrong place, which is the
    /// same defect `models::errors` pins for the length remap.
    #[test]
    fn every_variant_renders_and_the_length_error_does_not_swap_its_numbers() {
        assert_eq!(CryptoError::InvalidKey.to_string(), "invalid key material");
        assert_eq!(
            CryptoError::EncryptionFailed.to_string(),
            "box encryption failed"
        );
        assert_eq!(CryptoError::InvalidHex.to_string(), "invalid hex string");

        let length = CryptoError::InvalidLength {
            expected: 80,
            actual: 79,
        };
        assert_eq!(
            length.to_string(),
            "invalid length: expected 80 bytes, got 79"
        );
        assert_ne!(
            length.to_string(),
            CryptoError::InvalidLength {
                expected: 79,
                actual: 80,
            }
            .to_string(),
            "the two numbers are not interchangeable"
        );
    }

    /// It is a real `Error` (so `?` into `Box<dyn Error>` works), and it is `Clone`/`PartialEq` —
    /// the properties the callers that match on it rely on.
    #[test]
    fn it_is_an_error_value_like_any_other() {
        let e: Box<dyn std::error::Error> = Box::new(CryptoError::InvalidKey);
        assert_eq!(e.to_string(), "invalid key material");
        let cloned = CryptoError::InvalidHex.clone();
        assert_eq!(cloned, CryptoError::InvalidHex);
        assert_ne!(cloned, CryptoError::InvalidKey);
    }
}
