//! Block API surface (port of `BlockApi.scala`): the `BlockApi` trait plus the block-info
//! constructors shared with `BlockApiImpl`.

use async_trait::async_trait;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use rchain_block_storage::dag::dag_storage::DeployId;
use rchain_models::ast::Par;
use rchain_models::block_metadata::BlockMetadata;
use rchain_models::casper::protocol::casper_message::{BlockMessage, SignedDeployData};
use rchain_models::casper::protocol::deploy_service::{
    BlockInfo, BondInfo, ContinuationsWithBlockInfo, DataWithBlockInfo, DeployExecStatus,
    LightBlockInfo, Status,
};
use rchain_models::validator::Validator;
use rchain_shared::base16;
use rchain_shared::refined::NonNegI64;
use serde::{Deserialize, Serialize};

use crate::runtime_manager::CapturedReply;

/// A block-api error (the Scala `BlockApi.Error = String`).
pub type ApiErr<A> = Result<A, String>;

/// The node's block-creation mode + deploy-gating capabilities, exposed to apps so a wallet can
/// decide whether to surface `propose` / the faucet instead of hardcoding a devnet flag.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    /// Continuous block production (`--autopropose`).
    pub autopropose: bool,
    /// Propose immediately after a deploy is accepted (`--propose-on-deploy`).
    pub propose_on_deploy: bool,
    /// Blocks are produced only by an explicit `propose` (neither of the above).
    pub manual_propose: bool,
    /// The admin HTTP surface (`POST /api/v1/propose` on 40405) is published and reachable from a
    /// browser (`--admin` + `--api-enable-devnet-cors`).
    pub admin_http: bool,
    /// Dev mode is on (`--dev-mode`).
    pub dev_mode: bool,
}

/// **A shard's proposer health, as the API reports it.**
///
/// **The flag names its trigger, and that is the point.** Three paths feed the proposer queue — the
/// autopropose tap, the attest-on-new-blocks tap, and the admin `POST /api/propose` — and only the
/// autopropose *timer* stops when the failure count reaches `AUTOPROPOSE_MAX_CONSECUTIVE_FAILURES`
/// (`node/src/runtime/node_runtime.rs`). The other two keep running, so a node whose timer has halted
/// can still produce blocks on inbound traffic or on demand. A field called `halted` would claim more
/// than the code does; this one says which trigger stopped, and the counter is reported beside it
/// because that is what an operator needs to tell "quiet" from "broken" (#148's probe needs exactly
/// that witness — the API could not tell the three causes apart).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProposerHealth {
    /// Self-validation failures recorded in a row. The proposer clears it on a successful propose —
    /// "consecutive" is the whole quantity, since one success in between is a node that recovered.
    pub consecutive_self_validation_failures: u64,
    /// The shard's autopropose timer has stopped. Nothing restarts it but the process, and the value
    /// is therefore sticky: a later success does **not** clear it.
    pub autopropose_timer_halted: bool,
}

/// The per-shard cell behind [`ProposerHealth`]: the proposer bumps the counter, the autopropose loop
/// records the halt, and the API reads both. Shared by `Clone` (every field is an `Arc`), so all three
/// holders see one cell rather than three copies.
#[derive(Clone, Debug, Default)]
pub struct ProposeHealth {
    failures: Arc<AtomicU64>,
    timer_halted: Arc<AtomicBool>,
}

impl ProposeHealth {
    pub fn new() -> Self {
        Self::default()
    }

    /// The counter the proposer bumps on a self-validation failure and clears on success — the same
    /// `Arc<AtomicU64>` `Proposer::apply` already takes, so the proposer needs no metrics handle of
    /// its own to be observable.
    pub fn failures(&self) -> Arc<AtomicU64> {
        self.failures.clone()
    }

    /// Record that the autopropose timer has stopped, called once immediately before its `break`.
    pub fn note_timer_halted(&self) {
        self.timer_halted.store(true, Ordering::Relaxed);
    }

    /// Both values as one observation, so a reader cannot see a count from one instant and a flag from
    /// another.
    pub fn snapshot(&self) -> ProposerHealth {
        ProposerHealth {
            consecutive_self_validation_failures: self.failures.load(Ordering::Relaxed),
            autopropose_timer_halted: self.timer_halted.load(Ordering::Relaxed),
        }
    }
}

