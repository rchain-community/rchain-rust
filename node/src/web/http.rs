//! HTTP server routes (port of the http4s `service` methods in the `web` package and
//! `NewPrometheusReporter.service`).
//!
//! The public server mounts `/version`, `/metrics`, `/status`, the `/api` + `/api/v1` JSON routes,
//! the `/api/v1/openapi.json` OpenAPI document, and the reporting routes (`/reporting/trace` +
//! `/api/trace`). The admin server mounts the `/api`/`/api/v1` admin routes (propose) and its own
//! OpenAPI document. CORS and a per-request timeout are applied to both servers.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tower_http::cors::CorsLayer;
use tower_http::timeout::TimeoutLayer;

use rchain_casper::api::block_api::BlockApi;
use rchain_casper::api::block_report_api::BlockReportApi;
use rchain_casper::gateway::GatewayTxn;
use rchain_casper::protocol::comm_util::ConnectionsCell;
use rchain_comm::discovery::NodeDiscovery;
use rchain_comm::rp::rp_conf::RPConf;
use rchain_models::block_hash::BlockHash;
use rchain_shared::base16;
use rchain_shared::rate_limiter::RateLimiter;
use rchain_shared::refined::{Port, ShardId};

use crate::api::admin_web_api::AdminWebApi;
use crate::api::dto::{
    BlockApiException, DataAtNameByBlockHashRequest, DataAtNameRequest, DeployRequest,
    ExploreDeployRequest, FaucetRequest, TxnRecordDto, TxnRequest,
};
use crate::api::grpc::DEFAULT_API_RATE_LIMIT_PER_SEC;
use crate::api::web_api::WebApi;
use crate::diagnostics::NewPrometheusReporter;
use crate::web::reporting::transform_result;
use crate::web::status_info;
use crate::web::version_info;

/// Rate limit for the devnet faucet endpoint. Much stricter than the deploy route because each
/// request transfers real (dev) REV.
const FAUCET_RATE_LIMIT_PER_SEC: u64 = 1;

/// Comm state needed by `GET /status` (port of the `ConnectionsCell`/`NodeDiscovery`/`RPConfAsk`
/// arguments of `StatusInfo.service`).
#[derive(Clone)]
pub struct StatusProvider {
    pub connections: ConnectionsCell,
    pub rp_conf: RPConf,
    pub discovery: Arc<dyn NodeDiscovery>,
}

/// The shards a node validates for, primary first, each with the API that reads its head. Served by
/// `GET /api/v1/shards` — deliberately *not* folded into `/api/status`, whose `shardId` field
/// existing tooling extracts with a single greedy match.
#[derive(Clone)]
pub struct ShardRegistry {
    pub primary: ShardId,
    pub members: Vec<(ShardId, Arc<dyn BlockApi>)>,
}

/// State shared by the public HTTP server (port of the `webApi` + `prometheusReporter` +
/// `blockReportAPI` arguments of `acquireHttpServer`).
#[derive(Clone)]
pub struct HttpState {
    pub reporter: Arc<NewPrometheusReporter>,
    pub web_api: Arc<dyn WebApi>,
    pub block_report_api: Arc<BlockReportApi>,
    pub shards: Arc<ShardRegistry>,
    /// The on-node cross-shard 2PC coordinator, present only when this node is a multi-shard
    /// gateway (several memberships and a signing key).
    pub gateway: Option<Arc<GatewayTxn>>,
    /// Whether the cross-shard transaction routes are enabled (`api-server.enable-txn-api`).
    pub enable_txn_api: bool,
    pub status_provider: Option<StatusProvider>,
    pub enable_reporting: bool,
    /// Rate limiter for the unauthenticated deploy/explore-deploy routes (documented Scala
    /// deviation: the Scala HTTP deploy routes are unlimited).
    pub deploy_rate_limiter: Arc<RateLimiter>,
    /// Rate limiter for the devnet faucet endpoint.
    pub faucet_rate_limiter: Arc<RateLimiter>,
}

/// State shared by the admin HTTP server (port of the `adminWebApiRoutes` argument of
/// `acquireAdminHttpServer`).
#[derive(Clone)]
pub struct AdminState {
    pub admin_web_api: Arc<dyn AdminWebApi>,
    pub enable_devnet_cors: bool,
}

/// `GET /version` (port of `VersionInfo.service`): the node version string.
pub async fn version() -> String {
    version_info::get(env!("CARGO_PKG_VERSION"), None)
}

/// `GET /metrics` (port of `NewPrometheusReporter.service`): the Prometheus scrape data.
pub async fn metrics(State(state): State<HttpState>) -> String {
    state.reporter.scrape_data()
}

/// `GET /status` (port of `StatusInfo.service`): the node address, version, and peer/node counts.
pub async fn status(State(state): State<HttpState>) -> Response {
    match &state.status_provider {
        Some(provider) => {
            let connections = provider.connections.read().await;
            let discovered = provider.discovery.peers();
            let version = version_info::get(env!("CARGO_PKG_VERSION"), None);
            let status =
                status_info::status(&version, &connections, &discovered, &provider.rp_conf);
            (StatusCode::OK, Json(status)).into_response()
        }
        None => (StatusCode::NOT_FOUND, ()).into_response(),
    }
}

/// Map a `WebApi`/`AdminWebApi` result to an HTTP response: `200` with a JSON body on success,
/// `400` with a JSON error string on `BlockApiException` (port of the `handleResponseError`
/// handler in `WebApiRoutes`).
fn json_result<T: Serialize>(result: Result<T, BlockApiException>) -> Response {
    match result {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(err) => (StatusCode::BAD_REQUEST, Json(err.0)).into_response(),
    }
}

// --- Web API routes (port of `WebApiRoutes.service`) ---

async fn api_status(State(state): State<HttpState>) -> Response {
    json_result(state.web_api.status().await)
}

async fn api_deploys(State(state): State<HttpState>) -> Response {
    json_result(state.web_api.pooled_deploys().await)
}

async fn api_capabilities(State(state): State<HttpState>) -> Response {
    json_result(state.web_api.capabilities().await)
}

/// `GET /api/v1/shards` — the shards this node is a member of (a gateway is a member of several),
/// primary first, each with the height of its own chain. A client that needs to address a specific
/// shard reads the ids here; a deploy already names its shard in `DeployData.shardId`.
async fn api_shards(State(state): State<HttpState>) -> Response {
    let mut shards = Vec::with_capacity(state.shards.members.len());
    for (shard_id, api) in &state.shards.members {
        let status = api.status().await;
        shards.push(json!({
            "shardId": shard_id.to_string(),
            "primary": *shard_id == state.shards.primary,
            "latestBlockNumber": status.latest_block_number,
        }));
    }
    Json(json!({
        "primaryShard": state.shards.primary.to_string(),
        "shardCount": shards.len(),
        "shards": shards,
    }))
    .into_response()
}

