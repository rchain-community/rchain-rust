//! Genesis block creation (port of `casper/genesis/Genesis.scala`).

pub mod contracts;
pub mod rgov;
pub mod standard_deploys;

use std::collections::{BTreeMap, BTreeSet};

use rchain_crypto::public_key::PublicKey;
use rchain_models::block::state_hash::StateHash;
use rchain_models::block_version::CURRENT;
use rchain_models::casper::protocol::casper_message::{
    BlockMessage, ProcessedDeploy, RholangState, SignedDeployData,
};
use rchain_models::validator::Validator as ModelsValidator;
use rchain_rholang::native_state::{PosGenesis, PosParams};
use rchain_rholang::system_processes::BlockData;
use rchain_shared::base16;
use rchain_shared::refined::NonNegI64;

use crate::block_random_seed::BlockRandomSeed;
use crate::genesis::contracts::{ProofOfStake, Registry, Vault};
use crate::proto_util::unsigned_block_proto;
use crate::runtime_manager::RuntimeManager;
use crate::validator_identity::ValidatorIdentity;
use rchain_shared::refined::{BlockHeight, SeqNum};

/// Genesis parameters (port of `Genesis`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Genesis {
    pub sender: PublicKey,
    pub shard_id: String,
    pub block_number: i64,
    pub proof_of_stake: ProofOfStake,
    pub registry: Registry,
    pub vaults: Vec<Vault>,
}

/// Build the bonds map (validator pubkey → stake) from the PoS validators (port of `buildBondsMap`).
fn build_bonds_map(proof_of_stake: &ProofOfStake) -> BTreeMap<ModelsValidator, NonNegI64> {
    proof_of_stake
        .validators
        .iter()
        .map(|v| (ModelsValidator::from_slice(v.pk.bytes()), v.stake))
        .collect()
}

/// Decode a base16 validator public key (used for the config's `pos-multi-sig-public-keys`).
fn parse_validator_hex(key: &str) -> Option<ModelsValidator> {
    let bytes = base16::decode(key)?;
    ModelsValidator::try_from(bytes.as_slice()).ok()
}

/// Build the genesis PoS descriptors: the bond pool, the trusted stakeholder set (the genesis
/// validators plus any configured `pos-multi-sig-public-keys`), and the network parameters.
pub fn build_pos_genesis(proof_of_stake: &ProofOfStake) -> PosGenesis {
    let bonds = build_bonds_map(proof_of_stake);
    let mut trusted: BTreeSet<ModelsValidator> = bonds.keys().copied().collect();
    for key in &proof_of_stake.pos_multi_sig_public_keys {
        if let Some(validator) = parse_validator_hex(key) {
            trusted.insert(validator);
        }
    }
    PosGenesis {
        bonds,
        trusted,
        params: PosParams {
            minimum_bond: proof_of_stake.minimum_bond,
            maximum_bond: proof_of_stake.maximum_bond,
            epoch_length: i64::from(proof_of_stake.epoch_length),
            quarantine_length: i64::from(proof_of_stake.quarantine_length),
            number_of_active_validators: i64::from(proof_of_stake.number_of_active_validators),
        },
    }
}

/// Build the genesis PoS descriptors for one shard from its spec (the same pool/trusted/params that
/// shard's genesis ceremony installs). Used to seed `RuntimeManager` so a genesis-block replay
/// reconstructs the native genesis state. Call only on a genesis-ceremony (standalone) node: it may
/// create the bonds file via `parse_or_generate`.
pub fn pos_genesis_from_config(spec: &crate::conf::ShardSpec) -> Result<PosGenesis, String> {
    let gbd = &spec.genesis_block_data;
    let bonds = crate::bonds_parser::parse_or_generate(
        std::path::Path::new(&gbd.bonds_file),
        spec.autogen_shard_size,
    )?;
    let validators: Vec<contracts::Validator> = bonds
        .into_iter()
        .map(|(pk, stake)| contracts::Validator { pk, stake })
        .collect();
    let proof_of_stake = ProofOfStake {
        minimum_bond: gbd.bond_minimum,
        maximum_bond: gbd.bond_maximum,
        validators,
        epoch_length: gbd.epoch_length,
        quarantine_length: gbd.quarantine_length,
        number_of_active_validators: gbd.number_of_active_validators,
        pos_multi_sig_public_keys: gbd.pos_multi_sig_public_keys.clone(),
        pos_multi_sig_quorum: gbd.pos_multi_sig_quorum,
        pos_vault_pub_key: gbd.pos_vault_pub_key.clone(),
    };
    Ok(build_pos_genesis(&proof_of_stake))
}

