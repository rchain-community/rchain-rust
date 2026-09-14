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
    use rchain_models::casper::protocol::deploy_service::{
        BlockInfo, ContinuationsWithBlockInfo, DataWithBlockInfo, DeployExecStatus, LightBlockInfo,
        Status,
    };
    use rchain_models::rholang::RhoType::RhoString;
    use rchain_shared::refined::BlockHeight;

    /// A [`BlockApi`] stub for one shard. It **records every call it receives** and answers with a
    /// value carrying its own shard id, so a test can prove *which* member a routed request reached —
    /// the property the router exists to provide, and one that a returned value alone cannot show
    /// when two shards would answer identically.
    struct StubApi {
        shard_id: String,
        deployed: Mutex<Vec<String>>,
        calls: Mutex<Vec<&'static str>>,
    }

    impl StubApi {
        fn new(shard_id: &str) -> Self {
            StubApi {
                shard_id: shard_id.to_string(),
                deployed: Mutex::new(Vec::new()),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn record(&self, method: &'static str) {
            self.calls.lock().unwrap().push(method);
        }

        fn calls(&self) -> Vec<&'static str> {
            self.calls.lock().unwrap().clone()
        }

        fn deployed(&self) -> Vec<String> {
            self.deployed.lock().unwrap().clone()
        }

        /// A light block marked with this shard's id.
        fn marked_block(&self) -> LightBlockInfo {
            LightBlockInfo {
                version: 1,
                shard_id: self.shard_id.clone(),
                block_hash: format!("{}-hash", self.shard_id),
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
                timestamp: 0,
            }
        }
    }

    #[async_trait]
    impl BlockApi for StubApi {
        // --- the methods the router forwards to the primary ---
        async fn status(&self) -> Status {
            self.record("status");
            Status {
                version: rchain_models::casper::protocol::deploy_service::VersionInfo {
                    api: "1".to_string(),
                    node: self.shard_id.clone(),
                },
                address: self.shard_id.clone(),
                network_id: "testnet".to_string(),
                shard_id: self.shard_id.clone(),
                peers: 0,
                nodes: 0,
                min_phlo_price: 1,
                latest_block_number: 1,
            }
        }
        async fn pooled_deploys(&self) -> ApiErr<Vec<SignedDeployData>> {
            self.record("pooled_deploys");
            Ok(vec![self.marked_deploy()])
        }
        async fn capabilities(&self) -> Capabilities {
            self.record("capabilities");
            // `dev_mode` is the only free boolean that does not imply a devnet, so it carries the
            // marker for "which shard answered".
            Capabilities {
                autopropose: self.shard_id.ends_with("root"),
                propose_on_deploy: false,
                manual_propose: true,
                admin_http: false,
                dev_mode: false,
            }
        }
        async fn create_block(&self, _: bool) -> ApiErr<String> {
            self.record("create_block");
            Ok(self.shard_id.clone())
        }
        async fn get_propose_result(&self) -> ApiErr<String> {
            self.record("get_propose_result");
            Ok(self.shard_id.clone())
        }
        async fn get_listening_name_data_response(
            &self,
            depth: i32,
            _: &Par,
        ) -> ApiErr<(Vec<DataWithBlockInfo>, i32)> {
            self.record("get_listening_name_data_response");
            Ok((
                vec![DataWithBlockInfo {
                    post_block_data: vec![RhoString::apply(self.shard_id.clone())],
                    block: self.marked_block(),
                }],
                depth,
            ))
        }
        async fn get_listening_name_continuation_response(
            &self,
            _: i32,
            _: &[Par],
        ) -> ApiErr<(Vec<ContinuationsWithBlockInfo>, i32)> {
            self.record("get_listening_name_continuation_response");
            // The router forwards this one; the shard is proven by the call recording, so an empty
            // answer is enough.
            Ok((Vec::new(), 0))
        }
        async fn get_blocks_by_heights(&self, _: i64, _: i64) -> ApiErr<Vec<LightBlockInfo>> {
            self.record("get_blocks_by_heights");
            Ok(vec![self.marked_block()])
        }
        async fn visualize_dag(&self, _: i32, _: i32, _: bool) -> ApiErr<Vec<String>> {
            self.record("visualize_dag");
            Ok(vec![self.shard_id.clone()])
        }
        async fn machine_verifiable_dag(&self, _: i32) -> ApiErr<String> {
            self.record("machine_verifiable_dag");
            Ok(self.shard_id.clone())
        }
        async fn get_blocks(&self, _: i32) -> ApiErr<Vec<LightBlockInfo>> {
            self.record("get_blocks");
            Ok(vec![self.marked_block()])
        }
        async fn bond_status(&self, _: &[u8]) -> ApiErr<bool> {
            self.record("bond_status");
            Ok(self.shard_id.ends_with("root"))
        }
        async fn last_finalized_block(&self) -> ApiErr<BlockInfo> {
            self.record("last_finalized_block");
            Ok(BlockInfo {
                block_info: self.marked_block(),
                deploys: Vec::new(),
            })
        }
        async fn get_latest_message(&self) -> ApiErr<BlockMetadata> {
            self.record("get_latest_message");
            Ok(BlockMetadata {
                block_hash: rchain_models::block_hash::BlockHash::new([0u8; 32]),
                block_num: BlockHeight::try_from(1).map_err(|e| e.to_string())?,
                sender: rchain_models::validator::Validator::from_slice(&[0u8; 65]),
                seq_num: rchain_shared::refined::SeqNum::zero(),
                justifications: std::collections::BTreeSet::new(),
                bonds_map: std::collections::BTreeMap::new(),
                validated: true,
                validation_failed: false,
                member_of_fringe: None,
                fringe: std::collections::BTreeSet::new(),
                fringe_state_hash:
                    rchain_crypto::hash::blake2b256_hash::Blake2b256Hash::from_bytes([0u8; 32])
                        .into(),
            })
        }

        // --- the routed / probed methods ---
        async fn deploy(&self, deploy: &SignedDeployData) -> ApiErr<String> {
            self.record("deploy");
            self.deployed.lock().unwrap().push(deploy.data.term.clone());
            Ok(format!("{}:{}", self.shard_id, deploy.data.term))
        }
        async fn deploy_status(&self, _: &DeployId) -> ApiErr<DeployExecStatus> {
            self.record("deploy_status");
            Err(format!("{} has no such deploy", self.shard_id))
        }
        async fn find_deploy(&self, _: &DeployId) -> ApiErr<LightBlockInfo> {
            self.record("find_deploy");
            Err(format!("{} has no such deploy", self.shard_id))
        }
        async fn get_block(&self, hash: &str) -> ApiErr<BlockInfo> {
            self.record("get_block");
            if hash == format!("{}-hash", self.shard_id) {
                Ok(BlockInfo {
                    block_info: self.marked_block(),
                    deploys: Vec::new(),
                })
            } else {
                Err(format!("{} has no block {hash}", self.shard_id))
            }
        }
        async fn is_finalized(&self, hash: &str) -> ApiErr<bool> {
            self.record("is_finalized");
            if hash == format!("{}-hash", self.shard_id) {
                Ok(true)
            } else {
                Err(format!("{} has no block {hash}", self.shard_id))
            }
        }
        async fn get_data_at_par(
            &self,
            par: &Par,
            block_hash: &str,
            _: bool,
        ) -> ApiErr<(Vec<Par>, LightBlockInfo)> {
            self.record("get_data_at_par");
            if block_hash == format!("{}-hash", self.shard_id) {
                Ok((vec![par.clone()], self.marked_block()))
            } else {
                Err(format!("{} has no block {block_hash}", self.shard_id))
            }
        }
        async fn exploratory_deploy(
            &self,
            _: &str,
            block_hash: Option<&str>,
            _: bool,
        ) -> ApiErr<(Vec<Par>, LightBlockInfo)> {
            self.record("exploratory_deploy");
            match block_hash {
                None => Ok((
                    vec![RhoString::apply(self.shard_id.clone())],
                    self.marked_block(),
                )),
                Some(hash) if hash == format!("{}-hash", self.shard_id) => Ok((
                    vec![RhoString::apply(self.shard_id.clone())],
                    self.marked_block(),
                )),
                Some(hash) => Err(format!("{} has no block {hash}", self.shard_id)),
            }
        }
    }

    impl StubApi {
        /// A deploy marked with this shard's id, for the pooled-deploys marker.
        fn marked_deploy(&self) -> SignedDeployData {
            SignedDeployData {
                data: DeployData {
                    term: self.shard_id.clone(),
                    timestamp: 0,
                    phlo_price: 1,
                    phlo_limit: 1,
                    valid_after_block_number: 0,
                    shard_id: self.shard_id.clone(),
                },
                deployer: vec![0u8; 65],
                sig: Vec::new(),
                sig_algorithm: "secp256k1".to_string(),
            }
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

    /// Every keyed lookup probes the members — primary first, then the rest — and finds the shard
    /// that owns the value. Only `get_block` was covered before; the other four exercise the same
    /// generic `probe`, so this is a table over all five.
    #[tokio::test]
    async fn every_keyed_lookup_probes_the_members() {
        let (api, _, child) = router();

        // A value only the child holds, in each of the five keyed shapes.
        let child_hash = "/root/child-hash";

        let block = api.get_block(child_hash).await.expect("child block");
        assert_eq!(block.block_info.shard_id, "/root/child");

        assert!(api.is_finalized(child_hash).await.expect("finality"));

        let (data, block) = api
            .get_data_at_par(&RhoString::apply("x".to_string()), child_hash, false)
            .await
            .expect("data at par");
        assert_eq!(block.shard_id, "/root/child");
        assert_eq!(data.len(), 1);

        let (data, block) = api
            .exploratory_deploy("Nil", Some(child_hash), false)
            .await
            .expect("exploratory at hash");
        assert_eq!(block.shard_id, "/root/child");
        assert_eq!(data.len(), 1);

        // `deploy_status` and `find_deploy` are errors on every member in this stub, so the probe
        // can only be observed as "it asked the child too".
        assert!(api.deploy_status(&vec![1u8, 2]).await.is_err());
        assert!(api.find_deploy(&vec![1u8, 2]).await.is_err());
        assert!(
            child
                .calls()
                .iter()
                .any(|c| *c == "deploy_status" || *c == "find_deploy"),
            "the probe must reach the non-primary members: {:?}",
            child.calls()
        );
    }

    /// When no member has the key, the reported error is the **primary's** — not an invented one and
    /// not the last member's.
    #[tokio::test]
    async fn an_unknown_key_reports_the_primaries_error() {
        let (api, primary, _) = router();
        let err = api
            .get_block("nowhere")
            .await
            .expect_err("an unknown hash must be reported");
        assert!(err.starts_with("/root"), "{err}");
        assert_eq!(primary.calls(), vec!["get_block"]);
    }

    /// `exploratory_deploy` with no block hash is not a keyed lookup: it runs against the primary's
    /// head, and must not probe the other members.
    #[tokio::test]
    async fn exploratory_deploy_without_a_hash_uses_the_primary() {
        let (api, primary, child) = router();
        let (_, block) = api
            .exploratory_deploy("Nil", None, false)
            .await
            .expect("exploratory");
        assert_eq!(block.shard_id, "/root");
        assert_eq!(primary.calls(), vec!["exploratory_deploy"]);
        assert!(
            child.calls().is_empty(),
            "an unrouted request must not reach other members: {:?}",
            child.calls()
        );
    }

    /// The other **fourteen** `BlockApi` methods are not shard-selecting: they answer for the
    /// primary. The test asserts that by *which stub recorded the call* — the only way to tell when
    /// both members would answer with something plausible. (`exploratory_deploy` is the fifteenth
    /// unrouted call but only when it carries no block hash, so it has its own test above.)
    #[tokio::test]
    async fn the_primary_answers_every_unrouted_method() {
        let (api, primary, child) = router();

        // Each entry drives one method and asserts the primary's marker comes back (where the shape
        // carries one) — and, for all of them, that the child was never called.
        assert_eq!(api.status().await.shard_id, "/root");
        assert_eq!(api.pooled_deploys().await.unwrap()[0].data.term, "/root");
        assert!(api.capabilities().await.autopropose, "the primary's value");
        assert_eq!(api.create_block(true).await.unwrap(), "/root");
        assert_eq!(api.get_propose_result().await.unwrap(), "/root");
        let (data, _) = api
            .get_listening_name_data_response(1, &RhoString::apply("n".to_string()))
            .await
            .unwrap();
        assert!(matches!(
            RhoString::unapply(&data[0].post_block_data[0]),
            Some("/root")
        ));
        assert!(api
            .get_listening_name_continuation_response(1, &[])
            .await
            .is_ok());
        assert_eq!(
            api.get_blocks_by_heights(0, 1).await.unwrap()[0].shard_id,
            "/root"
        );
        assert_eq!(api.visualize_dag(1, 0, false).await.unwrap(), vec!["/root"]);
        assert_eq!(api.machine_verifiable_dag(1).await.unwrap(), "/root");
        assert_eq!(api.get_blocks(1).await.unwrap()[0].shard_id, "/root");
        assert!(api.bond_status(&[0u8; 65]).await.unwrap());
        assert_eq!(
            api.last_finalized_block()
                .await
                .unwrap()
                .block_info
                .shard_id,
            "/root"
        );
        assert_eq!(
            i64::from(api.get_latest_message().await.unwrap().block_num),
            1
        );

        assert!(
            child.calls().is_empty(),
            "no unrouted method may reach a non-primary member: {:?}",
            child.calls()
        );
        assert_eq!(
            primary.calls().len(),
            14,
            "every unrouted method reached the primary exactly once: {:?}",
            primary.calls()
        );
    }

    /// A one-member router is the identity: every request goes to the single shard, which is what
    /// makes a single-shard node's behaviour unchanged by this layer.
    #[tokio::test]
    async fn a_one_member_router_is_the_identity() {
        let only = Arc::new(StubApi::new("/root"));
        let mut shards: BTreeMap<ShardId, Arc<dyn BlockApi>> = BTreeMap::new();
        shards.insert(shard("/root"), only.clone());
        let api = ShardRoutingBlockApi::new(shards, shard("/root")).unwrap();

        assert_eq!(
            api.deploy(&deploy_to("/root", "term")).await.unwrap(),
            "/root:term"
        );
        assert_eq!(api.status().await.shard_id, "/root");
        assert_eq!(
            api.get_block("/root-hash")
                .await
                .unwrap()
                .block_info
                .shard_id,
            "/root"
        );
        // And a request for a shard it is not a member of is still refused.
        assert!(api.deploy(&deploy_to("/elsewhere", "t")).await.is_err());
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
