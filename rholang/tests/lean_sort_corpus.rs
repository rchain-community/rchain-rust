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
//! arity, an arithmetic node's operand order, the send channel arm — and, from case 13 on, the four
//! structures where this corpus's first run showed the model and the node **disagreeing**: a send's
//! field order, the par's own field order, the expression-class order and `Ground.bool`'s polarity.
//! The model was corrected to the node's score tags and those rows are its falsifiers.
//!
//! `the_boundary_the_model_cannot_pin` below is not an assertion: it prints the node's answer for the
//! pairs that are *outside* the model's algebra (the twelve `Expr` constructors the model does not
//! have, and `Ground.bytes`, which the node scores at tag 116 rather than with the grounds) — the
//! scope of the alignment, kept visible rather than implied.

use rchain_models::ast::{Par, Proc};
use rchain_rholang::normalizer::source_to_adt;

/// The corpus's declared size (`Rchain/Corpus.lean`'s `sortCaseCount`).
const SORT_CASES: usize = 19;

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

        // The equality case is its own: the two sides are the same term, so the sort cannot separate
        // them and the row says `eq`.
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

        // **The read is only exact if the pair's *scores* differ.** `sort_by` is stable, so two pars
        // with equal scores come back in the order they went in — and "the first element is the
        // smaller" would then answer `lt` for *both* directions. That is the same quotient trap the
        // pairwise design exists to avoid, one level down (the score, not the canonical form). Reading
        // the pair in both orders and requiring opposite answers is exactly the test that the two
        // scores differ: a lawful comparator gives opposite answers iff it does not call them equal.
        if expected != "eq" {
            let backwards = node_verdict(&right_par, &left_par);
            let want = if expected == "lt" { "gt" } else { "lt" };
            assert_eq!(
                backwards, want,
                "{left} vs {right}: the node reads {expected} one way and {backwards} the other, so the \
                 two scores are equal and the stable sort is answering with its input order — the row \
                 is passing vacuously. Its terms must be made to differ in a score, not just in \
                 spelling."
            );
        }

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

/// **The boundary, kept visible rather than implied.** The model's comparator family is an order on a
/// *coarser* algebra than the node's score tree: 21 `Expr` constructors against the node's 33, so a
/// term containing `EMethod`/`EMatches`/`EShortAnd`/`GBigInt`/… cannot be built here at all, and
/// `Ground.bytes` — which does exist — is scored by the node as `EBYTEARR`(116), *after* every var and
/// operator, rather than with the grounds where this model puts it. Those are the terms the alignment
/// cannot reach; this test prints what the node answers for them so the gap is a measurement rather
/// than a claim, and asserts nothing, because a corpus row needs a verdict the model can `decide`.
///
/// A byte array is the sharper case: the model *has* `Ground.bytes`, but the node's front end has no
/// byte-array literal (`@"c"!(b"a")` is a `SyntaxError: expected RParen`), so no corpus row can carry
/// one — the term is unspellable, not merely mis-scored. (Tried here first; that is why this file no
/// longer probes it.)
///
/// The pairs that *were* in this position (the send/par field orders, the expression-class order,
/// `Ground.bool`'s polarity) are now corpus rows 13–19, with the model aligned to them.
#[test]
fn the_boundary_the_model_cannot_pin() {
    let pairs: &[(&str, &str)] = &[
        // the operator tags the model does not have: `EMATCHES`(118) / `EPERCENTPERCENT`(119) sit
        // between `EOR`(114) and `EMOD`(122), so a row could only pin their *relative* position — and
        // these spellings parse as a method call, not as the expression the model would need.
        ("@\"c\"!(1 && 2)", "@\"c\"!(1 || 2)"),
        ("@\"c\"!(1 %% 2)", "@\"c\"!(1 % 2)"),
        // the remaining argument positions *within* the model's algebra, for contrast: grounds first,
        // then collections in tag order, then the operators.
        ("@\"c\"!(1 + 2)", "@\"c\"!(1 == 2)"),
        ("@\"c\"!(1 / 2)", "@\"c\"!(1 + 2)"),
        ("@\"c\"!(1 < 2)", "@\"c\"!(1 == 2)"),
        ("@\"c\"!(1)", "@\"c\"!(\"s\")"),
        ("@\"c\"!([1])", "@\"c\"!((1, 2))"),
        ("@\"c\"!([1])", "@\"c\"!({})"),
        ("@\"c\"!(1)", "@\"c\"!(1 + 2)"),
        ("@\"c\"!(-1)", "@\"c\"!(1 * 2)"),
        ("@\"c\"!(false)", "@\"c\"!(true)"),
        // the receive and the bundle: the node's `sort_receive` puts `persistent`/`peek` *before* the
        // binds and adds a `bind_count` child, neither of which the model's `cmpReceive` has.
        ("for (x <- @\"c\") { 0 }", "@\"c\"!(1)"),
        ("bundle+{ 1 }", "1"),
    ];
    for (left, right) in pairs {
        let l = normalized(left);
        let r = normalized(right);
        let got = if l == r { "eq" } else { node_verdict(&l, &r) };
        println!("node: {left}  vs  {right}  ->  {got}");
    }
}
