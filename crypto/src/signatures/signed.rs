//! A signed value.
//!
//! Mirrors `crypto/src/main/scala/coop/rchain/crypto/signatures/Signed.scala`.

use crate::errors::CryptoError;
use crate::hash::{blake2b256, keccak256};
use crate::private_key::PrivateKey;
use crate::public_key::PublicKey;
use crate::signatures::signatures_alg::SignaturesAlg;
use rchain_shared::serialize::Serialize;

/// A value `A` together with its signature and the signer's public key.
pub struct Signed<A> {
    pub data: A,
    pub pk: PublicKey,
    pub sig: Vec<u8>,
    pub sig_algorithm: &'static dyn SignaturesAlg,
}

impl<A: Serialize<A>> Signed<A> {
    /// Sign `data` with `sig_algorithm` and `sk`.
    pub fn new(
        data: A,
        sig_algorithm: &'static dyn SignaturesAlg,
        sk: &PrivateKey,
    ) -> Result<Self, CryptoError> {
        let serialized = <A as Serialize<A>>::encode(&data);
        let hash = signature_hash(sig_algorithm.name(), &serialized);
        let sig = sig_algorithm.sign(&hash, sk.bytes())?;
        let pk = sig_algorithm.to_public(sk)?;
        Ok(Self {
            data,
            pk,
            sig,
            sig_algorithm,
        })
    }

    /// Reconstruct a `Signed` from its parts, verifying the signature. Returns `None` on failure.
    pub fn from_signed_data(
        data: A,
        pk: PublicKey,
        sig: Vec<u8>,
        sig_algorithm: &'static dyn SignaturesAlg,
    ) -> Option<Self> {
        let serialized = <A as Serialize<A>>::encode(&data);
        let hash = signature_hash(sig_algorithm.name(), &serialized);
        if sig_algorithm.verify(&hash, &sig, pk.bytes()) {
            Some(Self {
                data,
                pk,
                sig,
                sig_algorithm,
            })
        } else {
            None
        }
    }
}

/// The hash that a signature is computed over, per algorithm.
pub fn signature_hash(sig_alg_name: &str, serialized_data: &[u8]) -> Vec<u8> {
    if sig_alg_name == "secp256k1:eth" {
        let mut prefix = eth_prefix(serialized_data.len());
        prefix.extend_from_slice(serialized_data);
        keccak256::hash(&prefix)
    } else {
        blake2b256::hash(serialized_data)
    }
}

