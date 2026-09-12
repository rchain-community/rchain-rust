//! The replay-space interface.
//!
//! Mirrors `rspace/src/main/scala/coop/rchain/rspace/IReplaySpace.scala`.

use async_trait::async_trait;
use rchain_crypto::hash::blake2b256_hash::Blake2b256Hash;

use crate::i_space::ISpace;
use crate::trace::Log;
use crate::util::ReplayException;

/// The replay-space interface (port of `IReplaySpace[F]`). The bounds mirror
/// [`ISpace`](crate::i_space::ISpace)'s — every implementor already carries them.
#[async_trait]
pub trait IReplaySpace<
    C: Send + Sync + 'static,
    P: Send + Sync + 'static,
    A: Send + Sync + 'static,
    K: Send + Sync + 'static,
>: ISpace<C, P, A, K>
{
    async fn rig(&self, log: Log);

    async fn rig_and_reset(&self, start_root: Blake2b256Hash, log: Log) -> Result<(), String>;

    async fn check_replay_data(&self) -> Result<(), ReplayException>;
}
