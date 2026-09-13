//! Casper configuration types (port of `CasperConf.scala`).

use std::path::PathBuf;
use std::time::Duration;

use rchain_shared::refined::ShardId;

/// Consensus configuration (port of the Scala `CasperConf` case class).
#[derive(Clone, Debug, PartialEq)]
pub struct CasperConf {
    pub validator_public_key: Option<String>,
    pub validator_private_key: Option<String>,
    pub validator_private_key_path: Option<PathBuf>,
    pub shard_name: String,
    /// The parent shard's id (the `/`-separated path this shard nests under). The full shard id is
    /// `{parent_shard_id}/{shard_name}`; the root's parent is `/` (Law 26).
    pub parent_shard_id: String,
    pub casper_loop_interval: Duration,
    pub requested_blocks_timeout: Duration,
    pub max_number_of_parents: i32,
    pub fork_choice_stale_threshold: Duration,
    pub fork_choice_check_if_stale_interval: Duration,
    pub synchrony_constraint_threshold: f64,
    pub height_constraint_threshold: i64,
    pub genesis_block_data: GenesisBlockData,
    pub autogen_shard_size: i32,
    pub min_phlo_price: i64,
    /// The effect-scheduler mode (Laws 20–25): `dfs` (default), `gate`, `relaxed`, or
    /// `relaxed-validated`. Parsed via `FromStr` at consumption (`EffectMode`); the block paths
    /// hard-reject `relaxed` (off-chain only) and accept `relaxed-validated` with
    /// sequential-reference validation (Laws 23–25,
    /// `docs/src/formal/onchain-scheduling.md`).
    pub effect_mode: String,
}

impl CasperConf {
    /// The full shard id `{parent_shard_id}/{shard_name}` — a validated, hierarchical path
    /// (Law 26). The root's parent is `/`, so the default `root` shard is `/root`.
    pub fn full_shard_id(&self) -> Result<ShardId, String> {
        let parent =
            ShardId::try_from(self.parent_shard_id.clone()).map_err(|e| e.to_string())?;
        Ok(parent.child(&self.shard_name))
    }
}

/// Genesis-block data configuration (port of the Scala `GenesisBlockData` case class).
#[derive(Clone, Debug, PartialEq)]
pub struct GenesisBlockData {
    pub genesis_data_dir: PathBuf,
    pub bonds_file: String,
    pub wallets_file: String,
    pub bond_minimum: i64,
    pub bond_maximum: i64,
    pub epoch_length: i32,
    pub quarantine_length: i32,
    pub genesis_block_number: i64,
    pub number_of_active_validators: i32,
    pub pos_multi_sig_public_keys: Vec<String>,
    pub pos_multi_sig_quorum: i32,
    pub pos_vault_pub_key: String,
    pub system_contract_pub_key: String,
}
