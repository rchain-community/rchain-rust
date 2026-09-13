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
        let term = txn_term(method, txn_id, data_args, needs_deployer);
        let signed = signed_invoke(&term, &self.key, 0, 1_000_000, 1, 0, shard_id)?;
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
    /// participant voted `ready`) or abort. Returns the phase-two outcome per leg.
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

        let ready: Vec<bool> = votes
            .iter()
            .map(|v| matches!(v, ShardOutcome::Value(p) if RhoString::unapply(p) == Some("ready")))
            .collect();
        let all_ready = ready.iter().all(|&b| b);
        let decision = if all_ready { "commit" } else { "abort" };

        // Phase 2 applies only to the legs that actually prepared (voted `ready`): a leg that voted
        // `abort` never locked resources, so it has nothing to commit or compensate.
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
}
