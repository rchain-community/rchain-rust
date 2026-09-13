//! Continuation dispatch (port of `dispatch.scala`).

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, Weak};

use async_trait::async_trait;
use rchain_crypto::hash::blake2b512_random::Blake2b512Random;
use rchain_models::ast::Par;
use rchain_models::runtime::{ListParWithRandom, TaggedContinuation};

use crate::env::Env;
use crate::errors::RholangError;
use crate::reduce::Dispatch;
use crate::scheduler::DfsPath;

/// A built-in continuation handler (port of the dispatch-table function), invoked at the DFS path
/// of its `ScalaBodyRef` continuation. Handlers that reply through [`ContractCall`] pass the path
/// on so the reply's matched continuation keeps its position in the scheduler's order (Laws 20–22).
pub type ScalaBodyFn = Box<
    dyn Fn(
            Vec<ListParWithRandom>,
            DfsPath,
        ) -> Pin<Box<dyn Future<Output = Result<(), RholangError>> + Send>>
        + Send
        + Sync,
>;

/// The `ParBody` continuation evaluator: evals a body in the env built from the matched data with
/// the merged random state, at the continuation's DFS path.
pub type EvalBodyFn = Box<
    dyn Fn(
            Par,
            Env<Par>,
            Blake2b512Random,
            DfsPath,
        ) -> Pin<Box<dyn Future<Output = Result<(), RholangError>> + Send>>
        + Send
        + Sync,
>;

/// Build an environment from the data captured by a match (port of `Dispatch.buildEnv`).
pub fn build_env(data_list: &[ListParWithRandom]) -> Env<Par> {
    Env::make_env(
        data_list
            .iter()
            .flat_map(|d| d.pars.iter().map(|p| p.as_par().clone())),
    )
}

/// Dispatches a continuation: eval `ParBody`, invoke the built-in handler for `ScalaBodyRef`, or
/// no-op for `Empty` (port of `RholangAndScalaDispatcher`).
///
/// `eval` is set after construction to break the reducer↔dispatcher cycle.
pub struct RholangAndScalaDispatcher {
    dispatch_table: Mutex<BTreeMap<i64, ScalaBodyFn>>,
    eval: Mutex<Option<EvalBodyFn>>,
}

impl RholangAndScalaDispatcher {
    pub fn new(dispatch_table: BTreeMap<i64, ScalaBodyFn>) -> Self {
        RholangAndScalaDispatcher {
            dispatch_table: Mutex::new(dispatch_table),
            eval: Mutex::new(None),
        }
    }

    pub fn set_eval(&self, eval: EvalBodyFn) {
        *self.eval.lock().unwrap_or_else(|p| p.into_inner()) = Some(eval);
    }

    pub fn set_dispatch_table(&self, table: BTreeMap<i64, ScalaBodyFn>) {
        *self
            .dispatch_table
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = table;
    }
}

#[async_trait]
impl Dispatch for RholangAndScalaDispatcher {
    async fn dispatch(
        &self,
        continuation: TaggedContinuation,
        data_list: Vec<ListParWithRandom>,
        path: DfsPath,
    ) -> Result<(), RholangError> {
        match &continuation {
            TaggedContinuation::ParBody(pwr) => {
                let env = build_env(&data_list);
                // Order-sensitive merge, Scala-faithful (`dispatch.scala:33`:
                // `parWithRand.randomState +: dataList.map(_.randomState)`): continuation first, then
                // the matched data in receive-bind order. `data_list` order is canonical (extracted in
                // pattern order by `extract_data_candidates`), so this is deterministic — do NOT sort
                // here (unlike the mergeable-channel merge, which merges an unordered set of branches).
                let mut randoms: Vec<Blake2b512Random> = vec![pwr.random_state.clone()];
                randoms.extend(data_list.iter().map(|d| d.random_state.clone()));
                let merged = Blake2b512Random::merge(&randoms);
                let fut = {
                    let eval = self.eval.lock().unwrap_or_else(|p| p.into_inner());
                    let f = eval.as_ref().ok_or_else(|| {
                        RholangError::BugFoundError("dispatcher eval not set".to_string())
                    })?;
                    f(pwr.body.as_par().clone(), env, merged, path)
                };
                fut.await
            }
            TaggedContinuation::ScalaBodyRef(r) => {
                let fut = {
                    let table = self
                        .dispatch_table
                        .lock()
                        .unwrap_or_else(|p| p.into_inner());
                    match table.get(r) {
                        Some(f) => f(data_list, path),
                        None => {
                            return Err(RholangError::ReduceError(format!(
                                "dispatch: no function for {r}"
                            )))
                        }
                    }
                };
                fut.await
            }
            TaggedContinuation::Empty => Ok(()),
        }
    }
}

