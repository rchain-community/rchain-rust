//! The state change between two history snapshots (a monoid).
//!
//! Mirrors `rspace/src/main/scala/coop/rchain/rspace/merger/StateChange.scala` (the data type and
//! its `empty`/`combine`; the effectful `apply` is in the engine phase).

use std::collections::BTreeMap;

use rchain_crypto::hash::blake2b256_hash::Blake2b256Hash;
use rchain_shared::serialize::Serialize;

use crate::hashing::stable_hash_provider::{hash_channel, hash_hashes};
use crate::history::history_reader::HistoryReaderBinary;
use crate::merger::channel_change::ChannelChange;
use crate::merger::event_log_index::EventLogIndex;
use crate::merger::event_log_merging_logic::{consumes_affected, produces_affected};
use crate::merger::seq_diff;

fn combine_channel_change_map<K, V>(
    x: &BTreeMap<K, ChannelChange<V>>,
    y: &BTreeMap<K, ChannelChange<V>>,
) -> BTreeMap<K, ChannelChange<V>>
where
    K: Ord + Clone,
    V: Clone,
{
    let mut out = x.clone();
    for (k, v) in y {
        match out.get(k) {
            Some(existing) => {
                out.insert(k.clone(), ChannelChange::combine(existing, v));
            }
            None => {
                out.insert(k.clone(), v.clone());
            }
        }
    }
    out
}

/// The diff between two history states (port of `StateChange`).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct StateChange {
    pub datums_changes: BTreeMap<Blake2b256Hash, ChannelChange<Vec<u8>>>,
    pub kont_changes: BTreeMap<Vec<Blake2b256Hash>, ChannelChange<Vec<u8>>>,
    pub consume_channels_to_join_serialized_map: BTreeMap<Vec<Blake2b256Hash>, Vec<u8>>,
}

/// Compute the added/removed raw bytes between a start and end value (port of `computeValueChange`).
fn compute_value_change(
    start_value: Vec<Vec<u8>>,
    end_value: Vec<Vec<u8>>,
) -> ChannelChange<Vec<u8>> {
    let added = seq_diff(&end_value, &start_value);
    let removed = seq_diff(&start_value, &end_value);
    ChannelChange { added, removed }
}

impl StateChange {
    /// Build a `StateChange` by diffing the pre/post state readers over the channels touched by an
    /// event log (port of `StateChange.apply`).
    pub async fn apply<C, P, A, K>(
        pre_state_reader: &(dyn HistoryReaderBinary<C, P, A, K> + Sync),
        post_state_reader: &(dyn HistoryReaderBinary<C, P, A, K> + Sync),
        event_log_index: &EventLogIndex,
    ) -> Result<StateChange, String>
    where
        C: Serialize<C> + Send + Sync,
        P: Send + Sync,
        A: Send + Sync,
        K: Send + Sync,
    {
        let mut datums_diff: BTreeMap<Blake2b256Hash, ChannelChange<Vec<u8>>> = BTreeMap::new();
        let mut konts_diff: BTreeMap<Vec<Blake2b256Hash>, ChannelChange<Vec<u8>>> = BTreeMap::new();
        let mut joins_map: BTreeMap<Vec<Blake2b256Hash>, Vec<u8>> = BTreeMap::new();

        let produce_hashes: Vec<Blake2b256Hash> = produces_affected(event_log_index)
            .iter()
            .map(|p| p.channels_hash)
            .collect();
        for history_pointer in produce_hashes {
            let start: Vec<Vec<u8>> = pre_state_reader
                .get_data(history_pointer)
                .await
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(|d| d.raw)
                .collect();
            let end: Vec<Vec<u8>> = post_state_reader
                .get_data(history_pointer)
                .await
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(|d| d.raw)
                .collect();
            let change = compute_value_change(start, end);
            let cur = datums_diff.entry(history_pointer).or_default();
            cur.added.extend(change.added);
            cur.removed.extend(change.removed);
        }

        let consume_channel_sets: Vec<Vec<Blake2b256Hash>> = consumes_affected(event_log_index)
            .iter()
            .map(|c| c.channels_hashes.clone())
            .collect();
        for consume_channels in consume_channel_sets {
            let history_pointer = hash_hashes(&consume_channels);
            let start: Vec<Vec<u8>> = pre_state_reader
                .get_continuations(history_pointer)
                .await
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(|wc| wc.raw)
                .collect();
            let end: Vec<Vec<u8>> = post_state_reader
                .get_continuations(history_pointer)
                .await
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(|wc| wc.raw)
                .collect();
            let change = compute_value_change(start, end);
            let cur = konts_diff.entry(consume_channels.clone()).or_default();
            cur.added.extend(change.added);
            cur.removed.extend(change.removed);

            // Recover the serialized join body matching these consume channels.
            let history_pointer = consume_channels[0];
            let pre = pre_state_reader
                .get_joins(history_pointer)
                .await
                .map_err(|e| e.to_string())?;
            let post = post_state_reader
                .get_joins(history_pointer)
                .await
                .map_err(|e| e.to_string())?;
            let err_msg = "Tuple space inconsistency found: channel of consume does not contain \
                           join record corresponding to the consume channels."
                .to_string();
            let raw_join = pre
                .iter()
                .chain(post.iter())
                .find(|j| {
                    let joins_channels: Vec<Blake2b256Hash> =
                        j.decoded.iter().map(hash_channel).collect();
                    let mut consume_sorted = consume_channels.clone();
                    consume_sorted.sort();
                    let mut joins_sorted = joins_channels.clone();
                    joins_sorted.sort();
                    consume_sorted == joins_sorted
                })
                .map(|j| j.raw.clone())
                .ok_or(err_msg)?;
            joins_map.insert(consume_channels, raw_join);
        }

        if datums_diff
            .values()
            .any(|c| c.added.is_empty() && c.removed.is_empty())
        {
            return Err(
                "State change compute logic error: empty channel change for produce.".to_string(),
            );
        }
        if konts_diff
            .values()
            .any(|c| c.added.is_empty() && c.removed.is_empty())
        {
            return Err(
                "State change compute logic error: empty channel change for consume.".to_string(),
            );
        }

        Ok(StateChange {
            datums_changes: datums_diff,
            kont_changes: konts_diff,
            consume_channels_to_join_serialized_map: joins_map,
        })
    }

