//! Client-side two-phase-commit coordinator (off-chain, Law 27).
//!
//! The per-shard [`rho:txn`] participant (a native system process, see
//! `rholang/src/system_processes.rs`) runs the local `prepare`/`commit`/`abort` state machine over a
//! REV escrow. This module is the **coordinator**: the off-chain driver that signs the phase deploys
//! to every participant shard, collects the `prepare` votes, decides commit-or-abort, and applies it.
//! The atomicity (all-or-nothing) is supplied here — by the vote collection and the single decision —
//! not by any one shard's reducer, which is deterministic and does no network I/O.
//!
//! The transport is the existing [`crate::shard_invoke`] primitive: each phase is an ordinary
//! caller-signed remote deploy whose reply arrives on `` `rho:rchain:deployId` ``.

use std::time::Duration;

use rchain_crypto::private_key::PrivateKey;
use rchain_crypto::public_key::PublicKey;
use rchain_models::ast::Par;
use rchain_models::casper::protocol::casper_message::SignedDeployData;
use rchain_models::rholang::RhoType::{RhoByteArray, RhoNumber, RhoString};
use rchain_rholang::pretty_printer::PrettyPrinter;
use rchain_shared::base16;

use crate::protocol::client::DeployService;
use crate::shard_invoke::{await_reply, signed_invoke, ShardOutcome};

/// The URN of the per-shard 2PC participant system process.
pub const TXN_URN: &str = "rho:txn";

/// Render one data argument as a rholang literal. A byte array renders as `"<hex>".hexToBytes()`
/// (the pretty printer would otherwise emit a bare hex string, which is not a rholang literal).
fn render_arg(p: &Par) -> String {
    if let Some(bytes) = RhoByteArray::unapply(p) {
        format!("\"{}\".hexToBytes()", base16::encode(bytes))
    } else {
        PrettyPrinter::new().build_string(p)
    }
}

/// Build the term that runs a `rho:txn` phase on a far shard. `data_args` are the phase's data
/// arguments (rendered to rholang literals); the caller's `*deployerId` and the reply channel
/// `` `rho:rchain:deployId` `` are appended automatically.
///
/// Public so the block-pipeline replay tests can drive the *production* phase terms through
/// `compute_state`/`replay_compute_state` rather than re-spelling the template.
pub fn txn_term(method: &str, txn_id: &[u8], data_args: &[Par], needs_deployer: bool) -> String {
    let mut args = vec![
        format!("\"{method}\""),
        format!("\"{}\".hexToBytes()", base16::encode(txn_id)),
    ];
    args.extend(data_args.iter().map(render_arg));
    if needs_deployer {
        args.push("*deployerId".to_string());
    }
    // The reply channel is the deploy's own id, bound via `new` so the normalizer injects the
    // NormalizerEnv binding (a bare `` `rho:rchain:deployId` `` URI stays a `GUri` ground and never
    // resolves to the unforgeable deploy id).
    args.push("*deployId".to_string());
    let call = args.join(", ");
    if needs_deployer {
        format!("new txn(`{TXN_URN}`), deployerId(`rho:rchain:deployerId`), deployId(`rho:rchain:deployId`) in {{ txn!({call}) }}")
    } else {
        format!("new txn(`{TXN_URN}`), deployId(`rho:rchain:deployId`) in {{ txn!({call}) }}")
    }
}

/// One leg of a 2PC transaction: the target shard, the REV amount to escrow, and the commit
/// destination REV address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TxnLeg {
    pub shard_id: String,
    pub amount: i64,
    pub to: String,
}

/// A client-side 2PC coordinator: signs the phase deploys with `key` and drives the participants.
pub struct TxnCoordinator {
    key: PrivateKey,
    pub_key: PublicKey,
}

impl TxnCoordinator {
    pub fn new(key: PrivateKey, pub_key: PublicKey) -> Self {
        TxnCoordinator { key, pub_key }
    }

    /// Sign, submit and await one phase on one participant shard.
    async fn run_phase(
        &self,
        service: &dyn DeployService,
        method: &str,
        txn_id: &[u8],
        shard_id: &str,
        data_args: &[Par],
        needs_deployer: bool,
    ) -> Result<ShardOutcome, String> {
        self.run_phase_at(
            service,
            method,
            txn_id,
            shard_id,
            data_args,
            needs_deployer,
            0,
        )
        .await
    }

