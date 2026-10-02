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
//! * **active** — the consensus validator set ([`pos_active_key`]): a draw of up to
//!   `number_of_active_validators` from the pool (0 = unlimited), redrawn at each epoch boundary
//!   from the seeded rule in `select_active`. Consensus (supermajority, finality fringe, bond
//!   queries) reads this set.
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
use rchain_models::block_metadata::SlashSeverity;
use rchain_models::validator::Validator;
use rchain_shared::refined::NonNegI64;
use rchain_shared::serialize::Serialize;

use rchain_rspace::native_store::{
    InMemNativeStore, PREFIX_HTTP, PREFIX_POS, PREFIX_REGISTRY, PREFIX_TXN, PREFIX_VAULT,
    PREFIX_VAULT_AUTH, PREFIX_VAULT_NAME,
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

/// Leaf key for a **vault handle** (the minted name's bytes, hashed).
///
/// The name's own bytes are the key rather than a hash of the address, because a handle is per-call:
/// the same vault handed out twice is two names, and both must resolve (the oracle mints a fresh
/// purse per `findOrCreate` too — `RevVault.rho:103-140`).
fn vault_name_key(name: &[u8]) -> Blake2b256Hash {
    Blake2b256Hash::create(name)
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
    /// Minimum accepted bond (stake) per validator. **`NonNegI64`, not `i64`** (the deferred item
    /// 1d): a negative minimum is not a configuration this protocol has a meaning for, and while the
    /// field was signed the only thing standing between a genesis file and a stored `-1` was the
    /// bond check's arithmetic — where a *negative* minimum silently accepts every bond, including
    /// the zero-stake one the check exists to refuse. `decode_params` now refuses one at the wire,
    /// so the invariant holds on every value that reaches this struct.
    pub minimum_bond: NonNegI64,
    /// Maximum accepted bond (stake) per validator. Refined for the same reason, in the direction
    /// that matters at the other end: `i64::MAX` is the "no maximum" value, and a negative maximum
    /// would refuse every bond.
    pub maximum_bond: NonNegI64,
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
            minimum_bond: NonNegI64::zero(),
            maximum_bond: NonNegI64::saturating(i64::MAX),
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
        out.extend_from_slice(&i64::from(self.minimum_bond).to_le_bytes());
        out.extend_from_slice(&i64::from(self.maximum_bond).to_le_bytes());
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
    // The two bond bounds are refused rather than clamped (the deferred item 1d): a stored negative
    // is not a value this protocol can mean, and clamping it would silently turn a corrupted or
    // malicious params leaf into "no minimum", which is the permissive reading of the same bytes.
    Ok(PosParams {
        minimum_bond: NonNegI64::try_from(read(0))
            .map_err(|e| format!("minimum_bond is not a valid accept threshold: {e}"))?,
        maximum_bond: NonNegI64::try_from(read(1))
            .map_err(|e| format!("maximum_bond is not a valid accept threshold: {e}"))?,
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
        select_active(
            &self.bonds,
            &BTreeMap::<Validator, ()>::new(),
            &self.params,
            &genesis_epoch_seed(),
        )
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

/// Select the active validator set from the pool: drop zero-stake and withdrawing validators, then
/// **draw** up to `number_of_active_validators` of the rest **in proportion to stake**, without
/// replacement, seeded by `seed` (`0` = unlimited, no draw).
///
/// **Why a draw and not the top N.** The set this returns *is* the finality weight set
/// (`casper/src/multi_parent_casper.rs`'s `Finalizer`), so who is in it decides who can finalise.
/// Ranking by stake made membership a pure function of a value the proposer of the drawing block
/// could steer: the old seed was `hash(shard_id, block_number, sender, pre_state_hash)` computed at
/// the moment of use, so every candidate block was a fresh, free reroll — a proposer could try
/// headers until it drew the set it wanted. See `close_block` step 5 for what anchors the draw now.
///
/// **Why weighted, and not uniform.** The rule was a uniform draw until 2026-10-02, and a uniform
/// draw pays per **key** rather than per stake: the first slot went to a dust validator as readily as
/// to the largest one, so splitting a stake across `k` keys bought about `k` times the expected slots
/// (registered as residual O3 in `spec/RUST-VS-SCALA.md` §3). That is a sybil incentive in the
/// finality weight set *and* an income bug — `epoch_rewards` pays the drawn members, so a staker that
/// split its stake out-earned the same stake held whole, while a single large validator earned less
/// than its share. The weighted draw gives the **first** slot to a validator with probability exactly
/// `stake / total`, which is the property that removes the split incentive, and it needs no
/// participation score, no delegation and no new state: `(pool, seed)` still determines the result.
///
/// **Successive weighting, and the residual that stays.** The draw is sequential — pick by weight,
/// remove, renormalise, repeat — so it is exactly proportional for the first slot and *approximately*
/// so for the later ones (an item already drawn cannot be drawn again, so the tail slightly favours
/// the items that were not picked). Splitting a stake is therefore no longer worth ~`k` times the
/// slots, but it is not provably neutral either, and the honest statement is that O3 is reduced rather
/// than closed. The exact-proportional alternative is a systematic (rotated-interval) scheme, which
/// assigns a fixed share instead of a random one and would collide for any stake above `1/k` of the
/// total; that is a different rule with a different variance, and it is not this one.
///
/// Generic over what the withdrawal map holds, because only its key set matters here and the port has
/// two of them: the staged requests (`validator → deadline`, `pendingWithdrawers`) and the claims
/// (`validator → Withdrawal`, `withdrawers`).
pub fn select_active<V>(
    pool: &BTreeMap<Validator, NonNegI64>,
    withdrawers: &BTreeMap<Validator, V>,
    params: &PosParams,
    seed: &EpochSeed,
) -> BTreeMap<Validator, NonNegI64> {
    // Eligible candidates, in the `BTreeMap`'s key order — the canonical container order, which is
    // what makes the draw a function of `(pool, seed)` and nothing else.
    let candidates: Vec<(&Validator, NonNegI64)> = pool
        .iter()
        .filter(|(v, stake)| i64::from(**stake) > 0 && !withdrawers.contains_key(*v))
        .map(|(v, stake)| (v, *stake))
        .collect();
    // `number_of_active_validators` of `0` means unlimited, and a cap that does not bite is not a
    // selection: return the pool as read rather than drawing. Fewer RNG calls, and it keeps `0`
    // meaning "no selection happens" as the params doc says.
    //
    // The `<= 0` arm is load-bearing and was missing in the first draft, which mapped a zero cap to
    // `cap = 0` and drew *nothing* — so a genesis-installed node came up with an empty active set and
    // no validator could propose. Every genesis test failed at once, which is how it was caught.
    if params.number_of_active_validators <= 0 {
        return candidates
            .into_iter()
            .map(|(v, stake)| (*v, stake))
            .collect();
    }
    let cap = usize::try_from(params.number_of_active_validators).unwrap_or(usize::MAX);
    if cap >= candidates.len() {
        return candidates
            .into_iter()
            .map(|(v, stake)| (*v, stake))
            .collect();
    }
    let mut rand = Blake2b512Random::from_init(&seed.rng_input());
    draw_weighted_without_replacement(&candidates, cap, &mut rand)
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
/// (`minimum_bond: NonNegI64::try_from(0).unwrap()`), so a fault is not a rule it can copy, and zero is the only value that leaves
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

    /// **The total REV this node's state holds, over every address it can see** (AUDIT C109).
    ///
    /// This is the invariant whose absence let the negative-`phlo_limit` mint live: the existing
    /// `total_rev` is a *test helper* that sums a caller-supplied list of addresses plus the two named
    /// vaults, so a credit to an address nobody listed is invisible to it. A conservation check that
    /// only sees what the caller thought to name cannot see a mint into a name nobody thought of —
    /// which is exactly what a mint looks like.
    ///
    /// It sums the Coop vault, the staking vault, and **every leaf under `PREFIX_VAULT`** — so a
    /// balance created at an address the test never mentions is counted, and a charge that credits one
    /// vault without debiting another shows up as a change in the total rather than as two numbers
    /// that quietly disagree.
    ///
    /// **Errors when the store has a base history**, because then it cannot see the whole state:
    /// `NativeHistoryReader` exposes a keyed read and no iteration, so after a checkpoint the trie
    /// holds balances this enumeration does not reach. Returning a partial total would be the same
    /// class of defect as the bug it exists to catch — an instrument reporting a number it cannot
    /// support — so it refuses instead. The conservation tests build a store that never checkpoints,
    /// where the overlay is the whole state and the total is complete.
    pub async fn total_value(&self) -> Result<i64, String> {
        if self.store.has_base_history() {
            return Err(
                "total_value: this store has a base history, whose balances cannot be enumerated \
                 (NativeHistoryReader is keyed-read only), so a total over it would silently omit \
                 part of the state"
                    .to_string(),
            );
        }
        let mut total: i64 = i64::from(self.coop_balance().await?)
            .checked_add(i64::from(self.pos_vault_balance().await?))
            .ok_or_else(|| "total_value: overflow summing the named vaults".to_string())?;
        for (key, value) in self.store.live_entries(PREFIX_VAULT) {
            let arr: [u8; 8] = value.as_slice().try_into().map_err(|_| {
                format!(
                    "total_value: vault leaf {} is {} bytes, expected 8",
                    key.to_hex(),
                    value.len()
                )
            })?;
            let balance = NonNegI64::try_from(i64::from_le_bytes(arr))
                .map_err(|_| format!("total_value: vault leaf {} is negative", key.to_hex()))?;
            total = total
                .checked_add(i64::from(balance))
                .ok_or_else(|| "total_value: overflow summing vault balances".to_string())?;
        }
        Ok(total)
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
    ///
    /// `amount: NonNegI64` rather than the `if amount <= 0 { return Ok(()) }` guard this used to
    /// carry: a silent no-op on an out-of-range amount is how the negative-`phlo_limit` mint stayed
    /// invisible (AUDIT C109) — the charge credited the deployer's vault and this function quietly
    /// declined to move the staking vault, so the two halves of one transfer disagreed and nothing
    /// failed. The type now refuses the value instead of absorbing it.
    pub async fn credit_pos_vault(&self, amount: NonNegI64) -> Result<(), String> {
        if amount == NonNegI64::zero() {
            return Ok(());
        }
        let balance = self.pos_vault_balance().await?;
        self.set_pos_vault_balance(balance_plus(
            balance,
            i64::from(amount),
            "pos vault credit",
        )?);
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
    pub async fn debit_pos_vault(&self, amount: NonNegI64) -> Result<(), String> {
        if amount == NonNegI64::zero() {
            return Ok(());
        }
        let balance = self.pos_vault_balance().await?;
        self.set_pos_vault_balance(balance_plus(
            balance,
            -i64::from(amount),
            "pos vault debit",
        )?);
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
        let active = select_active(
            &genesis.bonds,
            &withdrawers,
            &genesis.params,
            &genesis_epoch_seed(),
        );
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
        if stake < i64::from(params.minimum_bond) {
            return Ok(Err(format!(
                "Bond is less than minimum ({} < {}).",
                stake,
                i64::from(params.minimum_bond)
            )));
        }
        if stake > i64::from(params.maximum_bond) {
            return Ok(Err(format!(
                "Bond is greater than maximum ({} > {}).",
                stake,
                i64::from(params.maximum_bond)
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
        self.credit_pos_vault(NonNegI64::try_from(stake).map_err(|e| format!("bond: {e}"))?)
            .await?;
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
        // The **normalised** divisor, not the raw field (AUDIT C135): `epoch_divisor` is what
        // `is_epoch_boundary` divides by, and `epoch_length <= 0` means a one-block epoch, so
        // multiplying the raw field collapsed the quarantine to the absolute constant
        // `quarantine_length` and dropped the epoch offset entirely — a rule applied at one of two
        // sibling epoch sites, which is C109's tell.
        let deadline = checked_i64(
            i128::from(params.quarantine_length)
                + i128::from(epoch_divisor(&params))
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
    /// `fringe_state_hash` is the state hash of the **last finalised fringe** as of this block, and the
    /// close deploy carries it only to be folded into the seed the *next* boundary draws with (step 5).
    /// It is deliberately neither the deploy's `rand` — a function of the block being proposed, so a
    /// free reroll — nor the block's pre-state, which the proposer influences through its justification
    /// set. See `select_active` for the rule and step 5 for why the fringe is the value that closes it.
    pub async fn close_block(
        &self,
        block_number: i64,
        fringe_state_hash: Blake2b256Hash,
    ) -> Result<Result<(), String>, String> {
        let params = self.params().await?;
        let boundary = is_epoch_boundary(&params, block_number);
        // The epoch decision, logged where it is made. Everything about validator membership that is not
        // immediate - activation, rewards, withdrawal processing - happens here and nowhere else, and until
        // this line existed the only way to tell whether it ran was to infer it from a changed active set.
        eprintln!(
            "[pos] close_block {} boundary={} epoch_length={} max_active={} bond=[{}, {}]",
            block_number,
            boundary,
            params.epoch_length,
            params.number_of_active_validators,
            i64::from(params.minimum_bond),
            i64::from(params.maximum_bond)
        );
        if !boundary {
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
            // `spec/audit/passes.md` §6.
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
            self.debit_pos_vault(payable).await?;
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
        //
        // The seed was written one boundary *ago* — at `B_{k-1}`, from `B_{k-1}`'s pre-state — so the
        // block drawing here is not the block whose state chose the entropy. That separation is the
        // whole design; see `select_active` and `docs/src/node/security-audit.md` §8. A state with no
        // seed yet is a genesis-installed one before its first boundary, and falls back to the
        // genesis constant.
        let seed = self.epoch_seed().await?.unwrap_or_else(genesis_epoch_seed);
        let active = select_active(&pool, &withdrawers, &params, &seed);
        self.set_bonds(&pool);
        self.set_withdrawers(&withdrawers);
        self.set_pending_withdrawers(&pending);
        self.set_committed_rewards(&committed);
        self.set_active(&active);

        // 5. The seed the **next** boundary will draw with.
        //
        // **The entropy is the last finalised fringe's state hash — not this block's pre-state.** The
        // pre-state is a function of the justification set, and a proposer has some say in its own
        // justification set: the set is *derived* from the DAG (`get_pre_state_for_new_block` reads
        // `latest_msgs`), so a pre-state cannot be invented, but nothing requires a block to justify
        // everything it has seen — `validate::check_justification_regression` forbids going
        // *backwards* on the messages a block does carry, not omitting them. With the pre-state as an
        // input, a seed-writer therefore enumerates the candidates its own omissions induce, computes
        // the seed and the drawn set for each, and publishes the one it likes: a search, evaluated
        // offline, invisible in the result. The first version of this step used the pre-state and had
        // exactly that hole.
        //
        // The fringe state hash closes it because it is the **>2/3-agreed** object: it is computed from
        // the parents' seen-sets (`message_map::latest_fringe` plus `MergeScope::merge`), so a lone
        // proposer does not move it — omitting a message that has just arrived normally leaves the
        // fringe exactly where it was, which makes the seed *constant* across the candidates the writer
        // can actually choose between. **The residual, stated rather than implied:** the steering space
        // is now the number of *distinct fringes* those candidates induce — normally one, occasionally
        // a handful, if a proposer is willing to present a stale fringe. That is a reduction from "one
        // per subset" to "one per reachable fringe", not a proof of closure, and it is registered with
        // the rest of the rule's residuals in `spec/RUST-VS-SCALA.md` §3 item 12.
        //
        // **Nothing is published and nothing is verified**, which is the other half of the design: the
        // value is derived by each node from its own DAG (`replay_block`'s caller passes
        // `pre_state.fringe_state`; a replay that has no DAG reads the block's own metadata record), so
        // play and replay agree *by construction* rather than by a claim that has to be checked. The
        // block's header does not carry it, because a field the validator recomputes anyway is a second
        // derivation waiting to fall out of step.
        //
        // **The lag is two boundaries, and that is a property rather than a shortfall.** The seed read
        // at `B_k` was written at `B_{k-1}`, so the entropy behind epoch `k`'s draw is the fringe as of
        // `B_{k-1}`. A one-boundary lag is not available: the only values a writer cannot choose are
        // the ones finality has already fixed, and by the next boundary those have moved on. Older
        // entropy costs nothing — it only has to be *fixed* before the drawing proposer acts.
        //
        // Freshness is self-guaranteeing: the previous seed is an input to this one, so consecutive
        // seeds differ even on a chain whose fringe has not moved, and the seed sequence is a hash chain
        // over the fringes — no epoch's entropy can be swapped without changing every seed after it.
        let next_epoch = block_number / epoch_divisor(&params) + 1;
        self.set_epoch_seed(&EpochSeed {
            epoch: next_epoch,
            anchors: vec![previous_seed_anchor(&seed), fringe_state_hash],
        });
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
                epoch_reward(
                    pot,
                    i64::from(params.minimum_bond),
                    active_bonds,
                    i64::from(*stake),
                )?
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
    /// `slash` behavior). A pending withdrawal is cancelled (the stake is forfeited) and the
    /// validator's **accrued** rewards are dropped with it (AUDIT C197).
    ///
    /// Removal is **immediate** in the contract, unlike bonding: the slashing contract deletes the
    /// validator from `activeValidators` and zeroes its bond in the same state update as the transfer
    /// (`Pos.rhox:486-495`), so law 44's epoch gate does not apply to it. What the contract does not
    /// do is take the validator out of `pendingWithdrawers`; the port does, with the same payable
    /// outcome (see the note in `close_block`).
    ///
    /// **The accrued rewards go too, and leaving them behind was a deviation** (C197). `Pos.rhox`'s
    /// state update deletes the offender's `committedRewards` entry in the same write as the bond
    /// zeroing (`committedRewards: … .delete(slashedValidator)`). This port did not, and the entry was
    /// then unreachable in both directions: **never paid**, because `close_block` pays pool members
    /// and claims in `withdrawers` and the slashed validator is out of both, and **never removed**,
    /// because the only `committed.remove` in this file is the claim payment. Since `epoch_pot`
    /// subtracts *every* `committed` entry, the stranded balance went on shrinking the pot for every
    /// other validator for ever.
    ///
    /// **What is confiscated is a *share* of everything the validator holds, and the tier decides how
    /// much** (AUDIT C199). Three holdings are at risk, and all three are in the staking vault: the
    /// pool **bond**, any **escrowed claim** (a staged withdrawal whose bond has already left the pool),
    /// and the **accrued rewards**. The tier takes `basis_points` of their sum to the Coop vault and
    /// **returns the remainder to the validator's own vault**, so the loss is exactly the tier — which
    /// is the property an operator needs before bonding, and the reason a rule that always took
    /// everything made staking a bad deal whatever the reward was.
    ///
    /// Both sides of that are vault-to-vault transfers, so nothing is minted or burned, and
    /// `confiscated + returned == at_risk` exactly: the floor division's remainder is *returned*, not
    /// kept. `SlashSeverity::Unspecified` — a slash recorded before the tiers existed — takes the whole
    /// sum, which is what it would have done.
    pub async fn slash(
        &self,
        validator: &Validator,
        severity: SlashSeverity,
    ) -> Result<Result<(), String>, String> {
        let mut pool = self.bonds().await?;
        let mut active = self.active().await?;
        let mut withdrawers = self.withdrawers().await?;
        let mut pending = self.pending_withdrawers().await?;
        let mut committed = self.committed_rewards().await?;

        let bond = pool.remove(validator).map(i64::from).unwrap_or(0);
        active.remove(validator);
        let escrowed = withdrawers
            .remove(validator)
            .map(|claim| i64::from(claim.bond))
            .unwrap_or(0);
        pending.remove(validator);
        let accrued = committed.remove(validator).map(i64::from).unwrap_or(0);

        let at_risk = checked_i64(
            i128::from(bond) + i128::from(escrowed) + i128::from(accrued),
            "slashed holdings",
        )?;
        let confiscated =
            i64::try_from(i128::from(at_risk) * i128::from(severity.basis_points()) / 10_000)
                .map_err(|e| format!("slash share: {e}"))?;
        let returned = at_risk - confiscated;

        // The confiscated share leaves the staking vault for the Coop multisig vault
        // (`Pos.rhox:470-482`: `posVault!("transfer", coopMultiVaultAddr, valBond, posAuthKey)`).
        // Debiting the source is what makes this a *transfer*: crediting the Coop vault on its own —
        // which is what this did before the staking vault existed — mints the slashed stake from
        // nothing.
        if confiscated > 0 {
            self.debit_pos_vault(NonNegI64::try_from(confiscated).map_err(|e| e.to_string())?)
                .await?;
            let coop = self.coop_balance().await?;
            self.set_coop_balance(balance_plus(coop, confiscated, "slash coop")?);
        }
        // …and the rest is the validator's, back in its own vault.
        if returned > 0 {
            self.debit_pos_vault(NonNegI64::try_from(returned).map_err(|e| e.to_string())?)
                .await?;
            let address = self.vault_address(validator)?;
            let balance = self
                .vault_balance(&address)
                .await?
                .unwrap_or(NonNegI64::zero());
            self.set_vault_balance(&address, balance_plus(balance, returned, "slash return")?);
        }

        self.set_bonds(&pool);
        self.set_active(&active);
        self.set_withdrawers(&withdrawers);
        self.set_committed_rewards(&committed);
        // The pending-withdrawal entry this removes is in `pending`, and a slash that leaves it behind
        // hands it to whatever occupies this validator key next: a later accepted bond is then moved into
        // a claim at the epoch boundary instead of joining the active set. Found by the PoS review, which
        // measured the stale entry surviving slash in 9/9 configurations.
        self.set_pending_withdrawers(&pending);
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
    /// cannot revoke itself.
    ///
    /// **It does not confiscate the target's bond, and that is the fix rather than an omission**
    /// (AUDIT C111). This used to call `slash(target)` whenever the target was bonded, so any single
    /// trusted stakeholder could — with no evidence, no second signature and no delay — move any other
    /// validator's entire stake into the Coop vault. One signature, one deploy, and a peer's bond is
    /// gone; the only gate was "is the caller trusted", which the caller already was by definition.
    ///
    /// Trust is an admission list; a bond is property. Severing the first should not transfer the
    /// second, and confiscation belongs where it is **justified**: a block whose own justifications
    /// hold that validator responsible for an attributable failure, which is now checked on every
    /// validator rather than taken from the proposer (`casper::validate::slashable_senders`,
    /// AUDIT C110). An untrusted-but-bonded validator keeps its stake and can withdraw it, which is
    /// what makes revocation a governance act rather than a robbery. Quorum and delay were the other
    /// candidates; both still end with one actor deciding another's property, just more slowly.
    ///
    /// The `let _ = self.slash(target).await?` this replaces discarded the inner `Result` as well, so a
    /// slash that *failed* — a staking vault that could not cover the transfer — was silently ignored
    /// while the caller was told the revocation had succeeded. Removing the call removes that hazard
    /// with it rather than leaving a second one to fix.
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
    /// carries), and `spec/audit/passes.md` §6 holds it as a registered deviation).
    pub async fn find_or_create_vault(&self, address: &str) -> Result<(), String> {
        if self.vault_balance(address).await?.is_none() {
            self.set_vault_balance(address, NonNegI64::zero());
        }
        Ok(())
    }

    /// The address a **minted handle** opens, or `None` if this name was never handed out.
    ///
    /// `None` is the interesting answer — it is what refuses a caller who presents a name nobody
    /// minted, which is the whole authority check the capability rests on. The bytes are the base58
    /// address, stored as-is; a value that is not valid UTF-8 is treated as absent rather than as an
    /// error, because a corrupted leaf and a forged name are the same answer here (refuse).
    pub async fn vault_name_address(&self, name: &[u8]) -> Result<Option<String>, String> {
        match self
            .store
            .get(PREFIX_VAULT_NAME, &vault_name_key(name))
            .await?
        {
            Some(bytes) => Ok(String::from_utf8(bytes).ok()),
            None => Ok(None),
        }
    }

    /// The address a **minted authority** opens, or `None` if this name was never made an authority.
    ///
    /// This is the spend check's other half: `vault_name_address` answers "which vault does this name
    /// open", this answers "may the holder of this name spend from it". `findOrCreate` writes the
    /// first and never the second, so a handle it mints for someone else's address is not an
    /// authority over that vault.
    pub async fn vault_authority_address(&self, name: &[u8]) -> Result<Option<String>, String> {
        match self
            .store
            .get(PREFIX_VAULT_AUTH, &vault_name_key(name))
            .await?
        {
            Some(bytes) => Ok(String::from_utf8(bytes).ok()),
            None => Ok(None),
        }
    }

    /// Record that `name` **authorises** spends from the vault at `address` — what
    /// `unforgeableAuthKey` writes.
    pub fn set_vault_authority(&self, name: &[u8], address: &str) {
        self.store.put(
            PREFIX_VAULT_AUTH,
            vault_name_key(name),
            address.as_bytes().to_vec(),
        );
    }

    /// Record that `name` opens the vault at `address` — what `findOrCreate` writes when it mints a
    /// handle. Recorded in native state (not in the handler's closure) so the handle still resolves
    /// in a later block and on the replay path, both of which construct a fresh dispatcher.
    pub fn set_vault_name(&self, name: &[u8], address: &str) {
        self.store.put(
            PREFIX_VAULT_NAME,
            vault_name_key(name),
            address.as_bytes().to_vec(),
        );
    }

    /// Move `amount` from one vault to another: the spend rule, in one place.
    ///
    /// **The two callers are two shapes of the same authority** — the classic arm, where `from` comes
    /// from the caller's `deployerId`, and the capability handle, where it comes from the name the
    /// caller presented. They must not be able to disagree about what a legal spend is, so the
    /// movement lives here rather than in each arm.
    ///
    /// The return is deliberately two-layered: `Ok(Err(reason))` is a refusal the *caller* caused and
    /// should see (`(false, reason)` — insufficient balance, a sum that would overflow); `Err(e)` is
    /// the store failing, which is not their fault and must not be dressed as a verdict about their
    /// transfer.
    ///
    /// The self-transfer arm is a no-op **after** the balance check, preserving the arm's existing
    /// behaviour: in the oracle the purse split/deposit nets to zero, but an amount above the balance
    /// must still fail, and without the guard the two writes below would target one leaf.
    pub async fn transfer_vault(
        &self,
        from: &str,
        to: &str,
        amount: NonNegI64,
    ) -> Result<Result<(), String>, String> {
        let from_balance = self.vault_balance(from).await?.unwrap_or(NonNegI64::zero());
        if i64::from(from_balance) < i64::from(amount) {
            return Ok(Err("transfer: insufficient balance".to_string()));
        }
        if from == to {
            return Ok(Ok(()));
        }
        let to_balance = self.vault_balance(to).await?.unwrap_or(NonNegI64::zero());
        let new_from = NonNegI64::try_from(i64::from(from_balance) - i64::from(amount))
            .map_err(|e| e.to_string())?;
        // Accumulate in checked i64 so `to_balance + amount` cannot overflow (both are non-negative,
        // so only an i64::MAX-exceeding sum overflows).
        let new_to_i64 = i64::from(to_balance)
            .checked_add(i64::from(amount))
            .ok_or_else(|| "transfer: destination balance overflow".to_string())?;
        let new_to = NonNegI64::try_from(new_to_i64).map_err(|e| e.to_string())?;
        self.set_vault_balance(from, new_from);
        self.set_vault_balance(to, new_to);
        Ok(Ok(()))
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
    ///
    /// **`amount: NonNegI64` is the load-bearing half of AUDIT C109.** This used to take `i64` and
    /// guard `amount == 0`, so a negative amount walked past every check: the insufficient-funds test
    /// (`balance < amount`) is false for any `NonNegI64` balance against a negative `amount`, and the
    /// subtraction below (`balance - amount`) *adds*, so `NonNegI64::try_from` succeeded and the
    /// deployer's vault was **credited**. `credit_pos_vault` then no-opped on the negative
    /// (`amount <= 0` at its own guard), so the staking vault was untouched: REV minted from nothing.
    /// The sibling [`Self::refund`] guarded `amount <= 0` all along — the asymmetry is what the fix
    /// removes, by making the negative unrepresentable instead of guarded against.
    pub async fn pre_charge(
        &self,
        deployer: &PublicKey,
        amount: NonNegI64,
    ) -> Result<Result<(), String>, String> {
        if amount == NonNegI64::zero() {
            return Ok(Ok(()));
        }
        // Discharged once, into the raw value the balance arithmetic needs. Every use below reads
        // this binding, so there is no second place a sign could enter.
        let raw = i64::from(amount);
        let address = RevAddress::from_public_key(deployer)
            .ok_or_else(|| "preCharge: invalid deployer public key".to_string())?
            .to_base58();
        let balance = match self.vault_balance(&address).await? {
            Some(b) => b,
            None => NonNegI64::try_from(0).map_err(|e| e.to_string())?,
        };
        if i64::from(balance) < raw {
            return Ok(Err(format!(
                "preCharge: insufficient funds ({} < {raw})",
                i64::from(balance)
            )));
        }
        let new_balance =
            NonNegI64::try_from(i64::from(balance) - raw).map_err(|e| format!("preCharge: {e}"))?;
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
        amount: NonNegI64,
    ) -> Result<Result<(), String>, String> {
        if amount == NonNegI64::zero() {
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
        self.set_vault_balance(
            &address,
            balance_plus(balance, i64::from(amount), "refund")?,
        );
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
    /// A `NonNegI64` for test amounts. The charge path takes the refinement since AUDIT C109, so a
    /// test that wants an amount spells it as one — which is the point of the change: there is no way
    /// to write the negative that used to mint.
    fn nn(v: i64) -> NonNegI64 {
        NonNegI64::try_from(v).expect("a test amount is non-negative")
    }

    use super::*;

    fn validator(byte: u8) -> Validator {
        Validator::from_slice(&[byte; 65])
    }

    /// The **fringe** state hash a test's `close_block` is anchored to.
    ///
    /// It only has to be deterministic — play and replay must derive the same seed from it — and
    /// distinct per boundary where a test crosses two; no test here asserts a particular draw from it
    /// except the known-answer one, which spells its own.
    fn fringe_state(byte: u8) -> Blake2b256Hash {
        Blake2b256Hash::create(&[byte])
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

    /// **The conservation invariant behind AUDIT C109, and why it had to be built rather than
    /// asserted.**
    ///
    /// `total_rev` above sums a list the *test* supplies. That is right for the transfers those tests
    /// make between named accounts, and it is **blind by construction** to a credit at an address
    /// nobody named — which is exactly what a mint is. So this test never enumerates the address it
    /// checks: it compares [`NativeSystemState::total_value`] before and after, and an address that
    /// appears out of nowhere moves the total.
    #[tokio::test]
    async fn total_value_sees_a_balance_at_an_address_no_test_named() {
        let native =
            native_with(&[validator(1)], PosParams::default(), &[(validator(1), 10)]).await;
        let before = native.total_value().await.unwrap();

        // An address this test never mentions, credited straight into the store — from the
        // invariant's point of view, exactly the shape a credit to an unlisted account has.
        let ghost = "1111111111111111111111111111111111111111111111111111";
        native.store.put(
            PREFIX_VAULT,
            vault_key(ghost),
            500i64.to_le_bytes().to_vec(),
        );

        assert_eq!(
            native.total_value().await.unwrap(),
            before + 500,
            "a credit at an address nobody named must move the total. This is the assertion \
             `total_rev` could not make, and its absence is why the negative-phlo_limit mint \
             survived: the deployer's vault went up while every check looked at names it had listed"
        );
    }

    /// Every operation that moves value is a **transfer**, so the total is invariant across all of
    /// them: fund, bond, charge, refund, untrust, slash (AUDIT C109, C111).
    ///
    /// Each step asserts rather than only the endpoints, because a defect that both mints and burns
    /// in equal measure nets to zero and would pass an end-to-end check — it is the step that did it
    /// which has to be caught.
    #[tokio::test]
    async fn every_value_moving_operation_conserves_the_total() {
        let native = native_with(
            &[validator(1), validator(2)],
            PosParams::default(),
            &[(validator(1), 10)],
        )
        .await;
        let v2 = validator(2);
        fund(&native, &v2, 100).await;

        // The deployer whose vault will be charged, funded through the store.
        let deployer = PublicKey::new(vec![7u8; 65]);
        let payer = RevAddress::from_public_key(&deployer).unwrap().to_base58();
        native.set_vault_balance(&payer, NonNegI64::try_from(60).unwrap());

        let total = native.total_value().await.unwrap();
        macro_rules! conserved {
            ($what:expr) => {{
                // No `total = after`: the assertion below requires `after == total`, so the
                // reassignment was a no-op — and on this macro's last expansion `clippy::
                // unused_assignments` reads it as dead, which is what removed it. The total is the
                // *same* quantity throughout, and that is the whole content of the test.
                let after = native.total_value().await.unwrap();
                assert_eq!(
                    after, total,
                    "{} moved value — every one of these is a transfer, so the total is invariant",
                    $what
                );
            }};
        }

        native
            .bond(&v2, NonNegI64::try_from(40).unwrap(), 0)
            .await
            .unwrap()
            .unwrap();
        conserved!("bond");

        native
            .pre_charge(&deployer, NonNegI64::try_from(25).unwrap())
            .await
            .unwrap()
            .unwrap();
        conserved!("pre_charge");

        native
            .refund(&deployer, NonNegI64::try_from(10).unwrap())
            .await
            .unwrap()
            .unwrap();
        conserved!("refund");

        // Revoking trust must move **nothing**: it used to confiscate the target's bond (C111).
        native.untrust(&validator(1), &v2).await.unwrap().unwrap();
        conserved!("untrust");

        // A slash leaves the staking vault for the Coop vault.
        native
            .slash(&v2, SlashSeverity::Malicious)
            .await
            .unwrap()
            .unwrap();
        conserved!("slash");
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
            minimum_bond: NonNegI64::try_from(1).unwrap(),
            maximum_bond: NonNegI64::try_from(1000).unwrap(),
            epoch_length: 10,
            quarantine_length: 5,
            number_of_active_validators: 3,
        };
        assert_eq!(decode_params(&params.encode()).unwrap(), params);
    }

    /// **Deferred item 1d, and the shape of the hole it closed.** The wire record held the two bond
    /// bounds as raw `i64`s, so a stored `-1` was representable — and a *negative* minimum accepts
    /// every bond, including the zero-stake one `bond`'s range check exists to refuse. The bounds are
    /// `NonNegI64` now, and this is the boundary that makes that true for values that arrive as
    /// bytes: a negative one is refused, not clamped, because clamping a corrupted leaf to zero turns
    /// it into "no minimum", which is the permissive reading of the same bytes.
    ///
    /// Each field is exercised on its own, so this is a test of two checks rather than of one that
    /// happens to run first, and the control is the untouched record — a refusal function that
    /// refused everything would satisfy the other two assertions.
    #[test]
    fn a_negative_bond_bound_is_refused_at_the_wire() {
        let params = PosParams {
            minimum_bond: NonNegI64::try_from(7).unwrap(),
            maximum_bond: NonNegI64::try_from(1000).unwrap(),
            epoch_length: 10,
            quarantine_length: 5,
            number_of_active_validators: 3,
        };
        assert!(
            decode_params(&params.encode()).is_ok(),
            "the control: an in-range record still decodes"
        );

        let mut bytes = params.encode();
        bytes[0..8].copy_from_slice(&(-1i64).to_le_bytes());
        let err = decode_params(&bytes).expect_err("a negative minimum must not decode");
        assert!(
            err.contains("minimum_bond"),
            "and the refusal names the field that was wrong: {err}"
        );

        let mut bytes = params.encode();
        bytes[8..16].copy_from_slice(&(-1i64).to_le_bytes());
        let err = decode_params(&bytes).expect_err("a negative maximum must not decode");
        assert!(
            err.contains("maximum_bond"),
            "and the refusal names the field that was wrong: {err}"
        );
    }

    #[test]
    fn select_active_draws_a_deterministic_subset_of_the_cap() {
        // **This test used to assert `vec![validator(2), validator(3)]` — the top two by stake.**
        // That rule is gone; which validators are active is now a draw, so pinning a specific set here
        // would only pin the seed. What is pinned instead is the shape the rule guarantees for *any*
        // seed, and the exact set is pinned once, by the known-answer vector below.
        let pool = pool_of(&[(1, 10), (2, 30), (3, 20)]);
        let params = PosParams {
            number_of_active_validators: 2,
            ..PosParams::default()
        };
        let seed = genesis_epoch_seed();

        let first = select_active(&pool, &BTreeMap::<Validator, ()>::new(), &params, &seed);
        let again = select_active(&pool, &BTreeMap::<Validator, ()>::new(), &params, &seed);

        assert_eq!(
            first, again,
            "the same pool and seed must draw the same set"
        );
        assert_eq!(first.len(), 2, "the cap is respected");
        for (v, stake) in &first {
            assert_eq!(
                pool.get(v),
                Some(stake),
                "a drawn validator carries its pool stake, not a synthesised one"
            );
        }
    }

    /// **Distinct seeds must be able to draw distinct sets.** This is the property the old rule
    /// lacked in kind: it had exactly one answer for a given pool, so nothing could ever rotate. A
    /// draw that returned the same set for every seed would pass every other test here.
    #[test]
    fn distinct_seeds_draw_distinct_sets() {
        let pool = pool_of(&[(1, 10), (2, 20), (3, 30), (4, 40), (5, 50), (6, 60)]);
        let params = PosParams {
            number_of_active_validators: 3,
            ..PosParams::default()
        };
        let mut seen: BTreeSet<Vec<Validator>> = BTreeSet::new();
        for epoch in 0..64i64 {
            let seed = EpochSeed {
                epoch,
                anchors: vec![Blake2b256Hash::create(&epoch.to_le_bytes())],
            };
            seen.insert(
                select_active(&pool, &BTreeMap::<Validator, ()>::new(), &params, &seed)
                    .keys()
                    .copied()
                    .collect(),
            );
        }
        assert!(
            seen.len() > 1,
            "64 seeds produced one set: this is not a draw"
        );
    }

    /// **A zero cap means unlimited, and this is a regression test by name.** The first draft mapped
    /// it to `cap = 0` and drew *nothing*, so a genesis-installed node came up with an empty active
    /// set and no validator could propose. The old truncate-when-positive rule could not have had
    /// this bug, which is what makes it the shape a draw introduces.
    #[test]
    fn a_zero_cap_returns_the_whole_pool() {
        let pool = pool_of(&[(1, 10), (2, 20), (3, 30)]);
        let params = PosParams {
            number_of_active_validators: 0,
            ..PosParams::default()
        };
        let active = select_active(
            &pool,
            &BTreeMap::<Validator, ()>::new(),
            &params,
            &genesis_epoch_seed(),
        );
        assert_eq!(active.len(), 3, "zero means unlimited, not 'draw none'");
    }

    /// **A known-answer vector: this pool, this seed, exactly this set.**
    ///
    /// Every other test here asserts a *property*, so a refactor that changed the draw would satisfy
    /// all of them — only a full chain replay would notice. This pins the output itself, in the style
    /// of `Blake2b512Random`'s own vectors.
    ///
    /// **The vector moved on 2026-10-02 and that is the point of it.** It read
    /// `[1, 3, 5]` under the uniform draw; the weighted rule draws `[2, 3, 5]` — it takes the stake-20
    /// validator where the uniform rule took the stake-10 one. Both are the same kind of statement: a
    /// change here is a consensus change.
    #[test]
    fn the_draw_matches_a_known_answer_vector() {
        let pool = pool_of(&[(1, 10), (2, 20), (3, 30), (4, 40), (5, 50)]);
        let params = PosParams {
            number_of_active_validators: 3,
            ..PosParams::default()
        };
        let active = select_active(
            &pool,
            &BTreeMap::<Validator, ()>::new(),
            &params,
            &genesis_epoch_seed(),
        );
        assert_eq!(
            active.keys().copied().collect::<Vec<_>>(),
            vec![validator(2), validator(3), validator(5)],
            "the draw for this pool and seed; a change here is a consensus change"
        );
    }

    /// **The book's "cap bites" table, re-measured on the weighted rule — and the direction of the
    /// reversal has flipped.**
    ///
    /// `epoch_rewards` pays a pooled validator `bond / active_bonds` of the pot when it is drawn and
    /// zero otherwise, so a staker's expected income is the drawn holdings over the drawn total,
    /// averaged over seeds. The published table (pot 1, six rival keys at stake 10, cap 4) read
    /// **0.326 / 0.400 / 0.528** for one stake of 40 held as one key, four keys of 10 and twenty keys of
    /// 2 — splitting bought 62 %, holding whole was *penalised*, and every key was an independent
    /// lottery ticket. That is the sybil exposure O3 named.
    ///
    /// On the weighted rule the same measurement reads **0.5285 / 0.4005 / 0.1685**. Splitting a stake
    /// into twenty keys now **loses** 58 % of the fair share instead of gaining 32 %, so the per-key
    /// lever is gone — and the reason the four-equal-keys case sits exactly at the pro-rata 0.4 is that
    /// it *is* the rivals' configuration, which is the cleanest statement of proportionality available.
    ///
    /// **The concentration side of this is real and is stated rather than hidden.** Above the cap, a
    /// single large key earns ~32 % *more* than the flat pro-rata share, because the drawn set is capped
    /// at four and a large key both draws more often and crowds the denominator when it does. The
    /// uniform rule had the same magnitude pointing the other way. Neither is "pro-rata"; the cap is
    /// what breaks proportionality, and the two rules differ only in which side of it a staker is on.
    /// Weighting is the side chosen, for two reasons: a per-key income rule *mints finality weight for
    /// free* (a consensus-safety problem, not a preference), and a large bond is already the thing the
    /// cap exists to bound — a bond is bounded above by `maximum_bond`, and the operator carries the
    /// concentration risk itself, since everything at risk is per-validator. The seeds are fixed, so
    /// these are pins rather than samples.
    #[test]
    fn the_cap_regime_no_longer_rewards_a_split_stake() {
        let params = PosParams {
            number_of_active_validators: 4,
            ..PosParams::default()
        };
        let seeds = 4096i64;
        // Six rivals of stake 10, plus the holder's 40 however it is held: every configuration has the
        // same total weight, so the same pool total and the same denominator distribution.
        let measure = |held: &[i64]| -> f64 {
            let mut pool = pool_of(&[(1, 10), (2, 10), (3, 10), (4, 10), (5, 10), (6, 10)]);
            let mut held_keys = BTreeSet::new();
            for (index, stake) in held.iter().enumerate() {
                let address = u8::try_from(100 + index).expect("a test key");
                let key = validator(address);
                pool.insert(key, NonNegI64::try_from(*stake).expect("a test stake"));
                held_keys.insert(key);
            }
            let mut total = 0.0;
            for epoch in 0..seeds {
                let seed = EpochSeed {
                    epoch,
                    anchors: vec![Blake2b256Hash::create(&epoch.to_le_bytes())],
                };
                let active =
                    select_active(&pool, &BTreeMap::<Validator, ()>::new(), &params, &seed);
                let drawn_total: i64 = active.values().map(|s| i64::from(*s)).sum();
                let drawn_holdings: i64 = active
                    .iter()
                    .filter(|(v, _)| held_keys.contains(v))
                    .map(|(_, s)| i64::from(*s))
                    .sum();
                if drawn_total > 0 {
                    total += drawn_holdings as f64 / drawn_total as f64;
                }
            }
            total / seeds as f64
        };

        let whole = measure(&[40]);
        let four = measure(&[10, 10, 10, 10]);
        let twenty = measure(&[2; 20]);
        for (label, value, expected) in [
            ("one key of 40", whole, 0.5285),
            ("four keys of 10", four, 0.4005),
            ("twenty keys of 2", twenty, 0.1685),
        ] {
            assert!(
                (value - expected).abs() < 0.005,
                "{label} earned {value}, not {expected}: the weights moved, so the rule moved"
            );
        }
        assert!(
            whole > four && four > twenty,
            "income must not fall as a stake is held in fewer keys — the uniform rule had it the other \
             way round, which is the defect; got {whole}, {four}, {twenty}"
        );
        assert!(
            (four - 0.4).abs() < 0.005,
            "a staker that looks exactly like its rivals must earn exactly the pro-rata share; got {four}"
        );
    }

    /// How often, over `seeds` epochs, the single slot of a one-member draw goes to `target`.
    ///
    /// A *measurement*, in the register's sense: the drawn set is a deterministic function of the seed,
    /// so "in proportion to stake" is a property of the distribution over seeds rather than of one
    /// call, and the only way to state it is to draw many. The seeds are fixed constants, so the
    /// measurement is reproducible.
    fn slot_share(pool: &BTreeMap<Validator, NonNegI64>, target: &Validator, seeds: i64) -> f64 {
        let params = PosParams {
            number_of_active_validators: 1,
            ..PosParams::default()
        };
        let hits = (0..seeds)
            .filter(|epoch| {
                let seed = EpochSeed {
                    epoch: *epoch,
                    anchors: vec![Blake2b256Hash::create(&epoch.to_le_bytes())],
                };
                select_active(pool, &BTreeMap::<Validator, ()>::new(), &params, &seed)
                    .contains_key(target)
            })
            .count();
        hits as f64 / seeds as f64
    }

    /// **The B1 falsifier: the one slot is awarded in proportion to stake, not per key.**
    ///
    /// Three parts of stake against one. Under the uniform rule this was `0.5` — the ratio of *keys* —
    /// and that is the whole defect: a staker could split its stake across keys and buy slots, because
    /// each key was a ticket regardless of what it was worth. Under the weighted rule it is `0.75`,
    /// the ratio of stake.
    ///
    /// The tolerance is four standard deviations of the binomial at 512 trials, so this is a
    /// measurement of the rule rather than a flake: a correct rule misses the band about once in
    /// sixteen thousand runs, and restoring the uniform draw moves the value to `0.5`, far outside it.
    #[test]
    fn the_first_slot_is_awarded_in_proportion_to_stake() {
        let pool = pool_of(&[(1, 3), (2, 1)]);
        let share = slot_share(&pool, &validator(1), 512);
        assert!(
            (0.70..=0.80).contains(&share),
            "a three-to-one stake must take the slot about three times in four; got {share}"
        );
    }

    /// **And splitting a stake across keys does not buy slots** — the same measurement, stated the way
    /// a staker would try it.
    ///
    /// The stake is 4 either way. Held as one key against four equal keys it takes about half the
    /// draws; split into four keys of 1 against the same four, the *four keys together* still take
    /// about half — not four fifths, which is what a per-key rule would give them.
    #[test]
    fn splitting_a_stake_across_keys_does_not_buy_slots() {
        let whole = pool_of(&[(1, 4), (2, 1), (3, 1), (4, 1), (5, 1)]);
        let split = pool_of(&[
            (1, 1),
            (2, 1),
            (3, 1),
            (4, 1),
            (5, 1),
            (6, 1),
            (7, 1),
            (8, 1),
        ]);
        let params = PosParams {
            number_of_active_validators: 1,
            ..PosParams::default()
        };
        let split_group: BTreeSet<Validator> = (1..=4).map(validator).collect();
        let seeds = 512i64;

        let wins = |pool: &BTreeMap<Validator, NonNegI64>, group: &BTreeSet<Validator>| {
            (0..seeds)
                .filter(|epoch| {
                    let seed = EpochSeed {
                        epoch: *epoch,
                        anchors: vec![Blake2b256Hash::create(&epoch.to_le_bytes())],
                    };
                    select_active(pool, &BTreeMap::<Validator, ()>::new(), &params, &seed)
                        .keys()
                        .any(|v| group.contains(v))
                })
                .count() as f64
                / seeds as f64
        };

        let whole_share = wins(&whole, &BTreeSet::from([validator(1)]));
        let split_share = wins(&split, &split_group);
        assert!(
            (0.45..=0.55).contains(&whole_share),
            "half the stake takes about half the draws; got {whole_share}"
        );
        assert!(
            (0.40..=0.60).contains(&split_share),
            "the same half of the stake, split across four keys, must still take about half the \
             draws — a per-key rule gives it four fifths; got {split_share}"
        );
    }

    /// A pool whose bonds are the given `(index, stake)` pairs.
    fn pool_of(bonds: &[(u8, i64)]) -> BTreeMap<Validator, NonNegI64> {
        bonds
            .iter()
            .map(|(i, stake)| {
                (
                    validator(*i),
                    NonNegI64::try_from(*stake).expect("test stake is non-negative"),
                )
            })
            .collect()
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
        native
            .close_block(1, fringe_state(1))
            .await
            .unwrap()
            .unwrap();
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
            minimum_bond: NonNegI64::try_from(10).unwrap(),
            maximum_bond: NonNegI64::try_from(50).unwrap(),
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

        native
            .close_block(1, fringe_state(1))
            .await
            .unwrap()
            .unwrap();
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
        native
            .close_block(10, fringe_state(10))
            .await
            .unwrap()
            .unwrap();
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

        native
            .close_block(16, fringe_state(16))
            .await
            .unwrap()
            .unwrap();
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

    /// **AUDIT C135.** The withdrawal deadline uses the **normalised** epoch divisor, so
    /// `epoch_length <= 0` — which every other epoch site reads as a one-block epoch — cannot
    /// collapse the quarantine to an absolute constant.
    ///
    /// The regression is exactly that collapse, and it is why the two arms are *compared* rather
    /// than each asserted alone: with the raw field multiplied, `epoch_length = 0` gave
    /// `quarantine_length` while `epoch_length = 1` gave `quarantine_length + 1 + block_number`, so
    /// the quarantine the row is about was silently dropped for one setting and honoured for the
    /// other. The concrete value then pins the formula itself, so "both arms agree" cannot be
    /// satisfied by both being wrong the same way.
    #[tokio::test]
    async fn a_non_positive_epoch_length_keeps_the_withdrawal_quarantine() {
        let v = validator(3);
        let params = |epoch_length: i64| PosParams {
            epoch_length,
            quarantine_length: 100,
            ..PosParams::default()
        };

        let zero = native_with(&[v], params(0), &[(v, 100)]).await;
        zero.withdraw(&v, 10).await.unwrap().unwrap();
        let staged_at_zero = zero.pending_withdrawers().await.unwrap()[&v];

        let one = native_with(&[v], params(1), &[(v, 100)]).await;
        one.withdraw(&v, 10).await.unwrap().unwrap();
        let staged_at_one = one.pending_withdrawers().await.unwrap()[&v];

        assert_eq!(
            staged_at_zero, staged_at_one,
            "a zero epoch length means a one-block epoch at the withdrawal deadline as it does at \
             every other epoch site"
        );
        assert_eq!(
            staged_at_one,
            100 + 1 + 10,
            "the deadline is quarantine_length + divisor * (1 + block_number)"
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
        native.pre_charge(&payer, nn(10)).await.unwrap().unwrap();
        native.withdraw(&v, 3).await.unwrap().unwrap();
        let before = (
            native.pending_withdrawers().await.unwrap(),
            native.committed_rewards().await.unwrap(),
            native.bonds().await.unwrap(),
            native.active().await.unwrap(),
        );

        // 7 is not a multiple of 10: nothing happens.
        native
            .close_block(7, fringe_state(7))
            .await
            .unwrap()
            .unwrap();
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
        native
            .close_block(10, fringe_state(10))
            .await
            .unwrap()
            .unwrap();
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
        native
            .close_block(9, fringe_state(9))
            .await
            .unwrap()
            .unwrap();
        assert!(
            !native.active().await.unwrap().contains_key(&v2),
            "9 is not a boundary either"
        );
        native
            .close_block(10, fringe_state(10))
            .await
            .unwrap()
            .unwrap();
        assert!(
            native.active().await.unwrap().contains_key(&v2),
            "the boundary is where a bond becomes a validator"
        );
    }

    /// **Laws 45 and 46 — the split, checked against the model rather than a remembered number.**
    /// `Rchain/Pos.lean`'s `the_dust_is_real` is the `decide`d case `minimumBond = 3`, bonds
    /// `[4, 8]`, pot `10`: the normaliser is `12 / 3 = 4`, the scaled shares are `4 / 3 = 1` and
    /// `8 / 3 = 2`, and the validators are paid `10 * 1 / 4 = 2` and `10 * 2 / 4 = 5` — **7
    /// distributed of 10**, the rest being the dust of two integer divisions. This test builds exactly
    /// that state and reads the split back, so the implementation is checked against the arithmetic the
    /// Lean proves rather than against itself.
    ///
    /// **Why these bonds and not a smaller pair (AUDIT C149).** Bonds `[4, 5]` were the fixture until
    /// this was measured: both are `1` after the integer division by `minimum_bond = 3`, so the
    /// proportionality factor `bond / minimum_bond` was the *identity* for every validator the test
    /// built, and deleting it from `epoch_reward` left this test green — two deliberately different
    /// bonds producing the same share by construction. `4` and `8` straddle the divisor, so the factor
    /// is `1` for one validator and `2` for the other, and a split that ignores the bond is visible.
    /// `the_dust_is_real` carried the identical degeneracy over the same instance; it was moved to
    /// this one in the same pass, because a row certified on one side and unobservable on the other is
    /// not certified at all.
    #[tokio::test]
    async fn an_epoch_splits_the_pot_and_keeps_the_dust() {
        let params = PosParams {
            minimum_bond: NonNegI64::try_from(3).unwrap(),
            epoch_length: 1,
            ..PosParams::default()
        };
        let native = native_with(
            &[validator(1), validator(2)],
            params,
            &[(validator(1), 4), (validator(2), 8)],
        )
        .await;
        // Fill the pot with exactly 10 the way a deploy's phlo does.
        let payer = PublicKey::new(vec![9u8; 65]);
        let payer_addr = RevAddress::from_public_key(&payer).unwrap().to_base58();
        native.set_vault_balance(&payer_addr, NonNegI64::try_from(10).unwrap());
        native.pre_charge(&payer, nn(10)).await.unwrap().unwrap();
        assert_eq!(
            i64::from(native.pos_vault_balance().await.unwrap()),
            12 + 10,
            "the vault holds the bonds plus the phlo"
        );

        native
            .close_block(1, fringe_state(1))
            .await
            .unwrap()
            .unwrap();

        let committed = native.committed_rewards().await.unwrap();
        assert_eq!(
            i64::from(committed[&validator(1)]),
            2,
            "10 * (4 / 3) / (12 / 3) = 2"
        );
        assert_eq!(
            i64::from(committed[&validator(2)]),
            5,
            "10 * (8 / 3) / (12 / 3) = 5"
        );
        assert!(
            i64::from(committed[&validator(1)]) + i64::from(committed[&validator(2)]) < 10,
            "the shares do not sum to the pot — that inequality is the law"
        );
        assert_eq!(
            i64::from(native.pos_vault_balance().await.unwrap()),
            22,
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
            3,
            "the three units of dust stay in the pot, and the next epoch distributes them"
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
            minimum_bond: NonNegI64::try_from(3).unwrap(),
            ..PosParams::default()
        };
        let native = native_with(&[validator(1)], params, &[(validator(1), 40)]).await;
        let v = validator(1);
        native.set_vault_balance(&vault_address_of(&v), NonNegI64::zero());
        // 5 of phlo for the epoch, charged to a deployer.
        let payer = PublicKey::new(vec![9u8; 65]);
        let payer_addr = RevAddress::from_public_key(&payer).unwrap().to_base58();
        native.set_vault_balance(&payer_addr, NonNegI64::try_from(5).unwrap());
        native.pre_charge(&payer, nn(5)).await.unwrap().unwrap();

        // Stage the request, then close the block: the boundary pays the epoch's reward into the
        // committed map *and* moves the validator out of the pool, in that order.
        native.withdraw(&v, 1).await.unwrap().unwrap();
        native
            .close_block(1, fringe_state(1))
            .await
            .unwrap()
            .unwrap();
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
        native
            .close_block(2, fringe_state(2))
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
        native.pre_charge(&payer, nn(5)).await.unwrap().unwrap();

        native
            .close_block(1, fringe_state(1))
            .await
            .unwrap()
            .unwrap();
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
                minimum_bond: NonNegI64::try_from(100).unwrap(),
                epoch_length: 1,
                ..PosParams::default()
            },
            &[(validator(1), 40)],
        )
        .await;
        let native = native;
        native.set_vault_balance(&payer_addr, NonNegI64::try_from(5).unwrap());
        native.pre_charge(&payer, nn(5)).await.unwrap().unwrap();
        native
            .close_block(1, fringe_state(1))
            .await
            .unwrap()
            .unwrap();
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
                minimum_bond: NonNegI64::try_from(3).unwrap(),
                ..PosParams::default()
            },
            &[(validator(1), 40)],
        )
        .await;
        // Drain the vault below the bonded pool: a ledger that has already diverged.
        native.debit_pos_vault(nn(35)).await.unwrap();
        assert_eq!(i64::from(native.pos_vault_balance().await.unwrap()), 5);

        native
            .close_block(1, fringe_state(1))
            .await
            .unwrap()
            .unwrap();
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

        native
            .slash(&v, SlashSeverity::Unspecified)
            .await
            .unwrap()
            .unwrap();

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

    /// The current epoch pot, read the way `close_block` reads it.
    async fn pot_of(native: &NativeSystemState) -> i64 {
        epoch_pot(
            native.pos_vault_balance().await.unwrap(),
            &native.bonds().await.unwrap(),
            &native.withdrawers().await.unwrap(),
            &native.committed_rewards().await.unwrap(),
        )
        .unwrap()
    }

    /// **The C197 falsifier, restated for the tiers: a slash clears the offender's accrued rewards
    /// and *accounts* for them.** `Pos.rhox` deletes the entry; this port left it, and it was then
    /// unreachable both ways — never paid (the epoch pays pool members and claims in `withdrawers`,
    /// and the validator is out of both) and never removed (the only `committed.remove` is the claim
    /// payment) — so the balance sat in the staking vault shrinking the pot for every *other*
    /// validator, for ever.
    ///
    /// The consequence is now visible in two places at once, and they must agree: the entry is gone,
    /// **and** the amount left the staking vault — to the Coop vault under `Unspecified`, which is the
    /// pre-tier rule and takes everything. Red before C197: the entry survived and the Coop vault did
    /// not move by it.
    #[tokio::test]
    async fn a_slash_clears_the_accrued_rewards_and_accounts_for_them() {
        let native =
            native_with(&[validator(1)], PosParams::default(), &[(validator(1), 10)]).await;
        let v = validator(1);
        // Some phlo burned since the last boundary, so the pot is positive.
        native
            .credit_pos_vault(NonNegI64::try_from(30).unwrap())
            .await
            .unwrap();
        // …and an epoch's share this validator has earned and not withdrawn.
        let accrued = NonNegI64::try_from(7).unwrap();
        native.set_committed_rewards(&BTreeMap::from([(v, accrued)]));

        let pot_before = pot_of(&native).await;
        assert_eq!(
            pot_before, 23,
            "40 of vault (10 bonded + 30 burned) less the 10 bonded less the 7 accrued"
        );

        native
            .slash(&v, SlashSeverity::Unspecified)
            .await
            .unwrap()
            .unwrap();

        assert!(
            !native.committed_rewards().await.unwrap().contains_key(&v),
            "the offender's accrued rewards go with the bond"
        );
        assert_eq!(
            i64::from(native.coop_balance().await.unwrap()),
            17,
            "the bond (10) and the accrued (7) both left for the Coop vault — the pre-tier rule \
             takes everything at risk"
        );
        assert_eq!(
            pot_of(&native).await,
            pot_before,
            "and the pot is unchanged: the vault loses exactly what the claims lose, so a slash \
             neither shrinks nor subsidises what the epoch distributes"
        );
    }

    /// **The A3 falsifier, first tier: a misdemeanour takes a quarter and gives the rest back.**
    ///
    /// Everything the validator holds in the PoS system is at risk — the bond, the accrued rewards,
    /// and an escrowed withdrawal claim — and the tier takes a share of *that sum* while the
    /// remainder returns to the validator's own vault. The two halves must add up: the floor
    /// division's remainder is returned, never kept, so the loss is exactly the tier.
    ///
    /// Red before C199: the whole bond went to the Coop vault and nothing came back.
    #[tokio::test]
    async fn a_misdemeanour_takes_a_quarter_and_returns_the_rest() {
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
        // An accrual that does not divide evenly by four, so the floor's remainder is exercised.
        native.set_committed_rewards(&BTreeMap::from([(v, NonNegI64::try_from(7).unwrap())]));
        native
            .credit_pos_vault(NonNegI64::try_from(1_000).unwrap())
            .await
            .unwrap();

        let wallet_before = i64::from(
            native
                .vault_balance(&vault_address_of(&v))
                .await
                .unwrap()
                .unwrap_or(NonNegI64::zero()),
        );
        let at_risk = 40 + 7;
        let confiscated = at_risk * 2_500 / 10_000; // 11
        let returned = at_risk - confiscated; // 36

        native
            .slash(&v, SlashSeverity::Misdemeanour)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(
            i64::from(native.coop_balance().await.unwrap()),
            confiscated,
            "a quarter of everything at risk, floored"
        );
        let wallet_after = i64::from(
            native
                .vault_balance(&vault_address_of(&v))
                .await
                .unwrap()
                .unwrap_or(NonNegI64::zero()),
        );
        assert_eq!(
            wallet_after,
            wallet_before + returned,
            "and the remainder is the validator's again — including the floor's remainder, so \
             confiscated + returned is exactly what was at risk"
        );
        assert!(!native.bonds().await.unwrap().contains_key(&v));
        assert!(
            !native.committed_rewards().await.unwrap().contains_key(&v),
            "the validator is out of the pool and its claims are cleared either way"
        );
    }

    /// **The tier a validator answers for is the worst it committed.** Two failed blocks from one
    /// sender — a tenth and a quarter — cost a quarter, not two tenths and not the whole bond.
    ///
    /// And the *scope* is every holding, not the bond alone: an escrowed claim is at risk too, which
    /// is the half a bond-only cap would have missed (and which used to be forfeited outright, since
    /// `slash` removed the claim and nothing paid it).
    #[tokio::test]
    async fn a_slash_counts_the_escrowed_claim_and_takes_the_worst_tier() {
        let native = native_with(
            &[validator(1), validator(2)],
            PosParams::default(),
            &[(validator(1), 10)],
        )
        .await;
        let v = validator(2);
        fund(&native, &v, 100).await;
        native
            .bond(&v, NonNegI64::try_from(30).unwrap(), 0)
            .await
            .unwrap()
            .unwrap();
        // Staged and past a boundary: 20 of the bond has left the pool and is escrowed, so what the
        // validator has in the PoS system is a 10 bond plus a 20 claim.
        native.set_bonds(&BTreeMap::from([
            (validator(1), NonNegI64::try_from(10).unwrap()),
            (v, NonNegI64::try_from(10).unwrap()),
        ]));
        native.set_withdrawers(&BTreeMap::from([(
            v,
            Withdrawal {
                bond: NonNegI64::try_from(20).unwrap(),
                deadline: 10_000,
            },
        )]));
        native
            .credit_pos_vault(NonNegI64::try_from(1_000).unwrap())
            .await
            .unwrap();

        // Bond 10 + claim 20 = 30 at risk; a quarter is 7.
        native
            .slash(&v, SlashSeverity::Misdemeanour)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(
            i64::from(native.coop_balance().await.unwrap()),
            7,
            "the escrowed claim is part of what the tier is a share of — a bond-only cap would have \
             missed it, and before C199 the claim was forfeited outright"
        );
        assert!(
            !native.withdrawers().await.unwrap().contains_key(&v),
            "and the claim is cleared with the rest of the validator's records"
        );
    }

    #[tokio::test]
    async fn untrust_removes_trust_and_leaves_the_bond_alone() {
        // This test used to be `untrust_removes_and_confiscates` and asserted the opposite of its
        // last two lines: that the bond was gone and the Coop vault was up by the stake. It passed
        // for as long as the behaviour did, which is the point — the test was written from the code
        // rather than from the rule, so it could not see that one trusted stakeholder had been given
        // a one-signature power to take any peer's stake (AUDIT C111). The assertions below are the
        // rule: trust is admission, a bond is property, and revocation moves only the first.
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

        let coop_before = i64::from(native.coop_balance().await.unwrap());
        let vault_before = i64::from(native.pos_vault_balance().await.unwrap());

        native.untrust(&validator(1), &v).await.unwrap().unwrap();

        assert!(
            !native.trusted().await.unwrap().contains(&v),
            "untrust must remove the target from the trusted set"
        );
        assert!(
            native.bonds().await.unwrap().contains_key(&v),
            "untrust must NOT confiscate the target's bond — the stake is the validator's property, \
             and taking it was a one-signature robbery available to any single trusted stakeholder"
        );
        assert_eq!(
            i64::from(native.coop_balance().await.unwrap()),
            coop_before,
            "the Coop vault must not be enriched by a revocation"
        );
        assert_eq!(
            i64::from(native.pos_vault_balance().await.unwrap()),
            vault_before,
            "and no value may move out of the staking vault: a revocation is not a transfer"
        );
        assert_eq!(
            i64::from(
                native
                    .vault_balance(&native.vault_address(&v).unwrap())
                    .await
                    .unwrap()
                    .unwrap()
            ),
            60,
            "the validator keeps what it did not bond, so it can still withdraw the stake"
        );
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
        // Which two are active is a draw now, so this pins the *cap* and the *provenance* rather
        // than the set: exactly two, all from the pool. The exact set is pinned by the
        // known-answer vector beside `select_active`.
        assert_eq!(active.len(), 2, "the cap is respected");
        assert!(
            active
                .iter()
                .all(|v| [validator(1), validator(2), validator(3)].contains(v)),
            "every active validator comes from the pool: {active:?}"
        );
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
        native
            .close_block(1, fringe_state(1))
            .await
            .unwrap()
            .unwrap();
        native
            .close_block(2, fringe_state(2))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(native.active_validators().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn pre_charge_deducts_and_rejects_insufficient() {
        let native = NativeSystemState::new(Arc::new(InMemNativeStore::empty()));
        let pk = PublicKey::new(vec![1u8; 65]);
        let addr = RevAddress::from_public_key(&pk).unwrap().to_base58();
        native.set_vault_balance(&addr, NonNegI64::try_from(100).unwrap());

        // Deduct 40 -> 60.
        native.pre_charge(&pk, nn(40)).await.unwrap().unwrap();
        assert_eq!(
            i64::from(native.vault_balance(&addr).await.unwrap().unwrap()),
            60
        );

        // Deducting more than the balance fails via the user-error branch.
        let result = native.pre_charge(&pk, nn(100)).await.unwrap();
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
        native.pre_charge(&pk, nn(100)).await.unwrap().unwrap();
        assert_eq!(i64::from(native.pos_vault_balance().await.unwrap()), 100);
        assert_eq!(
            i64::from(native.vault_balance(&addr).await.unwrap().unwrap()),
            0
        );
        native.refund(&pk, nn(70)).await.unwrap().unwrap();

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

        // A zero refund succeeds without moving anything (`Pos.rhox:426`'s guard).
        native.refund(&pk, nn(0)).await.unwrap().unwrap();
        // **The negative case is gone, and its absence is the fix** (AUDIT C109). This line used to
        // read `native.refund(&pk, -5)` and assert the no-op — which was the sibling of the very bug
        // that minted: `refund` happened to guard `amount <= 0` while `pre_charge` guarded only
        // `amount == 0`, and the asymmetry was invisible because both were tested only for the
        // behaviour they already had. A negative amount is now unrepresentable, so there is no
        // runtime assertion to write; the compile-time one is that `nn` cannot produce it.
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
            native.refund(&pk, nn(1)).await.is_err(),
            "a refund cannot come out of an empty vault"
        );
        assert!(native.debit_pos_vault(nn(1)).await.is_err());
        assert_eq!(
            i64::from(native.pos_vault_balance().await.unwrap()),
            0,
            "a refused debit leaves the vault untouched"
        );
    }

    /// **A slash must not leave the pending withdrawal it removes behind.**
    ///
    /// `slash` deletes the validator's pending-withdrawal entry from a local copy of the map. Forgetting to
    /// write that copy back leaves the stale entry in the store, and the next key to occupy the validator's
    /// place - a later accepted bond, say - inherits it and is moved into a claim at the epoch boundary
    /// instead of joining the active set. Found by the PoS review, which measured the entry surviving slash
    /// in 9/9 configurations; this is the falsifier.
    #[tokio::test]
    async fn slash_persists_the_pending_map_it_edits() {
        let bonded = validator(1);
        let slashed = validator(2);
        let native = native_with(
            &[bonded],
            PosParams::default(),
            &[(bonded, 100), (slashed, 100)],
        )
        .await;

        native.set_pending_withdrawers(&BTreeMap::from([(slashed, 100)]));
        assert!(
            native
                .pending_withdrawers()
                .await
                .unwrap()
                .contains_key(&slashed),
            "the staged withdrawal must be in the store before the slash"
        );

        native
            .slash(&slashed, SlashSeverity::Malicious)
            .await
            .unwrap()
            .unwrap();

        assert!(
            !native.pending_withdrawers().await.unwrap().contains_key(&slashed),
            "slash removed the entry in memory and must persist that: otherwise the next holder of this key \
             inherits the withdrawal and never becomes active"
        );
    }
}

// ----------------------------------------------------------------------------------------------
// Appended rather than placed with its siblings, deliberately.
//
// `native_state.rs`'s line numbers are cited by 41 Lean declarations across `Rchain/Pos.lean`,
// `Rchain/Casper/Bonds.lean` and `Rchain/CrossShard.lean`, and by ten rows of hand-maintained spec.
// An insertion anywhere above them shifts every one, and a Lean citation can only be repaired by
// editing the Lean and re-emitting the register. Adding at the end shifts nothing, so the cost of
// putting this code where it reads best is a lake build and 51 citation edits -- paid once here, or
// avoided entirely by keeping the tail of this file append-only. The tail is append-only.
// ----------------------------------------------------------------------------------------------

/// Leaf key for the **seed the next epoch's active-set draw will use**.
///
/// This leaf is why the draw can be strengthened later without touching the rule. `select_active`
/// depends on a *value*, not on a *mechanism*: replacing what writes this leaf — with commit-reveal or
/// a VRF accumulator — changes nothing about the selection rule or any of its consumers. Today it is
/// written one boundary ahead, anchored to that boundary block's **pre-state hash**, so the proposer
/// of the drawing block cannot choose it.
pub fn pos_epoch_seed_key() -> Blake2b256Hash {
    Blake2b256Hash::create(b"pos:epoch_seed")
}

/// The seed one epoch's active-set draw is made from, as stored in `pos:epoch_seed`.
///
/// **Why this exists as a leaf rather than as a parameter.** The value is written one boundary ahead
/// and read by the *next* boundary's draw, so the block that draws is not the block that chose the
/// entropy — which is the whole point (see the module's `select_active`). Keeping it in state rather
/// than threading it through `SystemDeploy::close_block` means replay agrees *by construction*: the
/// seed is a function of the pre-state, not a second derivation that has to be kept in sync.
///
/// `anchors` are the digests this seed folds in, in order. Today's writer puts two there — the previous
/// seed's anchor and the boundary block's pre-state hash — but nothing in the rule depends on the
/// count: it is a `Vec` rather than a fixed array because a later writer (commit-reveal, a VRF
/// accumulator, a beacon window) may want a different number, and the codec length-checks it so a
/// malformed leaf cannot be read as a shorter list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EpochSeed {
    pub epoch: i64,
    pub anchors: Vec<Blake2b256Hash>,
}

/// A `Blake2b256Hash` on the wire.
const HASH_LEN: usize = 32;

impl EpochSeed {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(16 + self.anchors.len() * HASH_LEN);
        out.extend_from_slice(&self.epoch.to_le_bytes());
        out.extend_from_slice(&(self.anchors.len() as u64).to_le_bytes());
        for h in &self.anchors {
            out.extend_from_slice(h.as_bytes());
        }
        out
    }

    /// Decode (inverse of [`Self::encode`]). Refuses a length that disagrees with the declared count,
    /// so a truncated or padded leaf is an error rather than a silently shorter list — an anchor list of
    /// the wrong length would still draw a set, and the divergence would only surface as a post-state
    /// mismatch much later.
    pub fn decode(bytes: &[u8]) -> Result<EpochSeed, String> {
        if bytes.len() < 16 {
            return Err(format!(
                "epoch seed encoding has {} bytes, under the 16-byte header",
                bytes.len()
            ));
        }
        let epoch = i64::from_le_bytes(
            bytes[..8]
                .try_into()
                .map_err(|_| "epoch seed: invalid epoch field".to_string())?,
        );
        let declared = u64::from_le_bytes(
            bytes[8..16]
                .try_into()
                .map_err(|_| "epoch seed: invalid count field".to_string())?,
        );
        let count = usize::try_from(declared)
            .map_err(|_| format!("epoch seed: anchor count {declared} does not fit a usize"))?;
        let rest = &bytes[16..];
        let expected = count
            .checked_mul(HASH_LEN)
            .ok_or_else(|| format!("epoch seed: anchor count {count} overflows"))?;
        if rest.len() != expected {
            return Err(format!(
                "epoch seed declares {count} anchors ({expected} bytes) but carries {}",
                rest.len()
            ));
        }
        let anchors = rest
            .chunks_exact(HASH_LEN)
            .map(|chunk| {
                let bytes: [u8; HASH_LEN] = chunk
                    .try_into()
                    .map_err(|_| "epoch seed: invalid hash length".to_string())?;
                Ok(Blake2b256Hash::from_byte_array(&bytes))
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(EpochSeed { epoch, anchors })
    }
}

impl NativeSystemState {
    /// Read the seed the next epoch's active-set draw will use.
    ///
    /// An absent leaf is a *distinguishable* state, not a default: it means no draw has been seeded
    /// yet (a genesis-installed state, before its first boundary). The caller decides what that means —
    /// genesis supplies a constant, the boundary supplies its pre-state hash — so this returns `Option` rather
    /// than inventing a seed that would silently change the draw.
    pub async fn epoch_seed(&self) -> Result<Option<EpochSeed>, String> {
        let bytes = self
            .store
            .get(PREFIX_POS, &pos_epoch_seed_key())
            .await
            .map_err(|e| e.to_string())?;
        match bytes {
            Some(b) => EpochSeed::decode(&b).map(Some),
            None => Ok(None),
        }
    }

    /// Write the seed the next epoch's active-set draw will use.
    pub fn set_epoch_seed(&self, seed: &EpochSeed) {
        self.store
            .put(PREFIX_POS, pos_epoch_seed_key(), seed.encode());
    }
}

#[cfg(test)]
mod epoch_seed_tests {
    use super::*;

    /// The epoch seed round-trips, anchor list and all.
    #[test]
    fn epoch_seed_round_trips() {
        let seed = EpochSeed {
            epoch: 7,
            anchors: vec![
                Blake2b256Hash::create(b"block:1"),
                Blake2b256Hash::create(b"block:2"),
                Blake2b256Hash::create(b"block:3"),
            ],
        };
        assert_eq!(EpochSeed::decode(&seed.encode()).unwrap(), seed);
    }

    /// **An anchor list of the wrong length is an error, not a shorter list.** This is the failure this
    /// codec exists to prevent: a truncated or padded leaf would still *draw a set*, so a
    /// mis-encoded seed would not crash — it would produce a deterministic-but-wrong active set and
    /// surface much later as a post-state mismatch. Every length disagreement is refused here.
    #[test]
    fn an_epoch_seed_anchor_list_of_the_wrong_length_is_refused() {
        let seed = EpochSeed {
            epoch: 1,
            anchors: vec![Blake2b256Hash::create(b"a"), Blake2b256Hash::create(b"b")],
        };
        let encoded = seed.encode();

        // One byte short of the declared anchor list.
        assert!(
            EpochSeed::decode(&encoded[..encoded.len() - 1]).is_err(),
            "a truncated anchor list must be refused"
        );
        // One byte long.
        let mut padded = encoded.clone();
        padded.push(0);
        assert!(
            EpochSeed::decode(&padded).is_err(),
            "a padded anchor list must be refused"
        );
        // A header that disagrees with the body: claim three hashes, carry two.
        let mut lying = encoded.clone();
        lying[8..16].copy_from_slice(&3u64.to_le_bytes());
        assert!(
            EpochSeed::decode(&lying).is_err(),
            "a count that disagrees with the body must be refused"
        );
        // And a header too short to hold the epoch and count.
        assert!(EpochSeed::decode(&[0u8; 15]).is_err());
    }

    /// An *absent* seed is distinguishable from a zeroed one — the reader a boundary uses to decide
    /// whether a draw has been seeded at all.
    #[test]
    fn an_epoch_seed_with_no_anchors_is_still_a_seed() {
        let seed = EpochSeed {
            epoch: 0,
            anchors: vec![],
        };
        assert_eq!(EpochSeed::decode(&seed.encode()).unwrap(), seed);
    }
}

// ----------------------------------------------------------------------------------------------
// Randomised active-set selection (audit follow-on; see `docs/src/node/security-audit.md` §8).
//
// In the append-only tail for the reason the seam above gives: everything here is new, and new code
// at the end costs no citations.
// ----------------------------------------------------------------------------------------------

/// Domain separator for the epoch draw's RNG input.
///
/// Binding the epoch *and* a purpose string means a seed leaf cannot be replayed for the wrong epoch,
/// and cannot collide with any other use of `Blake2b512Random` seeded from similar bytes.
const EPOCH_DRAW_DOMAIN: &[u8] = b"rchain:pos:epoch-draw:v1";

/// Domain separator for the **anchor** one epoch's seed contributes to the next.
///
/// A separate string from [`EPOCH_DRAW_DOMAIN`] on purpose: the same `EpochSeed` is both an RNG input
/// and an input to its successor, and a shared prefix would make those two uses one value.
const EPOCH_SEED_ANCHOR_DOMAIN: &[u8] = b"rchain:pos:epoch-seed-anchor:v1";

/// One epoch's seed reduced to the digest the next epoch's seed folds in.
///
/// This is what makes the seed sequence a hash chain: `seed_{k+1}` depends on `seed_k`, so every seed
/// commits to all of its predecessors and no epoch's entropy can be replaced without changing every
/// later seed. Freshness comes free with it — consecutive seeds differ even on a chain where nothing
/// else did.
pub fn previous_seed_anchor(seed: &EpochSeed) -> Blake2b256Hash {
    let encoded = seed.rng_input();
    Blake2b256Hash::create_many(&[EPOCH_SEED_ANCHOR_DOMAIN, &encoded])
}

// Imported here rather than at the top of the file: a `use` above line 3115 shifts every cited line.
use rchain_crypto::hash::blake2b512_random::Blake2b512Random;

impl EpochSeed {
    /// The bytes handed to [`Blake2b512Random::from_init`] — domain-separated and epoch-bound.
    pub fn rng_input(&self) -> Vec<u8> {
        let body = self.encode();
        let mut out = Vec::with_capacity(EPOCH_DRAW_DOMAIN.len() + body.len());
        out.extend_from_slice(EPOCH_DRAW_DOMAIN);
        out.extend_from_slice(&body);
        out
    }
}

/// The seed the **genesis** active set is drawn from.
///
/// A fixed constant rather than a `PosGenesis` field, and that is a deliberate choice: a config value
/// would let two nodes disagree about the genesis post-state hash, which is a config-driven fork — the
/// same failure `spec/RUST-VS-SCALA.md` §3 item 5 names for `MAX_BLOCK_DEPLOYS`. A constant cannot
/// disagree. It is also *sound* despite being public: the genesis pool is fixed in the genesis file, so
/// a seed that produced a set you preferred would be a different chain's genesis, not this one's.
///
/// An empty anchor list is not "unseeded" — [`EPOCH_DRAW_DOMAIN`] plus epoch zero is the seed. It is
/// empty because genesis has no prior state to anchor to, and inventing a fake anchor would pretend the
/// chain had history it does not.
pub fn genesis_epoch_seed() -> EpochSeed {
    EpochSeed {
        epoch: 0,
        anchors: Vec::new(),
    }
}

/// A uniform `u128` in `[0, bound)`, rejection-sampled — never reduced modulo.
///
/// A modulo biases the low values when `bound` does not divide `2^128`, and "negligible bias" is not a
/// phrase this codebase accepts on a consensus path. `accept` is the largest multiple of `bound` that
/// fits a `u128`, so every accepted value maps to exactly `2^128 / bound` rejected-or-taken values and
/// `v % bound` is uniform.
///
/// **Why 128 bits and not 64.** The bound here is a *stake total*, not a count: it is the sum of the
/// pool's `i64`-bounded stakes, and a total that overflows the draw's width would make the draw
/// unreachable rather than biased. Two `usize` draws from the stream give the 128 bits without a
/// narrowing conversion in either direction, which is the property the previous 64-bit form was
/// written to keep (see the module's type-system notes).
///
/// **The rejection loop consumes the stream a data-dependent number of times.** That is harmless here
/// and it is checked rather than assumed: nothing reads the stream after a draw, and the draw's output
/// is pinned by a known-answer test, so a change in consumption shows up there.
fn uniform_below_u128(rand: &mut Blake2b512Random, bound: u128) -> u128 {
    if bound <= 1 {
        return 0;
    }
    let accept = (u128::MAX / bound) * bound;
    let width = std::mem::size_of::<usize>();
    let mut half = [0u8; std::mem::size_of::<usize>()];
    loop {
        let mut bytes = [0u8; 2 * std::mem::size_of::<usize>()];
        let low = rand.next();
        half.copy_from_slice(&low[..width]);
        bytes[..width].copy_from_slice(&half);
        let high = rand.next();
        half.copy_from_slice(&high[..width]);
        bytes[width..].copy_from_slice(&half);
        let value = u128::from_le_bytes(bytes);
        if value < accept {
            return value % bound;
        }
    }
}

/// Draw `count` distinct entries from `candidates` **in proportion to their weight**, without
/// replacement, in draw order.
///
/// `weight_of` is the stake, and the walk is a sequential weighted draw: take a uniform value in
/// `[0, remaining_total)`, walk the candidate list in its canonical order accumulating weights, take
/// the first entry whose cumulative weight exceeds the value, remove it, repeat. Exact integer
/// arithmetic throughout — no floats, no logarithms, no rounding, so `(pool, seed)` determines the
/// result on every machine. `candidates` is built from a `BTreeMap`, so its order is key order, and
/// `Vec::remove` preserves that order across the iteration, which is what makes the walk canonical.
///
/// **Exact for the first slot, and that is the property that matters.** The first pick is
/// `stake / total` exactly. The later picks renormalise over what is left, so they are proportional to
/// the *remaining* weights rather than to the original ones; an item's expected share of `count` slots
/// is therefore near — not identically — `count · stake / total`. The exact-proportional alternative
/// is a systematic (rotated-interval) scheme, which is a different rule with a different variance and
/// a collision problem for any stake above `1/count` of the total. This is the sequential rule, and
/// the residual is stated where the rule is documented rather than left implicit.
fn draw_weighted_without_replacement<T>(
    candidates: &[T],
    count: usize,
    rand: &mut Blake2b512Random,
) -> Vec<T>
where
    T: Clone + Weighted,
{
    let mut remaining: Vec<T> = candidates.to_vec();
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let total: u128 = remaining.iter().map(|c| u128::from(c.weight())).sum();
        if total == 0 {
            break;
        }
        let draw = uniform_below_u128(rand, total);
        let mut cumulative: u128 = 0;
        let mut pick = remaining.len() - 1;
        for (index, candidate) in remaining.iter().enumerate() {
            cumulative += u128::from(candidate.weight());
            if draw < cumulative {
                pick = index;
                break;
            }
        }
        out.push(remaining.remove(pick));
    }
    out
}

/// A candidate's weight in the active-set draw: its stake.
///
/// A trait rather than a closure so the helper stays generic and the weight is read in exactly one
/// place. It is implemented for the `(validator, stake)` pair the pool yields.
trait Weighted {
    fn weight(&self) -> u64;
}

impl Weighted for (&Validator, NonNegI64) {
    fn weight(&self) -> u64 {
        // Non-negative by the refinement on the field, and `i64`-bounded by the type, so the
        // conversion is total. The cast is the reason the weight is `u64` and not `i64`: a negative
        // weight would make the cumulative walk skip a candidate rather than refuse it.
        u64::try_from(i64::from(self.1)).unwrap_or(0)
    }
}

// ----------------------------------------------------------------------------------------------
// The seed writer (phase 3 of the randomised selection): `close_block` step 5.
//
// Appended for the seam's reason. These tests are the ones that make step 5 more than a claim:
// `close_block` writing the wrong thing — the drawing block's own pre-state, or nothing at all —
// leaves every other test in this file passing.
// ----------------------------------------------------------------------------------------------

#[cfg(test)]
mod epoch_seed_writer_tests {
    use super::*;

    /// A genesis-installed state with `size` equal-stake validators and an active-set cap of `cap`.
    ///
    /// `epoch_length: 0` means every block is a boundary and every block's epoch index is its own
    /// block number, which keeps the tests below about the *seed* rather than about the arithmetic of
    /// where boundaries fall.
    async fn pos_with_pool(size: u8, cap: i64) -> NativeSystemState {
        let native = NativeSystemState::new(Arc::new(InMemNativeStore::empty()));
        let validators: Vec<Validator> = (1..=size)
            .map(|i| Validator::from_slice(&[i; 65]))
            .collect();
        let bonds: BTreeMap<Validator, NonNegI64> = validators
            .iter()
            .map(|v| (*v, NonNegI64::try_from(10).expect("a test stake")))
            .collect();
        native
            .install_genesis(&PosGenesis {
                bonds,
                trusted: validators.into_iter().collect(),
                params: PosParams {
                    epoch_length: 0,
                    number_of_active_validators: cap,
                    ..PosParams::default()
                },
            })
            .unwrap();
        native
    }

    /// **The property the old rule did not have, and the reason this change exists.**
    ///
    /// The set drawn at a boundary comes from the seed the *previous* boundary wrote, so nothing the
    /// drawing block's proposer can vary moves it. Under the rule this replaced the seed was the
    /// drawing block's own pre-state, which the proposer reaches through its justification set; now the
    /// block's contribution is the value it *writes* for the next boundary. Two runs over the same
    /// history, drawing at the same boundary with two different values written there, must therefore
    /// draw the **same** set.
    ///
    /// The second assertion is what makes the first mean something. Without it the test passes if the
    /// value is ignored entirely, which is the same as not having it: the seed written for the
    /// *following* boundary must differ between the runs, so the mutation provably reached the code.
    ///
    /// **Falsified by construction, and the numbers are the point.** Replacing step 4's `epoch_seed`
    /// read with the drawing block's own pre-state — the rule this change removes — fails this test
    /// and exactly two others in this file; the other 45, and the whole rest of `native_state`'s
    /// suite, still pass. That is the audit's claim about the old rule, reproduced as a failing test:
    /// a proposer that varied its justifications was varying the draw, and nothing else in the tree
    /// noticed.
    #[tokio::test]
    async fn the_drawn_set_does_not_move_with_the_anchor_the_drawing_block_writes() {
        let anchor_b1 = Blake2b256Hash::create(b"B1 pre-state");
        let a = pos_with_pool(4, 2).await;
        let b = pos_with_pool(4, 2).await;
        a.close_block(1, anchor_b1).await.unwrap().unwrap();
        b.close_block(1, anchor_b1).await.unwrap().unwrap();
        assert_eq!(
            a.epoch_seed().await.unwrap(),
            b.epoch_seed().await.unwrap(),
            "the two histories agree up to the boundary that seeds epoch 2"
        );

        // The same boundary, two different pre-states — i.e. two different justification sets.
        a.close_block(
            2,
            Blake2b256Hash::create(b"B2 pre-state, justification set 1"),
        )
        .await
        .unwrap()
        .unwrap();
        b.close_block(
            2,
            Blake2b256Hash::create(b"B2 pre-state, justification set 2"),
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(
            a.active().await.unwrap(),
            b.active().await.unwrap(),
            "the set drawn at a block must not depend on that block's own pre-state"
        );
        assert_ne!(
            a.epoch_seed().await.unwrap(),
            b.epoch_seed().await.unwrap(),
            "…and the pre-state must still reach the seed the next boundary draws with, or this test \
             would pass for the wrong reason"
        );
    }

    /// The seed is a **hash chain**: the previous seed is folded into the next one, so consecutive
    /// boundaries cannot produce the same seed even when the pre-state hash repeats.
    ///
    /// Falsifiable by deletion, which is how it was checked: drop `previous_seed_anchor` from
    /// `close_block` step 5 and this test fails while every other test in the file still passes.
    #[tokio::test]
    async fn consecutive_seeds_differ_even_when_the_pre_state_hash_repeats() {
        let native = pos_with_pool(4, 2).await;
        let repeated = Blake2b256Hash::create(b"the same pre-state twice");

        native.close_block(1, repeated).await.unwrap().unwrap();
        let first = native.epoch_seed().await.unwrap().expect("seeded at B1");
        native.close_block(2, repeated).await.unwrap().unwrap();
        let second = native.epoch_seed().await.unwrap().expect("seeded at B2");

        assert_ne!(
            first, second,
            "the same pre-state at two boundaries must still give two seeds"
        );
        assert_eq!(
            second.anchors.first().copied(),
            Some(previous_seed_anchor(&first)),
            "and the first anchor must be the previous seed's anchor, which is what carries the chain"
        );
    }

    /// The anchors the writer records are the previous seed's anchor and **the fringe state hash this
    /// block extends**, in that order. Pinned exactly, because a swap is invisible to the two tests
    /// above — both anchors still move when the pre-state moves, and the chain still advances.
    #[tokio::test]
    async fn the_written_seed_anchors_are_the_previous_seed_and_this_blocks_fringe_state() {
        let native = pos_with_pool(4, 2).await;
        let b1 = Blake2b256Hash::create(b"B1 pre-state");
        native.close_block(1, b1).await.unwrap().unwrap();
        let seed_at_b1 = native.epoch_seed().await.unwrap().expect("seeded at B1");

        let b2 = Blake2b256Hash::create(b"B2 pre-state");
        native.close_block(2, b2).await.unwrap().unwrap();
        let seed_at_b2 = native.epoch_seed().await.unwrap().expect("seeded at B2");

        assert_eq!(
            seed_at_b2.anchors,
            vec![previous_seed_anchor(&seed_at_b1), b2],
            "the seed written at a boundary must anchor to the previous seed and this block's fringe"
        );
        assert_eq!(
            seed_at_b2.epoch, 3,
            "and it must be labelled with the epoch that will draw from it"
        );
    }

    /// A genesis-installed state has **no** seed leaf until its first boundary. The distinction is
    /// load-bearing: `close_block` treats an absent leaf as "draw from the genesis constant", so a
    /// zeroed seed and no seed at all must not be the same value.
    #[tokio::test]
    async fn genesis_has_no_seed_leaf_and_its_first_boundary_writes_one() {
        let native = pos_with_pool(2, 1).await;
        assert_eq!(
            native.epoch_seed().await.unwrap(),
            None,
            "genesis must not write a seed: the constant is not a window, and the leaf's absence is \
             how the boundary tells the two apart"
        );
        native
            .close_block(1, Blake2b256Hash::create(b"B1"))
            .await
            .unwrap()
            .unwrap();
        assert!(
            native.epoch_seed().await.unwrap().is_some(),
            "the first boundary must leave a seed behind for the next one"
        );
    }

    /// Why the genesis constant is a constant and not a `PosGenesis` field: the draw it seeds has to
    /// be identical on two nodes that installed the same genesis, with no configuration input. This
    /// is the property `casper/tests/consensus.rs`'s bonds-cache canary fails on if it is broken.
    #[tokio::test]
    async fn two_genesis_installs_of_the_same_pool_draw_the_same_set() {
        let a = pos_with_pool(4, 2).await;
        let b = pos_with_pool(4, 2).await;
        assert_eq!(
            a.active().await.unwrap(),
            b.active().await.unwrap(),
            "the genesis draw must be a pure function of the genesis file"
        );
    }
}
