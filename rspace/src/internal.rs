//! Core tuple-space data types.
//!
//! Mirrors `rspace/src/main/scala/coop/rchain/rspace/internal.scala`.

use std::collections::{BTreeSet, HashMap};
use std::hash::Hash;

use rchain_shared::serialize::Serialize;

use crate::trace::event::{Consume, Produce};

/// A value paired with its serialized bytes (port of `Encoded`).
#[derive(Clone, Debug, PartialEq)]
pub struct Encoded<D> {
    pub item: D,
    pub byte_vector: Vec<u8>,
}

/// A piece of data plus its produce source (port of `Datum`).
#[derive(Clone, Debug, PartialEq)]
pub struct Datum<A> {
    pub a: A,
    pub persist: bool,
    pub source: Produce,
}

impl<A> Datum<A> {
    pub fn create<C>(channel: &C, a: A, persist: bool) -> Self
    where
        C: Serialize<C>,
        A: Serialize<A>,
    {
        let source = Produce::apply(channel, &a, persist);
        Datum { a, persist, source }
    }
}

/// A waiting continuation plus its consume source (port of `WaitingContinuation`).
#[derive(Clone, Debug, PartialEq)]
pub struct WaitingContinuation<P, K> {
    pub patterns: Vec<P>,
    pub continuation: K,
    pub persist: bool,
    pub peeks: BTreeSet<usize>,
    pub source: Consume,
}

impl<P, K> WaitingContinuation<P, K> {
    pub fn create<C>(
        channels: &[C],
        patterns: Vec<P>,
        continuation: K,
        persist: bool,
        peeks: BTreeSet<usize>,
    ) -> Self
    where
        C: Serialize<C>,
        P: Serialize<P>,
        K: Serialize<K>,
    {
        let source = Consume::apply(channels, &patterns, &continuation, persist);
        WaitingContinuation {
            patterns,
            continuation,
            persist,
            peeks,
            source,
        }
    }
}

/// A matched datum candidate during consume (port of `ConsumeCandidate`).
#[derive(Clone, Debug, PartialEq)]
pub struct ConsumeCandidate<C, A> {
    pub channel: C,
    pub datum: Datum<A>,
    pub removed_datum: A,
    pub datum_index: i64,
}

/// A matched continuation candidate during produce (port of `ProduceCandidate`).
#[derive(Clone, Debug, PartialEq)]
pub struct ProduceCandidate<C, P, A, K> {
    pub channels: Vec<C>,
    pub continuation: WaitingContinuation<P, K>,
    pub continuation_index: usize,
    pub data_candidates: Vec<ConsumeCandidate<C, A>>,
}

/// A row of data and waiting continuations at a channel (port of `Row`).
#[derive(Clone, Debug, PartialEq)]
pub struct Row<P, A, K> {
    pub data: Vec<Datum<A>>,
    pub wks: Vec<WaitingContinuation<P, K>>,
}

impl<P, A, K> Default for Row<P, A, K> {
    fn default() -> Self {
        Row {
            data: Vec::new(),
            wks: Vec::new(),
        }
    }
}

/// A multi-map whose values form a multiset (port of `MultisetMultiMap`).
#[derive(Clone, Debug, Default)]
pub struct MultisetMultiMap<K, V> {
    map: HashMap<K, Vec<V>>,
}

impl<K, V> MultisetMultiMap<K, V>
where
    K: Eq + Hash,
    V: PartialEq,
{
    pub fn empty() -> Self {
        MultisetMultiMap {
            map: HashMap::new(),
        }
    }

    pub fn add_binding(&mut self, key: K, value: V) {
        self.map.entry(key).or_default().push(value);
    }

    pub fn remove_binding(&mut self, key: K, value: V) {
        let mut remove_key = false;
        if let Some(values) = self.map.get_mut(&key) {
            if let Some(pos) = values.iter().position(|v| v == &value) {
                values.remove(pos);
            }
            remove_key = values.is_empty();
        }
        if remove_key {
            self.map.remove(&key);
        }
    }
}

/// An installed continuation (port of `Install`; the unused `F`/`A` parameters are dropped).
#[derive(Clone, Debug, PartialEq)]
pub struct Install<P, K> {
    pub patterns: Vec<P>,
    pub continuation: K,
}

/// The installed-continuation map (port of `Installs`).
pub type Installs<C, P, K> = HashMap<Vec<C>, Install<P, K>>;

#[cfg(test)]
mod tests {
    use super::*;

    /// `Datum::create` derives the produce source from the datum itself, so the source's hash and
    /// bytes are the ones a lookup will match on — a datum whose `a` and `source` disagreed would
    /// insert under one hash and be searched for under another.
    #[test]
    fn a_datum_derives_its_produce_source_from_its_value() {
        let datum = Datum::create(&String::from("chan"), String::from("datum"), true);
        assert_eq!(datum.a, "datum");
        assert!(datum.persist);
        assert_eq!(
            datum.source,
            Produce::apply(&String::from("chan"), &String::from("datum"), true)
        );
        assert!(
            datum.source.persistent,
            "the datum's persistence is the source's"
        );

        // A non-persistent datum (the default in a COMM) is a linear produce.
        let linear = Datum::create(&String::from("chan"), String::from("datum"), false);
        assert!(!linear.persist);
        assert!(!linear.source.persistent);
        assert_ne!(
            linear.source.hash, datum.source.hash,
            "persistence is part of the produce identity"
        );
    }

