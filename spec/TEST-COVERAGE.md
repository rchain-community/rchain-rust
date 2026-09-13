# Test-coverage audit & gap analysis

This register records an adversarial audit of the Rust port's test coverage. It is the durable
companion to [`AUDIT.md`](AUDIT.md) (the *code* findings register): `AUDIT.md` records what the code
does wrong; this page records what the tests fail to catch.

**The register is machine-checked.** [`tools/audit-test-register.sh`](../tools/audit-test-register.sh)
recomputes every number below, verifies that every test the register names actually exists, and fails
on any still-deferred row. It exists because this page drifted: it claimed an `#[ignore]`d test that
no longer existed, "110 legacy contracts" when the tree held 165, and per-crate counts several
releases stale. Run it after changing this file:

```sh
tools/audit-test-register.sh              # hard: no deferred rows, no open tier items
tools/audit-test-register.sh --deferred-ok # while the tiered work is in flight
```

## Inventory

**769 `#[test]`/`#[tokio::test]` unit functions + 67 integration tests** across 13 crates, with **6
laws** carrying a randomized property test and **10 benchmark functions** in 6 Criterion groups. Only
**3 of 12 crates have integration tests** (`rholang`, `casper`, `node`).

| Crate | Unit | Integration | Property (laws) | Bench |
|---|---|---|---|---|
| `sdk` | 35 | — | — | — |
| `shared` | 77 | — | — | — |
| `crypto` | 52 | — | — | — |
| `graphz` | 11 | — | — | — |
| `models` | 104 | — | — | — |
| `block-storage` | 17 | — | — | — |
| `comm` | 59 | — | — | — |
| `rspace` | 79 | — | 6 | — |
| `rholang` | 107 | 32 | — | — |
| `casper` | 128 | 29 | — | — |
| `node` | 80 | 6 | — | — |
| `qucalc` | 20 | — | — | — |
| `rspace-bench` | — | — | — | 10 |

The counts are a **floor, not a target**: the linter fails only when the register claims *more* than
the tree holds, so this table may lag as tests are added (it reports the drift) but can never
overstate coverage.

