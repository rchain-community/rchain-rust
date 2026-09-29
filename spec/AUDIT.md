# Audit check-off

What the adversarial audit of the Rust port has left. One row per finding, three states, and the
coverage that bounds any claim this page makes. The account behind any row -- what was found, how
it was falsified, what the fix does not do -- is in [`audit/passes.md`](audit/passes.md), at the
§ the row cites.

The companion pages: [`RUST-VS-SCALA.md`](RUST-VS-SCALA.md) on how the rewrite made these fragile
patterns explicit; [`review-ledger.tsv`](review-ledger.tsv), the coverage data the count below is read
from; [`INVENTORY.md`](INVENTORY.md), the law catalog; and [`TYPE-SYSTEM.md`](TYPE-SYSTEM.md), the
discipline this is checked against.

**The review ledger's rendering and its gate are gone** (2026-09-27): `REVIEW-LEDGER.md`, its emitter,
and the 1,453-line `tools/audit-test-register.sh` that checked it were deleted. Their failures were
always "a document disagrees with the tree", nothing production-facing read them, and the gate needed
maintaining more often than it caught anything. What a reader wanted from that material is the count
on this page, which costs 2.5 seconds.

---

## What this page does not cover — the remainder

**The check-off below counts two specific rosters, and neither is the tree.** The findings count is
every finding the audit has filed; the coverage count is the ledger's **T1 rows** — the tier that can
fork the chain or lose funds, which is **28 files, 60 law-register rows and one workflow**, not 89
modules. Everything under that tier is the remainder, and the remainder is most of the ledger:
`spec/review-ledger.tsv` carries **626 rows, 507 of them `deferred`**.

| kind | deferred / rows | what a row is |
|---|---|---|
| `file` | **306 / 352** | a tracked source file — T1 28 (none deferred), T2 21 (18), T3 303 (288) |
| `config` | 104 / 105 | an operator-facing knob: a CLI option or a `defaults.conf` key |
| `ingress` | 40 / 45 | a wire or HTTP surface |
| `process` | **30 / 30** | a system process the node installs under its `rho:` URN |
| `tool` | 20 / 27 | a gate in `tools/` |
| `class` | 6 / 6 | a mutation class in the type-system sweep |
| `roster` | 1 / 1 | the ledger's own machinery, which is gone |
| `law` | 0 / 60 | the law register's rows — closed |

