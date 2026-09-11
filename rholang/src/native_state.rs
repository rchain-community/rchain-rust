//! Native system-contract state (registry / PoS / vault) layered over the rspace native store.
//!
//! The blessed rholang contracts (`Registry.rho`, `Pos.rhox`, `RevVault.rho`, …) re-implemented a
//! `TreeHashMap` trie *in interpreted rholang*; any interpreter bug made the registry silently empty.
//! This module replaces that fragile bootstrap with native Rust state: typed maps over the
//! content-addressed `InMemNativeStore`, exposed through the same `rho:*` protocol by native system
//! processes. The Scala contracts remain a *checklist* of required behavior only.
//!
//! # Dynamic validators
//!
//! The PoS state models the full validator lifecycle natively:
//!
//! * **observer** — any key that is not bonded; any node can run as an observer (no key, or a key
//!   that is not an active validator).
//! * **trusted** — admission into the validator *stakeholder group* ([`PosGenesis::trusted`]). Only
//!   a trusted key may bond; a trusted stakeholder confers trust via `rho:rchain:pos`'s `trust`.
//! * **bonded / pool** — the full bond pool ([`pos_bonds_key`]); a bond is within `[minimum,
//!   maximum]`, deducted from the validator's REV vault.
//! * **active** — the consensus validator set ([`pos_active_key`]): the top
//!   `number_of_active_validators` of the pool by stake (0 = unlimited), recomputed on every
//!   membership change. Consensus (supermajority, finality fringe, bond queries) reads this set.
//! * **withdrawing** — a withdrawal immediately deactivates the validator; the stake is escrowed
//!   until the quarantine deadline ([`pos_withdrawers_key`]) and refunded by `close_block`.
//! * **removed** — `slash`/`untrust` remove the validator and confiscate the stake to the Coop vault.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use rchain_crypto::hash::blake2b256_hash::Blake2b256Hash;
use rchain_crypto::public_key::PublicKey;
use rchain_models::ast::Par;
use rchain_models::validator::Validator;
use rchain_shared::refined::NonNegI64;
use rchain_shared::serialize::Serialize;

use rchain_rspace::native_store::{InMemNativeStore, PREFIX_POS, PREFIX_REGISTRY, PREFIX_VAULT};

use crate::util::rev_address::RevAddress;

/// A public key / validator key is 65 uncompressed secp256k1 bytes.
const VALIDATOR_LEN: usize = 65;
/// The size of a serialized `(Validator, NonNegI64)` bond entry (65-byte key + 8-byte stake).
const BOND_ENTRY_LEN: usize = VALIDATOR_LEN + 8;
/// The size of a serialized `(Validator, i64)` withdrawer entry.
const WITHDRAWER_ENTRY_LEN: usize = VALIDATOR_LEN + 8;
/// The size of a serialized trusted-set entry.
const TRUSTED_ENTRY_LEN: usize = VALIDATOR_LEN;
/// `PosParams` serializes as five little-endian `i64`s.
const PARAMS_LEN: usize = 5 * 8;

// --- Leaf keys ---------------------------------------------------------------

/// Leaf key for the PoS bond *pool* (all bonded validators).
pub fn pos_bonds_key() -> Blake2b256Hash {
    Blake2b256Hash::create(b"pos:bonds")
}

/// Leaf key for the *active* validator set (what consensus uses).
pub fn pos_active_key() -> Blake2b256Hash {
    Blake2b256Hash::create(b"pos:active")
}

/// Leaf key for the trusted validator-stakeholder set (admission gate for bonding).
pub fn pos_trusted_key() -> Blake2b256Hash {
    Blake2b256Hash::create(b"pos:trusted")
}

/// Leaf key for pending withdrawers (`validator → quarantine deadline block`).
pub fn pos_withdrawers_key() -> Blake2b256Hash {
    Blake2b256Hash::create(b"pos:withdrawers")
}

/// Leaf key for the immutable PoS parameters.
pub fn pos_params_key() -> Blake2b256Hash {
    Blake2b256Hash::create(b"pos:params")
}

/// Leaf key for the Coop slashing vault (confiscated stake).
pub fn pos_coop_key() -> Blake2b256Hash {
    Blake2b256Hash::create(b"pos:coop")
}

/// Leaf key for a registry URI (the URI string, hashed).
fn registry_key(uri: &str) -> Blake2b256Hash {
    Blake2b256Hash::create(uri.as_bytes())
}

/// Leaf key for a vault balance (the REV address base58 string, hashed).
fn vault_key(address: &str) -> Blake2b256Hash {
    Blake2b256Hash::create(address.as_bytes())
}

// --- Canonical encoders ------------------------------------------------------

/// Canonically encode a bonds map (sorted by `Validator`, 65-byte key + little-endian stake).
pub fn encode_bonds(bonds: &BTreeMap<Validator, NonNegI64>) -> Vec<u8> {
    let mut out = Vec::with_capacity(bonds.len() * BOND_ENTRY_LEN);
    for (v, stake) in bonds {
        out.extend_from_slice(v.as_bytes());
        out.extend_from_slice(&i64::from(*stake).to_le_bytes());
    }
    out
}

