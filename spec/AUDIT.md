# Adversarial audit of the Rust port — findings register

This register records the results of the adversarial audit of the Rust port (per the type-system
commitment in [`TYPE-SYSTEM.md`](TYPE-SYSTEM.md) and the invariant catalog in
[`INVENTORY.md`](INVENTORY.md)). It is the durable record of **what was found, what was fixed, what
was assessed faithful, and what remains**, including every deliberate deviation from the Scala
oracle.

The companion page [`RUST-VS-SCALA.md`](RUST-VS-SCALA.md) explains how the Rust rewrite made these
fragile patterns explicit and why it surpasses the Scala original for production readiness.

Audit dimensions (in the order applied):

1. **Type-system conformance — rules** (partiality, casting, raw bytes, untyped numbers) — machine-gated.
2. **Type-system conformance — spirit** (invariants carried structurally; no type escape).
3. **ρ-calculus mirroring** (internals reflect `Name = @Proc`, `Proc = *Name | …`).
4. **Red-team** (exploits, fragile patterns, DoS) — hardening allowed, each Scala deviation documented.

---

## 1. The machine gate

`tools/audit-type-system.sh` is the authoritative, re-runnable gate. It strips `#[cfg(test)]` blocks
(brace-depth aware), then **fails** (exit 1) on:

- **`panic`** — production `.unwrap()` / `.expect(` / `panic!` / `unreachable!` / `todo!` /
  `unimplemented!`, whitelisting `sdk/src/primitive.rs` (the Scala `getUnsafe` escape hatch); the
  rholang parser's `expect(Tok::…)` method is excluded (a method, not `Result::expect`).
- **`unsafe`** — `unsafe {` (must be zero; the crate graph is entirely safe Rust).
- **`silent`** — `try_into().unwrap()` / `try_into().expect(`, and `unwrap_or(0)` /
  `unwrap_or_default()` on a fallible numeric conversion (a fallible conversion must not be
  silently flattened to 0/Default).

Its `cast`/`lax`/`get` classes are candidate finders (soft reports). Baseline (post Phase-0 widen,
`cast` now includes `as i64`, and a new `lax` class catches `from_str_radix(..).unwrap_or(..)` +
`base16::unsafe_decode`): **`panic`/`unsafe`/`silent` clean**; `cast` = 284 candidates, `lax` = 21,
`get` = 79; `cargo clippy` casting lints (`--all-targets`) = 263 `cast_possible_truncation`, 51
`cast_sign_loss`, 26 `cast_precision_loss`, 49 `cast_lossless`. The remediation targets (all 284 cast
+ 21 lax + raw-byte/newtype bypasses) are the checklist in the ρ-pure remediation plan.

---

## 2. Type-system findings — fixed (genuine violations)

| # | Site | Violation | Fix |
|---|---|---|---|
| 1 | `block-storage/src/dag/{finalizer,message_state,message_map}.rs` + `casper/src/dag.rs` | DAG `Message { height: i64, sender_seq: i64 }` bypassed `BlockHeight`/`SeqNum`; casper discharged its (correct) newtypes back to raw `i64` | `Message.height`/`sender_seq` are now `BlockHeight`/`SeqNum`; `message_from_block_metadata` no longer discharges (`casper/src/dag.rs`); `fringe_height` returns `Option<BlockHeight>` (the old `-1` sentinel) |
| 2 | `shared/src/refined.rs` | `Add<i64> for BlockHeight`/`SeqNum` returned `BlockHeight(self.0 + rhs)` — `zero() + (-1)` silently produced a negative height | `Add<NonNegI64>` (the delta carries non-negativity); added `NonNegI64::one()`; call sites use `+ NonNegI64::one()` |
| 3 | `rspace/src/history/radix_tree.rs` | `type Node = Vec<Item>` with `NUM_ITEMS = 256` — the "exactly 256 slots" invariant implicit in a `Vec`; a short/corrupt node panicked on indexing | `Node = [Item; NUM_ITEMS]` (fixed array); `empty_node` = `std::array::from_fn` |
| 4 | `casper/src/runtime_manager.rs:190` | `u64::try_from(cost.value).unwrap_or(0)` — a negative gas cost silently coerced to 0 | reject negative cost (`map_err`); `PCost.cost` is a `uint64`, so a negative (over-charged) cost is an accounting anomaly |
| 11 | `comm/src/transport/chunker.rs` | `max_message_size - 2048` could underflow (wrap) | `checked_sub` returning `Err` on a too-small max size |

**No type escape** — verified: the refined newtypes (`BlockHeight`, `SeqNum`, `Port`, `Hash32`,
`WireLen`, `NonNegI64`) have no `Deref` impl, no public `.get()`/`.value()`
accessor, and no `.0` field access outside `shared/src/refined.rs`.

---

## 3. Type-system findings — assessed **faithful** (not fixed)

These `as` casts are the Rust equivalent of Scala's fixed-width `Int`/`Long`/`Byte` semantics. The
overflow/truncation cases are unreachable in practice (a `Par` cannot have > 2³¹ fields; a deploy
list cannot exceed 255; config durations/sizes cannot exceed `Long` range). Changing them would
**deviate from the Scala oracle**, so they are documented rather than "fixed".

| Site | Cast | Scala oracle | Assessment |
|---|---|---|---|
| `rholang/matcher/par_count.rs` + `par_spatial_matcher_utils.rs` | `par.sends.len() as i32` | `ParCount` fields are Scala `Int` | faithful |
| `casper/{runtime_replay,runtime_manager,block_creator}.rs` | `i as u8` / `(len + i) as u8` into `split_byte` | `Blake2b512Random.splitByte(Byte)` truncates `Int`→`Byte` | **superseded** — subsequently fixed to checked `u8::try_from` (see §8) |
| `node/configuration/{config_mapper,hocon}.rs`, `node/diagnostics/model.rs` | `as_nanos() as i64`, `(n * mult) as i64`, `as f64 … as i64` | Scala `Long` nanoseconds / `Long` byte counts | faithful |
| `crypto/util/sorting.rs` | `(*x as i8).cmp(&(*y as i8))` | Scala `Ordering.by(Array[Byte].toIterable)` orders **signed** `Byte` | **correct** (doc comment already states this) |

---

## 4. ρ-calculus mirroring

- **Fixed** — `rholang/src/matcher/spatial_matcher.rs:681`: the bipartite-match hook
  `spatial_match_fn(…).ok()?.into_iter().next()` silently swallowed an internal `RholangError`
  (e.g. a Law-5 `BugFoundError`) as "no match". Now the error is recorded and propagated when the
  bipartite search finds no matching.
- **Documented (sanctioned design)** — the flat `Par` ADT **erases** the quote `@`/eval `*`
  distinction (`rholang/src/normalizer.rs:131-145,166-178`); the Name/Proc sort is recovered
  structurally by `classify`/`is_pure_name` (`models/src/types.rs`), per `TYPE-SYSTEM.md` §1.1 and
  the Lean `Par.lean` flat record.
- **Stubbed semantics** (honest inventory for the formal spec): set difference `--`
  (`rholang/src/reduce.rs:516-518`); normalizer `defer(...)` cases — `process` dispatch, `complex
  input source`, `concurrent let` (`normalizer.rs`); `substituteAndCharge`/`Chargeable` deferrals
  (`substitute.rs:5`, `accounting.rs:5`, `storage.rs:88`).
- **Deliberate Scala deviations (determinism):** `New.injections` sorted by key
  (`models/src/sorter.rs:324-327`); `locally_free` excluded from equality/hash via `AlwaysEqual`
  (`models/src/ast.rs:35-77`).

---

## 5. Red-team findings

Severity order; all findings are now **Fixed** (or assessed faithful and documented) — no open
red-team items remain.

### Critical

