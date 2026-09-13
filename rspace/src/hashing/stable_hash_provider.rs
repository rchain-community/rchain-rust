//! Stable hashing for channels, joins, produces, and consumes (Law 7: join commutativity).
//!
//! Mirrors `rspace/src/main/scala/coop/rchain/rspace/hashing/StableHashProvider.scala`. Channel
//! keys are hashed in sorted order so that a join's hash is independent of the order the channels
//! were supplied in.

use rchain_crypto::hash::blake2b256_hash::Blake2b256Hash;
use rchain_shared::serialize::Serialize;

use crate::serializers::scodec_serialize::{
    bool8, encode_seq_byte_vectors, to_ordered_byte_vectors,
};

/// Hash a single channel (port of `hash[C](channel)`).
pub fn hash_channel<C>(channel: &C) -> Blake2b256Hash
where
    C: Serialize<C>,
{
    Blake2b256Hash::create(&<C as Serialize<C>>::encode(channel))
}

/// Hash each channel and sort the hashes (port of `hashSeq[C]`).
pub fn hash_seq<C>(channels: &[C]) -> Vec<Blake2b256Hash>
where
    C: Serialize<C>,
{
    let mut hashes: Vec<Blake2b256Hash> = channels.iter().map(hash_channel).collect();
    hashes.sort();
    hashes
}

/// Hash a join of channels (port of `hash[C](channels)`).
pub fn hash_channels<C>(channels: &[C]) -> Blake2b256Hash
where
    C: Serialize<C>,
{
    hash_hashes(&hash_seq(channels))
}

/// Hash a sorted sequence of channel hashes (port of `hash(channelsHashes)`).
pub fn hash_hashes(channel_hashes: &[Blake2b256Hash]) -> Blake2b256Hash {
    let mut sorted: Vec<&[u8; 32]> = channel_hashes.iter().map(|h| h.as_bytes()).collect();
    sorted.sort();
    let parts: Vec<&[u8]> = sorted.into_iter().map(|b| b as &[u8]).collect();
    Blake2b256Hash::create_many(&parts)
}

/// Hash a consume: sorted channel hashes + sorted patterns + continuation + persist (port of the
/// `hash[P, K]` overload used by `Consume.apply`).
pub fn hash_consume<P, K>(
    encoded_channels: &[Vec<u8>],
    patterns: &[P],
    continuation: &K,
    persist: bool,
) -> Blake2b256Hash
where
    P: Serialize<P>,
    K: Serialize<K>,
{
    let mut encoded_seq: Vec<Vec<u8>> = encoded_channels.to_vec();
    encoded_seq.extend(to_ordered_byte_vectors(patterns));
    encoded_seq.push(<K as Serialize<K>>::encode(continuation));
    encoded_seq.push(bool8(persist));
    Blake2b256Hash::create(&encode_seq_byte_vectors(&encoded_seq))
}

