//! Web API request/response data types (port of the DTOs in `api/WebApi.scala`).

use std::fmt;

use rchain_models::casper::protocol::casper_message::DeployData;
use rchain_models::casper::protocol::deploy_service::LightBlockInfo;
use serde::{Deserialize, Serialize};

use super::rho_expr::{RhoExpr, RhoUnforg};

/// A deploy request (port of `DeployRequest`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployRequest {
    pub data: DeployData,
    pub deployer: String,
    pub signature: String,
    pub sig_algorithm: String,
}

/// An exploratory-deploy request (port of `ExploreDeployRequest`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExploreDeployRequest {
    pub term: String,
    pub block_hash: String,
    pub use_pre_state_hash: bool,
}

/// A data-at-name request (port of `DataAtNameRequest`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataAtNameRequest {
    pub name: RhoUnforg,
    pub depth: i32,
}

/// A data-at-name-by-block-hash request (port of `DataAtNameByBlockHashRequest`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataAtNameByBlockHashRequest {
    pub name: RhoExpr,
    pub block_hash: String,
    pub use_pre_state_hash: bool,
}

/// A faucet request: the REV address to fund (devnet-only endpoint).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FaucetRequest {
    pub address: String,
}

/// A faucet response: the deploy id of the transfer (poll `deploy-status/{deployId}` for the result).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FaucetResponse {
    pub deploy_id: String,
    pub amount: i64,
    pub to: String,
}

/// API/node version info (port of `VersionInfo`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionInfo {
    pub api: String,
    pub node: String,
}

/// Node status (port of `ApiStatus`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiStatus {
    pub version: VersionInfo,
    pub address: String,
    pub network_id: String,
    pub shard_id: String,
    pub peers: i32,
    pub nodes: i32,
    pub min_phlo_price: i64,
    pub latest_block_number: i64,
    /// Continuous block production (`--autopropose`).
    pub autopropose: bool,
    /// Propose immediately after a deploy is accepted (`--propose-on-deploy`).
    pub propose_on_deploy: bool,
    /// Blocks are produced only by an explicit `propose`.
    pub manual_propose: bool,
    /// The admin HTTP surface is published and CORS-enabled.
    pub admin_http: bool,
    /// Dev mode is on.
    pub dev_mode: bool,
    /// #157: self-validation failures the proposer has recorded in a row. `0` on a healthy node, and
    /// cleared by a successful propose — this is what tells "quiet" from "broken" on a chain whose
    /// steady state is a node that produces nothing until work arrives.
    pub consecutive_self_validation_failures: u64,
    /// #157: the shard's autopropose **timer** has stopped, and nothing restarts it but the process.
    /// Deliberately not called `halted`: the tap and the admin `POST /api/propose` keep running, so a
    /// node can have a stopped timer and still be producing blocks.
    pub autopropose_timer_halted: bool,
    /// #156: stale-snapshot self-equivocations — the node's own block collided with its already-synced
    /// block at a sequence number derived from a stale parent set. Not a self-validation failure, so it
    /// does not count toward the timer halt; it is reported so a node that never recovers from the race
    /// is still visible.
    pub stale_snapshot_self_equivocations: u64,
    /// C249's F-U10-01: **why** finality is not advancing, as the merge gate last reported it. The
    /// reason existed only on stderr — a node that had stopped advancing said so to whoever was reading
    /// the log, and to nothing a probe could ask. `None` when the last observation saw a merge that
    /// advanced; that is not "healthy", it is "the gate reported no reason".
    pub finality_stall: Option<String>,
    /// …and how many times this process has entered a stall, so a reader can tell "stalled now" from
    /// "stalled and recovered" without watching.
    pub finality_stall_episodes: u64,
    /// #280's ledger, counted: merges that returned a report which was not quiet — a dropped chain, or a
    /// violated invariant. The live incident produced no counter and no line anywhere moving; this is
    /// the number it would have moved, and the log still carries the detail.
    pub non_quiet_merge_reports: u64,
    /// A poisoned lock was recovered from, which means a panic happened while shared state was held.
    /// Monotone and process-wide, because the poisoning is: the panic happened in whichever thread held
    /// the guard (`rspace::lock`). Counted rather than silently absorbed, which is F-U9-03.
    pub poison_recoveries: u64,
}

