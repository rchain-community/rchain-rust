//! Content-addressed radix trie (Law 10: Merkle determinism).
//!
//! Mirrors `rspace/src/main/scala/coop/rchain/rspace/history/RadixTree.scala`.

use std::collections::HashMap;
use std::sync::Arc;

use async_recursion::async_recursion;
use rchain_crypto::hash::blake2b256_hash::Blake2b256Hash;
use rchain_shared::typed_store::KeyValueTypedStore;

use crate::history::history_action::HistoryAction;
use crate::history::key_segment::KeySegment;

/// The number of child slots in a node (one per byte value).
pub const NUM_ITEMS: usize = 256;

/// A node slot (port of the sealed `Item` hierarchy).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    Empty,
    Leaf {
        prefix: KeySegment,
        value: Blake2b256Hash,
    },
    NodePtr {
        prefix: KeySegment,
        ptr: Blake2b256Hash,
    },
}

/// A node is a fixed 256-slot array of items (port of `RadixTree.Node`). The "exactly 256 slots"
/// invariant is carried structurally by the array type, so a short/corrupt node cannot be produced
/// by [`decode`] (which is total on the validated [`SerializedNode`] refinement).
pub type Node = [Item; NUM_ITEMS];

/// An empty node (port of `RadixTree.emptyNode`).
pub fn empty_node() -> Node {
    std::array::from_fn(|_| Item::Empty)
}

/// The hash of the empty node, i.e. the empty root (port of `RadixTree.emptyRootHash`).
pub fn empty_root_hash() -> Blake2b256Hash {
    hash_node(&empty_node()).0
}

/// Serialize a node to bytes (port of `RadixTree.Codecs.encode`).
pub fn encode(node: &Node) -> Vec<u8> {
    let mut size = 0;
    for item in node {
        match item {
            Item::Empty => {}
            Item::Leaf { prefix, .. } | Item::NodePtr { prefix, .. } => {
                size += 2 + prefix.len() + 32;
            }
        }
    }
    let mut out = Vec::with_capacity(size);
    for (idx, item) in node.iter().enumerate() {
        match item {
            Item::Empty => {}
            Item::Leaf { prefix, value } => {
                out.push(idx as u8);
                out.push((prefix.len() & 0x7F) as u8);
                out.extend_from_slice(prefix.as_bytes());
                out.extend_from_slice(value.as_bytes());
            }
            Item::NodePtr { prefix, ptr } => {
                out.push(idx as u8);
                out.push((0x80 | (prefix.len() & 0x7F)) as u8);
                out.extend_from_slice(prefix.as_bytes());
                out.extend_from_slice(ptr.as_bytes());
            }
        }
    }
    out
}

/// A well-formed serialized radix node. `TryFrom<&[u8]>` validates the record framing (bounds,
/// prefix sizes, 32-byte values, and no duplicate item index); [`decode`] is then *total* on this
/// refinement — it cannot observe malformed bytes.
pub struct SerializedNode<'a>(&'a [u8]);

impl<'a> TryFrom<&'a [u8]> for SerializedNode<'a> {
    type Error = String;

    fn try_from(bytes: &'a [u8]) -> Result<Self, String> {
        let mut pos = 0;
        let mut seen = [false; 256];
        while pos < bytes.len() {
            if pos + 2 > bytes.len() {
                return Err("truncated radix node header".to_string());
            }
            let idx = bytes[pos] as usize;
            if seen[idx] {
                return Err(format!("duplicate item index {idx}"));
            }
            seen[idx] = true;
            let prefix_size = (bytes[pos + 1] & 0x7F) as usize;
            let prefix_start = pos + 2;
            if prefix_start + prefix_size > bytes.len() {
                return Err("truncated radix node prefix".to_string());
            }
            let val_start = prefix_start + prefix_size;
            if val_start + 32 > bytes.len() {
                return Err("truncated radix node value".to_string());
            }
            pos = val_start + 32;
        }
        Ok(SerializedNode(bytes))
    }
}