/// Hash a produce: channel bytes + datum + persist (port of the `hash[A](channel, datum, persist)`
/// overload used by `Produce.apply`).
pub fn hash_produce<A>(channel: &[u8], datum: &A, persist: bool) -> Blake2b256Hash
where
    A: Serialize<A>,
{
    let encoded_seq = vec![
        channel.to_vec(),
        <A as Serialize<A>>::encode(datum),
        bool8(persist),
    ];
    Blake2b256Hash::create(&encode_seq_byte_vectors(&encoded_seq))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct ByteChannel(Vec<u8>);
    impl Serialize<ByteChannel> for ByteChannel {
        fn encode(a: &ByteChannel) -> Vec<u8> {
            a.0.clone()
        }
        fn decode(bytes: &[u8]) -> Result<ByteChannel, String> {
            Ok(ByteChannel(bytes.to_vec()))
        }
    }

    #[test]
    fn hash_seq_sorts_channel_hashes() {
        // Two channels supplied in one order hash the same as the reverse order.
        let a = ByteChannel(vec![1]);
        let b = ByteChannel(vec![2]);
        let forward = hash_seq(&[a.clone(), b.clone()]);
        let backward = hash_seq(&[b, a]);
        assert_eq!(forward, backward);
        // hashes are sorted ascending
        assert!(forward.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn join_hash_is_order_independent() {
        let a = ByteChannel(vec![1]);
        let b = ByteChannel(vec![2]);
        assert_eq!(
            hash_channels(&[a.clone(), b.clone()]),
            hash_channels(&[b, a])
        );
    }

    #[test]
    fn produce_hash_depends_on_channel_datum_persist() {
        let d1 = ByteChannel(vec![9]);
        let d2 = ByteChannel(vec![10]);
        let h = hash_produce(&[1, 2, 3], &d1, true);
        assert_ne!(h, hash_produce(&[1, 2, 3], &d2, true));
        assert_ne!(h, hash_produce(&[1, 2, 3], &d1, false));
    }
}

/// Differential tests against the Scala `StableHashProvider` (Law 7). Golden vectors are the
/// Blake2b256 digests produced by Scala for the same inputs, captured in
/// `testdata/differential/stable_hash.tsv`.
#[cfg(test)]
mod differential {
    use super::*;
    use rchain_shared::base16;
    use rchain_shared::serialize::Serialize;

    #[derive(Clone)]
    struct ByteChannel(Vec<u8>);
    impl Serialize<ByteChannel> for ByteChannel {
        fn encode(a: &ByteChannel) -> Vec<u8> {
            a.0.clone()
        }
        fn decode(bytes: &[u8]) -> Result<ByteChannel, String> {
            Ok(ByteChannel(bytes.to_vec()))
        }
    }

    /// Rows are `id<TAB>value<TAB>provenance`; `#` lines are the legend.
    fn rows() -> Vec<(String, String, String)> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/testdata/differential/stable_hash.tsv"
        );
        std::fs::read_to_string(path)
            .expect("the golden file")
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .map(|l| {
                let mut f = l.split('\t');
                (
                    f.next().unwrap_or_default().to_string(),
                    f.next().unwrap_or_default().to_string(),
                    f.next().unwrap_or_default().to_string(),
                )
            })
            .collect()
    }

    fn load(case: &str) -> String {
        rows()
            .into_iter()
            .find(|(id, _, _)| id == case)
            .unwrap_or_else(|| panic!("missing differential case: {case}"))
            .1
    }

    /// **Golden-drift guard** (see `models/src/wire.rs` for the shape): every row must be consumed,
    /// and every row must carry its provenance.
    #[test]
    fn every_golden_row_is_consumed() {
        const CONSUMED: &[&str] = &[
            "ch_0102",
            "ch_empty",
            "join_ab",
            "produce_0102_03_false",
            "produce_0102_03_true",
            "consume_1ch_false",
            "consume_2ch_true",
        ];
        let rows = rows();
        assert_eq!(rows.len(), CONSUMED.len(), "row count");
        for case in CONSUMED {
            assert!(
                rows.iter().any(|(id, _, _)| id == case),
                "{case} is missing"
            );
            assert!(!load(case).is_empty(), "{case} has an empty value");
        }
        for (id, _, provenance) in rows {
            assert!(
                provenance == "scala-ground-truth",
                "{id} must carry the oracle provenance, got {provenance:?}"
            );
        }
    }

    fn hex(h: &Blake2b256Hash) -> String {
        base16::encode(h.as_bytes())
    }

    #[test]
    fn differential_hash_channel() {
        assert_eq!(
            hex(&hash_channel(&ByteChannel(vec![0x01, 0x02]))),
            load("ch_0102")
        );
        assert_eq!(hex(&hash_channel(&ByteChannel(vec![]))), load("ch_empty"));
    }

    #[test]
    fn differential_join_hash() {
        let a = ByteChannel(vec![0x01]);
        let b = ByteChannel(vec![0x02]);
        assert_eq!(hex(&hash_channels(&[a, b])), load("join_ab"));
    }

    #[test]
    fn differential_produce_hash() {
        let d = ByteChannel(vec![0x03]);
        assert_eq!(
            hex(&hash_produce(&[0x01, 0x02], &d, false)),
            load("produce_0102_03_false")
        );
        assert_eq!(
            hex(&hash_produce(&[0x01, 0x02], &d, true)),
            load("produce_0102_03_true")
        );
    }

    #[test]
    fn differential_consume_hash() {
        let ch1 = hash_channel(&ByteChannel(vec![0x01]));
        let encoded1 = vec![ch1.to_byte_array().to_vec()];
        assert_eq!(
            hex(&hash_consume(
                &encoded1,
                &[ByteChannel(vec![0x04])],
                &ByteChannel(vec![0x05]),
                false,
            )),
            load("consume_1ch_false")
        );

        let mut encoded2: Vec<Vec<u8>> = vec![
            hash_channel(&ByteChannel(vec![0x01]))
                .to_byte_array()
                .to_vec(),
            hash_channel(&ByteChannel(vec![0x02]))
                .to_byte_array()
                .to_vec(),
        ];
        encoded2.sort();
        assert_eq!(
            hex(&hash_consume(
                &encoded2,
                &[ByteChannel(vec![0x04]), ByteChannel(vec![0x05])],
                &ByteChannel(vec![0x06]),
                true,
            )),
            load("consume_2ch_true")
        );
    }
}
