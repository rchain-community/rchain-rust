# Test-coverage audit & gap analysis

This register records an adversarial audit of the Rust port's test coverage. It is the durable
companion to [`AUDIT.md`](AUDIT.md) (the *code* findings register): `AUDIT.md` records what the code
does wrong; this page records what the tests fail to catch. The audit is re-runnable — the numbers
below are from `branch dev` after the rust-first reimplementation (Phases 0–5).

## Inventory

**~750 `#[test]`/`#[tokio::test]` functions** across the workspace (unit + 16 integration, 5
proptest, 7 bench — the per-crate table below is authoritative; the headline count drifts as tests are
added). Only **3 of 12 crates have integration tests** (`rholang`, `casper`, `node`).

| Crate | Unit | Integration | Property | Bench |
|---|---|---|---|---|
| `sdk` | 34 | — | — | — |
| `shared` | 76 | — | — | — |
| `crypto` | 49 | — | — | — |
| `graphz` | 11 | — | — | — |
| `models` | 95 | — | — | — |
| `block-storage` | 14 | — | — | — |
| `comm` | 51 | — | — | — |
| `rspace` | 42 | — | 5 | — |
| `rholang` | 61 | 7 | — | — |
| `casper` | 105 | 5 | — | — |
| `node` | 59 | 2 | — | — |
| `rspace-bench` | — | — | — | 7 |

- **Property tests**: only `rspace/src/property_tests.rs` (Laws 7–10: join commutativity, deterministic
  COMM, Merkle determinism, merge monoid).
- **Scheduler (Laws 20–22)** — added after the table snapshot above:
  - `rspace/src/concurrent/channel_queue.rs` unit tests (claim/`claim_more`/head-order/phase-two
    re-wait) and `rspace/src/hot_store.rs` `striped_store_equals_unstriped` (the 64-shard store is
    observably identical to the 1-shard store);
  - `rholang/tests/execution.rs` — `gate_and_sequential_state_hashes_match` (gate state hash *and*
    event log equal the sequential reference over the reduction corpus),
    `relaxed_mode_runs_all_corpus_terms_without_error`, `relaxed_preserves_same_channel_order`
    (per-channel COMM subsequences equal the sequential DFS order; standalone install/store events
    may interleave around a `Comm`, so only commits are compared; the S.3 counterexample term's
    final state is free);
  - `casper/tests/scheduler.rs` — `block_paths_reject_relaxed_mode` (both block-path entry points
    hard-reject under `relaxed`; sequential succeeds) and
    `exploratory_path_stays_open_in_relaxed_mode`;
  - `rspace-bench/benches/rspace_bench.rs` `sched` group — pingpong/fanout workloads, the
    dfs/gate/relaxed/relaxed-validated workers-1–8 sweep, and striped-vs-unstriped store
    contention (`make bench-scheduler`).
- **Scheduler, on-chain (Laws 23–25)** — the Lean formalization
  (`spec/Rchain/SchedulerOnchain.lean`, built by `cd spec && lake build`) proves the Law 24
  witnesses (`s3_pair_fails_validation`, `later_write_pollution_unsound`,
  `trace_equality_without_serializability`, plus the boundary witnesses
  `dispatched_serializable_log_inequality`, `certificate_blind_late_writer_diverges`,
  `writer_chain_needs_nodup`), the writer chain (`serializable_writer_chain`), the pinned
  publication theorem (`dfs_serializable_implies_log_equal`), and the Law 25 coordinator
  refinement (`validated_speculation_refines_apply`, `fallback_rerun_published`) — no axioms.
  The Rust tests: `casper/tests/scheduler.rs` `relaxed_validated_accepts_block_paths` (the mode
  passes the block-path entry points), `relaxed_validated_compute_state_matches_sequential`
  (post-state hash + per-channel COMM multisets equal the sequential reference across the
  corpus — now including the C/D and persistent-produce pairs — with the S.3 fallback term),
  and `relaxed_validated_never_diverges_from_sequential` (20 draws of each free-class term
  against the sequential manager); `rholang/tests/execution.rs` — the per-channel COMM-multiset
  oracle (`relaxed_preserves_same_channel_order`) and
  `relaxed_validated_mode_runs_corpus_without_error`;
  `rspace/src/concurrent/channel_queue.rs` `enqueue_window_sets_skew_and_version` and the
  `property_tests.rs` `law24_skew_signal_and_version_counter` proptest.