/// The node's capabilities, returned by `GET /api/v1/capabilities` (the app-facing "can I propose /
/// is the faucet available" surface).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeCapabilities {
    /// Continuous block production (`--autopropose`).
    pub autopropose: bool,
    /// Propose immediately after a deploy is accepted (`--propose-on-deploy`).
    pub propose_on_deploy: bool,
    /// Blocks are produced only by an explicit `propose` (neither of the above).
    pub manual_propose: bool,
    /// The admin HTTP surface (`POST /api/v1/propose` on 40405) is published and CORS-enabled.
    pub admin_http: bool,
    /// Dev mode is on (`--dev-mode`).
    pub dev_mode: bool,
    /// The `/api/v1/faucet` endpoint is available (dev mode + a deployer key).
    pub faucet: bool,
}

/// A pooled (not-yet-included) deploy (an entry in the `/api/v1/deploys` response).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PooledDeploy {
    /// The deploy id (base16 signature) — identical to what `POST /api/v1/deploy` returns and
    /// `deploy-status` accepts.
    pub deploy_id: String,
    pub timestamp: i64,
    pub deployer: String,
    pub term: String,
    pub phlo_price: i64,
    pub phlo_limit: i64,
    pub valid_after_block_number: i64,
}

/// The `/api/v1/deploys` response: the currently-pooled deploys, most-recent-first.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PooledDeploys {
    pub deploys: Vec<PooledDeploy>,
}

/// Exception thrown by the Block API (port of `BlockApiException`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockApiException(pub String);

impl fmt::Display for BlockApiException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for BlockApiException {}

/// A deploy-signature error (port of `SignatureException`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignatureException(pub String);

impl fmt::Display for SignatureException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for SignatureException {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::rho_expr::{RhoExpr, RhoUnforg};

    #[test]
    fn dto_fields_are_accessible() {
        let deploy = DeployRequest {
            data: DeployData {
                attachments: Vec::new(),
                term: "Nil".to_string(),
                timestamp: 0,
                phlo_price: 1,
                phlo_limit: 1,
                valid_after_block_number: 0,
                shard_id: "root".to_string(),
            },
            deployer: "de".to_string(),
            signature: "si".to_string(),
            sig_algorithm: "secp256k1".to_string(),
        };
        assert_eq!(deploy.sig_algorithm, "secp256k1");

        let data_at_name = DataAtNameRequest {
            name: RhoUnforg::UnforgPrivate("ab".to_string()),
            depth: 1,
        };
        assert_eq!(data_at_name.depth, 1);

        let by_hash = DataAtNameByBlockHashRequest {
            name: RhoExpr::ExprString("x".to_string()),
            block_hash: "h".to_string(),
            use_pre_state_hash: false,
        };
        assert!(!by_hash.use_pre_state_hash);

        let status = ApiStatus {
            version: VersionInfo {
                api: "1.0".to_string(),
                node: "2.0".to_string(),
            },
            address: "addr".to_string(),
            network_id: "testnet".to_string(),
            shard_id: "root".to_string(),
            peers: 1,
            nodes: 2,
            min_phlo_price: 3,
            latest_block_number: 4,
            autopropose: true,
            propose_on_deploy: true,
            manual_propose: false,
            admin_http: true,
            dev_mode: true,
            consecutive_self_validation_failures: 0,
            autopropose_timer_halted: false,
            stale_snapshot_self_equivocations: 0,
            finality_stall: None,
            finality_stall_episodes: 0,
            non_quiet_merge_reports: 0,
            poison_recoveries: 0,
        };
        assert_eq!(status.min_phlo_price, 3);
        assert_eq!(status.latest_block_number, 4);
    }