- **Property tests** — `rspace/src/property_tests.rs` only: 7 property functions over **6 distinct
  laws** (7 join commutativity, 8 deterministic COMM, 9 merge monoid, 10 Merkle determinism, 20
  per-channel path order, 24 the write-record layer). `proptest` is not a dependency of any other
  crate, so `AGENTS.md`'s "property test per law" is satisfied for 6 of 29 laws — see
  [Risk tiers](#risk-tiers-per-module) and the completion plan.
- **Scheduler (Laws 20–22)** — `rspace/src/concurrent/channel_queue.rs` unit tests (claim/`claim_more`/
  head-order/phase-two re-wait) and `rspace/src/hot_store.rs` `striped_store_equals_unstriped` (the
  64-shard store is observably identical to the 1-shard store); `rholang/tests/execution.rs`
  `gate_and_sequential_state_hashes_match`, `relaxed_mode_runs_all_corpus_terms_without_error`,
  `relaxed_preserves_same_channel_order`; `casper/tests/scheduler.rs`
  `block_paths_reject_relaxed_mode`, `exploratory_path_stays_open_in_relaxed_mode`; and the
  `rspace-bench` `sched` group (workers-1–8 sweep, striped-vs-unstriped contention).
- **Scheduler, on-chain (Laws 23–25)** — the Lean formalization (`spec/Rchain/SchedulerOnchain.lean`,
  `cd spec && lake build`) proves the Law 24 witnesses, the writer chain, the pinned publication
  theorem and the Law 25 coordinator refinement with **no axioms**. The Rust side:
  `casper/tests/scheduler.rs` `relaxed_validated_accepts_block_paths`,
  `relaxed_validated_compute_state_matches_sequential`,
  `relaxed_validated_never_diverges_from_sequential`; `rholang/tests/execution.rs`
  `relaxed_validated_mode_runs_corpus_without_error`; `rspace/src/concurrent/channel_queue.rs`
  `enqueue_window_sets_skew_and_version` and the `law24_skew_signal_and_version_counter` proptest.
- **Differential/golden** — `models/testdata/differential/wire.tsv`, `rspace/testdata/differential/
  {stable_hash,scodec}.tsv`, `rholang/testdata/differential/execution.tsv` (3 rows), and the crypto
  known-answer vectors. These are *captured* Scala vectors, not a live Scala-vs-Rust harness; the
  generator is `legacy/scripts/gen-differential-goldens.sh` (see the [open gap](#deferred-and-blocked)).
- **The legacy `.rho`/`.rhox` contract corpus** — **165 `.rho` + 1 `.rhox`** under `legacy/`. The one
  `.rhox` (`legacy/casper/src/main/resources/Pos.rhox`) is a Scala-side macro template, not a program.
  **None is referenced by any Rust test**: the parser and reducer are exercised only against inline
  strings and the `qucalc`/`examples` `.rho` files. This is the largest single untested surface — the
  Rust parser has never seen 165 real programs (see [Risk tiers](#risk-tiers-per-module)).

### There are no `#[ignore]`d tests

A live search finds **no `#[ignore]` anywhere in the Rust tree**. This section previously claimed
`casper/tests/finalization.rs:303 round_robin_finalizes_common_prefix` was ignored; that file is now
117 lines and contains one active test (`fork_structure_advances_fringe`), and the phantom entry is
what prompted the linter. The former second case, `rholang/tests/execution.rs` `list_channel_matches`,
was un-ignored long ago and passes.

## Machine-checked claims

Every row names a file and a test function; `tools/audit-test-register.sh` fails if the function is
not found in that file. Coverage claims live here rather than in prose so they cannot rot silently.

| Gap | File | Test |
|---|---|---|
| G1 | `casper/src/dag.rs` | `insert_rejects_equivocation_same_seq_num` |
| G2 | `casper/src/dag.rs` | `add_deploy_rejects_when_pool_full` |
| G2 | `comm/src/transport/chunker.rs` | `chunk_it_rejects_too_small_max_message_size` |
| G2 | `comm/src/transport/stream_handler.rs` | `restore_rejects_oversized_decompressed_content` |
| G2 | `shared/src/rate_limiter.rs` | `admits_exactly_max_per_window_then_refuses` |
| G2 | `node/src/web/http.rs` | `api_deploy_returns_429_when_the_limiter_is_exhausted` |
| G2 | `comm/src/transport/grpc_transport.rs` | `a_full_dispatch_queue_is_rejected_and_recovers` |
| G2 | `rholang/src/parser.rs` | `rejects_excessive_nesting_depth` |
| G3 | `rholang/src/native_state.rs` | `bond_requires_trust_admission` |
| G3 | `casper/tests/consensus.rs` | `bond_deploy_updates_the_active_validator_set` |
| G3 | `rholang/src/native_state.rs` | `refund_is_a_documented_no_op` |
| G4 | `casper/tests/consensus.rs` | `deploy_exceeding_phlo_limit_fails_and_next_runs` |
| G5 | `rspace/src/state/mod.rs` | `validate_state_items_accepts_valid_round_trip` |
| G5 | `rspace/src/state/mod.rs` | `validate_state_items_rejects_corrupted_data` |
| G5 | `rspace/src/state/instances.rs` | `a_populated_store_export_import_round_trips` |
| G6 | `rspace/src/rspace.rs` | `checkpoint_reset_returns_persisted_data` |
| G6 | `rspace/src/rspace.rs` | `soft_checkpoint_revert_rolls_back_produces` |
| G7 | `casper/tests/consensus.rs` | `replay_matches_play_for_persistent_and_peek` |
| G7 | `rholang/tests/execution.rs` | `replay_matches_play` |
| G7 | `casper/tests/determinism.rs` | `a_tampered_deploy_replays_to_a_rejected_state_hash` |
| G12 | `rholang/src/storage.rs` | `produce_at_charges_the_storage_up_front` |
| G12 | `rholang/src/storage.rs` | `commit_produce_charges_the_event_at_the_commit` |
| G8 | `block-storage/src/dag/finalizer.rs` | `calculate_finalization_advances_fringe_on_fork` |
| G9 | `comm/src/peer_node.rs` | `from_hex_rejects_malformed_input` |
| G9 | `comm/src/peer_node.rs` | `from_address_rejects_malformed_uris` |
| G9 | `rspace/src/history/key_segment.rs` | `try_from_rejects_oversized_segment` |
| G10 | `comm/src/transport/hostname_trust_manager.rs` | `server_verifier_rejects_wrong_hostname` |
| G11 | `comm/src/transport/grpc_transport.rs` | `send_round_trips_over_socket` |

## Gap analysis (severity-ordered)

For each gap: **code location** → **current test state** → **the seam a regression test attaches to**.
**No gap row is deferred any more.** All four ⏸ rows at the time of writing (G2's semaphore, G5, G7,
G12) were closed by the completion plan's Stage 2 — and G6's history internals were moved into the
risk-tier table below, where an open module is a defect the linter reports rather than a note. A `⏸`
now appears only if a *new* gap is deliberately left open, and `tools/audit-test-register.sh` fails on
it unless `--deferred-ok` is passed (which today tolerates only the tier table's open items).

- **G1 — Equivocation rejection** (`casper/src/dag.rs`). The H-1 fix rejects a second block by the
  same sender reusing `seq_num`. ✅ `insert_rejects_equivocation_same_seq_num`. *Seam:*
  `BlockDagKeyValueStorage::insert` over the in-memory `build_storage()` helper.

- **G2 — DoS / resource limits.** Four legs are ✅ with named tests: the deploy-pool cap
  (`casper/src/dag.rs` `add_deploy_rejects_when_pool_full`), the chunker underflow guard
  (`comm/src/transport/chunker.rs` `chunk_it_rejects_too_small_max_message_size`), the lz4
  decompression cap (`comm/src/transport/stream_handler.rs`
  `restore_rejects_oversized_decompressed_content`), and the parser depth guard
  (`rholang/src/parser.rs` `rejects_excessive_nesting_depth`, plus the end-to-end
  `rholang/tests/execution.rs` `unbounded_recursion_hits_depth_limit_not_stack_overflow`).
  The `RateLimiter` leg is now ✅ too (`admits_exactly_max_per_window_then_refuses`,
  `resets_after_its_window`, `zero_never_admits`, plus a 429 asserted through the deploy route) — an
  earlier revision of this register wrongly marked it covered when nothing tested it. The dispatch
  bound is ✅ too, but not by testing at 1024: reaching that honestly means **1025 concurrent TLS
  streams**, so `ConcurrencyLimits` makes the bound a parameter (a small production change, approved
  explicitly) and `a_full_dispatch_queue_is_rejected_and_recovers` saturates it at 1 — asserting both
  the refusal and that a released slot admits the next message, i.e. a queue rather than a latch.
  That test also surfaced a diagnostic wart worth knowing: the refusal reaches callers as
  `MessageTooLarge`, because `process_error` maps `ResourceExhausted` to it (AUDIT.md §15 C4).
  *Seams:* the rate limiter is pure (`new(max_per_sec)`/`allow()`) — window/reset/zero unit tests,
  plus a 429 assertion through the HTTP routes. The semaphore is local to a spawned task, so the
  honest test is **socket-level** (open `MAX+K` streams and assert the last is unanswered; precedent
  `node/src/api/grpc/tonic.rs` `serves_and_answers_propose`) — preferred over extracting an accessor,
  which would be a production change for a test.

- **G3 — PoS lifecycle mutations** (`rholang/src/native_state.rs`). ✅ `bond` trust admission +
  min/max + funds + activation, `withdraw` deactivation + quarantine refund via `close_block`,
  `slash`/`untrust` confiscation to the Coop vault, active-set top-N selection, genesis install,
  `pre_charge` incl. insufficient funds, and the revVault deposit/transfer paths. *Seam:*
  `NativeSystemState` over `InMemNativeStore::empty()`, plus the end-to-end
  deploy → active-set → replay path in `casper/tests/consensus.rs`. **Not covered:** `refund` (a
  documented no-op — to be pinned so a half-implemented refund trips) and reward distribution
  (deferred as a *feature*, not a test).

- **G4 — Gas-metering enforcement** (`rholang/src/storage.rs` `ChargingRSpace::produce/consume`).
  ✅ end-to-end: `deploy_exceeding_phlo_limit_fails_and_next_runs` in `casper/tests/consensus.rs`.
  *Seam for the remaining unit leg:* `ChargingRSpace::new(space, cost)` with a tiny balance, and
  `RuntimeManager::process_deploy` with a low `phlo_limit`.

- **G5 — State-sync export/import round-trip** (`rspace/src/state/*`). ✅ The export→validate
  round-trip *and* a populated store's export→import→compare
  (`a_populated_store_export_import_round_trips`: five items and a root through the real exporter and
  importer). *Seam:* real `RSpaceExporter`
  → real `RSpaceImporter`, K distinct channels, compare the root hash and spot leaves. Extend
  `rspace/src/state/exporters.rs`'s in-file tests (today only `MockExporter`).

- **G6 — History checkpoint/reset/rollback.** ✅ `RSpace::create_checkpoint`/`reset`/
  `create_soft_checkpoint`/`revert_to_soft_checkpoint` have in-file tests in `rspace/src/rspace.rs`
  (contrary to an earlier claim here that the file had no `mod tests`). The history *internals*
  (`history_repository.rs`, `roots_store.rs`, `root_repository.rs`, `checkpoint.rs`) remain untested —
  see the T1 tier.

- **G7 — Replay internals** (`casper/src/runtime_replay.rs`, `rspace/src/replay_rspace.rs` — note this
  register previously named a non-existent `rholang/src/runtime_replay.rs`). ✅ persistent + peek
  replay matches play, over non-trivial deploys. ✅ The negative path too, with a finding: see
  AUDIT.md §15 C3 and `a_tampered_deploy_replays_to_a_rejected_state_hash`. *Seam:* drive the *recorded* trace into divergence. **Spike:** whether the
  recorded trace is mutable from a test; if not, a `#[cfg(test)]` accessor is the minimal change and
  must be listed before writing.

- **G8 — The Finalizer full loop.** ✅ `calculate_finalization` fork/lockstep
  (`calculate_finalization_advances_fringe_on_fork`); the direct-loop gap is closed.
  `calculate_next_fringe_support_map`'s antichain property is not separately pinned.

- **G9 — Malformed-input rejection.** ✅ `NodeIdentifier::from_hex`/`from_address`, `KeySegment`
  (`try_from_rejects_oversized_segment`), `BlockHash::try_from`/`try_from_hex`, and the deploy
  signature verify.

- **G10 — TLS trust-manager decision logic.** ✅ wrong-hostname and stale-cert rejection
  (`comm/src/transport/hostname_trust_manager.rs`).

- **G11 — Socket-level transport.** ✅ a mutual-TLS gRPC send round-trip over a real socket
  (`comm/src/transport/grpc_transport.rs` `send_round_trips_over_socket`). The rest of the `comm`
  transport/discovery subtree is still pure-function-only — see the T2 tier.

- **G12 — Scheduled-path cost charging** (`rholang/src/storage.rs`
  `ChargingRSpace::{produce_at,consume_at,commit_produce}`). ✅ The placement is pinned: the storage
  cost lands up front, the produce event inline when phase one stores without a match, and the
  event+COMM costs at the commit — `produce_at_charges_the_storage_up_front`,
  `produce_at_fails_before_storing_when_the_balance_is_spent`,
  `commit_produce_charges_the_event_at_the_commit`. *Seam:* `ChargingRSpace::new(space, cost)` with a tiny balance +
  `produce_at`/`commit_produce` directly, mirroring the G4 charge-path tests.

## Remediation status

| Gap | Status |
|---|---|
| G1 equivocation | ✅ `insert_rejects_equivocation_same_seq_num` |
| G2 DoS limits | ✅ chunker underflow guard, deploy-pool cap, decompression cap, parser depth guard, the `RateLimiter` (window/zero/reset + a 429 through the route), and the dispatch bound (saturation *and* recovery) — the last via an injectable limit, since the production 1024 would need 1025 concurrent TLS streams |
| G3 PoS mutations | ✅ lifecycle + end-to-end bond→active-set→replay + the `refund` no-op pin; reward distribution **out of scope** (feature) |
| G4 gas enforcement | ✅ end-to-end phlo exhaustion; unit charge paths land with G12 |
| G5 state-sync | ✅ `validate_state_items_{accepts_valid_round_trip,rejects_corrupted_data}` + a populated store's export→import→compare (`a_populated_store_export_import_round_trips`) |
| G6 history checkpoint/reset/rollback | ✅ `RSpace::create_checkpoint`/`reset`/`revert`. The history *internals* (`history_repository.rs`, `roots_store.rs`, `root_repository.rs`) are not a gap row of their own: they are T1 tier entries in the table below, which the linter enforces, so an open one is a defect rather than a deferred note |
| G7 replay | ✅ `replay_matches_play{,_for_persistent_and_peek}`; ✅ the negative path — with a finding (AUDIT.md §15 C3): the inner trace check does not fire for a term tamper, so the state-hash comparison in `handle_errors` is what carries the invariant, and `a_tampered_deploy_replays_to_a_rejected_state_hash` pins both halves |
| G8 finalizer | ✅ `calculate_finalization` fork/lockstep |
| G9 malformed input | ✅ `NodeIdentifier`/`KeySegment`/`BlockHash` + deploy-signature verify |
| G10 TLS trust-manager | ✅ wrong-hostname + stale-cert rejection |
| G11 transport socket | ✅ loopback mutual-TLS gRPC round trip |
| G12 scheduled-path charging | ✅ `produce_at_charges_the_storage_up_front`, `produce_at_fails_before_storing_when_the_balance_is_spent`, `commit_produce_charges_the_event_at_the_commit` |

*(✅ = covered; ⏸ = deferred with the seam documented above.)*

### Deferred-gap seam classification

Each gap was read to decide whether closing it is a **test gap** (the behaviour is reachable through
existing public APIs) or a **seam gap** (production code must change first). Every one was a **test
gap** — closing them needed **no production change at all**, which is worth recording because the
opposite was the plan's standing assumption:

| Gap | Classification | Why |
|---|---|---|
| G2 dispatch semaphore | test gap — **closed via an injectable limit** | the bound is 1024, so the honest socket test would need 1025 concurrent TLS streams. `ConcurrencyLimits` makes the bound a parameter (a small production change, approved), so the test saturates at 1 and observes both the refusal and the recovery |
| G5 full store round-trip | test gap — **closed** | the concrete `RSpaceExporterStore`/`RSpaceImporterStore` are public and already used by the node (`create_rspace_exporter`/`create_rspace_importer`); `a_populated_store_export_import_round_trips` drives them with a populated store |
| G7 replay negative path | test gap — **closed, with a finding** | `ProcessedDeploy` is public, so a test can replay a tampered deploy. The spike found that the *inner* check does not fire for a term tamper (AUDIT.md §15 C3), so the test pins the state-hash comparison in `handle_errors` — the check that actually carries the invariant |
| G12 charge placement | test gap — **closed** | `ChargingRSpace::new` is public and `PendingProduce`'s fields are public, so both the phase-one and the commit charge points are reachable directly |

This matters for sequencing: Stage 2 is de-risked, and any future gap that *does* need a production
seam is called out here before it is written rather than smuggled into a test commit.

## Risk tiers (per-module)

The bar is **behavioural and tiered**: a defect in T1 can fork the chain or lose funds; T2 is
correctness-adjacent (wrong answers); T3 is presentation and utility. The tier is recorded here so a
bare T1 module is visible as a defect rather than lost in a statistic. Each row names the module and
the test that pins its **failure** path; `—` means the module is a registered open item and the
linter reports it.

| Tier | Module | Test pinning a failure path |
|---|---|---|
| T1 | `casper/src/conf.rs` | `a_spec_rejects_an_illegal_name_or_parent` |
| T1 | `casper/src/gateway/ledger.rs` | `an_abort_is_absorbing` |
| T1 | `casper/src/gateway/mod.rs` | `apply_phase_two_skips_legs_that_did_not_vote_ready` |
| T1 | `casper/src/txn_coordinator.rs` | `vote_from_reply_maps_every_reply` |
| T1 | `casper/src/multi_parent_casper.rs` | — |
| T1 | `casper/src/blocks/block_processor.rs` | — |
| T1 | `casper/src/runtime_replay.rs` | — |
| T1 | `casper/src/blocks/proposer/block_creator.rs` | — |
| T1 | `rspace/src/history/history_repository.rs` | `two_repositories_over_the_same_stores_agree_on_the_root` |
| T1 | `rspace/src/history/roots_store.rs` | `validate_and_set_refuses_an_unknown_root` |
| T1 | `rspace/src/history/root_repository.rs` | `an_unknown_root_is_an_error_and_does_not_move_the_current_root` |
| T1 | `rspace/src/checkpoint.rs` | — |
| T1 | `rspace/src/replay_rspace.rs` | — |
| T1 | `rspace/src/scheduled_space.rs` | — |
| T1 | `shared/src/rate_limiter.rs` | `zero_never_admits` |
| T1 | `rholang/src/scheduler.rs` | `effect_mode_rejects_an_unknown_name_and_names_the_alternatives` |
| T1 | `rholang/src/dispatch.rs` | — |
| T1 | `models/src/par_ops.rs` | `par_concat_preserves_the_canonical_form` |
| T2 | `rspace/src/merger/state_change_merger.rs` | — |
| T2 | `models/src/rholang.rs` | — |
| T2 | `casper/src/protocol/comm_util.rs` | — |
| T2 | `rholang/src/proc_ast.rs` | — |
| T2 | `node/src/configuration/configuration.rs` | `shards_and_scalar_keys_are_mutually_exclusive` |
| T2 | `node/src/configuration/commandline/config_mapper.rs` | — |
| T2 | `node/src/api/admin_web_api.rs` | — |
| T2 | `comm/src/discovery/kademlia_store.rs` | — |
| T3 | `graphz/src/lib.rs` | — |
| T3 | `node/src/web/status_info.rs` | — |

## Blocked and out of scope (by decision)

- **Multi-shard Docker devnet** — blocked on per-shard LFS sync. A two-shard devnet cannot peer
  because the LFS fringe exchange carries no shard id, so a node that is a member of several shards
  refuses to start without a local chain (the guard in `node/src/runtime/node_runtime.rs`: *"<shard>
  has no local chain and LFS sync is not shard-aware; start the node with --standalone …"*).
  **Per-shard LFS sync is a product feature, not a test gap.** Consequence: every multi-shard gateway
  claim is pinned **in-process** (`casper/tests/gateway_txn.rs`, `casper/tests/gateway_faults.rs`,
  `node/tests/gateway.rs`, `node/tests/gateway_routes.rs`), the Docker fuzzer stays single-shard, and
  the day per-shard LFS sync lands this register must grow a two-shard devnet row.
- **Reward computation/distribution** and the vault **unforgeable-name capability** — deferred with
  the simplified balance-map model ([`RUST-FIRST.md`](RUST-FIRST.md)); features, not test gaps.
- **Laws 12–13 (Rosette VM)** — orphaned (`rosette`/`roscala` out of scope), so no Rust test is
  expected and the law matrix is not empty there by neglect.

## Cross-links

- [`AUDIT.md`](AUDIT.md) — the code findings register (the security fixes the tests must pin).
- [`RUST-FIRST.md`](RUST-FIRST.md) — the native system-contract state model (G3/G5/G6 touch it).
- [`RHO-CALCULUS.md`](RHO-CALCULUS.md) / [`INVENTORY.md`](INVENTORY.md) — the 29-law oracle the
  property + replay tests assert.
