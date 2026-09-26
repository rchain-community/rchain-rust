//! The rholang↔rspace runtime bridge (port of `interpreter/storage/`).
//!
//! `RhoHistoryRepository` specializes `rspace::HistoryRepository` to the rholang types;
//! [`ChargingRSpace`] adapts the async rspace `Tuplespace` to the async rholang `reduce::Tuplespace`.

use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use rchain_models::ast::{EList, Expr, Par, Var};
use rchain_models::par_ops::from_expr;
use rchain_models::runtime::{BindPattern, ListParWithRandom, TaggedContinuation};
use rchain_models::sorted::SortedProc;
use rchain_rspace::errors::RSpaceError;
use rchain_rspace::history::history_repository::HistoryRepository;
use rchain_rspace::match_::Match;
use rchain_rspace::scheduled_space::{
    PendingProduce as RspacePendingProduce, ScheduledConsume as RspaceScheduledConsume,
    ScheduledProduce as RspaceScheduledProduce,
};
use rchain_rspace::tuple_space::{
    ContResult, Result as RSpaceResult, Tuplespace as RSpaceTuplespace,
};

use rchain_crypto::hash::blake2b512_random::Blake2b512Random;

use crate::accounting::{Cost, CostAccounting, Costs};
use crate::errors::RholangError;
use crate::matcher::{fold_match, spatial_match, FreeMap};
use crate::reduce::{Application, PendingProduce, ScheduledConsume, ScheduledProduce, Tuplespace};

/// The rholang history repository (port of `RhoHistoryRepository`).
pub type RhoHistoryRepository =
    Arc<HistoryRepository<SortedProc, BindPattern, ListParWithRandom, TaggedContinuation>>;

/// The rholang tuplespace (port of `RhoTuplespace`).
pub type RhoTuplespace =
    Arc<dyn RSpaceTuplespace<SortedProc, BindPattern, ListParWithRandom, TaggedContinuation>>;

/// Convert an rspace produce/consume result into the rholang `Application` (port of
/// `unpackOptionWithPeek`).
pub fn to_application(
    r: Option<(
        ContResult<SortedProc, BindPattern, TaggedContinuation>,
        Vec<RSpaceResult<SortedProc, ListParWithRandom>>,
    )>,
) -> Application {
    r.map(|(cont, data)| {
        (
            cont.continuation,
            data.into_iter()
                .map(|d| (d.channel, d.matched_datum, d.removed_datum, d.persistent))
                .collect(),
            cont.peek,
        )
    })
}

/// The spatial matcher instance for `(BindPattern, ListParWithRandom)` (port of `matchListPar`).
#[derive(Clone)]
pub struct RhoMatch;

impl Match<BindPattern, ListParWithRandom> for RhoMatch {
    fn get(
        &self,
        pattern: &BindPattern,
        data: &ListParWithRandom,
    ) -> Result<Option<ListParWithRandom>, RSpaceError> {
        let data_pars: Vec<Par> = data.pars.iter().map(|p| p.as_par().clone()).collect();
        let pattern_pars: Vec<Par> = pattern
            .patterns
            .iter()
            .map(|p| p.as_par().clone())
            .collect();
        // A matcher-internal failure is **not** "no match": this arm used `.ok()?`, which turned a
        // `RholangError::BugFoundError` into `None`, i.e. the datum would silently never match. The
        // oracle's `Match.get` is `F[Option[A]]` (`rspace/.../Match.scala:11`); the port's `Option`
        // return is what forced the two answers into one (AUDIT C52).
        let matches = fold_match(
            &data_pars,
            &pattern_pars,
            pattern.remainder.as_ref(),
            &FreeMap::new(),
            &spatial_match,
        )
        .map_err(|e| RSpaceError::MatcherFailed(e.to_string()))?;
        let Some((caught_rem, free_map)) = matches.into_iter().next() else {
            return Ok(None);
        };

        let mut remainder_map = free_map;
        if let Some(Var::FreeVar(level)) = pattern.remainder.as_ref() {
            remainder_map.insert(
                *level,
                from_expr(Expr::EList(EList {
                    ps: caught_rem,
                    ..Default::default()
                })),
            );
        }

        // **Padded, as the oracle pads.** A level the pattern's `free_count` declares and the matcher
        // did not bind becomes the empty par — the Scala's own `toSeq`
        // (`rholang/interpreter/storage/package.scala:22-29`'s `case None => Par.defaultInstance`) —
        // and that is *semantics*, not a flatten: for `for (@{x | y} <- @Nil) { ... }` the binding is
        // greedy, so `x` gets the whole par and `y` gets the empty one, and
        // `legacy/.../matching-parallel-processes.rho` documents precisely that as its expected output
        // (`@1!("success" | "success") | @2!(Nil)`).
        //
        // U10 refused here (AUDIT C52), on the reading that a level the map lacks contradicts the
        // count's meaning. **The corpus refuted the reading by measurement** (U17): a matcher that is
        // greedy *by design* leaves the trailing levels unbound, so the state is reachable from a
        // well-formed term — the refusal turned a differential vector into an error. The count's
        // meaning is "the levels the pattern *names*", not "the levels a given match happened to fill".
        let mut pars = Vec::new();
        for level in 0..pattern.free_count {
            pars.push(SortedProc::new(
                remainder_map.get(&level).cloned().unwrap_or_default(),
            ));
        }
        Ok(Some(ListParWithRandom {
            pars,
            random_state: data.random_state.clone(),
        }))
    }
}

