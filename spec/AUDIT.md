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
  `unimplemented!`, `assert!`/`assert_eq!`/`assert_ne!` **and their `debug_` forms** (the `debug_`
  forms joined the class on 2026-09-24 — see the blind-spot paragraph below), whitelisting
  `sdk/src/primitive.rs` (the Scala `getUnsafe` escape hatch); the rholang parser's `expect(Tok::…)`
  method is excluded (a method, not `Result::expect`).
- **`unsafe`** — `unsafe {` (must be zero; the crate graph is entirely safe Rust).
- **`silent`** — `try_into().unwrap()` / `try_into().expect(`, and `unwrap_or(0)` /
  `unwrap_or_default()` on a fallible numeric conversion (a fallible conversion must not be
  silently flattened to 0/Default).
- **`escape`** (added 2026-09-24) — a refinement newtype surrendering its invariant: `impl … Deref …
  for`, a public tuple field, **or a public `.get()`** — the third form `spec/TYPE-SYSTEM.md:112-115`
  names, added to the scan on 2026-09-24 (see the blind-spot paragraph), in the three files that hold
  the refinements. The `.get()` form is matched *inside an `impl` block that names a refinement type*
  rather than file-wide, because those files also hold error types with public fields
  (`RefineError(pub String)`) whose accessors are not escapes. This is what makes
  `spec/TYPE-SYSTEM.md` §1.7's "no type escape" rule a check rather than a promise.

**Two classes could not see the defect they named, until 2026-09-24 (Programme F, U2), and both
were falsified by probe before they were fixed.** The `panic` pattern was `\bassert(_eq|_ne)?!\(`, and
a word boundary never occurs before `assert` in `debug_assert!` — `_` *is* a word character — so the
tree's production `debug_assert!`s were invisible to the class that exists for them. The one that
mattered is `FreeCount::from_nonneg`'s `debug_assert!(n >= 0)`: it guarded a constructor that stored
whatever `i32` it was handed, so a negative count crossed the boundary *silently in release builds*
exactly where the guard was compiled out — see C52's row. And `scan_escapes` checked `Deref` and the
public tuple field but **not the public `.get()`** that `spec/TYPE-SYSTEM.md:112-115` names beside
them, so the escape count was green partly because the form was not looked for. *Measured:* with a
`debug_assert!` and a `pub fn get(&self)` on a refinement added to the tree, the gate printed "OK: no
hard production violations" and exited 0. After the two pattern fixes the same probes are reported and
the gate exits 1; a control accessor of that shape on `RefineError` (not a refinement) is *not*
reported; the probes were then removed. An instrument that cannot see the defect it names is not
evidence — this pass has now paid for that lesson twice (the two cost tripwires, and law 5's property
that could not fail). The one site the widened `panic` pattern finds in the tree,
`block-storage/src/dag/message_state.rs:101`'s `latest_msgs ⊆ msg_map`, is whitelisted beside its
justification where the whitelist is defined — an internal self-consistency check on two fields of one
struct, not a value carrying a fallible conversion — and `casper/src/block_random_seed.rs`'s
`debug_assert!(shard_id.is_ascii())` was already listed.

Its `cast`/`lax`/`get` classes are candidate finders (soft reports). **`panic`/`unsafe`/`silent`/
`escape` clean.** The soft counts have moved since this section's original baseline (284 cast / 21
lax / 79 get, post Phase-0 widen) and are re-measured here rather than left to read as current:
**`cast` = 326, `lax` = 12, `get` = 101 (2026-09-24)** — `cast` and `get` drift with the tree, and
`lax`'s drop of two is this pass's own work: both deleted `from_hex` helpers were `unsafe_decode`
callers. The remediation targets are the checklist in the ρ-pure remediation plan; the `cast` and
`get` drift is recorded as a measurement, not diagnosed.

