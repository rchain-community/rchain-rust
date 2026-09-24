//! Laws 30, 31 and 33 — the grammar, the parser's deviations and the printer's round trip, checked
//! 1:1 against the Lean model, through the node's own parser and printer.
//!
//! `spec/conformance/parse.tsv` is emitted by `lake exe rchain-corpus --layer parse` from
//! `spec/Rchain/Corpus.lean`, where every verdict is a `decide`d theorem about the *grammar* and the
//! deviation list (`Rchain/Parse.lean`: `grammarFragment`, `derives`, `parseDeviations`,
//! `parseCases_decide`). This file is the other party:
//!
//! - **The model's half is `derives`** — the grammar's list-bearing productions as data — which is
//!   what makes law 30's *soundness* direction ("every term the parser accepts is in the grammar")
//!   falsifiable: a `refused` row is a spelling neither derivable nor in the deviation list, so a port
//!   that grew more permissive fails it. The `accept` direction is law 31's completeness, and the
//!   `deviation` rows are the port's deliberate laxnesses (AUDIT C31) and refusals (law 32's table).
//! - **A `decide`d corpus is the model agreeing with itself.** The source column is *rendered from the
//!   model's own tokens* (`Rchain/Print.lean`'s `renderTokens`), so the row cannot carry a spelling its
//!   verdict was not decided from — and the node, not the model, is what answers here.
//! - **The source is the only thing shared.** Nothing in this file knows what the model believes: it
//!   parses, prints and compares, and the corpus's claim is what it is checked against.
//!
//! The two tests are the layer's two halves: `the_node_parser_agrees_with_the_lean_model` (laws 30/31)
//! and `the_printers_output_round_trips_through_the_node` (law 33, C13's regression) — the latter over
//! the `printer` rows, which are `Rchain/Print.lean`'s spelling of every production witness the
//! fragment can read.

use rchain_models::ast::{Expr, Par};
use rchain_rholang::normalizer::source_to_adt;
use rchain_rholang::parser::parse;
use rchain_rholang::pretty_printer::PrettyPrinter;

/// The corpus's declared size (`Rchain/Parse.lean`'s `parseCaseCount`). A corpus that shrinks silently
/// is a check that stopped checking, so the count is pinned on both sides.
const PARSE_CASES: usize = 123;

/// The corpus's four halves, each pinned (`parseCases_by_kind` in `Rchain/Parse.lean`). Pinning them
/// separately is what makes a *specific* list's shrinkage visible: `refused` is law 30's soundness
/// direction, and it is the half a more permissive parser would empty.
const DERIVABLE_CASES: usize = 17;
const REFUSED_CASES: usize = 11;
const DEVIATION_CASES: usize = 14;
const PRINTER_CASES: usize = 81;

/// How the `printer`+`accept` rows are accounted for. Many of the witness table's rows spell their
/// variables as bare names (`x`, `c`, `x /\ y`) and the normalizer rejects a globally free variable, so
/// they never reach the printer; the rows that do reach it either round-trip or are one of the port's
/// warts. Every count is pinned and they must *sum* to the `printer`+`accept` rows, so a row that stops
/// normalizing moves the arithmetic instead of disappearing.
const PRINTER_ROUND_TRIPPED: usize = 43;
const PRINTER_URN_WARTED: usize = 1;
const PRINTER_TUPLE_WARTED: usize = 1;
const PRINTER_NOT_WARTED: usize = 0;

/// The corpus path, with the emitter named in the failure — a missing corpus is a missing
/// `tools/emit-lean-corpus.sh`, not a test bug.
fn corpus() -> String {
    let path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../spec/conformance/parse.tsv");
    std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "read {}: {e}\n(run tools/emit-lean-corpus.sh)",
            path.display()
        )
    })
}

/// One row: the source the model's tokens spell, the verdict the node must give, and which half of
/// the layer it belongs to.
struct Row {
    source: String,
    accept: bool,
    kind: String,
}

fn rows() -> Vec<Row> {
    let text = corpus();
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let mut columns = line.split('\t');
        let layer = columns.next().unwrap_or_default();
        assert_eq!(
            layer,
            "parse",
            "corpus line {}: unexpected layer {layer:?}",
            i + 1
        );
        let source = columns.next().expect("the source column").to_string();
        let verdict = columns.next().expect("the verdict column");
        assert!(
            matches!(verdict, "accept" | "reject"),
            "corpus line {}: unknown verdict {verdict:?}",
            i + 1
        );
        let kind = columns.next().expect("the kind column").to_string();
        assert!(
            matches!(
                kind.as_str(),
                "derivable" | "refused" | "deviation" | "printer"
            ),
            "corpus line {}: unknown kind {kind:?}",
            i + 1
        );
        assert!(
            columns.next().is_none(),
            "corpus line {}: trailing columns",
            i + 1
        );
        out.push(Row {
            source,
            accept: verdict == "accept",
            kind,
        });
    }
    out
}

