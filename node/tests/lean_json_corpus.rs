//! Law 42 — the rho-value JSON, checked 1:1 against the Lean model.
//!
//! `spec/conformance/json.tsv` is emitted by `lake exe rchain-corpus --layer json` from
//! `Rchain/Corpus.lean`'s `jsonCases`, where each case's verdict is `decide`d against `Rchain/Json.lean`
//! — the model's `parToJE` (the encode, with `expr_from_par`'s envelope rule: one field unwrapped, none
//! *absent*, two or more an `ExprPar`) and its `render` (the wire text `serde` produces, compact and
//! externally tagged). A case's JSON column is therefore the model's own rendering, not a hand-written
//! expectation, and this consumer is the second party: it runs the same value through the node and
//! compares the node's JSON to the model's text.
//!
//! Each case additionally round-trips through the API's own pair — `rho_expr_to_par` of what
//! `expr_from_par` produced, then encoded again — which is `Rchain/Json.lean`'s `decode_encode` stated
//! at the wire level. A shape that drifts here is AUDIT C16's class: a client reads a field the node no
//! longer writes, and **nothing errors**.
//!
//! Each case runs on its own runtime and its term carries a control datum, so "the envelope is absent"
//! (the `Nil` row, whose JSON column is `-`) can be told from "the term never ran".

mod common;

use common::rho_runtime;

/// The corpus's declared size (`Rchain/Corpus.lean`'s `jsonCaseCount`).
const JSON_CASES: usize = 12;

#[tokio::test]
async fn the_api_json_is_the_lean_models_json_and_round_trips() {
    let path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../spec/conformance/json.tsv");
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
            "json",
            "corpus line {}: unexpected layer {layer:?}",
            i + 1
        );
        let source = columns.next().expect("the value column");
        let expected = columns.next().expect("the JSON column");
        assert!(
            columns.next().is_none(),
            "corpus line {}: trailing columns",
            i + 1
        );
        // The value as a datum, and a control datum that proves the term ran.
        let term = format!("@\"out\"!({source}) | @\"ctl\"!(\"ran\")");

        let rt = rho_runtime().await;
        let res = rt
            .evaluate_with_env(&term, &Default::default(), &fixed_rand())
            .await
            .expect("evaluate returns Ok");
        assert!(
            res.succeeded(),
            "{source}: the term failed to run: {:?}",
            res.errors
        );
        let ctl = rt.get_data_par(&chan("ctl")).await.expect("read ctl");
        assert_eq!(
            ctl.len(),
            1,
            "{source}: the control datum is missing, so the term never ran and the case proves \
             nothing"
        );
        let data = rt.get_data_par(&chan("out")).await.expect("read out");
        assert_eq!(
            data.len(),
            1,
            "{source}: the datum must be on `out` exactly once; got {data:?}"
        );

        // The node's own encode, and the model's text.
        let node = rchain_node::api::rho_expr::expr_from_par(&data[0]);
        if expected == "-" {
            assert!(
                node.is_none(),
                "{source}: the model says the envelope is *absent* — no JSON at all, which is what the \
                 0-field case is (`expr_from_par`'s `match exprs.len()`, `node/src/api/rho_expr.rs:57`) \
                 — and the node exposed {}",
                node.map(|e| serde_json::to_string(&e).expect("serializes"))
                    .unwrap_or_default()
            );
        } else {
            let expr = node.unwrap_or_else(|| {
                panic!(
                    "{source}: the node exposes no JSON for this value, the model says {expected}"
                )
            });
            let got = serde_json::to_string(&expr).expect("the encode serializes");
            assert_eq!(
                got, expected,
                "{source}: the node exposes {got}, the Lean model says {expected} \
                 (Rchain/Json.lean's `parToJE` and `render`: the envelope rule and the wire form). \
                 A shape that drifted here is AUDIT C16's class — a client reads a field the node no \
                 longer writes, and nothing errors."
            );
            // The API's own round-trip: decode what it encoded, encode again.
            let back =
                rchain_node::api::rho_expr::rho_expr_to_par(&expr).expect("the encode decodes");
            let again = rchain_node::api::rho_expr::expr_from_par(&back)
                .expect("the decoded par re-encodes");
            assert_eq!(
                serde_json::to_string(&again).expect("serializes"),
                got,
                "{source}: `rho_expr_to_par (expr_from_par p) = p` at the wire level — decoding what \
                 the API encoded must encode back to the same JSON (Rchain/Json.lean's \
                 `decode_encode`)."
            );
        }
        cases += 1;
    }

    assert_eq!(
        cases, JSON_CASES,
        "the corpus carries {JSON_CASES} cases (Rchain/Corpus.lean's jsonCaseCount); {cases} were \
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
