//! Law 32 — the operator surface, checked 1:1 against the Lean table.
//!
//! `spec/conformance/lex.tsv` is emitted by `lake exe rchain-corpus --layer lex` from `Rchain/Lex.lean`'s
//! `lexemes`, whose `decide`d checks are what a table can check about itself: the spellings are distinct
//! and punctuation, and **maximal munch** holds — for every row, the longest spelling that prefixes it
//! *is* it, so a table where `<` shadowed `<=` fails the `<=` row.
//!
//! What a table cannot check is what it *means*, and that is this consumer's half: each row carries a
//! **sample** term whose result changes if the spelling means one of its neighbours, and the row's
//! expectation says what must be observed — `value:<json>` (one datum on `@"out"`, compared through the
//! API's own JSON, law 42's machinery) or `answers:<n>` (n datums on `@"out"`, which is how `<-` — one
//! consume — and `<<-` — two peeks — are told apart). A swap like AUDIT C10's (`/\` and `\/` lexed
//! silently as each other) fails the sample, which is the point: nothing errored, the two connectives
//! simply meant each other.
//!
//! Each case runs on its own runtime; every sample carries its own control datum on `@"ctl"`, so a
//! sample that never ran is distinguishable from one that observed nothing.

mod common;

use common::rho_runtime;

/// The table's declared size (`Rchain/Lex.lean`'s `lexemeCount`).
const LEXEMES: usize = 18;

#[tokio::test]
async fn every_operator_means_what_the_lean_table_says() {
    let path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../spec/conformance/lex.tsv");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "read {}: {e}\n(run tools/emit-lean-corpus.sh)",
            path.display()
        )
    });

    let mut rows = 0usize;
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let mut columns = line.split('\t');
        let layer = columns.next().unwrap_or_default();
        assert_eq!(
            layer,
            "lex",
            "corpus line {}: unexpected layer {layer:?}",
            i + 1
        );
        let spelling = columns.next().expect("the spelling column");
        let token = columns.next().expect("the token column");
        let expected = columns.next().expect("the expectation column");
        let sample = columns.next().expect("the sample column");
        assert!(
            columns.next().is_none(),
            "corpus line {}: trailing columns",
            i + 1
        );
        let (kind, want) = expected
            .split_once(':')
            .unwrap_or_else(|| panic!("{spelling}: malformed expectation {expected:?}"));

        let rt = rho_runtime().await;
        let res = rt
            .evaluate_with_env(sample, &Default::default(), &fixed_rand())
            .await
            .expect("evaluate returns Ok");
        assert!(
            res.succeeded(),
            "{spelling} ({token}): the sample failed to run — a spelling the lexer does not accept is \
             exactly what C10 looked like from the outside: {:?}\n{sample}",
            res.errors
        );
        let ctl = rt.get_data_par(&chan("ctl")).await.expect("read ctl");
        assert_eq!(
            ctl.len(),
            1,
            "{spelling} ({token}): the control datum is missing, so the sample never ran and the case \
             proves nothing"
        );

        let data = rt.get_data_par(&chan("out")).await.expect("read out");
        match kind {
            "value" => {
                assert_eq!(
                    data.len(),
                    1,
                    "{spelling} ({token}): the sample must put exactly one datum on `out`; got {}",
                    data.len()
                );
                let got = rchain_node::api::rho_expr::expr_from_par(&data[0])
                    .map(|e| serde_json::to_string(&e).expect("serializes"))
                    .unwrap_or_else(|| "<no JSON: the value has no RhoExpr arm>".to_string());
                assert_eq!(
                    got, want,
                    "{spelling} ({token}): the sample observed {got}, the Lean table says {want} \
                     (Rchain/Lex.lean's `lexemes`). A spelling that means a neighbour is AUDIT C10: \
                     `a \\/ b` lexed as conjunction and nothing errored."
                );
            }
            "answers" => {
                let want: usize = want.parse().unwrap_or_else(|_| {
                    panic!("{spelling}: the answer count must be a number, got {want:?}")
                });
                assert_eq!(
                    data.len(),
                    want,
                    "{spelling} ({token}): the sample observed {} answers, the Lean table says {want} \
                     — the arrows differ exactly here (`<-` consumes what it reads, `<<-` does not, \
                     `<=` re-arms).",
                    data.len()
                );
            }
            other => panic!("{spelling}: unknown expectation kind {other:?}"),
        }
        rows += 1;
    }

    assert_eq!(
        rows, LEXEMES,
        "the table carries {LEXEMES} rows (Rchain/Lex.lean's lexemeCount); {rows} were read"
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