/// **Laws 30 and 31.** The node's parser answers each row, and the model's verdict must be what it
/// answers. This is the layer's whole point: a spelling the grammar neither derives nor lists as a
/// deviation must be *refused*, and one it derives must be *accepted*.
#[test]
fn the_node_parser_agrees_with_the_lean_model() {
    let rows = rows();
    let mut counts = [0usize; 4];
    for row in &rows {
        let parsed = parse(&row.source);
        let accepted = parsed.is_ok();
        let why = match &parsed {
            Ok(_) => String::new(),
            Err(e) => format!("\n  the node's error: {e}"),
        };
        let expected = row.accept;
        assert_eq!(
            accepted, row.accept,
            "{:?} (a `{}` row): the node says {accepted}, the Lean model says {expected}{why}\n  \
             An `accept` row the node refuses is law 31's completeness direction — the grammar \
             derives the spelling and the port does not, which belongs in `Rchain/Parse.lean`'s \
             deviation list as a `refuses` row. A `reject` row the node accepts is law 30's \
             soundness direction — the port is more permissive than the grammar and the deviation \
             list, which is what AUDIT C24's residual *claimed* about `[1 ..._]` before the \
             derivation was read right: the comma-less form is derivable and `[1, ..._]` is C31's \
             registered deviation (spec/conformance/parse.tsv, laws 30/31)",
            row.source, row.kind
        );
        let i = match row.kind.as_str() {
            "derivable" => 0,
            "refused" => 1,
            "deviation" => 2,
            _ => 3,
        };
        counts[i] += 1;
    }

    assert_eq!(
        counts,
        [
            DERIVABLE_CASES,
            REFUSED_CASES,
            DEVIATION_CASES,
            PRINTER_CASES
        ],
        "the corpus's four halves are derivable/refused/deviation/printer (Rchain/Parse.lean's \
         parseCases_by_kind); the rows read were {counts:?}"
    );
    assert_eq!(
        rows.len(),
        PARSE_CASES,
        "the corpus carries {PARSE_CASES} cases (Rchain/Parse.lean's parseCaseCount); {} were read",
        rows.len()
    );
}

/// **Law 33, and C13's regression.** For every `printer` row the fragment can read: parse and
/// normalize it, print it with the node's own printer, and parse the result again — the two normalized
/// terms must be the same. C13's two defects (a doubled group separator, a dropped `bundle` keyword)
/// are exactly what this fails on, and the port's own `printing_and_reparsing_is_the_identity` pins
/// the same property over 37 hand-written terms; this test pins it over the *model's* spellings, which
/// is what ties the printer to the corpus.
///
/// **The port's printer's warts, one detector each.** A detector answers `true` when the difference
/// between a term and its reprint is *exactly* the loss that wart names. `None` for the reprint means
/// the node could not read it back at all, which one wart's signature is: the urn detector requires
/// the cleared `New.uri`/`injections` to be the whole difference, the tuple one requires the reprint to
/// be the element the tuple wrapped, and the `not` one requires the reprint to be *unreadable* — the
/// port's `~(x)` is not a term, and C13's own test asserts the same of the printed form. That is what
/// makes `Rchain/Print.lean`'s `printWarts` a claim rather than a list of excuses: a row that differs
/// for any other reason fails the test, and the wart section below asserts each detector fires on a
/// term of its own.
fn urn_wart(first: &Par, reprinted: Option<&Par>) -> bool {
    let Some(second) = reprinted else {
        return false;
    };
    // The wart must have a urn to lose: clearing is the *difference* only if the term had one. Without
    // this the detector fires on any differing term whose `news` are all urn-less — which is every
    // other wart's row, and a detector that fires on everything detects nothing.
    if !first
        .news
        .iter()
        .any(|n| !n.uri.is_empty() || !n.injections.is_empty())
    {
        return false;
    }
    let mut loosened = first.clone();
    let mut tightened = second.clone();
    for n in loosened.news.iter_mut() {
        n.uri.clear();
        n.injections.clear();
    }
    for n in tightened.news.iter_mut() {
        n.uri.clear();
        n.injections.clear();
    }
    tightened == loosened && first != second
}

/// The one-element tuple: the port's printer prints `(1,)` as `(1)`, so the reprint is the element the
/// tuple wrapped — and the tuple is gone.
fn tuple_wart(first: &Par, reprinted: Option<&Par>) -> bool {
    let Some(second) = reprinted else {
        return false;
    };
    match first.exprs.as_slice() {
        [Expr::ETuple(t)] if t.ps.len() == 1 => t.ps[0] == *second,
        _ => false,
    }
}

