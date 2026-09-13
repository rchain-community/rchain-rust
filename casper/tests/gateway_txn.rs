//! The on-node 2PC gateway (Laws 26–29): `GatewayTxn` driving a cross-shard transaction across two
//! shards of the *same* node, with no off-chain client and no `DeployService` test double standing
//! in for the transport.
//!
//! The shards here are served by a test `BlockApi` that executes a pooled deploy on that shard's
//! runtime (the same evaluation the `InProcDeployService` double used), so these tests pin the
//! **coordination** semantics: uniformity, the durable record, restart recovery, and idempotency
//! under a repeated `txn_id`. The block-pipeline side of the legs — a real pooled deploy included in
//! a block, and play/replay agreement — is covered by `cross_shard_txn.rs` and the node-level
//! gateway test.

mod common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;

use rchain_block_storage::dag::dag_storage::DeployId;
use rchain_casper::api::block_api::{ApiErr, BlockApi, Capabilities};
use rchain_casper::construct_deploy;
use rchain_casper::gateway::ledger::{CoordState, TxnLedger, Vote};
use rchain_casper::gateway::{GatewayLeg, GatewayTxn, LocalShard, LocalShardDeployService};
use rchain_models::ast::Par;
use rchain_models::block_metadata::BlockMetadata;
use rchain_models::casper::protocol::casper_message::{DeployData, SignedDeployData};
use rchain_models::casper::protocol::deploy_service::{
    BlockInfo, ContinuationsWithBlockInfo, DataWithBlockInfo, DeployExecStatus, LightBlockInfo,
    Status,
};
use rchain_models::normalizer_env::NormalizerEnv;
use rchain_models::sorted::SortedProc;
use rchain_rholang::native_state::NativeSystemState;
use rchain_rholang::runtime::RhoRuntime;
use rchain_rholang::util::rev_address::RevAddress;
use rchain_shared::refined::{NonNegI64, ShardId};
use rchain_shared::store_manager::InMemoryStoreManager;

use common::build_runtime_manager;

/// A `BlockApi` for one in-process shard: "deploying" evaluates the term on that shard's runtime,
/// and a reply is read back from the runtime's own channel — enough to exercise the coordinator
/// without standing up the whole node.
struct TestShardApi {
    shard_id: String,
    runtime: Arc<rchain_casper::runtime_manager::RuntimeManager>,
    /// The height this shard reports (a leg is anchored at it).
    height: Mutex<i64>,
}

impl TestShardApi {
    fn new(shard_id: &str, runtime: Arc<rchain_casper::runtime_manager::RuntimeManager>) -> Self {
        TestShardApi {
            shard_id: shard_id.to_string(),
            runtime,
            height: Mutex::new(0),
        }
    }
}

#[async_trait]
impl BlockApi for TestShardApi {
    async fn status(&self) -> Status {
        unreachable!("the gateway does not call status")
    }

    async fn deploy(&self, d: &SignedDeployData) -> ApiErr<String> {
        let env = NormalizerEnv::new(d);
        let rand = rchain_crypto::hash::blake2b512_random::Blake2b512Random::from_init(&[0u8; 32]);
        let result = self
            .runtime
            .runtime()
            .evaluate_with_env(&d.data.term, env.to_env(), &rand)
            .await
            .map_err(|e| e.to_string())?;
        if !result.errors.is_empty() {
            return Err(result
                .errors
                .iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("; "));
        }
        // A deploy that arrived means this shard produced a block for it.
        if let Ok(mut height) = self.height.lock() {
            *height += 1;
        }
        Ok(rchain_shared::base16::encode(&d.sig))
    }

    async fn get_listening_name_data_response(
        &self,
        _depth: i32,
        listening_name: &Par,
    ) -> ApiErr<(Vec<DataWithBlockInfo>, i32)> {
        let data = self
            .runtime
            .runtime()
            .get_data_par(&SortedProc::new(listening_name.clone()))
            .await
            .map_err(|e| e.to_string())?;
        Ok((
            vec![DataWithBlockInfo {
                post_block_data: data,
                block: light_block(&self.shard_id),
            }],
            0,
        ))
    }