    /// Sign, submit and await one phase, anchoring the deploy at the shard's height.
    ///
    /// `valid_after_block_number` must be the *target shard's* current height: a deploy anchored at
    /// 0 is born expired once that chain is more than `DEPLOY_LIFESPAN` blocks past genesis, and the
    /// participant would never see the phase at all.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_phase_at(
        &self,
        service: &dyn DeployService,
        method: &str,
        txn_id: &[u8],
        shard_id: &str,
        data_args: &[Par],
        needs_deployer: bool,
        valid_after_block_number: i64,
    ) -> Result<ShardOutcome, String> {
        let term = txn_term(method, txn_id, data_args, needs_deployer);
        let signed = signed_invoke(
            &term,
            &self.key,
            0,
            1_000_000,
            1,
            valid_after_block_number,
            shard_id,
        )?;
        let deploy = SignedDeployData {
            data: signed.data,
            deployer: signed.pk.bytes().to_vec(),
            sig: signed.sig.clone(),
            sig_algorithm: "secp256k1".to_string(),
        };
        service.deploy(&deploy).await.map_err(|e| e.join("; "))?;
        Ok(await_reply(
            service,
            &signed.sig,
            Duration::from_millis(250),
            Duration::from_secs(30),
        )
        .await)
    }

    /// Drive the full 2PC over `legs`: prepare every leg, collect the votes, then commit (if every
    /// participant voted ready) or abort. Returns the phase-two outcome per leg.
    pub async fn run_2pc(
        &self,
        service: &dyn DeployService,
        txn_id: &[u8],
        legs: &[TxnLeg],
    ) -> Result<Vec<ShardOutcome>, String> {
        let mut votes = Vec::with_capacity(legs.len());
        for leg in legs {
            let outcome = self
                .run_phase(
                    service,
                    "prepare",
                    txn_id,
                    &leg.shard_id,
                    &[
                        RhoByteArray::apply(self.pub_key.bytes().to_vec()),
                        RhoNumber::apply(leg.amount),
                        RhoString::apply(leg.to.clone()),
                    ],
                    true,
                )
                .await?;
            votes.push(outcome);
        }

        let ready: Vec<bool> = votes.iter().map(vote_from_reply).collect();
        let all_ready = ready.iter().all(|&b| b);
        let decision = if all_ready { "commit" } else { "abort" };

        // Phase 2 applies only to the legs that actually prepared (voted ready): a leg that voted
        // abort never locked resources, so it has nothing to commit or compensate.
        let mut outcomes = Vec::with_capacity(legs.len());
        for (i, leg) in legs.iter().enumerate() {
            if ready[i] {
                outcomes.push(
                    self.run_phase(service, decision, txn_id, &leg.shard_id, &[], true)
                        .await?,
                );
            } else {
                outcomes.push(ShardOutcome::Error("not prepared".to_string()));
            }
        }
        Ok(outcomes)
    }
}

