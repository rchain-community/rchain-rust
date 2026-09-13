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

**1144 `#[test]`/`#[tokio::test]` unit functions + 84 integration tests** across 13 crates, with **6
laws** carrying a randomized property test and **10 benchmark functions** in 6 Criterion groups. Only
**3 of 12 crates have integration tests** (`rholang`, `casper`, `node`).

| Crate | Unit | Integration | Property (laws) | Bench |
|---|---|---|---|---|
| `sdk` | 46 | — | 2 | — |
| `shared` | 80 | — | — | — |
| `crypto` | 58 | — | — | — |
| `graphz` | 18 | — | — | — |
| `models` | 131 | — | 5 | — |
| `block-storage` | 28 | — | 3 | — |
| `comm` | 108 | — | — | — |
| `rspace` | 129 | — | 7 | — |
| `rholang` | 169 | 37 | 7 | — |
| `casper` | 210 | 38 | 3 | — |
| `node` | 147 | 9 | — | — |
| `qucalc` | 20 | — | — | — |
| `rspace-bench` | — | — | — | 10 |

**Measured line coverage: 81.30%** (`cargo llvm-cov --workspace --all-features`, 16868 of 90208
lines missed), which sets CI's floor to **79** — two points below the measurement, per the plan's rule
that the floor is a tripwire raised only *after* measuring. Raised three times: 73.68% ⇒ 71 (Stages
0–2), 79.69% ⇒ 77 (Stages 3–4: the legacy corpus, the tier sweep, the property sweep), 81.30% ⇒ 79
(the low-coverage sweep, which also found the printer and UPnP defects in AUDIT §16 C13/C14). The
per-file report is the place to look for the next tier's work, not this table:

```sh
cargo llvm-cov --workspace --all-features --summary-only   # then read the lowest percentages
```

At the time of writing the thinnest *behavioural* files (as opposed to error-enum `Display` arms,
which read as 1% because nothing formats them, and to trait/`mod` declaration files, which have no
behaviour to cover) are the tier table's open rows below — `comm/src/transport/grpc_transport_receiver.rs`
and `grpc_transport_client.rs`, `comm/src/discovery/*`, `node/src/api/grpc/*` and
`rholang/src/reporting_runtime.rs`. `rspace/src/history/history.rs` and
`comm/src/discovery/kademlia_handle_rpc.rs` read as thin because they *are* declarations plus
delegations; they are recorded in the removed-rows table rather than left to mislead this list.

The counts are a **floor, not a target**: the linter fails only when the register claims *more* than
the tree holds, so this table may lag as tests are added (it reports the drift) but can never
overstate coverage.

- **Property tests** — every law where randomized input is meaningful now has one, in a
  `property_tests.rs` per crate (`proptest` is a dev-dependency of `models`, `rspace`, `rholang`,
  `sdk`, `block-storage` and `casper`; it was already in `Cargo.lock`, so these are dev-dependency
  additions rather than new external dependencies). The matrix below is the whole 29-law oracle, with
  the evidence for each and the reason for the four laws that have none.