**The `lax` class's hex family, reviewed (2026-09-24, Programme F) — assessed faithful.** `base16`
carries both a strict `decode`/`try_decode` and a *named, documented* lax `unsafe_decode` ("non-hex
characters are silently dropped … Use `try_decode` at untrusted boundaries"), so the laxness is
declared rather than silent — the opposite of the class this register is about. Every production use
of it takes **trusted** input: the genesis key material and `empty_state_hash_fixed` are source
literals, and `rgov.rs`'s `contract_key` decodes a hex string it produced itself
(`base16::encode(blake2b256(…))`). Every ingress uses the checked sibling —
`node/src/web/http.rs:857-860` calls `BlockHash::try_from_hex` and its comment says why ("a malformed
block hash … must be a 400, not a panic in `BlockHash::from_hex`"). Two footguns are named rather
than actioned: `BlockHash::from_hex` — lax *and* panicking (via `from_slice`) on a short decode — and
its `crypto` twin `Blake2b256Hash::from_hex` were **deleted** (2026-09-24, U1 site 3): each had no
production caller anywhere in the workspace, only its own round-trip test, and every ingress already
used the checked sibling (`try_from_hex`/`from_hex_either`). One more is named rather than acted on:
`rgov.rs`'s `derive_uri` has no callers at all — dead public API in `casper/`, so removing it is a
cleanup decision rather than a tightening (nothing reaches it today). The `models/src/string_syntax.rs`
half was **actioned** (2026-09-24): its two lax decoders — `unsafe_decode_hex` and
`unsafe_hex_to_byte_string`, both `base16::unsafe_decode`, which silently drops non-hex characters —
were deleted, because a decoder whose name says "unsafe" and whose body skips validation is the one a
future caller reaches for when it wants "just get bytes out of this", which is what
`spec/TYPE-SYSTEM.md` §1.6 forbids. Measured while doing it, and worth recording because it is wider
than the naming implied: **all seven methods of that module have no production caller** — it is
ported surface kept for fidelity, not a live API with two dead methods, and the checked half stays as
that surface.

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
- **H6 — DAG message-state whole-map clone per insert.** **Fixed (partial), and the copies are gone
  as of the 2026-09-24 performance pass.** `Finalizer` borrows the message map (`Finalizer<'a>` holds
  `&'a BTreeMap`) instead of cloning it on every `create_message`; both call sites
  (`message_state.rs`, `multi_parent_casper.rs`) pass a reference. `insert` no longer clones
  `dag_message_state` per block (the guard is scoped to the synchronous region), `get_representation`
  returns `Arc<DagRepresentation>` instead of a by-value deep copy, and `Message.seen` is
  `Arc<BTreeSet>` so a `Message` clone is a refcount bump — at N=1,200, 200 reads take 0.17 ms against
  459 ms with the copy restored, and 50 inserts 230 ms against 339 ms, with a tripwire on each
  (`casper/src/dag.rs`; C56's row in §20 carries the attribution and the calibration trap: the larger
  figures a first pass quoted describe the pre-Stage-3 tree, and neither bound caught its own
  falsifier until it was re-measured). The per-message `seen`
  reachability cache remains an inherent Θ(N²) *residency* — Σ|seen| × 32 B ≈ 553 MB at a
  5,881-block chain — faithful to Scala's `seen` cache, and capping it would break finalization. That
  residency is the residual this row still records: **what was fixed is every copy of it, not its
  size.** A future pass that wants the size gone has a known, value-preserving design (a dense bitset
  with a hash→index table, ρ ≈ N²/8 bytes) recorded in C56's §20 row.

  **The tail (2026-09-24, Stage 5/6) finishes the "every copy" half and makes the residue measurable —
  and the residue is smaller than this row's earlier figure said.** Measured on the running node
  (isolated, the 5,844-block artifact, now at 5,937 blocks): **1.20 GiB RSS, with the DAG's own
  `logical_bytes` at 556 MB of that** and `seen_entries = 17,319,555` (N(N+1)/2, read off `/metrics`) —
  the ~9.8 GiB the row used to quote was the pre-Stage-1/3/4/5 tree in which every read, insert and
  request copied the DAG, so it is retired with the copies rather than restated.
  The last three per-block rebuilds are gone — `fringe_states` keyed by the store's own fringe hash,
  the merge's rejection map built for the final scope only, and the DAG index `Arc`-shared between the
  store and the representation instead of cloned per insert (C56's §20 row has each falsifier) — and
  the DAG now publishes its own gauges (`messages`, `seen_entries`, `fringe_states`, `index_entries`,
  `logical_bytes`) into the node's `MetricsRegistry`, whose snapshot `/metrics` renders: the wire that
  was missing, so every scrape had returned the reporter's placeholder. **The instrument reproduces
  this row's own figure, on the node and at scale**: the bench's `dag` curve measures `seen_entries` =
  500,500 at N=1,000 (a chain's `seen` is its ancestry, so Σ|seen| = N(N+1)/2) and `logical_bytes` =
  16.2 MB, and the running devnet node reports **`seen_entries` = 17,319,555 (= 5,885 × 5,886 / 2
  exactly) with `logical_bytes` = 556 MB**, the ≈553 MB this row estimated from Σ|seen| × 32 B — now
  read off the structure rather than extrapolated. The floor itself is still not fixed, and that
  remains the decision this row records.

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
| `New.bind_count`/`Receive.bind_count` are a **`FreeCount`** carrier, and a negative count is **rejected at the proto boundary** | `RhoTypes.proto`'s `int32 bindCount` on both messages; the Scala's `New.scala`/`Receive.scala` carry it as a signed `Int` and neither tree validates it, so the oracle carries a negative count into the term | the count is not descriptive: it is a `well_scoped_par` depth, a sort-key leaf (`sorter.rs`'s `leaf_i64`), the extent of `env.shift` in substitution and the argument to `new_bindings_cost`, and a negative value is meaningless to all four. The port used to *clamp* it where it was used (`.max(0)`, `types.rs:436,440`), which hid the malformed message and left a term whose sort key carries a value no rule defines. Refused where it arrives — the same validate-on-ingress rule as the sibling `free_count` fields (`ReceiveBind`/`MatchCase`, C52) and `BindPattern.freeCount` (the row above) — and carried by `FreeCount`, so the sign cannot be *written* from Rust either. **Stricter than the oracle** (the Scala validates nothing there), and **not a hard fork**: the two counts the normalizer derives go through `checked_level_count`, so no deploy produces a negative one and only a hand-built or wire-injected message can. Falsified first: `a_negative_bind_count_is_refused_at_the_wire_boundary` fails against the old pass-through |
| The store-items **server** logs and **drops** a request whose store cannot be read, where the oracle's page build fails the request | `rspace/.../exporters/RSpaceExporterItems.scala:28,52,76` — the page is assembled from `exporter.getNodes(startPath, skip, take)` inside `F`, so a store error propagates to the caller and the request fails | the port's handler (`casper/src/engine/node_running.rs`'s `handle_store_items_request`) has no error reply to send: `StoreItemsMessage` is the only response type, so its choices are a *state claim* (an empty or short page, which the requester then validates its own traversal against) or a drop. It drops — a timeout is retryable and a wrong answer may be accepted — which is also the policy the same function already applies to an over-large `take`, so it is consistent rather than novel. **The consequence is named**: the requester retries or times out and never receives a page that under-reports the state. Everything *inside* the page assembly is fallible in the oracle's shape now (`get_history_and_data` returns the store's error), so the drop is confined to the one place that cannot reply |
| RSpace candidate selection is **sorted-first** by content hash (not newest-first insertion order) | `RSpace.scala`/`RSpaceOps.scala` shuffle candidates via `Random.shuffle` before matching | live Scala is non-deterministic across runs; the port selects the sorted-first candidate for consensus. Implemented per `docs/src/node/sorted-matching.md` (changes post-state hashes only for multi-candidate deploys) |
| Block validation replays dependency-free blocks **concurrently** (per-block forked `ReplayRhoRuntime`, batch processor), then inserts serially | Scala `BlockProcessor` validates one block at a time | replay is verify-only (Law 11), so concurrent re-validation does not change the committed state — a throughput optimization, not a semantic change. See `docs/src/formal/concurrency.md` |
| LFS sync inserts the downloaded blocks in **ascending** height order (parents before children) | `NodeSyncing.populateDag` `heightMap.flatMap(_._2).toList.reverse` | `BlockDagStorage.insert` requires every justification to already be in the message map, and a justification is always at a strictly lower height; the Scala `reverse` inserts the newest block first and fails with "justification not present in message map" for any fresh observer |
| LFS block requester downloads the **full ancestry chain to genesis** (no `lowerBound`/`extraHeights` cutoff) | `LfsBlockRequester.ST` `lowerBound`/`extraHeights` + `NodeSyncing` `blockHeightsBeforeFringe = deployLifespan` (`populateDag` `minHeight` filter) | a syncing node's DAG is always empty (`NodeLaunch.apply` only syncs when `dagSet.isEmpty`), so the Scala cutoff stops ~50 blocks short of genesis and leaves the lowest downloaded block's justification dangling — the same "justification not present" failure once the fringe is past `deployLifespan`. **Measured consequence (2026-09-24, AUDIT C64)**: on a chain the walk is one generation — one round trip — per block, so this deviation is 6,300 round trips where the oracle's bounded walk makes ~50, and a fresh validator was observed at ~1.5 blocks/minute (~70 hours for that chain). The *pacing* is faithful and measured (`the_walk_advances_on_responses_not_on_the_idle_timeout`: 3.85 ms for a 6-block walk against a 30 s idle timeout); the per-generation latency is the syncing node's, not the requester's **Disposition (2026-09-24, R3): an efficiency divergence with a measured cost, not a correctness one.** The oracle's `lowerBound`/`extraHeights` and `blockHeightsBeforeFringe` are a *cutoff* — they stop a walk that would otherwise continue past what the DAG already holds — so the port downloads a **superset** of the blocks it needs, all of them valid, and the resulting DAG is the same. What differs is the traffic, and C64 measured it: **6,300 round trips where ~50 would do**. So the row records a divergence the oracle *bounds* rather than forbids, with its cost named. **Disposition (2026-09-25, settled rather than deferred): a deliberate divergence, kept.** The port walks the full ancestry on purpose — `BlockDagStorage::insert` requires every justification present and a syncing node's DAG is empty, so a cutoff would have to invent a boundary the port does not have, and the Scala's own cutoff leaves the lowest downloaded block's justification dangling once the fringe is past `deployLifespan`. Taking it is therefore not an open efficiency unit but a change that would need the truncated-ancestry DAG semantics defined first — and **nothing in this register would catch a wrong one, because the oracle has no model of the block requester at all** (AUDIT C94: no `lowerBound`, `extraHeights`, `requestStream` or `LfsBlockRequester` occurs in any `.lean`). The behaviour is reasoned, its cost is measured, and the reason a future pass must not simply re-add the cutoff is written here rather than discovered later |
| The store-items **server refuses** a page over 32 MiB (drop, no reply) where the oracle serves whatever it is asked | the Scala's `handleStoreItemsRequest` has no byte cap; its only bound is the requester's own `PAGE_SIZE` | the requester's `PAGE_SIZE` self-consistency is not a defence *for the responder*: `validate_state_items` requires the received keys to match the page the requester recomputes, so a responder that truncates is caught while one that is asked for 10,000 fat nodes pays tens of megabytes per request, per peer, in memory it must assemble before it can measure it. Registered with its cost: the assembly is still paid, the bandwidth and the per-request ceiling are not (AUDIT C73) |
| Active validator set = the **top-N by descending stake** (key-ascending tie-break) | `Pos.rhox:718-726` `pickActiveValidators` returns the first `$$numberOfActiveValidators$$` entries of `allBonds.toList()` — *key* order, not stake order; its own comment is `// TODO: Randomly select 100 active validators once we have on-chain randomness` | the contract's rule is not a rule to port: "the first N in map order" is a placeholder for a random selection, and it makes the consensus set depend on map iteration order rather than on anything consensus-relevant. The cap exists because finality's supermajority is stake-weighted, so the highest-staked N is the property the contract's TODO is aiming at; the port's choice is deterministic and stake-ordered (`select_active`, `rholang/src/native_state.rs`). The *timing* is unchanged — both apply it only at an epoch boundary (law 44) |
| A slashed validator that had a **staged withdrawal** is removed from the pending map | `Pos.rhox:491` leaves it in `pendingWithdrawers` while zeroing its bond (`:487`), so at the next boundary `movePendingWithdrawer` files it under `withdrawers` with a zero amount, where it stays forever (never paid, never removed — nothing deletes a zero claim) | the payable outcome is identical (zero), and the port does not carry a permanent tombstone: `slash` removes the validator from the pool, the active set, the claim map *and* the pending map. Recorded because it is a state-shape difference a reader would otherwise meet as a missing entry |
| Epoch length `0` (the port's permissive default) means **every block is a boundary**; the contract's `%`/`/` by it would fault | `Pos.rhox:517` `blockNumber % $$epochLength$$`, `:381` `blockNumber / $$epochLength$$`, `:249` `bonds / $$minimumBond$$` | the contract cannot express a zero epoch length or a zero `minimumBond` — it divides by both. The port's default parameters have both at zero (ad-hoc runtimes and tests install no genesis PoS state), so the port defines what the contract leaves as an arithmetic fault: a zero epoch length is the one-block epoch `epochLength = 1` means, and a zero (or zero-normalising) minimum bond pays a reward of zero rather than faulting the block. The Lean model states its conservation theorem for the defined case only (`Rchain.sum_rewards_le_pot`'s hypothesis `0 < activeBonds / minimumBond`), which is the same boundary |
| `deploy-status` has **no `Running`** state: a deploy the node is executing right now answers `Pooled`, or `Unknown` once the pool no longer holds it | `BlockApiImpl.scala:218-224` — `findCurrentlyExecutedDeploy` reads the node's `BlockExecutionTracker` (a per-node cache of the deploys its own block creator is executing) and answers `notProcessed("Running")`, and `findPooledOrRunningDeploy` consults it after the pool | the port has no execution tracker: deploys are pulled out of the pool into `compute_deploys_checkpoint` with no in-flight record, so there is nothing to look up. A derived answer would be a guess — "neither in the DAG nor in the pool" also describes a deploy whose block was just proposed and one dropped with its block — and the window is the length of one proposal. Registered rather than invented: the states a client can act on (pooled, processed with success, processed with error, unknown) are complete, and `Running` only ever meant "the node you asked is busy with it right now" |
| `metrics { prometheus, influxdb, influxdb-udp, zipkin, sigar }` (and the matching `--prometheus`/`--influxdb`/`--zipkin`/`--sigar` flags) are **parsed and never consulted** | `kamon.conf` — where `prometheus { enabled = false }` gates Kamon's scrape endpoint, and the influxdb/zipkin/sigar blocks configure reporters that push | nothing outside `node/src/configuration/` reads `MetricsConf`: the node always serves `GET /metrics` in Prometheus text format, there is no InfluxDB or UDP sender, and the tracing/span backends are out of scope (`diagnostics/mod.rs`, said there). So the switches are inert in both directions — `prometheus = false` does not disable the endpoint and `= true` does not start a reporter. Said once in `defaults.conf` where an operator reads the switch, and recorded here as the second config surface found doing nothing (`disable-state-exporter` was the first, wired in `687fc4b30`) **Disposition (2026-09-24/25, R3 then `bf44e5fc3`): the four reporters it cannot honour are refused at startup, and `prometheus` is accepted with a note.** The node now exits 1 naming each unimplemented reporter — `Configuration error: unimplemented metrics reporter(s) enabled: influxdb. This port has no InfluxDB sender, no InfluxDB UDP sender, no Zipkin span reporter and no Sigar collector, so these settings would report nothing — refused rather than accepted and ignored. GET /metrics serves Prometheus text and is always on; unset the setting to start.` — because an ignored knob is exactly the failure this pass keeps recording. `prometheus` is accepted **ungated**, with a note that the endpoint is always on rather than gated, since the endpoint *is* implemented (`web::http`'s scrape tests pin it). Two facts decided the shape: the config parser **ignores unknown keys** while clap **rejects unknown CLI flags**, so *deleting* the fields was safe for config files and breaking for command lines — which makes keep-and-refuse the only honest route; and the refusal can fire only for an operator who explicitly asked, since `defaults.conf` sets all five false and the oracle's defaults are false too |
| Vaults are a **balance map keyed by REV address**; `transfer` takes the caller's `deployerId` rather than a minted purse | `RevVault.rho:103-140` — `findOrCreate` → `_makeVault` returns a `MakeMint` **purse**, and `transfer`/`deposit`/`getBalance` are called *on the purse* with an `unforgeableAuthKey` | the *spend* rule holds either way: the port derives the `from` account from the caller's unforgeable `deployerId` (`system_processes.rs`'s `transfer`, "capability, not data"), so no deploy can spend another key's vault — what is unforgeable is the deploy's identity rather than a minted name. What the simplification **loses** is *delegation*: a purse could be handed to a contract that then spends from it, and here only the signing key can spend. **Decided 2026-09-23** (Programme B item B2) to keep it: nothing in the tree delegates (the wallet, the faucet, the gateway legs and the genesis ceremony all act as the key itself), and landing it needs the deploy's RNG threaded into a native call — the minted name is a `new`, so it must be replayable — plus a leaf and a client-visible API change. The *reply-shape* half is recorded where a client meets it: `spec/API-SCHEMA.md`'s `rho:rchain:revVault` row is ❌ open for exactly this |
| Storage is **refunded** when a produce/consume matches (the gas a matched op costs) | `ChargingRSpace.scala:105-127` — `refundForConsume` and `refundForRemovingProduces`, charged as negative `Cost`s *before* the event and COMM costs | the port charged the storage and never refunded it, recording the gap as a "safe over-charge"; the refunds are restored (law 49, `spec/Rchain/Charging.lean`). **Hard fork:** the recorded `PCost` of a deploy that matches changes, and cost is part of the block's state — on a chain that accepted such a block the old node recorded a *higher* cost than the Scala's, so the old value was already wrong, which is why this is a fix and not a behaviour that was ever correct. The order matters as much as the amounts: a refund credited after the exhaustion check cannot save a deploy that has already run out (`peak_refunds_first`), which is why the Scala charges them immediately |
| The Coop **multisig public keys** are read as the port's initial **trusted stakeholder** set | `Pos.rhox:122-128` creates the Coop multisig vault from those keys (and `$$posMultiSigQuorum$$`), and `:470-482` sends slashed stake to it; the Scala has no trusted-stakeholder concept at all — that is this port's extension for observer admission (`spec/RUST-FIRST.md`) | a mapping, not an accident (`06bf01a7f`, and the doc comment on `build_pos_genesis` says it): the Coop multisig is the network's governance body in the contract, and admission is the port's governance-shaped hook, so the closest analogue of "who may govern validators" is the keys the contract gives governance to. The alternative reading — that they are only slashing-vault owners, leaving the genesis `trusted` set as the validators alone — is equally supportable, and nothing in either tree says which was intended. Recorded because the consequence is silent: an operator setting these keys for vault control is also granting admission rights. The `--pos-multi-sig-quorum` option has no counterpart at all (no multisig vault), so it stays "Reserved" in its help text **Disposition (2026-09-24, R3): the oracle is silent, and the row should say that rather than that a choice was made.** `Pos.rhox` creates the Coop multisig from those keys and sends slashed stake to it; the Scala has no trusted-stakeholder concept at all, and **nothing in either tree says which was intended** — so the two readings are equally supportable and the row is the record of that, not of a preference |

| A pattern that binds one free level **twice is refused by the matcher** (`spatial_match`'s entry, `linear`), where the Scala silently merges the repeated binding | `SpatialMatcher.scala:144,209-211` — `handleRemainder` does a plain `insert`, so `@[v, v]` against `[1, 1]` matches with one binding winning right-biased | law 5 says a pattern binds each free level **at most once**, and the model states it as the matcher's entry condition (`spatialMatch` = `spatialMatchCore … && linear pattern`, `Match.lean:369`); the port now does too (AUDIT C42). The Scala's merge is why the finding was latent rather than visible: the repeated binding is *consistent* here, so the overwrite was silent. Not reachable from source either way — the normalizer refuses a twice-bound binder first (`normalizer.rs:111,289,590,1325`) — so this closes a silent-overwrite path rather than changing what any deploy does, and it is **not** a hard fork. Falsified: with the guard bypassed, `@[v0, v0]` against `[1, 1]` matches and `law5_a_pattern_that_binds_a_variable_twice_never_matches` fails |
| **The finalizer's progress guard**: a non-advancing fringe is dropped from the advance chain (`if nf == current { break }`, `block-storage/src/dag/finalizer.rs:225-226`) | `Finalizer.scala:175` — `LazyList.unfold(parentFringe)(nextFringe(_).map(nf => (nf, nf))).lastOption`, which compares fringes **never** (the gate is the support map) | the oracle would not terminate on a repeating fringe, because `lastOption` forces the whole lazy list; the port must not diverge. **Weaker than a cycle guard as a *guard*, but the walk is bounded by the chain, which is why the weaker shape is enough** (settled 2026-09-24, C69's second half): each iteration passes the previous fringe as the cutoff and `self_parents` stops at it, so for every sender the minimum message can only move along that sender's own unfinalized chain — finite and acyclic — in one direction, and a step that is not the fixed point moves strictly. The walk therefore terminates in at most (messages on the longest unfinalized chain) steps, and a cycle of *any* length would have to move strictly forever on a finite chain. Measured rather than argued: `the_fringe_walk_is_bounded_by_the_non_finalized_chain_length` drives the loop over a generated `L`-layer fork — **6 steps on 8 layers, against a 25-message bound** — with the assertion inside the loop, so a hypothetical cycle fails the test rather than hanging it. The guard stays as the belt for the fixed point (`nf == current`), not as the thing preventing divergence. Found 2026-09-24 with C69, which is the same reading
| **Equivocating blocks are refused** — at `insert` (H-1) and at restore (H1c) — where the oracle has no such gate | `legacy/block-storage/.../dag/BlockMetadataStore.scala:118-124` — `validateDagState` asserts only that the height map's numbers are contiguous, never `(sender, seq_num)`, so a forked store restores silently; and the Scala tree contains no equivocation check at all | an equivocating validator can neither enter the DAG nor stall finalization — the H-1 stall is a liveness failure the Scala admits, so the premise law 15's proof needs is **guaranteed here and only observed there** (AUDIT C84, and H1a's note) |
| An **empty window** in `visualizeDag` renders an empty graph where the oracle raises | `legacy/casper/.../api/GraphGenerator.scala:39` — `timeseries.head` on a `List` built from a `Set` throws on empty | the endpoint is a *view*, and a graph of nothing is the honest rendering of an empty DAG; refusing would turn a visualization request into an error (AUDIT C85) |
| **A resolved parent at or above the block's number is refused** — the **failed** ones included (H1b, `casper/src/validate.rs:152-154`, test `h1b_a_failed_parent_above_the_childs_height_is_refused` at `casper/src/dag.rs:1268`) | `legacy/casper/src/main/scala/coop/rchain/casper/Validate.scala:178-198` — `blockNumber` maps the justifications through `lookupUnsafe` and then `.filter(!_.validationFailed)`, so a failed parent is discarded before the maximum and **no resolved parent's height is compared against the block's number at all**; it admits the block | `Descends` (`spec/Rchain/Casper/Dag.lean:296-297`) is the premise law 15's proof consumes, and the laws are the port's oracle where they outrank the reference — a premise the port *guarantees* is worth a refusal a byzantine peer can trigger and **no honest proposer can**: justifications are `latest_msgs.values()` (`multi_parent_casper.rs:293-300`) and a failed block never enters `latest_msgs` (`block-storage/src/dag/message_state.rs:118-128`, the H-2 exclusion), so the operator consequence is a validator-side divergence on byzantine input only (AUDIT C82, C83) |

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
- **R15 (C1) — unbounded block-validation pipeline.** `node/src/runtime/node_runtime.rs:531` feeds replay validation through an `unbounded_channel`; the bounded ingress (S18) is upstream of it, so a peer streaming valid-signed blocks fills memory faster than replay drains. **Fixed** — the processor-input channel is now `mpsc::channel(MAX_PENDING_BLOCKS)` with backpressure (`send().await`), and `block_processor::apply` takes the bounded `Receiver`.
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
- **R28 (C4) — deploy pool never expires future-dated deploys.** `casper/src/dag.rs:409-413` — the pool ingress never bounded `valid_after_block_number`, so deploys anchored at `i64::MAX` filled `MAX_POOLED_DEPLOYS` permanently. **Fixed** — `BlockApiImpl::deploy` rejects deploys with `valid_after_block_number` more than `DEPLOY_LIFESPAN` ahead of the tip.
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
  (`spec/GENESIS.md`, `docs/src/node/devnet.md:203`, and the comment this replaces in
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

  **Residual (§6, a deviation — and this paragraph's direction was wrong; corrected in place):** a
  remainder *with* a preceding comma (`[1, ..._]`, `{a: 1, ...r}`, `Set(1, ...r)`) has no derivation.
  The grammar's list rule is `[X] ::= X | X "," [X]` (`separator Proc ","`, `rholang_mercury.cf:78`),
  so a separator may only stand *between* two elements, and `ProcRemainder` follows the list with no
  terminal of its own (`CollectList ::= "[" [Proc] ProcRemainder "]"`, `:179`). The comma-**less**
  `[1 ..._]` is the **derivable** form — a one-element list takes the singleton rule, which carries no
  separator at all — so the port's acceptance of it needs no excuse, and the earlier reading of this
  paragraph (which called the comma-less form the underivable one) had it backwards. The *comma* form
  is law 31's **deviation**: it is how the vendored contracts are written (C31 below records the 31
  sites), so the port accepts it deliberately rather than refusing it — the guard is
  `parser.rs:1108-1110` (break before `parse_proc` meets the ellipsis), with the same shape in the map
  (`:1179-1181`) and set (`:1201-1203`) loops, and both spellings are pinned by `parser.rs:1530`'s
  `a_comma_before_a_remainder_is_the_deviation_the_contracts_use` (`[a, ...rest]`, `{name: *voter,
  ...tail}`, `Set(a, ...rest)` and `[a ...rest]` all stay accepted).

  **Landed as the model's half (G3: `Rchain/Parse.lean` + `Rchain/Print.lean` + the `parse` corpus).**
  The grammar's list sites are data (`grammarFragment`), `derives` decides a token list, and
  `parseDeviations` is law 31's list with each row's *direction* checked against `derives` — so the
  comma-less form is derivable and the comma form is a registered deviation, exactly as this paragraph
  now says. The Rust consumer (`rholang/tests/lean_parse_corpus.rs`) then found three deviations the
  tree did not know: a `NameRemainder` in a **contract's** parameter list
  (`contract f(x ...@rest) = { Nil }`) is derived by `PContr` and refused by the port's parameter loop
  (`parser.rs:426-431`), and `ReceiveSendSource`'s `Name "?!"` is derived and refused by the receipt
  parser (`:1271-1315`) — while its sibling `SendReceiveSource` (`Name "!?" "(" [Proc] ")"`) **is**
  implemented (`Tok::BangQ`, read by `parse_name_source`). So the `refuses` direction of law 31, which
  had no row, has two.

  **One claim this residual's neighbourhood makes is now recorded where it belongs:** a
  *process-position connective* (`x /\ y`) is refused by the **normalizer** (`normalizer.rs:1762`,
  `TopLevelLogicalConnectivesNotAllowedError`), not by the parser — `parse("x /\\ y")` is `Ok`. That
  refusal is a fact about law 34/35's layer rather than laws 30/31's, so it is **not** a row of the
  parse layer's deviation list; the parse layer carries it as a note instead.

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
     verdicts depend on exactly that. The repair went two steps, and the second is the one worth
     recording: the statement first carried the domain as data (`allStringChans`), and then that
     hypothesis turned out to be **unnecessary** — C45's widening below put the string-channel
     condition into the rule itself (`hsend`/`hsrc`, the node's own condition), so the domain became a
     consequence of the rule rather than a side condition on the statement, the predicate was deleted
     with it, and the tie holds for every `p`. A domain hypothesis has to be re-checked when the thing
     it excluded is fixed, or the statement under-claims forever; this one had been true when it was
     written and had been false for a day by the time it was removed. `takesStep_sound` was always
     unrestricted (a `true` from the search already implies both channels were string channels), and
     `takesStep_complete` — the direction this entry left owed — is proved, so the tie is a theorem.

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

  **Fixed (2026-09-24, Programme F): the matcher enforces it too.** The finding concluded that the
  matcher's silent paths are defence-in-depth on a state the front end cannot produce — true of source
  terms, and the reason it was filed as an observation rather than a defect. Programme F's rule (the
  maths wins) makes that the wrong resting place anyway: the model states the rule **at the matcher's
  entry** — `spatialMatch` *is* `spatialMatchCore … && linear pattern` (`Match.lean:369`) — and the
  port did not. `spatial_match` is now the same split with the same names
  (`spatial_matcher.rs:194-230`): the clauses moved to `spatial_match_core`, which recurses into
  itself, and the pattern is checked once, at the entry, against `linear`
  (`models/src/types.rs:93`, the port's counterpart of `freeLevelsOfPar`). A non-linear pattern is a
  **no match**, not an error — the model's own answer — and the guard sits at both of the matcher's
  entries, `spatial_match` (with `spatial_match_result`) and the store's `RhoMatch::get`
  (`storage.rs:75` feeds `&spatial_match` to `fold_match`). Checking the whole pattern once is
  sufficient for the clauses and that is argued rather than assumed: every sub-pattern they descend
  into is reached by the same walk `linear` is built on, so a linear pattern has only linear
  sub-patterns (the note is at the guard). `aggregate_updates`' `BugFoundError` stays as the inner
  check for a clash the entry guard cannot see, and `handle_remainder`'s merge stays untouched — a
  pattern's top-level free variable is legitimately bound once per field, so refusing per insert would
  break the accumulator. The Scala *merges* that binding, so the new refusal is a registered
  deviation, §6.

  **The test that pins it could not fail before this pass, which is the second finding here.** The
  law-5 property's fixture (`rholang/src/property_tests.rs`) set `connective_used` on the outer par and
  on the elements but not on the **collection**: `from_expr` takes the flag from the expression
  (`par_ops.rs:87`), and `EList`'s defaults to `false`, so the pattern was concrete, took the matcher's
  equality fast path, and "no match" was true of every pattern — including a doubly-bound one that
  never reached a binding path. Both halves are fixed and both are falsified: with the flag set the
  doubly-bound pattern **matched** before the guard landed (binding level 0 to `GInt(1)` for `@[v0, v0]`
  against `[1, 1]`, the silent overwrite this finding is about), and with the guard bypassed the
  property fails in 0.08 s. `law5_a_pattern_that_binds_distinct_variables_matches` is the control —
  without it, "the matches are empty" is equally satisfied by a matcher that matches nothing. This is
  the same class as the two cost tripwires the 2026-09-24 performance pass had to rebuild: an
  instrument that passes on the defect it names is not evidence, and only the falsifier distinguishes
  them.

  **One cell was owed here, and it was named so it could not go quiet — and it is now discharged.** Law
  5's register row and `spec/INVENTORY.md`'s row 5 both said the matcher enforced linearity *only* on the
  aggregation path, "while its element-pair and conjunction paths overwrite silently" — the sentence the
  entry guard's landing made false. Both cells were re-written *with* that change (`144a900c5`, which
  states the normalizer's refusal, the matcher's entry guard and `aggregate_updates`' surviving role in
  their places), and the row followed it to `proved-tied` when its remaining debt turned out to be two
  theorems that already existed (`cda0b1554`). The correction was *planned* here rather than left
  implicit, which is why this note now records that it happened: a planned correction is only as good as
  the line saying so.

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

  **And the complete direction is proved too** (2026-09-24), so `takesStep_iff_reduces` is a theorem in
  both directions and C40's domain hypothesis is gone with the predicate that carried it — the rule now
  carries that domain itself. What stays is the **model's boundary rather than a debt**: the model has
  **no join rule**, so a join whose every channel is filled is a step in the node and has none here. The
  tie is between the rule and the search, both of which are silent on joins, so it is true of the model
  as the model is — and the gap to the node is the join rule, which is a modelling unit rather than a
  proof obligation, and which the row now states in those words.

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
  node whose indexing fails is precisely the node that does not have one. `tools/devnet.sh:270-283`
  starts validators 1..n−1 with `--bootstrap` and their validator key and nothing else — no
  `--wallets-file`. The vault list has exactly one production source, `vault_parser::parse`
  (`vault_parser.rs:16`) called from `node_launch.rs:66` inside `create_genesis_block` — so it exists
  only in the ceremony, and `parse_if_exists` (`:43`, the tolerant variant) is called by nothing but
  its own test. They also get no `--bonds-file`, and are not `standalone`, so
  `node_runtime.rs:1384-1388` gives them `PosGenesis::default()` as well. A joining validator therefore
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

  **Verification status — passed end-to-end (2026-09-24).** What is verified: `cargo check --workspace
  --all-targets` clean, `casper/tests/determinism.rs` 7/7 (including the tripwire),
  `is_genesis_pre_state_is_true_only_for_the_empty_state`, and the register gate. The end-to-end half
  landed after C55 was found and fixed: on a freshly built image of `11b2200dc`,
  `tools/devnet.sh up --validators 3` reaches a running chain and the joining validators **track it** —
  bootstrap `latestBlockNumber` 37, validator-1 37, validator-2 36, with **zero** occurrences of
  `regenerated mergeable channels` and zero refusals in either validator's log. That absence is
  evidence *now* in a way it was not before, because blocks were produced: both validators indexed
  block #0, which is the replay this fix repairs, and then followed the chain past the height they
  joined at. What this row records is that the two call sites supply the vaults, that the condition is
  the genesis exactly, and that a node without the files fails loudly rather than computing a wrong
  state.

  **The stall that blocked this check was C55 — and the red reported alongside it was a different
  defect.** *(Correcting this row's earlier reading.)* The bootstrap stall is C55 below: a Θ(N³)
  rebuild of the stored DAG, not the ceremony, not the comm layer, and not a regression of any kind.
  The `devnet-fuzz` nightly's ten days of red is **not** that stall, although both surface as
  `timed out waiting for devnet-bootstrap to serve /api/v1/status` — a symptom class, not a cause. In
  all 11 runs (2026-09-14 → 2026-09-24, CI runs `34825931798` … `35976009772`) the bootstrap **exits**
  rather than spinning:

  ```
  INFO  [coop.rchain.node.runtime.NodeRuntime] No need to open any port
  ERROR [coop.rchain.node.runtime.Setup] NodeLaunch exited with error: FAILED PARSING WALLETS FILE: /genesis/wallets.txt
  Permission denied (os error 13)
  ...error: invalid value 'rnode:// 99c6cf…@devnet-bootstrap?protocol=40400&discovery=40404'
          for '--bootstrap <BOOTSTRAP>': Can not parse the bootstrap address
  ```

  Two harness defects, both fixed in this pass in `tools/devnet.sh`: the genesis directory is created
  without traversal permission for the container's `rnode` uid (`chmod -R a+rX` in `genesis_files`),
  and `bootstrap_id` took the last `=`-field of an unnormalised `openssl x509 -subject`, which yields
  ` <hex>` with a leading space on the OpenSSL releases that print `CN = <hex>` (now `-nameopt
  RFC2253` plus a whitespace strip). A CI job also starts with no volume, so it could never have
  carried C55's signature at all. (2) The 1-validator devnet is green, including a live check of law 47 (a staged
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

  **Landed (2026-09-24, U12)**: the error channel this paragraph said was owed. The Scala's `F[Node]`
  carries a store failure out of `loadNode` and into `RadixHistory.new`/`reset`, whose callers can then
  refuse; the port's `load_node -> Node` and its `History` trait had none, so the only available
  flattening was a panic or an empty node. Both are fallible now — `load_node` returns
  `Result<Node, String>` and `RSpaceImporter::get_history_item` returns
  `Result<Option<Vec<u8>>, String>` — so a store failure and a genuinely absent node are different
  answers at the boundary that used to have one. The paragraph above is the interim fix that made them
  distinguishable at the assert; this is the channel. Falsified first: restoring the
  `.ok()` fails `history::radix_tree::tests::a_store_error_is_not_a_missing_node`, which asserts both
  halves together (an error surfaces *and* a genuinely absent node is still `None`), so a fix that made
  missing nodes an error would be caught as the opposite bug.

- **C54 — a wrong fix, caught by the corpus in one run: what the "set pattern over-claims" reading got
  wrong, and what it tells us about law 37's tie** (found and reverted 2026-09-24, Programme F; **no
  code change survives**, and the value is the record of how the error was made). The pass attempted law
  37's owed tie and, before proving anything, *probed the model's domain* — which is the right instinct
  and is what produced C50/C51. It measured

  ```
  spatialMatch @{1, 1} @{1} = true        -- the model accepts a longer set target
  spatialMatch @[1, 1] @[1] = false       -- while its own *positional* member refuses
  ```

  and read that as a model over-claim: the port's `list_match_single` refuses an unequal length when the
  pattern has no remainder (`exact_match = !wildcard && remainder.is_none()`, then
  `if exact_match && plen != tlen { return Ok(Vec::new()) }`, `spatial_matcher.rs:684-693`), so the two
  **searching** members were made to enforce it — a two-line change to the `eset`/`emap` arms, plus the
  `exprSat` proof threaded through the guard, plus a corpus row 21 (`@Set(1)` against `Set(1, 1)`,
  `expected := false`) and the case-count bumps.

  **The corpus consumer rejected it on the first run**: `@Set(1) vs Set(1, 1): the node says true`. The
  fix had made the model *stricter than the node*, on a shape that node **cannot hold**:

  - `eval_expr`'s `ESet` arm evaluates a set's elements and rebuilds it through `par_set`
    (`rholang/src/reduce.rs:783`), and `par_set` **deduplicates by raw equality then sorts**
    (`models/src/sorter.rs:834`, the port of Scala `ParSet.apply`) — so every set *value* on the node is
    canonical, and the datum `Set(1, 1)` is the datum `Set(1)`. The node's `true` is set *idempotence*,
    not a search that walked one element too far.
  - The model's `Par` has no such constructor: `Set(1, 1)` and `Set(1)` are different values, so
    `spatialMatch` answering `true` there is a statement about a target that no deploy can produce —
    the same shape as the arithmetic pattern (C51's neighbour: "the model's `false` is right about every
    reachable term and the law's quantifier was what was wrong").

  **So there was no defect, and the difference it pointed at is a hypothesis the tie needs.** The tie
  `spatialMatches t p ↔ t = p` cannot hold on a target with duplicated set/map elements, because the
  model matches it against a shorter pattern while `t ≠ p` — so the owed statement needs, beside C51's
  **singleton expression list**, that the collections' *contents* are canonical (no repeated element, and
  the order the node's constructor imposes). That is Law 10's `WellFormed` move again: narrow the
  statement to the invariant the code maintains rather than widen the model, because the model's
  algebra has no deduplicating set constructor to widen it *with*.

  **The lesson worth more than the finding.** The error was comparing the model against a *reading of the
  port* — and against the model's own list member — instead of against the **node**. The corpus consumer
  is the only thing in the tree that runs the node on the same shape, it disagreed immediately, and it
  named the direction ("the node says true"). A model change in this area is not believed until
  `lean_match_corpus` has been run against it; the `decide`d corpus cases cannot see this class at all,
  because they are the model agreeing with itself.

- **C55 — the devnet bootstrap never starts: restoring a stored chain folded the message state once per
  block, at Θ(N³)** (found 2026-09-24, by stack sample after a day of log reading; **fixed**, and it is
  what C46's end-to-end check was waiting behind). `BlockDagKeyValueStorage::create` rebuilds the
  in-memory DAG by folding every stored block through `DagMessageState::insert_msg`, which is written
  as a *persistent* operation — it clones the whole message map, and with it every message's `seen`
  set — and returns the new state. The oracle does the same thing in shape: `insertMsg` is
  `msgMap + ((msg.id, msg))` over Scala's **immutable** `Map` (`DagMessageState.scala:66`), a HAMT
  with structural sharing, so the same expression is O(log N) and copies nothing. `BTreeMap::clone` is
  a deep copy of every `Message` (id, height, sender, sender_seq, bonds_map, parents, fringe, and a
  `seen` set that holds the block's whole ancestry), so a rebuild of N blocks costs Σᵢ O(i · i) =
  Θ(N³) element copies and leaves Θ(N²) `seen` entries live. **The port kept the code and lost the
  asymptotics** — the same failure mode as C51's sibling below, and the reason both were invisible to
  tests: the state they build is identical either way.

  **What was observable.** `tools/devnet.sh up --validators 3` never reached `latestBlockNumber > 0`,
  so `up` exited 1 after its 120 s healthcheck; the container sat at ~100–104% of *one* core (measured
  on the host as 0.94 core) with 698 MiB RSS and exactly **three** log lines — the UPnP messages, which
  `create_comm_state` buffers and flushes only *after* `who_am_i::fetch_local_peer_node` returns
  (`node_runtime.rs:179-190`), so the stall is strictly downstream of UPnP and WhoAmI. The next
  observable event should have been `"Starting as genesis master, creating genesis block..."`
  (`node_launch.rs:216`), which never appeared; nor did `"Sending genesis block..."`. A `gdb` stack on
  the pegged thread put it exactly: `rchain_casper::dag::BlockDagKeyValueStorage::create` at
  `dag.rs:83`, called from `setup_shard` (`node_runtime.rs:1331`) in the **main** `block_on` path —
  which is why nothing else progressed, and why the HTTP API (bound later, in `serve()`) was never
  bound. The message map held **5,844** entries at that point.

  **Corroboration from the data directory, which is how the region was bounded before the stack.** The
  failing run's container volume still carried the store-open timeline: `eval/*` and `blockstorage`
  opened at 13:07:03, `deploypoolstorage` at 13:07:07, `dagstorage` at 13:09:12 — and
  `rspace/{cold,history}` **never**, their lock files still holding the previous run's timestamps. The
  rebuild sits between `dagstorage` and `create_history_repository(…, "rspace")`, which is precisely
  where the stack landed.

  **Fixed** by giving `DagMessageState` in-place insertion (`insert_msg_mut`,
  `insert_msg_without_latest_mut`) and leaving the persistent forms as one clone plus the same in-place
  insert — so the acceptance rule (Law 15's monotonicity, and the subset invariant the `debug_assert!`
  checks) still lives in exactly one place. `create` folds in place; the loop is Θ(N²) in the `seen`
  sets it must build, which is inherent. Falsified first: `casper/src/dag.rs`'s
  `restoring_a_stored_chain_is_not_cubic_in_the_message_state` drives `create` over a synthetic 1,200-block
  chain and bounds the fold; measured at N=1500 in a debug test build it is **5.1 s in place against
  108.6 s copying**, so the bound sits ~4.6× above the fixed fold and ~3.7× below the copying one.
  End to end on the same 5,881-block chain: **23 s to serve** (previously: never).

  **The trap that made it read as a code regression, recorded because it cost a day.** `tools/devnet.sh`
  mounts a **named** volume (`-v ${name}-data:/var/lib/rnode`), which `up` reuses and only `down -v`
  removes — so the *first* run against a fresh volume creates its genesis with an empty DAG, skips the
  loop body entirely, and works; every later run rebuilds the accumulated chain and the cost explodes
  as autopropose extends it. Nothing in the code changed, and the handover's exculpation of the C46
  change — "the pre-change image fails identically" — was true but empty: the control run reused the
  very state it was controlling for. A CI job, which starts with no volume, could never have carried
  this signature at all.

- **C56 — the per-block merge scope copied every message it looked at** (found 2026-09-24, while
  measuring C55's fix; **fixed**; the `seen`-set floor is owed and named). `MergeScope::from_fringes`
  computes `upper.seen \ lower.seen` — law 15's `seenOf` — and did it by materialising *messages*:
  `message_map::between` collected `msg_map.get(id).cloned()` into `BTreeSet<Message>`, and a `Message`
  carries its `seen` set, of size Θ(N) for a chain of N blocks. Two calls per block therefore cost
  Θ(N²) in copies, on top of `Message`'s derived `Ord`, which compares `seen` as well. The oracle's
  `upperSeen -- lowerSeen` is a difference over `Set[Message]`, and Scala's immutable `Set` holds
  *references* to the same case-class instances — it never copies a message to answer this. Measured on
  the same 5,855-block chain, isolated (one node, no peers): **0.78 GiB of churn per block** at one
  core and no plateau — 10.4 → 18.3 GiB over 180 s while producing 10 blocks — against **70 MiB flat**
  on a fresh chain of ~85 blocks, which is the ~4,700× that Θ(N²) against Θ(1) predicts.

  **Fixed** by making `between` take ids and return ids (`&BTreeSet<M> → BTreeSet<M>`), reading each
  bound's `seen` through the map; the call site keeps its three "not in dag" errors by checking
  membership instead of cloning. Nothing downstream wanted the messages: both results were used only as
  `is_empty()` and `.map(|m| m.id)`. Same chain, same isolation, after: **~4 MB per block, plateauing**
  (9.79 → 9.87 GiB over 140 s while producing 18 blocks) and CPU falling from a pinned 100% to 40%.
  Pinned by `between_is_the_id_set_difference_restricted_to_the_map` (which also closes the function's
  coverage gap).

  **The peer-sync half was attributed and fixed (2026-09-24, the performance pass).** What grew
  ~0.86 GiB per block while peers synced was not the sync path's own bookkeeping — it was the DAG
  being *copied* all over the per-block and per-request paths:

  - `get_representation` returned `DagRepresentation` **by value**, so every one of ~35 production
    call sites — every per-block path (`proposer.rs`, `validate.rs`, `multi_parent_casper.rs`), every
    peer-request handler (`node_running.rs`'s ForkChoiceTip / HasBlockRequest / FinalizedFringeRequest,
    `block_receiver.rs`) — paid a deep copy of `msg_map` with the read lock held throughout. It now
    returns `Arc<DagRepresentation>` (a refcount), and the writer takes its copy at most once per
    insert through `Arc::make_mut`, serialized by the storage's own lock.
  - `insert` cloned `dag_message_state` once per block purely to avoid holding the read guard across
    the awaits that follow it; everything it needed is synchronous, so the guard is now scoped to
    that region and the copy is gone.
  - `Message.seen` — the set that made a `Message` copy cost Θ(N) — is now `Arc<BTreeSet>`, so a
    `Message` clone is a refcount bump. **This does not move the value**, which is why it is not a
    register event: law 15 pins `seen` as the construction `seenOf js id = (js.map (·.seen)).join ++
    [id]` (`spec/Rchain/Casper/Fringe.lean:90`, both inclusion halves proved), and the representation
    is not what the law constrains.

  Measured in-process, at N=1,200, debug test build, with the copying behaviour restored for the
  comparison: **200 representation reads take 459 ms copying against 0.17 ms shared; 50 inserts take
  339 ms against 230 ms.** Both are now tripwires
  (`reading_the_dag_representation_does_not_copy_the_message_state`,
  `adding_a_block_does_not_copy_the_message_state`, `casper/src/dag.rs`).

  **Both had to be rebuilt, because the falsification that the house rule requires found neither
  could fail as written (2026-09-24).** They were calibrated against the *pre-Stage-3* tree, where
  `Message.seen` was still a plain `BTreeSet`: those are the figures a first pass quoted — 27.3 s and
  7.44 s — and they describe the *whole* pre-pass shape, not either tripwire's own falsifier. With
  `seen` shared, restoring the copy alone costs one to two orders of magnitude less, so both bounds
  cleared it. The read tripwire now also asserts the *mechanism* — consecutive reads must return the
  same allocation (`Arc::ptr_eq`), which is deterministic and fails on `Arc::new((**guard).clone())`,
  the one-line way the copy returns — and keeps a 5 s bound for the slower joint regression. The
  insert tripwire's ratio is 1.47×, which no timing bound can carry; it now pins the mechanism's
  persistent half instead (an insert into an unread DAG must leave the representation at the same
  allocation, so an unconditional copy fails it) and says in its own doc comment what it cannot see.
  The generalisable rule: **a tripwire calibrated before a sibling change landed is calibrated
  against a tree that no longer exists** — re-run the falsifier against the tree it lives in, or the
  bound is prose.

  **The tail of this finding landed (2026-09-24, Stage 5): the structures `insert` rebuilt per block
  stop being rebuilt, and the page being served stops being copied.** Each is representation-only —
  `logical_bytes` (Stage 6) and the representation digest are the negative controls, and they do not
  move: with the 5a keying mutation in place (the one that fails the keying pin) the digest is
  unchanged *and* the account is unchanged to the byte — `logical_bytes = 2032` and `seen_entries = 21`
  for the 6-block chain `the_dag_publishes_its_own_gauges` builds, both pinned there as literals, so a
  change that moved the value would be a change to the DAG rather than to how it is held — and each
  falsifier is the *mechanism* failing rather than a value changing:

  - `fringe_states` is keyed by `FringeData::fringe_hash_of(fringe)` — the **store's own key** — so the
    in-memory map is a faithful cache of the persisted one instead of a second, set-keyed index whose
    lookups compared whole fringe sets. Pinned by `fringe_states_are_keyed_by_the_stores_own_key`
    (map key, the datum's own `fringe_hash`, the persisted datum and the recorded `member_of_fringe`
    are one identity); the pre-Stage-5 shape cannot even express the comparison, which is why the pin
    is the identity assertion rather than a bound.
  - The merge builds its rejection map **from the final scope's own hashes**, holding a *borrow* of
    each fringe's `rejected_deploys`: the old shape walked every fringe and cloned every set — 500
    clones per merge on a 50-fringe DAG — to answer three lookups (`rejections_map.get` always treated
    absence as "not rejected"). Falsified against the tree it lives in: with the whole-DAG shape
    restored the map holds **500 entries against the final scope's 3**, and
    `rejections_are_indexed_for_the_final_scope_only` fails; its second assertion (the right fringes'
    rejections) keeps the bound from being met by indexing the wrong blocks.
  - The DAG index is held **once**: `DagState`'s three maps are `Arc`-shared with the representation, so
    the store's accessors hand back their own allocations and an insert moves three pointers instead of
    cloning the index again per block. `Arc::make_mut` keeps `add_block_to_dag_state` pure (the law
    15/18 property tests still pin it and `recreate_in_memory_state` by name) with the new
    `add_block_to_dag_state_mut` as the live path's form. Pinned by
    `the_index_is_shared_with_the_representation_not_copied` (`Arc::ptr_eq`, plus the copy-on-write
    half: a snapshot taken before an insert keeps the index it read). Falsified against this tree: an
    accessor returning `Arc::new((*map).clone())` — the copying behaviour, same type — fails it.
  - **The syncing page is no longer copied whole.** `comm/src/transport/chunker.rs` took two full copies
    of the packet content before any chunk existed (`blob.packet.content.clone()`, then a second clone
    of the same bytes in the uncompressed arm) and a third into the chunk proto's own `Vec<u8>`
    (`contentData` has no borrowed form), so a store-items page lived 3× at peak. It now borrows the
    packet and owns a buffer only when LZ4 compresses, leaving the chunk proto's copy as the only one.
    Pinned by `chunking_a_page_does_not_copy_it_whole`, whose instrument is **time relative to one
    explicit `Vec::clone` of the same payload in the same process**, so the bound is machine-speed
    invariant: measured here at a 128,000-byte page, 5,000 calls, min of three runs, **3.39–3.51
    µs/call borrowed against a 2.60–2.68 µs copy (ratio 1.30–1.33); with the two clones put back,
    10.15–10.28 µs/call (ratio 3.84–3.96)**, bound at 2.0. An honest limit: in the end-to-end page
    measurement below the removed copies are *within the noise* — first-touching the page's own fresh
    allocation dominates the extra memcpys — so the win this pins is peak copies (RSS), not loop time.

  **Still owed, and now measured rather than estimated (2026-09-24, Stage 6).** The steady *floor* is
  the Θ(N²) `seen` residency itself — H6 below, unchanged as a decision. Measured **on the running
  node**, isolated (one validator, `--data-volume devnet-stale-snapshot`, the 5,844-block artifact,
  blocking nothing): restored and serving at **`latestBlockNumber` 5,885 with 1.18 GiB RSS**, advancing
  to 5,937 at **1.20 GiB** (~+1.3 MB per block, no plateau needed to see it) with **zero** `not in
  graph` errors, zero refusals and no panic in the logs. The node's *own* accounting, read off
  `/metrics` (Stage 6's gauges, live): `rchain_dag_messages 5885`, `rchain_dag_seen_entries 17319555`
  (= N(N+1)/2 exactly — every block's `seen` is its whole ancestry in a single chain),
  `rchain_dag_index_entries 17655` (= 3N: blocks, child links, height groups),
  `rchain_dag_fringe_states 5883`, and **`rchain_dag_logical_bytes 556208877` (556 MB)** at 5,885
  blocks — H6's ≈553 MB, which was estimated from Σ|seen| × 32 B, now read off the structure. **The
  earlier "~9.8 GiB steady floor" figure in this row's history describes a tree that no longer
  exists** — the pre-Stage-1/3/4/5 one, where the DAG was copied on every read, insert and request —
  which is exactly the calibration trap this section's other half records; the floor that remains is
  the `seen` data (556 MB of the 1.18 GiB), not the copies of it. Still
  unfixed and named: a store-items page has only a node-count cap (`MAX_STORE_ITEMS_TAKE`) and no byte
  cap, and `handle_store_items_request` is **awaited inline in the dispatch loop** — measured on a page
  the handler produced (debug build, mock transport): an LFS page (750 nodes × 4 KiB, 3.1 MB) holds the
  loop **~50 ms** (18 ms serve + 32 ms chunk) and the largest page the cap admits (10,000 nodes, 41 MB)
  **~580 ms** (144 ms + 437 ms), during which no other message for that shard is handled
  (`node_launch.rs`'s single `recv`/`handle` loop) and the routing loop above backpressures on the same
  stall. Moving it off the loop is a behaviour change and its own unit; see C62.

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

- **C57 — law 16c's remaining tie is not a plumbing job: the model's encoder and `prost` disagree in
  three ways, and the register said only "the layer is the follow-up"** (measured 2026-09-24, Programme
  F, while sizing the `body` conformance layer; **a boundary finding, no defect, nothing to fix yet —
  the value is that it is now stated instead of discovered mid-attempt**). Law 16c's row is honest that
  `prost` is an external crate, so "these bytes are the node's bytes" is prose until a `body` layer
  exists. Sizing that layer meant reading both encoders, and they do not agree byte for byte:

  1. **Field order.** The model writes tags 4 (`blockNumber`), 6 (`seqNum`), 5 (`sender`), 17
     (`timestamp`), 9 (`justifications`), 14 — `Rchain/Casper/Validate.lean`'s `encodeBody`. `prost`
     writes in the generated struct's declaration order, which is the `.proto`'s ascending order
     (`models/proto/casper.proto:45-66` → `target/*/out/casper.rs`). The model's 4, 6, 5, 17, 9, 14 is
     not ascending, so the streams differ from the second field onwards.
  2. **Default-valued fields.** `prost`'s derive-generated encoder omits a field equal to its default
     (0, `""`, empty bytes, empty `repeated`); the model writes unconditionally
     (`varintField 32 (int64 b.number)` with no guard). A genesis block's `timestamp = 0` is the
     smallest instance, and an empty `justifications` list is another.
  3. **Field set.** The model writes **six** fields and collapses `version`, `shardId`,
     `preStateHash`, `postStateHash`, `bonds`, the three `rejected*` sets, `state` and `sigAlgorithm`
     into one opaque `header` blob written at tag 14 — which `prost` reads as `state`, a
     `RholangStateProto`. So the model's stream is *not* a protobuf encoding of a block body: it is the
     six fields the law's checks read, in the model's own order, which is what makes it a usable model
     and an unusable byte-tie.

  **Consequence, so the next attempt does not rediscover it.** A byte-level `body` layer needs the model
  to mirror `prost`: ascending order, default-skipping, and the full field list with the unmodelled ones
  as opaque blobs. That changes `encodeBody`'s *subject*, which puts `decodeBody_encodeBody` and
  `encodeBody_injective` back in play — and §17 already records that `decodeBody`'s first spelling (a
  `guard`-per-tag do-block) was a **compile-time** hazard: 98.6 s to compile and a build past 5 GB, the
  delegating `taggedVarint` per field fixing it at 224 ms. The options, recorded and not chosen:
  **(a)** mirror `prost` over the full field list — the real tie, and the largest; **(b)** mirror it over
  the modelled subset, with the omitted fields named as a boundary (a case's `header` bytes going into
  the proto's `state` field) — bounded, and it pins exactly the two rules a hand-written encoder gets
  wrong, order and skipping; **(c)** leave it prose, which is where it stands. Law 16c's row now cites
  this entry rather than saying only that the layer is owed.

- **C58 — law 1a's remaining gap is a *re-tagging of `cmpExpr`*, not eight more arms: the model cannot
  hold eight of the node's constructors, and the node's tags interleave** (counted 2026-09-24,
  Programme F, while sizing the sort-algebra extension; **a boundary finding, no defect, nothing changed
  — the value is that the gap and its cost are measured instead of described**). The register's law 1a
  row said the model's algebra is narrower than the node's ("21 against 33"). Counted exactly:

  - the node's **Expr-level tags are 33** (`models/src/sorter.rs:18-58`): 5 scalars (`BOOL` 1 … `URI` 4,
    `BIG_INT` 13), 4 collections (6–9), `EVAR` 100, 15 operators (101–114, `EMOD` 122), `EMETHOD` 115,
    `EBYTEARR` 116, `EMATCHES` 118, `EPERCENT` 119, `EPLUSPLUS` 120, `EMINUSMINUS` 121, `ESHORTAND` 123,
    `ESHORTOR` 124;
  - the model's `Expr` has **21** arms (`Rchain/Par.lean:49-70`) and its single `ground` arm covers five
    of the node's scalar tags plus `EBYTEARR` → 25 node tags covered, **8 uncovered**: `BIG_INT`,
    `EMETHOD`, `EMATCHES`, `EPERCENT`, `EPLUSPLUS`, `EMINUSMINUS`, `ESHORTAND`, `ESHORTOR`. The model's
    `Ground` has five arms and no `bigInt` (`Rchain/Syntax.lean:23-29`).

  **Five of the eight are terms the node can hold and the model cannot**, which is the opposite of the
  `bytes` case law 1a's row names as *unspellable*: `BigInt(42)` is a ground the grammar has a production
  for (`GroundBigInt`; C34's fix is what made it reachable), the parser accepts the `matches` operator,
  and `EMatches`/`EPercentPercent`/`EPlusPlus`/`EMinusMinus` are all in the port's own AST
  (`models/src/ast.rs:333-336`). So law 1a is not merely unpinned for them — it is unstatable, because
  the model has no value to state it about.

  **Why this is not "add eight arms".** The node's tags *interleave*: scalars 1–4, collections 6–9,
  `BIG_INT` **13**, vars 50–52, operators 100–124, `EBYTEARR` **116**. The model's `cmpExpr` classifies
  by constructor *kind* — one `ground` class that sorts before the collections, one class per operator —
  so a faithful order needs `ground (bigInt _)` to compare *after* `eset`/`emap`, and a byte array after
  every operator: a case split finer than one-class-per-constructor. Changing that split changes
  `cmpExpr` itself, and `cmpExpr` is compiled as `WellFounded.fix` (its block is one strongly connected
  component, so **nothing unfolds it**: `rfl` does not reduce it on constructors and `cmpExpr.eq_def`
  times out at `whnf`), which is exactly why its laws could only be proved by the 21 `@[simp]` arm
  lemmas over an `exprTag` numbering plus the two cross-case tag lemmas. A re-tagging has to be
  re-derived inside that structure. **That is the finding: the remaining gap is structural, and its
  cost is the comparator block, not the constructor list.**

  **Two things recorded beside it, both pre-existing.** (1) The model's `int` is Lean's `Int`
  (unbounded), so a model value `int (2^70)` corresponds to no node `GInt` (an `i64`) — a widening the
  `bigInt` work would sit next to rather than fix, and the reason the two cannot simply be merged.
  (2) **The corpus cannot pin any of this**: a `sort.tsv` verdict is `decide`d against the model, so a
  shape the model cannot *hold* cannot be a row. The gap is therefore a register fact and has to be
  findable in the register — which is what this entry is for, and why law 1a's row now cites it.

- **C59 — the set/map matcher over-claimed, and C54's reverted guard was the fix: what the reversion's
  corpus row could not see** (found, fixed and measured 2026-09-24, Programme F/the Lean pass;
  **the model changed, the port did not**). C54 reverted a length guard on the searching members because
  a corpus row it added (`@Set(1)` against `Set(1, 1)`, expected `false`) was answered `true` by the
  node, and concluded from that "there was no defect; the difference is a hypothesis the tie needs".
  **The first half of that conclusion is wrong, and the second half is incomplete.** Re-measured on the
  node through the receive path (`chan!(target) | for (bind <- chan)`) — the same shape the corpus's
  consumer runs, with the model's verdict `#eval`d against the committed `spatialMatch` beside it:

  | pattern | target | node | model, before | model, after |
  |---|---|---|---|---|
  | `@Set(2)` | `Set(1, 2)` | **false** | true | false |
  | `@{"b": 2}` | `{"a": 1, "b": 2}` | **false** | true | false |
  | `@Set(1, 2)` | `Set(1, 2)` | true | true | true |
  | `@Set(1, ..._)` | `Set(1, 2)` | true | true | true |
  | `@Set(1)` | `Set(1, 1)` | true | true | false |
  | `@Set(2, 1)` | `Set(1, 2)` | **true** | false | false |
  | `@Set(x, 1)` | `Set(1, 2)` | **true** | false | false |

  The first two rows are the defect, and **both sides are canonical** — sorted and duplicate-free — so
  C54's duplicate-element shape is not what they are about: the model's `matchListPar` drops *leading*
  targets, so a shorter canonical pattern matched a longer canonical target, while the port refuses an
  unequal length *before* it searches (`exact_match = !wildcard && remainder.is_none()`, then
  `if exact_match && plen != tlen`, `spatial_matcher.rs:684-693`). The guard is restored
  (`Match.lean`'s `eset`/`emap` arms), and C54's row is replaced by two whose targets the node evaluates
  to themselves — rows 21/22 of `match.tsv` — which the consumer accepts. **Why C54's row was not
  evidence either way**: its target `Set(1, 1)` is not a value the node can hold, because `par_set`
  deduplicates what `eval_expr` stores, so the node's `true` is set idempotence about the value `Set(1)`
  while the model was answering about the literal. A corpus row whose two sides are different values
  cannot test a clause.

  **The second finding is the direction C54 did not look**: the port's set/map assignment *backtracks*
  (`find_matches`' bipartite matching), so a pattern permuted relative to the target matches — rows six
  and seven above, both `true` on the node and `false` in the model, whose walk drops targets and never
  hands one back. That residue is **registered rather than fixed**: the matcher is a fuel-bounded
  function and a backtracking search needs a fuel at least quadratic in the nodes where `matchFuel` is
  linear (the port needs no fuel at all), so widening it moves the measure's arithmetic — the piece C47
  and C50 each recorded a defect in. The model is right on **canonical** inputs, which is the domain
  law 37's tie is stated over, and wrong on non-canonical *patterns*, which are reachable — so it is a
  divergence with a named boundary, not a modelling choice. Pinned by
  `a_permuted_pattern_is_refused`/`an_unaligned_variable_pattern_is_refused`; the register rows 5/37
  carry it.

  **The method note, which is C54's own lesson applied to C54.** That entry's error was comparing the
  model against a *reading of the port*; this one re-ran the node first, and the two divergences it
  found are in opposite directions — one the model over-claiming, one under-claiming. A clause-level
  fix cannot be validated by the corpus alone: the `decide`d rows are the model agreeing with itself,
  and the guard changes **no existing row** (all 20 were re-emitted byte-identical), so the only
  evidence for it is a new row whose verdict was measured on the node before the clause moved.


- **C60 — the tie's domain was still too wide, and the port's matcher is not the clauses at all: the
  fast path, the store's canonicalization, and the shapes they decide** (found and measured 2026-09-24,
  Programme F/the Lean pass; **the domain predicate changed, no port change**). Three findings, each
  measured on the node through the receive path before anything was written down, and each of which
  changes what law 37's owed tie can even say.

  **(1) The domain `modelledPar` + "a singleton expression list" + "canonical contents" is still too
  wide, and the shape that shows it is C51's own, nested.** A collection holding a *two-expression* `Par`
  is modelled and connective-free, its own expression list is a singleton, and the clauses answer
  `false` for the value against itself — because `spatialMatchExprs`' arm is `[p]` and the *inner*
  list has two entries:

  ```
  twoExprs = 1 | 2,  setHoldingTwo = Set(twoExprs)
  modelledPar setHoldingTwo = true          connectiveUsed setHoldingTwo = false
  spatialMatch setHoldingTwo setHoldingTwo = false      -- equality would say true
  ```

  The hypothesis therefore has to hold *at every level*: `pathPar` (a `Par` with every field but
  `exprs` empty, holding exactly one expression, that expression a ground or a remainder-free collection
  of `pathPar`s) is the predicate that says what "the shapes the clauses cover" always meant, and
  `a_nested_multi_expression_par_is_outside_the_path_domain` / `a_single_expression_par_is_in_the_path_domain`
  are its pins in both directions. `linear_of_pathPar` is the free-level half, proved.

  **(2) The port does not run these clauses for a concrete pattern at all.** `spatial_match_core`'s first
  act is `if !pattern.connective_used { return guard(fm, pattern == target) }`
  (`spatial_matcher.rs:193-196`) — and the port's *normalizer* names that same line as "the
  `pattern == target` short-circuit" (`normalizer.rs:1601`, in the comment explaining why a remainder
  must set `connective_used`). So on a connective-free pattern the answer is *equality*, not the clauses'
  verdict, and the two differ on a shape the node can hold:

  ```
  node:  @Set(1 | 2)  vs Set(1 | 2)   -> true      model: spatialMatch -> false
  node:  @[1 | 2]     vs [1 | 2]      -> true      model: spatialMatch -> false
  ```

  (A tuple cannot hold a par — `(1 | 2)` is a syntax error, measured.) This is C44's class again — a
  cost the model pays for a clause it does not have — except that the missing piece is not a clause: it
  is the short-circuit. It is **not** patched here, because patching it is a modelling decision for law
  37's row (adding the short-circuit makes the concrete domain definitional, which changes what the tie
  *is*), and because half of it would be worse than none: see (3).

  **(3) Both sides of every real match are already canonical, in the *port's* sense, and the model's
  `sortPar` is not that sense.** `BindPattern.patterns` and `ListParWithRandom.pars` are
  `Vec<SortedProc>` (`models/src/runtime.rs:20-36`), so the matcher receives `sort_par_term`-canonicalized
  terms on both sides; a datum is additionally `par_set`-deduplicated at `eval_expr`. The port's
  canonicalization sorts a `Par`'s *fields* and canonicalizes each collection element **in place** —
  `sort_expr`'s `EList`/`ETuple`/`ESet`/`EMap` arms are `ps.iter().map(sort_par)` with no `sort_by`
  (`models/src/sorter.rs:600-676`) — which is why the node keeps `[2, 1]` and `[1, 2]` apart (measured:
  `@[2, 1]` against `[1, 2]` is `false`) while a set literal's non-canonical order is harmless (measured:
  `@Set(2, 1)` against `Set(1, 2)` is `true`). **The model's `sortExpr` sorts collection contents**
  (`Rchain/Sort.lean:2010-2013`, `sortList parComparator (sortListPar ps)`), so it identifies
  `[2, 1]` and `[1, 2]` where the node does not. That divergence is definitional and none of the
  corpora can see it — a `sort.tsv` verdict is a pairwise verdict *between* terms, not a witness to what
  sorting a collection does to it, and the match corpus builds its `Par`s from literals — so it is
  recorded here as measured-from-the-definitions, then **measured for observability** (2026-09-24): the
  model applies `sortPar` in exactly four places — `Sort.lean` (law 1's own layer and the comparators),
  `Subst.lean` (`sort_subst`), `Ty.lean` (`closed_sortPar`, which holds for *any* sorting because sorting
  can neither introduce nor remove a free level) and `Corpus.lean` (the `sort` layer's emission, whose
  verdicts are *comparator* verdicts, which the model orders by its score tree and not by the sorted
  value) — and in **no** tied layer: `Store.lean`, `Match.lean` and `RSpace/*.lean` contain no `sortPar`
  at all, so no model layer compares a canonical *value*. `Casper/Validate.lean`'s `sortPar...` hits are
  its own `sortParents`, a different function. **So the divergence is definitional with no tied
  consequence**, and law 1's own statements (`sortPar_idempotent`, `sortPar_comm`) hold of the coarser
  form as well. What it does mean is smaller and is named rather than implied: the model's RSpace does not
  model payload canonicalization at all — the port's `BindPattern`/`ListParWithRandom` are
  `Vec<SortedProc>` and its `Eq`/`Ord`/`Hash` are canonical by construction — so a *future* model layer
  that compares payloads would need the port's `sort_par_term`, not this `sortPar`.

  **The consequence for the row**, which is why all three belong in one entry: law 37's obligation is
  stated as "the clauses decide exactly equality on the domain", and (2)+(3) mean the *port* does not
  decide that way on the domain the model can express — it decides by equality against the canonical
  form, and only consults the clauses when the pattern carries a variable or a remainder. The tie is
  still a true statement about the clauses (and is what `pathPar` is for), but its role changes: it
  justifies the short-circuit rather than describing the port's route. With (1) fixed and (2)+(3)
  measured, the remaining work is a modelling decision, and the row's note now says so.


- **C61 — a peer-supplied resume prefix of 128 bytes was one byte over the segment invariant, and the
  encoder silently truncated it to zero** (found and fixed 2026-09-24, Programme F's U1 item 7;
  `rspace/src/state/mod.rs::create_last_prefix`). The *export/resume* path codes a prefix as five
  hashes — the first hash's first byte is `sizePrefix`, the next four are 128 bytes of prefix — and
  the port's check was `if size_prefix > 128 { Err }`, ported from the Scala's
  `KeySegment(prefix128.take(sizePrefix))`, which has no check of its own. One byte over:
  `size_prefix == 128` was accepted, `KeySegment::new` built a 128-byte segment, and the radix
  encoder writes a segment's size in **7 bits** (`radix_tree.rs:121`'s `second & 0x7F`) — so 128
  serializes as 0 and the prefix is dropped, desynchronizing the restored tree against the hash that
  was meant to identify it. The Scala reaches `KeySegment.apply`'s `require(bv.size <= 127)` and
  throws; this port's stance for the same input is documented one line above the site ("the input is a
  peer-supplied resume path, so malformed shapes are an `Err`, not a panic"), so the fix is the
  refusal the stance promises, not a panic. **The fix is the checked constructor, not a corrected
  constant**: `KeySegment::try_from` *is* the 127-byte invariant (its own test pinned
  `vec![0u8; 128]` as refused before this site used it), so `prefix128.get(..size_prefix)` bounds the
  slice and the constructor bounds the domain — one boundary instead of two, and `TryFrom` gains its
  first production caller (it had none; a checked constructor nothing constructs enforces nothing).
  Falsified first: `a_128_byte_resume_prefix_is_refused` fails against the `> 128` bound (a
  128-byte segment comes back `Ok`). **The item's other three sites were measured, not changed**: the
  segment lengths they build are ≤ 127 by construction — `radix_tree.rs`'s node decoder takes the
  size from the same 7-bit mask the encoder writes, and `rspace_history_reader_impl.rs` builds a
  fixed 1 + 32 bytes — so routing them through `TryFrom` would add a refusal for an unconstructible
  case and, for the decoder, an error channel on a function that is total by design. Each carries a
  comment saying so, so the next sweep does not re-open them. `head()`/`tail()`'s empty-segment panic
  was already pinned with its reachability argument (`history_action.rs`'s
  `trimming_an_empty_key_panics`: "the tree never trims past a leaf, so this is latent — pinned
  because a change in reachability (or a guard) should fail a test rather than surface as a node
  panic"), and this pass leaves it there: carrying the invariant would be the ~30-site rewrite the
  plan's appendix calls option 7, and the argument for its unreachability is the tree walk's
  structure rather than an assert.


- **C62 — the sync path multiplied the page it served, and the node's own metrics surface had no
  wire** (found and fixed 2026-09-24, Programme F's performance tail; the *inline* store-items handler
  is measured and deliberately **not** fixed). Three findings on one path, and the instrument that
  makes the first two checkable rather than asserted:

  1. **A store-items page lived 3× in memory.** `comm/src/transport/chunker.rs::chunk_it` took two
     whole copies of the packet content before any chunk existed — `blob.packet.content.clone()`, then
     a second clone of the same bytes in the uncompressed arm — and a third into the chunk proto's own
     `Vec<u8>` (`ChunkData.contentData` has no borrowed form), so the page a syncing peer pulls most of
     existed three times at peak. It now borrows the packet and owns a buffer only when LZ4 actually
     compresses; the chunk proto's copy is the only one left, and it is the one ownership transfer the
     wire requires. **Falsified against the tree it lives in** — the rule this pass paid for twice:
     `chunking_a_page_does_not_copy_it_whole` times the work *relative to one explicit `Vec::clone` of
     the same payload in the same process*, so the bound is machine-speed invariant, and with the two
     clones put back it reads **3.84–3.96×** one copy against **1.30–1.33×** for the borrow (bound
     2.0; 10.15–10.28 µs/call against 3.39–3.51 µs/call, at a 128,000-byte page, 5,000 calls, min of
     three runs). Its honest limit is recorded with it: in the end-to-end page measurement below the
     removed copies are within the noise, because first-touching the page's own fresh allocation
     dominates the extra memcpys — the win pinned is peak copies (RSS), not loop time.
  2. **`/metrics` had no wire, so the node could not report any of this.** The route was mounted and
     `NewPrometheusReporter` could render, but nothing in production called `report_period_snapshot`,
     so every scrape returned the reporter's initial placeholder string. The handler now publishes a
     snapshot of the node's `MetricsRegistry` before rendering, and the DAG publishes into that
     registry — `messages`, `seen_entries`, `fringe_states`, `index_entries`, `logical_bytes` — from
     `create`, from `with_metrics` and from every `insert`, so `/metrics` reports the structure the
     cost claims are about. Pinned by `the_metrics_route_serves_the_registrys_own_numbers` (the
     falsifier — removing the single call — restores exactly the placeholder string) and by
     `the_dag_publishes_its_own_gauges` (the published number *equals* the representation's own
     accounting, and grows with the DAG). **No law covers this half, and none should**: it is the
     difference between a measurement that exists and one that is assumed — which is also why the
     gauges' own cost is stated rather than hidden: the sums are over `len()`s, Θ(N) per insert, not
     over the sets.
  3. **`handle_store_items_request` is awaited inline in the shard's dispatch loop — measured, not
     fixed.** `node_launch.rs` serves a shard's messages from one `recv`/`handle().await` loop, and
     this handler walks the exporter, materialises the page, serialises it and hands it to the
     transport before returning; the routing loop above it backpressures on the same stall. Measured
     with the handler's own response (debug build, mock transport, so the number is the shard-side
     part that holds the loop): an LFS page (750 nodes × 4 KiB, 3.1 MB) holds it **~50 ms** (18 ms
     serve + 32 ms chunk) and the largest page the cap admits (10,000 nodes, 41 MB) **~580 ms**. Moving
     the handler off the loop is a behaviour change, and a byte cap on a page is the other half of the
     same decision (the requester's `PAGE_SIZE` is self-consistent, so a *responder* that truncates
     breaks the requester — the deferred item this row deliberately leaves open, with its number).

  **The serving term, measured on the fixed tree (2026-09-24).** A node on the 5,844-block artifact
  (extended to 6,339 by the run) with **two fresh validators pulling from it** — brought up beside the
  existing network by `DEVNET_PREFIX=perfsync`, so the measurement needed no deletion:

  - the server's RSS grew **115 MB while serving 263 blocks ≈ 0.44 MB per block** (peak 2-minute
    window 0.71 MB/block), against the ~0.86 GiB/block this pass retired — and it is *slope*, not a
    spike that later plateaus;
  - **63 of 63 block requests answered, zero "block not found", zero rate-limit drops**, and exactly
    63 blocks received across the two validators — the serving path is not the bottleneck;
  - **zero** `History items are corrupted`, `Validate received state items` or panic lines on any of
    the three nodes over the whole window — Stage 2's criterion on the sync path.
  - **What this run does *not* show, and why**: neither validator reached the bootstrap's height. The
    LFS block walk runs *backwards* from the tip at **~1.5 blocks/minute per validator** (3 blocks in
    120 s, observed mid-walk at #6,018 → #6,008), so a 6,300-block chain is a ~70-hour walk at that
    cadence. **C64 measures what that rate is, and it is not the requester's pacing**: isolated against
    a peer that answers every request, a 6-block walk with the production 30 s idle timeout finishes in
    3.85 ms, so the requester is response-driven and faithful — the server answered every request it
    received (63/63, and `request_next` can broadcast only after a successful store read), and the
    ~25 s per generation is spent on the *syncing* node's single message loop, which carries blocks and
    the concurrent tuple-space state sync. What multiplies that latency into days is the registered §6
    deviation C64 names: the port walks the full ancestry to genesis, one generation per block, where
    the oracle's bounded walk makes ~50. It is recorded here because this unit touched the same path,
    so a later reader does not have to re-derive it. The *functional* three-validator path was verified
    independently by C46's end-to-end check.

  The law is **10**, whose Merkle determinism is what the page is a wire form of: a page is the trie's
  own nodes, and how a node materialises and copies them is the difference between serving a peer and
  OOMing while doing it.


- **C71 — the matcher's padding is reachable from a well-formed term, so U10's refusal was refuted by
  the differential vector** (found 2026-09-24 by the workspace suite; fixed the same day). C52's refusal
  — `resolve_match` and `RhoMatch::get` raising `BugFoundError`/`MatcherFailed` for a free level the
  matcher's map does not carry — rested on the claim that the state is unreachable, since the normalizer
  derives `free_count` from the same pattern. **It is reachable, and the corpus says so**:
  `legacy/rholang/src/main/k/rholang/tests/Std-Pattern-Matching/matching-parallel-processes.rho` is

      for( @{x | y} <- @Nil ){ @1!(x) | @2!(y) } | @Nil!( "success" | "success" )

  and the matcher is *greedy by design* — measured, not argued: for the pattern `{x | y}` against a par
  of two exprs, `spatial_match_result` returns bound levels **[0]** only, `x` holding the whole par. The
  level `y` names is then absent from the map, which the oracle pads with the empty par
  (`storage/package.scala:22-29`'s `case None => Par.defaultInstance`), and the program's own header
  documents that as its expected output: `@1!("success" | "success") | @2!(Nil)`. So `free_count` means
  "the levels the pattern *names*", not "the levels a given match happened to fill" — and U10's refusal
  turned a differential vector into `ReduceError("matcher failed: the pattern declares free level 1 of 2
  but the matcher bound no value for it")`.

  **Fixed by padding in both places**, as the oracle does — `RhoMatch::get` (`storage.rs`) and
  `resolve_match` (`reduce.rs`) — and the §6 deviation row U10 added is **deleted rather than amended**,
  because the port conforms again. Falsified by the corpus itself: `cargo test -p rchain-rholang --test
  legacy_contracts` fails on the restored refusal and passes with the padding; the two unit tests were
  rewritten to pin the oracle's semantics
  (`resolve_match_pads_a_level_the_pattern_does_not_bind`,
  `rho_match_pads_a_free_count_its_pattern_does_not_bind`), so the next reader sees the padding as
  *intended behaviour* rather than an oversight.

  **The process lesson, recorded because it is the transferable part:** U10 landed a change to the
  matcher's contract and ran only the `rspace`/`rholang` scoped suites, so its own differential vector
  never ran. From here, **a change in `rholang/src/{storage,reduce,matcher}` runs `legacy_contracts` and
  `execution` before it lands** — the scoped unit suites cannot see this class at all, because the
  matcher's *contract* is what the corpus pins.

- **C64 — a fresh validator cannot catch up, and it is a registered deviation multiplied by a
  latency that is not the requester's** (found 2026-09-24 by measurement, Programme F's U6 serving-term
  run; **no port change — the pacing is faithful and the extent is already registered**, but the
  operational consequence is named here for the first time). Two fresh validators pulling from a node
  on the 5,844-block artifact walked the chain at **~1.5 blocks per minute each** (3 blocks in 120 s,
  observed mid-walk at #6,018 → #6,008) — a **~70-hour** walk for that chain. A rate that looks like a
  requester defect, and it is not:

  1. **The requester's pacing is faithful, and measured.** `lfs_block_requester.rs`'s loop is the
     Scala's `requestStream`/`responseStream` structure element for element (`Queue.dequeueChunk`
     ↔ the trigger channel, `evalOnIdle(resendRequests, requestTimeout)` ↔
     `select! { recv, sleep(request_timeout) }`), and `LfsState` mirrors `ST`'s `getNext` / `received`
     / `add` / `done` / `isFinished`. Isolated against a transport that answers every request,
     the walk of a 6-block chain with the production 30 s `requestTimeout` finishes in **3.85 ms** —
     `casper/src/engine/lfs_block_requester.rs`'s
     `the_walk_advances_on_responses_not_on_the_idle_timeout`, bounded at 5 s, which is three orders of
     magnitude above the healthy shape and *below one idle timeout*, so a walk that advanced on the
     resend could not pass it. **Falsified in the sense that matters**: the instrument's units are the
     idle timeout, and the measured value is 4 orders below it.
  2. **What the walk is *for* is what makes it long.** On a chain a block's only justification is its
     parent, so the requester is one generation — one round trip — per block, in both implementations.
     The Scala's `ST` bounds the walk (`blockIsAccepted = isReceivedLatest || blockNumber >=
     minimumHeight`, with `blockHeightsBeforeFringe = deployLifespan`), so its own walk covers ~50
     blocks below the fringe; the port walks the **full ancestry to genesis** because a syncing node's
     DAG is empty and `dag.insert` requires every justification to be present — the deviation already
     registered in §6, whose reason is restated there with this measurement. That is 6,300 round trips
     where the oracle makes ~50: **the deviation is upstream of the rate, and the rate is upstream of
     the liveness fact.**
  3. **The per-generation latency is not the requester's.** The server answered **63 of 63** block
     requests (zero "block not found", zero rate-limit drops) and the two validators received exactly
     those 63 blocks, while the requester logged its own 30 s resend warning between arrivals — so the
     requests are answered, and the ~25 s per generation is spent on the *syncing* node, whose single
     message loop carries blocks and the concurrent tuple-space state sync (`run_approved_state_sync`
     joins a 30 s block walk with a 120 s state sync and then requests every downloaded block's state
     root). **That attribution is the remaining question, named with its experiment**: timestamp block
     arrivals against tuple-space page completions in a fresh validator's log to see whether they
     interleave — the distinguishing observation is whether arrivals cluster at page boundaries. It is
     *not* C65's swallowed store error, and that is checkable rather than argued: `request_next` can only
     broadcast after a successful `contains`, and the server logged 63 requests, all answered — so the
     store read was succeeding and that defect was not firing on this run.
  4. **The consequence, stated as a liveness fact rather than a cost preference**: a fresh validator
     does not "join" a long chain in any useful sense — it is hours-to-days behind, and the *chain
     length* is what makes it so. Two levers, neither of them a one-liner: the **extent** (bounding the
     walk as the Scala does needs a way to satisfy `dag.insert` without the full ancestry, i.e. a
     design change, not a cutoff) and the **per-generation latency** (item 3). This unit fixes
     neither; it names both, so the next one starts from the measurement rather than the symptom.


- **C65 — the block store's side of LFS sync swallowed two failures the oracle propagates, so a
  transient store error became a permanently missing block** (found 2026-09-24 by the Phase-0 sweep,
  Programme F's U13; **fixed**, both halves, falsifier first). The same class U12 closed in the
  tuple-space importer, unfixed on the block path it runs beside:

  1. **A failed block write was discarded and the block marked done anyway.** `save_block` read
     `already_saved` through `contains(..).unwrap_or_default().first().copied().unwrap_or(false)` — a
     store error reading as "not saved" — then `let _ = block_store.put(..)`, and then recorded the
     block as `done` regardless. `done` removes the key from the request map, so a block that was never
     persisted was also **never requested again**: a permanently missing block in a state the requester
     reports as synced, with a done bit claiming otherwise. The oracle's `saveBlock` is
     `containsBlock(..)` then `putBlockToStore(..)` in `F` and only then `st.done`: a failed write fails
     the sync attempt, and NodeSyncing's own `Lfs state sync failed` path is where it lands.
  2. **A failed store read read as "nothing to request".** `request_next` computed its work set from
     `block_store.contains(&hashes).await.unwrap_or_default()`: an error yields an **empty** `Vec<bool>`,
     the `zip` yields nothing, and every hash is silently neither reported as existing nor requested —
     so the walk does nothing until the idle resend nudges it. The oracle's
     `hashes.toList.filterA(containsBlock)` propagates.

  **Fixed** by making the walk fallible end to end: `save_block`/`process_block`/`request_next` return
  `Result<(), String>`, `request_blocks` returns `Result<St, String>`, and a store failure from either
  loop ends the walk as an error, landing in `NodeSyncing`'s existing `Lfs state sync failed` log — the
  same place the oracle's stream failure lands. **Neither implementation retries within that fringe**:
  the Scala's `startRequester` is a one-shot flag and the port's handler `take`s the importer and the
  receivers, so a transient store failure costs the attempt (and, with C68 fixed, the node stays in
  `NodeSyncing` rather than running on a partial DAG). **Falsified in the witnessing form, both halves**, against this tree:
  `a_failed_block_write_fails_the_walk_instead_of_marking_the_block_done` (with the discard restored the
  walk returns success over an empty store) and
  `a_failed_store_read_fails_the_walk_instead_of_requesting_nothing` (with `unwrap_or_default()` restored
  the walk neither fails nor progresses for the whole 5 s bound against a 1 s idle timeout).

  **Why this is recorded next to C64, and what it does *not* change.** Defect 2's symptom is
  *indistinguishable* from a pacing stall — a walk with an empty request set waits on exactly the
  timeout whose warning C64 reports — so the cadence finding had to rule it out rather than assume:
  `request_next` can only broadcast after a successful `contains`, and the devnet logged **63 block
  requests, all answered**, so that call was succeeding there and the defect was not firing. C64's
  attribution therefore stands on evidence, not on the absence of a suspect.


- **C68 — a failed LFS sync still signalled the node out of syncing, so it could run on an
  incomplete DAG** (found 2026-09-24 while fixing C65 — that fix is what makes it reachable; **fixed**).
  `NodeSyncing`'s sync task ends by notifying `finished`, and that `Notify` is the *only* signal
  `node_launch` waits on to leave the syncing state for `NodeRunning`. The port fired it *after* the
  match over the sync's outcome — on failure as well as success. The oracle sequences it inside the
  effect chain instead: `requestApprovedState` ends with `finished.complete(())` **after**
  `parJoinUnbounded.compile.drain`, and `putApprovedBlock` is deliberately sequenced after the state
  ("to restart requesting if interrupted with incomplete state"), so a failed attempt leaves the node in
  `NodeSyncing` with the approved block unrecorded.

  **Impact, and why it had to be fixed with C65 rather than after it**: a node whose LFS sync fails
  transitions to running on an empty or partial DAG. Before C65's fix that was reachable only through
  the tuple-space leg's error channel; C65 (correctly) routes block-store failures out of the walk into
  exactly this path — so fixing C65 alone would have *increased* how often the node leaves syncing with
  a bad DAG.

  **Fixed** by moving the signal behind the outcome (`notify_when_restored`): only `Ok` notifies, so a
  failed attempt leaves the node waiting on `finished` as the oracle does. **Falsified against this
  tree, both directions**: `a_failed_sync_does_not_signal_the_node_out_of_syncing` asserts the waiter
  stays asleep after an `Err` and wakes after an `Ok` — the second half so the first cannot pass by the
  signal never being wired — and with the unconditional notify restored it fails on the first
  assertion.

  **Closed end to end (2026-09-24, U16)**: `a_failed_sync_leaves_the_node_in_syncing` builds the
  fixture this entry asked for — a real `NodeSyncing`, a block store whose reads fail, an importer that
  refuses to open the root (so the tuple-space leg fails at once rather than waiting out its 120 s
  timeout), a silent transport and a recording log — drives the real handler with a fringe from the
  bootstrap, and asserts the `finished` handle is **not** notified while the log proves the attempt ran
  and failed ("LFS state sync failed"), so the assertion cannot pass because nothing happened. It also
  pins that the fringe is not recorded as approved, which is the oracle's own sequencing comment ("to
  restart requesting if interrupted with incomplete state").

  **The fixture's first version passed with the defect restored, and that is recorded because it is
  this pass's recurring lesson in a new form.** It created the waiter *after* the failure and asserted
  "no notification within 300 ms" — but `Notify::notify_waiters` wakes only waiters *already
  registered*, so the notification had long since fired into an empty waiter set. The test now
  registers the waiter before the attempt can signal (the same `select!` drives the handler and the
  waiter) and its falsifier fails correctly against the defect: the *moment* a tripwire observes is
  as much a part of its calibration as the numbers it uses.


- **C70 — the gate could not see a nested comment, and the widened token set is the only check that
  sees an assumption which is not an `axiom`** (found and fixed 2026-09-24, Programme F's hygiene unit;
  **harness**, no law — what was wrong is the instrument, not the term). Two findings in one scan, both
  in `tools/check-lean-conformance.sh`:

  1. **Every other check is blind to an assumption that is not an `axiom`.** `Laws.lean`'s accounting is
     exact equality over `axiom` *declarations*; the gate's other steps check `sorry`/`admit`, module
     completeness, the emitted corpora and the counts. So a definition written `opaque` (its body hidden
     from reduction), or `partial`/`unsafe`/`extern`, or a body swapped for an untrusted one with
     `@[implemented_by]`, is neither an axiom nor a `sorry` and passes every one of them. The scan now
     refuses those tokens — the point of the unit is that an assumption *arriving* should be refused
     rather than discovered later, and that the tree declares all of its assumptions as `axiom`s it
     counts.
  2. **The comment/string stripping had to be repaired before the token set could widen — and the blind
     spot was already there.** The state machine tracked a *single* block-comment level, so Lean's
     nested block comments closed at the first `-/` and the rest of the comment was scanned as code.
     **Stated at its worst, because that is what it was: a `sorry` written inside a nested comment would
     have been invisible to the gate that exists to find it** — the ratchet's zero was, for those
     regions, a statement about nothing, and nothing in the pass's history would have noticed. It was
     invisible for as long as the tokens were `sorry`/`admit` (no prose line happens to carry those
     words) and became 23 immediate hits when `partial`/`opaque` were added — exactly the
     "`external` contains `extern`"-class of trap the token shape already guarded against, one level
     down. String literals were scanned too, and the register keeps its own row prose in them
     (`note := "...opaque..."`): 11 more hits. Both are handled now — nesting depth, strings with
     escapes and line continuations, and a character literal holding a double quote (two of the tree's
     files have one) which must not open a string — and the tree scans clean.

  **Falsified in both directions at scan level**, which is the level the change lives at: the gate's own
  `awk` program, extracted from the script so the probe runs the identical text, over the tree's own file
  list plus probe files (nothing planted in the tree, so no peer's gate run could see it). A probe
  carrying each token fires on all five and **not** on `external`; a prose-only probe and a
  string-carried-prose probe fire on none; a nested comment containing `partial` fires on nothing while a
  real `sorry` beside it fires (the original ratchet intact); and a `'"'` character literal does not
  blind a following `opaque`. The tree's own files, which mention these words in prose constantly,
  produce zero hits.

  **Two things the edit itself had to learn, both recorded because a gate is not exempt from the pass's
  own rules.** (a) `Laws.lean` cites `tools/check-lean-conformance.sh:207` as law 30's witness, and a
  *line-anchored* citation into a script moves whenever a step above it is edited: the first version of
  this change grew the region by 40 lines and the register audit failed on the stale window. The fix was
  to keep the scan region exactly as long as it was (the program compresses to 19 lines with the same
  behaviour) and to put the prose explaining it *below* the corpus-to-test mapping. The deeper fix is
  named for whoever owns the audit's citation rule: **a citation into a script should anchor on a token,
  not a line**. (b) Compressing the awk program for that line-stability introduced a typo — the close
  marker read `"- /"` instead of `"-/"`, which left the scan *blind* (an unterminated block comment
  swallows the file) rather than noisy, and it still passed the tree (zero hits) and three of the seven
  probes. The probe battery caught it on the one case that can see it (five tokens must fire, and they
  read zero). That is the pass's rule exactly: an instrument that cannot see the defect it names is not
  evidence, and a scan that has gone blind looks identical to a clean tree.

  **The plant, in the tree** (the falsifier as the unit asked for it): `spec/Rchain/HygieneProbe.lean`
  carrying `opaque def probeRatchet : Nat := 0` makes the scan report it and therefore trips the gate's
  `[[ -s log ]]` fail branch; deleting the file restores a clean scan. The plant lived for one command
  and the file is gone.

  **The citation, and the fix that ends the fragility.** Law 30 cites a line of this script for
  `lean_parse_corpus`; the first version of this change moved it `207 → 247` and
  `tools/audit-test-register.sh` failed on the stale window (correctly — it is check 9's job). The
  landed version restores it (`208` against `207`, inside the check's ±8 window, audit green) by keeping
  the region line-neutral and putting the explanation below the mapping. `register-lean` is re-anchoring
  that citation in the **symbol** form (`tools/check-lean-conformance.sh:lean_parse_corpus`), which is the
  deeper fix this finding recommends: a citation into a script should name a token, not a line — the line
  moves every time a step above it is edited, and nothing in the register can tell.

  **What was not run: the whole gate.** Its first step is `lake build`, and the Lean slot belongs to the
  lead — so the end-to-end run of the modified gate is owed and named in this unit's handover rather than
  assumed here. The scan step is the one that changed, and it is the step that was exercised, in both
  directions.


- **C72 — four register cells, three prose paragraphs and two citations called something owed while the
  tree held it, in the one place the checks cannot see** (found 2026-09-24 by the hygiene unit, while
  reconciling the records against the tree; **records corrected, no code change**). The pass has two
  machine checks over its own records — the register audit (citations, counts, named tests) and the Lean
  gate (the emitted registers, the corpora, the counts) — and this is what they cannot read:

  1. **The law matrix in `spec/TEST-COVERAGE.md` is checked by neither.** A *prose* cell there has no
     `.tsv` and no count token, so nothing reads it: law 38's cell still said `takesStep_iff_reduces` was
     **owed** (law 38's row: both directions proved, so it is a theorem and not an axiom), law 42's said
     `decode_encode` was **owed** (its row: "the axiom is gone, and it was not merely owed"), C13's §20
     row said the round-trip corpus was "still open" while `Print.lean` and law 33's printer rows exist,
     and an aggregate row for laws 30/31/33/36 said "not yet defined" while the rows *directly above it
     in the same table* were proved-tied. Each cell was checked against the register's own row before it
     moved.
  2. **Three prose paragraphs named work the tree had done.** `AUDIT.md`'s C53 residue — "**Owed, and it
     is the real fix**: an error channel" — landed in U12 (`load_node` returns `Result<Node, String>`,
     `RSpaceImporter::get_history_item` returns `Result<Option<Vec<u8>>, String>`). `AUDIT.md`'s U12 note
     — "the outright conversion is **blocked** on two production call sites" — landed with the same unit
     (the three setters through their `?`, `RNodeStateManager::is_empty` fallible).
     `RUST-VS-SCALA.md` — "the 30 element-comparator axioms … remain to discharge" — is contradicted by
     law 1b's row ("**no axioms, from twelve — the residual is empty**") and by `Sort.lean` declaring no
     `axiom` at all.
  3. **Two citations pointed at a file this tree does not have.** `AGENTS.md` and `spec/INVENTORY.md`
     cite `casper/src/main/resources/casper.tla` for the bootstrap-ceremony model; it is
     `legacy/casper/src/main/resources/casper.tla`. Both now say so.
  4. **`faultTolerance` read as pending work and is legacy-only.** Both mentions cite
     `integration-tests/test/test_dag_correctness.py`, which exists only under `legacy/` — and the port's
     integration checklist *mirrors* that suite rather than running it
     (`tools/run-integration-tests.sh:34`), so no formula is owed here. Recorded as legacy-only.

  **Why nothing caught any of it, which is the generalisable half**: a record is checked where it is
  *machine-readable* — a `.tsv` count, a citation line, a test name — and unchecked where it is prose.
  Prose therefore drifts at exactly the rate the tree moves, and it is the part a reader meets first.
  The cells in that matrix that name a test or a count stayed correct throughout; the four that were
  sentences did not. The matrix now carries a comment saying so, because the fix for a blind spot is to
  state where it is, not to promise the next reader will check by hand.


- **C73 — a store-items page had a cap on its *count* and none on its *bytes*, so one request bought tens
  of megabytes of the responder's memory** (found 2026-09-24 in the performance walk's tail, fixed by the
  hygiene unit; **fixed as a refusal**, registered as a §6 deviation). `MAX_STORE_ITEMS_TAKE` bounds how
  many nodes a request may name (10,000, itself a hardening). Nothing bounded how much data those nodes
  carry: a node's *value* size is chosen by the chain's contents, not by the request, so the maximal
  `take` with fat values makes the responder materialise and send tens of megabytes — measured at
  **~41 MB** for 10,000 nodes of 4 KiB items in the handler's own test, and unbounded above that.

  **The fix is a refusal, and that is forced rather than chosen.** The requester recomputes the page it
  expects and requires the received keys to match (`validate_state_items`), so a *short* page is not a
  smaller answer — it is a wrong state claim the importer would validate its own traversal against, which
  is exactly what makes the requester's own `PAGE_SIZE` self-consistency insufficient as a defence. With
  no error reply to send, the responder's choices are a drop or a lie, and this handler already drops in
  the two neighbouring cases (an unreadable store, C63; an over-large `take`). `MAX_STORE_ITEMS_BYTES =
  32 MiB` sits ~10× above the largest legitimate page (`PAGE_SIZE = 750` × 4 KiB ≈ 3 MB) and below the
  ~41 MB the maximal `take` produces at that item size, so the two caps bracket the same hostile request
  from both sides. The check runs after assembly and before the response exists, so an oversized page is
  neither serialised nor sent.

  **Registered as a deviation**: the oracle has no such cap — it serves whatever it is asked — so a peer
  that asks for a page the Scala would serve gets a drop here (§6). The *cost* left standing is the
  assembly itself, which the responder pays before it can measure the page; what the cap removes is the
  bandwidth, the peer's memory, and the per-request ceiling a hostile peer could otherwise pick.

  **Falsified against this tree, both directions in one test**
  (`a_page_over_the_byte_cap_is_dropped_not_truncated`): a 200 × 200 KiB page (~40 MB) is dropped with
  **no streamed response**, and the largest legitimate page is still served — so the drop cannot pass as
  "this handler stopped working". Disabling the check makes the first half fail on exactly that assertion.


- **C74 — the name-shape vocabulary is a convention, not a predicate, and the measurement says which half
  is checkable** (found and measured 2026-09-24, Programme F's hygiene unit; **a boundary finding — no
  defect, nothing to fix**). `spec/STYLE.md` names six shapes for load-bearing declarations (`the_*`,
  `a_*`/`an_*`, `*_is_false`, `*_refutes_*`, `*_diverges`, `*_refuses_*`/`*_rejects_*`) and says every
  `provedTied`/`provedModel` row must name one in its `witness` field or carry a corpus. The rule is
  checked and holds — **0 of 49 proved rows lack both**. The shapes are not, and the numbers say why:

  - of the **126** entries in the register's `witness` field, **63 match a shape and 63 do not** — the 63
    are the model's own declarations, which is what the field is *for* (`joinKey_perm`,
    `mergeChanges_assoc`, `encodeNode_injective`, `reduce_not_deterministic`). A check asserting the
    shapes over that field fails on 63 legitimate rows, which is the falsification: the imagined check is
    wrong, not the names. Renaming a declaration to fit a style is renaming the mathematics.
  - the `falsifiable` column is no better a source: it mentions **275** names, **232** of which are
    declarations under discussion rather than witnesses, so "the witness is the backticked name" is false
    as a reading rule too.

  **What this leaves**: the shapes are a convention for whoever *writes* a falsifier, and the machine-checked
  part of the rule is presence plus existence plus not-an-axiom (checks 3, 4b, 5b). `spec/STYLE.md` now
  says exactly that, with these numbers, so a reader does not assume the gate enforces a style it cannot.
  No code and no Lean changed: this row exists so the next person who tries to mechanise the vocabulary
  starts from the count instead of from the table.


- **C75 — the gate could not see a Lean module at the top of `spec/`, so a `sorry` there was invisible to
  the ratchet that exists to find it** (found 2026-09-24, Programme F's hygiene unit; **fixed**). Both of
  the gate's file-scoped steps derived their scope from `Rchain/`: the completeness step built `expected`
  with `find Rchain -name '*.lean'` *inside* `spec/`, and the token scan listed `spec/Rchain.lean` plus
  `spec/Rchain/**`. A module at the top of `spec/` is outside both — and outside `lake build` too, since
  nothing imports it and it is never compiled. A `sorry` written in such a file was invisible **three
  times over**, and so is an `opaque`, a `partial` or an `unsafe` definition: the scans and the build all
  agree it does not exist.

  **Falsified before the fix, with two probes** — `spec/ProbeSorry.lean` carrying `sorry` and
  `spec/ProbeOpaque.lean` carrying `opaque def probeOpaque : Nat := 0` — run against the script's own
  commands: the token scan over its declared file list reported **0 hits** (neither probe is in the list),
  and the completeness step's `expected`/`actual` compared **equal** with both files present. Both probes
  are this row's evidence, and neither was a stray file: the finding is the blind scope, not their
  existence.

  **Fixed by widening both scopes to every `.lean` under `spec/` except the build tree**, with the policy
  written into the check rather than left implicit: **a Lean file under `spec/` is either the library
  root, a module the root imports, or a declared `lean_exe` root — there is no third kind, and a scratch
  file belongs outside `spec/`**. `.lake/` is excluded explicitly, and the exclusion is exactly why the
  old scope looked reasonable: it holds **5,668** generated `.lean` files, so a bare
  `find "$SPEC" -name '*.lean'` would put every one of them in the list. The library root is dropped from
  `expected` (it *is* the import list, so nothing imports it).

  **Falsified after the fix, both directions**: with the two probes present the token scan reports both
  lines (`ProbeOpaque.lean:1`'s `opaque` and `ProbeSorry.lean:2`'s `sorry`) and the completeness step
  fails on the two unimported modules; with them removed, both steps are green again. All four runs are at
  *step* level, using the script's own commands extracted from it — the whole script was **not** run,
  because its first step is `lake build`, the Lean slot is the lead's, and the sort unit makes the tree red
  by construction until it lands.

  **A side effect worth recording, because it is C70's lesson landing on this very edit**: widening the
  scope shifted the script's line numbers, and `Laws.lean` cites a *line* of it for law 30 — the register
  audit failed on the stale window, correctly. The explanation was therefore moved **below** the cited
  mapping and the edit held to a single added line (the mapping sits at 209 against a citation of 207,
  inside the check's ±8 window). That is the third time this session a line-anchored citation into a
  script has been moved by an edit above it; the symbol-form re-anchor `register-lean` is landing is the
  fix, and C70 already carries the recommendation.

  **The class, with three instances**: C70's nested comments (the ratchet's zero was a statement about
  nothing for any nested region), check 9's collapsed column (a comparison blind to a whole column), and
  this (a scope that excluded the files). Each was invisible for as long as the blind spot and the defect
  never coincided, and each was found by *extending or probing the instrument* rather than by reading the
  claim — the argument for probing an instrument at the edges of its scope as a matter of course.


## 20. The back-sweep: every incident to its law and its case

The programme began with ten defects of one class — "nothing errors" — found on a running node,
patched, and understood only afterwards. This table is the answer to the question they left: *which
law would have caught this one?* Each row names the incident, the law that now covers it, and the case
that fails if the behaviour returns. A row whose third column says **harness** or **retired** is a
finding that no law covers, and it says why rather than leaving the gap to inference.

- **C63 — the state-export path flattened store errors into "no state", and one of them into
  "corruption"** (found and fixed 2026-09-24, Programme F's U11; `rspace/src/state/`). C53's class
  again, in a *consumer*: `RSpaceExporterStore`/`RSpaceImporterStore` read their three stores through
  `unwrap_or_default()`, so a store that is momentarily unreadable produced **no nodes, no items and
  no root** — and the consequences compound, each naming the wrong thing. `get_history_and_data`
  turned a store error into `EmptyHistoryException` ("this state has no history");
  `RSpaceStateManagerImpl::is_empty` answers "yes" for it; and `write_to_disk`'s validation, which
  re-reads the *target* history store through a closure that flattened to `None`, made the traversal
  come up short and reported `"History items are corrupted."` — a verdict about the *peer's chunk* for
  a store that merely failed to answer. The oracle is `F`-shaped at every one of these points
  (`RSpaceExporter.scala:15`'s `def getRoot: F[KeyHash]`, `:28`'s
  `getFromHistory: Blake2b256Hash => F[Option[ByteVector]]`, and `traverseHistory` returning
  `F[Vector[TrieNode]]`), so the port was dropping a channel, not lacking one. **The fix is the
  checked-sibling shape**, the same one `RadixTree` uses: the traits gain
  `try_get_nodes`/`try_get_history_items`/`try_get_data_items`/`try_get_root` with *default* bodies
  delegating to the total forms (so every existing implementation, `casper`'s mocks included, compiles
  and behaves unchanged), the store-backed implementations override them with the real reads,
  `traverse_history` and `NodeDataReader` take `Result<Option<..>, String>` — the oracle's
  `F[Option[ByteVector]]` — `validate_state_items` gains a checked twin (its total form is a thin
  wrapper, because `casper` calls it with a closure over `RSpaceImporter::get_history_item`), and the
  port's own consumers (`get_history_and_data`, `write_to_disk`, `write_to_disk_dir`) call the checked
  forms. **Falsified first in the witnessing form**, because the fix changes those signatures: with a
  `FailingStore`, `get_nodes` answered an *empty traversal* and `get_root` `None` (asserted as the
  defect), and `get_history_and_data` answered `EmptyHistoryException` (also asserted as the defect);
  both witnesses now assert the refusal, including that the error names the store and *not* the state.
  **The residues are closed by the outright conversion** (2026-09-24, U12). U11 left three: the
  importer's four dropped `put` results, `RSpaceImporter::get_history_item`'s read, and
  `StateManager::is_empty` — each left total because a checked sibling would have had no caller while
  the *trait signatures* stayed total. Once the lead widened the scope, the signatures were converted
  outright and the defaulted siblings **deleted** (a default that silently delegates is where a future
  caller lands by accident — with the signatures converted there was nothing left for them to
  protect): `TrieExporter::{get_nodes,get_history_items,get_data_items}`, `TrieImporter::{set_history_items,
  set_data_items,set_root}`, `RSpaceExporter::get_root` and `RSpaceImporter::get_history_item` all
  return `Result<_, String>` (`Ok(None)`/`Ok(vec![])` is *absence*; `Err` is the store's), and
  `StateManager::is_empty` with them. The write half is the one that mattered most and the LFS sync is
  where it lived: `importer.set_history_items(..)?`/`set_data_items(..)?`/`set_root(..)?` at
  `lfs_tuple_space_requester.rs:270,271,328` and `node_runtime.rs:1993` now make the "chunk done" claim
  conditional on the state actually landing, and `set_root`'s `?` means a refused tag write stops
  before recording a `current-root` for it. Two sites beyond the approved list were *forced* by the
  conversion and are flagged rather than silently taken: `node_running.rs`'s store-items **server**
  (`handle_store_items_request`, production) logs and drops a request whose store cannot be read —
  the same policy it already applies to an over-large `take`, since it has no error reply to send and
  an empty page would be a state claim — and node's `RNodeStateManager::is_empty`, which was one line
  plus the trait definition. **No §6 row**: unlike C52's refusal, this one *conforms* to the oracle —
  the oracle has the channel and the port was not using it.

- **C67 — store errors read as negatives across the peer-sync, proposal and gateway paths** (found
  2026-09-24 by the U14 sweep; the gateway site fixed, the rest triaged site by site). *Numbered C67
  after two collisions: `C64` (the sync's pacing), `C65` (the block store's two fixes) and `C66` (a
  failed LFS sync signalling the node out of syncing) were all coined by the performance walk while this
  entry was being written — a shared register in a four-writer tree needs the number re-read from the
  file *and* the log immediately before the commit, which is what this note is the receipt of.* C63's class one
  layer out again, but the answer is a *negative* rather than an empty page: a `BlockStore`,
  `BlockDagStorage` or block-API read whose failure is flattened into `false` ("not stored", "not
  known"), `None` ("no block") or `0` ("height zero"), so the caller acts on a wrong state claim
  instead of on a failure. **The oracles were read per site, and two of the three families are
  deviations**: `BlockStoreSyntax.getUnsafe` (`block-storage/.../BlockStoreSyntax.scala:33-35`) lifts
  an absent *or errored* read into `BlockStoreInconsistencyError` in `F`, and `Proposer.scala:240-243`
  takes its bonds map through `BlockDagStorage[F].lookupUnsafe`, which does the same — so the port's
  `false`/`None` there is a dropped channel, not a design choice. The gateway has no Scala oracle (it
  is this port's own coordinator, `docs/src/node/shard-invoke.md`), so its site is decided by its own
  submit path, which already refuses a phase it cannot build.

  **Fixed: the gateway's phase anchor.** `gateway/mod.rs`'s `current_height` is `Result<i64, String>`
  now, and `phase_with_term` returns `ShardOutcome::Error` on a failed read — the shape it already used
  for a failed `signed_invoke`, one line below. Before: a head that could not be read anchored the
  deploy at `valid_after = 0`, so the phase deploy was *born expired* (the adjacent comment states it)
  and **nothing reported an error** — the participant simply never saw the leg. An unknown shard is
  still `Ok(0)`: a routing fact, not a read failure. Falsified in the witnessing form:
  `current_height_refuses_a_head_it_cannot_read` replaced a test whose assertion (`0` for an errored
  head) *passed on the defect*; run 2026-09-24 before the change.

  **Triaged, with the reason each is not in this commit.** *Held*: `casper/src/engine/node_running.rs`'s
  seven `contains`/`get` sites and `block_receiver.rs:247`'s `not_validated` — its signature change
  reaches `node_running.rs:563`, which the lead is holding for the performance walk. *With
  `perf-tail`* — now **fixed as C65**: `lfs_block_requester.rs:86` (a dropped `block_store.put` followed
  by `guard.done(&hash)`: the block was recorded as done and never re-requested) and `:128` (a `contains`
  flatten whose empty `Vec<bool>` emptied both `existing` and `missing`, so the walk requested nothing) —
  the two worst sites of this family, repaired there with their own falsifiers. **Fixed: the proposer's
  bonds read.** The closure's body is extracted as `is_active_validator(&dag, &sender) ->
  Result<bool, String>` (the `load_node`/`load_node_from_store` split, applied to a DAG read, so the
  behaviour is testable without a proposer fixture) and the `check_active_validator` closure carries
  `BoxFuture<Result<bool, String>>` into `propose`'s error path — all inside `proposer.rs`, so nothing
  outside it moved. A failed `lookup` became an empty bonds map, and an empty map answers `false` for
  every sender: a node whose DAG could not be read stopped proposing and reported `NotBonded`, with no
  error anywhere. An *absent* block under a height-map key is refused too, because that is the
  inconsistency `lookupUnsafe` raises on. Falsified in the witnessing form:
  `a_dag_read_that_fails_is_not_an_inactive_validator` first asserted `!is_active_validator(..)` for a
  failing DAG and **passed on exactly that** (run 2026-09-24), then flipped to the refusal. The
  extraction is the point as much as the fix: **giving the read its own fallible function is what made
  it testable without a proposer fixture** — the `load_node`/`load_node_from_store` split, applied to a
  DAG read, and the same trick that made `handle_remainder`'s reading cheap in U1. **Fixed: the
  validated-block pump.** `pump_validated_blocks` (`node_runtime.rs`, extracted from the `load_blocks`
  task's body by the same move) now reports a block-store read failure with the hash instead of
  dropping the block from the validate→process path with nothing recorded — the oracle's `getUnsafe`
  lifts an absent *or errored* read into `BlockStoreInconsistencyError` in `F`, and a spawned task has
  no reply to send, so the log line naming the hash is the honest answer. No plumbing was needed: the
  enclosing `wire_block_processing` already takes `log: Arc<dyn Log>`, so this cost **zero call sites**
  (the question the lead asked before the commit). Falsified in the witnessing form too:
  `a_block_that_cannot_be_read_back_is_reported` first asserted `log.messages().is_empty()` against a
  `FailingBlockStore` and **passed on exactly that** (run 2026-09-24), then flipped to one line naming
  the hash and the store's error. **Fixed: the parent-dependency read.** `parents_not_stored` (`block_receiver.rs`,
  extracted from `incoming_blocks`' loop by the same move — no stream harness needed after all) now
  returns `Result<Vec<(BlockHash, bool)>, String>`, and the loop logs with the block and skips it —
  the idiom that loop already uses for `end_stored`. The pre-fix body answered `true` ("not stored")
  for a store it could not read, which only caused a redundant request, *but silently*; a store that
  answers fewer presence bits than keys is refused too. Falsified in the witnessing form:
  `a_store_that_cannot_be_read_is_not_an_unstored_parent` first asserted `vec![(hash, true)]` for a
  `FailingBlockStore` and **passed on exactly that** (run 2026-09-24), then flipped to the refusal.
  **Fixed: the receiver's presence and block reads — the seven `node_running.rs` sites and
  `block_receiver.rs:247`.** Three checked reads now carry them: `block_is_known` (the four `contains`
  sites), `block_by_hash` (the three `get` sites) and `not_validated` (which *delegates* to
  `block_is_known`, so the two cannot drift), each returned as `Result` and each call site deciding
  per its channel: the HasBlockRequest handler skips the answer (an unanswered request is retryable, a
  `false` answer is a state claim) and the rest log with the hash and skip, which is the policy
  `handle_store_items_request` already carries in that file. Falsified in the witnessing form:
  `a_store_that_cannot_be_read_is_not_an_unknown_block` first asserted `false`/`None` for a
  `FailingBlockStore` and **passed on exactly that** (run 2026-09-24), then flipped; `not_validated`'s
  site carried the *same* expression, and `a_store_that_cannot_be_read_is_not_an_unvalidated_block`
  asserts its refusal through its own signature. **The distinction that decides these sites**: the
  flatten often takes the *conservative* direction (an unreadable store marking a parent "not stored",
  or a block "unknown", only causes a redundant fetch), so a reader who checks the direction will call
  the site benign — **the direction is safe; the silence is not**, and the oracle's `F` propagates to a
  caller that logs.
  *Benign, with the reason*: `block_receiver.rs:376` — a read failure takes the *conservative* direction
  ("not stored" → it re-`put`s the block) and the doomed put's error is logged with the block hash two
  lines later, so the operator sees the store failing and no wrong state claim is made; and
  `block_receiver.rs:398` — a read failure marks the parent `not_stored`, which only causes a redundant
  *request* (conservative), though silently: a log line is owed when a stream-level harness makes it
  falsifiable.

  **U12: the write half is witnessed, and the outright conversion is blocked on two production call
  sites.** `an_import_that_cannot_write_is_not_a_successful_import` pins the half that matters most —
  an import into a store that refuses writes reports *success* having restored less state than it was
  given, a wrong-state claim rather than a missing read (four refused writes, `()` returned by all
  three setters, and the pre-fix assertion passes on exactly that) — and the read residue
  (`RSpaceImporter::get_history_item`) and `StateManager::is_empty` are named beside it. Converting the
  traits *outright* (which is what the residue needs: a total body nothing calls is what let the
  flatten survive) requires editing five **production** lines outside this unit's scope —
  `casper/src/engine/lfs_tuple_space_requester.rs`'s apply path (`:256`'s closure over
  `get_history_item`, `:270,271`'s `set_history_items`/`set_data_items`, `:328`'s `set_root` — the LFS
  sync's own import) and `node/src/state/instances.rs:31`'s `RNodeStateManager::is_empty`, whose
  `rspace_state_manager.is_empty()` call is the shared trait's. **All five landed (U12,
  2026-09-24)**: the LFS apply path now calls `set_history_items`/`set_data_items`/`set_root` through
  their `?`, and `RNodeStateManager::is_empty` returns `Result<bool, String>` — so the outright
  conversion is in place, the residue has no total body to flatten into, and the witness has flipped to
  its refusal form.

| Incident | Law | What catches it now |
|---|---|---|
| C9 `(x)` parsed as a one-element tuple | 30, 31, 33 | `Rchain/Surface.lean`'s production table (a tuple is `TupleSingle`/`TupleMultiple`); `rholang/src/parser.rs`'s unit tests |
| C10 `/\` and `\/` lexed swapped | 32 | `spec/conformance/lex.tsv` rows `Conj`/`Disj`, each with a discriminating sample — `node/tests/lean_lex_corpus.rs` |
| C13 printer dropped `bundle` and doubled `|` | 33 | `Rchain/Surface.lean`'s witnesses (`bundle+/-/0/` rows) — the round-trip corpus is `Print.lean`'s, **landed with law 33**: its printer rows are `spec/conformance/parse.tsv`'s second half, consumed by `rholang/tests/lean_parse_corpus.rs` (this cell said "still open" until AUDIT C72 reconciled it) |
| C16 `DeployExecStatus` fields snake_case | 43 | `spec/conformance/envelope.tsv`'s `DeployExecStatus` row (variant *and* field keys) + the served document — `node/tests/lean_envelope_corpus.rs` |
| C17 list/set remainders did not parse | 31 | the `ProcRemainderVar` witness rows and the corpus's remainder cases |
| C18 `lookup` wrapped its reply in `(uri, value)` | 39 | `spec/conformance/protocol.tsv`'s `rho:registry:lookup` row + the doc tie to `spec/API-SCHEMA.md` |
| C19, C20 partial collection patterns never matched | 35, 37 | `flags.tsv` (concreteness includes the remainder) and `match.tsv` (a partial pattern's verdict) — `lean_normalize_corpus.rs`, `lean_match_corpus.rs` |
| C21 a non-first `if` was a no-op | 34 | `rholang/tests/if_par.rs` (five shapes, including `if` inside a receive body) — the Lean value-position model is `Surface.lean`'s `normalize` |
| C22 item 1 a reader consumed its store | 41 | `store.tsv`'s five cases + `casper/tests/genesis_registry.rs`'s `a_read_does_not_destroy_the_inbox` |
| C22 item 2 a wrong-arity capability call | 40 | `silence.tsv` case 10 (`write!(key, value)` against a three-argument `write`) — and the *rule* now has `commPs`, which is what makes the arity a rule clause (C40) |
| C22 item 3 a map remainder treated as concrete | 35 | `flags.tsv`'s remainder rows |
| C24 `[1 ..._]` and `@{..._}` | 35, 31 | `flags.tsv` (C24's own case) + `rholang/src/parser.rs`'s `collection_remainders_parse_for_lists_and_sets_not_only_maps`, and both remainder spellings are pinned by `:1530`'s `a_comma_before_a_remainder_is_the_deviation_the_contracts_use`. The comma-less form is *derivable* (the grammar's singleton list rule carries no separator); the comma form is a law-31 deviation, and its row in the deviation **list as data** is G3's (`Rchain/Parse.lean`), still to land |
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
| C40 law 38's tie was false, and the relation lacked its arity clause | 38, 40 | `commPs` as the rule's arity clause, the false `iff` corrected — and its domain hypothesis later *removed* rather than kept (C45), `allStringChans` deleted with it |
| C41 the diff accumulator could overflow where the merge refuses | 17 | **fixed**: `combining_refuses_a_diff_that_leaves_i64` (`event_log_index.rs`) fails on a `wrapping_add`, and the error reaches the merge through the now-fallible `EventLogIndex::combine`/`branches_are_conflicting`; `Merging.lean`'s `checkedAdd_refuses_overflow`/`mergeRandoms_perm` state the checked half and the call-site canonicalization |
| C42 law 5's linearity is the normalizer's, not the matcher's | 5 | `Match.lean`'s `aggregateUpdates_rejects_double_bind`/`freeMapMerge_overwrites` state the matcher's halves; the enforcing checks are `normalizer.rs:289` (a duplicate inside a *process* pattern) and `:1325` (a duplicate across a *join*'s binds), **measured by breaking each in turn** and now pinned by tests — four refusal shapes and three negative ones in `normalizer.rs`'s test module, which did not exist before 2026-09-24, when the invariant's only evidence was a devnet probe. `:111`/`:590` are listed in the finding but exercised by no reachable shape; `spec/conformance/match.tsv` documents the matcher in isolation |
| C43 the merge's associativity was untested, under a name that says otherwise | 9 | `Merge.lean`'s `mergeChanges_assoc` (proved) **and** `property_tests.rs`'s `law9_state_change_combine_is_associative`, over arbitrary state changes including the join map; the misnamed `state_change.rs` test now says what it asserts |
| C44 the matcher had no clause for a tuple, and the port has one | 5, 37 | `match.tsv` cases 15/16 (`@(1, 2)` against `(1, 2)` and against `(1, 2, 3)`) + `lean_match_corpus.rs`; the `ETuple` arm in `Match.lean`, and `modelledPar` on both sides of `concrete_matches_iff_eq`, whose old statement is refuted by `arithmetic_pattern_refutes_the_unrestricted_tie` |
| C45 the search claimed a step for a join, and the rule fixed the counts the port computes differently | 38, 40 | `silence.tsv` case 13 (a join with one channel filled declares `false`, and the node agrees) + `lean_silence_corpus.rs`; the search's single-bind requirement, the constructors' `freeCount`/`bindCount`/channel parameters, and `takesStep_sound` — three extraction lemmas and `exists_redex_split` |
| C46 a joining validator could not index the genesis: its sidecar regeneration replayed block #0 without the genesis vaults | 11 | **fixed (2026-09-24)**: `is_genesis_pre_state` conditions the vault re-install at both genesis-replay call sites, `genesis_descriptors_from_config` reads the network's genesis files on any node (`node_runtime.rs`), `tools/devnet.sh` gives validators 1..n−1 the files — and `interpreter_util.rs`'s `is_genesis_pre_state_is_true_only_for_the_empty_state` pins the condition that keeps an unconditional re-install from clobbering post-genesis balances. The row was missing from this table until then, which is its own small finding: law 11 is the law that covers it — a replay that does not reproduce the record — and the table's promise is that every incident names one. **Verified end to end (2026-09-24)**, once C55 unblocked the devnet: on a fresh volume, `tools/devnet.sh up --validators 3` reaches a chain and both joining validators index block #0 and then track it (bootstrap 37, validator-1 37, validator-2 36) with zero `regenerated mergeable channels` and zero refusals in their logs |
| C47 the matcher's fuel was short: the measure counted an empty `Par` as zero nodes | 5, 37 | `match.tsv` case 18 (`@Set(1, ..._)` against `Set(Nil × 6, 1)`) + `lean_match_corpus.rs`; `the_walk_past_empty_pars_is_paid_for`, and `parNodes`'s doc comment carrying the counterexample |
| C52 a peer's `BindPattern` could carry a negative `free_count`, which silently changed what the receive bound | 5, 37 | **fixed (2026-09-24)**: `bind_pattern_from_proto` validates the count like its two siblings already did, so a message that would have applied the continuation with a wrong number of bindings is refused where it arrives. Falsified first — restoring the pass-through fails the new assertion in `models/src/wire.rs`'s `the_runtime_payloads_round_trip`. The same three lines' `unwrap_or_default()` — a free level the case's `free_count` declares but the matcher's free map does not carry — is **closed for the half that can refuse** (U1 site 1, 2026-09-24): `resolve_match` (`reduce.rs:2009`) raises `BugFoundError` where the Scala defaults (`Reduce.scala:352`, `freeMap.getOrElse(e, Par())`), so the continuation is never handed a variable bound to nothing. Falsified first, at the time: the test that then asserted the *refusal* (named
`resolve_match_refuses_a_free_count_the_pattern_does_not_bind`, since re-purposed as
`resolve_match_pads_a_level_the_pattern_does_not_bind`) failed against the old `unwrap_or_default()`. **That refusal was reverted by measurement** (U17, C71): `legacy_contracts` reduces
`legacy/.../matching-parallel-processes.rho` — `for (@{x | y} <- @Nil)` — and the matcher is *greedy by
design*, binding only level 0 (`x` gets the whole par), so a level the pattern names and the match did not
fill is the ordinary case, exactly as the program's own expected output (`@2!(Nil)`) documents.
`resolve_match` pads again, like `Reduce.scala:352`, and the §6 row this claim carried was **deleted**,
not amended: the port conforms. The claim above that the state was "unreachable" was an inference, and
the corpus refuted it — kept here as a lesson rather than hidden. Its sibling `RhoMatch::get` used to **keep** the Scala's default, because `Match::get` returned `Option` — whose only refusal is `None`, "this datum does not match", a *worse* lie and one the Scala does not tell. **That channel is built now** (2026-09-24, U10): the oracle's `Match.get` is `F[Option[A]]` (`rspace/.../Match.scala:11`), so the port's trait returns `Result<Option<A>, RSpaceError>` — one new variant, `MatcherFailed(String)` — and the search functions (`space_matcher::{find_matching_data_candidate, extract_data_candidates, extract_first_match}`) are fallible with it, the Scala's being in `F` too. `RhoMatch::get` therefore **refuses** a level its matcher did not bind instead of padding it with the empty par: the same rule `resolve_match` applies, now through the trait. Falsified in its pre-fix form, which *witnessed* the defect rather than failing, because the fix changes the method's type: a `BindPattern` declaring `free_count: 3` with one bound level produced `Some(..)` with `pars.len() == 3` (two empty pars — the defect, and the pre-fix assertion passed on it), where it is now `Err(MatcherFailed("the pattern declares free level 1 of 3 but the matcher bound no value for it"))`. A second flatten went with it: `fold_match(..).ok()?` turned a matcher `BugFoundError` into "no match", so a datum that failed for a *bug's* reason silently stayed in the space — it maps to `MatcherFailed` too. **Stricter than the oracle**, which pads (`toSeq`'s `case None => Par.defaultInstance`), and registered in §6; unreachable from a well-formed term, since the normalizer computes the count from the same pattern. The appendix's *other* candidate site, the matcher's `handle_remainder`, was measured and is **not** a partiality spot: the absent level there is the accumulator's zero (`SpatialMatcher.scala:258`), and refusing it fails five of that module's remainder tests — `handle_remainder_starts_an_absent_level_from_the_merge_identity` pins the reading (U1 site 2). **`FreeCount::from_nonneg`'s `debug_assert!` is closed** (2026-09-24, U1 sites 4 and 5), and the closure has two halves. *(a) The carrier.* `New.bind_count`/`Receive.bind_count` are `FreeCount`s and a negative count is refused at both proto ingress points — §6's row; the two counts the normalizer derives go through `checked_level_count`, which refuses rather than clamps, so the `.max(0)` pair in `well_scoped_*` has nothing left to defend against and is gone. `from_nonneg` itself is *deleted*: the carrier's total entry is `from_len(usize)`, so the sign is not writable, and the sum of validated counts uses the carrier's own `Add`. *(b) Both directions of that claim, run.* With the carrier, a probe that builds a count from a negative **does not compile** — `FreeCount::from_len(-1)` is `error[E0600]: cannot apply unary operator - to type usize` and `FreeCount(-1)` is `error[E0603]: tuple struct constructor is private` (captured 2026-09-24). With the old shape restored inside the module (a total `i32` constructor with the `debug_assert!`), the same probe compiles and: in a **debug** test the assert fires (`free-count must be non-negative`, FAILED), and in a **release** test the assertion passes — `i32::from(probe_old_shape(-1)) == -1` — i.e. the invalid count was carried silently in exactly the builds that matter. That is the hole; (a) is what closes it. *(c) What is deliberately left.* `storage_printer`'s two sites read a *stored* `i32` field: `to_receive` now returns `Option<Par>` and a negative field **refuses the row** through the module's own fixed diagnostic message (`MALFORMED_PATTERN`), rather than clamping it to zero (a `for` binding nothing is a different term) or recovering a derivation — the port's own count convention cannot reproduce the value exactly, and the measurement is pinned: for `for (@[x, ...rest] <- c)` the normalizer writes 2 and the walk sees 1 (`the_walk_ignores_a_collection_remainder_where_the_normalizer_counts_it`). `BindPattern.free_count` therefore **stays an `i32`**, and stays the named durable fix — casper, node and the bench construct that struct with literals, so the carrier cannot move onto the field from this crate; both casper patterns are single free variables (`runtime_manager.rs:742`, `runtime_replay.rs:552`), so the convention gap above is latent there rather than live |
| C63 the state-export path flattened store errors into "no state", and validation into "corruption" | 10 | **fixed (2026-09-24)**: the exporter/importer traits gain checked siblings (`try_get_nodes`, `try_get_history_items`, `try_get_data_items`, `try_get_root`) whose defaults delegate to the total forms, so `casper`'s implementations are untouched, while the store-backed ones override them with the real reads; `traverse_history`/`NodeDataReader` take `Result<Option<..>, String>` (the oracle's `F[Option[ByteVector]]`); `validate_state_items` gains a checked twin, and `get_history_and_data`/`write_to_disk`/`write_to_disk_dir` call the checked forms. Falsified first in the witnessing form: `a_store_error_is_not_an_empty_traversal` (pre-fix: an empty traversal and a `None` root, asserted as the defect) and `a_store_error_is_not_an_empty_history` (pre-fix: `EmptyHistoryException`), both now asserting that the store's own error comes back. **Residue closed (U12, 2026-09-24)**: the traits were converted *outright* — `TrieExporter`'s three reads, `TrieImporter`'s three setters, `RSpaceExporter::get_root`, `RSpaceImporter::get_history_item` and `StateManager::is_empty` all return `Result<_, String>` — and the U11 defaulted siblings deleted, so no total body survives for a caller to land in by accident. The write half was the priority and the LFS sync is where it lived: `set_history_items(..)?`/`set_data_items(..)?`/`set_root(..)?` at `lfs_tuple_space_requester.rs:270,271,328` and `node_runtime.rs:1993` make the "chunk done" claim conditional on the state landing. Witnessed first and flipped: `an_import_that_cannot_write_is_not_a_successful_import` now asserts three refusals (three attempts, not four — `set_root` stops at its first), and `a_store_error_is_not_an_empty_traversal`/`..._history` assert the store's own error. Two sites beyond the approved list were *forced* by the conversion and reported: `node_running.rs`'s store-items server logs-and-drops an unreadable page (its own over-large-`take` policy — a §6 deviation, since the oracle's page build is in `F` and fails the request; the requester times out rather than receiving a wrong state claim), and node's `RNodeStateManager::is_empty` gained one line. No §6 row — this *conforms* to the oracle, which had the channel. **The same class is live on the block store, the DAG and the block API — see C67** |
| C67 store errors read as negatives across the peer-sync, proposal and gateway paths | 10 | **gateway fixed (2026-09-24)**: `current_height` returns `Result<i64, String>` and `phase_with_term` refuses the phase (`ShardOutcome::Error`) instead of anchoring it at `valid_after = 0`, where it was born expired with nothing reporting an error; falsified first by `current_height_refuses_a_head_it_cannot_read`, which replaced a test whose assertion passed on the defect. **The oracles were read per site**: `BlockStoreSyntax.getUnsafe` (`:33-35`) and `Proposer.scala:240-243`'s `lookupUnsafe` both lift an errored read into an error in `F`, so the port's `false`/`None` there is a dropped channel — which makes the *owed* sites deviations, not decisions. **Fixed: the proposer's bonds read** (`proposer.rs`, `is_active_validator` extracted so no proposer fixture was needed — *give the read its own fallible function and the test stops needing the world*) **and the validated-block pump** (`node_runtime.rs`, same extraction, zero call sites: `wire_block_processing` already took the `log`), both falsified first in the witnessing form. **Fixed: the parent-dependency read** (`block_receiver.rs`, `parents_not_stored` extracted so no stream harness was needed — the same move a third time), falsified first in the witnessing form. **Fixed: the whole site list is closed.** The gateway (`current_height` carries the failure), the proposer (`is_active_validator`), the validated-block pump (`pump_validated_blocks`), the parent-dependency read (`parents_not_stored`), and the receiver's three block reads (`block_is_known`, `block_by_hash`, `not_validated` — which delegates to the first, so the two presence reads cannot drift) plus their seven call sites, each deciding per its channel: skip the answer where a reply exists, log with the hash and skip where none does — the policy that file already carries. Each with a witnessing falsifier run before the fix. **The distinction that decides the conservative sites**: the flatten takes the safe *direction* (a redundant fetch) but does it silently — **the direction is safe; the silence is not**. **Fixed as C65** (the performance walk): `lfs_block_requester.rs:86,128`, the two worst. **Benign, with the reason**: `block_receiver.rs:376` (the conservative direction, and the doomed put is logged with the hash) |
| C71 the matcher's padding is reachable from a well-formed term, and U10's refusal broke a corpus program | 5, 37 | **fixed (2026-09-24)**: `RhoMatch::get` and `resolve_match` pad a declared-but-unfilled level with the empty par again, as the oracle does (`storage/package.scala:22-29`); the §6 row U10 added is **deleted** because the port conforms. Measured, not argued: for `for (@{x | y} <- @Nil)` the matcher returns bound levels `[0]` only (greedy by design), and `matching-parallel-processes.rho`'s own header documents the padding as its expected output (`@2!(Nil)`). Falsified by the corpus (`legacy_contracts` fails on the restored refusal) and pinned by two unit tests that now assert the oracle's semantics. **The lesson**: a change to the matcher's contract must run `legacy_contracts` and `execution` before landing — the scoped unit suites cannot see this class |
| C61 a peer-supplied resume prefix of 128 bytes entered the segment invariant by one byte | 10 | **fixed (2026-09-24)**: `create_last_prefix` routes the prefix through the checked constructor (`KeySegment::try_from`) instead of a hand-written `> 128` bound, so a size of 128 is refused rather than built and then silently truncated to zero by the radix encoder's 7-bit size field. Falsified first: `a_128_byte_resume_prefix_is_refused` fails against the old bound. The three other decode sites the appendix named are measured in range by construction and carry comments saying why; `head()`/`tail()`'s empty-segment panic keeps its existing pin and its reachability argument (`trimming_an_empty_key_panics`) **Closed (2026-09-24, `64ea1350f`), and the closure is a *shape* rather than a constant.** `KeySegment::new` is **deleted**; the type has exactly one checked way in (`TryFrom`, `:21`) plus a **private** `from_slice_of_valid` (`:37`) — and that privacy is the difference in kind from the deleted public `new`. **Growth is fallible and shrinking stays total**: `concat`/`append` return `Result<KeySegment, String>` (`:79`, `:88`) because the Scala's `++`/`:+` reach `apply`'s `require(bv.size <= 127)` and throw, while `empty`/`tail`/`common_prefix` (`:43`, `:63`, `:103`) are total **by monotonicity** — a suffix or prefix of a valid segment is valid. **And the defect was live, not latent**: `radix_tree::save_node_and_create_item` concatenates prefixes decoded from a node (each ≤ 127 by the 7-bit mask, `radix_tree.rs:65,70`) plus a one-byte index, reaching **up to 255**, and `export::init_node_path` accumulates along a path seeded from the **peer-supplied** `last_prefix` (`export.rs:67`) that C61 itself exercised — so the port was minting over-long segments and writing them with a truncated 7-bit size, which is this finding's desync on the *write* path. **The `head()` half is closed and the `tail()` half is left with its reasons** (`7ec9e16d5`, 2026-09-25, H2e). `head()` now **states its domain and refuses by name**: it reads through `head_option()` (`rspace/src/history/key_segment.rs:76-78` — the method's **first caller**, it had none) with an `expect` naming the domain and its enforcers (`:63-66`), so an empty segment yields "a segment's head is read only where the segment is known non-empty" instead of an index error that says nothing about whose domain was violated. The falsifier is stated as a run: `head_on_an_empty_segment_refuses_by_name` (`:188`, `#[should_panic(expected = "known non-empty")]`) **FAILED** against the old `value[0]` with `panic did not contain expected string / expected substring: "known non-empty"`, and `head_option_and_head_agree` (`:195`) pins the delegation so it cannot drift. **And all 13 call sites were measured non-empty first**, which makes this a *shape* fix rather than a live bug being patched: `export::init_node_path`'s `rest_prefix.is_empty()` early return, `create_node_from_item`'s `assert!(!prefix.is_empty())`, `update`/`delete`'s remainders, and `make_actions`' 33-byte action keys. **The `tail()` half is deliberately still open, named rather than left to inference**: `tail()` keeps its slice panic (`:72-74`), there is no `tail_option()`, and adding one ripples `HistoryAction::trim` → `make_actions` → the trie walk — the deferred "option 7" — with no site reachably empty, so half of that rewrite would be churn. Its pin, `history_action.rs`'s `trimming_an_empty_key_panics` (`:97`, `#[should_panic(expected = "range start index 1 out of range")]`), is unchanged. So the deferred "option 7" is **partly** done, and which part is now stated |
| C62 the sync path copied the page it served 3×, and `/metrics` had no wire | 10 | **fixed (2026-09-24)**: `chunk_it` borrows the packet instead of cloning it twice and owns a buffer only when LZ4 compresses — the chunk proto's `Vec<u8>` is the one copy the wire requires — pinned by `comm/src/transport/chunker.rs`'s `chunking_a_page_does_not_copy_it_whole`, whose instrument is time *relative to one explicit copy of the same payload in the same process* (machine-speed invariant) and which was falsified in this tree at 3.84–3.96× against a 2.0 bound (1.30–1.33× for the borrow); and the DAG now publishes `messages`/`seen_entries`/`fringe_states`/`index_entries`/`logical_bytes` into the node's `MetricsRegistry`, whose snapshot `/metrics` renders before scraping (`node/src/web/http.rs`), where nothing in production had ever called `report_period_snapshot` — the route served `EMPTY_SCRAPE_DATA` forever. Pinned by `the_metrics_route_serves_the_registrys_own_numbers` and `the_dag_publishes_its_own_gauges`, both falsified here (removing the publish restores the placeholder / leaves the gauges unset). **Measured, deliberately not fixed**: `handle_store_items_request` is awaited inline in the shard's dispatch loop — an LFS page (750 × 4 KiB) holds it ~50 ms (18 + 32 ms of chunking) and the cap's largest page (10,000 nodes) ~580 ms — and the page's node-count cap is now joined by a byte cap (32 MiB, refused rather than truncated — C73) |
| C64 a fresh validator cannot catch up: 1.5 blocks/minute, ~70 hours for a 6,300-block chain | 10 | **measured, no pacing defect**: the requester's loop and `LfsState` mirror the Scala's `requestStream`/`ST` element for element, and isolated against a transport that answers every request a 6-block walk with the production 30 s `requestTimeout` finishes in **3.85 ms** — `casper/src/engine/lfs_block_requester.rs`'s `the_walk_advances_on_responses_not_on_the_idle_timeout`, bound 5 s, *below one idle timeout*, so a walk advancing on the resend cannot pass. The length is the **registered §6 deviation** (the port walks the full ancestry to genesis, the Scala stops at `lowerBound`, ~50 blocks below the fringe): one generation per block on a chain, 6,300 round trips against ~50. The **~25 s per generation is not the requester's** — the server answered **63/63** requests with zero drops and the validators received exactly those 63 blocks — it is the syncing node's shared message loop with the concurrent state sync; the discriminating experiment is recorded with it. Consequence: a fresh validator is hours-to-days behind on a long chain, and the chain length is what makes it so |
| C65 the block store's side of LFS sync swallowed a failed write and a failed read, where the oracle propagates both | 10 | **fixed (2026-09-24)**: `save_block` marked a block `done` even when `put` failed (and discarded the error), so a block that was never persisted was also never re-requested — a permanently missing block in a "synced" state — and `request_next`'s `contains(..).unwrap_or_default()` made a store error an *empty* work set, so the walk requested nothing until the idle resend. Both now propagate: the walk is fallible end to end (`Result<St, String>`, reaching NodeSyncing's existing `Lfs state sync failed` path), matching the oracle's `F`-typed `saveBlock`/`filterA`. Falsified in the witnessing form: `a_failed_block_write_fails_the_walk_instead_of_marking_the_block_done` and `a_failed_store_read_fails_the_walk_instead_of_requesting_nothing`, each failing with its defect restored — the second reproducing C64's stall signature exactly, which is why C64's cadence had to rule it out (63 requests, all answered ⇒ `contains` was succeeding) |
| C68 a failed LFS sync still signalled the node out of syncing, so it could run on an incomplete DAG | 10 | **fixed (2026-09-24)**: the sync task notified `finished` — the only signal `node_launch` waits on to enter `NodeRunning` — *after* the outcome match, on failure as well as success, where the oracle sequences `finished.complete(())` after `parJoinUnbounded.compile.drain` and stores the approved block only after the state ("to restart requesting if interrupted with incomplete state"). Found while fixing C65, whose fix routes block-store failures into exactly this path, so the two had to land together. Now `notify_when_restored` signals on `Ok` only, falsified both directions by `a_failed_sync_does_not_signal_the_node_out_of_syncing` (with the unconditional notify restored the `Err` half fails). **Closed end to end (U16)**: `a_failed_sync_leaves_the_node_in_syncing` drives the real handler with a failing store and a silent transport, proves the attempt failed via the log, and asserts the handle was not notified — and its own first version passed with the defect restored — the waiter was created *after* the failure, and `Notify::notified()` captures the `notify_waiters` counter **at creation**, so the observation point is the thing being compared (tokio 1.52.3 `notify.rs:566-575`). Hardened the same day: the fixture is awaited with a 30 s deadline and the notification is read with one `biased` poll, so neither bound depends on the scheduler |
| C53 a store error read as an absent radix node | 10 | **fixed at the read boundary (2026-09-24)**: `load_node_from_store` propagates the store error instead of `.ok()`-ing it into the same `None` a missing node produces — the Scala keeps it in `F` (`RadixTree.scala:569`). Falsified first: restoring `.ok()` fails `radix_tree`'s `a_store_error_is_not_a_missing_node`, which pins both halves (the error surfaces; an absent node is still `None`). **The flattening above it is closed too** (2026-09-24, C53's own unit). `load_node` returns `Result<Node, String>` — the port's counterpart of the oracle's `F[Node]` — so the `no_assert` arm propagates a store error instead of standing the empty trie in for a root that could not be read, and the error travels the whole way up: `RadixTreeImpl::{read, update, delete, make_actions, construct_node_from_item}` → `History::{read, reset}` → `RadixHistory::{new, read, reset}` → `HistoryRepository::{reset, get_history_reader, get_native_reader}` → `create_play_rspace` → the `ISpace::reset` impls, which already returned `Result<(), String>` and now have something to report. **`no_assert` keeps the oracle's meaning for *absence***: `load_node` still asserts-or-empties for a *missing* node (`RadixTree.scala:586-598`), because the fix is to separate unreadability from absence, not to turn both into errors. The one boundary the port cannot fail *at*: `HistoryRepository::get_history_reader`/`get_native_reader` return `Arc<dyn HistoryReader>` and are called from `casper`, `node` and `rspace-bench`, so a load failure is carried *inside* the reader (`RSpaceHistoryReaderImpl::unreadable`) and answered on the first read, where the reader trait is already fallible — deferred, never dropped. Falsified first: `a_store_error_is_not_an_empty_root`, in its pre-fix form, failed — a store that is down yielded `empty_node()`, `left` and `right` the same 256-`Empty` array — and it now pins all three shapes (the `load_node` error, the still-not-an-error absence, and `RadixHistory::new` refusing). `cargo check --workspace --all-targets` is clean, so no consumer was left half-converted. **The same class was still live one layer out — in the state-export path's stores, where the consequence is under-reported state rather than a misread node: see C63** |
| C49 the replay property test fails on its own recording (~3 runs in 10) | 11 | **closed, and it was not the code**: the fixture rigged the replay with the play's *post-play* root, so the "replay" began from a half-finished tuple space — `rspace/src/property_tests.rs`'s `law11_a_replayed_script_matches_its_recording`, now taking the checkpoint before the script, passes over 4000 cases where it failed deterministically at `PROPTEST_CASES=1`. The seed stays as the pinned input; `check_replay_data` was never at fault |
| C50 the matcher's fuel was short again: the measure had no `etuple` case, so a tuple's contents were charged to nothing | 5, 37 | `match.tsv` case 20 (`@((1, 2), (3, 4))` against itself) + `lean_match_corpus.rs`; `a_nested_tuple_is_paid_for`, `a_tuple_pays_for_its_own_contents`, and `parNodesExpr`'s doc comment carrying the counterexample. While the defect stood it also **refuted** the axiom `concrete_matches_iff_eq` |
| C51 the tie's domain admitted a two-expression `Par`, which no clause accepts — so the tie was false | 5, 37 | the axiom `concrete_matches_iff_eq` is **deleted**; `a_two_expression_pattern_refutes_the_modelled_tie` is the counterexample, and rows 5/37 owe the tie for a **singleton** pattern instead |
| C48 the spec over-claimed a match: the searcher was wired into the list and tuple arms | 5, 37 | `match.tsv` case 19 (`@[1, ..._]` against `[Nil, 1]`) + `lean_match_corpus.rs`; `a_list_pattern_cannot_skip_a_target_element`, and the split into `matchListPos` (lists, tuples) / `matchListPar` (sets, maps) |
| C55 the devnet bootstrap never starts: restoring a stored chain folded the message state per block, at Θ(N³) | 15 | **fixed (2026-09-24)**: `DagMessageState::insert_msg_mut` / `insert_msg_without_latest_mut` extend the state in place (the persistent forms are now one clone plus that same insert, so the monotonicity and subset rules still live in one place), and `BlockDagKeyValueStorage::create` uses the in-place form. Falsified first — `restoring_a_stored_chain_is_not_cubic_in_the_message_state` bounds the fold over a synthetic 1,200-block chain; measured 5.1 s in place against 108.6 s copying at N=1500. End to end, a 5,881-block restart serves in 23 s where it previously never served at all. The named-volume path in `tools/devnet.sh` is what hid it (first run fresh, every later run a rebuild) and is now the reason `up` must not leave a state whose second run differs from its first |
| C56 the per-block merge scope copied every message it looked at — Θ(N²) in copies per block | 15 | **fixed (2026-09-24)**: `message_map::between` takes ids and returns ids (`&BTreeSet<M> -> BTreeSet<M>`), so nothing clones a `Message` (and its `seen` set) to answer `upper.seen \ lower.seen`; the call site keeps its three "not in dag" errors by checking membership. Measured, isolated, on a 5,855-block chain: **0.78 GiB per block → ~4 MB per block, plateauing**, CPU a pinned 100% → 40%. Pinned by `between_is_the_id_set_difference_restricted_to_the_map`. **Owed**: the steady floor is the Θ(N²) `seen` *residency*, H6's accepted-faithful residual — **measured on the running node at 556 MB of the DAG's own `logical_bytes` (5,885 blocks, `/metrics`), in a 1.18 GiB resident process**, which advances ~1.3 MB per block. (The ~9.8 GiB this cell used to quote was the pre-Stage-1/3/4/5 tree, where the DAG was copied per read and per insert; that figure is retired with the copies.) **The numbers are §19's** — `seen_entries = 17,319,555` (= N(N+1)/2 exactly, read off `/metrics`) and `logical_bytes = 556 MB` — and **the design this row's predecessor promised is written here rather than promised**, because the sentence that promised it ("recorded in C56's §20 row") pointed at §20 while the design lived in one §5 sentence and the numbers in §19: a broken cross-reference, of the class check 14 now reads. The parked, value-preserving design is **a dense bitset over a hash→index table** (ρ ≈ N²/8 bytes against the recorded Σ|seen| × 32 B, a 32× reduction), and it preserves the value because law 15 pins `seen`'s *value* — `seenOf js id = (js.map (·.seen)).join ++ [id]` (`Rchain/Casper/Fringe.lean:90`) — and not its representation; what it does not do is shrink the *number* of entries, so a pass taking it must re-measure `seen_entries` rather than expect it to move. The *copies* the tail still had are gone (Stage 5, §20): `fringe_states` is keyed by the store's own hash, the merge indexes rejections for the final scope only, the index is `Arc`-shared with the representation, and the sync-path chunker borrows the page instead of copying it 3× The **~0.86 GiB/block this cell called *unattributed* was the DAG being copied** on the per-block and per-request paths, attributed and fixed in `494336e70` (§20 below: `get_representation` by value, `insert`'s per-block clone, `Message.seen`). **The serving term is now measured on the fixed tree** — two fresh validators pulling from a node on the 5,844-block artifact (extended to 6,339 by the run): the server grew **115 MB while serving 263 blocks ≈ 0.44 MB per block** (window peak 0.71 MB/block), against the retired ~880 MB/block, with 63 of 63 block requests answered and zero `History items are corrupted` / `Validate received state items` on either side. It is *not* the peer reaching the same height, and that is recorded with its reason in §20 |
| C69 the height-monotone reading of law 15 is false of **both** implementations, and the port's termination guard is unregistered | 14b, 15 | **measured rather than asserted** (2026-09-24, G7). The *unqualified* claim — every message of `prev` at or below every message of the published fringe — is false, because the layer is built from the **justifications** and need not cover the senders `prev` covers: `prev = {m3 (sender 0, height 3)}` with a next layer `{q2 (sender 1, height 2)}` publishes below it, and the gate cannot see it because what `calculate_fringe` reads is the **support map**, not the heights (`block-storage/src/dag/finalizer.rs:165-184`). **The oracle is the same shape**: `Finalizer.scala:144-178`'s `calculateFinalization` ends in `LazyList.unfold(parentFringe)(nextFringe(_).map(nf => (nf, nf))).lastOption` — the gate is the support map and there is **no fringe comparison of any kind** — so the port is faithful here and this is an upstream design property, the `check_min_messages` disposition, not a port gap. What *is* a deviation, and is now in §6: the port's own `if nf == current { break }` (`finalizer.rs:225-226`), whose comment says why — "a non-advancing fringe would loop forever" — and which the oracle does not have. **What the heights are used for** is a *recency key*, not an order requirement: `fringe_height` and `latest_fringe` (`block-storage/src/dag/message_map.rs:54,80`) pick the highest-max-height fringe among *contemporaneous* candidates, which needs "later ⇒ higher" only as a consistency heuristic. The **per-sender** half is provable and the port's walk is why: `self_parents` filters by `!finalized.contains(x)` and never traverses *through* an excluded message, so a min message is the previous sentinel's direct successor. Law 15's statement narrows to that, with the cross-sender refutation kept as a `decide`d witness | **Second half settled (2026-09-24)**: the §6 row's "a longer cycle is unimpeded — nothing has measured one" is vacuous rather than unmeasured — the walk is bounded by the non-finalized chain length (the previous fringe is the cutoff, so a sender's minimum message moves strictly in one direction along a finite acyclic chain), pinned by `the_fringe_walk_is_bounded_by_the_non_finalized_chain_length` (an 8-layer fork: 6 steps against a 25-message bound, assertion inside the loop so a cycle fails rather than hangs). The `nf == current` guard is the fixed point's belt, not the termination argument |
| C70 the `sorry` ratchet could not see a nested comment, and nothing but this scan saw an `opaque`/`partial`/`extern`/`unsafe`/`@[implemented_by]` assumption | — **harness** | no law: the instrument, not the term. `Laws.lean`'s accounting is exact equality over `axiom` *declarations*, so an `opaque` definition is neither an axiom nor a `sorry` and passed every other step of `tools/check-lean-conformance.sh` — the token set now refuses all five, so an assumption arriving is refused rather than discovered. The stripping had to be repaired first: single-level block tracking closed Lean's *nested* comments at the first `-/` (invisible for `sorry`/`admit`, 23 prose hits once `partial`/`opaque` were added) and string literals were scanned as code, where the register keeps its own row prose (11 more). Both fixed (nesting depth, strings with escapes, a `'"'` char literal must not open a string) and the tree scans clean. Falsified at scan level with the script's own extracted `awk` over probe files: five tokens fire, `external` does not, prose/string/nested cases fire on nothing, the original `sorry` ratchet does. The full gate is owed: its first step is `lake build` and the Lean slot is the lead's |
| C72 four register cells, three prose paragraphs and two citations called something owed while the tree held it | — **records** (no law) | `spec/TEST-COVERAGE.md`'s law matrix is checked by neither machine check — the register audit reads `.tsv` counts, citations and test names, the Lean gate reads the emitted registers, and a *prose* cell is read by nothing: law 38's cell said `takesStep_iff_reduces` was owed (its row: proved, a theorem), law 42's said `decode_encode` was owed (its row: the axiom is gone), C13's §20 row said the round trip was "still open" (law 33's printer rows are `parse.tsv`'s second half), and an aggregate row contradicted the four rows above it in the same table. Plus `AUDIT.md`'s C53 "Owed: an error channel" and its U12 "blocked on two production call sites" (both landed), `RUST-VS-SCALA.md`'s "30 element-comparator axioms" (law 1b: no axioms, from twelve — the residual is empty, and `Sort.lean` declares none), two citations to `casper/src/main/resources/casper.tla` (it is under `legacy/`), and `faultTolerance` read as pending (legacy-only; the port's checklist mirrors that suite, `tools/run-integration-tests.sh:34`). **Why nothing caught it**: a record is checked where it is machine-readable and unchecked where it is prose — so prose drifts at the rate the tree moves. The matrix now says which of its cells are checked |
| C73 a store-items page had a cap on its count and none on its bytes | 10 | **fixed (2026-09-24)**: `MAX_STORE_ITEMS_TAKE` bounded the nodes a request names, nothing bounded what they carry — the maximal `take` with 4 KiB items is ~41 MB of the responder's memory per request, and a value's size is the chain's choice, not the request's. `MAX_STORE_ITEMS_BYTES = 32 MiB` (≈10× the largest legitimate page) now **refuses** the page: a truncated one is a wrong state claim (the requester recomputes it, `validate_state_items`), so with no error reply the responder's choices are a drop or a lie — and it already drops for an unreadable store (C63) and an over-large `take`. Registered in §6: the oracle has no cap. Falsified both directions in one test — a ~40 MB page is dropped unstreamed, the largest legitimate page is still served — and the check runs before the response exists, so an oversized page is neither serialised nor sent |
| C74 the name-shape vocabulary is a convention, not a predicate | — **boundary** (no law) | `spec/STYLE.md`'s six shapes cannot be a check over the register, and the measurement is the falsifier: **63 of 126** `witness` entries match a shape and 63 are the model's own declarations (`joinKey_perm`, `mergeChanges_assoc`, …), so a predicate over the field fails on 63 legitimate rows — and renaming them would rename the mathematics. The `falsifiable` prose mentions 275 names, 232 of them declarations under discussion. The *rule* is checked and holds (**0 of 49** proved rows lack both a witness and a corpus), and STYLE.md now states the scope with these numbers, so nobody mechanises the table later |\n
| C75 the gate could not see a Lean module at the top of `spec/` — a `sorry` there was invisible to the ratchet | — **harness** | both file-scoped steps derived their scope from `Rchain/`: `expected` from `find Rchain …` and the token scan from `spec/Rchain.lean` + `spec/Rchain/**`. A top-level module escapes the completeness check, the token scan *and* `lake build` (nothing imports it), so a `sorry` there was invisible three times over. Falsified before the fix with two probes — `spec/ProbeSorry.lean` (`sorry`) and `spec/ProbeOpaque.lean` (`opaque def`) — over the script's own commands: the scan reported 0 hits and `expected`/`actual` compared equal with both present. Fixed by widening both scopes to every `.lean` under `spec/` except `.lake/` (5,668 generated files, which is why the old scope looked reasonable), with the policy stated in the check: library root, imported module, or declared `lean_exe` root — no third kind. Falsified after: both probes fire the scan and fail completeness; removed, both green. The edit also shifted law 30's line citation, and **that was attributed here to the wrong commit**: C75's own mapping was at `:209` against the cited `:207`, in-window, so its arithmetic was true of its own commit — the citation was moved `209 → 222` by `94dfd3c68`'s 13-line stack block, and C76 now records the corrected account: an audit that failed in CI and went unread, not a missing check |
| C76 the register audit failed in CI and went unread for six minutes — an unread check, not a missing one | — **harness** | The breakage: `94dfd3c68` (*"the build needs 64 MB of stack, measured"*, a peer's fix) added a 13-line stack block at `tools/check-lean-conformance.sh:36`, moving law 30's `lean_parse_corpus` mapping `209 → 222` — outside the audit's ±8-line window of the cited 207 — without measuring the line impact on that citation. **C75 is not implicated**: its own mapping was at `:209`, in-window against the cited `:207`, so its message's arithmetic was true of its own commit (`git show f0d2bdadf:tools/check-lean-conformance.sh` → 209). **And the tree could tell**: CI's `test & coverage` job runs `tools/audit-test-register.sh`, and its log for `94dfd3c68` ends `FAIL law 30: \`tools/check-lean-conformance.sh:207-207\` holds no identifier the row names` — it **fired at 21:59Z and failed publicly and immediately**. So the gap was not a missing check; **the gap was that nobody read it**. Disposition: attention (CI already fails the build on the audit), not new code — plus the durable fix landed since: law 30's citation is the **symbol form** (`tools/check-lean-conformance.sh:lean_parse_corpus`), which a line insertion cannot shift, and `5b1ffe8f1` makes that form resolvable and checked. **How this entry was first written is the same defect class it records**: a `diff` between `f0d2bdadf^`'s blob and the *working tree* lumped C75 and `94dfd3c68` into one diff, and C75's message *discusses* the citation's line — so the shift was attributed to the wrong commit. A diff against the wrong base is an instrument pointed at the wrong target |
| C77 law 15's per-sender height comparison is false of the model, which is wider than the port's data | 15 | **found by attempting the unit that was meant to close the row** (2026-09-24): the walk takes **every** same-sender parent, so a same-sender **fork** on `mv`'s frontier puts a message from the other branch into its output, and that branch sits below no given sentinel — a counterexample the model admits and the port's data does not. **Not a bug in the port, and not merely a datum either**: a validator produces one block per height, and the port **refuses** a same-sender fork at ingress rather than relying on the observation — H-1's equivocation detection rejects a second distinct block by one sender reusing a `seq_num` before any partial write (`casper/src/dag.rs:244-252`), `sequence_number` requires the justified same-sender block to be exactly one `seq_num` lower (`casper/src/validate.rs:169-188`), and `check_justification_regression` admits at most one justification per sender and demands it be the latest (`:205-241`) — so `self_parents` walks a chain the ingress guarantees. (H-1 landed 2026-08-20, `76415d6c6`, a month before this entry's premise was written; the correction is recorded in law 15's row.) The obligation therefore becomes the hypothesis **"at most one same-sender parent per message"** plus the boundary (`selfParents_skips_finalized`) plus the descent (`Descends`) — smaller and better defined than "the arithmetic across a chain". **The asymmetry that earns it a number**: the same shape as C69's cross-sender reading of this law and as G9's law 47 — an `owed` conjunct that needs a hypothesis the **port's data** satisfies and the **model** does not carry; named once, the next attempt does not rediscover it by hitting the counterexample. **And the disposition**: the `owed` column was not emptied by proving a statement the tree does not hold — the pass's call for G9 and for this law's cross-sender clause, applied to the unit written to close the column |
| C78 `ShardId`'s own constructor does not maintain its type's invariant | 26b | **found while correcting 26b's witness** (2026-09-24). `ShardId::child` (`shared/src/refined.rs:382-386`) builds the newtype **directly** — `format!("/{name}")` at the root, `format!("{}/{name}", self.0)` below — so the validation the type exists for (`TryFrom<String>`, `:423-434`: non-empty ASCII) is **not** applied to the name it is given: `ShardId::root().child("\u{2713}")` yields an id `TryFrom` would have refused. **Latent rather than live, measured**: the one production call site validates first (`casper/src/conf.rs:43` runs `ShardId::try_from` on the name before `:48`'s `parent.child(&shard_name)`), so the invariant held — **by the caller's discipline, not by the type**, which was the opposite of what `TYPE-SYSTEM.md`'s "no type escape" convention claims for this newtype. **And it is now held by the type** (`74d21294d`): `child` returns `Result<ShardId, RefineError>` (`shared/src/refined.rs:396`) and refuses an empty name, so the validation the type exists for is performed *by the constructor* and the call site's comment becomes belt-and-braces rather than the guarantee. The remedy named at the end of this entry is therefore no longer a proposal — it is what landed. **What it cost, and why it is worth a number**: the model's `ShardId` is a `String`, so it inherits both directions of the gap, and row 26b's original witness (`validShardId (s.child n) = validShardId s`) was **false** on each: a non-ASCII name, and an invalid *parent* whose child is valid (`shardChild "" "x" = "/x"`). The row now carries the characterisation that is true (`validShardId_child`: given a valid parent, the child is valid exactly when the name is ASCII) and its `decide`d counterexample. Not a §6 deviation: the oracle's shard id is a plain `String` and has no invariant to maintain. **And the caller documents the gap at its single call site**: `casper/src/conf.rs:42-44` reads `// ShardId::child composes the name without validating it, so the name is checked on its own — otherwise a spec could carry an id that format_of_fields would later reject`, then `ShardId::try_from(shard_name.clone())` (`:43-44`) — and `grep -rn "\.child(&"` across `casper/src node/src rholang/src shared/src block-storage/src` returns exactly **one production call site on a `ShardId`** (that one; `rholang/src/scheduler.rs:33`'s `child(&self, i: u16)` is a different type's method). So the invariant is not merely held by caller discipline: **the caller knows why it must be and says so where it matters**, which is precisely the shape `TYPE-SYSTEM.md`'s "no type escape" convention claims the newtype removes. The remedy a reader will look for — validate inside `child`, or make it return `Result` — is inferable from the code and not taken now because it changes a public signature for a state no caller can currently produce |
| C79 `CasperFinality.tla` is described as "formalized", and no tool can run it | 14, 15, 16 | **measured** (2026-09-24): `legacy/casper/src/main/resources/CasperFinality.tla` (147 lines) defines the predicates — `SumStake`, `IsSuperMajority`, `SeenClosure`, `SupportingStake`, `Finalized`, `FaultToleranceMargin` and the rest (12 definitions) — and one invariant conjunction, `Inv` (`:140-145`): `JustificationsWellFormed /\ SeqNumStrictlyIncreases /\ FringeWellFormed /\ SeenMonotone /\ FinalizedIsSeenByAll`. It has **no `Init`, no `Next`, no `Spec`, no `THEOREM`** (`grep -cE "^(Init|Next|Spec|Specification) *="` → 0, `grep -cE "^THEOREM"` → 0) and no `tla2tools`/`TLC`/`tlaps` reference anywhere in `tools/`, the `Makefile` or `.github/`. TLC model-checks a *specification* against an invariant and TLAPS proves *theorems*; with neither present **no tool can run this file, installed or not**. It is a glossary plus an invariant predicate, and "formalized" is the overclaim. **The class, one level deeper than C70/C75/C76**: those were instruments that could not see the defect they named; this is an **artefact that cannot fail** — a named `Inv` reads as a checked invariant while nothing checks it, which is how it survived in the record. **And the plan's sizing was wrong**: Phase 2 called this "wiring it into the formal gate (or recording why not)", implying a missing checker; as measured, wiring it requires a `Spec` to be **written** first — an `Init`/`Next` over the DAG — which is a modelling decision, not a dependency. That is the *second* item tonight that turned out to be "the statement is missing" rather than "the tool is missing" (26b's absent string-order theory was the first). **The laws are not owed to it**: 14b is `provedModel` over `Dag.lean`'s derivation, 15 is narrowed with C77, and 16c has its layer — so the TLA is **superseded**, not merely unrun. **Disposition**: not wired, and named as a *definitional* gap rather than a gating one — it records intent in the reference tree; the trigger to revisit is a law needing DAG-level *temporal* reasoning the register cannot state. `spec/INVENTORY.md`'s open question is corrected in place to "states the intended invariants as a reference-tree artefact", with what would be needed first |
| C80 a dropped register row is invisible to every check — a catalogue with no count of itself is checked for internal consistency, not for completeness | — **register** | **found by accident, confirmed by measurement** (2026-09-24): an edit of mine that rewrote row 26b's cells deleted the **26c record** with it, and every check stayed green — the numbering check validates *numbers* (26a survived, so 1..49 was intact) and reference integrity validates *cited declarations*, so **neither can see a dropped row**: both are checks *within* the set rather than *on* it. What surfaced it was the emitter's own summary reading **"57 entries"** against a known baseline of 58 — an artefact that failed only because a person remembered the baseline, which is C79's shape (an artefact that cannot fail) one level up. **The fix, landed with this entry**: `LawsMain.lean` gains **`entryCeiling := 58`** beside `lawCeiling`, hand-bumped and deliberately not derived for `lawCeiling`'s stated reason (a check that counted the rows would compare the register to itself, and a dropped row would take its own evidence along), so a dropped clause is a build failure naming the expected and actual counts. **Falsified before it was believed**: deleting one clause fails the build with that message, and restoring it passes. Adding a row now carries the same maintenance tax `lawCeiling` already carries. 26c itself is restored byte-identical to HEAD, so the commit that lands this carries no change to that row |
| C81 the `run` subcommand has no discoverable help, and prints a usage line that offers none | — **harness** | **confirmed end-to-end on the built binary** (2026-09-25): `rnode run --help` → `error: unexpected argument '--help' found`, and the usage line it then prints — `Usage: rnode run [OPTIONS]` — names no help flag either; `rnode run -h` → `error: a value is required for '--api-port-http <API_PORT_HTTP>' but none was supplied`. **The cause is structural** (`node/src/configuration/commandline/options.rs`): the top-level `Options` sets `disable_help_flag = true` (`:72`), defines a **long-only** custom help (`:76`, `#[arg(long = "help", action = clap::ArgAction::Help)]`), and binds `-h` **twice** — `--grpc-host` (`:79`) and `--api-port-http` (`:364`) — so the subcommands inherit no help flag at all. **The class is this pass's again: a surface that reads as present and is not** — the operator's first move (`run --help`) is the thing that fails, and the failure names a *different* flag, so it reads as a mistyped argument rather than a missing feature. **Closed at `f96600bb2`, and the fix established two things wider than the report.** (1) **The defect was not `run`-specific**: *every* subcommand lacked a help flag — `run`, `deploy`, `repl` and `status` each exited 2 — and the cause is the one structural fact, `disable_help_flag = true`, the very setting that frees `-h` for `--grpc-host` leaving no help flag for any subcommand, so no subcommand's options were discoverable from the CLI. (2) **The fix is one attribute, not four fields**: the port's long-only `--help` becomes `global = true` (`options.rs:85`, with the reason in its comment), so each command renders *its own* options, while `disable_help_flag = true` **stays** (`:72`) because it is what keeps `-h` as `--grpc-host`; a per-command help field was rejected because four commands are payload-less variants (`Status`, `Repl`, `Mvdag`, `LastFinalizedBlock`) with nowhere to put one. **And the `-h` decision is a contract choice now stated rather than left implicit**: `-h` is **unchanged in both contexts** — `--grpc-host` at the top level, `--api-port-http` under `run` — because freeing it for help would break spellings that parse today (`rnode -h <host> deploy …`), while `-h`-for-help was never available in this port; the help text states the convention. Operator-facing consequence: **`rnode run -h` still errors, and `--help` is the spelling for help** — the fix makes help *discoverable*, not `-h`-spelled. **The falsifier is test-level and exact**: with the new tests in place and only the attribute reverted, `every_command_accepts_help_for_its_own_options` (`options.rs:630`) failed `left: UnknownArgument, right: DisplayHelp` for `["run", "--help"]`; `cargo test -p rchain-node --lib` is 190 passed / 0 failed, and the previous unit's metrics refusal still fires (`rnode run --influxdb` → `Configuration error: …`, exit 1), so argument parsing is untouched |
| C82 `Descends` quantifies over every parent, and the port bounds only the unfailed ones — so the descent order is violated by a state the port admits | 15 | **found by measurement, and reachable rather than merely representable** (2026-09-25, H1b). The model's `Descends` (`spec/Rchain/Casper/Dag.lean:296-297`) requires **every** resolved parent to be lower than the message naming it, while the port enforces that only for **unfailed** justifications: `validate::block_number` skips them (`casper/src/validate.rs:156-158`, `if !meta.validation_failed`), and a failed block's recorded height is its **claimed** `block_num` (`message_from_block_metadata`, `height: block.block_num`, `casper/src/dag.rs:45`) with nothing bounding it. **The violation is constructible through the port's own acceptance path**: genesis at 0, a failed block by validator 1 claiming **999**, then a child by validator 2 claiming **1** and justifying both — `block_number` answers `Ok(())` because it skips the failed justification, and `insert` takes the block, leaving `parent.height = 999 > me.height = 1`. Witnesses: the case is `h1b_a_failed_parent_above_the_childs_height_is_refused` (`dag.rs:1268`) — the construction was `..._breaks_the_descent_order` when it *was* the violation and the falsifier inverted with the code, so the same test now asserts the refusal — and `h1b_a_justified_bonded_failed_block_is_refused_rather_than_forced` (`:1328`) is the case below. **And the corrected brief is what makes the route the right one**: `neglected_invalid_block` does **not** force a block to justify bonded invalid blocks — it **refuses** a child justifying a failed block whose sender is still **bonded** (`NeglectedInvalidBlock`, `casper/src/validate.rs:266`), faithful to the oracle (`legacy/casper/.../Validate.scala:340-360`), so the bonded route is **closed** and the open one is a peer naming an **unbonded** (or zero-stake) validator's failed block, which no rule forbids. **Enforced, not restated — the disposition this row carried is superseded by the enforcement decision, and the divergence it predicted is now real and numbered** (`46c35b545`, 2026-09-25). `block_number` bounds **every** resolved parent *before* it skips the failed ones for the maximum: `if i64::from(meta.block_num) >= i64::from(b.block_number) { return Ok(Err(BlockStatus::InvalidBlockNumber)) }` (`casper/src/validate.rs:152-154`), with the failed-skip that computes the maximum untouched at `:156-158`. So the state that made the hypothesis false — the failed parent claiming **999** with a child at **1** — is **refused at ingress** rather than admitted, and the proof's second hypothesis (`Descends`, `spec/Rchain/Casper/Dag.lean:296-297`) now holds of **every state the port admits** instead of being merely *stated* of a state that refuted it. **Which states each form covers, precisely**: before, `Descends` *covered* — was asserted of — every admitted state including the failed-parent-above ones, and the measurement at the top of this row is that it did not hold of them; the bound now **excludes exactly those and nothing else**. It does not narrow the maximum the claimed number is computed from (that still skips failed justifications, `:156-158`), so an unfailed parent at or above the child was refused before and still is — what is new is the failed parent. The witness inverted with the code: `h1b_a_failed_parent_above_the_childs_height_is_refused` (`casper/src/dag.rs:1268`) is the same construction, asserting the refusal where it asserted the old code's admission. **And the divergence the old sentence predicted is now a fact with a number: C83.** The reference validator **filters failed parents out of `blockNumber` entirely** and never bounds a resolved parent's height (`legacy/casper/src/main/scala/coop/rchain/casper/Validate.scala:178-198`), so it **admits** the block this refuses. That is a §6 deviation of the **port** — kept, because the laws are the oracle and this is the premise law 15's proof consumes — and C83 carries the operator consequence; the model obligation this row was, becomes an enforcement the port now holds |
| C84 the port refuses an equivocating block — at insert **and** at restore — where the oracle has no equivocation gate at all | 15 | **found by reading the Scala for H1c's scoping, and it closes the arc H1a opened** (2026-09-25). The gate is H-1's (`casper/src/dag.rs:244-252`: a second, distinct block by one sender reusing a `seq_num` is rejected before any partial write, pinned by `insert_rejects_equivocation_same_seq_num` at `:568`), and H1c extended it to the **restore**: `BlockDagKeyValueStorage::create` folds the persisted `height_map`, and the persisted layer checks only *contiguity*, so a store already holding a fork used to restore straight into the DAG and the H-1 stall the gate prevents was live again with nothing saying so (`a2b20554e`, pinned by `h1c_a_store_holding_an_equivocation_is_refused_on_restore`; the check is a `BTreeMap<(Validator, SeqNum), BlockHash>` rather than `insert`'s per-message scan, so the restore stays O(N) under C55). **And the oracle does not do this**: the Scala has **no equivocation gate anywhere** — a search over `legacy/casper` and `legacy/block-storage` finds none — and its `validateDagState` (`legacy/block-storage/src/main/scala/coop/rchain/blockstorage/dag/BlockMetadataStore.scala:118-124`) asserts only that the height map's numbers are contiguous, never `(sender, seq_num)`, so a forked store restores silently there. **What the refusal buys, and why the deviation is kept**: an equivocating validator can neither enter the DAG nor stall finalization. **H1a's note points at this row** for the reason law 15's no-fork premise is *guaranteed* rather than observed; this is the other half of that arc — the premise is enforced by a departure the Scala does not share. **Operator consequence: a peer's equivocating block is refused, and a node restoring a forked store refuses to start** (the Scala starts and carries the fork). **And it exposed a fixture defect, recorded here as the same finding from the test side**: six tests sharing `store_chain` built a chain in which one sender reused `seq_num 0` for every block — a state H-1 and `sequence_number` both forbid, legal to store only because the persisted layer does not look at sequence numbers — so six tests had been pinning behaviour over states the port does not admit; the repair is in `a2b20554e`, and its best evidence is that the digest constant moved (`a6632357…` → `be621d6c…`, the digest being a function of the representation's value) while `logical_bytes == 2032` and `seen_entries == 21` did not |
| C85 `visualizeDag` panics on an empty window in the oracle and returns an empty graph here | 43 | **found by the pass's live bug (H2a), settled against the oracle** (2026-09-25): `dag_as_cluster` (`casper/src/api/graph_generator.rs:51`) read `timeseries[0]` unguarded, reachable from the public `visualize_dag` (`block_api_impl.rs:534`), so an empty DAG — or a `start_block_number` above the chain's top — handed the renderer an empty slice and the index panicked. Fixed with `timeseries.first()` and an empty graph when there is no lowest height to anchor clusters to (`da6c4a440`), and the caller's filter is extracted as `view_blocks_above` — a pure function of a `DagRepresentation` and the window's lowest height — so the reachable inputs are pinned without constructing a `BlockApiImpl` over a runtime. **The oracle raises**: `GraphGenerator.scala:39` is `val lowestHeight = timeseries.head` over `acc.timeseries.toList.sorted` (`:38`), a `List` built from a `Set`, so an empty window throws `NoSuchElementException`. **Operator consequence: an empty window renders an empty graph here where the Scala's endpoint raises** | **The law column reads 43, and it is the *endpoint* law rather than a DAG one**: what `visualizeDag` answers when there is nothing to draw is a `[]`-semantics and error-precedence question — "each endpoint's serialized shape equals the schema's" (`43`) — and not a fringe or ordering one. The cell read `47`, which is the PoS withdrawal-staging law and has nothing to do with this endpoint; corrected 2026-09-25 so the column points a reader at a law that covers the incident |
| C86 the port propagates a DER decode failure on the sign path, where the oracle silently returns the empty array | 19 | **settled against the oracle, and it goes the other way from C84/C85** (2026-09-25, H2c): `Secp256k1Eth::sign` returned `Ok(certificate_helper::decode_signature_der_to_rs(&der).unwrap_or_default())` — a decode failure flattened into a value **on the sign path**, which the `silent` scan cannot see because it matches `unwrap_or(0)`-shaped calls and this is `unwrap_or_default()`. **And the oracle does exactly that**: `legacy/crypto/.../signatures/Secp256k1Eth.scala:66-72` is `decodeSignatureDERtoRS(sigDER).getOrElse(Array[Byte]())`, annotated "DER conversion error silently returns empty array", so the port was **faithful before this fix** and the fix is a *deliberate departure*. It is worth taking because the flattened value is the **empty vector** — not a zeroed 64-byte signature, which is what a brief said — so `sign` returned `Ok(vec![])`: zero bytes where `sig_length()` promises 64, indistinguishable from a result. **The defect was latent, measured rather than asserted**: the writer probed **64 key pairs** and no `sign` result was ever the flattened value, structurally — the signer emits minimal DER and the decoder strips DER's sign padding (`left_pad_32`), so decoding what this code just produced cannot fail. That probe survives as the guard `sign_returns_a_verifying_64_byte_signature_and_never_zeros` (`d01f1be2d`), so the evidence for "unreachable" is a test rather than a sentence. **Operator consequence: a malformed DER reaching `sign` now returns `CryptoError::InvalidSignatureFormat` where the oracle returns the empty array** — the port preferring an error to a value that violates its own length contract |
| C87 the genesis seeder indexed a published datum's two elements without checking its arity, so a short datum panicked the seeding on the genesis **and** the replay path | — **partiality** (no law) | **found by measurement, and settled against the oracle by finding that there is none** (2026-09-25, H2b). `rgov::published_uri` (`casper/src/genesis/rgov.rs:268`) read the published `["<name>", <uri>]` datum with `items[0]`/`items[1]` **after** `RhoList::unapply`, which returns the items at *any* arity, so a one-element or empty datum panicked with `index out of bounds: the len is 1 but the index is 1` (`afec579e6`'s falsifier reports the old `items[1]` read at `rgov.rs:267`, the pre-fix line). Both reads are `items.first()?`/`items.get(1)?` now (`:271-273`) and the caller's existing `else { continue; }` makes a short datum **skipped** rather than fatal — `None` is the right answer rather than an error, because the caller already skips what it cannot parse. **The datums are chain-side, not local**: `seed_rgov_aliases_from` runs on the genesis path (`casper/src/runtime_manager.rs:709`) and on the **replay** path (`casper/src/runtime_replay.rs:190,222`), replaying them out of the block's own genesis state, which is law 11's stake in it — the alias writes are native writes outside the deploy log, so a replay that cannot reproduce them diverges the genesis hash. **There is no Scala counterpart, and establishing that is the finding**: the whole publish-channel mechanism is this port's own — `URI_PUBLISH_CHANNEL = "rnode:genesis:rgov-uri"` (`rgov.rs:141`) is a port constant, and a search of `legacy/` for `rgov`, `readcap`, `publishChannel` and the channel name returns **nothing** (the only `rgov` hits are wallet text files). Upstream deploys this set at runtime with a script outside the node (`scripts/bootstrap-rgov.ts`, which this module's own doc names and which is not in this repo) and feeds the resulting URIs back into the dependents; it has **no node-side reader of a name/uri datum at all**, so the "index blindly, ignore the datum, or raise" question this row opened has no Scala answer to give. The closest Scala is the *registration* path, and it does not inspect a datum's shape either: `legacy/casper/src/main/resources/Registry.rho:409`'s `insertArbitrary(@data, ret)` binds the whole datum as `@data` and stores it under a freshly built URI, inspecting nothing — so **no §6 row is written**: there is no oracle behaviour here to depart from, and the fix is the type discipline's (no silent partiality) rather than a deviation from the reference. **Latent through the port's own genesis, measured rather than asserted**: both producers of the datum are this module's and both emit exactly two elements — `publish_registration` (`rgov.rs:290`) appends `@"rnode:genesis:rgov-uri"!(["<name>", uri])` and the master-directory template (`:500`) publishes `["readcap", *URI]` — and the shape is itself pinned at **build time** by the assertion that the published term is present (`:618`), so a genesis *this* binary builds cannot produce a short datum. What the guard buys is therefore at the **boundary**: `published_uri` is `pub` and reads data that came from a chain, so whether a genesis whose publish datum drifted seeds its aliases or takes the node down is decided by *which revision replays it* — and an index panic is the wrong answer for a `pub` parser in any case. The fix is pinned by `published_uri_refuses_a_list_shorter_than_two` (`:773`), which asserts both refusals **and** the two-element control, so the test can fail. **Operator consequence: a published datum shorter than two elements is skipped rather than fatal** — where before it took the genesis seeding, and the replay, down with an index panic |
| C83 the port refuses a block naming a resolved parent at or above its own number — the failed ones included — where the Scala filters failed parents out and bounds no resolved parent at all | 15 | **the enforcement decision (H1b enforced, `46c35b545`), taken because the laws outrank the Scala where the proof needs the premise** (2026-09-25). The bound is `if i64::from(meta.block_num) >= i64::from(b.block_number) { return Ok(Err(BlockStatus::InvalidBlockNumber)) }` (`casper/src/validate.rs:152-154`), applied to **every** resolved parent before the maximum's failed-skip (`:156-158`); C82 is the row it serves, and the §6 register carries the deviation. **The oracle admits the block**: `legacy/casper/src/main/scala/coop/rchain/casper/Validate.scala:178-198`'s `blockNumber` resolves the justifications through `lookupUnsafe` and then **`filter(!_.validationFailed)`** — a failed parent is discarded before the maximum, and no resolved parent's height is ever compared against the block's number — so a Scala validator receiving a chain containing such a block accepts it where this port answers `InvalidBlockNumber`. **Operator consequence: a peer sending such a block is refused** — so a port validator rejects a block a Scala validator admits, **a validator-side divergence on this check alone**, which is what a §6 row is for. **And the measurement bounds the risk to byzantine input**: no honest proposer can produce the refused block, and that is read off the proposer's own arithmetic rather than argued. A proposal's justifications *are* `latest_msgs.values()` (`casper/src/multi_parent_casper.rs:293-300`, `get_pre_state_for_new_block`, called at `casper/src/blocks/proposer/proposer.rs:423`); a validation-failed block is recorded in the message map but **kept out of `latest_msgs`** (`casper/src/dag.rs:356-362` → `block-storage/src/dag/message_state.rs:118-128`, the H-2 exclusion, whose comment says why: a failed block that became a parent "would wedge block production"); and the number claimed is `max(parent heights) + 1` over exactly those parents (`casper/src/blocks/proposer/proposer.rs:426-432`). So the failed-parent route is closed **at the proposer** and the unfailed one was already refused **by the maximum**: only a **byzantine or custom peer** can send the refused block, and the honest ones cannot. **Why it was taken anyway**: `Descends` is the premise law 15's proof consumes, and a premise the port *guarantees* is worth a divergence a byzantine peer can trigger and an honest one cannot — the reasoning C82's superseded disposition predicted as hypothetical ("a validation divergence that would need a §6 row of its own") is now the row |
| C88 the shard-id domain check was a `debug_assert!` — it vanished at `--release`, so a non-ASCII shard id reached the seed law 11's determinism rests on | 26b | **found by the audit's asymmetry rule, and settled against the oracle as a *fix* rather than a deviation** (2026-09-25, H2d). `BlockRandomSeed::new` asserted the shard id's ASCII-ness under `debug_assert!`, so in a `--release` build the check **was not there**. The falsifier states that as a run rather than an argument: `a_non_ascii_shard_id_is_refused_in_any_profile` (`casper/src/block_random_seed.rs:224`) **FAILED** under `debug_assert!` at `--release` — there was nothing to refuse — and passes in **both** profiles as `assert!` (`:49`), pinning both halves (a non-ASCII id is refused, an ASCII one accepted, so it cannot pass vacuously). **And the oracle asserts it too**: `legacy/casper/src/main/scala/coop/rchain/casper/rholang/BlockRandomSeed.scala:39` is `assert(shardId.onlyAscii, "Shard name should contain only ASCII characters")`, a plain `Predef.assert` that only `-Xelide-below ASSERTION` would elide — and no build file in `legacy/` sets it — so the Scala's check **is live in production** and the port's `debug_assert!` was a **weaker port, not a faithful one**. The change is therefore **parity, not departure**, and no §6 row is written for it: `assert!` is what the reference does. **The boundary enforcer is named rather than the assert left bare**: `validate::format_of_fields` refuses an empty or non-ASCII shard id at ingress through `ShardId::try_from` (`casper/src/validate.rs:19-23`, test `format_of_fields_rejects_non_ascii_shard_id` at `:461`, and not debug-gated), and C78 records that the port's `ShardId` is the validated newtype the oracle's plain `String` is not. **One difference is worth naming while both trees are open, and it is not a §6 row**: on a *peer* block carrying a non-ASCII shard id the port answers **invalid**, where the Scala's `formatOfFields` accepts it — it checks only `isEmpty` (`legacy/casper/src/main/scala/coop/rchain/casper/Validate.scala:56-79`) — and the refusal happens *later*, as an assert in the validation path (`deploysShardIdentifier`, `:270-273`): a throw rather than a verdict. Both trees refuse the block, by different means and with the same consensus outcome, so this is robustness where the port is the cleaner of the two. **And C78's "the oracle's shard id has no invariant to maintain" means the *type* has none** — the oracle keeps ASCII-ness by assert in two places (`Validate.scala:272` and `BlockRandomSeed.scala:39`), which is exactly why the constructor assert is parity and not an addition. **Operator consequence: a non-ASCII shard id is refused in every profile** — before, a `--release` build accepted one into the block's random seed |
| C90 the type-system audit's panic scan never ran in CI, and the 29 "violations" it reported were its own staleness check reporting that the whitelist matched nothing | — **harness** | **found by the coverage job going red, and it is a harness finding: no law covers it, because the thing that was wrong was outside the term** (`1a0fa77b8`, 2026-09-25). `scan_panic` passed its pattern as `awk -v RE="$pattern"`, and **`-v` processes backslash escapes in the value**, so the escaped literal parens in the pattern became group-openers, the alternation became an invalid regexp, awk **fataled**, and the scan matched **zero sites**. CI's log is the mechanism verbatim: `awk: warning: escape sequence '\\(' treated as plain '('` then `awk: cmd. line:21: fatal: invalid regexp: Unmatched ( or \\(: /.unwrap()|.expect(|panic!|…/`. With no sites, every whitelist entry matched nothing — and the **per-site staleness check (added last pass) correctly reported each of them as stale**: twenty-nine violations, none of them a violation. **What the disguise was, and it is why a local run could not have caught it**: the `-v` form *worked on this machine*, because this box's awk is **mawk** (measured here: with the same pattern, `-v` accepts the mangled form and matches through it, while the environment form searches the literal-paren regex the pattern actually wrote), and CI's awk is **gawk**, whose escape warnings are the log lines above. So the local green was never evidence of a working scan. **Why it surfaced only now, which is the instrument's part of the finding**: the per-site staleness check is the *only* thing that could see it, because before the per-site conversion the whitelist was **file-level** and nothing counted its matches — **a scan that returned nothing looked exactly like a clean tree**. That is C76/C79/C80's shape once more (a record believed because nothing re-derives it), here wearing the **environment** as its disguise. **How long CI's panic class had been scanning nothing is not recoverable from here** and this row does not imply a date: what is certain is that the check that finally caught it is one whose absence had been invisible for the same reason. **The fix is portability rather than a workaround**: the pattern goes through the **environment** (`RE="$pattern" awk … ENVIRON["RE"]`), which no awk escape-processes on any implementation. **Verified in this lane, not relayed**: the audit now reports `29 panic-class site(s) in production code; 29 allowlisted by 28 entr(y|ies)` and `23 narrow-refinement construction(s) in 11 file(s)`, exit 0 in 16 s here — and **the count is itself evidence**: the same run reported 30 sites / 27 allowlisted before, and the difference is exactly the prose false positive the commit before this one removed, with the two then-unallowlisted sites being the ones it fixed. **Operator consequence: none for the node — the panic class was the *instrument*, so what was lost is detection, not behaviour**: a real panic site in production code would have been invisible to CI for as long as this lasted, and the whitelist's own entries are what finally forced it into the open |
| C91 the audit can print a green summary while a class never ran — its exit code reflects the classes that *completed*, and nothing checks that one did | — **harness** | **found by the lane paying for it rather than by reading, and it is the framework's own version of C90** (2026-09-25, H3, `9bb6c3dfe`). A malformed whitelist entry — a missing field, so `parse_entry`'s `${front:0:$(( ${#front} - ${#ev} - 2 ))}` (`tools/audit-type-system.sh:262-264`) computed a **negative** length — made the audit print `OK: no hard production violations (panic/unsafe/silent/escape).` **in 0.035 s while scanning nothing**, measured by the lane that hit it. The parser bug is fixed and the guard now runs full length (13.8 s here), but **the structure that allowed it is untouched**: there is no `set -e` (the script has no `set` line at all), `hard_failures` is a global counter incremented only in `note()` (`:167-173`), the classes are a plain loop (`:891-893` over `:887`'s roster), and the summary prints **unconditionally** after that loop, exiting 1 only if that one counter is non-zero (`:895-901`) — so **a class that ends without calling `note()` leaves the exit at 0**, whether it ended because a helper errored, because a subshell took its findings with it, or because it returned early, and the result is indistinguishable from a class that found nothing. **There is no check that a class completed, and the count of classes that ran is exactly the thing nobody counts.** **The contrast that states it best, and it is a clause rather than a row**: the component the *same unit* added is stricter about its own emptiness than the framework around it — H3's guard fails hard when it derives no newtype at all (`:828-829`: "a derivation that silently finds nothing passes silently, which is the failure this guard exists to prevent") and when an exemption matches nothing derived (`:818`), while the framework that runs it has no such rule for itself. **And this is C76/C79/C80/C90's shape one level up**: those were records believed because nothing re-derived them; this is the instrument's *own summary* believed for the same reason. **Disposition: open, and it wants a fix rather than only a row** — the file is `tools/`, so the instrument lane holds it. The cheap structural remedy is a **per-class completion marker**: each class appends its name as its last act and the summary refuses unless the completed set equals `classes`, because `set -e` alone cannot catch a class that *returns* early, and the marker is the same medicine this guard already applies to its own derivations. **Operator consequence: none for the node — the loss is detection, and the row's own statement is that the gate's `OK` is a claim about the classes that finished, not about the classes that were asked for** |
| C89 the register-anchor check **aborted** on an unresolvable citation instead of reporting it — `exit 1`, no `FAIL` line, nothing after its own section header | **harness** | **found by walking into it** (2026-09-25, landing law 19's tie), and it is C37's class with the sign flipped: that was a measurement that was not a measurement, this is a **check that was not a check**. `anchor_resolve` sets a global `file` and returns it to two callers, each of which prints `does not resolve — write the path in full` when it is empty. The printing never happened. A function's status is its last command's, and both branches end on a **test**: the `for cand in …; do [[ -f "$cand" ]] && { file=…; break; }; done` leaves the failed `[[ -f ]]` as the loop's status, and the enclosing `if`/`else` propagates it — so on exactly the input the function exists to report, it returned 1, and `set -euo pipefail` killed the script at the call site. **Measured, not argued**: with `Crypto/Spec.lean:41-49` in row 19's note the audit exited 1 with its log ending at `== register anchors (every cited line still holds what the row says) ==` and **zero** `FAIL` lines; the same tree with the path written in full (`spec/Rchain/Crypto/Spec.lean:41-49`) printed `OK: the register matches the tree`. The mechanism was then isolated in six lines — `set -euo pipefail`, a function whose last command is a failing `[[ -f ]]`, one call — which reproduces `rc=1` and no output. **And the abort was reachable from a one-word mistake**: five rows of this file instruct an author to "write the path in full", and the instruction carried no verdict when ignored. **Fixed** by an explicit `return 0` at the function's end, with the contract stated (`set file, say nothing`) and C89 named at the line, because the next helper written in this style will end on a test too. **Falsified both ways after the fix**: the unresolvable path now produces `FAIL  law 19: `Crypto/Spec.lean:41` does not resolve — write the path in full (its basename is ambiguous or absent)` and the run reaches its summary; the full path is green. **Operator consequence: none for the tree, and one for the instrument** — a citation defect in the register used to stop the audit at its ninth section with no explanation, which a reader would have to distinguish from a passing run by the exit code alone |
| C92 the model's PoS state cannot hold the bonds map the finalizer's gates read — `PosState.active` is ids without stakes, so law 16d could not be stated, let alone proved | 16d | **found by asking the law what its subject is** (2026-09-25, closing law 16d's sync site). The port's bonds map *is* the `pos:active` leaf: `compute_bonds` is one read, `get_native(PREFIX_POS, pos_active_key())` (`casper/src/runtime_manager.rs:1276-1281`), decoded as `BTreeMap<Validator, NonNegI64>` — **a map with stakes** (`rholang/src/native_state.rs:739-747`). Its writers are three and none of them is `bond`: `select_active(&pool, &withdrawers, &params)` at a boundary (`:1152-1157`), genesis installation (`:937`) and `slash` (`:1229`) — so between boundaries the pool's stakes move and the active map's do not, and the map the gates run on is a **boundary snapshot**. That is law 44's gate ("a bond pools but does not activate") seen from the finalizer's side, and it is exactly what law 16d is about. **The model has no such field**: `PosState.active : List Validator` (`spec/Rchain/Pos.lean:184`) with the stakes in `pool`. Deriving the bonds map from the model's state would answer with the *current pool stakes* — for a validator that bonded after the last boundary, a **different map from the port's** — so the derivation would be wrong exactly where law 44 says the difference is real, and `proved-model` rows 44–47 rest on that coarser state without saying so. **Consequence, and the fix's size**: law 16d moves `open` → `owed` with its sync site modelled (`Rchain/Casper/Bonds.lean`: the three sources, `the_sources_agree`, and the refusal `a_disagreeing_set_is_refused` with #73's shape as a concrete witness), and what remains is a change to `PosState` — record the stakes *selected* at the boundary, i.e. `active : List (Validator × Nat)` — which touches laws 44–47's model and their proofs. Stated here rather than in a note because the state's coarseness is what *made the row unstatable*, which is the register's own reason for existing: the law found the gap, and the gap is one field wide. **Closed (2026-09-25)**: `PosState.active` is a `List (Validator × Nat)` and `reselect` keeps the pairs instead of dropping them (`.map (·.1)` was the loss), so the model holds the map the gates read; the state's side is stated as a *pair* — `a_bond_does_not_move_the_map_the_gates_read` and `the_pool_derived_map_moves_where_the_states_does_not` (`Rchain/Casper/Bonds.lean`), because either half alone is a spelling — and law 16d moves `owed` → `proved-model`. Falsified first: making the first theorem read `pool` instead of `active` stops it being `rfl` and the build fails. The field change also made law 44's `a_boundary_activates_the_pool` **strictly stronger**, its `s'.claims = []` hypothesis going inert rather than load-bearing |
| C93 the model's min message was the **newest** same-sender ancestor where the port's is the oldest — the derivation's step 2 read the opposite end of the chain, and no instance could see it | 15 | **found by measuring the walk's order against the port's, on law 15's last conjunct** (2026-09-25). Step 2 of `next_fringe` is `chain.into_iter().last()` over `[p] ++ self_parents p` (`block-storage/src/dag/finalizer.rs:193-199`), and the port's `self_parents` builds its chain with `chain.push(m)` over a worklist it **replaces** each step (`:74-95`), so that chain is **newest-first** and `.last()` is the *oldest* non-finalized same-sender ancestor. The model's `selfParents` (`Rchain/Casper/Dag.lean`) prepends into its accumulator instead, so its list is **oldest-first** — and `minMsgs` read `getLast?`, i.e. the **newest**. **Measured**: for `10 (h 0) ← 11 (h 1) ← 12 (h 2)` the model's `selfParents` is `[10, 11]` and the port's chain is `[11, 10]`, so the model answered `11` where the port answers `10`; the transcribe-the-port version of the walk (`next.pop()`, `chain.push`, `next` replaced) returns `[11, 10]` under the model's own `sameSenderParents`, which is what pins the direction rather than a reading of the Rust's intent. **Why nothing failed, which is the finding**: every `decide`d instance in the file (`dag3`'s `minMsgs_dag3`, `a_derivation_publishes_a_layer`, `a_derivation_is_an_antichain`) gives a sender at most **one** same-sender ancestor, and with one element `getLast?` and `head?` agree — so a step of the derivation read the opposite end of the chain while `decide`, the corpora, the gate and every register check stayed green. A test that cannot distinguish the two ends is C37's class (a measurement that was not one) in the shape of an *instance*. **Fixed** to `.head?` with the measurement in the docstring, and pinned by two `decide`d theorems over a new `chain3` — `the_walk_is_oldest_first` and `a_chain_of_three_picks_the_oldest` — whose falsification is a revert: restoring `getLast?` makes the second one fail. The antichain laws above are unaffected (which of two same-sender messages the layer fold keeps does not change its keys), and law 15's *statement* is still `owed`: the step is now faithful, the per-sender sentinel comparison is not yet proved |
| C94 the register has **no model of the block requester** — the LFS ancestry walk is specified by the Scala alone, so no proof constrains it and nothing mechanical would catch a wrong cutoff | — **no law**: the requester is unmodelled | **found by sizing the §6 row that offered the Scala's cutoff as an efficiency unit** (2026-09-25, settling row 309 rather than implementing it). The walk is registered nowhere in the oracle: grepping every `.lean` under `spec/Rchain/` for `lowerBound`, `extraHeights`, `requestStream`, `LfsBlockRequester` or `cutoff` returns nothing, and `spec/Rchain/Casper/` holds exactly `Bonds`, `Dag`, `Fringe`, `Stake`, `Validate` — so this component's fidelity is anchored in `legacy/casper/src/main/scala/coop/rchain/casper/engine/LfsBlockRequester.scala` and nowhere else. **Why that settles the row rather than deferring it**: taking the cutoff needs the **truncated-ancestry DAG semantics** defined — what an absent justification *means* to `BlockDagStorage::insert`, to `finalized_blocks_set`'s seen-closure and to the finalizer's `seen.contains` gate — and a change that size has no law to be checked against; the register would have to gain a requester model first, which is the honest order of work. **Two measurements carried here rather than re-derived**: the requester's largest fixture is a **6-block chain** (`chain(6)`, `lfs_block_requester.rs`), so the §6 row's 6,300-round-trips-vs-~50 is a live/devnet figure with no unit-level before/after; and C64 already attributes the ~25 s per generation to the syncing node's shared message loop rather than to the requester's own loop, which paces faithfully (`the_walk_advances_on_responses_not_on_the_idle_timeout`: 3.85 ms for a six-block walk against a 30 s idle timeout). **Operator consequence: none today** — the port downloads a superset of the blocks it needs, every one of them valid, and the resulting DAG is the one the Scala's cutoff would also produce |
| C95 the model's fold can publish a message **below** the previous fringe, and fork-freedom does not exclude it — so a fringe-level monotonicity claim needs the *sequence* rule for a hypothesis | 15 | **found by trying to lift the sentinel theorem through the derivation's fold and refuting it instead** (2026-09-25, law 15). The sentinel half is proved (`selfParents_above_a_finalized_ancestor`: the walk's output is strictly above every finalized same-sender ancestor of the justification, under the fork-freedom H-1 gives at ingress), and the next step is the derivation's own fold — `nextLayer` adds the min messages' **candidate parents** whose sender is already a key (`Rchain/Casper/Dag.lean:129-135`). That step is not covered by fork-freedom, and the counterexample is machine-checked rather than argued: `fold4` is three messages, two sender-0 blocks `99` (height 2) and `100` (height 5) with **no** parents — a forest, which `NoFork` allows — and `101` (height 6) justifying `99`; with `100` finalized (it is the previous fringe's sender-0 message) the walk stops at `99` and the fold publishes it, so `the_fold_can_publish_below_the_previous_fringe` holds of a DAG for which `fold4_is_fork_free` holds too. **And the obvious next guess is refuted too, which is why this row names no hypothesis for the lift.** `fold5` adds `z` (103, sender 0, seq 6) justifying `q` (100, seq 5), so every same-sender edge in it has consecutive `seqNum`s — `fold5_satisfies_the_sequence_rule` — and it is fork-free as well (`fold5_is_fork_free`); it *still* publishes below the previous fringe (`the_sequence_rule_is_not_enough`), because `cands` is folded right-to-left and a candidate from an earlier justification (`c`, the lower sender-0 block that `p` justifies) wins the sender's slot over the min message. So **fork-freedom and the sequence rule are both excluded by machine-checked instances**, and the earlier sentence here — that the sequence rule was the hypothesis — is corrected by `fold5` rather than quietly dropped. **And the third candidate does not survive reading the code either — so this row states a question rather than a hypothesis.** `check_justification_regression` (`casper/src/validate.rs:205-241`) is *not* "the justification must be the sender's latest": it takes the block's own same-sender justification's parents and requires each justification of another sender to have seen at least what that sender's *previous* message saw (`just_prev_msg.seen ⊆ just.seen`), and it **returns `Ok(None)` — no verdict — when the block has no same-sender justification at all**, which is exactly `fold5`'s `p` (sender 1, no sender-1 justification). So whether the port *admits* `fold5`'s shape is not something the model can settle, and that is the open question this row leaves: if it does, the derivation can publish a sender's message below the previous fringe and law 15's per-sender reading is false of the *published* layer rather than merely unproved; if some other ingress rule excludes it, that rule is the lift's hypothesis. **Operator consequence, stated as a conditional rather than as a defect**: on such a DAG the port's own finalization would advance a fringe whose layer lies below the previous one for one sender, and the consumer of that layer is law 14a's support gate — the model here does not claim the port is exploitable, only that the exclusion is not in the rules this row can see. **What this row has cost, and why it is worth the three corrections**: two guesses at the lift's hypothesis and one at the port's rule were each refuted within the hour by writing the counterexample or reading the code, instead of by a proof attempt that would have stalled with no indication of which premise was wrong. **And the counterexamples are about the *model*, not the port — which is the correction this row exists to make.** The port's `calculate_next_layer` does **not** let a candidate take a sender's slot unconditionally: it replaces the entry only when the candidate's `sender_seq` is strictly greater (`block-storage/src/dag/finalizer.rs:119-125`), and the port's own comment names that guard — `sender_seq` replaces the sender's latest message, *"the Law 15 monotonicity invariant"* (`message_state.rs:77`). The model's `layerInsert` drops it, with a comment that is right about what it claims and silent about what it costs: *"the antichain does not depend on that choice"* is true of law 14b's **keys** and false of the **identity and height of the published message**, which is exactly what law 15's comparison is about. Under the port's guard `fold5` publishes the min message `z` (height 6) rather than the lower candidate `c` (height 2) — no violation — so both refutations are refutations of **the model's simplification**, and the earlier sentence here that they pointed at an unexcluded port behaviour was wrong. **What that leaves is a concrete modelling step rather than an open question**: the fold must carry the port's guard — including how the *seeding* collapses two same-sender min messages, which the port does by `BTreeMap::collect` (last wins) and the model by prepending — and then law 15's published-layer comparison can be stated at all. **Operator consequence: none** — the port's guard is the invariant, and what this row now records is that it is load-bearing rather than decorative. |
**The two rows that are not laws are the two worth keeping visible.** C37 is a *harness* finding —
a measurement that was not a measurement — and no law would have caught it, because the thing that
was wrong was outside the term. The retired static walk over vendored text (AUDIT §17 C22) is the
other: it was drafted, found unable to tell a terminal consume from a deferred restore from a
permanent loss, and retired unshipped rather than shipped with an exception list. A catalogue that
claimed to cover them would be claiming something false.
