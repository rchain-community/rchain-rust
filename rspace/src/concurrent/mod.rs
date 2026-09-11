//! Concurrency primitives (ordered multi-key locks, the Law-20 channel claim queue).
//!
//! Mirrors `rspace/src/main/scala/coop/rchain/rspace/concurrent/`.

use std::future::Future;
use std::pin::Pin;

pub mod channel_queue;
pub mod multi_lock;
pub mod two_step_lock;

/// A boxed `Send` future.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
