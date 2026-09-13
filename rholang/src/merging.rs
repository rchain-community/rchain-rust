//! Mergeable (number) channel merging (port of
//! `interpreter/merging/RholangMergingLogic.scala` + `RhoHistoryRepositorySyntax.scala`).
//!
//! Number channels are merged arithmetically: the merged value is the base value plus the sum of
//! the branch diffs, and the merged random generator is the merge of the branch generators.

use std::collections::{BTreeMap, BTreeSet};

use rchain_crypto::hash::blake2b256_hash::Blake2b256Hash;
use rchain_crypto::hash::blake2b512_random::Blake2b512Random;
use rchain_models::rholang::RhoType::RhoNumber;
use rchain_models::runtime::{BindPattern, ListParWithRandom, TaggedContinuation};
use rchain_models::sorted::SortedProc;
use rchain_rspace::hashing::stable_hash_provider::hash_produce;
use rchain_rspace::history::history_reader::HistoryReader;
use rchain_rspace::hot_store_trie_action::HotStoreTrieAction;
use rchain_rspace::internal::Datum;
use rchain_rspace::merger::channel_change::ChannelChange;
use rchain_rspace::serializers::scodec_serialize::{decode_datum, encode_datum_bytes};
use rchain_rspace::trace::event::Produce;
use rchain_shared::typed_store::Codec;

use crate::storage::RhoHistoryRepository;

/// The concrete hot-store trie action type for the rholang runtime.
pub type RhoHotStoreTrieAction =
    HotStoreTrieAction<SortedProc, BindPattern, ListParWithRandom, TaggedContinuation>;

/// The concrete (decoded) history reader.
pub type RhoHistoryReader =
    dyn HistoryReader<SortedProc, BindPattern, ListParWithRandom, TaggedContinuation>;

/// Extract the number + random state from a number-channel datum (port of `getNumberWithRnd`).
pub fn get_number_with_rnd(
    par_with_rnd: &ListParWithRandom,
) -> Result<(i64, Blake2b512Random), String> {
    let num = match par_with_rnd.pars.as_slice() {
        [p] => RhoNumber::unapply(p.as_par())
            .ok_or_else(|| "Number channel should contain single Int term.".to_string())?,
        _ => {
            return Err(format!(
                "Number channel should contain single Int term, found {} pars.",
                par_with_rnd.pars.len()
            ))
        }
    };
    Ok((num, par_with_rnd.random_state.clone()))
}

/// Decode the random state from a raw number-channel datum (port of `decodeRnd`).
pub fn decode_rnd(raw: &[u8]) -> Result<Blake2b512Random, String> {
    let datum: Datum<ListParWithRandom> =
        decode_datum(raw).map_err(|e| format!("decode number-channel datum: {e}"))?;
    Ok(datum.a.random_state)
}

/// Encode a merged number-channel datum (port of `createDatumEncoded`).
pub fn create_datum_encoded(
    channel_hash: Blake2b256Hash,
    num: i64,
    rnd: Blake2b512Random,
) -> Vec<u8> {
    let num_par = RhoNumber::apply(num);
    let par_with_rnd = ListParWithRandom {
        pars: vec![SortedProc::new(num_par)],
        random_state: rnd,
    };
    let data_hash = hash_produce(channel_hash.as_bytes(), &par_with_rnd, false);
    let produce = Produce::from_hash(channel_hash, data_hash, false);
    let datum = Datum {
        a: par_with_rnd,
        persist: false,
        source: produce,
    };
    encode_datum_bytes(&datum)
}