/// The block API (port of `BlockApi[F]`). Implementations read from the block store/DAG and drive
/// propose via the runtime.
#[async_trait]
pub trait BlockApi: Send + Sync {
    async fn status(&self) -> Status;

    async fn deploy(&self, deploy: &SignedDeployData) -> ApiErr<String>;

    async fn deploy_status(&self, deploy_id: &DeployId) -> ApiErr<DeployExecStatus>;

    /// The currently-pooled (not-yet-included) deploys.
    async fn pooled_deploys(&self) -> ApiErr<Vec<SignedDeployData>>;

    /// The node's block-creation mode + deploy-gating capabilities.
    async fn capabilities(&self) -> Capabilities;

    async fn create_block(&self, is_async: bool) -> ApiErr<String>;

    async fn get_propose_result(&self) -> ApiErr<String>;

    async fn get_listening_name_data_response(
        &self,
        depth: i32,
        listening_name: &Par,
    ) -> ApiErr<(Vec<DataWithBlockInfo>, i32)>;

    async fn get_listening_name_continuation_response(
        &self,
        depth: i32,
        listening_names: &[Par],
    ) -> ApiErr<(Vec<ContinuationsWithBlockInfo>, i32)>;

    async fn get_blocks_by_heights(
        &self,
        start_block_number: i64,
        end_block_number: i64,
    ) -> ApiErr<Vec<LightBlockInfo>>;

    async fn visualize_dag(
        &self,
        depth: i32,
        start_block_number: i32,
        show_justification_lines: bool,
    ) -> ApiErr<Vec<String>>;

    async fn machine_verifiable_dag(&self, depth: i32) -> ApiErr<String>;

    async fn get_blocks(&self, depth: i32) -> ApiErr<Vec<LightBlockInfo>>;

    async fn find_deploy(&self, id: &DeployId) -> ApiErr<LightBlockInfo>;

    async fn get_block(&self, hash: &str) -> ApiErr<BlockInfo>;

    async fn bond_status(&self, public_key: &[u8]) -> ApiErr<bool>;

    /// The reply *and where it was read from*: a bare `Vec<Par>` cannot say whether the term
    /// produced nothing or replied on a channel the node does not read (AUDIT C38).
    async fn exploratory_deploy(
        &self,
        term: &str,
        block_hash: Option<&str>,
        use_pre_state_hash: bool,
    ) -> ApiErr<(CapturedReply, LightBlockInfo)>;

    async fn get_data_at_par(
        &self,
        par: &Par,
        block_hash: &str,
        use_pre_state_hash: bool,
    ) -> ApiErr<(Vec<Par>, LightBlockInfo)>;

    async fn last_finalized_block(&self) -> ApiErr<BlockInfo>;

    async fn is_finalized(&self, hash: &str) -> ApiErr<bool>;

    async fn get_latest_message(&self) -> ApiErr<BlockMetadata>;
}

/// Build a bond info (port of `bondToBondInfo`).
pub fn bond_to_bond_info(bond: (&Validator, NonNegI64)) -> BondInfo {
    BondInfo {
        validator: base16::encode(bond.0.as_bytes()),
        stake: i64::from(bond.1),
    }
}

/// Build the full block info (port of `getFullBlockInfo`).
pub fn get_full_block_info(block: &BlockMessage) -> BlockInfo {
    construct_block_info(block)
}

/// Build the light block info (port of `getLightBlockInfo`).
pub fn get_light_block_info(block: &BlockMessage) -> LightBlockInfo {
    construct_light_block_info(block)
}

fn construct_block_info(block: &BlockMessage) -> BlockInfo {
    let light_block_info = construct_light_block_info(block);
    let deploys = block
        .state
        .deploys
        .iter()
        .map(|d| d.to_deploy_info())
        .collect();
    BlockInfo {
        block_info: light_block_info,
        deploys,
    }
}

