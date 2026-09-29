//! The native system-contract store: byte-oriented state (registry / PoS / vault) folded into the
//! same content-addressed radix trie as the tuple space.
//!
//! Native state is stored under dedicated trie prefixes (`PREFIX_REGISTRY`/`PREFIX_POS`/
//! `PREFIX_VAULT`) as `NativeLeaf` payloads. The `InMemNativeStore` is a write-through overlay on top
//! of a `NativeHistoryReader`: reads fall through to the persisted trie, writes are buffered in the
//! overlay, and `drain_changes` produces the `NativeStoreAction`s that the history repository folds
//! into the next checkpoint. This mirrors the `HotStore`/`HistoryRepository` split so native state
//! stays content-addressed, replayable, and queryable at an arbitrary state hash.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use async_trait::async_trait;
use rchain_crypto::hash::blake2b256_hash::Blake2b256Hash;

/// Trie prefix for native registry entries (`uri-bytes -> Par`).
pub const PREFIX_REGISTRY: u8 = 0x03;
/// Trie prefix for native PoS state (`bonds` / `active` / `withdrawers` / `params` leaves).
pub const PREFIX_POS: u8 = 0x04;
/// Trie prefix for native vault state (`rev-address -> balance`).
pub const PREFIX_VAULT: u8 = 0x05;
/// Trie prefix for native cross-shard transaction state (`txn-id -> TxnRecord`).
pub const PREFIX_TXN: u8 = 0x06;
/// Trie prefix for the native HTTP-result oracle (`url -> recorded value`); RCHIP #54.
pub const PREFIX_HTTP: u8 = 0x07;
/// Trie prefix for the vault **handle** map (`minted-name -> base58 rev-address`) — the record that
/// lets a name handed out by `findOrCreate` still resolve to its vault in a later block, and after a
/// replay. A handle is a rholang `GPrivate` name, and it is a *lookup* key only: holding the name a
/// vault was opened under is not by itself the right to spend from it — that is [`PREFIX_VAULT_AUTH`],
/// which is the distinction that keeps `findOrCreate(someone_else's_address)` from being an authority.
pub const PREFIX_VAULT_NAME: u8 = 0x08;
/// Trie prefix for the vault **authority** map (`unforgeable-name -> base58 rev-address`).
///
/// Separate from [`PREFIX_VAULT_NAME`] because the two are different questions: a *handle* says which
/// vault a name opens, and an *authority* says who may spend it. Keeping them in one map would make
/// `findOrCreate(victim_address)` an authority over the victim's vault — the handle alone would be
/// enough to spend, which is exactly the hole this level of the design exists to close. An authority
/// is recorded only by `unforgeableAuthKey`, whose argument the caller must already hold.
pub const PREFIX_VAULT_AUTH: u8 = 0x09;

/// A native-state mutation, folded into the trie at checkpoint (port of a `NativeStoreAction`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeStoreAction {
    Put {
        prefix: u8,
        key: Blake2b256Hash,
        value: Vec<u8>,
    },
    Delete {
        prefix: u8,
        key: Blake2b256Hash,
    },
}

impl NativeStoreAction {
    /// The **one** trie slot this action addresses, as `(prefix, key)`.
    ///
    /// This is the identity a batch may hold only once: `RadixHistory::process` refuses a batch with
    /// two actions on one key, and native actions share the trie's key space under their own
    /// prefixes (`PREFIX_REGISTRY`..`PREFIX_VAULT_AUTH`), so the check applies to them exactly as it
    /// does to tuple-space actions.
    ///
    /// It exists as a method because the *producer* of a batch has to dedupe by it and the reason is
    /// not obvious from either variant: a `Put` and a `Delete` on one slot collide just as two
    /// `Put`s do, and "one action per slot" is what a merge needs to restore when it concatenates
    /// the native effects of several blocks (issue #83).
    pub fn slot(&self) -> (u8, Blake2b256Hash) {
        match self {
            NativeStoreAction::Put { prefix, key, .. } => (*prefix, *key),
            NativeStoreAction::Delete { prefix, key } => (*prefix, *key),
        }
    }
}

/// A keyed read of native state from a history root (implemented by the history reader).
#[async_trait]
pub trait NativeHistoryReader: Send + Sync {
    async fn get_native(&self, prefix: u8, key: Blake2b256Hash) -> Result<Option<Vec<u8>>, String>;
}