/// Decode a bonds map (inverse of [`encode_bonds`]).
pub fn decode_bonds(bytes: &[u8]) -> Result<BTreeMap<Validator, NonNegI64>, String> {
    if bytes.len() % BOND_ENTRY_LEN != 0 {
        return Err(format!(
            "bonds encoding has {} bytes, not a multiple of {BOND_ENTRY_LEN}",
            bytes.len()
        ));
    }
    let mut out = BTreeMap::new();
    for chunk in bytes.chunks_exact(BOND_ENTRY_LEN) {
        let validator = Validator::from_slice(&chunk[..VALIDATOR_LEN]);
        let stake_bytes: [u8; 8] = chunk[VALIDATOR_LEN..BOND_ENTRY_LEN]
            .try_into()
            .map_err(|_| "bonds encoding: invalid stake length".to_string())?;
        let stake = i64::from_le_bytes(stake_bytes);
        let stake =
            NonNegI64::try_from(stake).map_err(|_| format!("negative bond stake {stake}"))?;
        out.insert(validator, stake);
    }
    Ok(out)
}

/// Canonically encode the trusted stakeholder set (sorted 65-byte keys).
pub fn encode_trusted(trusted: &BTreeSet<Validator>) -> Vec<u8> {
    let mut out = Vec::with_capacity(trusted.len() * TRUSTED_ENTRY_LEN);
    for v in trusted {
        out.extend_from_slice(v.as_bytes());
    }
    out
}

/// Decode the trusted stakeholder set (inverse of [`encode_trusted`]).
pub fn decode_trusted(bytes: &[u8]) -> Result<BTreeSet<Validator>, String> {
    if bytes.len() % TRUSTED_ENTRY_LEN != 0 {
        return Err(format!(
            "trusted encoding has {} bytes, not a multiple of {TRUSTED_ENTRY_LEN}",
            bytes.len()
        ));
    }
    Ok(bytes.chunks_exact(TRUSTED_ENTRY_LEN).map(Validator::from_slice).collect())
}

/// Canonically encode pending withdrawers (`validator → deadline`, sorted, LE deadline).
pub fn encode_withdrawers(withdrawers: &BTreeMap<Validator, i64>) -> Vec<u8> {
    let mut out = Vec::with_capacity(withdrawers.len() * WITHDRAWER_ENTRY_LEN);
    for (v, deadline) in withdrawers {
        out.extend_from_slice(v.as_bytes());
        out.extend_from_slice(&deadline.to_le_bytes());
    }
    out
}

/// Decode pending withdrawers (inverse of [`encode_withdrawers`]).
pub fn decode_withdrawers(bytes: &[u8]) -> Result<BTreeMap<Validator, i64>, String> {
    if bytes.len() % WITHDRAWER_ENTRY_LEN != 0 {
        return Err(format!(
            "withdrawers encoding has {} bytes, not a multiple of {WITHDRAWER_ENTRY_LEN}",
            bytes.len()
        ));
    }
    let mut out = BTreeMap::new();
    for chunk in bytes.chunks_exact(WITHDRAWER_ENTRY_LEN) {
        let validator = Validator::from_slice(&chunk[..VALIDATOR_LEN]);
        let deadline: [u8; 8] = chunk[VALIDATOR_LEN..WITHDRAWER_ENTRY_LEN]
            .try_into()
            .map_err(|_| "withdrawers encoding: invalid deadline length".to_string())?;
        out.insert(validator, i64::from_le_bytes(deadline));
    }
    Ok(out)
}

/// The immutable PoS parameters (installed at genesis, network-wide constants).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PosParams {
    /// Minimum accepted bond (stake) per validator.
    pub minimum_bond: i64,
    /// Maximum accepted bond (stake) per validator.
    pub maximum_bond: i64,
    /// Epoch length in blocks (`<= 1` = every block is an epoch boundary).
    pub epoch_length: i64,
    /// Quarantine length in blocks between a withdrawal request and the refund.
    pub quarantine_length: i64,
    /// Maximum size of the active set, by descending stake (`0` = unlimited).
    pub number_of_active_validators: i64,
}

impl Default for PosParams {
    /// Permissive parameters: no bond bounds, no quarantine, unlimited active set. Used when no
    /// genesis PoS state has been installed (ad-hoc runtimes/tests).
    fn default() -> Self {
        PosParams {
            minimum_bond: 0,
            maximum_bond: i64::MAX,
            epoch_length: 0,
            quarantine_length: 0,
            number_of_active_validators: 0,
        }
    }
}

impl PosParams {
    /// Encode as five little-endian `i64`s (inverse: [`decode_params`]).
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(PARAMS_LEN);
        out.extend_from_slice(&self.minimum_bond.to_le_bytes());
        out.extend_from_slice(&self.maximum_bond.to_le_bytes());
        out.extend_from_slice(&self.epoch_length.to_le_bytes());
        out.extend_from_slice(&self.quarantine_length.to_le_bytes());
        out.extend_from_slice(&self.number_of_active_validators.to_le_bytes());
        out
    }
}

