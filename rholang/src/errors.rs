//! Interpreter error types. Mirrors `rholang/src/main/scala/coop/rchain/rholang/interpreter/errors.scala`.

use std::fmt;

use rchain_models::ast::{Par, Var};
use rchain_models::sorted::SortedProc;

/// A source position (port of `compiler.SourcePosition`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourcePosition {
    pub row: i32,
    pub column: i32,
}

impl fmt::Display for SourcePosition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.row, self.column)
    }
}

/// The rholang interpreter error ADT (mirrors the Scala `InterpreterError` hierarchy).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RholangError {
    BugFoundError(String),
    NormalizerError(String),
    SyntaxError(String),
    LexerError(String),
    ParserError(String),
    UnboundVariableRef {
        var_name: String,
        line: i32,
        col: i32,
    },
    UnexpectedNameContext {
        var_name: String,
        proc_var_source_position: SourcePosition,
        name_source_position: SourcePosition,
    },
    UnexpectedReuseOfNameContextFree {
        var_name: String,
        first_use: SourcePosition,
        second_use: SourcePosition,
    },
    UnexpectedProcContext {
        var_name: String,
        name_var_source_position: SourcePosition,
        process_source_position: SourcePosition,
    },
    UnexpectedReuseOfProcContextFree {
        var_name: String,
        first_use: SourcePosition,
        second_use: SourcePosition,
    },
    UnexpectedBundleContent(String),
    UnrecognizedNormalizerError(String),
    OutOfPhlogistonsError,
    TopLevelWildcardsNotAllowedError(Box<Par>),
    TopLevelFreeVariablesNotAllowedError(Box<Par>),
    TopLevelLogicalConnectivesNotAllowedError(Box<Par>),
    SubstituteError {
        term: Var,
    },
    PatternReceiveError(String),
    SetupError(String),
    UnrecognizedInterpreterError(String),
    SortMatchError(String),
    ReduceError(String),
    /// Laws 23–25: a relaxed-validated commit failed Law 24 prefix visibility — the claimed
    /// channel's newest committed write was made by a DFS-later (or equal) path, so the
    /// speculative run read state no DFS-earlier effect produced. Permanent for the run: the
    /// block path must fall back to the sequential reference.
    SpeculationInvalid {
        /// The channel whose newest write invalidates the claim.
        channel: SortedProc,
        /// The invalidating writer's path.
        writer_path: Vec<u16>,
        /// The invalidated commit's own path.
        at_path: Vec<u16>,
    },
    MethodNotDefined {
        method: String,
        other_type: String,
    },
    MethodArgumentNumberMismatch {
        method: String,
        expected: i32,
        actual: i32,
    },
    OperatorNotDefined {
        op: String,
        other_type: String,
    },
    OperatorExpectedError {
        op: String,
        expected: String,
        other_type: String,
    },
    AggregateError {
        interpreter_errors: Vec<RholangError>,
        errors: Vec<String>,
    },
    ReceiveOnSameChannelsError {
        line: i32,
        col: i32,
    },
}

