//! Deploy construction helpers (port of `util/ConstructDeploy.scala`).

use rchain_crypto::private_key::PrivateKey;
use rchain_crypto::public_key::PublicKey;
use rchain_crypto::signatures::secp256k1::Secp256k1;
use rchain_crypto::signatures::signatures_alg::SignaturesAlg;
use rchain_crypto::signatures::signed::Signed;
use rchain_models::casper::protocol::casper_message::DeployData;
use rchain_shared::base16;

/// The default private key (port of `defaultSec`).
pub fn default_sec() -> PrivateKey {
    PrivateKey::new(base16::unsafe_decode(
        "a68a6e6cca30f81bd24a719f3145d20e8424bd7b396309b0708a16c7d8000b76",
    ))
}

/// The default public key (port of `defaultPub`).
pub fn default_pub() -> Result<PublicKey, String> {
    let sec = default_sec();
    Secp256k1
        .to_public(&sec)
        .map_err(|e| format!("derive default public key: {e}"))
}

/// The default (private, public) key pair (port of `defaultKeyPair`).
pub fn default_key_pair() -> Result<(PrivateKey, PublicKey), String> {
    let sec = default_sec();
    let pub_key = Secp256k1
        .to_public(&sec)
        .map_err(|e| format!("derive default public key: {e}"))?;
    Ok((sec, pub_key))
}

/// A second default private key (port of `defaultSec2`).
pub fn default_sec2() -> PrivateKey {
    PrivateKey::new(base16::unsafe_decode(
        "5a0bde2f5857124b1379c78535b07a278e3b9cefbcacc02e62ab3294c02765a1",
    ))
}

/// A second default public key (port of `defaultPub2`).
pub fn default_pub2() -> Result<PublicKey, String> {
    let sec = default_sec2();
    Secp256k1
        .to_public(&sec)
        .map_err(|e| format!("derive default public key: {e}"))
}

/// Build a signed deploy from source + parameters (port of `sourceDeploy`).
pub fn source_deploy(
    source: &str,
    timestamp: i64,
    phlo_limit: i64,
    phlo_price: i64,
    sec: &PrivateKey,
    vabn: i64,
    shard_id: &str,
) -> Result<Signed<DeployData>, String> {
    let data = DeployData {
        term: source.to_string(),
        timestamp,
        phlo_price,
        phlo_limit,
        valid_after_block_number: vabn,
        shard_id: shard_id.to_string(),
    };
    Signed::new(data, &Secp256k1, sec).map_err(|e| format!("sign deploy: {e}"))
}

/// Build a signed deploy with the current timestamp (port of `sourceDeployNow`).
pub fn source_deploy_now(
    source: &str,
    sec: &PrivateKey,
    phlo_limit: i64,
    vabn: i64,
    shard_id: &str,
) -> Result<Signed<DeployData>, String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    source_deploy(source, now, phlo_limit, 1, sec, vabn, shard_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rchain_shared::serialize::Serialize;

    #[test]
    fn default_pub_matches_default_sec() {
        let (sec, pub_key) = default_key_pair().unwrap();
        assert_eq!(Secp256k1.to_public(&sec).unwrap(), pub_key);
    }

    #[test]
    fn source_deploy_signs_with_default_key() {
        let sec = default_sec();
        let deploy = source_deploy("Nil", 0, 90000, 1, &sec, 0, "root").unwrap();
        assert_eq!(deploy.data.term, "Nil");
        assert_eq!(deploy.pk, default_pub().unwrap());
        // The signature verifies over the serialized deploy data.
        let serialized = <DeployData as Serialize<DeployData>>::encode(&deploy.data);
        let hash = rchain_crypto::signatures::signed::signature_hash("secp256k1", &serialized);
        assert!(Secp256k1.verify(&hash, &deploy.sig, deploy.pk.bytes()));
    }
}

#[cfg(test)]
mod builder_tests {
    use super::*;

    /// The second throwaway key pair is **distinct** from the first (it is the "other validator" in
    /// every test that needs two identities) and each is internally consistent.
    #[test]
    fn the_two_default_key_pairs_are_distinct() {
        let (sec, pub_key) = default_key_pair().expect("pair");
        let (sec2, pub2) = (default_sec2(), default_pub2().expect("pub2"));
        assert_eq!(Secp256k1.to_public(&sec).expect("pub"), pub_key);
        assert_eq!(Secp256k1.to_public(&sec2).expect("pub2"), pub2);
        assert_ne!(sec.bytes(), sec2.bytes());
        assert_ne!(pub_key.bytes(), pub2.bytes());
        assert_eq!(
            default_sec().bytes(),
            sec.bytes(),
            "the builders are stable"
        );
        assert_eq!(default_pub().expect("pub").bytes(), pub_key.bytes());
    }

