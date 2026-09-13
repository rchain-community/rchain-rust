//! Block-oriented Blake2b512 with online tree hashing.
//!
//! Mirrors `crypto/src/main/scala/coop/rchain/crypto/hash/Blake2b512Block.scala`, which is an
//! abbreviated version of BouncyCastle's `Blake2bDigest`. This is a hand-port: no crates.io crate
//! exposes the custom parameter-block + `peekFinalRoot`/`finalizeInternal` API. The `compress`
//! core, `IV`, `SIGMA`, and `ROUNDS` are copied verbatim.

const CHAIN_VALUE_LENGTH: usize = 8;
const BLOCK_LENGTH_BYTES: usize = 128;
const BLOCK_LENGTH_LONGS: usize = 16;
// Depth = 255, Fanout = ??, Keylength = 0, Digest length = 64 bytes.
const PARAM_VALUE_0: u64 = 0xFF00_0040;
// Inner length = 32 bytes.
const PARAM_VALUE_2: u64 = 0x2000;

/// Produced from the square roots of primes 2, 3, 5, 7, 11, 13, 17, 19 (same as SHA-512 IV).
const IV: [u64; 8] = [
    0x6A09_E667_F3BC_C908,
    0xBB67_AE85_84CA_A73B,
    0x3C6E_F372_FE94_F82B,
    0xA54F_F53A_5F1D_36F1,
    0x510E_527F_ADE6_82D1,
    0x9B05_688C_2B3E_6C1F,
    0x1F83_D9AB_FB41_BD6B,
    0x5BE0_CD19_137E_2179,
];

/// Message word permutations.
const SIGMA: [[u8; 16]; 12] = [
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
    [14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3],
    [11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4],
    [7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8],
    [9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13],
    [2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9],
    [12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11],
    [13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10],
    [6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5],
    [10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0],
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
    [14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3],
];

const ROUNDS: usize = 12;

use crate::errors::CryptoError;

/// Read a little-endian `u64` from `bytes[start..start + 8]`. The caller guarantees the 8-byte
/// window is in bounds (a full 128-byte message block or a length-checked serialization).
pub(crate) fn u64_from_le_at(bytes: &[u8], start: usize) -> u64 {
    let mut arr = [0u8; 8];
    arr.copy_from_slice(&bytes[start..start + 8]);
    u64::from_le_bytes(arr)
}

/// A Blake2b512 block hasher with a configurable fanout.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Blake2b512Block {
    chain_value: [u64; CHAIN_VALUE_LENGTH],
    t0: u64,
    t1: u64,
}

impl Blake2b512Block {
    /// Create a block hasher with the given fanout encoded into the parameter block.
    pub fn new(fanout: u8) -> Self {
        let param0_with_fanout = PARAM_VALUE_0 | ((fanout as u64 & 0xff) << 16);
        let mut chain_value = IV;
        chain_value[0] ^= param0_with_fanout;
        chain_value[2] ^= PARAM_VALUE_2;
        Self {
            chain_value,
            t0: 0,
            t1: 0,
        }
    }

    /// Copy constructor (the Scala `apply(src: Blake2b512Block)`).
    pub fn from_block(src: &Self) -> Self {
        Self {
            chain_value: src.chain_value,
            t0: src.t0,
            t1: src.t1,
        }
    }

    /// Compress a 128-byte block into the running state (the Scala `update(block, offset)`).
    pub fn update(&mut self, block: &[u8], offset: usize) {
        let (new_t0, new_t1) = self.new_t0_t1();
        let mut out = [0u64; CHAIN_VALUE_LENGTH];
        self.compress_into(block, offset, &mut out, false, false);
        self.chain_value = out;
        self.t0 = new_t0;
        self.t1 = new_t1;
    }

    /// Compute the final root as if `block` were the last block, without mutating state.
    pub fn peek_final_root(
        &self,
        block: &[u8],
        in_offset: usize,
        output: &mut [u8],
        out_offset: usize,
    ) {
        let mut temp = [0u64; CHAIN_VALUE_LENGTH];
        self.compress_into(block, in_offset, &mut temp, true, true);
        write_le_long(&temp, output, out_offset);
    }