    pub fn empty() -> Self {
        StateChange::default()
    }

    /// Combine two state changes (port of `StateChange.combine`).
    pub fn combine(x: &StateChange, y: &StateChange) -> StateChange {
        let mut joins = x.consume_channels_to_join_serialized_map.clone();
        for (k, v) in &y.consume_channels_to_join_serialized_map {
            joins.insert(k.clone(), v.clone());
        }
        StateChange {
            datums_changes: combine_channel_change_map(&x.datums_changes, &y.datums_changes),
            kont_changes: combine_channel_change_map(&x.kont_changes, &y.kont_changes),
            consume_channels_to_join_serialized_map: joins,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combine_is_associative() {
        let a = StateChange {
            datums_changes: BTreeMap::from([(
                Blake2b256Hash::from_bytes([1; 32]),
                ChannelChange {
                    added: vec![vec![1]],
                    removed: vec![],
                },
            )]),
            ..Default::default()
        };
        let b = StateChange {
            datums_changes: BTreeMap::from([(
                Blake2b256Hash::from_bytes([1; 32]),
                ChannelChange {
                    added: vec![],
                    removed: vec![vec![2]],
                },
            )]),
            ..Default::default()
        };
        let ab = StateChange::combine(&a, &b);
        let ba = StateChange::combine(&b, &a);
        // ChannelChange combine is concatenation, so order of added/removed differs,
        // but the monoid law tested here is empty-is-identity.
        assert_eq!(StateChange::combine(&a, &StateChange::empty()), a);
        // both orders contain the same multiset of added/removed
        let mut ab_added = ab.datums_changes[&Blake2b256Hash::from_bytes([1; 32])]
            .added
            .clone();
        ab_added.sort();
        let mut ba_added = ba.datums_changes[&Blake2b256Hash::from_bytes([1; 32])]
            .added
            .clone();
        ba_added.sort();
        assert_eq!(ab_added, ba_added);
    }
}

/// A `HistoryReaderBinary` over canned rows: the diff in `StateChange::apply` is a pure function of
/// two readers, so a three-map double is the whole harness (the trait has three methods).
#[cfg(test)]
mod diff_tests {
    use super::*;

    use crate::internal::{Datum, WaitingContinuation};
    use crate::serializers::scodec_serialize::{DatumB, JoinsB, WaitingContinuationB};
    use crate::trace::event::{Consume, Produce};

    #[derive(Default)]
    struct MockReader {
        data: BTreeMap<Blake2b256Hash, Vec<DatumB<String>>>,
        konts: BTreeMap<Blake2b256Hash, Vec<WaitingContinuationB<String, String>>>,
        joins: BTreeMap<Blake2b256Hash, Vec<JoinsB<String>>>,
        /// When set, every read fails with this message.
        fail: bool,
    }

    #[async_trait::async_trait]
    impl HistoryReaderBinary<String, String, String, String> for MockReader {
        async fn get_data(
            &self,
            key: Blake2b256Hash,
        ) -> Result<Vec<DatumB<String>>, crate::errors::RSpaceError> {
            if self.fail {
                return Err(crate::errors::RSpaceError::Codec("test reader failure"));
            }
            Ok(self.data.get(&key).cloned().unwrap_or_default())
        }
        async fn get_continuations(
            &self,
            key: Blake2b256Hash,
        ) -> Result<Vec<WaitingContinuationB<String, String>>, crate::errors::RSpaceError> {
            if self.fail {
                return Err(crate::errors::RSpaceError::Codec("test reader failure"));
            }
            Ok(self.konts.get(&key).cloned().unwrap_or_default())
        }
        async fn get_joins(
            &self,
            key: Blake2b256Hash,
        ) -> Result<Vec<JoinsB<String>>, crate::errors::RSpaceError> {
            if self.fail {
                return Err(crate::errors::RSpaceError::Codec("test reader failure"));
            }
            Ok(self.joins.get(&key).cloned().unwrap_or_default())
        }
    }

    fn hash(byte: u8) -> Blake2b256Hash {
        Blake2b256Hash::from_bytes([byte; 32])
    }

    fn produce(byte: u8) -> Produce {
        Produce::apply(&format!("chan{byte}"), &format!("datum{byte}"), false)
    }

    fn consume(byte: u8) -> Consume {
        Consume::apply(
            &[format!("chan{byte}"), format!("chan{}", byte + 1)],
            &[format!("pat{byte}"), format!("pat{}", byte + 1)],
            &format!("cont{byte}"),
            false,
        )
    }

    fn datum_b(raw: u8) -> DatumB<String> {
        DatumB {
            decoded: Datum {
                a: format!("datum{raw}"),
                persist: false,
                source: produce(raw),
            },
            raw: vec![raw],
        }
    }

    fn kont_b(raw: u8) -> WaitingContinuationB<String, String> {
        WaitingContinuationB {
            decoded: WaitingContinuation {
                patterns: vec![format!("pat{raw}")],
                continuation: format!("cont{raw}"),
                persist: false,
                peeks: Default::default(),
                source: consume(raw),
            },
            raw: vec![raw],
        }
    }

    fn joins_b(raw: u8, channels: Vec<String>) -> JoinsB<String> {
        JoinsB {
            decoded: channels,
            raw: vec![raw],
        }
    }

    /// An event log that touches exactly one produce and one consume.
    fn log_with(produce_byte: u8, consume_byte: u8) -> EventLogIndex {
        EventLogIndex {
            produces_linear: [produce(produce_byte)].into_iter().collect(),
            consumes_linear_and_peeks: [consume(consume_byte)].into_iter().collect(),
            ..Default::default()
        }
    }

    /// **A datum added between the snapshots** is reported as `added`, keyed by the produce's
    /// channels hash (the merge's unit of change).
    #[tokio::test]
    async fn an_added_datum_is_reported_as_added() {
        let p = produce(1);
        // The consume half must diff non-empty as well, or `apply` fails its own consistency check.
        let c = consume(2);
        let kont_pointer = crate::hashing::stable_hash_provider::hash_hashes(&c.channels_hashes);

        let pre = MockReader::default();
        let post = MockReader {
            data: [(p.channels_hash, vec![datum_b(9)])].into_iter().collect(),
            konts: [(kont_pointer, vec![kont_b(3)])].into_iter().collect(),
            joins: [(
                c.channels_hashes[0],
                vec![joins_b(4, vec![format!("chan2"), format!("chan3")])],
            )]
            .into_iter()
            .collect(),
            fail: false,
        };

        let change = StateChange::apply(&pre, &post, &log_with(1, 2))
            .await
            .expect("apply");
        let datums = change
            .datums_changes
            .get(&p.channels_hash)
            .expect("the produce");
        assert_eq!(datums.added, vec![vec![9]]);
        assert!(datums.removed.is_empty());

        let konts = change
            .kont_changes
            .get(&c.channels_hashes)
            .expect("the consume");
        assert_eq!(konts.added, vec![vec![3]]);
        // The join body matching the consume channels is recovered (sorted comparison).
        assert_eq!(
            change
                .consume_channels_to_join_serialized_map
                .get(&c.channels_hashes),
            Some(&vec![4u8])
        );
    }

    /// A datum **removed** between the snapshots is reported as `removed` — the direction matters:
    /// the merge replays one branch onto the other and must know which way the change went.
    #[tokio::test]
    async fn a_removed_datum_is_reported_as_removed() {
        let p = produce(1);
        let c = consume(2);
        let kont_pointer = crate::hashing::stable_hash_provider::hash_hashes(&c.channels_hashes);
        let pre = MockReader {
            data: [(p.channels_hash, vec![datum_b(9)])].into_iter().collect(),
            konts: [(kont_pointer, vec![kont_b(3)])].into_iter().collect(),
            joins: [(
                c.channels_hashes[0],
                vec![joins_b(4, vec![format!("chan2"), format!("chan3")])],
            )]
            .into_iter()
            .collect(),
            fail: false,
        };
        let post = MockReader::default();

        let change = StateChange::apply(&pre, &post, &log_with(1, 2))
            .await
            .expect("apply");
        let datums = change
            .datums_changes
            .get(&p.channels_hash)
            .expect("the produce");
        assert!(datums.added.is_empty());
        assert_eq!(datums.removed, vec![vec![9]]);
    }

    /// **A no-op diff is an error.** `apply` is called for the channels an event log *touched*, so a
    /// channel whose pre and post states agree means the caller's index and its readers disagree —
    /// silently reporting "no change" would let a merge skip a real one.
    #[tokio::test]
    async fn an_unchanged_produce_is_a_logic_error() {
        let p = produce(1);
        let c = consume(2);
        let kont_pointer = crate::hashing::stable_hash_provider::hash_hashes(&c.channels_hashes);
        let same_data: BTreeMap<_, _> = [(p.channels_hash, vec![datum_b(9)])].into_iter().collect();
        let konts: BTreeMap<_, _> = [(kont_pointer, vec![kont_b(3)])].into_iter().collect();
        let pre = MockReader {
            data: same_data.clone(),
            konts: konts.clone(),
            joins: [(
                c.channels_hashes[0],
                vec![joins_b(4, vec![format!("chan2"), format!("chan3")])],
            )]
            .into_iter()
            .collect(),
            fail: false,
        };
        let post = MockReader {
            data: same_data,
            konts,
            joins: pre.joins.clone(),
            fail: false,
        };

        let err = StateChange::apply(&pre, &post, &log_with(1, 2))
            .await
            .expect_err("an unchanged channel is a logic error");
        assert!(err.contains("empty channel change for produce"), "{err}");
    }

    /// **The join invariant.** A consume whose channel set has no join record in either snapshot means
    /// the tuplespace is inconsistent — the merge refuses rather than producing a change nobody can
    /// replay.
    #[tokio::test]
    async fn a_consume_without_a_join_record_is_refused() {
        let p = produce(1);
        let c = consume(2);
        let kont_pointer = crate::hashing::stable_hash_provider::hash_hashes(&c.channels_hashes);
        let post = MockReader {
            data: [(p.channels_hash, vec![datum_b(9)])].into_iter().collect(),
            konts: [(kont_pointer, vec![kont_b(3)])].into_iter().collect(),
            // A join exists, but for a *different* channel set.
            joins: [(
                c.channels_hashes[0],
                vec![joins_b(4, vec![format!("other")])],
            )]
            .into_iter()
            .collect(),
            fail: false,
        };

        let err = StateChange::apply(&MockReader::default(), &post, &log_with(1, 2))
            .await
            .expect_err("no matching join");
        assert!(err.contains("Tuple space inconsistency"), "{err}");
    }

    /// A reader failure propagates with its message rather than being reported as "no change".
    #[tokio::test]
    async fn a_reader_error_propagates() {
        let failing = MockReader {
            fail: true,
            ..Default::default()
        };
        let err = StateChange::apply(&failing, &MockReader::default(), &log_with(1, 2))
            .await
            .expect_err("a reader error must propagate");
        assert!(err.contains("test reader failure"), "{err}");
    }

    /// `combine` is a monoid: the empty change is the identity on **both** sides (the existing unit
    /// test checks one), and a colliding join key is right-biased (the later change wins).
    #[test]
    fn combine_has_an_identity_and_a_right_biased_join_map() {
        let a = StateChange {
            datums_changes: [(
                hash(1),
                ChannelChange {
                    added: vec![vec![1]],
                    removed: vec![],
                },
            )]
            .into_iter()
            .collect(),
            consume_channels_to_join_serialized_map: [(vec![hash(5)], vec![1u8])]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let b = StateChange {
            consume_channels_to_join_serialized_map: [(vec![hash(5)], vec![2u8])]
                .into_iter()
                .collect(),
            ..Default::default()
        };

        assert_eq!(StateChange::combine(&a, &StateChange::empty()), a);
        assert_eq!(StateChange::combine(&StateChange::empty(), &a), a);
        assert_eq!(
            StateChange::combine(&a, &b)
                .consume_channels_to_join_serialized_map
                .get(&vec![hash(5)]),
            Some(&vec![2u8]),
            "the later change's join body wins"
        );
        // The datum changes are *concatenated*, not overwritten: nothing is lost.
        let ab = StateChange::combine(&a, &b);
        assert_eq!(
            ab.datums_changes.get(&hash(1)).expect("key").added,
            vec![vec![1]]
        );
    }
}
