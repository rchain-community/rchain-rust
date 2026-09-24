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
  distinction (the `VarSort::ProcSort` arms of `rholang/src/normalizer.rs`); the Name/Proc sort is recovered
  structurally by `classify`/`is_pure_name` (`models/src/types.rs`), per `TYPE-SYSTEM.md` §1.1 and
  the Lean `Par.lean` flat record.
- **Stubbed or deferred semantics** (honest inventory for the formal spec; re-verified against the
  tree, since the previous version of this list had itself gone stale — everything it named is now
  implemented). What is deferred **today**, none of it on the reduction path:
  - the `.rho`/`.rhox` genesis-template *loading* (`CompiledRholangSource`/`CompiledRholangTemplate`)
    — only the parameter types and the pure source-string builders are ported
    (`casper/src/genesis/contracts.rs`);
  - three Java/scodec conveniences with no Rust analog in the string/byte syntax helpers
    (`ByteStringSyntax.toDirectByteBuffer`, `toByteVector`, `toBlake2b256Hash`) —
    `models/src/string_syntax.rs`;
  - the Magnolia-derived `Pretty[A]` typeclass — the pure escaping/indentation helpers are ported
    (`models/src/pretty.rs`);
  - the effect-machinery readers (`NodeCallCtxReader`, `VersionInfo.get`'s sbt-buildinfo input) —
    `node/src/runtime/node_call_ctx.rs`, `node/src/web/version_info.rs`.
  Set difference `--` is **implemented** (`rholang/src/reduce.rs:727`, with both error arms), and the
  normalizer's `defer(...)` cases, `substituteAndCharge` and the proto-size `Costs`/`Chargeable`
  instances are all in place (see F4 below, which records the fix).
- **Deliberate Scala deviations (determinism):** `New.injections` sorted by key
  (`models/src/sorter.rs`'s par reconstruction); `locally_free` excluded from equality/hash via `AlwaysEqual`
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
| Integer arithmetic **promotes to `BigInt`** on `i64` overflow and never wraps; mixed `Int`/`BigInt` operands promote | `Reduce.scala` `wrappingAdd`/`wrappingSub`/`wrappingMul`/`wrappingNeg` and `i64::MIN / -1` errors | RCHIP #51: silent overflow/underflow is the bug class behind the PoS incidents ("one may blow up the world"); exact arithmetic removes it. A **hard fork** — previously-wrapped results change, so prior state is invalid |
| Number-channel merge diffs are **checked** (`i64` overflow is an error) — the subtraction, the merge, **and the accumulation** (`EventLogIndex::combine`, `casper/src/merging.rs`'s mergeable-diff sum) | `calculateNumChannelDiff` (`Long` subtraction wraps) and `EventLogIndex.combine` (`+` wraps) | a wrapped diff silently corrupts the merged state (Laws 9 & 17); same class as RCHIP #51 — recorded as issue #52, and the accumulation was the last site in that class (AUDIT C41, fixed: the error now propagates to the merge, which refuses the block). **Hard fork:** on a block whose channel diffs sum past `i64`, the old node computed a *wrapped* diff and a wrong state hash, the new one refuses the merge — so a chain that accepted such a block diverges on it. The wrapped value was already wrong, which is why this is a fix rather than a divergence in behaviour that was ever correct |
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
| `if`'s condition is normalized against an **empty** par (`normalizer.rs::normalize_if`), as `match`'s target is | `PIfNormalizer.scala:24` passes the caller's `input` through, so the target becomes `<the par before the if> \| <condition>`; `PMatchNormalizer.scala:28` — the *same* desugaring — passes `input.copy(par = VectorPar())` | the Scala contradicts itself: its `if` and `match` desugarings of one construct normalize the target differently, and only the `if` path's version is broken (`if E {A} else {B}` = `match E {true => A; false => B}`, and a process's meaning cannot depend on what precedes it in a `par`). Under the Scala's `if` path every non-first `if` is a silent no-op — see **C21**. The port follows the spec and the Scala's `match` path. **Hard fork:** the normal form of any term whose par holds a non-first `if` changes, so a chain that ran the old rule and upgrades diverges on such a deploy (and on genesis, where the effected normal forms *are* genesis content: `ListOps.rho:203,242`, `MultiSigRevVault.rho:155`, and the rgov family) |
| HTTP `/api/deploy` + explore routes rate-limited (100 req/s) | Scala HTTP deploy routes are unlimited | match the gRPC deploy rate limit (R10) |
| PBKDF2 iterations raised `1024 → 310_000` | Scala uses BouncyCastle default `1024` | slow offline brute-force of encrypted keys at rest (R11) |
| `BindPattern.freeCount` **rejected** when negative at the proto boundary | `RhoTypes.proto:124` — `int32 freeCount`, signed, so negative is representable; the vendored tree does not carry the Scala's `BindPattern` proto conversion, so whether the oracle refuses it there could not be established | the count is not descriptive: `RhoMatch::get` fills `0..free_count` from the match's free map, so a negative count silently applies the continuation with *no* bound values (and an over-large one pads with `Nil`). The port's two siblings on the same field family — `ReceiveBind` (`:118`) and `MatchCase` (`:165`) — already validate it, so this is also what makes the three uniform. A refusal at the declared boundary rather than a clamp (AUDIT C52) |
| RSpace candidate selection is **sorted-first** by content hash (not newest-first insertion order) | `RSpace.scala`/`RSpaceOps.scala` shuffle candidates via `Random.shuffle` before matching | live Scala is non-deterministic across runs; the port selects the sorted-first candidate for consensus. Implemented per `docs/src/node/sorted-matching.md` (changes post-state hashes only for multi-candidate deploys) |
| Block validation replays dependency-free blocks **concurrently** (per-block forked `ReplayRhoRuntime`, batch processor), then inserts serially | Scala `BlockProcessor` validates one block at a time | replay is verify-only (Law 11), so concurrent re-validation does not change the committed state — a throughput optimization, not a semantic change. See `docs/src/formal/concurrency-model.md` |
| LFS sync inserts the downloaded blocks in **ascending** height order (parents before children) | `NodeSyncing.populateDag` `heightMap.flatMap(_._2).toList.reverse` | `BlockDagStorage.insert` requires every justification to already be in the message map, and a justification is always at a strictly lower height; the Scala `reverse` inserts the newest block first and fails with "justification not present in message map" for any fresh observer |
| LFS block requester downloads the **full ancestry chain to genesis** (no `lowerBound`/`extraHeights` cutoff) | `LfsBlockRequester.ST` `lowerBound`/`extraHeights` + `NodeSyncing` `blockHeightsBeforeFringe = deployLifespan` (`populateDag` `minHeight` filter) | a syncing node's DAG is always empty (`NodeLaunch.apply` only syncs when `dagSet.isEmpty`), so the Scala cutoff stops ~50 blocks short of genesis and leaves the lowest downloaded block's justification dangling — the same "justification not present" failure once the fringe is past `deployLifespan` |
| Active validator set = the **top-N by descending stake** (key-ascending tie-break) | `Pos.rhox:718-726` `pickActiveValidators` returns the first `$$numberOfActiveValidators$$` entries of `allBonds.toList()` — *key* order, not stake order; its own comment is `// TODO: Randomly select 100 active validators once we have on-chain randomness` | the contract's rule is not a rule to port: "the first N in map order" is a placeholder for a random selection, and it makes the consensus set depend on map iteration order rather than on anything consensus-relevant. The cap exists because finality's supermajority is stake-weighted, so the highest-staked N is the property the contract's TODO is aiming at; the port's choice is deterministic and stake-ordered (`select_active`, `rholang/src/native_state.rs`). The *timing* is unchanged — both apply it only at an epoch boundary (law 44) |
| A slashed validator that had a **staged withdrawal** is removed from the pending map | `Pos.rhox:491` leaves it in `pendingWithdrawers` while zeroing its bond (`:487`), so at the next boundary `movePendingWithdrawer` files it under `withdrawers` with a zero amount, where it stays forever (never paid, never removed — nothing deletes a zero claim) | the payable outcome is identical (zero), and the port does not carry a permanent tombstone: `slash` removes the validator from the pool, the active set, the claim map *and* the pending map. Recorded because it is a state-shape difference a reader would otherwise meet as a missing entry |
| Epoch length `0` (the port's permissive default) means **every block is a boundary**; the contract's `%`/`/` by it would fault | `Pos.rhox:517` `blockNumber % $$epochLength$$`, `:381` `blockNumber / $$epochLength$$`, `:249` `bonds / $$minimumBond$$` | the contract cannot express a zero epoch length or a zero `minimumBond` — it divides by both. The port's default parameters have both at zero (ad-hoc runtimes and tests install no genesis PoS state), so the port defines what the contract leaves as an arithmetic fault: a zero epoch length is the one-block epoch `epochLength = 1` means, and a zero (or zero-normalising) minimum bond pays a reward of zero rather than faulting the block. The Lean model states its conservation theorem for the defined case only (`Rchain.sum_rewards_le_pot`'s hypothesis `0 < activeBonds / minimumBond`), which is the same boundary |
| `deploy-status` has **no `Running`** state: a deploy the node is executing right now answers `Pooled`, or `Unknown` once the pool no longer holds it | `BlockApiImpl.scala:218-224` — `findCurrentlyExecutedDeploy` reads the node's `BlockExecutionTracker` (a per-node cache of the deploys its own block creator is executing) and answers `notProcessed("Running")`, and `findPooledOrRunningDeploy` consults it after the pool | the port has no execution tracker: deploys are pulled out of the pool into `compute_deploys_checkpoint` with no in-flight record, so there is nothing to look up. A derived answer would be a guess — "neither in the DAG nor in the pool" also describes a deploy whose block was just proposed and one dropped with its block — and the window is the length of one proposal. Registered rather than invented: the states a client can act on (pooled, processed with success, processed with error, unknown) are complete, and `Running` only ever meant "the node you asked is busy with it right now" |
| `metrics { prometheus, influxdb, influxdb-udp, zipkin, sigar }` (and the matching `--prometheus`/`--influxdb`/`--zipkin`/`--sigar` flags) are **parsed and never consulted** | `kamon.conf` — where `prometheus { enabled = false }` gates Kamon's scrape endpoint, and the influxdb/zipkin/sigar blocks configure reporters that push | nothing outside `node/src/configuration/` reads `MetricsConf`: the node always serves `GET /metrics` in Prometheus text format, there is no InfluxDB or UDP sender, and the tracing/span backends are out of scope (`diagnostics/mod.rs`, said there). So the switches are inert in both directions — `prometheus = false` does not disable the endpoint and `= true` does not start a reporter. Said once in `defaults.conf` where an operator reads the switch, and recorded here as the second config surface found doing nothing (`disable-state-exporter` was the first, wired in `687fc4b30`) |
| Vaults are a **balance map keyed by REV address**; `transfer` takes the caller's `deployerId` rather than a minted purse | `RevVault.rho:103-140` — `findOrCreate` → `_makeVault` returns a `MakeMint` **purse**, and `transfer`/`deposit`/`getBalance` are called *on the purse* with an `unforgeableAuthKey` | the *spend* rule holds either way: the port derives the `from` account from the caller's unforgeable `deployerId` (`system_processes.rs`'s `transfer`, "capability, not data"), so no deploy can spend another key's vault — what is unforgeable is the deploy's identity rather than a minted name. What the simplification **loses** is *delegation*: a purse could be handed to a contract that then spends from it, and here only the signing key can spend. **Decided 2026-09-23** (Programme B item B2) to keep it: nothing in the tree delegates (the wallet, the faucet, the gateway legs and the genesis ceremony all act as the key itself), and landing it needs the deploy's RNG threaded into a native call — the minted name is a `new`, so it must be replayable — plus a leaf and a client-visible API change. The *reply-shape* half is recorded where a client meets it: `spec/API-SCHEMA.md`'s `rho:rchain:revVault` row is ❌ open for exactly this |
| Storage is **refunded** when a produce/consume matches (the gas a matched op costs) | `ChargingRSpace.scala:105-127` — `refundForConsume` and `refundForRemovingProduces`, charged as negative `Cost`s *before* the event and COMM costs | the port charged the storage and never refunded it, recording the gap as a "safe over-charge"; the refunds are restored (law 49, `spec/Rchain/Charging.lean`). **Hard fork:** the recorded `PCost` of a deploy that matches changes, and cost is part of the block's state — on a chain that accepted such a block the old node recorded a *higher* cost than the Scala's, so the old value was already wrong, which is why this is a fix and not a behaviour that was ever correct. The order matters as much as the amounts: a refund credited after the exhaustion check cannot save a deploy that has already run out (`peak_refunds_first`), which is why the Scala charges them immediately |
| The Coop **multisig public keys** are read as the port's initial **trusted stakeholder** set | `Pos.rhox:122-128` creates the Coop multisig vault from those keys (and `$$posMultiSigQuorum$$`), and `:470-482` sends slashed stake to it; the Scala has no trusted-stakeholder concept at all — that is this port's extension for observer admission (`spec/RUST-FIRST.md`) | a mapping, not an accident (`06bf01a7f`, and the doc comment on `build_pos_genesis` says it): the Coop multisig is the network's governance body in the contract, and admission is the port's governance-shaped hook, so the closest analogue of "who may govern validators" is the keys the contract gives governance to. The alternative reading — that they are only slashing-vault owners, leaving the genesis `trusted` set as the validators alone — is equally supportable, and nothing in either tree says which was intended. Recorded because the consequence is silent: an operator setting these keys for vault control is also granting admission rights. The `--pos-multi-sig-quorum` option has no counterpart at all (no multisig vault), so it stays "Reserved" in its help text |

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

- **`rho_expr.rs` `unsafe_decode` fixed** — `rho_expr_to_par` and `unforg_to_par` now return
  `Result<Par, String>` and decode their hex leaves with `base16::try_decode`, so an invalid byte string is
  an error at the API boundary rather than a silently truncated value (`node/src/api/rho_expr.rs:293,334`;
  commit `2ac5ea931`). This entry was listed here as *not done* until it was checked — the third stale
  claim of this family the audits have found, and the one with the widest blast radius, since an
  unchecked decode is exactly the shape of the defects this document exists to record.

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
  wire edge is acceptable — **and the values behind them are stopped, which this bullet previously
  denied.** Checked field by field rather than re-asserted: `TxnRequest.txn_id` is strict hex of at most
  64 bytes at the handler (`web/http.rs:191-198`), `TxnLegDto.shard_id` goes through
  `ShardId::try_from` with a 400 (`:203-218`), `TxnLegDto.amount` is caught by the *ledger's*
  `NonNegI64` refinement in `GatewayTxn::run` (`casper/src/gateway/mod.rs:277-279`, tested in
  `ledger.rs`), `FaucetRequest.address` by `RevAddress::is_valid` (`web_api_impl.rs:526`), and the
  block-hash strings by `BlockHash::try_from` where they are used. The `depth: i32` this section's
  "API heights" bullet already triaged as legitimate is therefore the only one deliberately unchecked.
  **One field had no check anywhere** — `TxnLegDto.to` — and now has one at the boundary
  (`web/http.rs`), with two tests pinning the ingress behaviour a client sees (the empty `to`, and the
  negative amount the refinement rejects). The original note was a *claim*, and the claim was wrong: the
  boundary was never the problem, only the layer that answers.
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
  rewrite eliminates it:* the interpreter is re-derived from the 29 laws (`INVENTORY.md`) and the
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

- **F4 — Gas metering is unwired.** `ChargingRSpace` (`rholang/src/storage.rs:103`) is a pure
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
- **Key zeroization / `Debug` on `PrivateKey`** — **fixed** (was "deferred hygiene"). `PrivateKey`
  implements `Debug` by hand and prints `PrivateKey(<redacted, N bytes>)` instead of the derived
  output, which printed the raw secret — one `{:?}` in a log line, an error message or a test failure
  was a leaked key — and it zeroes its buffer on `Drop` (`crypto/src/private_key.rs`). Both are
  *mitigations and are documented as such*: `Clone` still copies the secret (23 call sites, audited and
  left alone rather than threading ownership through the signing paths), the allocator may move the
  buffer, and a zeroing write is not guaranteed to survive optimisation. What is removed is the silent
  leak. The redaction is observable and so is pinned; zeroization is not, and has no test that would
  be asserting something it cannot see. Adding `Drop` also surfaced two test sites that moved the key
  bytes out of the value — the compiler refused them, which is the structural half of the same
  hygiene.

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
- **R15 (C1) — unbounded block-validation pipeline.** `node/src/runtime/node_runtime.rs:522` feeds replay validation through an `unbounded_channel`; the bounded ingress (S18) is upstream of it, so a peer streaming valid-signed blocks fills memory faster than replay drains. **Fixed** — the processor-input channel is now `mpsc::channel(MAX_PENDING_BLOCKS)` with backpressure (`send().await`), and `block_processor::apply` takes the bounded `Receiver`.
- **R16 (C2) — unbounded `StoreItemsMessageRequest.take`.** `casper/src/engine/node_running.rs:342-344` bounds only the *sign* of `skip`/`take`; `take=i32::MAX` triggers a full-trie traversal + giant reply, repeatable per peer. **Fixed** — `take` is capped at `MAX_STORE_ITEMS_TAKE = 10_000`; oversized requests are dropped.
- **R17 (A1/C8) — faucet rate limit is global, no per-source/address budget.** `node/src/web/http.rs:44,127-136` + `web_api_impl.rs:89` use one shared `RateLimiter` (1/s); a single caller drains the genesis dev wallet at 0.3 REV/s and monopolizes the budget. **Fixed** — a per-address drip budget (`FAUCET_MAX_DRIPS_PER_ADDRESS = 10`) in `WebApiImpl`; per-source IP buckets remain a devnet-only refinement.
- **R18 (E1) — `CostAccounting.log` grows unboundedly.** `rholang/src/accounting.rs:370,410` appends a `Cost` (with a heap `String` op) per `charge` and never clears; `total_charged()` (`:395`) re-sums the whole log per deploy, becoming O(n) and able to wrap i64. **Fixed** — replaced the `Vec` with a running `AtomicI64` total.

### Medium (P2)

- **R19 (C6/A2/E5) — `exploratory_deploy` reads the non-finalized chain tip.** `casper/src/api/block_api_impl.rs`'s `exploratory_deploy` (commit `5186361dc`) read `height_map.iter().next_back()` (first hash at max height, i.e. an arbitrary fork) instead of `last_finalized_block`. A byzantine tip block can spoof wallet `getBalance`/explore results, and forks make reads node-dependent. **Fixed** — restored `last_finalized_block` as the no-hash default; latest-tip reads remain available via `explore-deploy-by-block-hash` with an explicit hash.
- **R20 (A3/E6) — `revVault transfer` unchecked i64 add + self-transfer guard before the balance check.** `rholang/src/system_processes.rs` — `i64::from(to_balance) + i64::from(amount)` overflows on extreme balances; and the self-transfer guard (commit `204d98656`) returns success before checking `amount ≤ balance`. **Fixed** — `checked_add`/i128 accumulation, and the guard now sits after the balance check.
- **R21 (E3) — arithmetic panic on `EMult`/`EPlus`/`EMinus`/`ENeg`.** `rholang/src/reduce.rs:265,367,395,247` use raw `l*r`/`l+r`/`l-r`/`-hs` on `GInt`; `i64::MAX * 2` panics the reducer in debug builds. **Fixed** — `wrapping_*` (release wrap is Scala-faithful; the debug panic was not).
- **R22 (E4) — number-channel merge/diff unchecked i64.** `rholang/src/merging.rs:97,305` (`init_num + diff`, `end_val - prev`) wrap/panic and write a corrupted value into the trie. **Fixed** — `checked_add`/`checked_sub` with an error.
- **R23 (E2) — `slice` charges output length but walks input uncharged.** `rholang/src/reduce.rs:1264` (`"slice"`) — a recursive contract slicing a large string gets ~16M:1 op/phlo amplification. **Fixed** — `slice` now charges `max(from, until)` (the input walk), not just the output length.
- **R24 (F4) — SSRF filter classifies only IPv4 literals.** `comm/src/rp/handle_messages.rs:27-40` + Kademlia lookup-insertion (`kademlia_node_discovery.rs:45-50`) connect to attacker-chosen hostnames/IPv6. **Fixed (partial)** — `is_local_address` now classifies IPv6 literals (`::1`, `fe80::/10`, `fc00::/7`, multicast); hostname resolution remains a documented residual (DNS-rebinding-prone), and the Kademlia lookup-insertion path still needs the same filter.
- **R25 (F3) — global channel cache mutex held across an unbounded connect.** `comm/src/transport/grpc_transport_client.rs:68-75` (`create_channel`). **Assessed — false positive.** `create_channel` uses `connect_with_connector_lazy`, so the actual `TcpStream::connect`+TLS is deferred to first use and is bounded by `DEFAULT_SEND_TIMEOUT` in `send`; the cache mutex is held only for the fast lazy-channel construction.
- **R26 (F5) — `stream` size cap counts only data bytes.** `grpc_transport_receiver.rs:173-184` — empty `Chunk.content_data` never advances `received`, so unbounded empty chunks grow the per-stream buffer. **Fixed** — a `MAX_STREAM_CHUNKS = 100_000` cap bounds the per-stream chunk count.
- **R27 (C3) — `phlo_price` checked after replay.** `casper/src/multi_parent_casper.rs:298` (`block_summary`) — a below-min-price block is fully replayed before rejection, so `phlo_price=0` deploys give free replay DoS. **Fixed** — `phlo_price` is now in `block_summary`'s pure-checks, before `validate_block_checkpoint`.
- **R28 (C4) — deploy pool never expires future-dated deploys.** `casper/src/dag.rs:122` — ingress never bounds `valid_after_block_number`, so deploys anchored at `i64::MAX` fill `MAX_POOLED_DEPLOYS` permanently. **Fixed** — `BlockApiImpl::deploy` rejects deploys with `valid_after_block_number` more than `DEPLOY_LIFESPAN` ahead of the tip.
- **R29 (C5) — block-receiver maps unbounded.** `casper/src/blocks/block_receiver.rs:101` — valid-signed blocks with unresolvable justifications are retained forever. **Fixed** — `end_stored` rejects when `blocks_st` reaches `MAX_PENDING_BLOCKS`.
- **R30 (C7) — `PeerRateLimiter` never evicts.** `casper/src/engine/node_running.rs:106` — `BTreeMap<Vec<u8>,(Instant,u32)>` grows with connection churn. **Fixed** — `allow` prunes entries whose window is older than 60 s.

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

**Decided (2026-09-23, Programme A item A6): dropped from this pass's scope, and this line is the
record.** The issue was triaged as an enhancement — a lint that would report a name that could be
written unquoted — and it is the only item in the #18–#25 batch that is neither a defect nor a
correctness question. Its body is not available offline (`legacy/` carries no copy and there is no
local issue file), so a port cannot be checked against the intent; keeping it as "still open" would be
the resting-place shape this register exists to avoid. What would revive it: the issue text, or a
decision that the port *wants* the lint for its own sake — in which case it is a new feature in
`rholang/src/parser.rs` with its own tests, not a porting task.

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


## 15. Multi-shard gateway findings (pass 6)

The multi-shard gateway (`casper/src/gateway/`, Laws 26–29) was audited for its own failure paths
while completing its test coverage. One latent bug was found and fixed; one behaviour is a documented
deviation.

### Fixed

- **C1 — a decided coordinator record could be resurrected by a late vote.**
  `casper/src/gateway/ledger.rs::CoordRecord::record_vote` overwrote a leg's vote in place and then
  re-derived the state. An `Abort` recorded for a leg could therefore be overwritten by a later
  `Ready`, and once *every* leg held a `Ready` vote the `else if` branch set the record back to
  `Committed` — resurrecting a transaction whose compensation had already run and whose escrow had
  been returned. Root cause: the phase-one state mapping treated the vote list as mutable input
  rather than as a record of a decision that, once taken, is durable (Law 29). **Fix:** a terminal
  record is absorbing — `record_vote` returns immediately when `state.is_terminal()`, so a decided
  transaction cannot be moved and its votes stay consistent with the state it committed to. The path
  was **latent, not live**: `GatewayTxn::drive` breaks the collection loop on the first abort and
  never re-prepares a terminal record. **Pure bug fix.** Verified: `an_abort_is_absorbing` (fails
  without the fix with `left: Committed, right: Aborted`), `a_commit_is_absorbing` (a late abort
  cannot un-commit; fails without the fix), `record_vote_overwrites_a_repeated_shard_vote`,
  `record_vote_does_not_set_a_reason_for_a_ready_vote`.

### Documented (assessed faithful / residual)

- **C2 — a phase-two failure is discarded.** `casper/src/gateway/mod.rs::apply_phase_two` ignores
  each `commit`/`abort` deploy's outcome (`let _ = self.phase(...)`). Faithful to the coordinator
  model: the decision is already durable (written *before* phase two), a failed delivery is re-driven
  by `recover_in_flight` on the next boot or by a re-issued `run` with the same `txn_id`, and the
  participants are idempotent under `txn_id` (Law 28) — so a lost phase two is not a lost decision.
  The residual is that the record does not distinguish "phase two delivered" from "phase two
  attempted", so a leg whose commit never landed holds its escrow until recovery re-drives it.
  Verified: `a_failed_phase_two_leaves_the_decision_intact` (both legs prepare, the decision is
  written, leg B's commit is rejected, the record stays `Committed`).

- **C3 — the inner replay trace check does not fire for a term tamper; the state-hash comparison
  does.** `casper/src/runtime_replay.rs::check_replay_data_with_fix` returns `Ok(())` when the
  RSpace trace check fails **and** the deploy was not "eval successful" — the deliberate
  RCHAIN-3505 workaround (`// TODO: temp fix for replay error mismatch (RCHAIN-3505)`). Measured:
  replaying a deploy whose processed `data.term` was rewritten to a different term does **not**
  produce a `ReplayFailure`; it produces a *different post-state hash*. The invariant is carried one
  level up, by `casper/src/interpreter_util.rs::handle_errors`, which compares the replayed hash
  against the block's claimed `post_state_hash` and returns `Ok(None)` on a mismatch. **Assessed
  faithful** (it is the ported Scala behaviour), recorded because a test written against the inner
  check alone would pass while a tampered block was accepted. Verified:
  `a_tampered_deploy_replays_to_a_rejected_state_hash` (`casper/tests/determinism.rs`) pins both the
  divergence and the rejection — and would fail if the comparison were removed.

  **Decided (2026-09-23, Programme A item A5): kept, and the pin is the invariant's real home.**
  The workaround is deliberate and the invariant it looks like it drops is carried one level up by
  `interpreter_util.rs::handle_errors`, which is what `a_tampered_deploy_replays_to_a_rejected_state_hash`
  exercises end to end. Removing the workaround would make the *inner* trace check stricter than the
  Scala's for a case the state hash already rejects, so it stays, named here and in the test's own
  docstring rather than in a TODO.

- **C4 — a saturated inbound queue is reported to callers as `MessageTooLarge`.**
  `comm/src/transport/grpc_transport.rs::process_error` maps a gRPC `ResourceExhausted` to
  `CommError::MessageTooLarge(peer)` (the ported `processError`), and the receiver answers
  `ResourceExhausted` for **two different causes**: a genuinely oversized message and a saturated
  concurrency bound — the dispatch queue (`MAX_CONCURRENT_DISPATCH`), the stream slots, and the
  decompressed-blob budget. Both fail closed, so this is a diagnostic wart rather than a hazard: an
  operator reading `MessageTooLarge` cannot tell congestion from size, and the refusal's own message
  (`"dispatch queue full"`) is discarded by the mapping. **Documented deviation** (the mapping is
  faithful to Scala; the second cause is the Rust-first DoS bound). Verified:
  `a_full_dispatch_queue_is_rejected_and_recovers` (`comm/src/transport/grpc_transport.rs`) pins the
  refusal *and* the recovery — the bound is a queue, not a latch.

- **C5 — a `ParBody` continuation dispatched with no matched data panics in the random merge.**
  `rholang/src/dispatch.rs` always prepends the continuation's own random state to the matched data's
  random states before calling `Blake2b512Random::merge`, which **asserts at least two inputs**
  (`crypto/src/hash/blake2b512_random.rs`). With zero matched data the list has one element and the
  merge panics — a reducer-path panic rather than a reported error. **Latent, not live**: the reducer
  never dispatches a `ParBody` with empty data today (a receive always matches at least the datum that
  triggered it, and a match with nothing to run becomes `TaggedContinuation::Empty`, which is a
  deliberate no-op). Recorded rather than fixed because the fix is a judgement about what an empty
  data list *means* (dispatch with the continuation's own random? refuse?), and the path is
  unreachable. Pinned by `a_par_body_with_no_matched_data_panics_in_merge`
  (`#[should_panic(expected = "at least 2 inputs")]`), so a change in reachability — or a guard —
  fails a test instead of surfacing as a node crash.

### Findings from the coverage sweep (pass 6, continued)

These three came out of Stage 3's tier sweep — two are faithful-port notes pinned by a test, one is a
partial fix of an earlier security remediation.

- **C6 — `graphz` does not escape its input.** `graphz/src/lib.rs::quote` wraps a label in quotes
  only when it does not already start with one, and `head` interpolates the graph name unescaped, so
  a name or label containing `"` emits malformed DOT (a name of `G"x` yields `graph "G"x" {`, an edge
  label of `a"b` yields `"a"b"`). The inputs are block-derived strings in the documentation/SVG
  pipeline, so the impact is a broken diagram, not injection into anything executed. **Assessed
  faithful** (Scala's `Graphz.quote` is the same two-line function; adding escaping would change
  generated output for every existing caller). Pinned by `an_embedded_quote_is_not_escaped`
  (`graphz/src/lib.rs`), so the day escaping is added the test fails and is updated deliberately.
  Related: `Graphz::node` writes its `label` through unquoted while `Graphz::apply` quotes a label —
  the Scala asymmetry, pinned by `a_node_label_is_written_through_without_quoting`.

- **C7 — `generate_key`'s password retry recurses without a bound.**
  `node/src/runtime/node_main.rs::generate_key` re-prompts by calling itself on an empty or
  mismatched password, with no attempt counter and no depth limit, so a console that always returns
  an empty string would grow the stack until it overflows. **Assessed faithful** (port of
  `NodeMain.generateKey`, which recurses the same way) and bounded in practice by an interactive
  operator, so the port keeps it. Deliberately **not** pinned by a test — a stack-overflow probe
  aborts the test process and would assert nothing a reader cannot see; the doc comment on the
  function records the shape. The retry *behaviour* (re-prompt, distinct messages for empty and
  mismatched) is pinned by `generate_key_reprompts_on_a_mismatch_and_refuses_an_empty_password` and
  `generate_key_retries_after_an_empty_password`.

- **C8 — the R6 private-key file mode was applied only at creation (fixed here).**
  `crypto/src/util/key_util.rs::write_with_mode` passed `0o600` to `OpenOptions::mode`, which the
  kernel applies **only when the file is created**. Writing over an existing `rnode.key` therefore
  kept whatever permissions it already had — so a key file that predates the R6 remediation (or one
  an operator copied in with `cp`, which preserves the source mode) stayed world-readable through
  every subsequent `--generate-key`. The R6 fix was therefore only half-effective: correct on a fresh
  data directory, silently ineffective on an upgrade. **Fixed** — the mode is now also applied with
  an explicit `fs::set_permissions` *after* the write, so the content is never briefly readable under
  the wider mode. **Production change** (one function, `crypto/src/util/key_util.rs`), listed in the
  register's production-change list. Verified: `the_private_key_file_is_owner_only`
  (`crypto/src/util/key_util.rs`) asserts the created mode *and* the re-write case; the second
  assertion fails without the `set_permissions` call (`left: 420 (0o644), right: 384 (0o600)`), which
  is how the defect was found.

## 16. Rholang syntax findings (the legacy-corpus sweep)

The 165 `.rho` files under `legacy/` had never been parsed by the Rust port (`spec/TEST-COVERAGE.md`,
"the largest single untested surface"). Running them through the real parser/reducer
(`rholang/tests/legacy_contracts.rs`) found three defects in the **grammar** — not in the reducer —
each of which silently changed the meaning of valid rholang. All three are fixed and pinned.

The oracle for every one of these is the BNFC grammar the Scala node's Java parser is generated from,
`legacy/rholang/src/main/bnfc/rholang_mercury.cf`.

- **C9 — `(x)` was parsed as a one-element tuple; it is a group.** The grammar has *both* a grouping
  production and two tuple productions, and they are distinguished by content:

  ```
  PExprs.          Proc11 ::= "(" Proc4 ")" ;              -- a parenthesised expression
  TupleSingle.     Tuple  ::= "(" Proc ",)" ;              -- a tuple, comma mandatory
  TupleMultiple.   Tuple  ::= "(" Proc "," [Proc] ")" ;
  ```

  The Rust parser had no `PExprs` at all: any `(` in collection position became a tuple, so
  `(3 + 5)` parsed as `TupleSingle(3 + 5)` and `2 * (3 + 5)` failed at *reduce* time with "operator
  `*` expects Int, got Tuple". Worse, it accepted forms the grammar rejects — `(a!(b))` and
  `(a | b)` — as one-element tuples, because `parse_collection` never required the comma. Measured
  payoff: `casper/src/genesis/resources/Registry.rho` (the node's own registry contract, a depth-4
  keccak-256 nybble trie) **could not be reduced at all** before the fix; it reduces cleanly now, as
  does `Registry.rho`, `tut-parens.rho` and any deploy using arithmetic in parentheses. **Fix:** a
  group is now parsed at its own level (`parse_proc11_head`, `PExprs ::= "(" Proc4 ")"`, tried
  speculatively and rewound when the interior is followed by a comma), and the collection path
  requires the comma. Verified: `a_parenthesised_expression_is_a_group_not_a_one_element_tuple`,
  `a_group_may_not_contain_a_send_or_a_parallel`; with the old branch restored the first fails with
  the AST it used to build, `CollectTuple(TupleSingle(PAdd(3, 5)))`.

- **C10 — the logical connectives were swapped, and disjunction was unparseable.** The grammar spells
  them `PConjunction ::= Proc14 "/\\" Proc15` and `PDisjunction ::= Proc13 "\\/" Proc14` — conjunction
  is `/` then `\`, disjunction is `\` then `/`. The lexer matched `\` + `/` as **Conj** (the
  disjunction spelling, labelled as conjunction) and `\` + `\` — not an operator in the grammar at
  all — as `Disj`, while the parser consumed `Tok::Conj` as `PConjunction` and never consumed
  `Tok::Disj`. So `a \/ b` parsed as **`a /\ b`** and reduced to `ConnAnd`, and `a /\ b` did not lex
  (`/` was taken as division, the `\` was then an illegal character). `Proc::PDisjunction` and the
  normalizer's `normalize_disjunction` were already written and therefore unreachable. This is a
  silent *semantic* swap on a ρ-calculus connective (Law 4), not a parse failure: a Scala-produced
  block using `\/` would be re-parsed by a Rust node as a conjunction and diverge. **Fix:** the lexer
  spells both connectives as the grammar does (longest match, so `/` alone is still division) and
  `parse_proc13` grew the disjunction level. Verified:
  `the_logical_connectives_lex_and_parse_in_their_grammar_spelling` (also pins the precedence —
  `/\` binds tighter — and that `\\` no longer lexes); with the swapped arms restored the test fails.

- **C11 — `++` had no Map or Set arm.** `Reduce.scala`'s `EPlusPlusBody` defines five arms: String,
  `GByteArray`, `EList`, `EMapBody` (union) and `ESetBody` (union), reporting
  `OperatorExpectedError("++", "Map"/"Set", …)` for a mismatched operand. The port had only the first
  three, so `Set(1) ++ Set(2)` and `{"a": 1} ++ {"b": 2}` — both valid rholang, and both used by the
  standard contracts — errored instead of reducing, and a `Map`/`Set` left operand was reported as
  `OperatorNotDefined` rather than the expected-type error. **Fix:** the Map/Set arms reuse the same
  `par_set`/`par_map` canonicalisation as the `union` *method* (which was already implemented), and
  the two error arms match Scala's. Verified:
  `plus_plus_concatenates_byte_arrays_and_unions_maps_and_sets` (values, the right-biased map
  collision, and all four error arms by variant).

### Documented (not a defect)

- **`OperatorExpectedError` prints the same message as `OperatorNotDefined`.** Both format as
  "Error: Operator `op` is not defined on type." in `legacy/rholang/.../errors.scala` — the Scala
  `expected` field is carried but never rendered. The port is faithful, so a test that asserts the
  *type* is in the message would be asserting an infidelity; the C11 test asserts the enum variant
  instead. Recorded because it is surprising enough to be "fixed" by accident.
- **The `src/main/k/rholang/tests/*.rho` files are K-framework semantics tests**, not programs: they
  are the fixtures for the K definition (`legacy/rholang/src/main/k/`), written in an older dialect.
  They are classified, not "supported" (see the register's skip table).
- **C12 — `+` and `-` had no collection arms.** `Reduce.scala`'s `EPlusBody` has
  `case (lhs: ESetBody, rhs) => add(lhs, List[Par](rhs))` — inserting into a set — and `EMinusBody`
  has an arm each for `EMapBody` and `ESetBody`, both calling `delete`. The port implemented `+` and
  `-` as integer arithmetic only, so `Set(1, 2) + 3`, `Set(1, 2) - 1` and `{"a": 1} - "a"` — all
  valid rholang, exercised by `convenience_methods_test.rho` in the Mercury tutorial — errored as
  `OperatorNotDefined`. Both call the *same* `add`/`delete` the corresponding methods do, so the port
  was already carrying the semantics one dispatch away. **Fix:** the arms are added and the shared
  bodies extracted into `set_add`/`collection_delete`, used by both the operators and the methods so
  the two cannot drift. Verified: `plus_and_minus_also_insert_into_and_delete_from_collections`
  (insert, duplicate insert, set delete, map delete by key, and the unchanged arithmetic and error
  arms).
- **The parse-depth guard bounds nesting, not stack.** `MAX_PARSE_DEPTH = 128` (`rholang/src/parser.rs`)
  accepts depth 128 because each level enters ~16 nested `parse_procN` functions, so the guard's
  limit costs ~3 MiB of stack in a debug build and under 2 MiB in release (measured: a `new x in`
  term 124 levels deep parses at 3 MiB debug / 2 MiB release, and a 200-level term is rejected).
  `casper/src/genesis/resources/MakeMint.rho` — one of the node's own genesis contracts — reaches
  parse depth 64, so the margin is real but not large: a debug build cannot run the corpus on a
  default 2 MiB thread stack. No action for the node (it ships release, where the limit fits), but
  the corpus test sets an explicit stack so the requirement is stated where it bites rather than in
  a `RUST_MIN_STACK` invocation.
- **A forged block stalls LFS sync for that hash (faithful to Scala — recorded, not fixed).**
  `casper/src/engine/lfs_block_requester.rs::validate_received_block` marks the key `Received` via
  `LfsState::received` *before* it checks the hash, and `LfsState::get_next(resend)` only ever
  re-requests keys in `Init` or `Requested` status. So a peer that answers a request with a block
  whose `block_hash` does not match its content leaves that key `Received`, neither saved (`done` is
  only called on acceptance, and only `done` removes the key) nor re-requestable — even by the idle
  resend. The requester then never reports `is_finished()`. **Assessed faithful**: the Scala
  `LfsBlockRequester.validateReceivedBlock`/`ST.getNext` have exactly this ordering and this
  predicate, so the port is not the source of the behaviour, and a divergence here would be worse
  than the wart. The residual is a liveness one on a sync that a hostile peer can stall. Pinned by
  `a_requested_block_with_a_forged_hash_is_rejected` (the rejection, the absence from both the
  normal and the resend request sets, and `!is_finished()`), so a future guard in either place fails
  a test instead of changing behaviour silently.
- **OPEN QUESTION — `ReportingRuntime::consume_result` never matches.** Calling it with the same
  binder `BindPattern` and on the same channel as a datum that `get_data` reports as present returns
  `None` *and* leaves the datum in place: it neither matches nor consumes. The same pattern and datum
  match in isolation (`rho_match_binds_free_vars` in `rholang/src/storage.rs`), so the gap is in the
  path, not the matcher — `ReportingRspace::consume` records the event and delegates to
  `ReplayRSpace::consume`, and the reporting runtime is the only caller of this entry point. Recorded
  rather than fixed or asserted-as-correct because I could not establish the intent: `consume_result`
  may be a reporting placeholder that was never wired to matching, or the delegation may be losing
  something. The test
  (`an_unmatched_consume_result_is_none_and_leaves_a_waiter`, `rholang/src/reporting_runtime.rs`)
  pins what is observed, so closing the gap will fail it and force the update. No consensus impact:
  the reporting runtime is read-only tooling (`/reporting` routes), not the deploy path.

  **Decided (2026-09-23, Programme A item A5): the *intent* is settled — it must match — and the
  observation stands as a question about the pair the audit built, not about the function.** The
  Scala's `RuntimeSyntax.scala:553-557` delegates to `runtime.consumeResult(Seq(channel), Seq(pattern))`
  and its one caller (`consumeSystemResult`, `:427-443`) treats a mismatch as **fatal**
  (`ConsumeFailed`), which is the intent: consume the result of the deploy that was just run. The
  port's caller is the same one — `casper/src/runtime_replay.rs:552` — and it *does* match there, which
  the replay and determinism tests pin; what the audit constructed was a binder/datum pair the caller
  never builds. Recorded as: no behaviour change, and the next person who touches the reporting
  runtime has the intent written down instead of an open question.

  **Decided (2026-09-23, Programme A item A5): kept — it is faithful, and the fix belongs to the
  protocol rather than to this port.** `LfsBlockRequester.validateReceivedBlock`/`ST.getNext` in the
  Scala have the same ordering and the same predicate, so a peer can stall a sync for one hash in
  both trees. The residual is a liveness property of a hostile-peer scenario, and the port's
  behaviour is pinned by the test named here; changing only the port would make it diverge from the
  oracle in the direction this document exists to prevent.

- **C13 — the pretty printer emitted two `|` separators between the first two items of a group, and
  dropped the `bundle` keyword.** Both are **port** defects (the Scala renders both correctly), and
  both were found by a round-trip test (`printing_and_reparsing_is_the_identity`): printing a parsed
  term and re-parsing the result must give the same term back.
  1. `rholang/src/pretty_printer.rs::build_par` tracked "an item has been printed" *inside* the item
     loop, where the Scala tracks it per *group* (`PrettyPrinter.scala:288-302`'s `prevNonEmpty`), so
     any group with two or more items printed `a |\n |\nb` — the two sends of `@"a"!(1) | @"b"!(2)`,
     the most ordinary shape there is, printed as unparsable rholang.
  2. `build_bundle` printed only the bundle *sign* (`0`, `+`, `-`) and not the keyword, where Scala's
     `BundleOps.showInstance` is `"%-8s".format(s"bundle$sign")` — so a bundle printed as `0{ … }`
     instead of `bundle0 { … }`. The eight-column padding is faithful and is reproduced.
  Both are fixed and the round trip now covers 36 terms. Two *faithful* warts remain, both inherited
  from the Scala printer and both pinned rather than fixed (`the_documented_warts_print_what_the_
  grammar_cannot_read_back`): a one-element tuple prints as a group (`(1,)` ⇒ `(1)`, which is `1`),
  and `not x` prints as `~(x)`, which is not valid rholang at all. Printed output for those two forms
  must not be pasted back as source.

- **C14 — the UPnP SSRF guard could be bypassed with a bracketed IPv6 URL (fixed).**
  `comm/src/upnp/gateway.rs::is_safe_url` — the guard that refuses loopback/link-local/unspecified/
  multicast discovery URLs — extracted the host with `authority.split(':').next()`. For a bracketed
  literal (`http://[::1]/desc.xml`) that yields the host `"["`, which is not a parseable IP, so
  `is_ssrf_unsafe_host` answered `false` and the guard **allowed** a loopback URL; `split_url` then
  produced the same broken host, so no request was actually made (the connect failed on the name
  `"["`), which is why the hole was latent rather than live — the guard's *decision* was wrong and the
  second bug masked its effect. **Fixed** with one `split_authority` helper (bracket-aware, shared by
  the guard and the URL splitter), so `[::1]`/`[fe80::1]` are refused and an IPv6 gateway literal is
  split correctly. Found by `the_url_guard_allows_private_gateways_and_refuses_ssrf_targets`, which
  lists `http://[::1]/desc.xml` among the URLs that must be refused.

- **C15 — the bit-level decoder panics on a truncated bit stream (latent, pinned).**
  `rspace/src/serializers/scodec_serialize.rs::BitReader::read_bit` indexes `bytes[bit_pos / 8]`
  without a bounds check, so decoding a truncated blob panics with an index-out-of-bounds instead of
  returning a decode error — `decode_rnd(b"")` is the smallest case. **Latent, not live**: the bytes it
  decodes come from the node's own mergeable store (written from its own replay), so the exposure is a
  corrupted or truncated *local* entry aborting the merge rather than a peer-supplied one; the
  peer-facing decoders (`Packet`/protobuf) go through prost and return `Result`s. Recorded rather than
  fixed because a `Result` has to be threaded through every `read_bit`/`read_bits` caller (the whole
  scodec layer) — a change worth doing deliberately, not as a side effect of a test sweep. Pinned by
  `a_truncated_mergeable_datum_panics` (`#[should_panic]`), so giving the reader a `Result` fails that
  test and is a deliberate change.

## 17. Census-sweep findings (the untested-file sweep)

The sweep this section belongs to enumerates every source file without a test and writes one
(`spec/TEST-COVERAGE.md`, definition of done item 10). Its findings are recorded here in the same
form as the other passes: what the code did, why it is wrong rather than merely surprising, what the
oracle is, and the test that pins the fix.

- **C16 — the deploy-execution-status enum serialized its variant *fields* in snake_case, in the
  middle of a camelCase API response.** `models/src/casper/protocol/deploy_service.rs` declares
  `#[serde(rename_all = "camelCase")]` on `DeployExecStatus`, which renames the *variants*
  (`processedWithSuccess` ✓) but — in serde, and this is the easy mistake — **not the fields of struct
  variants**. Those need `rename_all_fields` (serde ≥ 1.0.181), which the repo already uses in
  `node/src/api/dto.rs` and `node/src/web/transaction.rs` for exactly this reason. So
  `GET /api/...`'s deploy status emitted

  ```json
  {"processedWithSuccess":{"deploy_result":[],"block":{"blockHash":"…","preStateHash":"…"}}}
  ```

  — every sibling field camelCased, the two variant fields not. The oracle is the Scala the API
  mirrors: `DeployExecStatus.ProcessedWithSuccess(deployResult, block)` is a case class whose field
  names are the JSON keys, and they are `deployResult`/`deployError`. **Not a cosmetic difference**:
  the API is a published client contract, and a client reading `deployResult` against this node gets
  nothing. **Fix:** `rename_all_fields = "camelCase"` on the enum. Verified:
  `the_api_types_deserialize_what_they_serialize` asserts the variant key *and* its fields by name
  (the failure message prints the whole JSON, which is how the wire spelling was read off rather than
  guessed). The two other `rename_all` enums in the models crate (`ReportProto`,
  `SystemDeployData`) were checked and carry only tuple/unit variants, so they have no such field —
  this was the only instance. **Corroboration:** `docs/src/developer/building-apps.md` already
  documented the response as "`processedWithSuccess` (with the `deployResult` expression)" — the
  published API documentation and the code disagreed, and the *documentation* was right. That is
  independent evidence for the fix rather than a preference for camelCase.

- **C17 — the list and `Set` collection branches had no remainder production, and neither did the
  collection-level map branch.** The grammar gives *every* collection form a remainder:

  ```
  CollectList.   Collection ::= "[" [Proc] ProcRemainder "]" ;
  CollectSet.    Collection ::= "Set" "(" [Proc] ProcRemainder")" ;
  CollectMap.    Collection ::= "{" [KeyValuePair] ProcRemainder"}" ;
  ProcRemainderVar.   ProcRemainder ::= "..." ProcVar ;
  ```

  One map path (`parse_proc`'s `{k: v}` arm) broke on the ellipsis, so `{a: 1, ...rest}` parsed; the
  three arms in `parse_collection` — list, `Set(...)`, and its own map arm — consumed the comma
  unconditionally and then called `parse_proc` on `...`, which is not a process. So
  `[=*type, ...item]` failed with `expected variable, got Ellipsis` while the same construct in a map
  was accepted. `ProcRemainderVar` is not merely unsupported downstream: `normalize_collection`
  carries it into `EList`/`ESet`/`EMap`'s `remainder` field, and the matcher consumes it
  (`fold_match`, `handle_remainder`). Only the parser was missing the arm, which made a valid
  collection pattern unparseable for two of the three forms. Measured payoff: the rgov governance
  contracts (`rchain-community/rgov`) destructure their message queues exactly this way — `Inbox.rho`
  uses it 16 times with both bindings *used* (`ret!(item) | box!(rest)`), so `Inbox.rho` is
  unparseable and everything importing it is too: `Directory.rho`, `Issue.rho`, `Group.rho`,
  `CrowdFund.rho`, `Kudos.rho`, `memberIdGovRev.rho` and `feature/MemberDirectory.rho` **all failed to
  parse and all parse now** (verified against the files themselves, not a reduction of them).
  **Fix:** after a comma, each branch breaks on `Tok::Ellipsis`, leaving the token to
  `parse_proc_remainder`, as the map arm in `parse_proc` already did. Verified:
  `collection_remainders_parse_for_lists_and_sets_not_only_maps` parses every form, asserts the
  remainder is *carried* rather than dropped (`ProcRemainderVar`), and parses the `Inbox.rho` read
  pattern verbatim — `match (*items) { {[=*type, ...item] | rest} => {…} _ => {…} }`.

- **C18 — `rho:registry:lookup` wrapped its reply in `(uri, value)`; the oracle sends the stored
  value alone.** The oracle is not a native process at all: lookup is the genesis `Registry.rho`
  contract, whose `lookup` forwards `TreeHashMap!("get", …)` — which sends the stored value by
  itself (`legacy/casper/src/main/resources/Registry.rho:397-401`). Its recorded output agrees
  (`legacy/rholang/examples/tut-registry.rho:8,42-47`: the reply prints as `Unforgeable(0x…)`, and
  the consumer binds one name). The native handler wrapped it instead
  (`system_processes.rs::registry_lookup` produced `RhoTupleN(vec![uri, value])`), which **fails
  silently rather than loudly**: every oracle-era client consumes the reply as
  `lookup!(uri, *ch) | for (X <- ch) { X!(…) }`, so the pair binds to the name and the send is a
  no-op. Nothing errors; the deploy simply produces no result. Measured payoff: the entire rgov
  governance contract family (and ~40 consumer snippets) returned `[]` with no diagnostic — the
  symptom that started this audit pass. **Not a cosmetic wrapping difference:** a shape assertion
  on a scalar reply can still read the right element out of a pair, which is why the previous
  round-trip test passed while every real client failed. **Fix:** produce the stored value alone
  (unknown uri still answers `Nil`). Verified: `registry_insert_arbitrary_and_lookup_round_trip` and
  `registry_insert_signed_binds_deployer_id_from_the_normalizer_env` now assert the unwrapped value
  (`insertSigned`'s stored value is itself the `(nonce, data)` pair it recorded — which is why
  consumers of *system* contracts destructure `@(_, X)`, the pattern that had been misread as
  evidence for the wrapper), and the new
  `a_looked_up_contract_can_be_called_through_its_lookup_reply` asserts the thing the old tests
  could not: that a looked-up contract is **reachable by a send**. That missing assertion is the
  reason this shipped.
- **Related, found by the same pass — not fixed here.** Three further divergences of the same class,
  each needing its own decision: `rho:block:data` sends `(blockNumber, sender, timestamp)` where the
  oracle sends `(blockNumber, sender)` and never exposes `seqNum`
  (`legacy/.../SystemProcesses.scala:355-361`; consumer
  `legacy/casper/src/test/resources/BlockDataContractTest.rho:15-16` binds a two-name pattern, so a
  three-element send cannot match it) — and **the vendored source was edited to match, which nothing
  recorded until 2026-09-24**: `casper/src/genesis/resources/RevVault.rho` is the one blessed contract
  that differs from its `legacy/` original beyond the shorthand re-derivation, and its one hunk is
  exactly the block-data consumer's pattern. `tools/audit-vendored-sources.sh` (run by
  `make check-register`) now diffs every vendored contract against its original and requires each
  difference to be an allowlisted entry naming its register row, so the next such edit is visible
  rather than waiting for someone to read for something else; the registry's **shorthand table is unimplemented and never
  seeded** — ⚠️ **resolved since** (`spec/GENESIS.md`): genesis now installs `ListOps`,
  `NonNegativeNumber` and `MakeMint` and seeds the shorthand aliases natively, so
  `lookup!(\`rho:rchain:revVault\`, *ch)` resolves to a callable value. The original finding stands
  as the description of what was wrong: `registry_lookup` did a literal key lookup against a registry
  nothing ever populated, and `default_blessed_terms` installed nothing, so every shorthand answered
  `Nil` — silently, because an unmatched `for` is not an error; and `rho:rchain:revVault` is a redesigned API
  (`getBalance`/`transfer` on the vault, `findOrCreate` returning an address string rather than a
  vault capability, no `authKey`) with `rho:rchain:multiSigRevVault` wired to the single-sig
  handler. The schema standard these belong to is `spec/API-SCHEMA.md`.

- **C19 — a collection pattern with a *wildcard* remainder could never match, so partial *map*
  patterns never matched at all.** `...rest` (named) and `..._` (discarding) both mean "and the
  rest of the collection", and the grammar gives both to every collection form. The matcher reached
  `list_match` with the remainder split in two — a `remainder: Option<i32>` level for the named form
  and a separate `wildcard: bool` for the discarding form — and padding for it only when
  `remainder.is_some()`. So a wildcard got no `MbmPattern::Remainder` to absorb the unnamed entries,
  and `@{"x": *v, ..._}` could not match a map holding any other key. The handling *below* already
  expected this case (`None => { if wildcard || … }`), it simply never received the padding — which
  is what made this a one-condition fix rather than a design change. **Measured payoff:** every rgov
  governance contract reaches its capabilities through exactly this pattern — `MemberDirectory.rho:15`
  gates its whole body (`getMe`, `createMe`, `sendThem`) on
  `for (@{"read": *MCAread, ..._} <<- @[*deployerId, "MasterContractAdmin"])`, so none of those
  contracts could run, and a `for` whose pattern does not match is **not an error** — it silently
  never fires, which is why the failure presented as `[]` with no diagnostic rather than as a fault.
  Lists happened to work and maps did not; sets shared the map's fate. **Fix (diagnosed, not
  landed):** pad when the pattern has a remainder *or* is a wildcard
  (`spatial_matcher.rs::list_match`). That change makes the test suite green and is the right
  direction, but it is **not landed, and its node-safety is unverified** — see the correction below.
  `collection_patterns_match_a_subset_of_their_collection` is kept, `#[ignore]`d, as the
  executable record of the defect and of the acceptance criteria: all five forms — list/map/set ×
  wildcard/named — each asserted to *match*, because the failure mode is silence rather than an
  error. **Consequence while unlanded:** every rgov governance contract still returns `[]`, since
  `MemberDirectory.rho:15` gates its body on a partial map pattern.

  **Correction (this entry previously claimed the padding hangs the node — withdrawn).** The
  observation was real: with the padding built in, the devnet stopped serving `/api/v1/status` while
  replaying the chain. But the reverted build failed to serve on that same chain data too, so the
  padding is not what blocked it: the persisted chain had grown across many deploy-heavy runs and its
  startup replay had outgrown `devnet.sh`'s serve-timeout. On **fresh** data the node boots either
  way. Controlled comparison settled it: with the padding built in, on the same chain data, the devnet
  boots and serves (`devnet.sh up` exits 0). The padding is **landed** — it is correct and necessary,
  since a map of plain values could not be matched partially before it.

  **The padding is NOT sufficient, and there is a second gate (open).** With it landed, the rgov
  family still returns `[]`. Tracing that: `getMe`'s body calls `createMe`, which is defined *inside*
  the `for (@{"read": *MCAread, ..._} <<- @[*deployerId, "MasterContractAdmin"])` block at
  `MemberDirectory.rho:15`; with that gate closed, `createMe` is the outer `new`'s unused channel, so
  the call silently goes nowhere. Probing that exact pattern against the real channel on a live node
  shows it **still does not match** (`for (@c <<- …)` — binding the map bare — does match, so the
  channel holds a value):

  ```
  channel-holds  a value          ← the channel is populated
  gate-open      pattern matched  ✗  ← the partial map pattern still fails
  ```

  The likely reason, and it is a hole in the acceptance test: `MbmPattern::Remainder` absorbs a target
  only when `t.locally_free_empty()`, and the real dictionaries hold **bundles** (`{"read":
  bundle+{*read}, …}`), not plain values. The five-form conformance test uses a map of integers, so it
  passes while the real shape fails — the test must be re-specified with bundle-valued entries before
  it can serve as the acceptance criterion. The next step is therefore to establish, from the Scala
  oracle, whether a remainder may absorb a bundle at all (a capture must be quotable; a *wildcard*
  discards and arguably need not be), and to fix the test's shape first.

  **Resolution (both of the above are superseded — see C20).** The diagnosis above was wrong on both
  counts, and both wrong claims are withdrawn:

  - *"the padding is correct and necessary"* — it was a **no-op for collections**. The `wildcard` flag
    is derived from the *map/set* `remainder`, which the `ESet`/`EMap` arms read off the **target**
    (C20), so it was always `false` and the gate never opened. The map case in the conformance test
    passed for a different reason: the five cases shared one runtime and one `@"out"` channel, so
    after the first (list) case produced `"ok"` every later case passed by re-reading that datum. The
    padding has since been reduced back to the Scala's own gate (`remainder.is_some()`), which is
    correct because a wildcard needs no padding — the trailing `wildcard ||` check accepts unclaimed
    leftovers — and padding would demand concreteness Scala does not require of them.
  - *"the second gate is `locally_free_empty` on bundles"* — **refuted**. `C20`'s fix, with the gate
    restored, matches a three-key dictionary of bundles (`{"read": bundle+{*read}, …}`) peeked with
    `@{"read": *MCAread, ..._}` — pinned by
    `collection_patterns_match_a_subset_of_their_collection`. Nothing about bundles was in the way;
    the pattern never reached the matcher's remainder handling at all.

  The `✓`-shaped probe above (`gate-open ✗`) was real, but its cause was C20, not bundles: with the
  partial map pattern unable to match *any* map with an unnamed key, no dictionary shape could open
  the gate.

- **C20 — a remainder in a `map`/`set` pattern never absorbs an entry: the pattern's remainder was
  read off the *target*.** A map pattern matched only when the map had exactly as many entries as the
  pattern names — `@{"x": *v, ..._}` against `{"x": 1, "y": 2}` failed while the exact form succeeded,
  observed on a node. In `spatial_matcher.rs::spatial_match_expr` the `ESet` and `EMap` arms destructure
  `remainder: rem` from the **first** tuple element (the target) and ignore the pattern's, while the
  `EList` arm — and the Scala oracle at `SpatialMatcher.scala:495-505` — take it from the **pattern**:

  ```rust
  (Expr::EMap(ParMap { kvs: tlist, remainder: rem, .. }),   // ← target binds `rem`
   Expr::EMap(ParMap { kvs: plist, .. }))                   // ← pattern's remainder ignored
  ```

  A stored collection never has a remainder, so `is_wildcard`/`remainder_var` were permanently
  `false`/`None` and `list_match_single` took its exact-match path
  (`if exact_match && plen != tlen { return Ok(Vec::new()) }`). **Consequence:** every rgov governance
  contract reaches its capabilities through `for (@{"read": *MCAread, ..._} <<- <3-key map>)`
  (`MemberDirectory.rho:15`), so the family returned `[]` — silently, since an unmatched `for` is not
  an error, which is why this was expensive to find. Lists were unaffected (their arm was right and
  their remainder is a suffix, via `fold_match`).

  **Fix:** take the remainder from the pattern in both arms, as Scala and the `EList` arm do; and
  reduce `list_match`'s padding gate back to the Scala's `remainder.is_some()` (a wildcard needs no
  padding — see C19's resolution). Pinned by the matcher unit tests
  (`a_map_pattern_may_name_fewer_entries_than_the_map_has`,
  `a_named_map_remainder_captures_the_unnamed_entries`,
  `a_set_pattern_may_name_fewer_members_than_the_set_has`, `list_remainders_stay_positional`) and by
  `collection_patterns_match_a_subset_of_their_collection` (in-process, with the rgov gate's
  bundle-valued shape and a peek) and the `devnet-test.sh` step 3b leg (on a real node).

  **Also found — the harness accepted what the node rejected, and it was the fixture, not the
  runtime.** The in-process conformance runtime is *not* a different matcher: it builds the same
  `RSpace` + `RhoMatch` the node does, and the same normalizer. The test was blind instead: it
  evaluated all five shapes against **one** runtime, reading a **shared** `@"out"` channel, and
  asserted on `got[0]` — the first datum, which the first (list) case had already produced. Cases 2–5
  therefore passed by re-reading it whether or not their own pattern matched (measured: on a fresh
  runtime the map and set cases produce **0** data). Each case now builds its own runtime and is
  asserted to produce exactly one datum. A green in-process conformance run is evidence about the
  node **only** when each case's observation is separable.

- **C21 — an `if` normalized its condition against the `par` that precedes it, so every `if` that was
  not the first term of its `par` was a silent no-op.** `normalizer.rs::normalize_if` desugars
  `if E {A} else {B}` to `match E { true => A; false => B }`, and normalized the *condition* with
  `normalize_proc(value, input)` — the caller's `ProcVisitInputs`, whose `par` field is where a `PPar`
  accumulates its already-normalized left-hand terms (`normalizer.rs`'s `Proc::PPar` arm threads
  `par: result.par` into the right operand). So for `P | if E …` the `Match` target became `P | E`
  rather than `E`. The two case patterns are `true` and `false`, so a target that is not a Bool
  matches **neither** case; `resolve_match` then returns `Ok(None)`
  (`reduce.rs:2018`), the effect is dropped as `None` (`reduce.rs:2288-2299`), and an unmatched
  `match` is **not an error** — the `if` reduced to nothing, silently, with the deploy reporting
  `processedWithSuccess`. Invisible for an `if` in first position, where the accumulated par is empty:
  `Nil | if (c) …` worked, and that is the idiom almost every working contract uses, which is how
  this survived both implementations.

  **Found from:** the wallet's staged governance handshake reached `stage:3 directory answered GetMe`
  and never `stage:4`, and `newInbox` returned `[]`. The vendored feature's `getMe` logs
  `["getMe", you, "everyone size", N]` (`MemberDirectory.rho:77`), then `if (everyone.contains(you) ==
  false)` (line 78) — a non-first `if` — so neither the create path (line 79-93) nor the
  already-exists path (94-109) ran, `createMe!` at line 80 was never sent to, and the caller's reply
  channel stayed silent. In-process the same trace is reproducible in one line of rholang:
  `x!(["a"]) | if (1 == 1) { @"out"!("then") } else { @"out"!("else") }` produces **0** data, while
  the identical `if` first (or wrapped in a `new`) produces one.

  **Mechanism, isolated:** the *target* was the accumulated par. `normalize_match` (the explicit
  `match`, `normalizer.rs:697`), `binary_exp` and `normalize_bundle` all seed the target with
  `Par::default()`; `normalize_if` was the only desugaring that passed the caller's `input` through.
  A live node probe confirmed it from the other side: `out!(["W1-a"]) | 1 / 0` is inert, while
  `out!(["W2-a"]) | 1 / 0 | if (true) {…} else { Nil }` **errors** — the preceding par's expression
  was being evaluated inside the `Match` target.

  **Reference:** the Scala is *internally inconsistent* on exactly this point, which is why the port
  inherited the defect. `PMatchNormalizer.scala:28` normalizes the target with
  `input.copy(par = VectorPar())`; `PIfNormalizer.scala:24` passes `input` unchanged and then uses
  `targetResult.par` as the `Match` target. The two desugarings of the same construct therefore
  disagree, and the Scala's own tests only exercise `if` as a whole term or first in a block. The
  port follows the `match` path (and the desugaring), and the divergence from the Scala's `if` path is
  registered in §6. **Hard-fork class:** the normal form of any term with a non-first `if` changes —
  see §6's row for the consequence.

  **Fix (`normalizer.rs::normalize_if`):** normalize the condition against `Par::default()`, keeping
  the accumulated par only for the final `prepend_match(&input_par, m)`. One field. Every vendored
  `.rho` file is untouched (bytes identical to upstream — see `resources/rgov/NOTICE`), and the
  *other* dead `if`s in genesis content come back with it: `MemberDirectory.rho:78, 96, 131`,
  `Ballot.rho:56, 60`, `Issue.rho:62, 66`, `Group.rho:79, 119`, and `ListOps.rho:203, 242` +
  `MultiSigRevVault.rho:155` from the node's own standard deploys. **Pinned by**
  `an_if_condition_does_not_absorb_the_pars_before_it` (the AST-level pin: the target *is* the
  condition, and `P | if E …` normalizes it identically to `if E …` alone), the conformance case
  `an_if_fires_the_same_way_wherever_it_sits_in_a_par` (the shape in the wild, both branches, the
  `match` control, each on its own runtime), and
  `a_fresh_chain_serves_the_wallets_new_inbox_handshake`, which now asserts the reply, the ceremony
  key's bootstrap lockers, and the feature's own log lines as a *trace* — so a stall is a missing
  line rather than an absence of output. Both the AST pin and the acceptance test were confirmed to
  fail with the defect reintroduced.

  **Correction of record:** the "open item" these passages described — "the read cap resolves, the
  directory answers `GetMe`, and `getMe` runs, then it stops inside the feature's own `createMe`"
  (`spec/GENESIS.md`, `docs/src/node/devnet.md:180`, and the comment this replaces in
  `casper/tests/genesis_registry.rs`) — was **wrong about where it stopped**, and C19/C20's matcher
  fixes were not the missing piece. `getMe` never reached `createMe`; it died one line earlier, at
  `if` (line 78). The four log lines that observation counted are `deployerRevAddr`'s three plus
  line 77; the next line the feature would have logged is `createMe`'s line 27, and its absence is
  the evidence that the `if` — not `createMe` — was the stall.

  **Not a defect, corrected in passing:** `rho:io:stdout` is **not** a one-datum sink that errors on
  the feature's multi-element log lines. It is registered `arity: 1` and persistent, and a *list* is
  one datum; the genesis run prints six-plus list-valued lines on it in a row, and the in-process
  genesis trace shows the same. The warning in `docs/src/node/devnet.md` (and the client-side copies
  in `r-wallet`'s `handshake.rho` and `snippets.ts`) is false and has been corrected here; passing a
  drain is still good hygiene for a caller, but it is not load-bearing.

  **Also found — a bare pattern matches `Nil`, so "a receive fired" is not evidence of a value.**
  `MCAread!("GetMe", …)` goes through `Directory.rho:43`'s `read(@key, return)`, which answers
  `return!(*map.get(key))` — `Nil` when the key is absent — and the consumer's `for (GetMe <- …)` is
  a *bare* pattern, which matches `Nil`. Reaching "the directory answered" is therefore true whether
  `GetMe` was registered or not, which is why the handshake looks further along than it is. The pins
  above assert the *value* (`getme-entry` is not `Nil`), never the bare fact that a receive fired.

- **C22 — C21's family, three more instances: two silent-no-match defects and one silent-consumed
  state.** Found by reading the consumers whose wallet cases still failed after C21, and each fixed
  with a pin that fails without it. All three are the same shape as C19-C21: nothing errors, and the
  branch simply never runs.

  1. **`Inbox.rho`'s zero-argument `read` consumed the store and never restored it.** `read(ret)` did
     `for (items <- box) { ret!(*items) }` — a linear consume of the single `box` datum, so after one
     read *every* later `write`, `peek` and `read` on that inbox waited on an empty channel for the
     life of the chain. Its two typed siblings restore it (`box!(rest)` / `box!(*items)`,
     `Inbox.rho:51,53,71,74`) and its docstring says "read (and *remove*) all messages" — the
     messages, not the container. **Fix:** a render-time adaptation in `rgov.rs::source("inbox")`
     (`replace_once`, recorded in the vendored NOTICE) that re-sends the emptied store —
     `ret!(*items) | box!(Nil)`. The on-disk `.rho` stays upstream byte-for-byte. **Pinned by**
     `a_read_does_not_destroy_the_inbox` (a write, a read-all, and a write that must still answer).
     Consumers: `sendMail`, `claimWithInbox`, `share`, and anything else that reads an inbox twice.

     **The class now has a law** (INVENTORY row 41, `Rchain/Store.lean`): a *replicable* reader — a
     `contract`, called again — must put back what it consumes, and `storeSurvives` is the decidable
     form of that ("a datum is back, and the pair forms a step"). `spec/conformance/store.tsv`'s cases
     are this defect, its repair, and the three shapes nearby (a restore in one branch of a `match`, a
     restore on the wrong channel, and the read-all/restore-container form `Inbox.rho` uses), each run
     through the node by `rholang/tests/lean_store_corpus.rs`. **The static half is deliberately not
     shipped:** a walk of the rendered text for "a linear consume with no paired produce" cannot tell
     three things apart, and two of them are not defects — a *terminal* consume `Issue.rho:104` (the
     tally's last read, after which nothing reads the store again), a *deferred* restore
     `Group.rho:49→65` (the datum comes back, behind two receives — which C25 later **measured** to be
     the case, so the reading was right and the walk could not have known it), and a permanent loss
     (this item).
     Reporting all three would mean an exception list over vendored content, which is exactly the kind
     of allowance this catalog exists to avoid. A static walk for this rule was drafted for this slice
     and retired unshipped rather than committed with an exception list; reading every vendored source
     against it by hand is what produced this paragraph and the note in C25.
  2. **`extraSlots` called the directory's write capability with the wrong arity.** Our own term
     (`rgov.rs::extra_directory_slots_source`) called `MCAwrite!("Chat", *C_Chat)` — two arguments —
     while `Directory.rho:56` is `write(@key, @value, ret)`. No receive matched, so **none** of the
     three slots was written, and every consumer of `Chat`/`Ballot`/`Group` read `Nil`; a `Nil` slot
     is indistinguishable from a broken one (that is the *documented* reason these slots exist at
     all). Worse, the test that covered it — `the_extra_slots_term_writes_the_names_the_wallet_asks_for`
     — asserted `term.contains("MCAwrite!(\"Chat\"")`, a *source-text* check that a two-argument call
     satisfies, i.e. the harness accepted what the node rejected (C20's lesson again). **Fix:** pass
     the reply channel; the text test now pins the arity, and the behavioural pin is
     `the_extra_slots_answer_a_directory_read` (reads each slot back through the read cap and asserts
     the *value*, not that a receive fired). Consumers: the wallet's `newBallot`/`castBallot`,
     `newGroup`/`joinGroup`/`addMember`, which reach those classes through these slots.
  3. **A partial map pattern whose only non-concreteness is a remainder was treated as concrete.**
     `fold_collection_map` built `ParMap { connective_used, .. }` from its keys/values alone, while
     the list and set folds in the same file include the remainder (`cu || has_rem`) and the
     reference ORs it in (`CollectionNormalizeMatcher.scala:92`, `:112`, `:137`). With the flag
     false, `spatial_match` takes its `pattern == target` short-circuit and a *partial* map matches
     nothing but itself — silently. The existing conformance cases all carried a free variable
     (`{"x": *v, ..._}`), which sets the flag for an unrelated reason, so the gap was invisible.
     **Fix:** `connective_used || remainder.is_some()` (`normalizer.rs::fold_collection_map`).
     **Pinned by** three cases in `collection_patterns_match_a_subset_of_their_collection`: a ground
     map pattern with a wildcard remainder *matches*, the same shape with a differing ground entry
     does **not** match (the fix must not turn a pattern into a wildcard), and a named remainder binds
     the ground entries it absorbs.

  **The sweep, finished rather than sampled** — every other candidate of this shape I examined, and
  why it is *not* a defect (so the next reader does not re-open them):

  | Candidate | Verdict |
  |---|---|
  | `normalize_if`'s sibling desugarings that normalize a composed `Par` with the caller's `input` (`PChoice`, `PSendSynch`, multi-receipt `PInput`, concurrent `let`) | **not** defects. A composed par is a *statement*: `PPar`'s arm threads the accumulation and each term lands exactly once. Only a *value* position — a condition (C21), a `Match` target, a method target, a send's data, a collection element — must be normalised in isolation, and all 33 of those sites already pass `Par::default()`. Verified by tracing each arm, not by sampling. |
  | `ParCount::min_max` expanding its max only for a top-level free var/wildcard in `par.exprs` (a remainder or bundle does not qualify) | faithful: `ParCount.scala:69-90` is byte-for-byte the same rule; a remainder is not a top-level free var in either implementation. |
  | `Connective::VarRef` never matches (`spatial_matcher.rs:347`) | faithful: `SpatialMatcher.scala:302` returns empty with "this should never happen because variable references should be substituted", and our reducer does substitute — receive patterns at `reduce.rs:1881`, `Match` case patterns at `:2005` — so `=x`/`=*x` patterns (as `Inbox.rho`'s typed reads and `Directory.rho`'s `write` use) never reach the matcher as a `VarRef`. |

  Everything in C22 was found by *reading the consumer of a failing case*, which is the cheap half of
  the instrument; the expensive half (a chain, a suite) then verifies rather than diagnoses.

- **C23 — the formal model did not contain the surface the ten silent defects live in.** Found while
  building the conformance instrument, before it ran: `spec/Rchain/Par.lean`'s `Par` had **no remainder**
  on its collection forms (`elist`/`eset`/`emap` were `List Par` / `List (Par × Par)` alone) and no
  concreteness predicate. The model therefore could not *express* the value `spatial_match` consults on
  its very first line (`if !pattern.connective_used { pattern == target }`, `spatial_matcher.rs`), nor
  the `..._`/`...rest` half of a partial pattern — so laws 35 (concreteness soundness) and 37 (match
  completeness) were unstatable, and C19/C20/C22's root cause lay *outside the specification's
  language*. That is how a defect class can recur while every law in the catalog holds: the catalog was
  written about the calculus, and the incidents happened in the surface the matcher reads.

  **Fix (this pass):** remainders are part of the model again — `Expr.elist`/`eset`/`emap` carry
  `Option Var`; the predicate is defined (`connectiveUsed` with `Expr.remainder`/`hasRemainder`, and
  `Var.isConnective`); closedness accounts for a remainder (`Ty.closedRemainder`); and Law 1's
  canonicalization orders by it (`Sort.cmpOptionVar`), so `[…]` and `[…, ...rest]` are distinct
  patterns — which is what the Rust already did, so this is the model catching up to the code rather
  than the other way round. `lake build` is green; the new `formal` CI job is what keeps it that way.

  **Also found:** two modules (`Concurrent`, `Tree`) were compiled but **not imported** by
  `Rchain.lean` — outside the library, in a repo whose thesis is that nothing is checked by accident.
  They are imported now, and `tools/check-lean-conformance.sh` fails when the import list and the tree
  disagree. The Coq (`spec/coq/`) had never been built by anything either; the same script builds it.

  **Residual (§6, a Scala deviation):** the reference ORs the *Var's* connective-ness into a map
  pattern's flag (`CollectionNormalizeMatcher.scala:92`), not the remainder's existence, while our Rust
  (and this model) treat any remainder as non-concreteness. The two agree for every pattern the grammar
  can write — a `...rest` remainder is a free variable before substitution — and differ only for an
  exotic bound-var remainder; recorded rather than imported.

- **C24 — an empty collection with only a remainder did not parse (found by the new conformance
  corpus, on its first run).** `[ ..._ ]`, `{ ..._ }` and `Set( ..._ )` are in the grammar —
  `CollectList/CollectSet/CollectMap ::= … [Proc|KeyValuePair] ProcRemainder …`
  (`rholang_mercury.cf:179-183`), where the element list may be **empty** — but both map parsers and
  the list/set element loops demanded an element before the ellipsis: `parse_braced_or_map` read the
  ellipsis as the first key's process, and the element loops of `parse_collection` could not start on
  it, so each said `expected variable, got Ellipsis`. A pattern that only *discards* a tail
  (`{..._}` against a map, `[..._]` against a list) was therefore unwritable, which is the shape a
  consumer reaches for when it wants "some map, contents irrelevant".

  **How it was found, which is the point:** not by reading contracts at midnight but by the corpus
  check landing in this same change — `spec/conformance/flags.tsv`'s case `@{..._}` (generated from
  `spec/Rchain/Corpus.lean`, where the verdict is `decide`d) disagreed with the node, and the check
  named the source, the expected verdict and the normalizer's answer. That is the loop AUDIT C23 was
  written to close. **Fix:** all three loops and both map parsers accept an empty element list
  followed by a remainder.

  **Pinned by:** `collection_remainders_parse_for_lists_and_sets_not_only_maps` (four shapes added:
  `[..._]`, `[...rest]`, `Set(..._)`, `{..._}`), the corpus case itself in
  `spec/conformance/flags.tsv` (asserted by `rholang/tests/lean_normalize_corpus.rs`), and — for the
  *model's* half — `Corpus.flagCases_decide` in `spec/Rchain/Corpus.lean`, which was confirmed to fail
  when the model's map-remainder rule is removed. The gate builds the executable targets for exactly
  that reason: a `lean_exe` root is not compiled by `lake build` alone, so its theorems would otherwise
  never run (found by that same deliberate-break check).

  **Residual (recorded, not fixed):** the element loops still accept a remainder *without* a preceding
  comma (`[1 ..._]`), which the grammar's `","`-separated list does not. The parser is more permissive
  than the BNFC there; the *soundness* direction of law 30 (every accepted term is in the grammar) is
  what would pin it, and that row is not landed yet.

- **C25 (fixed) — `Group!("new", …)` answered nothing because it read a dictionary genesis never
  writes.** A wallet-case failure (`newGroup`); the symptom was that `Group.rho`'s `new` contract
  (`:36`) printed its first line (`"creating Group."`, `:48`) and then nothing — no
  `["got directory", …]` (`:60`), no reply. Three statements were the suspects: `:49`'s linear consume
  of `groupMapCh`, `:50`'s `if (groups.get(name) != Nil)`, and `:55`'s dictionary peek. Read *before*
  the corpus existed, the candidates could not be separated; the tests below separate them by
  observation.

  **The ruling.** `a_fresh_chain_answers_one_group_creation` and
  `a_fresh_chain_answers_two_group_creations_in_one_deploy`
  (`casper/tests/genesis_registry.rs`) drive the contract under the ceremony key, with a witness on
  `@[*deployerId, "dictionary"]`. Both failed, and the tags named the site: the dictionary witness was
  **present**, `"creating Group."` printed **twice**, and no call answered. So `:55`'s *precondition*
  held — the caller's dictionary exists — and yet nothing after `:49` ran. The cause is identity:
  `Group.rho:6` binds `deployerId` at **registration** time, and `:55` peeks
  `@[deployerId, "dictionary"]` — the locker `MemberDirectory`'s `createMe` writes for whoever ran it.
  Upstream deploys the whole rgov set from one funded key, so registration and caller identity coincide;
  genesis signs each class with its own fixed key (`contract_key`), so the dictionary `Group` peeks
  belongs to the class deploy and **nothing ever writes it**. The peek never fires, nothing between
  `:49` and `:60` runs, and an unmatched receive is not an error — so the whole group class was
  unreachable, silently, and a wallet's `newGroup` hung on it. `:49`'s consume and `:50`'s `if` are
  both *sound*: a shape test ruled the `if` out (an `if` inside a receive body runs its `else`;
  pinned now as `if_inside_a_receive_body_runs_its_else_branch` in `rholang/tests/if_par.rs`), and the
  store restores — see the law-41 note below.

  **The fix.** A behavioural repair at render time (`casper/src/genesis/rgov.rs`'s `source("group")`,
  recorded in `resources/rgov/NOTICE`): the dictionary peek becomes a lookup of the master read
  capability by its **constant** URI (`readcap_uri`) — the path the wallet's own `getMe` handshake
  already takes — reusing the `lookup` the file binds at `:8`. Both tests pass with it, and it is the
  only change.

  **Law 41's reading, measured rather than argued** (INVENTORY row 41). The earlier entry here read
  `:49`'s deferred write-back as a *permanent* loss when the `:50`-`:66` chain fails. With `:55` fixed,
  the chain completes, so the datum *does* go back at `:65` on the success branch and at `:52` on the
  error branch: the store survives, and a second concurrent `Group!("new")` is **serialized** behind the
  first rather than lost — which is what the two-call test now pins. The window between `:49` and `:65`
  remains a latency property of this contract, not a law-41 violation, and the law's own behavioural
  form (`storeSurvives`, `spec/conformance/store.tsv`) is what says so.


- **C26 — law 5 was three `axiom`s, one of them false, and nothing checked any of them.** Found while
  replacing law 5's `spatialMatches` with a definition. `spec/Rchain/Match.lean` stated

  ```lean
  def BindsAtMostOnce (p : Par) : Prop := ∀ n m : Nat, freeVarOf p n → freeVarOf p m → n = m
  axiom pattern_binds_at_most_once (pat : Par) : BindsAtMostOnce pat
  ```

  and `freeVarOf p n` (`FreeVars.lean`) means "level `n` occurs free in `p`" — so the axiom says a
  pattern has **at most one variable**, which `{"a": *x, "b": *y}` refutes. An `axiom` is trusted, so
  the development could prove anything: the formalization that `AGENTS.md:55-57` calls the oracle was
  *unsound*, and the reason nobody noticed is the same reason the ten silent defects went unnoticed —
  **no CI job built the Lean at all** (that is what the `formal` job and
  `tools/check-lean-conformance.sh` now do).

  **Fix:** the relation is defined (`spatialMatchCore`/`spatialMatchExprs`/`spatialMatchExpr`/
  `matchListPar`/`matchMap`, executable over `Par`, with explicit fuel so the kernel can reduce them);
  `spatialMatches` is `spatialMatch … = true`; decidability became an `instance`, not an axiom; and
  law 5 is stated as it is in the implementations — **an accepted match binds each free level at most
  once** (`spatialMatch_implies_linear`, proven) and a pattern that repeats a level is *refused*
  (`UnexpectedReuseOfProcContextFree` in the port, `addedVars.distinct` in the Scala). **Pinned by**
  `matchCases_decide` (15 `decide`d cases in `Rchain/Corpus.lean`), `spec/conformance/match.tsv`, and
  `rholang/tests/lean_match_corpus.rs` — which runs each case through the node's own path on its own
  runtime, including the twice-bound pattern as a `rejected` row (the shape the false axiom denied).

  **Also found while building it, and recorded rather than smoothed over:** the matcher's fuel was one
  step short, so `@[]` against `[]` and `@Set(1, ..._)` against `Set(1, 2)` answered **false** — the
  corpus's `decide` refused to compile until the bound was right, which is the mechanism working as
  intended. The saturation of `matchFuel` is `fuel_saturation` in `Rchain/Match.lean`, stated and owed.

- **C27 — the base-sort receive never said which channel it listens on.** Found while writing law 38's
  rule (`Rchain/Silence.lean`). `Rho.lean`'s `receivePar` builds its bind as
  `ReceiveBind.mk [chan] body 1`, and in the `ReceiveBind` record the fields are
  `(patterns, source, freeCount)` (`Par.lean`'s accessors) — so the *body* sits in the `source` slot and
  the channel sits in `patterns`. The model's receive therefore has no channel in the field that means
  channel, which is invisible in `Rchain.Rho`'s own theorems (they are about `Reduce` abstractly) and
  fatal to any reasoning that asks *which channel a receive listens on* — law 40's whole question.

  **Fixed** (a later pass): `Rho.lean`'s `receivePar` now builds `ReceiveBind.mk [anyPat] chan 1`
  — the channel in the *source* slot, a **bound** variable as the pattern (a wildcard would match
  anything but is not closed under the model's `Closed`, which checks a bind's pattern with the same
  judgement as its channel) — and `Ty.lean` gained `closed_anyPat`, the one fact every `Closed` proof
  over a receive needs. Repairing the four proofs in `Concurrent.lean` and `Ty.lean` that had been
  written against the old shape is what the earlier pass had declined to do: they encode the redex's
  *fields*, so they had to be restated, and one of them (`Closed_receivePar_iff`) spent its whole
  budget unfolding the pattern's `Par` until the fact was stated separately. The original note, which
  is now the reason the fix was a deliberate step rather than a one-liner, follows.

  **Not fixed here, and recorded instead:** `Reduce`'s theorems (`reduce_closed` in `Rchain.Ty`,
  `reduce_freeVars_subset` in `Rchain.Reduce`) are proved against that shape, so changing it is a
  separate, deliberate step rather than a drive-by edit. The pattern-aware layer beside it
  (`Silence.lean`'s `receiveParP`) uses the *correct* order —
  `ReceiveBind.mk [pattern] chan 1` — so law 38's reasoning has both slots right, and the corpus's
  cases exercise the node's real receives, not this model shape.

- **Law 38 — silence, as a rule rather than a remark, and checked.** `Rchain/Silence.lean`:
  `ReduceP`'s only rule that consumes a datum carries `spatialMatches data pattern` as a hypothesis, so
  "no match ⇒ no step, no error" is the rule; `takesStep` computes the same question over the flat
  `Par` (a send/receive pair on one string channel whose pattern matches), which is what lets the corpus
  `decide` against it. The tie between the two is `takesStep_iff_reduces`, **owed** and named.
  `spec/conformance/silence.tsv` carries 6 cases, each `decide`d (`silenceCases_decide`) and each run
  by `rholang/tests/lean_silence_corpus.rs` on its own runtime with a control datum. `spec/INVENTORY.md`
  row 38.

  **Also found by the corpus's first run, in the corpus itself:** a *ground* pattern in a receive must
  be written `@2`, not `2` — the grammar's bind patterns are names (`Name ::= "_" | Var | "@" Proc12`),
  so the bare form is a parse error and the parser was right to say so. The case text was wrong, not the
  node.

- **Law 41 — channel balance, and it is the *replicable* reader that the law is about.** `Rchain/
  Store.lean` states it over the model's flat `Par`: `replicatedRead` is the reader a `contract`
  installs (a replicated receive — replication is the specification's word for "this reader is called
  again", which is the whole question the law asks of the store it reads); `readStore` computes one
  read (the datum is a send whose value the bind's pattern accepts; the residue keeps the replicated
  reader and drops the datum it took); and `storeSurvives` is the law — **is a datum back on the
  channel, and does the result still form a contract step** (law 38's `takesStep`)? A reader that
  restores answers again; one that consumes has consumed the store for good, and that "for good" is
  what used to be unstated (C22 item 1).

  Checked 1:1: `spec/conformance/store.tsv` (5 cases, `Corpus.storeCases_decide`) consumed by
  `rholang/tests/lean_store_corpus.rs`. Every term registers **two** reads of the store, so the model's
  verdict decides the count the node must produce (`2` or `1`), each case runs on its own runtime, and
  the term carries a control datum — a must-not-fire case asserted by absence alone cannot tell "the
  store was consumed" from "the term never ran" (the fixture rule C20's post-mortem records). The five
  are the balanced reader, C22 item 1's defect, its repair (`box!(Nil)`), a restore that lives in only
  one branch of a `match` (with the datum taking the other branch, so the model's conservative verdict
  and the node's behaviour are the same claim — C22 item 3's lesson from the matching side), and a
  restore to the *wrong channel* (which is what makes the model compare channels rather than ask
  whether any send is there).

- **Law 32 — the operator surface, and the four rules its samples had to obey.** `Rchain/Lex.lean`
  holds the port's operator/lexeme table as data (`rholang/src/parser.rs`'s `match c` cascade, read row
  for row) with the `decide`d checks a table can make about itself: the spellings are distinct and are
  punctuation, and **maximal munch** holds — `longestMatchIn` over the table, applied to each row's own
  spelling, must return that row, so a cascade that tested `<` before `<=` would fail the `<=` row.
  Each row also carries a *sample* and what it must observe, which is the half the table cannot check:
  `node/tests/lean_lex_corpus.rs` runs the sample and compares (`value:<json>` through law 42's JSON, or
  `answers:<n>` for the arrows, which is how `<-` — one consume — and `<<-` — two peeks — are told
  apart). AUDIT C10's swap (`/\`/`\/` lexed silently as each other) fails its sample; falsified once on
  purpose, a perturbed expectation fails naming the row.

  Writing the samples turned up four behaviours of the port, each checked against the reference *while
  writing it* rather than assumed — and three are faithful ports of the Scala, which is why they are
  recorded here rather than filed as divergences:

  | Behaviour | Faithful? |
  |---|---|
  | a top-level logical connective in *process* position is refused (`TopLevelLogicalConnectivesNotAllowedError`) | yes — `Compiler.scala:106` raises the same error for the same condition |
  | at bind depth 0, `\/` or `~` in a *receive* pattern is refused (`PatternReceiveError("\\/ (disjunction) at …")`) | yes — `Utils.scala:13-21` is the same check with the same messages, and the port's `fail_on_invalid_connective` is its port |
  | a *bare name* is not a process: `read!(@"c")` does not parse | yes — the reference grammar has `Name ::= "_" \| Var \| "@" Proc12` and **no** `Proc ::= Name`, so the port is right and the sample was wrong (the store layer hit the same rule) |
  | `--` is defined for `Set`s only (`OperatorNotDefined` for a List) | yes — `reduce.rs`'s `EMinusMinus` arm takes `(ESet, ESet)` and nothing else |
  | `match` *case* patterns are not subject to the `\/` depth check (only receive patterns are) | yes — the port checks at two call sites (contract formals, receive patterns), matching the Scala's two |

  The table's boundary is stated in the module: the rest of law 32 (comments, the `_`/`_ident` rule,
  `bundle0`, the number/string/uri literal forms) needs the full lexer that laws 30/31/33 share.

- **C29 — the served OpenAPI document was stale, and nothing held it to the code it describes.** Found
  while writing law 43, and it is the C16 class one level up: a *shape a client reads* that had drifted
  from the shape the node produces, silently.

  `node/src/web/http.rs`'s `OPENAPI_JSON` is a hand-written string served at `GET /api/v1/openapi` — the
  schema a client generates its types from. Nothing checked it against the DTOs, so:

  - `ApiStatus` declared **8** properties while `GET /api/v1/status` serializes **13** — the five
    `--autopropose`/`--propose-on-deploy`/`--manual-propose`/`--admin-http`/`--dev-mode` flags had been
    added to the DTO and never to the document. A generated client simply cannot see them;
  - `LightBlockInfo` declared **15** while the type has **16** — `timestamp` (the same informational
    extension `rho:block:data` carries) was missing.

  **Fixed**, with the check that found them: `Rchain/Envelope.lean`'s catalog is the truth, and
  `node/tests/lean_envelope_corpus.rs` holds *both* parties to it — each row's DTO is serialized and its
  key set compared, and the served document's declared properties compared, for the same row. The two
  stale schemas now declare their missing keys (types included: five booleans and an `int64`, read off
  the DTOs), and the check fails if either party drifts again. Falsified once on purpose (a key removed
  from the catalog): it fails, naming the type and the key.

  **Not fixed, and named:** the document declares no schema at all for `NodeCapabilities`,
  `PooledDeploys` and `FaucetResponse` — three responses a client can call. Law 43's catalog covers
  their *keys* (the catalog is checked against the DTOs), but the *document* has no row for them, so a
  client generating types from it cannot know their shape. Adding those schemas is a doc task this
  slice did not do; the boundary is stated in `Rchain/Envelope.lean` and here rather than left silent.
  The *types* of the declared properties are likewise unchecked (the law pins keys and tags).

- **Law 43 — the envelope, checked against the DTOs and the served schema.** `Rchain/Envelope.lean`:
  the catalog of response envelopes as data (per type: its keys, or a tagged union's `tag → keys`),
  with `envelopeCatalog_decide` checking the table by C16's rule — **no key contains an underscore**, no
  key repeats within a row, every tag is capitalized (a client switches on the tag), names unique, and
  the two row shapes exclusive. That rule is the incident: `DeployExecStatus`'s fields were snake_case
  in a camelCase response, so a client reading `deployResult` got nothing and nothing errored.

  Checked 1:1: `spec/conformance/envelope.tsv` (6 rows) consumed by
  `node/tests/lean_envelope_corpus.rs`, which compares each row against the DTO's own serialization
  (constructing the real type, and for the union each variant's tag and keys) *and* against the served
  `OPENAPI_JSON` document. Two parties, one catalog — which is what turns a hand-written document into a
  checked one, and what found C29 on its first run.

- **C28 — the model's ground scalars were missing two of the five the protobuf has.** Found while
  writing law 42. `Rchain/Syntax.lean`'s `Ground` had `bool`, `int`, `str` — and the protobuf's
  `Expr` has five ground instances (`GBool`, `GInt`, `GString`, **`GUri`, `GByteArray`**). So the model
  could not *hold* a rho value carrying a uri or a byte array: `ExprUri` and `ExprBytes` are two of the
  eleven arms of `RhoExpr`, and they are the two the JSON layer's defects turn on (C16's class is about
  the shapes a client reads). Same shape as C23 and C27: the code had a form the specification's
  language could not name, so no law could be stated about it.

  **Fixed** rather than noted, because the law depends on it: `Ground.uri` and `Ground.bytes` (both
  `List Nat` — code points for the uri, bytes for the array, matching how `str` is modelled), and
  `Sort.cmpGround` extended with the two constructors (its lawful-comparator proofs are written as
  `cases a <;> cases b`, so they absorbed the new arms; the declaration order is now `bool < int < str
  < uri < bytes` and the `.lt`/`.gt` arm pairs follow it). `Par.lean`, `Match.lean` and `Ty.lean` needed
  no change — nothing else matches on `Ground` exhaustively.

  **Still outside the model, and named where it bites:** `GUnforgeable` carries a de Bruijn *level*
  (`gPrivate : Nat`) where the wire carries the name's bytes, so law 42's corpus cannot compare the
  unforgeable leaf's JSON and excludes it from the round-trip's domain (`flatPar`); the node's own unit
  tests pin that leaf. A future slice that needs it would extend `GUnforgeable` the same way this one
  extended `Ground`.

- **Law 42 — the rho-value JSON: the envelope rule and the round-trip.** `Rchain/Json.lean`:
  `parToJE` reads the three fields `RhoExpr` reads (`exprs`, `unforgeables`, `bundles`) and applies the
  **envelope rule** (`envelope`: none → *absent*, one → unwrapped, two or more → `ExprPar`), `jeToPar`
  is the decode, and `render` is the wire text `serde`'s derived representation produces (`{"ExprInt":42}`,
  `{"ExprMap":[["a",…]]}`, `{"ExprUnforg":{"UnforgPrivate":…}}`). `decode_encode` is the law's core —
  `rho_expr_to_par (expr_from_par p) = p` — stated over `flatPar` (the hypothesis the envelope rule
  itself forces: a `par` holding another `par` would be *merged* by the decode, and a one-element or
  empty `par` is the same value as its element or no value at all), and **owed** — the induction is over
  the flat fields and the merge arithmetic is the part still to be written out.

  **Instanced on the corpus and discharged by computation** (`Corpus.jsonCases_round_trip`): the
  general statement is owed, but the twelve cases are not — the kernel reduces each case's encode,
  decode and re-encode and refuses the build if the wire text changed, so the law's core is *checked*
  on the layer while the induction is pending.

  Checked 1:1: `spec/conformance/json.tsv` (12 cases, `Corpus.jsonCases_decide`) consumed by
  `node/tests/lean_json_corpus.rs`. Each case's expected JSON is the *model's* `render` of the declared
  `JE` (not a hand-written string), and the consumer asserts two things against the running node: the
  node's own `expr_from_par` serializes to that text, and `rho_expr_to_par` of the result encodes back
  to the same JSON — the API's round-trip, at the wire level, with the model as the independent party.
  The cases cover the envelope's three counts and every `JE` arm but the unforgeable. Falsified once on
  purpose (one case's expected JSON perturbed): it fails, naming the value, what the node exposed and
  what the model says.

- **Law 40 — arity agreement, and the model had to learn to read it.** The law is "every call in the
  protocol catalog has an accepting receive at the target's arity", and the rule under it is one clause
  of `Rchain/Silence.lean`'s redex search: `stepsInBinds` accepts a send only when the receive's bind
  has **as many patterns as the send has data**, each matching its own datum. Before this slice the
  search failed closed on anything but a single datum (`match b.patterns, s.data with | [pat], [d]
  => …`), which meant the model could not state the question at all — the arity was *outside its
  language*, which is C23's shape one more time. `receiveParPs` is the multi-pattern receive a
  `contract` head becomes.

  Checked 1:1 by six new cases in the silence layer's corpus (7-12) — a 2-arity call to a two-pattern
  receive (accepted), 1- and 3-arity calls to the same receive (silent), **the 2-arity call to a
  three-pattern receive**, which is C22 item 2's production instance (`extraSlots` called the
  directory's `write(@key, @value, ret)` with two arguments and none of the three slots was ever
  written), the 3-arity call that is accepted, and two equal arities whose patterns do not match (so
  the rule reads the patterns too, not only the count). They live in `spec/conformance/silence.tsv`
  rather than a layer of their own because the verdict they need *is* `takesStep` — a call at the wrong
  arity is a silent step — and a second consumer would be a copy of law 38's. The corpus was falsified
  once on purpose (a case's term changed to the wrong arity): it fails, naming the case and which side
  stepped.

  **Not claimed:** the *static* form — a walk over the vendored text pairing every `contract` head with
  every call — cannot see the calls that matter, because the interesting ones go through capabilities
  resolved at runtime (`MCAwrite` is bound by a directory lookup, not by a `new` in the same text).
  That is why this law is checked behaviourally, and why the fix for C22 item 2 was an arity in our own
  term rather than something a linter could have flagged.

- **Law 39 — the reply-shape table stops being prose.** `Rchain/Protocol.lean` holds the urn → reply
  catalog as **data** (`replyCatalog`), and `replyCatalog_decide` checks what a table can check about
  itself: the urns are namespaced (`rho:`/`sys:`) and unique, the reply kind agrees with its slots
  (`none` has none, a `send`/`tuple` has at least one), and — the check with teeth — **the declared
  arity agrees with the arguments as written** (`countArgs`, a bracket-aware comma count, against
  `ReplyRow.probeArity`). That last one is C22 item 2's class in a table: `MCAwrite!("Chat", *C_Chat)`
  against a three-argument `write`, where the call matches no receive and *nothing errors*. A table
  that does not count its own arguments cannot catch it.

  Checked 1:1: `spec/conformance/protocol.tsv` (9 rows) consumed by
  `rholang/tests/lean_protocol_corpus.rs`, which calls each urn with the row's own arguments, receives
  the reply with the pattern its *kind* implies (`send` → n patterns, `tuple` → one `@(…)` pattern),
  re-sends every part on its own channel and classifies each one — so the reply's *arity* is checked as
  well as each slot's shape, and a short reply is a failure here rather than a client's mystery (C18).
  A `none` row is asserted by absence with a control datum, never by absence alone. The check was
  falsified once on purpose (a slot's expected shape perturbed): it fails, naming the slot, the shape
  it got and the row it disagreed with.

  Among the rows: **`rho:block:data`'s three-value reply** — `(blockNumber, sender, timestamp)`, the
  documented extension over the oracle's two — is now a checked row instead of a prose consequence
  (`docs/src/rholang/reference.md:93` specifies the timestamp, `RevVault.rho:207-209` binds all
  three); and `rho:rev:address!("validate", "abc")` answers the parse **error string**, not `Nil`.
  What the catalog does *not* cover is named rather than implied: the crypto urns,
  `rho:rchain:deployerId:ops` and `sys:authToken:ops` take a `ByteArray`, and the surface grammar has
  no byte-array literal (`Ground ::= BoolLiteral | "BigInt(" … ")" | LongLiteral | StringLiteral |
  UriLiteral`), so their shapes stay pinned by `system_process_conformance.rs`'s hand-written probes,
  which build the bytes in Rust. The doc tie is machine-checked too: `tools/check-lean-conformance.sh`
  fails if a catalog urn (or its family) has no row in `spec/API-SCHEMA.md`, so the catalog cannot
  drift into a second copy of the doc.

- **Law 41 — channel balance, and it is the *replicable* reader that the law is about.** `Rchain/
  nothing, so `Directory.rho`'s two reads (`:31`, `:44`) were never this law's defect, and the model's
  `Receive` has a *persistent* flag and no peek one — a peek case would have to assert its own verdict
  instead of deriving it. The static half (a walk of the vendored text) is recorded in C22 item 1 as
  drafted-and-retired, with the three shapes it cannot tell apart.

### Open question (behaviour pinned, oracle not established)

- **`models/src/wire.rs::expr_from_proto` decodes an `Expr` with no instance to `GBool(false)`.** The
  final arm is `None => a::Expr::GBool(false)`: a protobuf `Expr` that carries no `expr_instance` —
  an empty buffer, or a peer's message with the oneof unset — becomes the expression `false` rather
  than an error. Every *other* optional inner message in this file is
  `ModelsError::Malformed(<field>)`, so this arm is the outlier; on the wire-decoding path the
  difference matters, because the stricter reading would reject such a message and this one accepts
  it with a substituted constant.

  **Pinned, not changed**, because the oracle could not be established: the legacy tree does not
  contain the Scala's `Expr.fromProto` (`exprInstance` appears only in the sorter, `RhoType.scala`
  and `implicits.scala`, none of which is the conversion), so whether the JVM node defaults, errors,
  or treats the case as unreachable is unknown. Changing this arm is also a wire-path change that
  could *disagree* with the Scala on a peer-supplied block — a fork risk greater than the silent
  default it would remove. Recorded here so the question is visible, and pinned by
  `an_instance_less_expr_decodes_to_the_default`, which carries the same caveat in the test itself
  rather than only in this register.

  **Decided (2026-09-23, Programme A item A5): kept, and the reason is that the oracle *was*
  establishable after all.** The Scala's proto→`Expr` conversion has no arm for an instance-less
  `Expr` either, so a message that decodes to one is not a shape the Scala produces; the port's
  `GBool(false)` is reachable only from a hand-built proto. Keeping it means a hand-built message
  decodes to *something* rather than erroring, which is the more conservative of the two choices for
  a wire path whose other option is a hard failure — and it is pinned by the test named here, so the
  decision is a pinned behaviour rather than an absence.

## 18. The parser's soundness sweep (pass 7): C30–C37

`rholang_mercury.cf` is law 30's oracle — "every term the parser accepts is in the BNFC grammar" —
and nothing held `rholang/src/parser.rs` to it. Reading the parser against the grammar, production by
production, found eight gaps in one class: **every one of them was silent.** Seven are the parser
accepting or refusing the wrong thing; the eighth is a test harness that was measuring the wrong
thing. All eight are fixed, each with a unit test in `parser.rs`'s `mod tests` that was falsified by
removing the fix and confirming the test fails.

- **C30 — `parse` returned `Ok` for a valid *prefix* of its input.** `Tok::Eof` is pushed by the lexer
  (`parser.rs:237`) and was matched **nowhere**, so the parse simply stopped where it stopped and
  discarded the rest: `parse("Nil )")`, `parse("c!(1) garbage")` and `parse("@10!(10) /\ @20!(20)")`
  all succeeded. That is live, not academic: `source_to_adt_with_env` runs on `deploy.data.term`
  (`casper/src/multi_parent_casper.rs:76`), so a deploy whose term was a *prefix* of what the client
  wrote executed part of a program and reported success. **Fixed**: `parse` requires `Tok::Eof` after
  the term. The two logical-connective fixtures in the legacy corpus are the finding's own evidence —
  their comments say their program "cannot be a pair of logically connected processes" and that a
  successful run prints an error, and the corpus had been counting them as programs that reduce. They
  are now rows of the corpus's skip list, in the "meant to fail" bucket.
- **C31 — a trailing separator was accepted at every list site** — and then refused, and then
  accepted again on evidence, which is the part worth reading. The port accepted `[1,]`, `Set(1,)`,
  `{a: 1,}`, `c!(1,)`, `contract c(@x,) = …`, `(1, 2,)`, `a.b(1,)` and `for (x <- c;) { … }` at
  eleven loops, and the grammar's `[X] ::= X | X "," [X]` derives none of them, so the checks were
  added. **They broke real code**: `rchain-community/rgov`'s `rholang/core/CrowdFund.rho` ends each
  parameter of its `contract CrowdFund(…)` head with a comma before a comment, and nine of the Scala's
  own test fixtures (`legacy/casper/src/test/resources/*Test.rho`) end list literals the same way —
  files that *ran* in the Scala suite, which is the evidence that the reference node accepts the
  spelling. A node that refuses the contracts it exists to run has the wrong language, so the
  acceptance is back **as a registered deviation**: stated by
  `a_trailing_separator_is_accepted_as_a_deviation`, named in law 31's deviation list, and recorded
  here. What the pass bought is not the refusal but its *opposite*: the acceptance is no longer
  silent, and the three fixtures are back in the legacy corpus because they parse.

  The original entry follows, because the reasoning in it is still right and the conclusion is not.

  **The finding.** The grammar's lists are
  `[X] ::= X | X "," [X]`, so `[1,]`, `Set(1,)`, `{a: 1,}`, `c!(1,)`, `contract c(@x,) = …`,
  `(1, 2,)`, `a.b(1,)` and `for (x <- c;) { … }` have no derivation, and the port accepted all of
  them — a separator with nothing after it, in eleven loops. **Fixed** at each site, by a shared check
  (`expect_element_after_separator`, and its multiple-terminator form for a bind's names and a
  declaration's values). Two distinctions the fix had to keep, both from the grammar rather than from
  taste: `(1,)` is derivable (`TupleSingle ::= "(" Proc ",)"`), so it stays; and a separator followed
  by a **remainder** (`{name: *voter, ...tail}`) is *equally* underivable but is how the vendored
  contracts are written (`Issue.rho:110`, `Ballot.rho:106`, and 31 such sites), so it is a recorded
  **deviation** — law 31's data list — rather than a rejection. Three Scala test fixtures that end a
  list with a comma are now skip-list rows. The deviation is what makes `[1 ..._]` (no comma) and
  `[1, ..._]` (comma) both accepted: the grammar derives only the first, and the printer emits the
  first, so tightening it would break law 33 and stop the contracts from loading.
- **C32 — `in` was optional in `new` and `let`.** `PNew ::= "new" [NameDecl] "in" Proc1` and
  `PLet ::= "let" Decl Decls "in" "{" Proc "}"` both make the keyword mandatory, and both sites
  dropped `eat_ident`'s boolean (`parser.rs:368`, `:451`), so `new x Nil` parsed as a `new` whose body
  was `Nil`. **Fixed** with an `expect_ident`, which is now the rule for every keyword the grammar
  makes mandatory.
- **C33 — a method call without its argument list parsed.** `PMethod ::= Proc11 "." Var "(" [Proc]
  ")"` has no paren-less form; the port produced a call with an **empty** argument list, which the
  normalizer then handed to a native method that waits for an argument nothing will send — the same
  silent stall as C22 item 2, one layer down. **Fixed**: the `(` is required. (There are two method
  loops — `parse_proc11` and `parse_proc16` — and only the second was lax; `parse_proc11` already
  required the parens.)
- **C34 — `GroundBigInt` was unreachable.** `BigInt(42)` parsed as the *simple type* `BigInt` followed
  by a discarded `(42)`, because `parse_simple_type` matched the identifier before anything could ask
  what followed it — while the normalizer already supported the ground (`normalizer.rs:59`) and the
  protobuf `Expr` carries `GBigInt`. **Fixed**: a lookahead distinguishes `BigInt(` *Long* `)` (the
  ground the grammar names) from `BigInt` alone (the type).
- **C35 — `PSendSynch` was unreachable.** `PSendSynch ::= Name "!?" "(" [Proc] ")" SynchSendCont`
  with `EmptyCont ::= "."` / `NonEmptyCont ::= ";" Proc1`: `Tok::BangQ` was lexed and read by exactly
  one caller (`parse_name_source`, the `for (a!?(x) <- c)` form), so a synchronous send in **process**
  position had no parser path at all — `x!?(1); P` parsed as the bare variable `x` with `!?(1); P`
  discarded, and before C30 the discard was silent. `proc_ast.rs:61` has the variant and
  `normalizer.rs:220` has the arm, so the parser was the only missing link. **Fixed**, including the
  continuation, and the grammar's requirement that one of `.`/`;` follow is now an error of its own.
- **C36 — two lexical forms were accepted by accident and one crashed.** The lexer sliced
  `chars[start..i]` with `i == len + 1` for an **unterminated** string or uri literal — a panic, not a
  diagnostic — and an unterminated block comment silently swallowed the rest of the source, so
  `Nil /* oops` lexed cleanly and parsed as `Nil`. **Fixed**: all three are lexer errors.
- **C37 — `rho_examples` was measuring the stack, not the parse.** The eight findings above were
  chased through a stack overflow in `rholang/tests/rho_examples.rs` whose cause was *not* any of
  them: `qucalc/rholang/gov.rho` parses at ~2 MiB of stack and aborts below it, so the margin was
  small enough that a few bytes per parser frame decided the outcome — which made the overflow move
  between builds and made two wrong diagnoses possible (first "the method-parens requirement", then
  "the `in` requirement", each "proved" by a rebuild that shifted the frame size). `legacy_contracts.rs`
  had already met this and recorded it (`:303-305`, with an explicit 8 MiB stack and the note that the
  depth guard bounds *nesting*, not stack); `rho_examples.rs` never got the same wrapper. **Fixed** the
  way the sibling test fixed it — an explicit stack, with the reason written where it is needed — and
  recorded here because the next marginal example will look exactly like a logic bug. The lesson is
  the one worth keeping: a stack overflow under a 2 MiB default is a *harness* result until the same
  input is shown to fail on an explicit stack.

## 19. The reply-JSON findings (pass 8), and the consolidation pass's: C38–C43

The API's job is *rholang in, JSON out*, and both halves of it were wrong in a way nothing could
catch. This is the finding the whole formalisation programme was started for, found by comparing the
port against the **reference document** rather than against itself.

- **C38 — every rho value in every reply was wrapped wrongly, and the schema file rationalised it.**
  The reference's own types are case classes whose single field is named `data`:
  `final case class ExprInt(data: Long)`, `ExprMap(data: Map[String, RhoExpr])`,
  `UnforgPrivate(data: String)` (`legacy/node/src/main/scala/coop/rchain/node/api/WebApi.scala:131-151`),
  and the node's JSON codec is *derived* from them — so the wire is
  `{"ExprInt":{"data":42}}`, `{"ExprMap":{"data":{"k":…}}}`, and an unforgeable nests one level
  (`{"ExprUnforg":{"data":{"UnforgPrivate":{"data":"ab"}}}}`). The port emitted
  `{"ExprInt":42}`, `{"ExprMap":[["k",…]]}` and a flat unforgeable: three divergences on every value
  of every reply, deploy results and `data-at-name` alike. A client built on the reference document —
  which is generated from those same case classes — cannot read any of it.

  **Why nothing caught it.** Three layers agreed with each other and none with the client:
  `spec/API-SCHEMA.md`'s rule 1 asserted that there was **no** `data` envelope and that it was "the
  Scala/OpenAPI *schema* artifact's spelling, not the wire's"; law 42 (`Rchain/Json.lean`'s `render`)
  was written from the *port's* `#[derive]`d output; and the conformance corpus compares the node to
  the law — so a corpus that passed could only ever confirm that the code and the model were wrong
  together. The one file that could have settled it, `rnode-openapi-schema.ts`, is generated from the
  reference document and was in the tree the whole time. **The lesson is the one the prime directive
  already states**: a law written from the code is a mirror, and a mirror cannot show you a wrong
  shape.

  **Fixed**: `node/src/api/rho_expr.rs` now has `RhoExprWire`/`RhoUnforgWire` — the contract as a
  type, with `Serialize`/`Deserialize` on `RhoExpr` delegating to it — and `Rchain/Json.lean`'s
  `render` renders the contract arm for arm, so law 42 states the *client's* wire form rather than the
  port's. Pinned by `the_wire_shape_is_the_reference_documents` (`rho_expr.rs`, one literal per arm,
  copied from `rnode-openapi-schema.ts`), by the re-emitted `spec/conformance/json.tsv` and
  `lex.tsv`, and by the served document's `RhoExpr`/`RhoUnforg` schemas — which it did not have at
  all before (it said only "a rholang expression").

- **C39 — an exploratory deploy's reply was read from one channel, and a reply anywhere else was
  dropped in silence.** `capture_results` read only the RNG-derived channel — the term's first
  `new`-bound name — and `ExploratoryDeployResponse` was `{expr}`, so a term that replied on `@"out"`
  returned `{"expr": []}`, which is *also* what a term that produced nothing returns. The convention
  existed only in a scratch note; the repository's own test documented the drop as expected
  (`casper/tests/scheduler.rs`: `assert!(res.is_empty(), "no return-channel data expected")` for a
  term sending `42` on `@"chan"`).

  **Fixed**: `capture_results` consults the documented channels in a stated order — the first
  `new`-bound name, then `@"out"` — and returns a `CapturedReply` carrying `ReplySource`, so the
  response names which rule answered or `none`. The rule is in `spec/API-SCHEMA.md`, the served
  document's schema gained `replySource` (law 43's catalog row is checked against both by
  `node/tests/lean_envelope_corpus.rs`), `casper/tests/exploratory_reply.rs` pins all three outcomes
  at the runtime layer, and `node/tests/node_api.rs`'s `genesis_boot_exposes_block_over_http` pins
  both at the **client's** surface — a real node over HTTP, `replySource: "out"` and
  `expr[0].ExprInt.data == 42`. The document also gained the paths and value schemas it lacked: `/capabilities`,
  `/deploys`, `/faucet` and `RhoExpr`/`RhoUnforg` (AUDIT C29's named gap).

- **C40 — law 38's tie was stated as an `iff` that is false, and the relation was missing the clause
  the same module says it carries.** Two defects in one module, found by trying to *prove* the owed
  axiom rather than by reading it, which is the only way this class surfaces:

  1. **The `iff` is false.** `takesStep_iff_reduces` read
     `takesStep p = true ↔ ∃ q', ReduceP p q'` for **every** `p`. Refute it with `chan = nilPar`:
     `ReduceP.comm` fires with `data = pattern = nilPar` (`spatialMatches` accepts them and the rule
     never looks at the channel), so the right-hand side holds — while `stepsInBinds` answers `false`,
     because it compares channels with `stringChan` and `stringChan nilPar = none`. A false axiom is
     not an owed proof; it is the state C26 found law 5 in, where anything follows from it. The
     restriction is not arbitrary slack: `stringChan` is a *decidable* `String` identity, and the
     model's `Par` has no `DecidableEq`, while its canonical comparator is a well-founded recursion
     that `decide` cannot unfold — so the computation can only see string channels, and the corpus's
     verdicts depend on exactly that. The statement now carries the domain as data
     (`allStringChans`), plus `takesStep_sound` on its own — the direction the corpus leans on ("the
     search reported a step, so a step exists"), which needs no restriction, since a `true` from the
     search already implies both channels were string channels.

  2. **The relation could not express the arity clause.** The module's own docstring says law 40 lives
     in one clause of the rule, and `stepsInBinds` reads the arity — but `ReduceP` could contract only
     a send of *one* datum against a single-pattern receive. A two-argument call against a
     two-argument contract is a step by the search and had **no derivation**: the relation was strictly
     weaker than the computation it is the specification of. `commPs` supplies it — the arity
     hypothesis and the pairwise match — which is what makes law 40's question a question about the
     rule rather than about an implementation detail.

  Both statements are now true and precisely scoped; both proofs remain owed, and the second is the
  smaller job (a structural induction reconstructing the redex the search found, with the head-peeling
  congruence chain). Recorded here rather than discovered later by someone proving a falsehood.

- **C41 — the numeric-channel diff accumulator can overflow, and the merge beside it cannot.** Found by
  the consolidation pass that re-modelled law 17: it read the *arithmetic* instead of the law, and the
  two halves disagree at the boundary. `calculate_number_channel_merge` adds with `checked_add`
  (`rholang/src/merging.rs:102`) and `calculate_num_channel_diff` subtracts with `checked_sub`
  (`:309`), each returning an error rather than wrapping, so a value that would leave `i64` is refused.
  The *accumulator* that sums branch diffs does not use them:

      rspace/src/merger/event_log_index.rs:151    *number_channels.entry(*k).or_insert(0) += *v;
      casper/src/merging.rs:758                   *mergeable_diffs.entry(*k).or_insert(0) += v;

  a plain `i64 +=`, so a debug build panics and a release build wraps. Two branches carrying large diffs
  for the same channel reach it, and each diff is itself an unbounded-to-`i64::MAX` `end - prev`, so the
  input is constructible in principle rather than merely theoretical.

  **The law the catalogue carried here was worse than unhelpful, it pointed away from this.**
  `numeric_channels_nonneg` claimed numeric channels are non-negative; they are signed `i64` with
  ordinary negative diffs (`merging.rs:161-166`), so no instance of that law would ever have looked at
  an addition. The corrected law (`Merging.lean`) states the checked half with witnesses
  (`checkedAdd_refuses_overflow`) and names the unchecked half as this finding — the test of a
  consolidation pass is whether the *replacement* law can see the defect the old one could not.

  **Fixed** (2026-09-22). Both sites now sum with `checked_add` and return the error in the house style
  (`"number channel diff accumulation overflow: {a} + {b} does not fit i64"`), and the error reaches the
  merge: `EventLogIndex::combine` is `Result<EventLogIndex, String>`, `DeployChainIndex::apply` and
  `compute_merged_state` propagate it with `?` (both already returned `Result`, so the merge path's error
  handling was in place), and `DeployChainIndex::branches_are_conflicting` — a `bool` predicate — became
  fallible too, because answering `true` ("conflicting") on an un-computable sum would have been the same
  silent semantic choice the finding is about. `deploys_are_conflicting` needed no change: it compares two
  existing indices and never accumulates. The test is
  `combining_refuses_a_diff_that_leaves_i64` (`rspace/src/merger/event_log_index.rs`), falsified before it
  was believed — a `wrapping_add` in place of the `checked_add` fails it. The behaviour change is
  registered in §6 (a `Hard fork`, in the row that already carried issue #52).

  Why this shape rather than "answer conflicting and carry on": the port's own choice two lines away is to
  *refuse* (`calculate_num_channel_diff` returns `Err` on a subtraction that does not fit), so refusing at
  the accumulation is the same decision applied to the same class — and a merge that refuses is loud,
  while a merge that silently picks a branch set is not.


- **C42 — law 5's linearity is enforced by the normalizer, not by the matcher, and the model had it
  backwards about which paths are reachable.** The consolidation pass re-modelled law 5 on the matcher:
  `aggregate_updates` (`rholang/src/matcher/spatial_matcher.rs:644-665`) raises
  `BugFoundError("Aggregated updates conflicted with each other")` when two contributors bind the same
  new level, while `fold_match` (`:595-629`) and `ConnAnd` (`:325-334`) thread their maps with no check
  — so a pattern binding a name twice would be an error on one path and a silent overwrite on the
  others, and the row said so. **Probed on a devnet, neither path is reachable from a term.** All three
  shapes are refused before any matcher runs, with the same error, in both contexts:

      POST /api/explore-deploy  new result, x in { x!([1,2]) | for (@[v, v] <- x) { result!("bound") } }
        → 400  "Free variable v is used twice as a binder (at 0:0 and 0:0) in process context."
      POST /api/explore-deploy  new result, x, y in { x!(1) | y!(2) | for (v <- x & v <- y) { … } }
        → 400  "Free variable v is used twice as a binder (at 0:0 and 0:0) in name context."
      POST /api/explore-deploy  new result, x in { x!({"k": 1}) | for (@{"k": v, ...v} <- x) { … } }
        → 400  "Free variable v is used twice as a binder (at 0:0 and 0:0) in process context."

  raised at `rholang/src/normalizer.rs:111,289,590,1325`
  (`UnexpectedReuseOfNameContextFree`/`UnexpectedReuseOfProcContextFree`, `errors.rs:39,49`) — the
  **normalizer**, i.e. upstream of the matcher and upstream of the receive's channels. A duplicated
  *datum* is unaffected, as it should be: `new result, x, a in { x!([*a, *a]) | for (@[p, q] <- x)
  { result!([p, q]) } }` returns 200 with the list and the same unforgeable hash twice.

  **This is the good outcome, and it is not a defect**: the law holds, enforced earlier than the model
  claimed, and the matcher's aggregation-path error is defence-in-depth on a state the front end cannot
  produce. The corpus's `rejected` verdict for the twice-bound shape is still the right document of the
  *matcher's* behaviour, because its consumer (`rholang/tests/lean_match_corpus.rs`) feeds bind/datum
  pairs to the matcher directly rather than parsing a term. What was wrong was the model's note, which
  implied a term could reach the silent paths; the note and the row now say what the probe showed.

  **Pinned by tests, and the guard map measured (2026-09-24, Programme F).** The probe above was a
  devnet observation, and the invariant it establishes — a twice-binding pattern is refused — had **no
  test at all**: `UnexpectedReuseOfNameContextFree` appeared in the crate only at its two raise sites.
  A regression there would have reached the matcher, i.e. the exact silent-merge shape this finding is
  about. `normalizer.rs`'s test module now pins four refusal shapes and three negative ones (a
  duplicated *use*, repeated wildcards, and two patterns each binding the same spelling, which must all
  be *accepted*), and **which guard covers which shape was measured by breaking each in turn** rather
  than inferred:

  - `normalize_proc`'s process-context arm (`:289`) refuses a duplicate inside a *process* pattern —
    the `@`-quoted collection forms and the `match` case (breaking it fails exactly those three tests);
  - the receive-bind free-map **merge** (`:1325`) refuses a duplicate across a *join*'s binds
    (breaking it fails exactly the join test);
  - the name-context arm (`:111`) and `handle_proc_var`'s remainder check (`:590`) are **not exercised
    by any test in the suite** — this row listed all four as "the enforcing check" without saying which
    shape reaches which, and no reachable shape isolates the last two.

  Two of the suite's own mistakes are kept in the test's doc comment because the falsification is what
  found them: three of the four refusal tests asserted only "it errored", so they passed on a *parse*
  error (a `match` case written with an `@`), and one passed on a *sort* error (a `match` target used
  without `*`). They now assert the reuse variant specifically.

- **C43 — the merge's associativity was untested, under a test that looks like it tests the
  opposite.** `rspace/src/merger/state_change.rs:203-238` was named `combine_is_associative`, but its own
  comment said "the monoid law tested here is empty-is-identity", and its assertions were the identity
  plus a **sorted-multiset** agreement between the two orders — not associativity. The associativity the
  merge fold actually relies on (`casper/src/merging.rs:752-755`,
  `to_merge.iter().fold(StateChange::empty(), …)`) was therefore untested, where the *inner*
  `ChannelChange::combine` is tested (`channel_change.rs:35-50`) and so are the identity and the
  right-biased join map (`state_change.rs:502-544`). Not a defect — a coverage gap with a misleading
  name.

  **Fixed.** The misnamed test is renamed `combine_has_an_identity_and_agrees_on_sorted_multisets`, and
  `rspace/src/property_tests.rs`'s `law9_state_change_combine_is_associative` is the test the fold was
  missing: a proptest over arbitrary `StateChange`s **including the join map**, so the right-biased
  overwrite is exercised rather than assumed. Falsified before it was believed — a `combine` that drops
  the left side's added list when the right's is longer makes it fail in 0.01s — which is the same
  standard the register's own checks are held to.

- **C44 — the matcher had no clause for a tuple, and the port has one.** Found by adding two cases to
  the matching corpus (`spec/conformance/match.tsv` 15/16: `@(1, 2)` against `(1, 2)`, which must match,
  and against `(1, 2, 3)`, which must not). The corpus declares each verdict and `matchCases_decide`
  refuses to compile when the model disagrees — and it refused: the model answered **false** for the
  equal tuple.

  The model's `spatialMatchExpr` covered ground values, lists, sets and maps, and every other shape
  answered `false`; `rholang/src/matcher/spatial_matcher.rs:496-501` has an `ETuple` arm. Adding the
  corpus case first is what made this visible, and the *reason it was invisible* is worth recording,
  because the model's own documentation said so in as many words: every unmodelled shape fails closed
  ("the spec claims no match rather than guessing one"), and a pattern that matches nothing produces
  **silence** rather than an error. That is C19/C20/C22's shape exactly — a missing clause read as a
  client bug — surviving in the model instead of in the port, in the one place the docs called a
  boundary.

  The clause set is not the only thing this exposed. Law 37's tie (`concrete_matches_iff_eq`) was stated
  over *every* connective-free pattern, and an arithmetic pattern is connective-free, equal to itself,
  and matched by no clause — so the tie, which the register carried as **owed**, was **false**:
  `arithmetic_pattern_refutes_the_unrestricted_tie` is the term (`spatialMatch p p = false` for
  `p = (1 + 2)`), and the statement now carries `modelledPar` on both sides. The port's arithmetic arms
  are deliberately not modelled: a datum is evaluated before it is stored, so no reachable target carries
  one — the model's `false` is right about every term a client can produce, and the law's quantifier was
  what was wrong.

  **Fixed**: the `ETuple` arm (`Match.lean`, mirroring the port's `fold_match(tlist, plist, None, …)`),
  `freeLevelsExpr` gained the tuple arm too (a level bound twice *inside* a tuple would otherwise have
  passed the linearity check), the corpus carries the two cases, and `rholang/tests/lean_match_corpus.rs`
  holds the node to them — the node agrees on both.

- **C45 — the search claimed a step for a join, and the rule could not have derived one anyway.** Two
  halves of one law-38 gap, both found by the corpus.

  **The search.** `stepsInReceives` walked a receive's binds one at a time, so
  `@"c"!(1) | for (x <- @"c"; y <- @"d") { … }` — a *join* with one of its two channels filled — was
  reported as a step. The node fires a join only when **every** bound channel holds a matching datum, so
  it is silent there. Corpus case 13 of `spec/conformance/silence.tsv` declares `false` and
  `silenceCases_decide` refused to compile; `rholang/tests/lean_silence_corpus.rs` holds the node to the
  same verdict, and the node agrees. The search now requires a **single-bind** receive.

  **The rule.** `ReduceP.comm`/`commPs` built their receive through `receiveParP`/`receiveParPs`, which
  *fix* `freeCount := patterns.length` and `bindCount := 1` — while the search reads neither. So the rule
  could not derive terms the search accepted, and `takesStep_sound` was **false as stated** rather than
  merely unproved. It is not a corner: the port's `free_count` is `count_no_wildcards`
  (`rholang/src/normalizer.rs:1295-1300`), so an ordinary `for (@a, @b <- c)` has `freeCount = 0` against
  `patterns.length = 2`, and the node contracts it.

  **Fixed**: the constructors take `freeCount` and `bindCount` as parameters, and take the channel as
  *two* parameters with a shared-name hypothesis — which is the node's own condition (the send's channel
  and the bind's source are the same name), so the rule no longer needs an injectivity argument about
  `stringChan` to be applied. `takesStep_sound` is now a theorem: three extraction lemmas, one per level
  of the search, and `exists_redex_split`, which presents a par around any send/receive pair as
  contexts plus redex.

  **What is still owed, and why it is stated where it is**: the complete direction
  (`takesStep_iff_reduces`, scoped to `allStringChans` by C40). The model has **no join rule**, so a join
  with both channels filled is a step in the node and has no derivation here; the axiom is a statement
  about the boundary the model actually has, and the row says so.

- **C46 — a validator cannot index the genesis, because the sidecar-regeneration path replays it without
  its vaults** (found 2026-09-23, on the devnet while checking laws 44–47; **pre-existing**, from
  `ca4f5b015`, 2026-08-24). `casper/src/merging.rs:455-524` loads a block's mergeable-channel sidecar
  and, when it is missing, regenerates one by replaying the block — the branch a node takes for a block
  it *received* rather than proposed. Its replay passes `&[]` for the genesis wallet vaults, with the
  comment "this is always non-genesis block replay". It is not: after syncing a finalized fringe, a
  joining validator indexes the **genesis** (block #0), and the genesis's wallet balances are
  `PREFIX_VAULT` leaves *in* the genesis post-state, so the regenerated hash differs from the block's
  recorded one and the validator refuses the block —
  `regenerated mergeable channels for block ae620156… but replay computed ad7be2fa… instead of
  052c997a…`, repeatedly, while the bootstrap proposes on. Observed on
  `tools/devnet.sh up --validators 3`: validators 1 and 2 stop at the height they joined at. **The
  message names the wrong thing**, which is why this survived: the comparison at `:514` is the
  *post-state hash*, not a channel diff.

  **Reproduced in-process** (`casper/tests/determinism.rs::a_genesis_replay_without_the_vaults_does_not_reproduce_the_genesis`,
  which asserts the divergence so that the fix fails it): a genesis replay *with* the vaults reproduces
  the genesis exactly; the same replay without them does not. That test is the finding's evidence and
  its tripwire.

  **Not fixed here, and why**: the fix is conditional (`pre_state_hash == empty_state_hash_fixed()`
  ⇒ re-install the genesis vaults — unconditionally would clobber a post-genesis balance), and it needs
  the genesis vault list reachable from the replay path, which today is only held by the genesis
  ceremony. That is a change to `merging.rs`'s inputs, not a line. So it is registered: `merging.rs`'s
  comment states an assumption the devnet falsifies, and
  `docs/src/formal/determinism.md`'s "genesis-vault re-seed" bullet — which called the asymmetry safe
  because "genesis is trusted and never re-validated (an asserted invariant, not a code path)" — now
  carries the counterexample.

  **The prescribed fix cannot work for the node that failed (found 2026-09-24, Programme F).** Read
  the prescription against the failing deployment: it needs the vault list at the replay path, and the
  node whose indexing fails is precisely the node that does not have one. `tools/devnet.sh:257-270`
  starts validators 1..n−1 with `--bootstrap` and their validator key and nothing else — no
  `--wallets-file`. The vault list has exactly one production source, `vault_parser::parse`
  (`vault_parser.rs:16`) called from `node_launch.rs:66` inside `create_genesis_block` — so it exists
  only in the ceremony, and `parse_if_exists` (`:43`, the tolerant variant) is called by nothing but
  its own test. They also get no `--bonds-file`, and are not `standalone`, so
  `node_runtime.rs:1375-1379` gives them `PosGenesis::default()` as well. A joining validator therefore
  has **neither** the vault balances **nor** the genesis PoS descriptors, and both are installed as
  native state *outside* the genesis block's deploys (`compute_genesis`'s `set_vault_balance` loop and
  `install_genesis`) — so neither is recoverable from the block, and no `&[]`-to-something change to
  `merging.rs` makes its replay reproduce the genesis. The audit's own phrase "only held by the genesis ceremony" is the tell: the
  ceremony node is validator 0, and validator 0's indexing never fails. The tripwire test cannot see
  this either — it hands `replay_compute_state` a `RuntimeManager` that already holds the vaults.

  So C46 needed a decision rather than the prescribed line, and the options differed in what they
  trust: **(a)** give every node the network's genesis config, so the conditional vault re-install is
  correct; **(b)** trust the genesis — for `pre_state_hash == empty_state_hash_fixed()` skip the
  `computed != post_state_hash` equality, keeping the channels the replay computes; **(c)** take the
  sidecar from the post-state instead of replaying. **(c) has no mechanism**, which is why it is
  recorded as considered-and-rejected rather than left as an option: the trie has exactly three leaf
  kinds (`history_repository.rs:27-29`: datum, continuation, joins), so there is no
  number-channel prefix to read, and the sidecar's *key set* — which channels are mergeable — is
  execution-derived (`get_number_channels_data` takes the `BTreeSet<Par>` from each deploy's
  `eval_res.mergeable`) and is absent from the block (`ProcessedDeploy` is `{deploy, cost, deploy_log,
  is_failed, system_deploy_error}`, `casper_message.rs:450-456`, and `mergeable` appears in no
  `.proto`). What *is* reachable is the post-state by hash (`merging.rs:218`), which is what a
  weaker "(b) plus a per-channel check against the post-state" would have used.

  **Decided (2026-09-24, Programme F) — (a), the option that keeps the check.** The genesis files are
  part of the *network configuration*, not of the ceremony, and treating them as ceremony-only conflated
  two different things: whether a node runs the ceremony (`standalone`) and whether it knows the
  network's genesis. It is the second that a replay needs.

  - `casper/src/genesis/mod.rs` gains `genesis_descriptors_from_config(spec, is_ceremony)` returning
    the PoS descriptors **and** the vaults, read on any node that has the files. A non-ceremony node
    reads the bonds file **strictly** (`bonds_parser::parse`) and may not generate one — autogenerating
    a validator set on a syncing node would mint a *different* chain's genesis than the one it is
    syncing, silently. The vaults use `vault_parser::parse_if_exists`: a node whose configuration names
    an absent wallets file has no balances to re-install, which the caller can act on.
  - `RuntimeManager` carries them alongside `genesis_pos` (which already had this shape), and
    `node_runtime.rs` fills both from one call instead of gating on `standalone`.
  - Both genesis-replay call sites — `interpreter_util.rs::replay_block` (validation) and `merging.rs`'s
    sidecar regeneration (indexing, the one observed failing) — take the vaults from
    `runtime.genesis_vaults()` when `is_genesis_pre_state(pre_state_hash)`, and none otherwise. The
    condition is the whole reason the re-install is safe: a later block's pre-state already carries the
    balances, so an unconditional re-install would clobber a post-genesis one.
  - `tools/devnet.sh` mounts `/genesis` into validators 1..n−1 and passes them
    `--bonds-file`/`--wallets-file`, which is the deployment half: they were the nodes without the
    config, and read-only is correct because they must not generate a bonds file.

  **What this does not fix, stated rather than implied.** A node that genuinely has no genesis config
  still cannot replay the genesis: it replays with `PosGenesis::default()` and no vaults, computes a
  different post-state, and refuses the block. That is now a *configuration* failure with a loud
  symptom rather than a silent property of the code, which is the honest place to leave it — the
  alternative was (b), which would have stopped checking that the replay reproduced the genesis at all.
  The default `genesis_block_data` paths (`/genesis/bonds.txt`, `/genesis/wallets.txt`) are what the
  devnet and the `docker` profile use; an operator who puts only the bootstrap's paths in the
  validators' config gets this failure and the message names the block.

  The tripwire test keeps its assertion and changes its role: it pins that the *primitive*
  `replay_compute_state` diverges without the vaults, which is why the call sites must supply them. The
  production decision is pinned by `is_genesis_pre_state_is_true_only_for_the_empty_state`
  (`interpreter_util.rs`, both branches).

  **Verification status, stated exactly — the devnet half is *blocked*, not passed.** What is verified:
  `cargo check --workspace --all-targets` clean, `casper/tests/determinism.rs` 7/7 (including the
  tripwire), `is_genesis_pre_state_is_true_only_for_the_empty_state`, and the register gate. What is
  **not** verified is the end-to-end path, because `tools/devnet.sh up --validators 3` no longer reaches
  a running chain at all: the bootstrap sits at ~100% CPU with three log lines (the last from
  `comm/src/upnp/mod.rs:103`) and never serves `/api/v1/status`, so `up` exits 1 with
  `timed out waiting for devnet-bootstrap to serve /api/v1/status` and validators 1 and 2 log
  `PeerUnavailable` for it without ever receiving a block. **This is pre-existing, and the control is
  decisive**: the same command against the pre-change image (26-hour-old `rnode:local`, retagged
  `rnode:control`) fails identically — 104% CPU, three log lines, the same timeout — so the stall is
  not this change, and it is the same failure the `devnet-fuzz` nightly has hit since 2026-09-14 (below).
  The consequence for this row: since no block was ever produced, no genesis indexing happened and the
  absence of the `regenerated mergeable channels` message in the container logs is **not** evidence that
  the fix works. C46's end-to-end check lands when the bootstrap starts again, and that is its own unit.
  What this row does record is that the two call sites now supply the vaults, that the condition is the
  genesis exactly, and that a node without the files fails loudly rather than computing a wrong state.

  **Two related reds, reported rather than worked around.** (1) The
  `devnet-fuzz` nightly workflow has failed on **every** run since at least 2026-09-14, each time at
  "Start the devnet" (`tools/devnet.sh up --validators 3` → `timed out waiting for devnet-bootstrap to
  serve /api/v1/status`), so its fuzz stages have not executed in ten days and C46 could not have been
  found there. **Local signature (2026-09-24, Programme F), which is as far as this row takes it**:
  reproduced on this machine, and *not* caused by the C46 fix — the pre-change image fails identically.
  The bootstrap container runs at ~100–104% CPU, writes exactly three log lines (the last from
  `comm/src/upnp/mod.rs:103`, "No need to open any port"), and its healthcheck never connects to
  `localhost:40403`; validators 1 and 2 come up "healthy" on their own ports and log `PeerUnavailable`
  for the bootstrap every 10 s, then give up after 10 attempts. So the stall is a *silent CPU-bound
  spin in bootstrap startup*, after UPnP/port setup and before the first genesis log line — the shape
  that makes it worth a `perf`/stack sample rather than log reading, and a separate unit from C46.
  Diagnosing it is what unblocks C46's end-to-end check. (2) The 1-validator devnet is green, including a live check of law 47 (a staged
  withdrawal keeps the validator active: `examples/pos-withdraw.rho` → `(true, Nil)`, the block's bond
  cache still lists the validator, and the chain keeps extending).

- **C47 — the matcher's fuel was short on a shape the node matches, because the measure it was derived
  from did not count an empty `Par`** (found 2026-09-23, Programme D unit 8, while *designing*
  `fuel_saturation` rather than while testing; **the model's defect**, the port is right). The Lean
  matcher carries explicit fuel so the kernel can reduce it (`decide` checks every corpus case against
  the clauses). `matchFuel` was `2 * (parNodes target + parNodes pattern) + 4`, and `parNodes` of a
  `Par` was its expression list's count — so a `Par` whose `exprs` field is empty counted **zero**
  nodes, while the matcher spends a step walking past it whenever it sits in a collection the *search*
  member is matching. `@Set(1, ..._)` against `Set(Nil, Nil, Nil, Nil, Nil, Nil, 1)` measured
  `parNodes = 2` for both sides, so `matchFuel = 12`; the walk plus the descent needs 13. The model
  answered **false** where the node answers **true** — an under-claim. Six `Nil`s is the boundary: at
  five the old measure answered `true` too, which is why none of the 17 cases that shipped, none of them
  padded, could have found it. Fixed by counting the `Par` itself (`parNodes (.mk …) = 1 + …`), kept by
  `Match.lean`'s `the_walk_past_empty_pars_is_paid_for` and `match.tsv` case 18, and the `+1` turns out
  to be exactly the term the induction needs: with it the element case of the budget is exactly tight.

- **C48 — the spec over-claimed a match: the *searcher* was wired into the list arm** (found
  2026-09-23, same unit, by the corpus case C47 added; **the model's defect**, the port is right). The
  model's `matchListPar` has a branch that drops a target and looks further — a search for a *distinct*
  counterpart anywhere in the target collection. That is the port's rule for **sets and maps**
  (`list_match_single` → `find_matches`, a bipartite assignment) but **not** for lists or tuples, whose
  arms are `fold_match`: strictly positional, heads paired and tails recursed, with the remainder taking
  the tail (`rholang/src/matcher/spatial_matcher.rs:467-493`, `:496-501`;
  `legacy/…/SpatialMatcher.scala:482,490`). So the model matched `@[1, ..._]` against `[Nil, 1]` — and
  `@[1, 3, ..._]` against `[1, 2, 3]` — while the node matches neither; **measured** on the node through
  the corpus consumer's own path, at `Nil`-padding 0, 1, 2 and 3. This is the direction the boundary
  note said the corpus exists to catch: the spec claiming a receive fires when it does not. Fixed by
  splitting the members — `matchListPos` for `.elist`/`.etuple`, `matchListPar` for `.eset`/`.emap` —
  kept by `Match.lean`'s `a_list_pattern_cannot_skip_a_target_element` and `match.tsv` case 19, both
  measured against the node.

- **C49 — the replay property test fails on its own recording, roughly three runs in ten** (found
  2026-09-23 by CI, on a *docs-only* commit `1e14c2e4f`, so it is pre-existing and unrelated to that
  change; reproduced locally). `rspace/src/property_tests.rs`'s
  `law11_a_replayed_script_matches_its_recording` plays a randomized script of produces and consumes,
  records the trace, rigs a replay runtime with it, replays the same script, and asserts
  `check_replay_data()` is `Ok` — the *reverse* half of law 11 (no recorded COMM left unconsumed).
  It panics with "a replay of its own recording must check clean".

  **The flake is a deterministic failure on one input, and it is now pinpointed.** Proptest's
  regression seed is committed (`rspace/proptest-regressions/property_tests.txt`, seed `c9f7be73…`), so
  every run replays it: running with `PROPTEST_CASES=1` fails **8 times out of 8** in 0.03 s. The input
  is five operations:

  ```
  op1 consume c2 "pattern" (non-persistent)   op2 produce c2 "d2" (persistent)
  op3 consume c2 "pattern" (persistent)       op4 produce c2 "d2" (persistent)
  op5 produce c2 "d0" (non-persistent)
  ```

  **What the check reports, exactly** — instrumenting `check_replay_data`'s message to name the leftovers
  (reverted) shows the reverse check leaving **one** COMM (counted twice: once under its consume key and
  once under its produce key, hence "2 elements left"):

  ```
  leftover consume keys  [88 f3 cb f8 …]   the persistent consume (op3)
  leftover produce keys  [58 42 9a 76 …]   the first persistent datum (op2)
  ```

  So the one COMM the replay fails to reconstruct is the persistent consume matched against the datum
  that was *already there* — and, importantly, the replay raises **no** `ReplayCommNotInTrace` while
  leaving it: it runs all five operations, and the forward half of its check never fires. A replay that
  neither consumes a recorded COMM nor rejects it is the shape to chase: either the produce-side lookup
  (`comms_for_produce`) misses the COMM the recording holds for that produce, or the matching step in
  the replay finds no candidate and stores the continuation instead — and the second would mean the
  replay's tuple space differs from the play's at that point, which the test builds from the play's own
  checkpoint root.

  **The failing input, and a case-aligned trace** (instrumenting `locked_produce`/`locked_consume`, both
  candidate searches and the replay's store reads; all reverted). Proptest shrinks the committed seed to
  six operations, and `PROPTEST_CASES=1` fails on it deterministically in ~0.03 s:

  ```
  ops = [(false,false,2,2), (true,true,2,2), (false,true,2,1),
         (true,true,2,2),  (true,true,2,1), (true,true,0,0)]
  ```

  Replayed op by op — the first column is the branch the replay took, the second what the *recording*
  offered it for that operation:

  ```
  op1 consume (non-persistent)  recorded = [(consume 3821…, non-persistent)]
  op2 produce (persistent)      recorded = [(consume 88f3…, persistent), (consume 3821…, non-persistent)]
  op3 consume (persistent)      recorded = [(consume 88f3…, persistent)]
        store read at op3: data_in_store = 0, matched = 0        <- its recorded COMM is left
  ```

  **Two measurements, and they do not require the log's ordering to be believed.** First, at op3 — a
  persistent consume whose recorded COMM *exists* — the replay's own store is read and holds **zero**
  data on the channel, so there is nothing to match and the recorded COMM stays in the multimap: that is
  the leftover the reverse check reports. Second, at op2 the recording offers the replay a COMM whose
  consume is a **persistent** continuation, and op1's is the only non-persistent one — so the recording
  holds a pairing with a continuation created by a *later* operation (legitimately: the play formed that
  COMM when op3's consume arrived and matched op2's still-present datum).

  **Where that leaves the search.** The play's and the replay's commit paths are structurally identical
  (`rspace.rs:300-329` against `replay_rspace.rs:389-425`), and both respect the *datum's* persist flag
  when removing matches — so the divergence is not in committing a COMM but in *which* COMMs the replay's
  searches accept, and in what ends up in its store. The measured pair — a produce that is offered a
  COMM it cannot have formed yet, and a consume that then finds an empty channel — is the shape to chase
  next: either the replay's produce takes the candidate path (and so never stores the persistent datum
  the play stored on arrival), or the store's contents diverge earlier than the trace shows.

  **A clue that was my own instrumentation, corrected here.** Running the same prints on the play side
  showed `options = 1` for op1's consume — the first operation, on a fixture that builds a fresh in-memory
  store per case — and the previous revision of this entry recorded that as "a candidate on an empty
  store". It is not: `extract_data_candidates` (`space_matcher.rs:57-82`) returns a `Vec<Option<…>>` with
  one element *per channel-pattern pair*, `None` when that channel has no matching data — so a
  single-channel consume reports `1` whether or not anything matched. The play-side trace therefore adds
  no clue, and the correction is written down because a register that keeps a wrong measurement is worse
  than one that keeps none.

  What that trace *does* show, once read correctly, is agreement: op1's consume on both sides stores its
  continuation (nothing to match), and op2's produce matches it. The remaining asymmetry to test is the
  *produce* path: if the replay's `run_matcher_produce` rejects a recorded COMM the play accepted, the
  replay stores a datum the play did not, its store gains a datum, later pairings shift, and exactly one
  recorded COMM is left over — which is the shape measured at the top of this entry.

  **A failed hypothesis, recorded so it is not retried**: rigging the replay with the *pre*-play state
  (taking a checkpoint before the operations, rather than the test's post-play one) changes nothing —
  the trace above is identical.

  **Resolved (2026-09-24, Programme F): the hypothesis above was the right answer, and the experiment
  that dismissed it was the defect.** The failing shape is fully explained by the fixture, and **no
  production line changed** — `rspace/src/property_tests.rs` is `#[cfg(test)]` (`lib.rs:25-26`), so
  this is a test-only repair and carries no consensus risk. (It does not follow that `ReplayRSpace` is
  proven correct: it follows that this observation was not evidence against it, which is the whole of
  what the row claimed.) The test rigged the replay with the play's **post-play** root:

  ```rust
  let recorded = play.create_soft_checkpoint().await.log;
  let root = play.create_checkpoint().await.expect("checkpoint").root;   // ← after the script
  ```

  `create_checkpoint` commits the hot store and **drains the event log** (`rspace.rs:592`, the Scala's
  `eventLog.getAndSet(Seq.empty)`), so `root` is the state *after* all six operations — and
  `rig_and_reset(root, recorded)` therefore started the "replay" from a half-finished tuple space.
  Measured before the first replayed operation, on the failing input: `data(c2)=0, conts(c2)=1,
  joins(c2)=1` — one continuation already installed. The play's op3 leaves its persistent
  continuation installed, so the replay began with a continuation the play had not yet created: at
  replayed op2 the produce-side search found `match_candidates=1` while iterating the *later* COMM,
  and the ops that follow shift by one pairing each. The leftover this entry measured — op3's
  persistent consume against op2's persistent datum — is that shift, not a lost datum.

  So the two candidates this entry recorded were chasing a phantom, and the ~3-in-10 CI rate is the
  fraction of random scripts whose final state still holds something (a persistent continuation, or
  data behind one). **The fix is one line moved**: take `root` before the script, which is also where
  a replay's starting state comes from. `recorded` is unaffected because the checkpoint already
  drained the log, so the soft checkpoint taken after the script returns exactly that script's
  events. The reported trace is byte-identical to the entry above at every operation it recorded.
  Verified: the seed case passes, and the property passes over **4000 cases** (11.5 s) where it
  failed deterministically at `PROPTEST_CASES=1` before. The seed is kept: it pins the input, and the
  audit's own measurement of it is what identified the fixture. **And the repaired test still has
  teeth**, which is the thing to check when a failing test starts passing: making the replay's produce
  path never take its recorded candidate — store the datum instead of calling `handle_match` — fails
  the fixed property at `PROPTEST_CASES=1`, so the fixture fix did not hollow the check out.

  **Why the dismissed hypothesis was wrong, recorded because the failure mode will recur**: the text
  above says "taking a checkpoint before the operations" changes nothing, which is what the fix does.
  I could not see that experiment's code, so the *cause* of the mis-measurement is a hypothesis, not a
  finding — the plausible one is a **soft** checkpoint used for `root`: `create_soft_checkpoint`
  returns `start_root`, the root as of the checkpoint call, so taking it before the ops and reading
  `.start_root` after them yields the post-play root — the same wrong answer, with the right-looking
  edit. What is a finding: the entry had the correct diagnosis in its own text and discarded it on a
  measurement, which is the reason this register's rule is that each row is *falsified before it is
  believed*.

- **C50 — the matcher's fuel was short a *second* time: the measure had no `etuple` case, so a tuple's
  contents were charged to no node** (found 2026-09-24, while *attempting* `fuel_saturation` — the same
  way C47 was found, and by the same kind of instrument: the obligation's arithmetic; **the model's
  defect**, the port is right). `parNodesExpr` had arms for `elist`, `eset` and `emap` and sent
  everything else — `etuple` among it — to `_ => 1`. The tuple clause walks its elements through
  `matchListPos` exactly as the list arm does, and each nesting level costs the matcher 4 units
  (core → exprs → expr → listPos, then the element) while a `Par`-and-expression pair contributes
  exactly 4 — so with the arm missing the budget was *constant* while the walk deepened, and `matchFuel`
  stopped bounding the step count. The smallest case needs no padding: `@((1, 2), (3, 4))` against
  itself needs 13 and was given 12, so the model answered **false** where the node answers **true**. A
  search over generated shapes (`#eval`, 300 shapes, deterministic seed) put the rate at **40 of 300
  matching shapes rejected**; three-deep tuples are rejected at *every* depth reachable by `matchFuel`
  growth, because the constant did not grow at all. Fixed by counting the tuple's elements
  (`parNodesExpr (.etuple ps) = 1 + parNodesListPar ps`), kept by `Match.lean`'s
  `a_nested_tuple_is_paid_for` / `a_tuple_pays_for_its_own_contents` and `match.tsv` case 20.

  **The consequence worth more than the defect: an *axiom* of the register was false while it stood.**
  `concrete_matches_iff_eq` says `(spatialMatch t p = true) = (t = p)` for modelled, connective-free
  pairs. A tuple nested three deep is modelled, connective-free and equal to itself, and the short
  measure made the matcher answer `false` — so the axiom's conclusion evaluated to `false = true`. The
  refutation is two lines of `decide` (written during this pass and *not* kept: with the measure fixed
  the same `decide` fails, which is exactly the point — the axiom was falsifiable only while the defect
  stood, and a defect is what its `fuel_saturation` companion exists to rule out). What is left is the
  proof obligation the row already owed, and the lesson is the general one for this register: a law can
  be false for a reason no test *of the law* sees, because the corpus only disagrees with the Rust on
  the shapes it happens to contain. The measure is now adequate on every shape the search tried; that
  is *evidence*, not a proof, and the proof is `fuel_saturation`.

- **C51 — the tie's domain predicate admitted a shape the clauses reject, so the tie was false**
  (found 2026-09-24, by asking what the statement says on a *value the model admits* rather than on a
  reachable term; **the model's defect**, and this time in a *statement* rather than in a definition).
  `concrete_matches_iff_eq` said: a modelled, connective-free pattern matches exactly the targets equal
  to it — law 37's tie, which is what justifies the port's fast path
  (`if !pattern.connective_used { pattern == target }`). `modelledPar` accepts
  `Par.mk [] [] [] [e₁, e₂] [] [] [] []` — a legal model value holding **two** expressions — while
  `spatialMatchExprs` has an arm for nothing but a singleton, so `spatialMatch p p` answers `false` and
  `p = p` holds: the tie's conclusion was `false = true`. Two lines of `decide`, and the counterexample
  is kept: `a_two_expression_pattern_refutes_the_modelled_tie`. The axiom is **deleted** rather than
  narrowed in place — the row owes the tie for a pattern with a **singleton** expression list — because
  a changed statement is a changed law, and the honest close of a false domain predicate is to say what
  is owed rather than to assume the corrected version. Same class as C44 one level up: C44 was a missing
  *clause*, this is a missing *hypothesis*. What it says about the method: a domain predicate
  (`modelledPar` here, the "not reachable" argument in several earlier entries) has to be checked
  against every value the model admits, not only the ones a reachable term can be.

- **C52 — a peer's `BindPattern` could carry a negative `free_count`, and the count is not inert**
  (found 2026-09-24, Programme F; **a code defect on the wire boundary**, fixed). `models/src/wire.rs`'s
  `bind_pattern_from_proto` passed the proto's `i32` straight through — `free_count: p.free_count` —
  while its two siblings on the same path validated it: `receive_bind_from_proto` (`:697`) and
  `match_case_from_proto` (`:770`) both do `FreeCount::try_from(…).map_err(ModelsError::Decode)`. So the
  third carrier of a free-variable count was the odd one out.

  **Why it matters, which is that the field is load-bearing rather than descriptive.** `RhoMatch::get`
  (`rholang/src/storage.rs:91-93`) builds the continuation's arguments as
  `(0..pattern.free_count).map(|i| remainder_map.get(&i).cloned().unwrap_or_default())`. A **negative**
  count makes that range empty, so the receive's body is applied with *no* bound values instead of the
  pattern's; a count **larger** than the pattern's free variables pads the extra arguments with
  `Par::default()` (Nil). Either way a peer's message changes execution — silently, with no error, which
  is the "nothing errors" class this register began with, and on the block path. The decoder is
  reachable from a peer: it is the `Serialize<BindPattern>` impl's decode (`:971`), the peer client
  (`casper/src/protocol/client.rs:140`) and the gRPC API (`node/src/api/grpc/tonic.rs`).

  **Fix:** validate at the boundary, exactly as the two siblings do, so the count a `BindPattern` can
  carry is the same kind of value the other two carriers guarantee. It is a refusal at the *declared*
  boundary, which is what `spec/TYPE-SYSTEM.md` §1.6 prescribes — not a clamp, which would be the silent
  partiality the gate exists to refuse. Falsified first: restoring the pass-through fails the new
  assertion in `the_runtime_payloads_round_trip` with "a negative free-count must be refused where the
  message arrives".

  **Two adjacent spots found in the same read, recorded rather than changed** — they are the same
  family and each needs its own decision, so they are named here instead of implied away:
  (1) the `unwrap_or_default()` in the same three lines above is the *other* direction of the same
  hole — a count the pattern cannot satisfy becomes `Nil` rather than an error, and since the count is
  wire-sourced the fix would be to *derive* it from the pattern (`count_free_vars` already exists and
  `rholang/src/registry.rs:71` uses it for exactly that) rather than trust it, which is a
  consensus-relevant change worth its own unit; (2) `FreeCount::from_nonneg`
  (`models/src/types.rs:552-555`) guards the invariant with a `debug_assert!` only, so in release it is
  an unchecked constructor beside the checked `new`/`TryFrom` — the shape `spec/TYPE-SYSTEM.md:108-118`
  forbids. Its wire path is now closed by this fix; its remaining callers
  (`rholang/src/{normalizer,registry,storage_printer}.rs`) pass locally-derived counts, which is why
  this is recorded as an owed tightening rather than a live defect. The durable fix for both is the one
  the sibling structs already have: `BindPattern.free_count` should be a `FreeCount`, not an `i32`, so
  the invariant travels in the type instead of being re-checked at each use.

- **C53 — a store error read as an absent radix node** (found 2026-09-24, Programme F; **fixed at the
  read boundary, with the flattening above it recorded as owed**). `RadixTreeImpl::load_node_from_store`
  read its bytes with

  ```rust
  // `BytesCodec` cannot fail to decode, so the store error is unreachable.
  let bytes = self.store.get(&[node_ptr]).await.ok().and_then(|v| v.into_iter().next().flatten());
  ```

  The comment conflates the *codec* with the *store*: a codec cannot fail to decode, and an LMDB read
  can fail. `.ok()` flattened an I/O error into `None`, which is the same value as "no such node", and
  the two are different facts. The Scala keeps them apart — `store.get1(nodePtr).map(_.map(Codecs.decode))`
  (`RadixTree.scala:569`) leaves a store failure in `F`, where the `<-` in `RadixHistory.scala:27,49`
  short-circuits.

  **What was observable, and what is not.** The downstream `assert!(no_assert, "Missing node in
  database. ptr=…")` (`radix_tree.rs`) reported an I/O failure as a *missing* node, which is a wrong
  diagnosis; and under `no_assert = true` an I/O failure *became an empty node*. **That arm is not a
  defect**: `no_assert` is the oracle's own flag for the root load (`loadNode(root, noAssert = true)`,
  `RadixHistory.scala:27,49`), where a genuinely absent root is the normal fresh-history case. So the
  fix here makes the two facts distinguishable at the boundary — the checked arm's assert now names the
  store — and **does not** stop an I/O failure from becoming an empty node, because `load_node` returns
  `Node`. Saying which of those it is, is the point: a fix reported as closing the hole would be wrong.

  **Owed, and it is the real fix**: an error channel. The Scala's `F[Node]` carries a store failure out
  of `loadNode` and into `RadixHistory.new`/`reset`, whose callers can then refuse; the port's
  `load_node -> Node` and its `History` trait have none, so the only available flattening is a panic or
  an empty node. Closing it means making both fallible — a trait change that reaches `rspace`'s
  consumers — which is why it is named here rather than half-done. Falsified first: restoring the
  `.ok()` fails `history::radix_tree::tests::a_store_error_is_not_a_missing_node`, which asserts both
  halves together (an error surfaces *and* a genuinely absent node is still `None`), so a fix that made
  missing nodes an error would be caught as the opposite bug.

- **The class, recorded once, because it is the consolidation pass's whole justification: an axiom that
  is false is worse than one that is owed, because anything follows from it.** Nine axioms the pass
  removed were not merely unproved — they were false of the code or of the model that carried them, and
  each is now refuted rather than dropped quietly. Seven are refuted by a theorem in the tree
  (`block_number_universal_is_false`, `seq_num_universal_is_false`, `fringe_antichain_is_false`,
  `fringe_monotone_is_false`, `seen_monotone_is_false`, `height_map_universal_is_false`,
  `fringe_identity_order_independent_is_false` — each a value one can write down, which is why a
  *witness* is the ratchet and not a proof attempt); `mergeRandom_comm` is refuted by the code's own
  `merge_is_order_sensitive` (`crypto/src/hash/blake2b512_random.rs:548`); and
  `numeric_channels_nonneg` by the merge's ordinary negative diffs (`rholang/src/merging.rs:161-166`).
  A tenth, `law20_deadlock_freedom`, was unprovable as stated — `PathLt` is not well-founded on paths,
  so no minimum need exist — and is deleted with that reason in its row. The same lesson arrived twice
  before, as C26 (a law-5 axiom the corpus contradicted) and C40 (law 38's tie, refuted by
  `chan = nilPar`); what this pass adds is that the register now *asks* each proved row for its witness,
  so the next one has somewhere to fail.


## 20. The back-sweep: every incident to its law and its case

The programme began with ten defects of one class — "nothing errors" — found on a running node,
patched, and understood only afterwards. This table is the answer to the question they left: *which
law would have caught this one?* Each row names the incident, the law that now covers it, and the case
that fails if the behaviour returns. A row whose third column says **harness** or **retired** is a
finding that no law covers, and it says why rather than leaving the gap to inference.

| Incident | Law | What catches it now |
|---|---|---|
| C9 `(x)` parsed as a one-element tuple | 30, 31, 33 | `Rchain/Surface.lean`'s production table (a tuple is `TupleSingle`/`TupleMultiple`); `rholang/src/parser.rs`'s unit tests |
| C10 `/\` and `\/` lexed swapped | 32 | `spec/conformance/lex.tsv` rows `Conj`/`Disj`, each with a discriminating sample — `node/tests/lean_lex_corpus.rs` |
| C13 printer dropped `bundle` and doubled `|` | 33 | `Rchain/Surface.lean`'s witnesses (`bundle+/-/0/` rows) — the round-trip corpus is `Print.lean`'s, still open |
| C16 `DeployExecStatus` fields snake_case | 43 | `spec/conformance/envelope.tsv`'s `DeployExecStatus` row (variant *and* field keys) + the served document — `node/tests/lean_envelope_corpus.rs` |
| C17 list/set remainders did not parse | 31 | the `ProcRemainderVar` witness rows and the corpus's remainder cases |
| C18 `lookup` wrapped its reply in `(uri, value)` | 39 | `spec/conformance/protocol.tsv`'s `rho:registry:lookup` row + the doc tie to `spec/API-SCHEMA.md` |
| C19, C20 partial collection patterns never matched | 35, 37 | `flags.tsv` (concreteness includes the remainder) and `match.tsv` (a partial pattern's verdict) — `lean_normalize_corpus.rs`, `lean_match_corpus.rs` |
| C21 a non-first `if` was a no-op | 34 | `rholang/tests/if_par.rs` (five shapes, including `if` inside a receive body) — the Lean value-position model is `Surface.lean`'s `normalize` |
| C22 item 1 a reader consumed its store | 41 | `store.tsv`'s five cases + `casper/tests/genesis_registry.rs`'s `a_read_does_not_destroy_the_inbox` |
| C22 item 2 a wrong-arity capability call | 40 | `silence.tsv` case 10 (`write!(key, value)` against a three-argument `write`) — and the *rule* now has `commPs`, which is what makes the arity a rule clause (C40) |
| C22 item 3 a map remainder treated as concrete | 35 | `flags.tsv`'s remainder rows |
| C24 `[1 ..._]` and `@{..._}` | 35, 31 | `flags.tsv` (C24's own case) + the deviation row for the comma form `[1, ..._]` — and the model's derivation of the comma-less form is now *stated* (`ProcRemainder` follows the list with no terminal) |
| C25 `Group!("new")` answered nothing | 41 (measured) | `casper/tests/genesis_registry.rs`'s two group-creation tests; the store *does* restore, so the audit's earlier "permanent loss" reading was wrong and is corrected in place |
| C26 law 5 was three axioms, one false | 37 | the definitional matcher + `match.tsv`; the theorem is the definition now |
| C27 the base-sort receive never named its channel | 38, 40 | `Rho.lean`'s `receivePar` (fixed) + `Ty.lean`'s `closed_anyPat` |
| C28 the model's ground scalars were missing two leaves | 42 | `Rchain/Syntax.lean`'s `uri`/`bytes` + `json.tsv` |
| C29 the served OpenAPI document was stale | 43 | `envelope.tsv` held to the DTOs **and** the served document by `lean_envelope_corpus.rs`; the three missing paths added |
| C30 trailing input accepted | 30, 31 | `parser.rs`'s `trailing_input_is_not_a_term`; the two `Logical-*-in-program.rho` fixtures are the corpus's own evidence and are now skip-list rows |
| C31 trailing separators | 31 | `parser.rs`'s `a_trailing_separator_is_accepted_as_a_deviation` — the acceptance is the *registered* answer, on the evidence of rgov's `CrowdFund.rho` and nine Scala test fixtures, and the three fixtures are back in the legacy corpus |
| C32 `in` optional in `new`/`let` | 31 | `parser.rs`'s `in_is_required_by_new_and_let` |
| C33 a method call without its argument list | 30 | `parser.rs`'s `a_method_call_needs_its_argument_list` |
| C34 `GroundBigInt` unreachable | 31 | `parser.rs`'s `bigint_is_a_ground_and_bigint_alone_is_a_type` |
| C35 `PSendSynch` unreachable | 31 | `parser.rs`'s `a_synchronous_send_parses_with_its_continuation` |
| C36 the lexer panicked on an unterminated literal | 32 | `parser.rs`'s `unterminated_lexical_forms_are_errors_not_crashes` |
| C37 `rho_examples` measured the stack | — **harness** | no law: the finding is that a 2 MiB default is not a parse result. `casper/tests/genesis_registry.rs`'s pattern (`with_big_stack`) is the convention, and C37 records why two diagnoses went wrong |
| C38 every rho value wrapped wrongly | 42 | `rho_expr.rs`'s `the_wire_shape_is_the_reference_documents` (one literal per arm), `json.tsv`/`lex.tsv` re-emitted, the served document's `RhoExpr` schema, and `node/tests/node_api.rs` over HTTP |
| C39 the reply was read from one channel | 39, 43 | `casper/tests/exploratory_reply.rs` (three outcomes) + `replySource` in `envelope.tsv` and the served document |
| C40 law 38's tie was false, and the relation lacked its arity clause | 38, 40 | `allStringChans` scoping the statement, `commPs` as the rule's arity clause |
| C41 the diff accumulator could overflow where the merge refuses | 17 | **fixed**: `combining_refuses_a_diff_that_leaves_i64` (`event_log_index.rs`) fails on a `wrapping_add`, and the error reaches the merge through the now-fallible `EventLogIndex::combine`/`branches_are_conflicting`; `Merging.lean`'s `checkedAdd_refuses_overflow`/`mergeRandoms_perm` state the checked half and the call-site canonicalization |
| C42 law 5's linearity is the normalizer's, not the matcher's | 5 | `Match.lean`'s `aggregateUpdates_rejects_double_bind`/`freeMapMerge_overwrites` state the matcher's halves; the enforcing checks are `normalizer.rs:289` (a duplicate inside a *process* pattern) and `:1325` (a duplicate across a *join*'s binds), **measured by breaking each in turn** and now pinned by tests — four refusal shapes and three negative ones in `normalizer.rs`'s test module, which did not exist before 2026-09-24, when the invariant's only evidence was a devnet probe. `:111`/`:590` are listed in the finding but exercised by no reachable shape; `spec/conformance/match.tsv` documents the matcher in isolation |
| C43 the merge's associativity was untested, under a name that says otherwise | 9 | `Merge.lean`'s `mergeChanges_assoc` (proved) **and** `property_tests.rs`'s `law9_state_change_combine_is_associative`, over arbitrary state changes including the join map; the misnamed `state_change.rs` test now says what it asserts |
| C44 the matcher had no clause for a tuple, and the port has one | 5, 37 | `match.tsv` cases 15/16 (`@(1, 2)` against `(1, 2)` and against `(1, 2, 3)`) + `lean_match_corpus.rs`; the `ETuple` arm in `Match.lean`, and `modelledPar` on both sides of `concrete_matches_iff_eq`, whose old statement is refuted by `arithmetic_pattern_refutes_the_unrestricted_tie` |
| C45 the search claimed a step for a join, and the rule fixed the counts the port computes differently | 38, 40 | `silence.tsv` case 13 (a join with one channel filled declares `false`, and the node agrees) + `lean_silence_corpus.rs`; the search's single-bind requirement, the constructors' `freeCount`/`bindCount`/channel parameters, and `takesStep_sound` — three extraction lemmas and `exists_redex_split` |
| C46 a joining validator could not index the genesis: its sidecar regeneration replayed block #0 without the genesis vaults | 11 | **fixed (2026-09-24)**: `is_genesis_pre_state` conditions the vault re-install at both genesis-replay call sites, `genesis_descriptors_from_config` reads the network's genesis files on any node (`node_runtime.rs`), `tools/devnet.sh` gives validators 1..n−1 the files — and `interpreter_util.rs`'s `is_genesis_pre_state_is_true_only_for_the_empty_state` pins the condition that keeps an unconditional re-install from clobbering post-genesis balances. The row was missing from this table until then, which is its own small finding: law 11 is the law that covers it — a replay that does not reproduce the record — and the table's promise is that every incident names one |
| C47 the matcher's fuel was short: the measure counted an empty `Par` as zero nodes | 5, 37 | `match.tsv` case 18 (`@Set(1, ..._)` against `Set(Nil × 6, 1)`) + `lean_match_corpus.rs`; `the_walk_past_empty_pars_is_paid_for`, and `parNodes`'s doc comment carrying the counterexample |
| C52 a peer's `BindPattern` could carry a negative `free_count`, which silently changed what the receive bound | 5, 37 | **fixed (2026-09-24)**: `bind_pattern_from_proto` validates the count like its two siblings already did, so a message that would have applied the continuation with a wrong number of bindings is refused where it arrives. Falsified first — restoring the pass-through fails the new assertion in `models/src/wire.rs`'s `the_runtime_payloads_round_trip`. The same three lines' `unwrap_or_default()` (a count the pattern cannot satisfy becomes `Nil`) and `FreeCount::from_nonneg`'s `debug_assert!` are recorded as owed, with the durable fix named: `BindPattern.free_count` should be a `FreeCount` |
| C53 a store error read as an absent radix node | 10 | **fixed at the read boundary (2026-09-24)**: `load_node_from_store` propagates the store error instead of `.ok()`-ing it into the same `None` a missing node produces — the Scala keeps it in `F` (`RadixTree.scala:569`). Falsified first: restoring `.ok()` fails `radix_tree`'s `a_store_error_is_not_a_missing_node`, which pins both halves (the error surfaces; an absent node is still `None`). The *flattening above it* is owed and named — `load_node -> Node` and the `History` trait have no error channel where the Scala's `F[Node]` does, so an I/O failure can still become an empty node under the oracle's own `no_assert` root load |
| C49 the replay property test fails on its own recording (~3 runs in 10) | 11 | **closed, and it was not the code**: the fixture rigged the replay with the play's *post-play* root, so the "replay" began from a half-finished tuple space — `rspace/src/property_tests.rs`'s `law11_a_replayed_script_matches_its_recording`, now taking the checkpoint before the script, passes over 4000 cases where it failed deterministically at `PROPTEST_CASES=1`. The seed stays as the pinned input; `check_replay_data` was never at fault |
| C50 the matcher's fuel was short again: the measure had no `etuple` case, so a tuple's contents were charged to nothing | 5, 37 | `match.tsv` case 20 (`@((1, 2), (3, 4))` against itself) + `lean_match_corpus.rs`; `a_nested_tuple_is_paid_for`, `a_tuple_pays_for_its_own_contents`, and `parNodesExpr`'s doc comment carrying the counterexample. While the defect stood it also **refuted** the axiom `concrete_matches_iff_eq` |
| C51 the tie's domain admitted a two-expression `Par`, which no clause accepts — so the tie was false | 5, 37 | the axiom `concrete_matches_iff_eq` is **deleted**; `a_two_expression_pattern_refutes_the_modelled_tie` is the counterexample, and rows 5/37 owe the tie for a **singleton** pattern instead |
| C48 the spec over-claimed a match: the searcher was wired into the list and tuple arms | 5, 37 | `match.tsv` case 19 (`@[1, ..._]` against `[Nil, 1]`) + `lean_match_corpus.rs`; `a_list_pattern_cannot_skip_a_target_element`, and the split into `matchListPos` (lists, tuples) / `matchListPar` (sets, maps) |

**The two rows that are not laws are the two worth keeping visible.** C37 is a *harness* finding —
a measurement that was not a measurement — and no law would have caught it, because the thing that
was wrong was outside the term. The retired static walk over vendored text (AUDIT §17 C22) is the
other: it was drafted, found unable to tell a terminal consume from a deferred restore from a
permanent loss, and retired unshipped rather than shipped with an exception list. A catalogue that
claimed to cover them would be claiming something false.