/// The identity of an op whose result is being charged (the Scala's `TriggeredBy`): the random state
/// it was created with, which is what the refunds compare a continuation's id against.
fn consume_id(continuation: &TaggedContinuation) -> Result<Blake2b512Random, RholangError> {
    match continuation {
        TaggedContinuation::ParBody(value) => Ok(value.random_state.copy()),
        // The Scala's `Blake2b512Random(ByteBuffer.allocate(8).putLong(value).array())`: a big-endian
        // long, which is `ByteBuffer`'s default order.
        TaggedContinuation::ScalaBodyRef(value) => {
            Ok(Blake2b512Random::from_init(&value.to_be_bytes()))
        }
        TaggedContinuation::Empty => {
            Err(RholangError::BugFoundError("Damn you pROTOBUF".to_string()))
        }
    }
}

/// The charging tuplespace bridge: adapts the async rspace to the async rholang `Tuplespace` (port
/// of `ChargingRSpace`). Charges the produce/consume storage + event/COMM costs, and **refunds** the
/// storage an op consumed when it matched:
///
/// * a non-persistent continuation's consume storage, and the consume storage of the persistent
///   continuation this op itself triggered (`refundForConsume`);
/// * the produce storage of every datum the op removed, save a *persistent* datum the op did not
///   itself produce — that one stays in the space, so its storage is still owed
///   (`refundForRemovingProduces`).
///
/// Both are charged *before* the event and COMM costs, in the Scala's order (`ChargingRSpace.scala:121-127`),
/// which is what makes them refunds rather than rebates: a charge that arrives after the exhaustion
/// check cannot save a deploy that had already run out.
#[derive(Clone)]
pub struct ChargingRSpace {
    space: RhoTuplespace,
    cost: Arc<CostAccounting>,
}

impl ChargingRSpace {
    /// Wrap `space` with the cost cell, charging produce/consume (port of `chargingRSpace`).
    pub fn new(space: RhoTuplespace, cost: Arc<CostAccounting>) -> Self {
        ChargingRSpace { space, cost }
    }

    /// The storage refunds a *matched* op is owed (port of `handleResult`'s `Some` arm, and of the two
    /// helpers it calls).
    ///
    /// `trigger_id` is the id of the op that triggered the COMM: for a produce, its own random state;
    /// for a consume, the id of the continuation *it* installed, which is what `consume_id` derives.
    /// (The trigger's *persistence* is not needed here — the caller's `!persist` guard on the event
    /// cost is the same value, which is the Scala's `lastIteration = !triggeredBy.persistent`.)
    fn refund_storage(
        &self,
        cont: &ContResult<SortedProc, BindPattern, TaggedContinuation>,
        data: &[RSpaceResult<SortedProc, ListParWithRandom>],
        trigger_id: &Blake2b512Random,
    ) -> Result<(), RholangError> {
        // `refundForConsume`. A persistent continuation that is *not* the one this op triggered keeps
        // its storage charged: it stays in the space and will be charged again when it next fires (the
        // Scala's comment: "We refund for non-persistent continuations, and for the persistent
        // continuation triggering the comm. That persistent continuation is going to be charged for
        // (without refund) once it has no matches in TS"). A non-persistent continuation is consumed by
        // the match, so its storage is refunded here — that is the whole point: the consume storage was
        // charged up front, and the op that paid it is gone.
        if !cont.persistent || &consume_id(&cont.continuation)? == trigger_id {
            let cost =
                Costs::storage_cost_consume(&cont.channels, &cont.patterns, &cont.continuation);
            self.cost
                .charge(Cost::new(-cost.value, "consume storage refund"))?;
        }
        // `refundForRemovingProduces`: every datum this op removed is refunded its produce cost —
        // except a persistent datum that this op did not itself produce, which stays in the space. The
        // channel is taken from the *continuation's* channels, positionally, as the Scala's `zip` does
        // (`data.channel` is not consulted).
        let mut refund: i64 = 0;
        for (channel, datum) in cont.channels.iter().zip(data) {
            if !datum.persistent || &datum.removed_datum.random_state == trigger_id {
                refund += Costs::storage_cost_produce(channel, &datum.removed_datum).value;
            }
        }
        self.cost
            .charge(Cost::new(-refund, "produces storage refund"))?;
        Ok(())
    }

