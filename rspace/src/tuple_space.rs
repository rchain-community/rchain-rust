//! The tuple-space API + result types.
//!
//! Mirrors `rspace/src/main/scala/coop/rchain/rspace/Tuplespace.scala` and the `Result`/`ContResult`
//! types from `ISpace.scala`.

use std::collections::BTreeSet;

use async_trait::async_trait;

use crate::errors::RSpaceError;
use crate::scheduled_space::{PendingProduce, ReleaseToken, ScheduledConsume, ScheduledProduce};

/// A matched datum result (port of `Result`).
#[derive(Clone, Debug, PartialEq)]
pub struct Result<C, A> {
    pub channel: C,
    pub matched_datum: A,
    pub removed_datum: A,
    pub persistent: bool,
}

/// A matched continuation result (port of `ContResult`).
#[derive(Clone, Debug, PartialEq)]
pub struct ContResult<C, P, K> {
    pub continuation: K,
    pub persistent: bool,
    pub channels: Vec<C>,
    pub patterns: Vec<P>,
    pub peek: bool,
}

/// The tuple-space interface (port of `Tuplespace[F]`).
///
/// The `Send + Sync + 'static` bounds on the type parameters carry the scheduled variants' async
/// futures: the generated trait objects are `Send + 'async_trait` futures, `consume_at` borrows
/// `&[C]`/`&[P]` across awaits (so they must be `Sync`), and `PendingProduce`/`ScheduledProduce`
/// capture `C`/`A` values across awaits (every concrete implementor already bounds its generics
/// exactly like this).
#[async_trait]
pub trait Tuplespace<
    C: Send + Sync + 'static,
    P: Send + Sync + 'static,
    A: Send + Sync + 'static,
    K: Send + Sync + 'static,
>: Send + Sync
{
    async fn consume(
        &self,
        channels: &[C],
        patterns: &[P],
        continuation: K,
        persist: bool,
        peeks: BTreeSet<usize>,
    ) -> std::result::Result<Option<(ContResult<C, P, K>, Vec<Result<C, A>>)>, RSpaceError>;

    async fn produce(
        &self,
        channel: C,
        data: A,
        persist: bool,
    ) -> std::result::Result<Option<(ContResult<C, P, K>, Vec<Result<C, A>>)>, RSpaceError>;

    async fn install(
        &self,
        channels: &[C],
        patterns: &[P],
        continuation: K,
    ) -> std::result::Result<Option<(K, Vec<A>)>, RSpaceError>;

    /// Produce *at* the given DFS path (Laws 20–22 scheduling entry point). The default is the
    /// plain produce plus `ReleaseToken::detached()` — non-scheduling spaces "don't schedule":
    /// their ops complete inline and no phase-two claim set is returned.
    async fn produce_at(
        &self,
        _path: Vec<u16>,
        channel: C,
        data: A,
        persist: bool,
    ) -> std::result::Result<ScheduledProduce<C, P, A, K>, RSpaceError> {
        let result = self.produce(channel, data, persist).await?;
        Ok(ScheduledProduce {
            joins: vec![],
            result: Ok(result),
            phase_two: None,
            release: ReleaseToken::detached(),
        })
    }

    /// Consume *at* the given DFS path. Default: the plain consume (no split is ever needed — the
    /// full static source set is claimed before this is called).
    async fn consume_at(
        &self,
        _path: Vec<u16>,
        channels: &[C],
        patterns: &[P],
        continuation: K,
        persist: bool,
        peeks: BTreeSet<usize>,
    ) -> std::result::Result<ScheduledConsume<C, P, A, K>, RSpaceError> {
        let result = self
            .consume(channels, patterns, continuation, persist, peeks)
            .await?;
        Ok(ScheduledConsume {
            result: Ok(result),
            release: ReleaseToken::detached(),
        })
    }

    /// Commit the phase two of a scheduled produce (only the scheduling `RSpace` ever defers one;
    /// the default is therefore unreachable).
    async fn commit_produce(
        &self,
        _pending: PendingProduce<C, A>,
    ) -> std::result::Result<Option<(ContResult<C, P, K>, Vec<Result<C, A>>)>, RSpaceError> {
        Ok(None)
    }
}
