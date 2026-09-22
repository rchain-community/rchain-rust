//! The concrete `HistoryReader` over a target history + cold store.
//!
//! Mirrors `rspace/src/main/scala/coop/rchain/rspace/history/instances/RSpaceHistoryReaderImpl.scala`.

use std::sync::Arc;

use async_trait::async_trait;
use rchain_crypto::hash::blake2b256_hash::Blake2b256Hash;
use rchain_shared::serialize::Serialize;

use crate::errors::RSpaceError;
use crate::hashing::stable_hash_provider::{hash_channel, hash_channels};
use crate::history::cold_store::{ColdKeyValueStore, PersistedData};
use crate::history::history::History;
use crate::history::history_reader::{HistoryReader, HistoryReaderBase, HistoryReaderBinary};
use crate::history::key_segment::KeySegment;
use crate::internal::{Datum, WaitingContinuation};
use crate::native_store::NativeHistoryReader;
use crate::serializers::scodec_serialize::{
    decode_continuations, decode_continuations_binary, decode_datums, decode_datums_binary,
    decode_joins, decode_joins_binary,
};

const PREFIX_DATUM: u8 = 0x00;
const PREFIX_KONT: u8 = 0x01;
const PREFIX_JOINS: u8 = 0x02;

/// The history reader implementation (port of `RSpaceHistoryReaderImpl`).
pub struct RSpaceHistoryReaderImpl<C, P, A, K> {
    target_history: Arc<dyn History>,
    leaf_store: ColdKeyValueStore,
    marker: std::marker::PhantomData<(C, P, A, K)>,
}

impl<C, P, A, K> RSpaceHistoryReaderImpl<C, P, A, K>
where
    C: Serialize<C> + Send + Sync + 'static,
    P: Serialize<P> + Send + Sync + 'static,
    A: Serialize<A> + Send + Sync + 'static,
    K: Serialize<K> + Send + Sync + 'static,
{
    pub fn new(target_history: Arc<dyn History>, leaf_store: ColdKeyValueStore) -> Self {
        RSpaceHistoryReaderImpl {
            target_history,
            leaf_store,
            marker: std::marker::PhantomData,
        }
    }

    async fn fetch_data(
        &self,
        prefix: u8,
        key: Blake2b256Hash,
    ) -> Result<Option<PersistedData>, String> {
        let mut seg = vec![prefix];
        seg.extend_from_slice(key.as_bytes());
        match self.target_history.read(&KeySegment::new(seg)).await {
            Some(leaf_hash) => {
                let data = self.leaf_store.get(&[leaf_hash]).await?;
                Ok(data.into_iter().next().flatten())
            }
            None => Ok(None),
        }
    }
}

