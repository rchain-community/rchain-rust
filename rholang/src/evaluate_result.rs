//! The result of evaluating a deploy (port of `interpreter/EvaluateResult`).

use std::collections::BTreeSet;

use rchain_models::ast::Par;

use crate::accounting::Cost;
use crate::errors::RholangError;

/// The result of reducing a term: the gas consumed, any interpreter errors, and the mergeable
/// (number) channels produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvaluateResult {
    pub cost: Cost,
    pub errors: Vec<RholangError>,
    pub mergeable: BTreeSet<Par>,
}

impl EvaluateResult {
    pub fn failed(&self) -> bool {
        !self.errors.is_empty()
    }

    pub fn succeeded(&self) -> bool {
        self.errors.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::RholangError;

    /// `succeeded`/`failed` are complements of each other and both derive from the **errors** list —
    /// gas is not an error, so a result that consumed its whole phlo limit and raised nothing is a
    /// success, and a result that errored with zero gas is a failure. A caller that read the cost
    /// instead would report a clean but expensive deploy as failed.
    #[test]
    fn success_and_failure_are_the_error_list_not_the_cost() {
        let clean = EvaluateResult {
            cost: Cost::new(0, "noop"),
            errors: Vec::new(),
            mergeable: BTreeSet::new(),
        };
        assert!(clean.succeeded());
        assert!(!clean.failed());

        let failed = EvaluateResult {
            cost: Cost::new(0, "noop"),
            errors: vec![RholangError::ReduceError("boom".to_string())],
            mergeable: BTreeSet::new(),
        };
        assert!(failed.failed());
        assert!(!failed.succeeded(), "the two predicates never disagree");

        let expensive = EvaluateResult {
            cost: Cost::new(1234, "deploy"),
            errors: Vec::new(),
            mergeable: BTreeSet::new(),
        };
        assert!(expensive.succeeded(), "spending gas is not an error");

        // Every error carries the message it was built with, so a multi-error result is reportable.
        let two = EvaluateResult {
            cost: Cost::new(1, "deploy"),
            errors: vec![
                RholangError::ReduceError("first".to_string()),
                RholangError::ReduceError("second".to_string()),
            ],
            mergeable: BTreeSet::new(),
        };
        assert_eq!(two.errors.len(), 2);
        assert!(two.failed());
    }
}