#[async_trait]
impl Dispatch for Arc<RholangAndScalaDispatcher> {
    async fn dispatch(
        &self,
        continuation: TaggedContinuation,
        data_list: Vec<ListParWithRandom>,
        path: DfsPath,
    ) -> Result<(), RholangError> {
        self.as_ref().dispatch(continuation, data_list, path).await
    }
}

#[async_trait]
impl Dispatch for Weak<RholangAndScalaDispatcher> {
    /// Upgrade the weak reference for the duration of the dispatch. System-process handlers hold the
    /// dispatcher weakly so the dispatcher does not keep itself (and the whole forked runtime) alive
    /// through its own dispatch table (issues #18/#23).
    async fn dispatch(
        &self,
        continuation: TaggedContinuation,
        data_list: Vec<ListParWithRandom>,
        path: DfsPath,
    ) -> Result<(), RholangError> {
        let dispatcher = self.upgrade().ok_or_else(|| {
            RholangError::BugFoundError("system dispatcher has been dropped".to_string())
        })?;
        dispatcher.dispatch(continuation, data_list, path).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rchain_models::runtime::ParWithRandom;
    use rchain_models::sorted::SortedProc;

    fn data(pars: Vec<Par>) -> ListParWithRandom {
        ListParWithRandom {
            pars: pars.into_iter().map(SortedProc::new).collect(),
            random_state: Blake2b512Random::new_random(128),
        }
    }

    fn int(n: i64) -> Par {
        Par {
            exprs: vec![rchain_models::ast::Expr::GInt(n)],
            ..Default::default()
        }
    }

    /// The environment is built positionally from the matched data, and `make_env` folds with
    /// `put`, which **prepends**: the *last* datum of the receive is index 0 and the first is the
    /// highest index — the De Bruijn convention the normalizer numbers a receive's variables with.
    /// Asserted explicitly because a change to the fold order is a change of *bindings*, not a
    /// cosmetic one (the execution corpus would catch it end-to-end, this catches it in one place).
    #[test]
    fn build_env_binds_the_data_by_de_bruijn_index() {
        let env = build_env(&[data(vec![int(10), int(11)]), data(vec![int(12)])]);
        let value = |i: i32| env.get(i).and_then(|p| p.exprs.first().cloned());
        assert_eq!(value(0), Some(rchain_models::ast::Expr::GInt(12)));
        assert_eq!(value(1), Some(rchain_models::ast::Expr::GInt(11)));
        assert_eq!(value(2), Some(rchain_models::ast::Expr::GInt(10)));
        assert_eq!(value(3), None, "nothing beyond the data");
    }

    #[test]
    fn build_env_of_no_data_is_empty() {
        let env = build_env(&[]);
        assert_eq!(env.get(0), None);
    }

    /// The `ParBody` path needs its evaluator: the reducer↔dispatcher cycle is broken deliberately
    /// (the eval closure is set after construction), so a dispatch that arrives before it is wired
    /// must **say so** rather than silently no-op and drop a continuation.
    #[tokio::test]
    async fn a_par_body_without_an_evaluator_is_a_bug_not_a_silent_no_op() {
        let dispatcher = RholangAndScalaDispatcher::new(BTreeMap::new());
        let err = dispatcher
            .dispatch(
                TaggedContinuation::ParBody(ParWithRandom {
                    body: SortedProc::new(int(1)),
                    random_state: Blake2b512Random::new_random(128),
                }),
                // One datum, so the random merge has the two inputs it asserts on (see the
                // zero-data test below, which pins that assertion).
                vec![data(vec![int(1)])],
                DfsPath::root(),
            )
            .await
            .expect_err("an unset evaluator must be reported");
        assert!(
            matches!(err, RholangError::BugFoundError(_)),
            "expected a bug report, got {err:?}"
        );
    }

    /// **Recorded, not endorsed** (AUDIT.md §15 C5): a `ParBody` continuation dispatched with **no
    /// matched data** panics inside `Blake2b512Random::merge`, which asserts at least two inputs —
    /// the dispatcher always prepends the continuation's own random, so zero data means one input.
    /// The reducer does not currently dispatch a `ParBody` with zero data (a receive always matches
    /// at least the datum that triggered it, and a match with nothing to run becomes
    /// `TaggedContinuation::Empty`), so this is a latent panic on an unreachable path. Pinned so that
    /// if the reachability ever changes — or someone adds the guard — the change is deliberate
    /// rather than discovered by a node crashing.
    #[tokio::test]
    #[should_panic(expected = "at least 2 inputs")]
    async fn a_par_body_with_no_matched_data_panics_in_merge() {
        let dispatcher = RholangAndScalaDispatcher::new(BTreeMap::new());
        let _ = dispatcher
            .dispatch(
                TaggedContinuation::ParBody(ParWithRandom {
                    body: SortedProc::new(int(1)),
                    random_state: Blake2b512Random::new_random(128),
                }),
                vec![],
                DfsPath::root(),
            )
            .await;
    }

    /// A `ScalaBodyRef` naming a body that is not installed is an error naming the ref — a system
    /// process reached for but never registered must not fail silently.
    #[tokio::test]
    async fn an_unregistered_body_ref_names_the_ref() {
        let dispatcher = RholangAndScalaDispatcher::new(BTreeMap::new());
        let err = dispatcher
            .dispatch(
                TaggedContinuation::ScalaBodyRef(42),
                vec![],
                DfsPath::root(),
            )
            .await
            .expect_err("an unregistered body ref must be reported");
        let rendered = format!("{err:?}");
        assert!(rendered.contains("42"), "{rendered}");
        assert!(rendered.contains("no function"), "{rendered}");
    }

    /// An `Empty` continuation is the deliberate no-op (a match that produced nothing to run), so it
    /// must succeed rather than be reported as a missing handler.
    #[tokio::test]
    async fn an_empty_continuation_is_a_no_op() {
        let dispatcher = RholangAndScalaDispatcher::new(BTreeMap::new());
        assert!(dispatcher
            .dispatch(TaggedContinuation::Empty, vec![], DfsPath::root())
            .await
            .is_ok());
    }

    /// A registered handler is invoked, and the `ParBody` evaluator receives the environment built
    /// from the data and the random state merged from the continuation **and** the data.
    #[tokio::test]
    async fn a_registered_handler_and_a_set_evaluator_are_both_reached() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let calls = Arc::new(AtomicUsize::new(0));
        let handler_calls = calls.clone();
        let mut table: BTreeMap<i64, ScalaBodyFn> = BTreeMap::new();
        table.insert(
            7,
            Box::new(move |_data, _path| {
                let calls = handler_calls.clone();
                Box::pin(async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })
            }),
        );
        let dispatcher = RholangAndScalaDispatcher::new(table);

