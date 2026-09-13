//! The multi-shard gateway: the on-node two-phase-commit coordinator (Laws 26–29, "Layer 2").
//!
//! The node-side realization of `docs/src/node/shard-invoke.md`'s *"a gateway peer that is a member
//! of both shards"*: a node that validates several shards can drive a cross-shard transaction
//! **itself**, submitting each leg as an ordinary signed deploy to the shard that owns it, instead
//! of relying on an off-chain client to reach both shards over the network.
//!
//! What is and is not consensus-critical:
//!
//! * the **legs** are ordinary signed deploys through the real pipeline — pooled, included in a
//!   block, executed by the `rho:txn` participant. They are never evaluated directly on the play
//!   runtime: that would mutate state outside a block and be discarded by the next proposal's
//!   merged pre-state;
//! * the **decision** (and its durable record, [`ledger`]) is coordinator-side. Law 27's
//!   all-or-nothing is supplied here — no shard's reducer does network I/O, and no other validator
//!   runs a coordinator;
//! * the **routing** — which shard a leg goes to — is this node's business, not part of any shard's
//!   consensus.

pub mod ledger;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;

use rchain_crypto::private_key::PrivateKey;
use rchain_crypto::public_key::PublicKey;
use rchain_models::casper::protocol::casper_message::SignedDeployData;
use rchain_models::casper::protocol::deploy_service::{
    BlockQuery, BlocksQuery, BondStatusQuery, ContinuationAtNameQuery, ContinuationsWithBlockInfo,
    DataAtNameQuery, DataWithBlockInfo, DeployExecStatus, FindDeployQuery, IsFinalizedQuery,
    MachineVerifyQuery, VisualizeDagQuery,
};
use rchain_models::rholang::RhoType::{RhoByteArray, RhoDeployId, RhoNumber, RhoString};
use rchain_shared::base16;
use rchain_shared::refined::{NonNegI64, ShardId};

use crate::api::block_api::BlockApi;
use crate::protocol::client::DeployService;
use crate::shard_invoke::{await_reply, signed_invoke, ShardOutcome};
use crate::txn_coordinator::txn_term;

pub use ledger::{CoordRecord, CoordState, LegRecord, TxnLedger, Vote};

/// One of the node's own shards, as a 2PC participant.
pub struct LocalShard {
    pub shard_id: ShardId,
    /// The shard's block API: used to pool a phase deploy (a real deploy, included in a block by the
    /// shard's own proposer) and to read the reply back.
    pub block_api: Arc<dyn BlockApi>,
    /// The shard's configured reply-listen limit (`api-server.max-blocks-limit`).
    pub max_listen_depth: i32,
}

/// A [`DeployService`] over the node's **own** shards, so a leg costs a local pool insert instead of
/// a network round trip.
///
/// Shaped after the `InProcDeployService` test double, with the two differences production needs: it
/// goes through the shard's `BlockApi` (not a direct runtime evaluation, which would write state
/// outside a block), and it clamps the reply-listen depth — `await_reply` asks for the maximum
/// depth, which the block API rejects outright, so an unclamped listen would never see the reply.
pub struct LocalShardDeployService {
    shards: BTreeMap<ShardId, LocalShard>,
    /// Which shard each submitted deploy went to, so a reply is read back from the same shard.
    deployed: Mutex<BTreeMap<Vec<u8>, ShardId>>,
}

impl LocalShardDeployService {
    pub fn new(shards: BTreeMap<ShardId, LocalShard>) -> Self {
        LocalShardDeployService {
            shards,
            deployed: Mutex::new(BTreeMap::new()),
        }
    }

