//! The gateway's durable coordinator record (Law 29).
//!
//! A coordinator's state and votes are **node-local, not consensus state**: no other validator runs
//! a coordinator, so writing them into the content-addressed RSpace trie would make a shard's state
//! hash depend on one node's off-chain coordination and diverge consensus. They therefore live in
//! their own node-local store ([`crate::storage`]'s `gateway-txn` database), while keeping the
//! `PREFIX_TXN_COORD` namespace byte and the canonical-encoding discipline of the consensus-side
//! `rholang::native_state` ledger — Law 29 asks for the record to be *durable*, not *consensus*.
//!
//! The record mirrors the Lean `CoordRecord` (`spec/Rchain/CrossShard.lean`): the legs as an ordered
//! list and one vote per leg, so `commit_record_deterministic`'s biconditional
//! (`state = committed ↔ every vote is ready`) holds verbatim for a stored record.

use std::sync::Arc;

use rchain_block_storage::dag::codecs::Blake2b256HashCodec;
use rchain_crypto::hash::blake2b256_hash::Blake2b256Hash;
use rchain_crypto::public_key::PublicKey;
use rchain_shared::refined::{NonNegI64, ShardId};
use rchain_shared::store_manager::KeyValueStoreManager;
use rchain_shared::typed_store::{BytesCodec, KeyValueTypedStore};

use crate::storage::GATEWAY_TXN_DB;

/// The namespace byte prefixed to a coordinator record (the same discipline `rspace`'s `PREFIX_*`
/// ledgers use, in a node-local store).
pub const PREFIX_TXN_COORD: u8 = 0x01;

/// A coordinator A public key is 65 uncompressed secp256k1 bytes.
const PUBLIC_KEY_LEN: usize = 65;

/// The coordinator's transaction state (the Lean `TxnState`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoordState {
    /// The legs are recorded; no vote has been counted yet.
    Proposed,
    /// At least one vote is recorded, but not one per leg (phase one in flight).
    Prepared,
    /// The commit point: every leg has a vote and every vote is `Ready`.
    Committed,
    /// A leg voted `Abort` (an explicit abort, an error, or a timeout recorded as an abort).
    Aborted,
}

impl CoordState {
    fn discriminant(self) -> u8 {
        match self {
            CoordState::Proposed => 0,
            CoordState::Prepared => 1,
            CoordState::Committed => 2,
            CoordState::Aborted => 3,
        }
    }

    fn from_discriminant(b: u8) -> Option<Self> {
        match b {
            0 => Some(CoordState::Proposed),
            1 => Some(CoordState::Prepared),
            2 => Some(CoordState::Committed),
            3 => Some(CoordState::Aborted),
            _ => None,
        }
    }

    /// The `camelCase` name used by the HTTP surface.
    pub fn as_str(self) -> &'static str {
        match self {
            CoordState::Proposed => "proposed",
            CoordState::Prepared => "prepared",
            CoordState::Committed => "committed",
            CoordState::Aborted => "aborted",
        }
    }

    /// Whether the transaction has reached a terminal state — the recovery loop leaves these alone.
    pub fn is_terminal(self) -> bool {
        matches!(self, CoordState::Committed | CoordState::Aborted)
    }
}

/// A participant's phase-one vote (the Lean `Vote`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vote {
    Ready,
    Abort,
}

impl Vote {
    fn discriminant(self) -> u8 {
        match self {
            Vote::Ready => 0,
            Vote::Abort => 1,
        }
    }

    fn from_discriminant(b: u8) -> Option<Self> {
        match b {
            0 => Some(Vote::Ready),
            1 => Some(Vote::Abort),
            _ => None,
        }
    }

    /// The `camelCase` name used by the HTTP surface.
    pub fn as_str(self) -> &'static str {
        match self {
            Vote::Ready => "ready",
            Vote::Abort => "abort",
        }
    }
}

/// One leg of a transaction, as recorded by the coordinator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegRecord {
    pub shard_id: ShardId,
    pub amount: NonNegI64,
    pub to: String,
}

/// The coordinator's durable record of one cross-shard transaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoordRecord {
    pub txn_id: Vec<u8>,
    pub state: CoordState,
    pub coordinator: PublicKey,
    /// The legs, in the caller's order (canonical — the Lean `List Leg`).
    pub legs: Vec<LegRecord>,
    /// One vote per leg, in leg order (the Lean `List (ShardId × Vote)`).
    pub votes: Vec<(ShardId, Vote)>,
    /// Why an abort happened (a timeout, a participant error), when one did.
    pub reason: Option<String>,
}

impl CoordRecord {
    /// The vote recorded for `shard_id`, if any.
    pub fn vote_for(&self, shard_id: &ShardId) -> Option<Vote> {
        self.votes
            .iter()
            .find(|(id, _)| id == shard_id)
            .map(|(_, vote)| *vote)
    }