| Law | Layer | Randomized property | Other evidence |
|---|---|---|---|
| 1 | Rholang | `models/src/property_tests.rs` `law1_sorting_is_idempotent`, `law1_parallel_composition_sorts_commutatively` | Lean `Sort.lean`; `sorter.rs` unit tests |
| 2 | Rholang | `law2_canonical_equality_agrees_with_canonical_hashing`, `law2_sorting_a_sequence_depends_only_on_its_elements` | `Rho.lean` (≡ core) |
| 3 | Rholang | `rholang/src/property_tests.rs` `law3_substituting_a_closed_value_keeps_the_term_closed`, `law3_substitution_and_sorting_commute` | Lean `Subst.lean` |
| 4 | Rholang | `law4_the_same_program_reduces_to_the_same_state` | `casper/tests/determinism.rs`; Lean `Reduce.lean` |
| 5 | Rholang | `law5_a_pattern_that_binds_a_variable_twice_never_matches`, `law5_a_variable_bind_pattern_matches_any_datum`, `law5_a_ground_pattern_matches_only_itself` | Lean `Match.lean` |
| 6 | Rholang | `law6_a_closed_term_is_accepted_and_the_predicate_agrees` | Lean `Ty.lean` |
| 7 | RSpace | `rspace/src/property_tests.rs` `law7_join_hash_commutes` | Lean `Join.lean` |
| 8 | RSpace | `law8_comm_sorts_produces` | Lean `Comm.lean` |
| 9 | RSpace | `law9_disjoint_state_changes_commute` | Lean `Merge.lean` |
| 10 | RSpace | `law10_merkle_root_is_insertion_order_independent` | Lean `Merkle.lean` |
| 11 | RSpace | `law11_a_replayed_script_matches_its_recording` | `rspace/src/replay_rspace.rs`, `casper/tests/determinism.rs` |
| 12 | Rosette | **none — the law is orphaned** (the Rosette VM is out of scope; no Rust obligation) | — |
| 13 | Rosette | **none — orphaned** (as above) | — |
| 14 | Casper | `sdk/src/property_tests.rs` `law14_super_majority_is_strictly_more_than_two_thirds`, `law14_super_majority_is_monotone_in_support`, `law14_the_two_thirds_boundary_survives_past_the_f64_mantissa` | Lean `Stake.lean`/`Fringe.lean` |
| 15 | Casper | `block-storage/src/property_tests.rs` `law15_adding_blocks_only_grows_the_state` | Lean `Fringe.lean` |
| 16 | Casper | (no randomized form: the law is about a fixed hash construction) `models/src/casper/protocol/casper_message.rs` `law16_to_proto_*`; `casper/src/proto_util.rs` `hash_block_is_deterministic_and_ignores_sig` | Lean `Validate.lean` |
| 17 | Casper | `sdk/src/property_tests.rs` `law17_the_chosen_rejection_is_one_of_the_options`, `law17_the_chosen_rejection_minimizes_the_total_cost`, `law17_the_survivors_of_a_rejection_option_are_conflict_free`, `law17_deploys_without_conflicts_need_no_rejection` | Lean `Validate.lean` |
| 18 | Storage | `block-storage/src/property_tests.rs` `law18_a_contiguous_chain_validates`, `law18_a_chain_with_a_hole_in_the_middle_is_refused`, `law18_a_missing_lowest_height_is_not_a_hole`, `law18_a_validation_failed_tip_is_not_indexed`, `law18_the_state_does_not_depend_on_insertion_order`, `law18_the_empty_dag_and_a_lone_block_are_contiguous` | `models/src/fringe_data.rs` `law18_fringe_hash_is_order_independent`; Lean `Validate.lean` |
| 19 | Crypto | **none by design — the law is an axiom**; the crypto KATs (Blake2b256, `Blake2b512Random` split/merge, secp256k1 sign/verify, Curve25519) pin the implementations | Lean `Crypto/Random.lean` (axiom) |
| 20 | Scheduler | `law20_per_channel_path_order` | Lean `Scheduler.lean` (`queue_commit_path_ordered`) |
| 21 | Scheduler | `rholang/src/property_tests.rs` `law21_the_gate_scheduler_refines_the_sequential_reference` (16 randomized programs, gate vs sequential state hash + event log) | Lean `gate_exec_refines_apply`; `rholang/tests/execution.rs` `gate_and_sequential_state_hashes_match` |
| 22 | Scheduler | **exemption: harness-heavy** — `rholang/src/reduce.rs` `law22_the_next_step_closure_is_computable_at_dispatch` pins the structural half (one effect per term, no space I/O, arities 1–6); a full property over the effect stream was spiked at >150 lines and the integration suite already asserts on it | Lean `next_step_closure_computable` |
| 23 | Scheduler | **exemption: covered by Law 8's property** — the law's Rust realization *is* content-addressed candidate selection, which `law8_comm_sorts_produces` randomizes (`read_state_determines_outcome` is Lean-proven) | Lean `SchedulerOnchain.lean` |
| 24 | Scheduler | `law24_record_layer_and_validation` | Lean `SchedulerOnchain.lean` |
| 25 | Scheduler | `rholang/src/property_tests.rs` `law25_the_validated_relaxed_scheduler_refines_sequential` | Lean `validated_speculation_refines_apply`; `casper/tests/scheduler.rs` |
| 26 | Cross-shard | `casper/src/property_tests.rs` `law26_a_shard_id_is_accepted_exactly_when_nonempty_ascii`, `law26_invalid_shard_names_are_refused`, `law26_a_child_id_nests_under_its_parent` | Lean `shard_scope_deterministic`; `casper/src/conf.rs` unit tests |
| 27 | Cross-shard | `law27_an_abort_vote_prevents_a_later_commit`, `law27_and_law29_the_state_agrees_with_the_votes`, `law27_a_legless_record_cannot_commit` | Lean `txn_atomic`; `casper/tests/cross_shard_txn.rs`, `gateway_faults.rs` |
| 28 | Cross-shard | `rholang/src/native_state.rs` `law28_txn_prepare_rejects_overdraw_and_is_idempotent` (idempotency is a *unit* property here: the verbs are async and the state is the store) | Lean `leg_idempotent` |
| 29 | Cross-shard | `law29_a_terminal_record_never_changes_again` (+ `law27_and_law29_…`) | Lean `commit_record_deterministic`; `casper/src/gateway/ledger.rs` |

  Two laws therefore have **no** randomized evidence, both deliberately: 12 and 13 are orphaned with
  the Rosette VM, and 19 is an axiom whose implementations are KAT-pinned. Law 22 and 23 have a
  stated exemption rather than a silently empty cell.

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
- **Differential/golden** — four TSVs under `*/testdata/differential/`, each row now
  `id<TAB>value<TAB>provenance` with a `#` legend, and each guarded by a test that asserts **every
  row is consumed** and that the provenance is the declared one:

  | File | Rows | Provenance |
  |---|---|---|
  | `models/testdata/differential/wire.tsv` | 6 | `scala-rule-transcribed` (proto schema + the scalapb `bitSetToByteString` rule) |
  | `rspace/testdata/differential/stable_hash.tsv` | 7 | `scala-ground-truth` (`StableHashOracle`) |
  | `rspace/testdata/differential/scodec.tsv` | 14 | `scala-ground-truth` (`ScodecOracle`) |
  | `rholang/testdata/differential/execution.tsv` | 3 | `rust-regression-pinned` — **no Scala oracle exists** for the rholang pipeline |

  The distinction is load-bearing and now asserted (`the_execution_goldens_declare_their_provenance`):
  the rholang rows pin the port against its own past behaviour and are *not* evidence of Scala
  agreement. `legacy/scripts/gen-differential-goldens.sh` was rewritten to match `legacy/` (it wrote
  to a `crates/` directory that no longer exists), to **fail loudly without sbt** naming the
  prerequisite (`sbt` is absent here, so the script is checkable but not runnable in this
  environment), and to write the provenance column itself; it deliberately does not touch the
  rholang file. What it would add is a rholang `ExecutionOracle.scala`, which does not exist.