/// The gateway, or the standard not-available response.
///
/// Mirrors the `enable-reporting` convention: the route is mounted unconditionally and answers 404
/// when the feature is off, so a single-shard node's surface is unchanged and a client can tell
/// "not a gateway" from "bad request".
fn gateway_or_not_found(state: &HttpState) -> Result<Arc<GatewayTxn>, Response> {
    match (&state.gateway, state.enable_txn_api) {
        (Some(gateway), true) => Ok(gateway.clone()),
        _ => Err((StatusCode::NOT_FOUND, ()).into_response()),
    }
}

/// `POST /api/v1/txn` — open (or resume) a cross-shard transaction and drive it to a terminal state.
///
/// The legs are ordinary deploys on this node's own shards, so the call returns once each leg has
/// been included in a block — seconds, not microseconds — and can fail by timeout. It is
/// idempotent under `txnId`: re-issuing a completed transaction returns its record and moves no
/// funds.
async fn api_txn_run(State(state): State<HttpState>, Json(req): Json<TxnRequest>) -> Response {
    let gateway = match gateway_or_not_found(&state) {
        Ok(gateway) => gateway,
        Err(response) => return response,
    };
    let txn_id = match base16::decode(&req.txn_id) {
        Some(bytes) if !bytes.is_empty() && bytes.len() <= 64 => bytes,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json("txnId must be non-empty hex of at most 64 bytes".to_string()),
            )
                .into_response()
        }
    };
    let mut legs = Vec::with_capacity(req.legs.len());
    for leg in &req.legs {
        match rchain_shared::refined::ShardId::try_from(leg.shard_id.clone()) {
            Ok(shard_id) => legs.push(rchain_casper::gateway::GatewayLeg {
                shard_id,
                amount: leg.amount,
                to: leg.to.clone(),
            }),
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(format!("invalid shardId '{}': {e}", leg.shard_id)),
                )
                    .into_response()
            }
        }
    }

    match gateway.run(&txn_id, &legs).await {
        Ok(record) => (StatusCode::OK, Json(TxnRecordDto::from_record(&record))).into_response(),
        Err(err) => (StatusCode::BAD_REQUEST, Json(err)).into_response(),
    }
}

/// `GET /api/v1/txn/:txnId` — the durable record of a transaction, or 404 when this node has none.
async fn api_txn_status(State(state): State<HttpState>, Path(txn_id): Path<String>) -> Response {
    let gateway = match gateway_or_not_found(&state) {
        Ok(gateway) => gateway,
        Err(response) => return response,
    };
    let Some(txn_id) = base16::decode(&txn_id) else {
        return (
            StatusCode::BAD_REQUEST,
            Json("txnId must be hex".to_string()),
        )
            .into_response();
    };
    match gateway.status(&txn_id).await {
        Ok(Some(record)) => {
            (StatusCode::OK, Json(TxnRecordDto::from_record(&record))).into_response()
        }
        Ok(None) => (StatusCode::NOT_FOUND, ()).into_response(),
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, Json(err)).into_response(),
    }
}

/// `GET /api/v1/txn` — the transactions this node is still coordinating.
async fn api_txn_list(State(state): State<HttpState>) -> Response {
    let gateway = match gateway_or_not_found(&state) {
        Ok(gateway) => gateway,
        Err(response) => return response,
    };
    match gateway.list().await {
        Ok(records) => {
            let records: Vec<TxnRecordDto> =
                records.iter().map(TxnRecordDto::from_record).collect();
            Json(json!({ "inFlight": records })).into_response()
        }
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, Json(err)).into_response(),
    }
}

async fn api_deploy(State(state): State<HttpState>, Json(req): Json<DeployRequest>) -> Response {
    if !state.deploy_rate_limiter.allow() {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json("deploy rate limit exceeded".to_string()),
        )
            .into_response();
    }
    json_result(state.web_api.deploy(&req).await)
}

async fn api_faucet(State(state): State<HttpState>, Json(req): Json<FaucetRequest>) -> Response {
    if !state.faucet_rate_limiter.allow() {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json("faucet rate limit exceeded".to_string()),
        )
            .into_response();
    }
    json_result(state.web_api.faucet(&req.address).await)
}

async fn api_explore_deploy(State(state): State<HttpState>, Json(term): Json<String>) -> Response {
    if !state.deploy_rate_limiter.allow() {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json("deploy rate limit exceeded".to_string()),
        )
            .into_response();
    }
    json_result(state.web_api.exploratory_deploy(&term, None, false).await)
}

async fn api_explore_deploy_by_block_hash(
    State(state): State<HttpState>,
    Json(req): Json<ExploreDeployRequest>,
) -> Response {
    if !state.deploy_rate_limiter.allow() {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json("deploy rate limit exceeded".to_string()),
        )
            .into_response();
    }
    let block_hash = if req.block_hash.is_empty() {
        None
    } else {
        Some(req.block_hash.as_str())
    };
    json_result(
        state
            .web_api
            .exploratory_deploy(&req.term, block_hash, req.use_pre_state_hash)
            .await,
    )
}

async fn api_data_at_name(
    State(state): State<HttpState>,
    Json(req): Json<DataAtNameRequest>,
) -> Response {
    json_result(state.web_api.listen_for_data_at_name(&req).await)
}

async fn api_data_at_name_by_block_hash(
    State(state): State<HttpState>,
    Json(req): Json<DataAtNameByBlockHashRequest>,
) -> Response {
    json_result(state.web_api.get_data_at_par(&req).await)
}

async fn api_last_finalized_block(State(state): State<HttpState>) -> Response {
    json_result(state.web_api.last_finalized_block().await)
}

async fn api_get_block(State(state): State<HttpState>, Path(hash): Path<String>) -> Response {
    json_result(state.web_api.get_block(&hash).await)
}

async fn api_get_blocks(State(state): State<HttpState>) -> Response {
    json_result(state.web_api.get_blocks(1).await)
}

async fn api_get_blocks_by_heights(
    State(state): State<HttpState>,
    Path((start, end)): Path<(i64, i64)>,
) -> Response {
    json_result(state.web_api.get_blocks_by_heights(start, end).await)
}

async fn api_get_blocks_by_depth(
    State(state): State<HttpState>,
    Path(depth): Path<i32>,
) -> Response {
    json_result(state.web_api.get_blocks(depth).await)
}

async fn api_find_deploy(
    State(state): State<HttpState>,
    Path(deploy_id): Path<String>,
) -> Response {
    json_result(state.web_api.find_deploy(&deploy_id).await)
}

async fn api_is_finalized(State(state): State<HttpState>, Path(hash): Path<String>) -> Response {
    json_result(state.web_api.is_finalized(&hash).await)
}

async fn api_get_transaction(State(state): State<HttpState>, Path(hash): Path<String>) -> Response {
    // Reporting is disabled by default (`api-server.enable-reporting = false`); the route answers
    // 404 unless explicitly enabled (M6 — the flag was read but never enforced for the transaction
    // route).
    if !state.enable_reporting {
        return (StatusCode::NOT_FOUND, ()).into_response();
    }
    if !state.deploy_rate_limiter.allow() {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json("deploy rate limit exceeded".to_string()),
        )
            .into_response();
    }
    json_result(state.web_api.get_transaction(&hash).await)
}

// --- Web API v1 routes (port of `WebApiRoutesV1`; the OpenAPI schema is served below) ---