    /// Compute the internal (32-byte) hash as if `block` were the last block, without mutating state.
    pub fn finalize_internal(
        &self,
        block: &[u8],
        in_offset: usize,
        output: &mut [u8],
        out_offset: usize,
    ) {
        let mut temp = [0u64; CHAIN_VALUE_LENGTH];
        self.compress_into(block, in_offset, &mut temp, true, false);
        write_le_long(&temp[..4], output, out_offset);
    }

    fn new_t0_t1(&self) -> (u64, u64) {
        let new_t0 = self.t0.wrapping_add(BLOCK_LENGTH_BYTES as u64);
        let new_t1 = if new_t0 == 0 {
            self.t1.wrapping_add(1)
        } else {
            self.t1
        };
        (new_t0, new_t1)
    }

    fn compress_into(
        &self,
        msg: &[u8],
        offset: usize,
        out: &mut [u64; CHAIN_VALUE_LENGTH],
        finalize: bool,
        root_finalize: bool,
    ) {
        let mut internal_state = [0u64; BLOCK_LENGTH_LONGS];
        let (new_t0, new_t1) = self.new_t0_t1();

        fn g(
            internal_state: &mut [u64; BLOCK_LENGTH_LONGS],
            m1: u64,
            m2: u64,
            pos_a: usize,
            pos_b: usize,
            pos_c: usize,
            pos_d: usize,
        ) {
            internal_state[pos_a] = internal_state[pos_a]
                .wrapping_add(internal_state[pos_b])
                .wrapping_add(m1);
            internal_state[pos_d] =
                (internal_state[pos_d] ^ internal_state[pos_a]).rotate_right(32);
            internal_state[pos_c] = internal_state[pos_c].wrapping_add(internal_state[pos_d]);
            internal_state[pos_b] =
                (internal_state[pos_b] ^ internal_state[pos_c]).rotate_right(24);
            internal_state[pos_a] = internal_state[pos_a]
                .wrapping_add(internal_state[pos_b])
                .wrapping_add(m2);
            internal_state[pos_d] =
                (internal_state[pos_d] ^ internal_state[pos_a]).rotate_right(16);
            internal_state[pos_c] = internal_state[pos_c].wrapping_add(internal_state[pos_d]);
            internal_state[pos_b] =
                (internal_state[pos_b] ^ internal_state[pos_c]).rotate_right(63);
        }

        // init
        internal_state[..CHAIN_VALUE_LENGTH].copy_from_slice(&self.chain_value);
        internal_state[CHAIN_VALUE_LENGTH..12].copy_from_slice(&IV[..4]);
        let f0: u64 = if finalize { u64::MAX } else { 0 };
        let f1: u64 = if root_finalize { u64::MAX } else { 0 };
        internal_state[12] = new_t0 ^ IV[4];
        internal_state[13] = new_t1 ^ IV[5];
        internal_state[14] = f0 ^ IV[6];
        internal_state[15] = f1 ^ IV[7];

        // message words
        let mut m = [0u64; BLOCK_LENGTH_LONGS];
        for (i, mi) in m.iter_mut().enumerate() {
            let start = offset + i * 8;
            *mi = u64_from_le_at(msg, start);
        }

        for round in 0..ROUNDS {
            let s = &SIGMA[round];
            g(
                &mut internal_state,
                m[s[0] as usize],
                m[s[1] as usize],
                0,
                4,
                8,
                12,
            );
            g(
                &mut internal_state,
                m[s[2] as usize],
                m[s[3] as usize],
                1,
                5,
                9,
                13,
            );
            g(
                &mut internal_state,
                m[s[4] as usize],
                m[s[5] as usize],
                2,
                6,
                10,
                14,
            );
            g(
                &mut internal_state,
                m[s[6] as usize],
                m[s[7] as usize],
                3,
                7,
                11,
                15,
            );
            g(
                &mut internal_state,
                m[s[8] as usize],
                m[s[9] as usize],
                0,
                5,
                10,
                15,
            );
            g(
                &mut internal_state,
                m[s[10] as usize],
                m[s[11] as usize],
                1,
                6,
                11,
                12,
            );
            g(
                &mut internal_state,
                m[s[12] as usize],
                m[s[13] as usize],
                2,
                7,
                8,
                13,
            );
            g(
                &mut internal_state,
                m[s[14] as usize],
                m[s[15] as usize],
                3,
                4,
                9,
                14,
            );
        }

        for i in 0..CHAIN_VALUE_LENGTH {
            out[i] = self.chain_value[i] ^ internal_state[i] ^ internal_state[i + 8];
        }
    }

