//! Devnet faucet: sign a REV transfer from the funded deployer wallet to a caller's address.
//!
//! Dev/testnet-only. The transfer is an ordinary signed rholang deploy calling the **native**
//! `rho:rchain:revVault` system process (`rholang/src/system_processes.rs`), whose `transfer` derives
//! the source vault from the caller's unforgeable `deployerId`. It is only reachable when the node
//! runs with `--dev-mode --deployer-private-key`.

use std::time::{SystemTime, UNIX_EPOCH};

use rchain_crypto::private_key::PrivateKey;
use rchain_crypto::public_key::PublicKey;
use rchain_crypto::signatures::secp256k1::Secp256k1;
use rchain_crypto::signatures::signed::Signed;
use rchain_models::casper::protocol::casper_message::{DeployData, SignedDeployData};
use rchain_rholang::util::rev_address::RevAddress;

/// One faucet drip, in the smallest REV unit ("drops"). `0.3 REV = 30_000_000` drops
/// (1 REV = 10^8 drops).
pub const FAUCET_AMOUNT: i64 = 30_000_000;

/// Phlo budget for a faucet transfer deploy (matches the devnet `deploy` helper).
const FAUCET_PHLO_LIMIT: i64 = 1_000_000;

/// The native `revVault` transfer term: `transfer(*deployerId, to, amount, ret)` derives the `from`
/// vault from the caller's unforgeable `deployerId` and creates the `to` vault implicitly
/// (`rholang/src/system_processes.rs:1383-1422`). `__TO__` (REV address string) and `__AMOUNT__`
/// (integer drops) are substituted. Kept as a `.replace` template rather than a `format!` so the
/// rholang `{ … }` blocks don't collide with `format!` braces.
const TRANSFER_TEMPLATE: &str = r#"new revVault(`rho:rchain:revVault`), deployerId(`rho:rchain:deployerId`), resultCh in {
  revVault!("transfer", *deployerId, "__TO__", __AMOUNT__, *resultCh) |
  for (_ <- resultCh) { Nil }
}"#;

/// Render the native `revVault` transfer term that moves `amount` drops to `to`.
pub fn build_transfer_term(to: &str, amount: i64) -> String {
    TRANSFER_TEMPLATE
        .replace("__TO__", to)
        .replace("__AMOUNT__", &amount.to_string())
}

/// Derive the deployer's REV address from its private key (secp256k1 pubkey → REV address).
pub fn deployer_rev_address(sk: &PrivateKey) -> Result<String, String> {
    let pk_bytes = Secp256k1::to_public_bytes(sk.bytes()).map_err(|e| e.to_string())?;
    let pk = PublicKey::new(pk_bytes);
    RevAddress::from_public_key(&pk)
        .map(|a| a.to_base58())
        .ok_or_else(|| "failed to derive REV address from deployer key".to_string())
}

/// Build and sign a faucet transfer deploy to `to` (port of the devnet `deploy` path; the signing
/// pattern mirrors `casper/src/blocks/proposer/proposer.rs`'s dummy deploy).
///
/// `valid_after_block_number` must be the current chain height (not `-1`): a deploy is expired once
/// `latest_block_number - valid_after_block_number > DEPLOY_LIFESPAN` (50), so `-1` is dropped from
/// the pool as soon as the node passes block 49.
pub fn sign_faucet_deploy(
    sk: &PrivateKey,
    to: &str,
    amount: i64,
    shard_id: &str,
    valid_after_block_number: i64,
) -> Result<SignedDeployData, String> {
    let data = DeployData {
        term: build_transfer_term(to, amount),
        timestamp: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0),
        phlo_price: 1,
        phlo_limit: FAUCET_PHLO_LIMIT,
        valid_after_block_number,
        shard_id: shard_id.to_string(),
    };
    // `Signed` holds a `&'static dyn SignaturesAlg` (not `Sync`); convert it into an owned
    // `SignedDeployData` in this block before any `.await`.
    let signed = Signed::new(data, &Secp256k1, sk).map_err(|e| e.to_string())?;
    Ok(SignedDeployData {
        data: signed.data,
        deployer: signed.pk.bytes().to_vec(),
        sig: signed.sig,
        sig_algorithm: signed.sig_algorithm.name().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_term_embeds_address_and_amount() {
        let term = build_transfer_term("toAddr", 30_000_000);
        assert!(term.contains("revVault!(\"transfer\""));
        assert!(term.contains("\"toAddr\""));
        assert!(term.contains("30000000"));
        assert!(!term.contains("__TO__") && !term.contains("__AMOUNT__"));
        // The native transfer derives `from` from the deployerId; there must be no string-address
        // `findOrCreate`/`deployerAuthKey` Scala-API remnants.
        assert!(!term.contains("findOrCreate") && !term.contains("deployerAuthKey"));
    }

    #[test]
    fn deployer_rev_address_derives_from_key() {
        // The devnet deployer key; the derived address must match the genesis wallet address.
        let sk = PrivateKey::new(
            rchain_shared::base16::decode(
                "a68a6e6cca30f81bd24a719f3145d20e8424bd7b396309b0708a16c7d8000b76",
            )
            .unwrap(),
        );
        let addr = deployer_rev_address(&sk).unwrap();
        assert_eq!(
            addr,
            "11112VYAt8rUGNRRZX3eJdgagaAhtWTK8Js7F7X5iqddMVqyDTtYau"
        );
    }
}