    async fn get_latest_message(&self) -> ApiErr<BlockMetadata> {
        let height = self.height.lock().map(|h| *h).unwrap_or(0);
        Ok(BlockMetadata {
            block_hash: rchain_models::block_hash::BlockHash::new([0u8; 32]),
            block_num: rchain_shared::refined::BlockHeight::try_from(height)
                .map_err(|e| e.to_string())?,
            sender: rchain_models::validator::Validator::from_slice(&[0u8; 65]),
            seq_num: rchain_shared::refined::SeqNum::zero(),
            justifications: std::collections::BTreeSet::new(),
            bonds_map: std::collections::BTreeMap::new(),
            validated: true,
            validation_failed: false,
            member_of_fringe: None,
            fringe: std::collections::BTreeSet::new(),
            fringe_state_hash: rchain_crypto::hash::blake2b256_hash::Blake2b256Hash::from_bytes(
                [0u8; 32],
            )
            .into(),
        })
    }

    async fn deploy_status(&self, _: &DeployId) -> ApiErr<DeployExecStatus> {
        Err("not used by the gateway".to_string())
    }
    async fn pooled_deploys(&self) -> ApiErr<Vec<SignedDeployData>> {
        Err("not used by the gateway".to_string())
    }
    async fn capabilities(&self) -> Capabilities {
        unreachable!("the gateway does not call capabilities")
    }
    async fn create_block(&self, _: bool) -> ApiErr<String> {
        Err("not used by the gateway".to_string())
    }
    async fn get_propose_result(&self) -> ApiErr<String> {
        Err("not used by the gateway".to_string())
    }
    async fn get_listening_name_continuation_response(
        &self,
        _: i32,
        _: &[Par],
    ) -> ApiErr<(Vec<ContinuationsWithBlockInfo>, i32)> {
        Err("not used by the gateway".to_string())
    }
    async fn get_blocks_by_heights(&self, _: i64, _: i64) -> ApiErr<Vec<LightBlockInfo>> {
        Err("not used by the gateway".to_string())
    }
    async fn visualize_dag(&self, _: i32, _: i32, _: bool) -> ApiErr<Vec<String>> {
        Err("not used by the gateway".to_string())
    }
    async fn machine_verifiable_dag(&self, _: i32) -> ApiErr<String> {
        Err("not used by the gateway".to_string())
    }
    async fn get_blocks(&self, _: i32) -> ApiErr<Vec<LightBlockInfo>> {
        Err("not used by the gateway".to_string())
    }
    async fn find_deploy(&self, _: &DeployId) -> ApiErr<LightBlockInfo> {
        Err("not used by the gateway".to_string())
    }
    async fn get_block(&self, _: &str) -> ApiErr<BlockInfo> {
        Err("not used by the gateway".to_string())
    }
    async fn bond_status(&self, _: &[u8]) -> ApiErr<bool> {
        Err("not used by the gateway".to_string())
    }
    async fn exploratory_deploy(
        &self,
        _: &str,
        _: Option<&str>,
        _: bool,
    ) -> ApiErr<(Vec<Par>, LightBlockInfo)> {
        Err("not used by the gateway".to_string())
    }
    async fn get_data_at_par(
        &self,
        _: &Par,
        _: &str,
        _: bool,
    ) -> ApiErr<(Vec<Par>, LightBlockInfo)> {
        Err("not used by the gateway".to_string())
    }
    async fn last_finalized_block(&self) -> ApiErr<BlockInfo> {
        Err("not used by the gateway".to_string())
    }
    async fn is_finalized(&self, _: &str) -> ApiErr<bool> {
        Err("not used by the gateway".to_string())
    }
}

fn light_block(shard_id: &str) -> LightBlockInfo {
    LightBlockInfo {
        version: 1,
        shard_id: shard_id.to_string(),
        block_hash: String::new(),
        block_number: 0,
        sender: String::new(),
        seq_num: 0,
        pre_state_hash: String::new(),
        post_state_hash: String::new(),
        justifications: Vec::new(),
        bonds: Vec::new(),
        sig_algorithm: String::new(),
        sig: String::new(),
        block_size: "0".to_string(),
        deploy_count: 0,
        rejected_deploys: Vec::new(),
        timestamp: 0,
    }
}

fn shard(id: &str) -> ShardId {
    ShardId::try_from(id.to_string()).unwrap()
}

/// Fund an address's vault on a shard.
fn fund(runtime: &RhoRuntime, address: &str, amount: i64) {
    NativeSystemState::new(runtime.native_store())
        .set_vault_balance(address, NonNegI64::try_from(amount).expect("non-negative"));
}

