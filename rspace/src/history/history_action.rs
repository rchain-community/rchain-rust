//! Radix-tree mutation actions.
//!
//! Mirrors `rspace/src/main/scala/coop/rchain/rspace/history/HistoryAction.scala`.

use rchain_crypto::hash::blake2b256_hash::Blake2b256Hash;

use crate::history::key_segment::KeySegment;

/// A radix-tree insert or delete (port of `HistoryAction`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HistoryAction {
    Insert {
        key: KeySegment,
        hash: Blake2b256Hash,
    },
    Delete {
        key: KeySegment,
    },
}

impl HistoryAction {
    pub fn key(&self) -> &KeySegment {
        match self {
            HistoryAction::Insert { key, .. } => key,
            HistoryAction::Delete { key } => key,
        }
    }

    /// Drop the first byte of the key (port of the `trimKeys` helper).
    pub fn trim(&self) -> HistoryAction {
        match self {
            HistoryAction::Insert { key, hash } => HistoryAction::Insert {
                key: key.tail(),
                hash: *hash,
            },
            HistoryAction::Delete { key } => HistoryAction::Delete { key: key.tail() },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(bytes: &[u8]) -> KeySegment {
        KeySegment::new(bytes.to_vec())
    }

    fn hash(byte: u8) -> Blake2b256Hash {
        Blake2b256Hash::from_byte_array(&[byte; 32])
    }

    /// `key()` is the accessor the radix-tree walk keys its buckets on, for both variants.
    #[test]
    fn key_reports_the_segment_for_both_variants() {
        let k = key(&[1, 2, 3]);
        let insert = HistoryAction::Insert {
            key: k.clone(),
            hash: hash(9),
        };
        let delete = HistoryAction::Delete { key: k.clone() };
        assert_eq!(insert.key(), &k);
        assert_eq!(delete.key(), &k);
    }

    /// `trim` drops the first byte and keeps the payload — the `trimKeys` step of the tree walk
    /// (descending one level drops one nibble/byte of the path).
    #[test]
    fn trim_drops_the_first_byte_of_the_key() {
        let insert = HistoryAction::Insert {
            key: key(&[1, 2, 3]),
            hash: hash(9),
        };
        assert_eq!(
            insert.trim(),
            HistoryAction::Insert {
                key: key(&[2, 3]),
                hash: hash(9)
            }
        );
        let delete = HistoryAction::Delete {
            key: key(&[1, 2, 3]),
        };
        assert_eq!(delete.trim(), HistoryAction::Delete { key: key(&[2, 3]) });
        // A one-byte key trims to the empty segment, which is a valid tree position.
        assert_eq!(
            HistoryAction::Delete { key: key(&[7]) }.trim().key(),
            &KeySegment::empty()
        );
    }

    /// Trimming an already-empty key panics rather than yielding an empty segment: `KeySegment::tail`
    /// indexes `value[1..]`. The tree never trims past a leaf, so this is latent — pinned because a
    /// change in reachability (or a guard) should fail a test rather than surface as a node panic.
    #[test]
    #[should_panic(expected = "range start index 1 out of range")]
    fn trimming_an_empty_key_panics() {
        let _ = HistoryAction::Delete {
            key: KeySegment::empty(),
        }
        .trim();
    }
}