- **Differential/golden**: `models` wire bitset, `rspace` scodec + stable-hash TSV, `rholang`
  execution post-state hashes, and crypto known-answer vectors — the Scala-ground-truth tests.
- **The 110 legacy `.rho`/`.rhox` contracts** under `legacy/` are **not referenced by any Rust test**;
  the parser/reducer are only exercised against inline strings (and, historically, the now-native
  genesis contracts).

## The 1 remaining `#[ignore]`d test

- **`casper/tests/finalization.rs:303` `round_robin_finalizes_common_prefix`** — a lockstep/round-robin
  DAG never advances the fringe (the Scala `MultiParentCasperFinalizationSpec` is itself `ignore`d).
  The finalizing shape is covered by `fork_structure_advances_fringe` (active).

*(The former second ignored test, `rholang/tests/execution.rs` `list_channel_matches`, was a
list-as-channel hash/equality bug that was resolved during the reimplementation; it is now
un-ignored and passing.)*

## Gap analysis (severity-ordered)

For each gap: **code location** → **current test state** → **the seam a regression test attaches to**.

- **G1 — Equivocation rejection is untested** (`casper/src/dag.rs:146-160`). The H-1 fix rejects a
  second block by the same sender reusing `seq_num`; zero tests exercise it. *Seam:*
  `BlockDagKeyValueStorage::insert` over the in-memory `build_storage()` helper.

- **G2 — DoS / resource limits are untested.** `RateLimiter` (`node/src/api/grpc/mod.rs:57`), the
  chunker underflow guard (`comm/src/transport/chunker.rs:43` `checked_sub`), and the dispatch
  semaphore (`comm/src/transport/grpc_transport_receiver.rs`, `MAX_CONCURRENT_DISPATCH`). *Seam:*
  each is a pure function or an `Arc<Semaphore>`.

- **G3 — PoS lifecycle mutations** (`rholang/src/native_state.rs`): the dynamic-validator lifecycle is
  covered — `bond` trust admission + min/max + funds + activation, `withdraw` deactivation + quarantine
  refund via `close_block`, `slash`/`untrust` confiscation to the Coop vault, active-set top-N
  selection, and genesis install. `pre_charge` (incl. insufficient funds) and the revVault
  deposit/transfer paths are tested. *Seam:* `NativeSystemState` over `InMemNativeStore::empty()`, plus
  `casper/tests/consensus.rs::bond_deploy_updates_the_active_validator_set` for the end-to-end
  deploy → active-set → replay path. Not covered: `refund` (still a documented no-op) and reward
  distribution (deferred).

- **G4 — Gas-metering enforcement is under-tested.** `ChargingRSpace::produce/consume`
  (`rholang/src/storage.rs:106-131`) charge paths have no test; there is no end-to-end test that a
  deploy exceeding `phlo_limit` fails. *Seam:* `ChargingRSpace::new(space, cost)` with a tiny balance,
  and `RuntimeManager::process_deploy` with a low `phlo_limit`.

- **G5 — State-sync export/import has no round-trip test** (`rspace/src/state/*`). Only
  `MockExporter`/trivial single-leaf tests. *Seam:* real `RSpaceExporter` → real `RSpaceImporter`.

- **G6 — History checkpoint/reset/rollback is untested.** `history_repository.rs`, `roots_store.rs`,
  `root_repository.rs`, `checkpoint.rs`, and core `rspace/src/rspace.rs` have no `mod tests`. *Seam:*
  `RSpace::create_checkpoint`/`reset`/`create_soft_checkpoint`/`revert_to_soft_checkpoint`.