- **The legacy `.rho`/`.rhox` contract corpus** — **165 `.rho` + 1 `.rhox`** under `legacy/`, now
  driven by `rholang/tests/legacy_contracts.rs`. The one `.rhox`
  (`legacy/casper/src/main/resources/Pos.rhox`) is a Scala-side macro template, not a program.
  **75 of the 165 reduce cleanly**; the other 90 are listed in that test's `SKIPS` with a reason from
  a closed enum (56 in a dialect the BNFC grammar does not have, 17 needing a Scala-test-harness
  native, 8 templates, 4 meant to fail or needing a live context, 5 past the fuel bound), and the
  test asserts `attempted + skipped == 165` so the corpus cannot shrink silently. Running it found
  four defects in the port — see `AUDIT.md` §16.

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
| corpus | `rholang/tests/legacy_contracts.rs` | `legacy_contracts_parse_and_reduce` |
| corpus | `rholang/tests/legacy_contracts.rs` | `the_corpus_is_the_size_the_register_records` |
| corpus | `rholang/tests/legacy_contracts.rs` | `every_skip_entry_names_a_real_corpus_file` |
| syntax | `rholang/src/parser.rs` | `a_parenthesised_expression_is_a_group_not_a_one_element_tuple` |
| syntax | `rholang/src/parser.rs` | `a_group_may_not_contain_a_send_or_a_parallel` |
| syntax | `rholang/src/parser.rs` | `the_logical_connectives_lex_and_parse_in_their_grammar_spelling` |
| syntax | `rholang/src/reduce.rs` | `plus_plus_concatenates_byte_arrays_and_unions_maps_and_sets` |
| syntax | `rholang/src/reduce.rs` | `plus_and_minus_also_insert_into_and_delete_from_collections` |

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
| T1 | `casper/src/multi_parent_casper.rs` | `validate_error_separates_an_invalid_block_from_an_internal_failure` |
| T1 | `casper/src/blocks/block_processor.rs` | `the_parallel_validation_cap_is_never_zero` |
| T1 | `casper/src/runtime_replay.rs` | `get_number_with_rnd_rejects_a_malformed_number_channel` |
| T1 | `rspace/src/history/history_repository.rs` | `two_repositories_over_the_same_stores_agree_on_the_root` |
| T1 | `rspace/src/history/roots_store.rs` | `validate_and_set_refuses_an_unknown_root` |
| T1 | `rspace/src/history/root_repository.rs` | `an_unknown_root_is_an_error_and_does_not_move_the_current_root` |
| T1 | `rspace/src/replay_rspace.rs` | `a_rig_whose_comm_never_happens_is_reported` |
| T1 | `rspace/src/scheduled_space.rs` | `a_commit_whose_candidate_vanished_stores_instead_of_delivering` |
| T1 | `casper/src/engine/lfs_block_requester.rs` | `a_requested_block_with_a_forged_hash_is_rejected` |
| T1 | `models/src/validator.rs` | `try_from_rejects_a_wrong_length_and_names_both` |
| T1 | `rspace/src/history/history_action.rs` | `trimming_an_empty_key_panics` |
| T1 | `rspace/src/history/codecs.rs` | `decode_rejects_a_wrong_length` |
| T1 | `rholang/src/contract_call.rs` | `a_matched_reply_dispatches_at_the_child_path` |
| T2 | `rspace/src/merger/mod.rs` | `seq_diff_removes_the_first_occurrence_and_preserves_order` |
| T2 | `rspace/src/merger/event_log_merging_logic.rs` | `a_shared_destroyed_produce_conflicts_unless_it_is_mergeable_on_both_sides` |
| T2 | `comm/src/transport/grpc_transport_receiver.rs` | `a_sender_from_another_network_is_refused_and_not_dispatched` |
| T2 | `comm/src/transport/grpc_transport_client.rs` | `an_over_long_peer_id_is_a_parse_error` |
| T2 | `comm/src/discovery/kademlia_node_discovery.rs` | `each_lookup_targets_the_bit_flip_of_its_bucket` |
| T2 | `comm/src/discovery/grpc_kademlia_rpc.rs` | `a_peer_that_accepts_but_stalls_is_given_up_on` |
| T2 | `comm/src/discovery/grpc_kademlia_rpc_server.rs` | `a_ping_claiming_a_local_host_never_reaches_the_handler` |
| T2 | `comm/src/discovery/mod.rs` | `an_out_of_range_port_is_rejected_by_name` |
| T2 | `node/src/api/grpc/deploy_grpc_service_v1.rs` | `a_non_hex_report_hash_is_refused_by_name` |
| T2 | `rholang/src/reporting_runtime.rs` | `a_reduce_error_is_captured_in_the_result` |
| T3 | `rholang/src/storage_printer.rs` | `a_non_par_body_continuation_renders_an_empty_body` |
| T3 | `node/src/api/grpc/repl_grpc_service.rs` | `a_syntax_error_is_reported_without_evaluating` |
| T3 | `models/src/errors.rs` | `the_length_remap_keeps_got_and_expected_the_right_way_round` |