/// Build the unsigned genesis block from processed deploys (port of
/// `createBlockWithProcessedDeploys`). `bonds` is the *active* validator set.
fn create_block_with_processed_deploys(
    genesis: &Genesis,
    bonds: BTreeMap<ModelsValidator, NonNegI64>,
    pre_state_hash: StateHash,
    post_state_hash: StateHash,
    processed_deploys: Vec<ProcessedDeploy>,
) -> Result<BlockMessage, String> {
    if let Some(failed) = processed_deploys.iter().find(|d| d.is_failed) {
        return Err(format!(
            "Genesis block contains a failed deploy (deployer {:?})",
            failed.deploy.deployer
        ));
    }
    let state = RholangState {
        deploys: processed_deploys,
        system_deploys: Vec::new(),
    };
    Ok(unsigned_block_proto(
        CURRENT,
        genesis.shard_id.clone(),
        BlockHeight::try_from(genesis.block_number).map_err(|e| e.to_string())?,
        ModelsValidator::from_slice(genesis.sender.bytes()),
        SeqNum::zero(),
        pre_state_hash,
        post_state_hash,
        Vec::new(),
        bonds,
        BTreeSet::new(),
        state,
        // Genesis carries a fixed timestamp (0) so every node agrees; informational only.
        0,
    ))
}

/// The order the blessed deploys depend on: `(dependent, what must already be installed)`.
///
/// Exactly one entry today, and it is the sharp kind: `MakeMint.rho:27` sends
/// `lookup!(\`rho:lang:nonNegativeNumber\`, …)` **during its own deploy** and waits on a reply pattern
/// a `Nil` reply cannot match. An unmatched `for` is not an error, so with the wrong order the deploy
/// *succeeds* while `MakeMint` never registers at all — nothing fails, and `lookup!` on `makeMint`
/// just keeps answering `Nil`. What catches it is the genesis ceremony's completeness check
/// (`missing_genesis_aliases`), and
/// `installing_make_mint_before_its_dependency_is_caught_by_the_genesis_check` pins both halves of
/// that: the silence, and the check that ends it.
///
/// The vendored rgov contracts need no entry: `memberIdGovRev` looks its `directory`/`inbox` imports
/// up **per call**, not at deploy time, so its position in the genesis list is free (all three are
/// genesis content, so a client calling it always finds them). That was worth checking rather than
/// assuming — the first version of this table claimed an order for it, and the negative test refuted
/// the claim.
#[cfg(test)]
const BLESSED_DEPENDENCIES: &[(&str, &[&str])] = &[("make_mint", &["non_negative_number"])];

/// The blessed set with its manifest names, in install order.
///
/// The governance block ([`rgov::governance_deploys`]) follows the libraries: its own order is
/// internal (classes → master directory → the extra slots → the `GetMe` feature), because each step
/// resolves what the previous one published. It is **testnet-only** — see the module doc in
/// `rgov.rs` and `spec/GENESIS.md` for what a public network must do instead.
fn blessed_terms_named(shard_id: &str) -> Result<Vec<(&'static str, SignedDeployData)>, String> {
    let standard: Vec<(&'static str, SignedDeployData)> = vec![
        (
            "list_ops",
            standard_deploys::StandardDeploys::list_ops(shard_id)?,
        ),
        (
            "non_negative_number",
            standard_deploys::StandardDeploys::non_negative_number(shard_id)?,
        ),
        (
            "make_mint",
            standard_deploys::StandardDeploys::make_mint(shard_id)?,
        ),
    ];
    let rgov = rgov::governance_deploys(shard_id)?;
    Ok(standard.into_iter().chain(rgov).collect())
}

