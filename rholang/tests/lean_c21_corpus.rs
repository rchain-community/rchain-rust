//! Law 34 — the value-position rule, checked 1:1 against the Lean model (AUDIT C21).
//!
//! `spec/conformance/c21.tsv` is emitted by `lake exe rchain-corpus --layer c21` from
//! `spec/Rchain/Corpus.lean`, where each case's verdict is a `decide`d fact about the model: the `Match`
//! an `if` desugars into has *the condition*, normalized alone, as its target. This test parses both
//! source strings with the node's own parser and normalizer and asserts the same relation of the node.
//!
//! Why this rule, and why a corpus rather than a proof: `normalizeAt` in the model threads only a binder
//! stack, so there is no accumulated `par` to misuse and the rule cannot be *falsified* there — the
//! model's half is true by construction. What C21 was is a defect in the node's `normalize_if`, which
//! seeded the condition with the caller's `input.par` (the terms preceding the `if` in its `|`-chain), so
//! the `Match` target became `P | E` rather than `E`; the pattern cases are `true`/`false`, so a target
//! that is neither matches nothing, and the `if` reduced to **nothing** — silently, because an unmatched
//! `match` is not an error. Invisible for an `if` in first position, which is the idiom most contracts
//! use, which is how it survived two implementations (`MemberDirectory.rho:78` is a non-first `if`).
//!
//! So the tie is the check: the model says the relation holds of its own normalization, the node must
//! say so of its own, and the two sides never compare representations across the boundary — what they
//! share is the source text. Under the defect the node's half of case 1 fails while the model's stays
//! true, which is the shape a corpus is for.

use rchain_models::ast::{Par, Proc};
use rchain_rholang::normalizer::source_to_adt;

/// The corpus's declared size (`Rchain/Corpus.lean`'s `c21CaseCount`). A corpus that silently shrinks is
/// a check that stopped checking, so the count is pinned on both sides.
const C21_CASES: usize = 5;

/// Parse and normalize a closed term, as the node does on the deploy path.
fn normalized(source: &str) -> Proc {
    let closed = source_to_adt(source)
        .unwrap_or_else(|e| panic!("{source}: parse + normalize failed: {e:?}"));
    let par: Par = closed.into();
    par
}

#[test]
fn the_value_position_rule_agrees_with_the_lean_model() {
    let path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../spec/conformance/c21.tsv");
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
            "c21",
            "corpus line {}: unexpected layer {layer:?}",
            i + 1
        );
        let source = columns.next().expect("the term column");
        let condition = columns.next().expect("the condition column");
        assert!(
            columns.next().is_none(),
            "corpus line {}: trailing columns",
            i + 1
        );

        let whole = normalized(source);
        let condition_par = normalized(condition);

        // The `if` (or the `match`, in the control) desugars to one `Match` at the top of the term's
        // par chain: no `new` wrapper is needed, because every case is a closed term that names its
        // channels by quoted ground — a free variable would be refused by the normalizer, not this test.
        assert_eq!(
            whole.matches.len(),
            1,
            "{source}: expected the desugared `Match` at the top of the term, found {} matches",
            whole.matches.len()
        );

        // The rule: the target is the condition, normalized alone — **not** the condition preceded by
        // the terms before the `if` in its par, which is what C21 put there.
        let target: Proc = (*whole.matches[0].target).clone().eval();
        assert_eq!(
            target, condition_par,
            "{source}: the desugared `Match` target is not the condition \
             (spec/conformance/c21.tsv, law 34). A target that is the condition *beside the terms \
             preceding the `if`* matches neither case, so the `if` reduces to nothing — silently, \
             which is AUDIT C21.",
        );

        // Non-degeneracy, the corpus's own guard: the case is a probe, so "the target is the condition"
        // is not true of the whole term by accident.
        assert_ne!(
            whole, condition_par,
            "{source}: the term normalizes to its own condition, so the case above proves nothing"
        );

        cases += 1;
    }

    assert_eq!(
        cases, C21_CASES,
        "the corpus carries {C21_CASES} cases (Rchain/Corpus.lean's c21CaseCount); {cases} were read"
    );
}
