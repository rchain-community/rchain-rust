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

/// The snapshot could not be rendered: a stored continuation's pattern declares a free-level count
/// the store cannot hold (`stored_free_count`). A fixed message rather than a partial rendering,
/// because a rendering that dropped or clamped the bindings would describe a term other than the one
/// stored. Unreachable — such a pattern is refused on arrival (AUDIT C52) — so there is nothing to
/// recover from and nothing to guess.
pub const MALFORMED_PATTERN: &str =
    "The store could not be rendered: a continuation pattern declares a negative free count.";
const EMPTY_SPACE: &str =
    "The space is empty. Note that top level terms that are not sends or receives are discarded.";

/// Render the full hot-changes snapshot as rholang (port of `StoragePrinter.prettyPrint`).
pub async fn pretty_print(runtime: &RhoRuntime) -> String {
    let mapped = runtime.get_hot_changes().await;
    let rows: Option<Vec<Par>> = mapped
        .iter()
        .map(|(channels, row)| {
            Some(if row.data.is_empty() && row.wks.is_empty() {
                Par::default()
            } else if row.wks.is_empty() {
                to_sends(&row.data, channels)
            } else if row.data.is_empty() {
                to_receive(&row.wks, channels)?
            } else {
                par_concat(
                    &to_sends(&row.data, channels),
                    &to_receive(&row.wks, channels)?,
                )
            })
        })
        .collect();
    let Some(pars) = rows else {
        return MALFORMED_PATTERN.to_string();
    };

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

/// A stored `BindPattern`'s free-level count as the carrier, or `None` when the store's field says a
/// negative one.
///
/// The Scala reads `pattern.freeCount` (`StoragePrinter.scala:151`) and sums the fields (`:161`), and
/// so does the port — but `BindPattern.free_count` is an `i32`, not a `FreeCount`: casper, node and
/// the bench construct that struct with literals, so the carrier cannot move onto the field from
/// here (named as the durable fix by AUDIT C52).
///
/// So this renderer *checks* rather than asserts (the deleted `FreeCount::from_nonneg` used a
/// `debug_assert!`, compiled out in release), and a negative field refuses the row instead of being
/// clamped: the count is what `RhoMatch::get` fills the continuation's environment from, so a
/// negative one describes a term the store cannot hold, and a clamp to zero would render a `for`
/// that binds *nothing* — a term that means something other than what is stored. Unreachable in
/// practice: such a pattern is refused on arrival by `bind_pattern_from_proto` (AUDIT C52), and every
/// in-crate construction site passes a count. Nothing is recovered here, because nothing can be
/// recovered exactly: the port's own count convention excludes remainder binders (the model's,
/// `Match.lean`'s `freeLevelsOfPar`), so a derived answer would be a *second* approximation rather
/// than the truth.
fn stored_free_count(pattern: &BindPattern) -> Option<FreeCount> {
    FreeCount::new(pattern.free_count)
}

fn to_receive(
    wks: &[WaitingContinuation<BindPattern, TaggedContinuation>],
    channels: &[SortedProc],
) -> Option<Par> {
    let mut acc = Par::default();
    for wk in wks {
        let binds: Vec<ReceiveBind> = channels
            .iter()
            .zip(wk.patterns.iter())
            .map(|(channel, pattern)| {
                Some(ReceiveBind {
                    patterns: pattern
                        .patterns
                        .iter()
                        .map(|p| p.as_par().clone().quote())
                        .collect(),
                    source: Box::new(channel.as_par().clone().quote()),
                    remainder: pattern.remainder.clone().map(Box::new),
                    free_count: stored_free_count(pattern)?,
                })
            })
            .collect::<Option<Vec<ReceiveBind>>>()?;
        let (body, bind_count) = match &wk.continuation {
            TaggedContinuation::ParBody(p) => {
                let mut total = FreeCount::ZERO;
                for pattern in &wk.patterns {
                    total = total + stored_free_count(pattern)?;
                }
                (p.body.as_par().clone(), total)
            }
            _ => (Par::default(), FreeCount::ZERO),
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
    Some(acc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    use rchain_crypto::hash::blake2b512_random::Blake2b512Random;
    use rchain_models::ast::Expr;
    use rchain_models::par_ops::from_expr;
    use rchain_models::runtime::ParWithRandom;
    use rchain_models::types::count_free_vars_refined;
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

    /// **The count convention, pinned where the two are easy to confuse.** `count_free_vars` walks
    /// the free *variables* of a term and deliberately does not count remainder binders — the model's
    /// convention too (`Match.lean`'s `freeLevelsOfPar` ignores the field), and the right one for its
    /// own callers (law 5's `linear`). It is therefore **not** the count a receive's `free_count`
    /// carries when a pattern has a *collection* remainder: for `for (@[x, ...rest] <- c)` the
    /// normalizer writes 2 — both levels are allocated (`handle_proc_var`, `normalizer.rs:594-611`) —
    /// where the walk sees one free variable and no named remainder. Recorded because casper's runtime
    /// builders assign `count_free_vars` straight into `BindPattern.free_count`
    /// (`runtime_manager.rs:742`, `runtime_replay.rs:552`): those two patterns are single free
    /// variables, so the difference is latent rather than live, and this is the shape that would
    /// expose it.
    #[test]
    fn the_walk_ignores_a_collection_remainder_where_the_normalizer_counts_it() {
        // Wrapped in a `new` so the terms are closed (`source_to_adt` refuses top-level free
        // variables, which is what the source language requires of a deploy).
        let plain = |source: &str| -> (i32, i32) {
            let p: Par = crate::normalizer::source_to_adt(source)
                .expect("parse")
                .into();
            let rb = &p.news[0].p.receives[0].binds[0];
            let walk = rb
                .patterns
                .iter()
                .map(|s| i32::from(count_free_vars_refined(&s.clone().eval())))
                .sum::<i32>()
                + i32::from(rb.remainder.is_some());
            (walk, i32::from(rb.free_count))
        };

        for source in [
            "new c in { for (x <- c) { Nil } }",
            "new c in { for (x, y <- c) { Nil } }",
            "new c in { for (x, ...rest <- c) { Nil } }",
        ] {
            let (walk, free_count) = plain(source);
            assert_eq!(walk, free_count, "the walk agrees for `{source}`");
        }

        let (walk, free_count) = plain("new c in { for (@[x, ...rest] <- c) { Nil } }");
        assert_eq!(
            (walk, free_count),
            (1, 2),
            "a collection remainder is a level the normalizer counts and the walk does not"
        );
    }

    /// A stored pattern whose `free_count` is negative **refuses the row** rather than clamping it:
    /// the count is what `RhoMatch::get` fills the continuation's environment from, so a negative one
    /// describes a term the store cannot hold, and a clamp to zero would render a `for` binding
    /// nothing — a term that means something other than what is stored. A fixed diagnostic message is
    /// the snapshot's answer (the module already speaks this way for an empty space), not a partial
    /// rendering.
    ///
    /// Unreachable in practice — such a pattern is refused where it arrives
    /// (`bind_pattern_from_proto`, AUDIT C52) — which is exactly why the answer is a refusal and not
    /// a recovery: there is nothing to recover.
    ///
    /// Falsifier: with `stored_free_count`'s check replaced by the deleted `FreeCount::from_nonneg`,
    /// this fails (the row renders with a negative `free_count` in it).
    #[test]
    fn a_negative_stored_free_count_refuses_the_row() {
        let mut wk = waiting(TaggedContinuation::Empty, false, false, &[0]);
        assert!(
            to_receive(&[wk.clone()], &[channel("a")]).is_some(),
            "a well-formed row renders"
        );
        wk.patterns[0].free_count = -1;
        assert!(
            to_receive(&[wk], &[channel("a")]).is_none(),
            "a negative stored count refuses the whole row"
        );
        assert!(
            super::MALFORMED_PATTERN.contains("negative free count"),
            "…and the snapshot says so rather than rendering a different term"
        );
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

        let rendered = to_receive(&[wk], &[channel("a"), channel("b")]).expect("well formed");
        assert_eq!(rendered.receives.len(), 1);
        let receive = &rendered.receives[0];
        assert_eq!(*receive.body, body, "the continuation is the loop body");
        assert_eq!(
            i32::from(receive.bind_count),
            3,
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
            let rendered = to_receive(&[wk], &[channel("a")]).expect("well formed");
            let receive = &rendered.receives[0];
            assert_eq!(
                *receive.body,
                Par::default(),
                "a non-rholang continuation has no body to print"
            );
            assert_eq!(i32::from(receive.bind_count), 0, "…and no binders to count");
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