async fn api_v1_deploy_status(
    State(state): State<HttpState>,
    Path(deploy_signature): Path<String>,
) -> Response {
    json_result(state.web_api.deploy_status(&deploy_signature).await)
}

/// `GET /api/v1/openapi.json` — the OpenAPI 3.0 document describing the v1 API. Hand-written from the
/// endpoint DTOs (the Scala derives the same schema from its endpoints4s algebra).
const OPENAPI_JSON: &str = r##"{
  "openapi": "3.0.0",
  "info": { "title": "RNode API", "version": "1.0" },
  "paths": {
    "/status": {
      "get": {
        "summary": "Node status",
        "responses": {
          "200": {
            "description": "Node version, address and peer/node counts",
            "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ApiStatus" } } }
          }
        }
      }
    },
    "/shards": {
      "get": {
        "summary": "The shards this node is a member of",
        "description": "Primary shard first, each with the height of its own chain. A gateway node is a member of several. Deploys name their target shard in `DeployRequest.data.shardId`.",
        "responses": {
          "200": { "description": "Membership list", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ShardsResponse" } } } }
        }
      }
    },
    "/txn": {
      "post": {
        "summary": "Open or resume a cross-shard transaction",
        "description": "Runs a two-phase commit across this node's own member shards: each leg is an ordinary deploy on the shard that owns it, so the call returns once every leg is in a block (seconds) and can fail by timeout. Idempotent under `txnId`: re-issuing a completed transaction returns its record and moves no funds. 404 when this node is not a multi-shard gateway.",
        "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/TxnRequest" } } } },
        "responses": {
          "200": { "description": "The durable coordinator record", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/TxnRecord" } } } },
          "400": { "description": "Invalid txnId, a non-member shard, or a failed leg" },
          "404": { "description": "This node is not a gateway (`api-server.enable-txn-api` is off, or it has a single shard)" }
        }
      },
      "get": {
        "summary": "Transactions this node is still coordinating",
        "responses": {
          "200": { "description": "The in-flight records", "content": { "application/json": { "schema": { "type": "object", "properties": { "inFlight": { "type": "array", "items": { "$ref": "#/components/schemas/TxnRecord" } } } } } } },
          "404": { "description": "This node is not a gateway" }
        }
      }
    },
    "/txn/{txnId}": {
      "get": {
        "summary": "A cross-shard transaction's durable record",
        "parameters": [ { "name": "txnId", "in": "path", "required": true, "schema": { "type": "string" }, "description": "Hex-encoded transaction id" } ],
        "responses": {
          "200": { "description": "The coordinator record", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/TxnRecord" } } } },
          "400": { "description": "txnId is not hex" },
          "404": { "description": "Unknown transaction, or this node is not a gateway" }
        }
      }
    },
    "/deploy": {
      "post": {
        "summary": "Deploy a signed rholang term",
        "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/DeployRequest" } } } },
        "responses": {
          "200": { "description": "Deploy accepted", "content": { "application/json": { "schema": { "type": "string" } } } },
          "400": { "description": "Invalid deploy" }
        }
      }
    },
    "/deploy-status/{deploySignature}": {
      "get": {
        "summary": "Deploy execution status",
        "parameters": [ { "name": "deploySignature", "in": "path", "required": true, "schema": { "type": "string" } } ],
        "responses": {
          "200": { "description": "Deploy execution status", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/DeployExecStatus" } } } },
          "400": { "description": "Invalid deploy signature" }
        }
      }
    },
    "/explore-deploy": {
      "post": {
        "summary": "Run an exploratory deploy",
        "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ExploreDeployRequest" } } } },
        "responses": {
          "200": { "description": "Result expression", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ExploratoryDeployResponse" } } } },
          "400": { "description": "Deploy failed" }
        }
      }
    },
    "/explore-deploy-by-block-hash": {
      "post": {
        "summary": "Exploratory deploy at a block hash",
        "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ExploreDeployRequest" } } } },
        "responses": {
          "200": { "description": "Result expression", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ExploratoryDeployResponse" } } } },
          "400": { "description": "Deploy failed" }
        }
      }
    },
    "/data-at-name-by-block-hash": {
      "post": {
        "summary": "Data at a name, at a block hash",
        "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/DataAtNameByBlockHashRequest" } } } },
        "responses": {
          "200": { "description": "Data at the name", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/RhoDataResponse" } } } },
          "400": { "description": "Invalid request" }
        }
      }
    },
    "/blocks": {
      "get": {
        "summary": "Recent blocks",
        "responses": {
          "200": { "description": "Recent lightweight block info", "content": { "application/json": { "schema": { "type": "array", "items": { "$ref": "#/components/schemas/LightBlockInfo" } } } } }
        }
      }
    },
    "/block/{hash}": {
      "get": {
        "summary": "A block by hash",
        "parameters": [ { "name": "hash", "in": "path", "required": true, "schema": { "type": "string" } } ],
        "responses": {
          "200": { "description": "Block and its deploys", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/BlockInfo" } } } },
          "400": { "description": "Invalid block hash" }
        }
      }
    },
    "/propose": {
      "post": {
        "summary": "Propose a block",
        "responses": {
          "200": { "description": "Proposal result", "content": { "application/json": { "schema": { "type": "string" } } } }
        }
      }
    }
  },
  "components": {
    "schemas": {
      "VersionInfo": {
        "type": "object",
        "properties": {
          "api": { "type": "string" },
          "node": { "type": "string" }
        }
      },
      "TxnLeg": {
        "type": "object",
        "properties": {
          "shardId": { "type": "string", "description": "The member shard that escrows this leg" },
          "amount": { "type": "integer", "description": "REV to escrow" },
          "to": { "type": "string", "description": "The REV address a commit credits" }
        }
      },
      "TxnRequest": {
        "type": "object",
        "properties": {
          "txnId": { "type": "string", "description": "Hex; caller-supplied so a retry is the same transaction" },
          "legs": { "type": "array", "items": { "$ref": "#/components/schemas/TxnLeg" } }
        }
      },
      "TxnVote": {
        "type": "object",
        "properties": {
          "shardId": { "type": "string" },
          "vote": { "type": "string", "enum": ["ready", "abort"] }
        }
      },
      "TxnRecord": {
        "type": "object",
        "properties": {
          "txnId": { "type": "string" },
          "state": { "type": "string", "enum": ["proposed", "prepared", "committed", "aborted"] },
          "coordinator": { "type": "string", "description": "The key the participants gate commit/abort on" },
          "recordHash": { "type": "string", "description": "The record's content address" },
          "legs": { "type": "array", "items": { "$ref": "#/components/schemas/TxnLeg" } },
          "votes": { "type": "array", "items": { "$ref": "#/components/schemas/TxnVote" } },
          "reason": { "type": "string", "nullable": true, "description": "Why an abort happened, when one did" }
        }
      },
      "ShardsResponse": {
        "type": "object",
        "properties": {
          "primaryShard": { "type": "string", "description": "The default target for requests that do not name a shard" },
          "shardCount": { "type": "integer" },
          "shards": {
            "type": "array",
            "items": {
              "type": "object",
              "properties": {
                "shardId": { "type": "string" },
                "primary": { "type": "boolean" },
                "latestBlockNumber": { "type": "integer" }
              }
            }
          }
        }
      },
      "ApiStatus": {
        "type": "object",
        "properties": {
          "version": { "$ref": "#/components/schemas/VersionInfo" },
          "address": { "type": "string" },
          "networkId": { "type": "string" },
          "shardId": { "type": "string" },
          "peers": { "type": "integer", "format": "int32" },
          "nodes": { "type": "integer", "format": "int32" },
          "minPhloPrice": { "type": "integer", "format": "int64" },
          "latestBlockNumber": { "type": "integer", "format": "int64" }
        }
      },
      "DeployData": {
        "type": "object",
        "properties": {
          "term": { "type": "string" },
          "timestamp": { "type": "integer", "format": "int64" },
          "phloPrice": { "type": "integer", "format": "int64" },
          "phloLimit": { "type": "integer", "format": "int64" },
          "validAfterBlockNumber": { "type": "integer", "format": "int64" },
          "shardId": { "type": "string" }
        }
      },
      "DeployRequest": {
        "type": "object",
        "properties": {
          "data": { "$ref": "#/components/schemas/DeployData" },
          "deployer": { "type": "string" },
          "signature": { "type": "string" },
          "sigAlgorithm": { "type": "string" }
        }
      },
      "BondInfo": {
        "type": "object",
        "properties": {
          "validator": { "type": "string" },
          "stake": { "type": "integer", "format": "int64" }
        }
      },
      "LightBlockInfo": {
        "type": "object",
        "properties": {
          "version": { "type": "integer", "format": "int32" },
          "shardId": { "type": "string" },
          "blockHash": { "type": "string" },
          "blockNumber": { "type": "integer", "format": "int64" },
          "sender": { "type": "string" },
          "seqNum": { "type": "integer", "format": "int64" },
          "preStateHash": { "type": "string" },
          "postStateHash": { "type": "string" },
          "justifications": { "type": "array", "items": { "type": "string" } },
          "bonds": { "type": "array", "items": { "$ref": "#/components/schemas/BondInfo" } },
          "sigAlgorithm": { "type": "string" },
          "sig": { "type": "string" },
          "blockSize": { "type": "string" },
          "deployCount": { "type": "integer", "format": "int32" },
          "rejectedDeploys": { "type": "array", "items": { "type": "string" } }
        }
      },
      "DeployInfo": {
        "type": "object",
        "properties": {
          "deployer": { "type": "string" },
          "term": { "type": "string" },
          "timestamp": { "type": "integer", "format": "int64" },
          "sig": { "type": "string" },
          "sigAlgorithm": { "type": "string" },
          "phloPrice": { "type": "integer", "format": "int64" },
          "phloLimit": { "type": "integer", "format": "int64" },
          "validAfterBlockNumber": { "type": "integer", "format": "int64" },
          "cost": { "type": "integer", "format": "int64" },
          "errored": { "type": "boolean" },
          "systemDeployError": { "type": "string" }
        }
      },
      "BlockInfo": {
        "type": "object",
        "properties": {
          "blockInfo": { "$ref": "#/components/schemas/LightBlockInfo" },
          "deploys": { "type": "array", "items": { "$ref": "#/components/schemas/DeployInfo" } }
        }
      },
      "ExploreDeployRequest": {
        "type": "object",
        "properties": {
          "term": { "type": "string" },
          "blockHash": { "type": "string" },
          "usePreStateHash": { "type": "boolean" }
        }
      },
      "DataAtNameByBlockHashRequest": {
        "type": "object",
        "properties": {
          "name": { "type": "object", "description": "A rholang expression" },
          "blockHash": { "type": "string" },
          "usePreStateHash": { "type": "boolean" }
        }
      },
      "RhoDataResponse": {
        "type": "object",
        "properties": {
          "expr": { "type": "array", "items": { "type": "object", "description": "A rholang expression" } },
          "block": { "$ref": "#/components/schemas/LightBlockInfo" }
        }
      },
      "ExploratoryDeployResponse": {
        "type": "object",
        "properties": {
          "expr": { "type": "array", "items": { "type": "object", "description": "A rholang expression" } },
          "block": { "$ref": "#/components/schemas/LightBlockInfo" }
        }
      },
      "DeployExecStatus": {
        "oneOf": [
          { "type": "object", "properties": { "deployResult": { "type": "array", "items": { "type": "object" } }, "block": { "$ref": "#/components/schemas/LightBlockInfo" } } },
          { "type": "object", "properties": { "deployError": { "type": "string" }, "block": { "$ref": "#/components/schemas/LightBlockInfo" } } },
          { "type": "object", "properties": { "status": { "type": "string" } } }
        ]
      }
    }
  }
}"##;

