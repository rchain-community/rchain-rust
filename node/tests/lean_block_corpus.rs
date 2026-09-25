//! Laws 16a and 16b's `block` layer — the node's own `block_number` and `sequence_number`, on the same
//! data.
//!
//! `spec/conformance/block.tsv` is emitted by `lake exe rchain-corpus --layer block` from
//! `Rchain/Corpus.lean`'s `blockCases`: each row is a block as the two checks read it — its number, its
//! sequence number, its sender, and its justifications as `(sender, number, seqNum, failed)` — with
//! **two** verdicts, one per law, `decide`d in the tree against the model's predicates. This file is the
//! other party: it builds the same data as real `BlockMessage`s and calls the port's own
//! `casper::validate::block_number` (`casper/src/validate.rs:141`) and `sequence_number` (`:169`).
//!
//! **The model side is a conjunction, and one case is what that means.** The node's `block_number` is
//! *two* rules: the max-non-failed justification plus one (`BlockNumberValid`), **and** every resolved
//! parent strictly below the block (H1b, `:152-154` — the port refuses a parent at or above the child,
//! failed or not; the Scala does not). The corpus states both, which is why the last row — a block whose
//! only justification has failed — is refused: the number must be `0` by the max rule and exceed the
//! parent by the descent rule, and no block satisfies both. That case exists to pin the conjunction:
//! neither rule alone refuses it.
//!
//! The two checks touch the DAG only through `lookup`, so the storage double below holds metadata and
//! nothing else — `get_representation` is `unreachable!()`, which is a claim this test would falsify if
//! it ever changed.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use async_trait::async_trait;

use rchain_block_storage::dag::dag_storage::{BlockDagStorage, DeployId};
use rchain_block_storage::dag::representation::DagRepresentation;
use rchain_casper::validate::{block_number, sequence_number};
use rchain_models::block_hash::BlockHash;
use rchain_models::block_metadata::BlockMetadata;
use rchain_models::casper::protocol::casper_message::BlockMessage;
use rchain_models::casper::protocol::casper_message::RholangState;
use rchain_models::validator::Validator;
use rchain_shared::refined::{BlockHeight, SeqNum};

/// The corpus's declared size (`Rchain/Corpus.lean`'s `blockCaseCount`).
const BLOCK_CASES: usize = 10;

/// A DAG holding only metadata. The two checks read `lookup` and nothing else, so every other method is
/// a stub and `get_representation` says so by refusing to answer.
struct MetadataOnly(BTreeMap<BlockHash, BlockMetadata>);

#[async_trait]
impl BlockDagStorage for MetadataOnly {
    async fn get_representation(&self) -> Arc<DagRepresentation> {
        unreachable!("law 16a/16b read `lookup` only; this test would fail if that changed")
    }
    async fn insert(&self, _m: BlockMetadata, _b: BlockMessage) -> Result<(), String> {
        Ok(())
    }
    async fn lookup(&self, h: &BlockHash) -> Result<Option<BlockMetadata>, String> {
        Ok(self.0.get(h).cloned())
    }
    async fn lookup_by_deploy_id(&self, _d: &DeployId) -> Result<Option<BlockHash>, String> {
        Ok(None)
    }
    async fn add_deploy(
        &self,
        _d: rchain_models::casper::protocol::casper_message::SignedDeployData,
    ) -> Result<(), String> {
        Ok(())
    }
    async fn pooled_deploys(
        &self,
    ) -> Result<
        BTreeMap<DeployId, rchain_models::casper::protocol::casper_message::SignedDeployData>,
        String,
    > {
        Ok(BTreeMap::new())
    }
    async fn contains_deploy_in_pool(&self, _d: &DeployId) -> Result<bool, String> {
        Ok(false)
    }
}

/// The hash of the `n`-th justification: distinct per index, and nothing else reads its value.
fn parent_hash(n: usize) -> BlockHash {
    let mut bytes = [0u8; 32];
    bytes[0] = 0xa0 + n as u8;
    BlockHash::new(bytes)
}

/// A block as the two checks read it.
fn block(sender: u8, block_num: i64, seq: i64, justifications: Vec<BlockHash>) -> BlockMessage {
    BlockMessage {
        version: 1,
        shard_id: "root".to_string(),
        block_hash: BlockHash::new([0xee; 32]),
        block_number: BlockHeight::try_from(block_num).unwrap(),
        sender: Validator::new([sender; 65]),
        seq_num: SeqNum::try_from(seq).unwrap(),
        pre_state_hash: rchain_models::block::state_hash::StateHash::new([1u8; 32]),
        post_state_hash: rchain_models::block::state_hash::StateHash::new([2u8; 32]),
        justifications,
        bonds: BTreeMap::new(),
        rejected_deploys: BTreeSet::new(),
        rejected_blocks: BTreeSet::new(),
        rejected_senders: BTreeSet::new(),
        state: RholangState::default(),
        sig_algorithm: "secp256k1".to_string(),
        sig: vec![1],
        timestamp: 0,
    }
}

#[tokio::test]
async fn the_node_agrees_with_the_model_on_every_block_number_and_sequence_number() {
    let path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../spec/conformance/block.tsv");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "read {}: {e}\n(run tools/emit-lean-corpus.sh)",
            path.display()
        )
    });

    let mut cases = 0usize;
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let columns: Vec<&str> = line.split('\t').collect();
        assert_eq!(columns.len(), 7, "corpus line {}: columns", i + 1);
        assert_eq!(columns[0], "block", "corpus line {}: layer", i + 1);
        let number: i64 = columns[1].parse().expect("number");
        let seq_num: i64 = columns[2].parse().expect("seqNum");
        let sender: u64 = columns[3].parse().expect("sender");
        let expected_number: bool = columns[5].parse().expect("the 16a verdict");
        let expected_seq: bool = columns[6].parse().expect("the 16b verdict");

        // The parents, as blocks the DAG holds metadata for.
        let mut metadata = BTreeMap::new();
        let mut justifications: Vec<BlockHash> = Vec::new();
        for (n, entry) in columns[4].split(',').filter(|s| !s.is_empty()).enumerate() {
            let f: Vec<&str> = entry.split(':').collect();
            assert_eq!(f.len(), 4, "corpus line {}: parent {entry:?}", i + 1);
            let p = block(
                f[0].parse().expect("parent sender"),
                f[1].parse().expect("parent number"),
                f[2].parse().expect("parent seqNum"),
                Vec::new(),
            );
            let mut meta = BlockMetadata::from_block(&p);
            meta.validation_failed = f[3] == "1";
            let h = parent_hash(n);
            meta.block_hash = h;
            metadata.insert(h, meta);
            justifications.push(h);
        }

        let b = block(sender as u8, number, seq_num, justifications);
        let dag = MetadataOnly(metadata);

        let number_says = matches!(block_number(&dag, &b).await, Ok(Ok(())));
        let seq_says = matches!(sequence_number(&dag, &b).await, Ok(Ok(())));

        assert_eq!(
            number_says,
            expected_number,
            "corpus line {}: block_number says {number_says}, the model says {expected_number} \
             (spec/conformance/block.tsv, law 16a)",
            i + 1
        );
        assert_eq!(
            seq_says,
            expected_seq,
            "corpus line {}: sequence_number says {seq_says}, the model says {expected_seq} \
             (spec/conformance/block.tsv, law 16b)",
            i + 1
        );
        cases += 1;
    }

    assert_eq!(
        cases, BLOCK_CASES,
        "the corpus carries {BLOCK_CASES} cases (Rchain/Corpus.lean's blockCaseCount); {cases} were read"
    );
}