    /// The member with this id, or the membership error.
    pub fn member(&self, shard_id: &str) -> Result<&LocalShard, Vec<String>> {
        self.shards
            .iter()
            .find(|(id, _)| id.to_string() == shard_id)
            .map(|(_, shard)| shard)
            .ok_or_else(|| {
                vec![format!(
                    "Deploy shardId '{shard_id}' is not a member of this node's shards: [{}]",
                    self.shards
                        .keys()
                        .map(|id| id.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )]
            })
    }

    /// The shard a previously-submitted deploy went to.
    fn shard_of(&self, sig: &[u8]) -> Option<&LocalShard> {
        let recorded = self.deployed.lock().ok()?.get(sig).cloned()?;
        self.shards.get(&recorded)
    }

    /// The shard's current height, which a phase deploy is anchored at: with a hardcoded
    /// `valid_after = 0` the deploy is *born expired* once the chain is more than `DEPLOY_LIFESPAN`
    /// blocks past genesis, and the participant never sees it.
    async fn current_height(&self, shard_id: &ShardId) -> i64 {
        match self.shards.get(shard_id) {
            Some(shard) => match shard.block_api.get_latest_message().await {
                Ok(metadata) => i64::from(metadata.block_num),
                Err(_) => 0,
            },
            None => 0,
        }
    }
}

#[async_trait]
impl DeployService for LocalShardDeployService {
    async fn deploy(&self, d: &SignedDeployData) -> Result<String, Vec<String>> {
        let shard = self.member(&d.data.shard_id)?;
        let sig_hex = base16::encode(&d.sig);
        shard.block_api.deploy(d).await.map_err(|e| vec![e])?;
        if let Ok(mut deployed) = self.deployed.lock() {
            deployed.insert(d.sig.clone(), shard.shard_id.clone());
        }
        Ok(sig_hex)
    }

    async fn listen_for_data_at_name(
        &self,
        q: &DataAtNameQuery,
    ) -> Result<Vec<DataWithBlockInfo>, Vec<String>> {
        let sig = RhoDeployId::unapply(&q.name)
            .ok_or_else(|| vec!["name is not a deploy id".to_string()])?;
        let shard = self
            .shard_of(sig)
            .ok_or_else(|| vec!["no shard is known for this deploy id".to_string()])?;
        let depth = q.depth.min(shard.max_listen_depth);
        // The shard's own reply data, with its own block info — no re-wrapping.
        let (data, _depth) = shard
            .block_api
            .get_listening_name_data_response(depth, &q.name)
            .await
            .map_err(|e| vec![e])?;
        Ok(data)
    }

    // Driving a 2PC only submits phase deploys and reads their replies. The remaining queries are
    // reported as unsupported rather than answered wrongly.
    async fn deploy_status(&self, _: &FindDeployQuery) -> Result<DeployExecStatus, Vec<String>> {
        Err(vec!["deploy_status is not used by the gateway".to_string()])
    }
    async fn get_block(&self, _: &BlockQuery) -> Result<String, Vec<String>> {
        Err(vec!["get_block is not used by the gateway".to_string()])
    }
    async fn get_blocks(&self, _: &BlocksQuery) -> Result<String, Vec<String>> {
        Err(vec!["get_blocks is not used by the gateway".to_string()])
    }
    async fn visualize_dag(&self, _: &VisualizeDagQuery) -> Result<String, Vec<String>> {
        Err(vec!["visualize_dag is not used by the gateway".to_string()])
    }
    async fn machine_verifiable_dag(&self, _: &MachineVerifyQuery) -> Result<String, Vec<String>> {
        Err(vec![
            "machine_verifiable_dag is not used by the gateway".to_string()
        ])
    }
    async fn find_deploy(&self, _: &FindDeployQuery) -> Result<String, Vec<String>> {
        Err(vec!["find_deploy is not used by the gateway".to_string()])
    }
    async fn listen_for_continuation_at_name(
        &self,
        _: &ContinuationAtNameQuery,
    ) -> Result<Vec<ContinuationsWithBlockInfo>, Vec<String>> {
        Err(vec![
            "listen_for_continuation_at_name is not used by the gateway".to_string(),
        ])
    }
    async fn last_finalized_block(&self) -> Result<String, Vec<String>> {
        Err(vec![
            "last_finalized_block is not used by the gateway".to_string()
        ])
    }
    async fn is_finalized(&self, _: &IsFinalizedQuery) -> Result<String, Vec<String>> {
        Err(vec!["is_finalized is not used by the gateway".to_string()])
    }
    async fn bond_status(&self, _: &BondStatusQuery) -> Result<String, Vec<String>> {
        Err(vec!["bond_status is not used by the gateway".to_string()])
    }
    async fn status(&self) -> Result<String, Vec<String>> {
        Err(vec!["status is not used by the gateway".to_string()])
    }
}

/// One leg of a transaction, as the caller asks for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GatewayLeg {
    pub shard_id: ShardId,
    pub amount: i64,
    pub to: String,
}

