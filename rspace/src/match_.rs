//! Pattern-matching typeclass.
//!
//! Mirrors `rspace/src/main/scala/coop/rchain/rspace/Match.scala`.

use crate::errors::RSpaceError;

/// Typeclass for matching a pattern against a datum (port of `Match[F, P, A]`).
///
/// `F[Option[A]]` in the oracle (`Match.scala:11`), so the result distinguishes "no match"
/// (`Ok(None)`) from "the matcher could not decide" (`Err`) — the port's `F` is `Result`. Returning
/// a bare `Option` made the two the same answer, which is how a pattern whose declared `free_count`
/// outran what the matcher bound came to be *padded* with empty pars rather than refused (AUDIT C52).
pub trait Match<P, A>: Send + Sync {
    fn get(&self, p: &P, a: &A) -> Result<Option<A>, RSpaceError>;
}
