//! Tonic (gRPC) bindings for the gRPC service adapters.
//!
//! Bridges the tonic-generated `ProposeService`/`Repl` traits (whose message types are the prost
//! wire types in `rchain_models::proto`) to the hand-written adapters in this module. The
//! `DeployService` binding is added incrementally.

use std::pin::Pin;

use tokio_stream::Stream;
use tonic::{Request, Response, Status};

use rchain_models::casper::protocol::casper_message::{Peek, SignedDeployData, SystemDeployData};
use rchain_models::casper::protocol::deploy_service::{
    deploy_info_from_wire, light_block_info_from_wire, BlockInfo, BlockQuery, BlocksQuery,
    BlocksQueryByHeight, BondInfo, BondStatusQuery, ContinuationAtNameQuery,
    ContinuationsWithBlockInfo, DataAtNameByBlockQuery, DataAtNameQuery, DataWithBlockInfo,
    DeployExecStatus, DeployInfo, ExploratoryDeployQuery, FindDeployQuery, IsFinalizedQuery,
    LightBlockInfo, MachineVerifyQuery, ReportQuery, ServiceError, Status as CasperStatus,
    VersionInfo, VisualizeDagQuery, WaitingContinuationInfo,
};
use rchain_models::casper::protocol::propose_service::{ProposeQuery, ProposeResultQuery};
use rchain_models::casper::protocol::report::{
    BlockEventInfo, DeployInfoWithEventData, ReportCommProto, ReportConsumeProto,
    ReportProduceProto, ReportProto, SingleReport, SystemDeployInfoWithEventData,
};
use rchain_models::proto::casper as wire;
use rchain_models::proto::casper::deploy_service_server::DeployService;
use rchain_models::proto::casper::propose_service_server::ProposeService;
use rchain_models::proto::casper::{
    propose_response, propose_result_response, ProposeResponse, ProposeResultResponse,
    ServiceError as TonicServiceError,
};
use rchain_models::proto::repl::repl_server::Repl;
use rchain_models::proto::repl::ReplResponse as TonicReplResponse;
use rchain_models::wire::{
    bind_pattern_from_proto, bind_pattern_to_proto, list_par_with_random_from_proto,
    list_par_with_random_to_proto, par_from_proto, par_to_proto,
};

use super::deploy_grpc_service_v1::DeployGrpcServiceV1;
use super::propose_grpc_service_v1::ProposeGrpcServiceV1;
use super::repl_grpc_service::{CmdRequest, EvalRequest, ReplGrpcService};

fn to_tonic_service_error(e: ServiceError) -> TonicServiceError {
    TonicServiceError {
        messages: e.messages,
    }
}

fn propose_response(r: Result<String, ServiceError>) -> ProposeResponse {
    let message = match r {
        Ok(s) => propose_response::Message::Result(s),
        Err(e) => propose_response::Message::Error(to_tonic_service_error(e)),
    };
    ProposeResponse {
        message: Some(message),
    }
}

fn propose_result_response(r: Result<String, ServiceError>) -> ProposeResultResponse {
    let message = match r {
        Ok(s) => propose_result_response::Message::Result(s),
        Err(e) => propose_result_response::Message::Error(to_tonic_service_error(e)),
    };
    ProposeResultResponse {
        message: Some(message),
    }
}

#[tonic::async_trait]
impl ProposeService for ProposeGrpcServiceV1 {
    async fn propose(
        &self,
        request: Request<rchain_models::proto::casper::ProposeQuery>,
    ) -> Result<Response<ProposeResponse>, Status> {
        let req = request.into_inner();
        let r = self
            .propose(&ProposeQuery {
                is_async: req.is_async,
            })
            .await;
        Ok(Response::new(propose_response(r)))
    }

    async fn propose_result(
        &self,
        _request: Request<rchain_models::proto::casper::ProposeResultQuery>,
    ) -> Result<Response<ProposeResultResponse>, Status> {
        let r = self.propose_result(&ProposeResultQuery).await;
        Ok(Response::new(propose_result_response(r)))
    }
}

#[tonic::async_trait]
impl Repl for ReplGrpcService {
    async fn run(
        &self,
        request: Request<rchain_models::proto::repl::CmdRequest>,
    ) -> Result<Response<TonicReplResponse>, Status> {
        let req = request.into_inner();
        let resp = self.run(&CmdRequest { line: req.line }).await;
        Ok(Response::new(TonicReplResponse {
            output: resp.output,
        }))
    }

    async fn eval(
        &self,
        request: Request<rchain_models::proto::repl::EvalRequest>,
    ) -> Result<Response<TonicReplResponse>, Status> {
        let req = request.into_inner();
        let resp = self
            .eval(&EvalRequest {
                program: req.program,
                print_unmatched_sends_only: req.print_unmatched_sends_only,
            })
            .await;
        Ok(Response::new(TonicReplResponse {
            output: resp.output,
        }))
    }
}

// -------------------------------------------------------------------------------------------------
// DeployService conversion helpers
// -------------------------------------------------------------------------------------------------

fn bond_info_to_wire(b: &BondInfo) -> wire::BondInfo {
    wire::BondInfo {
        validator: b.validator.clone(),
        stake: b.stake,
    }
}

fn light_block_info_to_wire(b: &LightBlockInfo) -> wire::LightBlockInfo {
    wire::LightBlockInfo {
        version: b.version,
        shard_id: b.shard_id.clone(),
        block_hash: b.block_hash.clone(),
        block_number: b.block_number,
        sender: b.sender.clone(),
        seq_num: b.seq_num,
        pre_state_hash: b.pre_state_hash.clone(),
        post_state_hash: b.post_state_hash.clone(),
        justifications: b.justifications.clone(),
        bonds: b.bonds.iter().map(bond_info_to_wire).collect(),
        sig_algorithm: b.sig_algorithm.clone(),
        sig: b.sig.clone(),
        block_size: b.block_size.clone(),
        deploy_count: b.deploy_count,
        rejected_deploys: b.rejected_deploys.clone(),
        timestamp: b.timestamp,
    }
}

