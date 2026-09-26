//! A block hash.
//!
//! Mirrors `models/src/main/scala/coop/rchain/models/BlockHash.scala`.
//!
//! The 32-byte storage is the shared [`Hash32`](rchain_shared::refined::Hash32) newtype.

use rchain_shared::base16;
use rchain_shared::refined::Hash32;

use crate::errors::ModelsError;

/// The length of a `BlockHash` in bytes.
pub const LENGTH: usize = 32;

/// A 32-byte block hash.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct BlockHash(Hash32);

impl BlockHash {
    /// Wrap a 32-byte array.
    pub fn new(bytes: [u8; LENGTH]) -> Self {
        Self(Hash32::new(bytes))
    }

    /// Wrap a 32-byte slice (panics if not exactly [`LENGTH`] bytes).
    ///
    /// **Internal invariants only.** Every byte that originates off the wire or the API goes through
    /// [`TryFrom<&[u8]>`](BlockHash::try_from) instead (validate-on-ingress, AUDIT §11 R12 and C97,
    /// which is where four paths that did not are recorded). This constructor is what the fixed-width
    /// refinement exists to make unnecessary at a boundary; `TYPE-SYSTEM.md` §1.7's Construction rule
    /// is the general form of the rule.
    pub fn from_slice(bytes: &[u8]) -> Self {
        assert_eq!(bytes.len(), LENGTH, "expected {LENGTH} bytes");
        let mut arr = [0u8; LENGTH];
        arr.copy_from_slice(bytes);
        Self(Hash32::new(arr))
    }

    /// The underlying 32 bytes.
    pub fn as_bytes(&self) -> &[u8; LENGTH] {
        self.0.as_bytes()
    }

    /// Hex-encode the hash.
    pub fn to_hex(&self) -> String {
        self.0.to_hex()
    }

    /// Parse a full 32-byte hex string, rejecting non-hex or wrong-length input.
    ///
    /// The only hex *constructor* since 2026-09-24: the lax `from_hex` — `unsafe_decode` (non-hex
    /// stripped) followed by `from_slice`'s length assert, so an arbitrary string could reach a
    /// panic — was deleted (U1 site 3, AUDIT C52). It had no production caller in the workspace, only
    /// its own round-trip test.
    ///
    /// **This sentence used to end "every ingress path already used this checked form (AUDIT §11
    /// R12)", and that was false** (corrected 2026-09-26, C97): `HasBlockRequest`/`HasBlock`/
    /// `BlockRequest` carried raw `Vec<u8>` hashes into `from_slice` from the socket, and `Event`'s
    /// hashes reached `Blake2b256Hash::from_byte_array` the same way. The claim is worth keeping as a
    /// correction rather than a deletion because it is the same failure mode the type system exists
    /// to prevent: a statement about the sweep, written where a statement about the tree was read.
    pub fn try_from_hex(s: &str) -> Result<Self, ModelsError> {
        let bytes = base16::try_decode(s).map_err(ModelsError::Decode)?;
        Self::try_from(bytes.as_slice())
    }

    /// Whether the hash begins with `prefix` (used by `DagRepresentation.find`).
    pub fn starts_with(&self, prefix: &[u8]) -> bool {
        self.0.starts_with(prefix)
    }
}

impl TryFrom<&[u8]> for BlockHash {
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

impl From<Hash32> for BlockHash {
    fn from(h: Hash32) -> Self {
        BlockHash(h)
    }
}

impl From<BlockHash> for Hash32 {
    fn from(h: BlockHash) -> Self {
        h.0
    }
}

/// Total conversion from the canonical digest type (both are fixed 32-byte wrappers).
impl From<rchain_crypto::hash::blake2b256_hash::Blake2b256Hash> for BlockHash {
    fn from(h: rchain_crypto::hash::blake2b256_hash::Blake2b256Hash) -> Self {
        Self(h.into())
    }
}

impl From<BlockHash> for rchain_crypto::hash::blake2b256_hash::Blake2b256Hash {
    fn from(h: BlockHash) -> Self {
        h.0.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips() {
        let h = BlockHash::from_slice(&[0xab; 32]);
        assert_eq!(BlockHash::try_from_hex(&h.to_hex()).unwrap(), h);
    }

    #[test]
    fn length_is_32() {
        assert_eq!(LENGTH, 32);
    }

    #[test]
    fn orders_lexicographically() {
        let a = BlockHash::new([0u8; 32]);
        let b = BlockHash::new([1u8; 32]);
        assert!(a < b);
    }

    #[test]
    fn starts_with_prefix() {
        let mut bytes = [0u8; 32];
        bytes[..4].copy_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
        let h = BlockHash::new(bytes);
        assert!(h.starts_with(&[0xde, 0xad]));
        assert!(!h.starts_with(&[0xde, 0xae]));
    }

    #[test]
    fn try_from_rejects_wrong_length() {
        assert!(BlockHash::try_from(&[0u8; 31][..]).is_err());
        assert!(BlockHash::try_from(&[0u8; 33][..]).is_err());
        assert!(BlockHash::try_from(&[0u8; 32][..]).is_ok());
    }

    #[test]
    fn try_from_hex_rejects_malformed() {
        assert!(BlockHash::try_from_hex("zz").is_err());
        assert!(BlockHash::try_from_hex("abcd").is_err()); // wrong length
        let h = BlockHash::new([0xab; 32]);
        assert_eq!(BlockHash::try_from_hex(&h.to_hex()).unwrap(), h);
    }
}