/// Deserialize a node from validated bytes (total; port of `RadixTree.Codecs.decode`).
pub fn decode(node: &SerializedNode) -> Node {
    let bytes = node.0;
    let mut result = empty_node();
    let mut pos = 0;
    while pos < bytes.len() {
        let idx = bytes[pos] as usize;
        let second = bytes[pos + 1];
        let prefix_size = (second & 0x7F) as usize;
        let prefix_start = pos + 2;
        // Through the checked constructor with a *proof* rather than a caller's promise: `prefix_size`
        // comes from the same 7-bit mask the encoder writes with, so it is ≤ 127 by the encoding
        // itself and this `expect` is unreachable. That keeps `decode` total by design
        // (`RadixTree.Codecs.decode`) where an error channel would have to invent a refusal for an
        // unconstructible case — C61 measured all three decode sites this way, and the promise is now
        // local to the site instead of held by `new`'s caller (C78's class).
        let prefix = KeySegment::try_from(bytes[prefix_start..prefix_start + prefix_size].to_vec())
            .expect("a 7-bit size field is at most 127");
        let val_start = prefix_start + prefix_size;
        let val = Blake2b256Hash::from_byte_array(&bytes[val_start..val_start + 32]);
        let item = if (second & 0x80) == 0 {
            Item::Leaf { prefix, value: val }
        } else {
            Item::NodePtr { prefix, ptr: val }
        };
        result[idx] = item;
        pos = val_start + 32;
    }
    result
}

/// Hash a serialized node (port of `RadixTree.hashNode`).
pub fn hash_node(node: &Node) -> (Blake2b256Hash, Vec<u8>) {
    let bytes = encode(node);
    (Blake2b256Hash::create(&bytes), bytes)
}

/// The store-backed radix tree (port of `RadixTree.RadixTreeImpl`).
pub struct RadixTreeImpl {
    store: Arc<dyn KeyValueTypedStore<Blake2b256Hash, Vec<u8>>>,
    cache_read: std::sync::Mutex<HashMap<Blake2b256Hash, Node>>,
    cache_write: std::sync::Mutex<HashMap<Blake2b256Hash, Vec<u8>>>,
}

impl RadixTreeImpl {
    pub fn new(store: Arc<dyn KeyValueTypedStore<Blake2b256Hash, Vec<u8>>>) -> Self {
        RadixTreeImpl {
            store,
            cache_read: std::sync::Mutex::new(HashMap::new()),
            cache_write: std::sync::Mutex::new(HashMap::new()),
        }
    }

    async fn load_node_from_store(&self, node_ptr: Blake2b256Hash) -> Result<Option<Node>, String> {
        // A store error is **kept separate from a missing node**. The comment that stood here —
        // "`BytesCodec` cannot fail to decode, so the store error is unreachable" — is true of the
        // *codec* and false of the store: an LMDB read can fail. `.ok()` flattened that into `None`,
        // so `load_node` reported an I/O failure as "Missing node in database", or — under the
        // `no_assert = true` its two root-load callers pass (`RadixHistory::new`/`reset`, and the
        // Scala's `loadNode(root, noAssert = true)` at `RadixHistory.scala:27,49`) — as an empty node.
        // The Scala keeps the error in `F` (`store.get1(nodePtr).map(...)`, `RadixTree.scala:569`), so
        // propagating it here is the faithful reading.
        //
        // **And the flatten above it is closed too** (2026-09-24, C53's own unit): `load_node` returns
        // `Result`, so this error is *propagated* rather than turned into an empty node under
        // `no_assert`. The error channel is the port's counterpart of the Scala's `F`: `RadixTreeImpl`
        // carries `Result<Node, String>` the way `RadixTree.scala` carries `F[Node]`, and the trait
        // methods that reach it (`History::read`/`reset`, and the `ISpace::reset` impls that already
        // returned `Result<(), String>`) carry it up rather than dropping it.
        let bytes = self
            .store
            .get(&[node_ptr])
            .await
            .map_err(|e| format!("store error reading node {}: {e}", node_ptr.to_hex()))?
            .into_iter()
            .next()
            .flatten();
        match bytes {
            None => Ok(None),
            Some(bytes) => {
                let serialized = SerializedNode::try_from(bytes.as_slice())
                    .map_err(|e| format!("corrupt node {}: {e}", node_ptr.to_hex()))?;
                Ok(Some(decode(&serialized)))
            }
        }
    }

