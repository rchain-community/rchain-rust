//! Web API implementation (port of `WebApi.WebApiImpl`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use rchain_casper::api::block_api::{BlockApi, DeployerInfo};
use rchain_casper::dag::deployer_index_key;
use rchain_crypto::hash::blake2b256_hash::Blake2b256Hash;
use rchain_crypto::private_key::PrivateKey;
use rchain_models::casper::protocol::casper_message::SignedDeployData;
use rchain_models::casper::protocol::deploy_service::{BlockInfo, LightBlockInfo};
use rchain_rholang::util::rev_address::RevAddress;
use rchain_shared::base16;

use super::conversion::{
    to_api_status, to_data_at_name_response, to_deploy_exec_status, to_exploratory_deploy_response,
    to_node_capabilities, to_pooled_deploy, to_rho_data_response, to_signed_deploy,
};
use super::dto::{
    ApiStatus, BlockApiException, DataAtNameByBlockHashRequest, DataAtNameRequest,
    DataAtNameResponse, DeployExecStatus, DeployRequest, ExploratoryDeployResponse, FaucetResponse,
    NodeCapabilities, PooledDeploys, RhoDataResponse,
};
use super::faucet;
use super::rho_expr::{rho_expr_to_par, unforg_to_par};
use super::web_api::WebApi;
use crate::web::transaction::{TransactionApi, TransactionResponse};

/// The web API implementation (port of `WebApi.WebApiImpl`).
pub struct WebApiImpl {
    block_api: Arc<dyn BlockApi>,
    transaction_api: Arc<dyn TransactionApi>,
    /// The dev deployer key (dev-mode only); `None` disables the faucet.
    deployer_key: Option<PrivateKey>,
    shard_id: String,
    /// Per-address faucet drip count (R17): bounds how much REV any single address can pull.
    drip_counts: Arc<Mutex<HashMap<String, u32>>>,
}