/// Decode [`PosParams`] (inverse of [`PosParams::encode`]).
pub fn decode_params(bytes: &[u8]) -> Result<PosParams, String> {
    if bytes.len() != PARAMS_LEN {
        return Err(format!(
            "params encoding has {} bytes, expected {PARAMS_LEN}",
            bytes.len()
        ));
    }
    let read = |i: usize| -> i64 {
        let mut arr = [0u8; 8];
        arr.copy_from_slice(&bytes[i * 8..i * 8 + 8]);
        i64::from_le_bytes(arr)
    };
    Ok(PosParams {
        minimum_bond: read(0),
        maximum_bond: read(1),
        epoch_length: read(2),
        quarantine_length: read(3),
        number_of_active_validators: read(4),
    })
}

/// The genesis PoS descriptors: the initial bond pool, the initial trusted stakeholder set, and the
/// network parameters. `active` is *derived* from the pool at install time (see [`select_active`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PosGenesis {
    /// The initial bond pool (validator → stake).
    pub bonds: BTreeMap<Validator, NonNegI64>,
    /// The initial trusted stakeholder set. If empty, the initial validators are trusted.
    pub trusted: BTreeSet<Validator>,
    /// The immutable PoS parameters.
    pub params: PosParams,
}

impl PosGenesis {
    /// The initial active set (top-N of the pool by stake).
    pub fn active_bonds(&self) -> BTreeMap<Validator, NonNegI64> {
        select_active(&self.bonds, &BTreeMap::new(), &self.params)
    }
}

/// Select the active validator set from the pool: drop zero-stake and withdrawing validators, sort
/// by descending stake then ascending `Validator` (deterministic), and truncate to
/// `number_of_active_validators` (`0` = unlimited).
pub fn select_active(
    pool: &BTreeMap<Validator, NonNegI64>,
    withdrawers: &BTreeMap<Validator, i64>,
    params: &PosParams,
) -> BTreeMap<Validator, NonNegI64> {
    let mut candidates: Vec<(&Validator, NonNegI64)> = pool
        .iter()
        .filter(|(v, stake)| i64::from(**stake) > 0 && !withdrawers.contains_key(*v))
        .map(|(v, stake)| (v, *stake))
        .collect();
    candidates.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    if params.number_of_active_validators > 0 {
        candidates.truncate(usize::try_from(params.number_of_active_validators).unwrap_or(usize::MAX));
    }
    candidates.into_iter().map(|(v, stake)| (*v, stake)).collect()
}

fn checked_i64(value: i128, what: &str) -> Result<i64, String> {
    i64::try_from(value).map_err(|_| format!("{what} overflow: {value}"))
}

/// The typed native system state, wrapping the shared byte-oriented [`InMemNativeStore`].
#[derive(Clone)]
pub struct NativeSystemState {
    store: Arc<InMemNativeStore>,
}

impl NativeSystemState {
    pub fn new(store: Arc<InMemNativeStore>) -> Self {
        NativeSystemState { store }
    }

    // --- PoS: read/write primitives --------------------------------------

    async fn read_bonds(&self, key: Blake2b256Hash) -> Result<BTreeMap<Validator, NonNegI64>, String> {
        match self.store.get(PREFIX_POS, &key).await? {
            Some(bytes) => decode_bonds(&bytes),
            None => Ok(BTreeMap::new()),
        }
    }

    /// Read the full bond *pool* (all bonded validators).
    pub async fn bonds(&self) -> Result<BTreeMap<Validator, NonNegI64>, String> {
        self.read_bonds(pos_bonds_key()).await
    }

    /// Write the bond pool (appends a native `Put` action).
    pub fn set_bonds(&self, bonds: &BTreeMap<Validator, NonNegI64>) {
        self.store.put(PREFIX_POS, pos_bonds_key(), encode_bonds(bonds));
    }

    /// Read the *active* validator bond map (the consensus set).
    pub async fn active(&self) -> Result<BTreeMap<Validator, NonNegI64>, String> {
        self.read_bonds(pos_active_key()).await
    }

    /// Write the active validator bond map.
    pub fn set_active(&self, active: &BTreeMap<Validator, NonNegI64>) {
        self.store
            .put(PREFIX_POS, pos_active_key(), encode_bonds(active));
    }

    /// The active-validator set (the consensus validator set).
    pub async fn active_validators(&self) -> Result<BTreeSet<Validator>, String> {
        Ok(self.active().await?.into_keys().collect())
    }

    /// Read the trusted stakeholder set.
    pub async fn trusted(&self) -> Result<BTreeSet<Validator>, String> {
        match self.store.get(PREFIX_POS, &pos_trusted_key()).await? {
            Some(bytes) => decode_trusted(&bytes),
            None => Ok(BTreeSet::new()),
        }
    }

    /// Write the trusted stakeholder set.
    pub fn set_trusted(&self, trusted: &BTreeSet<Validator>) {
        self.store
            .put(PREFIX_POS, pos_trusted_key(), encode_trusted(trusted));
    }

    /// Read the pending withdrawers (`validator → quarantine deadline`).
    pub async fn withdrawers(&self) -> Result<BTreeMap<Validator, i64>, String> {
        match self.store.get(PREFIX_POS, &pos_withdrawers_key()).await? {
            Some(bytes) => decode_withdrawers(&bytes),
            None => Ok(BTreeMap::new()),
        }
    }