    /// Load a node, using the read cache and falling back to the store (port of `loadNode`).
    ///
    /// The `Err` arm carries the oracle's error channel: `RadixTree.scala`'s `loadNode` is
    /// `F[Node]` and lets the store's failure through `F`, asserting only on an *absent* node
    /// (`:586-598`). This used to be a bare `Node`, so a store error became `empty_node()` under
    /// `no_assert` — the empty trie standing in for a root that could not be read (AUDIT C53).
    /// `no_assert` therefore still means "a *missing* node is expected here" (the two root loads
    /// pass it, `RadixHistory::new`/`reset`), and it cannot mean "an unreadable node is fine".
    pub async fn load_node(
        &self,
        node_ptr: Blake2b256Hash,
        no_assert: bool,
    ) -> Result<Node, String> {
        if let Some(node) = crate::lock::mlock(&self.cache_read).get(&node_ptr).cloned() {
            return Ok(node);
        }
        match self.load_node_from_store(node_ptr).await {
            Ok(Some(node)) => {
                crate::lock::mlock(&self.cache_read).insert(node_ptr, node.clone());
                Ok(node)
            }
            Ok(None) => {
                assert!(
                    no_assert,
                    "Missing node in database. ptr={}",
                    node_ptr.to_hex()
                );
                Ok(empty_node())
            }
            // Not a default: the store could not be read, and a tree whose root silently became the
            // empty node would answer "nothing" for every key under it.
            Err(e) => Err(e),
        }
    }

    pub fn clear_read_cache(&self) {
        crate::lock::mlock(&self.cache_read).clear();
    }

    /// Serialize + hash a node, caching the bytes for a later commit (port of `saveNode`).
    pub fn save_node(&self, node: &Node) -> Blake2b256Hash {
        let (hash, bytes) = hash_node(node);
        let mut cache_read = crate::lock::mlock(&self.cache_read);
        if let Some(existing) = cache_read.get(&hash) {
            assert!(
                existing == node,
                "Collision in cache: record with key = {} has already existed.",
                hash.to_hex()
            );
        } else {
            cache_read.insert(hash, node.clone());
        }
        drop(cache_read);
        crate::lock::mlock(&self.cache_write).insert(hash, bytes);
        hash
    }

    /// Write the write-cache to the store, checking for collisions (port of `commit`).
    pub async fn commit(&self) -> Result<(), String> {
        let kv_pairs: Vec<(Blake2b256Hash, Vec<u8>)> = crate::lock::mlock(&self.cache_write)
            .iter()
            .map(|(k, v)| (*k, v.clone()))
            .collect();
        let keys: Vec<Blake2b256Hash> = kv_pairs.iter().map(|(k, _)| *k).collect();
        let present = self.store.contains(&keys).await?;
        let absent: Vec<(Blake2b256Hash, Vec<u8>)> = kv_pairs
            .iter()
            .zip(present.iter())
            .filter(|(_, &p)| !p)
            .map(|((k, v), _)| (*k, v.clone()))
            .collect();
        let existing_keys: Vec<Blake2b256Hash> = kv_pairs
            .iter()
            .zip(present.iter())
            .filter(|(_, &p)| p)
            .map(|((k, _), _)| *k)
            .collect();
        let existing_values = self.store.get(&existing_keys).await?;
        let cache_map: HashMap<Blake2b256Hash, Vec<u8>> = kv_pairs.iter().cloned().collect();
        for (k, existing) in existing_keys.iter().zip(existing_values.iter()) {
            let cached = cache_map
                .get(k)
                .ok_or_else(|| "cached key must be present".to_string())?;
            let stored = existing.clone().unwrap_or_default();
            if cached != &stored {
                return Err(format!("collision in KVDB (key = {})", k.to_hex()));
            }
        }
        self.store.put(&absent).await?;
        Ok(())
    }

