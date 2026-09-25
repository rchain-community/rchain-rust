//! Law 6's `closed` layer — the node's own `is_closed` on the same source text.
//!
//! `spec/conformance/closed.tsv` is emitted by `lake exe rchain-corpus --layer closed` from
//! `Rchain/Corpus.lean`'s `closedCases`, where each case's verdict is `decide`d against the model's
//! `closed` checker. This file is the other party: it parses each row's source with the node's own
//! parser, normalizes it with the node's own normalizer, and asks `models::types::is_closed`
//! (`models/src/types.rs:371`) — the same predicate `Closed::new` refuses on rather than panicking. The
//! two sides never compare representations: each computes its own verdict on the same text, and the
//! text is what ties them.
//!
//! **Both directions are read from the same call, and the refusal is not an obstacle.** The node's
//! top-level normalizer *rejects* a term with a free variable — `TopLevelFreeVariablesNotAllowedError`,
//! which is exactly the right behaviour for a deploy — and the error **carries the normalized `Par`** it
//! refused. So a row whose verdict is `false` is checked against that par (`is_closed` must say `false`
//! of it), and a row whose verdict is `true` against the successful normalization. Reading the verdict
//! off the error rather than around it is what keeps the layer about the node's *predicate* rather than
//! about its *entry point*.

use rchain_models::ast::Par;
use rchain_models::types::is_closed;
use rchain_rholang::errors::RholangError;
use rchain_rholang::normalizer::source_to_adt;

/// The corpus's declared size (`Rchain/Corpus.lean`'s `closedCaseCount`). A corpus that shrinks
/// silently is a check that stopped checking, so the consumer refuses to read fewer rows than the
/// model says it wrote.
const CLOSED_CASES: usize = 10;

#[test]
fn the_node_agrees_that_these_terms_are_closed_exactly_when_the_model_does() {
    let path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../spec/conformance/closed.tsv");
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
            "closed",
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

        let node_says = match source_to_adt(source) {
            Ok(closed) => {
                let par: Par = closed.into();
                is_closed(&par)
            }
            // The node's top-level normalizer refuses a free variable — and hands back the par it
            // refused, which is the term `is_closed` is a predicate about.
            Err(RholangError::TopLevelFreeVariablesNotAllowedError(par)) => is_closed(&par),
            Err(e) => panic!("{source}: the node could not normalize it: {e:?}"),
        };

        assert_eq!(
            node_says, expected,
            "{source}: the node's is_closed says {node_says}, the Lean model says {expected} \
             (spec/conformance/closed.tsv, law 6). The predicate is what `Closed::new` refuses on, so a \
             disagreement is a term the model admits and the port rejects — or the reverse."
        );
        cases += 1;
    }

    assert_eq!(
        cases, CLOSED_CASES,
        "the corpus carries {CLOSED_CASES} cases (Rchain/Corpus.lean's closedCaseCount); {cases} were read"
    );
}