/// The on-node 2PC coordinator: opens a durable record for a transaction and drives it to a terminal
/// state, resuming an interrupted one on restart (Law 29).
pub struct GatewayTxn {
    local: Arc<LocalShardDeployService>,
    ledger: Arc<TxnLedger>,
    /// The signing key — the node's validator key. The participant gates `commit`/`abort` on
    /// `deployerId == the coordinator recorded at prepare`, so the same key must run both phases.
    key: PrivateKey,
    coordinator: PublicKey,
    /// Per-phase wait for a deploy to be included in a block.
    phase_timeout: Duration,
}

impl GatewayTxn {
    pub fn new(
        local: Arc<LocalShardDeployService>,
        ledger: Arc<TxnLedger>,
        key: PrivateKey,
        coordinator: PublicKey,
        phase_timeout: Duration,
    ) -> Self {
        GatewayTxn {
            local,
            ledger,
            key,
            coordinator,
            phase_timeout,
        }
    }

    /// The durable record for a transaction, if the node has one.
    pub async fn status(&self, txn_id: &[u8]) -> Result<Option<CoordRecord>, String> {
        self.ledger.get(txn_id).await
    }

    /// Every transaction still awaiting a decision.
    pub async fn list(&self) -> Result<Vec<CoordRecord>, String> {
        self.ledger.in_flight().await
    }

    /// Open (or resume) the transaction and drive it to a terminal state.
    ///
    /// Idempotent under `txn_id` (Law 28's coordinator-side analogue): a terminal record is returned
    /// untouched and **no participant is contacted**, so re-issuing a completed transaction cannot
    /// move funds again.
    pub async fn run(&self, txn_id: &[u8], legs: &[GatewayLeg]) -> Result<CoordRecord, String> {
        if let Some(existing) = self.ledger.get(txn_id).await? {
            if existing.state.is_terminal() {
                return Ok(existing);
            }
            return self.drive(existing).await;
        }
        if legs.is_empty() {
            return Err("a cross-shard transaction needs at least one leg".to_string());
        }
        // Membership is a *caller* error, not a participant failure: a leg on a shard this node does
        // not validate can never prepare, so reject the request up front instead of opening a
        // transaction that is guaranteed to abort. (A leg that fails at the participant — short
        // escrow, timeout — is different: that aborts and compensates, which is Law 27 working.)
        for leg in legs {
            self.local
                .member(&leg.shard_id.to_string())
                .map_err(|err| err.join("; "))?;
        }
        let record = CoordRecord {
            txn_id: txn_id.to_vec(),
            state: CoordState::Proposed,
            coordinator: self.coordinator.clone(),
            legs: legs
                .iter()
                .map(|leg| {
                    Ok(LegRecord {
                        shard_id: leg.shard_id.clone(),
                        amount: NonNegI64::try_from(leg.amount)
                            .map_err(|e| format!("leg amount {}: {e}", leg.amount))?,
                        to: leg.to.clone(),
                    })
                })
                .collect::<Result<Vec<_>, String>>()?,
            votes: Vec::new(),
            reason: None,
        };
        // The record is durable *before* the first leg is sent: a crash after a participant locked
        // its escrow must be recoverable.
        self.ledger.put(&record).await?;
        self.drive(record).await
    }

    /// Every in-flight record, resumed. Called once at startup so a prepared participant's escrow is
    /// not left locked by a node that restarted mid-transaction.
    pub async fn recover_in_flight(&self) -> Result<Vec<CoordRecord>, String> {
        let mut recovered = Vec::new();
        for record in self.ledger.in_flight().await? {
            recovered.push(self.drive(record).await?);
        }
        Ok(recovered)
    }