- **C1 — unauthenticated arbitrary-rholang Repl on `0.0.0.0`.** **Fixed.** The gRPC server is split
  into an **external** (deploy, `40401`) and an **internal** (propose + repl, `40402`) listener; the
  internal listener binds `127.0.0.1` (documented deviation from Scala's `0.0.0.0`). The Repl
  `eval` now enforces a phlo limit (`REPL_PHLO_LIMIT = 1e9`, the reducer aborts with
  `OutOfPhlogistonsError` when exhausted) and a wall-clock deadline (`REPL_EVAL_TIMEOUT = 60 s`).
- **C2 — transport `stream` buffered all chunks before the size breaker.** **Fixed:**
  `grpc_transport_receiver.rs::stream` now enforces `max_stream_message_size` *while* draining.
- **C3 — `send` spawned a task per inbound message, unbounded.** **Fixed:** a `Semaphore`
  (`MAX_CONCURRENT_DISPATCH = 1024`) bounds in-flight dispatches; an exhausted semaphore returns
  `ResourceExhausted`.

### High

- **H1/H2 — unauthenticated propose + deploy flooding (autopropose amplification).** **Fixed.**
  Propose now lives on the loopback-only internal server (H1). The external deploy server applies a
  fixed-window rate limit (`DEFAULT_API_RATE_LIMIT_PER_SEC = 100` req/s, H2) via a tonic
  interceptor; excess requests return `ResourceExhausted`.
- **H3 — global `connections` write-lock held across outbound `send`.** **Fixed.**
  `handle_messages::handle`/`handle_protocol_handshake` now take the `RwLock<Vec<PeerNode>>` and hold
  the write lock only for the brief mutation; the handshake `send` runs *before* the lock is taken,
  so a slow peer can no longer stall `/status` or peer dispatch.
- **H4 — block-request bandwidth amplification.** **Fixed.** `PeerRateLimiter`
  (`DEFAULT_BLOCK_REQUEST_LIMIT_PER_SEC = 100` per peer) throttles `handle_block_request`; excess
  requests are dropped with a log.
- **H5 — `BlockRetriever.requested` map unbounded.** **Fixed.** `MAX_REQUESTED_BLOCKS = 10_000` caps
  the map (new hashes are rejected with `AdmitHashStatus::CapacityReached`), and
  `MAX_WAITING_LIST_PER_HASH = 32` caps the per-hash waiting list.
- **H6 — DAG message-state whole-map clone per insert.** **Fixed (partial).** `Finalizer` now
  borrows the message map (`Finalizer<'a>` holds `&'a BTreeMap`) instead of cloning it on every
  `create_message`; both call sites (`message_state.rs`, `multi_parent_casper.rs`) pass a reference.
  The per-message `seen` reachability cache remains an inherent O(N²) structure (faithful to Scala's
  `seen` cache; capping it would break finalization), documented rather than "fixed".

### Medium

- **M2 — `assert!`/`assert_eq!` in the block-receiver state machine.** **Fixed.**
  `end_stored`/`finished` return `Result<…, String>`; the call sites log the error and skip the block
  instead of panicking a spawned task.
- **M4 — no outbound send timeout; `DEFAULT_SEND_TIMEOUT` was dead.** **Fixed:** unary `send` now
  wraps `tokio::time::timeout(DEFAULT_SEND_TIMEOUT, …)`.
- **M3 — blocking LMDB I/O inside async handlers.** **Fixed.** `KeyValueTypedStoreCodec` offloads
  every store op (`get`/`put`/`delete`/`contains`/`to_map`) to `tokio::task::spawn_blocking` (via
  `blocking_lock`), so fsync'd LMDB transactions no longer run on async worker threads. The
  `rchain-shared` `tokio` feature now enables `rt`.
- **M1 — serialized TLS accept.** **Fixed.** The transport receiver now accepts connections in a
  tight loop and spawns each handshake (bounded `MAX_CONCURRENT_HANDSHAKES = 128`), feeding accepted
  streams through a bounded channel; a stalled handshake no longer serializes inbound connections.
  The `0.0.0.0` bind is *faithful* to Scala (the `protocol-server.host` config is the advertised
  address, not the bind address).
- **M5 — peer-table fillability.** **Fixed.** `update_last_seen` evicts the least-recently-seen
  entry when a bucket is full and every entry is already pending a ping, so a full bucket can never
  saturate permanently.
- **M6 — unauth `/reporting/trace` forceReplay.** **Fixed.** `reporting_trace` returns `404` unless
  `api-server.enable-reporting` is set (the flag is now threaded through `HttpState`).
- **M7 — plaintext-HTTP external-IP discovery.** **Assessed faithful** — Scala uses the same
  plaintext `http://` endpoints (`WhoAmI.scala`). Switching to `https://` requires a TLS-client
  dependency; documented as a low-risk limitation.
- **M8 — bootstrap retry-forever.** **Fixed.** `keep_on_requesting_till_running` gives up after
  `MAX_BOOTSTRAP_RETRIES = 10` attempts, so a dead bootstrap no longer blocks node startup.

**Crypto** is defensive: signature-verify and key parsing return `false`/`Err` on malformed input.
Noted low-severity: `PBKDF2_ITERATIONS = 1024` (`crypto/util/key_util.rs:24`, local-only).

---

## 6. Scala-deviation register

Every place the Rust port deliberately departs from the Scala oracle, with the reason.

| Deviation | Oracle location | Reason |
|---|---|---|
| `New.injections` sorted by key (determinism) | `models/.../rholang/*` | `HashMap` order is non-deterministic in Rust |
| `locally_free` excluded from `Eq`/`Hash` (`AlwaysEqual`) | `models/.../Par.scala` | the cache field is not part of structural identity |
| Negative deploy cost **rejected** (not wrapped to `uint64`) | `accounting/Costs.scala` `toProto` = `PCost(c.value)` | Scala wraps a negative `Long` into `uint64` (latent bug); reject is safer |
| `Add<NonNegI64>` for heights (no negative delta) | — | invariant preserved structurally |
| gRPC `max_decoding_message_size` wired (was 4 MB tonic default) | `defaults.conf` `grpc-max-recv-message-size = 16M` | honors the existing config |
| transport `send` timeout (`DEFAULT_SEND_TIMEOUT`) | `GrpcTransportClient.DefaultSendTimeout` | the constant existed but was unused |
| stream size cap enforced while draining | `StreamHandler.collect` | Scala checks the cap during the fold; the port had moved it after full buffering |
| semaphore-bounded inbound dispatch | per-peer `LimitedBufferObservable` | bounded-queue analog |
| super-majority as exact integer `3·stake > 2·total` | `sdk/consensus/Stake.scala` `stake.toDouble / totalStake > 2d/3` | Law 14 is "strictly > 2/3"; the f64 form loses precision for stakes ≥ 2⁵³ (recorded in §2 of the ρ-pure remediation) |
| `bonds_map`/stake carried as `NonNegI64` (reject negative) | `Message.bondsMap`/`BlockMetadata.bondsMap`/`BlockMessage.bonds` are `Long` in Scala | stake is non-negative by the PoS invariant; negative stakes are rejected at the proto/genesis boundary rather than silently carried as signed `i64` |
| internal gRPC (propose + repl) binds `127.0.0.1` | Scala binds both servers to `0.0.0.0` | unauthenticated propose/repl are no longer network-reachable (C1/H1) |
| external/internal gRPC split (deploy `40401`, propose+repl `40402`) | Scala has the same split (`port-grpc-external`/`internal`) | the port previously put all three services on one listener |
| Repl phlo limit + wall-clock deadline | Scala runs Repl with no limit | a runaway term must not drain the node (C1) |
| deploy gRPC rate limit (100 req/s) | Scala has no limit | bound unauthenticated deploy flooding (H2) |
| per-peer block-request rate limit (100/s) | Scala serves every request | bound block-request bandwidth amplification (H4) |
| `BlockRetriever.requested` capped (10k) + waiting-list capped (32) | Scala map is unbounded | bound peer-advertised hash flooding (H5) |
| `Finalizer` borrows the message map (no clone) | Scala clones the map per call | remove the O(map) clone per message (H6) |
| `connections` write-lock released before outbound I/O | Scala holds the `Ref` across the send | a slow peer must not stall the connection table (H3) |
| concurrent (bounded) TLS handshake accept | Scala serializes accepts on the handshake | a stalled handshake must not stall inbound connections (M1) |
| store ops offloaded to `spawn_blocking` | Scala runs LMDB on the effect runtime | fsync'd LMDB writes must not block async workers (M3) |
| peer-table evicts least-recently-seen when saturated | Scala drops the peer (relies on the ping RPC) | without a ping RPC a full bucket would saturate permanently (M5) |
| `/reporting/trace` gated on `enable-reporting` | Scala reads the flag but does not enforce it | the flag must actually gate the route (M6) |
| bootstrap request gives up after 10 retries | Scala `keepOnRequestingTillRunning` retries forever | a dead bootstrap must not block startup (M8) |
| deploy pool capped (`MAX_POOLED_DEPLOYS = 10_000`) | Scala deploy pool is unbounded | a remote flood must not exhaust the deploy store (R2) |
| stream decompression capped (`content_length ≤ max_stream_message_size`) | Scala `LZ4Compressor` does not bound decompressed size | reject a decompression bomb before allocating (R3) |
| Kademlia RPC rate-limited (100 req/s) | Scala Kademlia ping/lookup are unlimited | bound sybil/routing-table pollution + peer enumeration (R5) |
| exploratory deploy phlo limit (`1e9`) + 60 s deadline | Scala runs exploratory deploy with no limit | a runaway term must not drain a read-only node (R6) |
| private keys written owner-only (`0o600`) | Scala `fs.write` uses default perms | secret material must not be world-readable (R8) |
| rholang parser depth guard (`MAX_PARSE_DEPTH = 512`) | Scala BNFC parser has no depth guard | a deeply-nested term must not overflow the stack (R9) |
| HTTP `/api/deploy` + explore routes rate-limited (100 req/s) | Scala HTTP deploy routes are unlimited | match the gRPC deploy rate limit (R10) |
| PBKDF2 iterations raised `1024 → 310_000` | Scala uses BouncyCastle default `1024` | slow offline brute-force of encrypted keys at rest (R11) |
| RSpace candidate selection is **sorted-first** by content hash (not newest-first insertion order) | `RSpace.scala`/`RSpaceOps.scala` shuffle candidates via `Random.shuffle` before matching | live Scala is non-deterministic across runs; the port selects the sorted-first candidate for consensus. Implemented per `docs/src/node/sorted-matching.md` (changes post-state hashes only for multi-candidate deploys) |
| Block validation replays dependency-free blocks **concurrently** (per-block forked `ReplayRhoRuntime`, batch processor), then inserts serially | Scala `BlockProcessor` validates one block at a time | replay is verify-only (Law 11), so concurrent re-validation does not change the committed state — a throughput optimization, not a semantic change. See `docs/src/formal/concurrency-model.md` |
| LFS sync inserts the downloaded blocks in **ascending** height order (parents before children) | `NodeSyncing.populateDag` `heightMap.flatMap(_._2).toList.reverse` | `BlockDagStorage.insert` requires every justification to already be in the message map, and a justification is always at a strictly lower height; the Scala `reverse` inserts the newest block first and fails with "justification not present in message map" for any fresh observer |
| LFS block requester downloads the **full ancestry chain to genesis** (no `lowerBound`/`extraHeights` cutoff) | `LfsBlockRequester.ST` `lowerBound`/`extraHeights` + `NodeSyncing` `blockHeightsBeforeFringe = deployLifespan` (`populateDag` `minHeight` filter) | a syncing node's DAG is always empty (`NodeLaunch.apply` only syncs when `dagSet.isEmpty`), so the Scala cutoff stops ~50 blocks short of genesis and leaves the lowest downloaded block's justification dangling — the same "justification not present" failure once the fringe is past `deployLifespan` |

---

## 7. Verification

- `cargo check --workspace` — clean.
- `cargo test --workspace --exclude rchain-crypto` — all green (crypto has a pre-existing flaky
  `read_key_pair_round_trips_private_key`).
- `tools/audit-type-system.sh` — zero hard production violations (`panic`/`unsafe`/`silent`).
- New tests: `arithmetic_preserves_non_negativity` (`refined.rs`); existing radix-tree / message-state
  tests cover the array-node and `BlockHeight`/`SeqNum` refactors.

---

## 8. ρ-pure remediation (post prime-directive change)

Under the new oracle (the ρ-calculus spec, not Scala), the following were **fixed**:

- **Consensus super-majority** — `sdk/src/consensus.rs` f64 → exact integer `3·stake > 2·total`
  (Law 14 precision loss for stakes ≥ 2⁵³).
- **Stake → `NonNegI64`** — `Message`/`BlockMetadata`/`BlockMessage`/`compute_bonds`/
  `fringe_bonds_map`/`unsigned_block_proto`/`bonds_parser`/`contracts.Validator.stake`; negative
  stake rejected at the proto/genesis/bonds-file boundary.
- **`split_byte` seed** — `i as u8`/`(len+i) as u8`/`i as u16` → checked `u8::try_from`/`u16::try_from`
  (replay/proposer/reduce), so an oversized deploy list errors rather than wrapping the seed.
- **`BlockData` height/seq** — `BlockHeight`/`SeqNum` carried through; discharge only at the
  `rho:block:data` contract boundary.
- **Raw-byte escapes** — `PublicKey` field closed (private), `KeySegment` `TryFrom<Vec<u8>>` (≤127),
  `NodeIdentifier::from_hex` rejects odd-length/non-hex, `base16::try_decode` added and used at the
  `is_finalized` API boundary.
- **Signed-byte ordering** — `cmp_signed_byte` helper in `crypto::util::sorting` (shared with the sorter).
- **Rholang parser completed** — map-vs-block disambiguation (`{k:v}` was misparsed as a braced
  process), `_` wildcard lexing, `bundle0` → `bundle`+`0`, and multi-receipt `for` desugaring
  (`for (r1; r2; …) { P }` → nested receives). The 9 blessed genesis contracts now parse.
- **Genesis boot fixed** — the deploy normalizer env now binds `rho:rchain:deployerId`/`deployId`
  (`NormalizerEnv::new(deploy)`; it was empty, so the URI was unbound at `eval_new`); the
  `tokio::spawn(node_launch)` result is logged instead of dropped; `create_block_with_processed_deploys`
  `assert!` → `Result`; the runtime uses a 32 MiB worker stack (the blessed terms recurse past the
  2 MiB default).

**Assessed (not fixed — unreachable / boundary / over-engineering):**

- **Length prefixes** (`radix_tree` `& 0x7F`, `certificate_helper` DER, `merging`/`block_random_seed`
  varints+`uint16`, `state/mod`, `scodec`): the lengths are bounded by the wire format (`KeySegment`
  ≤127), the protocol (32-byte hash / 65-byte key), or gas. The `& 0x7F` mask is the 7-bit
  flag+length wire format, not a bug.
- **Config/diagnostics casts** (`as_nanos() as i64`, `(n*mult) as i64`, f64→i64): faithful to Scala
  `Long` nanoseconds/bytes; the truncation needs > 292-year durations or > 2⁶³-byte sizes.
- **API heights** (`block_api_impl` `i32`/`i64` query params, `latest_block_number() -> i64`): the
  `i32` query depth and the potentially-negative lower bound are legitimate API types; `m.height` is
  already `BlockHeight` with discharge at the DTO.
- **DTO boundary** (`deploy_service.rs`, `node/src/api/dto.rs`, `web/*`): `String`/`Vec<u8>` at the
  wire edge is acceptable; validate-on-ingress remains a follow-up (noted, not done).
- **`rho_expr.rs` `unsafe_decode`**: converting `rho_expr_to_par`/`unforg_to_par` to `Result` is a
  follow-up (the recursive `.map` would become `try_collect`).

**Cast triage (Phase 2):** the ~300 `cast` sites were triaged. The overwhelming majority are **faithful
Scala fixed-width equivalents** — matcher `len() as i32` (Scala `Int`), trie `byte as usize`/`u8 as usize`
(widening), `i as u8` `split_byte` (Scala `Byte`), config `as i64`/`as u64`/`as f64` (Scala `Long`),
crypto `*x as i8` (Scala signed `Byte`), diagnostics `as f64` (display), wire-format varint/zigzag.
The consensus-critical path is already exact (`NonNegI64` stakes, `i128` super-majority). Two genuinely
untrusted-input narrowing casts were **fixed**: state-sync `skip`/`take` (`node_running.rs` — negative
`i32` no longer wraps to a huge `usize`) and the store-node-key index (`store_node_key_from_proto` —
out-of-range `i32` no longer truncates via `as u8`). The remaining casts are bounded by the protocol
(32-byte hash / 65-byte key), gas, or block count.

---

## 9. Rust-first reimplementation — fragility audit of the Scala-port rholang layer

This section is the justification for the rust-first reimplementation (plan
`delegated-crafting-phoenix.md`; plans live outside the repo, in the project's `~/.claude/plans/`
directory). It documents *why* the current rholang layer is fragile — not as a
list of bugs to patch, but as the record of the Scala legacy the reimplementation removes. For each
finding: **what it is → why it is fragile/exploitable → how the rust-first rewrite eliminates it**.
Scala remains a *checklist* of required behavior only, never an implementation guide.

- **F1 — The interpreter core is a mechanical Scala port.** `rholang/src/reduce.rs` (1773 lines)
  mirrors `Reduce.scala`/`DebruijnInterpreter`; `normalizer.rs` (1807 lines) mirrors the
  BNFC-derived compiler; `matcher/*` ports the cats-effect `StateT`/`StreamT` monad stack to concrete
  `Vec<FreeMap>` backtracking. *Why fragile:* the effect stack is shoe-horned into `async` +
  concrete collections, and the structural invariants (`locally_free`, `connective_used`) are
  maintained **by hand** (`reduce.rs:35-59`, `substitute.rs`) rather than carried by the type. A
  single inversion (the `normalize_contr` formal-order reversal, fixed in `5ae8dc4df`) silently broke
  list-as-channel matching — latent bugs are invisible until one contract exercises them. *How the
  rewrite eliminates it:* the interpreter is re-derived from the 25 laws (`INVENTORY.md`) and the
  grammar in `RHO-CALCULUS.md`, with the `Par<S>` sort split and the `Closed`/`WellScoped`/
  `BindsAtMostOnce` refinements carrying the invariants structurally (Phase 4).

- **F2 — The blessed genesis contracts re-implement a HashMap trie in interpreted rholang.**
  `casper/src/genesis/resources/Registry.rho:80-368` is a depth-4 keccak-256 nybble trie with 16-bit
  power-of-two bitmasks, built on `@[node, *storeToken]` list-channels and `@(map, "depth")`
  tuple-channels. *Why fragile:* it stresses every exotic interpreter feature at once — peek `<<-`,
  persistent `!!`, list/tuple channels, method calls, keccak trie hashing, bitmask arithmetic — and
  any one of them failing makes the registry **silently empty** (the `process_deploy` path only
  surfaces *reported* errors, `runtime_manager.rs:198`). *How the rewrite eliminates it:* the
  registry/PoS/vault become **native Rust system processes** over a native `BTreeMap` state, exposed
  on the same `rho:*` protocol but with no rholang trie to execute (Phases 1–3).

- **F3 — Silent partiality hides the failure.** `compute_bonds` (`casper/src/runtime_manager.rs:503-509`)
  runs an exploratory deploy (`BONDS_QUERY_SOURCE`, `:547-552`); a non-matching receive yields 0
  results → an empty bond map → "Incorrect number of results: 0", with no indication *which* link
  broke (registry lookup? PoS `getBonds`? the `for(@(_, Pos) <- poSCh)` pattern?). *Why fragile:*
  three separate reductions must all succeed for a single fact (the bonds map) to be observable, and
  the failure mode is a count mismatch rather than a typed error. *How the rewrite eliminates it:*
  `compute_bonds` becomes a single native read (`HistoryReader::get_native(PREFIX_POS, …)`), total and
  typed (Phase 2).

- **F4 — Gas metering is unwired.** `ChargingRSpace` (`rholang/src/storage.rs:88`) is a pure
  passthrough (storage/event charging deferred); `substituteAndCharge` (`substitute.rs:5`) and the
  proto-size cost table (`accounting.rs:5`) are deferred; `Chargeable` has no instances. *Why
  fragile:* the node's primary DoS defense (phlo) is not actually enforced against untrusted deploy
  work. *How the rewrite eliminates it:* `substituteAndCharge`, the proto-size `Costs`, `Chargeable`,
  and `ChargingRSpace` storage/event charging are implemented (Phase 4.7).

- **F5 — Scala-specific encodings leaked into the port.** The CRC14 + 270-bit `ZBase32` registry URI
  (a Scala-specific `org.lightningj.util` dependency) was carried into the port before being dropped
  for the rust-first `rho:id:` + z-base-32 encoding (`rholang/src/registry.rs`); the `.rho`/`.rhox`
  headers still document the stale 55-char Scala URIs. *Why fragile:* non-spec, non-reproducible
  encodings coupling the state hash to a JVM library. *How the rewrite eliminates it:* the URI
  derivation is spec-driven and self-consistent, and the blessed contracts are demoted to checklist
  fixtures (Phase 3).

**Status:** the reimplementation is tracked by the plan `delegated-crafting-phoenix.md` (plans live
outside the repo, in `~/.claude/plans/`); each phase closes the corresponding finding (F1→Phase 4,
F2/F3→Phases 1–3, F4→Phase 4.7, F5→Phase 3).

---

## 10. Full-system HAZOP

A guideword analysis over the whole crate graph. **Guidewords** (domain-adapted): **No/Not** (missing
check/field), **More** (unbounded / overflow / amplification), **Less** (truncation / underflow /
negative), **As well as** (extra unvalidated input), **Part of** (partial data), **Reverse** (ordering
/ sign flip), **Other than** (wrong identity / type / field), **Early/Late** (TOCTOU / expiry),
**Before/After** (state ordering / race). Each Safeguard references the finding it closes (R1–R12, or
the prior C1–C3/H1–H6/M1–M8 register).

| Node | Parameter | Guideword | Consequence | Safeguard |
|---|---|---|---|---|
| N1 Transport | `content_length` (decompressed) | More | i32::MAX allocation → OOM | cap ≤ `max_stream_message_size` (R3) |
| N1 Transport | compressed stream bytes | More | unbounded buffering | cap enforced while draining (C2) |
| N1 Transport | inbound dispatch concurrency | More | unbounded tasks | semaphore 1024 (C3) |
| N1 Transport | TLS handshake concurrency | More | serialized accept | bounded 128 (M1) |
| N1 Transport | peer send | Late | no timeout | 5 s `send` timeout (M4) |
| N1 Transport | key file mode | Other than | world-readable key | `0o600` write (R8) |
| N2 Deploy | deploy signature | No | unsigned deploy accepted on gRPC | verify at ingress (R1) |
| N2 Deploy | deploy pool size | More | disk exhaustion | cap 10k (R2) |
| N2 Deploy | `phlo_limit × phlo_price` | More | i64 wrap → gas bypass | `checked_mul` (R4) |
| N2 Deploy | `phlo_limit` sign | Less | negative limit | reject at ingress (R4) |
| N2 Deploy | HTTP deploy rate | More | flood | HTTP limiter (R10) |
| N3 Consensus | `attestation_stake`/`total_stake` sum | More | i64 overflow → false majority | accumulate in i128 (R7) |
| N3 Consensus | super-majority comparison | More | f64 precision loss | exact `3·stake > 2·total` (Law 14) |
| N3 Consensus | equivocation (`seq_num` reuse) | As well as | double block by a sender | rejected before write (H-1) |
| N4 Crypto | PBKDF2 iterations | Less | fast offline brute-force | 310 000 (R11) |
| N4 Crypto | signature/key parse | Other than | panic on malformed input | `false`/`Err` (defensive) |
| N5 State/replay | LMDB I/O in async | Before/After | blocked workers | `spawn_blocking` (M3) |
| N6 Parser | parse nesting | More | stack exhaustion | `MAX_PARSE_DEPTH` (R9) |
| N6 Parser | `from_slice` length | Other than | panic on wire input | `TryFrom<&[u8]>` (R12) |
| N6 Parser | hex decode | As well as | lax non-hex decode | `try_decode` at ingress (R12) |
| N7 Discovery | ping/lookup rate | More | table pollution / sybil | rate limit 100/s (R5) |
| N7 Discovery | routing `(id, host, port)` | Other than | arbitrary outbound conn | enforced at mTLS handshake (mTLS) |
| N8 RSpace | radix node slots | Less | short node panic | fixed `[Item; 256]` (finding 3) |
| N9 Cost/gas | exploratory deploy | More | unbounded phlo/time | phlo cap + deadline (R6) |
| N9 Cost/gas | gas metering | No | phlo not enforced | charging implemented (F4) |

---

## 11. Red-team re-audit findings (pass 2)

A fresh attacker's-eye pass over the network surface (this session). Severity order; **all fixed** in
this pass. Each entry: **site → root cause → fix → classification** (pure bug fix vs documented Scala
deviation) → verification.

### Critical (P0)

- **R1 — deploy signature not verified on the gRPC path.** `casper/src/api/block_api_impl.rs::deploy`
  checked read-only/shard/forbidden-key/phlo-price but never the signature; the HTTP path verifies
  (`node/src/api/conversion.rs::to_signed_deploy` → `Signed::from_signed_data`). **Fix:**
  `SignedDeployData::verify_signature` (`models/src/casper/protocol/casper_message.rs`) recomputes the
  same hash/verify as the HTTP path, and `BlockApiImpl::deploy` rejects an invalid signature first.
  **Pure bug fix.** Verified: `verify_signature_accepts_valid_deploy` / `_rejects_tampered_term` /
  `_rejects_unknown_algorithm`.
- **R2 — unbounded deploy pool.** `casper/src/dag.rs::add_deploy` put without bound (keyed by
  signature). **Fix:** `MAX_POOLED_DEPLOYS = 10_000` cap via a new `count()` on `KeyValueTypedStore`
  (O(1) `num_records` on the byte store). **Documented deviation.** Verified:
  `add_deploy_rejects_when_pool_full`.
- **R3 — lz4 decompression bomb.** `comm/src/transport/stream_handler.rs::decompress_content`
  allocated attacker-controlled `content_length` before `lz4_flex::block::decompress`; the receiver
  capped only compressed bytes. **Fix:** thread `max_decompressed_size` into `restore`/`decompress_content`
  and reject before allocating (`grpc_transport_receiver.rs` passes `max_stream_message_size`).
  **Documented deviation.** Verified: `restore_rejects_oversized_decompressed_content`.
- **R4 — unchecked phlo multiply.** `models/.../casper_message.rs::total_phlo_charge` did
  `phlo_limit * phlo_price` in i64 (wrap/panic); `runtime_replay.rs::refund_amount` repeated it.
  **Fix:** `Option<i64>` via `i128::checked_mul`; propagate `Err` at the two pre-charge sites; clamp
  the refund; reject negative `phlo_limit` at ingress. **Pure bug fix.** Verified:
  `total_phlo_charge_does_not_wrap_on_overflow`.

### High (P1)

- **R5 — plaintext unauthenticated Kademlia discovery on `0.0.0.0:40404`.** Only a non-secret
  `network_id` gates ping/lookup; peers inject arbitrary `(id, host, port)`. **Fix:** rate-limit the
  RPC (`DEFAULT_KADEMLIA_RATE_LIMIT_PER_SEC = 100`) via the shared `RateLimiter`. The `0.0.0.0` bind
  is kept (faithful to Scala and to the transport's own bind convention; the discovered external IP is
  not a local bind address, and the routing table only affects peer discovery, not consensus safety).
  **Documented deviation** with the residual "plaintext + non-secret network_id" risk noted.
- **R6 — unbounded `exploratory_deploy`.** `casper/src/runtime_manager.rs::capture_results` ran
  `evaluate` with no phlo/timeout (reachable on public read-only nodes). **Fix:** mirror the Repl
  bound — `EXPLORATORY_PHLO_LIMIT = 1e9` + `EXPLORATORY_EVAL_TIMEOUT = 60 s`. **Documented deviation.**
- **R7 — i64 stake-sum overflow.** `casper/.../proposer.rs` and `block-storage/.../finalizer.rs`
  summed `NonNegI64` stakes into `i64` before the super-majority comparison. **Fix:** accumulate in
  `i128`; widen `sdk/src/consensus.rs::is_super_majority` to `(i128, i128)`. **Pure bug fix.**
  Verified: `i64_overflowing_stakes_do_not_wrap`.

### Medium (P2)

- **R8 — private keys/certs written with default perms.** `generate_certificate_if_absent.rs`,
  `bonds_parser.rs`, `key_util.rs` used `fs::write` (default `0666 & ~umask`) for secret material.
  **Fix:** `crypto::util::key_util::write_private_key` (owner-only `0o600`) at all three sites.
  **Documented deviation.**
- **R9 — rholang parser has no recursion-depth guard.** `rholang/src/parser.rs` recursed without bound
  (≤16 MB terms via gRPC). **Fix:** `MAX_PARSE_DEPTH = 128` + `Parser::with_depth` wrapping the
  recursive-descent roots (`parse_proc`, `parse_proc1`, `parse_proc10`, `parse_proc15`, `parse_name`).
  **Documented deviation.** Verified: `rejects_excessive_nesting_depth`.
- **R10 — HTTP `/api/deploy` not rate-limited.** Only the gRPC deploy server was rate-limited.
  **Fix:** shared `RateLimiter` promoted to `rchain_shared`; `HttpState.deploy_rate_limiter` gates
  `api_deploy`/`api_explore_deploy`/`api_explore_deploy_by_block_hash` (`429`). **Documented deviation.**

### Low (P3)

- **R11 — `PBKDF2_ITERATIONS = 1024`.** **Fix:** raised to `310_000` (OWASP PBKDF2-HMAC-SHA256).
  **Documented deviation** (interop caveat: keys written with the new count are not readable by
  BouncyCastle-1024 tooling and vice-versa).
- **R12 — validate-on-ingress `from_slice` asserts.** `models/{block_hash,block/state_hash,validator}.rs`
  `from_slice` panicked on wrong-length wire input (whitelisted in the gate); `BlockHash::from_hex`
  used lax `unsafe_decode`. **Fix:** `TryFrom<&[u8]>` checked constructors (a new `ModelsError::Length`
  variant) + `BlockHash::try_from_hex` (via `base16::try_decode`); the `/reporting/trace` ingress now
  returns 400 instead of panicking. Completed in the follow-up pass: the wire-ingress decoders
  (`BlockMessage`/`BlockMetadata`/`FringeData`/`FinalizedFringe`/`BlockHashMessage`/`StoreItemsMessage*`/
  `SystemDeployData` `from_proto`/`from_bytes`) and the API hex-query sites (`block_api_impl`,
  `deploy_grpc_service_v1`) now use `TryFrom` and return `Result`; `Blake2b256Hash::from_byte_array`
  gained a `TryFrom<&[u8]>` and the state-sync `StoreItemsMessage` path uses it. The panicking
  `from_slice`/`from_byte_array` are now reachable only from internally-produced fixed-width data
  (internal invariants). **Pure bug fix.**

**Verification (this pass):** `cargo check --workspace` clean; `tools/audit-type-system.sh` zero hard
violations (the `expect(Tok::…)` parser exclusion was widened to cover the `with_depth` receiver `p`);
`cargo test` green for `rchain-models`/`rchain-sdk`/`rchain-comm`/`rchain-rholang`/`rchain-casper`/
`rchain-node`; `rchain-crypto` green except the pre-existing flaky
`read_key_pair_round_trips_private_key` (shared temp-dir race, unrelated); `cargo clippy --workspace
--all-targets` clean on the changed files. The parser guard is `MAX_PARSE_DEPTH = 128` (512/256
overflowed the 2 MiB test-thread stack during recursion; 128 is stack-safe and far deeper than any
real rholang term).

---

## 12. Security remediation (pass 3)

A third, fresh red-team pass (attacker's-eye over crypto / interpreter / P2P / consensus-storage /
HTTP-config) surfaced ~40 new findings beyond §5/§10/§11. This section records the remediation. Each
finding is either **fixed** (code change + regression test) or **documented** (assessed faithful /
residual — the accepted outcome for changes that would otherwise break the ρ-calculus/Scala oracle or
consensus determinism). The machine gate (`tools/audit-type-system.sh`) and clippy were already clean;
these are logic/crypto/resource-exhaustion/identity-binding issues, not memory-safety issues.

Decisions (confirmed): ECDSA high-S malleability is fixed by **normalizing `s → low-S` at the deploy
dedup key** (verify semantics unchanged); the P2P `header.sender`-not-bound-to-TLS gap is addressed by
**pragmatic mitigation** (bounds + endpoint validation) with the residual documented; changes are
committed and pushed to `origin/dev`.

### Fixed (bug fixes)

| # | Site | Fix |
|---|---|---|
| S1 | `crypto/util/certificate_helper.rs` | `encode_signature_rs_to_der` rejects RS length ≠ 64; `der_integer` guards empty input — closes the `secp256k1:eth` 1-byte-sig remote panic. |
| S2 | `crypto/util/certificate_helper.rs` | `decode_signature_der_to_rs` validates `end`/integer lengths **before** slicing — no panic on crafted DER. |
| S3 | `crypto/encryption/curve25519.rs` | `to_public`/`secret_key_from` return `CryptoError::InvalidLength` instead of `copy_from_slice` panic. |
| S4 | `crypto/signatures/signatures_alg.rs` | `normalize_signature_low_s` (DER + raw-RS) canonicalizes `s → n−s` when high; idempotent, never panics. |
| S5 | `rholang/src/reduce.rs` | `EDiv`/`EMod` reject `l == i64::MIN && r == -1` — no `MIN / -1` / `MIN % -1` panic. |
| S6 | `rholang/src/accounting.rs` | `charge` clamps the balance at 0 on exhaustion (no negative cell). |
| S7 | `rholang/src/reduce.rs` | Receive continuation body uses `substitute_par_and_charge` (deviation: Scala charges). |
| S8 | `rholang/src/parser.rs` | `MAX_CHAIN_LENGTH = 512` guard on the ten flat operator/pipe/conjunction/method chains — no depth-N left-leaning AST. |
| S9 | `comm/transport/grpc_transport_receiver.rs` | `MAX_CONCURRENT_STREAMS = 1024` semaphore on the inbound `stream` RPC. |
| S10 | `comm/transport/grpc_transport_client.rs` | client `stream` wrapped in `DEFAULT_SEND_TIMEOUT`; `channels` cache capped (`MAX_CACHED_CHANNELS = 1024`). |
| S11 | `comm/rp/connect.rs` + `handle_messages.rs` | `connections` capped (`MAX_CONNECTIONS = 1024`); residual identity-binding gap documented. |
| S12 | `comm/discovery/grpc_kademlia_rpc_server.rs` | reject private/loopback/link-local/unspecified discovery endpoints (SSRF). |
| S13 | `comm/upnp/gateway.rs` | `is_safe_url` rejects loopback/link-local/unspecified/multicast (allows RFC1918); bodies capped at 64 KiB. |
| S14 | `comm/who_am_i.rs` | external-IP body capped at 8 KiB. |
| S15 | `casper/multi_parent_casper.rs` | `phlo_price` result is honored — below-min-price blocks rejected (deviation from Scala `recoverWith`). |
| S16 | `block-storage/dag/metadata_store.rs` | `validate_dag_state` returns `Result` instead of `assert!`; propagated at both call sites. |
| S17 | `casper/blocks/block_receiver.rs` | block-store `put` error is logged and the block skipped. |
| S18 | `casper/engine/node_running.rs` + `node/runtime/node_runtime.rs` | `incoming_blocks` is a bounded channel (`MAX_PENDING_BLOCKS = 1024`, `try_send`); receiver side re-typed. |
| S19 | `casper/blocks/block_processor.rs` | validation-failed / internal-error blocks are no longer re-broadcast. |
| S20 | `casper/block_random_seed.rs` | `bytes.len() as u8` → `u8::try_from` (no truncation). |
| S21 | `casper/api/block_api_impl.rs` | negative `depth` rejected (listen-at-name); `visualize_dag` clamps `start_block_number ≥ 0`; block-range check uses `checked_sub`. |
| S22 | `casper/api/block_report_api.rs` | `block_lock_map` bounded (`MAX_LOCKED_BLOCKS = 4096`, oldest evicted). |
| S23 | `casper/validate.rs` | `repeat_deploy` keys the dedup set on `normalize_signature_low_s` (malleability). |
| S24 | `node/api/grpc/tonic.rs` + `node/web/{http,transaction}.rs` | `getEventByHash` and `/api/transactions/:hash` gated on `enable-reporting`. |
| S25 | `node/web/http.rs` | `/api/v1/propose` `GET → POST`; admin CORS gated by `--api-enable-devnet-cors`; admin HTTP binds `api-server.host` (matches Scala — reverts the earlier loopback-only bind so a browser wallet can reach `/api/propose` through a published port); report/replay routes rate-limited. |
| S26 | `node/configuration/configuration.rs` | `data_dir` escaped before HOCON interpolation. |

### Documented (assessed faithful / residual)

- **ECDSA/Ed25519 high-S acceptance** (`crypto/signatures/{secp256k1,ed25519}.rs`) — verify stays
  faithful to the Scala/libsecp256k1 oracle; malleability is neutralized at the dedup key (S4/S23).
- **Synchronous COMM recursion** (`rholang/reduce.rs`) — mirrors the Scala synchronous interpreter; a
  depth cap would be a semantic deviation. Residual: a funded deployer can still overflow the stack
  before gas exhaustion.
- **Matcher CPU uncharged** (`rholang/reduce.rs:449,1491`) — bounded by `MAX_SPLIT_COMBINATIONS`.
- **Deployer-declared `phlo_limit` / `i32::MAX` default pool** (`rholang/runtime.rs`) — gas is
  economic, not a hard bound.
- **Cost-metadata arithmetic overflow** (`rholang/accounting.rs`) — deploy-size-bounded.
- **Validation-failed block wedging its sender's seq** (`casper/dag.rs`) — liveness edge; changing the
  equivocation gate risks safety.
- **Genesis-bonds path trusts peer bonds** (`casper/interpreter_util.rs`) — cross-checked by
  `bonds_cache`.
- **Unpruned block store / DAG map** — inherent chain history; the ingress queue is bounded (S18).
- **`--validator-private-key` visible in `/proc/<pid>/cmdline`** — config design.
- **Error-body echo** (`node/web/http.rs`) — mostly attacker-input reflection.
- **Fixed-window rate limiter burst** (`shared/rate_limiter.rs`) — per-server, not per-source.
- **Missing key zeroization / `Debug` on `PrivateKey`** — deferred hygiene.

### Verification

- `cargo check --workspace` — clean (only the pre-existing `nom v4.2.3` future-incompat notice).
- `tools/audit-type-system.sh` — zero hard violations (`panic`/`unsafe`/`silent`).
- `cargo test --lib` for `rchain-crypto` (52), `rchain-comm` (59), `rchain-casper` (110),
  `rchain-block-storage` (17), `rchain-rholang` (72) — all green, including the new regression tests
  (`division_and_modulo_overflow_are_errors`, `normalize_signature_low_s_*`,
  `verify_short_eth_signature_returns_false_without_panicking`).

### Dependencies (cargo audit)

`cargo audit` (RustSec, 1225 advisories) surfaced 5 transitive vulnerabilities, all in the network
stack; fixed by dependency changes in `node/Cargo.toml`:

- **h2 0.4.15 → 0.4.16** (RUSTSEC-2026-0258, unbounded empty DATA frames DoS) — patched in the
  `tonic`/`axum` server stack.
- **reqwest 0.11 → 0.12** — removed the old `hyper 0.14`/`rustls 0.21` stack and with it
  **h2 0.3.27** (same DoS) and **rustls-webpki 0.101.7** (RUSTSEC-2026-0098/-0099/-0104:
  name-constraint + CRL-parse issues). `influxdb.rs` client API is unchanged.
- **hocon 0.9 `default-features = false`** — dropped `url-support` (the node parses HOCON text only,
  never URLs), eliminating hocon's `reqwest 0.11` dependency.

**Unmaintained-crate warnings** (informational, no known vulns):

- **rustls-pemfile 2.2.0 — fixed.** Migrated `comm` to the maintained `rustls-pki-types` API
  (`rustls::pki_types::pem::PemObject`: `CertificateDer::pem_reader_iter` /
  `PrivateKeyDer::from_pem_reader`), removing the `rustls-pemfile` dependency. `comm/Cargo.toml` +
  `hostname_trust_manager.rs`.
- **encoding 0.2.33 — accepted residual.** Pulled transitively via `hocon 0.9 → java-properties 1.4`
  (legacy charset handling for `.properties` files; the node parses UTF-8 HOCON text only). The only
  removal path is swapping `hocon` for the third-party `hocon_` fork (0.10.17, 4 minor versions
  diverged, uses `reqwest 0.13`/`java-properties 2.0`) — a disproportionate risk to the node's
  critical config-parsing startup path for a non-vulnerability. Documented rather than swapped.

Note `Cargo.lock` is gitignored in this repo, so the audited dependency set is not pinned in git; a
fresh build re-resolves within the `node/Cargo.toml` constraints (which already force the patched
versions).

## 13. Full red-team audit (pass 4)

Four parallel red-team agents re-audited the workspace against four axes — fragile code, exploits/DoS,
atomicity/determinism, and production-blockchain patterns — cross-checked against §5/§9/§10/§11/§12 to
avoid re-reporting known items, and focused on regressions from `58ca075ec`..`5186361dc`. Severity
reuses §11's P0–P3 model. Findings are deduplicated across clusters.

### Critical (P0)

- **R13 (F1) — decompressed-blob memory amplification on the `stream` path.** `comm/src/transport/grpc_transport_receiver.rs:173-217` buffers up to `max_stream_message_size` (default 256 MiB) per stream, then `tokio::spawn(handle_streamed)` with no concurrency bound; `handle_streamed` blocks on a 50-capacity queue holding its up-to-256-MiB blob. A peer sending LZ4-compressed zeros saturates memory with a few dozen streams. **Fixed** — a `blob_slots` semaphore (`MAX_CONCURRENT_BLOBS = 16`) is acquired before the spawn and held across `handle_streamed`; excess blobs are rejected. (The 256-MiB per-stream default remains a config concern.)

### High (P1)

- **R14 (F2) — unbounded concurrent TLS handshakes.** `grpc_transport_receiver.rs:248-263` spawns one accept task per TCP connection with no timeout; the 128-slot channel bounds only *completed* handshakes. A peer opening thousands of idle connections holds sockets/rustls state until TCP timeouts. **Fixed** — a `handshake_slots` semaphore (`MAX_CONCURRENT_HANDSHAKES`) is acquired before each spawn (excess connections are dropped), and `acceptor.accept` is wrapped in a 10 s timeout.
- **R15 (C1) — unbounded block-validation pipeline.** `node/src/runtime/node_runtime.rs:533` feeds replay validation through an `unbounded_channel`; the bounded ingress (S18) is upstream of it, so a peer streaming valid-signed blocks fills memory faster than replay drains. **Fixed** — the processor-input channel is now `mpsc::channel(MAX_PENDING_BLOCKS)` with backpressure (`send().await`), and `block_processor::apply` takes the bounded `Receiver`.
- **R16 (C2) — unbounded `StoreItemsMessageRequest.take`.** `casper/src/engine/node_running.rs:307` bounds only the *sign* of `skip`/`take`; `take=i32::MAX` triggers a full-trie traversal + giant reply, repeatable per peer. **Fixed** — `take` is capped at `MAX_STORE_ITEMS_TAKE = 10_000`; oversized requests are dropped.
- **R17 (A1/C8) — faucet rate limit is global, no per-source/address budget.** `node/src/web/http.rs:44,127-136` + `web_api_impl.rs:89` use one shared `RateLimiter` (1/s); a single caller drains the genesis dev wallet at 0.3 REV/s and monopolizes the budget. **Fixed** — a per-address drip budget (`FAUCET_MAX_DRIPS_PER_ADDRESS = 10`) in `WebApiImpl`; per-source IP buckets remain a devnet-only refinement.
- **R18 (E1) — `CostAccounting.log` grows unboundedly.** `rholang/src/accounting.rs:342,390` appends a `Cost` (with a heap `String` op) per `charge` and never clears; `total_charged()` (`:363`) re-sums the whole log per deploy, becoming O(n) and able to wrap i64. **Fixed** — replaced the `Vec` with a running `AtomicI64` total.

### Medium (P2)

- **R19 (C6/A2/E5) — `exploratory_deploy` reads the non-finalized chain tip.** `casper/src/api/block_api_impl.rs:626-638` (commit `5186361dc`) reads `height_map.iter().next_back()` (first hash at max height, i.e. an arbitrary fork) instead of `last_finalized_block`. A byzantine tip block can spoof wallet `getBalance`/explore results, and forks make reads node-dependent. **Fixed** — restored `last_finalized_block` as the no-hash default; latest-tip reads remain available via `explore-deploy-by-block-hash` with an explicit hash.
- **R20 (A3/E6) — `revVault transfer` unchecked i64 add + self-transfer guard before the balance check.** `rholang/src/system_processes.rs` — `i64::from(to_balance) + i64::from(amount)` overflows on extreme balances; and the self-transfer guard (commit `204d98656`) returns success before checking `amount ≤ balance`. **Fixed** — `checked_add`/i128 accumulation, and the guard now sits after the balance check.
- **R21 (E3) — arithmetic panic on `EMult`/`EPlus`/`EMinus`/`ENeg`.** `rholang/src/reduce.rs:265,367,395,247` use raw `l*r`/`l+r`/`l-r`/`-hs` on `GInt`; `i64::MAX * 2` panics the reducer in debug builds. **Fixed** — `wrapping_*` (release wrap is Scala-faithful; the debug panic was not).
- **R22 (E4) — number-channel merge/diff unchecked i64.** `rholang/src/merging.rs:100,304` (`init_num + diff`, `end_val - prev`) wrap/panic and write a corrupted value into the trie. **Fixed** — `checked_add`/`checked_sub` with an error.
- **R23 (E2) — `slice` charges output length but walks input uncharged.** `rholang/src/reduce.rs:1044,1048` — a recursive contract slicing a large string gets ~16M:1 op/phlo amplification. **Fixed** — `slice` now charges `max(from, until)` (the input walk), not just the output length.
- **R24 (F4) — SSRF filter classifies only IPv4 literals.** `comm/src/rp/handle_messages.rs:27-40` + Kademlia lookup-insertion (`kademlia_node_discovery.rs:45-50`) connect to attacker-chosen hostnames/IPv6. **Fixed (partial)** — `is_local_address` now classifies IPv6 literals (`::1`, `fe80::/10`, `fc00::/7`, multicast); hostname resolution remains a documented residual (DNS-rebinding-prone), and the Kademlia lookup-insertion path still needs the same filter.
- **R25 (F3) — global channel cache mutex held across an unbounded connect.** `comm/src/transport/grpc_transport_client.rs:94-108`. **Assessed — false positive.** `create_channel` uses `connect_with_connector_lazy`, so the actual `TcpStream::connect`+TLS is deferred to first use and is bounded by `DEFAULT_SEND_TIMEOUT` in `send`; the cache mutex is held only for the fast lazy-channel construction.
- **R26 (F5) — `stream` size cap counts only data bytes.** `grpc_transport_receiver.rs:173-184` — empty `Chunk.content_data` never advances `received`, so unbounded empty chunks grow the per-stream buffer. **Fixed** — a `MAX_STREAM_CHUNKS = 100_000` cap bounds the per-stream chunk count.
- **R27 (C3) — `phlo_price` checked after replay.** `casper/src/multi_parent_casper.rs:351` — a below-min-price block is fully replayed before rejection, so `phlo_price=0` deploys give free replay DoS. **Fixed** — `phlo_price` is now in `block_summary`'s pure-checks, before `validate_block_checkpoint`.
- **R28 (C4) — deploy pool never expires future-dated deploys.** `casper/src/dag.rs:122` — ingress never bounds `valid_after_block_number`, so deploys anchored at `i64::MAX` fill `MAX_POOLED_DEPLOYS` permanently. **Fixed** — `BlockApiImpl::deploy` rejects deploys with `valid_after_block_number` more than `DEPLOY_LIFESPAN` ahead of the tip.
- **R29 (C5) — block-receiver maps unbounded.** `casper/src/blocks/block_receiver.rs:105` — valid-signed blocks with unresolvable justifications are retained forever. **Fixed** — `end_stored` rejects when `blocks_st` reaches `MAX_PENDING_BLOCKS`.
- **R30 (C7) — `PeerRateLimiter` never evicts.** `casper/src/engine/node_running.rs:90` — `BTreeMap<Vec<u8>,(Instant,u32)>` grows with connection churn. **Fixed** — `allow` prunes entries whose window is older than 60 s.

### Low (P3)

- **R31 (F6)** — attacker-influenced UPnP gateway can set the advertised external host (hostname bypasses `is_ssrf_unsafe_host`). `comm/src/upnp/gateway.rs:119-136`.
- **R32 (F7)** — attacker-controlled large `sender.host` retained in the connections table. `comm/src/rp/handle_messages.rs:70-93`.
- **R33 (A4)** — faucet to the deployer's own address is a no-op that still consumes the rate budget and submits a deploy.
- **R34 (A5)** — `/api/faucet` routes are mounted unconditionally on the public router; the dev-mode gate is only inside the handler.
- **R35 (A6)** — a faucet drip is silently dropped once the tip passes `height+50` (`DEPLOY_LIFESPAN`), after `200` was already returned.
- **R36 (A7)** — the single `deploy_rate_limiter` is shared by deploy + explore-deploy, so explore floods starve deploys.
- **R37 (E7)** — play sorts channel data by `Datum.source` but replay keeps store order; correct today, but an undocumented play-vs-replay fragility.

---

## 14. Priority-issue remediation (pass 5)

The GitHub issues #18–#25 were triaged; the highest-severity bugs were fixed in this pass: one
reducer RNG invariant (#19), one RSpace join invariant (#21/#22), and one runtime ownership
invariant (#18/#23).

### Fixed

- **#19 — one-binder persistent receive in a nested `new` does not terminate.** Root cause:
  `resolve_new` (`rholang/src/reduce.rs`) passed the **pre-allocation** RNG state to the new
  body (`Effect::Par(body, new_env, (*rand).clone())`). A nested `new` therefore drew the same
  fresh-name bytes as its parent — `c` collided with the outer `out`, the contract's body sent on
  its own channel, and the persistent receive re-fired until the step budget tripped. **Fix:** the
  body runs with the RNG state advanced past the freshly-allocated names (`Effect::Par(..., r)`).
  **Pure bug fix** (the Scala oracle advances the RNG across `new`). Verified:
  `nested_one_binder_contract_terminates` / `_sequentially`, `flat_one_binder_contract_terminates`
  (`rholang/tests/execution.rs`). The `empty_state` golden vector in
  `rholang/testdata/differential/execution.tsv` changed (`c6a92b38…` → `ece1d876…`) because the
  registry bootstrap contains nested `new`s; the old vector pinned the collision.
- **#21 / #22 — persistent continuations lost their join records after a produce-side COMM.**
  Root cause: `RSpace::process_match_found` (`rspace/src/rspace.rs`) and
  `ReplayRSpace::handle_match` (`rspace/src/replay_rspace.rs`) called
  `remove_matched_datum_and_join` unconditionally. For a persistent continuation the continuation
  itself was left installed while its join records were removed, so the next produce found no join
  and simply stored its datum — a contract served exactly one produce-side call, and the post-state
  diff then failed with `Tuple space inconsistency found: channel of consume does not contain join
  record …`. **Fix:** only the linear path removes joins; the persistent path keeps them and uses
  `store_persistent_data` (remove matched data only), mirroring the consume-side match path.
  **Pure bug fix.** Verified: `persistent_continuation_keeps_join_records_across_produces`
  (`rspace/src/rspace.rs`), `contract_serves_repeated_calls_via_published_name`
  (`rholang/tests/execution.rs`).

- **#18 / #23 — per-request memory retention in exploratory deploys.** Root cause: two `Arc`
  reference cycles kept every forked exploratory runtime (and its whole RSpace/hot-store) alive
  forever: (1) the dispatcher's dispatch-table handlers held a `ContractCall` that held a strong
  `Arc` back to the same dispatcher; (2) the dispatcher's eval closure held a strong `Arc` to the
  reducer, which held the dispatcher. Every `fork_play_runtime` therefore leaked one runtime core
  per request (cancelled or successful). **Fix:** system-process handlers hold the dispatcher
  **weakly** (`ContractCall<ChargingRSpace, Weak<RholangAndScalaDispatcher>>` +
  `Dispatch for Weak<…>`), and the eval closure captures `Arc::downgrade(&reducer)` and upgrades at
  dispatch time. **Pure bug fix.** Verified: `dropping_runtime_releases_its_space`
  (`rholang/tests/execution.rs`) — a dropped `RhoRuntime` now releases its RSpace.
- **#24 — conformance test for qucalc/gov/registry system processes.** Added
  `rholang/tests/system_process_conformance.rs`: nine end-to-end tests call each process from
  rholang by its `rho:*` urn, with the documented argument shapes, and assert the shape of the
  answer — `qucalc:zfa` `(zfa, phase)`, `qucalc:grant` uri / `Nil`, `qucalc:verify` `Bool`,
  `qucalc:fuse` `(geometry, cap)` / `Nil`, `gov:resolveWeights` weight map, `gov:trustLevels`
  level map, `gov:censure` `(discredited, newLevels)`, `gov:tally` ranked + approval winner, and
  `registry:insertArbitrary`/`insertSigned`/`lookup` round-trips. `insertSigned` binds
  `rho:rchain:deployerId` through `evaluate_with_env` (the signed-deploy env shape).

### Still open (triaged, not fixed in this pass)

- **#25** — quoted-name lint. Enhancement, not a bug.

### Verification (this pass)

- `cargo check --workspace` — clean.
- `tools/audit-type-system.sh` — zero hard production violations.
- `cargo test -p rchain-rspace` — 54 passed (incl. the new join-persistence regression).
- `cargo test -p rchain-rholang` — 83 lib + 17 execution + 1 rho_examples + 9
  system_process_conformance passed (incl. the new nested-contract, repeated-call, runtime-drop,
  and urn-conformance regressions).
- `cargo test -p rchain-casper --lib` — 113 passed; `cargo test -p rchain-casper --test determinism` —
  4 passed.
- `cargo clippy -p rchain-rspace -p rchain-rholang --all-targets` — no new warnings on the changed
  files (the pre-existing clone-on-copy / await-holding-lock warnings in test modules remain).

