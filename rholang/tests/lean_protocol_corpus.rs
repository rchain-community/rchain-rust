//! Law 39 — reply shapes, checked 1:1 against the Lean catalog.
//!
//! `spec/conformance/protocol.tsv` is emitted by `lake exe rchain-corpus --layer protocol` from
//! `spec/Rchain/Protocol.lean`'s `replyCatalog`, where each row is a *probe*: the urn, the arguments
//! the call passes, the call's arity, and the reply's kind and slots. The row's consistency is
//! `decide`d in Lean (unique namespaced urns, and — the check with teeth — the declared arity
//! agreeing with the arguments as written, which is AUDIT C22 item 2's class: a call whose arity
//! matches no receive is not an error, it is silence). What the row cannot check is whether the *node*
//! answers that way; this is.
//!
//! Each case calls the urn the row spells and re-sends every part of the reply on its own channel, so
//! the *arity* of the reply is checked as well as each slot's shape: a reply that comes back short
//! leaves the receive waiting, and the missing datum is a failure here rather than a client's mystery
//! (C18: `rho:registry:lookup` wrapped its reply in `(uri, value)` and every consumer destructured the
//! bare value, so **it failed silently rather than loudly**).
//!
//! Each case runs on its own runtime and its term carries a control datum: an urn that answers
//! *nothing* (`rho:io:stdout`, whose row's kind is `none`) is asserted by absence, and absence alone
//! cannot tell "the node agreed with the row" from "the term never ran".

mod common;

use common::build_runtime_pair;
use rchain_models::ast::Par;

/// The catalog's declared size (`Rchain/Protocol.lean`'s `replyCaseCount`).
const PROTOCOL_CASES: usize = 9;

#[tokio::test]
async fn every_urn_replies_in_the_shape_the_lean_catalog_says() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../spec/conformance/protocol.tsv");
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
            "protocol",
            "corpus line {}: unexpected layer {layer:?}",
            i + 1
        );
        let urn = columns.next().expect("the urn column");
        let args = columns.next().expect("the arguments column");
        let kind = columns.next().expect("the reply-kind column");
        let slots = columns.next().expect("the slots column");
        assert!(
            columns.next().is_none(),
            "corpus line {}: trailing columns",
            i + 1
        );
        let slot_tags: Vec<&str> = if slots == "-" {
            Vec::new()
        } else {
            slots.split(',').collect()
        };
        assert!(
            !slot_tags.contains(&"unknown"),
            "{urn}: an unknown slot shape is not a shape the law can name"
        );

        // The receive pattern and the re-sends, built from the row's *kind*: a `send` is an n-arity
        // send (n patterns), a `tuple` is one datum that is an n-tuple (one `@(…)` pattern), and a
        // `none` row still registers a receive so that an unexpected reply is caught rather than
        // silently ignored.
        let tags = if kind == "none" { 1 } else { slot_tags.len() };
        let patterns = match kind {
            "none" => "@x0".to_string(),
            "send" => (0..tags)
                .map(|i| format!("@x{i}"))
                .collect::<Vec<_>>()
                .join(", "),
            "tuple" => format!(
                "@({})",
                (0..tags)
                    .map(|i| format!("x{i}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            other => panic!("{urn}: unknown reply kind {other:?}"),
        };
        let resends = (0..tags)
            .map(|i| format!("@\"o{i}\"!(x{i}) | "))
            .collect::<String>();
        // The reply channel is the call's last argument, and only a row that expects a reply passes
        // one: an urn with no reply takes exactly what its row spells. Getting this wrong is silence
        // rather than an error — the arity is what a call is matched against — so it is spelled out
        // here rather than defaulted.
        let call = match (kind, args) {
            ("none", a) => format!("c!({a})"),
            (_, "-") => "c!(*ret)".to_string(),
            (_, a) => format!("c!({a}, *ret)"),
        };
        let term = format!(
            "new c(`{urn}`), ret, o0, o1, o2, o3 in {{ {call} \
             | for ({patterns} <- ret) {{ {resends}Nil }} }} | @\"ctl\"!(\"ran\")"
        );

        let (rt, _) = build_runtime_pair().await;
        let res = rt
            .evaluate_with_env(&term, &Default::default(), &fixed_rand())
            .await
            .expect("evaluate returns Ok");
        assert!(
            res.succeeded(),
            "{urn}: the probe failed to run: {:?}\n{term}",
            res.errors
        );

        let ctl = rt.get_data_par(&chan("ctl")).await.expect("read ctl");
        assert_eq!(
            ctl.len(),
            1,
            "{urn}: the control datum is missing, so the probe never ran and the case proves \
             nothing"
        );

        for (slot, tag) in slot_tags.iter().enumerate() {
            let data = rt
                .get_data_par(&chan(&format!("o{slot}")))
                .await
                .expect("read a reply slot");
            assert_eq!(
                data.len(),
                1,
                "{urn}: slot {slot} must carry exactly one value; the reply's arity is part of the \
                 law too, and a short reply leaves the receive waiting (silently — C18, C22 item 2). \
                 Got {data:?}"
            );
            if *tag == "any" {
                continue;
            }
            let got = classify(&data[0]);
            assert_eq!(
                got, *tag,
                "{urn}: slot {slot} came back as {got}, the catalog says {tag} (spec/API-SCHEMA.md's \
                 row, pinned as data in spec/Rchain/Protocol.lean's `replyCatalog`). A shape that \
                 drifted here is the C18 class: the client's pattern stops matching and nothing errors."
            );
        }
        if kind == "none" {
            let o0 = rt.get_data_par(&chan("o0")).await.expect("read o0");
            assert!(
                o0.is_empty(),
                "{urn}: the row says this urn replies not at all, and a datum arrived on the reply \
                 channel: {o0:?}"
            );
        }
        cases += 1;
    }

    assert_eq!(
        cases, PROTOCOL_CASES,
        "the catalog carries {PROTOCOL_CASES} rows (Rchain/Protocol.lean's replyCaseCount); {cases} \
         were read"
    );
}

/// Which slot shape a value is, in the catalog's vocabulary. `other` is never a legal tag: a value the
/// catalog cannot name is a failure of the row, not a pass.
fn classify(p: &Par) -> &'static str {
    use rchain_models::rholang::RhoType::*;
    if RhoNil::unapply(p) {
        "nil"
    } else if RhoBoolean::unapply(p).is_some() {
        "bool"
    } else if RhoNumber::unapply(p).is_some() {
        "int"
    } else if RhoString::unapply(p).is_some() {
        "string"
    } else if RhoUri::unapply(p).is_some() {
        "uri"
    } else if RhoByteArray::unapply(p).is_some() {
        "byteArray"
    } else if RhoMap::unapply(p).is_some() {
        "map"
    } else if RhoSet::unapply(p).is_some() {
        "set"
    } else if RhoList::unapply(p).is_some() {
        "list"
    } else {
        "other"
    }
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