Two `file` rows moved from `deferred` to `exempt` on 2026-09-28 (the ZFA prototypes deleted under
[#38](https://github.com/rchain-community/rchain-rust/issues/38)), which is why the file tile reads
306 and not the 308 an earlier count carried. Four crates carry **no verdict at all** — `block-storage`
(14 rows), `sdk` (10), `qucalc` (2), `graphz` (1) — and RSpace, the largest, carries 8 verdicts of 57.

**Why "all 89 T1 modules read" and "306 deferred file rows" are both true.** The coverage line counts
`deferred` rows *whose tier is T1*, and the tier cuts across kinds rather than being the file roster:
those 89 rows are **28 files, 60 law-register rows and one workflow** (`coverage.yml`) — so the line
narrows twice, once to a tier and then, inside the file roster, to 28 of 352. Neither narrowing is
hidden, and `tools/audit-status.sh` prints the tier it applied. The sharpest true reading of the line
is "every T1 row carries a verdict, and the T1 files are read"; a reader who takes it as a claim about
the tree has taken it one roster too far. Nothing here is a rounding.

**The reading order for what is left, and it is not a preference.** *RSpace first, then
`block-storage/src/dag/*`.* RSpace holds the merge and the trie: all six `rspace/src/merger/*` rows are
deferred (a `StateChange` merger and an event-log merger among them), as are ten of the fifteen under
`rspace/src/history/*`. The merge is where the consensus laws (9, 19, 20, 21) are stated, and where the
chain-halting duplicate-action panic of
[#83](https://github.com/rchain-community/rchain-rust/issues/83) is assembled. Then
`block-storage/src/dag/*`, where **all eight** rows are deferred — `finalizer.rs`, `representation.rs`
and `message_state.rs` among them, the fringe and finality machinery. The order follows consequence,
and it is corroborated rather than asserted: the one open defect that halts a chain points into the
first of the two.

**A clean read is a claim, not evidence.** §22 records the pass that read the T1 roster and got one
file wrong — it read `casper/src/txn_coordinator.rs`, reported "Nothing found", and a second reader
found C166 in it the same day, in a defect its own doc comment warned about six lines above. These
files should be read the way that pass's other nineteen reads were *not*: with a second reader, or a
probe, before a `cleared` verdict is written.

**The process and config rows are not reading jobs — the decision [#94](https://github.com/rchain-community/rchain-rust/issues/94) asks for.**

- A `process` row is an installed URN, not a file. What reaches a deploy is its **wire protocol** —
  arity and reply shape — which is what `spec/conformance/protocol.tsv` holds, and
  `every_catalog_urn_arity_matches_the_definition_the_node_installs` ties each row to the node's own
  `Definition.arity` (C158, falsified by planting a drifted arity). **Nine of the thirty are in that
  corpus.** The other twenty-one are closed by extending it, one row each, falsified the same way — not
  by twenty-one adversarial reads. Their bodies live in `rholang/src/system_processes.rs`, a **T1 file
  already read and carrying a finding**: the file-level question is answered, and what remains is
  arithmetic.
- A `config` row is a name in the operator-facing configuration surface — an `#[arg(...)]` in
  `node/src/configuration/commandline/options.rs` or a `defaults.conf` key — and it is a roster of
  **names rather than declarations**. Three of the rows say so in their own labels: `depth`, `content`
  and `type` each carry a second entry labelled `@2`, and each of the three names is declared twice in
  `options.rs` (`:168`/`:173`, `:184`/`:191`, `:182`/`:189`), so the suffix marks the second
  declaration of one name. The seeding rule that wrote the roster was deleted with the emitter on
  2026-09-27. So the first job is not a read but a **resolving pass** — each of the 105 names to the
  declaration it means — and the second is the adversarial question C143 already worked for nine keys:
  **is it enforced, or parsed and ignored?** Both halves are mechanical, and both are one unit with a
  worked example in the register. Reading the rows one at a time would answer the same question 105
  times, and for the three duplicates it could not be answered at all: `content@2` does not name a
  declaration, so there is nothing to read.

**Two things this note does not do.** It does not lower the declared ceiling: `unhousedCeiling` and the
per-kind `ceiling` lines in `spec/review-ledger.tsv` stay where the 2026-09-27 measurement put them,
because the historical backlog they count is a different question from what has been read. And it does
not promise a schedule. What is here is a denominator, an order to work in, and the two decisions taken
on the rows that are not reads.

---

<!--EMPTY: the emitter writes the check-off below this line-->

## Check-off

**Findings  TODO 2 · IN PROGRESS 0 · DONE 207** &nbsp;&nbsp;·&nbsp;&nbsp; **Coverage  all 89 T1 modules read**

Closed when both halves are zero. A **done** row is settled -- fixed, assessed faithful, a
deliberate deviation, or refuted -- and names what holds it. A **todo** row names what would
close it.

### TODO — findings (2)

| id | what | what closes it | account |
|---|---|---|---|
| `C171` | an all-live net attesting on every remote block runs a block storm: the guard cannot suppress while the quorum IS reachable, so four deploys produced 126 blocks in about three minutes with autopropose off (reproduced 2026-09-29; 276 in a minute on #70) | the pace half on attest_warranted (node/src/runtime/node_runtime.rs:2590): this node's own latest message at least k heights behind the tip, so an all-live net's attestation rate is bounded. Falsifier: a three-validator devnet with --attest-on-new-blocks whose block growth per deploy is bounded, and red when the pace term is deleted. #70 increment 2 | §23 |
| `C173` | one attributable failure in a bonded validator's block estranges the node that recorded it, permanently: the height maximum skips failed justifications, and neglected_invalid_block refuses any block justifying a failed bonded sender — so a node that fails one block is refused every block above it (#105) | a fork decision, since both rules are the oracle's: count failed justifications in the height maximum (the H1b descent bound still refuses a failed parent at or above the child), or give a stranded node an explicit path back (revalidation of the failed metadata, or a bounded re-fetch). Falsifier: a node that has marked one block failed must still accept the next block above it | §25 |

### T1 coverage — closed

All 89 rows for the modules that can fork the chain or lose funds have been read: 61 carry a
verdict of `cleared`, 24 produced a finding, and 4 are `exempt` with a reason class.
The twenty reads of the 2026-09-27 coverage pass are in the pass record, and two of them found
defects this register had not recorded (C164, C165).

### In progress — none

Nothing is in flight. The state exists because a person mid-read needs somewhere to say so;
that it is empty is the fact, and it is said rather than shown as a table with no rows.

### DONE (207)

| id | what | evidence | account |
|---|---|---|---|
| `C1` | unauthenticated arbitrary-rholang Repl on `0.0.0.0` | — | §5 |
| `C2` | transport `stream` buffered all chunks before the size breaker | max_stream_message_size | §5 |
| `C3` | `send` spawned a task per inbound message, unbounded | — | §5 |
| `C4` | a saturated inbound queue is reported to callers as `MessageTooLarge` | a_full_dispatch_queue_is_rejected_and_recovers | §15 |
| `C5` | a `ParBody` continuation dispatched with no matched data panics in the random merge | a_par_body_with_no_matched_data_panics_in_merge | §15 |
| `C6` | `graphz` does not escape its input. `graphz/src/lib.rs::quote` wraps a label in quotes | an_embedded_quote_is_not_escaped/a_node_label_is_written_through_without_quoting | §15 |
| `C7` | `generate_key`'s password retry recurses without a bound | generate_key_reprompts_on_a_mismatch_and_refuses_an_empty_password/generate_key_retries_after_an_empty_password | §15 |
| `C8` | the R6 private-key file mode was applied only at creation (fixed here) | the_private_key_file_is_owner_only | §15 |
| `C9` | `(x)` was parsed as a one-element tuple; it is a group | a_parenthesised_expression_is_a_group_not_a_one_element_tuple | §16 |
| `C10` | the logical connectives were swapped, and disjunction was unparseable | the_logical_connectives_lex_and_parse_in_their_grammar_spelling | §16 |
| `C11` | `++` had no Map or Set arm. `Reduce.scala`'s `EPlusPlusBody` defines five arms: String, | plus_plus_concatenates_byte_arrays_and_unions_maps_and_sets | §16 |
| `C12` | `+` and `-` had no collection arms. `Reduce.scala`'s `EPlusBody` has | plus_and_minus_also_insert_into_and_delete_from_collections | §16 |
| `C13` | the pretty printer emitted two `|` separators between the first two items of a group, and | printing_and_reparsing_is_the_identity | §16 |
| `C14` | the UPnP SSRF guard could be bypassed with a bracketed IPv6 URL (fixed) | is_ssrf_unsafe_host/the_url_guard_allows_private_gateways_and_refuses_ssrf_targets | §16 |
| `C15` | the bit-level decoder panics on a truncated bit stream (latent, pinned) | a_truncated_mergeable_datum_panics | §16 |
| `C16` | the deploy-execution-status enum serialized its variant *fields* in snake_case, in the | the_api_types_deserialize_what_they_serialize | §17 |
| `C17` | the list and `Set` collection branches had no remainder production, and neither did the | collection_remainders_parse_for_lists_and_sets_not_only_maps | §17 |
| `C18` | `rho:registry:lookup` wrapped its reply in `(uri, value)`; the oracle sends the stored | registry_insert_arbitrary_and_lookup_round_trip | §17 |
| `C19` | a collection pattern with a *wildcard* remainder could never match, so partial *map* | collection_patterns_match_a_subset_of_their_collection/locally_free_empty | §17 |
| `C20` | a remainder in a `map`/`set` pattern never absorbs an entry: the pattern's remainder was | list_match_single/a_map_pattern_may_name_fewer_entries_than_the_map_has/a_named_map_remainder_captures_the_unnamed_entries | §17 |
| `C21` | an `if` normalized its condition against the `par` that precedes it, so every `if` that was | an_if_condition_does_not_absorb_the_pars_before_it/an_if_fires_the_same_way_wherever_it_sits_in_a_par/a_fresh_chain_serves_the_wallets_new_inbox_handshake | §17 |
| `C22` | C21's family, three more instances: two silent-no-match defects and one silent-consumed | a_read_does_not_destroy_the_inbox/the_extra_slots_term_writes_the_names_the_wallet_asks_for/the_extra_slots_answer_a_directory_read | §17 |
| `C23` | the formal model did not contain the surface the ten silent defects live in | — | §17 |
| `C24` | an empty collection with only a remainder did not parse (found by the new conformance | collection_remainders_parse_for_lists_and_sets_not_only_maps | §17 |
| `C25` | `Group!("new", …)` answered nothing because it read a dictionary genesis never | a_fresh_chain_answers_one_group_creation/a_fresh_chain_answers_two_group_creations_in_one_deploy/if_inside_a_receive_body_runs_its_else_branch | §17 |
| `C26` | law 5 was three `axiom`s, one of them false, and nothing checked any of them | — | §17 |
| `C27` | the base-sort receive never said which channel it listens on | — | §17 |
| `C28` | the model's ground scalars were missing two of the five the protobuf has | — | §17 |
| `C29` | the served OpenAPI document was stale, and nothing held it to the code it describes | — | §17 |
| `C30` | `parse` returned `Ok` for a valid *prefix* of its input. `Tok::Eof` is pushed by the lexer | source_to_adt_with_env | §18 |
| `C31` | a trailing separator was accepted at every list site — and then refused, and then | a_trailing_separator_is_accepted_as_a_deviation/expect_element_after_separator | §18 |
| `C32` | `in` was optional in `new` and `let`. `PNew ::= "new" [NameDecl] "in" Proc1` and | — | §18 |
| `C33` | a method call without its argument list parsed. `PMethod ::= Proc11 "." Var "(" [Proc] | — | §18 |
| `C34` | `GroundBigInt` was unreachable. `BigInt(42)` parsed as the *simple type* `BigInt` followed | parse_simple_type | §18 |
| `C35` | `PSendSynch` was unreachable. `PSendSynch ::= Name "!?" "(" [Proc] ")" SynchSendCont` | parse_name_source | §18 |
| `C36` | two lexical forms were accepted by accident and one crashed | — | §18 |
| `C37` | `rho_examples` was measuring the stack, not the parse | — | §18 |
| `C38` | every rho value in every reply was wrapped wrongly, and the schema file rationalised it | the_wire_shape_is_the_reference_documents | §19 |
| `C39` | an exploratory deploy's reply was read from one channel, and a reply anywhere else was | genesis_boot_exposes_block_over_http | §19 |
| `C40` | law 38's tie was stated as an `iff` that is false, and the relation was missing the clause | takesStep_sound/takesStep_complete | §19 |
| `C41` | the numeric-channel diff accumulator can overflow, and the merge beside it cannot | calculate_number_channel_merge/calculate_num_channel_diff/numeric_channels_nonneg | §19 |
| `C42` | law 5's linearity is enforced by the normalizer, not by the matcher, and the model had it | handle_proc_var/spatial_match_core/spatial_match_result | §19 |
| `C43` | the merge's associativity was untested, under a test that looks like it tests the | combine_is_associative/combine_has_an_identity_and_agrees_on_sorted_multisets/law9_state_change_combine_is_associative | §19 |
| `C44` | the matcher had no clause for a tuple, and the port has one | concrete_matches_iff_eq/arithmetic_pattern_refutes_the_unrestricted_tie | §19 |
| `C45` | the search claimed a step for a join, and the rule could not have derived one anyway | count_no_wildcards/exists_redex_split | §19 |
| `C46` | a validator cannot index the genesis, because the sidecar-regeneration path replays it without | create_genesis_block/parse_if_exists/set_vault_balance | §19 |
| `C47` | the matcher's fuel was short on a shape the node matches, because the measure it was derived | the_walk_past_empty_pars_is_paid_for | §19 |
| `C48` | the spec over-claimed a match: the *searcher* was wired into the list arm (found | a_list_pattern_cannot_skip_a_target_element | §19 |
| `C49` | the replay property test fails on its own recording, roughly three runs in ten (found | law11_a_replayed_script_matches_its_recording/check_replay_data/comms_for_produce | §19 |
| `C50` | the matcher's fuel was short a *second* time: the measure had no `etuple` case, so a tuple's | fuel_saturation | §19 |
| `C51` | the tie's domain predicate admitted a shape the clauses reject, so the tie was false | concrete_matches_iff_eq/a_two_expression_pattern_refutes_the_modelled_tie | §19 |
| `C52` | a peer's `BindPattern` could carry a negative `free_count`, and the count is not inert | bind_pattern_from_proto/receive_bind_from_proto/match_case_from_proto | §19 |
| `C53` | a store error read as an absent radix node (found 2026-09-24, Programme F; fixed at the | RadixTreeImpl::load_node_from_store/RSpaceImporter::get_history_item | §19 |
| `C54` | a wrong fix, caught by the corpus in one run: what the "set pattern over-claims" reading got | list_match_single/lean_match_corpus | §19 |
| `C55` | the devnet bootstrap never starts: restoring a stored chain folded the message state once per | create_comm_state/insert_msg_mut/insert_msg_without_latest_mut | §19 |
| `C56` | the per-block merge scope copied every message it looked at (found 2026-09-24, while | between_is_the_id_set_difference_restricted_to_the_map/dag_message_state/reading_the_dag_representation_does_not_copy_the_message_state | §19 |
| `C57` | law 16c's remaining tie is not a plumbing job: the model's encoder and `prost` disagree in | — | §19 |
| `C58` | law 1a's remaining gap is a *re-tagging of `cmpExpr`*, not eight more arms: the model cannot | cmpExpr/ebigint | §19 |
| `C59` | the set/map matcher over-claimed, and C54's reverted guard was the fix: what the reversion's | a_permuted_pattern_is_refused/an_unaligned_variable_pattern_is_refused | §19 |
| `C60` | the tie's domain was still too wide, and the port's matcher is not the clauses at all: the | spatialMatches_iff_eq | §19 |
| `C61` | a peer-supplied resume prefix of 128 bytes was one byte over the segment invariant, and the | a_128_byte_resume_prefix_is_refused | §19 |
| `C62` | the sync path multiplied the page it served, and the node's own metrics surface had no | chunking_a_page_does_not_copy_it_whole/report_period_snapshot/the_metrics_route_serves_the_registrys_own_numbers | §19 |
| `C64` | a fresh validator's catch-up is slow, a measured cost -- but the port does **not** diverge: the oracle's `lowerBound`/`extraHeights` cutoff is **inert in its own production path**, so both trees walk the full ancestry to genesis, and the `owes` cell's remedy (adopt the cutoff) would have been a no-op. `LfsBlockRequester.scala:125` defaults `lowerBound = 0`; the only construction in the tree (`:318`) passes `latest` and `extraHeights` and never `lowerBound`; so `blockIsAccepted = isReceivedLatest || blockNumber >= minimumHeight` (`:228`) and `NodeSyncing`'s `blockHeightOk = blockHeight >= minHeight` are both `>= 0` and vacuously true. `extraHeights` is `deployLifespan` (50, `MultiParentCasper.scala:35`) and *lowers* the bound further, so it lengthens the walk rather than bounding it. The ~6,300-vs-~50 round trip figure compared the port against a cutoff the oracle never applies | a_walk_longer_than_deploy_lifespan_reaches_genesis | §19 |
| `C65` | the block store's side of LFS sync swallowed two failures the oracle propagates, so a | a_failed_block_write_fails_the_walk_instead_of_marking_the_block_done | §19 |
| `C68` | a failed LFS sync still signalled the node out of syncing, so it could run on an | notify_when_restored/a_failed_sync_does_not_signal_the_node_out_of_syncing/a_failed_sync_leaves_the_node_in_syncing/an_empty_fringe_does_not_consume_the_sync_trigger/an_empty_fringe_finishes_the_walk_at_once_with_nothing_in_it | §19 |
| `C70` | the gate could not see a nested comment, and the widened token set is the only check that | check_rust_witnesses | §19 |
| `C71` | the matcher's padding is reachable from a well-formed term, so U10's refusal was refuted by | spatial_match_result/resolve_match_pads_a_level_the_pattern_does_not_bind/rho_match_pads_a_free_count_its_pattern_does_not_bind | §19 |
| `C72` | four register cells, three prose paragraphs and two citations called something owed while the | RSpaceImporter::get_history_item/RNodeStateManager::is_empty | §19 |
| `C73` | a store-items page had a cap on its *count* and none on its *bytes*, so one request bought tens | validate_state_items/a_page_over_the_byte_cap_is_dropped_not_truncated | §19 |
| `C74` | the name-shape vocabulary is a convention, not a predicate, and the measurement says which half | reduce_not_deterministic | §19 |
| `C75` | the gate could not see a Lean module at the top of `spec/`, so a `sorry` there was invisible to | — | §19 |
| `C120` | a deploy's `deployer` is unauthenticated on the block-validation path | a_block_whose_deploy_is_not_signed_by_its_named_deployer_is_refused | §21 |
| `C121` | `POST /api/txn` is unauthenticated and spends the node's own validator REV to a caller-chosen address | api_txn_run/deploy_rate_limiter/api_get_transaction | §21 |
| `C122` | the C110 slash rule reads a flag the DAG's own store cannot return, so slashing is unreachable | the_slashable_flag_survives_the_store_round_trip_and_reaches_the_slash_rule/add_and_lookup_round_trip | §21 |
| `C123` | a deploy pool holding 256 valid deploys makes block production impossible | the_per_block_budget_leaves_room_for_every_seed_the_block_needs/the_selection_is_capped_and_takes_the_pools_canonical_prefix | §21 |
| `C124` | the matcher's subset enumeration is performed before the bound that refuses it | min_max_subsets/a_single_dimension_cannot_outgrow_the_product_cap | §21 |
| `C125` | `GatewayTxn::run`'s check-then-act on the durable coordinator ledger is not atomic | recover_in_flight | §21 |
| `C126` | the `silent` hard class is line-anchored, so a rustfmt-wrapped `try_from(..).unwrap_or(0)` chain is invisible | — | §21 |
| `C127` | the `escape` class is scoped to eleven files, so `impl Deref for <refinement>` in a sibling file is invisible | — | §21 |
| `C128` | `check-rust-witnesses.sh` counts a `... ignored` line as a passing witness | — | §21 |
| `C129` | R19 refuted: `exploratory_deploy` reads the non-finalized chain tip again | exploration_anchor | §21 |
| `C130` | the `unsafe` class matches only `unsafe {`, so `unsafe fn` / `unsafe impl` with real unsafe operations are green | unsafe_op_in_unsafe_fn | §21 |
| `C131` | the completeness guard's premise for excluding public-field newtypes is false for any name not already in `REFINEMENT_TYPES` | — | §21 |
| `C132` | registered deviation S25 is false of this tree | admin_bind_host | §21 |
| `C133` | the admin router's CORS comment promises a protection a simple cross-origin POST needs no preflight to bypass | — | §21 |
| `C134` | `Secp256k1::verify_bytes` does not bind the message it verifies | every_algorithms_verify_refuses_malformed_input | §21 |
| `C135` | `withdraw`'s deadline multiplies the raw `epoch_length` while every other epoch site divides by `epoch_divisor` | is_epoch_boundary | §21 |
| `C136` | `find_and_connect` dials every discovered peer serially; the routing table admits thousands and `MAX_CONNECTIONS` bounds the table, not the work | find_and_connect | §21 |
| `C137` | `coverage.yml` runs unpinned action refs with no `permissions:` block | — | §21 |
| `C141` | a malformed operator config file is silently truncated | a_config_file_with_a_syntax_error_is_refused_not_truncated | §21 |
| `C142` | a server that fails to bind is never reported | create_comm_state | §21 |
| `C143` | nine config keys in `defaults.conf` are parsed and never enforced | use_random_ports/keep_alive_time/keep_alive_timeout | §21 |
| `C144` | SIGTERM is ignored and there is no shutdown path | with_graceful_shutdown/acquire_http_server/grpc_transport_receiver | §21 |
| `C145` | the production logger emits no timestamp and has no verbosity control | is_trace_enabled | §21 |
| `C146` | a dangling `C 315` pointer in the operator-facing config, invisible to the check whose job is resolving pointers | — | §21 |
| `C147` | an unknown `--profile` is silently replaced by the default profile | an_unknown_profile_is_refused_rather_than_replaced_by_the_default | §21 |
| `C148` | no read surface for the PoS epoch, the active validator set, or pending withdrawals | WebApi::status | §21 |
| `C149` | law 46's declared witness cannot fail on the mechanism it names, because its own fixture makes the proportionality factor the identity for every validator it builds | an_epoch_splits_the_pot_and_keeps_the_dust/a_released_withdrawal_pays_the_bond_plus_the_committed_rewards/the_dust_is_real | §21 |
| `C150` | law 21's declared witness cannot fail on the gate, because its fixture runs on a single-threaded runtime where the gate has no observable effect | law21_the_gate_scheduler_refines_the_sequential_reference | §21 |
| `C151` | law 25's declared witness cannot fail when the block path's validation gate is switched off. **Both halves are now settled, and the `owes` cell's remedy turned out not to be dischargeable as written.** The second half — "the switch from scheduler mode to validation flag is covered by no test" — is closed: `law24_the_effect_mode_is_what_enables_the_certificate` observes the behaviour the wiring controls (a claim on a channel whose newest write is DFS-later is refused under `RelaxedValidated` and accepted under every other mode), where replacing the wiring with `false` had left every test green. The first half was attacked by *searching* for the divergence the remedy asks for: **seventeen hand-written shapes, including the ordering law 24 exists for** (a produce `o`, a consume `p` and a second produce `q` with `o < p < q` in DFS order, where `q` writes `p`'s channel — the DFS-later write the certificate refuses at the queue level), run under `EffectMode::Relaxed`, **which has no certificate at all**, at 40 repetitions each against the sequential reference. **None diverged on state hash or event count.** The reason is structural rather than a weak search: `Relaxed` preserves *per-channel* DFS op order in the claim queue (law 20) and frees only cross-channel interleaving, which changes neither how many events commit nor what the space holds for any program of this class — the certificate's observable effect is at the acquire, not on the resulting state. So `law25_...` cannot be made to fail by deleting the gate, not because its strategy is too narrow but because no state-level divergence exists to witness. **What would close it more strongly than a test**: a proof that relaxed ≡ sequential on state for the scheduler's fragment, which is a modelling obligation rather than a witness | law24_the_effect_mode_is_what_enables_the_certificate/c151_the_unvalidated_relaxed_scheduler_reaches_sequential_on_every_shape_tried | §21 |
| `C152` | law 11's machine-checked `rustWitness` names a test that cannot fail on the row's mechanism, while the row's own prose names the two that can | a_rig_whose_comm_never_happens_is_reported | §21 |
| `C153` | law 27's register entry points its Rust evidence at two end-to-end integration tests and at nothing else, while three tests named `law27_*` carry the row's per-guard evidence | decision_is_deterministic/law27_a_legless_record_cannot_commit | §21 |
| `C154` | law 19's `rustWitness` names four tests, none of which carries the merge claims the row's own statement makes | merge_with_two_children/merge_with_many_children | §21 |
| `C155` | the register's own anchor check cannot see a citation move by up to eight lines | — | §21 |
| `C156` | law 30's corpus does not catch the parser accepting trailing input, though the row's own `falsifiable` prose names trailing input among the shapes the corpus refuses | the_node_parser_agrees_with_the_lean_model | §21 |
| `C157` | `tools/audit-mutate.sh` reported `green` for runs in which no test ran | the_node_parser_agrees_with_the_lean_model/the_printers_output_round_trips_through_the_node | §21 |
| `C158` | an arity drift on a catalog entry that does not reply was caught by nothing: the corpus carried the call's arguments and the reply's shape, but not `callArity`, so the only way a consumer could see a drifted arity was to wait for a reply that a `kind = none` row (`rho:io:stdout`, `rho:io:stderr`) never sends. **`callArity` is now emitted into `spec/conformance/protocol.tsv` and tied to the node's own `Definition.arity`** — the referent `Rchain/Protocol.lean`'s doc names — by `every_catalog_urn_arity_matches_the_definition_the_node_installs`, with `a_drifted_catalog_arity_is_reported` showing the comparison can fail. Falsified end to end: planting `2` on the `rho:io:stdout` row of the emitted corpus reddens the test with "the catalog declares arity 2, the node installs 1" | every_catalog_urn_arity_matches_the_definition_the_node_installs/a_drifted_catalog_arity_is_reported | §21 |
| `C159` | `tools/audit-type-system.sh`'s `silent` class listed every violation it found and counted none of them | scan_spanning | §21 |
| `C160` | a decided coordinator record could be resurrected by a late vote | an_abort_is_absorbing | §15 |
| `C161` | a phase-two failure is discarded. `casper/src/gateway/mod.rs::apply_phase_two` ignores | a_failed_phase_two_leaves_the_decision_intact | §15 |
| `C162` | the inner replay trace check does not fire for a term tamper; the state-hash comparison | a_tampered_deploy_replays_to_a_rejected_state_hash | §15 |
| `C163` | C129's fix left `node/tests/node_api.rs` asserting the old explore-deploy behaviour, and C129's own commit never ran that file | genesis_boot_exposes_block_over_http | §21 |
| `C164` | a truncated `current-root` panicked the node on the state-read path: `RootsStore::current_root` built the hash from whatever the store returned, and `Blake2b256Hash::from_byte_array` asserts its length | a_truncated_current_root_is_refused_rather_than_panicking | §22 |
| `C165` | the private key is written *before* its file is narrowed, so a pre-existing 0644 `rnode.key` held the secret world-readable for the length of the write — and the comment says the opposite | the_mode_is_narrowed_before_anything_is_written | §22 |
| `C166` | `TxnCoordinator::run_2pc` anchored **every** phase deploy at block 0 -- it called the 0-hardcoded `run_phase` wrapper -- so on any chain taller than `DEPLOY_LIFESPAN` (50) every prepare and commit was *born expired*: the participant never saw it, nothing reported an error, and the transaction silently did not happen. `run_phase_at` had been added to fix exactly this trap, and `protocol/client.rs`'s `resolve_valid_after_block_number` doc names it as one of the sites that anchor correctly -- but the whole-transaction driver kept calling the wrapper, and `run_phase_anchors_at_zero` pinned that as "kept for the client path". The anchor is a required `TxnLeg` field now, `run_2pc` forwards each leg's own height, and the wrapper is deleted. Found reading `casper/src/txn_coordinator.rs` (T1) | run_2pc_anchors_each_phase_at_its_legs_height | §22 |
| `C167` | law 50a's own falsifier could not fire: its witness was `n` siblings rather than `n` nested levels, so the mutation test would have passed vacuously, under the cell that cites law 22 | parDepth_notsDepth/a_dropped_arm_breaks_soundness | §20 |
| `C168` | law 50a's Rust half was pinned by three tests that do not test it, while the every-constructor test that does exists and belongs to clause b | every_construct_in_the_parser_walk_is_descended | §20 |
| `C169` | law 50b's statement was not true as written: the walk's counted quantity is `Par`-nesting, and the gap is a tight factor of 3 rather than a slack of 128 | parNestDepth/walkValuePar/parDepth_le_three_mul_parNestDepth | §20 |
| `C170` | the attestation guard counted a validator that had *ever* spoken as moving stake, and its supermajority clause was short-circuited by a deploy-bearing parent — so a node that had lost over a third of its stake attested at every height, the reverse of what the shipped comment claimed | moving_attestation_stake/attestation_suppressed/a_silent_validators_stale_message_does_not_carry_the_quorum/an_unreachable_supermajority_suppresses_even_with_a_deploy_bearing_parent/but_a_node_quiet_past_the_window_speaks_again | §23 |
| `C172` | BlockMetadataStore::add updates the in-memory DAG index before the store it indexes, so has_all_deps (index) can queue a child whose justification block_summary (store) cannot resolve: ValidateError::Internal is dropped with no re-queue, the receiver never sees the block finish, and the stall is permanent until a restart (#103) | validate_dag_state_after/a_refused_height_gap_is_not_left_in_the_index/a_failed_store_write_is_not_left_in_the_index/validate_after_agrees_with_validating_the_extended_state | §24 |
| `C174` | the fringe gate asked two questions of one map, so a bonded validator that produced no message made the full-partition filter unsatisfiable and capped finality whatever share of the stake the survivors held (80 % survivors did not resume finality; the same shape froze #105's 91 % survivor) | live_weight_set/calculate_fringe/a_silent_bonded_validator_does_not_cap_the_fringe/the_quorum_is_measured_against_the_whole_bonded_map_not_the_live_one | §26 |
| `F1` | The interpreter core is a mechanical Scala port. `rholang/src/reduce.rs` (1773 lines) | — | §9 |
| `F2` | The blessed genesis contracts re-implement a HashMap trie in interpreted rholang | — | §9 |
| `F3` | Silent partiality hides the failure. `compute_bonds` (`casper/src/runtime_manager.rs:503-509`) | — | §9 |
| `F4` | Gas metering is unwired. `ChargingRSpace` (`rholang/src/storage.rs:103`) is a pure | — | §9 |
| `F5` | Scala-specific encodings leaked into the port | — | §9 |
| `H1` | /H2 — unauthenticated propose + deploy flooding (autopropose amplification) | — | §5 |
| `H3` | global `connections` write-lock held across outbound `send` | handle_protocol_handshake | §5 |
| `H4` | block-request bandwidth amplification | handle_block_request | §5 |
| `H5` | `BlockRetriever.requested` map unbounded | AdmitHashStatus::CapacityReached | §5 |
| `H6` | DAG message-state whole-map clone per insert | dag_message_state | §5 |
| `M1` | serialized TLS accept | — | §5 |
| `M2` | `assert!`/`assert_eq!` in the block-receiver state machine | — | §5 |
| `M3` | blocking LMDB I/O inside async handlers | tokio::task::spawn_blocking | §5 |
| `M4` | no outbound send timeout; `DEFAULT_SEND_TIMEOUT` was dead | — | §5 |
| `M5` | peer-table fillability | update_last_seen | §5 |
| `M6` | unauth `/reporting/trace` forceReplay | — | §5 |
| `M7` | plaintext-HTTP external-IP discovery | — | §5 |
| `M8` | bootstrap retry-forever | keep_on_requesting_till_running | §5 |
| `R1` | deploy signature not verified on the gRPC path. `casper/src/api/block_api_impl.rs::deploy` | verify_signature_accepts_valid_deploy/_rejects_tampered_term | §11 |
| `R2` | unbounded deploy pool. `casper/src/dag.rs::add_deploy` put without bound (keyed by | add_deploy_rejects_when_pool_full | §11 |
| `R3` | lz4 decompression bomb. `comm/src/transport/stream_handler.rs::decompress_content` | restore_rejects_oversized_decompressed_content | §11 |
| `R4` | unchecked phlo multiply. `models/.../casper_message.rs::total_phlo_charge` did | total_phlo_charge_does_not_wrap_on_overflow | §11 |
| `R5` | plaintext unauthenticated Kademlia discovery on `0.0.0.0:40404` | — | §11 |
| `R6` | unbounded `exploratory_deploy`. `casper/src/runtime_manager.rs::capture_results` ran | casper/src/runtime_manager.rs::capture_results | §11 |
| `R7` | i64 stake-sum overflow. `casper/.../proposer.rs` and `block-storage/.../finalizer.rs` | i64_overflowing_stakes_do_not_wrap | §11 |
| `R8` | private keys/certs written with default perms. `generate_certificate_if_absent.rs`, | fs::write/crypto::util::key_util::write_private_key | §11 |
| `R9` | rholang parser has no recursion-depth guard. `rholang/src/parser.rs` recursed without bound | rejects_excessive_nesting_depth | §11 |
| `R10` | HTTP `/api/deploy` not rate-limited | api_explore_deploy/api_explore_deploy_by_block_hash | §11 |
| `R11` | `PBKDF2_ITERATIONS = 1024` | — | §11 |
| `R12` | validate-on-ingress `from_slice` asserts. `models/{block_hash,block/state_hash,validator}.rs` | block_api_impl/deploy_grpc_service_v1/from_byte_array | §11 |
| `R13` | decompressed-blob memory amplification on the `stream` path. `comm/src/transport/grpc_transport_receiver.rs:173-217` buffers up to `max_stream_message_size` (default 256 MiB) per stream, then … | max_stream_message_size | §13 |
| `R14` | unbounded concurrent TLS handshakes. `grpc_transport_receiver.rs:248-263` spawns one accept task per TCP connection with no timeout; the 128-slot channel bounds only *completed* handshakes | — | §13 |
| `R15` | unbounded block-validation pipeline. `node/src/runtime/node_runtime.rs:531` feeds replay validation through an `unbounded_channel`; the bounded ingress (S18) is upstream of it, so a peer streaming … | block_processor::apply | §13 |
| `R16` | unbounded `StoreItemsMessageRequest.take`. `casper/src/engine/node_running.rs:342-344` bounds only the *sign* of `skip`/`take`; `take=i32::MAX` triggers a full-trie traversal + giant reply … | — | §13 |
| `R17` | faucet rate limit is global, no per-source/address budget. `node/src/web/http.rs:44,127-136` + `web_api_impl.rs:89` use one shared `RateLimiter` (1/s); a single caller drains the genesis dev … | — | §13 |
| `R18` | `CostAccounting.log` grows unboundedly. `rholang/src/accounting.rs:370,410` appends a `Cost` (with a heap `String` op) per `charge` and never clears; `total_charged()` (`:395`) re-sums the whole … | — | §13 |
| `R19` | `exploratory_deploy` reads the non-finalized chain tip. `casper/src/api/block_api_impl.rs`'s `exploratory_deploy` (commit `5186361dc`) read `height_map.iter().next_back()` (first hash at max … | last_finalized_block | §13 |
| `R20` | `revVault transfer` unchecked i64 add + self-transfer guard before the balance check. `rholang/src/system_processes.rs` — `i64::from(to_balance) + i64::from(amount)` overflows on extreme balances … | — | §13 |
| `R21` | arithmetic panic on `EMult`/`EPlus`/`EMinus`/`ENeg`. `rholang/src/reduce.rs:265,367,395,247` use raw `l*r`/`l+r`/`l-r`/`-hs` on `GInt`; `i64::MAX * 2` panics the reducer in debug builds | — | §13 |
| `R22` | number-channel merge/diff unchecked i64. `rholang/src/merging.rs:97,305` (`init_num + diff`, `end_val - prev`) wrap/panic and write a corrupted value into the trie | — | §13 |
| `R23` | `slice` charges output length but walks input uncharged. `rholang/src/reduce.rs:1264` (`"slice"`) — a recursive contract slicing a large string gets ~16M:1 op/phlo amplification | — | §13 |
| `R24` | SSRF filter classifies only IPv4 literals. `comm/src/rp/handle_messages.rs:27-40` + Kademlia lookup-insertion (`kademlia_node_discovery.rs:45-50`) connect to attacker-chosen hostnames/IPv6 | is_local_address | §13 |
| `R25` | global channel cache mutex held across an unbounded connect. `comm/src/transport/grpc_transport_client.rs:68-75` (`create_channel`) | connect_with_connector_lazy | §13 |
| `R26` | `stream` size cap counts only data bytes. `grpc_transport_receiver.rs:173-184` — empty `Chunk.content_data` never advances `received`, so unbounded empty chunks grow the per-stream buffer | — | §13 |
| `R27` | `phlo_price` checked after replay. `casper/src/multi_parent_casper.rs:298` (`block_summary`) — a below-min-price block is fully replayed before rejection, so `phlo_price=0` deploys give free … | validate_block_checkpoint | §13 |
| `R28` | deploy pool never expires future-dated deploys. `casper/src/dag.rs:428-432` — the pool ingress never bounded `valid_after_block_number`, so deploys anchored at `i64::MAX` filled … | valid_after_block_number | §13 |
| `R29` | block-receiver maps unbounded. `casper/src/blocks/block_receiver.rs:101` — valid-signed blocks with unresolvable justifications are retained forever | — | §13 |
| `R30` | `PeerRateLimiter` never evicts. `casper/src/engine/node_running.rs:106` — `BTreeMap<Vec<u8>,(Instant,u32)>` grows with connection churn | — | §13 |
| `R31` | attacker-influenced UPnP gateway can set the advertised external host (hostname bypasses `is_ssrf_unsafe_host`). `comm/src/upnp/gateway.rs:119-136` | a_hostname_external_address_is_published_and_named_as_one | §13 |
| `R32` | attacker-controlled large `sender.host` retained in the connections table. `comm/src/rp/handle_messages.rs:70-93` | a_host_over_the_bound_is_refused_at_the_wire | §13 |
| `R33` | faucet to the deployer's own address is a no-op that still consumes the rate budget and submits a deploy | the_faucet_refuses_a_drip_to_the_deployers_own_address | §13 |
| `R34` | `/api/faucet` routes are mounted unconditionally on the public router; the dev-mode gate is only inside the handler | a_node_without_the_faucet_does_not_mount_the_route | §13 |
| `R35` | a faucet drip is silently dropped once the tip passes `height+50` (`DEPLOY_LIFESPAN`), after `200` was already returned | — | §13 |
| `R36` | the single `deploy_rate_limiter` is shared by deploy + explore-deploy, so explore floods starve deploys | an_explore_flood_does_not_spend_the_deploy_budget | §13 |
| `R37` | play sorts channel data by `Datum.source` but replay keeps store order; correct today, but an undocumented play-vs-replay fragility | run_matcher_consume | §13 |
| `S1` | `encode_signature_rs_to_der` rejects RS length ≠ 64; `der_integer` guards empty input — closes the `secp256k1:eth` 1-byte-sig remote panic. | encode_signature_rs_to_der | §12 |
| `S2` | `decode_signature_der_to_rs` validates `end`/integer lengths before slicing — no panic on crafted DER. | decode_signature_der_to_rs | §12 |
| `S3` | `to_public`/`secret_key_from` return `CryptoError::InvalidLength` instead of `copy_from_slice` panic. | secret_key_from/copy_from_slice | §12 |
| `S4` | `normalize_signature_low_s` (DER + raw-RS) canonicalizes `s → n−s` when high; idempotent, never panics. | normalize_signature_low_s | §12 |
| `S5` | `EDiv`/`EMod` reject `l == i64::MIN && r == -1` — no `MIN / -1` / `MIN % -1` panic. | — | §12 |
| `S6` | `charge` clamps the balance at 0 on exhaustion (no negative cell). | — | §12 |
| `S7` | Receive continuation body uses `substitute_par_and_charge` (deviation: Scala charges). | substitute_par_and_charge | §12 |
| `S8` | `MAX_CHAIN_LENGTH = 512` guard on the ten flat operator/pipe/conjunction/method chains — no depth-N left-leaning AST. | — | §12 |
| `S9` | `MAX_CONCURRENT_STREAMS = 1024` semaphore on the inbound `stream` RPC. | — | §12 |
| `S10` | client `stream` wrapped in `DEFAULT_SEND_TIMEOUT`; `channels` cache capped (`MAX_CACHED_CHANNELS = 1024`). | — | §12 |
| `S11` | `connections` capped (`MAX_CONNECTIONS = 1024`); residual identity-binding gap documented. | — | §12 |
| `S12` | reject private/loopback/link-local/unspecified discovery endpoints (SSRF). | — | §12 |
| `S13` | `is_safe_url` rejects loopback/link-local/unspecified/multicast (allows RFC1918); bodies capped at 64 KiB. | is_safe_url | §12 |
| `S14` | external-IP body capped at 8 KiB. | — | §12 |
| `S15` | `phlo_price` result is honored — below-min-price blocks rejected (deviation from Scala `recoverWith`). | — | §12 |
| `S16` | `validate_dag_state` returns `Result` instead of `assert!`; propagated at both call sites. | validate_dag_state | §12 |
| `S17` | block-store `put` error is logged and the block skipped. | — | §12 |
| `S18` | `incoming_blocks` is a bounded channel (`MAX_PENDING_BLOCKS = 1024`, `try_send`); receiver side re-typed. | — | §12 |
| `S19` | validation-failed / internal-error blocks are no longer re-broadcast. | — | §12 |
| `S20` | `bytes.len() as u8` → `u8::try_from` (no truncation). | u8::try_from | §12 |
| `S21` | negative `depth` rejected (listen-at-name); `visualize_dag` clamps `start_block_number ≥ 0`; block-range check uses `checked_sub`. | — | §12 |
| `S22` | `block_lock_map` bounded (`MAX_LOCKED_BLOCKS = 4096`, oldest evicted). | block_lock_map | §12 |
| `S23` | `repeat_deploy` keys the dedup set on `normalize_signature_low_s` (malleability). | normalize_signature_low_s | §12 |
| `S24` | `getEventByHash` and `/api/transactions/:hash` gated on `enable-reporting`. | — | §12 |
| `S25` | `/api/v1/propose` `GET → POST`; admin CORS gated by `--api-enable-devnet-cors`; admin HTTP binds loopback unless `api-server.enable-devnet-admin-public` is set — *corrected 2026-09-27 (AUDIT C132): th | admin_bind_host | §12 |
| `S26` | `data_dir` escaped before HOCON interpolation. | — | §12 |
| `TS1` | `Message.height`/`sender_seq` are now `BlockHeight`/`SeqNum`; `message_from_block_metadata` no longer discharges (`casper/src/dag.rs`); `fringe_height` returns `Option<BlockHeight>` (the old `-1` sent | message_from_block_metadata | §2 |
| `TS2` | `Add<NonNegI64>` (the delta carries non-negativity); added `NonNegI64::one()`; call sites use `+ NonNegI64::one()` | — | §2 |
| `TS3` | `Node = [Item; NUM_ITEMS]` (fixed array); `empty_node` = `std::array::from_fn` | std::array::from_fn | §2 |
| `TS4` | reject negative cost (`map_err`); `PCost.cost` is a `uint64`, so a negative (over-charged) cost is an accounting anomaly | — | §2 |
| `TS11` | `checked_sub` returning `Err` on a too-small max size | — | §2 |

