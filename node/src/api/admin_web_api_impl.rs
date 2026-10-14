//! Admin web API implementation (port of `AdminWebApi.AdminWebApiImpl`).

use std::sync::Arc;

use async_trait::async_trait;

use rchain_casper::api::block_api::BlockApi;

use super::admin_web_api::AdminWebApi;
use super::dto::{BlockApiException, OperatorStatus};

/// The admin web API implementation (port of `AdminWebApi.AdminWebApiImpl`).
pub struct AdminWebApiImpl {
    block_api: Arc<dyn BlockApi>,
}

impl AdminWebApiImpl {
    pub fn new(block_api: Arc<dyn BlockApi>) -> Self {
        AdminWebApiImpl { block_api }
    }
}

#[async_trait]
impl AdminWebApi for AdminWebApiImpl {
    async fn propose(&self) -> Result<String, BlockApiException> {
        self.block_api
            .create_block(false)
            .await
            .map_err(BlockApiException)
    }

    async fn propose_result(&self) -> Result<String, BlockApiException> {
        self.block_api
            .get_propose_result()
            .await
            .map_err(BlockApiException)
    }

    /// **The operator half of the status** (W-N3-01), read here rather than on the public listener.
    ///
    /// Everything it needs is reachable from `self.block_api` plus four process-wide counters — which
    /// is why this is a method on the admin API and not a new dependency threaded into `HttpState`:
    /// the capability was never the missing part, the *route* was on the wrong listener.
    async fn operator_status(&self) -> Result<OperatorStatus, BlockApiException> {
        let status = self.block_api.status().await;
        let caps = self.block_api.capabilities().await;
        let health = self.block_api.proposer_health().await;
        // C249's F-U10-01: the merge's and finality's observations, which reached the log and nothing
        // else — plus the poisoned-lock count, read from `rspace` because the poisoning is a
        // storage-layer event and the count is process-wide (F-U9-03).
        let finality = self.block_api.finality_health().await;
        // Law 69's other half: the sync task's own state, so a node latched in `NodeSyncing` says so on
        // this surface instead of only on stderr. `None` is the ordinary reading — the task has
        // returned — and it is not the same as a completed sync.
        let sync = self.block_api.sync_health().await;
        let core = super::conversion::to_public_status(&status, &caps);
        Ok(super::conversion::to_operator_status(
            core,
            &caps,
            &health,
            &finality,
            sync.as_ref(),
            // F-U12-02: the durable reset marker, rendered. Read from the marker store's own cell — which
            // every writer writes — so it cannot go stale behind the store it reports.
            rchain_casper::interpreter_util::reset_marker_state().map(|m| {
                use rchain_block_storage::dag::dag_storage::ResetPhase;
                let phase = match m.phase {
                    ResetPhase::Rewinding => "rewinding",
                    ResetPhase::Rewound => "rewound",
                };
                format!(
                    "{phase} to {} at height {} (the catch-up walk clears it when its frontier passes \
                     that height)",
                    m.anchor.to_hex(),
                    i64::from(m.height)
                )
            }),
            rchain_rspace::lock::poison_recoveries(),
            // C254: the supervisor's counters. Every long-lived background task is watched by
            // `rchain_shared::supervise::spawn_supervised`, so "a subsystem stopped" is a number a
            // probe can read rather than an absence an operator has to infer from the silence.
            rchain_shared::supervise::tasks_panicked(),
            rchain_shared::supervise::tasks_exited(),
            // C254's other residual: `rchain_shared::chan` counts every send a closed channel
            // swallowed, and nothing published the count.
            rchain_shared::chan::undelivered_sends(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use rchain_casper::api::block_api::{ApiErr, Capabilities};
    use rchain_casper::runtime_manager::CapturedReply;
    use rchain_models::ast::Par;
    use rchain_models::block_metadata::BlockMetadata;
    use rchain_models::casper::protocol::casper_message::SignedDeployData;
    use rchain_models::casper::protocol::deploy_service::{
        BlockInfo, ContinuationsWithBlockInfo, DataWithBlockInfo, DeployExecStatus, LightBlockInfo,
        Status,
    };

    struct StubBlockApi;
    #[async_trait]
    impl BlockApi for StubBlockApi {
        async fn status(&self) -> Status {
            unimplemented!()
        }
        async fn deploy(&self, _: &SignedDeployData) -> ApiErr<String> {
            unimplemented!()
        }
        async fn deploy_status(&self, _: &Vec<u8>) -> ApiErr<DeployExecStatus> {
            unimplemented!()
        }
        async fn pooled_deploys(&self) -> ApiErr<Vec<SignedDeployData>> {
            unimplemented!()
        }
        async fn capabilities(&self) -> Capabilities {
            unimplemented!()
        }
        async fn create_block(&self, _: bool) -> ApiErr<String> {
            Ok("Success! Block created.".to_string())
        }
        async fn get_propose_result(&self) -> ApiErr<String> {
            Ok("Success! Block created.".to_string())
        }
        /// Delegates to the floored read: this double has no unfinalised view of its own, and the
        /// method is required rather than defaulted so that is a decision rather than an oversight
        /// (`ShardRoutingBlockApi` is the implementation that had to forward it for real).
        async fn get_unfinalised_name_data_response(
            &self,
            depth: i32,
            listening_name: &Par,
        ) -> ApiErr<(Vec<DataWithBlockInfo>, i32)> {
            self.get_listening_name_data_response(depth, listening_name)
                .await
        }

        async fn get_listening_name_data_response(
            &self,
            _: i32,
            _: &Par,
        ) -> ApiErr<(Vec<DataWithBlockInfo>, i32)> {
            unimplemented!()
        }
        async fn get_listening_name_continuation_response(
            &self,
            _: i32,
            _: &[Par],
        ) -> ApiErr<(Vec<ContinuationsWithBlockInfo>, i32)> {
            unimplemented!()
        }
        async fn get_blocks_by_heights(&self, _: i64, _: i64) -> ApiErr<Vec<LightBlockInfo>> {
            unimplemented!()
        }
        async fn visualize_dag(&self, _: i32, _: i32, _: bool) -> ApiErr<Vec<String>> {
            unimplemented!()
        }
        async fn machine_verifiable_dag(&self, _: i32) -> ApiErr<String> {
            unimplemented!()
        }
        async fn get_blocks(&self, _: i32) -> ApiErr<Vec<LightBlockInfo>> {
            unimplemented!()
        }
        async fn find_deploy(&self, _: &Vec<u8>) -> ApiErr<LightBlockInfo> {
            unimplemented!()
        }
        async fn get_block(&self, _: &str) -> ApiErr<BlockInfo> {
            unimplemented!()
        }
        async fn bond_status(&self, _: &[u8]) -> ApiErr<bool> {
            unimplemented!()
        }
        async fn exploratory_deploy(
            &self,
            _: &str,
            _: Option<&str>,
            _: bool,
        ) -> ApiErr<(CapturedReply, LightBlockInfo)> {
            unimplemented!()
        }
        async fn get_data_at_par(
            &self,
            _: &Par,
            _: &str,
            _: bool,
        ) -> ApiErr<(Vec<Par>, LightBlockInfo)> {
            unimplemented!()
        }
        async fn last_finalized_block(&self) -> ApiErr<BlockInfo> {
            unimplemented!()
        }
        async fn is_finalized(&self, _: &str) -> ApiErr<bool> {
            unimplemented!()
        }
        async fn get_latest_message(&self) -> ApiErr<BlockMetadata> {
            unimplemented!()
        }
    }

    #[tokio::test]
    async fn propose_and_result_delegate_to_block_api() {
        let api = AdminWebApiImpl::new(Arc::new(StubBlockApi));
        assert_eq!(api.propose().await.unwrap(), "Success! Block created.");
        assert_eq!(
            api.propose_result().await.unwrap(),
            "Success! Block created."
        );
    }
}
