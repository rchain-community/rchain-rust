# Test-coverage audit & gap analysis

This register records an adversarial audit of the Rust port's test coverage. It is the durable
companion to [`AUDIT.md`](AUDIT.md) (the *code* findings register): `AUDIT.md` records what the code
does wrong; this page records what the tests fail to catch.

**The register is machine-checked.** [`tools/audit-test-register.sh`](../tools/audit-test-register.sh)
recomputes every number below, verifies that every test the register names actually exists, requires
every source file to be tested or exempt with a reason class, and fails on any still-deferred row. It
exists because this page drifted: it claimed an `#[ignore]`d test that no longer existed, "110 legacy
contracts" when the tree held 165, and per-crate counts several releases stale. Run it after changing
this file:

```sh
tools/audit-test-register.sh               # hard: what `make check-register` and CI run
tools/audit-test-register.sh --deferred-ok # reports what hard mode would fail on (a burn-down view)
```

### The census: files, not per-crate totals

The counts below were the register's only measure for most of its life, and a per-crate total cannot
see a file that has *no* test. A census of every `*.rs` under `*/src` — **354 files** — shows what the
totals hid: **282 files carry at least one test, 72 are exempt** (the table in `## Exempt modules`),
and **no file is left without one or the other** — the linter's check 7 passes in hard mode, and CI and `make` now run it in hard mode (the `--deferred-ok` flag was dropped when the last file closed).

## Inventory

**1363 `#[test]`/`#[tokio::test]` unit functions + 102 integration tests** across 13 crates, with **6
laws** carrying a randomized property test and **10 benchmark functions** in 6 Criterion groups. Only
**3 of 12 crates have integration tests** (`rholang`, `casper`, `node`).

| Crate | Unit | Integration | Property (laws) | Bench |
|---|---|---|---|---|
| `sdk` | 46 | — | 2 | — |
| `shared` | 83 | — | — | — |
| `crypto` | 89 | — | — | — |
| `graphz` | 18 | — | — | — |
| `models` | 153 | — | 5 | — |
| `block-storage` | 40 | — | 3 | — |
| `comm` | 123 | — | — | — |
| `rspace` | 167 | — | 7 | — |
| `rholang` | 204 | 50 | 7 | — |
| `casper` | 239 | 47 | 3 | — |
| `node` | 178 | 12 | — | — |
| `qucalc` | 20 | — | — | — |
| `rspace-bench` | — | — | — | 10 |

