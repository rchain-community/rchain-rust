//! Law 14a's `stake` layer — the node's own `is_super_majority` on the same pairs.
//!
//! `spec/conformance/stake.tsv` is emitted by `lake exe rchain-corpus --layer stake` from
//! `Rchain/Corpus.lean`'s `stakeCases`, where each row's verdict is `decide`d against the model's
//! `isSuperMajority` (`Rchain/Casper/Stake.lean`). This file is the other party: it reads the same
//! `(stake, total)` pair and calls `rchain_sdk::consensus::is_super_majority` — the port's own exact
//! integer comparison, `3 * stake > 2 * total` in `i128`.
//!
//! **What the layer is about.** The comparison is *exact*: no division, no rounding, no floating point,
//! and the boundary is exactly `3 * stake = 2 * total` — not a supermajority — with one unit over it
//! being one. So the corpus is the boundary family plus two pairs wide enough that a 64-bit accumulator
//! would have decided them wrongly (`3 * stake` past `i64` in one, `stake` past it in the other), which
//! is why the signature takes `i128` and the model's takes `Nat`. The Rust side parses the pair as
//! `i128` for the same reason.
//!
//! The two sides never compare representations — each computes its own verdict on the same numbers, and
//! the numbers are what ties them.

use rchain_sdk::consensus::is_super_majority;

/// The corpus's declared size (`Rchain/Corpus.lean`'s `stakeCaseCount`).
const STAKE_CASES: usize = 12;

#[test]
fn the_node_agrees_with_the_model_on_every_supermajority_pair() {
    let path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../spec/conformance/stake.tsv");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "read {}: {e}\n(run tools/emit-lean-corpus.sh)",
            path.display()
        )
    });

    let mut cases = 0usize;
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let mut columns = line.split('\t');
        let layer = columns.next().unwrap_or_default();
        assert_eq!(
            layer,
            "stake",
            "corpus line {}: unexpected layer {layer:?}",
            i + 1
        );
        let stake: i128 = columns
            .next()
            .expect("the stake column")
            .parse()
            .unwrap_or_else(|_| panic!("corpus line {}: stake is not an integer", i + 1));
        let total: i128 = columns
            .next()
            .expect("the total column")
            .parse()
            .unwrap_or_else(|_| panic!("corpus line {}: total is not an integer", i + 1));
        let expected: bool = columns
            .next()
            .expect("the verdict column")
            .parse()
            .unwrap_or_else(|_| panic!("corpus line {}: verdict is not a bool", i + 1));
        assert!(
            columns.next().is_none(),
            "corpus line {}: trailing columns",
            i + 1
        );

        assert_eq!(
            is_super_majority(stake, total),
            expected,
            "stake {stake} of {total}: the node says {}, the Lean model says {expected} \
             (spec/conformance/stake.tsv, law 14a). The comparison is `3 * stake > 2 * total` exactly, \
             so a disagreement at the boundary is an off-by-one in one of the two spellings.",
            is_super_majority(stake, total)
        );
        cases += 1;
    }

    assert_eq!(
        cases, STAKE_CASES,
        "the corpus carries {STAKE_CASES} cases (Rchain/Corpus.lean's stakeCaseCount); {cases} were read"
    );
}