    /// **The C100 depth check, in one place so that *every* produce path carries it** (AUDIT C100).
    ///
    /// A runtime-built value can be deeper than any parsed term — a contract folding its accumulator
    /// reaches depth `n` in `O(n)` steps — and every consumer of a stored value then recurses once per
    /// level. This refuses such a value at the boundary, before the storage charge, so a refused value
    /// is not one the space accepted and then billed for.
    ///
    /// It is a method rather than a loop inside `produce` because that was this unit's first draft and
    /// its first mistake: the *scheduled* produce (`produce_at`) reaches RSpace without passing through
    /// `produce`, so the guard covered one of the two produce entries. One implementation, two callers.
    /// The routes this still does **not** cover (a produce's channel, a consume's channels/patterns, a
    /// continuation body, state restored at boot) are the residues named in AUDIT C100's row.
    fn check_value_depth(&self, data: &ListParWithRandom) -> Result<(), RholangError> {
        for p in &data.pars {
            if rchain_models::types::exceeds_value_depth(p.as_par(), MAX_VALUE_DEPTH) {
                return Err(RholangError::ReduceError(
                    "a produced value is nested too deeply".to_string(),
                ));
            }
        }
        Ok(())
    }
}

/// The deepest **runtime-built value** RSpace will accept, and why it is not the parser's number.
///
/// `parser::MAX_AST_DEPTH` (768) bounds a *parsed term*, measured against the parser-route walks: a
/// debug build survives AST depth 732 and aborts at 994. The **value** route has different, larger
/// frames, and its own measurement says so — a fold to depth 101 evaluates (7.3 s of CPU in debug) and
/// a fold to 401 **aborts the process** (`thread 'tokio-rt-worker' has overflowed its stack`, SIGABRT)
/// inside `eval_single_expr`'s recursion over the value. So the value bound is below that abort with
/// margin, 256 rather than 768, and the two numbers are kept apart because they were *measured*
/// against different walks: sharing one would have been a bound that does not fire before the crash it
/// exists to prevent (and did not, in this unit's first draft — the guard sat at 768 while the fold
/// only reached 401, so it never fired and the test aborted the process).
///
/// 256 is generous for real programs: depth counts *nesting*, not length, so a flat list of any size is
/// depth 2, and it is the accumulator-nesting shape — which is what an attacker builds and no contract
/// needs — that this refuses.
///
/// **The relation to the parser's bound is structural, not asserted.** It must never sit *above*
/// `MAX_AST_DEPTH` — a value the parser admitted and the space then refused would make the parser's
/// budget meaningless — so it is written as the smaller of the measured number and the parser's. A
/// parser bound lowered below 256 lowers this with it; the ordering cannot drift. The spellings tried
/// first, and why this one: a bare `256` with a `#[test]` asserting the relation (a test of two
/// constants can never fail — the linter refused the shape, correctly); the `const` block the linter
/// suggests in its place (an `assert!` is a *panic site in production code*, which
/// `tools/audit-type-system.sh` counts, so it fails the gate); a local `#[allow]` (the tree carries
/// none, and the debt list is central in CI rather than scattered); and `256.min(..)`, which is not
/// const-callable on this toolchain. The `if` costs no counted site and no exception, and it says what
/// the invariant says.
const MAX_VALUE_DEPTH: usize = if 256 < crate::parser::MAX_AST_DEPTH {
    256
} else {
    crate::parser::MAX_AST_DEPTH
};

#[async_trait]
impl Tuplespace for ChargingRSpace {
    async fn produce(
        &self,
        channel: &SortedProc,
        data: ListParWithRandom,
        persist: bool,
    ) -> Result<Application, RholangError> {
        // **The value route into RSpace is depth-checked here (AUDIT C100)** — see
        // `check_value_depth`, which both produce paths call. What it bounds is the values the *space*
        // holds, and so every later reader of them (`sort_par` on the way in, the matcher and the
        // printer on the way out); what it does *not* bound is the evaluator's own recursion while
        // building the value, because this runs after that walk. The honest statement of the mechanism
        // is therefore "the deepest value ever built is capped at the bound + 1": iteration `i` of a
        // folding contract evaluates depth `i` and then produces depth `i + 1`, so the produce at
        // `i = MAX_VALUE_DEPTH` is what stops it. It is `O(size)` once per store, not `O(depth)` per
        // evaluation, so the reducer's hot path is untouched.
        self.check_value_depth(&data)?;
        self.cost
            .charge(Costs::storage_cost_produce(channel, &data))?;
        // The triggering op's id, captured before the datum moves into the space (the Scala's
        // `TriggeredBy = Produce(data.randomState, persist)`).
        let trigger_id = data.random_state.copy();
        let result = self
            .space
            .produce(channel.clone(), data, persist)
            .await
            .map_err(|e| RholangError::ReduceError(e.to_string()))?;
        match &result {
            None => self.cost.charge(Costs::event_storage_cost(1))?,
            Some((cont, data_list)) => {
                self.refund_storage(cont, data_list, &trigger_id)?;
                if !persist {
                    self.cost.charge(Costs::event_storage_cost(1))?;
                }
                self.cost
                    .charge(Costs::comm_event_storage_cost(cont.channels.len() as i64))?;
            }
        }
        Ok(to_application(result))
    }