async fn api_v1_openapi() -> Response {
    let doc: serde_json::Value =
        serde_json::from_str(OPENAPI_JSON).unwrap_or_else(|_| serde_json::Value::Null);
    (StatusCode::OK, Json(doc)).into_response()
}

// --- Reporting routes (port of `ReportingRoutes.service`) ---

/// `GET /reporting/trace` query params (`blockHash` + optional `forceReplay`).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReportingQuery {
    block_hash: String,
    force_replay: Option<bool>,
}

async fn reporting_trace(
    State(state): State<HttpState>,
    Query(query): Query<ReportingQuery>,
) -> Response {
    // Reporting is disabled by default (`api-server.enable-reporting = false`); the route answers
    // 404 unless explicitly enabled (M6 — the flag was read but never enforced).
    if !state.enable_reporting {
        return (StatusCode::NOT_FOUND, ()).into_response();
    }
    if !state.deploy_rate_limiter.allow() {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json("deploy rate limit exceeded".to_string()),
        )
            .into_response();
    }
    // Validate-on-ingress: a malformed block hash (non-hex / wrong length) must be a 400, not a
    // panic in `BlockHash::from_hex`.
    let hash = match BlockHash::try_from_hex(&query.block_hash) {
        Ok(h) => h,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json("invalid block hash".to_string()),
            )
                .into_response()
        }
    };
    let result = state
        .block_report_api
        .block_report(&hash, query.force_replay.unwrap_or(false))
        .await;
    (StatusCode::OK, Json(transform_result(result))).into_response()
}

// --- Admin Web API routes (port of `AdminWebApiRoutes.service`) ---

async fn admin_propose(State(state): State<AdminState>) -> Response {
    json_result(state.admin_web_api.propose().await)
}

