//! System-contract message unapplying (port of `ContractCall.scala`).

use rchain_crypto::hash::blake2b512_random::Blake2b512Random;
use rchain_models::ast::Par;
use rchain_models::runtime::ListParWithRandom;
use rchain_models::sorted::SortedProc;

use crate::errors::RholangError;
use crate::reduce::{Dispatch, Tuplespace};
use crate::scheduler::DfsPath;

/// Unapplies a message sent to a system contract, returning the producer and the message content
/// (port of `ContractCall`). The producer is separated into [`ContractCall::produce`] plus the
/// recovered random state, so callers can send a reply without capturing the store.
#[derive(Clone)]
pub struct ContractCall<T: Tuplespace, D: Dispatch> {
    space: T,
    dispatcher: D,
}

impl<T: Tuplespace, D: Dispatch> ContractCall<T, D> {
    pub fn new(space: T, dispatcher: D) -> Self {
        ContractCall { space, dispatcher }
    }

    /// Send `values` through `ch`, dispatching any matched continuation (port of `produce`).
    /// `path` addresses the handler invocation this reply is a child of; the dispatched
    /// continuation is addressed at `path.child(0)` (the same convention as `apply_effect`).
    pub async fn produce(
        &self,
        rand: &Blake2b512Random,
        values: &[Par],
        ch: &Par,
        path: DfsPath,
    ) -> Result<(), RholangError> {
        let result = self
            .space
            .produce(
                &SortedProc::new(ch.clone()),
                ListParWithRandom {
                    pars: values.iter().map(|p| SortedProc::new(p.clone())).collect(),
                    random_state: rand.clone(),
                },
                false,
            )
            .await?;
        if let Some((continuation, data_list, _)) = result {
            let data: Vec<ListParWithRandom> = data_list
                .iter()
                .map(|(_, matched, _, _)| matched.clone())
                .collect();
            self.dispatcher
                .dispatch(continuation, data, path.child(0))
                .await?;
        }
        Ok(())
    }

