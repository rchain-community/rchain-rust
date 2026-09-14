//! Turning a [`StateChange`] into hash-addressed trie actions (Law 9: deterministic merge).
//!
//! Mirrors `rspace/src/main/scala/coop/rchain/rspace/merger/StateChangeMerger.scala`.

use std::collections::BTreeMap;

use rchain_crypto::hash::blake2b256_hash::Blake2b256Hash;

use crate::hashing::stable_hash_provider::hash_hashes;
use crate::history::history_reader::HistoryReaderBinary;
use crate::hot_store_trie_action::HotStoreTrieAction;
use crate::merger::channel_change::ChannelChange;
use crate::merger::event_log_index::NumberChannelsDiff;
use crate::merger::seq_diff;
use crate::merger::state_change::StateChange;

/// A join to add or remove on merge (port of `StateChangeMerger.JoinAction`).
enum JoinAction {
    AddJoin(Vec<Blake2b256Hash>),
    RemoveJoin(Vec<Blake2b256Hash>),
}

impl JoinAction {
    fn channels(&self) -> &[Blake2b256Hash] {
        match self {
            JoinAction::AddJoin(channels) | JoinAction::RemoveJoin(channels) => channels,
        }
    }
}

/// Produce a single trie action from a channel change against the base reader (port of
/// `StateChangeMerger.mkTrieAction`).
async fn mk_trie_action<C, P, A, K, R, U>(
    history_pointer: Blake2b256Hash,
    init_value: Vec<Vec<u8>>,
    changes: &ChannelChange<Vec<u8>>,
    remove_action: R,
    update_action: U,
) -> Result<HotStoreTrieAction<C, P, A, K>, String>
where
    R: FnOnce(Blake2b256Hash) -> HotStoreTrieAction<C, P, A, K>,
    U: FnOnce(Blake2b256Hash, Vec<Vec<u8>>) -> HotStoreTrieAction<C, P, A, K>,
{
    let new_val = {
        let mut v = seq_diff(&init_value, &changes.removed);
        v.extend(changes.added.clone());
        v
    };
    if new_val.is_empty() && !init_value.is_empty() {
        Ok(remove_action(history_pointer))
    } else if init_value != new_val {
        Ok(update_action(history_pointer, new_val))
    } else {
        Err(
            "Merging logic error: empty channel change for produce or join when computing trie action."
                .to_string(),
        )
    }
}

