//! Shard routing for the client-facing [`BlockApi`] (Law 26).
//!
//! A node that is a member of several shards presents **one** client surface (one deploy port, one
//! HTTP port, one protocol port). `ShardRoutingBlockApi` dispatches each request to the member that
//! owns it, so the gRPC and HTTP services above it stay shard-agnostic:
//!
//! * a deploy names its shard on the wire (`DeployData.shard_id`), so it is dispatched exactly, and
//!   a deploy for a shard this node is not a member of is **rejected** — never relayed (relaying is
//!   an explicit non-goal, `docs/src/node/shard-invoke.md`);
//! * the keyed lookups (`get_block`, `is_finalized`, `deploy_status`, `find_deploy`,
//!   `get_data_at_par`) probe the members, because the key exists in exactly one shard. Block hashes
//!   and deploy ids are shard-disjoint — the shard id is a signed field of the block and part of its
//!   hash — so a probe cannot match two members;
//! * everything that names neither a shard nor a key resolves to the **primary** shard, so a
//!   single-shard node behaves exactly as before and a multi-shard node has a defined default
//!   rather than an arbitrary one.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;

use rchain_block_storage::dag::dag_storage::DeployId;
use rchain_casper::api::block_api::{ApiErr, BlockApi, Capabilities};
use rchain_models::ast::Par;
use rchain_models::block_metadata::BlockMetadata;
use rchain_models::casper::protocol::casper_message::SignedDeployData;
use rchain_models::casper::protocol::deploy_service::{
    BlockInfo, ContinuationsWithBlockInfo, DataWithBlockInfo, DeployExecStatus, LightBlockInfo,
    Status,
};
use rchain_shared::refined::ShardId;

/// A [`BlockApi`] over the node's member shards: routes by shard id where the request carries one,
/// probes by key where it carries a key, and defaults to the primary shard otherwise.
pub struct ShardRoutingBlockApi {
    shards: BTreeMap<ShardId, Arc<dyn BlockApi>>,
    /// The primary shard's id **paired with its API**, so `primary_api` is total: there is no lookup
    /// that could fail and therefore no `expect` on the hot path of every unrouted request.
    primary: (ShardId, Arc<dyn BlockApi>),
}

impl ShardRoutingBlockApi {
    /// Build the router. `primary` must be one of `shards` — a primary the node is not a member of
    /// would have no API to dispatch to.
    pub fn new(
        shards: BTreeMap<ShardId, Arc<dyn BlockApi>>,
        primary: ShardId,
    ) -> Result<Self, String> {
        let primary_api = shards
            .get(&primary)
            .ok_or_else(|| {
                format!("primary shard '{primary}' is not among the node's member shards")
            })?
            .clone();
        Ok(ShardRoutingBlockApi {
            shards,
            primary: (primary, primary_api),
        })
    }

    /// The member that owns `shard_id`, or the membership error.
    fn member(&self, shard_id: &str) -> ApiErr<&Arc<dyn BlockApi>> {
        // The comparison is on the canonical full id string, which is what the wire carries
        // (`DeployData.shard_id` and `BlockMessage.shard_id` are both full ids).
        self.shards
            .iter()
            .find(|(id, _)| id.to_string() == shard_id)
            .map(|(_, api)| api)
            .ok_or_else(|| {
                format!(
                    "Deploy shardId '{shard_id}' is not a member of this node's shards: [{}]",
                    self.member_ids().join(", ")
                )
            })
    }

    fn member_ids(&self) -> Vec<String> {
        self.shards.keys().map(|id| id.to_string()).collect()
    }

    fn primary_api(&self) -> &Arc<dyn BlockApi> {
        &self.primary.1
    }

    /// The first member that answers `probe` with `Ok`, or the primary's result when none does —
    /// so a value unknown to every member reports the primary's error rather than an invented one.
    async fn probe<T, F, Fut>(&self, probe: F) -> ApiErr<T>
    where
        F: Fn(Arc<dyn BlockApi>) -> Fut,
        Fut: std::future::Future<Output = ApiErr<T>>,
    {
        // Primary first, so the common single-shard case costs one call.
        let primary = self.primary_api().clone();
        let primary_result = probe(primary.clone()).await;
        if primary_result.is_ok() {
            return primary_result;
        }
        for (id, api) in &self.shards {
            if *id == self.primary.0 {
                continue;
            }
            if let Ok(value) = probe(api.clone()).await {
                return Ok(value);
            }
        }
        primary_result
    }
}