impl fmt::Display for RholangError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RholangError::BugFoundError(m) => write!(f, "{m}"),
            RholangError::SpeculationInvalid {
                channel,
                writer_path,
                at_path,
            } => write!(
                f,
                "speculation invalid (Law 24): newest write on {channel:?} at {writer_path:?} is not DFS-earlier than the committing path {at_path:?}"
            ),
            RholangError::NormalizerError(m) => write!(f, "{m}"),
            RholangError::SyntaxError(m) => write!(f, "{m}"),
            RholangError::LexerError(m) => write!(f, "{m}"),
            RholangError::ParserError(m) => write!(f, "{m}"),
            RholangError::UnboundVariableRef { var_name, line, col } => {
                write!(f, "Variable reference: ={var_name} at {line}:{col} is unbound.")
            }
            RholangError::UnexpectedNameContext {
                var_name,
                proc_var_source_position,
                name_source_position,
            } => write!(
                f,
                "Proc variable: {var_name} at {proc_var_source_position} used in Name context at {name_source_position}"
            ),
            RholangError::UnexpectedReuseOfNameContextFree {
                var_name,
                first_use,
                second_use,
            } => write!(
                f,
                "Free variable {var_name} is used twice as a binder (at {first_use} and {second_use}) in name context."
            ),
            RholangError::UnexpectedProcContext {
                var_name,
                name_var_source_position,
                process_source_position,
            } => write!(
                f,
                "Name variable: {var_name} at {name_var_source_position} used in process context at {process_source_position}"
            ),
            RholangError::UnexpectedReuseOfProcContextFree {
                var_name,
                first_use,
                second_use,
            } => write!(
                f,
                "Free variable {var_name} is used twice as a binder (at {first_use} and {second_use}) in process context."
            ),
            RholangError::UnexpectedBundleContent(m) => write!(f, "{m}"),
            RholangError::UnrecognizedNormalizerError(m) => write!(f, "{m}"),
            RholangError::OutOfPhlogistonsError => write!(f, "Computation ran out of phlogistons."),
            RholangError::TopLevelWildcardsNotAllowedError(w) => {
                write!(f, "Top level wildcards are not allowed: {w:?}.")
            }
            RholangError::TopLevelFreeVariablesNotAllowedError(v) => {
                write!(f, "Top level free variables are not allowed: {v:?}.")
            }
            RholangError::TopLevelLogicalConnectivesNotAllowedError(c) => {
                write!(f, "Top level logical connectives are not allowed: {c:?}.")
            }
            RholangError::SubstituteError { term } => write!(f, "Illegal Substitution [{term:?}]"),
            RholangError::PatternReceiveError(c) => {
                write!(f, "Invalid pattern in the receive: {c}. Only logical AND is allowed.")
            }
            RholangError::SetupError(m) => write!(f, "{m}"),
            RholangError::UnrecognizedInterpreterError(_) => {
                write!(f, "Unrecognized interpreter error")
            }
            RholangError::SortMatchError(m) => write!(f, "{m}"),
            RholangError::ReduceError(m) => write!(f, "{m}"),
            RholangError::MethodNotDefined { method, other_type } => {
                write!(f, "Error: Method `{method}` is not defined on {other_type}.")
            }
            RholangError::MethodArgumentNumberMismatch {
                method,
                expected,
                actual,
            } => write!(
                f,
                "Error: Method `{method}` expects {expected} Par argument(s), but got {actual} argument(s)."
            ),
            RholangError::OperatorNotDefined { op, other_type } => {
                write!(f, "Error: Operator `{op}` is not defined on {other_type}.")
            }
            RholangError::OperatorExpectedError {
                op,
                expected: _,
                other_type,
            } => write!(f, "Error: Operator `{op}` is not defined on {other_type}."),
            RholangError::AggregateError {
                interpreter_errors,
                errors,
            } => {
                write!(f, "Error: Aggregate Error")?;
                for e in interpreter_errors {
                    write!(f, "\n{e}")?;
                }
                for e in errors {
                    write!(f, "\n{e}")?;
                }
                Ok(())
            }
            RholangError::ReceiveOnSameChannelsError { line, col } => write!(
                f,
                "Receiving on the same channels is currently not allowed (at {line}:{col}). Ref. RCHAIN-4032."
            ),
        }
    }
}

impl std::error::Error for RholangError {}

/// Convenience result alias.
pub type Result<A> = std::result::Result<A, RholangError>;

#[cfg(test)]
mod tests {
    use super::*;

    use rchain_models::ast::Par;

    fn pos(row: i32, column: i32) -> SourcePosition {
        SourcePosition { row, column }
    }