        dispatcher
            .dispatch(TaggedContinuation::ScalaBodyRef(7), vec![], DfsPath::root())
            .await
            .expect("the handler is registered");
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // The evaluator, once set, is reached with the built environment.
        let seen = Arc::new(Mutex::new(None));
        let seen_env = seen.clone();
        dispatcher.set_eval(Box::new(move |_body, env, _rand, _path| {
            let seen = seen_env.clone();
            Box::pin(async move {
                *seen.lock().unwrap() = Some(env.get(0).is_some());
                Ok(())
            })
        }));
        dispatcher
            .dispatch(
                TaggedContinuation::ParBody(ParWithRandom {
                    body: SortedProc::new(int(1)),
                    random_state: Blake2b512Random::new_random(128),
                }),
                vec![data(vec![int(9)])],
                DfsPath::root(),
            )
            .await
            .expect("the evaluator is set");
        assert_eq!(
            *seen.lock().unwrap(),
            Some(true),
            "the env passed to the evaluator carries the matched data"
        );
    }

    /// The weak dispatcher exists so a system process does not keep its runtime alive through the
    /// dispatch table (issues #18/#23). Once the dispatcher is dropped, dispatching through the weak
    /// handle is a bug report — not a panic and not a silent success.
    #[tokio::test]
    async fn a_dropped_dispatcher_is_reported_through_the_weak_handle() {
        let dispatcher = Arc::new(RholangAndScalaDispatcher::new(BTreeMap::new()));
        let weak: Weak<RholangAndScalaDispatcher> = Arc::downgrade(&dispatcher);
        drop(dispatcher);

        let err = weak
            .dispatch(TaggedContinuation::Empty, vec![], DfsPath::root())
            .await
            .expect_err("a dropped dispatcher must be reported");
        assert!(
            matches!(err, RholangError::BugFoundError(_)),
            "expected a bug report, got {err:?}"
        );
    }
}