    /// Serialize to 80 bytes: chain value (8 LE longs) + t0 + t1.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(80);
        for v in &self.chain_value {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&self.t0.to_le_bytes());
        out.extend_from_slice(&self.t1.to_le_bytes());
        out
    }

    /// Deserialize from 80 bytes (the Scala `typeMapper.toCustom`).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CryptoError> {
        if bytes.len() < 80 {
            return Err(CryptoError::InvalidLength {
                expected: 80,
                actual: bytes.len(),
            });
        }
        let mut chain_value = [0u64; CHAIN_VALUE_LENGTH];
        for (i, cv) in chain_value.iter_mut().enumerate() {
            *cv = u64_from_le_at(bytes, i * 8);
        }
        let t0 = u64_from_le_at(bytes, 64);
        let t1 = u64_from_le_at(bytes, 72);
        Ok(Self {
            chain_value,
            t0,
            t1,
        })
    }

    /// For testing only — will give invalid results otherwise.
    pub fn tweak_t0(&mut self) {
        self.t0 = u64::MAX;
    }

    /// Diagnostic string (mirrors `Blake2b512Block.debugStr`).
    pub fn debug_str(&self) -> String {
        let cv = self
            .chain_value
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        format!("chainValue: {cv}\nt0: {}\nt1: {}\n", self.t0, self.t1)
    }
}