#[async_trait]
impl<C, P, A, K> HistoryReader<C, P, A, K> for RSpaceHistoryReaderImpl<C, P, A, K>
where
    C: Serialize<C> + Send + Sync + 'static,
    P: Serialize<P> + Send + Sync + 'static,
    A: Serialize<A> + Send + Sync + 'static,
    K: Serialize<K> + Send + Sync + 'static,
{
    fn root(&self) -> Blake2b256Hash {
        self.target_history.root()
    }

    async fn get_data(&self, key: Blake2b256Hash) -> Result<Vec<Datum<A>>, RSpaceError> {
        match self
            .fetch_data(PREFIX_DATUM, key)
            .await
            .map_err(|_| RSpaceError::Codec("persisted data"))?
        {
            Some(PersistedData::DataLeaf(bytes)) => decode_datums(&bytes),
            Some(_) => Err(RSpaceError::UnexpectedLeaf("data")),
            None => Ok(Vec::new()),
        }
    }

    async fn get_continuations(
        &self,
        key: Blake2b256Hash,
    ) -> Result<Vec<WaitingContinuation<P, K>>, RSpaceError> {
        match self
            .fetch_data(PREFIX_KONT, key)
            .await
            .map_err(|_| RSpaceError::Codec("persisted data"))?
        {
            Some(PersistedData::ContinuationsLeaf(bytes)) => decode_continuations(&bytes),
            Some(_) => Err(RSpaceError::UnexpectedLeaf("continuations")),
            None => Ok(Vec::new()),
        }
    }

    async fn get_joins(&self, key: Blake2b256Hash) -> Result<Vec<Vec<C>>, RSpaceError> {
        match self
            .fetch_data(PREFIX_JOINS, key)
            .await
            .map_err(|_| RSpaceError::Codec("persisted data"))?
        {
            Some(PersistedData::JoinsLeaf(bytes)) => decode_joins(&bytes),
            Some(_) => Err(RSpaceError::UnexpectedLeaf("joins")),
            None => Ok(Vec::new()),
        }
    }

    async fn get_native(
        &self,
        prefix: u8,
        key: Blake2b256Hash,
    ) -> Result<Option<Vec<u8>>, RSpaceError> {
        match self
            .fetch_data(prefix, key)
            .await
            .map_err(|_| RSpaceError::Codec("native"))?
        {
            Some(PersistedData::NativeLeaf(bytes)) => Ok(Some(bytes)),
            Some(_) => Err(RSpaceError::UnexpectedLeaf("native")),
            None => Ok(None),
        }
    }

    fn base(&self) -> Arc<dyn HistoryReaderBase<C, P, A, K>> {
        Arc::new(BaseReader {
            reader: Arc::new(RSpaceHistoryReaderImpl {
                target_history: self.target_history.clone(),
                leaf_store: self.leaf_store.clone(),
                marker: std::marker::PhantomData,
            }),
        })
    }

    fn reader_binary(&self) -> Arc<dyn HistoryReaderBinary<C, P, A, K>> {
        Arc::new(RSpaceHistoryReaderImpl {
            target_history: self.target_history.clone(),
            leaf_store: self.leaf_store.clone(),
            marker: std::marker::PhantomData,
        })
    }
}

#[async_trait]
impl<C, P, A, K> NativeHistoryReader for RSpaceHistoryReaderImpl<C, P, A, K>
where
    C: Serialize<C> + Send + Sync + 'static,
    P: Serialize<P> + Send + Sync + 'static,
    A: Serialize<A> + Send + Sync + 'static,
    K: Serialize<K> + Send + Sync + 'static,
{
    async fn get_native(&self, prefix: u8, key: Blake2b256Hash) -> Result<Option<Vec<u8>>, String> {
        match self.fetch_data(prefix, key).await? {
            Some(PersistedData::NativeLeaf(bytes)) => Ok(Some(bytes)),
            Some(_) => Err("unexpected leaf while looking for native".to_string()),
            None => Ok(None),
        }
    }
}

#[async_trait]
impl<C, P, A, K> HistoryReaderBinary<C, P, A, K> for RSpaceHistoryReaderImpl<C, P, A, K>
where
    C: Serialize<C> + Send + Sync + 'static,
    P: Serialize<P> + Send + Sync + 'static,
    A: Serialize<A> + Send + Sync + 'static,
    K: Serialize<K> + Send + Sync + 'static,
{
    async fn get_data(
        &self,
        key: Blake2b256Hash,
    ) -> Result<Vec<crate::serializers::scodec_serialize::DatumB<A>>, RSpaceError> {
        match self
            .fetch_data(PREFIX_DATUM, key)
            .await
            .map_err(|_| RSpaceError::Codec("persisted data"))?
        {
            Some(PersistedData::DataLeaf(bytes)) => decode_datums_binary(&bytes),
            Some(_) => Err(RSpaceError::UnexpectedLeaf("data")),
            None => Ok(Vec::new()),
        }
    }

    async fn get_continuations(
        &self,
        key: Blake2b256Hash,
    ) -> Result<Vec<crate::serializers::scodec_serialize::WaitingContinuationB<P, K>>, RSpaceError>
    {
        match self
            .fetch_data(PREFIX_KONT, key)
            .await
            .map_err(|_| RSpaceError::Codec("persisted data"))?
        {
            Some(PersistedData::ContinuationsLeaf(bytes)) => decode_continuations_binary(&bytes),
            Some(_) => Err(RSpaceError::UnexpectedLeaf("continuations")),
            None => Ok(Vec::new()),
        }
    }

    async fn get_joins(
        &self,
        key: Blake2b256Hash,
    ) -> Result<Vec<crate::serializers::scodec_serialize::JoinsB<C>>, RSpaceError> {
        match self
            .fetch_data(PREFIX_JOINS, key)
            .await
            .map_err(|_| RSpaceError::Codec("persisted data"))?
        {
            Some(PersistedData::JoinsLeaf(bytes)) => decode_joins_binary(&bytes),
            Some(_) => Err(RSpaceError::UnexpectedLeaf("joins")),
            None => Ok(Vec::new()),
        }
    }
}

