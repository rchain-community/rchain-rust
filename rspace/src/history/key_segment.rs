//! A radix-tree key segment (0–127 bytes).
//!
//! Mirrors `rspace/src/main/scala/coop/rchain/rspace/history/KeySegment.scala`.

use rchain_shared::base16;

/// A path segment of a radix key (port of `KeySegment`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct KeySegment {
    value: Vec<u8>,
}

/// Validated construction: a key segment is at most 127 bytes (the radix-tree wire invariant).
///
/// This is the type's **only** constructor. The port used to carry a second, unchecked `new` beside
/// it, documented "the caller guarantees `value.len() <= 127`" — the bypass C61 was exploited
/// through (`rspace/src/state/mod.rs` built a peer-supplied 128 with it) and which C61's fix then
/// routed *around* rather than repaired. C61's own words name the repair: "the checked constructor,
/// not a corrected constant: `KeySegment::try_from` **is** the 127-byte invariant". A second checked
/// name would only be a second place to drift, so there is one.
impl TryFrom<Vec<u8>> for KeySegment {
    type Error = String;
    fn try_from(value: Vec<u8>) -> Result<Self, String> {
        if value.len() <= 127 {
            Ok(KeySegment { value })
        } else {
            Err(format!("key segment length {} exceeds 127", value.len()))
        }
    }
}

impl KeySegment {
    /// Build from a value that is provably a *slice of a valid segment*: no growth, so the 127-byte
    /// invariant is inherited rather than re-established, and the `expect` cannot fire (a slice is
    /// never longer than its source). Private — the distinction that makes it a different thing in
    /// kind from the public `new` it replaces, which any caller could reach with any length.
    fn from_slice_of_valid(value: Vec<u8>) -> KeySegment {
        KeySegment::try_from(value).expect("a slice of a valid segment is at most 127 bytes")
    }

    /// The empty segment. Total: `0` bytes, so the invariant holds trivially and there is no
    /// refusal to report.
    pub fn empty() -> Self {
        KeySegment { value: Vec::new() }
    }

    pub fn len(&self) -> usize {
        self.value.len()
    }

    pub fn is_empty(&self) -> bool {
        self.value.is_empty()
    }

    pub fn head(&self) -> u8 {
        self.value[0]
    }

    /// Drop the first byte. Total by monotonicity: the result is a suffix of a valid segment, hence
    /// strictly shorter, so it cannot leave the invariant and reports no failure. (Panics on an
    /// *empty* segment via the slice — latent, with its reachability argument pinned by
    /// `history_action.rs`'s `trimming_an_empty_key_panics`.)
    pub fn tail(&self) -> KeySegment {
        Self::from_slice_of_valid(self.value[1..].to_vec())
    }

    pub fn head_option(&self) -> Option<u8> {
        self.value.first().copied()
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.value
    }

    /// Concatenate two segments (port of `++`). **Fallible because this grows**: two valid segments
    /// can sum to 254 bytes, and the Scala's `KeySegment(value ++ other.value)` reaches `apply`'s
    /// `require(bv.size <= 127)` and throws — an `Err` here is that refusal rendered as a value
    /// rather than a panic.
    pub fn concat(&self, other: &KeySegment) -> Result<KeySegment, String> {
        let mut value = self.value.clone();
        value.extend_from_slice(&other.value);
        KeySegment::try_from(value)
    }

    /// Append a byte (port of `:+`). Fallible for the same reason as [`Self::concat`]: a maximal
    /// 127-byte segment plus one byte is 128 — and 128's length is written in 7 bits, so the radix
    /// encoder would serialize it as `0` and drop the whole prefix (C61's truncation).
    pub fn append(&self, byte: u8) -> Result<KeySegment, String> {
        let mut value = self.value.clone();
        value.push(byte);
        KeySegment::try_from(value)
    }

    pub fn to_hex(&self) -> String {
        base16::encode(&self.value)
    }