    pub fn clear_write_cache(&self) {
        crate::lock::mlock(&self.cache_write).clear();
    }

    /// Read the leaf value at `start_prefix` under `start_node` (port of `read`).
    ///
    /// `F[Option[Blake2b256Hash]]` in the oracle (`RadixTree.scala`), so an unreadable node is an
    /// error here rather than "no value at this key" (AUDIT C53).
    pub async fn read(
        &self,
        start_node: &Node,
        start_prefix: &KeySegment,
    ) -> Result<Option<Blake2b256Hash>, String> {
        let mut node = start_node.clone();
        let mut prefix = start_prefix.clone();
        loop {
            if prefix.is_empty() {
                return Ok(None);
            }
            match &node[prefix.head() as usize] {
                Item::Empty => return Ok(None),
                Item::Leaf {
                    prefix: leaf_prefix,
                    value,
                } => {
                    return Ok(if *leaf_prefix == prefix.tail() {
                        Some(*value)
                    } else {
                        None
                    });
                }
                Item::NodePtr {
                    prefix: ptr_prefix,
                    ptr,
                } => {
                    let (_, prefix_rest, ptr_prefix_rest) =
                        KeySegment::common_prefix(&prefix.tail(), ptr_prefix);
                    if ptr_prefix_rest.is_empty() {
                        node = self.load_node(*ptr, false).await?;
                        prefix = prefix_rest;
                    } else {
                        return Ok(None);
                    }
                }
            }
        }
    }

    /// Build a one-item node from a non-empty `Item` (port of `createNodeFromItem`).
    fn create_node_from_item(item: &Item) -> Node {
        match item {
            Item::Empty => empty_node(),
            Item::Leaf { prefix, value } => {
                assert!(!prefix.is_empty(), "LeafPrefix should be non empty.");
                let mut node = empty_node();
                node[prefix.head() as usize] = Item::Leaf {
                    prefix: prefix.tail(),
                    value: *value,
                };
                node
            }
            Item::NodePtr { prefix, ptr } => {
                assert!(!prefix.is_empty(), "NodePtrPrefix should be non empty.");
                let mut node = empty_node();
                node[prefix.head() as usize] = Item::NodePtr {
                    prefix: prefix.tail(),
                    ptr: *ptr,
                };
                node
            }
        }
    }

    /// Optimize and save a node, creating the item that points to it (port of
    /// `saveNodeAndCreateItem`).
    fn save_node_and_create_item(
        &self,
        node: &Node,
        prefix: &KeySegment,
        compaction: bool,
    ) -> Result<Item, String> {
        if compaction {
            let mut non_empty: Vec<(usize, &Item)> = Vec::new();
            for (idx, item) in node.iter().enumerate() {
                if item != &Item::Empty {
                    non_empty.push((idx, item));
                    if non_empty.len() == 2 {
                        break;
                    }
                }
            }
            Ok(match non_empty.len() {
                0 => Item::Empty,
                1 => {
                    let (idx, item) = non_empty[0];
                    let idx_seg = KeySegment::try_from(vec![idx as u8])
                        .expect("a single byte is at most 127 bytes");
                    match item {
                        Item::Empty => Item::Empty,
                        Item::Leaf {
                            prefix: leaf_prefix,
                            value,
                        } => Item::Leaf {
                            prefix: prefix.concat(&idx_seg)?.concat(leaf_prefix)?,
                            value: *value,
                        },
                        Item::NodePtr {
                            prefix: ptr_prefix,
                            ptr,
                        } => Item::NodePtr {
                            prefix: prefix.concat(&idx_seg)?.concat(ptr_prefix)?,
                            ptr: *ptr,
                        },
                    }
                }
                _ => Item::NodePtr {
                    prefix: prefix.clone(),
                    ptr: self.save_node(node),
                },
            })
        } else {
            Ok(Item::NodePtr {
                prefix: prefix.clone(),
                ptr: self.save_node(node),
            })
        }
    }