### The census behind these rows

The rows marked `—` came from a **second pass**: the first tier table was written from the completion
plan's prose list (~25 modules), but `spec/TEST-COVERAGE.md`'s own "absent rows" check only covered
files the table named. A census of the Stage 3 directories (`rspace/src/{merger,state,history}`,
`models/src`, `casper/src/{protocol,engine,api}`, `rholang/src`, `comm/src/{discovery,transport}`,
`node/src/{api/grpc,configuration,web}`) for files with **no test at all**, then a read of each to
separate "has branching/error logic" from "trait or data declaration", produced 19 more testable
modules — 17 above (five of them T1), plus the two closed in an earlier pass. Two of the earlier "untestable" calls were
**wrong** and were corrected here: `rspace/src/scheduled_space.rs` (5 async functions, including the
phase-two re-validation that makes relaxed scheduling sound) and
`casper/src/blocks/proposer/block_creator.rs` (a 120-line `create`). Both were mis-called by a grep
that anchored `fn` at column 0 and so missed every indented method — a reminder that "no test is
expected" deserves the same evidence as any other claim.
| T1 | `shared/src/rate_limiter.rs` | `zero_never_admits` |
| T1 | `rholang/src/scheduler.rs` | `effect_mode_rejects_an_unknown_name_and_names_the_alternatives` |
| T1 | `rholang/src/dispatch.rs` | `a_par_body_without_an_evaluator_is_a_bug_not_a_silent_no_op` |
| T1 | `models/src/par_ops.rs` | `par_concat_preserves_the_canonical_form` |
| T1 | `crypto/src/util/key_util.rs` | `the_private_key_file_is_owner_only` |
| T2 | `rspace/src/merger/state_change_merger.rs` | `an_empty_channel_change_is_reported_as_an_error` |
| T2 | `models/src/rholang.rs` | `an_accessor_rejects_another_types_par` |
| T2 | `models/src/runtime.rs` | `the_random_state_is_not_carried_through_serde` |
| T2 | `casper/src/protocol/comm_util.rs` | `send_with_retry_gives_up_after_the_bounded_number_of_attempts` |
| T2 | `casper/src/engine/node_launch.rs` | `the_connection_wait_blocks_while_there_are_no_peers` |
| T2 | `rholang/src/env.rs` | `an_unbound_index_resolves_to_none` |
| T2 | `node/src/configuration/hocon.rs` | `parse_size_rejects_what_it_cannot_parse` |
| T2 | `node/src/configuration/configuration.rs` | `shards_and_scalar_keys_are_mutually_exclusive` |
| T2 | `node/src/configuration/commandline/config_mapper.rs` | `an_explicit_shard_flag_is_visible_to_the_exclusivity_check` |
| T2 | `node/src/runtime/node_main.rs` | `generate_key_reprompts_on_a_mismatch_and_refuses_an_empty_password` |
| T2 | `comm/src/discovery/kademlia_store.rs` | `an_unknown_key_is_not_found` |
| T3 | `graphz/src/lib.rs` | `an_embedded_quote_is_not_escaped` |
| T3 | `node/src/web/status_info.rs` | `a_lone_node_reports_zero_peers_and_its_own_address` |

