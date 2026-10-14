//! Admin web API interface (port of `api/AdminWebApi.scala`).

use async_trait::async_trait;

use super::dto::{BlockApiException, OperatorStatus};

/// The admin web API contract (port of `AdminWebApi[F]`).
#[async_trait]
pub trait AdminWebApi: Send + Sync {
    async fn propose(&self) -> Result<String, BlockApiException>;

    async fn propose_result(&self) -> Result<String, BlockApiException>;

    /// **The operator half of the status**, on this listener rather than the public one (W-N3-01).
    ///
    /// It is a method here and not on `WebApi` on purpose: the public API type no longer *has* the
    /// data, so a later change cannot mount this route on the public router by reaching for a field —
    /// it would have to move a capability, which is the reviewable act.
    async fn operator_status(&self) -> Result<OperatorStatus, BlockApiException>;
}
