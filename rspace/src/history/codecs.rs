//! Codecs for history store keys/values.
//!
//! Mirrors `rspace/.../hashing/Blake2b256Hash.codecBlake2b256Hash`.

use rchain_crypto::hash::blake2b256_hash::Blake2b256Hash;
use rchain_shared::typed_store::Codec;

/// A fixed 32-byte codec for `Blake2b256Hash`.
#[derive(Default)]
pub struct Blake2b256HashCodec;

impl Codec<Blake2b256Hash> for Blake2b256HashCodec {
    fn encode(&self, value: &Blake2b256Hash) -> Vec<u8> {
        value.to_byte_array().to_vec()
    }

    fn decode(&self, bytes: &[u8]) -> Result<Blake2b256Hash, String> {
        if bytes.len() != 32 {
            return Err(format!("expected 32 bytes, got {}", bytes.len()));
        }
        Ok(Blake2b256Hash::from_byte_array(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(byte: u8) -> Blake2b256Hash {
        Blake2b256Hash::from_byte_array(&[byte; 32])
    }

    /// The codec is the history store's key format, so a round trip that did not reproduce the
    /// bytes exactly would make every store lookup a miss.
    #[test]
    fn a_hash_round_trips() {
        let codec = Blake2b256HashCodec;
        let h = hash(0x5A);
        let encoded = codec.encode(&h);
        assert_eq!(encoded.len(), 32);
        assert_eq!(encoded, vec![0x5A; 32]);
        assert_eq!(codec.decode(&encoded).expect("decode"), h);
    }

    /// **The decode arm that matters.** A corrupted or truncated store value must be an error naming
    /// the length, not a hash built from whatever bytes arrived — a padded read would silently
    /// address the wrong node in the radix tree.
    #[test]
    fn decode_rejects_a_wrong_length() {
        let codec = Blake2b256HashCodec;
        for bad in [vec![], vec![0u8; 31], vec![0u8; 33], vec![0u8; 64]] {
            let len = bad.len();
            let err = codec.decode(&bad).expect_err("wrong length");
            assert_eq!(err, format!("expected 32 bytes, got {len}"));
        }
    }
}