**Measured line coverage: 84.07%** (`cargo llvm-cov --workspace --all-features`, 15530 of 97490
lines missed), which sets CI's floor to **82** — two points below the measurement, per the plan's rule
that the floor is a tripwire raised only *after* measuring. Raised four times: 73.68% ⇒ 71 (Stages
0–2), 79.69% ⇒ 77 (Stages 3–4: the legacy corpus, the tier sweep, the property sweep), 81.30% ⇒ 79
(the low-coverage sweep, which found the printer and UPnP defects in AUDIT §16 C13/C14), 84.07% ⇒ 82
(the census sweep: every source file tested or exempt, plus the node gRPC wire conversions, the
faucet budget, the history readers and the transport server's configuration path). The
per-file report is the place to look for the next tier's work, not this table:

```sh
cargo llvm-cov --workspace --all-features --summary-only   # then read the lowest percentages
```

At the time of writing the thinnest *behavioural* files (as opposed to error-enum `Display` arms,
which read as 1% because nothing formats them, and to trait/`mod` declaration files, which have no
behaviour to cover) are the ones with **one test over hundreds of lines** — `casper/src/runtime_manager.rs`
(1219 lines, 1 test), `node/src/api/grpc/tonic.rs` (972, 1 — 26 `*_to_wire`/`*_from_wire` conversions
with none), `casper/src/api/block_api_impl.rs` (837, 1) and `rholang/src/reduce.rs` (3405, 15). They
have a test, so the file-level census of item 10 does not reach them: they are item 11, the second
track, ordered by uncovered lines from the report above. `rspace/src/history/history.rs` and
`comm/src/discovery/kademlia_handle_rpc.rs` read as thin because they *are* declarations plus
delegations; they are recorded in `## Exempt modules` rather than left to mislead this list.

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
| 9 | RSpace | `law9_disjoint_state_changes_commute`, `law9_state_change_combine_is_associative` | Lean `Merge.lean` |
| 10 | RSpace | `law10_merkle_root_is_insertion_order_independent` | Lean `Merkle.lean` |
| 11 | RSpace | `law11_a_replayed_script_matches_its_recording` | `rspace/src/replay_rspace.rs`, `casper/tests/determinism.rs` |
| 12 | Rosette | **none — the law is orphaned** (the Rosette VM is out of scope; no Rust obligation) | — |
| 13 | Rosette | **none — orphaned** (as above) | — |
| 14 | Casper | `sdk/src/property_tests.rs` `law14_super_majority_is_strictly_more_than_two_thirds`, `law14_super_majority_is_monotone_in_support`, `law14_the_two_thirds_boundary_survives_past_the_f64_mantissa` | Lean `Stake.lean`/`Fringe.lean` |
| 15 | Casper | `block-storage/src/property_tests.rs` `law15_adding_blocks_only_grows_the_state` | Lean `Fringe.lean` |
| 16 | Casper | (no randomized form: the law is about a fixed hash construction) `models/src/casper/protocol/casper_message.rs` `law16_to_proto_*`; `casper/src/proto_util.rs` `hash_block_is_deterministic_and_ignores_sig` | Lean `Validate.lean` |
| 17 | Casper | `sdk/src/property_tests.rs` `law17_the_chosen_rejection_is_one_of_the_options`, `law17_the_chosen_rejection_minimizes_the_total_cost`, `law17_the_survivors_of_a_rejection_option_are_conflict_free`, `law17_deploys_without_conflicts_need_no_rejection`; `rspace/src/merger/event_log_index.rs` `combining_refuses_a_diff_that_leaves_i64` (the checked accumulation, AUDIT C41) | Lean `Validate.lean` |
| 18 | Storage | `block-storage/src/property_tests.rs` `law18_a_contiguous_chain_validates`, `law18_a_chain_with_a_hole_in_the_middle_is_refused`, `law18_a_missing_lowest_height_is_not_a_hole`, `law18_a_validation_failed_tip_is_not_indexed`, `law18_the_state_does_not_depend_on_insertion_order`, `law18_the_empty_dag_and_a_lone_block_are_contiguous` | `models/src/fringe_data.rs` `law18_fringe_hash_is_order_independent`; Lean `Validate.lean` |
| 19 | Crypto | **none by design — the law is an axiom**; the crypto KATs (Blake2b256, `Blake2b512Random` split/merge, secp256k1 sign/verify, Curve25519) pin the implementations | Lean `Crypto/Random.lean` (axiom) |
| 20 | Scheduler | `law20_per_channel_path_order` | Lean `Scheduler.lean` (`queue_commit_path_ordered`) |
| 21 | Scheduler | `rholang/src/property_tests.rs` `law21_the_gate_scheduler_refines_the_sequential_reference` (16 randomized programs, gate vs sequential state hash + event log) | Lean `gate_exec_refines_apply`; `rholang/tests/execution.rs` `gate_and_sequential_state_hashes_match` |
| 22 | Scheduler | **exemption: harness-heavy** — `rholang/src/reduce.rs` `law22_the_next_step_closure_is_computable_at_dispatch` pins the structural half (one effect per term, no space I/O, arities 1–6); a full property over the effect stream was spiked at >150 lines and the integration suite already asserts on it | Lean `next_step_closure_computable` |
| 23 | Scheduler | `rspace/src/property_tests.rs` `law23_read_state_determines_outcome` — permutes `Comm::apply`'s candidates and asserts the whole event is unchanged, so the commit is a function of the state *read* and not of the order it arrived in. (Was an exemption: "covered by Law 8's property". Law 8 checks the produces come out sorted — the mechanism; this checks the property the mechanism exists for, and it is the one that fails if the sort is removed. Falsified once by removing it.) | Lean `SchedulerOnchain.lean` `read_state_determines_outcome` |
| 24 | Scheduler | `law24_record_layer_and_validation` | Lean `SchedulerOnchain.lean` |
| 25 | Scheduler | `rholang/src/property_tests.rs` `law25_the_validated_relaxed_scheduler_refines_sequential` | Lean `validated_speculation_refines_apply`; `casper/tests/scheduler.rs` |
| 26 | Cross-shard | `casper/src/property_tests.rs` `law26_a_shard_id_is_accepted_exactly_when_nonempty_ascii`, `law26_invalid_shard_names_are_refused`, `law26_a_child_id_nests_under_its_parent` | Lean `shard_scope_deterministic`; `casper/src/conf.rs` unit tests |
| 27 | Cross-shard | `law27_an_abort_vote_prevents_a_later_commit`, `law27_and_law29_the_state_agrees_with_the_votes`, `law27_a_legless_record_cannot_commit` | Lean `txn_atomic`; `casper/tests/cross_shard_txn.rs`, `gateway_faults.rs` |
| 28 | Cross-shard | `rholang/src/native_state.rs` `law28_txn_prepare_rejects_overdraw_and_is_idempotent` (idempotency is a *unit* property here: the verbs are async and the state is the store) | Lean `leg_idempotent` + the ledger verbs (`txnPrepare_idempotent`/`txnCommit_idempotent`/`txnAbort_idempotent`, the fences) |
| 29 | Cross-shard | `law29_a_terminal_record_never_changes_again` (+ `law27_and_law29_…`) | Lean `commit_record_deterministic`; `casper/src/gateway/ledger.rs` |

The surface the silent defects live in (AUDIT C9-C26) is rows **30-43** of `spec/INVENTORY.md`. Each
row's evidence is a Lean declaration, a corpus emitted from it, and a Rust consumer that runs the same
cases through the node — not a randomized property, because what these laws pin is a *shape* rather
than an algebraic identity. Explicit rather than implied, per the rule that no cell is silently empty:

| # | Law | Lean | Corpus + Rust consumer | Status |
|---|-----|------|------------------------|--------|
| 34 | A *value* position is normalized against an empty `par` — only a statement continuation inherits (AUDIT C21's rule) | `Surface.lean` `normalizeAt` (the accumulator-less desugaring) + `Corpus.c21Holds` (the desugared `Match`'s target **is** the condition, decided through `cmpPar`; `c21IsProbe` rejects a case that is its own condition) | `spec/conformance/c21.tsv` ← `Corpus.c21Cases` · `rholang/tests/lean_c21_corpus.rs` | **checked on the shapes C21 broke** (5 cases, including the explicit-`match` control). The universal form is the named boundary: the model threads no accumulator, so the rule is true of it by construction and the corpus ties the *node* rather than proving the model |
| 35 | Concreteness is sound (`connective_used` ⟺ connective/free var/wildcard/remainder) | `Par.lean` `connectiveUsed` (+ `Sort.cmpOptionVar`, `Ty.closedRemainder`) | `spec/conformance/flags.tsv` ← `Corpus.flagCases_decide` · `rholang/tests/lean_normalize_corpus.rs` | **checked** |
| 37 | Match soundness and completeness (law 5 strengthened) | `Match.lean` `spatialMatch*` defined; `spatialMatch_implies_linear` proven, plus the aggregation model (`aggregateUpdates_rejects_double_bind`, `freeMapMerge_overwrites`) | `spec/conformance/match.tsv` ← `Corpus.matchCases_decide` · `rholang/tests/lean_match_corpus.rs` | **checked** |
| 38 | Silence is specified (no match ⇒ no step, no error) | `Silence.lean` `ReduceP` (the contract rule carries the match) + `takesStep`; `takesStep_iff_reduces` owed | `spec/conformance/silence.tsv` ← `Corpus.silenceCases_decide` · `rholang/tests/lean_silence_corpus.rs` | **checked** |
| 39 | Reply shapes: every urn's reply arity/shape is its `spec/API-SCHEMA.md` row | `Protocol.lean` `replyCatalog` + `replyCatalog_decide` (unique namespaced urns, kind agrees with slots, arity agrees with the arguments as written) | `spec/conformance/protocol.tsv` ← `replyCatalog` · `rholang/tests/lean_protocol_corpus.rs` · the doc tie in `tools/check-lean-conformance.sh` | **checked** for the rows the surface language can spell (the `ByteArray`-argument urns are named in `spec/INVENTORY.md` row 39) |
| 40 | Protocol agreement: a call has an accepting receive at the target's arity | `Silence.lean` `stepsInBinds` reads the arity (as many patterns as data), `receiveParPs` | `spec/conformance/silence.tsv` cases 7-12 ← `Corpus.silenceCases_decide` · `rholang/tests/lean_silence_corpus.rs` | **checked** (case 10 is C22 item 2) |
| 41 | Channel balance: a replicable reader restores what it consumes | `Store.lean` `readStore` / `storeSurvives` (a datum back *and* `takesStep`), `replicatedRead` | `spec/conformance/store.tsv` ← `Corpus.storeCases_decide` · `rholang/tests/lean_store_corpus.rs` | **checked** (the installed content: `casper/tests/genesis_registry.rs`'s `a_read_does_not_destroy_the_inbox`, and — measured on the contract that was unreachable — `a_fresh_chain_answers_one_group_creation` / `..._two_group_creations_in_one_deploy`, which is AUDIT C25's fix; the store is restored on both branches, so the `:49`-`:65` window is latency, not loss) |
| 42 | rho-value JSON: the envelope rule and `rho_expr_to_par (expr_from_par p) = p` | `Json.lean` `parToJE` (the envelope), `jeToPar`, `render`; `decode_encode` stated over `flatPar` and **owed** | `spec/conformance/json.tsv` ← `Corpus.jsonCases_decide` · `node/tests/lean_json_corpus.rs` | **checked** at the wire level (the model's `render` is the expectation; the node's JSON and its own round-trip are the second party) |
| 43 | The endpoint envelope equals the served schema (camelCase keys, tags) | `Envelope.lean` `envelopeCatalog` + `envelopeCatalog_decide` (no underscore in a key, tags capitalized, names unique) | `spec/conformance/envelope.tsv` ← `envelopeCatalog` · `node/tests/lean_envelope_corpus.rs` (the DTOs *and* the served OpenAPI document) | **checked** for keys and tags (the value types and the document's coverage are the named boundary) |
| 32 | Lexical determinism for the operator surface (maximal munch; each spelling one way) | `Lex.lean` `lexemes` + `lexemes_decide` (distinct punctuation spellings; each its own longest match) | `spec/conformance/lex.tsv` ← `lexemes` · `node/tests/lean_lex_corpus.rs` (each row's sample run through the node) | **checked for the operator surface** (the rest of the law is the named boundary) |
| 30, 31, 33, 36 | grammar, print/parse round-trip | *(named per row in `spec/INVENTORY.md`; not yet defined)* | *(none yet)* | **open** — the next slices |

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
| G3 | `rholang/src/native_state.rs` | `the_phlo_charge_funds_the_pot_and_the_refund_returns_the_surplus` |
| G3 | `rholang/src/native_state.rs` | `the_epoch_gate_does_nothing_off_a_boundary` |
| G3 | `rholang/src/native_state.rs` | `an_epoch_splits_the_pot_and_keeps_the_dust` |
| G3 | `rholang/src/native_state.rs` | `a_released_withdrawal_pays_the_bond_plus_the_committed_rewards` |
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
| C20 | `rholang/src/matcher/spatial_matcher.rs` | `a_map_pattern_may_name_fewer_entries_than_the_map_has` |
| C20 | `rholang/src/matcher/spatial_matcher.rs` | `a_named_map_remainder_captures_the_unnamed_entries` |
| C20 | `rholang/src/matcher/spatial_matcher.rs` | `a_set_pattern_may_name_fewer_members_than_the_set_has` |
| C20 | `rholang/src/matcher/spatial_matcher.rs` | `list_remainders_stay_positional` |
| C20 | `rholang/tests/system_process_conformance.rs` | `collection_patterns_match_a_subset_of_their_collection` |
| genesis | `casper/tests/genesis_registry.rs` | `a_fresh_chain_resolves_and_can_call_every_seeded_shorthand` |
| genesis | `casper/tests/genesis_registry.rs` | `the_seeded_registry_is_identical_across_fresh_genesis_ceremonies` |
| genesis | `casper/src/genesis/standard_deploys.rs` | `aliased_contract_uris_are_pinned` |
| genesis | `casper/src/genesis/standard_deploys.rs` | `every_genesis_alias_has_a_source` |
| genesis | `casper/src/genesis/standard_deploys.rs` | `the_make_mint_epilogue_is_adapted` |
| genesis | `casper/src/genesis/rgov.rs` | `every_rendered_contract_parses_and_normalizes` |
| genesis | `casper/src/genesis/rgov.rs` | `the_class_registration_keeps_upstreams_shape` |
| genesis | `casper/src/genesis/rgov.rs` | `the_published_keys_are_constants` |
| genesis | `casper/src/genesis/rgov.rs` | `the_governance_terms_are_signed_by_the_ceremony_key` |
| genesis | `casper/src/genesis/rgov.rs` | `the_extra_slots_term_writes_the_names_the_wallet_asks_for` |
| genesis | `casper/tests/genesis_registry.rs` | `a_fresh_chain_serves_the_wallets_new_inbox_handshake` |
| genesis | `casper/src/genesis/rgov.rs` | `the_member_directory_imports_the_installed_contracts` |
| genesis | `casper/src/genesis/rgov.rs` | `the_master_directory_template_carries_the_installed_uris` |
| genesis | `casper/src/genesis/rgov.rs` | `the_deploy_time_self_tests_are_removed` |
| genesis | `casper/tests/genesis_registry.rs` | `a_fresh_chain_installs_the_rgov_contracts_and_they_answer` |
| genesis | `casper/tests/genesis_registry.rs` | `installing_make_mint_before_its_dependency_is_caught_by_the_genesis_check` |
| genesis | `casper/src/genesis/mod.rs` | `blessed_terms_are_ordered_by_dependency` |
| syntax | `rholang/src/reduce.rs` | `plus_and_minus_also_insert_into_and_delete_from_collections` |

## Gap analysis (severity-ordered)

For each gap: **code location** → **current test state** → **the seam a regression test attaches to**.
**No gap row is deferred any more.** All four ⏸ rows at the time of writing (G2's semaphore, G5, G7,
G12) were closed by the completion plan's Stage 2 — and G6's history internals were moved into the
risk-tier table below, where an open module is a defect the linter reports rather than a note. A `⏸`
now appears only if a *new* gap is deliberately left open, and `tools/audit-test-register.sh` fails on
every one of them in hard mode — the default that `make check-register` and CI run. `--deferred-ok`
is now only a burn-down *view*: it tolerates the three work-in-flight classes (deferred gap rows,
tier rows with no test, and source files with neither a test nor an exemption row) and prints what
hard mode would fail on, so that a half-finished sweep is legible instead of invisible.

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
  min/max + funds, deferred activation at the boundary, `withdraw` staging + quarantine payout via
  `close_block`, `slash`/`untrust` confiscation to the Coop vault, active-set top-N selection, genesis
  install, `pre_charge` incl. insufficient funds, and the revVault deposit/transfer paths. ✅ The
  staking vault and its flow: `bond_moves_the_stake_into_the_staking_vault`,
  `the_phlo_charge_funds_the_pot_and_the_refund_returns_the_surplus` (the charge in, the surplus back
  out, the burned phlo left as the pot) and
  `a_short_staking_vault_fails_the_transfer_rather_than_half_paying` — with the *conservation* of
  total REV across bond / slash / charge / refund / withdrawal asserted by the `total_rev` helper in
  every one of them. ✅ The epoch: `the_epoch_gate_does_nothing_off_a_boundary` (law 44 — nothing
  changes off a boundary, and a bond becomes a validator only *at* one),
  `an_epoch_splits_the_pot_and_keeps_the_dust` (laws 45/46 — the model's `the_dust_is_real` case, six
  distributed of ten, read back from the implementation),
  `a_released_withdrawal_pays_the_bond_plus_the_committed_rewards` (law 47's payoff, including the
  order that lets a validator earn in the epoch it leaves),
  `an_epoch_with_a_zero_normaliser_pays_nothing`, and
  `a_drafted_vault_floors_the_pot_instead_of_wrapping`. *Seam:* `NativeSystemState` over
  `InMemNativeStore::empty()`, plus the end-to-end deploy → boundary → active-set → replay path in
  `casper/tests/consensus.rs` (which now passes the CloseBlock system deploy a real block carries).

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
| G3 PoS mutations | ✅ lifecycle + end-to-end bond→active-set→replay + the staking vault's flow (charge in, refund out, conservation) ; reward distribution **out of scope** (feature) |
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

## Exempt modules

The linter's seventh check enumerates every `*/src/**/*.rs` (excluding `legacy/`, `spec/`, `docs/`,
`target/`) and requires each file to either contain a test attribute or appear in the table below with a
**reason class**. A row is a claim like any other: the class must be one of `data` (declarations with no
behaviour), `generated` (`include!`d prost/tonic output), `dev-tool` (an entry-point binary with no logic
of its own), `peer-bound` (needs a live peer) or `harness-bound` (needs the whole node as a fixture);
the file must exist; and a file that *gains* a test while
still holding a row **fails** the check. The table burns down — it is not a permanent allowlist. A
`peer-bound` row must name its covering test as `path::test`, and the linter verifies that the test
exists, like every other named test in this register.

Some of these rows were listed in an earlier version of the tier table and then **removed**, because a
row that can never close is worse than no row: the behaviour they appear to name is either absent or
tested where it actually lives. Removing a row is the one edit to this register the linter cannot check,
so the reason is recorded here rather than only in the commit that did it.

| Class | Module | Why no test is expected | Covering test |
|---|---|---|---|
| `data` | `casper/src/lib.rs`, `casper/src/api/mod.rs`, `casper/src/blocks/mod.rs`, `casper/src/blocks/proposer/mod.rs`, `casper/src/protocol/mod.rs` | Module-declaration shims (`pub mod` re-exports plus `lib.rs`'s four `pub use conf::{…}` re-exports); the submodules carry the tests. | — |
| `harness-bound` | `casper/src/blocks/proposer/block_creator.rs` | `BlockCreator::create` takes a live `RuntimeManager` — the interpreter over an RSpace over a history over a store — so a unit test would rebuild the proposer's whole fixture. The path is covered end to end by `casper/tests/consensus.rs`'s `ESCROW`-driven block production and `node/tests/deploy_block.rs`'s `deploy_is_processed_into_a_block`, both of which produce a real block through it. What is **not** pinned at the unit level: the slash/close seed indices (computed from the *selected* deploy count, not the requested id list) and the `u8::try_from` bound that rejects a block with more than 255 system deploys. Recorded rather than dropped — see "Named exceptions (harness-bound)" above for the full note. | `node/tests/deploy_block.rs::deploy_is_processed_into_a_block` |
| `data` | `sdk/src/block.rs` | A trait declaration with no implementation in the file (tested through its implementors). | — |
| `data` | `sdk/src/dag.rs` | A module-declaration file (`pub mod data/merging/syntax`) with no items of its own; the three submodules carry 15 tests. | — |
| `data` | `sdk/src/lib.rs` | Module-declaration shim (`pub mod` re-exports only); the submodules carry the tests. | — |
| `data` | `shared/src/lib.rs` | Module-declaration shim (`pub mod` re-exports only); the submodules carry the tests. | — |
| `data` | `shared/src/state.rs` | Trait declarations (`TrieExporter`/`TrieImporter`/`StateManager`, no default bodies) and one data carrier; the interface is pinned through its implementors. | — |
| `data` | `crypto/src/lib.rs` | Module-declaration shim (`pub mod` re-exports only); the submodules carry the tests. | — |
| `data` | `crypto/src/encryption/mod.rs` | Module-declaration shim (`pub mod` re-exports only); the submodules carry the tests. | — |
| `data` | `crypto/src/hash/mod.rs` | Module-declaration shim (`pub mod` re-exports only); the submodules carry the tests. | — |
| `data` | `crypto/src/signatures/mod.rs` | Module-declaration shim (`pub mod` re-exports only); the submodules carry the tests. | — |
| `data` | `crypto/src/util/mod.rs` | Module-declaration shim (`pub mod` re-exports only); the submodules carry the tests. | — |
| `data` | `crypto/src/util/secure_random_util.rs` | A single re-export of the OS CSPRNG (`pub use rand::rngs::OsRng;`) — no code of its own; the RNG is exercised through the key generation it feeds. | — |
| `data` | `models/src/lib.rs` | Module-declaration shim (`pub mod` re-exports only); the submodules carry the tests. | — |
| `data` | `models/src/proto.rs` | `include!` of the prost-generated wire types — no hand-written behaviour; the types are pinned where they are used (round trips and the `BTreeMap` determinism trap). | — |
| `data` | `models/src/block/mod.rs` | Module-declaration shim (`pub mod` re-exports only); the submodules carry the tests. | — |
| `data` | `models/src/block_version.rs` | Two consensus constants (`CURRENT`, `SUPPORTED`) and no function; they are read through the block-version checks that consume them. | — |
| `data` | `models/src/casper/mod.rs` | Module-declaration shim (`pub mod` re-exports only); the submodules carry the tests. | — |
| `data` | `models/src/casper/protocol/mod.rs` | Module-declaration shim (`pub mod` re-exports only); the submodules carry the tests. | — |
| `data` | `models/src/casper/protocol/propose_service.rs` | Plain data carriers (`struct`, no `impl` block): two query messages with derive-only behaviour. | — |
| `data` | `models/src/casper/protocol/report.rs` | Plain data carriers (`struct`/`enum`, no `impl` block): field declarations with derive-only behaviour, exercised where they are built and read. | — |
| `data` | `models/src/comm/mod.rs` | Module-declaration shim (`pub mod` re-exports only); the submodules carry the tests. | — |
| `data` | `models/src/comm/discovery/mod.rs` | A re-export of the prost/tonic-generated Kademlia wire types (`pub use crate::proto::discovery::*`) — those are generated, and these are pinned where `comm` encodes and decodes them. | — |
| `data` | `comm/src/lib.rs` | Module-declaration shim (`pub mod` re-exports only); the submodules carry the tests. | — |
| `data` | `comm/src/rp/mod.rs` | Module-declaration shim (`pub mod` re-exports only); the submodules carry the tests. | — |
| `data` | `comm/src/rp/rp_conf.rs` | Plain data carriers (`struct`, no `impl`): field declarations with derive-only behaviour, exercised where they are built and read. | — |
| `data` | `comm/src/transport/mod.rs` | Module-declaration shim (`pub mod` re-exports only); the submodules carry the tests. | — |
| `data` | `comm/src/transport/buffer/mod.rs` | Module-declaration shim (`pub mod` re-exports only); the submodules carry the tests. | — |
| `data` | `comm/src/transport/tls_conf.rs` | Plain data carriers (`struct`, no `impl`): field declarations with derive-only behaviour, exercised where they are built and read. | — |
| `data` | `comm/src/transport/transport_layer.rs`, `comm/src/discovery/kademlia_rpc.rs` | Trait declarations with no default bodies (`TransportLayer` 3 signatures, `KademliaRpc` 2). The contracts are pinned through their implementors — `GrpcTransportClient` and the recording `TransportLayer` in `transport_layer_syntax.rs`'s tests; `GrpcKademliaRpc` and the `StubRpc` in `node_discovery.rs`'s tests. | — |
| `data` | `comm/src/transport/messages.rs` | Two message carriers (`Send`, `StreamMessage`) and the empty marker trait they implement: no behaviour of its own, exercised where the transport server delivers them. | — |
| `data` | `node/src/lib.rs` | Module-declaration shim (`pub mod` re-exports only); the submodules carry the tests. | — |
| `data` | `node/src/api/mod.rs` | Module-declaration shim (`pub mod` re-exports only); the submodules carry the tests. | — |
| `data` | `node/src/api/admin_web_api.rs` | A 13-line trait declaration (two method signatures, no default bodies) — the same class as `sdk/src/block.rs`. Its two callers (`propose`/`propose_result`) are tested at the HTTP layer. | — |
| `data` | `node/src/api/web_api.rs` | A trait declaration (16 signatures, no default bodies): the web API contract. It is pinned through its implementor `WebApiImpl` — whose tests (this sweep) cover the faucet budget, the deploy-id validation and the pooled-deploy ordering — and through the HTTP layer that calls it. | — |
| `harness-bound` | `node/src/instances/proposer_instance.rs` | `create` takes a `Proposer`, which needs a live `MultiParentCasper` and its runtime — the whole node again. Its behaviour (the semaphore that turns a concurrent propose into `ProposerResult::Empty`, and the re-enqueue trigger) is exercised wherever a real node proposes: `node/tests/deploy_block.rs` boots through `node_runtime`, which builds this stream. Not pinned at the unit level: that a *concurrent* second propose answers `Empty` without blocking. | `node/tests/deploy_block.rs::deploy_is_processed_into_a_block` |
| `dev-tool` | `qucalc/src/main.rs` | An example binary: it resolves a census path (argument, then `QUCALC_CENSUS`, then the default), calls `Census::load` and prints a report. The logic it drives is the `qucalc` library's `Census`/`fold`, which has its own tests; the binary needs a census file and a stdout to observe, and refactoring it into a testable function was the plan's explicitly deferred alternative. | — |
| `data` | `node/src/configuration/mod.rs` | Module-declaration shim (`pub mod` re-exports only); the submodules carry the tests. | — |
| `data` | `node/src/configuration/commandline/mod.rs` | Module-declaration shim (`pub mod` re-exports only); the submodules carry the tests. | — |
| `data` | `node/src/configuration/model.rs` | Plain data carriers (`struct`/`enum`, no `impl` block): HOCON field declarations with derive-only behaviour, exercised where the configuration is parsed and read. | — |
| `data` | `node/src/diagnostics/mod.rs` | Module-declaration shim (`pub mod` re-exports only); the submodules carry the tests. | — |
| `data` | `node/src/effects/mod.rs` | Module-declaration shim (`pub mod` re-exports only); the submodules carry the tests. | — |
| `data` | `node/src/instances/mod.rs` | Module-declaration shim (`pub mod` re-exports only); the submodules carry the tests. | — |
| `data` | `node/src/runtime/mod.rs` | Module-declaration shim (`pub mod` re-exports only); the submodules carry the tests. | — |
| `data` | `node/src/state/mod.rs` | Module-declaration shim (`pub mod` re-exports only); the submodules carry the tests. | — |
| `data` | `node/src/web/mod.rs` | Module-declaration shim (`pub mod` re-exports only); the submodules carry the tests. | — |
| `data` | `block-storage/src/lib.rs` | Module-declaration shim (`pub mod` re-exports only); the submodules carry the tests. | — |
| `data` | `block-storage/src/dag/mod.rs` | Module-declaration shim (`pub mod` re-exports only); the submodules carry the tests. | — |
| `data` | `block-storage/src/dag/dag_storage.rs` | A trait declaration (`BlockDagStorage`, seven signatures, no default bodies) plus the `DeployId` alias: there is no behaviour in the file. The contract is pinned through its implementors — `StubDagStorage` in `syntax.rs`'s tests (the `lookup`/`insert` arms) and casper's `BlockDagKeyValueStorage`, whose tests are in the tier table. | — |
| `data` | `block-storage/src/test_support.rs` | A `#[cfg(test)]`-only fixture module: value constructors, no behaviour, and absent from a release build (declared in the production-changes table when it was added). | — |
| `data` | `rholang/src/lib.rs` | Module-declaration shim (`pub mod` re-exports only); the submodules carry the tests. | — |
| `data` | `rholang/src/util/mod.rs` | Module-declaration shim (`pub mod` re-exports only); the submodules carry the tests. | — |
| `data` | `rholang/src/matcher/mod.rs` | Module-declaration shim (`pub mod` re-exports only); the submodules carry the tests. | — |
| `data` | `rholang/src/proc_ast.rs` | 266 lines of pure `enum`/`struct` declarations with no `impl` block: there is no behaviour, so a test could only assert that a value equals itself. The AST's behaviour is pinned where it is produced (`rholang/src/parser.rs`) and consumed (the normalizer + reducer). | — |
| `data` | `rspace/src/lib.rs` | Module-declaration shim (`pub mod` re-exports only); the submodules carry the tests. | — |
| `data` | `rspace/src/checkpoint.rs` | Plain data carriers (`SoftCheckpoint` and friends); the checkpoint *behaviour* lives in `rspace/src/rspace.rs`. | — |
| `data` | `rspace/src/concurrent/mod.rs` | Module-declaration shim (`pub mod` re-exports only); the submodules carry the tests. | — |
| `data` | `rspace/src/hashing/mod.rs` | Module-declaration shim (`pub mod` re-exports only); the submodules carry the tests. | — |
| `data` | `rspace/src/history/mod.rs` | Module-declaration shim (`pub mod` re-exports only); the submodules carry the tests. | — |
| `data` | `rspace/src/history/history.rs`, `rspace/src/history/history_reader.rs` | Trait declarations (`async_trait` methods have default bodies in their *implementors*, not here) plus `empty_root_hash_value`, a one-line delegation to `radix_tree::empty_root_hash`. The interface is pinned through `HistoryRepository`, whose tests are in the tier table. | — |
| `data` | `rspace/src/history/instances/mod.rs` | Module-declaration shim (`pub mod` re-exports only); the submodules carry the tests. | — |
| `data` | `rspace/src/i_space.rs`, `rspace/src/i_replay_space.rs`, `rspace/src/tuple_space.rs`, `rspace/src/match_.rs` | Trait declarations with no default bodies (`ISpace` 9 signatures, `IReplaySpace` 3, `ITupleSpace` 6, `Match` 1): there is no behaviour in the files. The contracts are pinned through their implementors — `rspace.rs`, `replay_rspace.rs`, `reporting_rspace.rs`, `rspace_history_reader_impl.rs` — whose tests are in the tier table and in this sweep. | — |
| `data` | `rspace/src/hot_store_action.rs` | A plain data carrier (`enum`, no `impl`): there is no behaviour to pin; it is exercised where it is produced and consumed. | — |
| `data` | `rspace/src/hot_store_trie_action.rs` | A plain data carrier (`enum`, no `impl`): there is no behaviour to pin; it is exercised where it is produced and consumed. | — |
| `data` | `rspace/src/serializers/mod.rs` | Module-declaration shim (`pub mod` re-exports only); the submodules carry the tests. | — |
| `data` | `rspace/src/trace/mod.rs` | Module-declaration shim (`pub mod` re-exports only); the submodules carry the tests. | — |

## Definition of done — where each item stands

| Item | State |
|---|---|
| 1. No deferred gap rows (the register's own marker); every ✅ names a findable test | **done** — the linter's hard mode passes with no deferred rows and no open tier rows |
| 2. The register linter recomputes counts, verifies named tests, fails on a bare tier module | **done** — `tools/audit-test-register.sh` (seven checks; hard mode green) |
| 3. Every law has a property test or a recorded exemption | **done** — the matrix in Inventory; exempt: 12/13 (orphaned), 19 (axiom, KAT-pinned), 22/23 (stated reason), 28 (unit idempotency) |
| 4. `make test-unit` runs `--all-features` | **done** (with `test-integration`) |
| 5. The coverage floor raised after measuring (four times) | **done** — 73.68 ⇒ 71, 79.69 ⇒ 77, 81.30 ⇒ 79, 84.07 ⇒ 82 |
| 6. `rspace-bench`'s Criterion groups are smoke-run | **done** — `make bench-smoke` |
| 7. `parsed + skipped == 165` for the legacy corpus, closed-enum skip reasons | **done** — `rholang/tests/legacy_contracts.rs` |
| 8. `gen-differential-goldens.sh` completes or fails loudly; every committed TSV row is consumed | **done** — provenance columns + a per-file drift guard; the script exits 2 without sbt |
| 9. The Stage 1 audit list appears verbatim in the register, each entry mapped to a test | **done** — the machine-checked claims table |
| 10. Every `*/src/**/*.rs` is tested or exempt with a reason class (linter check 7) | **done** — the census of all 354 source files closes: **282 tested, 72 exempt, 0 unaccounted**. Hard mode is the gate (`make check-register`, CI's coverage job) and `--deferred-ok` is gone, so a new source file with no test fails the build rather than joining a list nobody reads. The earlier tier table could not have caught this: it was checked against itself, not against the tree. |
| 11. The thin-coverage files' unreached failure arms are pinned | **not started** — the second track, ordered by uncovered lines. First `node/src/api/grpc/tonic.rs` (972 lines, 1 test, 26 `*_to_wire`/`*_from_wire` conversions with no test), then `casper/src/runtime_manager.rs` (1219 lines, 1 test) and the rest. Check 7 does not cover these files — they already have a test — which is why they are a separate item rather than part of 10. |

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
| `rholang/src/matcher/spatial_matcher.rs` — the `ESet`/`EMap` arms take their `remainder` from the *pattern* (as the `EList` arm and the Scala do), and `list_match`'s padding gate returns to the Scala's `remainder.is_some()` | Those two arms read the collection remainder off the **target**, which never has one, so `is_wildcard`/`remainder_var` were permanently `false`/`None` and a partial map/set pattern could not match at all — every rgov governance contract gates its entire body on one, and an unmatched `for` is silent, so the whole family returned `[]` with no diagnostic. The padding gate was a no-op for collections for the same reason (AUDIT §17 C20, and C19's resolution) | `a_map_pattern_may_name_fewer_entries_than_the_map_has`, `a_named_map_remainder_captures_the_unnamed_entries`, `a_set_pattern_may_name_fewer_members_than_the_set_has`, `list_remainders_stay_positional`, `collection_patterns_match_a_subset_of_their_collection` |
| `rholang/src/normalizer.rs::normalize_if` — the `if` condition is normalized against an empty `par`, not the accumulated one | `normalize_if` passed the caller's `ProcVisitInputs` (whose `par` holds everything already normalized *before* the `if` in its `\|`-chain) into the condition's normalization, so the `Match` target became `<the terms before the if> \| <condition>`. The desugared cases are `true`/`false`, a non-Bool target matches neither, and an unmatched `match` is not an error — so **every `if` that was not the first term of its `par` reduced to nothing, silently**, with the deploy reporting success. `MemberDirectory.rho:78` is such an `if` (its line 77 logs first), which is why `getMe` logged "everyone size" and then neither created the member nor answered the client; the same shape was dead in `Ballot.rho`, `Issue.rho`, `Group.rho`, and in the node's own `ListOps.rho` and `MultiSigRevVault.rho`. The Scala is inconsistent with itself here (`PIfNormalizer.scala:24` passes `input` through; `PMatchNormalizer.scala:28`, the same desugaring, uses `VectorPar()`) — a deliberate deviation, registered in AUDIT §6, and a hard-fork class change (AUDIT §17 C21) | `an_if_condition_does_not_absorb_the_pars_before_it`, `an_if_fires_the_same_way_wherever_it_sits_in_a_par`, `a_fresh_chain_serves_the_wallets_new_inbox_handshake` |
| `rholang/src/normalizer.rs::fold_collection_map` — a map pattern's remainder makes it non-concrete (`connective_used`) | The list and set folds include it (`cu \|\| has_rem`) and the reference ORs it in (`CollectionNormalizeMatcher.scala:92/112/137`); the map fold did not, so a partial map pattern's `connective_used` stayed false and `spatial_match` took its `pattern == target` short-circuit — a partial map matched nothing but itself, silently, which is the C19-C22 family's failure mode. Every existing conformance case carried a free variable, which set the flag for another reason, so the gap was invisible (AUDIT §17 C22) | `collection_patterns_match_a_subset_of_their_collection` (the ground-entry cases: matches, does-not-match, and the named remainder's capture) |
| `casper/src/genesis/rgov.rs::source("inbox")` — a render-time repair of the vendored `Inbox.rho`: `read(ret)` re-sends the store it empties (`ret!(*items) \| box!(Nil)`), via `replace_once` (which fails the build if upstream reword the block) | The zero-argument read consumed the single `box` datum and never restored it — unlike its two typed siblings and unlike its own docstring ("read (and *remove*) all messages") — so one read left that inbox answering nothing for the life of the chain, silently. The vendored `.rho` bytes stay upstream-identical; the repair is recorded in `resources/rgov/NOTICE` (AUDIT §17 C22) | `a_read_does_not_destroy_the_inbox` |
| `rholang/src/parser.rs` — an empty collection with only a remainder parses (`[..._]`, `Set(..._)`, `{..._}`), in the list/set element loops and in both map parsers | The grammar's element lists may be empty (`rholang_mercury.cf:179-183`), but the loops demanded an element before the ellipsis, so each form was rejected with `expected variable, got Ellipsis`: a "discard the tail" pattern was unwritable. **Found by the new conformance corpus on its first run** (`spec/conformance/flags.tsv`'s `@{..._}`, whose verdict is `decide`d in `spec/Rchain/Corpus.lean`) — AUDIT §17 C24 | `collection_remainders_parse_for_lists_and_sets_not_only_maps`, `the_connective_used_flag_agrees_with_the_lean_model` (the corpus consumer), `Corpus.flagCases_decide` |
| `casper/src/genesis/rgov.rs::extra_directory_slots_source` — the three extra directory slots are written with the directory's own `write` arity | It called `MCAwrite!("Chat", *C_Chat)` — two arguments — while `Directory.rho:56` is `write(@key, @value, ret)`, so no receive matched and **none** of `Chat`/`Ballot`/`Group` was ever written; consumers read `Nil`, which they cannot distinguish from broken (the documented reason the slots exist). The covering test asserted a `contains` on the source text, which a two-argument call satisfies — the harness accepted what the node rejected (AUDIT §17 C22) | `the_extra_slots_answer_a_directory_read` (reads each slot back and asserts the *value*), `the_extra_slots_term_writes_the_names_the_wallet_asks_for` (now pins the arity) |
| `casper/src/genesis/{mod.rs,standard_deploys.rs,runtime_replay.rs}` + `rholang/src/{native_state.rs,system_processes.rs,runtime.rs}` — genesis installs `ListOps`/`NonNegativeNumber`/`MakeMint`, seeds the shorthand aliases natively (native channels + the blessed contracts' `rho:id` entries), adapts `MakeMint.rho`'s epilogue, and reproduces the seeding on replay | A fresh chain's registry was empty, so `lookup!(\`rho:rchain:revVault\`, *ch)` answered `Nil` — silently, because an unmatched `for` is not an error, which is why the rgov family and the wallet's bonding path failed as if in client code. The `MakeMint` epilogue waits on two channels this port does not have, so it could never register (`spec/GENESIS.md`). The replay twin must seed identically or the replayed genesis hash diverges (Law 11) | `a_fresh_chain_resolves_and_can_call_every_seeded_shorthand`, `the_seeded_registry_is_identical_across_fresh_genesis_ceremonies`, `aliased_contract_uris_are_pinned`, `every_genesis_alias_has_a_source`, `the_make_mint_epilogue_is_adapted`, `a_drifted_make_mint_source_is_an_error` |
| `casper/src/genesis/rgov.rs` + `casper/src/genesis/resources/rgov/**` — the vendored rgov governance class contracts: fixed keys derived from a named hash, class registration converted from `insertArbitrary` to `insertSigned` (constants instead of per-chain URIs), dependency markers substituted, deploy-time self-tests removed, master-directory template rendered with the installed URIs | Upstream deploys the set per chain and *records* the resulting URIs, so the master directory's member list shifted on every chain and a recorded master URI went stale silently. Installing the classes with fixed keys makes their URIs constants (`spec/GENESIS.md`); the licence position (upstream declares Apache-2.0 but ships no LICENSE file) and every adaptation are recorded in `resources/rgov/NOTICE` | `every_rendered_contract_parses_and_normalizes`, `the_class_registration_is_the_signed_one`, `the_uris_are_constants`, `the_member_directory_imports_the_installed_contracts`, `the_master_directory_template_carries_the_installed_uris`, `the_deploy_time_self_tests_are_removed`, `a_fresh_chain_installs_the_rgov_contracts_and_they_answer` |
| `casper/src/genesis/rgov.rs` (rewritten), `casper/src/genesis/mod.rs`, `casper/src/runtime_{manager,replay}.rs`, `resources/rgov/**` (11 vendored files) — the vendored rgov set now keeps **upstream's** registration shape, publishes each registered URI, and has it copied onto a chosen constant key; the master directory, three extra slots and the `GetMe` feature are installed at genesis, signed by **the genesis ceremony's key** (not a key derivable from the source, as an earlier revision used) | Two defects, both found on a node: (a) converting the class registration to `insertSigned` changed the stored *value* to `(nonce, value)`, and every rgov consumer destructures the bare value, so the master-directory template stalled silently (`processedWithSuccess`, empty result); (b) installing the template and the feature under *different* keys leaves the feature's registration gated on a capability it cannot see, so the directory answers `Nil` for `GetMe` and a client's first call gets silence. Genesis now makes the whole set resolvable under constants a client hardcodes (`spec/GENESIS.md`). The stall that a probe then showed between "the directory answered `GetMe`" and a client's answer was **not** the genesis installation and not the feature's flow: it was AUDIT C21 (an `if` that was not the first term of its `par` reduced to nothing, so `getMe` never reached its `createMe`), fixed in `normalizer.rs::normalize_if` — see the row above | `the_class_registration_keeps_upstreams_shape`, `the_published_keys_are_constants`, `the_governance_terms_are_signed_by_the_ceremony_key`, `the_extra_slots_term_writes_the_names_the_wallet_asks_for`, `every_rendered_contract_parses_and_normalizes`, `a_fresh_chain_installs_the_rgov_contracts_and_they_answer`, `a_fresh_chain_serves_the_wallets_new_inbox_handshake` |

### The census sweep (definition of done items 10–11)

| Change | Why | Pinned by |
|---|---|---|
| `models/src/casper/protocol/deploy_service.rs::DeployExecStatus` — `rename_all_fields = "camelCase"` added beside `rename_all` | `rename_all` renames an enum's *variants*, not the fields of its struct variants, so the API emitted `deploy_result`/`deploy_error` in snake_case inside an otherwise camelCase response — a break against the Scala case-class field names the API mirrors (AUDIT §17 C16) | `the_api_types_deserialize_what_they_serialize` |
| `block-storage/src/test_support.rs` (`#[cfg(test)] pub mod test_support;` in `lib.rs`) — shared fixtures for the crate's unit tests | `BlockMessage` has 17 fields and no `Default`, so three test modules were carrying three copies of the same 20-line fixture; a fixture that drifts between copies makes two tests disagree about what "a block" is without either failing. **`#[cfg(test)]`-only: it does not exist in a release build and is not reachable from another crate.** Listed here because "it is only a test seam" is how a test seam becomes a production seam | `codecs.rs`, `syntax.rs`, `approved_store.rs`, `block_store.rs` tests all construct through it |

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

**Executed 2026-09-22**, on a single-validator devnet (the `valid`, `malformed` and `gateway` modes;
`scheduler` needs the network restarted with `--effect-scheduler`, which this devnet was not). The first
run that mattered was not green, and what it found were **two defects in the harness itself** rather than
in the node:

* `signed_deploy` hardcoded `--shard-id root` while the devnet's shard is `/root`, so **every** signed
  deploy failed the node's own shard check (`Deploy shardId 'root' is not a member of this node's shards:
  [/root]`) — the whole block-path half of the `valid` mode had never worked. It now reads the shard from
  the node's `/api/v1/status`.
* the `spliced-delimiter` generator emitted `@"f"!()` for the lone-closing case, which is **valid** — a
  send with one empty group — so the run reported it accepted (200). A generator that emits a well-formed
  term cannot test an input guard; the payload is doubled now.

After both fixes the run is green end to end (`robustness + determinism OK`, exit 0): 15 explore-deploys,
3 signed deploys through the block path with the height advancing 3039 → 3043, 23 malformed cases all 4xx
with every node still answering, and the single-shard surface unchanged. That both defects were the
harness's is the point worth recording: an unrun check is not a check, and this one had been "written and
syntactically checked" since it landed.

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