#[async_trait]
impl BlockApi for ShardRoutingBlockApi {
    async fn status(&self) -> Status {
        self.primary_api().status().await
    }

    /// Dispatch by the shard named in the deploy; reject a non-member shard outright.
    async fn deploy(&self, deploy: &SignedDeployData) -> ApiErr<String> {
        self.member(&deploy.data.shard_id)?.deploy(deploy).await
    }

    async fn deploy_status(&self, deploy_id: &DeployId) -> ApiErr<DeployExecStatus> {
        let id = deploy_id.clone();
        self.probe(move |api| {
            let id = id.clone();
            async move { api.deploy_status(&id).await }
        })
        .await
    }

    async fn pooled_deploys(&self) -> ApiErr<Vec<SignedDeployData>> {
        self.primary_api().pooled_deploys().await
    }

    async fn capabilities(&self) -> Capabilities {
        self.primary_api().capabilities().await
    }

    async fn create_block(&self, is_async: bool) -> ApiErr<String> {
        self.primary_api().create_block(is_async).await
    }

    async fn get_propose_result(&self) -> ApiErr<String> {
        self.primary_api().get_propose_result().await
    }

    async fn get_listening_name_data_response(
        &self,
        depth: i32,
        listening_name: &Par,
    ) -> ApiErr<(Vec<DataWithBlockInfo>, i32)> {
        self.primary_api()
            .get_listening_name_data_response(depth, listening_name)
            .await
    }

    async fn get_listening_name_continuation_response(
        &self,
        depth: i32,
        listening_names: &[Par],
    ) -> ApiErr<(Vec<ContinuationsWithBlockInfo>, i32)> {
        self.primary_api()
            .get_listening_name_continuation_response(depth, listening_names)
            .await
    }

    async fn get_blocks_by_heights(
        &self,
        start_block_number: i64,
        end_block_number: i64,
    ) -> ApiErr<Vec<LightBlockInfo>> {
        self.primary_api()
            .get_blocks_by_heights(start_block_number, end_block_number)
            .await
    }

    async fn visualize_dag(
        &self,
        depth: i32,
        start_block_number: i32,
        show_justification_lines: bool,
    ) -> ApiErr<Vec<String>> {
        self.primary_api()
            .visualize_dag(depth, start_block_number, show_justification_lines)
            .await
    }

    async fn machine_verifiable_dag(&self, depth: i32) -> ApiErr<String> {
        self.primary_api().machine_verifiable_dag(depth).await
    }

    async fn get_blocks(&self, depth: i32) -> ApiErr<Vec<LightBlockInfo>> {
        self.primary_api().get_blocks(depth).await
    }

    async fn find_deploy(&self, id: &DeployId) -> ApiErr<LightBlockInfo> {
        let id = id.clone();
        self.probe(move |api| {
            let id = id.clone();
            async move { api.find_deploy(&id).await }
        })
        .await
    }

    async fn get_block(&self, hash: &str) -> ApiErr<BlockInfo> {
        let hash = hash.to_string();
        self.probe(move |api| {
            let hash = hash.clone();
            async move { api.get_block(&hash).await }
        })
        .await
    }

    async fn bond_status(&self, public_key: &[u8]) -> ApiErr<bool> {
        self.primary_api().bond_status(public_key).await
    }

    async fn exploratory_deploy(
        &self,
        term: &str,
        block_hash: Option<&str>,
        use_pre_state_hash: bool,
    ) -> ApiErr<(Vec<Par>, LightBlockInfo)> {
        // An exploratory deploy against a named block belongs to that block's shard; with no block
        // named it runs against the primary shard's head.
        match block_hash {
            Some(hash) => {
                let term = term.to_string();
                let hash = hash.to_string();
                self.probe(move |api| {
                    let term = term.clone();
                    let hash = hash.clone();
                    async move {
                        api.exploratory_deploy(&term, Some(&hash), use_pre_state_hash)
                            .await
                    }
                })
                .await
            }
            None => {
                self.primary_api()
                    .exploratory_deploy(term, None, use_pre_state_hash)
                    .await
            }
        }
    }