/// Build the public HTTP routes (port of `acquireHttpServer`'s route map: `/version`, `/metrics`,
/// `/status`, the `/api` JSON routes, `/reporting` + `/api/trace`, the `/api/v1` routes, and the
/// `/api/v1/openapi.json` OpenAPI document).
pub fn router(state: HttpState) -> Router {
    Router::new()
        .route("/version", get(version))
        .route("/metrics", get(metrics))
        .route("/status", get(status))
        .route("/reporting/trace", get(reporting_trace))
        .route("/api/trace", get(reporting_trace))
        .route("/api/status", get(api_status))
        .route("/api/capabilities", get(api_capabilities))
        .route("/api/shards", get(api_shards))
        .route("/api/txn", get(api_txn_list).post(api_txn_run))
        .route("/api/txn/:txn_id", get(api_txn_status))
        .route("/api/deploys", get(api_deploys))
        .route("/api/deploy", post(api_deploy))
        .route("/api/faucet", post(api_faucet))
        .route("/api/explore-deploy", post(api_explore_deploy))
        .route(
            "/api/explore-deploy-by-block-hash",
            post(api_explore_deploy_by_block_hash),
        )
        .route("/api/data-at-name", post(api_data_at_name))
        .route(
            "/api/data-at-name-by-block-hash",
            post(api_data_at_name_by_block_hash),
        )
        .route("/api/last-finalized-block", get(api_last_finalized_block))
        .route("/api/block/:hash", get(api_get_block))
        .route("/api/blocks", get(api_get_blocks))
        .route("/api/blocks/:start/:end", get(api_get_blocks_by_heights))
        .route("/api/blocks/:depth", get(api_get_blocks_by_depth))
        .route("/api/deploy/:deploy_id", get(api_find_deploy))
        .route("/api/is-finalized/:hash", get(api_is_finalized))
        .route("/api/transactions/:hash", get(api_get_transaction))
        .route("/api/v1/status", get(api_status))
        .route("/api/v1/capabilities", get(api_capabilities))
        .route("/api/v1/shards", get(api_shards))
        .route("/api/v1/txn", get(api_txn_list).post(api_txn_run))
        .route("/api/v1/txn/:txn_id", get(api_txn_status))
        .route("/api/v1/deploys", get(api_deploys))
        .route("/api/v1/deploy", post(api_deploy))
        .route("/api/v1/faucet", post(api_faucet))
        .route(
            "/api/v1/deploy-status/:deploy_signature",
            get(api_v1_deploy_status),
        )
        .route("/api/v1/explore-deploy", post(api_explore_deploy))
        .route(
            "/api/v1/explore-deploy-by-block-hash",
            post(api_explore_deploy_by_block_hash),
        )
        .route(
            "/api/v1/data-at-name-by-block-hash",
            post(api_data_at_name_by_block_hash),
        )
        .route("/api/v1/blocks", get(api_get_blocks))
        .route("/api/v1/block/:hash", get(api_get_block))
        .route("/api/v1/openapi.json", get(api_v1_openapi))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

/// Build the admin HTTP routes (port of `acquireAdminHttpServer`'s `/api` + `/api/v1` admin routes).
pub fn admin_router(state: AdminState) -> Router {
    // Restrictive CORS (no allowed origins) by default, so a browser on another origin cannot
    // trigger block production via the loopback admin server (H-3). Devnet / browser-wallet access
    // opts into permissive CORS via `api-server.enable-devnet-cors`.
    let cors = if state.enable_devnet_cors {
        CorsLayer::permissive()
    } else {
        CorsLayer::new()
    };
    Router::new()
        .route("/api/propose", post(admin_propose))
        .route("/api/v1/propose", post(admin_propose))
        .route("/api/v1/openapi.json", get(api_v1_openapi))
        .layer(cors)
        .with_state(state)
}

/// Bind and serve the public HTTP routes (port of `web/acquireHttpServer`), with a CORS layer and a
/// per-request timeout (`api-server.max-connection-idle`).
#[allow(clippy::too_many_arguments)]
pub async fn acquire_http_server(
    host: &str,
    port: Port,
    reporter: Arc<NewPrometheusReporter>,
    web_api: Arc<dyn WebApi>,
    block_report_api: Arc<BlockReportApi>,
    shards: Arc<ShardRegistry>,
    gateway: Option<Arc<GatewayTxn>>,
    status_provider: Option<StatusProvider>,
    max_connection_idle: Duration,
    enable_reporting: bool,
    enable_txn_api: bool,
) -> Result<(), String> {
    let port = u16::from(port); // single discharge at the bind boundary
    let addr: SocketAddr = format!("{host}:{port}")
        .parse()
        .map_err(|e| format!("invalid bind address {host}:{port}: {e}"))?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| e.to_string())?;
    let app = router(HttpState {
        reporter,
        web_api,
        block_report_api,
        shards,
        gateway,
        enable_txn_api,
        status_provider,
        enable_reporting,
        deploy_rate_limiter: Arc::new(RateLimiter::new(DEFAULT_API_RATE_LIMIT_PER_SEC)),
        faucet_rate_limiter: Arc::new(RateLimiter::new(FAUCET_RATE_LIMIT_PER_SEC)),
    })
    .layer(TimeoutLayer::new(max_connection_idle));
    axum::serve(listener, app).await.map_err(|e| e.to_string())
}

