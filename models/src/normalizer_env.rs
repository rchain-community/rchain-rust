//! Deploy normalization environment (port of `models/NormalizerEnv.scala`).
//!
//! The Scala uses a shapeless record (`HList` keyed by singleton URI types) with `ToEnvMap` /
//! `Contains` typeclasses; the runtime value is just a `Map[String, Par]`, which the port models
//! directly as a `BTreeMap`.

use std::collections::BTreeMap;

use rchain_crypto::public_key::PublicKey;

use crate::ast::Par;
use crate::casper::protocol::casper_message::SignedDeployData;
use crate::rholang::RhoType::{RhoByteArray, RhoDeployId, RhoDeployerId};

/// The `rho:rchain:deployId` binding key.
pub const DEPLOY_ID_URI: &str = "rho:rchain:deployId";
/// The `rho:rchain:deployerId` binding key.
pub const DEPLOYER_ID_URI: &str = "rho:rchain:deployerId";
/// The `rho:attachment:N` binding prefix (RCHIP #39; `N` is 1-based).
pub const ATTACHMENT_URI_PREFIX: &str = "rho:attachment:";

/// The binding URI for the `index1`-th (1-based) deploy attachment, e.g. `rho:attachment:1`.
pub fn attachment_uri(index1: usize) -> String {
    format!("{ATTACHMENT_URI_PREFIX}{index1}")
}

/// The environment the normalizer resolves free URI names against (port of `NormalizerEnv`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NormalizerEnv {
    env: BTreeMap<String, Par>,
}

impl NormalizerEnv {
    /// The empty environment (port of `NormalizerEnv.Empty`).
    pub fn empty() -> Self {
        NormalizerEnv {
            env: BTreeMap::new(),
        }
    }

    /// An environment binding only the deployer id (port of `NormalizerEnv.withDeployerId`).
    pub fn with_deployer_id(deployer_pk: &PublicKey) -> Self {
        let mut env = BTreeMap::new();
        env.insert(
            DEPLOYER_ID_URI.to_string(),
            RhoDeployerId::apply(deployer_pk.bytes().to_vec()),
        );
        NormalizerEnv { env }
    }

    /// An environment binding the deploy id, deployer id and binary attachments (port of
    /// `NormalizerEnv.apply`, extended by RCHIP #39).
    pub fn new(deploy: &SignedDeployData) -> Self {
        let mut env = BTreeMap::new();
        env.insert(
            DEPLOY_ID_URI.to_string(),
            RhoDeployId::apply(deploy.sig.clone()),
        );
        env.insert(
            DEPLOYER_ID_URI.to_string(),
            RhoDeployerId::apply(deploy.deployer.clone()),
        );
        // RCHIP #39: the i-th attachment (1-based) is addressable as `` `rho:attachment:i` ``, which
        // evaluates to its bytes as a `ByteArray`.
        for (index, bytes) in deploy.data.attachments.iter().enumerate() {
            env.insert(
                attachment_uri(index + 1),
                RhoByteArray::apply(bytes.clone()),
            );
        }
        NormalizerEnv { env }
    }

    /// The URI → `Par` bindings (port of `NormalizerEnv.toEnv`).
    pub fn to_env(&self) -> &BTreeMap<String, Par> {
        &self.env
    }

    /// Look up a binding by URI (port of `NormalizerEnv.get`).
    pub fn get(&self, uri: &str) -> Option<&Par> {
        self.env.get(uri)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::casper::protocol::casper_message::DeployData;

    #[test]
    fn empty_has_no_bindings() {
        assert!(NormalizerEnv::empty().to_env().is_empty());
    }

    #[test]
    fn with_deployer_id_binds_deployer() {
        let pk = PublicKey::new(vec![1; 65]);
        let env = NormalizerEnv::with_deployer_id(&pk);
        let map = env.to_env();
        assert_eq!(map.len(), 1);
        let expected = RhoDeployerId::apply(vec![1; 65]);
        assert_eq!(map.get(DEPLOYER_ID_URI), Some(&expected));
    }

    #[test]
    fn apply_binds_deploy_and_deployer() {
        let deploy = SignedDeployData {
            data: DeployData {
                attachments: Vec::new(),
                term: "x".to_string(),
                timestamp: 0,
                phlo_price: 1,
                phlo_limit: 1,
                valid_after_block_number: 0,
                shard_id: "root".to_string(),
            },
            deployer: vec![2; 65],
            sig: vec![3; 64],
            sig_algorithm: "secp256k1".to_string(),
        };
        let env = NormalizerEnv::new(&deploy);
        let map = env.to_env();
        assert_eq!(map.len(), 2);

        let expected_id = RhoDeployId::apply(vec![3; 64]);
        assert_eq!(map.get(DEPLOY_ID_URI), Some(&expected_id));
        let expected_deployer = RhoDeployerId::apply(vec![2; 65]);
        assert_eq!(map.get(DEPLOYER_ID_URI), Some(&expected_deployer));

        assert_eq!(env.get(DEPLOYER_ID_URI), Some(&expected_deployer));
        assert_eq!(env.get("unknown"), None);
    }

    #[test]
    fn attachments_are_bound_as_byte_arrays() {
        // RCHIP #39: the i-th attachment (1-based) is addressable as `rho:attachment:i`.
        let deploy = SignedDeployData {
            data: DeployData {
                term: "Nil".to_string(),
                timestamp: 0,
                phlo_price: 1,
                phlo_limit: 1,
                valid_after_block_number: 0,
                shard_id: "root".to_string(),
                attachments: vec![vec![0xde, 0xad], vec![0xbe, 0xef, 0x00]],
            },
            deployer: vec![2; 65],
            sig: vec![3; 64],
            sig_algorithm: "secp256k1".to_string(),
        };
        let env = NormalizerEnv::new(&deploy);
        let map = env.to_env();
        // deployId + deployerId + 2 attachments.
        assert_eq!(map.len(), 4);

        assert_eq!(attachment_uri(1), "rho:attachment:1");
        assert_eq!(
            map.get(&attachment_uri(1)),
            Some(&RhoByteArray::apply(vec![0xde, 0xad]))
        );
        assert_eq!(
            map.get(&attachment_uri(2)),
            Some(&RhoByteArray::apply(vec![0xbe, 0xef, 0x00]))
        );
        // Beyond the attachments there is no binding.
        assert_eq!(map.get(&attachment_uri(3)), None);
    }
}
