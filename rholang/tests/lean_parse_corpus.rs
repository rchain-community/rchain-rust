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

use rchain_models::ast::Par;
use rchain_rholang::normalizer::source_to_adt;
use rchain_rholang::parser::parse;
use rchain_rholang::pretty_printer::PrettyPrinter;

/// The corpus's declared size (`Rchain/Parse.lean`'s `parseCaseCount`). A corpus that shrinks silently
/// is a check that stopped checking, so the count is pinned on both sides.
const PARSE_CASES: usize = 66;

/// The corpus's four halves, each pinned (`parseCases_by_kind` in `Rchain/Parse.lean`). Pinning them
/// separately is what makes a *specific* list's shrinkage visible: `refused` is law 30's soundness
/// direction, and it is the half a more permissive parser would empty.
const DERIVABLE_CASES: usize = 17;
const REFUSED_CASES: usize = 11;
const DEVIATION_CASES: usize = 14;
const PRINTER_CASES: usize = 24;

/// How the `printer`+`accept` rows are accounted for. Most of the witness table spells its variables as
/// bare names (`x`, `c`, `x /\ y`) and the normalizer rejects a globally free variable, so those rows
/// never reach the printer — and one row (`new x(`rho:id:y`) in Nil`) loses its urn to the printer's
/// wart. All three counts are pinned, and they must *sum* to the `printer`+`accept` rows, so a row that
/// stops normalizing moves the arithmetic instead of disappearing.
const PRINTER_ROUND_TRIPPED: usize = 7;
const PRINTER_URN_WARTED: usize = 1;

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
    let mut urn_warted = 0usize;
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
        let second: Par = source_to_adt(&printed)
            .unwrap_or_else(|e| {
                panic!(
                    "printing {:?} produced {printed:?}, which the node cannot read back: {e}",
                    row.source
                )
            })
            .into();
        if second == first {
            round_tripped += 1;
            continue;
        }
        // A difference is allowed for exactly one named wart — the printer drops `New`'s `uri` and
        // `injections` (`Rchain/Print.lean`'s `printWarts`, third row, which this layer's first run
        // found). Clear them on both sides and require the rest to agree, **and** require the drop to
        // have happened: the exemption fails if it ever rots into a no-op.
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
        assert_eq!(
            tightened, loosened,
            "printing {:?} produced {printed:?}, which reparses differently — the printer changed \
             what the term means (AUDIT C13: a doubled group separator and a dropped `bundle` \
             keyword were both this). The only difference this test tolerates is the `new`-urn wart \
             (`Rchain/Print.lean`'s `printWarts`)",
            row.source
        );
        assert_ne!(
            second, first,
            "printing {:?} produced {printed:?}, which differs from the term by something other than \
             the `new`-urn wart this bucket exists for (Rchain/Print.lean's `printWarts`)",
            row.source
        );
        urn_warted += 1;
    }

    assert_eq!(
        round_tripped, PRINTER_ROUND_TRIPPED,
        "the `printer` half carries {PRINTER_ROUND_TRIPPED} round-tripping rows (closed terms whose \
         print is the identity); {round_tripped} were"
    );
    assert_eq!(
        urn_warted, PRINTER_URN_WARTED,
        "the `printer` half carries {PRINTER_URN_WARTED} row(s) that lose their `new` urn to the \
         printer's wart; {urn_warted} did"
    );
    assert_eq!(
        round_tripped + urn_warted + closed_refused,
        accept_rows,
        "every `printer` row that must be accepted is round-tripped, urn-warted, or accounted as not \
         closed: {round_tripped} + {urn_warted} + {closed_refused} != {accept_rows}"
    );
}