impl WebApiImpl {
    pub fn new(
        block_api: Arc<dyn BlockApi>,
        transaction_api: Arc<dyn TransactionApi>,
        deployer_key: Option<PrivateKey>,
        shard_id: String,
    ) -> Self {
        WebApiImpl {
            block_api,
            transaction_api,
            deployer_key,
            shard_id,
            drip_counts: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

fn invalid_deploy_id() -> BlockApiException {
    BlockApiException("Deploy id is not valid base16 format.".to_string())
}

/// Maximum faucet drips any single address may receive (0.3 REV each). Bounds the dev-wallet drain
/// a single caller can cause before other developers are starved.
const FAUCET_MAX_DRIPS_PER_ADDRESS: u32 = 10;

#[cfg(test)]
mod tests {
    use super::*;
    use rchain_casper::runtime_manager::CapturedReply;
    use std::sync::Mutex as StdMutex;

    use rchain_block_storage::dag::dag_storage::DeployId;
    use rchain_casper::api::block_api::{ApiErr, Capabilities};
    use rchain_models::ast::Par;
    use rchain_models::block_metadata::BlockMetadata;
    use rchain_models::casper::protocol::casper_message::DeployData;
    use rchain_models::casper::protocol::deploy_service::{
        BlockInfo, ContinuationsWithBlockInfo, DataWithBlockInfo,
        DeployExecStatus as DomainExecStatus, LightBlockInfo, Status, VersionInfo,
    };

    /// A block API that answers from its fields and records what it was asked — the `StubBlockApi`
    /// pattern `deploy_grpc_service_v1.rs` and `tonic.rs`'s tests use. The methods this file never
    /// calls are `unreachable!`: a stub that returns a plausible value there would hide a call the
    /// implementation is not supposed to make.
    struct StubBlockApi {
        caps: Capabilities,
        pooled: Vec<SignedDeployData>,
        deploy_status: ApiErr<DomainExecStatus>,
        /// What `deploy` was asked to pool, shared by handle so a test can read it after the stub
        /// has been boxed behind `Arc<dyn BlockApi>`.
        deployed: Arc<StdMutex<Vec<SignedDeployData>>>,
    }

    impl Default for StubBlockApi {
        fn default() -> Self {
            StubBlockApi {
                caps: Capabilities {
                    autopropose: false,
                    propose_on_deploy: false,
                    manual_propose: true,
                    admin_http: false,
                    dev_mode: true,
                },
                pooled: Vec::new(),
                deploy_status: Ok(DomainExecStatus::NotProcessed {
                    status: "pending".to_string(),
                }),
                deployed: Arc::new(StdMutex::new(Vec::new())),
            }
        }
    }

    fn block_status() -> Status {
        Status {
            version: VersionInfo {
                api: "1".to_string(),
                node: "2".to_string(),
            },
            address: "addr".to_string(),
            network_id: "net".to_string(),
            shard_id: "root".to_string(),
            peers: 3,
            nodes: 4,
            min_phlo_price: 5,
            latest_block_number: 42,
        }
    }

    fn deploy_with(sig: u8, timestamp: i64) -> SignedDeployData {
        SignedDeployData {
            data: DeployData {
                attachments: Vec::new(),
                term: format!("term-{sig}"),
                timestamp,
                phlo_price: 1,
                phlo_limit: 2,
                valid_after_block_number: 0,
                shard_id: "root".to_string(),
            },
            deployer: vec![sig; 65],
            sig: vec![sig],
            sig_algorithm: "secp256k1".to_string(),
        }
    }

    /// The trait's method list, copied signature-for-signature: every method this file does not
    /// call is `unreachable!`, so a call the implementation is not supposed to make fails loudly
    /// instead of being answered by a plausible stub value.
    #[async_trait]
    impl BlockApi for StubBlockApi {
        async fn status(&self) -> Status {
            block_status()
        }

        async fn deploy(&self, deploy: &SignedDeployData) -> ApiErr<String> {
            self.deployed.lock().unwrap().push(deploy.clone());
            Ok(base16::encode(&deploy.sig))
        }

        async fn deploy_status(&self, _: &DeployId) -> ApiErr<DomainExecStatus> {
            self.deploy_status.clone()
        }

        async fn pooled_deploys(&self) -> ApiErr<Vec<SignedDeployData>> {
            Ok(self.pooled.clone())
        }

        async fn capabilities(&self) -> Capabilities {
            self.caps.clone()
        }

        async fn create_block(&self, _: bool) -> ApiErr<String> {
            unreachable!("WebApiImpl does not create blocks")
        }
        async fn get_propose_result(&self) -> ApiErr<String> {
            unreachable!("WebApiImpl does not read propose results")
        }
        async fn get_listening_name_data_response(
            &self,
            _: i32,
            _: &Par,
        ) -> ApiErr<(Vec<DataWithBlockInfo>, i32)> {
            unreachable!("not exercised here")
        }
        async fn get_listening_name_continuation_response(
            &self,
            _: i32,
            _: &[Par],
        ) -> ApiErr<(Vec<ContinuationsWithBlockInfo>, i32)> {
            unreachable!("not exercised here")
        }
        async fn get_blocks_by_heights(&self, _: i64, _: i64) -> ApiErr<Vec<LightBlockInfo>> {
            unreachable!("not exercised here")
        }
        async fn visualize_dag(&self, _: i32, _: i32, _: bool) -> ApiErr<Vec<String>> {
            unreachable!("not exercised here")
        }
        async fn machine_verifiable_dag(&self, _: i32) -> ApiErr<String> {
            unreachable!("not exercised here")
        }
        async fn get_blocks(&self, _: i32) -> ApiErr<Vec<LightBlockInfo>> {
            unreachable!("not exercised here")
        }
        async fn find_deploy(&self, _: &DeployId) -> ApiErr<LightBlockInfo> {
            unreachable!("not exercised here")
        }
        /// Echoes the first byte of the hash it was asked about, so a test can tell which hash arrived.
        async fn find_deployer(&self, deployer_hash: &[u8]) -> ApiErr<DeployerInfo> {
            assert_eq!(
                deployer_hash.len(),
                32,
                "the block API is always asked by hash"
            );
            Ok(DeployerInfo {
                block: None,
                indexed_from_height: i64::from(deployer_hash[0]),
            })
        }
        async fn get_block(&self, _: &str) -> ApiErr<BlockInfo> {
            unreachable!("not exercised here")
        }
        async fn bond_status(&self, _: &[u8]) -> ApiErr<bool> {
            unreachable!("not exercised here")
        }
        async fn exploratory_deploy(
            &self,
            _: &str,
            _: Option<&str>,
            _: bool,
        ) -> ApiErr<(CapturedReply, LightBlockInfo)> {
            unreachable!("not exercised here")
        }
        async fn get_data_at_par(
            &self,
            _: &Par,
            _: &str,
            _: bool,
        ) -> ApiErr<(Vec<Par>, LightBlockInfo)> {
            unreachable!("not exercised here")
        }
        async fn last_finalized_block(&self) -> ApiErr<BlockInfo> {
            unreachable!("not exercised here")
        }
        async fn is_finalized(&self, _: &str) -> ApiErr<bool> {
            unreachable!("not exercised here")
        }
        async fn get_latest_message(&self) -> ApiErr<BlockMetadata> {
            unreachable!("not exercised here")
        }
    }

    struct StubTransactionApi;
    #[async_trait]
    impl TransactionApi for StubTransactionApi {
        async fn get_transaction(
            &self,
            _: &Blake2b256Hash,
        ) -> Result<Vec<crate::web::transaction::TransactionInfo>, String> {
            unreachable!("WebApiImpl's get_transaction is not exercised here")
        }
    }

    /// A deploy-mode key pair and a REV address derived from it — the address the faucet will
    /// accept, since a REV address is checked for its checksum and prefix, not merely its length.
    fn key_and_address() -> (PrivateKey, String) {
        let alg = rchain_crypto::signatures::signatures_alg::from_algorithm("secp256k1")
            .expect("secp256k1 is registered");
        let (sk, pk) = alg.new_key_pair();
        let address = RevAddress::from_public_key(&pk).expect("an address from the key");
        (sk, address.to_base58())
    }

    /// A REV address that is **not** the deployer's own.
    ///
    /// `key_and_address` derives both halves from one key, so its address *is* the deployer's — and
    /// every faucet test in this module was dripping to it. That is AUDIT R33 exactly: the drip is a
    /// transfer from the node's account to the node's account, which moves nothing, and nothing
    /// refused it. With the refusal in place those tests would all fail for the wrong reason, so a
    /// test that wants to exercise a real drip names somebody else.
    fn faucet_target() -> String {
        key_and_address().1
    }

    fn api(block_api: StubBlockApi, deployer_key: Option<PrivateKey>) -> WebApiImpl {
        WebApiImpl::new(
            Arc::new(block_api),
            Arc::new(StubTransactionApi),
            deployer_key,
            "root".to_string(),
        )
    }

    /// `capabilities` reports the faucet as available only when **both** dev mode and a deployer key
    /// are configured: advertising it with no key would send a wallet at a 500.
    #[tokio::test]
    async fn the_faucet_is_advertised_only_with_dev_mode_and_a_key() {
        let (sk, _) = key_and_address();

        for (dev_mode, has_key, expected) in [
            (true, true, true),
            (true, false, false),
            (false, true, false),
            (false, false, false),
        ] {
            let mut block_api = StubBlockApi::default();
            block_api.caps.dev_mode = dev_mode;
            let key = if has_key { Some(sk.clone()) } else { None };
            let caps = api(block_api, key)
                .capabilities()
                .await
                .expect("capabilities");
            assert_eq!(
                caps.faucet, expected,
                "dev_mode = {dev_mode}, key = {has_key}"
            );
        }
    }

    /// The deployer lookup takes the key's hash as is (so a wallet need not reveal an unused key) or
    /// the 65-byte key, which it hashes the way the index does; anything else is refused.
    #[tokio::test]
    async fn the_deployer_lookup_takes_a_key_hash_or_a_key() {
        let web = api(StubBlockApi::default(), None);

        let hash = [0xabu8; 32];
        let by_hash = web.find_deployer(&base16::encode(&hash)).await.unwrap();
        assert_eq!(by_hash.indexed_from_height, 0xab);

        let key = [4u8; 65];
        // The same vector r-wallet's unit tests pin for `deployer_key_hash`: the two must agree.
        assert_eq!(
            base16::encode(&deployer_index_key(&key)),
            "b0ec3ad69aacbdc6499f533d58abd768331f5977fb42d302c3cf7f8a401e75f1"
        );
        let by_key = web.find_deployer(&base16::encode(&key)).await.unwrap();
        assert_eq!(
            by_key.indexed_from_height,
            i64::from(deployer_index_key(&key)[0])
        );

        for bad in ["zz", "04", &base16::encode(&[4u8; 33])] {
            assert!(web.find_deployer(bad).await.is_err(), "{bad} was accepted");
        }
    }

    /// A non-hex deploy id is refused **by name** before the block API is consulted — the id comes
    /// from a URL path, so it is untrusted input.
    #[tokio::test]
    async fn a_non_hex_deploy_id_is_refused_by_name() {
        let err = api(StubBlockApi::default(), None)
            .deploy_status("not-hex!")
            .await
            .expect_err("not base16");
        assert_eq!(
            err,
            BlockApiException("Deploy id is not valid base16 format.".to_string())
        );

        // Odd-length hex is not base16 either.
        assert!(api(StubBlockApi::default(), None)
            .deploy_status("abc")
            .await
            .is_err());
    }

    /// A hex id reaches the block API, and the returned status is converted — the stub's status
    /// comes back as the API's own `NotProcessed` variant with its message intact.
    #[tokio::test]
    async fn a_valid_deploy_id_returns_the_converted_status() {
        let status = api(StubBlockApi::default(), None)
            .deploy_status("aabb")
            .await
            .expect("a valid id");
        match status {
            DeployExecStatus::NotProcessed { status } => assert_eq!(status, "pending"),
            other => panic!("expected NotProcessed, got {other:?}"),
        }
    }

    /// Pooled deploys are returned **most-recent-first**, by the deploy's own timestamp: the pool's
    /// key order is the signature bytes, so without the sort the API's order would look random to a
    /// client (and change between calls).
    #[tokio::test]
    async fn pooled_deploys_come_back_most_recent_first() {
        let block_api = StubBlockApi {
            pooled: vec![
                deploy_with(1, 500),
                deploy_with(2, 900),
                deploy_with(3, 100),
            ],
            ..StubBlockApi::default()
        };
        let pooled = api(block_api, None).pooled_deploys().await.expect("pooled");

        let timestamps: Vec<i64> = pooled.deploys.iter().map(|d| d.timestamp).collect();
        assert_eq!(timestamps, vec![900, 500, 100]);
        assert_eq!(pooled.deploys[0].deploy_id, base16::encode(&[2u8]));
        assert_eq!(pooled.deploys[1].term, "term-1");
    }

    /// The faucet validates the REV address **before** spending anything: an invalid address is an
    /// error naming it (a valid REV address has a checksum and a coin prefix, so a plausible-looking
    /// string is not enough).
    #[tokio::test]
    async fn the_faucet_refuses_an_invalid_address_by_name() {
        let (sk, _) = key_and_address();
        let err = api(StubBlockApi::default(), Some(sk))
            .faucet("rBdXnotARealAddress")
            .await
            .expect_err("invalid");
        assert_eq!(
            err,
            BlockApiException("Invalid REV address: rBdXnotARealAddress".to_string())
        );
    }

    /// **A drip to the deployer's own address is refused** (AUDIT R33).
    ///
    /// `build_transfer_term` signs a transfer from the deployer's account to whatever address is
    /// asked for, and the genesis faucet's deployer **is** the funded account — so asking for the
    /// deployer's own address builds a transfer that moves nothing between two accounts the node
    /// already holds. It still spent one of that address's drips and submitted a deploy that pays
    /// phlo to do it, and nothing refused it. Every test in this module was dripping to exactly that
    /// address without noticing, which is what R33 means by a no-op that is not free.
    ///
    /// Two arms, so a refusal cannot pass for a fix on a faucet that refuses everything: the
    /// self-drip is refused **by name and reason**, and a drip to anybody else still succeeds.
    #[tokio::test]
    async fn the_faucet_refuses_a_drip_to_the_deployers_own_address() {
        let (sk, own) = key_and_address();
        let web = api(StubBlockApi::default(), Some(sk));

        let err = web
            .faucet(&own)
            .await
            .expect_err("the deployer's own address is not a target");
        assert!(
            err.0.contains(&own) && err.0.contains("deployer's own address"),
            "the refusal names the address and what is wrong with it: {}",
            err.0
        );

        let target = faucet_target();
        assert!(target != own, "the two keys differ");
        web.faucet(&target)
            .await
            .expect("somebody else is a target");
    }

    /// The faucet's per-address budget (R17) is enforced: ten drips for one address, the eleventh
    /// refused by name — the fix for a single caller draining the dev wallet.
    #[tokio::test]
    async fn the_faucet_enforces_its_per_address_budget() {
        let (sk, _) = key_and_address();
        let address = faucet_target();
        let web = api(StubBlockApi::default(), Some(sk));

        for i in 0..FAUCET_MAX_DRIPS_PER_ADDRESS {
            let response = web.faucet(&address).await.expect("a drip");
            assert_eq!(response.amount, faucet::FAUCET_AMOUNT);
            assert_eq!(response.to, address);
            assert!(!response.deploy_id.is_empty(), "drip {i} has an id");
        }

        let err = web.faucet(&address).await.expect_err("the budget is spent");
        assert_eq!(
            err,
            BlockApiException(format!(
                "faucet: address {address} has reached its drip budget ({FAUCET_MAX_DRIPS_PER_ADDRESS})"
            ))
        );

        // A *different* address still has its own budget.
        let (_, other) = key_and_address();
        assert!(other != address, "the two keys differ");
        web.faucet(&other).await.expect("another address drips");
    }

    /// With no deployer key the faucet refuses, naming the flags an operator needs — and it refuses
    /// **before charging the drip**, which is a change (AUDIT R33).
    ///
    /// This test used to pin the opposite as deliberate: the budget was taken before the key was
    /// checked, so a dev node started without `--deployer-private-key` burned ten drips per address
    /// serving nothing. That is the same defect as R33 one step over — a no-op that spends the budget
    /// — and R33's fix needs the key *earlier* than the charge, so both went at once.
    #[tokio::test]
    async fn the_faucet_without_a_key_names_the_flags_it_needs() {
        let (_, address) = key_and_address();
        let err = api(StubBlockApi::default(), None)
            .faucet(&address)
            .await
            .expect_err("no key");
        assert_eq!(
            err,
            BlockApiException("faucet requires --dev-mode --deployer-private-key".to_string())
        );

        // **The budget was not touched.** Wiring a key afterwards leaves the address its full
        // allowance — the whole point of moving the check, and the arm that fails if the key is
        // resolved after the charge again.
        let (sk, _) = key_and_address();
        let web = api(StubBlockApi::default(), Some(sk));
        for _ in 0..FAUCET_MAX_DRIPS_PER_ADDRESS {
            web.faucet(&address).await.expect("a drip");
        }
        assert!(
            web.faucet(&address).await.is_err(),
            "the budget is spent — ten, not nine"
        );
    }

    /// A successful drip signs a transfer and hands it to the block API. The deploy the API
    /// received carries the **public key derived from the deployer's secret** (the node's `deployer`
    /// field is what the vault's `transfer` derives `from` from), the configured shard, and a
    /// non-empty signature — so the receipt's deploy id is the signature's hex.
    #[tokio::test]
    async fn a_drip_signs_a_transfer_and_pools_it() {
        let (sk, _) = key_and_address();
        let address = faucet_target();
        let stub = StubBlockApi::default();
        let deployed = stub.deployed.clone();
        let web = api(stub, Some(sk.clone()));

        let response = web.faucet(&address).await.expect("a drip");
        assert_eq!(response.amount, faucet::FAUCET_AMOUNT);
        assert_eq!(response.to, address);

        let recorded = deployed.lock().unwrap();
        assert_eq!(recorded.len(), 1, "the deploy reached the block API");
        assert_eq!(recorded[0].sig_algorithm, "secp256k1");
        assert!(!recorded[0].sig.is_empty(), "it is signed");
        assert_eq!(recorded[0].data.shard_id, "root", "the node's shard");
        assert_eq!(
            recorded[0].data.valid_after_block_number, 42,
            "anchored to the chain height"
        );

        let alg = rchain_crypto::signatures::signatures_alg::from_algorithm("secp256k1")
            .expect("registered");
        let expected_deployer = alg.to_public(&sk).expect("public key").bytes().to_vec();
        assert_eq!(
            recorded[0].deployer, expected_deployer,
            "the deployer is the public key of the signing key"
        );
        assert_eq!(
            response.deploy_id,
            base16::encode(&recorded[0].sig),
            "the receipt's id is the deploy's signature"
        );

        // The transfer term names the recipient, so the drip actually pays the address asked for.
        assert!(
            recorded[0].data.term.contains(&address),
            "the term is a transfer to {address}: {}",
            recorded[0].data.term
        );
    }
}

#[async_trait]
impl WebApi for WebApiImpl {
    async fn status(&self) -> Result<ApiStatus, BlockApiException> {
        let status = self.block_api.status().await;
        let caps = self.block_api.capabilities().await;
        let health = self.block_api.proposer_health().await;
        // C249's F-U10-01: the merge's and finality's observations, which reached the log and nothing
        // else — plus the poisoned-lock count, read from `rspace` because the poisoning is a
        // storage-layer event and the count is process-wide (F-U9-03).
        let finality = self.block_api.finality_health().await;
        Ok(to_api_status(
            &status,
            &caps,
            &health,
            &finality,
            rchain_rspace::lock::poison_recoveries(),
        ))
    }

    async fn deploy(&self, request: &DeployRequest) -> Result<String, BlockApiException> {
        // `Signed<DeployData>` holds a `&dyn SignaturesAlg` (not `Sync`), so keep it in a block
        // that ends before the `.await`.
        let deploy = {
            let signed = to_signed_deploy(request).map_err(|e| BlockApiException(e.0))?;
            SignedDeployData {
                data: signed.data.clone(),
                deployer: signed.pk.bytes().to_vec(),
                sig: signed.sig.clone(),
                sig_algorithm: signed.sig_algorithm.name().to_string(),
            }
        };
        self.block_api
            .deploy(&deploy)
            .await
            .map_err(BlockApiException)
    }

    async fn pooled_deploys(&self) -> Result<PooledDeploys, BlockApiException> {
        let mut pooled = self
            .block_api
            .pooled_deploys()
            .await
            .map_err(BlockApiException)?;
        // Most-recent-first: the pool's key order is the deploy signature bytes, not insertion time.
        pooled.sort_by_key(|d| std::cmp::Reverse(d.data.timestamp));
        let deploys = pooled.iter().map(to_pooled_deploy).collect();
        Ok(PooledDeploys { deploys })
    }

    async fn capabilities(&self) -> Result<NodeCapabilities, BlockApiException> {
        let caps = self.block_api.capabilities().await;
        // The faucet is gated on both dev mode and a configured deployer key.
        let faucet = caps.dev_mode && self.deployer_key.is_some();
        Ok(to_node_capabilities(&caps, faucet))
    }

    async fn deploy_status(&self, deploy_id: &str) -> Result<DeployExecStatus, BlockApiException> {
        let id = base16::decode(deploy_id).ok_or_else(invalid_deploy_id)?;
        let status = self
            .block_api
            .deploy_status(&id)
            .await
            .map_err(BlockApiException)?;
        to_deploy_exec_status(&status)
            .ok_or_else(|| BlockApiException("Deploy status protobuf message error".to_string()))
    }

    async fn faucet(&self, address: &str) -> Result<FaucetResponse, BlockApiException> {
        if !RevAddress::is_valid(address) {
            return Err(BlockApiException(format!("Invalid REV address: {address}")));
        }
        // **The key is resolved before anything is charged, and that ordering is two findings'
        // fix.** AUDIT R33: a drip to the deployer's *own* address moves nothing — `build_transfer_term`
        // signs a transfer from the deployer's account to the deployer's account — and it still spent
        // one of the address's drips and submitted a deploy that pays a phlo cost to do nothing.
        // Catching that needs the key, so the key has to come first; and once it does, a node started
        // without `--deployer-private-key` stops burning its whole drip budget on refusals, which this
        // module's own test used to pin as deliberate ("a dev node without a key burns its drips
        // without serving any"). Both are the same shape: a no-op that spends the budget.
        let sk = self.deployer_key.as_ref().ok_or_else(|| {
            BlockApiException("faucet requires --dev-mode --deployer-private-key".to_string())
        })?;
        let own = faucet::deployer_rev_address(sk).map_err(BlockApiException)?;
        if address == own {
            return Err(BlockApiException(format!(
                "faucet: {address} is the deployer's own address — the drip would move nothing between \
                 two accounts the node already holds, and would still spend the budget"
            )));
        }
        // Per-address drip budget (R17): bound how much REV one address can pull, so a single caller
        // cannot monopolize the rate limit and drain the genesis dev wallet.
        {
            let mut counts = self.drip_counts.lock().unwrap_or_else(|p| p.into_inner());
            let count = counts.entry(address.to_string()).or_insert(0);
            if *count >= FAUCET_MAX_DRIPS_PER_ADDRESS {
                return Err(BlockApiException(format!(
                    "faucet: address {address} has reached its drip budget ({FAUCET_MAX_DRIPS_PER_ADDRESS})"
                )));
            }
            *count += 1;
        }
        // Valid-from-now: a deploy with `valid_after_block_number = -1` is treated as expired once
        // the node is past `DEPLOY_LIFESPAN` (50) blocks, so anchor it to the current height.
        let vabn = self.block_api.status().await.latest_block_number;
        let signed =
            faucet::sign_faucet_deploy(sk, address, faucet::FAUCET_AMOUNT, &self.shard_id, vabn)
                .map_err(BlockApiException)?;
        // `deploy` validates, pools, and (with propose-on-deploy) proposes the transfer.
        self.block_api
            .deploy(&signed)
            .await
            .map_err(BlockApiException)?;
        Ok(FaucetResponse {
            deploy_id: base16::encode(&signed.sig),
            amount: faucet::FAUCET_AMOUNT,
            to: address.to_string(),
        })
    }

    async fn listen_for_data_at_name(
        &self,
        request: &DataAtNameRequest,
    ) -> Result<DataAtNameResponse, BlockApiException> {
        let par = unforg_to_par(&request.name).map_err(BlockApiException)?;
        let (dbs, length) = self
            .block_api
            .get_listening_name_data_response(request.depth, &par)
            .await
            .map_err(BlockApiException)?;
        Ok(to_data_at_name_response(&dbs, length))
    }

    async fn get_data_at_par(
        &self,
        request: &DataAtNameByBlockHashRequest,
    ) -> Result<RhoDataResponse, BlockApiException> {
        let par = rho_expr_to_par(&request.name).map_err(BlockApiException)?;
        let (pars, block) = self
            .block_api
            .get_data_at_par(&par, &request.block_hash, request.use_pre_state_hash)
            .await
            .map_err(BlockApiException)?;
        Ok(to_rho_data_response(&pars, &block))
    }

    async fn last_finalized_block(&self) -> Result<BlockInfo, BlockApiException> {
        self.block_api
            .last_finalized_block()
            .await
            .map_err(BlockApiException)
    }

    async fn get_block(&self, hash: &str) -> Result<BlockInfo, BlockApiException> {
        self.block_api
            .get_block(hash)
            .await
            .map_err(BlockApiException)
    }

    async fn get_blocks(&self, depth: i32) -> Result<Vec<LightBlockInfo>, BlockApiException> {
        self.block_api
            .get_blocks(depth)
            .await
            .map_err(BlockApiException)
    }

    async fn find_deploy(&self, deploy_id: &str) -> Result<LightBlockInfo, BlockApiException> {
        let id = base16::decode(deploy_id).ok_or_else(invalid_deploy_id)?;
        self.block_api
            .find_deploy(&id)
            .await
            .map_err(BlockApiException)
    }

    async fn find_deployer(&self, key: &str) -> Result<DeployerInfo, BlockApiException> {
        // Either the 32-byte `blake2b256` hash of the key (what a wallet checking a key that has
        // never signed should send: the hash does not reveal the key) or, for convenience, the
        // 65-byte key itself, hashed here. Anything else is refused rather than answered: "not seen"
        // for a mistyped key would tell a wallet the wrong thing about the key it meant.
        let hash = match base16::decode(key) {
            Some(h) if h.len() == 32 => h,
            Some(k) if k.len() == 65 => deployer_index_key(&k),
            _ => {
                return Err(BlockApiException(
                    "Expected the hex blake2b256 hash of a public key (32 bytes) or a 65-byte \
                     uncompressed secp256k1 public key."
                        .to_string(),
                ))
            }
        };
        self.block_api
            .find_deployer(&hash)
            .await
            .map_err(BlockApiException)
    }

    async fn exploratory_deploy(
        &self,
        term: &str,
        block_hash: Option<&str>,
        use_pre_state_hash: bool,
    ) -> Result<ExploratoryDeployResponse, BlockApiException> {
        let (reply, block) = self
            .block_api
            .exploratory_deploy(term, block_hash, use_pre_state_hash)
            .await
            .map_err(BlockApiException)?;
        Ok(to_exploratory_deploy_response(&reply, &block))
    }

    async fn get_blocks_by_heights(
        &self,
        start_block_number: i64,
        end_block_number: i64,
    ) -> Result<Vec<LightBlockInfo>, BlockApiException> {
        self.block_api
            .get_blocks_by_heights(start_block_number, end_block_number)
            .await
            .map_err(BlockApiException)
    }

    async fn is_finalized(&self, hash: &str) -> Result<bool, BlockApiException> {
        self.block_api
            .is_finalized(hash)
            .await
            .map_err(BlockApiException)
    }

    async fn get_transaction(&self, hash: &str) -> Result<TransactionResponse, BlockApiException> {
        if hash.is_empty() {
            return Err(BlockApiException("Block hash cannot be empty.".to_string()));
        }
        let blake =
            Blake2b256Hash::from_hex_either(hash).map_err(|e| BlockApiException(e.to_string()))?;
        let data = self
            .transaction_api
            .get_transaction(&blake)
            .await
            .map_err(BlockApiException)?;
        Ok(TransactionResponse { data })
    }
}
