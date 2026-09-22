//! Law 41 — channel balance, checked 1:1 against the Lean model.
//!
//! `spec/conformance/store.tsv` is emitted by `lake exe rchain-corpus --layer store` from
//! `spec/Rchain/Corpus.lean`, where every verdict is a `decide`d theorem about `storeSurvives`
//! (`Rchain/Store.lean`): after one read of the store on the channel, is a datum back, and does the
//! result form a contract step? A *replicable* reader — a `contract`, called again — must answer again;
//! one that consumes has consumed the store for good, and that is AUDIT C22 item 1: `Inbox.rho`'s
//! zero-argument `read` took the store's only datum and put nothing back, so every later read waited on
//! a channel that would never speak again. Nothing errored.
//!
//! Every term here registers exactly **two** reads of the store, so the model's verdict decides the
//! count the node must produce: the store answers both (`2`) or only the first (`1`). Each case runs on
//! its own runtime and the term carries a control datum, for the reasons the match corpus's module note
//! gives: a shared runtime let later cases pass by re-reading an earlier datum, and a must-not-fire case
//! asserted by absence alone cannot tell "the store was consumed" from "the term never ran".

mod common;

use common::build_runtime_pair;

/// The corpus's declared size (`Rchain/Corpus.lean`'s `storeCaseCount`).
const STORE_CASES: usize = 5;

#[tokio::test]
async fn stores_answer_the_reads_the_lean_model_says_they_survive() {
    let path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../spec/conformance/store.tsv");
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
            "store",
            "corpus line {}: unexpected layer {layer:?}",
            i + 1
        );
        let term = columns.next().expect("the term column");
        let answers: usize = columns
            .next()
            .expect("the answer-count column")
            .parse()
            .unwrap_or_else(|_| panic!("corpus line {}: the count is not a number", i + 1));
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
        let controls = data
            .iter()
            .filter(|p| rchain_models::rholang::RhoType::RhoString::unapply(p) == Some("control"))
            .count();
        assert_eq!(
            controls, 1,
            "{term}: the control datum must be on `out` exactly once, or the term never ran and the \
             case proves nothing — the store's answers are counted as everything else on the channel. \
             Got {} datums",
            data.len()
        );

        // Every datum that is not the control is a read the store answered. The answers are counted
        // rather than matched because one case's store holds a *list* (the read-all/restore-container
        // shape `Inbox.rho` uses), so the channel carries a list as happily as a string.
        let fired = data.len() - controls;
        assert_eq!(
            fired, answers,
            "{term}: the node answered {fired} of the store's two reads, the Lean model says \
             {answers} (`Rchain/Store.lean`'s `storeSurvives`: a reader that does not put back what \
             it consumed has consumed the store — every later read waits silently, AUDIT C22 item 1). \
             A disagreement here is either a store the model thinks survives and the node lost, or \
             one the model says is lost and the node kept."
        );
        cases += 1;
    }

    assert_eq!(
        cases, STORE_CASES,
        "the corpus carries {STORE_CASES} cases (Rchain/Corpus.lean's storeCaseCount); {cases} were \
         read"
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