/// A no-op native reader (returns `None` for every key) — used as the initial reader before any
/// checkpoint/reset has established a history root.
struct NoopNativeReader;

#[async_trait]
impl NativeHistoryReader for NoopNativeReader {
    async fn get_native(
        &self,
        _prefix: u8,
        _key: Blake2b256Hash,
    ) -> Result<Option<Vec<u8>>, String> {
        Ok(None)
    }
}

/// Snapshot of the native-store overlay (for soft-checkpoint revert). `Some(v)` is a written value,
/// `None` is a tombstone (deletion).
#[derive(Clone, Default, Debug)]
pub struct NativeStoreState {
    overlay: BTreeMap<(u8, Blake2b256Hash), Option<Vec<u8>>>,
}

/// The in-memory native store: a write-through overlay over a [`NativeHistoryReader`].
pub struct InMemNativeStore {
    /// Written keys and tombstones since the last `drain_changes` / `reset`. `Some(v)` = value,
    /// `None` = deleted.
    overlay: Mutex<BTreeMap<(u8, Blake2b256Hash), Option<Vec<u8>>>>,
    /// The base reader for keys not present in the overlay (updated on checkpoint/reset).
    reader: RwLock<Arc<dyn NativeHistoryReader>>,
    /// Set once a non-noop reader is installed, so [`InMemNativeStore::live_entries`] can report that
    /// it is no longer looking at the whole state.
    has_history: AtomicBool,
}

impl InMemNativeStore {
    pub fn new(reader: Arc<dyn NativeHistoryReader>) -> Self {
        InMemNativeStore {
            overlay: Mutex::new(BTreeMap::new()),
            reader: RwLock::new(reader),
            has_history: AtomicBool::new(false),
        }
    }

    /// A store with no backing reader (reads return `None` until a reader is set).
    pub fn empty() -> Self {
        Self::new(Arc::new(NoopNativeReader))
    }

    /// Read a native value, consulting the overlay first and falling through to the persisted trie.
    pub async fn get(&self, prefix: u8, key: &Blake2b256Hash) -> Result<Option<Vec<u8>>, String> {
        {
            let overlay = self.overlay.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(v) = overlay.get(&(prefix, *key)) {
                return Ok(v.clone());
            }
        }
        let reader = self
            .reader
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        reader.get_native(prefix, *key).await
    }

    /// Write a native value into the overlay (and record a `Put` action).
    pub fn put(&self, prefix: u8, key: Blake2b256Hash, value: Vec<u8>) {
        self.overlay
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert((prefix, key), Some(value));
    }

    /// Delete a native value (record a `Delete` action via a tombstone).
    pub fn delete(&self, prefix: u8, key: &Blake2b256Hash) {
        self.overlay
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert((prefix, *key), None);
    }

    /// Drain the pending mutations, clearing the overlay (the caller folds the actions into a
    /// checkpoint).
    pub fn drain_changes(&self) -> Vec<NativeStoreAction> {
        let mut overlay = self.overlay.lock().unwrap_or_else(|p| p.into_inner());
        let actions = overlay
            .iter()
            .map(|(&(prefix, key), value)| match value {
                Some(v) => NativeStoreAction::Put {
                    prefix,
                    key,
                    value: v.clone(),
                },
                None => NativeStoreAction::Delete { prefix, key },
            })
            .collect();
        overlay.clear();
        actions
    }

    /// Capture the current overlay for a soft-checkpoint rollback.
    pub fn snapshot(&self) -> NativeStoreState {
        NativeStoreState {
            overlay: self
                .overlay
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone(),
        }
    }

    /// Restore a previously captured overlay (soft-checkpoint rollback).
    pub fn revert(&self, state: NativeStoreState) {
        *self.overlay.lock().unwrap_or_else(|p| p.into_inner()) = state.overlay;
    }

    /// Point the store at a new history root (called on checkpoint/reset).
    pub fn set_reader(&self, reader: Arc<dyn NativeHistoryReader>) {
        *self.reader.write().unwrap_or_else(|p| p.into_inner()) = reader;
        self.has_history.store(true, Ordering::SeqCst);
    }