    /// `source_deploy` signs the **exact** deploy data the caller supplied: the term, the timestamp,
    /// the phlo fields, the `valid_after_block_number` and the shard id all reach the signed payload,
    /// and the signature verifies against the deployer's key. A builder that dropped a field would
    /// produce a deploy the node accepts but the caller did not ask for.
    #[test]
    fn a_source_deploy_carries_every_field_it_was_given() {
        let (sec, pub_key) = default_key_pair().expect("pair");
        let signed =
            source_deploy("@\"out\"!(1)", 1234, 500_000, 1, &sec, 7, "/root/child").expect("sign");

        assert_eq!(signed.data.term, "@\"out\"!(1)");
        assert_eq!(signed.data.timestamp, 1234);
        assert_eq!(signed.data.phlo_limit, 500_000);
        assert_eq!(signed.data.phlo_price, 1);
        assert_eq!(signed.data.valid_after_block_number, 7);
        assert_eq!(signed.data.shard_id, "/root/child");
        assert_eq!(
            signed.pk.bytes(),
            pub_key.bytes(),
            "signed by the key that was passed"
        );

        // The signature covers **every** field and the deployer's key: verifying the written payload
        // against the written key succeeds, and a re-signed deploy with any field changed produces a
        // *different* signature (so no field is outside what was signed).
        let verify = |signed: &rchain_crypto::signatures::signed::Signed<
            rchain_models::casper::protocol::casper_message::DeployData,
        >| {
            let serialized =
                <rchain_models::casper::protocol::casper_message::DeployData as
                    rchain_shared::serialize::Serialize<
                        rchain_models::casper::protocol::casper_message::DeployData,
                    >>::encode(&signed.data);
            let hash = rchain_crypto::signatures::signed::signature_hash(
                signed.sig_algorithm.name(),
                &serialized,
            );
            Secp256k1.verify(&hash, &signed.sig, signed.pk.bytes())
        };
        assert!(
            verify(&signed),
            "the signature covers the payload the builder wrote"
        );

        let renamed =
            source_deploy("@\"out\"!(2)", 1234, 500_000, 1, &sec, 7, "/root/child").expect("sign");
        assert_ne!(
            signed.sig, renamed.sig,
            "a changed term changes the signature: the term is inside what was signed"
        );
        let revalidated =
            source_deploy("@\"out\"!(1)", 1234, 500_000, 1, &sec, 8, "/root/child").expect("sign");
        assert_ne!(
            signed.sig, revalidated.sig,
            "valid_after_block_number is inside what was signed"
        );
        let resharded =
            source_deploy("@\"out\"!(1)", 1234, 500_000, 1, &sec, 7, "/other").expect("sign");
        assert_ne!(signed.sig, resharded.sig, "the shard id is signed");
        let repriced =
            source_deploy("@\"out\"!(1)", 1234, 500_000, 2, &sec, 7, "/root/child").expect("sign");
        assert_ne!(signed.sig, repriced.sig, "the phlo price is signed");
        let retimed =
            source_deploy("@\"out\"!(1)", 9999, 500_000, 1, &sec, 7, "/root/child").expect("sign");
        assert_ne!(signed.sig, retimed.sig, "the timestamp is signed");
    }

    /// `source_deploy_now` stamps a **wall-clock** timestamp (milliseconds since the epoch) and a
    /// phlo price of 1 — the shape every test that deploys through the real gRPC path relies on.
    #[test]
    fn a_source_deploy_now_stamps_the_current_time() {
        let before = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_millis() as i64;
        let (sec, _) = default_key_pair().expect("pair");
        let signed = source_deploy_now("Nil", &sec, 1000, 0, "root").expect("sign");
        let after = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_millis() as i64;

        assert!(
            signed.data.timestamp >= before && signed.data.timestamp <= after,
            "the timestamp {} is not between {before} and {after}",
            signed.data.timestamp
        );
        assert_eq!(signed.data.phlo_price, 1, "the default price");
        assert_eq!(signed.data.phlo_limit, 1000);
        assert_eq!(signed.data.shard_id, "root");
    }
}