    /// The common prefix of `a` and `b`, plus their remainders (port of `commonPrefix`).
    ///
    /// Total by monotonicity: each of the three results is a slice of `a` or `b` — the prefix is
    /// bounded by the shorter argument, the remainders are suffixes — so none can exceed the 127
    /// bytes its source already satisfies. There is no growth here for the invariant to survive.
    pub fn common_prefix(a: &KeySegment, b: &KeySegment) -> (KeySegment, KeySegment, KeySegment) {
        let mut i = 0;
        while i < a.value.len() && i < b.value.len() && a.value[i] == b.value[i] {
            i += 1;
        }
        (
            Self::from_slice_of_valid(a.value[..i].to_vec()),
            Self::from_slice_of_valid(a.value[i..].to_vec()),
            Self::from_slice_of_valid(b.value[i..].to_vec()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_from_rejects_oversized_segment() {
        // ≤127 bytes is the radix-tree wire invariant; 128 must be rejected, not truncated.
        assert!(KeySegment::try_from(vec![0u8; 127]).is_ok());
        assert!(KeySegment::try_from(vec![0u8; 128]).is_err());
    }

    /// The C61 shape at the constructor C61 routed *around* (the class C78 fixed in `ShardId`): a
    /// segment one byte over the invariant is refused, and refused by every way of building or
    /// growing one — the constructor, `append`, `concat`.
    ///
    /// Before the fix this test did not compile: `new` was total and `append`/`concat` returned a
    /// `KeySegment`, so there was no refusal to observe and the 128-byte breach above was reachable
    /// through the public API.
    #[test]
    fn a_segment_one_byte_over_the_invariant_is_refused_at_every_constructor() {
        assert!(KeySegment::try_from(vec![0u8; 127]).is_ok());
        assert!(KeySegment::try_from(vec![0u8; 128]).is_err());

        let maximal = KeySegment::try_from(vec![0u8; 127]).expect("127 <= 127");
        let one = KeySegment::try_from(vec![0u8; 1]).expect("1 <= 127");
        assert!(
            maximal.append(1).is_err(),
            "append must refuse to grow a segment past 127"
        );
        assert!(
            maximal.concat(&one).is_err(),
            "concat must refuse to grow a segment past 127"
        );
        assert!(
            one.concat(&maximal).is_err(),
            "concat must refuse in either order"
        );

        // Growth that stays inside the bound still succeeds, so the refusal is the bound and not the
        // operation: 127 + 0 is fine, and so is a 126-byte segment plus one byte.
        let kept = maximal.concat(&KeySegment::empty()).expect("127 + 0 <= 127");
        assert_eq!(kept.len(), 127);
        let near_max = KeySegment::try_from(vec![0u8; 126]).expect("126 <= 127");
        assert_eq!(near_max.append(7).expect("126 + 1 <= 127").len(), 127);

        // The shrinking constructors stay *total*, by monotonicity: their results are slices of a
        // valid segment (`tail` drops the first byte; `common_prefix`'s three parts are a prefix and
        // two suffixes of its arguments), so they cannot exceed 127 and report no failure.
        assert_eq!(maximal.tail().len(), 126);
        let (prefix, rest_a, rest_b) = KeySegment::common_prefix(&maximal, &maximal.tail());
        assert_eq!((prefix.len(), rest_a.len(), rest_b.len()), (126, 1, 0));

        // And an empty segment is still the total zero case.
        assert!(KeySegment::empty().is_empty());
    }

    #[test]
    fn common_prefix_splits_into_prefix_and_remainders() {
        let a = KeySegment::try_from(vec![1, 2, 3]).expect("3 bytes is at most 127");
        let b = KeySegment::try_from(vec![1, 2, 4]).expect("3 bytes is at most 127");
        let (prefix, ra, rb) = KeySegment::common_prefix(&a, &b);
        assert_eq!(prefix.as_bytes(), &[1, 2]);
        assert_eq!(ra.as_bytes(), &[3]);
        assert_eq!(rb.as_bytes(), &[4]);
    }
}