/// Merge a number-channel value from multiple changes + base state (port of
/// `calculateNumberChannelMerge`).
pub async fn calculate_number_channel_merge(
    channel_hash: Blake2b256Hash,
    diff: i64,
    changes: &ChannelChange<Vec<u8>>,
    base_reader: &(dyn HistoryReader<SortedProc, BindPattern, ListParWithRandom, TaggedContinuation>
          + Sync),
) -> Result<RhoHotStoreTrieAction, String> {
    // Read the initial value of the number channel from the base state.
    let data = base_reader
        .get_data(channel_hash)
        .await
        .map_err(|e| e.to_string())?;
    if data.len() > 1 {
        return Err(
            "To calculate difference on a number channel, single value is expected.".to_string(),
        );
    }
    let init_num = match data.first() {
        Some(d) => get_number_with_rnd(&d.a)?.0,
        None => 0,
    };

    let new_val = init_num
        .checked_add(diff)
        .ok_or_else(|| "number channel merge overflow".to_string())?;

    let unique_added: BTreeSet<&Vec<u8>> = changes.added.iter().collect();
    let new_rnd = if unique_added.len() == 1 {
        decode_rnd(
            changes
                .added
                .first()
                .ok_or_else(|| "Number channel merge has no added changes.".to_string())?,
        )?
    } else {
        // Multiple branches: merge the distinct, sorted random generators.
        let mut randoms: Vec<Blake2b512Random> = changes
            .added
            .iter()
            .map(|raw| decode_rnd(raw))
            .collect::<Result<_, String>>()?;
        let mut seen: BTreeSet<Vec<u8>> = BTreeSet::new();
        randoms.retain(|r| seen.insert(r.to_bytes()));
        randoms.sort_by_key(|r| r.to_bytes());
        Blake2b512Random::merge(&randoms)
    };

    let datum_encoded = create_datum_encoded(channel_hash, new_val, new_rnd);
    Ok(HotStoreTrieAction::TrieInsertBinaryProduce(
        channel_hash,
        vec![datum_encoded],
    ))
}

/// Read the numeric values of mergeable channels from the base state (port of
/// `readMergeableValues`).
pub async fn read_mergeable_values(
    history_repository: &RhoHistoryRepository,
    base_state: Blake2b256Hash,
    channel_hashes: &BTreeSet<Blake2b256Hash>,
) -> Result<BTreeMap<Blake2b256Hash, i64>, String> {
    let history_reader = history_repository.get_history_reader(base_state).await;
    let binary = history_reader.reader_binary();
    let mut out = BTreeMap::new();
    for ch in channel_hashes {
        let data = binary.get_data(*ch).await.map_err(|e| e.to_string())?;
        if data.len() > 1 {
            return Err(
                "To calculate difference on a number channel, single value is expected."
                    .to_string(),
            );
        }
        let num = match data.first() {
            Some(d) => get_number_with_rnd(&d.decoded.a)?.0,
            None => 0,
        };
        out.insert(*ch, num);
    }
    Ok(out)
}

/// A single mergeable (number) channel diff (port of `NumberChannel`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NumberChannel {
    pub hash: Blake2b256Hash,
    pub diff: i64,
}

/// A deploy's mergeable-channel data (port of `DeployMergeableData`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeployMergeableData {
    pub channels: Vec<NumberChannel>,
}

/// Zigzag-encode an `i64` into `u64`.
pub fn zigzag_encode(n: i64) -> u64 {
    ((n << 1) ^ (n >> 63)) as u64
}

/// LEB128 varint-encode a `u64`.
pub fn varint_encode(mut n: u64) -> Vec<u8> {
    let mut out = Vec::new();
    while n >= 0x80 {
        out.push((n as u8 & 0x7f) | 0x80);
        n >>= 7;
    }
    out.push(n as u8);
    out
}

/// scodec `vlong` — zigzag + LEB128 varint.
pub fn vlong_encode(n: i64) -> Vec<u8> {
    varint_encode(zigzag_encode(n))
}

fn uint16_be(n: u16) -> [u8; 2] {
    n.to_be_bytes()
}

fn int64_be(n: i64) -> [u8; 8] {
    n.to_be_bytes()
}

/// `variableSizeBytes(uint16, bytes)` — a 2-byte length prefix followed by the bytes.
fn var_size_u16(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + 2);
    out.extend_from_slice(&uint16_be(bytes.len() as u16));
    out.extend_from_slice(bytes);
    out
}

/// Encode the mergeable-store key (port of `codecMergeableKey`).
pub fn encode_mergeable_key(state_hash: &Blake2b256Hash, creator: &[u8], seq_num: i64) -> Vec<u8> {
    let mut out = var_size_u16(state_hash.as_bytes());
    out.extend(var_size_u16(creator));
    out.extend(vlong_encode(seq_num));
    out
}

/// Encode a deploy's mergeable-channel data (port of `deployMergeableDataCodec`).
pub fn encode_deploy_mergeable_data(data: &DeployMergeableData) -> Vec<u8> {
    let mut out = uint16_be(data.channels.len() as u16).to_vec();
    for ch in &data.channels {
        out.extend_from_slice(ch.hash.as_bytes());
        out.extend_from_slice(&int64_be(ch.diff));
    }
    out
}

