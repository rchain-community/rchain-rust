//! Casper configuration types (port of `CasperConf.scala`).

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use rchain_shared::refined::ShardId;

/// One shard this node is a member of (Law 26), fully resolved: the validated full id plus the
/// per-shard genesis and consensus data.
///
/// The node keeps the `(parent-shard-id, shard-name)` *authoring* form — it is how the hierarchy is
/// expressed and `ShardId::child` is the only constructor for a child id — and resolves it once, at
/// construction, so every consumer downstream reads the same validated `shard_id`.
#[derive(Clone, Debug, PartialEq)]
pub struct ShardSpec {
    /// The validated full id `{parent_shard_id}/{shard_name}`. This is the single value the
    /// proposer, the block receiver's `check_if_of_interest`, the deploy APIs and the RNG seed all
    /// agree on.
    pub shard_id: ShardId,
    /// The bare name. Kept because `Genesis.shard_id` is a `String` (the genesis path needs it) and
    /// because it is the authoring half the operator writes.
    pub shard_name: String,
    /// The parent shard's id (the `/`-separated path this shard nests under); the root's is `/`.
    pub parent_shard_id: String,
    /// This shard's genesis data. A multi-shard node gives each membership its own bonds and
    /// wallets — different validator sets per shard are the point.
    pub genesis_block_data: GenesisBlockData,
    pub autogen_shard_size: i32,
}

impl ShardSpec {
    /// Resolve a spec from its authoring form, validating both halves (Law 26: a non-empty, ASCII
    /// shard id).
    pub fn new(
        shard_name: String,
        parent_shard_id: String,
        genesis_block_data: GenesisBlockData,
        autogen_shard_size: i32,
    ) -> Result<Self, String> {
        // `ShardId::child` composes the name without validating it, so the name is checked on its
        // own — otherwise a spec could carry an id that `format_of_fields` would later reject.
        ShardId::try_from(shard_name.clone())
            .map_err(|e| format!("invalid shard-name '{shard_name}': {e}"))?;
        let parent = ShardId::try_from(parent_shard_id.clone())
            .map_err(|e| format!("invalid parent-shard-id '{parent_shard_id}': {e}"))?;
        Ok(ShardSpec {
            shard_id: parent.child(&shard_name),
            shard_name,
            parent_shard_id,
            genesis_block_data,
            autogen_shard_size,
        })
    }
}

/// The shards this node is a member of — **always at least one**, index 0 being the *primary* shard.
///
/// Non-emptiness is carried by the type rather than checked at each use, so [`Self::primary`] is
/// total and every shard-selector-less query has a well-defined default. A single-shard node is the
/// one-element case, and is byte-for-byte today's behaviour.
#[derive(Clone, Debug, PartialEq)]
pub struct ShardMemberships {
    primary: ShardSpec,
    others: Vec<ShardSpec>,
}

impl ShardMemberships {
    /// Build the memberships from the configured specs, rejecting an empty list (a node must be a
    /// member of at least one shard) and duplicate ids (two memberships with one id would share a
    /// state hash and a data directory).
    pub fn new(specs: Vec<ShardSpec>) -> Result<Self, String> {
        let mut iter = specs.into_iter();
        let primary = iter
            .next()
            .ok_or_else(|| "a node must be a member of at least one shard".to_string())?;
        let memberships = ShardMemberships {
            primary,
            others: iter.collect(),
        };
        let mut seen: BTreeSet<&ShardId> = BTreeSet::new();
        for spec in memberships.iter() {
            if !seen.insert(&spec.shard_id) {
                return Err(format!("duplicate shard membership '{}'", spec.shard_id));
            }
        }
        Ok(memberships)
    }

    /// The primary shard: the default target for shard-selector-less queries.
    pub fn primary(&self) -> &ShardSpec {
        &self.primary
    }

    pub fn primary_mut(&mut self) -> &mut ShardSpec {
        &mut self.primary
    }

    /// Every membership, primary first.
    pub fn iter(&self) -> impl Iterator<Item = &ShardSpec> {
        std::iter::once(&self.primary).chain(self.others.iter())
    }

    pub fn len(&self) -> usize {
        1 + self.others.len()
    }

    /// Never empty — a membership set always has at least its primary shard. Present because a
    /// public `len` without `is_empty` trips `clippy::len_without_is_empty`.
    pub fn is_empty(&self) -> bool {
        false
    }

    /// The membership with this id, if the node is a member of it.
    pub fn get(&self, shard_id: &ShardId) -> Option<&ShardSpec> {
        self.iter().find(|spec| &spec.shard_id == shard_id)
    }
}

/// Consensus configuration (port of the Scala `CasperConf` case class).
#[derive(Clone, Debug, PartialEq)]
pub struct CasperConf {
    pub validator_public_key: Option<String>,
    pub validator_private_key: Option<String>,
    pub validator_private_key_path: Option<PathBuf>,
    /// The shards this node is a member of (Law 26); index 0 is the primary shard. A single-shard
    /// node is a one-element list.
    pub shards: ShardMemberships,
    pub casper_loop_interval: Duration,
    pub requested_blocks_timeout: Duration,
    pub max_number_of_parents: i32,
    pub fork_choice_stale_threshold: Duration,
    pub fork_choice_check_if_stale_interval: Duration,
    pub synchrony_constraint_threshold: f64,
    pub height_constraint_threshold: i64,
    pub min_phlo_price: i64,
    /// The effect-scheduler mode (Laws 20–25): `dfs` (default), `gate`, `relaxed`, or
    /// `relaxed-validated`. Parsed via `FromStr` at consumption (`EffectMode`); the block paths
    /// hard-reject `relaxed` (off-chain only) and accept `relaxed-validated` with
    /// sequential-reference validation (Laws 23–25,
    /// `docs/src/formal/onchain-scheduling.md`).
    pub effect_mode: String,
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