fn deploy_info_to_wire(d: &DeployInfo) -> wire::DeployInfo {
    wire::DeployInfo {
        deployer: d.deployer.clone(),
        term: d.term.clone(),
        timestamp: d.timestamp,
        sig: d.sig.clone(),
        sig_algorithm: d.sig_algorithm.clone(),
        phlo_price: d.phlo_price,
        phlo_limit: d.phlo_limit,
        valid_after_block_number: d.valid_after_block_number,
        cost: d.cost,
        errored: d.errored,
        system_deploy_error: d.system_deploy_error.clone(),
    }
}

fn block_info_to_wire(b: &BlockInfo) -> wire::BlockInfo {
    wire::BlockInfo {
        block_info: Some(light_block_info_to_wire(&b.block_info)),
        deploys: b.deploys.iter().map(deploy_info_to_wire).collect(),
    }
}

fn version_info_to_wire(v: &VersionInfo) -> wire::VersionInfo {
    wire::VersionInfo {
        api: v.api.clone(),
        node: v.node.clone(),
    }
}

fn status_to_wire(s: &CasperStatus) -> wire::Status {
    wire::Status {
        version: Some(version_info_to_wire(&s.version)),
        address: s.address.clone(),
        network_id: s.network_id.clone(),
        shard_id: s.shard_id.clone(),
        peers: s.peers,
        nodes: s.nodes,
        min_phlo_price: s.min_phlo_price,
        latest_block_number: s.latest_block_number,
    }
}

fn deploy_exec_status_to_wire(s: &DeployExecStatus) -> wire::DeployExecStatus {
    use wire::deploy_exec_status::Status as WireStatus;
    let status = match s {
        DeployExecStatus::ProcessedWithSuccess {
            deploy_result,
            block,
        } => WireStatus::ProcessedWithSuccess(wire::ProcessedWithSuccess {
            deploy_result: deploy_result.iter().map(par_to_proto).collect(),
            block: Some(light_block_info_to_wire(block)),
        }),
        DeployExecStatus::ProcessedWithError {
            deploy_error,
            block,
        } => WireStatus::ProcessedWithError(wire::ProcessedWithError {
            deploy_error: deploy_error.clone(),
            block: Some(light_block_info_to_wire(block)),
        }),
        DeployExecStatus::NotProcessed { status } => WireStatus::NotProcessed(wire::NotProcessed {
            status: status.clone(),
        }),
    };
    wire::DeployExecStatus {
        status: Some(status),
    }
}

fn data_with_block_info_to_wire(d: &DataWithBlockInfo) -> wire::DataWithBlockInfo {
    wire::DataWithBlockInfo {
        post_block_data: d.post_block_data.iter().map(par_to_proto).collect(),
        block: Some(light_block_info_to_wire(&d.block)),
    }
}

fn waiting_continuation_info_to_wire(w: &WaitingContinuationInfo) -> wire::WaitingContinuationInfo {
    wire::WaitingContinuationInfo {
        post_block_patterns: w
            .post_block_patterns
            .iter()
            .map(bind_pattern_to_proto)
            .collect(),
        post_block_continuation: Some(par_to_proto(&w.post_block_continuation)),
    }
}

fn continuations_with_block_info_to_wire(
    c: &ContinuationsWithBlockInfo,
) -> wire::ContinuationsWithBlockInfo {
    wire::ContinuationsWithBlockInfo {
        post_block_continuations: c
            .post_block_continuations
            .iter()
            .map(waiting_continuation_info_to_wire)
            .collect(),
        block: Some(light_block_info_to_wire(&c.block)),
    }
}

fn report_produce_to_wire(r: &ReportProduceProto) -> wire::ReportProduceProto {
    wire::ReportProduceProto {
        channel: Some(par_to_proto(&r.channel)),
        data: Some(list_par_with_random_to_proto(&r.data)),
    }
}

fn report_consume_to_wire(r: &ReportConsumeProto) -> wire::ReportConsumeProto {
    wire::ReportConsumeProto {
        channels: r.channels.iter().map(par_to_proto).collect(),
        patterns: r.patterns.iter().map(bind_pattern_to_proto).collect(),
        peeks: r
            .peeks
            .iter()
            .map(|p| wire::PeekProto {
                channel_index: p.channel_index,
            })
            .collect(),
    }
}

fn report_comm_to_wire(r: &ReportCommProto) -> wire::ReportCommProto {
    wire::ReportCommProto {
        consume: Some(report_consume_to_wire(&r.consume)),
        produces: r.produces.iter().map(report_produce_to_wire).collect(),
    }
}

fn report_proto_to_wire(r: &ReportProto) -> wire::ReportProto {
    use wire::report_proto::Report as WireReport;
    let report = match r {
        ReportProto::Produce(p) => WireReport::Produce(report_produce_to_wire(p)),
        ReportProto::Consume(c) => WireReport::Consume(report_consume_to_wire(c)),
        ReportProto::Comm(c) => WireReport::Comm(report_comm_to_wire(c)),
    };
    wire::ReportProto {
        report: Some(report),
    }
}

fn single_report_to_wire(s: &SingleReport) -> wire::SingleReport {
    wire::SingleReport {
        events: s.events.iter().map(report_proto_to_wire).collect(),
    }
}

fn deploy_info_with_event_data_to_wire(
    d: &DeployInfoWithEventData,
) -> wire::DeployInfoWithEventData {
    wire::DeployInfoWithEventData {
        deploy_info: Some(deploy_info_to_wire(&d.deploy_info)),
        report: d.report.iter().map(single_report_to_wire).collect(),
    }
}

fn system_deploy_info_with_event_data_to_wire(
    s: &SystemDeployInfoWithEventData,
) -> wire::SystemDeployInfoWithEventData {
    wire::SystemDeployInfoWithEventData {
        system_deploy: Some(s.system_deploy.to_proto()),
        report: s.report.iter().map(single_report_to_wire).collect(),
    }
}

