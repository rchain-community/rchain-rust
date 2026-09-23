//! Law 1 — the canonical order, checked **pairwise** against the Lean model.
//!
//! `spec/conformance/sort.tsv` is emitted by `lake exe rchain-corpus --layer sort` from
//! `spec/Rchain/Corpus.lean`, where each row's verdict is a `decide`d fact about the model: `cmpPar`
//! of the two terms answers the row's verdict (`sortCases_decide`), and all three verdicts appear
//! (`sortCases_verdicts`), so a table that answered one ordering everywhere fails the build.
//!
//! **Why pairwise, and why the first design for this layer was vacuous.** A corpus of "the sorted
//! spelling of this par" cannot tie a comparator: the AST's fields are *canonical* values, so both the
//! node's output and the expected spelling are re-sorted by the same comparator before anything is
//! compared, and any total order satisfies the assertion. Canonicalization *is* the quotient by the
//! comparator, and a quotient cannot distinguish two comparators that induce the same equality. So each
//! row here is a *pair* and a verdict, and the node's answer is read from the only place it is
//! observable — which element `sort_par` puts first — where canonicalization cannot hide it, because
//! sorting a two-element par is exactly the comparison.
//!
//! The cases pin the arms a port can get wrong silently: the leaf arms, the **ground constructor
//! order** (`bool < int < str`), the **collection constructor order** (`elist < etuple < eset`), list
//! arity, an arithmetic node's operand order, the send channel arm, and the pair whose field order the
//! model and the score tree could disagree about (see `the_field_order_the_two_orders_would_disagree_on`
//! below, which prints the node's answer rather than asserting it while the question is open).

use rchain_models::ast::{Par, Proc};
use rchain_models::sorter::sort_par_term;
use rchain_rholang::normalizer::source_to_adt;

/// The corpus's declared size (`Rchain/Corpus.lean`'s `sortCaseCount`).
const SORT_CASES: usize = 12;

/// Parse and normalize a closed term, as the node does on the deploy path.
fn normalized(source: &str) -> Proc {
    let closed = source_to_adt(source)
        .unwrap_or_else(|e| panic!("{source}: parse + normalize failed: {e:?}"));
    let par: Par = closed.into();
    par
}

/// **Which of the two the node's canonical order puts first**, read the only way it is observable:
/// put both in one `par`, sort, and see which element landed at index 0. `lt` means `left` came first.
fn node_verdict(left: &Par, right: &Par) -> &'static str {
    let mut both = left.clone();
    both.sends.extend(right.sends.iter().cloned());
    let sorted = sort_par_term(&both);
    let first = sorted
        .sends
        .first()
        .expect("a two-send par sorts to a two-send par");
    // Both sides are single sends, so `sends[0]` identifies the side that came first.
    if *first == left.sends[0] {
        "lt"
    } else if *first == right.sends[0] {
        "gt"
    } else {
        panic!("neither side's send is at index 0: the sorted form is not one of the inputs");
    }
}

#[test]
fn the_canonical_order_is_the_lean_models_pairwise() {
    let path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../spec/conformance/sort.tsv");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "read {}: {e}\n(run tools/emit-lean-corpus.sh)",
            path.display()
        )
    });

    let mut cases = 0usize;
    let mut verdicts: Vec<&str> = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let mut columns = line.split('\t');
        let layer = columns.next().unwrap_or_default();
        assert_eq!(
            layer,
            "sort",
            "corpus line {}: unexpected layer {layer:?}",
            i + 1
        );
        let left = columns.next().expect("the left term");
        let right = columns.next().expect("the right term");
        let expected = columns.next().expect("the verdict");
        assert!(
            columns.next().is_none(),
            "corpus line {}: trailing columns",
            i + 1
        );

        let left_par = normalized(left);
        let right_par = normalized(right);

        // Both sides are single sends, which is what makes the sort-based read of the verdict exact:
        // canonicalizing a one-element field is the identity, so nothing is reordered before the
        // comparison the row is about.
        assert_eq!(left_par.sends.len(), 1, "{left}: expected one send");
        assert_eq!(right_par.sends.len(), 1, "{right}: expected one send");

        // The equality case is its own: the two sides are the same term, and `sends[0]` then matches
        // both, so it is decided before the positional read.
        let got = if left_par == right_par {
            "eq"
        } else {
            node_verdict(&left_par, &right_par)
        };
        assert_eq!(
            got, expected,
            "{left} vs {right}: the node's canonical order says {got}, the Lean model's comparator says \
             {expected} (spec/conformance/sort.tsv, law 1). The two orders are the thing this corpus \
             exists to tie: the model's comparator is `cmpPar` in Rchain/Sort.lean, the node's is the \
             score tree `sort_par` sorts by, and a disagreement is a state-hash divergence between two \
             nodes that canonicalize the same block."
        );
        verdicts.push(expected);
        cases += 1;
    }

    assert_eq!(
        cases, SORT_CASES,
        "the corpus carries {SORT_CASES} cases (Rchain/Corpus.lean's sortCaseCount); {cases} were read"
    );

    // The corpus's own non-degeneracy, mirrored from the Lean side (`sortCases_verdicts`): all three
    // verdicts must appear, or a table that answered one ordering everywhere would pass.
    verdicts.sort_unstable();
    verdicts.dedup();
    assert_eq!(
        verdicts.len(),
        3,
        "the table carries {} distinct verdicts, so it does not exercise the order's three answers",
        verdicts.len()
    );
}

/// **An open question, made checkable rather than asserted.** The model's `cmpSend` compares a send's
/// *channel* first (`Rchain/Sort.lean`'s `cmpSend` is a `lex` over channel, data, persistence), while
/// the node's score tree builds a send's score with its children in the order
/// `[persistent, chan, data…, connective_use]` (`models/src/sorter.rs`'s `sort_send`) — so for a pair
/// whose persistence *and* channel orderings disagree, the two orders can differ. This test prints what
/// the node answers for such a pair; it is not an assertion, because which side is right is a question
/// for the corpus's own reference (`ScoreTree.scala`, which the port mirrors) rather than for this
/// file. When the answer is settled the pair belongs in the corpus proper, with the model corrected if
/// it is the model that is wrong.
#[test]
fn the_field_order_the_two_orders_would_disagree_on() {
    // `@"a"!!(1)` is persistent on the smaller channel; `@"b"!(1)` is not persistent on the larger one.
    let persistent_a = normalized("@\"a\"!!(1)");
    let plain_b = normalized("@\"b\"!(1)");
    let got = node_verdict(&persistent_a, &plain_b);
    println!(
        "node: persistent @\"a\" vs plain @\"b\" -> {got}; the model's comparator says lt \
         (channel first), the score tree's child order says gt (persistence first)"
    );
}