    /// Drive a record to a terminal state: prepare the legs that have no vote, write the decision,
    /// then apply phase two.
    async fn drive(&self, mut record: CoordRecord) -> Result<CoordRecord, String> {
        // Phase one: prepare every leg that has not voted yet. A leg that already voted is not
        // re-prepared — that is what makes a resume safe.
        if !record.state.is_terminal() {
            for leg in record.legs.clone() {
                if record.vote_for(&leg.shard_id).is_some() {
                    continue;
                }
                let vote = self.prepare_leg(&mut record, &leg).await;
                // Durable as each vote arrives, so a crash mid-phase-one loses nothing.
                self.ledger.put(&record).await?;
                if vote == Vote::Abort {
                    break;
                }
            }
        }

        // Decide. `record_vote` derives the state from the votes: `Committed` once every leg has
        // voted ready, `Aborted` on any abort. A record that is still undecided here has no legs at
        // all (a well-formed transaction always has at least one) — abort it rather than leaving it
        // in flight forever.
        if !record.state.is_terminal() {
            record.state = CoordState::Aborted;
            record.reason = Some("transaction has no legs".to_string());
        }
        // The commit point: the decision is durable *before* any phase-two message, so a crash
        // between the two is finished by `recover_in_flight`.
        self.ledger.put(&record).await?;

        self.apply_phase_two(&record).await;
        // Idempotent rewrite: the phase-two replies are not part of the decision.
        self.ledger.put(&record).await?;
        Ok(record)
    }

    /// Send phase two to every leg that voted ready — the only legs that locked an escrow.
    async fn apply_phase_two(&self, record: &CoordRecord) {
        let method = match record.state {
            CoordState::Committed => "commit",
            CoordState::Aborted => "abort",
            // Undecided records never reach phase two.
            CoordState::Proposed | CoordState::Prepared => return,
        };
        for leg in &record.legs {
            if record.vote_for(&leg.shard_id) != Some(Vote::Ready) {
                continue;
            }
            let _ = self.phase(record, leg, method).await;
        }
    }

    /// Prepare one leg: submit the phase-one deploy and record the participant's vote.
    async fn prepare_leg(&self, record: &mut CoordRecord, leg: &LegRecord) -> Vote {
        let args = vec![
            RhoByteArray::apply(self.coordinator.bytes().to_vec()),
            RhoNumber::apply(i64::from(leg.amount)),
            RhoString::apply(leg.to.clone()),
        ];
        let term = txn_term("prepare", &record.txn_id, &args, true);
        let outcome = self.phase_with_term(leg, &term).await;
        let vote = vote_of(&outcome);
        let reason = match (&outcome, vote) {
            (ShardOutcome::Error(err), _) => Some(format!("{}: {err}", leg.shard_id)),
            (_, Vote::Abort) => Some(format!("{} voted abort", leg.shard_id)),
            (_, Vote::Ready) => None,
        };
        record.record_vote(leg.shard_id.clone(), vote, reason);
        vote
    }

    /// Submit one unbundled phase deploy to a leg's shard and await its reply.
    async fn phase(&self, record: &CoordRecord, leg: &LegRecord, method: &str) -> ShardOutcome {
        let term = txn_term(method, &record.txn_id, &[], true);
        self.phase_with_term(leg, &term).await
    }

    async fn phase_with_term(&self, leg: &LegRecord, term: &str) -> ShardOutcome {
        let shard_id = leg.shard_id.to_string();
        let valid_after = self.local.current_height(&leg.shard_id).await;
        // `Signed<DeployData>` borrows a `&dyn SignaturesAlg`, which is not `Sync`, so it must not be
        // held across an `.await` — the future would stop being `Send` and could not be spawned.
        let (deploy, sig) = {
            let signed =
                match signed_invoke(term, &self.key, 0, 1_000_000, 1, valid_after, &shard_id) {
                    Ok(signed) => signed,
                    Err(err) => return ShardOutcome::Error(err),
                };
            let deploy = SignedDeployData {
                data: signed.data,
                deployer: signed.pk.bytes().to_vec(),
                sig: signed.sig.clone(),
                sig_algorithm: "secp256k1".to_string(),
            };
            (deploy, signed.sig.clone())
        };
        if let Err(err) = self.local.deploy(&deploy).await {
            return ShardOutcome::Error(err.join("; "));
        }
        await_reply(
            &*self.local,
            &sig,
            Duration::from_millis(250),
            self.phase_timeout,
        )
        .await
    }
}

/// Interpret a participant's phase-one reply as a vote — the same reading the client-side
/// coordinator uses ([`crate::txn_coordinator::vote_from_reply`]), so the two cannot drift on what
/// counts as ready.
fn vote_of(outcome: &ShardOutcome) -> Vote {
    if crate::txn_coordinator::vote_from_reply(outcome) {
        Vote::Ready
    } else {
        Vote::Abort
    }
}