pub fn block_event_info_to_wire(b: &BlockEventInfo) -> wire::BlockEventInfo {
    wire::BlockEventInfo {
        block_info: Some(light_block_info_to_wire(&b.block_info)),
        deploys: b
            .deploys
            .iter()
            .map(deploy_info_with_event_data_to_wire)
            .collect(),
        system_deploys: b
            .system_deploys
            .iter()
            .map(system_deploy_info_with_event_data_to_wire)
            .collect(),
        post_state_hash: b.post_state_hash.clone(),
    }
}

fn report_produce_from_wire(r: &wire::ReportProduceProto) -> Result<ReportProduceProto, String> {
    let channel = r.channel.as_ref().ok_or("missing channel")?;
    let data = r.data.as_ref().ok_or("missing data")?;
    Ok(ReportProduceProto {
        channel: par_from_proto(channel).map_err(|e| e.to_string())?,
        data: list_par_with_random_from_proto(data).map_err(|e| e.to_string())?,
    })
}

fn report_consume_from_wire(r: &wire::ReportConsumeProto) -> Result<ReportConsumeProto, String> {
    let mut channels = Vec::new();
    for c in &r.channels {
        channels.push(par_from_proto(c).map_err(|e| e.to_string())?);
    }
    let mut patterns = Vec::new();
    for p in &r.patterns {
        patterns.push(bind_pattern_from_proto(p).map_err(|e| e.to_string())?);
    }
    let peeks = r
        .peeks
        .iter()
        .map(|p| Peek {
            channel_index: p.channel_index,
        })
        .collect();
    Ok(ReportConsumeProto {
        channels,
        patterns,
        peeks,
    })
}

fn report_comm_from_wire(r: &wire::ReportCommProto) -> Result<ReportCommProto, String> {
    let consume = r.consume.as_ref().ok_or("missing consume")?;
    let mut produces = Vec::new();
    for p in &r.produces {
        produces.push(report_produce_from_wire(p)?);
    }
    Ok(ReportCommProto {
        consume: report_consume_from_wire(consume)?,
        produces,
    })
}

fn report_proto_from_wire(r: &wire::ReportProto) -> Result<ReportProto, String> {
    use wire::report_proto::Report as WireReport;
    match r.report.as_ref().ok_or("missing report")? {
        WireReport::Produce(p) => Ok(ReportProto::Produce(report_produce_from_wire(p)?)),
        WireReport::Consume(c) => Ok(ReportProto::Consume(report_consume_from_wire(c)?)),
        WireReport::Comm(c) => Ok(ReportProto::Comm(report_comm_from_wire(c)?)),
    }
}

fn single_report_from_wire(s: &wire::SingleReport) -> Result<SingleReport, String> {
    let mut events = Vec::new();
    for e in &s.events {
        events.push(report_proto_from_wire(e)?);
    }
    Ok(SingleReport { events })
}

fn deploy_info_with_event_data_from_wire(
    d: &wire::DeployInfoWithEventData,
) -> Result<DeployInfoWithEventData, String> {
    let deploy_info = d.deploy_info.as_ref().ok_or("missing deploy_info")?;
    let mut report = Vec::new();
    for r in &d.report {
        report.push(single_report_from_wire(r)?);
    }
    Ok(DeployInfoWithEventData {
        deploy_info: deploy_info_from_wire(deploy_info),
        report,
    })
}

fn system_deploy_info_with_event_data_from_wire(
    s: &wire::SystemDeployInfoWithEventData,
) -> Result<SystemDeployInfoWithEventData, String> {
    let system_deploy = s.system_deploy.as_ref().ok_or("missing system_deploy")?;
    let mut report = Vec::new();
    for r in &s.report {
        report.push(single_report_from_wire(r)?);
    }
    Ok(SystemDeployInfoWithEventData {
        system_deploy: SystemDeployData::from_proto(system_deploy).map_err(|e| e.to_string())?,
        report,
    })
}

/// Decode a `wire::BlockEventInfo` back into the domain `BlockEventInfo` (mirror of
/// [`block_event_info_to_wire`]).
pub fn block_event_info_from_wire(b: &wire::BlockEventInfo) -> Result<BlockEventInfo, String> {
    let block_info = b.block_info.as_ref().ok_or("missing block_info")?;
    let mut deploys = Vec::new();
    for d in &b.deploys {
        deploys.push(deploy_info_with_event_data_from_wire(d)?);
    }
    let mut system_deploys = Vec::new();
    for s in &b.system_deploys {
        system_deploys.push(system_deploy_info_with_event_data_from_wire(s)?);
    }
    Ok(BlockEventInfo {
        block_info: light_block_info_from_wire(block_info),
        deploys,
        system_deploys,
        post_state_hash: b.post_state_hash.clone(),
    })
}

type StreamOf<T> = Pin<Box<dyn Stream<Item = Result<T, Status>> + Send + 'static>>;

// -------------------------------------------------------------------------------------------------
// DeployService tonic binding
// -------------------------------------------------------------------------------------------------

#[tonic::async_trait]
impl DeployService for DeployGrpcServiceV1 {
    async fn do_deploy(
        &self,
        request: Request<wire::DeployDataProto>,
    ) -> Result<Response<wire::DeployResponse>, Status> {
        let req = request.into_inner();
        let signed = SignedDeployData::from_proto(&req);
        let r = self.do_deploy(&signed).await;
        let message = match r {
            Ok(s) => wire::deploy_response::Message::Result(s),
            Err(e) => wire::deploy_response::Message::Error(to_tonic_service_error(e)),
        };
        Ok(Response::new(wire::DeployResponse {
            message: Some(message),
        }))
    }

    async fn deploy_status(
        &self,
        request: Request<wire::FindDeployQuery>,
    ) -> Result<Response<wire::DeployStatusResponse>, Status> {
        let req = request.into_inner();
        let r = self
            .deploy_status(&FindDeployQuery {
                deploy_id: req.deploy_id,
            })
            .await;
        let message = match r {
            Ok(s) => wire::deploy_status_response::Message::DeployExecStatus(
                deploy_exec_status_to_wire(&s),
            ),
            Err(e) => wire::deploy_status_response::Message::Error(to_tonic_service_error(e)),
        };
        Ok(Response::new(wire::DeployStatusResponse {
            message: Some(message),
        }))
    }