/// Bind and serve the admin HTTP routes (port of `web/acquireAdminHttpServer`).
pub async fn acquire_admin_http_server(
    host: &str,
    port: Port,
    admin_web_api: Arc<dyn AdminWebApi>,
    enable_devnet_cors: bool,
    max_connection_idle: Duration,
) -> Result<(), String> {
    let port = u16::from(port); // single discharge at the bind boundary
    let addr: SocketAddr = format!("{host}:{port}")
        .parse()
        .map_err(|e| format!("invalid bind address {host}:{port}: {e}"))?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| e.to_string())?;
    let app = admin_router(AdminState {
        admin_web_api,
        enable_devnet_cors,
    })
    .layer(TimeoutLayer::new(max_connection_idle));
    axum::serve(listener, app).await.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::dto::{
        ApiStatus, DataAtNameResponse, DeployExecStatus, FaucetResponse, NodeCapabilities,
        PooledDeploy, PooledDeploys, RhoDataResponse, VersionInfo,
    };
    use crate::diagnostics::scrape_data_builder::Configuration;
    use crate::web::transaction::TransactionResponse;
    use async_trait::async_trait;
    use axum::body::to_bytes;
    use rchain_block_storage::dag::codecs::{BlockHashCodec, BlockMessageCodec};
    use rchain_casper::reporting::noop;
    use rchain_comm::peer_node::{NodeIdentifier, PeerNode};
    use rchain_comm::rp::rp_conf::ClearConnectionsConf;
    use rchain_models::casper::protocol::deploy_service::{BlockInfo, LightBlockInfo};
    use rchain_models::casper::protocol::report::BlockEventInfo;
    use rchain_shared::store::InMemoryKeyValueStore;
    use rchain_shared::typed_store::{Codec, KeyValueTypedStoreCodec, SharedStore};
    use std::marker::PhantomData;

    struct JsonCodec<T>(PhantomData<T>);

    struct NoopDiscovery;
    #[async_trait]
    impl NodeDiscovery for NoopDiscovery {
        async fn discover(&self) {}
        fn peers(&self) -> Vec<PeerNode> {
            Vec::new()
        }
    }

    impl<T: Serialize + serde::de::DeserializeOwned + Send + Sync> Codec<T> for JsonCodec<T> {
        fn encode(&self, value: &T) -> Vec<u8> {
            serde_json::to_vec(value).expect("json encode")
        }

        fn decode(&self, bytes: &[u8]) -> Result<T, String> {
            serde_json::from_slice(bytes).map_err(|e| e.to_string())
        }
    }

    fn test_block_report_api() -> Arc<BlockReportApi> {
        let store: SharedStore = Arc::new(tokio::sync::Mutex::new(Box::new(
            InMemoryKeyValueStore::default(),
        )));
        let block_store = Arc::new(KeyValueTypedStoreCodec::new(
            store.clone(),
            Arc::new(BlockHashCodec),
            Arc::new(BlockMessageCodec),
        ));
        let report_store = Arc::new(KeyValueTypedStoreCodec::new(
            store,
            Arc::new(BlockHashCodec),
            Arc::new(JsonCodec::<BlockEventInfo>(PhantomData)),
        ));
        Arc::new(BlockReportApi::new(
            block_store,
            Arc::new(noop()),
            report_store,
            None,
        ))
    }

    fn test_status() -> ApiStatus {
        ApiStatus {
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
        }
    }

    struct MockWebApi {
        status: ApiStatus,
        pooled_deploys: PooledDeploys,
    }

    #[async_trait]
    impl WebApi for MockWebApi {
        async fn status(&self) -> Result<ApiStatus, BlockApiException> {
            Ok(self.status.clone())
        }

        async fn deploy(&self, _: &DeployRequest) -> Result<String, BlockApiException> {
            unimplemented!()
        }

        async fn deploy_status(&self, _: &str) -> Result<DeployExecStatus, BlockApiException> {
            unimplemented!()
        }

        async fn pooled_deploys(&self) -> Result<PooledDeploys, BlockApiException> {
            Ok(self.pooled_deploys.clone())
        }

        async fn capabilities(&self) -> Result<NodeCapabilities, BlockApiException> {
            unimplemented!()
        }

        async fn faucet(&self, _: &str) -> Result<FaucetResponse, BlockApiException> {
            unimplemented!()
        }

        async fn listen_for_data_at_name(
            &self,
            _: &DataAtNameRequest,
        ) -> Result<DataAtNameResponse, BlockApiException> {
            unimplemented!()
        }

        async fn get_data_at_par(
            &self,
            _: &DataAtNameByBlockHashRequest,
        ) -> Result<RhoDataResponse, BlockApiException> {
            unimplemented!()
        }

        async fn last_finalized_block(&self) -> Result<BlockInfo, BlockApiException> {
            unimplemented!()
        }

        async fn get_block(&self, _: &str) -> Result<BlockInfo, BlockApiException> {
            unimplemented!()
        }

        async fn get_blocks(&self, _: i32) -> Result<Vec<LightBlockInfo>, BlockApiException> {
            unimplemented!()
        }

        async fn find_deploy(&self, _: &str) -> Result<LightBlockInfo, BlockApiException> {
            unimplemented!()
        }

        async fn exploratory_deploy(
            &self,
            _: &str,
            _: Option<&str>,
            _: bool,
        ) -> Result<RhoDataResponse, BlockApiException> {
            unimplemented!()
        }

        async fn get_blocks_by_heights(
            &self,
            _: i64,
            _: i64,
        ) -> Result<Vec<LightBlockInfo>, BlockApiException> {
            unimplemented!()
        }

        async fn is_finalized(&self, _: &str) -> Result<bool, BlockApiException> {
            unimplemented!()
        }

        async fn get_transaction(&self, _: &str) -> Result<TransactionResponse, BlockApiException> {
            unimplemented!()
        }
    }

    fn state() -> HttpState {
        HttpState {
            reporter: Arc::new(NewPrometheusReporter::new(Configuration::default())),
            web_api: Arc::new(MockWebApi {
                status: test_status(),
                pooled_deploys: PooledDeploys {
                    deploys: Vec::new(),
                },
            }),
            block_report_api: test_block_report_api(),
            // The HTTP tests drive the routes, not the shard list; one member stands in.
            shards: Arc::new(ShardRegistry {
                primary: rchain_shared::refined::ShardId::try_from("/root".to_string()).unwrap(),
                members: Vec::new(),
            }),
            // No gateway in these tests: the transaction routes answer 404.
            gateway: None,
            enable_txn_api: false,
            status_provider: None,
            enable_reporting: true,
            deploy_rate_limiter: Arc::new(RateLimiter::new(DEFAULT_API_RATE_LIMIT_PER_SEC)),
            faucet_rate_limiter: Arc::new(RateLimiter::new(FAUCET_RATE_LIMIT_PER_SEC)),
        }
    }

    #[tokio::test]
    async fn version_returns_node_version() {
        assert!(version().await.starts_with("RChain Node "));
    }

    #[tokio::test]
    async fn metrics_returns_scrape_data() {
        let out = metrics(State(state())).await;
        assert_eq!(
            out,
            "# The kamon-prometheus module didn't receive any data just yet.\n"
        );
    }

    #[tokio::test]
    async fn api_status_returns_json() {
        let response = api_status(State(state())).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["address"], "addr");
        assert_eq!(json["version"]["api"], "1.0");
    }

    #[tokio::test]
    async fn api_deploys_returns_wrapped_pool() {
        let mut s = state();
        s.web_api = Arc::new(MockWebApi {
            status: test_status(),
            pooled_deploys: PooledDeploys {
                deploys: vec![PooledDeploy {
                    deploy_id: "deadbeef".to_string(),
                    timestamp: 1724500000000,
                    deployer: "00".to_string(),
                    term: "Nil".to_string(),
                    phlo_price: 1,
                    phlo_limit: 100,
                    valid_after_block_number: -1,
                }],
            },
        });
        let response = api_deploys(State(s)).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["deploys"][0]["deployId"], "deadbeef");
        assert_eq!(json["deploys"][0]["timestamp"], 1724500000000i64);
    }

    #[tokio::test]
    async fn api_deploys_empty_pool_returns_empty_array() {
        let response = api_deploys(State(state())).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["deploys"], serde_json::json!([]));
    }

    #[test]
    fn openapi_json_is_valid() {
        let doc: serde_json::Value =
            serde_json::from_str(OPENAPI_JSON).expect("OPENAPI_JSON parses");
        assert_eq!(doc["openapi"], "3.0.0");
        assert!(doc["paths"].is_object());
        assert!(doc["components"]["schemas"].is_object());
    }

    fn status_provider() -> StatusProvider {
        let local = PeerNode::from(
            NodeIdentifier::new(vec![1]),
            "localhost".to_string(),
            rchain_shared::refined::Port::new(40400),
            rchain_shared::refined::Port::new(40404),
        );
        StatusProvider {
            connections: Arc::new(tokio::sync::RwLock::new(vec![local])),
            rp_conf: RPConf {
                local: PeerNode::from(
                    NodeIdentifier::new(vec![2]),
                    "localhost".to_string(),
                    rchain_shared::refined::Port::new(40400),
                    rchain_shared::refined::Port::new(40404),
                ),
                network_id: "testnet".to_string(),
                bootstrap: None,
                default_timeout: std::time::Duration::from_secs(10),
                max_num_of_connections: 100,
                clear_connections: ClearConnectionsConf {
                    num_of_connections_pinged: 10,
                },
            },
            discovery: Arc::new(NoopDiscovery),
        }
    }

    #[tokio::test]
    async fn status_returns_comm_state() {
        let mut s = state();
        s.status_provider = Some(status_provider());
        let response = status(State(s)).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["peers"], 1);
        assert_eq!(json["nodes"], 0);
    }

    /// The deploy route is rate-limited, and an exhausted limiter is a 429 — not a silent drop and
    /// not an unbounded accept. (A limiter configured to zero is closed, so this needs no sleep.)
    #[tokio::test]
    async fn api_deploy_returns_429_when_the_limiter_is_exhausted() {
        let mut s = state();
        s.deploy_rate_limiter = Arc::new(RateLimiter::new(0));
        let request: DeployRequest = serde_json::from_value(serde_json::json!({
            "data": {
                "term": "Nil",
                "timestamp": 0,
                "phloPrice": 1,
                "phloLimit": 1,
                "validAfterBlockNumber": 0,
                "shardId": "/root"
            },
            "deployer": "",
            "signature": "",
            "sigAlgorithm": "secp256k1"
        }))
        .expect("deploy request");
        let response = api_deploy(State(s), Json(request)).await;
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    // --- the shard list and the cross-shard transaction routes (Laws 26–29) ---

    /// A `BlockApi` for the transaction-route tests: only the three methods the gateway's phase
    /// path touches are real, because these tests drive *validation* and the gate, not a live 2PC.
    struct TxnStubApi {
        shard_id: String,
    }

    #[async_trait]
    impl BlockApi for TxnStubApi {
        async fn deploy(
            &self,
            _: &rchain_models::casper::protocol::casper_message::SignedDeployData,
        ) -> Result<String, String> {
            Err("no phase deploys in these tests".to_string())
        }
        async fn get_latest_message(
            &self,
        ) -> Result<rchain_models::block_metadata::BlockMetadata, String> {
            Err("no head in these tests".to_string())
        }
        async fn get_listening_name_data_response(
            &self,
            depth: i32,
            _: &rchain_models::ast::Par,
        ) -> Result<
            (
                Vec<rchain_models::casper::protocol::deploy_service::DataWithBlockInfo>,
                i32,
            ),
            String,
        > {
            Ok((Vec::new(), depth))
        }
        async fn status(&self) -> rchain_models::casper::protocol::deploy_service::Status {
            rchain_models::casper::protocol::deploy_service::Status {
                version: rchain_models::casper::protocol::deploy_service::VersionInfo {
                    api: "1".to_string(),
                    node: "test".to_string(),
                },
                address: self.shard_id.clone(),
                network_id: "testnet".to_string(),
                shard_id: self.shard_id.clone(),
                peers: 0,
                nodes: 0,
                min_phlo_price: 1,
                latest_block_number: if self.shard_id == "/root" { 7 } else { 3 },
            }
        }
        async fn deploy_status(
            &self,
            _: &rchain_block_storage::dag::dag_storage::DeployId,
        ) -> Result<rchain_models::casper::protocol::deploy_service::DeployExecStatus, String>
        {
            Err("unused".to_string())
        }
        async fn pooled_deploys(
            &self,
        ) -> Result<Vec<rchain_models::casper::protocol::casper_message::SignedDeployData>, String>
        {
            Err("unused".to_string())
        }
        async fn capabilities(&self) -> rchain_casper::api::block_api::Capabilities {
            unreachable!("unused in these tests")
        }
        async fn create_block(&self, _: bool) -> Result<String, String> {
            Err("unused".to_string())
        }
        async fn get_propose_result(&self) -> Result<String, String> {
            Err("unused".to_string())
        }
        async fn get_listening_name_continuation_response(
            &self,
            _: i32,
            _: &[rchain_models::ast::Par],
        ) -> Result<
            (
                Vec<rchain_models::casper::protocol::deploy_service::ContinuationsWithBlockInfo>,
                i32,
            ),
            String,
        > {
            Err("unused".to_string())
        }
        async fn get_blocks_by_heights(
            &self,
            _: i64,
            _: i64,
        ) -> Result<Vec<LightBlockInfo>, String> {
            Err("unused".to_string())
        }
        async fn visualize_dag(&self, _: i32, _: i32, _: bool) -> Result<Vec<String>, String> {
            Err("unused".to_string())
        }
        async fn machine_verifiable_dag(&self, _: i32) -> Result<String, String> {
            Err("unused".to_string())
        }
        async fn get_blocks(&self, _: i32) -> Result<Vec<LightBlockInfo>, String> {
            Err("unused".to_string())
        }
        async fn find_deploy(
            &self,
            _: &rchain_block_storage::dag::dag_storage::DeployId,
        ) -> Result<LightBlockInfo, String> {
            Err("unused".to_string())
        }
        async fn get_block(&self, _: &str) -> Result<BlockInfo, String> {
            Err("unused".to_string())
        }
        async fn bond_status(&self, _: &[u8]) -> Result<bool, String> {
            Err("unused".to_string())
        }
        async fn exploratory_deploy(
            &self,
            _: &str,
            _: Option<&str>,
            _: bool,
        ) -> Result<(Vec<rchain_models::ast::Par>, LightBlockInfo), String> {
            Err("unused".to_string())
        }
        async fn get_data_at_par(
            &self,
            _: &rchain_models::ast::Par,
            _: &str,
            _: bool,
        ) -> Result<(Vec<rchain_models::ast::Par>, LightBlockInfo), String> {
            Err("unused".to_string())
        }
        async fn last_finalized_block(&self) -> Result<BlockInfo, String> {
            Err("unused".to_string())
        }
        async fn is_finalized(&self, _: &str) -> Result<bool, String> {
            Err("unused".to_string())
        }
    }

    /// A [`KeyValueStoreManager`] whose stores reject every read — how `api_txn_status`'s 500 arm is
    /// reached (a ledger that opens and then cannot be read).
    struct UnreadableStoreManager;

    struct UnreadableStore;

    impl rchain_shared::store::KeyValueStore for UnreadableStore {
        fn get(&self, _: &[Vec<u8>]) -> Result<Vec<Option<Vec<u8>>>, String> {
            Err("store read failed".to_string())
        }
        fn put(&mut self, _: Vec<(Vec<u8>, Vec<u8>)>) -> Result<(), String> {
            Err("store write failed".to_string())
        }
        fn delete(&mut self, _: &[Vec<u8>]) -> Result<usize, String> {
            Err("store delete failed".to_string())
        }
        fn entries(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>, String> {
            Err("store scan failed".to_string())
        }
    }

    #[async_trait]
    impl rchain_shared::store_manager::KeyValueStoreManager for UnreadableStoreManager {
        async fn store(&self, _: &str) -> Result<SharedStore, String> {
            let store: Box<dyn rchain_shared::store::KeyValueStore + Send + Sync> =
                Box::new(UnreadableStore);
            Ok(Arc::new(tokio::sync::Mutex::new(store)))
        }
        async fn shutdown(&self) {}
    }

    fn txn_shards() -> Vec<(rchain_shared::refined::ShardId, Arc<dyn BlockApi>)> {
        let shard = |id: &str| rchain_shared::refined::ShardId::try_from(id.to_string()).unwrap();
        vec![
            (
                shard("/root"),
                Arc::new(TxnStubApi {
                    shard_id: "/root".to_string(),
                }) as Arc<dyn BlockApi>,
            ),
            (
                shard("/root/child"),
                Arc::new(TxnStubApi {
                    shard_id: "/root/child".to_string(),
                }) as Arc<dyn BlockApi>,
            ),
        ]
    }

    /// A gateway over the stub shards, with a real ledger.
    async fn gateway() -> Arc<GatewayTxn> {
        gateway_over(Arc::new(
            rchain_shared::store_manager::InMemoryStoreManager::default(),
        ))
        .await
    }

    async fn gateway_over(
        manager: Arc<dyn rchain_shared::store_manager::KeyValueStoreManager>,
    ) -> Arc<GatewayTxn> {
        let (key, pub_key) = rchain_casper::construct_deploy::default_key_pair().unwrap();
        let mut shards = std::collections::BTreeMap::new();
        for (shard_id, api) in txn_shards() {
            shards.insert(
                shard_id.clone(),
                rchain_casper::gateway::LocalShard {
                    shard_id,
                    block_api: api,
                    max_listen_depth: 50,
                },
            );
        }
        Arc::new(rchain_casper::gateway::GatewayTxn::new(
            Arc::new(rchain_casper::gateway::LocalShardDeployService::new(shards)),
            Arc::new(
                rchain_casper::gateway::ledger::TxnLedger::open(manager.as_ref())
                    .await
                    .expect("ledger"),
            ),
            key,
            pub_key,
            Duration::from_millis(50),
        ))
    }

    /// State with a shard list and an optional gateway.
    fn txn_state(gateway: Option<Arc<GatewayTxn>>, enable_txn_api: bool) -> HttpState {
        let mut s = state();
        let shard = |id: &str| rchain_shared::refined::ShardId::try_from(id.to_string()).unwrap();
        s.shards = Arc::new(ShardRegistry {
            primary: shard("/root"),
            members: txn_shards(),
        });
        s.gateway = gateway;
        s.enable_txn_api = enable_txn_api;
        s
    }

    fn txn_body(txn_id: &str, legs: serde_json::Value) -> crate::api::dto::TxnRequest {
        serde_json::from_value(serde_json::json!({ "txnId": txn_id, "legs": legs })).unwrap()
    }

    #[tokio::test]
    async fn api_shards_lists_every_member_primary_first() {
        let response = api_shards(State(txn_state(None, false))).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["primaryShard"], "/root");
        assert_eq!(json["shardCount"], 2);
        assert_eq!(json["shards"][0]["shardId"], "/root");
        assert_eq!(json["shards"][0]["primary"], true);
        assert_eq!(json["shards"][0]["latestBlockNumber"], 7);
        assert_eq!(json["shards"][1]["shardId"], "/root/child");
        assert_eq!(json["shards"][1]["primary"], false);
        assert_eq!(json["shards"][1]["latestBlockNumber"], 3);
    }

    #[tokio::test]
    async fn api_shards_with_no_members_is_an_empty_list() {
        let mut s = txn_state(None, false);
        let primary = s.shards.primary.clone();
        s.shards = Arc::new(ShardRegistry {
            primary,
            members: Vec::new(),
        });
        let response = api_shards(State(s)).await;
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["shardCount"], 0);
        assert_eq!(json["shards"].as_array().unwrap().len(), 0);
    }

    /// The gate: without a gateway, or with the feature switched off, the transaction routes answer
    /// 404 — the same convention the reporting routes use. The `(Some, false)` case is the one no
    /// test reached before, and it is what stops a gateway node from serving the routes before an
    /// operator enables them.
    #[tokio::test]
    async fn txn_routes_404_when_the_gateway_is_off_or_disabled() {
        for (gateway, enabled) in [(None, false), (Some(gateway().await), false), (None, true)] {
            let state = || txn_state(gateway.clone(), enabled);
            assert_eq!(
                api_txn_list(State(state())).await.status(),
                StatusCode::NOT_FOUND
            );
            assert_eq!(
                api_txn_status(State(state()), Path("aabb".to_string()))
                    .await
                    .status(),
                StatusCode::NOT_FOUND
            );
            assert_eq!(
                api_txn_run(
                    State(state()),
                    Json(txn_body("/root", serde_json::json!([])))
                )
                .await
                .status(),
                StatusCode::NOT_FOUND
            );
        }
    }

    #[tokio::test]
    async fn api_txn_run_rejects_a_malformed_transaction_id() {
        for bad in ["", "zz", "abc", &"aa".repeat(65)] {
            let response = api_txn_run(
                State(txn_state(Some(gateway().await), true)),
                Json(txn_body(bad, serde_json::json!([]))),
            )
            .await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "txnId {bad:?}");
        }
    }

    #[tokio::test]
    async fn api_txn_run_rejects_an_invalid_leg_shard_or_an_empty_leg_list() {
        let response = api_txn_run(
            State(txn_state(Some(gateway().await), true)),
            Json(txn_body(
                "aabb",
                serde_json::json!([{ "shardId": "", "amount": 1, "to": "d" }]),
            )),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("invalid shardId"));

        let response = api_txn_run(
            State(txn_state(Some(gateway().await), true)),
            Json(txn_body("aabb", serde_json::json!([]))),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let err = String::from_utf8_lossy(&body);
        assert!(err.contains("at least one leg"), "{err}");
    }

    /// A leg on a shard this node does not validate is a caller error, and the membership is
    /// reported back.
    #[tokio::test]
    async fn api_txn_run_reports_a_non_member_shard() {
        let response = api_txn_run(
            State(txn_state(Some(gateway().await), true)),
            Json(txn_body(
                "aabb",
                serde_json::json!([{ "shardId": "/elsewhere", "amount": 1, "to": "d" }]),
            )),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let err = String::from_utf8_lossy(&body);
        assert!(err.contains("/elsewhere"), "{err}");
        assert!(err.contains("/root, /root/child"), "{err}");
    }

    #[tokio::test]
    async fn api_txn_status_400_non_hex_and_404_unknown_and_500_unreadable() {
        let g = gateway().await;
        assert_eq!(
            api_txn_status(
                State(txn_state(Some(g.clone()), true)),
                Path("zz".to_string())
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        // A well-formed id this node has never seen.
        assert_eq!(
            api_txn_status(State(txn_state(Some(g), true)), Path("aabb".to_string()))
                .await
                .status(),
            StatusCode::NOT_FOUND
        );

        // A ledger that cannot be read is a server error, not a 404.
        let unreadable = gateway_over(Arc::new(UnreadableStoreManager)).await;
        assert_eq!(
            api_txn_status(
                State(txn_state(Some(unreadable), true)),
                Path("aabb".to_string())
            )
            .await
            .status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[tokio::test]
    async fn api_txn_list_is_empty_for_a_fresh_gateway() {
        let response = api_txn_list(State(txn_state(Some(gateway().await), true))).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["inFlight"].as_array().unwrap().len(), 0);
    }
}
