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
//!   written to `` `rho:rchain:deployId` `` — the deploy's own id, which is the
//!   reply *channel* the caller listens on.
//! * [`signed_invoke`] signs that term with the caller's key.
//! * [`await_reply`] awaits that reply channel with the node's listen
//!   (`listenForDataAtName`), mapping the channel's value to the caller-visible
//!   result. It does **not** poll `deployStatus`: a reply is data on a channel, so
//!   the client listens for it.
//!
//! See `docs/src/node/shard-invoke.md` for the design and its consequences
//! (client-orchestrated, non-atomic bilateral exchange; Layer 2 is out of scope).

use std::time::Duration;

use rchain_crypto::private_key::PrivateKey;
use rchain_crypto::signatures::signed::Signed;
use rchain_models::ast::Par;
use rchain_models::casper::protocol::casper_message::DeployData;
use rchain_models::casper::protocol::deploy_service::{DataAtNameQuery, DataWithBlockInfo};
use rchain_models::rholang::RhoType::{RhoDeployId, RhoString, RhoTupleN};
use rchain_rholang::pretty_printer::PrettyPrinter;
use tokio::time::{sleep, Instant};

use crate::construct_deploy;
use crate::protocol::client::DeployService;

/// The unforgeable reply channel a remote deploy writes its result to. The far node
/// binds it from the deploy signature, so the caller can listen on it once it knows
/// the deploy id.
pub const REMOTE_REPLY_CHANNEL: &str = "rho:rchain:deployId";

/// The native registry-lookup system process used to resolve the target capability.
pub const REGISTRY_LOOKUP: &str = "rho:registry:lookup";

/// The tag of the failure-as-value tuple: `("shard-error", reason)`.
pub const SHARD_ERROR_TAG: &str = "shard-error";

/// Build the term run on the target shard for `$at(shard, targetUri)!(method, args…)`.
///
/// `args` are already-normalized rholang `Par`s; they are rendered with the pretty
/// printer so names/data keep their rholang literal form. The invoked capability's
/// reply is sent to `` `rho:rchain:deployId` `` — the reply channel the caller listens
/// on (see [`reply_channel`] / [`await_reply`]).
///
/// A registry miss yields `Nil`; the `for` then does not fire and the deploy produces
/// nothing on the reply channel, which [`await_reply`] reports as a `shard-error`.
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

/// The reply channel of a remote deploy: its own id (signature) as the unforgeable
/// name `` `rho:rchain:deployId` `` resolves to on the far shard.
pub fn reply_channel(deploy_id: &[u8]) -> Par {
    RhoDeployId::apply(deploy_id.to_vec())
}

/// The caller-visible outcome of a remote invoke.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShardOutcome {
    /// The reply channel produced a value.
    Value(Par),
    /// The invoke failed or the reply channel stayed empty; rendered as
    /// `("shard-error", reason)` by [`ShardOutcome::into_value`].
    Error(String),
}

impl ShardOutcome {
    /// The rholang value the caller's `for` observes: the reply, or
    /// `("shard-error", reason)`.
    pub fn into_value(self) -> Par {
        match self {
            ShardOutcome::Value(v) => v,
            ShardOutcome::Error(reason) => shard_error(&reason),
        }
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

/// The first value produced on the reply channel, from a listen result.
///
/// Data at the deploy's id channel *is* the reply; an empty result means the deploy
/// has not produced one.
pub fn reply_outcome(data: &[DataWithBlockInfo]) -> ShardOutcome {
    match data.iter().find_map(|d| d.post_block_data.first()) {
        Some(v) => ShardOutcome::Value(v.clone()),
        None => ShardOutcome::Error("no reply on the deploy reply channel".to_string()),
    }
}

/// Listen on a remote deploy's reply channel until the reply appears (or `timeout`
/// elapses), returning the value or a `shard-error`.
///
/// This is the channel-wait the primitive is built on — **no `deployStatus` polling**.
/// The node's `listenForDataAtName` is a one-shot query (the same shape as the Scala
/// oracle, whose client waits with `listenAtNameUntilChanges`), so the await
/// re-listens on the *channel* at `listen_interval` until the reply commits. Callers
/// that want the wait in the transport should use a streaming listen instead.
pub async fn await_reply(
    service: &dyn DeployService,
    deploy_id: &[u8],
    listen_interval: Duration,
    timeout: Duration,
) -> ShardOutcome {
    let query = DataAtNameQuery {
        depth: i32::MAX,
        name: reply_channel(deploy_id),
    };
    let interval = if listen_interval.is_zero() {
        Duration::from_millis(250)
    } else {
        listen_interval
    };
    let deadline = Instant::now() + timeout;
    loop {
        match service.listen_for_data_at_name(&query).await {
            Ok(data) => {
                if data.iter().any(|d| !d.post_block_data.is_empty()) {
                    return reply_outcome(&data);
                }
                if Instant::now() >= deadline {
                    return ShardOutcome::Error(format!(
                        "timed out listening on the reply channel of deploy {}",
                        rchain_shared::base16::encode(deploy_id)
                    ));
                }
                sleep(interval).await;
            }
            Err(errors) => return ShardOutcome::Error(errors.join("; ")),
        }
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

    fn data_with(value: Option<Par>) -> DataWithBlockInfo {
        DataWithBlockInfo {
            post_block_data: value.into_iter().collect(),
            block: light_block(),
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
    fn reply_channel_is_the_deploy_id() {
        let sig = vec![7u8; 64];
        let channel = reply_channel(&sig);
        assert_eq!(RhoDeployId::unapply(&channel), Some(sig.as_slice()));
    }

    #[test]
    fn reply_outcome_reads_the_channel_value() {
        let reply = RhoString::apply("42".to_string());
        assert_eq!(
            reply_outcome(&[data_with(Some(reply.clone()))]),
            ShardOutcome::Value(reply)
        );
        assert!(matches!(
            reply_outcome(&[data_with(None)]),
            ShardOutcome::Error(_)
        ));
        assert!(matches!(reply_outcome(&[]), ShardOutcome::Error(_)));
    }

    #[test]
    fn failure_is_a_shard_error_tuple() {
        let value = ShardOutcome::Error("no route".to_string()).into_value();
        let tuple = RhoTupleN::unapply(&value).expect("shard-error is a tuple");
        assert_eq!(tuple.len(), 2);
        assert_eq!(RhoString::unapply(&tuple[0]), Some(SHARD_ERROR_TAG));
        assert_eq!(RhoString::unapply(&tuple[1]), Some("no route"));
    }
}