    /// Record a leg's vote, keeping leg order and the phase-one state mapping.
    ///
    /// `Committed` is written only once every leg has voted `Ready`; any `Abort` makes the record
    /// `Aborted` immediately. A record can therefore never be `Prepared` with a complete all-ready
    /// vote list, which is what makes `commit_record_deterministic`'s biconditional exact.
    pub fn record_vote(&mut self, shard_id: ShardId, vote: Vote, reason: Option<String>) {
        match self.votes.iter_mut().find(|(id, _)| *id == shard_id) {
            Some(entry) => entry.1 = vote,
            None => self.votes.push((shard_id, vote)),
        }
        if vote == Vote::Abort {
            self.state = CoordState::Aborted;
            if reason.is_some() {
                self.reason = reason;
            }
        } else if self.votes.len() == self.legs.len()
            && self.votes.iter().all(|(_, v)| *v == Vote::Ready)
        {
            self.state = CoordState::Committed;
        } else {
            self.state = CoordState::Prepared;
        }
    }

    /// Whether this record satisfies Law 29's biconditional, as the Lean statement has it.
    pub fn decision_is_deterministic(&self) -> bool {
        let all_ready = self.votes.len() == self.legs.len()
            && !self.legs.is_empty()
            && self.votes.iter().all(|(_, v)| *v == Vote::Ready);
        (self.state == CoordState::Committed) == all_ready
    }
}

/// Canonically encode a coordinator record: the namespace byte, the state discriminant, the
/// coordinator key, the txn id, the legs, the votes and the optional reason — all length-prefixed.
fn encode_coord(rec: &CoordRecord) -> Vec<u8> {
    let mut out = Vec::new();
    out.push(PREFIX_TXN_COORD);
    out.push(rec.state.discriminant());
    out.extend_from_slice(rec.coordinator.bytes());
    push_bytes(&mut out, &rec.txn_id);
    push_len(&mut out, rec.legs.len());
    for leg in &rec.legs {
        push_bytes(&mut out, leg.shard_id.to_string().as_bytes());
        out.extend_from_slice(&i64::from(leg.amount).to_le_bytes());
        push_bytes(&mut out, leg.to.as_bytes());
    }
    push_len(&mut out, rec.votes.len());
    for (shard_id, vote) in &rec.votes {
        push_bytes(&mut out, shard_id.to_string().as_bytes());
        out.push(vote.discriminant());
    }
    match &rec.reason {
        Some(reason) => {
            out.push(1);
            push_bytes(&mut out, reason.as_bytes());
        }
        None => out.push(0),
    }
    out
}

fn push_len(out: &mut Vec<u8>, len: usize) {
    // A record's lists are small; a length that does not fit `u32` is a programming error that shows
    // up as a truncated decode, not as a silent wrap.
    out.extend_from_slice(&u32::try_from(len).unwrap_or(u32::MAX).to_le_bytes());
}

fn push_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    push_len(out, bytes.len());
    out.extend_from_slice(bytes);
}

/// A bounds-checked reader over an encoded record.
struct Reader<'a> {
    bytes: &'a [u8],
    off: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self
            .off
            .checked_add(n)
            .ok_or_else(|| "coord record: offset overflow".to_string())?;
        if end > self.bytes.len() {
            return Err("coord record truncated".to_string());
        }
        let slice = &self.bytes[self.off..end];
        self.off = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, String> {
        let bytes: [u8; 4] = self
            .take(4)?
            .try_into()
            .map_err(|_| "coord record: invalid u32".to_string())?;
        Ok(u32::from_le_bytes(bytes))
    }

    fn len(&mut self) -> Result<usize, String> {
        let n = usize::try_from(self.u32()?)
            .map_err(|_| "coord record: length overflow".to_string())?;
        // A length longer than the remaining bytes can never be satisfied; rejecting it here keeps
        // every subsequent `take` in-bounds and the decode total.
        if n > self.bytes.len().saturating_sub(self.off) {
            return Err("coord record: length exceeds the record".to_string());
        }
        Ok(n)
    }

    fn byte_string(&mut self) -> Result<Vec<u8>, String> {
        let n = self.len()?;
        Ok(self.take(n)?.to_vec())
    }

    fn string(&mut self) -> Result<String, String> {
        let bytes = self.byte_string()?;
        String::from_utf8(bytes).map_err(|_| "coord record: non-UTF8 string".to_string())
    }

    fn shard_id(&mut self) -> Result<ShardId, String> {
        let s = self.string()?;
        ShardId::try_from(s).map_err(|e| format!("coord record: invalid shard id: {e}"))
    }

    /// Whether the record is fully consumed — trailing bytes are a decode error.
    fn is_done(&self) -> bool {
        self.off == self.bytes.len()
    }
}