/// The `not` spelling: the port's printer prints `not x` as `~(x)`, which the node **cannot read back
/// at all** — `parser.rs`'s group fallback demands a comma, so `~(true)` is "a tuple needs a comma",
/// and C13's own test asserts the same of the printed form. So the term had the expression `ENot` and
/// its reprint is unreadable.
fn not_wart(first: &Par, reprinted: Option<&Par>) -> bool {
    first.exprs.iter().any(|e| matches!(e, Expr::ENot(_))) && reprinted.is_none()
}

/// The `printer` rows whose term is not closed are accounted for rather than skipped silently (see
/// `PRINTER_ROUND_TRIPPED`).
#[test]
fn the_printers_output_round_trips_through_the_node() {
    let rows = rows();
    let accept_rows = rows
        .iter()
        .filter(|r| r.kind == "printer" && r.accept)
        .count();
    let mut round_tripped = 0usize;
    let mut warted = [0usize; 3];
    let mut closed_refused = 0usize;
    for row in rows.iter().filter(|r| r.kind == "printer" && r.accept) {
        let Ok(closed) = source_to_adt(&row.source) else {
            // Not closed: the normalizer rejects a globally free variable, so the printer (which
            // renders a normalized term) has nothing to render. Accounted, not ignored.
            closed_refused += 1;
            continue;
        };
        let first: Par = closed.into();
        let printed = PrettyPrinter::new().build_string(&first);
        // A reprint the node cannot read back is not a bug this test may assume away: it is one of the
        // signatures a wart can have (`not_wart`), so it goes to the detectors like any difference.
        let second: Option<Par> = source_to_adt(&printed).map(Par::from).ok();
        if second.as_ref() == Some(&first) {
            round_tripped += 1;
            continue;
        }
        // A difference has to be one of the port's named warts, and the detector has to fire: there is
        // no bucket for "the printer did something else".
        let detectors: [(&str, fn(&Par, Option<&Par>) -> bool); 3] = [
            ("the `new`-urn wart", urn_wart),
            ("the one-element-tuple wart", tuple_wart),
            ("the `not`-spelling wart", not_wart),
        ];
        match detectors.iter().position(|(_, d)| d(&first, second.as_ref())) {
            Some(i) => warted[i] += 1,
            None => panic!(
                "printing {:?} produced {printed:?}, which reparses to a different term for no wart \
                 this test knows (`Rchain/Print.lean`'s `printWarts`) — the printer changed what the \
                 term means (AUDIT C13: a doubled group separator and a dropped `bundle` keyword were \
                 both this)",
                row.source
            ),
        }
    }

    // Each wart's detector is exercised on a term of its own, and each is checked with its *own*
    // detector: a detector that never fires is an exemption that cannot fail, and the corpus cannot
    // exercise all three by itself (the `not` row's variable is free, so the normalizer refuses it).
    for (source, detector) in [
        (
            "new x(`rho:id:y`) in { Nil }",
            urn_wart as fn(&Par, Option<&Par>) -> bool,
        ),
        ("(1,)", tuple_wart as fn(&Par, Option<&Par>) -> bool),
        ("not true", not_wart as fn(&Par, Option<&Par>) -> bool),
    ] {
        let first: Par = source_to_adt(source)
            .unwrap_or_else(|e| panic!("{source:?} is closed and must normalize: {e}"))
            .into();
        let printed = PrettyPrinter::new().build_string(&first);
        let second: Option<Par> = source_to_adt(&printed).map(Par::from).ok();
        assert!(
            detector(&first, second.as_ref()),
            "{source:?} is not the wart this table says it is: {printed:?} came back as {second:?}, \
             and the detector for that wart did not fire (Rchain/Print.lean's `printWarts`)"
        );
    }

    assert_eq!(
        round_tripped, PRINTER_ROUND_TRIPPED,
        "the `printer` half carries {PRINTER_ROUND_TRIPPED} round-tripping rows (closed terms whose \
         print is the identity); {round_tripped} were"
    );
    assert_eq!(
        warted,
        [PRINTER_URN_WARTED, PRINTER_TUPLE_WARTED, PRINTER_NOT_WARTED],
        "the `printer` half accrues urn/tuple/`not` warts; it accrued {warted:?}"
    );
    assert_eq!(
        round_tripped + warted[0] + warted[1] + warted[2] + closed_refused,
        accept_rows,
        "every `printer` row that must be accepted is round-tripped, one of the three warts, or \
         accounted as not closed: {round_tripped} + {warted:?} + {closed_refused} != {accept_rows}"
    );
}
