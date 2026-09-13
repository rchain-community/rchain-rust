//! Event-log merging (Law 9: merge is a monoid; non-conflicting logs commute).
//!
//! Mirrors `rspace/src/main/scala/coop/rchain/rspace/merger/`.

pub mod channel_change;
pub mod event_log_index;
pub mod event_log_merging_logic;
pub mod state_change;
pub mod state_change_merger;

/// Multiset difference of two slices (port of Scala `Seq.diff`): remove the first occurrence of
/// each element of `b` from `a`, preserving order.
pub(crate) fn seq_diff<T: PartialEq + Clone>(a: &[T], b: &[T]) -> Vec<T> {
    let mut out: Vec<T> = a.to_vec();
    for item in b {
        if let Some(pos) = out.iter().position(|x| x == item) {
            out.remove(pos);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `seq_diff` is `Seq.diff` (Law 9's merge layer): a **multiset** difference that removes the
    /// first occurrence of each element of `b` and preserves `a`'s order — not a set difference, and
    /// not "remove all occurrences". The three cases that tell those apart are pinned.
    #[test]
    fn seq_diff_removes_the_first_occurrence_and_preserves_order() {
        // Order is `a`'s, and only the matched occurrences move out.
        assert_eq!(
            seq_diff(&[1, 2, 3, 4], &[2, 4]),
            vec![1, 3],
            "the survivors keep their original order"
        );
        // A duplicate in `b` removes *two* occurrences of the same value...
        assert_eq!(
            seq_diff(&[1, 1, 1], &[1, 1]),
            vec![1],
            "a repeated element in `b` removes one occurrence each"
        );
        // ...while a single one leaves the other duplicate in place (the case a set difference
        // would get wrong).
        assert_eq!(seq_diff(&[1, 1, 1], &[1]), vec![1, 1]);
        // An element of `b` that is absent from `a` is a no-op, not an error.
        assert_eq!(seq_diff(&[1, 2], &[9]), vec![1, 2]);
        // Empty inputs.
        assert_eq!(seq_diff(&[1, 2], &[]), vec![1, 2]);
        assert_eq!(seq_diff::<i32>(&[], &[1]), Vec::<i32>::new());
        // Elements that only compare equal by value are still distinct in `a`'s order: strings,
        // where "remove the first occurrence" is observable.
        assert_eq!(
            seq_diff(&["a", "b", "a"], &["a"]),
            vec!["b", "a"],
            "the *first* `a` goes, not the last"
        );
    }
}