    /// Extract the message content and its random state if there is exactly one argument (port of
    /// `unapply`).
    pub fn unapply(
        &self,
        contract_args: &[ListParWithRandom],
    ) -> Option<(Vec<Par>, Blake2b512Random)> {
        if let [single] = contract_args {
            Some((
                single.pars.iter().map(|p| p.as_par().clone()).collect(),
                single.random_state.clone(),
            ))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::sync::{Arc, Mutex};

    use rchain_models::ast::Expr;
    use rchain_models::par_ops::from_expr;
    use rchain_models::runtime::{BindPattern, TaggedContinuation};

    use crate::reduce::Application;

    /// A space that records what was produced and returns a canned `Application`, so the two arms
    /// (matched / not matched) are chosen by the test rather than by a real store.
    #[derive(Default)]
    struct RecordingSpace {
        produced: Mutex<Vec<(SortedProc, ListParWithRandom, bool)>>,
        /// `None` = nothing waiting; `Some` = a match to return.
        reply: Application,
        /// When set, `produce` fails with this message.
        fail: Option<String>,
    }

    #[async_trait::async_trait]
    impl Tuplespace for RecordingSpace {
        async fn produce(
            &self,
            channel: &SortedProc,
            data: ListParWithRandom,
            persist: bool,
        ) -> Result<Application, RholangError> {
            self.produced
                .lock()
                .unwrap()
                .push((channel.clone(), data, persist));
            match &self.fail {
                Some(msg) => Err(RholangError::ReduceError(msg.clone())),
                None => Ok(self.reply.clone()),
            }
        }
        async fn consume(
            &self,
            _channels: &[SortedProc],
            _patterns: &[BindPattern],
            _continuation: TaggedContinuation,
            _persist: bool,
            _peeks: BTreeSet<usize>,
        ) -> Result<Application, RholangError> {
            unreachable!("a system-contract reply never consumes")
        }
    }

    /// `ContractCall` takes its space and dispatcher by value, so the test needs to read them back
    /// after the call — hence the `Arc` forwarding impls (the same shape `dispatch.rs` uses for
    /// `Arc<RholangAndScalaDispatcher>`).
    #[async_trait::async_trait]
    impl Tuplespace for Arc<RecordingSpace> {
        async fn produce(
            &self,
            channel: &SortedProc,
            data: ListParWithRandom,
            persist: bool,
        ) -> Result<Application, RholangError> {
            self.as_ref().produce(channel, data, persist).await
        }
        async fn consume(
            &self,
            channels: &[SortedProc],
            patterns: &[BindPattern],
            continuation: TaggedContinuation,
            persist: bool,
            peeks: BTreeSet<usize>,
        ) -> Result<Application, RholangError> {
            self.as_ref()
                .consume(channels, patterns, continuation, persist, peeks)
                .await
        }
    }

    /// Records the dispatch's three arguments.
    #[derive(Default)]
    struct RecordingDispatch {
        dispatched: Mutex<Vec<(TaggedContinuation, Vec<ListParWithRandom>, DfsPath)>>,
    }

    #[async_trait::async_trait]
    impl Dispatch for RecordingDispatch {
        async fn dispatch(
            &self,
            continuation: TaggedContinuation,
            data_list: Vec<ListParWithRandom>,
            path: DfsPath,
        ) -> Result<(), RholangError> {
            self.dispatched
                .lock()
                .unwrap()
                .push((continuation, data_list, path));
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl Dispatch for Arc<RecordingDispatch> {
        async fn dispatch(
            &self,
            continuation: TaggedContinuation,
            data_list: Vec<ListParWithRandom>,
            path: DfsPath,
        ) -> Result<(), RholangError> {
            self.as_ref().dispatch(continuation, data_list, path).await
        }
    }

    fn lpw(values: &[i64]) -> ListParWithRandom {
        ListParWithRandom {
            pars: values
                .iter()
                .map(|v| SortedProc::new(from_expr(Expr::GInt(*v))))
                .collect(),
            random_state: Blake2b512Random::from_init(&[3u8; 32]),
        }
    }

    fn pars(values: &[i64]) -> Vec<SortedProc> {
        values
            .iter()
            .map(|v| SortedProc::new(from_expr(Expr::GInt(*v))))
            .collect()
    }

    fn ch(name: &str) -> Par {
        from_expr(Expr::GString(name.to_string()))
    }

    /// `unapply` is the *exactly one argument* pattern (Scala's `case List(single) =>`): a system
    /// contract's reply takes one message, so anything else must not be unwrapped — the second
    /// element of a two-argument reply would otherwise be silently dropped.
    #[test]
    fn unapply_requires_exactly_one_argument() {
        let call = ContractCall::new(
            Arc::new(RecordingSpace::default()),
            Arc::new(RecordingDispatch::default()),
        );

        let (values, rand) = call.unapply(&[lpw(&[7])]).expect("one argument unwraps");
        assert_eq!(values, vec![from_expr(Expr::GInt(7))]);
        assert_eq!(
            rand,
            Blake2b512Random::from_init(&[3u8; 32]),
            "the argument's random state comes back with it"
        );

        assert!(call.unapply(&[]).is_none(), "no argument does not unwrap");
        assert!(
            call.unapply(&[lpw(&[1]), lpw(&[2])]).is_none(),
            "two arguments do not unwrap"
        );
    }

    /// A reply with nothing waiting produces and dispatches nothing — the common case for a
    /// fire-and-forget system process.
    #[tokio::test]
    async fn a_reply_with_no_match_dispatches_nothing() {
        let space = Arc::new(RecordingSpace::default());
        let dispatch = Arc::new(RecordingDispatch::default());
        let call = ContractCall::new(space.clone(), dispatch.clone());

        call.produce(
            &Blake2b512Random::from_init(&[1u8; 32]),
            &[from_expr(Expr::GInt(9))],
            &ch("out"),
            DfsPath::root(),
        )
        .await
        .expect("produce");

        let produced = space.produced.lock().unwrap();
        assert_eq!(produced.len(), 1);
        assert_eq!(produced[0].0, SortedProc::new(ch("out")));
        assert_eq!(produced[0].1.pars, pars(&[9]));
        assert!(!produced[0].2, "a reply is not persistent");
        assert!(
            dispatch.dispatched.lock().unwrap().is_empty(),
            "no match, no continuation"
        );
    }

    /// A matched reply dispatches at **`path.child(0)`** — the same convention `apply_effect` uses,
    /// so the continuation's own path is a child of the handler invocation that produced it. The
    /// path is what the scheduler keys its per-path bookkeeping on, so getting it wrong is a
    /// scheduling bug rather than a cosmetic one.
    #[tokio::test]
    async fn a_matched_reply_dispatches_at_the_child_path() {
        let space = Arc::new(RecordingSpace {
            produced: Mutex::new(Vec::new()),
            // The 4-tuple is `(channel, matched data, removed data, persistent)`; the matched and
            // removed values are deliberately *different* here (the peek shape), so the assertion
            // below distinguishes the field the continuation receives from the one that left the
            // space.
            reply: Some((
                TaggedContinuation::Empty,
                vec![(SortedProc::new(ch("out")), lpw(&[4]), lpw(&[99]), false)],
                false,
            )),
            fail: None,
        });
        let dispatch = Arc::new(RecordingDispatch::default());
        let call = ContractCall::new(space, dispatch.clone());

        call.produce(
            &Blake2b512Random::from_init(&[1u8; 32]),
            &[from_expr(Expr::GInt(9))],
            &ch("out"),
            DfsPath(vec![2, 5]),
        )
        .await
        .expect("produce");

        let dispatched = dispatch.dispatched.lock().unwrap();
        assert_eq!(dispatched.len(), 1);
        assert_eq!(
            dispatched[0].2,
            DfsPath(vec![2, 5, 0]),
            "the continuation is addressed at the *child* of the invoking path"
        );
        assert_eq!(
            dispatched[0].1.len(),
            1,
            "exactly the matched datum is dispatched"
        );
        assert_eq!(
            dispatched[0].1[0].pars,
            pars(&[4]),
            "the *matched* datum is what the continuation receives, not the removed one"
        );
    }

    /// A space error is propagated, not swallowed: a system contract whose reply could not be
    /// produced must fail the deploy rather than report success.
    #[tokio::test]
    async fn a_produce_error_is_propagated() {
        let call = ContractCall::new(
            Arc::new(RecordingSpace {
                fail: Some("store is down".to_string()),
                ..RecordingSpace::default()
            }),
            Arc::new(RecordingDispatch::default()),
        );

        let err = call
            .produce(
                &Blake2b512Random::from_init(&[1u8; 32]),
                &[],
                &ch("out"),
                DfsPath::root(),
            )
            .await
            .expect_err("the space's error must escape");
        assert!(format!("{err}").contains("store is down"), "{err}");
    }
}
