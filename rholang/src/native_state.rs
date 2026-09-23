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
//!
//! # The staking vault
//!
//! Every movement of REV in the PoS mechanism is a transfer between three places: a user's vault, the
//! **staking vault** ([`pos_vault_key`], the contract's `posVault`), and the Coop multisig vault. A
//! bond moves the stake in, a refund moves a deploy's unused phlo back out (the phlo it *did* use
//! stays in and funds the rewards), a slashing moves a stake to the Coop vault, and a withdrawal pays
//! a stake plus its committed rewards back out. The vault's balance is therefore the epoch pot's
//! source, and no step of the mechanism mints: the only credits are a bond, a phlo charge, and the
//! genesis install that funds the vault with exactly the initial bond sum.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use rchain_crypto::hash::blake2b256_hash::Blake2b256Hash;
use rchain_crypto::public_key::PublicKey;
use rchain_models::ast::Par;
use rchain_models::validator::Validator;
use rchain_shared::refined::NonNegI64;
use rchain_shared::serialize::Serialize;

use rchain_rspace::native_store::{
    InMemNativeStore, PREFIX_HTTP, PREFIX_POS, PREFIX_REGISTRY, PREFIX_TXN, PREFIX_VAULT,
};

use crate::util::rev_address::RevAddress;

/// A public key / validator key is 65 uncompressed secp256k1 bytes.
const VALIDATOR_LEN: usize = 65;
/// The size of a serialized `(Validator, NonNegI64)` bond entry (65-byte key + 8-byte stake).
const BOND_ENTRY_LEN: usize = VALIDATOR_LEN + 8;
/// The size of a serialized withdrawal entry (65-byte key + 8-byte bond + 8-byte deadline).
const WITHDRAWER_ENTRY_LEN: usize = VALIDATOR_LEN + 8 + 8;
/// The size of a serialized withdrawal *request* entry (65-byte key + 8-byte deadline).
const PENDING_ENTRY_LEN: usize = VALIDATOR_LEN + 8;
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

/// Leaf key for the withdrawal *requests* — `validator → epoch-boundary deadline`
/// (`Pos.rhox`'s `pendingWithdrawers`). A request stays here until the next epoch boundary, where
/// `close_block` moves it into [`pos_withdrawers_key`] and takes the validator out of the pool.
pub fn pos_pending_withdrawers_key() -> Blake2b256Hash {
    Blake2b256Hash::create(b"pos:pending_withdrawers")
}

/// Leaf key for rewards earned but not yet paid out — `validator → NonNegI64`
/// (`Pos.rhox`'s `committedRewards`). A validator's entry accumulates across epochs and is paid with
/// its bond when its withdrawal is released.
pub fn pos_committed_key() -> Blake2b256Hash {
    Blake2b256Hash::create(b"pos:committed")
}

/// Leaf key for the immutable PoS parameters.
pub fn pos_params_key() -> Blake2b256Hash {
    Blake2b256Hash::create(b"pos:params")
}

/// Leaf key for the Coop slashing vault (confiscated stake).
pub fn pos_coop_key() -> Blake2b256Hash {
    Blake2b256Hash::create(b"pos:coop")
}

/// Leaf key for the PoS *staking vault*: the escrowed bonds, plus the phlo charged from deploys that
/// has not been refunded. This is the contract's `posVault` (`Pos.rhox:161-174`), and it is the pot
/// an epoch distributes — see [`NativeSystemState::close_block`].
pub fn pos_vault_key() -> Blake2b256Hash {
    Blake2b256Hash::create(b"pos:vault")
}

/// Leaf key for the HTTP-result oracle table (`url → (value, captured block)`); RCHIP #54.
pub fn http_records_key() -> Blake2b256Hash {
    Blake2b256Hash::create(b"http:records")
}

/// Leaf key for a registry URI (the URI string, hashed).
fn registry_key(uri: &str) -> Blake2b256Hash {
    Blake2b256Hash::create(uri.as_bytes())
}

/// Leaf key for a vault balance (the REV address base58 string, hashed).
fn vault_key(address: &str) -> Blake2b256Hash {
    Blake2b256Hash::create(address.as_bytes())
}

/// Leaf key for a cross-shard transaction record (the `txn_id` bytes, hashed).
fn txn_key(id: &[u8]) -> Blake2b256Hash {
    Blake2b256Hash::create(id)
}

/// The state of a cross-shard two-phase-commit transaction record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxnState {
    Prepared,
    Committed,
    Aborted,
}

impl TxnState {
    fn discriminant(self) -> u8 {
        match self {
            TxnState::Prepared => 0,
            TxnState::Committed => 1,
            TxnState::Aborted => 2,
        }
    }

    fn from_discriminant(b: u8) -> Option<Self> {
        match b {
            0 => Some(TxnState::Prepared),
            1 => Some(TxnState::Committed),
            2 => Some(TxnState::Aborted),
            _ => None,
        }
    }
}

/// A cross-shard 2PC transaction record: the escrowed REV, its source and destination, and the
/// coordinator key authorized to drive commit/abort.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TxnRecord {
    pub state: TxnState,
    pub coordinator: PublicKey,
    pub amount: NonNegI64,
    pub from: String,
    pub to: String,
}

/// Canonically encode a transaction record: 1-byte state, 65-byte coordinator key, 8-byte LE amount,
/// then two length-prefixed (u32 LE) REV addresses.
fn encode_txn(rec: &TxnRecord) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + VALIDATOR_LEN + 8 + 8 + rec.from.len() + rec.to.len());
    out.push(rec.state.discriminant());
    out.extend_from_slice(rec.coordinator.bytes());
    out.extend_from_slice(&i64::from(rec.amount).to_le_bytes());
    for addr in [&rec.from, &rec.to] {
        out.extend_from_slice(&(addr.len() as u32).to_le_bytes());
        out.extend_from_slice(addr.as_bytes());
    }
    out
}

/// Read a length-prefixed (u32 LE) string at `off`, advancing it.
fn read_len_prefixed(bytes: &[u8], off: &mut usize) -> Result<String, String> {
    let len_end = off.checked_add(4).ok_or("txn record: offset overflow")?;
    if len_end > bytes.len() {
        return Err("txn record truncated".to_string());
    }
    let len_bytes: [u8; 4] = bytes[*off..len_end]
        .try_into()
        .map_err(|_| "txn record: invalid length prefix".to_string())?;
    let len = usize::try_from(u32::from_le_bytes(len_bytes))
        .map_err(|_| "txn record: length does not fit usize".to_string())?;
    *off = len_end;
    let s_end = off.checked_add(len).ok_or("txn record: length overflow")?;
    if s_end > bytes.len() {
        return Err("txn record truncated".to_string());
    }
    let s = String::from_utf8(bytes[*off..s_end].to_vec())
        .map_err(|_| "txn record: non-UTF8 address".to_string())?;
    *off = s_end;
    Ok(s)
}

/// Decode a transaction record (inverse of [`encode_txn`]).
fn decode_txn(bytes: &[u8]) -> Result<TxnRecord, String> {
    if bytes.len() < 1 + VALIDATOR_LEN + 8 + 4 + 4 {
        return Err("txn record too short".to_string());
    }
    let state = TxnState::from_discriminant(bytes[0])
        .ok_or_else(|| format!("txn record: unknown state {}", bytes[0]))?;
    let coordinator = PublicKey::new(bytes[1..1 + VALIDATOR_LEN].to_vec());
    let amount_bytes: [u8; 8] = bytes[1 + VALIDATOR_LEN..1 + VALIDATOR_LEN + 8]
        .try_into()
        .map_err(|_| "txn record: invalid amount".to_string())?;
    let amount = NonNegI64::try_from(i64::from_le_bytes(amount_bytes))
        .map_err(|_| "txn record: negative amount".to_string())?;
    let mut off = 1 + VALIDATOR_LEN + 8;
    let from = read_len_prefixed(bytes, &mut off)?;
    let to = read_len_prefixed(bytes, &mut off)?;
    if off != bytes.len() {
        return Err("txn record has trailing bytes".to_string());
    }
    Ok(TxnRecord {
        state,
        coordinator,
        amount,
        from,
        to,
    })
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

// --- HTTP-result oracle (RCHIP #54) -----------------------------------------

/// A recorded HTTP value: the captured body and the block height at which it was captured.
pub type HttpRecord = (String, i64);

/// Canonically encode the HTTP-record table (sorted by URL; length-prefixed strings and a
/// little-endian capture block).
pub fn encode_http_records(records: &BTreeMap<String, HttpRecord>) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(records.len() as u32).to_le_bytes());
    for (url, (value, block_number)) in records {
        out.extend_from_slice(&(url.len() as u32).to_le_bytes());
        out.extend_from_slice(url.as_bytes());
        out.extend_from_slice(&(value.len() as u32).to_le_bytes());
        out.extend_from_slice(value.as_bytes());
        out.extend_from_slice(&block_number.to_le_bytes());
    }
    out
}