    async fn get_data_at_par(
        &self,
        par: &Par,
        block_hash: &str,
        use_pre_state_hash: bool,
    ) -> ApiErr<(Vec<Par>, LightBlockInfo)> {
        let par = par.clone();
        let hash = block_hash.to_string();
        self.probe(move |api| {
            let par = par.clone();
            let hash = hash.clone();
            async move { api.get_data_at_par(&par, &hash, use_pre_state_hash).await }
        })
        .await
    }

    async fn last_finalized_block(&self) -> ApiErr<BlockInfo> {
        self.primary_api().last_finalized_block().await
    }

    async fn is_finalized(&self, hash: &str) -> ApiErr<bool> {
        let hash = hash.to_string();
        self.probe(move |api| {
            let hash = hash.clone();
            async move { api.is_finalized(&hash).await }
        })
        .await
    }

    async fn get_latest_message(&self) -> ApiErr<BlockMetadata> {
        self.primary_api().get_latest_message().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use rchain_models::casper::protocol::casper_message::DeployData;

    /// A [`BlockApi`] stub that records the deploys it is handed and answers one block hash.
    struct StubApi {
        shard_id: String,
        deployed: Mutex<Vec<String>>,
    }

    impl StubApi {
        fn new(shard_id: &str) -> Self {
            StubApi {
                shard_id: shard_id.to_string(),
                deployed: Mutex::new(Vec::new()),
            }
        }

        fn deployed(&self) -> Vec<String> {
            self.deployed.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl BlockApi for StubApi {
        async fn status(&self) -> Status {
            unreachable!("not used in these tests")
        }
        async fn deploy(&self, deploy: &SignedDeployData) -> ApiErr<String> {
            self.deployed.lock().unwrap().push(deploy.data.term.clone());
            Ok(format!("{}:{}", self.shard_id, deploy.data.term))
        }
        async fn deploy_status(&self, _: &DeployId) -> ApiErr<DeployExecStatus> {
            Err(format!("{} has no such deploy", self.shard_id))
        }
        async fn pooled_deploys(&self) -> ApiErr<Vec<SignedDeployData>> {
            unreachable!("not used in these tests")
        }
        async fn capabilities(&self) -> Capabilities {
            unreachable!("not used in these tests")
        }
        async fn create_block(&self, _: bool) -> ApiErr<String> {
            unreachable!("not used in these tests")
        }
        async fn get_propose_result(&self) -> ApiErr<String> {
            unreachable!("not used in these tests")
        }
        async fn get_listening_name_data_response(
            &self,
            _: i32,
            _: &Par,
        ) -> ApiErr<(Vec<DataWithBlockInfo>, i32)> {
            unreachable!("not used in these tests")
        }
        async fn get_listening_name_continuation_response(
            &self,
            _: i32,
            _: &[Par],
        ) -> ApiErr<(Vec<ContinuationsWithBlockInfo>, i32)> {
            unreachable!("not used in these tests")
        }
        async fn get_blocks_by_heights(&self, _: i64, _: i64) -> ApiErr<Vec<LightBlockInfo>> {
            unreachable!("not used in these tests")
        }
        async fn visualize_dag(&self, _: i32, _: i32, _: bool) -> ApiErr<Vec<String>> {
            unreachable!("not used in these tests")
        }
        async fn machine_verifiable_dag(&self, _: i32) -> ApiErr<String> {
            unreachable!("not used in these tests")
        }
        async fn get_blocks(&self, _: i32) -> ApiErr<Vec<LightBlockInfo>> {
            unreachable!("not used in these tests")
        }
        async fn find_deploy(&self, _: &DeployId) -> ApiErr<LightBlockInfo> {
            Err(format!("{} has no such deploy", self.shard_id))
        }
        async fn get_block(&self, hash: &str) -> ApiErr<BlockInfo> {
            if hash == format!("{}hash", self.shard_id) {
                Ok(BlockInfo {
                    block_info: LightBlockInfo {
                        version: 1,
                        shard_id: self.shard_id.clone(),
                        block_hash: hash.to_string(),
                        block_number: 1,
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
                    },
                    deploys: Vec::new(),
                })
            } else {
                Err(format!("{} has no block {hash}", self.shard_id))
            }
        }
        async fn bond_status(&self, _: &[u8]) -> ApiErr<bool> {
            unreachable!("not used in these tests")
        }
        async fn exploratory_deploy(
            &self,
            _: &str,
            _: Option<&str>,
            _: bool,
        ) -> ApiErr<(Vec<Par>, LightBlockInfo)> {
            unreachable!("not used in these tests")
        }
        async fn get_data_at_par(
            &self,
            _: &Par,
            _: &str,
            _: bool,
        ) -> ApiErr<(Vec<Par>, LightBlockInfo)> {
            unreachable!("not used in these tests")
        }
        async fn last_finalized_block(&self) -> ApiErr<BlockInfo> {
            unreachable!("not used in these tests")
        }
        async fn is_finalized(&self, _: &str) -> ApiErr<bool> {
            Err("unknown block".to_string())
        }
        async fn get_latest_message(&self) -> ApiErr<BlockMetadata> {
            unreachable!("not used in these tests")
        }
    }

    fn shard(id: &str) -> ShardId {
        ShardId::try_from(id.to_string()).unwrap()
    }

    fn deploy_to(shard_id: &str, term: &str) -> SignedDeployData {
        SignedDeployData {
            data: DeployData {
                term: term.to_string(),
                timestamp: 0,
                phlo_price: 1,
                phlo_limit: 1,
                valid_after_block_number: 0,
                shard_id: shard_id.to_string(),
            },
            deployer: vec![0u8; 65],
            sig: Vec::new(),
            sig_algorithm: "secp256k1".to_string(),
        }
    }

    fn router() -> (ShardRoutingBlockApi, Arc<StubApi>, Arc<StubApi>) {
        let primary = Arc::new(StubApi::new("/root"));
        let child = Arc::new(StubApi::new("/root/child"));
        let mut shards: BTreeMap<ShardId, Arc<dyn BlockApi>> = BTreeMap::new();
        shards.insert(shard("/root"), primary.clone());
        shards.insert(shard("/root/child"), child.clone());
        (
            ShardRoutingBlockApi::new(shards, shard("/root")).unwrap(),
            primary,
            child,
        )
    }

    #[tokio::test]
    async fn deploy_is_dispatched_to_the_named_shard() {
        let (api, primary, child) = router();
        assert_eq!(
            api.deploy(&deploy_to("/root/child", "child-term"))
                .await
                .unwrap(),
            "/root/child:child-term"
        );
        assert_eq!(
            api.deploy(&deploy_to("/root", "primary-term"))
                .await
                .unwrap(),
            "/root:primary-term"
        );
        // Each shard saw exactly its own deploy — no cross-shard leakage.
        assert_eq!(primary.deployed(), vec!["primary-term".to_string()]);
        assert_eq!(child.deployed(), vec!["child-term".to_string()]);
    }

    #[tokio::test]
    async fn deploy_to_a_non_member_shard_is_rejected_with_the_membership() {
        let (api, primary, child) = router();
        let err = api
            .deploy(&deploy_to("/elsewhere", "term"))
            .await
            .expect_err("a non-member shard must be rejected");
        assert!(err.contains("/elsewhere"), "{err}");
        assert!(err.contains("/root, /root/child"), "{err}");
        assert!(primary.deployed().is_empty());
        assert!(child.deployed().is_empty());
    }

    /// A keyed lookup finds the member that owns the value, in either direction.
    #[tokio::test]
    async fn keyed_lookups_probe_the_members() {
        let (api, _, _) = router();
        assert_eq!(
            api.get_block("/root/childhash")
                .await
                .unwrap()
                .block_info
                .shard_id,
            "/root/child"
        );
        assert_eq!(
            api.get_block("/roothash")
                .await
                .unwrap()
                .block_info
                .shard_id,
            "/root"
        );
        let err = api
            .get_block("nowhere")
            .await
            .expect_err("an unknown hash must be reported");
        assert!(err.contains("nowhere"), "{err}");
    }

    #[tokio::test]
    async fn a_router_needs_a_primary_that_is_a_member() {
        let mut shards: BTreeMap<ShardId, Arc<dyn BlockApi>> = BTreeMap::new();
        shards.insert(shard("/root/child"), Arc::new(StubApi::new("/root/child")));
        let err = match ShardRoutingBlockApi::new(shards, shard("/root")) {
            Ok(_) => panic!("a foreign primary must be rejected"),
            Err(err) => err,
        };
        assert!(err.contains("not among"), "{err}");
    }
}