    async fn get_block(
        &self,
        request: Request<wire::BlockQuery>,
    ) -> Result<Response<wire::BlockResponse>, Status> {
        let req = request.into_inner();
        let r = self.get_block(&BlockQuery { hash: req.hash }).await;
        let message = match r {
            Ok(b) => wire::block_response::Message::BlockInfo(block_info_to_wire(&b)),
            Err(e) => wire::block_response::Message::Error(to_tonic_service_error(e)),
        };
        Ok(Response::new(wire::BlockResponse {
            message: Some(message),
        }))
    }

    type visualizeDagStream = StreamOf<wire::VisualizeBlocksResponse>;
    async fn visualize_dag(
        &self,
        request: Request<wire::VisualizeDagQuery>,
    ) -> Result<Response<Self::visualizeDagStream>, Status> {
        let req = request.into_inner();
        let r = self
            .visualize_dag(&VisualizeDagQuery {
                depth: req.depth,
                show_justification_lines: req.show_justification_lines,
                start_block_number: req.start_block_number,
            })
            .await;
        let items = match r {
            Ok(strings) => strings
                .into_iter()
                .map(|s| {
                    Ok(wire::VisualizeBlocksResponse {
                        message: Some(wire::visualize_blocks_response::Message::Content(s)),
                    })
                })
                .collect(),
            Err(e) => vec![Ok(wire::VisualizeBlocksResponse {
                message: Some(wire::visualize_blocks_response::Message::Error(
                    to_tonic_service_error(e),
                )),
            })],
        };
        Ok(Response::new(Box::pin(tokio_stream::iter(items))))
    }

    async fn machine_verifiable_dag(
        &self,
        request: Request<wire::MachineVerifyQuery>,
    ) -> Result<Response<wire::MachineVerifyResponse>, Status> {
        let req = request.into_inner();
        let r = self
            .machine_verifiable_dag(&MachineVerifyQuery { depth: req.depth })
            .await;
        let message = match r {
            Ok(s) => wire::machine_verify_response::Message::Content(s),
            Err(e) => wire::machine_verify_response::Message::Error(to_tonic_service_error(e)),
        };
        Ok(Response::new(wire::MachineVerifyResponse {
            message: Some(message),
        }))
    }

    type getBlocksStream = StreamOf<wire::BlockInfoResponse>;
    async fn get_blocks(
        &self,
        request: Request<wire::BlocksQuery>,
    ) -> Result<Response<Self::getBlocksStream>, Status> {
        let req = request.into_inner();
        let r = self.get_blocks(&BlocksQuery { depth: req.depth }).await;
        let items = match r {
            Ok(blocks) => blocks
                .into_iter()
                .map(|b| {
                    Ok(wire::BlockInfoResponse {
                        message: Some(wire::block_info_response::Message::BlockInfo(
                            light_block_info_to_wire(&b),
                        )),
                    })
                })
                .collect(),
            Err(e) => vec![Ok(wire::BlockInfoResponse {
                message: Some(wire::block_info_response::Message::Error(
                    to_tonic_service_error(e),
                )),
            })],
        };
        Ok(Response::new(Box::pin(tokio_stream::iter(items))))
    }

    async fn listen_for_data_at_name(
        &self,
        request: Request<wire::DataAtNameQuery>,
    ) -> Result<Response<wire::ListeningNameDataResponse>, Status> {
        let req = request.into_inner();
        // The wire `Par` is the prost type; the domain query needs the domain `Par`.
        let name = rchain_models::wire::par_from_proto(
            &req.name
                .ok_or_else(|| Status::invalid_argument("missing name"))?,
        )
        .map_err(|e| Status::invalid_argument(e.to_string()))?;
        let r = self
            .listen_for_data_at_name(&DataAtNameQuery {
                depth: req.depth,
                name,
            })
            .await;
        let message = match r {
            Ok((block_info, length)) => wire::listening_name_data_response::Message::Payload(
                wire::ListeningNameDataPayload {
                    block_info: block_info
                        .iter()
                        .map(data_with_block_info_to_wire)
                        .collect(),
                    length,
                },
            ),
            Err(e) => wire::listening_name_data_response::Message::Error(to_tonic_service_error(e)),
        };
        Ok(Response::new(wire::ListeningNameDataResponse {
            message: Some(message),
        }))
    }

    async fn get_data_at_name(
        &self,
        request: Request<wire::DataAtNameByBlockQuery>,
    ) -> Result<Response<wire::RhoDataResponse>, Status> {
        let req = request.into_inner();
        let par = rchain_models::wire::par_from_proto(
            &req.par
                .ok_or_else(|| Status::invalid_argument("missing par"))?,
        )
        .map_err(|e| Status::invalid_argument(e.to_string()))?;
        let r = self
            .get_data_at_name(&DataAtNameByBlockQuery {
                par,
                block_hash: req.block_hash,
                use_pre_state_hash: req.use_pre_state_hash,
            })
            .await;
        let message = match r {
            Ok((pars, block)) => wire::rho_data_response::Message::Payload(wire::RhoDataPayload {
                par: pars.iter().map(par_to_proto).collect(),
                block: Some(light_block_info_to_wire(&block)),
            }),
            Err(e) => wire::rho_data_response::Message::Error(to_tonic_service_error(e)),
        };
        Ok(Response::new(wire::RhoDataResponse {
            message: Some(message),
        }))
    }

    async fn listen_for_continuation_at_name(
        &self,
        request: Request<wire::ContinuationAtNameQuery>,
    ) -> Result<Response<wire::ContinuationAtNameResponse>, Status> {
        let req = request.into_inner();
        let names: Result<Vec<_>, _> = req
            .names
            .into_iter()
            .map(|n| {
                rchain_models::wire::par_from_proto(&n)
                    .map_err(|e| Status::invalid_argument(e.to_string()))
            })
            .collect();
        let r = self
            .listen_for_continuation_at_name(&ContinuationAtNameQuery {
                depth: req.depth,
                names: names?,
            })
            .await;
        let message = match r {
            Ok((continuations, length)) => wire::continuation_at_name_response::Message::Payload(
                wire::ContinuationAtNamePayload {
                    block_results: continuations
                        .iter()
                        .map(continuations_with_block_info_to_wire)
                        .collect(),
                    length,
                },
            ),
            Err(e) => {
                wire::continuation_at_name_response::Message::Error(to_tonic_service_error(e))
            }
        };
        Ok(Response::new(wire::ContinuationAtNameResponse {
            message: Some(message),
        }))
    }

