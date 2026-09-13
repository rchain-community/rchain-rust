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

#[cfg(test)]
mod tests {
    use super::*;

    fn genesis() -> GenesisBlockData {
        GenesisBlockData {
            genesis_data_dir: PathBuf::from("/genesis"),
            bonds_file: "/genesis/bonds.txt".to_string(),
            wallets_file: "/genesis/wallets.txt".to_string(),
            bond_minimum: 1,
            bond_maximum: 100,
            epoch_length: 10,
            quarantine_length: 10,
            genesis_block_number: 0,
            number_of_active_validators: 10,
            pos_multi_sig_public_keys: Vec::new(),
            pos_multi_sig_quorum: 0,
            pos_vault_pub_key: String::new(),
            system_contract_pub_key: String::new(),
        }
    }

    fn spec(name: &str, parent: &str) -> ShardSpec {
        ShardSpec::new(name.to_string(), parent.to_string(), genesis(), 5).expect("valid spec")
    }

    /// A shard id must be non-empty and ASCII (Law 26), and `ShardId::child` does not check its
    /// argument — so a spec that skipped this validation could carry an id that `format_of_fields`
    /// would later reject at block validation.
    #[test]
    fn a_spec_rejects_an_illegal_name_or_parent() {
        let err = ShardSpec::new(String::new(), "/".to_string(), genesis(), 5)
            .expect_err("an empty name must be rejected");
        assert!(err.contains("invalid shard-name"), "{err}");

        let err = ShardSpec::new("røøt".to_string(), "/".to_string(), genesis(), 5)
            .expect_err("a non-ASCII name must be rejected");
        assert!(err.contains("invalid shard-name"), "{err}");

        let err = ShardSpec::new("root".to_string(), String::new(), genesis(), 5)
            .expect_err("an empty parent must be rejected");
        assert!(err.contains("invalid parent-shard-id"), "{err}");
    }

    /// The full id is the parent path plus the name, with the root's parent `/` collapsing so the
    /// default shard is `/root` rather than `//root`.
    #[test]
    fn a_spec_resolves_its_full_id_from_the_hierarchy() {
        assert_eq!(spec("root", "/").shard_id.to_string(), "/root");
        assert_eq!(spec("child", "/root").shard_id.to_string(), "/root/child");
        assert_eq!(
            spec("leaf", "/root/child").shard_id.to_string(),
            "/root/child/leaf"
        );
    }

    /// A membership set is never empty, and `primary` is total *because* of that — the property the
    /// per-shard assembly and the request router both rely on.
    #[test]
    fn memberships_are_never_empty_and_primary_is_the_first() {
        let memberships = ShardMemberships::new(vec![spec("root", "/")]).expect("one shard");
        assert_eq!(memberships.len(), 1);
        assert!(
            !memberships.is_empty(),
            "a membership set always has a primary"
        );
        assert_eq!(memberships.primary().shard_id.to_string(), "/root");

        let err = ShardMemberships::new(Vec::new())
            .expect_err("a node must be a member of at least one shard");
        assert!(err.contains("at least one shard"), "{err}");
    }

    #[test]
    fn memberships_keep_their_order_and_reject_duplicates() {
        let memberships = ShardMemberships::new(vec![
            spec("root", "/"),
            spec("child", "/root"),
            spec("leaf", "/root/child"),
        ])
        .expect("three shards");
        let ids: Vec<String> = memberships.iter().map(|s| s.shard_id.to_string()).collect();
        assert_eq!(ids, vec!["/root", "/root/child", "/root/child/leaf"]);
        assert_eq!(memberships.primary().shard_id.to_string(), "/root");

        let duplicate = ShardMemberships::new(vec![spec("root", "/"), spec("root", "/")]);
        let err = duplicate.expect_err("two memberships with one id must be rejected");
        assert!(err.contains("duplicate shard membership"), "{err}");
    }

    /// `get` distinguishes a member from a non-member — the lookup the deploy router is built on.
    #[test]
    fn get_finds_a_member_and_reports_a_non_member() {
        let memberships =
            ShardMemberships::new(vec![spec("root", "/"), spec("child", "/root")]).expect("two");
        let root = ShardId::try_from("/root".to_string()).unwrap();
        let elsewhere = ShardId::try_from("/elsewhere".to_string()).unwrap();
        assert_eq!(
            memberships.get(&root).map(|s| s.shard_name.as_str()),
            Some("root")
        );
        assert!(memberships.get(&elsewhere).is_none());
    }

    /// `primary_mut` edits the primary in place — how the test harnesses point a shard at a
    /// temporary genesis file.
    #[test]
    fn primary_mut_updates_the_primary_in_place() {
        let mut memberships = ShardMemberships::new(vec![spec("root", "/")]).expect("one");
        memberships.primary_mut().genesis_block_data.bonds_file = "/tmp/bonds".to_string();
        assert_eq!(
            memberships.primary().genesis_block_data.bonds_file,
            "/tmp/bonds"
        );
    }
}