/// Whether a participant's phase-one reply counts as a *ready* vote.
///
/// The participant is idempotent under `txn_id`, so a re-run can be answered with an
/// **already-terminal** state instead of `ready`:
///
/// * `committed` means the leg is applied and the transaction is committed — counting it as ready
///   keeps the decision uniform. Reading it as "not ready" would abort the other legs and leave one
///   shard committed and another aborted, breaking Law 27 on exactly the retry path that recovery
///   makes reachable;
/// * `prepared` means an earlier attempt locked the escrow — also ready;
/// * anything else (`abort`, `aborted`, an unexpected value, a timeout) is an abort vote.
pub fn vote_from_reply(outcome: &ShardOutcome) -> bool {
    match outcome {
        ShardOutcome::Value(p) => matches!(
            RhoString::unapply(p),
            Some("ready") | Some("prepared") | Some("committed")
        ),
        ShardOutcome::Error(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The generated `rho:txn` terms must parse as real rholang (catches template syntax drift).
    #[test]
    fn txn_term_parses() {
        for (method, needs_deployer) in [
            ("prepare", true),
            ("commit", true),
            ("abort", true),
            ("recover", false),
        ] {
            let term = txn_term(method, &[1u8, 2, 3], &[], needs_deployer);
            rchain_rholang::normalizer::source_to_adt(&term)
                .unwrap_or_else(|e| panic!("{method} term must parse: {e}\n{term}"));
        }
    }

    /// `prepare` carries the coordinator key, amount and destination; `commit`/`abort` carry the
    /// caller's deployer id; all phases reply on `rho:rchain:deployId`.
    #[test]
    fn txn_term_structure() {
        let term = txn_term(
            "prepare",
            &[9u8],
            &[
                RhoByteArray::apply(vec![4u8; 65]),
                RhoNumber::apply(100),
                RhoString::apply("dest".to_string()),
            ],
            true,
        );
        assert!(term.contains("rho:txn"), "{term}");
        assert!(term.contains("\"prepare\""), "{term}");
        assert!(term.contains("rho:rchain:deployerId"), "{term}");
        assert!(term.contains("*deployerId"), "{term}");
        assert!(term.contains("rho:rchain:deployId"), "{term}");
    }

    /// A byte array is the one argument the pretty printer would render into something that is not a
    /// rholang literal (a bare hex string), so it is special-cased.
    #[test]
    fn render_arg_renders_a_byte_array_as_hex_to_bytes() {
        let rendered = render_arg(&RhoByteArray::apply(vec![0xAB, 0xCD]));
        assert_eq!(rendered, "\"abcd\".hexToBytes()");
    }

    #[test]
    fn render_arg_falls_back_to_the_pretty_printer() {
        // A string stays a string literal; the pretty printer owns the rendering.
        assert!(render_arg(&RhoString::apply("dest".to_string())).contains("dest"));
        assert_eq!(render_arg(&RhoNumber::apply(40)), "40");
    }

    /// The reply mapping is the fix for the Law 27 retry bug: a participant that is already
    /// *committed* says so instead of `ready`, and reading that as "not ready" would abort the other
    /// legs. Every arm is pinned here, including the `prepared` one no test reached before.
    #[test]
    fn vote_from_reply_maps_every_reply() {
        let value = |s: &str| ShardOutcome::Value(RhoString::apply(s.to_string()));
        for yes in ["ready", "prepared", "committed"] {
            assert!(vote_from_reply(&value(yes)), "{yes} is a ready vote");
        }
        for no in ["abort", "aborted", "unexpected", ""] {
            assert!(!vote_from_reply(&value(no)), "{no} is an abort vote");
        }
        assert!(!vote_from_reply(&ShardOutcome::Value(RhoNumber::apply(1))));
        assert!(!vote_from_reply(&ShardOutcome::Error(
            "timeout".to_string()
        )));
    }

    /// A `DeployService` that records the deploys it is handed and answers each reply immediately
    /// with a canned value — enough to pin what the coordinator sends *and* how it decides, without
    /// waiting out `await_reply`'s 30-second timeout. (The timeout path itself is pinned by the
    /// gateway's `a_leg_whose_reply_never_arrives_times_out`, which has a 50 ms phase timeout.)
    struct RecordingService {
        deploys: std::sync::Mutex<Vec<SignedDeployData>>,
        /// The reply each listen yields; `None` means no reply ever appears.
        reply: Option<Par>,
    }

    impl RecordingService {
        fn replying(reply: &str) -> Self {
            RecordingService {
                deploys: std::sync::Mutex::new(Vec::new()),
                reply: Some(RhoString::apply(reply.to_string())),
            }
        }
    }

    impl RecordingService {
        fn anchors(&self) -> Vec<i64> {
            self.deploys
                .lock()
                .unwrap()
                .iter()
                .map(|d| d.data.valid_after_block_number)
                .collect()
        }

        fn terms(&self) -> Vec<String> {
            self.deploys
                .lock()
                .unwrap()
                .iter()
                .map(|d| d.data.term.clone())
                .collect()
        }
    }

    #[async_trait::async_trait]
    impl DeployService for RecordingService {
        async fn deploy(&self, d: &SignedDeployData) -> Result<String, Vec<String>> {
            self.deploys.lock().unwrap().push(d.clone());
            Ok(base16::encode(&d.sig))
        }

        async fn listen_for_data_at_name(
            &self,
            _: &rchain_models::casper::protocol::deploy_service::DataAtNameQuery,
        ) -> Result<
            Vec<rchain_models::casper::protocol::deploy_service::DataWithBlockInfo>,
            Vec<String>,
        > {
            use rchain_models::casper::protocol::deploy_service::{
                DataWithBlockInfo, LightBlockInfo,
            };
            let post_block_data = match &self.reply {
                Some(par) => vec![par.clone()],
                None => Vec::new(),
            };
            Ok(vec![DataWithBlockInfo {
                post_block_data,
                block: LightBlockInfo {
                    version: 1,
                    shard_id: "/root".to_string(),
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
                },
            }])
        }

        async fn deploy_status(
            &self,
            _: &rchain_models::casper::protocol::deploy_service::FindDeployQuery,
        ) -> Result<rchain_models::casper::protocol::deploy_service::DeployExecStatus, Vec<String>>
        {
            Err(vec!["not used".to_string()])
        }
        async fn get_block(
            &self,
            _: &rchain_models::casper::protocol::deploy_service::BlockQuery,
        ) -> Result<String, Vec<String>> {
            Err(vec!["not used".to_string()])
        }
        async fn get_blocks(
            &self,
            _: &rchain_models::casper::protocol::deploy_service::BlocksQuery,
        ) -> Result<String, Vec<String>> {
            Err(vec!["not used".to_string()])
        }
        async fn visualize_dag(
            &self,
            _: &rchain_models::casper::protocol::deploy_service::VisualizeDagQuery,
        ) -> Result<String, Vec<String>> {
            Err(vec!["not used".to_string()])
        }
        async fn machine_verifiable_dag(
            &self,
            _: &rchain_models::casper::protocol::deploy_service::MachineVerifyQuery,
        ) -> Result<String, Vec<String>> {
            Err(vec!["not used".to_string()])
        }
        async fn find_deploy(
            &self,
            _: &rchain_models::casper::protocol::deploy_service::FindDeployQuery,
        ) -> Result<String, Vec<String>> {
            Err(vec!["not used".to_string()])
        }
        async fn listen_for_continuation_at_name(
            &self,
            _: &rchain_models::casper::protocol::deploy_service::ContinuationAtNameQuery,
        ) -> Result<
            Vec<rchain_models::casper::protocol::deploy_service::ContinuationsWithBlockInfo>,
            Vec<String>,
        > {
            Err(vec!["not used".to_string()])
        }
        async fn last_finalized_block(&self) -> Result<String, Vec<String>> {
            Err(vec!["not used".to_string()])
        }
        async fn is_finalized(
            &self,
            _: &rchain_models::casper::protocol::deploy_service::IsFinalizedQuery,
        ) -> Result<String, Vec<String>> {
            Err(vec!["not used".to_string()])
        }
        async fn bond_status(
            &self,
            _: &rchain_models::casper::protocol::deploy_service::BondStatusQuery,
        ) -> Result<String, Vec<String>> {
            Err(vec!["not used".to_string()])
        }
        async fn status(&self) -> Result<String, Vec<String>> {
            Err(vec!["not used".to_string()])
        }
    }

    fn coordinator() -> TxnCoordinator {
        let (key, pub_key) = crate::construct_deploy::default_key_pair().unwrap();
        TxnCoordinator::new(key, pub_key)
    }

    /// `run_phase_at` anchors the deploy at the height it is given — the parameter the generic
    /// `run_phase` hardcoded to 0, which made a phase deploy born expired on a chain past
    /// `DEPLOY_LIFESPAN`.
    #[tokio::test]
    async fn run_phase_at_anchors_the_deploy_at_the_given_height() {
        let service = RecordingService::replying("ready");
        coordinator()
            .run_phase_at(&service, "prepare", b"txn", "/root", &[], true, 42)
            .await
            .expect("phase");

        assert_eq!(service.anchors(), vec![42]);
        let terms = service.terms();
        assert_eq!(terms.len(), 1);
        assert!(terms[0].contains("prepare"), "{}", terms[0]);
    }

    /// `run_phase` is the 0-anchored wrapper — kept for the client path, pinned so the wrapper and the
    /// parameterised form cannot drift.
    #[tokio::test]
    async fn run_phase_anchors_at_zero() {
        let service = RecordingService::replying("ready");
        coordinator()
            .run_phase(&service, "prepare", b"txn", "/root", &[], true)
            .await
            .expect("phase");
        assert_eq!(service.anchors(), vec![0]);
    }

    /// Both legs reply `ready`, so the coordinator commits every one of them and sends phase two to
    /// each: two prepares then two commits, and no leg is left un-decided. (The abort-on-a-failed-leg
    /// path is covered end-to-end in `casper/tests/cross_shard_txn.rs`.)
    #[tokio::test]
    async fn run_2pc_commits_every_leg_when_all_reply_ready() {
        let service = RecordingService::replying("ready");
        let legs = [
            TxnLeg {
                shard_id: "/root".to_string(),
                amount: 30,
                to: "dest".to_string(),
            },
            TxnLeg {
                shard_id: "/root/child".to_string(),
                amount: 40,
                to: "dest".to_string(),
            },
        ];
        let outcomes = coordinator()
            .run_2pc(&service, b"txn", &legs)
            .await
            .expect("run_2pc");

        assert_eq!(outcomes.len(), 2);
        for outcome in &outcomes {
            match outcome {
                ShardOutcome::Value(p) => assert_eq!(RhoString::unapply(p), Some("ready")),
                other => panic!("expected the ready reply, got {other:?}"),
            }
        }
        let terms = service.terms();
        assert_eq!(terms.len(), 4, "two prepares then two commits: {terms:?}");
        assert!(terms[0].contains("prepare") && terms[1].contains("prepare"));
        assert!(terms[2].contains("commit") && terms[3].contains("commit"));
    }
}