/// Encode a sequence of deploy mergeable data (port of `deployMergeableDataSeqCodec`).
pub fn encode_deploy_mergeable_data_seq(seq: &[DeployMergeableData]) -> Vec<u8> {
    let mut out = uint16_be(seq.len() as u16).to_vec();
    for d in seq {
        out.extend(encode_deploy_mergeable_data(d));
    }
    out
}

fn read_u16(bytes: &[u8], idx: &mut usize) -> Result<u16, String> {
    if *idx + 2 > bytes.len() {
        return Err("mergeable data: unexpected end of input".to_string());
    }
    let v = u16::from_be_bytes([bytes[*idx], bytes[*idx + 1]]);
    *idx += 2;
    Ok(v)
}

fn read_i64(bytes: &[u8], idx: &mut usize) -> Result<i64, String> {
    if *idx + 8 > bytes.len() {
        return Err("mergeable data: unexpected end of input".to_string());
    }
    let mut arr = [0u8; 8];
    arr.copy_from_slice(&bytes[*idx..*idx + 8]);
    *idx += 8;
    Ok(i64::from_be_bytes(arr))
}

/// Decode a sequence of deploy mergeable data (inverse of `encode_deploy_mergeable_data_seq`).
pub fn decode_deploy_mergeable_data_seq(bytes: &[u8]) -> Result<Vec<DeployMergeableData>, String> {
    let mut idx = 0usize;
    let count = read_u16(bytes, &mut idx)? as usize;
    let mut seq = Vec::with_capacity(count);
    for _ in 0..count {
        let channel_count = read_u16(bytes, &mut idx)? as usize;
        let mut channels = Vec::with_capacity(channel_count);
        for _ in 0..channel_count {
            if idx + 32 > bytes.len() {
                return Err("mergeable data: unexpected end of input".to_string());
            }
            let hash = Blake2b256Hash::from_byte_array(&bytes[idx..idx + 32]);
            idx += 32;
            let diff = read_i64(bytes, &mut idx)?;
            channels.push(NumberChannel { hash, diff });
        }
        seq.push(DeployMergeableData { channels });
    }
    Ok(seq)
}

/// A codec for the mergeable store value `Vec<DeployMergeableData>`.
pub struct DeployMergeableDataCodec;

impl Codec<Vec<DeployMergeableData>> for DeployMergeableDataCodec {
    fn encode(&self, value: &Vec<DeployMergeableData>) -> Vec<u8> {
        encode_deploy_mergeable_data_seq(value)
    }

    fn decode(&self, bytes: &[u8]) -> Result<Vec<DeployMergeableData>, String> {
        decode_deploy_mergeable_data_seq(bytes)
    }
}