    /// `WaitingContinuation::create` derives the consume source from the channels, patterns and
    /// body — the continuation's own hash is over those three, so a mismatch is a continuation that
    /// cannot be found again.
    #[test]
    fn a_waiting_continuation_derives_its_consume_source() {
        let wk: WaitingContinuation<String, String> = WaitingContinuation::create(
            &["a".to_string(), "b".to_string()],
            vec!["pat".to_string()],
            "body".to_string(),
            false,
            BTreeSet::from([0usize]),
        );
        assert_eq!(wk.patterns, vec!["pat".to_string()]);
        assert_eq!(wk.continuation, "body");
        assert!(!wk.persist);
        assert_eq!(wk.peeks, BTreeSet::from([0usize]));
        assert_eq!(
            wk.source,
            Consume::apply(
                &["a".to_string(), "b".to_string()],
                &["pat".to_string()],
                &"body".to_string(),
                false
            )
        );
        assert_eq!(
            wk.source.channels_hashes.len(),
            2,
            "one channel hash per channel, in order"
        );
        assert_ne!(
            wk.source.channels_hashes[0], wk.source.channels_hashes[1],
            "the two channels are distinct"
        );
    }

    /// A row starts empty, and the two halves are independent.
    #[test]
    fn a_row_starts_empty() {
        let row: Row<String, String, String> = Row::default();
        assert!(row.data.is_empty());
        assert!(row.wks.is_empty());
    }

    /// The candidate carriers hold what they say: the consume candidate pairs a datum with the
    /// **removed** value (the peek/consume distinction lives in the two fields, not in the datum),
    /// and the produce candidate carries its data candidates in channel order.
    #[test]
    fn the_candidates_carry_their_parts() {
        let datum = Datum::create(&String::from("chan"), String::from("datum"), false);
        let candidate = ConsumeCandidate {
            channel: "chan",
            datum: datum.clone(),
            removed_datum: String::from("removed"),
            datum_index: 3,
        };
        assert_eq!(candidate.datum, datum);
        assert_eq!(candidate.removed_datum, "removed");
        assert_ne!(
            candidate.removed_datum, candidate.datum.a,
            "the removed value and the datum are separate fields (a peek removes nothing)"
        );
        assert_eq!(candidate.datum_index, 3);

        let produce = ProduceCandidate {
            channels: vec!["chan"],
            continuation: WaitingContinuation::create(
                &[String::from("chan")],
                vec![String::from("pat")],
                String::from("body"),
                false,
                BTreeSet::new(),
            ),
            continuation_index: 1,
            data_candidates: vec![candidate],
        };
        assert_eq!(produce.channels.len(), 1);
        assert_eq!(produce.continuation_index, 1);
        assert_eq!(produce.data_candidates.len(), 1);
    }

    /// A multiset multi-map: two bindings of the *same* value under one key are two entries, so
    /// removing one leaves the other behind — the multiset semantics a plain map-of-sets would not
    /// have.
    ///
    /// This is the only test here that reads the derived `Debug` output, and it says why: the inner
    /// map is private and the type has no accessor, so nothing else can observe it. It also records
    /// the larger fact a reviewer should weigh — **the type has no caller in the port.** In the
    /// Scala it is `ReplayData` (`rspace/trace/package.scala`); the port inlined that as
    /// `replay_rspace::ReplayData` (two `BTreeMap`s), so this is a faithful port of a type the port
    /// does not use, and deleting it would be the honest alternative to the test below.
    #[test]
    fn a_binding_is_a_multiset_entry_not_a_set_member() {
        let mut bindings: MultisetMultiMap<&str, &str> = MultisetMultiMap::empty();
        assert_eq!(
            format!("{bindings:?}"),
            format!("{:?}", MultisetMultiMap::<&str, &str>::default())
        );

        bindings.add_binding("k", "v");
        bindings.add_binding("k", "v");
        assert!(format!("{bindings:?}").contains("v"));

        // One removal takes one of the two; the other is still there.
        bindings.remove_binding("k", "v");
        assert!(format!("{bindings:?}").contains("k"));
        bindings.remove_binding("k", "v");
        assert!(
            !format!("{bindings:?}").contains('k'),
            "removing the last binding drops the key rather than leaving an empty vector"
        );

        // Removing what was never added, or from a key that is gone, is a no-op — not a panic and
        // not a removal of some other key's value.
        let mut other: MultisetMultiMap<&str, &str> = MultisetMultiMap::empty();
        other.add_binding("k", "kept");
        other.remove_binding("k", "absent");
        other.remove_binding("missing", "kept");
        let printed = format!("{other:?}");
        assert!(printed.contains("kept"), "{printed}");
        assert!(!printed.contains("missing"), "{printed}");
    }
}