fn construct_light_block_info(block: &BlockMessage) -> LightBlockInfo {
    LightBlockInfo {
        version: block.version,
        shard_id: block.shard_id.clone(),
        block_hash: base16::encode(block.block_hash.as_bytes()),
        block_number: i64::from(block.block_number),
        sender: base16::encode(block.sender.as_bytes()),
        seq_num: i64::from(block.seq_num),
        pre_state_hash: base16::encode(block.pre_state_hash.as_bytes()),
        post_state_hash: base16::encode(block.post_state_hash.as_bytes()),
        justifications: block
            .justifications
            .iter()
            .map(|h| base16::encode(h.as_bytes()))
            .collect(),
        bonds: block
            .bonds
            .iter()
            .map(|(v, s)| bond_to_bond_info((v, *s)))
            .collect(),
        sig_algorithm: block.sig_algorithm.clone(),
        sig: base16::encode(&block.sig),
        block_size: block.to_bytes().len().to_string(),
        deploy_count: block.state.deploys.len() as i32,
        rejected_deploys: block
            .rejected_deploys
            .iter()
            .map(|d| base16::encode(d))
            .collect(),
        timestamp: block.timestamp,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rchain_models::block_hash::BlockHash;
    use rchain_models::casper::protocol::casper_message::RholangState;
    use std::collections::{BTreeMap, BTreeSet};

    fn block() -> BlockMessage {
        BlockMessage {
            version: 1,
            shard_id: "root".to_string(),
            block_hash: BlockHash::new([1u8; 32]),
            block_number: 5.try_into().unwrap(),
            sender: Validator::new([2u8; 65]),
            seq_num: 3.try_into().unwrap(),
            pre_state_hash: rchain_models::block::state_hash::StateHash::new([0xab; 32]),
            post_state_hash: rchain_models::block::state_hash::StateHash::new([0xcd; 32]),
            justifications: vec![BlockHash::new([9u8; 32])],
            bonds: BTreeMap::from([(Validator::new([2u8; 65]), 100.try_into().unwrap())]),
            rejected_deploys: BTreeSet::new(),
            rejected_blocks: BTreeSet::new(),
            rejected_senders: BTreeSet::new(),
            state: RholangState::default(),
            sig_algorithm: "secp256k1".to_string(),
            sig: vec![0xee],
            timestamp: 1_700_000_000_000,
        }
    }

    #[test]
    fn light_block_info_renders_hashes_as_hex() {
        let info = get_light_block_info(&block());
        assert_eq!(info.block_number, 5);
        assert_eq!(info.seq_num, 3);
        assert_eq!(info.deploy_count, 0);
        assert_eq!(info.bonds.len(), 1);
        assert_eq!(info.bonds[0].stake, 100);
        assert!(info.block_hash.starts_with("0101"));
        // The new block format's informational timestamp is exposed to clients.
        assert_eq!(info.timestamp, 1_700_000_000_000);
    }

    #[test]
    fn full_block_info_carries_deploys() {
        let info = get_full_block_info(&block());
        assert_eq!(info.block_info.block_number, 5);
        assert!(info.deploys.is_empty());
    }

    /// **The witness #157 is about, at the seam where its two halves meet.**
    ///
    /// The value the API reads is the cell the proposer writes — not a copy taken at setup, which
    /// would read zero for ever — and the two halves of the snapshot keep their distinct semantics: the
    /// count is *consecutive* (a success clears it), while the halt is *sticky* (the timer really has
    /// stopped, and a later success is exactly the case where a reader must not conclude the node is
    /// healthy).
    #[test]
    fn propose_health_is_the_live_cell_and_the_halt_outlives_a_success() {
        let health = ProposeHealth::new();
        assert_eq!(health.snapshot(), ProposerHealth::default());

        // The proposer's handle and the reader's view are the same cell.
        let proposer_handle = health.failures();
        proposer_handle.fetch_add(1, Ordering::Relaxed);
        proposer_handle.fetch_add(1, Ordering::Relaxed);
        assert_eq!(health.snapshot().consecutive_self_validation_failures, 2);
        assert!(!health.snapshot().autopropose_timer_halted);

        health.note_timer_halted();
        // The node recovers and proposes: the count clears, the halt does not.
        proposer_handle.store(0, Ordering::Relaxed);
        assert_eq!(
            health.snapshot(),
            ProposerHealth {
                consecutive_self_validation_failures: 0,
                autopropose_timer_halted: true,
            }
        );
    }
}