    /// Construct a node from an item, loading the child for an empty-prefix `NodePtr` (port of
    /// `constructNodeFromItem`).
    async fn construct_node_from_item(&self, item: &Item) -> Result<Node, String> {
        match item {
            Item::NodePtr { prefix, ptr } if prefix.is_empty() => self.load_node(*ptr, false).await,
            _ => Ok(Self::create_node_from_item(item)),
        }
    }

    /// Insert a leaf into the subtree rooted at `cur_item` (port of `update`).
    #[async_recursion]
    async fn update(
        &self,
        cur_item: Item,
        ins_prefix: KeySegment,
        ins_value: Blake2b256Hash,
    ) -> Result<Option<Item>, String> {
        match cur_item {
            Item::Empty => Ok(Some(Item::Leaf {
                prefix: ins_prefix,
                value: ins_value,
            })),
            Item::Leaf {
                prefix: leaf_prefix,
                value: leaf_value,
            } => {
                assert_eq!(
                    leaf_prefix.len(),
                    ins_prefix.len(),
                    "The length of all prefixes in the subtree must be the same."
                );
                if leaf_prefix == ins_prefix {
                    if ins_value == leaf_value {
                        Ok(None)
                    } else {
                        Ok(Some(Item::Leaf {
                            prefix: ins_prefix,
                            value: ins_value,
                        }))
                    }
                } else {
                    let (comm_prefix, ins_prefix_rest, leaf_prefix_rest) =
                        KeySegment::common_prefix(&ins_prefix, &leaf_prefix);
                    let mut new_node = empty_node();
                    new_node[leaf_prefix_rest.head() as usize] = Item::Leaf {
                        prefix: leaf_prefix_rest.tail(),
                        value: leaf_value,
                    };
                    new_node[ins_prefix_rest.head() as usize] = Item::Leaf {
                        prefix: ins_prefix_rest.tail(),
                        value: ins_value,
                    };
                    Ok(Some(self.save_node_and_create_item(
                        &new_node,
                        &comm_prefix,
                        false,
                    )?))
                }
            }
            Item::NodePtr {
                prefix: ptr_prefix,
                ptr,
            } => {
                assert!(
                    ptr_prefix.len() < ins_prefix.len(),
                    "Radix key should be longer than NodePtr key."
                );
                let (comm_prefix, ins_prefix_rest, ptr_prefix_rest) =
                    KeySegment::common_prefix(&ins_prefix, &ptr_prefix);
                if ptr_prefix_rest.is_empty() {
                    let child_node = self.load_node(ptr, false).await?;
                    let child_item_idx = ins_prefix_rest.head() as usize;
                    let child_ins_prefix = ins_prefix_rest.tail();
                    let child_item = child_node[child_item_idx].clone();
                    let child_item_opt =
                        self.update(child_item, child_ins_prefix, ins_value).await?;
                    match child_item_opt {
                        None => Ok(None),
                        Some(new_child_item) => {
                            let mut updated_child_node = child_node.clone();
                            updated_child_node[child_item_idx] = new_child_item;
                            Ok(Some(self.save_node_and_create_item(
                                &updated_child_node,
                                &comm_prefix,
                                false,
                            )?))
                        }
                    }
                } else {
                    let mut new_node = empty_node();
                    new_node[ptr_prefix_rest.head() as usize] = Item::NodePtr {
                        prefix: ptr_prefix_rest.tail(),
                        ptr,
                    };
                    new_node[ins_prefix_rest.head() as usize] = Item::Leaf {
                        prefix: ins_prefix_rest.tail(),
                        value: ins_value,
                    };
                    Ok(Some(self.save_node_and_create_item(
                        &new_node,
                        &comm_prefix,
                        false,
                    )?))
                }
            }
        }
    }

