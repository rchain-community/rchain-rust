//! Pretty-print the tuplespace state (port of `interpreter/storage/StoragePrinter.scala`).
//!
//! The deploy-evaluation variants (`prettyPrintUnmatchedSends(deploy, …)`) depend on `evaluate`
//! (parse + run), which is not yet ported; only the hot-changes snapshot printers are provided.

use rchain_models::ast::{AlwaysEqual, Par, Receive, ReceiveBind, Send};
use rchain_models::par_ops::{par_concat, prepend_receive, prepend_send};
use rchain_models::runtime::{BindPattern, ListParWithRandom, TaggedContinuation};
use rchain_models::sorted::SortedProc;
use rchain_models::types::FreeCount;
use rchain_rspace::internal::{Datum, WaitingContinuation};

use crate::pretty_printer::PrettyPrinter;
use crate::runtime::RhoRuntime;

pub const NO_UNMATCHED_SENDS: &str = "No unmatched sends.";
const EMPTY_SPACE: &str =
    "The space is empty. Note that top level terms that are not sends or receives are discarded.";

/// Render the full hot-changes snapshot as rholang (port of `StoragePrinter.prettyPrint`).
pub async fn pretty_print(runtime: &RhoRuntime) -> String {
    let mapped = runtime.get_hot_changes().await;
    let pars: Vec<Par> = mapped
        .iter()
        .map(|(channels, row)| {
            if row.data.is_empty() && row.wks.is_empty() {
                Par::default()
            } else if row.wks.is_empty() {
                to_sends(&row.data, channels)
            } else if row.data.is_empty() {
                to_receive(&row.wks, channels)
            } else {
                par_concat(
                    &to_sends(&row.data, channels),
                    &to_receive(&row.wks, channels),
                )
            }
        })
        .collect();

    if pars.is_empty() {
        EMPTY_SPACE.to_string()
    } else {
        let merged = pars
            .into_iter()
            .reduce(|a, b| par_concat(&a, &b))
            .unwrap_or_else(|| Par::default());
        PrettyPrinter::new().build_string(&merged)
    }
}

/// Render only the unmatched sends (produced data) (port of
/// `StoragePrinter.prettyPrintUnmatchedSends(runtime)`).
pub async fn pretty_print_unmatched_sends(runtime: &RhoRuntime) -> String {
    let mapped = runtime.get_hot_changes().await;
    let pars: Vec<Par> = mapped
        .iter()
        .map(|(channels, row)| to_sends(&row.data, channels))
        .collect();
    if pars.is_empty() {
        NO_UNMATCHED_SENDS.to_string()
    } else {
        let merged = pars
            .into_iter()
            .reduce(|a, b| par_concat(&a, &b))
            .unwrap_or_else(|| Par::default());
        PrettyPrinter::new().build_string(&merged)
    }
}

fn to_sends(data: &[Datum<ListParWithRandom>], channels: &[SortedProc]) -> Par {
    let mut acc = Par::default();
    for datum in data {
        for channel in channels {
            let send = Send {
                chan: Box::new(channel.as_par().clone().quote()),
                data: datum
                    .a
                    .pars
                    .iter()
                    .map(|p| p.as_par().clone().quote())
                    .collect(),
                persistent: datum.persist,
                locally_free: AlwaysEqual(vec![]),
                connective_used: false,
            };
            acc = prepend_send(&acc, send);
        }
    }
    acc
}