/// Copy the **published** entries of the vendored governance contracts onto the constant keys this
/// node chooses (`rgov::contract_uri_for`, `rgov::readcap_uri`).
///
/// A class registers with `insertArbitrary` — upstream's shape, which consumers destructure — so its
/// own URI is `blake2b256` of the deploy's RNG state: deterministic on genesis, but moved by
/// inserting any deploy before it. Each class therefore publishes `["<name>", uri]` on
/// `rgov::URI_PUBLISH_CHANNEL`, and this copies the stored entry to the constant key so a client can
/// hardcode it and does not depend on the install order.
///
/// Idempotent, and called after every blessed deploy — the template's read cap is published by the
/// deploy that mints it, which is several deploys after the classes.
pub async fn seed_rgov_aliases(runtime: &RuntimeManager) -> Result<usize, String> {
    let published = runtime
        .runtime()
        .get_data_par(&rgov::publish_channel())
        .await
        .map_err(|e| e.to_string())?;
    let native =
        rchain_rholang::native_state::NativeSystemState::new(runtime.runtime().native_store());
    seed_rgov_aliases_from(&published, &native).await
}

/// The play/replay-independent half of [`seed_rgov_aliases`]: it takes the published datums and the
/// native store, so the replay path (which holds a `ReplayRuntime`, not a `RuntimeManager`) can
/// reproduce the seeding exactly — a native write outside the deploy log must match or the replayed
/// genesis hash diverges (Law 11).
pub async fn seed_rgov_aliases_from(
    published: &[rchain_models::ast::Par],
    native: &rchain_rholang::native_state::NativeSystemState,
) -> Result<usize, String> {
    let mut seeded = 0;
    for datum in published {
        let Some((name, uri)) = rgov::published_uri(datum) else {
            continue;
        };
        let target = if name == "readcap" {
            rgov::readcap_uri()?
        } else {
            rgov::contract_uri_for(&name)?
        };
        if native
            .registry_lookup(&target)
            .await
            .map_err(|e| e.to_string())?
            .is_some()
        {
            continue;
        }
        let Some(value) = native
            .registry_lookup(&uri)
            .await
            .map_err(|e| e.to_string())?
        else {
            // Published before its registration landed; the next call will see it.
            continue;
        };
        native.registry_insert(&target, &value);
        seeded += 1;
    }
    Ok(seeded)
}

/// Every governance key a fresh chain must resolve, for the ceremony's completeness check: the
/// classes, the master directory's read cap, and every seeded shorthand.
pub async fn missing_governance_keys(
    native: &rchain_rholang::native_state::NativeSystemState,
) -> Result<Vec<String>, String> {
    let mut missing = Vec::new();
    for name in rgov::RGOV_CORE {
        let uri = rgov::contract_uri_for(name)?;
        if native
            .registry_lookup(&uri)
            .await
            .map_err(|e| e.to_string())?
            .is_none()
        {
            missing.push(uri);
        }
    }
    let readcap = rgov::readcap_uri()?;
    if native
        .registry_lookup(&readcap)
        .await
        .map_err(|e| e.to_string())?
        .is_none()
    {
        missing.push(readcap);
    }
    Ok(missing)
}

/// The ordered list of blessed (standard) genesis deploys (port of `defaultBlessedTerms`).
///
/// Rust-first: the registry, PoS and vault *system* contracts are native (`rholang::native_state` +
/// `system_deploy::NativeSystemDeployOp`), so their `.rho`/`.rhox` sources stay a checklist and are
/// **not** installed — installing them would shadow consensus-critical logic with interpreted
/// equivalents. What genesis does install is the small set of interpreted contracts a consumer
/// actually reaches through `rho:registry:lookup`, in dependency order ([`BLESSED_DEPENDENCIES`]),
/// plus the registry aliases that make those lookups resolve ([`seed_registry_aliases`],
/// `spec/GENESIS.md`).
///
/// Before this, a fresh chain's registry was empty, so `lookup!(\`rho:rchain:revVault\`, *ch)`
/// answered `Nil` — and a consumer cannot tell a `Nil` reply from a pattern that never matched.
pub fn default_blessed_terms(
    _proof_of_stake: &ProofOfStake,
    _registry: &Registry,
    _vaults: &[Vault],
    shard_id: &str,
) -> Result<Vec<SignedDeployData>, String> {
    Ok(blessed_terms_named(shard_id)?
        .into_iter()
        .map(|(_, deploy)| deploy)
        .collect())
}

