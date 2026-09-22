//! Orderings.
//!
//! Mirrors `crypto/src/main/scala/coop/rchain/crypto/util/Sorting.scala`.

use std::cmp::Ordering;

/// Compare two bytes as **signed** bytes (Scala `Byte` is signed; `Ordering.by(Array[Byte].toIterable)`
/// orders `0x80..0xFF` *before* `0x00..0x7F`).
pub fn cmp_signed_byte(a: u8, b: u8) -> Ordering {
    (a as i8).cmp(&(b as i8))
}

/// Lexicographic ordering over signed bytes (mirrors Scala `Ordering.by((_: Array[Byte]).toIterable)`).
pub fn compare_byte_arrays(a: &[u8], b: &[u8]) -> Ordering {
    for (x, y) in a.iter().zip(b.iter()) {
        let ord = cmp_signed_byte(*x, *y);
        if ord != Ordering::Equal {
            return ord;
        }
    }
    a.len().cmp(&b.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Scala `Byte` is **signed**, so this is not the unsigned comparison: `0x80..=0xFF` are
    /// negative and sort *before* `0x00..=0x7F`. Getting that wrong reorders the validator set that
    /// `PublicKey`'s `Ord` derives from this function, so it is the point of the module rather than
    /// an implementation detail.
    #[test]
    fn signed_bytes_order_the_high_half_first() {
        assert_eq!(cmp_signed_byte(0x80, 0x00), Ordering::Less);
        assert_eq!(cmp_signed_byte(0xFF, 0x7F), Ordering::Less);
        assert_eq!(cmp_signed_byte(0x00, 0x7F), Ordering::Less);
        assert_eq!(cmp_signed_byte(0x7F, 0x7F), Ordering::Equal);
        assert_eq!(cmp_signed_byte(0x00, 0x80), Ordering::Greater);
        // The extremes: 0x7F is 127 (largest), 0x80 is −128 (smallest), and 0xFF is −1 — so the
        // high half is ordered *among itself* in reverse, 0x80 before 0xFF.
        assert_eq!(cmp_signed_byte(0x80, 0xFF), Ordering::Less);
        assert_eq!(cmp_signed_byte(0xFF, 0x80), Ordering::Greater);
    }

    /// Lexicographic over the signed order: the first differing byte decides, and the length is the
    /// tie-break *only* when every compared byte is equal — so a proper prefix sorts first.
    #[test]
    fn byte_arrays_compare_lexicographically_with_the_shorter_prefix_first() {
        assert_eq!(compare_byte_arrays(&[], &[]), Ordering::Equal);
        assert_eq!(compare_byte_arrays(&[], &[0]), Ordering::Less);
        assert_eq!(compare_byte_arrays(&[0], &[]), Ordering::Greater);
        assert_eq!(compare_byte_arrays(&[1, 2], &[1, 2]), Ordering::Equal);
        assert_eq!(compare_byte_arrays(&[1, 2], &[1, 2, 3]), Ordering::Less);
        assert_eq!(compare_byte_arrays(&[1, 3], &[1, 2, 3]), Ordering::Greater);
        // The signed rule reaches through the array comparison: 0xFF sorts before 0x00, where an
        // unsigned lexicographic comparison would place it after.
        assert_eq!(compare_byte_arrays(&[0xFF], &[0x00]), Ordering::Less);
        assert_eq!(
            compare_byte_arrays(&[0x00, 0xFF], &[0x00, 0x80]),
            Ordering::Greater,
            "0xFF is −1 and 0x80 is −128, so the second array is smaller"
        );
    }
}