    /// Write the pending withdrawers.
    pub fn set_withdrawers(&self, withdrawers: &BTreeMap<Validator, i64>) {
        self.store
            .put(PREFIX_POS, pos_withdrawers_key(), encode_withdrawers(withdrawers));
    }

    /// Read the immutable PoS parameters (permissive defaults if absent).
    pub async fn params(&self) -> Result<PosParams, String> {
        match self.store.get(PREFIX_POS, &pos_params_key()).await? {
            Some(bytes) => decode_params(&bytes),
            None => Ok(PosParams::default()),
        }
    }

    /// Write the PoS parameters.
    pub fn set_params(&self, params: &PosParams) {
        self.store
            .put(PREFIX_POS, pos_params_key(), params.encode());
    }

    /// Read the Coop slashing-vault balance.
    pub async fn coop_balance(&self) -> Result<NonNegI64, String> {
        match self.store.get(PREFIX_POS, &pos_coop_key()).await? {
            Some(bytes) => {
                let arr: [u8; 8] = bytes.as_slice().try_into().map_err(|_| {
                    format!("coop balance is {} bytes, expected 8", bytes.len())
                })?;
                NonNegI64::try_from(i64::from_le_bytes(arr))
                    .map_err(|_| "coop balance is negative".to_string())
            }
            None => Ok(NonNegI64::zero()),
        }
    }

    /// Write the Coop slashing-vault balance.
    pub fn set_coop_balance(&self, balance: NonNegI64) {
        self.store.put(
            PREFIX_POS,
            pos_coop_key(),
            i64::from(balance).to_le_bytes().to_vec(),
        );
    }

    /// Install the genesis PoS state: the pool, the trusted set, the parameters, the derived active
    /// set, an empty withdrawer map, and an empty Coop vault. This is the deterministic entry point
    /// shared by genesis creation and genesis replay.
    pub fn install_genesis(&self, genesis: &PosGenesis) {
        let trusted: BTreeSet<Validator> = if genesis.trusted.is_empty() {
            genesis.bonds.keys().copied().collect()
        } else {
            genesis.trusted.clone()
        };
        let withdrawers = BTreeMap::new();
        let active = select_active(&genesis.bonds, &withdrawers, &genesis.params);
        self.set_bonds(&genesis.bonds);
        self.set_active(&active);
        self.set_trusted(&trusted);
        self.set_params(&genesis.params);
        self.set_withdrawers(&withdrawers);
        self.set_coop_balance(NonNegI64::zero());
    }

    // --- PoS: validator lifecycle ----------------------------------------

    /// Bond `amount` stake for `validator` and activate it (dynamic validator lifecycle).
    ///
    /// Rejected when the validator is already bonded/active, is not in the trusted stakeholder set,
    /// the amount is outside `[minimum_bond, maximum_bond]`, or the validator's REV vault cannot
    /// cover the stake.
    pub async fn bond(
        &self,
        validator: &Validator,
        amount: NonNegI64,
        block_number: i64,
    ) -> Result<Result<(), String>, String> {
        let mut pool = self.bonds().await?;
        let active = self.active().await?;
        if pool.contains_key(validator) || active.contains_key(validator) {
            return Ok(Err("Public key is already bonded.".to_string()));
        }
        let trusted = self.trusted().await?;
        if !trusted.contains(validator) {
            return Ok(Err(
                "Validator is not trusted: observer admission is required before bonding."
                    .to_string(),
            ));
        }
        let params = self.params().await?;
        let stake = i64::from(amount);
        if stake < params.minimum_bond {
            return Ok(Err(format!(
                "Bond is less than minimum ({} < {}).",
                stake, params.minimum_bond
            )));
        }
        if stake > params.maximum_bond {
            return Ok(Err(format!(
                "Bond is greater than maximum ({} > {}).",
                stake, params.maximum_bond
            )));
        }
        let address = self.vault_address(validator)?;
        let balance = match self.vault_balance(&address).await? {
            Some(b) => b,
            None => NonNegI64::zero(),
        };
        if i64::from(balance) < stake {
            return Ok(Err(format!(
                "insufficient funds to bond {} (have {})",
                stake,
                i64::from(balance)
            )));
        }
        let new_balance = NonNegI64::try_from(i64::from(balance) - stake)
            .map_err(|e| format!("bond: {e}"))?;
        self.set_vault_balance(&address, new_balance);
        pool.insert(*validator, amount);
        let withdrawers = self.withdrawers().await?;
        let new_active = select_active(&pool, &withdrawers, &params);
        self.set_bonds(&pool);
        self.set_active(&new_active);
        let _ = block_number; // activation is immediate in this model (dynamic validation)
        Ok(Ok(()))
    }