/// Seed the genesis registry aliases whose source is now available. Idempotent, so the genesis loop
/// can call it after every blessed deploy and once at the end.
///
/// Native system channels are seeded on the first call. A *contract* alias is seeded as soon as the
/// blessed deploy that registered it has run — the entry is copied from the contract's deterministic
/// `rho:id` onto the shorthand consumers look up. This is called after *every* deploy rather than
/// once at the end because `MakeMint` looks up `rho:lang:nonNegativeNumber` **during** its deploy,
/// so that alias must already exist when MakeMint runs.
pub async fn seed_registry_aliases(
    native: &rchain_rholang::native_state::NativeSystemState,
) -> Result<usize, String> {
    let mut seeded = 0;
    for alias in standard_deploys::GENESIS_ALIASES {
        if native
            .registry_lookup(alias.shorthand)
            .await
            .map_err(|e| e.to_string())?
            .is_some()
        {
            continue;
        }
        let value = match alias.source {
            standard_deploys::GenesisAliasSource::NativeChannel => {
                standard_deploys::native_channel_alias(alias.shorthand).ok_or_else(|| {
                    format!(
                        "genesis alias {} names a native channel that is not a definition",
                        alias.shorthand
                    )
                })?
            }
            standard_deploys::GenesisAliasSource::Contract { private_key_hex } => {
                let uri = standard_deploys::contract_uri(private_key_hex)?;
                match native
                    .registry_lookup(&uri)
                    .await
                    .map_err(|e| e.to_string())?
                {
                    Some(entry) => entry,
                    // Its blessed deploy has not run yet; the next call after it has will seed this.
                    None => continue,
                }
            }
        };
        native.registry_insert(alias.shorthand, &value);
        seeded += 1;
    }
    Ok(seeded)
}

/// Every shorthand the genesis registry must resolve after the blessed deploys have run. Used to
/// fail the genesis loudly if an alias did not get seeded — a missing alias is the silent-`Nil`
/// failure this whole module exists to remove.
pub async fn missing_genesis_aliases(
    native: &rchain_rholang::native_state::NativeSystemState,
) -> Result<Vec<&'static str>, String> {
    let mut missing = Vec::new();
    for alias in standard_deploys::GENESIS_ALIASES {
        if native
            .registry_lookup(alias.shorthand)
            .await
            .map_err(|e| e.to_string())?
            .is_none()
        {
            missing.push(alias.shorthand);
        }
    }
    Ok(missing)
}