/// Decode the HTTP-record table (inverse of [`encode_http_records`]).
pub fn decode_http_records(bytes: &[u8]) -> Result<BTreeMap<String, HttpRecord>, String> {
    if bytes.len() < 4 {
        return Err("http records: truncated count".to_string());
    }
    let count_bytes: [u8; 4] = bytes[..4]
        .try_into()
        .map_err(|_| "http records: invalid count".to_string())?;
    let count = usize::try_from(u32::from_le_bytes(count_bytes))
        .map_err(|_| "http records: count does not fit usize".to_string())?;
    let mut out = BTreeMap::new();
    let mut off = 4usize;
    for _ in 0..count {
        let url = read_len_prefixed(bytes, &mut off)?;
        let value = read_len_prefixed(bytes, &mut off)?;
        let bn_end = off.checked_add(8).ok_or("http records: offset overflow")?;
        if bn_end > bytes.len() {
            return Err("http records: truncated block number".to_string());
        }
        let block_bytes: [u8; 8] = bytes[off..bn_end]
            .try_into()
            .map_err(|_| "http records: invalid block number".to_string())?;
        off = bn_end;
        out.insert(url, (value, i64::from_le_bytes(block_bytes)));
    }
    if off != bytes.len() {
        return Err("http records: trailing bytes".to_string());
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
    Ok(bytes
        .chunks_exact(TRUSTED_ENTRY_LEN)
        .map(Validator::from_slice)
        .collect())
}

/// Canonically encode pending withdrawers (`validator → epoch-boundary deadline`, sorted, LE
/// deadline).
pub fn encode_pending_withdrawers(pending: &BTreeMap<Validator, i64>) -> Vec<u8> {
    let mut out = Vec::with_capacity(pending.len() * PENDING_ENTRY_LEN);
    for (v, deadline) in pending {
        out.extend_from_slice(v.as_bytes());
        out.extend_from_slice(&deadline.to_le_bytes());
    }
    out
}

/// Decode pending withdrawers (inverse of [`encode_pending_withdrawers`]).
pub fn decode_pending_withdrawers(bytes: &[u8]) -> Result<BTreeMap<Validator, i64>, String> {
    if bytes.len() % PENDING_ENTRY_LEN != 0 {
        return Err(format!(
            "pending withdrawers encoding has {} bytes, not a multiple of {PENDING_ENTRY_LEN}",
            bytes.len()
        ));
    }
    let mut out = BTreeMap::new();
    for chunk in bytes.chunks_exact(PENDING_ENTRY_LEN) {
        let validator = Validator::from_slice(&chunk[..VALIDATOR_LEN]);
        let deadline: [u8; 8] = chunk[VALIDATOR_LEN..PENDING_ENTRY_LEN]
            .try_into()
            .map_err(|_| "pending withdrawers encoding: invalid deadline length".to_string())?;
        out.insert(validator, i64::from_le_bytes(deadline));
    }
    Ok(out)
}

/// An escrowed withdrawal (`Pos.rhox`'s `withdrawers` entry): the bond taken out of the pool at the
/// epoch boundary that moved the validator out, and the block at which it may be paid.
///
/// The stored amount is the **bond** alone: `movePendingWithdrawer` sets the entry from
/// `allBonds.get(pk)` (`Pos.rhox:582`), and the reward is added at payment time from the committed
/// map (`removeQuarantinedWithdrawers`, `:604` — `bonds + committedRewards.getOrElse(pk, 0)`). The
/// contract's header comment describes the pair as `(original bond + reward, quantinue length)`
/// (`:189-192`), which is what the payee *receives*; the code at `:582` is what is stored, and the
/// difference is what makes the epoch's last reward reach a validator that has already left the pool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Withdrawal {
    /// The stake escrowed out of the pool when the validator was moved here.
    pub bond: NonNegI64,
    /// The block at which the claim may be paid (`currentBlockNumber >= deadline`).
    pub deadline: i64,
}

/// Canonically encode withdrawals: 65-byte validator, LE bond, LE deadline.
pub fn encode_withdrawers(withdrawers: &BTreeMap<Validator, Withdrawal>) -> Vec<u8> {
    let mut out = Vec::with_capacity(withdrawers.len() * WITHDRAWER_ENTRY_LEN);
    for (v, w) in withdrawers {
        out.extend_from_slice(v.as_bytes());
        out.extend_from_slice(&i64::from(w.bond).to_le_bytes());
        out.extend_from_slice(&w.deadline.to_le_bytes());
    }
    out
}

/// Decode withdrawals (inverse of [`encode_withdrawers`]).
pub fn decode_withdrawers(bytes: &[u8]) -> Result<BTreeMap<Validator, Withdrawal>, String> {
    if bytes.len() % WITHDRAWER_ENTRY_LEN != 0 {
        return Err(format!(
            "withdrawers encoding has {} bytes, not a multiple of {WITHDRAWER_ENTRY_LEN}",
            bytes.len()
        ));
    }
    let mut out = BTreeMap::new();
    for chunk in bytes.chunks_exact(WITHDRAWER_ENTRY_LEN) {
        let validator = Validator::from_slice(&chunk[..VALIDATOR_LEN]);
        let bond: [u8; 8] = chunk[VALIDATOR_LEN..VALIDATOR_LEN + 8]
            .try_into()
            .map_err(|_| "withdrawers encoding: invalid bond length".to_string())?;
        let bond = NonNegI64::try_from(i64::from_le_bytes(bond))
            .map_err(|e| format!("withdrawers encoding: {e}"))?;
        let deadline: [u8; 8] = chunk[VALIDATOR_LEN + 8..WITHDRAWER_ENTRY_LEN]
            .try_into()
            .map_err(|_| "withdrawers encoding: invalid deadline length".to_string())?;
        out.insert(
            validator,
            Withdrawal {
                bond,
                deadline: i64::from_le_bytes(deadline),
            },
        );
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
        select_active(&self.bonds, &BTreeMap::<Validator, ()>::new(), &self.params)
    }

    /// The initial bond sum — the amount the staking vault is created with (`Pos.rhox:167-174`:
    /// `ListOps!("fold", $$initialBonds$$.toList(), 0, *sumFromPair, …)` and then
    /// `createWithBalance(posDeployerRevAddress, bondSum)` before the transfer into the PoS vault).
    /// A sum that does not fit an `i64` is a genesis that cannot be installed, not a genesis to
    /// clamp.
    pub fn bond_sum(&self) -> Result<NonNegI64, String> {
        let sum: i128 = self.bonds.values().map(|s| i128::from(i64::from(*s))).sum();
        let sum = checked_i64(sum, "genesis bond sum")?;
        NonNegI64::try_from(sum).map_err(|_| format!("genesis bond sum is negative: {sum}"))
    }
}

/// Select the active validator set from the pool: drop zero-stake and withdrawing validators, sort
/// by descending stake then ascending `Validator` (deterministic), and truncate to
/// `number_of_active_validators` (`0` = unlimited).
///
/// Generic over what the withdrawal map holds, because only its key set matters here and the port has
/// two of them: the staged requests (`validator → deadline`, `pendingWithdrawers`) and the claims
/// (`validator → Withdrawal`, `withdrawers`).
///
/// **This is `pickActiveValidators` with a different selection rule** — the contract takes the first
/// `$$numberOfActiveValidators$$` entries of the bonds map in *key* order (`Pos.rhox:718-726`, whose
/// own TODO marks it a placeholder for a random selection), and the port takes the highest-staked.
/// Registered in `spec/AUDIT.md` §6.
pub fn select_active<V>(
    pool: &BTreeMap<Validator, NonNegI64>,
    withdrawers: &BTreeMap<Validator, V>,
    params: &PosParams,
) -> BTreeMap<Validator, NonNegI64> {
    let mut candidates: Vec<(&Validator, NonNegI64)> = pool
        .iter()
        .filter(|(v, stake)| i64::from(**stake) > 0 && !withdrawers.contains_key(*v))
        .map(|(v, stake)| (v, *stake))
        .collect();
    candidates.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    if params.number_of_active_validators > 0 {
        candidates
            .truncate(usize::try_from(params.number_of_active_validators).unwrap_or(usize::MAX));
    }
    candidates
        .into_iter()
        .map(|(v, stake)| (*v, stake))
        .collect()
}

fn checked_i64(value: i128, what: &str) -> Result<i64, String> {
    i64::try_from(value).map_err(|_| format!("{what} overflow: {value}"))
}

/// `balance + delta` as a `NonNegI64`, with both the `i64` range and the sign checked. The single
/// place the port's balance arithmetic happens, so a balance can neither wrap nor go negative
/// silently. `delta` is an `i64` to keep the debit direction explicit at the call sites.
fn balance_plus(balance: NonNegI64, delta: i64, what: &str) -> Result<NonNegI64, String> {
    let sum = checked_i64(i128::from(i64::from(balance)) + i128::from(delta), what)?;
    NonNegI64::try_from(sum).map_err(|_| format!("{what} would become negative: {sum}"))
}

/// The epoch length the port **divides by**.
///
/// The contract divides by `$$epochLength$$` directly, in both of law 44's places — the boundary test
/// (`Pos.rhox:517`, `blockNumber % $$epochLength$$`) and the withdrawal deadline (`:381`,
/// `blockNumber / $$epochLength$$`). With `epochLength == 0` both fault. The port's default parameters
/// are permissive (`epoch_length: 0`, see [`PosParams::default`]), and the meaning of a zero epoch
/// length is "every block is an epoch boundary" — which is what `epochLength == 1` means to the
/// contract. So the divisor is `max(epoch_length, 1)`, which agrees with the contract for every
/// `epoch_length >= 1` and turns the unrepresentable zero into the one-block epoch it must mean.
fn epoch_divisor(params: &PosParams) -> i64 {
    if params.epoch_length <= 0 {
        1
    } else {
        params.epoch_length
    }
}

/// Is `block_number` an epoch boundary (`Pos.rhox:517`)? At a non-boundary the contract does
/// **nothing at all** — no reward, no activation, no payment.
fn is_epoch_boundary(params: &PosParams, block_number: i64) -> bool {
    block_number % epoch_divisor(params) == 0
}

/// The epoch's distributable pot: the staking vault less every outstanding claim on it — the bonded
/// pool, the escrowed withdrawals, and the rewards already committed but not yet paid
/// (`Pos.rhox:249`: `posBalance - totalBond - totalWithdraw - totalCommittedRewards`).
///
/// **Floored at zero.** The Scala computes this in `Long`, so a vault that cannot cover the
/// outstanding claims yields a *negative* pot and therefore negative rewards; this port's balances are
/// `NonNegI64` and a negative reward is not a value it will model. Through the protocol the floor is
/// unreachable — every debit is bounded by the credit that funded it, so the vault always covers its
/// claims — which is why this is a defensive floor and not a silent clamp, and why
/// `a_drafted_vault_floors_the_pot_instead_of_wrapping` has to build the diverged state by hand.
///
/// One consequence of the formula, and not a bug in it: the dust an epoch leaves un-distributed is
/// **not lost**. It stays in the pot, so the next epoch distributes it along with its own phlo.
fn epoch_pot(
    vault: NonNegI64,
    bonds: &BTreeMap<Validator, NonNegI64>,
    withdrawers: &BTreeMap<Validator, Withdrawal>,
    committed: &BTreeMap<Validator, NonNegI64>,
) -> Result<i64, String> {
    let claims: i128 = bonds
        .values()
        .map(|s| i128::from(i64::from(*s)))
        .sum::<i128>()
        + withdrawers
            .values()
            .map(|w| i128::from(i64::from(w.bond)))
            .sum::<i128>()
        + committed
            .values()
            .map(|s| i128::from(i64::from(*s)))
            .sum::<i128>();
    checked_i64((i128::from(i64::from(vault)) - claims).max(0), "epoch pot")
}

