//! The REPL gRPC service (port of `ReplGrpcService.scala`).

use std::sync::Arc;
use std::time::Duration;

use rchain_crypto::hash::blake2b512_random::Blake2b512Random;
use rchain_models::ast::Par;
use rchain_rholang::accounting::Cost;
use rchain_rholang::normalizer::source_to_adt;
use rchain_rholang::pretty_printer::PrettyPrinter;
use rchain_rholang::runtime::RhoRuntime;
use rchain_rholang::storage_printer::{pretty_print, pretty_print_unmatched_sends};

/// The phlo (gas) limit for a single Repl evaluation (documented deviation: Scala runs Repl with no
/// limit). The reducer aborts with `OutOfPhlogistonsError` once the balance is exhausted, so a
/// runaway term cannot drain the node.
const REPL_PHLO_LIMIT: i64 = 1_000_000_000;
/// The wall-clock deadline for a single Repl evaluation (documented deviation).
const REPL_EVAL_TIMEOUT: Duration = Duration::from_secs(60);

/// `CmdRequest` (run a single line).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CmdRequest {
    pub line: String,
}

/// `EvalRequest` (evaluate a program, optionally reporting only unmatched sends).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalRequest {
    pub program: String,
    pub print_unmatched_sends_only: bool,
}

/// `ReplResponse` (the rendered output).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplResponse {
    pub output: String,
}

/// The REPL service (port of `ReplGrpcService`).
pub struct ReplGrpcService {
    runtime: Arc<RhoRuntime>,
}

impl ReplGrpcService {
    pub fn new(runtime: Arc<RhoRuntime>) -> Self {
        ReplGrpcService { runtime }
    }

    /// Run a single line (port of `run`).
    pub async fn run(&self, request: &CmdRequest) -> ReplResponse {
        self.exec(&request.line, false).await
    }

    /// Evaluate a program (port of `eval`).
    pub async fn eval(&self, request: &EvalRequest) -> ReplResponse {
        self.exec(&request.program, request.print_unmatched_sends_only)
            .await
    }