    async fn find_deploy(
        &self,
        request: Request<wire::FindDeployQuery>,
    ) -> Result<Response<wire::FindDeployResponse>, Status> {
        let req = request.into_inner();
        let r = self
            .find_deploy(&FindDeployQuery {
                deploy_id: req.deploy_id,
            })
            .await;
        let message = match r {
            Ok(b) => wire::find_deploy_response::Message::BlockInfo(light_block_info_to_wire(&b)),
            Err(e) => wire::find_deploy_response::Message::Error(to_tonic_service_error(e)),
        };
        Ok(Response::new(wire::FindDeployResponse {
            message: Some(message),
        }))
    }

    async fn last_finalized_block(
        &self,
        _request: Request<wire::LastFinalizedBlockQuery>,
    ) -> Result<Response<wire::LastFinalizedBlockResponse>, Status> {
        let r = self.last_finalized_block().await;
        let message = match r {
            Ok(b) => {
                wire::last_finalized_block_response::Message::BlockInfo(block_info_to_wire(&b))
            }
            Err(e) => {
                wire::last_finalized_block_response::Message::Error(to_tonic_service_error(e))
            }
        };
        Ok(Response::new(wire::LastFinalizedBlockResponse {
            message: Some(message),
        }))
    }

    async fn is_finalized(
        &self,
        request: Request<wire::IsFinalizedQuery>,
    ) -> Result<Response<wire::IsFinalizedResponse>, Status> {
        let req = request.into_inner();
        let r = self
            .is_finalized(&IsFinalizedQuery { hash: req.hash })
            .await;
        let message = match r {
            Ok(b) => wire::is_finalized_response::Message::IsFinalized(b),
            Err(e) => wire::is_finalized_response::Message::Error(to_tonic_service_error(e)),
        };
        Ok(Response::new(wire::IsFinalizedResponse {
            message: Some(message),
        }))
    }

    async fn bond_status(
        &self,
        request: Request<wire::BondStatusQuery>,
    ) -> Result<Response<wire::BondStatusResponse>, Status> {
        let req = request.into_inner();
        let r = self
            .bond_status(&BondStatusQuery {
                public_key: req.public_key,
            })
            .await;
        let message = match r {
            Ok(b) => wire::bond_status_response::Message::IsBonded(b),
            Err(e) => wire::bond_status_response::Message::Error(to_tonic_service_error(e)),
        };
        Ok(Response::new(wire::BondStatusResponse {
            message: Some(message),
        }))
    }

    async fn exploratory_deploy(
        &self,
        request: Request<wire::ExploratoryDeployQuery>,
    ) -> Result<Response<wire::ExploratoryDeployResponse>, Status> {
        let req = request.into_inner();
        let r = self
            .exploratory_deploy(&ExploratoryDeployQuery {
                term: req.term,
                block_hash: req.block_hash,
                use_pre_state_hash: req.use_pre_state_hash,
            })
            .await;
        let message = match r {
            Ok((pars, block)) => {
                wire::exploratory_deploy_response::Message::Result(wire::DataWithBlockInfo {
                    post_block_data: pars.iter().map(par_to_proto).collect(),
                    block: Some(light_block_info_to_wire(&block)),
                })
            }
            Err(e) => wire::exploratory_deploy_response::Message::Error(to_tonic_service_error(e)),
        };
        Ok(Response::new(wire::ExploratoryDeployResponse {
            message: Some(message),
        }))
    }

    type getBlocksByHeightsStream = StreamOf<wire::BlockInfoResponse>;
    async fn get_blocks_by_heights(
        &self,
        request: Request<wire::BlocksQueryByHeight>,
    ) -> Result<Response<Self::getBlocksByHeightsStream>, Status> {
        let req = request.into_inner();
        let r = self
            .get_blocks_by_heights(&BlocksQueryByHeight {
                start_block_number: req.start_block_number,
                end_block_number: req.end_block_number,
            })
            .await;
        let items = match r {
            Ok(blocks) => blocks
                .into_iter()
                .map(|b| {
                    Ok(wire::BlockInfoResponse {
                        message: Some(wire::block_info_response::Message::BlockInfo(
                            light_block_info_to_wire(&b),
                        )),
                    })
                })
                .collect(),
            Err(e) => vec![Ok(wire::BlockInfoResponse {
                message: Some(wire::block_info_response::Message::Error(
                    to_tonic_service_error(e),
                )),
            })],
        };
        Ok(Response::new(Box::pin(tokio_stream::iter(items))))
    }

    async fn get_event_by_hash(
        &self,
        request: Request<wire::ReportQuery>,
    ) -> Result<Response<wire::EventInfoResponse>, Status> {
        // Reporting is disabled by default (`api-server.enable-reporting = false`); the RPC answers
        // NotFound unless explicitly enabled (M6 — the flag was read but never enforced for the
        // event-report RPC).
        if !self.enable_reporting {
            return Err(Status::not_found("reporting is disabled"));
        }
        let req = request.into_inner();
        let r = self
            .get_event_by_hash(&ReportQuery {
                hash: req.hash,
                force_replay: req.force_replay,
            })
            .await;
        let message = match r {
            Ok(b) => wire::event_info_response::Message::Result(block_event_info_to_wire(&b)),
            Err(e) => wire::event_info_response::Message::Error(to_tonic_service_error(e)),
        };
        Ok(Response::new(wire::EventInfoResponse {
            message: Some(message),
        }))
    }