    /// Request withdrawal of a validator's bond. The validator deactivates immediately; the stake is
    /// escrowed until the quarantine deadline and refunded by [`Self::close_block`].
    pub async fn withdraw(
        &self,
        validator: &Validator,
        block_number: i64,
    ) -> Result<Result<(), String>, String> {
        let pool = self.bonds().await?;
        if !pool.contains_key(validator) {
            return Ok(Err("User is not bonded".to_string()));
        }
        let mut withdrawers = self.withdrawers().await?;
        if withdrawers.contains_key(validator) {
            return Ok(Err(
                "Validator has already requested withdrawal".to_string()
            ));
        }
        let params = self.params().await?;
        let deadline = checked_i64(
            i128::from(block_number) + i128::from(params.quarantine_length),
            "withdraw deadline",
        )?;
        withdrawers.insert(*validator, deadline);
        // Deactivate immediately so the validator stops participating in consensus.
        let active = select_active(&pool, &withdrawers, &params);
        self.set_withdrawers(&withdrawers);
        self.set_active(&active);
        Ok(Ok(()))
    }

    /// Close a block: refund any withdrawal whose quarantine has elapsed, and recompute the active
    /// set from the pool (minus withdrawing validators). Called at the end of every block; epoch
    /// bookkeeping is intentionally immediate in this model.
    pub async fn close_block(&self, block_number: i64) -> Result<Result<(), String>, String> {
        let mut pool = self.bonds().await?;
        let mut withdrawers = self.withdrawers().await?;
        let mut refunded: Vec<(Validator, NonNegI64)> = Vec::new();
        for (validator, deadline) in withdrawers.clone() {
            if deadline <= block_number {
                if let Some(stake) = pool.remove(&validator) {
                    refunded.push((validator, stake));
                }
                withdrawers.remove(&validator);
            }
        }
        for (validator, stake) in &refunded {
            let address = self.vault_address(validator)?;
            let balance = match self.vault_balance(&address).await? {
                Some(b) => b,
                None => NonNegI64::zero(),
            };
            let new_balance = NonNegI64::try_from(i64::from(balance) + i64::from(*stake))
                .map_err(|e| format!("withdraw refund: {e}"))?;
            self.set_vault_balance(&address, new_balance);
        }
        let params = self.params().await?;
        let active = select_active(&pool, &withdrawers, &params);
        self.set_bonds(&pool);
        self.set_withdrawers(&withdrawers);
        self.set_active(&active);
        Ok(Ok(()))
    }

    /// Remove a validator and confiscate its stake to the Coop slashing vault (port of the PoS
    /// `slash` behavior). A pending withdrawal is cancelled (the stake is forfeited).
    pub async fn slash(&self, validator: &Validator) -> Result<Result<(), String>, String> {
        let mut pool = self.bonds().await?;
        let mut active = self.active().await?;
        let mut withdrawers = self.withdrawers().await?;
        let stake = pool.remove(validator);
        active.remove(validator);
        withdrawers.remove(validator);
        if let Some(stake) = stake {
            let coop = self.coop_balance().await?;
            let new_coop = NonNegI64::try_from(i64::from(coop) + i64::from(stake))
                .map_err(|e| format!("slash coop: {e}"))?;
            self.set_coop_balance(new_coop);
        }
        self.set_bonds(&pool);
        self.set_active(&active);
        self.set_withdrawers(&withdrawers);
        Ok(Ok(()))
    }

    /// Admit `target` into the trusted stakeholder set. Only an existing trusted stakeholder
    /// (`caller`) may confer trust. This is the observer → validator admission step.
    pub async fn trust(
        &self,
        caller: &Validator,
        target: &Validator,
    ) -> Result<Result<(), String>, String> {
        let mut trusted = self.trusted().await?;
        if !trusted.contains(caller) {
            return Ok(Err(
                "Only a trusted stakeholder can admit validators.".to_string()
            ));
        }
        trusted.insert(*target);
        self.set_trusted(&trusted);
        Ok(Ok(()))
    }

    /// Revoke `target`'s trust. Only an existing trusted stakeholder may revoke, and a stakeholder
    /// cannot revoke itself. If `target` is bonded it is removed and its stake confiscated.
    pub async fn untrust(
        &self,
        caller: &Validator,
        target: &Validator,
    ) -> Result<Result<(), String>, String> {
        let mut trusted = self.trusted().await?;
        if !trusted.contains(caller) {
            return Ok(Err(
                "Only a trusted stakeholder can revoke validators.".to_string()
            ));
        }
        if caller == target {
            return Ok(Err(
                "A stakeholder cannot revoke its own trust.".to_string()
            ));
        }
        trusted.remove(target);
        self.set_trusted(&trusted);
        let bonded = self.bonds().await?.contains_key(target);
        if bonded {
            let _ = self.slash(target).await?;
        }
        Ok(Ok(()))
    }

    // --- Vault ------------------------------------------------------------

    /// Read a vault balance (address is the REV base58 string).
    pub async fn vault_balance(&self, address: &str) -> Result<Option<NonNegI64>, String> {
        match self.store.get(PREFIX_VAULT, &vault_key(address)).await? {
            Some(bytes) => {
                let arr: [u8; 8] = bytes
                    .as_slice()
                    .try_into()
                    .map_err(|_| format!("vault balance for {address} is {} bytes", bytes.len()))?;
                NonNegI64::try_from(i64::from_le_bytes(arr))
                    .map(Some)
                    .map_err(|_| format!("vault balance for {address} is negative"))
            }
            None => Ok(None),
        }
    }