    /// **Every `Display` arm, formatted.** These strings are what an operator reads in a failed
    /// deploy's error field (and what the register noted read as ~1% coverage "because nothing
    /// formats them"), so each variant is constructed and its message pinned — including the four
    /// that take a `Par` and the two that stringify a source position.
    #[test]
    fn every_variant_renders_its_message() {
        let par = Box::new(Par::default());
        let cases: Vec<(RholangError, &str)> = vec![
            // The pass-through variants carry their own message verbatim (faithful to Scala).
            (RholangError::BugFoundError("bug".into()), "bug"),
            (RholangError::NormalizerError("norm".into()), "norm"),
            (RholangError::SyntaxError("syntax".into()), "syntax"),
            (RholangError::LexerError("lex".into()), "lex"),
            (RholangError::ParserError("parse".into()), "parse"),
            (RholangError::UnexpectedBundleContent("bundle".into()), "bundle"),
            (RholangError::UnrecognizedNormalizerError("unk".into()), "unk"),
            (RholangError::SetupError("setup".into()), "setup"),
            (RholangError::SortMatchError("sort".into()), "sort"),
            (RholangError::ReduceError("reduce".into()), "reduce"),
            // The typed variants build their message from the fields.
            (
                RholangError::UnboundVariableRef {
                    var_name: "x".into(),
                    line: 1,
                    col: 2,
                },
                "Variable reference: =x at 1:2 is unbound.",
            ),
            (
                RholangError::UnexpectedNameContext {
                    var_name: "x".into(),
                    proc_var_source_position: pos(1, 0),
                    name_source_position: pos(0, 0),
                },
                "Proc variable: x at 1:0 used in Name context at 0:0",
            ),
            (
                RholangError::UnexpectedReuseOfNameContextFree {
                    var_name: "x".into(),
                    first_use: pos(0, 0),
                    second_use: pos(0, 0),
                },
                "Free variable x is used twice as a binder (at 0:0 and 0:0) in name context.",
            ),
            (
                RholangError::UnexpectedProcContext {
                    var_name: "x".into(),
                    name_var_source_position: pos(0, 0),
                    process_source_position: pos(0, 0),
                },
                "Name variable: x at 0:0 used in process context at 0:0",
            ),
            (
                RholangError::UnexpectedReuseOfProcContextFree {
                    var_name: "x".into(),
                    first_use: pos(0, 0),
                    second_use: pos(0, 0),
                },
                "Free variable x is used twice as a binder (at 0:0 and 0:0) in process context.",
            ),
            (
                RholangError::OutOfPhlogistonsError,
                "Computation ran out of phlogistons.",
            ),
            (
                RholangError::TopLevelWildcardsNotAllowedError(par.clone()),
                "Top level wildcards are not allowed:",
            ),
            (
                RholangError::TopLevelFreeVariablesNotAllowedError(par.clone()),
                "Top level free variables are not allowed:",
            ),
            (
                RholangError::TopLevelLogicalConnectivesNotAllowedError(par.clone()),
                "Top level logical connectives are not allowed:",
            ),
            (
                RholangError::SubstituteError {
                    term: rchain_models::ast::Var::Wildcard,
                },
                "Illegal Substitution [Wildcard]",
            ),
            (
                RholangError::PatternReceiveError("\\/ (disjunction)".into()),
                "Invalid pattern in the receive: \\/ (disjunction). Only logical AND is allowed.",
            ),
            (
                RholangError::UnrecognizedInterpreterError("whatever".into()),
                "Unrecognized interpreter error",
            ),
            (
                RholangError::MethodNotDefined {
                    method: "foo".into(),
                    other_type: "Int".into(),
                },
                "Error: Method `foo` is not defined on Int.",
            ),
            (
                RholangError::MethodArgumentNumberMismatch {
                    method: "foo".into(),
                    expected: 1,
                    actual: 2,
                },
                "Error: Method `foo` expects 1 Par argument(s), but got 2 argument(s).",
            ),
            (
                RholangError::OperatorNotDefined {
                    op: "%".into(),
                    other_type: "Tuple".into(),
                },
                "Error: Operator `%` is not defined on Tuple.",
            ),
            // `expected` is carried but *not* rendered — faithful to the Scala message (see
            // AUDIT.md §16), so the assertion deliberately expects the same text as
            // `OperatorNotDefined`.
            (
                RholangError::OperatorExpectedError {
                    op: "++".into(),
                    expected: "Set".into(),
                    other_type: "List".into(),
                },
                "Error: Operator `++` is not defined on List.",
            ),
            (
                RholangError::ReceiveOnSameChannelsError { line: 3, col: 4 },
                "Receiving on the same channels is currently not allowed (at 3:4). Ref. RCHAIN-4032.",
            ),
            (
                RholangError::SpeculationInvalid {
                    channel: rchain_models::sorted::SortedProc::new(
                        rchain_models::par_ops::from_expr(rchain_models::ast::Expr::GString(
                            "c".to_string(),
                        )),
                    ),
                    writer_path: vec![0],
                    at_path: vec![1],
                },
                "speculation invalid (Law 24)",
            ),
        ];

        for (error, expected) in cases {
            let rendered = error.to_string();
            assert!(
                rendered.contains(expected),
                "arm for {error:?} rendered {rendered:?}, expected it to contain {expected:?}"
            );
            assert!(!rendered.is_empty());
        }
    }

    /// `AggregateError` is the one arm that is not a single `write!`: it leads with its own line and
    /// then appends every collected error, interpreter errors first. A dropped inner error would make
    /// a multi-error deploy report only part of the failure.
    #[test]
    fn an_aggregate_error_renders_every_inner_error() {
        let error = RholangError::AggregateError {
            interpreter_errors: vec![RholangError::ReduceError("first".to_string())],
            errors: vec!["second".to_string()],
        };
        let rendered = error.to_string();
        let lines: Vec<&str> = rendered.lines().collect();
        assert_eq!(lines[0], "Error: Aggregate Error");
        assert_eq!(lines[1], "first");
        assert_eq!(
            lines[2], "second",
            "interpreter errors come first, then the others"
        );
    }

    /// `SourcePosition` renders `row:column` — the shape every position-bearing message depends on.
    #[test]
    fn a_source_position_renders_row_and_column() {
        assert_eq!(pos(7, 11).to_string(), "7:11");
        assert_eq!(pos(0, 0).to_string(), "0:0");
        // The two-position elements of a `Display` (`first_use`, `second_use`) are `i32` pairs, and
        // they are rendered through this same `Display`.
        assert_eq!(format!("{} and {}", pos(1, 2), pos(3, 4)), "1:2 and 3:4");
    }

    /// The error type is a real `std::error::Error`, so `?` and `Box<dyn Error>` sinks work — and it
    /// is `Clone`/`PartialEq`, which is what lets callers match on it (the `?` operator's `From`
    /// conversions included).
    #[test]
    fn it_is_a_std_error() {
        let error = RholangError::ReduceError("nope".into());
        let boxed: Box<dyn std::error::Error> = Box::new(error.clone());
        assert_eq!(boxed.to_string(), "nope");
        assert_eq!(error.clone(), error);
    }
}