/// Decode a coordinator record (inverse of [`encode_coord`]). Total and strict: every length is
/// bounds-checked, unknown discriminants and trailing bytes are rejected, and nothing panics.
pub fn decode_coord(bytes: &[u8]) -> Result<CoordRecord, String> {
    let mut r = Reader { bytes, off: 0 };
    if r.u8()? != PREFIX_TXN_COORD {
        return Err("coord record: bad namespace byte".to_string());
    }
    let state = CoordState::from_discriminant(r.u8()?)
        .ok_or_else(|| "coord record: unknown state".to_string())?;
    let coordinator = PublicKey::new(r.take(PUBLIC_KEY_LEN)?.to_vec());
    let txn_id = r.byte_string()?;

    let leg_count = r.len()?;
    let mut legs = Vec::with_capacity(leg_count);
    for _ in 0..leg_count {
        let shard_id = r.shard_id()?;
        let amount_bytes: [u8; 8] = r
            .take(8)?
            .try_into()
            .map_err(|_| "coord record: invalid amount".to_string())?;
        let amount = NonNegI64::try_from(i64::from_le_bytes(amount_bytes))
            .map_err(|_| "coord record: negative amount".to_string())?;
        let to = r.string()?;
        legs.push(LegRecord {
            shard_id,
            amount,
            to,
        });
    }

    let vote_count = r.len()?;
    let mut votes = Vec::with_capacity(vote_count);
    for _ in 0..vote_count {
        let shard_id = r.shard_id()?;
        let vote = Vote::from_discriminant(r.u8()?)
            .ok_or_else(|| "coord record: unknown vote".to_string())?;
        votes.push((shard_id, vote));
    }

    let reason = match r.u8()? {
        0 => None,
        1 => Some(r.string()?),
        _ => return Err("coord record: invalid reason flag".to_string()),
    };
    if !r.is_done() {
        return Err("coord record: trailing bytes".to_string());
    }

    Ok(CoordRecord {
        txn_id,
        state,
        coordinator,
        legs,
        votes,
        reason,
    })
}

/// The content address of a record: its canonical encoding, hashed. This is the "content-addressed
/// decision record" made checkable — two nodes that hold the same decision agree on this hash, and
/// nothing about it enters consensus.
pub fn record_hash(rec: &CoordRecord) -> Blake2b256Hash {
    Blake2b256Hash::create(&encode_coord(rec))
}

/// The durable coordinator ledger: node-local, keyed by `txn_id`.
pub struct TxnLedger {
    store: Arc<dyn KeyValueTypedStore<Blake2b256Hash, Vec<u8>>>,
}

impl TxnLedger {
    /// Open the ledger over a key-value store manager.
    pub async fn open(manager: &dyn KeyValueStoreManager) -> Result<Self, String> {
        let store = Arc::new(
            rchain_shared::store_manager::database(
                manager,
                GATEWAY_TXN_DB,
                Arc::new(Blake2b256HashCodec),
                Arc::new(BytesCodec),
            )
            .await?,
        );
        Ok(TxnLedger { store })
    }

    /// The record for `txn_id`, if the node has one.
    pub async fn get(&self, txn_id: &[u8]) -> Result<Option<CoordRecord>, String> {
        let key = txn_key(txn_id);
        let stored = self.store.get(&[key]).await?.into_iter().next().flatten();
        stored.map(|bytes| decode_coord(&bytes)).transpose()
    }

    /// Write a record (durably — every write precedes the I/O it authorizes).
    pub async fn put(&self, record: &CoordRecord) -> Result<(), String> {
        let key = txn_key(&record.txn_id);
        self.store.put(&[(key, encode_coord(record))]).await
    }

    /// Every record still in flight (`Proposed` or `Prepared`) — what a restart must finish.
    pub async fn in_flight(&self) -> Result<Vec<CoordRecord>, String> {
        let mut out = Vec::new();
        for (_, bytes) in self.store.to_map().await? {
            let record = decode_coord(&bytes)?;
            if !record.state.is_terminal() {
                out.push(record);
            }
        }
        // A `BTreeMap` iteration is already canonical by key; sort by `txn_id` so the report is too.
        out.sort_by(|a, b| a.txn_id.cmp(&b.txn_id));
        Ok(out)
    }
}