fn to_receive(
    wks: &[WaitingContinuation<BindPattern, TaggedContinuation>],
    channels: &[SortedProc],
) -> Par {
    let mut acc = Par::default();
    for wk in wks {
        let binds: Vec<ReceiveBind> = channels
            .iter()
            .zip(wk.patterns.iter())
            .map(|(channel, pattern)| ReceiveBind {
                patterns: pattern
                    .patterns
                    .iter()
                    .map(|p| p.as_par().clone().quote())
                    .collect(),
                source: Box::new(channel.as_par().clone().quote()),
                remainder: pattern.remainder.clone().map(Box::new),
                free_count: FreeCount::from_nonneg(pattern.free_count),
            })
            .collect();
        let (body, bind_count) = match &wk.continuation {
            TaggedContinuation::ParBody(p) => (
                p.body.as_par().clone(),
                wk.patterns.iter().map(|p| p.free_count).sum(),
            ),
            _ => (Par::default(), 0),
        };
        let receive = Receive {
            binds,
            body: Box::new(body),
            persistent: wk.persist,
            peek: !wk.peeks.is_empty(),
            bind_count,
            locally_free: AlwaysEqual(vec![]),
            connective_used: false,
        };
        acc = prepend_receive(&acc, receive);
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    use rchain_crypto::hash::blake2b512_random::Blake2b512Random;
    use rchain_models::ast::Expr;
    use rchain_models::par_ops::from_expr;
    use rchain_models::runtime::ParWithRandom;
    use rchain_rspace::trace::event::{Consume, Produce};

    fn channel(name: &str) -> SortedProc {
        SortedProc::new(from_expr(Expr::GString(name.to_string())))
    }

    fn datum(values: &[i64], persist: bool) -> Datum<ListParWithRandom> {
        Datum {
            a: ListParWithRandom {
                pars: values
                    .iter()
                    .map(|v| SortedProc::new(from_expr(Expr::GInt(*v))))
                    .collect(),
                random_state: Blake2b512Random::from_init(&[0u8; 32]),
            },
            persist,
            source: Produce::apply(&"c".to_string(), &"d".to_string(), persist),
        }
    }

    fn waiting(
        continuum: TaggedContinuation,
        persist: bool,
        peek: bool,
        free_counts: &[i32],
    ) -> WaitingContinuation<BindPattern, TaggedContinuation> {
        WaitingContinuation {
            patterns: free_counts
                .iter()
                .map(|fc| BindPattern {
                    patterns: vec![SortedProc::new(from_expr(Expr::GInt(0)))],
                    remainder: None,
                    free_count: *fc,
                })
                .collect(),
            continuation: continuum,
            persist,
            peeks: if peek {
                [0].into_iter().collect()
            } else {
                BTreeSet::new()
            },
            source: Consume::apply(
                &["c".to_string()],
                &["p".to_string()],
                &"k".to_string(),
                persist,
            ),
        }
    }

    /// A datum on two channels renders as **one send per channel** (the snapshot is per-channel),
    /// with the datum's pars as the payload and its persistence carried over.
    #[test]
    fn a_datum_renders_as_one_send_per_channel() {
        let rendered = to_sends(&[datum(&[1, 2], true)], &[channel("a"), channel("b")]);
        assert_eq!(
            rendered.sends.len(),
            2,
            "one send per channel: {rendered:?}"
        );
        for send in &rendered.sends {
            assert!(send.persistent, "the datum's persistence is rendered");
            assert_eq!(
                send.data,
                vec![
                    from_expr(Expr::GInt(1)).quote(),
                    from_expr(Expr::GInt(2)).quote()
                ],
                "the payload is the datum's pars, quoted into names"
            );
        }
        // An empty snapshot renders nothing.
        assert_eq!(to_sends(&[], &[]), Par::default());
    }

    /// A waiting continuation renders as a `for` whose **body** is the continuation and whose
    /// `bind_count` is the sum of the patterns' free counts — the number the receiver's binder needs.
    #[test]
    fn a_waiting_continuation_renders_its_body_and_bind_count() {
        let body = from_expr(Expr::GInt(7));
        let wk = waiting(
            TaggedContinuation::ParBody(ParWithRandom {
                body: SortedProc::new(body.clone()),
                random_state: Blake2b512Random::from_init(&[0u8; 32]),
            }),
            false,
            false,
            &[1, 2],
        );

        let rendered = to_receive(&[wk], &[channel("a"), channel("b")]);
        assert_eq!(rendered.receives.len(), 1);
        let receive = &rendered.receives[0];
        assert_eq!(*receive.body, body, "the continuation is the loop body");
        assert_eq!(
            receive.bind_count, 3,
            "the sum of the patterns' free counts"
        );
        assert!(!receive.peek, "not a peek");
        assert_eq!(receive.binds.len(), 2, "one bind per channel");
    }

    /// **The non-`ParBody` arm**: a continuation that is not rholang code (a built-in reference, or
    /// the empty one) has no term to print, so the body is `Nil` and the bind count is zero. A
    /// printer that unwrapped it anyway would print a built-in's ref as if it were source.
    #[test]
    fn a_non_par_body_continuation_renders_an_empty_body() {
        for continuum in [
            TaggedContinuation::Empty,
            TaggedContinuation::ScalaBodyRef(3),
        ] {
            let wk = waiting(continuum, true, true, &[5]);
            let rendered = to_receive(&[wk], &[channel("a")]);
            let receive = &rendered.receives[0];
            assert_eq!(
                *receive.body,
                Par::default(),
                "a non-rholang continuation has no body to print"
            );
            assert_eq!(receive.bind_count, 0, "…and no binders to count");
            assert!(receive.persistent, "its persistence is still rendered");
            assert!(receive.peek, "…and so is its peeking");
        }
    }

    /// The two fixed messages are what an RPC caller sees for an empty space, so they are part of
    /// the interface rather than decoration — pinned so they cannot drift.
    #[test]
    fn the_empty_space_messages_are_fixed() {
        assert_eq!(NO_UNMATCHED_SENDS, "No unmatched sends.");
        assert!(EMPTY_SPACE.starts_with("The space is empty."));
    }
}