fn eth_prefix(msg_length: usize) -> Vec<u8> {
    format!("\u{19}Ethereum Signed Message:\n{msg_length}").into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signatures::signatures_alg::from_algorithm;

    fn secp() -> &'static dyn SignaturesAlg {
        from_algorithm("secp256k1").expect("registered")
    }
    fn eth() -> &'static dyn SignaturesAlg {
        from_algorithm("secp256k1:eth").expect("registered")
    }

    /// The hash a signature is computed over is per algorithm: BLAKE2b-256 for `secp256k1`, and for
    /// `secp256k1:eth` the keccak of the **EIP-191 personal-message prefix** followed by the data.
    /// The prefix embeds the *decimal length*, so the same bytes at a different length hash
    /// differently — the property the prefix exists for.
    #[test]
    fn the_signed_hash_is_blake2_for_secp_and_keccak_with_the_eip191_prefix_for_eth() {
        let data = b"hello";
        assert_eq!(
            signature_hash("secp256k1", data),
            crate::hash::blake2b256::hash(data)
        );
        assert_eq!(
            signature_hash("Secp256k1", data),
            crate::hash::blake2b256::hash(data),
            "any name other than the eth one takes the BLAKE2b path"
        );

        let mut prefixed = b"\x19Ethereum Signed Message:\n5".to_vec();
        prefixed.extend_from_slice(data);
        assert_eq!(
            signature_hash("secp256k1:eth", data),
            crate::hash::keccak256::hash(&prefixed)
        );

        // The length in the prefix is the data's length, and it is what makes the eth hash
        // length-sensitive beyond the data itself.
        assert_ne!(
            signature_hash("secp256k1:eth", b"hello"),
            signature_hash("secp256k1:eth", b"helloo")
        );
        let mut prefixed_ten = b"\x19Ethereum Signed Message:\n10".to_vec();
        prefixed_ten.extend_from_slice(b"helloworld");
        assert_eq!(
            signature_hash("secp256k1:eth", b"helloworld"),
            crate::hash::keccak256::hash(&prefixed_ten)
        );

        // The prefix itself: 0x19, then the ASCII phrase, then the decimal length, then the data.
        assert_eq!(eth_prefix(5), b"\x19Ethereum Signed Message:\n5".to_vec());
        assert_eq!(eth_prefix(0), b"\x19Ethereum Signed Message:\n0".to_vec());
        assert_eq!(
            eth_prefix(100).len(),
            b"\x19Ethereum Signed Message:\n".len() + 3
        );
    }

    /// A `Signed` value round-trips: signing and then reconstructing from the parts verifies, and
    /// the signature is over the *serialized* data with the algorithm's hash.
    #[test]
    fn a_signed_value_round_trips_and_carries_its_algorithm() {
        let (sk, _pk) = secp().new_key_pair();
        let signed = Signed::new("payload".to_string(), secp(), &sk).expect("sign");
        assert_eq!(signed.data, "payload");
        assert_eq!(signed.sig_algorithm.name(), "secp256k1");
        assert_eq!(
            crate::util::certificate_helper::decode_signature_der_to_rs(&signed.sig)
                .expect("a DER signature")
                .len(),
            64,
            "`sign` returns the DER form, which unwraps to the 64-byte R‖S"
        );

        let restored = Signed::from_signed_data(
            signed.data.clone(),
            signed.pk.clone(),
            signed.sig.clone(),
            secp(),
        )
        .expect("the signature verifies");
        assert_eq!(restored.data, signed.data);
        assert_eq!(restored.sig, signed.sig);
    }

    /// The eth variant signs the keccak/EIP-191 hash, not the BLAKE2b one, and round-trips through
    /// the same constructor — a mixed-up hash would verify a different message than it signed.
    #[test]
    fn the_eth_variant_round_trips_through_its_own_hash() {
        let (sk, _pk) = eth().new_key_pair();
        let signed = Signed::new("payload".to_string(), eth(), &sk).expect("sign");
        assert_eq!(signed.sig_algorithm.name(), "secp256k1:eth");
        assert_eq!(signed.sig.len(), 64, "raw RS, no DER wrapper");

        assert!(Signed::from_signed_data(
            signed.data.clone(),
            signed.pk.clone(),
            signed.sig.clone(),
            eth()
        )
        .is_some());
        assert_eq!(
            signature_hash(
                "secp256k1",
                &<String as Serialize<String>>::encode(&signed.data)
            ),
            crate::hash::blake2b256::hash(b"payload"),
            "the secp256k1 hash of the same payload is a different hash, and would not verify"
        );
    }

    /// The failure arms: a tampered signature, tampered data, or a different public key all return
    /// `None` rather than a value that claims to be verified.
    #[test]
    fn a_tampered_signature_data_or_key_is_refused() {
        let (sk, pk) = secp().new_key_pair();
        let signed = Signed::new("payload".to_string(), secp(), &sk).expect("sign");

        let mut tampered_sig = signed.sig.clone();
        tampered_sig[10] ^= 0x01;
        assert!(Signed::from_signed_data(
            signed.data.clone(),
            signed.pk.clone(),
            tampered_sig,
            secp()
        )
        .is_none());

        assert!(
            Signed::from_signed_data(
                "other".to_string(),
                signed.pk.clone(),
                signed.sig.clone(),
                secp()
            )
            .is_none(),
            "the signature is over the data"
        );

        assert!(
            Signed::from_signed_data(signed.data.clone(), pk.clone(), signed.sig.clone(), eth())
                .is_none(),
            "the eth variant verifies a different hash, so the secp signature does not carry over"
        );

        assert!(Signed::from_signed_data(signed.data.clone(), pk, vec![0u8; 64], secp()).is_none());
    }
}