/// Convert final number-channel values to per-deploy diffs (port of `calculateNumChannelDiff`).
///
/// `init_values` are the pre-state values for every channel key (default `0` when absent).
pub fn calculate_num_channel_diff(
    channel_values: &[BTreeMap<Blake2b256Hash, i64>],
    init_values: &BTreeMap<Blake2b256Hash, i64>,
) -> Vec<BTreeMap<Blake2b256Hash, i64>> {
    let mut prev_vals = init_values.clone();
    let mut result = Vec::with_capacity(channel_values.len());
    for end_vals in channel_values {
        let mut diff_map = BTreeMap::new();
        for (ch, end_val) in end_vals {
            if let Some(prev) = prev_vals.get(ch) {
                diff_map.insert(*ch, end_val.wrapping_sub(*prev));
            }
        }
        for (ch, end_val) in end_vals {
            prev_vals.insert(*ch, *end_val);
        }
        result.push(diff_map);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(byte: u8) -> Blake2b256Hash {
        Blake2b256Hash::from_bytes([byte; 32])
    }

    #[test]
    fn mergeable_data_round_trips() {
        let seq = vec![
            DeployMergeableData {
                channels: vec![NumberChannel {
                    hash: h(1),
                    diff: 10,
                }],
            },
            DeployMergeableData {
                channels: vec![
                    NumberChannel {
                        hash: h(1),
                        diff: -5,
                    },
                    NumberChannel {
                        hash: h(2),
                        diff: 7,
                    },
                ],
            },
        ];
        let bytes = encode_deploy_mergeable_data_seq(&seq);
        assert_eq!(decode_deploy_mergeable_data_seq(&bytes).unwrap(), seq);
    }

    #[test]
    fn calculate_diff_accumulates_values() {
        let a = h(1);
        let values = vec![
            BTreeMap::from([(a, 20i64)]),
            BTreeMap::from([(a, 25i64)]),
            BTreeMap::from([(a, 15i64)]),
        ];
        let init = BTreeMap::from([(a, 10i64)]);
        let diffs = calculate_num_channel_diff(&values, &init);
        assert_eq!(
            diffs,
            vec![
                BTreeMap::from([(a, 10i64)]),
                BTreeMap::from([(a, 5i64)]),
                BTreeMap::from([(a, -10i64)]),
            ]
        );
    }
}

#[cfg(test)]
mod codec_tests {
    use super::*;

    /// The mergeable key and channel data are a **scodec wire format** shared with the Scala node's
    /// mergeable store, so the encodings are pinned byte-for-byte: a change here would make the Rust
    /// node's mergeable keys unreadable (or silently *different*) from a Scala-produced one.
    #[test]
    fn the_scodec_primitives_match_scodec() {
        // `vlong` is zigzag + LEB128: the zigzag maps signed magnitudes onto the unsigned space so
        // small negatives are short, and LEB128 keeps the continuation bit in the top bit.
        assert_eq!(zigzag_encode(0), 0);
        assert_eq!(zigzag_encode(-1), 1);
        assert_eq!(zigzag_encode(1), 2);
        assert_eq!(zigzag_encode(-2), 3);
        assert_eq!(zigzag_encode(i64::MAX), u64::MAX - 1);
        assert_eq!(zigzag_encode(i64::MIN), u64::MAX);

        assert_eq!(varint_encode(0), vec![0x00]);
        assert_eq!(varint_encode(1), vec![0x01]);
        assert_eq!(varint_encode(127), vec![0x7F]);
        assert_eq!(varint_encode(128), vec![0x80, 0x01]);
        assert_eq!(varint_encode(300), vec![0xAC, 0x02]);
        assert_eq!(varint_encode(16384), vec![0x80, 0x80, 0x01]);
        assert_eq!(
            varint_encode(u64::MAX).len(),
            10,
            "a u64 needs at most 10 groups"
        );

        assert_eq!(vlong_encode(-1), vec![0x01], "zigzag first, then varint");
        assert_eq!(vlong_encode(1), vec![0x02]);

        // The fixed-width helpers are big-endian, which is what `variableSizeBytes(uint16, …)` and
        // the channel diffs are built from.
        assert_eq!(uint16_be(1), [0x00, 0x01]);
        assert_eq!(uint16_be(0x1234), [0x12, 0x34]);
        assert_eq!(int64_be(1), [0, 0, 0, 0, 0, 0, 0, 1]);
        assert_eq!(int64_be(-1), [0xFF; 8]);
        assert_eq!(var_size_u16(&[0xAB, 0xCD]), vec![0x00, 0x02, 0xAB, 0xCD]);
        assert_eq!(
            var_size_u16(&[]),
            vec![0x00, 0x00],
            "an empty value is a zero length"
        );
    }

    /// The mergeable **key** is `variableSizeBytes(stateHash) ‖ variableSizeBytes(creator) ‖ vlong(seqNum)`
    /// — the layout the mergeable store looks up by, so its field order and widths are the contract.
    #[test]
    fn the_mergeable_key_layout_is_pinned() {
        let state_hash = Blake2b256Hash::from_bytes([0x11; 32]);
        let creator = [0x22u8; 20];
        let key = encode_mergeable_key(&state_hash, &creator, 5);

        let mut expected = vec![0x00, 0x20];
        expected.extend_from_slice(&[0x11; 32]);
        expected.extend_from_slice(&[0x00, 0x14]);
        expected.extend_from_slice(&[0x22; 20]);
        expected.extend_from_slice(&[0x0A]); // vlong(5) = varint(zigzag(5) = 10)
        assert_eq!(key, expected);

        // A negative sequence number still encodes in one byte (zigzag), and a large one grows.
        let negative = encode_mergeable_key(&state_hash, &creator, -1);
        assert_eq!(negative.last(), Some(&0x01));
        let large = encode_mergeable_key(&state_hash, &creator, 1000);
        assert!(large.len() > expected.len());
        assert_eq!(
            &large[..expected.len() - 1],
            &expected[..expected.len() - 1]
        );
    }

    /// The per-deploy mergeable data is `uint16` channel count followed by
    /// `32-byte hash ‖ int64 diff` per channel, and the sequence form prefixes its own count — a
    /// reader that expected a different count width would read the first hash's bytes as a count.
    #[test]
    fn the_mergeable_channel_data_layout_is_pinned() {
        let channels = vec![
            NumberChannel {
                hash: Blake2b256Hash::from_bytes([0x01; 32]),
                diff: 7,
            },
            NumberChannel {
                hash: Blake2b256Hash::from_bytes([0x02; 32]),
                diff: -3,
            },
        ];
        let encoded = encode_deploy_mergeable_data(&DeployMergeableData {
            channels: channels.clone(),
        });

        assert_eq!(&encoded[..2], &[0x00, 0x02], "the channel count");
        assert_eq!(encoded.len(), 2 + 2 * (32 + 8));
        assert_eq!(&encoded[2..34], &[0x01; 32], "the first hash");
        assert_eq!(
            &encoded[34..42],
            &[0, 0, 0, 0, 0, 0, 0, 7],
            "the first diff"
        );
        assert_eq!(&encoded[42..74], &[0x02; 32], "the second hash");
        assert_eq!(
            &encoded[74..82],
            &[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFD],
            "the second diff is a signed int64 (-3)"
        );

        // The sequence form adds an outer count and concatenates the per-deploy encodings.
        let seq = encode_deploy_mergeable_data_seq(&[
            DeployMergeableData {
                channels: channels.clone(),
            },
            DeployMergeableData {
                channels: Vec::new(),
            },
        ]);
        assert_eq!(&seq[..2], &[0x00, 0x02], "the deploy count");
        assert_eq!(&seq[2..2 + encoded.len()], encoded.as_slice());
        assert_eq!(
            seq.len(),
            2 + encoded.len() + 2,
            "an empty deploy is just its count"
        );

        // An empty deploy list is two zero bytes, not an empty vector.
        assert_eq!(encode_deploy_mergeable_data_seq(&[]), vec![0x00, 0x00]);
    }

    /// A **round trip through the varint** is the property that matters for the key's `seqNum`: any
    /// `i64` must survive zigzag + LEB128, since the mergeable key is looked up by decoding it back.
    #[test]
    fn every_sequence_number_round_trips_through_the_varint() {
        let decode = |bytes: &[u8]| -> i64 {
            let mut value: u64 = 0;
            for (i, b) in bytes.iter().enumerate() {
                value |= u64::from(b & 0x7f) << (7 * i);
            }
            // Undo the zigzag: the low bit is the sign.
            let n = (value >> 1) as i64;
            if value & 1 == 1 {
                !n
            } else {
                n
            }
        };
        for n in [
            0i64,
            1,
            -1,
            63,
            64,
            -64,
            127,
            128,
            -128,
            1000,
            -1000,
            i32::MAX as i64,
            i32::MIN as i64,
        ] {
            assert_eq!(decode(&vlong_encode(n)), n, "{n}");
        }
    }

    /// `decode_rnd` decodes the random state the mergeable tag carries, and the **encoded form round
    /// trips** — that is the case the mergeable store depends on.
    #[test]
    fn a_random_state_round_trips_through_the_datum_encoding() {
        let rand = Blake2b512Random::from_init(&[7u8; 32]);
        let encoded = create_datum_encoded(Blake2b256Hash::from_bytes([3u8; 32]), 1, rand.clone());
        let decoded = decode_rnd(&encoded).expect("the encoded form decodes");
        assert_eq!(decoded, rand);
    }

    /// **A truncated blob panics rather than erroring** (`BitReader::read_bit` indexes
    /// `bytes[bit_pos / 8]` unchecked). Latent, not live: the bytes come from the node's own
    /// mergeable store, which the node wrote from its own replay — so the exposure is a corrupted or
    /// truncated *local* entry aborting the merge instead of reporting a decode error. Recorded as
    /// AUDIT §16 C15 and pinned here, so giving the reader a `Result` fails this test and is a
    /// deliberate change.
    #[test]
    #[should_panic(expected = "index out of bounds")]
    fn a_truncated_mergeable_datum_panics() {
        let _ = decode_rnd(b"");
    }
}
