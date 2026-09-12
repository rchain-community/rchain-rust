# RNodeRust test targets.
#
# Layered suites:
#   test-unit        inline #[test]/#[tokio::test] unit tests across all crates
#   test-integration in-process node/casper/rholang integration tests (node/tests, casper/tests, ...)
#   test-multinode    multi-node consensus harness (casper/tests/multinode.rs)
#   test-all          unit + integration (default)
#   coverage          line/region coverage via cargo-llvm-cov (excludes the flaky crypto crate)

.PHONY: test test-unit test-integration test-multinode test-all coverage bench-scheduler spec

test: test-all

test-unit:
	cargo test --workspace --lib

test-integration:
	cargo test -p rchain-node -p rchain-casper -p rchain-rholang --tests

test-multinode:
	cargo test -p rchain-casper --test multinode

test-all: test-unit test-integration

coverage:
	cargo llvm-cov --workspace --exclude rchain-crypto

# The channel-scheduler benchmarks (Laws 20–22): pingpong/fanout workloads, the dfs/gate/relaxed
# mode sweep, and the striped-vs-unstriped hot-store comparison. Run with cargo and lake strictly
# serialized (each peaks ~6.5 GiB RAM).
bench-scheduler:
	cargo bench -p rspace-bench -- sched/

spec:
	cd spec && lake build