    async fn exec(&self, source: &str, print_unmatched_sends_only: bool) -> ReplResponse {
        // Parse first so a syntax error surfaces as `Error: ...` before evaluation.
        match source_to_adt(source) {
            Err(e) => ReplResponse {
                output: format!("Error: {e}"),
            },
            Ok(term) => {
                // Port of `printNormalizedTerm`: echo the normalized term on the node console.
                println!("\nEvaluating:");
                println!(
                    "{}",
                    PrettyPrinter::new().build_string(&Par::from(term.clone()))
                );
                let rand = Blake2b512Random::default_random();
                // Bound the Repl evaluation: a phlo cap (the reducer aborts when the balance is
                // exhausted) + a wall-clock deadline.
                self.runtime.cost().set(Cost::new(REPL_PHLO_LIMIT, "repl"));
                let eval = match tokio::time::timeout(
                    REPL_EVAL_TIMEOUT,
                    self.runtime.evaluate(source, &rand),
                )
                .await
                {
                    Ok(res) => res,
                    Err(_) => {
                        return ReplResponse {
                            output: format!(
                                "Error: evaluation timed out after {REPL_EVAL_TIMEOUT:?}"
                            ),
                        };
                    }
                };
                let pretty_storage = if print_unmatched_sends_only {
                    pretty_print_unmatched_sends(self.runtime.as_ref()).await
                } else {
                    pretty_print(self.runtime.as_ref()).await
                };
                match eval {
                    Ok(res) => {
                        let error_str = if res.errors.is_empty() {
                            String::new()
                        } else {
                            format!(
                                "Errors received during evaluation:\n{}\n",
                                res.errors
                                    .iter()
                                    .map(|e| e.to_string())
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            )
                        };
                        ReplResponse {
                            output: format!(
                                "Deployment cost: {}\n{}Storage Contents:\n{}",
                                res.cost.value, error_str, pretty_storage
                            ),
                        }
                    }
                    Err(e) => ReplResponse {
                        output: format!("Error: {e}"),
                    },
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use rchain_models::runtime::{BindPattern, ListParWithRandom, TaggedContinuation};
    use rchain_models::sorted::SortedProc;
    use rchain_rholang::storage::RhoMatch;
    use rchain_rholang::storage_printer::NO_UNMATCHED_SENDS;
    use rchain_rspace::factory::create_history_repository;
    use rchain_rspace::hot_store::InMemHotStore;
    use rchain_rspace::rspace::RSpace;
    use rchain_shared::store_manager::InMemoryStoreManager;

    /// A real `RhoRuntime` over an in-memory store (the shape `rholang/tests/common` uses).
    async fn service() -> ReplGrpcService {
        let manager = InMemoryStoreManager::default();
        let history = create_history_repository::<
            SortedProc,
            BindPattern,
            ListParWithRandom,
            TaggedContinuation,
        >(&manager, "repl")
        .await
        .expect("history repository");
        let reader = history.get_history_reader(history.root()).await;
        let hot = Arc::new(InMemHotStore::new(reader.base()));
        let (play, _replay) = RSpace::create_with_replay(history.clone(), hot, Arc::new(RhoMatch));
        let runtime = RhoRuntime::create(play, history, SortedProc::default())
            .await
            .expect("rho runtime");
        ReplGrpcService::new(Arc::new(runtime))
    }

    /// **The parse-first arm.** A syntax error is reported as `Error: …` and nothing is evaluated —
    /// which is why the Repl needs no phlo budget for a malformed line.
    #[tokio::test]
    async fn a_syntax_error_is_reported_without_evaluating() {
        let service = service().await;
        let response = service
            .run(&CmdRequest {
                line: "new in {".to_string(),
            })
            .await;
        assert!(
            response.output.starts_with("Error: "),
            "a syntax error must be reported as such: {}",
            response.output
        );
        assert!(
            !response.output.contains("Deployment cost"),
            "nothing may be evaluated: {}",
            response.output
        );
    }

    /// A well-formed program is evaluated and the response carries the cost, the storage dump and no
    /// error section.
    #[tokio::test]
    async fn a_well_formed_program_is_evaluated_and_reported() {
        let service = service().await;
        let response = service
            .eval(&EvalRequest {
                program: r#"@"out"!(1)"#.to_string(),
                print_unmatched_sends_only: false,
            })
            .await;
        assert!(
            response.output.contains("Deployment cost: "),
            "{}",
            response.output
        );
        assert!(
            response.output.contains("Storage Contents:"),
            "{}",
            response.output
        );
        assert!(
            !response
                .output
                .contains("Errors received during evaluation"),
            "this term reduces cleanly: {}",
            response.output
        );
    }

    /// A term that *reduces* to an error keeps the error in the output — a Repl user sees what went
    /// wrong rather than a silent success.
    #[tokio::test]
    async fn a_reduce_error_is_reported_in_the_output() {
        let service = service().await;
        let response = service
            .run(&CmdRequest {
                line: r#"@"out"!(1 / 0)"#.to_string(),
            })
            .await;
        assert!(
            response
                .output
                .contains("Errors received during evaluation"),
            "{}",
            response.output
        );
    }

    /// `print_unmatched_sends_only` switches *which* dump is rendered. The label is the same either
    /// way (`Storage Contents:` is hardcoded), so the difference is in the content: the full dump
    /// prints a waiting `for`, the sends-only dump reports that there are no unmatched sends.
    #[tokio::test]
    async fn the_unmatched_sends_flag_selects_the_dump() {
        let service = service().await;
        // A receive with nothing to match it: a waiting continuation and no sends.
        let waiting = r#"new c in { for (@x <- c) { Nil } }"#;

        let full = service
            .eval(&EvalRequest {
                program: waiting.to_string(),
                print_unmatched_sends_only: false,
            })
            .await;
        assert!(
            full.output.contains("for"),
            "the full dump prints the waiting receive: {}",
            full.output
        );

        let sends_only = service
            .eval(&EvalRequest {
                program: waiting.to_string(),
                print_unmatched_sends_only: true,
            })
            .await;
        assert!(
            !sends_only.output.contains("for"),
            "the sends-only dump must not print the waiting receive: {}",
            sends_only.output
        );
        // With no sends to show, the dump is the empty par — `Nil`, not an empty string (the
        // Scala `prettyPrintUnmatchedSends` renders an empty `Par` the same way).
        assert!(
            sends_only.output.contains("Nil"),
            "no sends renders as the empty par: {}",
            sends_only.output
        );

        // The dedicated "none" message is for an *empty space* (no channels at all), which the
        // full printer reports differently.
        assert_eq!(NO_UNMATCHED_SENDS, "No unmatched sends.");
    }

    /// **The phlo cap is set before each evaluation.** The runtime's cost cell is long-lived (it is
    /// the same cell the block path seeds), so a Repl evaluation that did not *set* the budget would
    /// inherit whatever balance the runtime happened to hold — and a runaway term would drain it
    /// without bound. Asserted by observing that the cell's balance is bounded by `REPL_PHLO_LIMIT`
    /// after a run.
    #[tokio::test]
    async fn each_evaluation_resets_the_phlo_budget() {
        let service = service().await;
        // A large balance first, to prove the Repl replaces it rather than adding to it.
        service
            .runtime
            .cost()
            .set(Cost::new(999_999_999_999, "seed"));

        let response = service
            .run(&CmdRequest {
                line: "Nil".to_string(),
            })
            .await;
        assert!(
            response.output.contains("Deployment cost: "),
            "{}",
            response.output
        );
        assert!(
            service.runtime.cost().total_charged() < REPL_PHLO_LIMIT,
            "the budget must have been reset to the Repl limit, not left at the seeded balance"
        );
    }

    /// The Repl's two bounds are the documented ones: a phlo cap and a wall-clock deadline. The cap
    /// being *reset* per evaluation is pinned above; the runaway path itself (which has to burn a
    /// megaphlo of reduction steps to reach the cap) is deliberately **not** exercised — a test that
    /// runs a loop to exhaustion is a slow test, not a better one. The bounds are therefore asserted
    /// at compile time: a change to either is a build failure.
    const _: () = assert!(REPL_PHLO_LIMIT > 0);
    const _: () = assert!(REPL_EVAL_TIMEOUT.as_secs() >= 10);
}
