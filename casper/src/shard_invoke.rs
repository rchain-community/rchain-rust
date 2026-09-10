//! Cross-shard invoke as a remote signed deploy (issue #33, Layer 1).
//!
//! A cross-shard invoke is **not** a relay or a bespoke envelope protocol: it is an
//! ordinary, caller-signed deploy submitted to the target shard. Identity is already
//! the same account everywhere — the far node binds `rho:rchain:deployerId` from the
//! deploy signature, which is the caller's own key — so a `deployerId`-gated
//! capability on the far shard sees exactly the caller it would locally.
//!
//! This module is the client-side primitive. Because only the keyholder can sign,
//! it is a client helper (the node performs no server-side invoke):
//!
//! * [`invoke_term`] builds the term that runs on the target shard. The reply is
//!   written to `` `rho:rchain:deployId` `` — the only channel a far deploy can
//!   always reach and whose contents the deploy service returns.
//! * [`signed_invoke`] signs that term with the caller's key.
//! * [`outcome`] maps the far shard's [`DeployExecStatus`] to the caller-visible
//!   value, turning every failure into `("shard-error", reason)` — never a hang.
//!
//! See `docs/src/node/shard-invoke.md` for the design and its consequences
//! (client-orchestrated, non-atomic bilateral exchange; Layer 2 is out of scope).

use rchain_crypto::private_key::PrivateKey;
use rchain_crypto::signatures::signed::Signed;
use rchain_models::ast::Par;
use rchain_models::casper::protocol::casper_message::DeployData;
use rchain_models::casper::protocol::deploy_service::DeployExecStatus;
use rchain_models::rholang::RhoType::{RhoString, RhoTupleN};
use rchain_rholang::pretty_printer::PrettyPrinter;

use crate::construct_deploy;

/// The unforgeable reply channel a remote deploy writes its result to. Bound by the
/// far node's normalizer environment from the deploy signature.
pub const REMOTE_REPLY_CHANNEL: &str = "rho:rchain:deployId";

/// The native registry-lookup system process used to resolve the target capability.
pub const REGISTRY_LOOKUP: &str = "rho:registry:lookup";

/// The tag of the failure-as-value tuple: `("shard-error", reason)`.
pub const SHARD_ERROR_TAG: &str = "shard-error";

/// Build the term run on the target shard for `$at(shard, targetUri)!(method, args…)`.
///
/// `args` are already-normalized rholang `Par`s; they are rendered with the pretty
/// printer so names/data keep their rholang literal form. The invoked capability's
/// reply is sent to `` `rho:rchain:deployId` `` and read back by the caller from the
/// deploy result.
///
/// A registry miss yields `Nil`; the `for` then does not fire and the deploy produces
/// nothing, which [`outcome`] reports as a `shard-error`.
pub fn invoke_term(target_uri: &str, method: &str, args: &[Par]) -> String {
    let pp = PrettyPrinter::new();
    let uri_lit = pp.build_string(&RhoString::apply(target_uri.to_string()));
    let method_lit = pp.build_string(&RhoString::apply(method.to_string()));

    let mut payload: Vec<String> = Vec::with_capacity(args.len() + 1);
    payload.push(method_lit);
    payload.extend(args.iter().map(|a| pp.build_string(a)));
    let payload = payload.join(", ");

    format!(
        "new lookup(`{REGISTRY_LOOKUP}`), cap in {{ \
           lookup!({uri_lit}, *cap) | \
           for (@(_, target) <- cap) {{ \
             @target!({payload}, `{REMOTE_REPLY_CHANNEL}`) \
           }} \
         }}"
    )
}

/// Sign the far-shard term with the caller's key (port of `deployFileProgram`'s
/// signing), producing the deploy to submit to the target shard's deploy service.
///
/// `shard_id` must be the target shard's id (the far node rejects a mismatched
/// `DeployData.shardId`). `deployerId` on the far shard is this key's public key.
pub fn signed_invoke(
    term: &str,
    caller_key: &PrivateKey,
    timestamp: i64,
    phlo_limit: i64,
    phlo_price: i64,
    valid_after_block_number: i64,
    shard_id: &str,
) -> Result<Signed<DeployData>, String> {
    construct_deploy::source_deploy(
        term,
        timestamp,
        phlo_limit,
        phlo_price,
        caller_key,
        valid_after_block_number,
        shard_id,
    )
}

/// The caller-visible outcome of a remote invoke.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShardOutcome {
    /// The far shard committed and returned a reply.
    Value(Par),
    /// The invoke failed; the reason is rendered as `("shard-error", reason)` by
    /// [`ShardOutcome::into_value`].
    Error(String),
    /// The far shard has not committed the deploy yet — poll again.
    Pending(String),
}

impl ShardOutcome {
    /// The rholang value the caller's `for` observes: the reply, or
    /// `("shard-error", reason)`. A pending status is also surfaced as a value so an
    /// error is never confused with a hang.
    pub fn into_value(self) -> Par {
        match self {
            ShardOutcome::Value(v) => v,
            ShardOutcome::Error(reason) => shard_error(&reason),
            ShardOutcome::Pending(status) => shard_error(&format!("pending: {status}")),
        }
    }

    /// Whether the far shard has committed (successfully or not).
    pub fn is_committed(&self) -> bool {
        matches!(self, ShardOutcome::Value(_) | ShardOutcome::Error(_))
    }
}