    #[test]
    fn exceptions_carry_messages() {
        let e = BlockApiException("boom".to_string());
        assert_eq!(e.to_string(), "boom");
        let e = SignatureException("bad sig".to_string());
        assert_eq!(e.to_string(), "bad sig");
    }

    /// Every field of a coordinator record survives the DTO mapping — including the aborted shape,
    /// which is the one an operator reads when a transaction fails.
    #[test]
    fn txn_record_dto_maps_every_field() {
        use rchain_casper::gateway::ledger::{CoordRecord, CoordState, LegRecord, Vote};
        use rchain_crypto::public_key::PublicKey;
        use rchain_shared::refined::{NonNegI64, ShardId};

        let shard = |id: &str| ShardId::try_from(id.to_string()).unwrap();
        let mut record = CoordRecord {
            txn_id: vec![0xAA, 0xBB],
            state: CoordState::Proposed,
            coordinator: PublicKey::new(vec![7u8; 65]),
            legs: vec![LegRecord {
                shard_id: shard("/root"),
                amount: NonNegI64::try_from(30).unwrap(),
                to: "dest".to_string(),
            }],
            votes: Vec::new(),
            reason: None,
        };

        // Proposed: no votes, no reason.
        let dto = TxnRecordDto::from_record(&record);
        assert_eq!(dto.txn_id, "aabb");
        assert_eq!(dto.state, "proposed");
        assert_eq!(dto.coordinator.len(), 130, "65 bytes as hex");
        assert_eq!(dto.legs.len(), 1);
        assert_eq!(dto.legs[0].shard_id, "/root");
        assert_eq!(dto.legs[0].amount, 30);
        assert_eq!(dto.legs[0].to, "dest");
        assert!(dto.votes.is_empty());
        assert!(dto.reason.is_none());
        assert_eq!(dto.record_hash.len(), 64, "the record's content address");

        // Aborted with a reason: the terminal shape a client has to interpret.
        record.record_vote(shard("/root"), Vote::Abort, Some("short".to_string()));
        let dto = TxnRecordDto::from_record(&record);
        assert_eq!(dto.state, "aborted");
        assert_eq!(dto.votes.len(), 1);
        assert_eq!(dto.votes[0].shard_id, "/root");
        assert_eq!(dto.votes[0].vote, "abort");
        assert_eq!(dto.reason.as_deref(), Some("short"));
    }

    /// The API speaks `camelCase` both ways, so a client's request and the response it reads back
    /// use the same names.
    #[test]
    fn txn_dtos_round_trip_through_json() {
        let request: TxnRequest = serde_json::from_str(
            r#"{"txnId":"aabb","legs":[{"shardId":"/root","amount":30,"to":"dest"}]}"#,
        )
        .expect("request parses");
        assert_eq!(request.txn_id, "aabb");
        assert_eq!(request.legs[0].shard_id, "/root");
        assert_eq!(request.legs[0].amount, 30);

        let record = TxnRecordDto {
            txn_id: "aabb".to_string(),
            state: "committed".to_string(),
            coordinator: "00".to_string(),
            record_hash: "11".to_string(),
            legs: request.legs.clone(),
            votes: vec![TxnVoteDto {
                shard_id: "/root".to_string(),
                vote: "ready".to_string(),
            }],
            reason: None,
        };
        let json = serde_json::to_string(&record).expect("serializes");
        assert!(json.contains("\"recordHash\""), "{json}");
        assert!(json.contains("\"txnId\""), "{json}");
        assert!(json.contains("\"shardId\""), "{json}");
        let back: TxnRecordDto = serde_json::from_str(&json).expect("round trip");
        assert_eq!(back, record);
    }
}

/// A deploy execution status (port of the `DeployExecStatus` ADT in `WebApi.scala`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all_fields = "camelCase")]
pub enum DeployExecStatus {
    ProcessedWithSuccess {
        deploy_result: Vec<RhoExpr>,
        block: LightBlockInfo,
    },
    ProcessedWithError {
        deploy_error: String,
        block: LightBlockInfo,
    },
    NotProcessed {
        status: String,
    },
}