struct BaseReader<C, P, A, K> {
    reader: Arc<RSpaceHistoryReaderImpl<C, P, A, K>>,
}

#[async_trait]
impl<C, P, A, K> HistoryReaderBase<C, P, A, K> for BaseReader<C, P, A, K>
where
    C: Serialize<C> + Send + Sync + 'static,
    P: Serialize<P> + Send + Sync + 'static,
    A: Serialize<A> + Send + Sync + 'static,
    K: Serialize<K> + Send + Sync + 'static,
{
    async fn get_data(&self, key: &C) -> Result<Vec<Datum<A>>, RSpaceError> {
        HistoryReader::get_data(self.reader.as_ref(), hash_channel(key)).await
    }

    async fn get_continuations(
        &self,
        key: &[C],
    ) -> Result<Vec<WaitingContinuation<P, K>>, RSpaceError> {
        HistoryReader::get_continuations(self.reader.as_ref(), hash_channels(key)).await
    }

    async fn get_joins(&self, key: &C) -> Result<Vec<Vec<C>>, RSpaceError> {
        HistoryReader::get_joins(self.reader.as_ref(), hash_channel(key)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc as StdArc;

    use rchain_shared::store_manager::{database, InMemoryStoreManager};

    use crate::history::codecs::Blake2b256HashCodec;
    use crate::history::cold_store::{encode_persisted_data, PersistedData, PersistedDataCodec};
    use crate::history::history_action::HistoryAction;
    use crate::history::instances::radix_history::{empty_root, RadixHistory};
    use crate::history::radix_tree::empty_root_hash;
    use crate::serializers::scodec_serialize::{
        encode_continuations, encode_continuations_binary, encode_datums, encode_datums_binary,
        encode_joins,
    };
    use rchain_shared::typed_store::BytesCodec;

    /// A reader over a history and a cold store, built the way the node's factory builds them
    /// (`database` from a manager, with the real codec pairs) — so what these tests exercise is the
    /// reader the node runs, not a stand-in.
    struct Rig {
        history: Arc<dyn History>,
        cold: ColdKeyValueStore,
    }

    impl Rig {
        async fn new() -> Self {
            let manager = InMemoryStoreManager::default();
            let history_store = database(
                &manager,
                "history",
                StdArc::new(Blake2b256HashCodec),
                StdArc::new(BytesCodec),
            )
            .await
            .expect("history store");
            let cold = database(
                &manager,
                "cold",
                StdArc::new(Blake2b256HashCodec),
                StdArc::new(PersistedDataCodec),
            )
            .await
            .expect("cold store");
            Rig {
                history: RadixHistory::new(empty_root(), StdArc::new(history_store)).await,
                cold: StdArc::new(cold),
            }
        }

        fn reader(&self) -> StdArc<dyn HistoryReader<String, String, String, String>> {
            StdArc::new(
                RSpaceHistoryReaderImpl::<String, String, String, String>::new(
                    self.history.clone(),
                    self.cold.clone(),
                ),
            )
        }

        fn concrete(&self) -> RSpaceHistoryReaderImpl<String, String, String, String> {
            RSpaceHistoryReaderImpl::new(self.history.clone(), self.cold.clone())
        }

        /// Write a leaf into the cold store and point the history's `[prefix ‖ channel-hash]`
        /// segment at it — the two-step write the node performs at checkpoint.
        async fn commit(&mut self, prefix: u8, channel_hash: Blake2b256Hash, leaf: PersistedData) {
            let leaf_hash = Blake2b256Hash::create(&encode_persisted_data(&leaf));
            self.cold
                .put(&[(leaf_hash, leaf)])
                .await
                .expect("cold store write");
            let mut segment = vec![prefix];
            segment.extend_from_slice(channel_hash.as_bytes());
            self.history = self
                .history
                .process(&[HistoryAction::Insert {
                    key: KeySegment::new(segment),
                    hash: leaf_hash,
                }])
                .await
                .expect("history commit");
        }
    }

    const CHANNEL: &str = "chan";

    fn channel_hash() -> Blake2b256Hash {
        hash_channel(&CHANNEL.to_string())
    }

    /// An absent key is an **empty result**, not an error and not a panic: a channel that has never
    /// been used is the normal case for every getter on a fresh node.
    #[tokio::test]
    async fn absent_keys_read_as_empty() {
        let rig = Rig::new().await;
        let reader = rig.reader();

        assert_eq!(reader.root(), empty_root_hash());
        assert!(reader
            .get_data(channel_hash())
            .await
            .expect("data")
            .is_empty());
        assert!(reader
            .get_continuations(channel_hash())
            .await
            .expect("continuations")
            .is_empty());
        assert!(reader
            .get_joins(channel_hash())
            .await
            .expect("joins")
            .is_empty());
        assert_eq!(
            reader
                .get_native(0x07, channel_hash())
                .await
                .expect("native"),
            None,
            "the native reader distinguishes absence (None) from a wrong leaf (Err)"
        );
        assert_eq!(
            NativeHistoryReader::get_native(&rig.concrete(), 0x07, channel_hash())
                .await
                .expect("native"),
            None
        );
    }

    /// Data, continuations and joins round-trip through the three prefixes: each getter reads its
    /// **own** segment, so a leaf written for one kind is not served as another.
    #[tokio::test]
    async fn each_leaf_kind_round_trips_through_its_own_prefix() {
        let mut rig = Rig::new().await;
        let channel = CHANNEL.to_string();
        let datum: Datum<String> = Datum::create(&channel, "value".to_string(), true);
        rig.commit(
            PREFIX_DATUM,
            channel_hash(),
            PersistedData::DataLeaf(encode_datums(&[datum])),
        )
        .await;
        rig.commit(
            PREFIX_KONT,
            channel_hash(),
            PersistedData::ContinuationsLeaf(encode_continuations(&[WaitingContinuation::create(
                &[channel.clone()],
                vec!["pat".to_string()],
                "body".to_string(),
                false,
                std::collections::BTreeSet::new(),
            )])),
        )
        .await;
        rig.commit(
            PREFIX_JOINS,
            channel_hash(),
            PersistedData::JoinsLeaf(encode_joins(&[vec![channel.clone()]])),
        )
        .await;

        let reader = rig.reader();
        let data = reader.get_data(channel_hash()).await.expect("data");
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].a, "value");
        assert!(data[0].persist, "persistence survives the round trip");

        let konts = reader
            .get_continuations(channel_hash())
            .await
            .expect("continuations");
        assert_eq!(konts.len(), 1);
        assert_eq!(konts[0].continuation, "body");
        assert_eq!(konts[0].patterns, vec!["pat".to_string()]);

        let joins = reader.get_joins(channel_hash()).await.expect("joins");
        assert_eq!(joins, vec![vec![channel.clone()]]);

        // Each prefix is read separately: the datum getter does not see the joins leaf, and vice
        // versa — even though all three sit at the same channel hash.
        assert!(reader.get_data(hash_channels::<String>(&[])).await.is_ok());
        assert!(reader.get_joins(channel_hash()).await.expect("joins").len() == 1);
    }

    /// A leaf of the **wrong kind** under the expected prefix is an error, not an empty result:
    /// the datum getter that found a continuations leaf has hit a store inconsistency, and
    /// answering "no data" would hide it. The native reader reports the same condition as a string.
    #[tokio::test]
    async fn a_leaf_of_the_wrong_kind_is_an_error_not_an_empty_result() {
        let mut rig = Rig::new().await;
        rig.commit(
            PREFIX_DATUM,
            channel_hash(),
            PersistedData::ContinuationsLeaf(encode_continuations::<String, String>(&[])),
        )
        .await;
        rig.commit(
            0x07,
            channel_hash(),
            PersistedData::DataLeaf(encode_datums::<String>(&[])),
        )
        .await;

        let reader = rig.reader();
        assert_eq!(
            reader
                .get_data(channel_hash())
                .await
                .expect_err("wrong kind"),
            RSpaceError::UnexpectedLeaf("data")
        );
        assert_eq!(
            reader
                .get_native(0x07, channel_hash())
                .await
                .expect_err("wrong kind"),
            RSpaceError::UnexpectedLeaf("native")
        );
        assert_eq!(
            NativeHistoryReader::get_native(&rig.concrete(), 0x07, channel_hash())
                .await
                .expect_err("wrong kind"),
            "unexpected leaf while looking for native"
        );
        // The right getter for the leaf that *is* there succeeds, so the error above is about the
        // kind and not about the key being unreadable.
        assert!(reader
            .get_continuations(channel_hash())
            .await
            .expect("the continuations leaf is where it belongs")
            .is_empty());
    }

    /// A native leaf round-trips, and its own prefix space (above `0x02`) is separate from the three
    /// rspace prefixes — which is what lets the native system contracts store registry/PoS/vault
    /// state in the same trie.
    #[tokio::test]
    async fn a_native_leaf_round_trips_under_its_own_prefix() {
        let mut rig = Rig::new().await;
        let payload = vec![0xDE, 0xAD, 0xBE, 0xEF];
        rig.commit(
            0x07,
            channel_hash(),
            PersistedData::NativeLeaf(payload.clone()),
        )
        .await;

        let reader = rig.reader();
        assert_eq!(
            reader
                .get_native(0x07, channel_hash())
                .await
                .expect("native"),
            Some(payload.clone())
        );
        assert_eq!(
            NativeHistoryReader::get_native(&rig.concrete(), 0x07, channel_hash())
                .await
                .expect("native"),
            Some(payload)
        );
        // The same key under a *different* native prefix is a different leaf.
        assert_eq!(
            reader
                .get_native(0x08, channel_hash())
                .await
                .expect("other prefix"),
            None
        );
    }

    /// `base()` hashes the channel itself, so it reaches the same leaf as the raw reader given the
    /// hash — the indirection the write path relies on when it holds channels rather than hashes.
    #[tokio::test]
    async fn the_base_reader_hashes_the_channel_for_its_caller() {
        let mut rig = Rig::new().await;
        let channel = CHANNEL.to_string();
        rig.commit(
            PREFIX_DATUM,
            channel_hash(),
            PersistedData::DataLeaf(encode_datums(&[Datum::create(
                &channel,
                "value".to_string(),
                false,
            )])),
        )
        .await;
        rig.commit(
            PREFIX_KONT,
            hash_channels(&[channel.clone()]),
            PersistedData::ContinuationsLeaf(encode_continuations(&[WaitingContinuation::create(
                &[channel.clone()],
                vec!["pat".to_string()],
                "body".to_string(),
                false,
                std::collections::BTreeSet::new(),
            )])),
        )
        .await;

        let base = rig.reader().base();
        assert_eq!(
            base.get_data(&channel).await.expect("data").len(),
            1,
            "the base reader hashed the channel to the same key"
        );
        assert_eq!(
            base.get_joins(&channel).await.expect("joins"),
            Vec::<Vec<String>>::new(),
            "the joins were never written"
        );
        assert_eq!(
            base.get_continuations(&[channel.clone()])
                .await
                .expect("continuations")
                .len(),
            1,
            "a continuation is keyed by the *channels*, not one channel"
        );
    }

    /// The binary reader reads the **same trie** as the typed one, and an absent key is empty for
    /// both. Its *positive* cases cannot be written here at all: a binary element is a bit-packed
    /// scodec layout whose only producer is the typed writer (the merge path, covered by
    /// `rspace.rs`'s tests), so a hand-written leaf is not merely awkward to produce — it panics,
    /// which is the AUDIT §16 C15 behaviour pinned immediately below.
    #[tokio::test]
    async fn the_binary_reader_reads_the_same_trie_as_the_typed_one() {
        let rig = Rig::new().await;
        let binary = rig.reader().reader_binary();

        assert!(binary
            .get_data(channel_hash())
            .await
            .expect("absent")
            .is_empty());
        assert!(binary
            .get_continuations(channel_hash())
            .await
            .expect("absent")
            .is_empty());
        assert!(binary
            .get_joins(channel_hash())
            .await
            .expect("absent")
            .is_empty());

        // The typed reader reads a leaf the binary reader cannot decode, from the same store — the
        // difference is in the decode, not in the trie walk. (The panic that decode produces is
        // pinned by the two tests below.)
        let mut typed_rig = Rig::new().await;
        typed_rig
            .commit(
                PREFIX_DATUM,
                channel_hash(),
                PersistedData::DataLeaf(encode_datums(&[Datum::create(
                    &CHANNEL.to_string(),
                    "value".to_string(),
                    false,
                )])),
            )
            .await;
        assert_eq!(
            typed_rig
                .reader()
                .get_data(channel_hash())
                .await
                .expect("the typed reader reads what it wrote")
                .len(),
            1
        );
    }

    /// A binary leaf whose bytes are not a valid element for its kind **panics** rather than
    /// returning an error — AUDIT §16 C15, reached through this reader. The bytes come from this
    /// test's own store, which is exactly the C15 situation (a corrupted or truncated *local* entry
    /// aborts the merge); a peer-supplied one cannot reach this path, because inbound packets go
    /// through prost. These two tests exist so that giving `BitReader` a `Result` fails them, which
    /// is the deliberate change C15 records as the right fix.
    #[tokio::test]
    #[should_panic]
    async fn a_malformed_binary_datum_leaf_panics_today() {
        let mut rig = Rig::new().await;
        rig.commit(
            PREFIX_DATUM,
            channel_hash(),
            PersistedData::DataLeaf(encode_datums_binary(&[b"first".to_vec()])),
        )
        .await;
        let _ = rig.reader().reader_binary().get_data(channel_hash()).await;
    }

    /// The same for a continuation leaf: the panic is in the shared bit reader, not in one codec.
    #[tokio::test]
    #[should_panic]
    async fn a_malformed_binary_continuation_leaf_panics_today() {
        let mut rig = Rig::new().await;
        rig.commit(
            PREFIX_KONT,
            channel_hash(),
            PersistedData::ContinuationsLeaf(encode_continuations_binary(&[vec![6, 7]])),
        )
        .await;
        let _ = rig
            .reader()
            .reader_binary()
            .get_continuations(channel_hash())
            .await;
    }

    /// The three prefixes are the ones the writer uses (`0x00` datum, `0x01` continuations,
    /// `0x02` joins). They are the on-disk contract: a node that changed one would not find the
    /// state a previous build wrote.
    #[test]
    fn the_leaf_prefixes_are_the_documented_constants() {
        assert_eq!(
            (PREFIX_DATUM, PREFIX_KONT, PREFIX_JOINS),
            (0x00, 0x01, 0x02)
        );
    }
}