/// The two-shard fixture: shard A and B, each with a runtime and its own `BlockApi`.
struct Fixture {
    gateway: GatewayTxn,
    coordinator_addr: String,
    destination: String,
    shard_a: Arc<TestShardApi>,
    ledger: Arc<TxnLedger>,
    pub_key: rchain_crypto::public_key::PublicKey,
    /// The shards' runtimes, for asserting the escrow where it actually lives.
    rm_a: Arc<rchain_casper::runtime_manager::RuntimeManager>,
    rm_b: Arc<rchain_casper::runtime_manager::RuntimeManager>,
}

/// Build the fixture over `manager` so a test can rebuild it (a "restart") against the same ledger.
async fn fixture(manager: Arc<InMemoryStoreManager>) -> Fixture {
    let (key, pub_key) = construct_deploy::default_key_pair().unwrap();
    let coordinator_addr = RevAddress::from_public_key(&pub_key).unwrap().to_base58();
    let destination = "destRevAddress".to_string();

    let rm_a = Arc::new(build_runtime_manager().await);
    let rm_b = Arc::new(build_runtime_manager().await);
    fund(rm_a.runtime(), &coordinator_addr, 100);
    fund(rm_b.runtime(), &coordinator_addr, 100);

    let api_a = Arc::new(TestShardApi::new("/root", Arc::clone(&rm_a)));
    let api_b = Arc::new(TestShardApi::new("/root/child", Arc::clone(&rm_b)));

    let ledger = Arc::new(TxnLedger::open(manager.as_ref()).await.expect("ledger"));
    let mut shards = BTreeMap::new();
    shards.insert(
        shard("/root"),
        LocalShard {
            shard_id: shard("/root"),
            block_api: api_a.clone(),
            max_listen_depth: 50,
        },
    );
    shards.insert(
        shard("/root/child"),
        LocalShard {
            shard_id: shard("/root/child"),
            block_api: api_b.clone(),
            max_listen_depth: 50,
        },
    );
    let local = Arc::new(LocalShardDeployService::new(shards));
    let gateway = GatewayTxn::new(
        local,
        ledger.clone(),
        key.clone(),
        pub_key.clone(),
        Duration::from_secs(5),
    );
    Fixture {
        gateway,
        coordinator_addr,
        destination,
        shard_a: api_a,
        ledger,
        pub_key,
        rm_a,
        rm_b,
    }
}

fn legs(destination: &str) -> Vec<GatewayLeg> {
    vec![
        GatewayLeg {
            shard_id: shard("/root"),
            amount: 30,
            to: destination.to_string(),
        },
        GatewayLeg {
            shard_id: shard("/root/child"),
            amount: 40,
            to: destination.to_string(),
        },
    ]
}

/// A two-leg transaction against the node's own two shards commits on both, and the durable record
/// says so (Law 29's biconditional: committed exactly when every vote is ready).
#[tokio::test]
async fn gateway_commits_a_two_shard_transaction_and_records_the_decision() {
    let fx = fixture(Arc::new(InMemoryStoreManager::default())).await;

    let record = fx
        .gateway
        .run(b"gw-commit", &legs(&fx.destination))
        .await
        .expect("run");

    assert_eq!(record.state, CoordState::Committed);
    assert_eq!(record.legs.len(), 2);
    assert_eq!(record.votes.len(), 2);
    assert!(record.votes.iter().all(|(_, v)| *v == Vote::Ready));
    assert!(record.decision_is_deterministic());
    // The record is durable and reported by the ledger.
    let stored = fx.ledger.get(b"gw-commit").await.unwrap().expect("stored");
    assert_eq!(stored, record);
    assert!(fx.gateway.list().await.unwrap().is_empty());
}

/// The escrow actually moved on both shards, in the shards' own state.
#[tokio::test]
async fn gateway_moves_the_escrow_on_every_leg() {
    let fx = fixture(Arc::new(InMemoryStoreManager::default())).await;

    fx.gateway
        .run(b"gw-balances", &legs(&fx.destination))
        .await
        .expect("run");

    let native_a = NativeSystemState::new(fx.rm_a.runtime().native_store());
    let native_b = NativeSystemState::new(fx.rm_b.runtime().native_store());
    // Each shard escrowed its own leg out of the coordinator's vault and credited the destination.
    assert_eq!(
        i64::from(
            native_a
                .vault_balance(&fx.coordinator_addr)
                .await
                .unwrap()
                .unwrap()
        ),
        70
    );
    assert_eq!(
        i64::from(
            native_b
                .vault_balance(&fx.coordinator_addr)
                .await
                .unwrap()
                .unwrap()
        ),
        60
    );
    assert_eq!(
        i64::from(
            native_a
                .vault_balance(&fx.destination)
                .await
                .unwrap()
                .unwrap()
        ),
        30
    );
    assert_eq!(
        i64::from(
            native_b
                .vault_balance(&fx.destination)
                .await
                .unwrap()
                .unwrap()
        ),
        40
    );
}

