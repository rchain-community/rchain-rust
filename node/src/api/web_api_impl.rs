//! Web API implementation (port of `WebApi.WebApiImpl`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use rchain_casper::api::block_api::BlockApi;
use rchain_crypto::hash::blake2b256_hash::Blake2b256Hash;
use rchain_crypto::private_key::PrivateKey;
use rchain_models::casper::protocol::casper_message::SignedDeployData;
use rchain_models::casper::protocol::deploy_service::{BlockInfo, LightBlockInfo};
use rchain_rholang::util::rev_address::RevAddress;
use rchain_shared::base16;

use super::conversion::{
    to_api_status, to_data_at_name_response, to_deploy_exec_status, to_node_capabilities,
    to_pooled_deploy, to_rho_data_response, to_signed_deploy,
};
use super::dto::{
    ApiStatus, BlockApiException, DataAtNameByBlockHashRequest, DataAtNameRequest,
    DataAtNameResponse, DeployExecStatus, DeployRequest, FaucetResponse, NodeCapabilities,
    PooledDeploys, RhoDataResponse,
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

/// Maximum faucet drips any single address may receive (0.3 REV each). Bounds the dev-wallet drain
/// a single caller can cause before other developers are starved.
const FAUCET_MAX_DRIPS_PER_ADDRESS: u32 = 10;

fn invalid_deploy_id() -> BlockApiException {
    BlockApiException("Deploy id is not valid base16 format.".to_string())
}

#[async_trait]
impl WebApi for WebApiImpl {
    async fn status(&self) -> Result<ApiStatus, BlockApiException> {
        let status = self.block_api.status().await;
        let caps = self.block_api.capabilities().await;
        Ok(to_api_status(&status, &caps))
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
        let sk = self.deployer_key.as_ref().ok_or_else(|| {
            BlockApiException("faucet requires --dev-mode --deployer-private-key".to_string())
        })?;
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

    async fn exploratory_deploy(
        &self,
        term: &str,
        block_hash: Option<&str>,
        use_pre_state_hash: bool,
    ) -> Result<RhoDataResponse, BlockApiException> {
        let (pars, block) = self
            .block_api
            .exploratory_deploy(term, block_hash, use_pre_state_hash)
            .await
            .map_err(BlockApiException)?;
        Ok(to_rho_data_response(&pars, &block))
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