- **G7 — Replay is only tested on a trivial `@chan!(42)`.** `replay_rspace.rs`/`runtime_replay.rs`
  internals (`check_replay_data`) have no direct tests. *Seam:* non-trivial deploys (multi-channel
  COMM, persistent `!!`, peek `<<-`, native mutation) through `replay_compute_state`.

- **G8 — The Finalizer full loop is only indirectly tested.** `calculate_next_fringe_support_map`
  and `calculate_finalization` have no direct tests. *Seam:* the `msg()` helper in
  `finalizer.rs` tests.

- **G9 — Malformed-input rejection is partial.** `NodeIdentifier::from_hex`/`from_address`
  (`comm/src/peer_node.rs:25-31,125`) and `KeySegment` (`rspace/src/history/key_segment.rs`) reject
  bad input but have no negative tests. (`base16` and the validate layer are well covered.)

- **G10 — TLS trust-manager decision logic is untested**
  (`comm/src/transport/hostname_trust_manager.rs:59-172`). Only cert-*generation* consistency is
  tested; wrong-CN and unknown-client-cert rejection are not. *Seam:* the `verify_server_cert`/
  `verify_client_cert` fns + `generate_certificate_if_absent`.

- **G11 — No socket-level transport test exists anywhere.** Every `comm` test is a pure-function
  unit test; no gRPC/TLS handshake or message round-trip over real I/O. *Seam:* a loopback tonic
  server+client (precedent: `node/src/api/grpc/tonic.rs:968` `serves_and_answers_propose`).

- **G12 — The scheduled-path cost charging is untested directly** (`rholang/src/storage.rs`
  `ChargingRSpace::{produce_at,consume_at,commit_produce}`). The corpus tests exercise the paths
  end-to-end, but no unit test pins *where* the storage/event/COMM charges land in the
  phase-one/two split (up-front storage cost; produce event cost inline when `phase_two` is `None`,
  else event+COMM costs at commit). *Seam:* `ChargingRSpace::new(space, cost)` with a tiny balance
  + `produce_at`/`commit_produce` directly, mirroring the G4 charge-path tests.

## Cross-links

- [`AUDIT.md`](AUDIT.md) — the code findings register (the security fixes the tests must pin).
- [`RUST-FIRST.md`](RUST-FIRST.md) — the native system-contract state model (G3/G5/G6 touch it).
- [`RHO-CALCULUS.md`](RHO-CALCULUS.md) / [`INVENTORY.md`](INVENTORY.md) — the 25-law oracle the
  property + replay tests assert.

## Remediation status

| Gap | Status |
|---|---|
| G1 equivocation | ✅ regression test (`casper/dag.rs`) |
| G2 DoS limits | ✅ RateLimiter + chunker + deploy-pool cap (R2) + decompression cap (R3) + parser depth guard (R9); ⏸ dispatch-semaphore (over tokio's tested primitive) |
| G3 PoS mutations | ✅ `bond`/`withdraw`/`close_block`/`slash`/`trust`/`untrust` + active-set cap + end-to-end bond→active-set→replay (`casper/tests/consensus.rs`) |
| G4 gas enforcement | ✅ `ChargingRSpace` + `phlo_limit` exhaustion end-to-end |
| G5 state-sync | ✅ export→validate round-trip; ⏸ full store export→import→compare |
| G6 history checkpoint/reset/rollback | ✅ `RSpace::create_checkpoint`/`reset`/`revert` |
| G7 replay | ✅ persistent+peek replay matches play; ⏸ `check_replay_data` negative path |
| G8 finalizer | ✅ `calculate_finalization` fork/lockstep |
| G9 malformed input | ✅ `NodeIdentifier`/`KeySegment` + `BlockHash::try_from`/`try_from_hex` (R12) + deploy-signature verify (R1) |
| G10 TLS trust-manager | ✅ wrong-hostname + stale-cert rejection |
| G11 transport socket | ✅ loopback mutual-TLS gRPC send round-trip (`grpc_transport.rs`) |
| G12 scheduled-path charging | ⏸ corpus tests cover it end-to-end; the charge-placement unit test is open |

*(✅ = covered; ⏸ = deferred with the seam documented above.)*