### Named exceptions (harness-bound)

Two things in the sections above resist a cargo test by construction, and are recorded here rather
than left as open rows or dropped silently. Both are *covered*, at the level above the file:

- **`casper/src/engine/node_launch.rs`'s LFS-syncing branch** (`apply` with an empty DAG and
  `standalone = false`) blocks on a `packet_rx` stream fed by a real peer's handshake, so a test
  would have to stand up a second node. The **genesis branch is covered end to end** — a real
  standalone validator is booted by `node/tests/deploy_block.rs` (`deploy_is_processed_into_a_block`)
  and `node/tests/node_api.rs`, which walks `create_store_broadcast_genesis` →
  `create_genesis_block_from_config` → `create_genesis_block` — and the syncing branch's failure arm
  is pinned at the unit below it (`request_finalized_fringe_is_an_error_when_there_is_no_bootstrap`).
  What remains unpinned is genuinely "a live peer is required", which the Docker devnet covers and
  which no PR gate can (`spec/TEST-COVERAGE.md`'s decision that nothing docker-based gates a PR).

- **`casper/src/blocks/proposer/block_creator.rs`'s `BlockCreator::create`** is reachable only through
  `Proposer` with a live `RuntimeManager`, DAG and deploy pool, so a unit test would rebuild the
  proposer's whole fixture. The path is covered end to end — `casper/tests/consensus.rs` and
  `node/tests/deploy_block.rs` (`deploy_is_processed_into_a_block`) both produce a real block through
  it — but the file's own arms (the slash/close seed indices computed from the *selected* deploy
  count, and the `u8::try_from` bound that rejects a block with more than 255 system deploys) are not
  pinned. An earlier version of this table claimed the file had "no function"; it has a 120-line
  one, which is how the claim was caught.