    async fn status(
        &self,
        _request: Request<()>,
    ) -> Result<Response<wire::StatusResponse>, Status> {
        let s = self.status().await;
        Ok(Response::new(wire::StatusResponse {
            message: Some(wire::status_response::Message::Status(status_to_wire(&s))),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use async_trait::async_trait;
    use rchain_casper::api::block_api::{ApiErr, BlockApi, Capabilities};
    use rchain_models::ast::Par;
    use rchain_models::block_metadata::BlockMetadata;
    use rchain_models::casper::protocol::casper_message::SignedDeployData;
    use rchain_models::casper::protocol::deploy_service::{
        BlockInfo, ContinuationsWithBlockInfo, DataWithBlockInfo, DeployExecStatus, LightBlockInfo,
    };
    use rchain_models::proto::casper::propose_service_client::ProposeServiceClient;
    use rchain_models::proto::casper::propose_service_server::ProposeServiceServer;

    struct StubBlockApi;
    #[async_trait]
    impl BlockApi for StubBlockApi {
        async fn status(&self) -> CasperStatus {
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
        ) -> ApiErr<(Vec<Par>, LightBlockInfo)> {
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
    async fn serves_and_answers_propose() {
        let svc = ProposeGrpcServiceV1::new(Arc::new(StubBlockApi));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = ::tonic::transport::Server::builder()
            .add_service(ProposeServiceServer::new(svc))
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener));
        tokio::spawn(server);

        let mut client = ProposeServiceClient::connect(format!("http://{addr}"))
            .await
            .unwrap();
        let resp = client
            .propose(wire::ProposeQuery { is_async: false })
            .await
            .unwrap()
            .into_inner();
        assert!(resp.message.is_some());
    }

    /// The wire conversions: 26 functions that no test named. They are pure, they sit on the
    /// boundary between the node's domain and every gRPC/HTTP client, and a transposition in one of
    /// them is invisible until a client reads the wrong field.
    mod wire_conversions {
        use super::*;
        use rchain_crypto::hash::blake2b512_random::Blake2b512Random;
        use rchain_models::ast::{Expr, Par};
        use rchain_models::casper::protocol::deploy_service::{
            BondInfo, DeployInfo, VersionInfo,
        };
        use rchain_models::casper::protocol::report::{
            ReportCommProto, ReportConsumeProto, ReportProduceProto, ReportProto, SingleReport,
        };
        use rchain_models::runtime::{BindPattern, ListParWithRandom};
        use rchain_models::sorted::Sorted;

        /// A par carrying a distinct integer, so two pars in one message are distinguishable —
        /// `Par::default()` is the same empty par for every field, which would let a swap pass.
        fn par_of(n: i64) -> Par {
            let mut par = Par::default();
            par.exprs = vec![Expr::GInt(n)];
            par
        }

        fn sorted(n: i64) -> Sorted<rchain_models::ast::ProcSort> {
            Sorted::new(par_of(n))
        }

        fn list_par(n: i64) -> ListParWithRandom {
            ListParWithRandom {
                pars: vec![sorted(n)],
                random_state: Blake2b512Random::from_init(&[n as u8]),
            }
        }

        fn bind_pattern(n: i64) -> BindPattern {
            BindPattern {
                patterns: vec![sorted(n)],
                remainder: None,
                free_count: n as i32,
            }
        }

        /// A block info whose **every field is distinct**, with the two easy-to-transpose hash
        /// fields (`pre`/`post`) and the two integer pairs (`block_number`/`seq_num`) marked.
        fn block_info() -> LightBlockInfo {
            LightBlockInfo {
                version: 1,
                shard_id: "shard".to_string(),
                block_hash: "block-hash".to_string(),
                block_number: 4,
                sender: "sender".to_string(),
                seq_num: 5,
                pre_state_hash: "PRE".to_string(),
                post_state_hash: "POST".to_string(),
                justifications: vec!["j1".to_string(), "j2".to_string()],
                bonds: vec![
                    BondInfo {
                        validator: "v1".to_string(),
                        stake: 11,
                    },
                    BondInfo {
                        validator: "v2".to_string(),
                        stake: 22,
                    },
                ],
                sig_algorithm: "sig-alg".to_string(),
                sig: "sig".to_string(),
                block_size: "1234".to_string(),
                deploy_count: 6,
                rejected_deploys: vec!["r1".to_string()],
                timestamp: 7000,
            }
        }

        fn deploy_info() -> DeployInfo {
            DeployInfo {
                deployer: "deployer".to_string(),
                term: "term".to_string(),
                timestamp: 1,
                sig: "deploy-sig".to_string(),
                sig_algorithm: "deploy-alg".to_string(),
                phlo_price: 2,
                phlo_limit: 3,
                valid_after_block_number: 4,
                cost: 5,
                errored: true,
                system_deploy_error: "system-error".to_string(),
            }
        }

        /// The domain→wire conversions are field-for-field, for the flat messages and for the
        /// nested ones. A field left at its default (a `Some` forgotten on a nested message) shows
        /// up as a missing inner value rather than as a wrong one.
        #[test]
        fn the_domain_to_wire_conversions_carry_every_field() {
            let b = light_block_info_to_wire(&block_info());
            assert_eq!(b.version, 1);
            assert_eq!(b.shard_id, "shard");
            assert_eq!(b.block_hash, "block-hash");
            assert_eq!(b.block_number, 4);
            assert_eq!(b.sender, "sender");
            assert_eq!(b.seq_num, 5);
            assert_eq!(b.pre_state_hash, "PRE", "pre is not post");
            assert_eq!(b.post_state_hash, "POST");
            assert_eq!(b.justifications, vec!["j1", "j2"]);
            assert_eq!(b.bonds.len(), 2);
            assert_eq!(b.bonds[0].validator, "v1");
            assert_eq!(b.bonds[0].stake, 11);
            assert_eq!(b.bonds[1].stake, 22);
            assert_eq!(b.sig_algorithm, "sig-alg");
            assert_eq!(b.sig, "sig");
            assert_eq!(b.block_size, "1234");
            assert_eq!(b.deploy_count, 6);
            assert_eq!(b.rejected_deploys, vec!["r1"]);
            assert_eq!(b.timestamp, 7000);

            let d = deploy_info_to_wire(&deploy_info());
            assert_eq!(d.deployer, "deployer");
            assert_eq!(d.term, "term");
            assert_eq!(d.timestamp, 1);
            assert_eq!(d.sig, "deploy-sig");
            assert_eq!(d.sig_algorithm, "deploy-alg");
            assert_eq!(d.phlo_price, 2, "price is not limit");
            assert_eq!(d.phlo_limit, 3);
            assert_eq!(d.valid_after_block_number, 4);
            assert_eq!(d.cost, 5, "cost is the u64, not a phlo bound");
            assert!(d.errored);
            assert_eq!(d.system_deploy_error, "system-error");

            let block = block_info_to_wire(&BlockInfo {
                block_info: block_info(),
                deploys: vec![deploy_info()],
            });
            assert_eq!(block.block_info.expect("inner block").shard_id, "shard");
            assert_eq!(block.deploys.len(), 1);
            assert_eq!(block.deploys[0].term, "term");

            let status = status_to_wire(&CasperStatus {
                version: VersionInfo {
                    api: "1".to_string(),
                    node: "2".to_string(),
                },
                address: "addr".to_string(),
                network_id: "net".to_string(),
                shard_id: "shard".to_string(),
                peers: 3,
                nodes: 4,
                min_phlo_price: 5,
                latest_block_number: 6,
            });
            assert_eq!(status.version.expect("version").api, "1");
            assert_eq!(status.address, "addr");
            assert_eq!(status.network_id, "net");
            assert_eq!(status.shard_id, "shard");
            assert_eq!(status.peers, 3);
            assert_eq!(status.nodes, 4);
            assert_eq!(status.min_phlo_price, 5);
            assert_eq!(status.latest_block_number, 6);

            // The list-bearing messages keep their element order and their nested `Some`s.
            let data = data_with_block_info_to_wire(&DataWithBlockInfo {
                post_block_data: vec![par_of(1), par_of(2)],
                block: block_info(),
            });
            assert_eq!(data.post_block_data.len(), 2);
            assert_eq!(data.block.expect("block").timestamp, 7000);

            let continuations = continuations_with_block_info_to_wire(&ContinuationsWithBlockInfo {
                post_block_continuations: vec![rchain_models::casper::protocol::deploy_service::WaitingContinuationInfo {
                    post_block_patterns: vec![bind_pattern(9)],
                    post_block_continuation: par_of(10),
                }],
                block: block_info(),
            });
            assert_eq!(continuations.post_block_continuations.len(), 1);
            assert!(continuations.post_block_continuations[0]
                .post_block_continuation
                .is_some());
        }

        /// The three execution-status variants each map to their own wire variant, and the payload
        /// of each lands inside it — `ProcessedWithSuccess` carries its deploy results and block,
        /// `ProcessedWithError` its message, `NotProcessed` its status.
        #[test]
        fn each_execution_status_maps_to_its_own_wire_variant() {
            use rchain_models::proto::casper::deploy_exec_status::Status as WireStatus;

            let success = deploy_exec_status_to_wire(&DeployExecStatus::ProcessedWithSuccess {
                deploy_result: vec![par_of(1)],
                block: block_info(),
            });
            match success.status.expect("a status") {
                WireStatus::ProcessedWithSuccess(s) => {
                    assert_eq!(s.deploy_result.len(), 1);
                    assert_eq!(s.block.expect("block").block_hash, "block-hash");
                }
                other => panic!("expected ProcessedWithSuccess, got {other:?}"),
            }

            let error = deploy_exec_status_to_wire(&DeployExecStatus::ProcessedWithError {
                deploy_error: "boom".to_string(),
                block: block_info(),
            });
            match error.status.expect("a status") {
                WireStatus::ProcessedWithError(e) => {
                    assert_eq!(e.deploy_error, "boom");
                    assert!(e.block.is_some());
                }
                other => panic!("expected ProcessedWithError, got {other:?}"),
            }

            let pending = deploy_exec_status_to_wire(&DeployExecStatus::NotProcessed {
                status: "pending".to_string(),
            });
            match pending.status.expect("a status") {
                WireStatus::NotProcessed(n) => assert_eq!(n.status, "pending"),
                other => panic!("expected NotProcessed, got {other:?}"),
            }
        }

        /// The report tree — the shape `getEventByHash` returns — round-trips: three nested event
        /// kinds, with the produce/consume/comm variants each carrying their own fields. The domain
        /// and wire types have the same shape, so a *structural* equality after `to_wire` →
        /// `from_wire` is the strongest available check, and it is what catches a swapped field.
        #[test]
        fn the_report_tree_round_trips_through_the_wire() {
            let produce = ReportProduceProto {
                channel: par_of(1),
                data: list_par(2),
            };
            let consume = ReportConsumeProto {
                channels: vec![par_of(3), par_of(4)],
                patterns: vec![bind_pattern(5)],
                peeks: vec![rchain_models::casper::protocol::casper_message::Peek { channel_index: 1 }],
            };
            let comm = ReportCommProto {
                consume: consume.clone(),
                produces: vec![produce.clone()],
            };
            let report = SingleReport {
                events: vec![
                    ReportProto::Produce(produce),
                    ReportProto::Consume(consume),
                    ReportProto::Comm(comm),
                ],
            };

            let wire_report = single_report_to_wire(&report);
            assert_eq!(wire_report.events.len(), 3);
            // The variant tags are the wire's own, one per domain variant.
            assert!(matches!(
                wire_report.events[0].report,
                Some(rchain_models::proto::casper::report_proto::Report::Produce(_))
            ));
            assert!(matches!(
                wire_report.events[1].report,
                Some(rchain_models::proto::casper::report_proto::Report::Consume(_))
            ));
            assert!(matches!(
                wire_report.events[2].report,
                Some(rchain_models::proto::casper::report_proto::Report::Comm(_))
            ));

            let back = single_report_from_wire(&wire_report).expect("round trip");
            assert_eq!(back.events.len(), 3, "no event is dropped");
            // The pars survive as distinct values, in order.
            match &back.events[1] {
                ReportProto::Consume(c) => {
                    assert_eq!(c.channels.len(), 2);
                    assert_eq!(c.peeks.len(), 1);
                    assert_eq!(c.peeks[0].channel_index, 1);
                    assert_eq!(c.patterns.len(), 1);
                }
                other => panic!("expected a consume, got {other:?}"),
            }
            match &back.events[2] {
                ReportProto::Comm(c) => {
                    assert_eq!(c.produces.len(), 1);
                    assert_eq!(c.consume.channels.len(), 2);
                }
                other => panic!("expected a comm, got {other:?}"),
            }
        }

        /// The `from_wire` conversions are **partial**: a message missing an inner message is an
        /// error naming what is missing, never a panic and never a defaulted value. Each of the
        /// seven `ok_or` arms is exercised, and the message says which field was absent — this is
        /// what an operator sees when a peer (or a client) sends a malformed report request.
        #[test]
        fn a_missing_inner_message_is_an_error_naming_the_field() {
            let empty_produce = wire::ReportProduceProto::default();
            assert_eq!(
                report_produce_from_wire(&empty_produce).expect_err("no channel"),
                "missing channel"
            );
            let mut with_channel = wire::ReportProduceProto::default();
            with_channel.channel = Some(Default::default());
            assert_eq!(
                report_produce_from_wire(&with_channel).expect_err("no data"),
                "missing data"
            );

            let empty_comm = wire::ReportCommProto::default();
            assert_eq!(
                report_comm_from_wire(&empty_comm).expect_err("no consume"),
                "missing consume"
            );

            let empty_proto = wire::ReportProto::default();
            assert_eq!(
                report_proto_from_wire(&empty_proto).expect_err("no report"),
                "missing report"
            );

            let empty_deploy = wire::DeployInfoWithEventData::default();
            assert_eq!(
                deploy_info_with_event_data_from_wire(&empty_deploy).expect_err("no deploy info"),
                "missing deploy_info"
            );

            let empty_system = wire::SystemDeployInfoWithEventData::default();
            assert_eq!(
                system_deploy_info_with_event_data_from_wire(&empty_system)
                    .expect_err("no system deploy"),
                "missing system_deploy"
            );

            let empty_block = wire::BlockEventInfo::default();
            assert_eq!(
                block_event_info_from_wire(&empty_block).expect_err("no block info"),
                "missing block_info"
            );

            // An *empty* report is not an error: a deploy that produced no events is a valid answer.
            assert_eq!(
                single_report_from_wire(&wire::SingleReport::default())
                    .expect("an empty report")
                    .events
                    .len(),
                0
            );
        }

        /// The nested `BlockEventInfo` — the whole `getEventByHash` payload — carries its deploys,
        /// system deploys and post-state hash, and round-trips through both directions.
        #[test]
        fn the_block_event_info_round_trips_with_its_nested_parts() {
            let info = BlockEventInfo {
                block_info: block_info(),
                deploys: vec![rchain_models::casper::protocol::report::DeployInfoWithEventData {
                    deploy_info: deploy_info(),
                    report: vec![SingleReport {
                        events: vec![ReportProto::Produce(ReportProduceProto {
                            channel: par_of(1),
                            data: list_par(2),
                        })],
                    }],
                }],
                system_deploys: Vec::new(),
                post_state_hash: b"POST-STATE".to_vec(),
            };

            let wire_info = block_event_info_to_wire(&info);
            assert_eq!(
                wire_info.post_state_hash,
                b"POST-STATE".to_vec(),
                "post-state hash is carried"
            );
            assert_eq!(wire_info.deploys.len(), 1);
            assert!(wire_info.deploys[0].deploy_info.is_some());
            assert_eq!(wire_info.deploys[0].report.len(), 1);
            assert_eq!(
                wire_info
                    .block_info
                    .as_ref()
                    .expect("block info")
                    .deploy_count,
                6
            );

            let back = block_event_info_from_wire(&wire_info).expect("round trip");
            assert_eq!(back.post_state_hash, b"POST-STATE".to_vec());
            assert_eq!(back.deploys.len(), 1);
            assert_eq!(back.deploys[0].deploy_info.term, "term");
            assert_eq!(back.deploys[0].report[0].events.len(), 1);
            assert_eq!(back.block_info.block_hash, "block-hash");
        }

        /// `propose_response` and `propose_result_response` are where a *domain* `Result` becomes a
        /// gRPC oneof: `Ok` becomes `Result`, `Err` becomes `Error` carrying every message. A
        /// handler that mapped both to `Result` would report a failed proposal as successful.
        #[test]
        fn a_domain_result_becomes_the_right_response_variant() {
            let ok = propose_response(Ok("proposed".to_string()));
            match ok.message.expect("a message") {
                propose_response::Message::Result(s) => assert_eq!(s, "proposed"),
                propose_response::Message::Error(e) => panic!("expected Result, got {e:?}"),
            }

            let err = propose_response(Err(ServiceError {
                messages: vec!["no".to_string(), "way".to_string()],
            }));
            match err.message.expect("a message") {
                propose_response::Message::Error(e) => {
                    assert_eq!(e.messages, vec!["no".to_string(), "way".to_string()]);
                }
                propose_response::Message::Result(s) => panic!("expected Error, got {s}"),
            }

            // The result-response variant is a different oneof with the same shape.
            let ok = propose_result_response(Ok("done".to_string()));
            match ok.message.expect("a message") {
                propose_result_response::Message::Result(s) => assert_eq!(s, "done"),
                propose_result_response::Message::Error(e) => panic!("expected Result, got {e:?}"),
            }
            let err = propose_result_response(Err(ServiceError::new("bad")));
            match err.message.expect("a message") {
                propose_result_response::Message::Error(e) => assert_eq!(e.messages, vec!["bad"]),
                propose_result_response::Message::Result(s) => panic!("expected Error, got {s}"),
            }

            // `to_tonic_service_error` carries the messages through unchanged.
            let converted = to_tonic_service_error(ServiceError::new("x"));
            assert_eq!(converted.messages, vec!["x"]);
        }
    }
}