fn write_le_long(values: &[u64], output: &mut [u8], out_offset: usize) {
    for (i, v) in values.iter().enumerate() {
        let bytes = v.to_le_bytes();
        output[out_offset + i * 8..out_offset + i * 8 + 8].copy_from_slice(&bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 128-byte message block these tests feed to `update`: a fixed pattern, distinct per
    /// 8-byte word so an offset mistake cannot pass unnoticed.
    fn message(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i as u8).wrapping_mul(7).wrapping_add(3)).collect()
    }

    fn word_of(bytes: &[u8], index: usize) -> u64 {
        let mut arr = [0u8; 8];
        arr.copy_from_slice(&bytes[index * 8..index * 8 + 8]);
        u64::from_le_bytes(arr)
    }

    /// The **oracle for the compression core is not this module**: `blake2b512_random.rs` pins
    /// `update`/`peek_final_root`/`finalize_internal` against the Scala spec's expected hashes
    /// (including the `t0` wraparound). What follows pins what those vectors cannot name — the
    /// parameter block, the counters, the non-mutation of the peek/finalize pair, and the
    /// serialization.
    ///
    /// The parameter block is the part with no second witness: the Scala passes `fanout` into
    /// `PARAM_VALUE_0`'s bits 16–23 and XORs `depth = 255` and the 32-byte inner length into words
    /// 0 and 2, and nothing else in the crate would notice if that wiring changed.
    #[test]
    fn the_parameter_block_carries_depth_fanout_and_inner_length() {
        let bytes = Blake2b512Block::new(1).to_bytes();
        assert_eq!(bytes.len(), 80, "8 chain words + t0 + t1");

        assert_eq!(word_of(&bytes, 0), IV[0] ^ (PARAM_VALUE_0 | (1u64 << 16)));
        assert_eq!(word_of(&bytes, 2), IV[2] ^ PARAM_VALUE_2);
        for i in [1usize, 3, 4, 5, 6, 7] {
            assert_eq!(word_of(&bytes, i), IV[i], "word {i} is unmodified IV");
        }
        assert_eq!(word_of(&bytes, 8), 0, "t0 starts at zero");
        assert_eq!(word_of(&bytes, 9), 0, "t1 starts at zero");

        // The fanout is the only thing that moves between the two hashers the tree constructs.
        let one = Blake2b512Block::new(1).to_bytes();
        let two = Blake2b512Block::new(2).to_bytes();
        assert_ne!(one, two);
        assert_eq!(
            one.iter().zip(two.iter()).filter(|(a, b)| a != b).count(),
            1,
            "only the fanout byte differs: {}",
            one.iter().zip(two.iter()).position(|(a, b)| a != b).unwrap()
        );
        let iv0 = IV[0].to_le_bytes();
        assert_eq!(one[2], iv0[2] ^ 1, "fanout 1 in PARAM_VALUE_0's second byte");
        assert_eq!(two[2], iv0[2] ^ 2, "…and 2 for fanout 2");
        assert_eq!(one[0], iv0[0] ^ 0x40, "the digest length (64) is the low byte");
    }

    /// `from_block` is a **copy constructor**: mutating the copy must not disturb the original,
    /// because `Blake2b512Random` forks its stream by copying a digest and advancing the copy.
    #[test]
    fn from_block_copies_the_state_rather_than_sharing_it() {
        let original = Blake2b512Block::new(1);
        let mut copy = Blake2b512Block::from_block(&original);
        assert_eq!(copy, original);

        copy.update(&message(128), 0);
        assert_ne!(copy, original, "the copy advanced");
        assert_eq!(
            original.to_bytes(),
            Blake2b512Block::new(1).to_bytes(),
            "the original is still the initial state"
        );
    }

    /// Each compressed block advances the 128-bit counter by its length: `t0` counts bytes, and
    /// `t1` is the high half, so an un-wrapped sequence leaves `t1` at zero.
    #[test]
    fn update_advances_the_block_counter_by_128_bytes() {
        let mut hasher = Blake2b512Block::new(1);
        hasher.update(&message(128), 0);
        assert_eq!(word_of(&hasher.to_bytes(), 8), 128);
        assert_eq!(word_of(&hasher.to_bytes(), 9), 0);

        hasher.update(&message(128), 0);
        assert_eq!(word_of(&hasher.to_bytes(), 8), 256);
        assert_eq!(word_of(&hasher.to_bytes(), 9), 0, "no carry yet");
    }

    /// The carry is exact rather than a rollover: `t0` wrapping to zero bumps `t1` by one. Reached
    /// by deserializing a state whose counter is 128 bytes short of the top — `tweak_t0` alone
    /// cannot get here (it sets `u64::MAX`, which wraps to 127, not 0).
    #[test]
    fn the_counter_carries_into_t1_exactly_when_t0_wraps() {
        let mut bytes = Blake2b512Block::new(1).to_bytes();
        let short_of_the_top = u64::MAX - (BLOCK_LENGTH_BYTES as u64 - 1);
        bytes[64..72].copy_from_slice(&short_of_the_top.to_le_bytes());
        bytes[72..80].copy_from_slice(&7u64.to_le_bytes());

        let mut hasher = Blake2b512Block::from_bytes(&bytes).expect("80 bytes");
        hasher.update(&message(128), 0);

        let after = hasher.to_bytes();
        assert_eq!(word_of(&after, 8), 0, "t0 wrapped exactly to zero");
        assert_eq!(word_of(&after, 9), 8, "…and carried into t1");
    }

    /// `peek_final_root` and `finalize_internal` are *peeks*: they hash a block as if it were last
    /// without touching the running state, so the same hasher can be used again afterwards. They
    /// differ from each other because `root_finalize` sets a different finalization flag.
    #[test]
    fn the_finalizers_only_peek_at_the_state_and_differ_from_each_other() {
        let hasher = Blake2b512Block::new(1);
        let before = hasher.to_bytes();
        let msg = message(128);

        let mut root = vec![0u8; 64];
        hasher.peek_final_root(&msg, 0, &mut root, 0);
        let mut internal = vec![0u8; 32];
        hasher.finalize_internal(&msg, 0, &mut internal, 0);

        assert_eq!(hasher.to_bytes(), before, "neither call mutated the hasher");
        assert_ne!(
            root[..32],
            internal[..],
            "the root finalization flag changes the compression output"
        );
        assert_ne!(root[..32], before[..32], "…and both differ from the state");

        // Deterministic: the same inputs give the same outputs.
        let mut again = vec![0u8; 64];
        hasher.peek_final_root(&msg, 0, &mut again, 0);
        assert_eq!(again, root);
    }

    /// The 80-byte serialization round-trips exactly, and a short buffer is an `InvalidLength` that
    /// names both numbers rather than a panic. (A *longer* buffer is accepted: the read is bounded
    /// by the layout, so trailing bytes are ignored.)
    #[test]
    fn serialization_round_trips_and_rejects_a_short_buffer() {
        let mut hasher = Blake2b512Block::new(3);
        hasher.update(&message(128), 0);
        hasher.update(&message(128), 0);

        let bytes = hasher.to_bytes();
        let restored = Blake2b512Block::from_bytes(&bytes).expect("80 bytes");
        assert_eq!(restored, hasher);
        assert_eq!(restored.to_bytes(), bytes);

        let err = Blake2b512Block::from_bytes(&bytes[..79]).expect_err("79 bytes is short");
        assert_eq!(
            err,
            CryptoError::InvalidLength {
                expected: 80,
                actual: 79
            }
        );
        assert_eq!(
            Blake2b512Block::from_bytes(&[]).expect_err("empty"),
            CryptoError::InvalidLength {
                expected: 80,
                actual: 0
            }
        );

        let mut longer = bytes.clone();
        longer.extend_from_slice(&[0xAA; 8]);
        assert_eq!(
            Blake2b512Block::from_bytes(&longer).expect("the first 80 bytes"),
            hasher
        );
    }

    /// `tweak_t0` pins the counter at its maximum — the seam `Blake2b512Random::tweak_length0`
    /// needs to force the wraparound path. `debug_str` reports it, and the state stays a valid
    /// 80-byte serialization.
    #[test]
    fn tweak_t0_sets_the_counter_to_its_maximum() {
        let mut hasher = Blake2b512Block::new(1);
        hasher.tweak_t0();
        assert_eq!(word_of(&hasher.to_bytes(), 8), u64::MAX);

        // The next block wraps t0 to 127 (not 0), so this tweak does *not* carry.
        hasher.update(&message(128), 0);
        assert_eq!(word_of(&hasher.to_bytes(), 8), 127);
        assert_eq!(word_of(&hasher.to_bytes(), 9), 0);

        let mut state = Blake2b512Block::new(1);
        state.tweak_t0();
        let printed = state.debug_str();
        assert!(printed.starts_with("chainValue: "), "{printed}");
        assert!(printed.contains(&format!("\nt0: {}\n", u64::MAX)), "{printed}");
        assert!(printed.contains("\nt1: 0\n"), "{printed}");
    }

    /// The offset selects which 128 bytes of the message are compressed, so the same buffer at two
    /// offsets is two different blocks (this is how `Blake2b512Random` walks a transcript).
    #[test]
    fn update_compresses_the_block_at_the_offset_it_is_given() {
        let buf = message(256);
        let mut first = Blake2b512Block::new(1);
        first.update(&buf, 0);
        let mut second = Blake2b512Block::new(1);
        second.update(&buf, 128);

        assert_ne!(first, second);
        // The offset is not merely a shift of the same content: block 0 ≠ block 1 of this message.
        let mut replayed = Blake2b512Block::new(1);
        replayed.update(&buf[..128], 0);
        assert_eq!(first, replayed);
    }

    /// A message shorter than the 128-byte block it claims to be is a **caller** error, and it
    /// panics on the slice bound rather than silently compressing whatever is adjacent in memory.
    /// The contract is the doc on `u64_from_le_at`; this pins that it is enforced.
    #[test]
    #[should_panic]
    fn a_truncated_message_block_panics_rather_than_reading_past_it() {
        let mut hasher = Blake2b512Block::new(1);
        hasher.update(&message(127), 0);
    }
}
