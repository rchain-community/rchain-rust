//! Tests for the build-provenance policy in `node/build_provenance.rs`.
//!
//! The policy is `include!`d rather than declared as a library module, because `build.rs` is its only
//! production consumer and a build script cannot test itself — a build script's own `#[cfg(test)]`
//! never runs under `cargo test`. Including the same source here is what lets the precedence in
//! W-N7-01 be pinned by a test: it is the decision, not the build, that is under test.

include!("../build_provenance.rs");
