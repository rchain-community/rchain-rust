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
/// hand the node both and ask *it* to sort them — `sort_pars` is the node's own list sort, driven by
/// the same score tree `sort_par` uses — and see which landed first. `lt` means `left` came first.
///
/// This works for *any* two terms, not only single sends, which matters: the par's own field order
/// (which of its eight lists is compared first) is only reachable by comparing two pars with different
/// field structure, and canonicalization would hide that inside a single par.
fn node_verdict(left: &Par, right: &Par) -> &'static str {
    let sorted = rchain_models::sorter::sort_pars(vec![left.clone(), right.clone()]);
    let first = sorted.first().expect("two pars sort to two pars");
    if first == left {
        "lt"
    } else if first == right {
        "gt"
    } else {
        panic!("neither input is at index 0: the sorted list is not a permutation of its input");
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
    // Every pair here is a *candidate corpus row*: the node's answer is what the model must be
    // aligned to, so this prints the node's verdict for each and nothing is asserted until the model
    // matches.
    let pairs: &[(&str, &str)] = &[
        // the field order: persistence first (score) against channel first (the model's `cmpSend`).
        ("@\"a\"!!(1)", "@\"b\"!(1)"),
        // the expression-class order: the tags put every collection *before* the vars and operators.
        ("@\"c\"!([1])", "@\"c\"!(1 + 2)"),
        ("@\"c\"!(Set(1))", "@\"c\"!(1 + 2)"),
        ("@\"c\"!([1])", "@\"c\"!(\"s\")"),
        // the arithmetic tags: EMULT=102 < EDIV=103 < EPLUS=104 < EMINUS=105 < ELT=106 < … < EEQ=110.
        ("@\"c\"!(1 * 2)", "@\"c\"!(1 + 2)"),
        ("@\"c\"!(1 - 2)", "@\"c\"!(1 * 2)"),
        ("@\"c\"!(1 + 2)", "@\"c\"!(1 == 2)"),
        ("@\"c\"!(1 / 2)", "@\"c\"!(1 + 2)"),
        ("@\"c\"!(1 < 2)", "@\"c\"!(1 == 2)"),
        // agreeing pairs, for contrast: grounds first, then collections in tag order.
        ("@\"c\"!(1)", "@\"c\"!(\"s\")"),
        ("@\"c\"!([1])", "@\"c\"!((1, 2))"),
        ("@\"c\"!([1])", "@\"c\"!({})"),
        ("@\"c\"!(1)", "@\"c\"!(1 + 2)"),
        ("@\"c\"!(-1)", "@\"c\"!(1 * 2)"),
        // the **par's own field order**: a par with `news` and `exprs` against a par with `exprs`
        // only. The node's `sort_par` gathers exprs *before* news; the model's `cmpPar` compares news
        // first — so the two orders disagree exactly here, and this pair is the only kind that can see
        // it (canonicalization hides it inside a single par).
        ("new x in { Nil } | 1", "[1]"),
        ("[1]", "new x in { Nil } | 1"),
        // `GBool`'s **polarity**: the node scores `true` as 0 and `false` as 1, so `false` sorts
        // *after* `true` — the reverse of the model's `linearOrderComparator Bool`.
        ("@\"c\"!(false)", "@\"c\"!(true)"),
        ("@\"c\"!(true)", "@\"c\"!(false)"),
        // the bool/int/str/uri order within the grounds (tags 1,2,3,4).
        ("@\"c\"!(true)", "@\"c\"!(1)"),
        ("@\"c\"!(1)", "@\"c\"!(\"s\")"),
        // the remaining argument positions: `eand`(113) < `eor`(114) < `emod`(122) in the tags, while
        // the model's declaration order puts `emod` before both.
        ("@\"c\"!(1 % 2)", "@\"c\"!(1 && 2)"),
        ("@\"c\"!(1 && 2)", "@\"c\"!(1 || 2)"),
        ("@\"c\"!(1 %% 2)", "@\"c\"!(1 % 2)"),
    ];
    for (left, right) in pairs {
        let l = normalized(left);
        let r = normalized(right);
        let got = if l == r { "eq" } else { node_verdict(&l, &r) };
        println!("node: {left}  vs  {right}  ->  {got}");
    }
}