    /// Delete a leaf from the subtree rooted at `cur_item` (port of `delete`).
    #[async_recursion]
    async fn delete(&self, cur_item: Item, del_prefix: KeySegment) -> Result<Option<Item>, String> {
        match cur_item {
            Item::Empty => Ok(None),
            Item::Leaf {
                prefix: leaf_prefix,
                ..
            } => {
                if leaf_prefix == del_prefix {
                    Ok(Some(Item::Empty))
                } else {
                    Ok(None)
                }
            }
            Item::NodePtr {
                prefix: ptr_prefix,
                ptr,
            } => {
                let (comm_prefix, del_prefix_rest, ptr_prefix_rest) =
                    KeySegment::common_prefix(&del_prefix, &ptr_prefix);
                if !ptr_prefix_rest.is_empty() || del_prefix_rest.is_empty() {
                    Ok(None)
                } else {
                    let child_node = self.load_node(ptr, false).await?;
                    let del_item_idx = del_prefix_rest.head() as usize;
                    let del_item_prefix = del_prefix_rest.tail();
                    let child_item = child_node[del_item_idx].clone();
                    let child_item_opt = self.delete(child_item, del_item_prefix).await?;
                    match child_item_opt {
                        None => Ok(None),
                        Some(new_child_item) => {
                            let mut new_child_node = child_node.clone();
                            new_child_node[del_item_idx] = new_child_item;
                            Ok(Some(self.save_node_and_create_item(
                                &new_child_node,
                                &comm_prefix,
                                true,
                            )?))
                        }
                    }
                }
            }
        }
    }

    /// Apply a batch of `HistoryAction`s to a subtree (port of `makeActions`).
    #[async_recursion]
    async fn make_actions(
        &self,
        cur_node: &Node,
        actions: &[HistoryAction],
    ) -> Result<Option<Node>, String> {
        // Group actions by the first byte of their key.
        let mut grouped: Vec<(u8, Vec<HistoryAction>)> = Vec::new();
        for action in actions {
            let first = action.key().head();
            match grouped.iter_mut().find(|(b, _)| *b == first) {
                Some((_, list)) => list.push(action.clone()),
                None => grouped.push((first, vec![action.clone()])),
            }
        }

        let mut new_group_items: Vec<(usize, Option<Item>)> = Vec::new();
        for (group_idx, actions_in_group) in grouped {
            let item_idx = group_idx as usize;
            let item = cur_node[item_idx].clone();
            let result = if actions_in_group.len() == 1 {
                let action = &actions_in_group[0];
                let new_item = match action {
                    HistoryAction::Insert { key, hash } => {
                        self.update(item, key.tail(), *hash).await?
                    }
                    HistoryAction::Delete { key } => self.delete(item, key.tail()).await?,
                };
                (item_idx, new_item)
            } else {
                let has_insert = actions_in_group
                    .iter()
                    .any(|a| matches!(a, HistoryAction::Insert { .. }));
                let cleared = if item == Item::Empty && !has_insert {
                    Vec::new()
                } else {
                    actions_in_group.clone()
                };
                if cleared.is_empty() {
                    (item_idx, None)
                } else {
                    let created_node = self.construct_node_from_item(&item).await?;
                    let new_actions: Vec<HistoryAction> =
                        cleared.iter().map(|a| a.trim()).collect();
                    let new_node_opt = self.make_actions(&created_node, &new_actions).await?;
                    let new_item = match new_node_opt {
                        None => None,
                        Some(n) => Some(
                            self.save_node_and_create_item(&n, &KeySegment::empty(), true)?,
                        ),
                    };
                    (item_idx, new_item)
                }
            };
            new_group_items.push(result);
        }

        let mut new_cur_node = cur_node.clone();
        for (idx, new_item_opt) in new_group_items {
            if let Some(item) = new_item_opt {
                new_cur_node[idx] = item;
            }
        }
        Ok(if new_cur_node != *cur_node {
            Some(new_cur_node)
        } else {
            None
        })
    }