    /// Write a vault balance.
    pub fn set_vault_balance(&self, address: &str, balance: NonNegI64) {
        self.store.put(
            PREFIX_VAULT,
            vault_key(address),
            i64::from(balance).to_le_bytes().to_vec(),
        );
    }

    /// Ensure a vault exists for `address`, creating it with a zero balance if absent (port of the
    /// RevVault `findOrCreate` behavior, simplified: the vault is keyed by REV address, so the
    /// unforgeable-name capability is not modeled).
    pub async fn find_or_create_vault(&self, address: &str) -> Result<(), String> {
        if self.vault_balance(address).await?.is_none() {
            self.set_vault_balance(address, NonNegI64::zero());
        }
        Ok(())
    }

    // --- Registry ------------------------------------------------------------

    /// Look up a registered `Par` by URI.
    pub async fn registry_lookup(&self, uri: &str) -> Result<Option<Par>, String> {
        match self.store.get(PREFIX_REGISTRY, &registry_key(uri)).await? {
            Some(bytes) => Ok(Some(<Par as Serialize<Par>>::decode(&bytes)?)),
            None => Ok(None),
        }
    }

    /// Register a `Par` under `uri`.
    pub fn registry_insert(&self, uri: &str, value: &Par) {
        self.store.put(
            PREFIX_REGISTRY,
            registry_key(uri),
            <Par as Serialize<Par>>::encode(value),
        );
    }

    // --- System-deploy operations (native) -------------------------------

    /// Charge `amount` to the deployer's REV vault (port of the PoS `chargeDeploy` behavior). The
    /// outer `Result` is a platform failure; the inner is the `(Bool, Either)` user result.
    pub async fn pre_charge(
        &self,
        deployer: &PublicKey,
        amount: i64,
    ) -> Result<Result<(), String>, String> {
        if amount == 0 {
            return Ok(Ok(()));
        }
        let address = RevAddress::from_public_key(deployer)
            .ok_or_else(|| "preCharge: invalid deployer public key".to_string())?
            .to_base58();
        let balance = match self.vault_balance(&address).await? {
            Some(b) => b,
            None => NonNegI64::try_from(0).map_err(|e| e.to_string())?,
        };
        if i64::from(balance) < amount {
            return Ok(Err(format!(
                "preCharge: insufficient funds ({} < {amount})",
                i64::from(balance)
            )));
        }
        let new_balance = NonNegI64::try_from(i64::from(balance) - amount)
            .map_err(|e| format!("preCharge: {e}"))?;
        self.set_vault_balance(&address, new_balance);
        Ok(Ok(()))
    }

    /// Refund `amount` (port of the PoS `refundDeploy` behavior). The refund vault is not yet
    /// modeled, so this is a successful no-op for now.
    pub async fn refund(&self, _amount: i64) -> Result<Result<(), String>, String> {
        Ok(Ok(()))
    }