/// One leg cannot prepare (its escrow is short) ⇒ the whole transaction aborts and the prepared leg
/// is compensated: Law 27's uniformity.
#[tokio::test]
async fn gateway_aborts_every_leg_when_one_cannot_prepare() {
    let manager = Arc::new(InMemoryStoreManager::default());
    let (key, pub_key) = construct_deploy::default_key_pair().unwrap();
    let coordinator_addr = RevAddress::from_public_key(&pub_key).unwrap().to_base58();
    let destination = "destRevAddress".to_string();

    let rm_a = Arc::new(build_runtime_manager().await);
    let rm_b = Arc::new(build_runtime_manager().await);
    fund(rm_a.runtime(), &coordinator_addr, 100);
    // Shard B holds only 10 REV, so its 40 REV prepare fails.
    fund(rm_b.runtime(), &coordinator_addr, 10);

    let api_a = Arc::new(TestShardApi::new("/root", Arc::clone(&rm_a)));
    let api_b = Arc::new(TestShardApi::new("/root/child", Arc::clone(&rm_b)));
    let mut shards = BTreeMap::new();
    shards.insert(
        shard("/root"),
        LocalShard {
            shard_id: shard("/root"),
            block_api: api_a.clone(),
            max_listen_depth: 50,
        },
    );
    shards.insert(
        shard("/root/child"),
        LocalShard {
            shard_id: shard("/root/child"),
            block_api: api_b.clone(),
            max_listen_depth: 50,
        },
    );
    let ledger = Arc::new(TxnLedger::open(manager.as_ref()).await.expect("ledger"));
    let gateway = GatewayTxn::new(
        Arc::new(LocalShardDeployService::new(shards)),
        ledger.clone(),
        key,
        pub_key,
        Duration::from_secs(5),
    );

    let record = gateway
        .run(b"gw-abort", &legs(&destination))
        .await
        .expect("run");
    assert_eq!(record.state, CoordState::Aborted);
    assert!(
        record.reason.is_some(),
        "an abort must record why: {:?}",
        record
    );
    assert!(record.decision_is_deterministic());

    // The prepared leg was compensated: shard A is back to its opening balance, and nothing reached
    // the destination anywhere.
    let native_a = NativeSystemState::new(rm_a.runtime().native_store());
    let native_b = NativeSystemState::new(rm_b.runtime().native_store());
    assert_eq!(
        i64::from(
            native_a
                .vault_balance(&coordinator_addr)
                .await
                .unwrap()
                .unwrap()
        ),
        100
    );
    assert_eq!(
        i64::from(
            native_b
                .vault_balance(&coordinator_addr)
                .await
                .unwrap()
                .unwrap()
        ),
        10
    );
    assert_eq!(native_a.vault_balance(&destination).await.unwrap(), None);
    assert_eq!(native_b.vault_balance(&destination).await.unwrap(), None);
}

/// A re-run of a completed transaction returns the same record and touches no participant, so a
/// retried request cannot move funds twice (Law 28 at the coordinator boundary).
#[tokio::test]
async fn gateway_is_idempotent_under_txn_id() {
    let manager = Arc::new(InMemoryStoreManager::default());
    let fx = fixture(manager).await;

    let first = fx
        .gateway
        .run(b"gw-idempotent", &legs(&fx.destination))
        .await
        .expect("first run");
    let second = fx
        .gateway
        .run(b"gw-idempotent", &legs(&fx.destination))
        .await
        .expect("second run");

    assert_eq!(first, second);
    assert_eq!(second.state, CoordState::Committed);
    // A terminal record is returned without contacting a shard, so no further block was produced.
    let height_after_first = *fx.shard_a.height.lock().unwrap();
    fx.gateway
        .run(b"gw-idempotent", &legs(&fx.destination))
        .await
        .expect("third run");
    assert_eq!(
        *fx.shard_a.height.lock().unwrap(),
        height_after_first,
        "a retry must not submit another deploy"
    );
}

