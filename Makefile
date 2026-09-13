# RNodeRust test targets.
#
# Layered suites:
#   test-unit        inline #[test]/#[tokio::test] unit tests across all crates
#   test-integration in-process node/casper/rholang integration tests (node/tests, casper/tests, ...)
#   test-multinode    multi-node consensus harness (casper/tests/multinode.rs)
#   test-all          unit + integration (default)
#   check-register    verify spec/TEST-COVERAGE.md against the tree
#   coverage          line/region coverage via cargo-llvm-cov
#   bench-smoke       compile the Criterion benches so they cannot rot
#
# `--all-features` matters for the unit/integration suites: the `lmdb`-gated tests in
# `shared/src/lmdb.rs` and `rspace/src/state/exporters.rs` are silently skipped without it, which is
# how they escaped local runs while CI (`--all-features`) still exercised them.

.PHONY: test test-unit test-integration test-multinode test-all check-register coverage bench-scheduler bench-smoke spec

test: test-all

test-unit:
	cargo test --workspace --lib --all-features

test-integration:
	cargo test -p rchain-node -p rchain-casper -p rchain-rholang --tests --all-features

test-multinode:
	cargo test -p rchain-casper --test multinode --all-features

test-all: test-unit test-integration

# The coverage register must match the tree: no overstated counts, no phantom tests, no deferred
# rows. Drop `--deferred-ok` when the last deferred gap closes (the completion plan's Stage 2).
check-register:
	tools/audit-test-register.sh --deferred-ok

# Coverage. CI (`.github/workflows/coverage.yml`) has always run the whole workspace with
# `--all-features` and no exclusions, and is green on every PR — so the crypto crate is *not* flaky
# under instrumentation, and the local `--exclude rchain-crypto` that used to sit here was drift that
# made the local number disagree with the gate. Both now run the same command.
#
# The floor lives in CI (`--fail-under-lines`): raise it only after measuring, never to a number the
# plan hopes to reach. `cargo install cargo-llvm-cov` + `rustup component add llvm-tools-preview`
# are the prerequisites locally.
coverage:
	cargo llvm-cov --workspace --all-features

# The channel-scheduler benchmarks (Laws 20–22): pingpong/fanout workloads, the dfs/gate/relaxed
# mode sweep, and the striped-vs-unstriped hot-store comparison. Run with cargo and lake strictly
# serialized (each peaks ~6.5 GiB RAM).
bench-scheduler:
	cargo bench -p rspace-bench -- sched/

# Compile every bench target so an API change that breaks them fails a gate rather than going
# unnoticed until someone runs the benchmarks. `bench-scheduler` is the one that executes.
bench-smoke:
	cargo bench -p rspace-bench --no-run

spec:
	cd spec && lake build