    async fn consume(
        &self,
        channels: &[SortedProc],
        patterns: &[BindPattern],
        continuation: TaggedContinuation,
        persist: bool,
        peeks: BTreeSet<usize>,
    ) -> Result<Application, RholangError> {
        self.cost.charge(Costs::storage_cost_consume(
            channels,
            patterns,
            &continuation,
        ))?;
        // The triggering op's id (the Scala's `TriggeredBy = Consume(consumeId(continuation), persist,
        // channels.size)`). Computed *before* the space call because the continuation moves into it —
        // the Scala computes it after the op instead; the only difference is which partial state a
        // protobuf-bug continuation leaves behind, and that fails the deploy either way.
        let trigger_id = consume_id(&continuation)?;
        let result = self
            .space
            .consume(channels, patterns, continuation, persist, peeks)
            .await
            .map_err(|e| RholangError::ReduceError(e.to_string()))?;
        match &result {
            None => self
                .cost
                .charge(Costs::event_storage_cost(channels.len() as i64))?,
            Some((cont, data_list)) => {
                self.refund_storage(cont, data_list, &trigger_id)?;
                if !persist {
                    self.cost
                        .charge(Costs::event_storage_cost(channels.len() as i64))?;
                }
                self.cost
                    .charge(Costs::comm_event_storage_cost(cont.channels.len() as i64))?;
            }
        }
        Ok(to_application(result))
    }

    async fn produce_at(
        &self,
        path: Vec<u16>,
        channel: &SortedProc,
        data: ListParWithRandom,
        persist: bool,
    ) -> Result<ScheduledProduce, RholangError> {
        // The same depth check as the plain path, and the reason it is a shared method: this entry
        // reaches RSpace without passing through `produce`, so a guard written there alone left this
        // one open (AUDIT C100).
        self.check_value_depth(&data)?;
        self.cost
            .charge(Costs::storage_cost_produce(channel, &data))?;
        let scheduled = self
            .space
            .produce_at(path, channel.clone(), data, persist)
            .await
            .map_err(|e| RholangError::ReduceError(e.to_string()))?;
        let RspaceScheduledProduce {
            joins,
            result,
            phase_two,
            release,
        } = scheduled;
        // Propagate the op's error before charging (mirror of the plain `produce` override).
        let result = result.map_err(|e| RholangError::ReduceError(e.to_string()))?;
        if phase_two.is_none() {
            // Phase one stored the datum inline (no match), so the produce event cost lands with
            // phase one; a deferred commit charges its event/COMM costs in `commit_produce`.
            self.cost.charge(Costs::event_storage_cost(1))?;
        }
        Ok(ScheduledProduce {
            joins,
            application: to_application(result),
            phase_two: phase_two.map(|p| PendingProduce {
                trigger: p.trigger,
                data: p.data,
                persist: p.persist,
            }),
            release,
        })
    }

    async fn consume_at(
        &self,
        path: Vec<u16>,
        channels: &[SortedProc],
        patterns: &[BindPattern],
        continuation: TaggedContinuation,
        persist: bool,
        peeks: BTreeSet<usize>,
    ) -> Result<ScheduledConsume, RholangError> {
        self.cost.charge(Costs::storage_cost_consume(
            channels,
            patterns,
            &continuation,
        ))?;
        let scheduled = self
            .space
            .consume_at(path, channels, patterns, continuation, persist, peeks)
            .await
            .map_err(|e| RholangError::ReduceError(e.to_string()))?;
        let RspaceScheduledConsume { result, release } = scheduled;
        // Propagate the op's error before charging (mirror of the plain `consume` override).
        let result = result.map_err(|e| RholangError::ReduceError(e.to_string()))?;
        match &result {
            None => self
                .cost
                .charge(Costs::event_storage_cost(channels.len() as i64))?,
            Some((cont, _)) => {
                if !persist {
                    self.cost
                        .charge(Costs::event_storage_cost(channels.len() as i64))?;
                }
                self.cost
                    .charge(Costs::comm_event_storage_cost(cont.channels.len() as i64))?;
            }
        }
        Ok(ScheduledConsume {
            application: to_application(result),
            release,
        })
    }