/// A gateway that restarts mid-transaction finishes it from the ledger alone.
///
/// The fixture is dropped and rebuilt over the *same* ledger (an in-memory store standing in for the
/// node's `gateway-txn` database), which is what `recover_in_flight` sees on boot.
#[tokio::test]
async fn gateway_recovers_an_in_flight_transaction_from_the_ledger() {
    let manager = Arc::new(InMemoryStoreManager::default());

    // Drive a record to `Prepared` — one leg voted ready, the other not yet — then "crash".
    let fx = fixture(manager.clone()).await;
    let mut record = rchain_casper::gateway::CoordRecord {
        txn_id: b"gw-recover".to_vec(),
        state: CoordState::Proposed,
        coordinator: fx.pub_key.clone(),
        legs: vec![
            rchain_casper::gateway::LegRecord {
                shard_id: shard("/root"),
                amount: NonNegI64::try_from(30).unwrap(),
                to: fx.destination.clone(),
            },
            rchain_casper::gateway::LegRecord {
                shard_id: shard("/root/child"),
                amount: NonNegI64::try_from(40).unwrap(),
                to: fx.destination.clone(),
            },
        ],
        votes: Vec::new(),
        reason: None,
    };
    // One vote of two legs is the genuinely-in-flight state: `Prepared`, not yet decided.
    record.record_vote(shard("/root"), Vote::Ready, None);
    assert_eq!(record.state, CoordState::Prepared);
    assert!(!record.state.is_terminal());
    fx.ledger.put(&record).await.expect("put in-flight record");
    drop(fx);

    // A fresh gateway over the same ledger finds the record in flight and finishes it.
    let restarted = fixture(manager).await;
    let in_flight = restarted.ledger.in_flight().await.expect("in flight");
    assert_eq!(in_flight.len(), 1);
    assert_eq!(in_flight[0].state, CoordState::Prepared);

    let recovered = restarted
        .gateway
        .recover_in_flight()
        .await
        .expect("recover");
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].txn_id, b"gw-recover".to_vec());
    assert_eq!(recovered[0].state, CoordState::Committed);
    assert!(recovered[0].decision_is_deterministic());
    // Nothing is left in flight, and the decision is durable.
    assert!(restarted.gateway.list().await.unwrap().is_empty());
    assert_eq!(
        restarted
            .ledger
            .get(b"gw-recover")
            .await
            .unwrap()
            .expect("stored")
            .state,
        CoordState::Committed
    );
}

/// A transaction naming a shard this node is not a member of is rejected, never relayed.
#[tokio::test]
async fn gateway_rejects_a_non_member_shard() {
    let fx = fixture(Arc::new(InMemoryStoreManager::default())).await;
    let err = fx
        .gateway
        .run(
            b"gw-foreign",
            &[GatewayLeg {
                shard_id: shard("/elsewhere"),
                amount: 1,
                to: fx.destination.clone(),
            }],
        )
        .await
        .expect_err("a non-member shard must be rejected");
    assert!(err.contains("/elsewhere"), "{err}");
    assert!(err.contains("/root, /root/child"), "{err}");
}

/// A deploy addressed at the local service for a non-member shard is rejected there too — the
/// points of entry agree.
#[tokio::test]
async fn local_deploy_service_rejects_a_non_member_shard() {
    use rchain_casper::protocol::client::DeployService;

    let manager = Arc::new(InMemoryStoreManager::default());
    let fx = fixture(manager).await;
    let _ = &fx;

    let rm = Arc::new(build_runtime_manager().await);
    let api = Arc::new(TestShardApi::new("/root", Arc::clone(&rm)));
    let mut shards = BTreeMap::new();
    shards.insert(
        shard("/root"),
        LocalShard {
            shard_id: shard("/root"),
            block_api: api,
            max_listen_depth: 50,
        },
    );
    let local = LocalShardDeployService::new(shards);
    let deploy = SignedDeployData {
        data: DeployData {
            term: "Nil".to_string(),
            timestamp: 0,
            phlo_price: 1,
            phlo_limit: 1,
            valid_after_block_number: 0,
            shard_id: "/elsewhere".to_string(),
        },
        deployer: vec![0u8; 65],
        sig: Vec::new(),
        sig_algorithm: "secp256k1".to_string(),
    };
    let err = local
        .deploy(&deploy)
        .await
        .expect_err("a non-member shard must be rejected");
    assert!(err.join(" ").contains("/elsewhere"), "{err:?}");
}
