//! Law 37 — the matcher, checked 1:1 against the Lean definition, through the node's own path.
//!
//! `spec/conformance/match.tsv` is emitted by `lake exe rchain-corpus --layer match` from
//! `spec/Rchain/Corpus.lean`, where every verdict is a `decide`d theorem about `spatialMatch`
//! (`Rchain/Match.lean`). This test runs each case the way a *consumer* does — send the datum, receive
//! with the pattern — and asserts the node agrees.
//!
//! Three things are deliberate:
//!
//! - **Each case gets its own runtime.** The C20 post-mortem records the harness that shared one
//!   runtime and one output channel, where later cases passed by re-reading an earlier case's datum
//!   whether or not their own pattern matched. A shared fixture here would hide exactly the defect
//!   class under test.
//! - **Every term carries a control datum** (`"control"`, always produced). A must-not-match case
//!   asserted by absence alone cannot tell "the pattern did not match" from "the term never ran" —
//!   the same fixture bug, one level down.
//! - **The pattern arrives as a receive bind**, not through a matcher API: the shape under test is the
//!   shape that runs (`for (<bind> <- chan)`), and `@`-quoting a collection is what the grammar
//!   requires, so the corpus carries the bind text.

mod common;

use common::build_runtime_pair;

/// The corpus's declared size (`Rchain/Corpus.lean`'s `matchCaseCount`). A corpus that shrinks
/// silently is a check that stopped checking, so the count is pinned on both sides.
const MATCH_CASES: usize = 22;

#[tokio::test]
async fn the_matcher_agrees_with_the_lean_model() {
    let path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../spec/conformance/match.tsv");
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
            "match",
            "corpus line {}: unexpected layer {layer:?}",
            i + 1
        );
        let bind = columns.next().expect("the bind column");
        let target = columns.next().expect("the target column");
        // Three-valued: `true`/`false` are the matcher's verdicts, `rejected` says the pattern is
        // refused before matching (a level bound twice), which is law 5 in the port.
        let verdict = columns.next().expect("the verdict column").to_string();
        assert!(
            matches!(verdict.as_str(), "true" | "false" | "rejected"),
            "corpus line {}: unknown verdict {verdict:?}",
            i + 1
        );
        assert!(
            columns.next().is_none(),
            "corpus line {}: trailing columns",
            i + 1
        );

        // A fresh runtime per case (see the module note) and a term with a control datum.
        let (rt, _) = build_runtime_pair().await;
        let term = format!(
            r#"new chan in {{
                 chan!({target}) |
                 for ({bind} <- chan) {{ @"out"!("matched") }} |
                 @"out"!("control")
               }}"#
        );
        let evaluated = rt
            .evaluate_with_env(&term, &Default::default(), &fixed_rand())
            .await;
        if verdict == "rejected" {
            // Law 5's port-side shape: the normalizer refuses the pattern outright, loudly — a
            // repeated free variable is `UnexpectedReuseOfProcContextFree`, not a silent no-match.
            let refused = match &evaluated {
                Err(_) => true,
                Ok(res) => !res.succeeded(),
            };
            assert!(
                refused,
                "{bind} vs {target}: the Lean model says this pattern is refused (a level bound \
                 twice, law 5), but the node accepted it: {evaluated:?}"
            );
            cases += 1;
            continue;
        }
        let res = evaluated.expect("evaluate returns Ok");
        assert!(
            res.succeeded(),
            "{bind} vs {target}: the term failed to run: {:?}",
            res.errors
        );
        let data = rt
            .get_data_par(&chan("out"))
            .await
            .expect("read the output channel");
        let mut tags: Vec<String> = data
            .iter()
            .filter_map(|p| {
                rchain_models::rholang::RhoType::RhoString::unapply(p).map(str::to_string)
            })
            .collect();
        tags.sort();

        assert!(
            tags.contains(&"control".to_string()),
            "{bind} vs {target}: the control datum is missing, so the term never ran — the case \
             proves nothing. Got {tags:?}"
        );
        let matched = tags.contains(&"matched".to_string());
        let expected = verdict == "true";
        assert_eq!(
            matched, expected,
            "{bind} vs {target}: the node says {matched}, the Lean model says {expected} \
             (spec/conformance/match.tsv, law 37). An unmatched receive is not an error — it simply \
             never fires — so a disagreement here is a pattern that silently matches nothing (AUDIT \
             C19/C20/C22)."
        );
        cases += 1;
    }

    assert_eq!(
        cases, MATCH_CASES,
        "the corpus carries {MATCH_CASES} cases (Rchain/Corpus.lean's matchCaseCount); {cases} were read"
    );
}

/// A fixed, deterministic random seed so fresh-name allocation is reproducible.
fn fixed_rand() -> rchain_crypto::hash::blake2b512_random::Blake2b512Random {
    rchain_crypto::hash::blake2b512_random::Blake2b512Random::from_init(&[0u8; 32])
}

/// The `SortedProc` for a string channel.
fn chan(name: &str) -> rchain_models::sorted::SortedProc {
    rchain_models::sorted::SortedProc::new(rchain_models::par_ops::from_expr(
        rchain_models::ast::Expr::GString(name.to_string()),
    ))
}