    async fn commit_produce(&self, pending: PendingProduce) -> Result<Application, RholangError> {
        let persist = pending.persist;
        let result = self
            .space
            .commit_produce(RspacePendingProduce {
                trigger: pending.trigger,
                data: pending.data,
                persist,
            })
            .await
            .map_err(|e| RholangError::ReduceError(e.to_string()))?;
        // The event/COMM costs land with the commit (the op's completion, phase two).
        match &result {
            None => self.cost.charge(Costs::event_storage_cost(1))?,
            Some((cont, _)) => {
                if !persist {
                    self.cost.charge(Costs::event_storage_cost(1))?;
                }
                self.cost
                    .charge(Costs::comm_event_storage_cost(cont.channels.len() as i64))?;
            }
        }
        Ok(to_application(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rchain_crypto::hash::blake2b512_random::Blake2b512Random;
    use rchain_models::ast::Par;
    use rchain_rspace::errors::RSpaceError;
    use std::sync::Mutex;

    use crate::accounting::Cost;

    fn par(exprs: Vec<Expr>) -> Par {
        Par {
            exprs,
            ..Default::default()
        }
    }

    fn lpw(pars: Vec<Par>) -> ListParWithRandom {
        ListParWithRandom {
            pars: pars.into_iter().map(SortedProc::new).collect(),
            random_state: Blake2b512Random::new_random(128),
        }
    }

    struct MockSpace {
        produced: Mutex<Vec<(SortedProc, ListParWithRandom, bool)>>,
        /// When set, `produce` reports a match instead of storing — the shape a refund is owed for.
        matched: Mutex<Option<Matched>>,
    }

    /// What `produce` returns when it matched: the continuation, and the datum it removed.
    type Matched = (
        ContResult<SortedProc, BindPattern, TaggedContinuation>,
        Vec<RSpaceResult<SortedProc, ListParWithRandom>>,
    );

    #[async_trait]
    impl RSpaceTuplespace<SortedProc, BindPattern, ListParWithRandom, TaggedContinuation>
        for MockSpace
    {
        async fn consume(
            &self,
            _channels: &[SortedProc],
            _patterns: &[BindPattern],
            _continuation: TaggedContinuation,
            _persist: bool,
            _peeks: BTreeSet<usize>,
        ) -> Result<
            Option<(
                ContResult<SortedProc, BindPattern, TaggedContinuation>,
                Vec<RSpaceResult<SortedProc, ListParWithRandom>>,
            )>,
            RSpaceError,
        > {
            Ok(None)
        }

        async fn produce(
            &self,
            channel: SortedProc,
            data: ListParWithRandom,
            persist: bool,
        ) -> Result<
            Option<(
                ContResult<SortedProc, BindPattern, TaggedContinuation>,
                Vec<RSpaceResult<SortedProc, ListParWithRandom>>,
            )>,
            RSpaceError,
        > {
            self.produced
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push((channel, data, persist));
            Ok(self
                .matched
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone())
        }

        async fn install(
            &self,
            _channels: &[SortedProc],
            _patterns: &[BindPattern],
            _continuation: TaggedContinuation,
        ) -> Result<Option<(TaggedContinuation, Vec<ListParWithRandom>)>, RSpaceError> {
            Ok(None)
        }
    }

    /// A mock space plus a fresh cost account, for the scheduled-path charge tests.
    fn charging_space(initial: i64) -> (ChargingRSpace, Arc<CostAccounting>, Arc<MockSpace>) {
        let mock = Arc::new(MockSpace {
            produced: Mutex::new(Vec::new()),
            matched: Mutex::new(None),
        });
        let cost = Arc::new(CostAccounting::from_initial(Cost::new(initial, "init")));
        let charging = ChargingRSpace::new(mock.clone() as RhoTuplespace, cost.clone());
        (charging, cost, mock)
    }

    fn sample_channel() -> SortedProc {
        SortedProc::new(par(vec![Expr::GInt(1)]))
    }

    /// Where the storage cost lands on the scheduled path: **up front**, with phase one. The corpus
    /// tests exercise `produce_at` end-to-end but cannot say *where* a charge lands, which is what
    /// this pins.
    #[tokio::test]
    async fn produce_at_charges_the_storage_up_front() {
        let (charging, cost, _) = charging_space(1_000_000);
        let channel = sample_channel();
        let data = lpw(vec![par(vec![Expr::GInt(2)])]);

        charging
            .produce_at(Vec::new(), &channel, data.clone(), false)
            .await
            .expect("produce_at");

        // The mock never matches, so phase one stored the datum inline and the produce event cost
        // lands with it rather than being deferred to a commit. The expected total is built by
        // charging a reference account the same two costs, rather than by arithmetic on `Cost`.
        let reference = CostAccounting::from_initial(Cost::new(1_000_000, "init"));
        reference
            .charge(Costs::storage_cost_produce(&channel, &data))
            .expect("storage cost");
        reference
            .charge(Costs::event_storage_cost(1))
            .expect("event cost");
        assert_eq!(cost.total_charged(), reference.total_charged());
    }

    /// An exhausted balance fails **before** the datum is stored — the charge is not a flush after
    /// the fact, so a caller cannot get a free store out of an underfunded deploy.
    #[tokio::test]
    async fn produce_at_fails_before_storing_when_the_balance_is_spent() {
        let (charging, _, mock) = charging_space(1);
        let err = charging
            .produce_at(
                Vec::new(),
                &sample_channel(),
                lpw(vec![par(vec![Expr::GInt(2)])]),
                false,
            )
            .await
            .expect_err("an exhausted balance must fail");
        assert!(!err.to_string().is_empty());
        assert!(
            mock.produced.lock().unwrap().is_empty(),
            "nothing may be stored when the storage charge fails"
        );
    }

    /// The other half of the split: when a produce defers to phase two, the event cost is charged at
    /// the **commit**, not up front — so an aborted phase two does not pay for an event it never
    /// produced.
    #[tokio::test]
    async fn commit_produce_charges_the_event_at_the_commit() {
        let (charging, cost, _) = charging_space(1_000_000);
        let before = cost.total_charged();

        charging
            .commit_produce(PendingProduce {
                trigger: sample_channel(),
                data: lpw(vec![par(vec![Expr::GInt(2)])]),
                persist: false,
            })
            .await
            .expect("commit_produce");

        let charged = cost.total_charged() - before;
        let reference = CostAccounting::from_initial(Cost::new(1_000_000, "init"));
        reference
            .charge(Costs::event_storage_cost(1))
            .expect("event cost");
        assert_eq!(
            charged,
            reference.total_charged(),
            "the commit charges the event cost for the produce it completes"
        );
    }

    /// **A matched op is refunded the storage it consumed, and the refund lands *before* the event
    /// and COMM costs** (`ChargingRSpace.scala:105-127`). Two assertions, because the two are
    /// different facts:
    ///
    /// * the matched call costs what the *same* op with no match costs, plus the COMM cost the match
    ///   adds, **minus the two refunds** — the consume storage of the continuation and the produce
    ///   storage of the datum the op removed, which the Scala charges as negative `Cost`s;
    /// * with a balance that cannot cover the event/COMM costs on its own, the op still **completes**,
    ///   which is only true if the refunds were credited first. Order is not decoration: `charge`
    ///   raises `OutOfPhlogistonsError` when the balance would go negative, so a refund arriving after
    ///   the check cannot save a deploy that had already run out.
    #[tokio::test]
    async fn a_matched_produce_refunds_its_storage_before_the_event_costs() {
        let channel = sample_channel();
        let data = lpw(vec![par(vec![Expr::GInt(42)])]);
        let removed = lpw(vec![par(vec![Expr::GInt(7)])]);
        let cont = ContResult {
            continuation: TaggedContinuation::ParBody(rchain_models::runtime::ParWithRandom {
                body: SortedProc::new(par(vec![Expr::GInt(0)])),
                random_state: Blake2b512Random::new_random(128),
            }),
            // Non-persistent: the continuation is consumed by the match, so its storage is refunded.
            persistent: false,
            channels: vec![channel.clone()],
            patterns: vec![BindPattern {
                patterns: Vec::new(),
                free_count: 0,
                remainder: None,
            }],
            peek: false,
        };
        let removed_results = vec![RSpaceResult {
            channel: channel.clone(),
            matched_datum: removed.clone(),
            removed_datum: removed.clone(),
            persistent: false,
        }];

        // One space per reading: `total_charged` is cumulative, so two calls in one account would
        // compare a sum against a single call's charges.
        let (unmatched_space, unmatched_cost, _) = charging_space(1_000_000);
        unmatched_space
            .produce(&channel, data.clone(), false)
            .await
            .unwrap();
        let unmatched = unmatched_cost.total_charged();

        let (matched_space, matched_cost, mock) = charging_space(1_000_000);
        *mock.matched.lock().unwrap() = Some((cont.clone(), removed_results));
        matched_space.produce(&channel, data, false).await.unwrap();
        let matched = matched_cost.total_charged();

        let consume_refund =
            Costs::storage_cost_consume(&cont.channels, &cont.patterns, &cont.continuation).value;
        let produce_refund = Costs::storage_cost_produce(&channel, &removed).value;
        assert!(
            consume_refund > 0 && produce_refund > 0,
            "both refunds have content ({consume_refund}, {produce_refund})"
        );
        assert_eq!(
            matched,
            unmatched + Costs::comm_event_storage_cost(1).value - consume_refund - produce_refund,
            "a match refunds the consume storage and the removed datum's produce storage \
             (consume {consume_refund}, produce {produce_refund})"
        );

        let cont_for_tight = cont.clone();
        // Order, which the total above cannot see (a sum is order-independent but a *running* balance
        // is not): `charge` refuses a step that would take the balance negative, so what a deploy
        // needs is the largest prefix sum of its charges, and a refund credited *before* a later charge
        // lowers that peak. With the refunds last the peak is `storage + event + comm`; with them first
        // it is that minus the refunds — so a balance in between completes under one order and fails
        // under the other.
        let tight_datum = lpw(vec![par(vec![Expr::GInt(1)])]);
        let removed_results_for_tight = vec![RSpaceResult {
            channel: channel.clone(),
            matched_datum: tight_datum.clone(),
            removed_datum: tight_datum.clone(),
            persistent: false,
        }];
        let storage = Costs::storage_cost_produce(&channel, &tight_datum).value;
        let event = Costs::event_storage_cost(1).value;
        let comm = Costs::comm_event_storage_cost(1).value;
        let tight_refund = produce_refund;
        let peak_with_refunds_first = storage + event + comm - consume_refund - tight_refund;
        assert!(
            peak_with_refunds_first < storage + event + comm,
            "the refunds have to move the peak for this to test order at all"
        );

        let (tight_space, tight_cost, mock) = charging_space(1_000_000);
        *mock.matched.lock().unwrap() = Some((cont, removed_results_for_tight.clone()));
        tight_cost.set(Cost::new(peak_with_refunds_first, "tight"));
        assert!(
            tight_space
                .produce(&channel, tight_datum.clone(), false)
                .await
                .is_ok(),
            "at the refunds-first peak the deploy completes — the refund was credited before the \
             event and COMM charges"
        );

        let (tighter_space, tighter_cost, mock) = charging_space(1_000_000);
        *mock.matched.lock().unwrap() = Some((cont_for_tight, removed_results_for_tight));
        tighter_cost.set(Cost::new(peak_with_refunds_first - 1, "tighter"));
        assert!(
            tighter_space
                .produce(&channel, tight_datum, false)
                .await
                .is_err(),
            "one phlo less and it does not, which is what makes the assertion above about the \
             refund and not about a generous balance"
        );
    }

    #[tokio::test]
    async fn charging_rspace_charges_and_enforces_balance() {
        let mock: RhoTuplespace = Arc::new(MockSpace {
            produced: Mutex::new(Vec::new()),
            matched: Mutex::new(None),
        });
        let cost = Arc::new(CostAccounting::from_initial(Cost::new(1_000_000, "init")));
        let charging = ChargingRSpace::new(mock, cost.clone());

        charging
            .produce(
                &SortedProc::new(par(vec![Expr::GInt(1)])),
                lpw(vec![par(vec![Expr::GInt(2)])]),
                false,
            )
            .await
            .unwrap();
        assert!(
            cost.total_charged() > 0,
            "produce must charge storage/event cost"
        );

        // A near-zero balance is exhausted by the upfront storage charge.
        let tiny_cost = Arc::new(CostAccounting::from_initial(Cost::new(1, "tiny")));
        let tiny = ChargingRSpace::new(
            Arc::new(MockSpace {
                produced: Mutex::new(Vec::new()),
                matched: Mutex::new(None),
            }),
            tiny_cost,
        );
        let err = tiny
            .produce(
                &SortedProc::new(par(vec![Expr::GInt(1)])),
                lpw(vec![par(vec![Expr::GInt(2)])]),
                false,
            )
            .await;
        assert!(err.is_err(), "exhausted balance must fail produce");
    }

    #[test]
    fn rho_match_binds_free_vars() {
        let pattern = BindPattern {
            patterns: vec![SortedProc::new(Par {
                exprs: vec![Expr::EVar(Box::new(Var::FreeVar(0)))],
                connective_used: true,
                ..Default::default()
            })],
            remainder: None,
            free_count: 1,
        };
        let data = ListParWithRandom {
            pars: vec![SortedProc::new(par(vec![Expr::GInt(42)]))],
            random_state: rchain_crypto::hash::blake2b512_random::Blake2b512Random::new_random(128),
        };
        let result = RhoMatch
            .get(&pattern, &data)
            .expect("the matcher decided")
            .expect("a bare free variable matches any datum");
        assert_eq!(
            result.pars,
            vec![SortedProc::new(par(vec![Expr::GInt(42)]))]
        );
    }

    /// **A level the pattern names and the match did not fill is padded, as the oracle pads.**
    ///
    /// `RhoMatch::get` fills the continuation's environment from the matcher's free map, one level per
    /// `0..free_count`, and the Scala answers a missing one with the empty par
    /// (`interpreter/storage/package.scala:22-29`'s `case None => Par.defaultInstance`). The matcher is
    /// greedy by design, so `for (@{x | y} <- @Nil)` binds `x` to the whole par and leaves `y` empty —
    /// `legacy/.../matching-parallel-processes.rho` documents that as its expected output.
    ///
    /// U10 refused here (AUDIT C52); the legacy corpus refuted the reading by measurement (U17).
    ///
    /// Falsifier, in the form that matters: restore the `MatcherFailed` and this fails — and so does
    /// `legacy_contracts`, whose corpus program is the oracle's own vector for it.
    #[test]
    fn rho_match_pads_a_free_count_its_pattern_does_not_bind() {
        let pattern = BindPattern {
            patterns: vec![SortedProc::new(Par {
                exprs: vec![Expr::EVar(Box::new(Var::FreeVar(0)))],
                connective_used: true,
                ..Default::default()
            })],
            remainder: None,
            free_count: 3,
        };
        let data = ListParWithRandom {
            pars: vec![SortedProc::new(par(vec![Expr::GInt(42)]))],
            random_state: rchain_crypto::hash::blake2b512_random::Blake2b512Random::new_random(128),
        };

        // The oracle's answer: the two levels the pattern names and the match did not fill are the
        // empty par, and the continuation is applied with them.
        let padded = RhoMatch
            .get(&pattern, &data)
            .expect("the matcher decided")
            .expect("a bare free variable matches any datum");
        assert_eq!(padded.pars.len(), 3, "one binding per declared level");
        assert_eq!(
            padded.pars[0],
            SortedProc::new(par(vec![Expr::GInt(42)])),
            "the level the match filled carries the datum"
        );
        for extra in &padded.pars[1..] {
            assert_eq!(
                *extra,
                SortedProc::new(Par::default()),
                "a declared level the match did not fill is the empty par, as `toSeq` pads it"
            );
        }
    }

    #[test]
    fn to_application_converts() {
        let cont = ContResult {
            continuation: TaggedContinuation::Empty,
            persistent: false,
            channels: vec![SortedProc::new(par(vec![Expr::GInt(1)]))],
            patterns: vec![],
            peek: true,
        };
        let data =
            RSpaceResult {
                channel: SortedProc::new(par(vec![Expr::GInt(1)])),
                matched_datum: ListParWithRandom {
                    pars: vec![],
                    random_state:
                        rchain_crypto::hash::blake2b512_random::Blake2b512Random::new_random(128),
                },
                removed_datum: ListParWithRandom {
                    pars: vec![],
                    random_state:
                        rchain_crypto::hash::blake2b512_random::Blake2b512Random::new_random(128),
                },
                persistent: false,
            };
        let app = to_application(Some((cont, vec![data]))).unwrap();
        assert!(matches!(app.0, TaggedContinuation::Empty));
        assert!(app.2);
        assert_eq!(app.1.len(), 1);
    }

    /// A `Par` whose value-depth is exactly `n`: `New` nests one level per step, and the empty `Par` is
    /// depth 1. Built in memory rather than parsed, so the boundary is testable with no parser, no
    /// runtime and no big stack.
    fn nested_par(n: usize) -> Par {
        let mut p = Par::default();
        for _ in 1..n {
            p = Par {
                news: vec![rchain_models::ast::New {
                    p: Box::new(p),
                    ..Default::default()
                }],
                ..Default::default()
            };
        }
        p
    }

    /// **The value bound's boundary, at *both* produce entries** (AUDIT C100). A datum *at* the bound is
    /// stored and charged for; one past it is refused with a message that names the reason, stored
    /// nowhere and charged nothing — the check runs before the storage charge, so a refused value never
    /// entered and costs nothing. Both directions matter: a guard that refused everything, or nothing,
    /// would pass a one-sided test.
    ///
    /// `produce_at` earns its place here because it is **not** a route through `produce` — it reaches
    /// RSpace directly, and a guard written only inside `produce` left it open (which is the shape this
    /// unit's first draft shipped; the fake space's `produce_at` defaults to `produce`, so a test that
    /// only exercised `produce` could not have noticed).
    #[tokio::test]
    async fn a_value_at_the_bound_is_stored_and_one_past_it_is_refused_on_both_produce_paths() {
        for n in [1usize, 2, MAX_VALUE_DEPTH] {
            let (charging, cost, mock) = charging_space(1_000_000);
            charging
                .produce(&sample_channel(), lpw(vec![nested_par(n)]), false)
                .await
                .unwrap_or_else(|e| panic!("depth {n} is at the bound and must be stored: {e:?}"));
            assert_eq!(
                mock.produced.lock().unwrap().len(),
                1,
                "depth {n} was not stored"
            );
            assert!(
                cost.total_charged() > 0,
                "depth {n} must still be charged for"
            );

            let (charging, cost, mock) = charging_space(1_000_000);
            charging
                .produce_at(
                    Vec::new(),
                    &sample_channel(),
                    lpw(vec![nested_par(n)]),
                    false,
                )
                .await
                .unwrap_or_else(|e| {
                    panic!("scheduled: depth {n} is at the bound and must be stored: {e:?}")
                });
            assert_eq!(
                mock.produced.lock().unwrap().len(),
                1,
                "scheduled: depth {n} was not stored"
            );
            assert!(
                cost.total_charged() > 0,
                "scheduled: depth {n} must still be charged for"
            );
        }

        let over = MAX_VALUE_DEPTH + 1;

        let (charging, cost, mock) = charging_space(1_000_000);
        let err = charging
            .produce(&sample_channel(), lpw(vec![nested_par(over)]), false)
            .await
            .expect_err("depth MAX_VALUE_DEPTH + 1 is past the bound and must be refused");
        assert!(
            err.to_string().contains("nested too deeply"),
            "the refusal has to say what it refused: {err}"
        );
        assert!(
            mock.produced.lock().unwrap().is_empty(),
            "a refused value must not be stored"
        );
        assert_eq!(
            cost.total_charged(),
            0,
            "a refused value must not be charged for: the check runs before the charge"
        );

        let (charging, cost, mock) = charging_space(1_000_000);
        let err = charging
            .produce_at(
                Vec::new(),
                &sample_channel(),
                lpw(vec![nested_par(over)]),
                false,
            )
            .await
            .expect_err("scheduled: depth MAX_VALUE_DEPTH + 1 is past the bound");
        assert!(
            err.to_string().contains("nested too deeply"),
            "scheduled: the refusal has to say what it refused: {err}"
        );
        assert!(
            mock.produced.lock().unwrap().is_empty(),
            "scheduled: a refused value must not be stored"
        );
        assert_eq!(
            cost.total_charged(),
            0,
            "scheduled: a refused value must not be charged for"
        );
    }
}