### Rows removed as untestable

Eight modules were listed as tier rows and then **removed**, because a row that can never close is
worse than no row: the behaviour they appear to name is either absent or tested where it actually
lives. Removing a row is the one edit to this table that the linter cannot check, so the reasons are
recorded here rather than only in the commit that did it.

| Module | Why no test is expected |
|---|---|
| `sdk/src/block.rs` | A trait declaration with no implementation in the file (tested through its implementors). |
| `sdk/src/dag.rs` | A module-declaration file (`pub mod data/merging/syntax`) with no items of its own; the three submodules carry 15 tests. |
| `rspace/src/history/history.rs`, `history_reader.rs` | Trait declarations (`async_trait` methods have default bodies in their *implementors*, not here) plus `empty_root_hash_value`, a one-line delegation to `radix_tree::empty_root_hash`. The interface is pinned through `HistoryRepository`, whose tests are in the tier table. |
| `models/src/proto.rs` | `include!` of the prost-generated wire types — no hand-written behaviour; the types are pinned where they are used (round trips and the `BTreeMap` determinism trap). |
| `rspace/src/checkpoint.rs` | Plain data carriers (`SoftCheckpoint` and friends); the checkpoint *behaviour* lives in `rspace/src/rspace.rs`. |
| `rholang/src/proc_ast.rs` | 266 lines of pure `enum`/`struct` declarations with no `impl` block: there is no behaviour, so a test could only assert that a value equals itself. The AST's behaviour is pinned where it is produced (`rholang/src/parser.rs`) and consumed (the normalizer + reducer). |
| `node/src/api/admin_web_api.rs` | A 13-line trait declaration (two method signatures, no default bodies) — the same class as `sdk/src/block.rs`. Its two callers (`propose`/`propose_result`) are tested at the HTTP layer. |

## Definition of done — where each item stands