    /// Whether a backing history reader has been installed (AUDIT C109's invariant).
    ///
    /// [`Self::live_entries`] can enumerate the **overlay** and nothing else: `NativeHistoryReader`
    /// exposes `get_native` and no iteration, so a store with a history root holds values this cannot
    /// see. That is not a detail to paper over — a total that silently omits part of its subject is
    /// the failure this repository keeps finding — so [`Self::live_entries`]' only caller refuses to
    /// run when this is true. A store that cannot be enumerated is reported as such rather than
    /// summed as though it had been.
    pub fn has_base_history(&self) -> bool {
        self.has_history.load(Ordering::SeqCst)
    }

    /// Every live `(key, value)` in the overlay under `prefix`, tombstones excluded.
    ///
    /// The overlay is the whole state for a store that never took a checkpoint
    /// ([`Self::empty`]), which is the store the conservation tests build. See
    /// [`Self::has_base_history`] for what this cannot see and why its caller must check.
    pub fn live_entries(&self, prefix: u8) -> Vec<(Blake2b256Hash, Vec<u8>)> {
        self.overlay
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .filter_map(|(&(p, key), value)| match value {
                Some(v) if p == prefix => Some((key, v.clone())),
                _ => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn put_then_get_round_trips_without_reader() {
        let store = InMemNativeStore::empty();
        let key = Blake2b256Hash::from_bytes([7u8; 32]);
        store.put(PREFIX_POS, key, vec![1, 2, 3]);
        assert_eq!(
            store.get(PREFIX_POS, &key).await.unwrap(),
            Some(vec![1, 2, 3])
        );
    }

    /// `slot` is the identity a batch may hold once, so it must report the same thing for a `Put`
    /// and a `Delete` on one key — a mix the trie refuses exactly as it refuses two `Put`s, and the
    /// shape a merge has to collapse (issue #83). Both axes are checked, because a `slot` that
    /// ignored the prefix would merge two different stores' keys.
    #[test]
    fn a_slot_is_the_prefix_and_key_whatever_the_variant() {
        let key = Blake2b256Hash::from_bytes([7u8; 32]);
        let put = NativeStoreAction::Put {
            prefix: PREFIX_POS,
            key,
            value: vec![1],
        };
        let delete = NativeStoreAction::Delete {
            prefix: PREFIX_POS,
            key,
        };
        assert_eq!(put.slot(), (PREFIX_POS, key));
        assert_eq!(delete.slot(), (PREFIX_POS, key));
        assert_eq!(
            put.slot(),
            delete.slot(),
            "a write and a delete on one key are one slot, not two"
        );
        let other_prefix = NativeStoreAction::Delete {
            prefix: PREFIX_VAULT,
            key,
        };
        assert_ne!(
            put.slot(),
            other_prefix.slot(),
            "the prefix is part of the identity: the same key under two prefixes is two slots"
        );
    }

    #[tokio::test]
    async fn drain_changes_produces_put_actions() {
        let store = InMemNativeStore::empty();
        let key = Blake2b256Hash::from_bytes([8u8; 32]);
        store.put(PREFIX_REGISTRY, key, vec![9]);
        let changes = store.drain_changes();
        assert_eq!(
            changes,
            vec![NativeStoreAction::Put {
                prefix: PREFIX_REGISTRY,
                key,
                value: vec![9],
            }]
        );
        // Overlay is cleared; a read falls through to the (no-op) reader.
        assert_eq!(store.get(PREFIX_REGISTRY, &key).await.unwrap(), None);
    }

    #[tokio::test]
    async fn delete_records_tombstone_and_blocks_reader() {
        let store = InMemNativeStore::empty();
        let key = Blake2b256Hash::from_bytes([9u8; 32]);
        store.put(PREFIX_VAULT, key, vec![1]);
        store.delete(PREFIX_VAULT, &key);
        assert_eq!(store.get(PREFIX_VAULT, &key).await.unwrap(), None);
        let changes = store.drain_changes();
        assert_eq!(
            changes,
            vec![NativeStoreAction::Delete {
                prefix: PREFIX_VAULT,
                key
            }]
        );
    }

    #[tokio::test]
    async fn snapshot_and_revert_restore_overlay() {
        let store = InMemNativeStore::empty();
        let key = Blake2b256Hash::from_bytes([10u8; 32]);
        store.put(PREFIX_POS, key, vec![1]);
        let snap = store.snapshot();
        store.put(PREFIX_POS, key, vec![2]);
        assert_eq!(store.get(PREFIX_POS, &key).await.unwrap(), Some(vec![2]));
        store.revert(snap);
        assert_eq!(store.get(PREFIX_POS, &key).await.unwrap(), Some(vec![1]));
    }
}