    /// The REV address (base58) of a validator's public key.
    fn vault_address(&self, validator: &Validator) -> Result<String, String> {
        let pk = PublicKey::new(validator.as_bytes().to_vec());
        RevAddress::from_public_key(&pk)
            .map(|a| a.to_base58())
            .ok_or_else(|| "invalid validator public key".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn validator(byte: u8) -> Validator {
        Validator::from_slice(&[byte; 65])
    }

    fn vault_address_of(v: &Validator) -> String {
        RevAddress::from_public_key(&PublicKey::new(v.as_bytes().to_vec()))
            .unwrap()
            .to_base58()
    }

    async fn native_with(
        trusted: &[Validator],
        params: PosParams,
        pool: &[(Validator, i64)],
    ) -> NativeSystemState {
        let native = NativeSystemState::new(Arc::new(InMemNativeStore::empty()));
        let bonds: BTreeMap<Validator, NonNegI64> = pool
            .iter()
            .map(|(v, s)| (*v, NonNegI64::try_from(*s).unwrap()))
            .collect();
        native.install_genesis(&PosGenesis {
            bonds,
            trusted: trusted.iter().copied().collect(),
            params,
        });
        for (v, _) in pool {
            native.set_vault_balance(&vault_address_of(v), NonNegI64::zero());
        }
        native
    }

    async fn fund(native: &NativeSystemState, v: &Validator, amount: i64) {
        let address = vault_address_of(v);
        let balance = native
            .vault_balance(&address)
            .await
            .unwrap()
            .map(i64::from)
            .unwrap_or(0);
        native.set_vault_balance(&address, NonNegI64::try_from(balance + amount).unwrap());
    }

    #[test]
    fn bonds_round_trip() {
        let mut bonds = BTreeMap::new();
        bonds.insert(validator(2), NonNegI64::try_from(20).unwrap());
        bonds.insert(validator(1), NonNegI64::try_from(10).unwrap());
        let encoded = encode_bonds(&bonds);
        assert_eq!(decode_bonds(&encoded).unwrap(), bonds);
    }

    #[test]
    fn decode_bonds_rejects_trailing_bytes() {
        assert!(decode_bonds(&[0u8; 3]).is_err());
    }

    #[test]
    fn trusted_round_trip() {
        let trusted: BTreeSet<Validator> = [validator(1), validator(2)].into_iter().collect();
        assert_eq!(decode_trusted(&encode_trusted(&trusted)).unwrap(), trusted);
    }

    #[test]
    fn params_round_trip() {
        let params = PosParams {
            minimum_bond: 1,
            maximum_bond: 1000,
            epoch_length: 10,
            quarantine_length: 5,
            number_of_active_validators: 3,
        };
        assert_eq!(decode_params(&params.encode()).unwrap(), params);
    }

    #[test]
    fn select_active_orders_by_stake_then_key_and_caps() {
        let pool: BTreeMap<Validator, NonNegI64> = [
            (validator(1), NonNegI64::try_from(10).unwrap()),
            (validator(2), NonNegI64::try_from(30).unwrap()),
            (validator(3), NonNegI64::try_from(20).unwrap()),
        ]
        .into_iter()
        .collect();
        let params = PosParams {
            number_of_active_validators: 2,
            ..PosParams::default()
        };
        let active = select_active(&pool, &BTreeMap::new(), &params);
        assert_eq!(
            active.keys().copied().collect::<Vec<_>>(),
            vec![validator(2), validator(3)]
        );
    }

    #[tokio::test]
    async fn genesis_installs_pool_active_and_trusted() {
        let native = native_with(
            &[validator(1), validator(2)],
            PosParams::default(),
            &[(validator(1), 10), (validator(2), 20)],
        )
        .await;
        assert_eq!(native.bonds().await.unwrap().len(), 2, "pool");
        assert_eq!(native.active().await.unwrap().len(), 2, "active");
        assert_eq!(native.trusted().await.unwrap().len(), 2, "trusted");
        assert_eq!(i64::from(native.coop_balance().await.unwrap()), 0);
    }

    #[tokio::test]
    async fn bond_requires_trust_admission() {
        // v2 is NOT trusted; v1 (trusted) admits it.
        let native = native_with(&[validator(1)], PosParams::default(), &[(validator(1), 10)]).await;
        let v2 = validator(2);
        fund(&native, &v2, 100).await;

        let rejected = native.bond(&v2, NonNegI64::try_from(40).unwrap(), 0).await.unwrap();
        assert!(rejected.is_err(), "untrusted observer must not bond");

        native
            .trust(&validator(1), &v2)
            .await
            .unwrap()
            .unwrap();
        native
            .bond(&v2, NonNegI64::try_from(40).unwrap(), 0)
            .await
            .unwrap()
            .unwrap();
        assert!(native.active().await.unwrap().contains_key(&v2));
    }

    #[tokio::test]
    async fn trust_requires_a_trusted_caller() {
        let native = native_with(&[validator(1)], PosParams::default(), &[(validator(1), 10)]).await;
        let result = native
            .trust(&validator(9), &validator(2))
            .await
            .unwrap();
        assert!(result.is_err(), "a non-stakeholder cannot admit validators");
    }

    #[tokio::test]
    async fn bond_enforces_min_and_max() {
        let params = PosParams {
            minimum_bond: 10,
            maximum_bond: 50,
            ..PosParams::default()
        };
        let native = native_with(&[validator(1), validator(2)], params, &[(validator(1), 10)]).await;
        let v = validator(2);
        fund(&native, &v, 100).await;

        assert!(native
            .bond(&v, NonNegI64::try_from(5).unwrap(), 0)
            .await
            .unwrap()
            .is_err());
        assert!(native
            .bond(&v, NonNegI64::try_from(51).unwrap(), 0)
            .await
            .unwrap()
            .is_err());
        assert!(native
            .bond(&v, NonNegI64::try_from(20).unwrap(), 0)
            .await
            .unwrap()
            .is_ok());
    }

    #[tokio::test]
    async fn bond_deducts_vault_and_activates() {
        let native = native_with(&[validator(1)], PosParams::default(), &[]).await;
        let v = validator(1);
        fund(&native, &v, 100).await;

        native
            .bond(&v, NonNegI64::try_from(40).unwrap(), 0)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(
            i64::from(native.vault_balance(&vault_address_of(&v)).await.unwrap().unwrap()),
            60
        );
        assert_eq!(i64::from(native.bonds().await.unwrap()[&v]), 40);
        assert!(native.active().await.unwrap().contains_key(&v));
    }

    #[tokio::test]
    async fn bond_rejects_already_bonded() {
        let native = native_with(&[validator(1)], PosParams::default(), &[]).await;
        let v = validator(1);
        fund(&native, &v, 100).await;

        native
            .bond(&v, NonNegI64::try_from(40).unwrap(), 0)
            .await
            .unwrap()
            .unwrap();
        let result = native
            .bond(&v, NonNegI64::try_from(10).unwrap(), 0)
            .await
            .unwrap();
        assert!(result.is_err(), "already bonded must be rejected");
    }

    #[tokio::test]
    async fn withdraw_deactivates_then_refunds_after_quarantine() {
        let params = PosParams {
            quarantine_length: 10,
            ..PosParams::default()
        };
        let native = native_with(&[validator(1)], params, &[(validator(1), 40)]).await;
        let v = validator(1);
        native.set_vault_balance(&vault_address_of(&v), NonNegI64::zero());

        // Withdraw at block 5 -> refund at block >= 15.
        native.withdraw(&v, 5).await.unwrap().unwrap();
        assert!(
            !native.active().await.unwrap().contains_key(&v),
            "withdrawing validator is deactivated immediately"
        );

        native.close_block(10).await.unwrap().unwrap();
        assert_eq!(
            i64::from(native.vault_balance(&vault_address_of(&v)).await.unwrap().unwrap()),
            0,
            "stake remains escrowed before the quarantine elapses"
        );

        native.close_block(15).await.unwrap().unwrap();
        assert!(
            !native.bonds().await.unwrap().contains_key(&v),
            "withdrawn validator removed from the pool"
        );
        assert_eq!(
            i64::from(native.vault_balance(&vault_address_of(&v)).await.unwrap().unwrap()),
            40,
            "stake refunded after quarantine"
        );
    }

    #[tokio::test]
    async fn slash_confiscates_to_coop_vault() {
        let native = native_with(&[validator(1), validator(2)], PosParams::default(), &[(validator(1), 10)]).await;
        let v = validator(2);
        fund(&native, &v, 100).await;
        native
            .bond(&v, NonNegI64::try_from(40).unwrap(), 0)
            .await
            .unwrap()
            .unwrap();

        native.slash(&v).await.unwrap().unwrap();

        assert!(!native.bonds().await.unwrap().contains_key(&v));
        assert!(!native.active().await.unwrap().contains_key(&v));
        assert_eq!(
            i64::from(native.coop_balance().await.unwrap()),
            40,
            "slashed stake is confiscated to the Coop vault"
        );
    }

    #[tokio::test]
    async fn untrust_removes_and_confiscates() {
        let native = native_with(&[validator(1), validator(2)], PosParams::default(), &[(validator(1), 10)]).await;
        let v = validator(2);
        fund(&native, &v, 100).await;
        native
            .bond(&v, NonNegI64::try_from(40).unwrap(), 0)
            .await
            .unwrap()
            .unwrap();

        native
            .untrust(&validator(1), &v)
            .await
            .unwrap()
            .unwrap();

        assert!(!native.trusted().await.unwrap().contains(&v));
        assert!(!native.bonds().await.unwrap().contains_key(&v));
        assert_eq!(i64::from(native.coop_balance().await.unwrap()), 40);
    }

    #[tokio::test]
    async fn active_set_respects_the_cap() {
        let params = PosParams {
            number_of_active_validators: 2,
            ..PosParams::default()
        };
        let native = native_with(
            &[validator(1), validator(2), validator(3)],
            params,
            &[(validator(1), 10), (validator(2), 20), (validator(3), 30)],
        )
        .await;

        let active: Vec<Validator> = native.active_validators().await.unwrap().into_iter().collect();
        // The two highest-stake validators (2 and 3); `BTreeSet` iteration is by key order.
        assert_eq!(active, vec![validator(2), validator(3)]);
        assert_eq!(native.bonds().await.unwrap().len(), 3, "pool keeps the third");
    }

    #[tokio::test]
    async fn close_block_is_idempotent_for_active_set() {
        let native = native_with(
            &[validator(1), validator(2)],
            PosParams::default(),
            &[(validator(1), 10), (validator(2), 20)],
        )
        .await;
        native.close_block(1).await.unwrap().unwrap();
        native.close_block(2).await.unwrap().unwrap();
        assert_eq!(native.active_validators().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn pre_charge_deducts_and_rejects_insufficient() {
        let native = NativeSystemState::new(Arc::new(InMemNativeStore::empty()));
        let pk = PublicKey::new(vec![1u8; 65]);
        let addr = RevAddress::from_public_key(&pk).unwrap().to_base58();
        native.set_vault_balance(&addr, NonNegI64::try_from(100).unwrap());

        // Deduct 40 -> 60.
        native.pre_charge(&pk, 40).await.unwrap().unwrap();
        assert_eq!(
            i64::from(native.vault_balance(&addr).await.unwrap().unwrap()),
            60
        );

        // Deducting more than the balance fails via the user-error branch.
        let result = native.pre_charge(&pk, 100).await.unwrap();
        assert!(result.is_err(), "insufficient funds must be rejected");
    }

    #[tokio::test]
    async fn find_or_create_vault_creates_zero_balance_once() {
        let native = NativeSystemState::new(Arc::new(InMemNativeStore::empty()));
        let addr = "someRevAddress".to_string();

        // First call creates the vault with a zero balance.
        native.find_or_create_vault(&addr).await.unwrap();
        assert_eq!(
            native.vault_balance(&addr).await.unwrap(),
            Some(NonNegI64::zero())
        );

        // A subsequent call must not reset an existing balance.
        native.set_vault_balance(&addr, NonNegI64::try_from(42).unwrap());
        native.find_or_create_vault(&addr).await.unwrap();
        assert_eq!(
            i64::from(native.vault_balance(&addr).await.unwrap().unwrap()),
            42
        );
    }
}