/// Compute the trie actions that apply a [`StateChange`] on top of a base state (port of
/// `StateChangeMerger.computeTrieActions`).
///
/// `handle_channel_change` is the mergeable-channel override hook; in the Scala it is effectful
/// (`=> F[Option[HotStoreTrieAction]]`), here it is a plain `Option`-returning closure.
pub async fn compute_trie_actions<C, P, A, K, F>(
    changes: &StateChange,
    base_reader: &(dyn HistoryReaderBinary<C, P, A, K> + Sync),
    mergeable_chs: NumberChannelsDiff,
    handle_channel_change: F,
) -> Result<Vec<HotStoreTrieAction<C, P, A, K>>, String>
where
    C: Send + Sync,
    P: Send + Sync,
    A: Send + Sync,
    K: Send + Sync,
    F: Fn(
        &Blake2b256Hash,
        &ChannelChange<Vec<u8>>,
        &NumberChannelsDiff,
    ) -> Option<HotStoreTrieAction<C, P, A, K>>,
{
    // Consume trie actions and the joins they add/remove.
    let mut consume_trie_actions: Vec<HotStoreTrieAction<C, P, A, K>> = Vec::new();
    let mut join_actions: Vec<JoinAction> = Vec::new();

    for (consume_channels, channel_change) in &changes.kont_changes {
        let history_pointer = hash_hashes(consume_channels);
        let init: Vec<Vec<u8>> = base_reader
            .get_continuations(history_pointer)
            .await
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|wc| wc.raw)
            .collect();
        let new_val = {
            let mut v = seq_diff(&init, &channel_change.removed);
            v.extend(channel_change.added.clone());
            v
        };
        if init == new_val {
            return Err(
                "Merging logic error: empty consume change when computing trie action.".to_string(),
            );
        }
        if new_val.is_empty() && !init.is_empty() {
            consume_trie_actions.push(HotStoreTrieAction::TrieDeleteConsume(history_pointer));
            join_actions.push(JoinAction::RemoveJoin(consume_channels.clone()));
        } else if init.is_empty() {
            consume_trie_actions.push(HotStoreTrieAction::TrieInsertBinaryConsume(
                history_pointer,
                new_val,
            ));
            join_actions.push(JoinAction::AddJoin(consume_channels.clone()));
        } else {
            consume_trie_actions.push(HotStoreTrieAction::TrieInsertBinaryConsume(
                history_pointer,
                new_val,
            ));
        }
    }

    // Produce trie actions.
    let mut produce_trie_actions: Vec<HotStoreTrieAction<C, P, A, K>> = Vec::new();
    for (history_pointer, channel_change) in &changes.datums_changes {
        match handle_channel_change(history_pointer, channel_change, &mergeable_chs) {
            Some(action) => produce_trie_actions.push(action),
            None => {
                let init: Vec<Vec<u8>> = base_reader
                    .get_data(*history_pointer)
                    .await
                    .map_err(|e| e.to_string())?
                    .into_iter()
                    .map(|d| d.raw)
                    .collect();
                let action = mk_trie_action(
                    *history_pointer,
                    init,
                    channel_change,
                    HotStoreTrieAction::TrieDeleteProduce,
                    HotStoreTrieAction::TrieInsertBinaryProduce,
                )
                .await?;
                produce_trie_actions.push(action);
            }
        }
    }

    // Join trie actions: spread each join add/remove over its member channels.
    let mut joins_changes: BTreeMap<Blake2b256Hash, ChannelChange<Vec<u8>>> = BTreeMap::new();
    for join_action in &join_actions {
        let join_channels = join_action.channels();
        let join = changes
            .consume_channels_to_join_serialized_map
            .get(join_channels)
            .ok_or_else(|| {
                "No ByteVector value for join found when merging when computing trie action."
                    .to_string()
            })?;
        for c in join_channels {
            let cur = joins_changes.remove(c).unwrap_or_default();
            let new_val = match join_action {
                JoinAction::AddJoin(_) => ChannelChange {
                    added: [vec![join.clone()], cur.added].concat(),
                    removed: cur.removed,
                },
                JoinAction::RemoveJoin(_) => ChannelChange {
                    added: cur.added,
                    removed: [vec![join.clone()], cur.removed].concat(),
                },
            };
            joins_changes.insert(*c, new_val);
        }
    }

    let mut joins_trie_actions: Vec<HotStoreTrieAction<C, P, A, K>> = Vec::new();
    for (history_pointer, channel_change) in &joins_changes {
        let init: Vec<Vec<u8>> = base_reader
            .get_joins(*history_pointer)
            .await
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|j| j.raw)
            .collect();
        let action = mk_trie_action(
            *history_pointer,
            init,
            channel_change,
            HotStoreTrieAction::TrieDeleteJoins,
            HotStoreTrieAction::TrieInsertBinaryJoins,
        )
        .await?;
        joins_trie_actions.push(action);
    }

    let mut result = produce_trie_actions;
    result.extend(consume_trie_actions);
    result.extend(joins_trie_actions);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(n: u8) -> Blake2b256Hash {
        Blake2b256Hash::from_bytes([n; 32])
    }

    fn change(added: Vec<Vec<u8>>, removed: Vec<Vec<u8>>) -> ChannelChange<Vec<u8>> {
        ChannelChange { added, removed }
    }

    /// `JoinAction`'s two variants answer the same channel list — the merge reads the channels off
    /// whichever action it built, so a variant that carried them differently would send the trie
    /// action to the wrong place.
    #[test]
    fn join_actions_report_their_channels() {
        let channels = vec![hash(1), hash(2)];
        assert_eq!(JoinAction::AddJoin(channels.clone()).channels(), channels);
        assert_eq!(
            JoinAction::RemoveJoin(channels.clone()).channels(),
            channels
        );
    }

    /// The three outcomes of a channel change: everything removed ⇒ a removal action; some content
    /// left ⇒ an update action carrying exactly the new value. The value is `init − removed + added`,
    /// so a removal actually takes effect and an addition lands (Law 9's deterministic merge).
    #[tokio::test]
    async fn a_channel_change_produces_a_remove_or_an_update_action() {
        type Action = HotStoreTrieAction<String, String, String, String>;

        // Everything removed and nothing added: the channel is emptied.
        let removed = mk_trie_action::<String, String, String, String, _, _>(
            hash(9),
            vec![b"a".to_vec(), b"b".to_vec()],
            &change(vec![], vec![b"a".to_vec(), b"b".to_vec()]),
            |pointer| Action::TrieDeleteProduce(pointer),
            |_, _| panic!("an emptied channel must be removed, not updated"),
        )
        .await
        .expect("removal");
        assert!(
            matches!(removed, Action::TrieDeleteProduce(..)),
            "{removed:?}"
        );

        // A partial change: the update action carries the survivors plus the addition.
        let updated = mk_trie_action::<String, String, String, String, _, _>(
            hash(9),
            vec![b"a".to_vec(), b"b".to_vec()],
            &change(vec![b"c".to_vec()], vec![b"a".to_vec()]),
            |_| panic!("a non-empty channel must be updated, not removed"),
            |_, value| Action::TrieInsertBinaryProduce(Blake2b256Hash::from_bytes([0; 32]), value),
        )
        .await
        .expect("update");
        match updated {
            Action::TrieInsertBinaryProduce(_, value) => {
                assert_eq!(value, vec![b"b".to_vec(), b"c".to_vec()]);
            }
            other => panic!("expected an update, got {other:?}"),
        }
    }

    /// **A change that changes nothing is an error, not a no-op.** The merger is called only when
    /// there is a difference to merge, so `init == new` means the caller and the merger disagree
    /// about the channel's state — silently returning a no-op action would hide that and leave the
    /// trie action queue inconsistent with the values it was computed from.
    #[tokio::test]
    async fn an_empty_channel_change_is_reported_as_an_error() {
        type Action = HotStoreTrieAction<String, String, String, String>;

        let err = mk_trie_action::<String, String, String, String, _, _>(
            hash(9),
            vec![b"a".to_vec()],
            &change(vec![], vec![]),
            |_| Action::TrieDeleteProduce(Blake2b256Hash::from_bytes([0; 32])),
            |_, _| panic!("a no-op change must not reach an action"),
        )
        .await
        .expect_err("a change that changes nothing must be reported");
        assert!(err.contains("Merging logic error"), "{err}");

        // The degenerate all-empty case is the same error, not a removal of nothing.
        let err = mk_trie_action::<String, String, String, String, _, _>(
            hash(9),
            Vec::new(),
            &change(vec![], vec![]),
            |_| Action::TrieDeleteProduce(Blake2b256Hash::from_bytes([0; 32])),
            |_, _| panic!("a no-op change must not reach an action"),
        )
        .await
        .expect_err("an empty change against an empty channel is reported too");
        assert!(err.contains("Merging logic error"), "{err}");
    }
}