| Item | State |
|---|---|
| 1. No deferred gap rows (the register's own marker); every ✅ names a findable test | **done** — the linter's hard mode passes with no deferred rows and no open tier rows |
| 2. The register linter recomputes counts, verifies named tests, fails on a bare tier module | **done** — `tools/audit-test-register.sh` (five checks; hard mode green) |
| 3. Every law has a property test or a recorded exemption | **done** — the matrix in Inventory; exempt: 12/13 (orphaned), 19 (axiom, KAT-pinned), 22/23 (stated reason), 28 (unit idempotency) |
| 4. `make test-unit` runs `--all-features` | **done** (with `test-integration`) |
| 5. The coverage floor raised after measuring (three times) | **done** — 73.68 ⇒ 71, 79.69 ⇒ 77, 81.30 ⇒ 79 |
| 6. `rspace-bench`'s Criterion groups are smoke-run | **done** — `make bench-smoke` |
| 7. `parsed + skipped == 165` for the legacy corpus, closed-enum skip reasons | **done** — `rholang/tests/legacy_contracts.rs` |
| 8. `gen-differential-goldens.sh` completes or fails loudly; every committed TSV row is consumed | **done** — provenance columns + a per-file drift guard; the script exits 2 without sbt |
| 9. The Stage 1 audit list appears verbatim in the register, each entry mapped to a test | **done** — the machine-checked claims table |

## Production changes made under this plan

The plan allows production changes but requires each to be listed explicitly and kept out of a
"test-only" commit, so that a reviewer sees every behaviour change rather than inferring it from a
green diff. These are all of them.

| Change | Why | Pinned by |
|---|---|---|
| `casper/src/gateway/ledger.rs::CoordRecord::record_vote` — a terminal record is absorbing | A late `Ready` could resurrect an aborted (compensated) transaction (AUDIT §15 C1) | `an_abort_is_absorbing`, `a_commit_is_absorbing` |
| `comm/src/transport/grpc_transport_receiver.rs` — `ConcurrencyLimits` + `serve_with_limits` | `MAX_CONCURRENT_DISPATCH` was a constant with no seam, so the DoS bound could not be tested at all | `a_full_dispatch_queue_is_rejected_and_recovers` |
| `crypto/src/util/key_util.rs::write_with_mode` — apply `0o600` with `set_permissions` after the write | `OpenOptions::mode` applies only at creation, so the R6 private-key fix was ineffective on a pre-existing key file (AUDIT §15 C8) | `the_private_key_file_is_owner_only` |
| `comm/src/transport/grpc_transport_receiver.rs::GrpcTransportReceiver::for_test` — a `#[cfg(test)]`-only constructor | The inbound guards (network-id rejection, dispatch bound, missing protocol) are otherwise reachable only over a socket, because production builds the receiver behind the TLS accept loop | `a_sender_from_another_network_is_refused_and_not_dispatched`, `a_full_dispatch_queue_is_refused` |
| `rholang/src/pretty_printer.rs::build_par` — the separator flag is per *group*, not per item | A group with two or more items printed `a |\n |\nb`, which is not parsable rholang (AUDIT §16 C13) | `printing_and_reparsing_is_the_identity` |
| `rholang/src/pretty_printer.rs::build_bundle` — print the `bundle` keyword with Scala's 8-column padding | Bundles printed as `0{ … }`, losing the keyword (AUDIT §16 C13) | `printing_and_reparsing_is_the_identity` |
| `comm/src/upnp/gateway.rs::split_authority` — a bracket-aware authority split, shared by the SSRF guard and the URL splitter | The guard read `[::1]` as the host `"["`, allowing a loopback discovery URL (AUDIT §16 C14) | `the_url_guard_allows_private_gateways_and_refuses_ssrf_targets` |

Everything else this plan has touched is a test, the register itself, the audit register, the linter,
or a `Makefile`/CI target.

## The Docker fuzzer (what it asserts, and why it is not a PR gate)

`tools/devnet-fuzz.py` drives a running Docker devnet (`tools/devnet.sh up`) and asserts four things,
one per `--mode`. `devnet.sh` grew an `--effect-scheduler` flag so the node's mode (Laws 20–25) can be
swept end to end;

| Mode | Assertion | In-process equivalent (what gates a PR) |
|---|---|---|
| `valid` (default) | random valid terms reduce without a 5xx, signed deploys keep 3 validators in lock-step | `casper/tests/determinism.rs`, `casper/tests/consensus.rs` |
| `malformed` | truncation, spliced delimiters, nesting past `MAX_PARSE_DEPTH`, huge integers and non-UTF-8 bodies are all 4xx, never 5xx, and every node still answers `/api/v1/status` afterwards | `rholang/src/parser.rs` `rejects_excessive_nesting_depth`; the R-series input guards |
| `scheduler` | the corpus reduces under the configured `--effect-scheduler` mode (including `relaxed`, which the *block* path must reject at runtime) | `casper/tests/scheduler.rs::block_paths_reject_relaxed_mode`, `rholang/tests/execution.rs` |
| `gateway` | a single-shard devnet reports exactly one shard and does **not** serve the txn routes — the "single-shard surface unchanged" claim at process level | `node/tests/gateway_routes.rs` (404s), `gateway.rs` |

`.github/workflows/devnet-fuzz.yml` runs `--mode all` nightly (`23 3 * * *`, off the hour) and on
`workflow_dispatch`, with the mode, scheduler and iteration count as inputs. It has **no
`pull_request` trigger by design**: it needs Docker, a built image and a live network, so a flaky
harness in the PR path would cost more than the coverage it adds. Nothing in it is required for a
merge.

**Not run in this environment**: the devnet modes need `docker` with a built `rnode:local` image (a
full Rust build in a container) and a live network, so they are written, syntactically checked
(`bash -n`, `ast.parse`) and dry-run where possible (`--dry-run` prints the malformed corpus) but not
executed here. The in-process suites above are what was run.

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