/// Create the signed genesis block (port of `Genesis.createGenesisBlock`).
pub async fn create_genesis_block(
    validator: &ValidatorIdentity,
    genesis: &Genesis,
    runtime: &RuntimeManager,
) -> Result<BlockMessage, String> {
    let blessed_terms = default_blessed_terms(
        &genesis.proof_of_stake,
        &genesis.registry,
        &genesis.vaults,
        &genesis.shard_id,
    )?;
    let block_data = BlockData {
        block_number: BlockHeight::try_from(genesis.block_number).map_err(|e| e.to_string())?,
        sender: genesis.sender.clone(),
        seq_num: SeqNum::zero(),
        // Genesis carries a fixed timestamp (0) so every node agrees; informational only.
        timestamp: 0,
    };
    let rand = BlockRandomSeed::random_generator_from_shard_id(&genesis.shard_id);
    let pos_genesis = build_pos_genesis(&genesis.proof_of_stake);
    let (start_hash, state_hash, processed_results) = runtime
        .compute_genesis(
            &blessed_terms,
            &rand,
            block_data,
            &pos_genesis,
            &genesis.vaults,
        )
        .await?;
    // Surface deploy evaluation errors (the Scala `require` only checks the `isFailed` flag; the
    // underlying errors are otherwise lost, making genesis failures opaque).
    for (i, r) in processed_results.iter().enumerate() {
        if !r.eval_result.errors.is_empty() {
            return Err(format!(
                "Genesis deploy #{i} failed: {:?}",
                r.eval_result.errors
            ));
        }
    }
    // The ceremony is where the full manifest must hold: a chain that starts with a shorthand
    // answering `Nil` gives every consumer a silent no-op, and the place to refuse that is here —
    // not on a client's first lookup days later.
    let native =
        rchain_rholang::native_state::NativeSystemState::new(runtime.runtime().native_store());
    let missing = missing_genesis_aliases(&native).await?;
    if !missing.is_empty() {
        return Err(format!(
            "genesis registry aliases were not seeded: {missing:?} — the chain would answer `Nil` \
             to those lookups"
        ));
    }
    // The governance block too: a class that registered but never published (or a template that
    // never minted its read cap) leaves a key a client hardcodes resolving to `Nil`, which is the
    // silent no-op this whole module exists to refuse.
    let missing_governance = missing_governance_keys(&native).await?;
    if !missing_governance.is_empty() {
        return Err(format!(
            "genesis governance keys were not published: {missing_governance:?} — a client \
             hardcoding them would get `Nil`"
        ));
    }
    let processed_deploys: Vec<ProcessedDeploy> =
        processed_results.into_iter().map(|r| r.deploy).collect();

    let unsigned_block = create_block_with_processed_deploys(
        genesis,
        pos_genesis.active_bonds(),
        start_hash.into(),
        state_hash.into(),
        processed_deploys,
    )?;
    let signed_block = validator
        .sign_block(&unsigned_block)
        .map_err(|e| e.to_string())?;

    // Signing must not change the block hash.
    if unsigned_block.block_hash != signed_block.block_hash {
        return Err("Signed block has different block hash than unsigned".to_string());
    }
    Ok(signed_block)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genesis::contracts::Validator;

    fn pos() -> ProofOfStake {
        ProofOfStake {
            minimum_bond: 1,
            maximum_bond: 100,
            validators: vec![
                Validator {
                    pk: PublicKey::new(vec![1; 65]),
                    stake: 10.try_into().unwrap(),
                },
                Validator {
                    pk: PublicKey::new(vec![2; 65]),
                    stake: 20.try_into().unwrap(),
                },
            ],
            epoch_length: 0,
            quarantine_length: 0,
            number_of_active_validators: 0,
            pos_multi_sig_public_keys: vec![],
            pos_multi_sig_quorum: 0,
            pos_vault_pub_key: String::new(),
        }
    }

    #[test]
    fn build_bonds_map_extracts_stakes() {
        let bonds = build_bonds_map(&pos());
        assert_eq!(bonds.len(), 2);
        assert_eq!(i64::from(bonds[&ModelsValidator::from_slice(&[1; 65])]), 10);
        assert_eq!(i64::from(bonds[&ModelsValidator::from_slice(&[2; 65])]), 20);
    }

    /// The blessed list is in dependency order, and every contract the table names is really in the
    /// list. Both halves matter: a dependent that is missing from the list is a typo that would
    /// otherwise pass, and a dependency ordered *after* its dependent is the silent failure
    /// `BLESSED_DEPENDENCIES` documents.
    #[test]
    fn blessed_terms_are_ordered_by_dependency() {
        let named = blessed_terms_named("root").expect("the blessed list builds");
        let names: Vec<&str> = named.iter().map(|(name, _)| *name).collect();
        let position = |name: &str| {
            names
                .iter()
                .position(|n| *n == name)
                .unwrap_or_else(|| panic!("{name} is not in the blessed list: {names:?}"))
        };
        for (dependent, dependencies) in BLESSED_DEPENDENCIES {
            for dependency in *dependencies {
                assert!(
                    position(dependency) < position(dependent),
                    "{dependent} resolves {dependency} during its own deploy, so {dependency} must \
                     be installed first (order: {names:?}) — a violation is silent, not an error"
                );
            }
        }
        // The names the table keys on are the ones the list carries: a rename that broke the table
        // would otherwise leave every assertion above vacuous.
        for (dependent, _) in BLESSED_DEPENDENCIES {
            position(dependent);
        }
        assert_eq!(
            names,
            vec![
                "list_ops",
                "non_negative_number",
                "make_mint",
                "kudos",
                "inbox",
                "directory",
                "roll",
                "issue",
                "ballot",
                "chat",
                "group",
                "masterDirectory",
                "extraSlots",
                "memberDirectory",
            ],
            "the install order is part of the chain's identity — a change here is a genesis change"
        );
    }
}