/// One active validator's share of the pot (`Pos.rhox:249`):
///
/// ```text
/// pot * (bond / minimumBond) / (activeBonds / minimumBond)
/// ```
///
/// two integer divisions, which is why the shares do not add up to the pot: see
/// `Rchain.sum_rewards_le_pot` in `spec/Rchain/Pos.lean`, whose statement is the inequality and whose
/// `the_dust_is_real` is the case where it is strict.
///
/// **Zero where the contract's formula is undefined.** `minimumBond == 0`, or a normaliser of zero
/// (`activeBonds < minimumBond`), makes the Scala divide by zero — which faults the `closeBlock`
/// deploy rather than producing a value. The port's parameters are permissive by default
/// (`minimum_bond: 0`), so a fault is not a rule it can copy, and zero is the only value that leaves
/// the epoch total. That is the same case the Lean model leaves as a hypothesis
/// (`hD : 0 < activeBonds / minimumBond`): the model states the theorem for the defined case, the port
/// pays nothing in the undefined one. `an_epoch_with_a_zero_normaliser_pays_nothing` pins it.
fn epoch_reward(pot: i64, minimum_bond: i64, active_bonds: i64, bond: i64) -> Result<i64, String> {
    if minimum_bond <= 0 {
        return Ok(0);
    }
    let normaliser = active_bonds / minimum_bond;
    if normaliser <= 0 {
        return Ok(0);
    }
    // The product is exact in `i128` rather than wrapped as the Scala's `Long` multiplication would
    // be, and the quotient is checked rather than clamped: `active_bonds / minimum_bond` is at least
    // `bond / minimum_bond` for an active validator, so the model's lemma bounds the quotient by the
    // pot — the check cannot fire, and if the reasoning is wrong it is an error rather than a silent
    // clamp.
    let scaled = i128::from(pot) * i128::from(bond / minimum_bond);
    checked_i64(scaled / i128::from(normaliser), "epoch reward")
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

    async fn read_bonds(
        &self,
        key: Blake2b256Hash,
    ) -> Result<BTreeMap<Validator, NonNegI64>, String> {
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
        self.store
            .put(PREFIX_POS, pos_bonds_key(), encode_bonds(bonds));
    }

    // --- HTTP-result oracle (RCHIP #54) -----------------------------------------------------

    /// Read the whole HTTP-record table (`url → (value, captured block)`).
    pub async fn http_records(&self) -> Result<BTreeMap<String, HttpRecord>, String> {
        match self.store.get(PREFIX_HTTP, &http_records_key()).await? {
            Some(bytes) => decode_http_records(&bytes),
            None => Ok(BTreeMap::new()),
        }
    }

    /// Read a recorded HTTP value (`None` if the URL was never captured).
    pub async fn http_record(&self, url: &str) -> Result<Option<HttpRecord>, String> {
        Ok(self.http_records().await?.get(url).cloned())
    }

    /// Capture `value` for `url` — **first writer wins**. Returns `true` if this call recorded the
    /// value, `false` if a record already existed (which is left untouched). Recording is a pure
    /// function of the deploy, so replay is deterministic; later deploys compare against the record
    /// rather than re-fetching, which is what makes an HTTP value usable under consensus.
    pub async fn record_http(
        &self,
        url: &str,
        value: &str,
        block_number: i64,
    ) -> Result<bool, String> {
        let mut records = self.http_records().await?;
        if records.contains_key(url) {
            return Ok(false);
        }
        records.insert(url.to_string(), (value.to_string(), block_number));
        self.store.put(
            PREFIX_HTTP,
            http_records_key(),
            encode_http_records(&records),
        );
        Ok(true)
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
    pub async fn withdrawers(&self) -> Result<BTreeMap<Validator, Withdrawal>, String> {
        match self.store.get(PREFIX_POS, &pos_withdrawers_key()).await? {
            Some(bytes) => decode_withdrawers(&bytes),
            None => Ok(BTreeMap::new()),
        }
    }

    /// Write the pending withdrawers.
    pub fn set_withdrawers(&self, withdrawers: &BTreeMap<Validator, Withdrawal>) {
        self.store.put(
            PREFIX_POS,
            pos_withdrawers_key(),
            encode_withdrawers(withdrawers),
        );
    }

    /// Read the withdrawal *requests* (`validator → epoch-boundary deadline`; the contract's
    /// `pendingWithdrawers`).
    pub async fn pending_withdrawers(&self) -> Result<BTreeMap<Validator, i64>, String> {
        match self
            .store
            .get(PREFIX_POS, &pos_pending_withdrawers_key())
            .await?
        {
            Some(bytes) => decode_pending_withdrawers(&bytes),
            None => Ok(BTreeMap::new()),
        }
    }

    /// Write the withdrawal requests.
    pub fn set_pending_withdrawers(&self, pending: &BTreeMap<Validator, i64>) {
        self.store.put(
            PREFIX_POS,
            pos_pending_withdrawers_key(),
            encode_pending_withdrawers(pending),
        );
    }

    /// Read the committed rewards (`validator → NonNegI64`).
    pub async fn committed_rewards(&self) -> Result<BTreeMap<Validator, NonNegI64>, String> {
        match self.store.get(PREFIX_POS, &pos_committed_key()).await? {
            Some(bytes) => decode_bonds(&bytes),
            None => Ok(BTreeMap::new()),
        }
    }

    /// Write the committed rewards (the same canonical encoding as the bond maps).
    pub fn set_committed_rewards(&self, committed: &BTreeMap<Validator, NonNegI64>) {
        self.store
            .put(PREFIX_POS, pos_committed_key(), encode_bonds(committed));
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
        self.read_balance(PREFIX_POS, &pos_coop_key(), "coop balance")
            .await
    }

    /// Write the Coop slashing-vault balance.
    pub fn set_coop_balance(&self, balance: NonNegI64) {
        self.write_balance(PREFIX_POS, &pos_coop_key(), balance);
    }

    /// Read the PoS staking-vault balance (the epoch pot's source; `Pos.rhox:203`'s
    /// `posVault!("balance", …)`).
    pub async fn pos_vault_balance(&self) -> Result<NonNegI64, String> {
        self.read_balance(PREFIX_POS, &pos_vault_key(), "pos vault balance")
            .await
    }

    /// Write the PoS staking-vault balance.
    pub fn set_pos_vault_balance(&self, balance: NonNegI64) {
        self.write_balance(PREFIX_POS, &pos_vault_key(), balance);
    }

    /// Read a leaf holding a single little-endian `NonNegI64` — a balance. An absent leaf is zero,
    /// which is how the store distinguishes "never written" from "written as zero".
    async fn read_balance(
        &self,
        prefix: u8,
        key: &Blake2b256Hash,
        what: &str,
    ) -> Result<NonNegI64, String> {
        match self.store.get(prefix, key).await? {
            Some(bytes) => {
                let arr: [u8; 8] = bytes
                    .as_slice()
                    .try_into()
                    .map_err(|_| format!("{what} is {} bytes, expected 8", bytes.len()))?;
                NonNegI64::try_from(i64::from_le_bytes(arr))
                    .map_err(|_| format!("{what} is negative"))
            }
            None => Ok(NonNegI64::zero()),
        }
    }

    /// Write a leaf holding a single little-endian `NonNegI64`.
    fn write_balance(&self, prefix: u8, key: &Blake2b256Hash, balance: NonNegI64) {
        self.store
            .put(prefix, *key, i64::from(balance).to_le_bytes().to_vec());
    }

    /// Credit `amount` to the staking vault (a bond's stake, or a deploy's phlo charge).
    pub async fn credit_pos_vault(&self, amount: i64) -> Result<(), String> {
        if amount <= 0 {
            return Ok(());
        }
        let balance = self.pos_vault_balance().await?;
        self.set_pos_vault_balance(balance_plus(balance, amount, "pos vault credit")?);
        Ok(())
    }

    /// Debit `amount` from the staking vault (a withdrawal's bond + rewards, a refund, a slashing).
    ///
    /// **A short vault is a platform error, not a user error.** Every debit is a transfer whose
    /// source is guaranteed by the accounting: a refund is bounded by the pre-charge that funded it,
    /// a withdrawal by the bond it escrowed, a slash by the stake it confiscated. So a vault that
    /// cannot cover the transfer means the ledger has already diverged, and failing the deploy
    /// loudly is the only honest response — the Scala's `payWithdrawer` has a
    /// `// FIXME fix transfer in failure case` here and removes the withdrawer from the maps even
    /// when the transfer failed, which loses the bond. This port refuses instead.
    pub async fn debit_pos_vault(&self, amount: i64) -> Result<(), String> {
        if amount <= 0 {
            return Ok(());
        }
        let balance = self.pos_vault_balance().await?;
        self.set_pos_vault_balance(balance_plus(balance, -amount, "pos vault debit")?);
        Ok(())
    }

    /// Install the genesis PoS state: the pool, the trusted set, the parameters, the derived active
    /// set, empty withdrawal/commitment maps, an empty Coop vault, and a staking vault holding
    /// exactly the initial bond sum. This is the deterministic entry point shared by genesis creation
    /// and genesis replay.
    ///
    /// The vault is funded with the bond sum and nothing more, so the *pot* an epoch distributes
    /// (`vault − bonded − withdrawers − committed rewards`) is **zero** at genesis: the rewards an
    /// epoch pays come from the phlo of the deploys since the last boundary, not from the bonds.
    ///
    /// The genesis active set is taken directly from the pool (`Pos.rhox:178`,
    /// `pickActiveValidators!($$initialBonds$$.toList(), *initialActiveCh)`), because genesis *is* a
    /// boundary.
    pub fn install_genesis(&self, genesis: &PosGenesis) -> Result<(), String> {
        // Computed before anything is written, so a genesis whose bond sum does not fit an `i64`
        // leaves the store untouched rather than half-installed.
        let bond_sum = genesis.bond_sum()?;
        let trusted: BTreeSet<Validator> = if genesis.trusted.is_empty() {
            genesis.bonds.keys().copied().collect()
        } else {
            genesis.trusted.clone()
        };
        let withdrawers: BTreeMap<Validator, Withdrawal> = BTreeMap::new();
        let active = select_active(&genesis.bonds, &withdrawers, &genesis.params);
        self.set_bonds(&genesis.bonds);
        self.set_active(&active);
        self.set_trusted(&trusted);
        self.set_params(&genesis.params);
        self.set_withdrawers(&withdrawers);
        self.set_pending_withdrawers(&BTreeMap::new());
        self.set_committed_rewards(&BTreeMap::new());
        self.set_coop_balance(NonNegI64::zero());
        self.set_pos_vault_balance(bond_sum);
        Ok(())
    }

    // --- PoS: validator lifecycle ----------------------------------------

    /// Bond `amount` stake for `validator` (port of the PoS `bond`, `Pos.rhox:330-362`).
    ///
    /// The stake moves out of the validator's vault into the staking vault and into the pool. The
    /// validator becomes **active at the next epoch boundary**, not here: the contract's `bond` only
    /// writes `allBonds` (`:355`), and `activeValidators` is recomputed by `pickActiveValidators`
    /// inside `closeBlock` (`:546`). With the permissive default parameters that is the same block's
    /// `close_block`, so a bonded validator is active by the block's post-state; with an epoch length
    /// greater than one it waits, exactly as the contract does.
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
        let new_balance =
            NonNegI64::try_from(i64::from(balance) - stake).map_err(|e| format!("bond: {e}"))?;
        self.set_vault_balance(&address, new_balance);
        // The stake moves from the validator's vault into the staking vault (`Pos.rhox:392-410`'s
        // `deposit!(deployerId, amount, posVaultAddr)`). Crediting the destination is what makes the
        // later reward and refund transfers possible at all: without it the vault would hold nothing
        // to pay out of, and the epoch would distribute a pot that does not exist.
        self.credit_pos_vault(stake).await?;
        pool.insert(*validator, amount);
        // The pool only. Activation is the epoch boundary's (`close_block`'s
        // `pickActiveValidators`, `Pos.rhox:546`), which is why this does not touch `pos:active`:
        // promoting a validator mid-epoch would change the consensus set inside an epoch, which is
        // the rule law 44 is about.
        self.set_bonds(&pool);
        let _ = block_number; // the contract's `bond` reads no block data at all
        Ok(Ok(()))
    }

    /// Request withdrawal of a validator's bond (port of the PoS `withdraw`, `Pos.rhox:363-387`).
    ///
    /// The request only **stages** the withdrawal. The validator stays bonded and stays in the active
    /// set, earning, until the next epoch boundary — the contract's `withdraw` writes
    /// `pendingWithdrawers` and nothing else. The deadline it records is
    /// `quarantineLength + epochLength * (1 + blockNumber / epochLength)` (`:381`): the end of the
    /// epoch *after* this one, plus the quarantine, so the money is payable at the first boundary at
    /// which the quarantine has elapsed.
    ///
    /// A repeat request re-stages with a fresh deadline, which is what the contract's unconditional
    /// `.set` does (`:378-381`). Before this, the port deactivated the validator immediately and
    /// refused a second request: neither is the contract's rule — the first is the deviation law 47
    /// is about, and the second refused a request the contract accepts.
    pub async fn withdraw(
        &self,
        validator: &Validator,
        block_number: i64,
    ) -> Result<Result<(), String>, String> {
        let pool = self.bonds().await?;
        if !pool.contains_key(validator) {
            return Ok(Err("User is not bonded".to_string()));
        }
        let params = self.params().await?;
        let deadline = checked_i64(
            i128::from(params.quarantine_length)
                + i128::from(params.epoch_length)
                    * (1 + i128::from(block_number) / i128::from(epoch_divisor(&params))),
            "withdraw deadline",
        )?;
        let mut pending = self.pending_withdrawers().await?;
        pending.insert(*validator, deadline);
        self.set_pending_withdrawers(&pending);
        Ok(Ok(()))
    }

    /// Close a block — the **epoch transition**, which happens only at an epoch boundary
    /// (`Pos.rhox:517`: if `blockNumber % $$epochLength$$ != 0`, nothing happens at all).
    ///
    /// At a boundary the contract runs one sequence, and the order carries the meaning
    /// (`Pos.rhox:528-551`):
    ///
    /// 1. **reward** — every pooled validator's share of the pot, computed from the state *as it
    ///    stands*, is added to the committed map (`getCurrentEpochRewards`, `:241-256`, then
    ///    `commitCurrentEpochRewards`, `:568-576`). A validator that is not in the active set gets an
    ///    entry of zero, so the committed map has a key for every pooled validator.
    /// 2. **move** — each staged withdrawal becomes a claim: `withdrawers[pk] = (pool[pk], deadline)`
    ///    and the validator leaves the pool (`movePendingWithdrawer`, `:577-587`). This is what a
    ///    request staged during the epoch waited for, and because step 1 ran first, the epoch it
    ///    spent its last blocks in still paid it.
    /// 3. **pay** — every claim whose deadline has passed is paid `bond + committed[pk]` out of the
    ///    staking vault, and both entries are removed (`removeQuarantinedWithdrawers`, `:592-621`).
    /// 4. **re-select** the active set from what is left of the pool (`pickActiveValidators`, `:546`)
    ///    — the only place a bonded validator becomes active, and the only place a validator that was
    ///    below the active cap can be promoted into it.
    ///
    /// Rewards are computed before steps 2–4 change anything, so the sequence is **not idempotent**:
    /// a second call at the same height would distribute the pot again (which is the previous epoch's
    /// dust, since the committed claims now cover the rest). The system deploy calls it once per
    /// block.
    pub async fn close_block(&self, block_number: i64) -> Result<Result<(), String>, String> {
        let params = self.params().await?;
        if !is_epoch_boundary(&params, block_number) {
            // `Pos.rhox:519`: "Epoch change does not occur." Nothing is written — no reward, no
            // activation, no payment. The active set is not even recomputed, which is faithful: the
            // contract's membership changes take effect at boundaries, and every change (bond's
            // insertion, slash's removal) is applied to the *pool* immediately where the contract
            // applies it.
            return Ok(Ok(()));
        }

        let mut pool = self.bonds().await?;
        let mut withdrawers = self.withdrawers().await?;
        let mut pending = self.pending_withdrawers().await?;
        let mut committed = self.committed_rewards().await?;

        // 1. The epoch's rewards, from the state as it stands.
        let rewards = self
            .epoch_rewards(&pool, &withdrawers, &committed, &params)
            .await?;
        for (validator, reward) in &rewards {
            let carried = committed
                .get(validator)
                .copied()
                .unwrap_or(NonNegI64::zero());
            committed.insert(
                *validator,
                balance_plus(carried, i64::from(*reward), "committed reward")?,
            );
        }

        // 2. Staged withdrawals become claims against the vault.
        for (validator, deadline) in std::mem::take(&mut pending) {
            if let Some(bond) = pool.remove(&validator) {
                withdrawers.insert(validator, Withdrawal { bond, deadline });
            }
            // A pending validator that is no longer in the pool — slashed, or untrusted — has no
            // stake to claim, and its entry is dropped with it. The contract zeroes a slashed
            // validator's bond but leaves it in `pendingWithdrawers` (`Pos.rhox:491`), so at the next
            // boundary it lands in `withdrawers` with a zero amount and is never paid; dropping it
            // here is the same payable outcome without the permanent tombstone. Registered in
            // `spec/AUDIT.md` §6.
        }

        // 3. Pay the claims whose quarantine has elapsed.
        let due: Vec<Validator> = withdrawers
            .iter()
            .filter(|(_, w)| w.deadline <= block_number)
            .map(|(v, _)| *v)
            .collect();
        for validator in due {
            let Some(claim) = withdrawers.remove(&validator) else {
                continue;
            };
            let reward = committed.remove(&validator).unwrap_or(NonNegI64::zero());
            let payable = balance_plus(claim.bond, i64::from(reward), "withdrawal payment")?;
            self.debit_pos_vault(i64::from(payable)).await?;
            let address = self.vault_address(&validator)?;
            let balance = self
                .vault_balance(&address)
                .await?
                .unwrap_or(NonNegI64::zero());
            self.set_vault_balance(
                &address,
                balance_plus(balance, i64::from(payable), "refund")?,
            );
        }

        // 4. The active set for the epoch that starts now.
        let active = select_active(&pool, &withdrawers, &params);
        self.set_bonds(&pool);
        self.set_withdrawers(&withdrawers);
        self.set_pending_withdrawers(&pending);
        self.set_committed_rewards(&committed);
        self.set_active(&active);
        Ok(Ok(()))
    }

    /// Step 1 of the epoch sequence: every pooled validator's share of the pot, zero for the ones
    /// outside the active set (`getCurrentEpochRewards`, `Pos.rhox:241-256`).
    ///
    /// The active set it reads is the **stored** one, not a recomputation — which is what makes a
    /// validator that bonded during this epoch earn nothing yet (it is not in the active set until
    /// step 4) and a validator whose withdrawal was staged this epoch earn its last reward here.
    async fn epoch_rewards(
        &self,
        pool: &BTreeMap<Validator, NonNegI64>,
        withdrawers: &BTreeMap<Validator, Withdrawal>,
        committed: &BTreeMap<Validator, NonNegI64>,
        params: &PosParams,
    ) -> Result<BTreeMap<Validator, NonNegI64>, String> {
        let active = self.active().await?;
        let pot = epoch_pot(
            self.pos_vault_balance().await?,
            pool,
            withdrawers,
            committed,
        )?;
        let active_bonds: i128 = pool
            .iter()
            .filter(|(v, _)| active.contains_key(*v))
            .map(|(_, stake)| i128::from(i64::from(*stake)))
            .sum();
        let active_bonds = checked_i64(active_bonds, "active bonds")?;
        let mut rewards = BTreeMap::new();
        for (validator, stake) in pool {
            let reward = if active.contains_key(validator) {
                epoch_reward(pot, params.minimum_bond, active_bonds, i64::from(*stake))?
            } else {
                0
            };
            rewards.insert(
                *validator,
                NonNegI64::try_from(reward).map_err(|e| e.to_string())?,
            );
        }
        Ok(rewards)
    }

    /// Remove a validator and confiscate its stake to the Coop slashing vault (port of the PoS
    /// `slash` behavior). A pending withdrawal is cancelled (the stake is forfeited).
    ///
    /// Removal is **immediate** in the contract, unlike bonding: the slashing contract deletes the
    /// validator from `activeValidators` and zeroes its bond in the same state update as the transfer
    /// (`Pos.rhox:486-495`), so law 44's epoch gate does not apply to it. What the contract does not
    /// do is take the validator out of `pendingWithdrawers`; the port does, with the same payable
    /// outcome (see the note in `close_block`).
    pub async fn slash(&self, validator: &Validator) -> Result<Result<(), String>, String> {
        let mut pool = self.bonds().await?;
        let mut active = self.active().await?;
        let mut withdrawers = self.withdrawers().await?;
        let mut pending = self.pending_withdrawers().await?;
        let stake = pool.remove(validator);
        active.remove(validator);
        withdrawers.remove(validator);
        pending.remove(validator);
        if let Some(stake) = stake {
            // The stake leaves the staking vault for the Coop multisig vault (`Pos.rhox:470-482`:
            // `posVault!("transfer", coopMultiVaultAddr, valBond, posAuthKey)`). Debiting the source
            // is what makes this a *transfer*: crediting the Coop vault on its own — which is what
            // this did before the staking vault existed — mints the slashed bond out of nothing.
            self.debit_pos_vault(i64::from(stake)).await?;
            let coop = self.coop_balance().await?;
            self.set_coop_balance(balance_plus(coop, i64::from(stake), "slash coop")?);
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
            return Ok(Err("A stakeholder cannot revoke its own trust.".to_string()));
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
    /// unforgeable-name capability is not modeled — `spec/RUST-FIRST.md`'s B2 decision says what that
    /// costs (delegation) and what it does not (the spend rule, which the caller's `deployerId`
    /// carries), and `spec/AUDIT.md` §6 holds it as a registered deviation).
    pub async fn find_or_create_vault(&self, address: &str) -> Result<(), String> {
        if self.vault_balance(address).await?.is_none() {
            self.set_vault_balance(address, NonNegI64::zero());
        }
        Ok(())
    }

    // --- Cross-shard 2PC transactions ----------------------------------------

    /// Read a cross-shard transaction record, if present.
    pub async fn txn(&self, id: &[u8]) -> Result<Option<TxnRecord>, String> {
        match self.store.get(PREFIX_TXN, &txn_key(id)).await? {
            Some(bytes) => decode_txn(&bytes).map(Some),
            None => Ok(None),
        }
    }

    /// Write a cross-shard transaction record.
    fn set_txn(&self, id: &[u8], rec: &TxnRecord) {
        self.store.put(PREFIX_TXN, txn_key(id), encode_txn(rec));
    }

    /// Escrow `amount` REV from `from`'s vault and record a `Prepared` transaction. Idempotent under
    /// the transaction id (Law 28): a duplicate `prepare` returns the existing state unchanged.
    pub async fn txn_prepare(
        &self,
        id: &[u8],
        coordinator: &PublicKey,
        amount: NonNegI64,
        from: &str,
        to: &str,
    ) -> Result<TxnState, String> {
        if let Some(existing) = self.txn(id).await? {
            return Ok(existing.state);
        }
        let from_balance = self.vault_balance(from).await?.unwrap_or(NonNegI64::zero());
        if i64::from(from_balance) < i64::from(amount) {
            return Err("txn prepare: insufficient balance".to_string());
        }
        let new_from = NonNegI64::try_from(i64::from(from_balance) - i64::from(amount))
            .map_err(|e| e.to_string())?;
        self.set_vault_balance(from, new_from);
        self.set_txn(
            id,
            &TxnRecord {
                state: TxnState::Prepared,
                coordinator: coordinator.clone(),
                amount,
                from: from.to_string(),
                to: to.to_string(),
            },
        );
        Ok(TxnState::Prepared)
    }

    /// Commit a prepared transaction: transfer the escrow to `to`. Idempotent (a committed record is
    /// returned unchanged); committing an aborted transaction is an error.
    pub async fn txn_commit(&self, id: &[u8]) -> Result<TxnState, String> {
        let Some(rec) = self.txn(id).await? else {
            return Err("txn commit: unknown transaction".to_string());
        };
        match rec.state {
            TxnState::Committed => return Ok(TxnState::Committed),
            TxnState::Aborted => return Err("txn commit: already aborted".to_string()),
            TxnState::Prepared => {}
        }
        let to_balance = self
            .vault_balance(&rec.to)
            .await?
            .unwrap_or(NonNegI64::zero());
        let new_to = NonNegI64::try_from(
            i64::from(to_balance)
                .checked_add(i64::from(rec.amount))
                .ok_or("txn commit: destination balance overflow")?,
        )
        .map_err(|e| e.to_string())?;
        self.set_vault_balance(&rec.to, new_to);
        self.set_txn(
            id,
            &TxnRecord {
                state: TxnState::Committed,
                ..rec.clone()
            },
        );
        Ok(TxnState::Committed)
    }

    /// Abort a prepared transaction: return the escrow to `from`. Idempotent (an aborted record is
    /// returned unchanged); aborting a committed transaction is an error.
    pub async fn txn_abort(&self, id: &[u8]) -> Result<TxnState, String> {
        let Some(rec) = self.txn(id).await? else {
            return Err("txn abort: unknown transaction".to_string());
        };
        match rec.state {
            TxnState::Aborted => return Ok(TxnState::Aborted),
            TxnState::Committed => return Err("txn abort: already committed".to_string()),
            TxnState::Prepared => {}
        }
        let from_balance = self
            .vault_balance(&rec.from)
            .await?
            .unwrap_or(NonNegI64::zero());
        let new_from = NonNegI64::try_from(
            i64::from(from_balance)
                .checked_add(i64::from(rec.amount))
                .ok_or("txn abort: source balance overflow")?,
        )
        .map_err(|e| e.to_string())?;
        self.set_vault_balance(&rec.from, new_from);
        self.set_txn(
            id,
            &TxnRecord {
                state: TxnState::Aborted,
                ..rec.clone()
            },
        );
        Ok(TxnState::Aborted)
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
        // The charge is deposited into the staking vault (`Pos.rhox:397-404`:
        // `deposit!(deployerId, amount, posVaultAddr)`), which is where an epoch's reward pot comes
        // from. The charge is the deploy's *maximum* phlo; what the deploy does not consume comes
        // back out in [`Self::refund`], so what the vault keeps is exactly the phlo burned.
        self.credit_pos_vault(amount).await?;
        Ok(Ok(()))
    }

    /// Refund `amount` to `deployer`'s vault out of the staking vault (port of the PoS
    /// `refundDeploy` behavior, `Pos.rhox:417-454`: `posVault!("transfer", deployerRevAddress,
    /// refundAmount, posAuthKey)`).
    ///
    /// `amount <= 0` succeeds without moving anything — the contract's own `if (refundAmount > 0)`
    /// branch. The Scala takes the deployer from the `currentDeployerData` cell that `chargeDeploy`
    /// filled, because the contract's `refundDeploy` is called with only the amount; this port has no
    /// such limitation and carries the deployer in the system deploy itself (`Refund { deployer,
    /// amount }` in `casper::system_deploy`), the same way the pre-charge already does — so the payer
    /// is in the type rather than in a mutable cell.
    ///
    /// This used to be a **documented no-op** (`refund_is_a_documented_no_op`): the charged phlo was
    /// burned rather than returned. Now that the staking vault exists it is the transfer the contract
    /// describes, and the test pins the two-way movement — including that the over-charge does *not*
    /// reach the reward pot.
    pub async fn refund(
        &self,
        deployer: &PublicKey,
        amount: i64,
    ) -> Result<Result<(), String>, String> {
        if amount <= 0 {
            return Ok(Ok(()));
        }
        let address = RevAddress::from_public_key(deployer)
            .ok_or_else(|| "refund: invalid deployer public key".to_string())?
            .to_base58();
        self.debit_pos_vault(amount).await?;
        let balance = self
            .vault_balance(&address)
            .await?
            .unwrap_or(NonNegI64::zero());
        self.set_vault_balance(&address, balance_plus(balance, amount, "refund")?);
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
        native
            .install_genesis(&PosGenesis {
                bonds,
                trusted: trusted.iter().copied().collect(),
                params,
            })
            .unwrap();
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

    /// The total REV the native state holds: the addresses given, the Coop vault, and the staking
    /// vault. Every step of the mechanism — bond, slash, phlo charge, refund, withdrawal payment —
    /// is a transfer *between* these places, so this total is invariant. Checking it is what makes
    /// "the staking vault is the only source of every payout" a property rather than a claim about
    /// the code: the version before this one credited the Coop vault on a slash without debiting
    /// anywhere, and this is the assertion that would have caught it.
    async fn total_rev(native: &NativeSystemState, addresses: &[String]) -> i64 {
        let mut total = i64::from(native.coop_balance().await.unwrap())
            + i64::from(native.pos_vault_balance().await.unwrap());
        for address in addresses {
            total += native
                .vault_balance(address)
                .await
                .unwrap()
                .map(i64::from)
                .unwrap_or(0);
        }
        total
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
        let active = select_active(&pool, &BTreeMap::<Validator, ()>::new(), &params);
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
        let native =
            native_with(&[validator(1)], PosParams::default(), &[(validator(1), 10)]).await;
        let v2 = validator(2);
        fund(&native, &v2, 100).await;

        let rejected = native
            .bond(&v2, NonNegI64::try_from(40).unwrap(), 0)
            .await
            .unwrap();
        assert!(rejected.is_err(), "untrusted observer must not bond");

        native.trust(&validator(1), &v2).await.unwrap().unwrap();
        native
            .bond(&v2, NonNegI64::try_from(40).unwrap(), 0)
            .await
            .unwrap()
            .unwrap();
        native.close_block(1).await.unwrap().unwrap();
        assert!(native.active().await.unwrap().contains_key(&v2));
    }

    #[tokio::test]
    async fn trust_requires_a_trusted_caller() {
        let native =
            native_with(&[validator(1)], PosParams::default(), &[(validator(1), 10)]).await;
        let result = native.trust(&validator(9), &validator(2)).await.unwrap();
        assert!(result.is_err(), "a non-stakeholder cannot admit validators");
    }

    #[tokio::test]
    async fn bond_enforces_min_and_max() {
        let params = PosParams {
            minimum_bond: 10,
            maximum_bond: 50,
            ..PosParams::default()
        };
        let native =
            native_with(&[validator(1), validator(2)], params, &[(validator(1), 10)]).await;
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

    /// A bond escrows the stake and joins the pool immediately, and becomes **active at the epoch
    /// boundary** — the contract's `bond` writes only `allBonds` (`Pos.rhox:355`) and
    /// `activeValidators` is recomputed inside `closeBlock` (`:546`). With the permissive default
    /// parameters every block is a boundary, so the boundary is the same block's `close_block`; a
    /// validator active before it would be a consensus set that changes inside an epoch.
    #[tokio::test]
    async fn bond_escrows_the_stake_and_activates_at_the_boundary() {
        let native = native_with(&[validator(1)], PosParams::default(), &[]).await;
        let v = validator(1);
        fund(&native, &v, 100).await;

        native
            .bond(&v, NonNegI64::try_from(40).unwrap(), 0)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(
            i64::from(
                native
                    .vault_balance(&vault_address_of(&v))
                    .await
                    .unwrap()
                    .unwrap()
            ),
            60
        );
        assert_eq!(i64::from(native.bonds().await.unwrap()[&v]), 40, "pooled");
        assert!(
            !native.active().await.unwrap().contains_key(&v),
            "a bonded validator is not in the consensus set until the boundary"
        );

        native.close_block(1).await.unwrap().unwrap();
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

    /// **Law 47 — a withdrawal is staged, not immediate.** The request records a deadline and changes
    /// nothing else: the validator stays bonded and stays in the active set (the contract's
    /// `withdraw` writes only `pendingWithdrawers`, `Pos.rhox:377-382`). It is moved out of the pool
    /// at the *next* epoch boundary and paid at the first boundary at which its quarantine has
    /// elapsed.
    ///
    /// `epoch_length: 1` makes every block a boundary, so the arithmetic is the contract's own:
    /// `quarantineLength + epochLength * (1 + blockNumber / epochLength)` at block 5 with a
    /// quarantine of 10 is `10 + 1 * 6 = 16`.
    #[tokio::test]
    async fn withdraw_stages_the_validator_until_the_next_boundary() {
        let params = PosParams {
            epoch_length: 1,
            quarantine_length: 10,
            ..PosParams::default()
        };
        let native = native_with(&[validator(1)], params, &[(validator(1), 40)]).await;
        let v = validator(1);
        native.set_vault_balance(&vault_address_of(&v), NonNegI64::zero());

        native.withdraw(&v, 5).await.unwrap().unwrap();
        assert_eq!(
            native.pending_withdrawers().await.unwrap()[&v],
            16,
            "the request stages a deadline, and only that"
        );
        assert!(
            native.bonds().await.unwrap().contains_key(&v),
            "the stake stays in the pool until the boundary"
        );
        assert!(
            native.active().await.unwrap().contains_key(&v),
            "and the validator keeps validating — and earning — until then"
        );

        // The boundary moves it out of the pool and into an escrowed claim.
        native.close_block(10).await.unwrap().unwrap();
        assert!(native.pending_withdrawers().await.unwrap().is_empty());
        assert!(
            !native.bonds().await.unwrap().contains_key(&v),
            "withdrawn validator removed from the pool at the boundary"
        );
        assert_eq!(
            native.withdrawers().await.unwrap()[&v],
            Withdrawal {
                bond: NonNegI64::try_from(40).unwrap(),
                deadline: 16,
            }
        );
        assert!(
            !native.active().await.unwrap().contains_key(&v),
            "and out of the active set"
        );
        assert_eq!(
            i64::from(
                native
                    .vault_balance(&vault_address_of(&v))
                    .await
                    .unwrap()
                    .unwrap()
            ),
            0,
            "the stake is still escrowed: block 10 is before the deadline of 16"
        );

        native.close_block(16).await.unwrap().unwrap();
        assert!(native.withdrawers().await.unwrap().is_empty());
        assert_eq!(
            i64::from(
                native
                    .vault_balance(&vault_address_of(&v))
                    .await
                    .unwrap()
                    .unwrap()
            ),
            40,
            "the stake is paid out at the first boundary past the quarantine"
        );
    }

    /// **Law 44 — the epoch gate.** `close_block` changes *nothing* except at a boundary
    /// (`Pos.rhox:517-519`): no reward is committed, no staged withdrawal moves, no claim is paid,
    /// and the active set is not recomputed. Done at a non-boundary, the whole sequence is a no-op
    /// even when there is a pending withdrawal and a full pot waiting.
    #[tokio::test]
    async fn the_epoch_gate_does_nothing_off_a_boundary() {
        let params = PosParams {
            epoch_length: 10,
            quarantine_length: 0,
            ..PosParams::default()
        };
        let native = native_with(&[validator(1)], params, &[(validator(1), 40)]).await;
        let v = validator(1);
        native.set_vault_balance(&vault_address_of(&v), NonNegI64::zero());
        let addr = vault_address_of(&v);
        // Fund the pot the way the protocol does: a deploy pays its phlo into the staking vault.
        let payer = PublicKey::new(vec![9u8; 65]);
        let payer_addr = RevAddress::from_public_key(&payer).unwrap().to_base58();
        native.set_vault_balance(&payer_addr, NonNegI64::try_from(10).unwrap());
        native.pre_charge(&payer, 10).await.unwrap().unwrap();
        native.withdraw(&v, 3).await.unwrap().unwrap();
        let before = (
            native.pending_withdrawers().await.unwrap(),
            native.committed_rewards().await.unwrap(),
            native.bonds().await.unwrap(),
            native.active().await.unwrap(),
        );

        // 7 is not a multiple of 10: nothing happens.
        native.close_block(7).await.unwrap().unwrap();
        assert_eq!(
            (
                native.pending_withdrawers().await.unwrap(),
                native.committed_rewards().await.unwrap(),
                native.bonds().await.unwrap(),
                native.active().await.unwrap(),
            ),
            before,
            "an epoch that does not occur changes nothing at all"
        );
        assert_eq!(
            i64::from(native.pos_vault_balance().await.unwrap()),
            50,
            "the pot is untouched — no reward was committed"
        );
        assert_eq!(
            i64::from(native.vault_balance(&addr).await.unwrap().unwrap()),
            0,
            "and no claim was paid"
        );

        // 10 is a boundary: the sequence runs. The request staged at block 3 was given the deadline
        // `0 + 10 * (1 + 3 / 10) = 10`, so this boundary both moves it out of the pool and pays it.
        native.close_block(10).await.unwrap().unwrap();
        assert!(
            native.pending_withdrawers().await.unwrap().is_empty(),
            "the staged withdrawal moved at the boundary"
        );
        assert!(
            !native.bonds().await.unwrap().contains_key(&v),
            "out of the pool"
        );
        assert_eq!(
            i64::from(native.vault_balance(&addr).await.unwrap().unwrap()),
            40,
            "and paid: the bond, plus the epoch's committed reward"
        );

        // The gate's other half, with an epoch length a block cannot reach: a bond joins the pool at
        // once but joins the *consensus set* only at a boundary.
        let native = native_with(&[validator(1)], params, &[]).await;
        let v2 = validator(1);
        fund(&native, &v2, 100).await;
        native
            .bond(&v2, NonNegI64::try_from(40).unwrap(), 7)
            .await
            .unwrap()
            .unwrap();
        assert!(
            native.bonds().await.unwrap().contains_key(&v2),
            "pooled at once"
        );
        assert!(!native.active().await.unwrap().contains_key(&v2));
        native.close_block(9).await.unwrap().unwrap();
        assert!(
            !native.active().await.unwrap().contains_key(&v2),
            "9 is not a boundary either"
        );
        native.close_block(10).await.unwrap().unwrap();
        assert!(
            native.active().await.unwrap().contains_key(&v2),
            "the boundary is where a bond becomes a validator"
        );
    }

    /// **Laws 45 and 46 — the split, checked against the model rather than a remembered number.**
    /// `Rchain/Pos.lean`'s `the_dust_is_real` is the `decide`d case `minimumBond = 3`, bonds
    /// `[4, 5]`, pot `10`: the normaliser is `9 / 3 = 3`, each scaled share is `4 / 3 = 5 / 3 = 1`,
    /// and each validator is paid `10 * 1 / 3 = 3` — **6 distributed of 10**, the rest being the dust
    /// of two integer divisions. This test builds exactly that state and reads the split back, so the
    /// implementation is checked against the arithmetic the Lean proves rather than against itself.
    #[tokio::test]
    async fn an_epoch_splits_the_pot_and_keeps_the_dust() {
        let params = PosParams {
            minimum_bond: 3,
            epoch_length: 1,
            ..PosParams::default()
        };
        let native = native_with(
            &[validator(1), validator(2)],
            params,
            &[(validator(1), 4), (validator(2), 5)],
        )
        .await;
        // Fill the pot with exactly 10 the way a deploy's phlo does.
        let payer = PublicKey::new(vec![9u8; 65]);
        let payer_addr = RevAddress::from_public_key(&payer).unwrap().to_base58();
        native.set_vault_balance(&payer_addr, NonNegI64::try_from(10).unwrap());
        native.pre_charge(&payer, 10).await.unwrap().unwrap();
        assert_eq!(
            i64::from(native.pos_vault_balance().await.unwrap()),
            9 + 10,
            "the vault holds the bonds plus the phlo"
        );

        native.close_block(1).await.unwrap().unwrap();

        let committed = native.committed_rewards().await.unwrap();
        assert_eq!(
            i64::from(committed[&validator(1)]),
            3,
            "10 * (4 / 3) / (9 / 3) = 3"
        );
        assert_eq!(
            i64::from(committed[&validator(2)]),
            3,
            "10 * (5 / 3) / (9 / 3) = 3"
        );
        assert!(
            i64::from(committed[&validator(1)]) + i64::from(committed[&validator(2)]) < 10,
            "the shares do not sum to the pot — that inequality is the law"
        );
        assert_eq!(
            i64::from(native.pos_vault_balance().await.unwrap()),
            19,
            "the vault is not debited by a commitment: the reward is paid when the validator leaves"
        );
        assert_eq!(
            epoch_pot(
                native.pos_vault_balance().await.unwrap(),
                &native.bonds().await.unwrap(),
                &native.withdrawers().await.unwrap(),
                &native.committed_rewards().await.unwrap(),
            )
            .unwrap(),
            4,
            "the four units of dust stay in the pot, and the next epoch distributes them"
        );
    }

    /// **Law 47's payoff.** A validator that earns, then asks to leave, is paid **its bond plus every
    /// reward committed while it was bonded** — including the epoch it spent its last blocks in, which
    /// is exactly what the sequence's order (commit, then move, then pay) is for.
    #[tokio::test]
    async fn a_released_withdrawal_pays_the_bond_plus_the_committed_rewards() {
        let params = PosParams {
            epoch_length: 1,
            quarantine_length: 0,
            // A positive minimum, or the split is the undefined case and pays nothing.
            minimum_bond: 3,
            ..PosParams::default()
        };
        let native = native_with(&[validator(1)], params, &[(validator(1), 40)]).await;
        let v = validator(1);
        native.set_vault_balance(&vault_address_of(&v), NonNegI64::zero());
        // 5 of phlo for the epoch, charged to a deployer.
        let payer = PublicKey::new(vec![9u8; 65]);
        let payer_addr = RevAddress::from_public_key(&payer).unwrap().to_base58();
        native.set_vault_balance(&payer_addr, NonNegI64::try_from(5).unwrap());
        native.pre_charge(&payer, 5).await.unwrap().unwrap();

        // Stage the request, then close the block: the boundary pays the epoch's reward into the
        // committed map *and* moves the validator out of the pool, in that order.
        native.withdraw(&v, 1).await.unwrap().unwrap();
        native.close_block(1).await.unwrap().unwrap();
        assert_eq!(
            i64::from(native.committed_rewards().await.unwrap()[&v]),
            5,
            "the epoch it was still active for paid it the whole pot (one validator, no rounding)"
        );
        assert!(
            !native.bonds().await.unwrap().contains_key(&v),
            "and it left the pool at the same boundary"
        );

        // The claim's deadline is 0 + 1 * (1 + 1) = 2, so the next boundary pays it.
        assert_eq!(
            native.withdrawers().await.unwrap()[&v].deadline,
            2,
            "quarantineLength + epochLength * (1 + blockNumber / epochLength)"
        );
        native.close_block(2).await.unwrap().unwrap();
        assert_eq!(
            i64::from(
                native
                    .vault_balance(&vault_address_of(&v))
                    .await
                    .unwrap()
                    .unwrap()
            ),
            45,
            "bond 40 + committed 5, out of the staking vault"
        );
        assert!(
            native.committed_rewards().await.unwrap().is_empty(),
            "the claim is spent: committed and withdrawers both cleared"
        );
        assert!(native.withdrawers().await.unwrap().is_empty());
        assert_eq!(
            i64::from(native.pos_vault_balance().await.unwrap()),
            0,
            "the vault is exactly emptied — the phlo went to the validator, not to nowhere"
        );
    }

    /// Where the contract's formula is undefined the port pays nothing rather than faulting. With
    /// `minimum_bond == 0` the Scala's `bonds / $$minimumBond$$` is a division by zero, and the
    /// port's parameters are permissive by default, so a fault is not a rule it can copy; a
    /// normaliser of zero (`activeBonds < minimumBond`) is the same story. `Pos.lean` states its
    /// theorem for `0 < activeBonds / minimumBond` and says nothing about the rest — this is what the
    /// port does with the rest.
    #[tokio::test]
    async fn an_epoch_with_a_zero_normaliser_pays_nothing() {
        // minimum_bond 0: the normaliser is a division by zero in the contract.
        let native = native_with(
            &[validator(1)],
            PosParams {
                epoch_length: 1,
                ..PosParams::default()
            },
            &[(validator(1), 40)],
        )
        .await;
        let payer = PublicKey::new(vec![9u8; 65]);
        let payer_addr = RevAddress::from_public_key(&payer).unwrap().to_base58();
        native.set_vault_balance(&payer_addr, NonNegI64::try_from(5).unwrap());
        native.pre_charge(&payer, 5).await.unwrap().unwrap();

        native.close_block(1).await.unwrap().unwrap();
        assert_eq!(
            i64::from(native.committed_rewards().await.unwrap()[&validator(1)]),
            0,
            "an undefined split pays zero"
        );
        assert_eq!(
            i64::from(native.pos_vault_balance().await.unwrap()),
            45,
            "and the pot is left where it was"
        );

        // A normaliser of zero with a positive minimum: bonds below the minimum scale to nothing.
        let native = native_with(
            &[validator(1)],
            PosParams {
                minimum_bond: 100,
                epoch_length: 1,
                ..PosParams::default()
            },
            &[(validator(1), 40)],
        )
        .await;
        let native = native;
        native.set_vault_balance(&payer_addr, NonNegI64::try_from(5).unwrap());
        native.pre_charge(&payer, 5).await.unwrap().unwrap();
        native.close_block(1).await.unwrap().unwrap();
        assert_eq!(
            i64::from(native.committed_rewards().await.unwrap()[&validator(1)]),
            0
        );
    }

    /// The pot floors at zero instead of wrapping. The state this needs cannot be reached through the
    /// protocol — every debit is bounded by the credit that funded it, so the vault always covers its
    /// claims — so the test builds the diverged ledger by hand to show the floor holds. The Scala
    /// computes the pot in `Long` and would produce a *negative* reward here; `NonNegI64` has no such
    /// value, and the model's subtraction is `Nat` (floored), so the port floors with it.
    #[tokio::test]
    async fn a_drafted_vault_floors_the_pot_instead_of_wrapping() {
        let native = native_with(
            &[validator(1)],
            PosParams {
                epoch_length: 1,
                minimum_bond: 3,
                ..PosParams::default()
            },
            &[(validator(1), 40)],
        )
        .await;
        // Drain the vault below the bonded pool: a ledger that has already diverged.
        native.debit_pos_vault(35).await.unwrap();
        assert_eq!(i64::from(native.pos_vault_balance().await.unwrap()), 5);

        native.close_block(1).await.unwrap().unwrap();
        assert_eq!(
            i64::from(native.committed_rewards().await.unwrap()[&validator(1)]),
            0,
            "a pot of nothing pays nothing — not a wrap, and not a negative reward"
        );
    }

    #[tokio::test]
    async fn slash_confiscates_to_coop_vault() {
        let native = native_with(
            &[validator(1), validator(2)],
            PosParams::default(),
            &[(validator(1), 10)],
        )
        .await;
        let v = validator(2);
        fund(&native, &v, 100).await;
        native
            .bond(&v, NonNegI64::try_from(40).unwrap(), 0)
            .await
            .unwrap()
            .unwrap();

        // v1's genesis bond (10) plus v2's (40) are in the staking vault.
        assert_eq!(i64::from(native.pos_vault_balance().await.unwrap()), 50);
        let addresses = [vault_address_of(&v)];
        let before = total_rev(&native, &addresses).await;

        native.slash(&v).await.unwrap().unwrap();

        assert!(!native.bonds().await.unwrap().contains_key(&v));
        assert!(!native.active().await.unwrap().contains_key(&v));
        assert_eq!(
            i64::from(native.coop_balance().await.unwrap()),
            40,
            "slashed stake is confiscated to the Coop vault"
        );
        assert_eq!(
            i64::from(native.pos_vault_balance().await.unwrap()),
            10,
            "…and it came *out* of the staking vault (`Pos.rhox:470-482`'s transfer), leaving v1's bond"
        );
        assert_eq!(
            total_rev(&native, &addresses).await,
            before,
            "a slashing is a transfer to the Coop vault, not a mint"
        );
    }

    #[tokio::test]
    async fn untrust_removes_and_confiscates() {
        let native = native_with(
            &[validator(1), validator(2)],
            PosParams::default(),
            &[(validator(1), 10)],
        )
        .await;
        let v = validator(2);
        fund(&native, &v, 100).await;
        native
            .bond(&v, NonNegI64::try_from(40).unwrap(), 0)
            .await
            .unwrap()
            .unwrap();

        native.untrust(&validator(1), &v).await.unwrap().unwrap();

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

        let active: Vec<Validator> = native
            .active_validators()
            .await
            .unwrap()
            .into_iter()
            .collect();
        // The two highest-stake validators (2 and 3); `BTreeSet` iteration is by key order.
        assert_eq!(active, vec![validator(2), validator(3)]);
        assert_eq!(
            native.bonds().await.unwrap().len(),
            3,
            "pool keeps the third"
        );
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

    #[test]
    fn txn_record_round_trip() {
        let rec = TxnRecord {
            state: TxnState::Committed,
            coordinator: PublicKey::new(vec![7u8; 65]),
            amount: NonNegI64::try_from(12345).unwrap(),
            from: "fromRevAddress".to_string(),
            to: "toRevAddress".to_string(),
        };
        let encoded = encode_txn(&rec);
        assert_eq!(decode_txn(&encoded).unwrap(), rec);
    }

    #[tokio::test]
    async fn txn_prepare_commit_round_trip() {
        let native = NativeSystemState::new(Arc::new(InMemNativeStore::empty()));
        let coordinator = PublicKey::new(vec![4u8; 65]);
        let from = "fromAddr".to_string();
        let to = "toAddr".to_string();
        let id: &[u8] = b"txn-1";

        native.set_vault_balance(&from, NonNegI64::try_from(100).unwrap());

        // Prepare escrows 40 REV from `from`.
        let state = native
            .txn_prepare(
                id,
                &coordinator,
                NonNegI64::try_from(40).unwrap(),
                &from,
                &to,
            )
            .await
            .unwrap();
        assert_eq!(state, TxnState::Prepared);
        assert_eq!(
            i64::from(native.vault_balance(&from).await.unwrap().unwrap()),
            60
        );

        // Commit transfers 40 REV to `to`, and is idempotent.
        let state = native.txn_commit(id).await.unwrap();
        assert_eq!(state, TxnState::Committed);
        assert_eq!(
            i64::from(native.vault_balance(&to).await.unwrap().unwrap()),
            40
        );
        assert_eq!(native.txn_commit(id).await.unwrap(), TxnState::Committed);
        assert_eq!(
            i64::from(native.vault_balance(&to).await.unwrap().unwrap()),
            40
        );
    }

    #[tokio::test]
    async fn txn_prepare_abort_returns_escrow() {
        let native = NativeSystemState::new(Arc::new(InMemNativeStore::empty()));
        let coordinator = PublicKey::new(vec![4u8; 65]);
        let from = "fromAddr".to_string();
        let to = "toAddr".to_string();
        let id: &[u8] = b"txn-2";

        native.set_vault_balance(&from, NonNegI64::try_from(100).unwrap());
        native
            .txn_prepare(
                id,
                &coordinator,
                NonNegI64::try_from(30).unwrap(),
                &from,
                &to,
            )
            .await
            .unwrap();

        // Abort returns the escrow to `from`, and is idempotent.
        assert_eq!(native.txn_abort(id).await.unwrap(), TxnState::Aborted);
        assert_eq!(
            i64::from(native.vault_balance(&from).await.unwrap().unwrap()),
            100
        );
        assert_eq!(native.txn_abort(id).await.unwrap(), TxnState::Aborted);
    }

    #[tokio::test]
    async fn law28_txn_prepare_rejects_overdraw_and_is_idempotent() {
        let native = NativeSystemState::new(Arc::new(InMemNativeStore::empty()));
        let coordinator = PublicKey::new(vec![4u8; 65]);
        let from = "fromAddr".to_string();
        let to = "toAddr".to_string();
        let id: &[u8] = b"txn-3";

        native.set_vault_balance(&from, NonNegI64::try_from(50).unwrap());

        // Overdraw is rejected and leaves the vault untouched.
        assert!(native
            .txn_prepare(
                id,
                &coordinator,
                NonNegI64::try_from(51).unwrap(),
                &from,
                &to
            )
            .await
            .is_err());
        assert_eq!(
            i64::from(native.vault_balance(&from).await.unwrap().unwrap()),
            50
        );

        // A successful prepare is idempotent (a duplicate re-deducts nothing).
        native
            .txn_prepare(
                id,
                &coordinator,
                NonNegI64::try_from(40).unwrap(),
                &from,
                &to,
            )
            .await
            .unwrap();
        let state = native
            .txn_prepare(
                id,
                &coordinator,
                NonNegI64::try_from(40).unwrap(),
                &from,
                &to,
            )
            .await
            .unwrap();
        assert_eq!(state, TxnState::Prepared);
        assert_eq!(
            i64::from(native.vault_balance(&from).await.unwrap().unwrap()),
            10
        );
    }

    #[test]
    fn http_records_round_trip() {
        let mut records = BTreeMap::new();
        records.insert(
            "https://example.com/price".to_string(),
            ("42.5".to_string(), 7i64),
        );
        records.insert(
            "https://example.com/time".to_string(),
            ("".to_string(), 0i64),
        );
        let bytes = encode_http_records(&records);
        assert_eq!(decode_http_records(&bytes).unwrap(), records);
        assert!(decode_http_records(&bytes[..bytes.len() - 1]).is_err());
        assert!(decode_http_records(&[]).is_err());
    }

    #[tokio::test]
    async fn http_record_is_first_writer_wins() {
        let native = NativeSystemState::new(Arc::new(InMemNativeStore::empty()));
        let url = "https://example.com/price";

        // Nothing recorded yet.
        assert_eq!(native.http_record(url).await.unwrap(), None);

        // First capture wins and records the capture height.
        assert!(native.record_http(url, "42.5", 7).await.unwrap());
        assert_eq!(
            native.http_record(url).await.unwrap(),
            Some(("42.5".to_string(), 7))
        );

        // A later capture does not overwrite (this is what makes `check` meaningful).
        assert!(!native.record_http(url, "99.9", 8).await.unwrap());
        assert_eq!(
            native.http_record(url).await.unwrap(),
            Some(("42.5".to_string(), 7))
        );

        // Distinct URLs are independent.
        assert!(native
            .record_http("https://example.com/other", "x", 9)
            .await
            .unwrap());
        assert_eq!(native.http_records().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn bond_moves_the_stake_into_the_staking_vault() {
        let native = native_with(&[validator(1)], PosParams::default(), &[]).await;
        let v = validator(1);
        fund(&native, &v, 100).await;
        assert_eq!(
            i64::from(native.pos_vault_balance().await.unwrap()),
            0,
            "a genesis with an empty pool funds the vault with nothing"
        );
        let addresses = [vault_address_of(&v)];
        let before = total_rev(&native, &addresses).await;

        native
            .bond(&v, NonNegI64::try_from(40).unwrap(), 0)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(
            i64::from(native.pos_vault_balance().await.unwrap()),
            40,
            "the stake is escrowed in the staking vault"
        );
        assert_eq!(
            total_rev(&native, &addresses).await,
            before,
            "a bond is a transfer, not a mint"
        );
    }

    /// The charge goes into the staking vault and the refund takes the surplus back out, so what the
    /// vault keeps is exactly the phlo the deploy burned. This is the flow an epoch's reward pot
    /// comes from: `Pos.rhox:397-404`'s `deposit!(deployerId, amount, posVaultAddr)` on the charge,
    /// `Pos.rhox:417-454`'s `posVault!("transfer", deployerRevAddress, refundAmount, posAuthKey)` on
    /// the refund. Before the staking vault existed the charge was burned, so the pot could only ever
    /// be zero and a reward law over it could not have failed.
    #[tokio::test]
    async fn the_phlo_charge_funds_the_pot_and_the_refund_returns_the_surplus() {
        let native = NativeSystemState::new(Arc::new(InMemNativeStore::empty()));
        let pk = PublicKey::new(vec![1u8; 65]);
        let addr = RevAddress::from_public_key(&pk).unwrap().to_base58();
        native.set_vault_balance(&addr, NonNegI64::try_from(100).unwrap());
        let addresses = [addr.clone()];
        let before = total_rev(&native, &addresses).await;

        // Pre-charge the maximum phlo, then return all but 30 of it.
        native.pre_charge(&pk, 100).await.unwrap().unwrap();
        assert_eq!(i64::from(native.pos_vault_balance().await.unwrap()), 100);
        assert_eq!(
            i64::from(native.vault_balance(&addr).await.unwrap().unwrap()),
            0
        );
        native.refund(&pk, 70).await.unwrap().unwrap();

        assert_eq!(
            i64::from(native.vault_balance(&addr).await.unwrap().unwrap()),
            70,
            "the surplus returns to the deployer"
        );
        assert_eq!(
            i64::from(native.pos_vault_balance().await.unwrap()),
            30,
            "the vault keeps the phlo the deploy burned — the epoch's pot"
        );
        assert_eq!(
            total_rev(&native, &addresses).await,
            before,
            "charging and refunding is a transfer, not a mint"
        );

        // A non-positive refund succeeds without moving anything (`Pos.rhox:426`'s guard).
        native.refund(&pk, 0).await.unwrap().unwrap();
        native.refund(&pk, -5).await.unwrap().unwrap();
        assert_eq!(i64::from(native.pos_vault_balance().await.unwrap()), 30);
    }

    /// A payout the staking vault cannot cover **fails the deploy** instead of half-paying. The
    /// Scala's `payWithdrawer` ignores its failed transfer and removes the withdrawer from the maps
    /// anyway (`// FIXME fix transfer in failure case`, `Pos.rhox:603`), which loses the bond; the
    /// port refuses. The invariant that makes this unreachable in normal operation — every debit is
    /// bounded by the credit that funded it — is checked by the conservation assertions in the tests
    /// around this one.
    #[tokio::test]
    async fn a_short_staking_vault_fails_the_transfer_rather_than_half_paying() {
        let native = NativeSystemState::new(Arc::new(InMemNativeStore::empty()));
        let pk = PublicKey::new(vec![1u8; 65]);

        // No charge has happened, so the vault holds nothing to refund out of.
        assert!(
            native.refund(&pk, 1).await.is_err(),
            "a refund cannot come out of an empty vault"
        );
        assert!(native.debit_pos_vault(1).await.is_err());
        assert_eq!(
            i64::from(native.pos_vault_balance().await.unwrap()),
            0,
            "a refused debit leaves the vault untouched"
        );
    }
}