/// A rholang expression plus the block it was found in (port of `RhoExprWithBlock`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RhoExprWithBlock {
    pub expr: RhoExpr,
    pub block: LightBlockInfo,
}

/// A data-at-name response (port of `DataAtNameResponse`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataAtNameResponse {
    pub exprs: Vec<RhoExprWithBlock>,
    pub length: i32,
}

/// An exploratory-deploy response (port of `ExploratoryDeployResponse`) — what
/// `POST /api/v1/explore-deploy` returns, and **the type the served document has always named for
/// that path** (`node/src/web/http.rs`): the handler returned `RhoDataResponse` instead, which was
/// invisible while the two were field-for-field identical.
///
/// `reply_source` is the part a client could not get before: **which channel the reply was read
/// from**, per the rule in `spec/API-SCHEMA.md`. Without it an empty `expr` meant both "your term
/// produced nothing" and "your reply is on a channel the node does not read" — a dropped reply with
/// nothing to catch it (AUDIT C38).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExploratoryDeployResponse {
    pub expr: Vec<RhoExpr>,
    pub block: LightBlockInfo,
    /// `firstPrivateName`, `out`, or `none` — the channel that answered, named by the rule.
    pub reply_source: String,
}

/// A rho data response (port of `RhoDataResponse`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RhoDataResponse {
    pub expr: Vec<RhoExpr>,
    pub block: LightBlockInfo,
}

// --- Cross-shard transactions (the multi-shard gateway, Laws 26–29) ---

/// `POST /api/v1/txn` — open (or resume) a cross-shard transaction on this node.
///
/// `txn_id` is caller-supplied rather than generated: a retried request must be the *same*
/// transaction, so the coordinator's idempotency (Law 28 at the API boundary) applies to it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TxnRequest {
    pub txn_id: String,
    pub legs: Vec<TxnLegDto>,
}

/// One leg: which shard escrows, how much REV, and where a commit credits it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TxnLegDto {
    pub shard_id: String,
    pub amount: i64,
    pub to: String,
}

/// A recorded vote, as reported.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TxnVoteDto {
    pub shard_id: String,
    pub vote: String,
}

/// The coordinator's durable record, as reported by `POST /api/v1/txn` and `GET /api/v1/txn/:id`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TxnRecordDto {
    pub txn_id: String,
    /// `proposed` | `prepared` | `committed` | `aborted`.
    pub state: String,
    /// The coordinator key the participants gate `commit`/`abort` on.
    pub coordinator: String,
    /// The record's content address — the same decision on two nodes hashes the same, and nothing
    /// about it is consensus state.
    pub record_hash: String,
    pub legs: Vec<TxnLegDto>,
    pub votes: Vec<TxnVoteDto>,
    /// Why an abort happened (a participant error or a timeout), when one did.
    pub reason: Option<String>,
}

impl TxnRecordDto {
    /// Render a coordinator record for the API.
    pub fn from_record(record: &rchain_casper::gateway::ledger::CoordRecord) -> Self {
        TxnRecordDto {
            txn_id: rchain_shared::base16::encode(&record.txn_id),
            state: record.state.as_str().to_string(),
            coordinator: rchain_shared::base16::encode(record.coordinator.bytes()),
            record_hash: rchain_shared::base16::encode(
                rchain_casper::gateway::ledger::record_hash(record).as_bytes(),
            ),
            legs: record
                .legs
                .iter()
                .map(|leg| TxnLegDto {
                    shard_id: leg.shard_id.to_string(),
                    amount: i64::from(leg.amount),
                    to: leg.to.clone(),
                })
                .collect(),
            votes: record
                .votes
                .iter()
                .map(|(shard_id, vote)| TxnVoteDto {
                    shard_id: shard_id.to_string(),
                    vote: vote.as_str().to_string(),
                })
                .collect(),
            reason: record.reason.clone(),
        }
    }
}
