//! Law 35 — the concreteness flag, checked 1:1 against the Lean model.
//!
//! `spec/conformance/flags.tsv` is emitted by `lake exe rchain-corpus --layer flags` from
//! `spec/Rchain/Corpus.lean`, where each case's verdict is a `decide`d theorem about the model. This
//! test parses the same source with the node's own parser and normalizer and asserts the flag the
//! normalizer produced equals the model's verdict.
//!
//! Why this flag is the one to check: `spatial_match`'s first line is
//! `if !pattern.connective_used { pattern == target }`. A pattern whose flag is *wrong* is therefore
//! not a slower match, it is a different one — and when the flag says "concrete" for a pattern that is
//! not, the pattern matches nothing and says nothing (AUDIT C19/C20/C22's whole failure mode).
//!
//! The pattern is normalized the way a consumer writes one — as a receive bind (`for (<pattern> <- ch)`)
//! — rather than through a synthetic wrapper: the shape under test is the shape that runs.

use rchain_models::ast::Par;
use rchain_rholang::normalizer::source_to_adt;

/// The corpus's declared size (`Rchain/Corpus.lean`'s `flagCaseCount`). A corpus that silently shrinks
/// is a check that stopped checking, so the count is pinned on both sides.
const FLAG_CASES: usize = 17;

#[test]
fn the_connective_used_flag_agrees_with_the_lean_model() {
    let path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../spec/conformance/flags.tsv");
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
            "flags",
            "corpus line {}: unexpected layer {layer:?}",
            i + 1
        );
        let source = columns.next().expect("the source column");
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

        let term = format!("new chan in {{ for ({source} <- chan) {{ Nil }} }}");
        let closed = source_to_adt(&term)
            .unwrap_or_else(|e| panic!("{source}: parse + normalize failed: {e:?}"));
        let par: Par = closed.into();
        // The term is `new chan in { … }`, so the receive sits inside the binder's body.
        let inner = par
            .news
            .first()
            .map(|n| (*n.p).clone())
            .expect("the wrapper declares one `new`");
        let pattern = &inner.receives[0].binds[0].patterns[0];

        assert_eq!(
            pattern.connective_used, expected,
            "{source}: the normalizer produced connective_used = {}, the Lean model says {expected} \
             (spec/conformance/flags.tsv, law 35). This flag is what `spatial_match` consults first, \
             so a disagreement is a pattern that silently matches nothing.",
            pattern.connective_used
        );
        cases += 1;
    }

    assert_eq!(
        cases, FLAG_CASES,
        "the corpus carries {FLAG_CASES} cases (Rchain/Corpus.lean's flagCaseCount); {cases} were read"
    );
}
