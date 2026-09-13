//! A validator identity.
//!
//! Mirrors `models/src/main/scala/coop/rchain/models/Validator.scala`.

use rchain_shared::base16;

use crate::errors::ModelsError;

/// The length of a `Validator` in bytes (an uncompressed secp256k1 public key).
pub const LENGTH: usize = 65;

/// A 65-byte validator identity.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Validator([u8; LENGTH]);

impl Validator {
    pub fn new(bytes: [u8; LENGTH]) -> Self {
        Self(bytes)
    }

    pub fn from_slice(bytes: &[u8]) -> Self {
        assert_eq!(bytes.len(), LENGTH, "expected {LENGTH} bytes");
        let mut arr = [0u8; LENGTH];
        arr.copy_from_slice(bytes);
        Self(arr)
    }

    pub fn as_bytes(&self) -> &[u8; LENGTH] {
        &self.0
    }
}

impl TryFrom<&[u8]> for Validator {
    type Error = ModelsError;

    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        if bytes.len() != LENGTH {
            return Err(ModelsError::Length {
                got: bytes.len(),
                expected: LENGTH,
            });
        }
        Ok(Self::from_slice(bytes))
    }
}

// `Validator` serializes as lowercase hex (the Scala `ByteString` → `buildStringNoLimit`).
impl serde::Serialize for Validator {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&base16::encode(self.as_bytes()))
    }
}

impl<'de> serde::Deserialize<'de> for Validator {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        let bytes =
            base16::decode(&s).ok_or_else(|| serde::de::Error::custom("invalid validator hex"))?;
        if bytes.len() != LENGTH {
            return Err(serde::de::Error::custom("invalid validator length"));
        }
        let mut arr = [0u8; LENGTH];
        arr.copy_from_slice(&bytes);
        Ok(Validator(arr))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(b: u8) -> Vec<u8> {
        vec![b; LENGTH]
    }

    /// The *checked* constructor: a wrong length is a typed error naming both lengths, which is what
    /// every untrusted boundary (a proto field, a genesis file) must use.
    #[test]
    fn try_from_rejects_a_wrong_length_and_names_both() {
        let ok = Validator::try_from(bytes(7).as_slice()).expect("65 bytes");
        assert_eq!(ok.as_bytes(), &bytes(7)[..]);

        for bad in [Vec::new(), vec![1u8; LENGTH - 1], vec![1u8; LENGTH + 1]] {
            let got = bad.len();
            let err = Validator::try_from(bad.as_slice()).expect_err("wrong length");
            assert_eq!(
                err,
                ModelsError::Length {
                    got,
                    expected: LENGTH
                },
                "the error must say what it got and what it wanted"
            );
        }
    }

    /// `from_slice` is the *unchecked* path — it asserts rather than returning an error, because its
    /// callers (block metadata, bonds maps) carry the length invariant structurally. Pinned so that
    /// a caller reaching it with untrusted bytes is a loud failure, not a truncated identity.
    #[test]
    #[should_panic(expected = "expected 65 bytes")]
    fn from_slice_panics_on_a_short_slice() {
        let _ = Validator::from_slice(&[0u8; LENGTH - 1]);
    }

    /// The wire form is lowercase hex (Scala's `ByteString` → `buildStringNoLimit`), and it round
    /// trips: a validator that shortened or reordered a key would break bonds lookups silently.
    #[test]
    fn serde_round_trips_lowercase_hex() {
        let v = Validator::from_slice(&bytes(0xAB));
        let json = serde_json::to_string(&v).expect("serialize");
        assert_eq!(json, format!("\"{}\"", "ab".repeat(LENGTH)));
        assert_eq!(
            serde_json::from_str::<Validator>(&json).expect("deserialize"),
            v
        );
    }

    /// The two decode error arms: not hex at all, and hex of the wrong length. Both are reachable
    /// from a malformed bonds map, so neither may silently produce a zeroed validator.
    #[test]
    fn deserialize_rejects_non_hex_and_a_wrong_length() {
        let err = serde_json::from_str::<Validator>("\"zzz\"").expect_err("not hex");
        assert!(format!("{err}").contains("invalid validator hex"), "{err}");

        let short = "\"ab\"";
        let err = serde_json::from_str::<Validator>(short).expect_err("too short");
        assert!(
            format!("{err}").contains("invalid validator length"),
            "{err}"
        );

        let long = format!("\"{}\"", "ab".repeat(LENGTH + 1));
        assert!(serde_json::from_str::<Validator>(&long).is_err());
    }

    /// Identity is byte-wise: two validators with the same key are equal and hash alike (they are
    /// map keys in the bonds map), and different keys order consistently — `Ord` drives the sorted
    /// bond caches, so an order that disagreed with the bytes would make a state hash depend on
    /// insertion order.
    #[test]
    fn identity_is_byte_wise() {
        let a = Validator::from_slice(&bytes(1));
        let b = Validator::from_slice(&bytes(2));
        assert_eq!(a, Validator::from_slice(&bytes(1)));
        assert_ne!(a, b);
        assert!(a < b, "0x01… sorts before 0x02…");
        let set: std::collections::BTreeSet<Validator> = [b, a].into_iter().collect();
        assert_eq!(set.iter().next(), Some(&a), "the set orders by the key");
    }
}
