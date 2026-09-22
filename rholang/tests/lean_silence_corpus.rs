//! Law 38 — silence, checked 1:1 against the Lean model.
//!
//! `spec/conformance/silence.tsv` is emitted by `lake exe rchain-corpus --layer silence` from
//! `spec/Rchain/Corpus.lean`, where every verdict is a `decide`d theorem about `takesStep`
//! (`Rchain/Silence.lean`): does this term hold a send/receive pair that forms a contract step? The
//! model's *rule* is what makes silence a law rather than a remark — `ReduceP`'s only rule that
//! consumes a datum carries `spatialMatches data pattern` as a hypothesis, so a receive whose pattern
//! does not match has no step and reports no error, which is exactly how the defects of AUDIT C9-C26
//! presented (a broken pattern reads as a client bug).
//!
//! Each case runs on its own runtime and its term carries a control datum, for the reasons the match
//! corpus's module note gives: a shared runtime let later cases pass by re-reading an earlier datum,
//! and a must-not-step case asserted by absence alone cannot tell "no step" from "never ran".

mod common;

use common::build_runtime_pair;

/// The corpus's declared size (`Rchain/Corpus.lean`'s `silenceCaseCount`).
const SILENCE_CASES: usize = 12;

#[tokio::test]
async fn silence_agrees_with_the_lean_model() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../spec/conformance/silence.tsv");
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
            "silence",
            "corpus line {}: unexpected layer {layer:?}",
            i + 1
        );
        let term = columns.next().expect("the term column");
        let steps: bool = columns
            .next()
            .expect("the verdict column")
            .parse()
            .unwrap_or_else(|_| panic!("corpus line {}: verdict is not a bool", i + 1));
        assert!(
            columns.next().is_none(),
            "corpus line {}: trailing columns",
            i + 1
        );

        let (rt, _) = build_runtime_pair().await;
        let full = format!("{term} | @\"out\"!(\"control\")");
        let res = rt
            .evaluate_with_env(&full, &Default::default(), &fixed_rand())
            .await
            .expect("evaluate returns Ok");
        assert!(
            res.succeeded(),
            "{term}: the term failed to run: {:?}",
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
            "{term}: the control datum is missing, so the term never ran — the case proves \\
             nothing. Got {tags:?}"
        );
        let stepped = tags.contains(&"step".to_string());
        assert_eq!(
            stepped, steps,
            "{term}: the node {}step, the Lean model says it {}. A receive that does not match \\
             performs no step and reports no error (law 38, `Rchain/Silence.lean`'s `ReduceP`), so a \\
             disagreement here is either a pattern that silently matched nothing or one that \\
             silently did (AUDIT C19/C20/C22).",
            if stepped { "did " } else { "did not " },
            if steps { "does" } else { "does not" }
        );
        cases += 1;
    }

    assert_eq!(
        cases, SILENCE_CASES,
        "the corpus carries {SILENCE_CASES} cases (Rchain/Corpus.lean's silenceCaseCount); {cases} \
         were read"
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