    /// Apply actions to a subtree, persist, and return the new root (port of `saveAndCommit`).
    pub async fn save_and_commit(
        &self,
        root_node: &Node,
        actions: &[HistoryAction],
    ) -> Result<Option<(Node, Blake2b256Hash)>, String> {
        let result = match self.make_actions(root_node, actions).await? {
            Some(new_root_node) => {
                let new_root_hash = self.save_node(&new_root_node);
                self.commit().await?;
                Some((new_root_node, new_root_hash))
            }
            None => None,
        };
        self.clear_write_cache();
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_root_is_content_addressed_and_stable() {
        let root = empty_root_hash();
        assert_eq!(root, empty_root_hash());
        // hash of the empty node's canonical encoding
        assert_eq!(root, Blake2b256Hash::create(&encode(&empty_node())));
    }

    #[test]
    fn node_encoding_round_trips() {
        let mut node = empty_node();
        node[1] = Item::Leaf {
            prefix: KeySegment::try_from(vec![0xff]).expect("1 byte is at most 127"),
            value: Blake2b256Hash::from_bytes([0x11; 32]),
        };
        node[2] = Item::NodePtr {
            prefix: KeySegment::empty(),
            ptr: Blake2b256Hash::from_bytes([0x22; 32]),
        };
        let decoded = decode(&SerializedNode::try_from(encode(&node).as_slice()).unwrap());
        assert_eq!(decoded, node);
        // node hash is deterministic
        assert_eq!(hash_node(&node).0, hash_node(&decoded).0);
    }

    #[test]
    fn empty_node_encoding_is_empty() {
        assert!(encode(&empty_node()).is_empty());
        assert_eq!(
            decode(&SerializedNode::try_from(&[][..]).unwrap()),
            empty_node()
        );
    }

    fn in_memory_tree() -> RadixTreeImpl {
        use rchain_shared::store::{InMemoryKeyValueStore, KeyValueStore};
        use rchain_shared::typed_store::{BytesCodec, KeyValueTypedStoreCodec};

        let shared: rchain_shared::typed_store::SharedStore = Arc::new(tokio::sync::Mutex::new(
            Box::new(InMemoryKeyValueStore::default()) as Box<dyn KeyValueStore + Send + Sync>,
        ));
        let typed = Arc::new(KeyValueTypedStoreCodec::new(
            shared,
            Arc::new(crate::history::codecs::Blake2b256HashCodec),
            Arc::new(BytesCodec),
        ));
        RadixTreeImpl::new(typed)
    }

    /// A store whose every operation fails, for the case a real LMDB read fails — which
    /// `load_node_from_store` used to flatten into "no bytes" with `.ok()`.
    struct FailingStore;

    #[async_trait::async_trait]
    impl rchain_shared::typed_store::KeyValueTypedStore<Blake2b256Hash, Vec<u8>> for FailingStore {
        async fn get(&self, _keys: &[Blake2b256Hash]) -> Result<Vec<Option<Vec<u8>>>, String> {
            Err("the store is down".to_string())
        }
        async fn put(&self, _pairs: &[(Blake2b256Hash, Vec<u8>)]) -> Result<(), String> {
            Err("the store is down".to_string())
        }
        async fn delete(&self, _keys: &[Blake2b256Hash]) -> Result<usize, String> {
            Err("the store is down".to_string())
        }
        async fn contains(&self, _keys: &[Blake2b256Hash]) -> Result<Vec<bool>, String> {
            Err("the store is down".to_string())
        }
        async fn to_map(
            &self,
        ) -> Result<std::collections::BTreeMap<Blake2b256Hash, Vec<u8>>, String> {
            Err("the store is down".to_string())
        }
    }

    /// **A store error is not an empty root.** `load_node`'s signature is `Node`, so the checked
    /// sibling's `Err` arm was flattened into `empty_node()` — under `no_assert = true`, which is
    /// exactly what the two root loads pass (`RadixHistory::new`/`reset`), a history whose `root_hash`
    /// names a real root would come back with the **empty trie** as its root node: every read under it
    /// answers "nothing" and the next write rebuilds the root from nothing. The Scala keeps the error
    /// in `F` (`loadNodeFromStore: F[Option[Node]]`, `RadixTree.scala:568-598`) and lets only the
    /// *absent* case reach `assert(noAssert)`/`emptyNode`; this is the port's error channel for it
    /// (AUDIT C53).
    ///
    /// Falsifier, in its pre-fix form: with `load_node` returning a bare `Node`, a store that is down
    /// yielded `empty_node()` and the first assertion failed (run 2026-09-24: `left` and `right` were
    /// the same 256-`Empty` array). The signature is `Result` now, so the same case is an `Err` here —
    /// and the *absent*-node half is asserted beside it, because a fix that turned absence into an
    /// error would be the opposite bug.
    #[tokio::test]
    async fn a_store_error_is_not_an_empty_root() {
        let failing = RadixTreeImpl::new(Arc::new(FailingStore));
        let err = failing
            .load_node(Blake2b256Hash::from_bytes([0x11; 32]), true)
            .await
            .expect_err("a store error must never read as an empty root");
        assert!(
            err.contains("store error reading node"),
            "and it must say which failure it was, got: {err}"
        );

        let empty = in_memory_tree();
        assert_eq!(
            empty
                .load_node(Blake2b256Hash::from_bytes([0x11; 32]), true)
                .await
                .expect("an absent node under `no_assert` is the oracle's `emptyNode`"),
            empty_node(),
            "absence is still not an error — only unreadability is"
        );

        // …and the *same* failure at the place a root is actually loaded: the two callers that pass
        // `no_assert = true` (`RadixHistory::new`/`reset`) refuse to build a history rather than hand
        // back one whose root is the empty trie — which would answer "no data" for every key of a
        // state that exists, and rebuild that root from nothing on the next write.
        let unbuildable = match crate::history::instances::radix_history::RadixHistory::new(
            Blake2b256Hash::from_bytes([0x11; 32]),
            Arc::new(FailingStore),
        )
        .await
        {
            Ok(_) => panic!("a history whose root cannot be read must not be built"),
            Err(e) => e,
        };
        assert!(
            unbuildable.contains("store error reading node"),
            "and it must name the failure, got: {unbuildable}"
        );
    }

    /// **A store error is not a missing node.** Both used to be `None` here, so the caller asserted
    /// "Missing node in database" for an I/O failure — or, under `no_assert`, returned an empty node
    /// for it — and read a `PREFIX_*` leaf as absent when it could not be read at all. The two cases
    /// are asserted together, because a fix that made *missing* nodes an error would be the opposite
    /// bug and this test would catch it.
    #[tokio::test]
    async fn a_store_error_is_not_a_missing_node() {
        let failing = RadixTreeImpl::new(Arc::new(FailingStore));
        let err = failing
            .load_node_from_store(Blake2b256Hash::from_bytes([0x11; 32]))
            .await
            .expect_err("a store error must surface, not read as an absent node");
        assert!(
            err.contains("store error reading node"),
            "and it must say which failure it was, got: {err}"
        );

        let empty = in_memory_tree();
        assert_eq!(
            empty
                .load_node_from_store(Blake2b256Hash::from_bytes([0x11; 32]))
                .await
                .expect("an empty store is not an error"),
            None,
            "a genuinely absent node is still `None` — the two are different facts"
        );
    }

    #[tokio::test]
    async fn insert_read_update_delete_round_trip() {
        let tree = in_memory_tree();
        let root = empty_node();
        let key = KeySegment::try_from(vec![1, 2, 3]).expect("3 bytes is at most 127");
        let value = Blake2b256Hash::from_bytes([0x42; 32]);

        let (root1, _) = tree
            .save_and_commit(
                &root,
                &[HistoryAction::Insert {
                    key: key.clone(),
                    hash: value,
                }],
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(tree.read(&root1, &key).await.expect("read"), Some(value));

        // update
        let value2 = Blake2b256Hash::from_bytes([0x43; 32]);
        let (root2, _) = tree
            .save_and_commit(
                &root1,
                &[HistoryAction::Insert {
                    key: key.clone(),
                    hash: value2,
                }],
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(tree.read(&root2, &key).await.expect("read"), Some(value2));

        // delete returns to the empty root
        let (root3, _) = tree
            .save_and_commit(&root2, &[HistoryAction::Delete { key: key.clone() }])
            .await
            .unwrap()
            .unwrap();
        assert_eq!(tree.read(&root3, &key).await.expect("read"), None);
        assert_eq!(root3, empty_node());
    }
}
