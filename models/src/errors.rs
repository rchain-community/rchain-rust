//! Model (de)serialization errors.
//!
//! Hard error type for the models crate's protobuf/packet decode boundary. Free-form prost decode
//! messages are carried as `String`; the structural cases (a missing field/variant, a mismatched
//! packet type tag) are typed.

use std::fmt;

/// An error decoding a model from its protobuf or packet representation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelsError {
    /// Protobuf/byte decoding failed.
    Decode(String),
    /// A protobuf message was malformed (a missing field or unexpected variant).
    Malformed(&'static str),
    /// A packet's type tag did not match the expected tag.
    PacketTypeMismatch { got: String, expected: String },
    /// A fixed-width value had the wrong byte length (validate-on-ingress).
    Length { got: usize, expected: usize },
}

impl fmt::Display for ModelsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModelsError::Decode(m) => write!(f, "decode error: {m}"),
            ModelsError::Malformed(m) => write!(f, "malformed: {m}"),
            ModelsError::PacketTypeMismatch { got, expected } => {
                write!(f, "Got {got} packet - need {expected} packet")
            }
            ModelsError::Length { got, expected } => {
                write!(f, "expected {expected} bytes, got {got}")
            }
        }
    }
}

impl std::error::Error for ModelsError {}

impl From<rchain_crypto::errors::CryptoError> for ModelsError {
    fn from(e: rchain_crypto::errors::CryptoError) -> Self {
        match e {
            rchain_crypto::errors::CryptoError::InvalidLength { expected, actual } => {
                ModelsError::Length {
                    got: actual,
                    expected,
                }
            }
            other => ModelsError::Decode(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use rchain_crypto::errors::CryptoError;

    /// **The remap's field order.** `CryptoError::InvalidLength { expected, actual }` becomes
    /// `ModelsError::Length { got, expected }`: the source's `actual` is the target's `got`. A
    /// transposition here produces a message that reads plausibly and states the opposite ("expected
    /// 5 bytes, got 32"), which is exactly the kind of error a reader would believe.
    #[test]
    fn the_length_remap_keeps_got_and_expected_the_right_way_round() {
        let mapped: ModelsError = CryptoError::InvalidLength {
            expected: 32,
            actual: 5,
        }
        .into();
        assert_eq!(
            mapped,
            ModelsError::Length {
                got: 5,
                expected: 32
            }
        );
        assert_eq!(mapped.to_string(), "expected 32 bytes, got 5");
    }

    /// Every other crypto error is carried as a decode failure with the source's own message, so no
    /// variant is silently dropped (an `unwrap`-style mapping would lose the reason).
    #[test]
    fn any_other_crypto_error_becomes_a_decode_error_carrying_its_message() {
        for source in [
            CryptoError::InvalidKey,
            CryptoError::EncryptionFailed,
            CryptoError::InvalidHex,
        ] {
            let message = source.to_string();
            let mapped: ModelsError = source.into();
            assert_eq!(mapped, ModelsError::Decode(message.clone()));
            assert!(
                mapped.to_string().contains(&message),
                "the source's reason must survive the mapping: {mapped}"
            );
        }
    }

    /// The four `Display` arms. `PacketTypeMismatch` uses the Scala wording
    /// (`"Got X packet - need Y packet"`) rather than a Rust-shaped sentence, because the string is
    /// what a peer's rejection reports.
    #[test]
    fn every_display_arm_renders_its_case() {
        assert_eq!(
            ModelsError::Decode("bad bytes".to_string()).to_string(),
            "decode error: bad bytes"
        );
        assert_eq!(
            ModelsError::Malformed("missing field").to_string(),
            "malformed: missing field"
        );
        assert_eq!(
            ModelsError::PacketTypeMismatch {
                got: "BlockRequest".to_string(),
                expected: "HasBlock".to_string()
            }
            .to_string(),
            "Got BlockRequest packet - need HasBlock packet"
        );
        // Note the rendering order: the *expected* length comes first, unlike the field order.
        assert_eq!(
            ModelsError::Length {
                got: 5,
                expected: 32
            }
            .to_string(),
            "expected 32 bytes, got 5"
        );
    }

    /// The type is a real `std::error::Error`, so `?` across the crate boundary and any
    /// `Box<dyn Error>` sink work — and it is `Clone`/`PartialEq`, which is what lets a caller match
    /// on it after the fact.
    #[test]
    fn it_is_a_std_error() {
        let e = ModelsError::Malformed("x");
        let boxed: Box<dyn std::error::Error> = Box::new(e.clone());
        assert_eq!(boxed.to_string(), "malformed: x");
        assert_eq!(e.clone(), e);
    }
}