/// The ledger key of a transaction: the `txn_id` hashed, mirroring the participant's `txn_key`.
fn txn_key(txn_id: &[u8]) -> Blake2b256Hash {
    Blake2b256Hash::create(txn_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shard(id: &str) -> ShardId {
        ShardId::try_from(id.to_string()).unwrap()
    }

    fn sample() -> CoordRecord {
        CoordRecord {
            txn_id: b"txn-1".to_vec(),
            state: CoordState::Proposed,
            coordinator: PublicKey::new(vec![7u8; 65]),
            legs: vec![
                LegRecord {
                    shard_id: shard("/root"),
                    amount: NonNegI64::try_from(30).unwrap(),
                    to: "dest".to_string(),
                },
                LegRecord {
                    shard_id: shard("/root/child"),
                    amount: NonNegI64::try_from(40).unwrap(),
                    to: "dest".to_string(),
                },
            ],
            votes: Vec::new(),
            reason: None,
        }
    }

    #[test]
    fn encode_decode_round_trips() {
        let mut record = sample();
        record.record_vote(shard("/root"), Vote::Ready, None);
        let decoded = decode_coord(&encode_coord(&record)).expect("round trip");
        assert_eq!(decoded, record);
        assert_eq!(record_hash(&decoded), record_hash(&record));
    }

    /// The encoding is canonical: rebuilding the same record produces the same bytes and address.
    #[test]
    fn encoding_and_record_hash_are_stable() {
        let a = sample();
        let b = sample();
        assert_eq!(encode_coord(&a), encode_coord(&b));
        assert_eq!(record_hash(&a), record_hash(&b));
        // A different decision is a different address.
        let mut committed = sample();
        committed.record_vote(shard("/root"), Vote::Ready, None);
        committed.record_vote(shard("/root/child"), Vote::Ready, None);
        assert_ne!(record_hash(&committed), record_hash(&a));
    }

    #[test]
    fn decode_rejects_malformed_records() {
        let good = encode_coord(&sample());

        // Truncated anywhere.
        for cut in 0..good.len() {
            assert!(
                decode_coord(&good[..cut]).is_err(),
                "a {cut}-byte prefix must not decode"
            );
        }
        // A trailing byte.
        let mut trailing = good.clone();
        trailing.push(0);
        assert!(decode_coord(&trailing).is_err());
        // A bad namespace byte.
        let mut bad_ns = good.clone();
        bad_ns[0] = 0xEE;
        assert!(decode_coord(&bad_ns).is_err());
        // An unknown state discriminant.
        let mut bad_state = good.clone();
        bad_state[1] = 0xEE;
        assert!(decode_coord(&bad_state).is_err());
        // A length prefix that overruns the record must not panic or allocate wildly.
        let mut overlong = good.clone();
        overlong[1 + 1 + PUBLIC_KEY_LEN..1 + 1 + PUBLIC_KEY_LEN + 4]
            .copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode_coord(&overlong).is_err());
    }

    /// The state mapping is what makes Law 29's biconditional hold: `Committed` exactly when every
    /// leg has voted `Ready`, and never a complete all-ready list in a non-committed state.
    #[test]
    fn committed_state_iff_every_vote_is_ready() {
        let mut record = sample();
        assert!(
            record.decision_is_deterministic(),
            "no votes: not committed"
        );

        record.record_vote(shard("/root"), Vote::Ready, None);
        assert_eq!(record.state, CoordState::Prepared);
        assert!(record.decision_is_deterministic());

        record.record_vote(shard("/root/child"), Vote::Ready, None);
        assert_eq!(record.state, CoordState::Committed);
        assert!(record.decision_is_deterministic());

        // Any abort, on any leg, aborts the whole record — including after a ready vote.
        let mut aborted = sample();
        aborted.record_vote(shard("/root"), Vote::Ready, None);
        aborted.record_vote(
            shard("/root/child"),
            Vote::Abort,
            Some("timeout".to_string()),
        );
        assert_eq!(aborted.state, CoordState::Aborted);
        assert_eq!(aborted.reason.as_deref(), Some("timeout"));
        assert!(aborted.decision_is_deterministic());
    }

    #[tokio::test]
    async fn ledger_round_trips_and_reports_only_in_flight_records() {
        let manager = rchain_shared::store_manager::InMemoryStoreManager::default();
        let ledger = TxnLedger::open(&manager).await.expect("open ledger");

        let mut in_flight = sample();
        in_flight.record_vote(shard("/root"), Vote::Ready, None);
        ledger.put(&in_flight).await.expect("put");

        let mut done = sample();
        done.txn_id = b"txn-2".to_vec();
        done.record_vote(shard("/root"), Vote::Ready, None);
        done.record_vote(shard("/root/child"), Vote::Ready, None);
        ledger.put(&done).await.expect("put");

        assert_eq!(
            ledger.get(b"txn-1").await.unwrap().unwrap().state,
            CoordState::Prepared
        );
        assert_eq!(
            ledger.get(b"txn-2").await.unwrap().unwrap().state,
            CoordState::Committed
        );
        assert!(ledger.get(b"absent").await.unwrap().is_none());

        let pending = ledger.in_flight().await.expect("in flight");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].txn_id, b"txn-1".to_vec());
    }
}