/// `("shard-error", reason)` — an ordinary rholang tuple value, so failure composes
/// with local rholang instead of blocking.
pub fn shard_error(reason: &str) -> Par {
    RhoTupleN::apply(vec![
        RhoString::apply(SHARD_ERROR_TAG.to_string()),
        RhoString::apply(reason.to_string()),
    ])
}

/// Map a far-shard deploy status to the caller-visible outcome.
///
/// * `ProcessedWithSuccess` with a reply -> [`ShardOutcome::Value`].
/// * `ProcessedWithSuccess` with no reply (e.g. a registry miss, so the `for` never
///   fired) -> [`ShardOutcome::Error`].
/// * `ProcessedWithError` -> [`ShardOutcome::Error`] carrying the deploy error.
/// * `NotProcessed` -> [`ShardOutcome::Pending`].
pub fn outcome(status: &DeployExecStatus) -> ShardOutcome {
    match status {
        DeployExecStatus::ProcessedWithSuccess { deploy_result, .. } => match deploy_result
            .as_slice()
        {
            [] => ShardOutcome::Error("no reply produced on the deploy result channel".to_string()),
            [only] => ShardOutcome::Value(only.clone()),
            [first, ..] => ShardOutcome::Value(first.clone()),
        },
        DeployExecStatus::ProcessedWithError { deploy_error, .. } => {
            ShardOutcome::Error(deploy_error.clone())
        }
        DeployExecStatus::NotProcessed { status } => ShardOutcome::Pending(status.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use rchain_crypto::signatures::secp256k1::Secp256k1;
    use rchain_crypto::signatures::signatures_alg::SignaturesAlg;
    use rchain_crypto::signatures::signed::signature_hash;
    use rchain_models::casper::protocol::deploy_service::LightBlockInfo;
    use rchain_shared::serialize::Serialize;

    fn light_block() -> LightBlockInfo {
        LightBlockInfo {
            version: 1,
            shard_id: "root".to_string(),
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
        }
    }

    #[test]
    fn invoke_term_parses() {
        let term = invoke_term(
            "rho:id:theOracle",
            "getPrice",
            &[RhoString::apply("ETH".to_string())],
        );
        // Must parse + normalize as real rholang (catches syntax drift in the template).
        rchain_rholang::normalizer::source_to_adt(&term).expect("generated invoke term must parse");
    }

    #[test]
    fn invoke_term_targets_uri_method_and_deploy_id() {
        let term = invoke_term("rho:id:locker", "open", &[]);
        assert!(term.contains("rho:registry:lookup"), "{term}");
        assert!(term.contains("rho:id:locker"), "{term}");
        assert!(term.contains("\"open\""), "{term}");
        // The reply channel is the deploy id, never a caller-local name.
        assert!(term.contains(REMOTE_REPLY_CHANNEL), "{term}");
    }

    #[test]
    fn signed_invoke_is_signed_by_the_caller() {
        let caller = construct_deploy::default_sec();
        let caller_pub = Secp256k1.to_public(&caller).unwrap();

        let term = invoke_term("rho:id:x", "m", &[]);
        let deploy = signed_invoke(&term, &caller, 0, 90_000, 1, 0, "root").unwrap();

        // The far shard will bind this public key as `deployerId`.
        assert_eq!(deploy.pk, caller_pub);
        assert_eq!(deploy.data.term, term);
        assert_eq!(deploy.data.shard_id, "root");

        let serialized = <DeployData as Serialize<DeployData>>::encode(&deploy.data);
        let hash = signature_hash("secp256k1", &serialized);
        assert!(Secp256k1.verify(&hash, &deploy.sig, deploy.pk.bytes()));
    }

    #[test]
    fn outcome_maps_success_error_and_pending() {
        let success = DeployExecStatus::ProcessedWithSuccess {
            deploy_result: vec![RhoString::apply("42".to_string())],
            block: light_block(),
        };
        assert_eq!(
            outcome(&success),
            ShardOutcome::Value(RhoString::apply("42".to_string()))
        );
        assert!(outcome(&success).is_committed());

        let empty = DeployExecStatus::ProcessedWithSuccess {
            deploy_result: Vec::new(),
            block: light_block(),
        };
        assert!(matches!(outcome(&empty), ShardOutcome::Error(_)));

        let failed = DeployExecStatus::ProcessedWithError {
            deploy_error: "boom".to_string(),
            block: light_block(),
        };
        assert_eq!(outcome(&failed), ShardOutcome::Error("boom".to_string()));

        let pending = DeployExecStatus::NotProcessed {
            status: "Pooled".to_string(),
        };
        assert_eq!(
            outcome(&pending),
            ShardOutcome::Pending("Pooled".to_string())
        );
        assert!(!outcome(&pending).is_committed());
    }

    #[test]
    fn failure_is_a_shard_error_tuple() {
        let value = ShardOutcome::Error("no route".to_string()).into_value();
        let tuple = RhoTupleN::unapply(&value).expect("shard-error is a tuple");
        assert_eq!(tuple.len(), 2);
        assert_eq!(RhoString::unapply(&tuple[0]), Some(SHARD_ERROR_TAG));
        assert_eq!(RhoString::unapply(&tuple[1]), Some("no route"));
    }

    #[test]
    fn pending_is_surfaced_as_a_value_not_a_hang() {
        let value = ShardOutcome::Pending("Pooled".to_string()).into_value();
        let tuple = RhoTupleN::unapply(&value).expect("pending is a shard-error tuple");
        assert_eq!(RhoString::unapply(&tuple[0]), Some(SHARD_ERROR_TAG));
        assert_eq!(RhoString::unapply(&tuple[1]), Some("pending: Pooled"));
    }
}
