# The audit's pass record

<!-- Split out of `spec/AUDIT.md` on 2026-09-27. This is the evidence, kept word for word: one
     section per audit pass, in the order the passes ran. The check-off a reader wants first is
     `spec/AUDIT.md`; this file is what its rows cite. Do not reorganise it -- `§N` is cited from
     `spec/`, `docs/` and the source tree, and the pointer check in `tools/audit-test-register.sh`
     resolves every one of them against a heading here. -->

`spec/AUDIT.md` records what each finding came to; this is where each was found, why it was
believed, how it was falsified and what the fix does not do. The sections are the four audit
dimensions's passes in the order they ran.

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

**Re-measured, and changed in kind rather than in digits (2026-09-26, C98).** "Drift" is not a
tolerable property of a number a reader might act on, so the counts are now a **ratchet**: the gate
compares each class to `tools/type-system-baseline.tsv` in *both* directions and fails on either, so
a site cannot arrive or leave without the same commit saying so. Three things changed with it. (1)
**`get`'s 101 was not what the class was named for**: measured, its accessor half (`.get(..).unwrap()`,
`.next().unwrap()`) is **0** and its literal-index half is all 95 of a 95-site class on this tree — the
class was named for what it barely contained. (2) The literal-index half is now the `index` class,
which sees **313** sites because it also matches variable indices and slice ranges — the class the
2026-09-26 audit found the gate could not see at all. (3) `div` (49) and `overflow` (41, scoped to the
eleven refinement files, where an unguarded `a - b` is a refinement leaving its own domain rather than
a design choice) join them. The current numbers are the baseline file's, by construction; they are not
restated here, because a count written twice is a count checked once.

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
| 4 | `casper/src/runtime_manager.rs:516-523` | `u64::try_from(cost.value).unwrap_or(0)` — a negative gas cost silently coerced to 0 | reject negative cost (`map_err`); `PCost.cost` is a `uint64`, so a negative (over-charged) cost is an accounting anomaly |
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
Noted low-severity: `PBKDF2_ITERATIONS = 1024` (`crypto/util/key_util.rs:25-31`, local-only).
**Raised since**, to `310_000` — the OWASP floor for PBKDF2-HMAC-SHA256 — and recorded as a documented
deviation in the constant's own doc, with the interop caveat that a key written at one count cannot be
read at the other. The `1024` above is what this pass found, not what the tree says now.

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
| rholang depth guards (`MAX_PARSE_DEPTH = 128` on the parser's own recursion, `MAX_CHAIN_LENGTH = 512` on one flat chain, `MAX_AST_DEPTH = 768` on the term) | Scala BNFC parser has no depth guard | a deeply-nested term must not overflow the stack (R9). **The first version of this row said `MAX_PARSE_DEPTH = 512`, which was never its value** — 512 is the chain limit, and the depth limit has always been 128 — and the two bounds do not compose, which is AUDIT C99. Row corrected 2026-09-26 |
| `MAX_VALUE_DEPTH = 256` on a **produced value** (`rholang/src/storage.rs`, both produce paths) | the Scala has no such bound | a *runtime-built* value aborts the process inside `eval_single_expr`'s recursion at a depth of ~401 — a few hundred bytes of source, before any phlo or balance check — and no parsed term is involved (AUDIT C100). **Hard fork:** a deploy that sends a value nested deeper than 256 evaluates on an old node (up to ~400, where it aborts) and is **refused** by the new one, so a chain upgrading in place diverges on such a deploy; lockstep upgrade is the practice, and this is the row that says so. The number sits deliberately below the parser's 768 because the two walks have different frames: 256 is measured against the value route's own abort (~401), while 768 is measured against the normalizer's (~1,000) |
| `if`'s condition is normalized against an **empty** par (`normalizer.rs::normalize_if`), as `match`'s target is | `PIfNormalizer.scala:24` passes the caller's `input` through, so the target becomes `<the par before the if> \| <condition>`; `PMatchNormalizer.scala:28` — the *same* desugaring — passes `input.copy(par = VectorPar())` | the Scala contradicts itself: its `if` and `match` desugarings of one construct normalize the target differently, and only the `if` path's version is broken (`if E {A} else {B}` = `match E {true => A; false => B}`, and a process's meaning cannot depend on what precedes it in a `par`). Under the Scala's `if` path every non-first `if` is a silent no-op — see **C21**. The port follows the spec and the Scala's `match` path. **Hard fork:** the normal form of any term whose par holds a non-first `if` changes, so a chain that ran the old rule and upgrades diverges on such a deploy (and on genesis, where the effected normal forms *are* genesis content: `ListOps.rho:203,242`, `MultiSigRevVault.rho:155`, and the rgov family) |
| HTTP `/api/deploy` + explore routes rate-limited (100 req/s) | Scala HTTP deploy routes are unlimited | match the gRPC deploy rate limit (R10) |
| PBKDF2 iterations raised `1024 → 310_000` | Scala uses BouncyCastle default `1024` | slow offline brute-force of encrypted keys at rest (R11) |
| `BindPattern.freeCount` **rejected** when negative at the proto boundary | `RhoTypes.proto:124` — `int32 freeCount`, signed, so negative is representable; the vendored tree does not carry the Scala's `BindPattern` proto conversion, so whether the oracle refuses it there could not be established | the count is not descriptive: `RhoMatch::get` fills `0..free_count` from the match's free map, so a negative count silently applies the continuation with *no* bound values (and an over-large one pads with `Nil`). The port's two siblings on the same field family — `ReceiveBind` (`:118`) and `MatchCase` (`:165`) — already validate it, so this is also what makes the three uniform. A refusal at the declared boundary rather than a clamp (AUDIT C52) |
| `New.bind_count`/`Receive.bind_count` are a **`FreeCount`** carrier, and a negative count is **rejected at the proto boundary** | `RhoTypes.proto`'s `int32 bindCount` on both messages; the Scala's `New.scala`/`Receive.scala` carry it as a signed `Int` and neither tree validates it, so the oracle carries a negative count into the term | the count is not descriptive: it is a `well_scoped_par` depth, a sort-key leaf (`sorter.rs`'s `leaf_i64`), the extent of `env.shift` in substitution and the argument to `new_bindings_cost`, and a negative value is meaningless to all four. The port used to *clamp* it where it was used (`.max(0)`, `types.rs:436,440`), which hid the malformed message and left a term whose sort key carries a value no rule defines. Refused where it arrives — the same validate-on-ingress rule as the sibling `free_count` fields (`ReceiveBind`/`MatchCase`, C52) and `BindPattern.freeCount` (the row above) — and carried by `FreeCount`, so the sign cannot be *written* from Rust either. **Stricter than the oracle** (the Scala validates nothing there), and **not a hard fork**: the two counts the normalizer derives go through `checked_level_count`, so no deploy produces a negative one and only a hand-built or wire-injected message can. Falsified first: `a_negative_bind_count_is_refused_at_the_wire_boundary` fails against the old pass-through |
| The store-items **server** logs and **drops** a request whose store cannot be read, where the oracle's page build fails the request | `rspace/.../exporters/RSpaceExporterItems.scala:28,52,76` — the page is assembled from `exporter.getNodes(startPath, skip, take)` inside `F`, so a store error propagates to the caller and the request fails | the port's handler (`casper/src/engine/node_running.rs`'s `handle_store_items_request`) has no error reply to send: `StoreItemsMessage` is the only response type, so its choices are a *state claim* (an empty or short page, which the requester then validates its own traversal against) or a drop. It drops — a timeout is retryable and a wrong answer may be accepted — which is also the policy the same function already applies to an over-large `take`, so it is consistent rather than novel. **The consequence is named**: the requester retries or times out and never receives a page that under-reports the state. Everything *inside* the page assembly is fallible in the oracle's shape now (`get_history_and_data` returns the store's error), so the drop is confined to the one place that cannot reply |
| RSpace candidate selection is **sorted-first** by content hash (not newest-first insertion order) | `RSpace.scala`/`RSpaceOps.scala` shuffle candidates via `Random.shuffle` before matching | live Scala is non-deterministic across runs; the port selects the sorted-first candidate for consensus. Implemented per `docs/src/node/sorted-matching.md` (changes post-state hashes only for multi-candidate deploys) |
| Block validation replays dependency-free blocks **concurrently** (per-block forked `ReplayRhoRuntime`, batch processor), then inserts serially | Scala `BlockProcessor` validates one block at a time | replay is verify-only (Law 11), so concurrent re-validation does not change the committed state — a throughput optimization, not a semantic change. See `docs/src/formal/concurrency.md` |
| LFS sync inserts the downloaded blocks in **ascending** height order (parents before children) | `NodeSyncing.populateDag` `heightMap.flatMap(_._2).toList.reverse` | `BlockDagStorage.insert` requires every justification to already be in the message map, and a justification is always at a strictly lower height; the Scala `reverse` inserts the newest block first and fails with "justification not present in message map" for any fresh observer |
| LFS block requester downloads the **full ancestry chain to genesis** (no `lowerBound`/`extraHeights` cutoff) | `LfsBlockRequester.ST` `lowerBound`/`extraHeights` + `NodeSyncing` `blockHeightsBeforeFringe = deployLifespan` (`populateDag` `minHeight` filter) | a syncing node's DAG is always empty (`NodeLaunch.apply` only syncs when `dagSet.isEmpty`), so the Scala cutoff stops ~50 blocks short of genesis and leaves the lowest downloaded block's justification dangling — the same "justification not present" failure once the fringe is past `deployLifespan`. **Measured consequence (2026-09-24, AUDIT C64)**: on a chain the walk is one generation — one round trip — per block, so this deviation is 6,300 round trips where the oracle's bounded walk makes ~50, and a fresh validator was observed at ~1.5 blocks/minute (~70 hours for that chain). The *pacing* is faithful and measured (`the_walk_advances_on_responses_not_on_the_idle_timeout`: 3.85 ms for a 6-block walk against a 30 s idle timeout); the per-generation latency is the syncing node's, not the requester's **Disposition (2026-09-24, R3): an efficiency divergence with a measured cost, not a correctness one.** The oracle's `lowerBound`/`extraHeights` and `blockHeightsBeforeFringe` are a *cutoff* — they stop a walk that would otherwise continue past what the DAG already holds — so the port downloads a **superset** of the blocks it needs, all of them valid, and the resulting DAG is the same. What differs is the traffic, and C64 measured it: **6,300 round trips where ~50 would do**. So the row records a divergence the oracle *bounds* rather than forbids, with its cost named. **Disposition (2026-09-25, settled rather than deferred): a deliberate divergence, kept.** The port walks the full ancestry on purpose — `BlockDagStorage::insert` requires every justification present and a syncing node's DAG is empty, so a cutoff would have to invent a boundary the port does not have, and the Scala's own cutoff leaves the lowest downloaded block's justification dangling once the fringe is past `deployLifespan`. Taking it is therefore not an open efficiency unit but a change that would need the truncated-ancestry DAG semantics defined first — and **nothing in this register would catch a wrong one, because the oracle has no model of the block requester at all** (AUDIT C94: no `lowerBound`, `extraHeights`, `requestStream` or `LfsBlockRequester` occurs in any `.lean`). **CORRECTED (2026-09-27, AUDIT C64): the premise above is false, and the deviation does not exist.** The oracle's cutoff is **inert in its own production path**. `LfsBlockRequester.scala:125` declares `lowerBound: Long = 0` on `ST.apply`, and the **only** construction of the block requester's `ST` in the tree — `:318`, inside `stream()` — passes `initialHashes`, `latest = finalizedHashes` and `extraHeights = blockHeightsBeforeFringe` and **never passes `lowerBound`**. So `minimumHeight` is `0` for the whole sync, and both gates are `>= 0` and vacuously true: `blockIsAccepted = isReceivedLatest || isReceived && blockNumber >= minimumHeight` (`:228`) and `NodeSyncing.populateDag`'s `blockHeightOk = blockHeight >= minHeight`, which is fed `st.lowerBound`. `extraHeights` is `deployLifespan` (`MultiParentCasper.scala:35`, `val deployLifespan = 50`) and is not a bound on the walk at all: it appears only as `max(0, min(height - 1, lowerBound) - extraHeights)`, which *reduces* the bound further, i.e. lengthens the walk. The `lowerBound` machinery is exercised by nothing but `LfsBlockRequesterStateSpec`, which passes `lowerBound = 200` itself — so every number in this row that treats `~50` as the oracle's walk length ("stops ~50 blocks short of genesis", "~50 blocks below the fringe", "6,300 round trips where the oracle's bounded walk makes ~50") compares the port against a cutoff **the oracle never applies**. Both trees walk the full ancestry to genesis; the port is **faithful**, and AUDIT C64's `owes` ("adopt the Scala's `lowerBound` cutoff") would have been a **no-op** — or, seeded, a behaviour change *away* from the oracle. What remains true is the measurement this row already carries: a fresh validator catches up slowly, and C64's own account attributes it to the syncing node's shared message loop (~25 s per generation), not to the walk's extent. **Witness**: `a_walk_longer_than_deploy_lifespan_reaches_genesis` (`casper/src/engine/lfs_block_requester.rs`) walks a 51-block chain and requires the genesis block to be fetched; planting the cutoff reddens it and two older tests, so the port's full-ancestry behaviour was already multiply pinned. This also discharges the "nothing would catch a wrong cutoff" clause below: the port's side of it is now witnessed |
| The store-items **server refuses** a page over 32 MiB (drop, no reply) where the oracle serves whatever it is asked | the Scala's `handleStoreItemsRequest` has no byte cap; its only bound is the requester's own `PAGE_SIZE` | the requester's `PAGE_SIZE` self-consistency is not a defence *for the responder*: `validate_state_items` requires the received keys to match the page the requester recomputes, so a responder that truncates is caught while one that is asked for 10,000 fat nodes pays tens of megabytes per request, per peer, in memory it must assemble before it can measure it. Registered with its cost: the assembly is still paid, the bandwidth and the per-request ceiling are not (AUDIT C73) |
| Active validator set = a **uniform draw without replacement** from the eligible pool, seeded one epoch ahead | `Pos.rhox:718-726` `pickActiveValidators` returns the first `$$numberOfActiveValidators$$` entries of `allBonds.toList()` — *key* order, not stake order; its own comment is `// TODO: Randomly select 100 active validators once we have on-chain randomness` | the contract's rule is not a rule to port: "the first N in map order" is a placeholder for a random selection, and it makes the consensus set depend on map iteration order rather than on anything consensus-relevant. The cap exists because finality's supermajority is stake-weighted, so membership decides who can finalise; the port's first rule was therefore deterministic and stake-ordered. **That rule was replaced by the contract's own intent (2026-09-28, `spec/RUST-VS-SCALA.md` §3 item 12)**: `select_active` now draws `N` uniformly without replacement from the eligible pool in its canonical `BTreeMap` order, seeded by a `pos:epoch_seed` leaf written **one boundary ahead** from the state hash of the last **finalised fringe** — the >2/3-agreed frontier, which a lone proposer does not move — so the block that draws is not the block that chose the entropy. Why it changed: the rule's seed was `hash(shard_id, block_number, sender, pre_state_hash)` computed at the moment of use, so the drawing proposer could reroll freely by proposing a different block. **The residuals are named in §3 item 12** — the seed-setter's influence (O1, reduced rather than closed: a proposer can present a stale fringe, so the steering space is the distinct fringes its candidates induce, normally one), pre-positioning (O2), the sybil exposure uniform sampling carries where stake-weighting does not, in the weight set and in the rewards (O3), and the fluctuating security budget (O4) — and the uniform rule is the one decision in that item worth revisiting; weighted sampling is a one-function swap. Its consumer sweep is recorded there too: three readers of the active set make a *decision* on it (`validate::neglected_invalid_block`, the proposer's own `check_active_validator`, and the finaliser's pruned-history fallback), and the draw narrows what each covers. The *timing* is unchanged — both apply it only at an epoch boundary (law 44) |
| A slashed validator that had a **staged withdrawal** is removed from the pending map | `Pos.rhox:491` leaves it in `pendingWithdrawers` while zeroing its bond (`:487`), so at the next boundary `movePendingWithdrawer` files it under `withdrawers` with a zero amount, where it stays forever (never paid, never removed — nothing deletes a zero claim) | the payable outcome is identical (zero), and the port does not carry a permanent tombstone: `slash` removes the validator from the pool, the active set, the claim map *and* the pending map. Recorded because it is a state-shape difference a reader would otherwise meet as a missing entry |
| Epoch length `0` (the port's permissive default) means **every block is a boundary**; the contract's `%`/`/` by it would fault | `Pos.rhox:517` `blockNumber % $$epochLength$$`, `:381` `blockNumber / $$epochLength$$`, `:249` `bonds / $$minimumBond$$` | the contract cannot express a zero epoch length or a zero `minimumBond` — it divides by both. The port's default parameters have both at zero (ad-hoc runtimes and tests install no genesis PoS state), so the port defines what the contract leaves as an arithmetic fault: a zero epoch length is the one-block epoch `epochLength = 1` means, and a zero (or zero-normalising) minimum bond pays a reward of zero rather than faulting the block. The Lean model states its conservation theorem for the defined case only (`Rchain.sum_rewards_le_pot`'s hypothesis `0 < activeBonds / minimumBond`), which is the same boundary |
| `deploy-status` has **no `Running`** state: a deploy the node is executing right now answers `Pooled`, or `Unknown` once the pool no longer holds it | `BlockApiImpl.scala:218-224` — `findCurrentlyExecutedDeploy` reads the node's `BlockExecutionTracker` (a per-node cache of the deploys its own block creator is executing) and answers `notProcessed("Running")`, and `findPooledOrRunningDeploy` consults it after the pool | the port has no execution tracker: deploys are pulled out of the pool into `compute_deploys_checkpoint` with no in-flight record, so there is nothing to look up. A derived answer would be a guess — "neither in the DAG nor in the pool" also describes a deploy whose block was just proposed and one dropped with its block — and the window is the length of one proposal. Registered rather than invented: the states a client can act on (pooled, processed with success, processed with error, unknown) are complete, and `Running` only ever meant "the node you asked is busy with it right now" |
| `metrics { prometheus, influxdb, influxdb-udp, zipkin, sigar }` (and the matching `--prometheus`/`--influxdb`/`--zipkin`/`--sigar` flags) are **parsed and never consulted** | `kamon.conf` — where `prometheus { enabled = false }` gates Kamon's scrape endpoint, and the influxdb/zipkin/sigar blocks configure reporters that push | nothing outside `node/src/configuration/` reads `MetricsConf`: the node always serves `GET /metrics` in Prometheus text format, there is no InfluxDB or UDP sender, and the tracing/span backends are out of scope (`diagnostics/mod.rs`, said there). So the switches are inert in both directions — `prometheus = false` does not disable the endpoint and `= true` does not start a reporter. Said once in `defaults.conf` where an operator reads the switch, and recorded here as the second config surface found doing nothing (`disable-state-exporter` was the first, wired in `687fc4b30`) **Disposition (2026-09-24/25, R3 then `bf44e5fc3`): the four reporters it cannot honour are refused at startup, and `prometheus` is accepted with a note.** The node now exits 1 naming each unimplemented reporter — `Configuration error: unimplemented metrics reporter(s) enabled: influxdb. This port has no InfluxDB sender, no InfluxDB UDP sender, no Zipkin span reporter and no Sigar collector, so these settings would report nothing — refused rather than accepted and ignored. GET /metrics serves Prometheus text and is always on; unset the setting to start.` — because an ignored knob is exactly the failure this pass keeps recording. `prometheus` is accepted **ungated**, with a note that the endpoint is always on rather than gated, since the endpoint *is* implemented (`web::http`'s scrape tests pin it). Two facts decided the shape: the config parser **ignores unknown keys** while clap **rejects unknown CLI flags**, so *deleting* the fields was safe for config files and breaking for command lines — which makes keep-and-refuse the only honest route; and the refusal can fire only for an operator who explicitly asked, since `defaults.conf` sets all five false and the oracle's defaults are false too |
| Vaults are a **balance map keyed by REV address**; `transfer` takes the caller's `deployerId` rather than a minted purse | `RevVault.rho:103-140` — `findOrCreate` → `_makeVault` returns a `MakeMint` **purse**, and `transfer`/`deposit`/`getBalance` are called *on the purse* with an `unforgeableAuthKey` | the *spend* rule holds either way: the port derives the `from` account from the caller's unforgeable `deployerId` (`system_processes.rs`'s `transfer`, "capability, not data"), so no deploy can spend another key's vault — **conditional on the `deployer` field being authenticated, which is what C120 found missing and this row used to assert unconditionally.** It held at the deploy *ingress* (where `SignedDeployData::verify_signature` ran) and not on the *block* path, where a peer's `ProcessedDeploy` carries `deployer` and `sig` copied verbatim off the wire; a bonded proposer could therefore name any account and have every validator charge it. The block path now runs the same check (`casper/src/validate.rs::deploy_signatures` in `block_summary`'s pure list), so the claim is unconditional again — and it is a claim about *two* paths now, not one. What is unforgeable is the deploy's identity rather than a minted name. What the simplification **loses** is *delegation*: a purse could be handed to a contract that then spends from it, and here only the signing key can spend. **That half is no longer lost (2026-09-27): `findOrCreate` returns a minted handle, and a handle spends.** `Dispatch::register` and `Tuplespace::install` let a native handler bind a freshly minted name; the name is drawn from the send's own RNG, so replay mints the same bytes; and `transfer_vault` is now one spend rule for both the `deployerId` arm and the handle arm. The paragraphs below are kept as the record of the decision this supersedes — the reasoning is still the reason the *classic* shape is unchanged. **Decided 2026-09-23** (Programme B item B2) to keep it: nothing in the tree delegates (the wallet, the faucet, the gateway legs and the genesis ceremony all act as the key itself), and landing it needs the deploy's RNG threaded into a native call — the minted name is a `new`, so it must be replayable — plus a leaf and a client-visible API change. The *reply-shape* half was recorded where a client meets it: `spec/API-SCHEMA.md`'s `rho:rchain:revVault` row — **❌ open when this was written and ✅ since 2026-09-27**, carrying the minted-handle shape beside the classic one, so the deviation this sentence points at is closed rather than pending. What remains of C114 is the sealer/unsealer path's **coverage**, not a shape |
| Storage is **refunded** when a produce/consume matches (the gas a matched op costs) | `ChargingRSpace.scala:105-127` — `refundForConsume` and `refundForRemovingProduces`, charged as negative `Cost`s *before* the event and COMM costs | the port charged the storage and never refunded it, recording the gap as a "safe over-charge"; the refunds are restored (law 49, `spec/Rchain/Charging.lean`). **Hard fork:** the recorded `PCost` of a deploy that matches changes, and cost is part of the block's state — on a chain that accepted such a block the old node recorded a *higher* cost than the Scala's, so the old value was already wrong, which is why this is a fix and not a behaviour that was ever correct. The order matters as much as the amounts: a refund credited after the exhaustion check cannot save a deploy that has already run out (`peak_refunds_first`), which is why the Scala charges them immediately |
| The Coop **multisig public keys** are read as the port's initial **trusted stakeholder** set | `Pos.rhox:122-128` creates the Coop multisig vault from those keys (and `$$posMultiSigQuorum$$`), and `:470-482` sends slashed stake to it; the Scala has no trusted-stakeholder concept at all — that is this port's extension for observer admission (`spec/RUST-FIRST.md`) | a mapping, not an accident (`06bf01a7f`, and the doc comment on `build_pos_genesis` says it): the Coop multisig is the network's governance body in the contract, and admission is the port's governance-shaped hook, so the closest analogue of "who may govern validators" is the keys the contract gives governance to. The alternative reading — that they are only slashing-vault owners, leaving the genesis `trusted` set as the validators alone — is equally supportable, and nothing in either tree says which was intended. Recorded because the consequence is silent: an operator setting these keys for vault control is also granting admission rights. The `--pos-multi-sig-quorum` option has no counterpart at all (no multisig vault), so it stays "Reserved" in its help text **Disposition (2026-09-24, R3): the oracle is silent, and the row should say that rather than that a choice was made.** `Pos.rhox` creates the Coop multisig from those keys and sends slashed stake to it; the Scala has no trusted-stakeholder concept at all, and **nothing in either tree says which was intended** — so the two readings are equally supportable and the row is the record of that, not of a preference |

| A pattern that binds one free level **twice is refused by the matcher** (`spatial_match`'s entry, `linear`), where the Scala silently merges the repeated binding | `SpatialMatcher.scala:144,209-211` — `handleRemainder` does a plain `insert`, so `@[v, v]` against `[1, 1]` matches with one binding winning right-biased | law 5 says a pattern binds each free level **at most once**, and the model states it as the matcher's entry condition (`spatialMatch` = `spatialMatchCore … && linear pattern`, `Match.lean:369`); the port now does too (AUDIT C42). The Scala's merge is why the finding was latent rather than visible: the repeated binding is *consistent* here, so the overwrite was silent. Not reachable from source either way — the normalizer refuses a twice-bound binder first (`normalizer.rs:111,289,590,1325`) — so this closes a silent-overwrite path rather than changing what any deploy does, and it is **not** a hard fork. Falsified: with the guard bypassed, `@[v0, v0]` against `[1, 1]` matches and `law5_a_pattern_that_binds_a_variable_twice_never_matches` fails |
| **The finalizer's progress guard**: a non-advancing fringe is dropped from the advance chain (`if nf == current { break }`, `block-storage/src/dag/finalizer.rs:361-363`) | `Finalizer.scala:175` — `LazyList.unfold(parentFringe)(nextFringe(_).map(nf => (nf, nf))).lastOption`, which compares fringes **never** (the gate is the support map) | the oracle would not terminate on a repeating fringe, because `lastOption` forces the whole lazy list; the port must not diverge. **Weaker than a cycle guard as a *guard*, but the walk is bounded by the chain, which is why the weaker shape is enough** (settled 2026-09-24, C69's second half): each iteration passes the previous fringe as the cutoff and `self_parents` stops at it, so for every sender the minimum message can only move along that sender's own unfinalized chain — finite and acyclic — in one direction, and a step that is not the fixed point moves strictly. The walk therefore terminates in at most (messages on the longest unfinalized chain) steps, and a cycle of *any* length would have to move strictly forever on a finite chain. Measured rather than argued: `the_fringe_walk_is_bounded_by_the_non_finalized_chain_length` drives the loop over a generated `L`-layer fork — **6 steps on 8 layers, against a 25-message bound** — with the assertion inside the loop, so a hypothetical cycle fails the test rather than hanging it. The guard stays as the belt for the fixed point (`nf == current`), not as the thing preventing divergence. Found 2026-09-24 with C69, which is the same reading
| **Equivocating blocks are refused** — at `insert` (H-1) and at restore (H1c) — where the oracle has no such gate | `legacy/block-storage/.../dag/BlockMetadataStore.scala:118-124` — `validateDagState` asserts only that the height map's numbers are contiguous, never `(sender, seq_num)`, so a forked store restores silently; and the Scala tree contains no equivocation check at all | an equivocating validator can neither enter the DAG nor stall finalization — the H-1 stall is a liveness failure the Scala admits, so the premise law 15's proof needs is **guaranteed here and only observed there** (AUDIT C84, and H1a's note) |
| An **empty window** in `visualizeDag` renders an empty graph where the oracle raises | `legacy/casper/.../api/GraphGenerator.scala:39` — `timeseries.head` on a `List` built from a `Set` throws on empty | the endpoint is a *view*, and a graph of nothing is the honest rendering of an empty DAG; refusing would turn a visualization request into an error (AUDIT C85) |
| **A resolved parent at or above the block's number is refused** — the **failed** ones included (H1b, `casper/src/validate.rs:152-154`, test `h1b_a_failed_parent_above_the_childs_height_is_refused` at `casper/src/dag.rs:1287`) | `legacy/casper/src/main/scala/coop/rchain/casper/Validate.scala:178-198` — `blockNumber` maps the justifications through `lookupUnsafe` and then `.filter(!_.validationFailed)`, so a failed parent is discarded before the maximum and **no resolved parent's height is compared against the block's number at all**; it admits the block | `Descends` (`spec/Rchain/Casper/Dag.lean:296-297`) is the premise law 15's proof consumes, and the laws are the port's oracle where they outrank the reference — a premise the port *guarantees* is worth a refusal a byzantine peer can trigger and **no honest proposer can**: justifications are `latest_msgs.values()` (`multi_parent_casper.rs:293-300`) and a failed block never enters `latest_msgs` (`block-storage/src/dag/message_state.rs:118-128`, the H-2 exclusion), so the operator consequence is a validator-side divergence on byzantine input only (AUDIT C82, C83) |
| The block path **verifies every deploy's signature** — `validate::deploy_signatures` in `block_summary`'s pure list, refusing with `InvalidDeploySignature` before any replay | `legacy/casper/src/main/scala/coop/rchain/casper/Validate.scala:92-116` — `blockSummary` validates the deploy's shard, window and dedup and **never its signature**; `legacy/models/src/main/scala/coop/rchain/models/NormalizerEnv.scala:33-36` binds `deployerId` from `deploy.pk` with nothing having checked it either. So the oracle has the *same* defect on both paths, which is why this row records a **shared defect** rather than a port divergence — the port simply closes it in the stricter of the two trees | a deploy's `deployer` is the field the replay reads to decide whose vault is charged and paid, so an unauthenticated one is an authorization claim rather than a malformed datum: a bonded proposer could name any account, put arbitrary bytes in `sig`, and have every validator debit that account and pay the proposer's term, with a post-state hash the proposer computed honestly — the block was **valid and unattributable** (AUDIT C120). The check belongs in `block_summary` beside `phloLimit`, for the reason that list's own comment gives, and it is the second instance of the port choosing to be stricter there than the oracle (the first is that `blockSummary` here checks `phloPrice` and `phloLimit` at all, which the Scala does not). **Hard fork:** a block whose deploy signature does not verify was accepted before and is refused now, so a chain upgrading in place diverges on such a block — and the honest reading of that is that only a proposer which *forged* the deploy could have produced one, which is the point of the refusal rather than a cost of it |
| The node's own block metadata **carries the `slashable` flag** (`BlockMetadataProto.slashable = 22`), where the port dropped it | — (no Scala counterpart: `BlockMetadata.slashable` is this port's own distinction, and the Scala has neither the field nor the rule that reads it) | the flag is the input to C110's slash rule, and the port hard-coded it to `false` in `from_proto` while writing it into every stored metadata — so the rule had no reachable input, a proposer's `to_slash` was always empty, and the **receiving** side refused *every* `Slash` as unjustified (`slash_is_unjustified` is `!slashed.is_subset(&justified)`, and `justified` was always empty). Carrying it restores the economic consequence of an attributable failure, which C111 left as the only seizure rule in the tree (AUDIT C122). **Hard fork:** a proposer on a fixed node may include a `Slash` an unfixed one would not, and a fixed node accepts a justified `Slash` that an unfixed one refuses with `UnjustifiedSlash` — so a chain upgrading in place diverges on any block containing one; lockstep upgrade is the practice, and this is the row that says so. The metadata is node-local and never on the wire, so the field itself changes no format and no state hash |
| `Secp256k1::verify_bytes` **refuses a message that is not the 32-byte prehash** (a named `PREHASH_LEN`), where the dependency truncates a longer one to its leftmost 32 bytes | `Secp256k1.scala` / `NativeSecp256k1` take exactly 32 bytes and the Scala's doc warns of an **assertion exception** on other lengths, so the oracle either asserts (a crash, if the JNI assertion is enabled) or its C++ truncates — the ambiguity is C134's and is unresolved in the oracle | the truncation made this function answer for a *prefix* of its message, which on `rho:crypto:secp256k1Verify` is a verdict a contract can receive for a message nobody signed. The port refuses: a defined `false` for an input that is not a prehash, which is neither the crash nor the silent truncation the oracle offers, and is the same preference this register records elsewhere — a refusal at the boundary rather than a value from a failure. Safe for every caller because `signature_hash` produces 32 bytes for `secp256k1` and `secp256k1:eth` alike (AUDIT C134). **Hard fork:** a deploy whose contract verified a suffixed message was answered `true` before and `false` now, so a chain upgrading in place diverges on it; lockstep upgrade is the practice, and this is the row that says so |
| **Native writes join the merge's conflict relation** (issue #83): two chains of different blocks that wrote a common native key conflict when neither block has seen the other, and depend on each other when one has; a native-writing block's chains are accepted or rejected together; the accepted writes are applied ancestors first (`NativeRelations`, `casper/src/merging.rs`) | — (no Scala counterpart: the Scala's PoS and vault state is tuple-space data, so `deploysAreConflicting` sees it through the event logs; the port's native state has no event log) | a native write is an absolute value from its block's own pre-state, so two concurrent writers can be neither concatenated (duplicate keys: every node panicked at the first epoch boundary with two sibling blocks) nor de-duplicated (two equal phlo charges write equal vault balances, and keeping one destroys the other's REV). **Behaviour change:** concurrent blocks that both write a native key - both boundary blocks at one height, or both charging phlo - now conflict, so one is rejected exactly as a tuple-space conflict would be, where before the merge panicked. Tests: `boundary_merge_tests` |
| **An empty `FinalizedFringe` is refused as a sync target**, and a finished LFS walk that received no block fails the attempt | `NodeSyncing.scala:124-128` — `startRequester.modify { case true if isValid => (false, true); … }` — latches on the **first** fringe from the bootstrap and inspects nothing about its contents, so it starts the sync on an empty one and `requestApprovedState` then reports the state restored | the genesis master **broadcasts** `FinalizedFringe { hashes: Vec::new() }` as it creates genesis (`node_launch.rs::create_store_broadcast_genesis`) — an announcement that the approved state *is* the genesis, not a sync target. A node already connected receives it **before** the answer to its own request: measured on a devnet, 34 ms after the announcement and 83 ms *before* the master had even seen the request, so the trigger was consumed by the announcement, the correct answer was discarded in silence (a later fringe from the bootstrap logs nothing at all), and the node logged `LFS state is successfully restored.` having restored nothing, then ran on an empty DAG and rejected every block it heard about (#100). Under the oracle's shape a fresh multi-validator network never forms at all. **Not a hard fork**: it changes which fringe a *joining* node acts on, not any block's validity, and no block or deploy changes meaning. One thing keeps the refusal narrow: the responder can never emit an empty fringe — both of its branches return at least one hash — so the only producer of one is the genesis broadcast, and this refuses exactly the input the oracle mishandles. **The companion guard is defence in depth, not the fix**: `run_approved_state_sync` also fails a walk that finishes with an empty `height_map`, which the empty fringe is the only way to reach, because `LfsState::received` writes a `height_map` entry only for a key it actually requested. Witnesses: `an_empty_fringe_does_not_consume_the_sync_trigger` (the regression pin — fails with the check disabled) and `an_empty_fringe_finishes_the_walk_at_once_with_nothing_in_it` (the premise, in the block requester) |
| `check_min_messages` requires the minimum-message **sender set** to equal the bonded set, where the oracle compares counts (issue #97) | `legacy/block-storage/src/main/scala/coop/rchain/blockstorage/dag/Finalizer.scala:64-66` — the identical body, and the identical TODO above it: *"add support for epoch changes, simple comparison for senders count is not enough"* | count-only coverage admitted `[A, A, B]` for bonds `{A, B, C}`: `calculate_next_layer` collapses the duplicate sender into one entry, so the published fringe **omitted bonded validator `C`** while presenting A's stake twice — 90 of 100 support on the contributing change's own fixture, a malformed fringe a byzantine proposer can present as a supermajority. **Stricter than the oracle, and not a hard fork**: the only newly refused case is equal count with a different sender set, and that case used to publish a fringe no honest node could derive from the same justifications — no block or deploy changes meaning. The oracle has *not* made this decision (the TODO is upstream's, open), so this row is a deliberate departure rather than a divergence by oversight, and the register's law 14a/14b rows plus `spec/Rchain/Casper/Dag.lean`'s `checkMinMessages` carry the same change. Falsified both ways: `check_min_messages_needs_all_bonded_senders` and `calculate_finalization_requires_exact_sender_coverage` fail against the count-only body, and the model's `the_gate_demands_the_bonded_senders` fails against the count-only model |
| **The finality gate's partition is the *live weight set***: `calculate_fringe` takes the set a candidate must have been seen by and the quorum's denominator as **separate maps**, and the node passes the bonded validators whose latest message is within `LIVENESS_WINDOW` (5) heights of the tip as the partition, and the whole bonded map as the quorum (issue #70) | `Finalizer.scala`'s `calculateFinalization` takes one `bonds_map` and uses it for both the full-partition filter and `totalStake`; there is **no liveness predicate anywhere upstream** — a validator that stops producing messages keeps blocking the fringe | with one map, a bonded validator that produces no message can never be "seen by every seer", so the partition is unsatisfiable and finality stops **whatever share of the stake the survivors hold**: measured on a three-validator devnet at `100/100/50` on 2026-09-29, the two survivors at 80 % of the pool did not resume finality after the third was stopped, and the same shape froze #105's two-validator chain with the survivor holding 91 %. **Shrinking only the *partition* is what keeps the safety property**: a quorum measured over the live set instead would be reached by *any* self-consistent subset — `3·F > 2·L` with `F ≤ L` over the live set is unconditional — so under a partition each side would finalise its own view and two conflicting finalisations would exist. "Finality needs quorum stake, not live nodes" is the requirement, and the denominator is what keeps it. **Hard fork** (#51 category A): which fringe is agreed changes, hence the merge base and every block hash after it. Falsified both ways by `a_silent_bonded_validator_does_not_cap_the_fringe`, and the predicate itself by the `liveness` unit tests |
| **The LFS block walk gives up when it stops completing blocks**: `request_blocks` fails after `MAX_IDLE_ROUNDS` (3) **consecutive** idle resend intervals in which `LfsState::finished` did not grow (`casper/src/engine/lfs_block_requester.rs`, issue #102) | `LfsBlockRequester.scala:309-312` — `requestStream.evalOnIdle(resendRequests, requestTimeout).terminateAfter(_.isFinished) concurrently responseStream`: the **only** termination condition is `isFinished`, so a fringe naming state no peer has retries forever. Nothing upstream bounds the walk: `requestTimeout` is the *resend* interval, not a deadline | the port did the same, and the consequence is #102's first defect rather than a theoretical one: `run_approved_state_sync` `join!`s the block walk with the tuple-space request, so a walk that never ends is a **sync attempt that never ends** — the spawned task never returns, `notify_when_restored` never fires, and the node sits in `NodeSyncing` for good **with a serving API and no error line**, which is the "silently stuck" class this register keeps finding. **Why a pace rule and not a deadline**: a long chain legitimately takes longer than any fixed duration, so a wall-clock bound would abandon a walk that is *long* rather than *stuck*; the quantity that distinguishes them is whether blocks are still being completed, and `finished` is monotone (`done` only adds, `add` refuses an existing key), which is what makes "did it move" well-formed — **Law 51a**'s `Drift`, the same shape as C171 and refused by the same `Paced`. **Pace, not a rate**: a slow peer that needs several resends per block is untouched, because the counter resets on every completed block. Falsified in both directions: deleting the give-up leaves the walk hanging (`a_walk_nobody_serves_fails_rather_than_hangs` reports #102's exact symptom after its 2 s harness bound), and deleting the reset abandons a progressing walk (`a_slow_but_progressing_walk_is_not_abandoned` fails on the second block) — the two mutations cannot both be satisfied by a rule that is not this one |

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

  **This fix was incomplete, and C99 is the measurement (2026-09-26).** `MAX_PARSE_DEPTH` bounds the
  parser's *recursion*, and a chain-length guard (`MAX_CHAIN_LENGTH = 512`) was added beside it — but
  the two do not compose: a flat operator chain is built iteratively, so it costs a bounded number of
  parser frames and produces an AST of depth `n`; nesting `d` levels of `c` operators gives an AST of
  depth `d × c` with both guards satisfied. Every consumer of the term then recurses over that depth
  (normalizer, sorter, `well_scoped`, evaluator, matcher, printer), and on the node's 32 MiB worker a
  debug build **aborts the process** between AST depth 732 and 994 — a ~4 KB deploy term. The remedy
  is `MAX_AST_DEPTH` + a measured walk (`exceeds_ast_depth`), which refuses the *product* the two
  guards leave free; the numbers, the curve and the constant's justification are in C99 and at the
  constant.
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

  **That closing sentence was false, and it is corrected rather than deleted (2026-09-26, C97).** A
  red-team audit traced peer bytes to `BlockHash::from_slice` through *four* paths this row's
  enumeration does not name: `HasBlockRequest`/`HasBlock`/`BlockRequest`
  (`models/src/casper/protocol/casper_message.rs`, each `pub hash: Vec<u8>`, cloned unchecked in
  `CasperMessage::from_proto` and asserted in the handler's **first** statement —
  `casper/src/engine/node_running.rs:166`, before the rate limiter, and `:644`/`:650`), and `Event`
  (`Event::from_proto` → `casper/src/event_converter.rs`'s `bytes_to_hash` →
  `Blake2b256Hash::from_byte_array`), reached during block replay of a peer's `BlockMessage`. The
  lesson is the one this register keeps relearning: **an enumeration is a claim about the sweep that
  produced it, not about the tree.** The seven types listed above were the ones the sweep looked at;
  the three hash-carrying request types and the trace `Event` were not, and the row certified the
  class anyway. The remedy is C97's, and the completeness half is now carried by a length refusal at
  each ingress plus a test per ingress rather than by a sentence.

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
| S25 | `node/web/http.rs` | `/api/v1/propose` `GET → POST`; admin CORS gated by `--api-enable-devnet-cors`; **admin HTTP binds loopback unless `api-server.enable-devnet-admin-public` is set** — *corrected 2026-09-27 (AUDIT C132): this row used to say the bind was `api-server.host` and "matches Scala", which was true before C112 and false after it, and the two rows contradicted each other and the code. The Scala's wildcard bind is the defect C112 records, not the fidelity this row should claim — `admin_bind_host` (`node/src/runtime/node_runtime.rs`) is the rule and `defaults.conf` documents it as C112's fix.*; report/replay routes rate-limited. |
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

- **R19 (C6/A2/E5) — `exploratory_deploy` reads the non-finalized chain tip.** `casper/src/api/block_api_impl.rs`'s `exploratory_deploy` (commit `5186361dc`) read `height_map.iter().next_back()` (first hash at max height, i.e. an arbitrary fork) instead of `last_finalized_block`. A byzantine tip block can spoof wallet `getBalance`/explore results, and forks make reads node-dependent. **Fixed** — restored `last_finalized_block` as the no-hash default; latest-tip reads remain available via `explore-deploy-by-block-hash` with an explicit hash. **That claim was false of this tree for a year, and the correction is C129's:** commit `63c535c6d` (2026-08-26) reverted it to the tip read — and never touched this register — so the paragraph above described an intention rather than the code, and every gate stayed green because nothing reads a register row against the tree. The tip read is reverted *again* as of 2026-09-27, deliberately this time (see C129's row for why the decision goes this way), and the choice is now pinned by a test rather than by this sentence. A row that says "Fixed" is a claim about the tree, not a note of what a commit intended.
- **R20 (A3/E6) — `revVault transfer` unchecked i64 add + self-transfer guard before the balance check.** `rholang/src/system_processes.rs` — `i64::from(to_balance) + i64::from(amount)` overflows on extreme balances; and the self-transfer guard (commit `204d98656`) returns success before checking `amount ≤ balance`. **Fixed** — `checked_add`/i128 accumulation, and the guard now sits after the balance check.
- **R21 (E3) — arithmetic panic on `EMult`/`EPlus`/`EMinus`/`ENeg`.** `rholang/src/reduce.rs:265,367,395,247` use raw `l*r`/`l+r`/`l-r`/`-hs` on `GInt`; `i64::MAX * 2` panics the reducer in debug builds. **Fixed** — `wrapping_*` (release wrap is Scala-faithful; the debug panic was not).
- **R22 (E4) — number-channel merge/diff unchecked i64.** `rholang/src/merging.rs:97,305` (`init_num + diff`, `end_val - prev`) wrap/panic and write a corrupted value into the trie. **Fixed** — `checked_add`/`checked_sub` with an error.
- **R23 (E2) — `slice` charges output length but walks input uncharged.** `rholang/src/reduce.rs:1264` (`"slice"`) — a recursive contract slicing a large string gets ~16M:1 op/phlo amplification. **Fixed** — `slice` now charges `max(from, until)` (the input walk), not just the output length.
- **R24 (F4) — SSRF filter classifies only IPv4 literals.** `comm/src/rp/handle_messages.rs:27-40` + Kademlia lookup-insertion (`kademlia_node_discovery.rs:45-50`) connect to attacker-chosen hostnames/IPv6. **Fixed (partial)** — `is_local_address` now classifies IPv6 literals (`::1`, `fe80::/10`, `fc00::/7`, multicast); hostname resolution remains a documented residual (DNS-rebinding-prone), and the Kademlia lookup-insertion path still needs the same filter.
- **R25 (F3) — global channel cache mutex held across an unbounded connect.** `comm/src/transport/grpc_transport_client.rs:68-75` (`create_channel`). **Assessed — false positive.** `create_channel` uses `connect_with_connector_lazy`, so the actual `TcpStream::connect`+TLS is deferred to first use and is bounded by `DEFAULT_SEND_TIMEOUT` in `send`; the cache mutex is held only for the fast lazy-channel construction.
- **R26 (F5) — `stream` size cap counts only data bytes.** `grpc_transport_receiver.rs:173-184` — empty `Chunk.content_data` never advances `received`, so unbounded empty chunks grow the per-stream buffer. **Fixed** — a `MAX_STREAM_CHUNKS = 100_000` cap bounds the per-stream chunk count.
- **R27 (C3) — `phlo_price` checked after replay.** `casper/src/multi_parent_casper.rs:298` (`block_summary`) — a below-min-price block is fully replayed before rejection, so `phlo_price=0` deploys give free replay DoS. **Fixed** — `phlo_price` is now in `block_summary`'s pure-checks, before `validate_block_checkpoint`.
- **R28 (C4) — deploy pool never expires future-dated deploys.** `casper/src/dag.rs:428-432` — the pool ingress never bounded `valid_after_block_number`, so deploys anchored at `i64::MAX` filled `MAX_POOLED_DEPLOYS` permanently. **Fixed** — `BlockApiImpl::deploy` rejects deploys with `valid_after_block_number` more than `DEPLOY_LIFESPAN` ahead of the tip.
- **R29 (C5) — block-receiver maps unbounded.** `casper/src/blocks/block_receiver.rs:101` — valid-signed blocks with unresolvable justifications are retained forever. **Fixed** — `end_stored` rejects when `blocks_st` reaches `MAX_PENDING_BLOCKS`.
- **R30 (C7) — `PeerRateLimiter` never evicts.** `casper/src/engine/node_running.rs:106` — `BTreeMap<Vec<u8>,(Instant,u32)>` grows with connection churn. **Fixed** — `allow` prunes entries whose window is older than 60 s.

### Low (P3)

- **R31 (F6)** — attacker-influenced UPnP gateway can set the advertised external host (hostname bypasses `is_ssrf_unsafe_host`). `comm/src/upnp/gateway.rs:135-152`, the bypass at `:150` (`Err(_) => false, // hostname, not an IP-based SSRF target`); the advertised host is handed out unvalidated at `comm/src/upnp/mod.rs:170`. **Dispositioned 2026-09-27 as deliberate, with the reason pinned rather than asserted.** The bypass is real — a hostname reaches `who_am_i` unvalidated — and it is not a defect, because **the value is published, never dialled**: the only read in `comm/src/upnp/mod.rs` is the classification log at `:138-145`, and `is_ssrf_unsafe_host` answers a different question, which is whether this node may *connect* to an address a peer supplied. Whoever can answer `GetExternalIPAddress` is already the operator's own gateway: they can redirect this node's traffic outright, so a bogus hostname buys nothing they did not have. **What the row did turn up is a wrong diagnostic**, and that is fixed: the classification said "Can't parse gateway's external IP address. It's maybe IPv6" for a hostname too, sending an operator after an addressing problem that is not there. It now says "not an IP literal — it is IPv6 or a hostname". **Falsified both ways:** reverting the wording reddens two tests, `the_external_address_classification_is_reported` and the new `a_hostname_external_address_is_published_and_named_as_one`, whose two arms are that the hostname comes back **unchanged** (published, not refused) and that the log **names** it.
- **R32 (F7)** — attacker-controlled large `sender.host` retained in the connections table. `comm/src/rp/handle_messages.rs:110-129` (the retention) and `comm/src/rp/connect.rs:20,50-51` (`MAX_CONNECTIONS`, which bounds the entry *count* and not the string). **Fixed 2026-09-27, at the boundary rather than at the table.** `PeerNode::from_node` refuses a host over `MAX_HOST_BYTES` (256, in `comm/src/peer_node.rs`), so a wire `Node` with a huge host never becomes a `PeerNode` in the first place. **Why 256 and not a policy number:** the DNS limit for a hostname is 253 bytes and the longest IP literal is 45, so the bound admits everything dialable and refuses what cannot be a host — it is derived from the formats rather than chosen. **Why the boundary and not `add_conn`:** the table is capped at 1024 entries, so applying it there would still parse and hold 1024 oversized strings before evicting them, and `add_conn` returns a `Vec<PeerNode>` rather than a `Result` — it has no way to report a refusal. `from_node` already returns `Result` for the ports, so the refusal lands where the value enters. **Falsified in both directions:** with the bound's condition forced false, `a_host_over_the_bound_is_refused_at_the_wire` is the single test that reddens, and its second arm — a 253-byte hostname must still be admitted — is what stops the bound from being a boundary that rejects everything.
- **R33 (A4)** — faucet to the deployer's own address is a no-op that still consumes the rate budget and submits a deploy. **Fixed 2026-09-27.** `build_transfer_term` signs a transfer *from* the deployer's account, and the genesis faucet's deployer **is** the funded account, so asking for its own address builds a transfer between two accounts the node already holds — it moves nothing, and it spent a drip and paid phlo to do it. `WebApiImpl::faucet` now resolves the deployer key and refuses `address == deployer_rev_address(sk)` **before** the budget is charged. **The fix forced a second one, and that one had been pinned as deliberate:** the key was resolved *after* the charge, so a node started without `--deployer-private-key` burned ten drips per address serving nothing — the same shape, a no-op that spends the budget. That ordering is reversed, and `the_faucet_without_a_key_names_the_flags_it_needs` now asserts the budget is **untouched** rather than spent. **Falsified both ways:** with the refusal's condition forced false, `the_faucet_refuses_a_drip_to_the_deployers_own_address` is the single test that reddens. **What it exposed about the tests:** `key_and_address()` derives the address from the same key it returns, so every faucet test in that module had been dripping to the deployer's own address — which is why none of them noticed. They name a different address now, through a `faucet_target` helper whose own comment says why.
- **R34 (A5)** — `/api/faucet` routes are mounted unconditionally on the public router; the dev-mode gate is only inside the handler. **Fixed 2026-09-27, and it was hiding a wrong published contract.** `GET /api/v1/openapi.json` documents this route's refusal as **404** ("The faucet is disabled on this node", `node/src/web/http.rs:571`), while `json_result` maps every `BlockApiException` to **400** — so a client reading the schema and a client reading the wire disagreed about the same event, which is what "the gate is only inside the handler" actually cost. The routes are now mounted only when `faucet_enabled`, which is dev mode **and** a deployer key, resolved in `node_runtime.rs` where both are in hand and carried to `acquire_http_server` — the same predicate `WebApiImpl::capabilities` derives asynchronously (`web_api_impl.rs:563`), which a synchronously-built router cannot consult. **Falsified both ways, and the failure prints the defect:** with the mount forced unconditional, `a_node_without_the_faucet_does_not_mount_the_route` fails with `left: 400, right: 404` — the schema and the wire, in one assertion. **The test drives the router rather than a handler**, because the mount is the thing under test, and `tower::ServiceExt` was already available through axum's dependency. **And it needed a mock fix first, the same one R36 needed:** `MockWebApi::faucet` was `unimplemented!()`, so the "route exists and answers" arm had nothing to answer with.
- **R35 (A6)** — a faucet drip is silently dropped once the tip passes `height+50` (`DEPLOY_LIFESPAN`), after `200` was already returned.
- **R36 (A7)** — the single `deploy_rate_limiter` is shared by deploy + explore-deploy, so explore floods starve deploys. **Fixed 2026-09-27, by separating the budgets rather than by tightening one.** `HttpState` gains `explore_rate_limiter`, and the two explore handlers read it; `api_deploy` keeps `deploy_rate_limiter`. The faucet already had its own limiter, so the precedent was one route over. **The rate is deliberately unchanged** at `DEFAULT_API_RATE_LIMIT_PER_SEC`: this row is about the *sharing*, and "explore is more expensive per request, so it deserves a stricter number" is a policy question with no oracle behind it — the Scala's HTTP deploy routes are unlimited — so it is left where it was rather than answered by a guess. **Falsified by putting the sharing back:** pointing both explore handlers at `deploy_rate_limiter` reddens `an_explore_flood_does_not_spend_the_deploy_budget` on its explore arm. **One thing the fix had to repair first:** `MockWebApi::deploy` was `unimplemented!()`, so the mock could assert "you never reached the handler" but never "you reached it and it answered" — which is the arm this row needs. It returns an id now.
- **R37 (E7)** — play sorts channel data by `Datum.source` but replay keeps store order; correct today, but an undocumented play-vs-replay fragility. **Dispositioned 2026-09-27, in the source rather than here, because that is where the next reader meets it.** `rspace/src/rspace.rs:167-169` sorts a channel's data by source "so the sorted-first matching datum is chosen regardless of insertion order"; `replay_rspace.rs`'s `run_matcher_consume` filters with `matches` and leaves store order alone. **They are not choosing among the same set**, which is why one needs the sort and the other does not: `matches` keeps only the data the *recorded* COMM names (`comm.produces`, `times_repeated`), so what survives is the recording's choice, while play picks among everything on the channel. **And the order that remains cannot change the answer:** `Produce::apply` (`trace/event.rs:24-36`) hashes `(channel, datum, persistent)`, so two data sharing a `source` are the same bytes — swapping them is swapping nothing. **The residual is real and now written down where it lives:** if `matches` were ever widened to a predicate admitting *distinct* content, replay would pick by store order while play picked by hash and a replay would diverge from the play it is replaying. That is a property of the filter, not of the loop, and the comment in `replay_rspace.rs` says so. No behaviour changed; this row's `owes` cell asked for a written disposition and that is what it got.

---

## 14. Priority-issue remediation (pass 5)

The GitHub issues #18–#25 were triaged; the highest-severity bugs were fixed in this pass: one
reducer RNG invariant (#19), one RSpace join invariant (#21/#22), and one runtime ownership
invariant (#18/#23).


**All seven were re-verified against the tree on 2026-09-27, having been written in pass 4 and never
given a disposition.** Six are still unaddressed; R35 is not, and had been fixed and
never recorded. The re-verification also found the two citations above had moved — R31's range
by sixteen lines, R32's entirely, onto unrelated SSRF code — which nothing would have caught,
because the check that resolved `path:line` citations was deleted the same day with the gate it
lived in. A one-line bullet has no test behind it; if it is going to carry a citation, the
citation has to be re-read by hand.

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

- **C160 — a decided coordinator record could be resurrected by a late vote.**
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

- **C161 — a phase-two failure is discarded.** `casper/src/gateway/mod.rs::apply_phase_two` ignores
  each `commit`/`abort` deploy's outcome (`let _ = self.phase(...)`). Faithful to the coordinator
  model: the decision is already durable (written *before* phase two), a failed delivery is re-driven
  by `recover_in_flight` on the next boot or by a re-issued `run` with the same `txn_id`, and the
  participants are idempotent under `txn_id` (Law 28) — so a lost phase two is not a lost decision.
  The residual is that the record does not distinguish "phase two delivered" from "phase two
  attempted", so a leg whose commit never landed holds its escrow until recovery re-drives it.
  Verified: `a_failed_phase_two_leaves_the_decision_intact` (both legs prepare, the decision is
  written, leg B's commit is rejected, the record stays `Committed`).

- **C162 — the inner replay trace check does not fire for a term tamper; the state-hash comparison
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
  the development could prove anything: the formalization that `AGENTS.md`'s *Prime directive* calls the oracle was
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
  **Closed 2026-09-27, and this row was wrong for as long as the tree was right.** Both proofs
  are in the tree: `takesStep_sound` (`Silence.lean:235`) and `takesStep_complete` (`:372`),
  with `grep -c sorry` at **0** over the file and `spec/laws.tsv` law 38 reading `proved-tied`.
  What the row kept saying was owed — "both proofs remain owed" — had been discharged and
  nothing flipped the state. That is the defect class this register exists to catch, committed
  by the register itself, and it is the reason Wave 0 of the close-out re-derives every open
  row against the tree rather than trusting the list.

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
  answered `false`; `rholang/src/matcher/spatial_matcher.rs:538-544` has an `ETuple` arm. Adding the
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
  the tail (`rholang/src/matcher/spatial_matcher.rs:509-535`, `:538-544`;
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
  **Closed 2026-09-27.** `fuel_saturation` is at `Match.lean:1029`, beside the two shape pins
  the fix added (`a_nested_tuple_is_paid_for` at `:806`, `a_tuple_pays_for_its_own_contents` at
  `:817`), and the file has **0** `sorry`. The row's `owes` cell still asked for the proof. The
  same finding as C40: the work landed and the state was never moved.

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
  **Closed 2026-09-27 — the comparator block was built.** Every constructor this row names as one
  the model "cannot hold" is now in `Rchain/Par.lean` (`ebigint`, `emethod`, `ematches`,
  `epercentPercent`, `eplusPlus`, `eminusMinus`, `eshortand`, `eshortor`), each with the node's
  own `exprTag` in `Rchain/Sort.lean` and its arms in `cmpExpr`. The row's own cost estimate —
  "the remaining gap is structural, and its cost is the comparator block" — was right, and the
  block was paid for on 2026-09-25 without this row being told.

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
  (`spatial_matcher.rs:235-237`) — and the port's *normalizer* names that same line as "the
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

  **Decided 2026-09-27: the model keeps the clauses and does not take the short-circuit.** The
  decision is the one the Lean had already made without the row recording it — `spatialMatches_iff_eq`
  is stated over `pathPar` on both sides (`Match.lean:1991-1993`), and its own doc says in as many
  words that this is "the reason the port's `pattern == target` short-circuit is sound *for the shapes
  the clauses cover*". So the model's job is the **rule**, the port's short-circuit is the **route**,
  and the tie is the proof that the route agrees with the rule wherever the clauses reach. Three
  reasons to leave it that way rather than add the short-circuit to `spatialMatch`, and the third is
  the one that decides it. (1) Adding it would make the model a copy of the implementation instead of
  its specification, which is the direction this tree's oracle runs. (2) The shape where the two
  genuinely differ — `@Set(1 | 2)` against itself, `true` by the short-circuit and `false` by the
  clauses — is **outside `pathPar`**, which is what (1) above established the domain has to be, so
  the tie still holds on the domain it is stated over; that is a fact about the domain and not a
  concession. (3) The short-circuit's *whole justification* is the tie: modelling it would delete the
  statement that licenses it, and a model that assumed its own implementation would have nothing left
  to say about it. The row's `owes` cell offered exactly this route — "or a decision that the model
  need not hold it" — and this is the decision, recorded with what it rests on rather than asserted.


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
  **Re-examined 2026-09-27, and the row is left `todo` deliberately — this is the one finding on the
  check-off that a guard cannot close.** Three things were established, so the next unit starts from
  the answer rather than the question. (1) **The latency lever is not a defect.** Against a transport
  answering every request, a 6-block walk takes 3.85 ms with the production 30 s timeout, pinned by
  `the_walk_advances_on_responses_not_on_the_idle_timeout`; what makes the walk long is the *extent*,
  so the `owes` cell's second half is discharged. (2) **The premise for not adopting the oracle's
  cutoff is real, and was verified rather than inherited:** `CasperDag::insert` refuses a block whose
  justification is absent from the message map — `casper/src/dag.rs:293`, `.ok_or_else(||
  "justification not present in message map")` — so bounding the walk leaves the DAG without the
  ancestry the insert demands, and every block below the bound fails to insert rather than arriving
  late. (3) **The rule to adopt is the oracle's own**, which is what would make this a fidelity fix
  rather than an invention: `blockIsAccepted = isReceivedLatest || blockNumber >= minimumHeight` with
  `blockHeightsBeforeFringe = deployLifespan` (`LfsBlockRequester.ST`) — ~50 blocks below the fringe,
  where the port walks 5,844. **So closing this means changing what `dag.insert` requires:** a
  contract on the consensus path, behind the finalizer and the fork choice. Doing that badly, late in
  a session that has already moved five crates, is worse than naming it, and the `owes` cell now
  carries that so the row stays open for the right reason rather than being closed for a tidy one.

  **Refuted the same day, and the third point above is where this re-examination was wrong.** A second
  session read `LfsBlockRequester.scala` itself instead of taking the account's figures, and found
  that **the oracle's cutoff is inert**: `lowerBound` is declared `Long = 0` at `:125` and the only
  construction of the requester's state in the tree (`:318`, inside `stream()`) never passes it, so
  `minimumHeight` is `0` for the whole sync and both of the oracle's gates are vacuously true.
  `blockHeightsBeforeFringe` is not a bound at all — it is `extraHeights`, `deployLifespan`, appearing
  only as `max(0, min(height - 1, lowerBound) - extraHeights)`, which *reduces* the bound and so
  lengthens the walk. **Both trees walk the full ancestry to genesis, and the port is faithful.** So
  the "~50 blocks" figure this row repeated through two sessions — and that this re-examination
  repeated too — described a cutoff the oracle never applies; the `lowerBound` machinery is exercised
  only by `LfsBlockRequesterStateSpec`, which passes `lowerBound = 200` itself. **What survives from
  above:** point (1), the latency measurement, and point (2), that `dag.insert` requires the ancestry
  — which is *why* the full walk is the faithful shape rather than a deviation to fix. What does not
  survive is point (3) and the whole remedy: there was never a bound to adopt. The lesson is the
  register's own and it caught the register: a figure repeated in an account is not a reading of the
  oracle, and this row was planned against one for two sessions. C64 is `done` — refuted, not fixed —
  and C94's row and the §6 deviation note were corrected with it.


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

  **Closed 2026-09-27 — the end-to-end run, which is all the row's `owes` cell ever asked for.** It had
  been owed since the fix landed, because the change was exercised at the scan step only: the gate's
  first step is `lake build` and the Lean slot belonged to the lead. `tools/check-lean-conformance.sh`
  now runs whole and **green**, and the parts that matter to this row are the ones the scan step cannot
  reach: the Coq trust surface at its ceiling of 8, all **17** committed corpora agreeing with the Lean
  definitions they are emitted from, the law register emitted and matching, every marked count against
  the register (50 laws, 60 entries), law 39's catalog reconciled with `spec/API-SCHEMA.md`, and **all
  91 Rust witnesses ran and passed**. So the widened token set is not merely exercised in isolation —
  it is exercised in the gate that ships, behind a build that actually completed, over a corpus set that
  actually re-emitted. Nothing was changed to close this; a row whose remedy is a run is closed by
  running it, and the output is the evidence.

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
  *Benign, with the reason*: `block_receiver.rs:402-408` — a read failure takes the *conservative* direction
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
| C67 store errors read as negatives across the peer-sync, proposal and gateway paths | 10 | **gateway fixed (2026-09-24)**: `current_height` returns `Result<i64, String>` and `phase_with_term` refuses the phase (`ShardOutcome::Error`) instead of anchoring it at `valid_after = 0`, where it was born expired with nothing reporting an error; falsified first by `current_height_refuses_a_head_it_cannot_read`, which replaced a test whose assertion passed on the defect. **The oracles were read per site**: `BlockStoreSyntax.getUnsafe` (`:33-35`) and `Proposer.scala:240-243`'s `lookupUnsafe` both lift an errored read into an error in `F`, so the port's `false`/`None` there is a dropped channel — which makes the *owed* sites deviations, not decisions. **Fixed: the proposer's bonds read** (`proposer.rs`, `is_active_validator` extracted so no proposer fixture was needed — *give the read its own fallible function and the test stops needing the world*) **and the validated-block pump** (`node_runtime.rs`, same extraction, zero call sites: `wire_block_processing` already took the `log`), both falsified first in the witnessing form. **Fixed: the parent-dependency read** (`block_receiver.rs`, `parents_not_stored` extracted so no stream harness was needed — the same move a third time), falsified first in the witnessing form. **Fixed: the whole site list is closed.** The gateway (`current_height` carries the failure), the proposer (`is_active_validator`), the validated-block pump (`pump_validated_blocks`), the parent-dependency read (`parents_not_stored`), and the receiver's three block reads (`block_is_known`, `block_by_hash`, `not_validated` — which delegates to the first, so the two presence reads cannot drift) plus their seven call sites, each deciding per its channel: skip the answer where a reply exists, log with the hash and skip where none does — the policy that file already carries. Each with a witnessing falsifier run before the fix. **The distinction that decides the conservative sites**: the flatten takes the safe *direction* (a redundant fetch) but does it silently — **the direction is safe; the silence is not**. **Fixed as C65** (the performance walk): `lfs_block_requester.rs:86,128`, the two worst. **Benign, with the reason**: `block_receiver.rs:402-408` (the conservative direction, and the doomed put is logged with the hash) |
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
| C69 the height-monotone reading of law 15 is false of **both** implementations, and the port's termination guard is unregistered | 14b, 15 | **measured rather than asserted** (2026-09-24, G7). The *unqualified* claim — every message of `prev` at or below every message of the published fringe — is false, because the layer is built from the **justifications** and need not cover the senders `prev` covers: `prev = {m3 (sender 0, height 3)}` with a next layer `{q2 (sender 1, height 2)}` publishes below it, and the gate cannot see it because what `calculate_fringe` reads is the **support map**, not the heights (`block-storage/src/dag/finalizer.rs:219-235`). **The oracle is the same shape**: `Finalizer.scala:144-178`'s `calculateFinalization` ends in `LazyList.unfold(parentFringe)(nextFringe(_).map(nf => (nf, nf))).lastOption` — the gate is the support map and there is **no fringe comparison of any kind** — so the port is faithful here and this is an upstream design property, the `check_min_messages` disposition, not a port gap. What *is* a deviation, and is now in §6: the port's own `if nf == current { break }` (`finalizer.rs:361-363`), whose comment says why — "a non-advancing fringe would loop forever" — and which the oracle does not have. **What the heights are used for** is a *recency key*, not an order requirement: `fringe_height` and `latest_fringe` (`block-storage/src/dag/message_map.rs:54,80`) pick the highest-max-height fringe among *contemporaneous* candidates, which needs "later ⇒ higher" only as a consistency heuristic. The **per-sender** half is provable and the port's walk is why: `self_parents` filters by `!finalized.contains(x)` and never traverses *through* an excluded message, so a min message is the previous sentinel's direct successor. Law 15's statement narrows to that, with the cross-sender refutation kept as a `decide`d witness | **Second half settled (2026-09-24)**: the §6 row's "a longer cycle is unimpeded — nothing has measured one" is vacuous rather than unmeasured — the walk is bounded by the non-finalized chain length (the previous fringe is the cutoff, so a sender's minimum message moves strictly in one direction along a finite acyclic chain), pinned by `the_fringe_walk_is_bounded_by_the_non_finalized_chain_length` (an 8-layer fork: 6 steps against a 25-message bound, assertion inside the loop so a cycle fails rather than hangs). The `nf == current` guard is the fixed point's belt, not the termination argument |
| C70 the `sorry` ratchet could not see a nested comment, and nothing but this scan saw an `opaque`/`partial`/`extern`/`unsafe`/`@[implemented_by]` assumption | — **harness** | no law: the instrument, not the term. `Laws.lean`'s accounting is exact equality over `axiom` *declarations*, so an `opaque` definition is neither an axiom nor a `sorry` and passed every other step of `tools/check-lean-conformance.sh` — the token set now refuses all five, so an assumption arriving is refused rather than discovered. The stripping had to be repaired first: single-level block tracking closed Lean's *nested* comments at the first `-/` (invisible for `sorry`/`admit`, 23 prose hits once `partial`/`opaque` were added) and string literals were scanned as code, where the register keeps its own row prose (11 more). Both fixed (nesting depth, strings with escapes, a `'"'` char literal must not open a string) and the tree scans clean. Falsified at scan level with the script's own extracted `awk` over probe files: five tokens fire, `external` does not, prose/string/nested cases fire on nothing, the original `sorry` ratchet does. The full gate is owed: its first step is `lake build` and the Lean slot is the lead's |
| C72 four register cells, three prose paragraphs and two citations called something owed while the tree held it | — **records** (no law) | `spec/TEST-COVERAGE.md`'s law matrix is checked by neither machine check — the register audit reads `.tsv` counts, citations and test names, the Lean gate reads the emitted registers, and a *prose* cell is read by nothing: law 38's cell said `takesStep_iff_reduces` was owed (its row: proved, a theorem), law 42's said `decode_encode` was owed (its row: the axiom is gone), C13's §20 row said the round trip was "still open" (law 33's printer rows are `parse.tsv`'s second half), and an aggregate row contradicted the four rows above it in the same table. Plus `AUDIT.md`'s C53 "Owed: an error channel" and its U12 "blocked on two production call sites" (both landed), `RUST-VS-SCALA.md`'s "30 element-comparator axioms" (law 1b: no axioms, from twelve — the residual is empty, and `Sort.lean` declares none), two citations to `casper/src/main/resources/casper.tla` (it is under `legacy/`), and `faultTolerance` read as pending (legacy-only; the port's checklist mirrors that suite, `tools/run-integration-tests.sh:34`). **Why nothing caught it**: a record is checked where it is machine-readable and unchecked where it is prose — so prose drifts at the rate the tree moves. The matrix now says which of its cells are checked |
| C73 a store-items page had a cap on its count and none on its bytes | 10 | **fixed (2026-09-24)**: `MAX_STORE_ITEMS_TAKE` bounded the nodes a request names, nothing bounded what they carry — the maximal `take` with 4 KiB items is ~41 MB of the responder's memory per request, and a value's size is the chain's choice, not the request's. `MAX_STORE_ITEMS_BYTES = 32 MiB` (≈10× the largest legitimate page) now **refuses** the page: a truncated one is a wrong state claim (the requester recomputes it, `validate_state_items`), so with no error reply the responder's choices are a drop or a lie — and it already drops for an unreadable store (C63) and an over-large `take`. Registered in §6: the oracle has no cap. Falsified both directions in one test — a ~40 MB page is dropped unstreamed, the largest legitimate page is still served — and the check runs before the response exists, so an oversized page is neither serialised nor sent |
| C74 the name-shape vocabulary is a convention, not a predicate | — **boundary** (no law) | `spec/STYLE.md`'s six shapes cannot be a check over the register, and the measurement is the falsifier: **63 of 126** `witness` entries match a shape and 63 are the model's own declarations (`joinKey_perm`, `mergeChanges_assoc`, …), so a predicate over the field fails on 63 legitimate rows — and renaming them would rename the mathematics. The `falsifiable` prose mentions 275 names, 232 of them declarations under discussion. The *rule* is checked and holds (**0 of 49** proved rows lack both a witness and a corpus), and STYLE.md now states the scope with these numbers, so nobody mechanises the table later |\n
| C75 the gate could not see a Lean module at the top of `spec/` — a `sorry` there was invisible to the ratchet | — **harness** | both file-scoped steps derived their scope from `Rchain/`: `expected` from `find Rchain …` and the token scan from `spec/Rchain.lean` + `spec/Rchain/**`. A top-level module escapes the completeness check, the token scan *and* `lake build` (nothing imports it), so a `sorry` there was invisible three times over. Falsified before the fix with two probes — `spec/ProbeSorry.lean` (`sorry`) and `spec/ProbeOpaque.lean` (`opaque def`) — over the script's own commands: the scan reported 0 hits and `expected`/`actual` compared equal with both present. Fixed by widening both scopes to every `.lean` under `spec/` except `.lake/` (5,668 generated files, which is why the old scope looked reasonable), with the policy stated in the check: library root, imported module, or declared `lean_exe` root — no third kind. Falsified after: both probes fire the scan and fail completeness; removed, both green. The edit also shifted law 30's line citation, and **that was attributed here to the wrong commit**: C75's own mapping was at `:209` against the cited `:207`, in-window, so its arithmetic was true of its own commit — the citation was moved `209 → 222` by `94dfd3c68`'s 13-line stack block, and C76 now records the corrected account: an audit that failed in CI and went unread, not a missing check |
| C76 the register audit failed in CI and went unread for six minutes — an unread check, not a missing one | — **harness** | The breakage: `94dfd3c68` (*"the build needs 64 MB of stack, measured"*, a peer's fix) added a 13-line stack block at `tools/check-lean-conformance.sh:43-44`, moving law 30's `lean_parse_corpus` mapping `209 → 222` — outside the audit's ±8-line window of the cited 207 — without measuring the line impact on that citation. **C75 is not implicated**: its own mapping was at `:209`, in-window against the cited `:207`, so its message's arithmetic was true of its own commit (`git show f0d2bdadf:tools/check-lean-conformance.sh` → 209). **And the tree could tell**: CI's `test & coverage` job runs `tools/audit-test-register.sh`, and its log for `94dfd3c68` ends `FAIL law 30: \`tools/check-lean-conformance.sh:207-207\` holds no identifier the row names` — it **fired at 21:59Z and failed publicly and immediately**. So the gap was not a missing check; **the gap was that nobody read it**. Disposition: attention (CI already fails the build on the audit), not new code — plus the durable fix landed since: law 30's citation is the **symbol form** (`tools/check-lean-conformance.sh:lean_parse_corpus`), which a line insertion cannot shift, and `5b1ffe8f1` makes that form resolvable and checked. **How this entry was first written is the same defect class it records**: a `diff` between `f0d2bdadf^`'s blob and the *working tree* lumped C75 and `94dfd3c68` into one diff, and C75's message *discusses* the citation's line — so the shift was attributed to the wrong commit. A diff against the wrong base is an instrument pointed at the wrong target |
| C77 law 15's per-sender height comparison is false of the model, which is wider than the port's data | 15 | **found by attempting the unit that was meant to close the row** (2026-09-24): the walk takes **every** same-sender parent, so a same-sender **fork** on `mv`'s frontier puts a message from the other branch into its output, and that branch sits below no given sentinel — a counterexample the model admits and the port's data does not. **Not a bug in the port, and not merely a datum either**: a validator produces one block per height, and the port **refuses** a same-sender fork at ingress rather than relying on the observation — H-1's equivocation detection rejects a second distinct block by one sender reusing a `seq_num` before any partial write (`casper/src/dag.rs:244-252`), `sequence_number` requires the justified same-sender block to be exactly one `seq_num` lower (`casper/src/validate.rs:169-188`), and `check_justification_regression` admits at most one justification per sender and demands it be the latest (`:205-241`) — so `self_parents` walks a chain the ingress guarantees. (H-1 landed 2026-08-20, `76415d6c6`, a month before this entry's premise was written; the correction is recorded in law 15's row.) The obligation therefore becomes the hypothesis **"at most one same-sender parent per message"** plus the boundary (`selfParents_skips_finalized`) plus the descent (`Descends`) — smaller and better defined than "the arithmetic across a chain". **The asymmetry that earns it a number**: the same shape as C69's cross-sender reading of this law and as G9's law 47 — an `owed` conjunct that needs a hypothesis the **port's data** satisfies and the **model** does not carry; named once, the next attempt does not rediscover it by hitting the counterexample. **And the disposition**: the `owed` column was not emptied by proving a statement the tree does not hold — the pass's call for G9 and for this law's cross-sender clause, applied to the unit written to close the column |
| C78 `ShardId`'s own constructor does not maintain its type's invariant | 26b | **found while correcting 26b's witness** (2026-09-24). `ShardId::child` (`shared/src/refined.rs:382-386`) builds the newtype **directly** — `format!("/{name}")` at the root, `format!("{}/{name}", self.0)` below — so the validation the type exists for (`TryFrom<String>`, `:423-434`: non-empty ASCII) is **not** applied to the name it is given: `ShardId::root().child("\u{2713}")` yields an id `TryFrom` would have refused. **Latent rather than live, measured**: the one production call site validates first (`casper/src/conf.rs:43` runs `ShardId::try_from` on the name before `:48`'s `parent.child(&shard_name)`), so the invariant held — **by the caller's discipline, not by the type**, which was the opposite of what `TYPE-SYSTEM.md`'s "no type escape" convention claims for this newtype. **And it is now held by the type** (`74d21294d`): `child` returns `Result<ShardId, RefineError>` (`shared/src/refined.rs:396`) and refuses an empty name, so the validation the type exists for is performed *by the constructor* and the call site's comment becomes belt-and-braces rather than the guarantee. The remedy named at the end of this entry is therefore no longer a proposal — it is what landed. **What it cost, and why it is worth a number**: the model's `ShardId` is a `String`, so it inherits both directions of the gap, and row 26b's original witness (`validShardId (s.child n) = validShardId s`) was **false** on each: a non-ASCII name, and an invalid *parent* whose child is valid (`shardChild "" "x" = "/x"`). The row now carries the characterisation that is true (`validShardId_child`: given a valid parent, the child is valid exactly when the name is ASCII) and its `decide`d counterexample. Not a §6 deviation: the oracle's shard id is a plain `String` and has no invariant to maintain. **And the caller documents the gap at its single call site**: `casper/src/conf.rs:42-44` reads `// ShardId::child composes the name without validating it, so the name is checked on its own — otherwise a spec could carry an id that format_of_fields would later reject`, then `ShardId::try_from(shard_name.clone())` (`:43-44`) — and `grep -rn "\.child(&"` across `casper/src node/src rholang/src shared/src block-storage/src` returns exactly **one production call site on a `ShardId`** (that one; `rholang/src/scheduler.rs:33`'s `child(&self, i: u16)` is a different type's method). So the invariant is not merely held by caller discipline: **the caller knows why it must be and says so where it matters**, which is precisely the shape `TYPE-SYSTEM.md`'s "no type escape" convention claims the newtype removes. The remedy a reader will look for — validate inside `child`, or make it return `Result` — is inferable from the code and not taken now because it changes a public signature for a state no caller can currently produce |
| C79 `CasperFinality.tla` is described as "formalized", and no tool can run it | 14, 15, 16 | **measured** (2026-09-24): `legacy/casper/src/main/resources/CasperFinality.tla` (147 lines) defines the predicates — `SumStake`, `IsSuperMajority`, `SeenClosure`, `SupportingStake`, `Finalized`, `FaultToleranceMargin` and the rest (12 definitions) — and one invariant conjunction, `Inv` (`:140-145`): `JustificationsWellFormed /\ SeqNumStrictlyIncreases /\ FringeWellFormed /\ SeenMonotone /\ FinalizedIsSeenByAll`. It has **no `Init`, no `Next`, no `Spec`, no `THEOREM`** (`grep -cE "^(Init|Next|Spec|Specification) *="` → 0, `grep -cE "^THEOREM"` → 0) and no `tla2tools`/`TLC`/`tlaps` reference anywhere in `tools/`, the `Makefile` or `.github/`. TLC model-checks a *specification* against an invariant and TLAPS proves *theorems*; with neither present **no tool can run this file, installed or not**. It is a glossary plus an invariant predicate, and "formalized" is the overclaim. **The class, one level deeper than C70/C75/C76**: those were instruments that could not see the defect they named; this is an **artefact that cannot fail** — a named `Inv` reads as a checked invariant while nothing checks it, which is how it survived in the record. **And the plan's sizing was wrong**: Phase 2 called this "wiring it into the formal gate (or recording why not)", implying a missing checker; as measured, wiring it requires a `Spec` to be **written** first — an `Init`/`Next` over the DAG — which is a modelling decision, not a dependency. That is the *second* item tonight that turned out to be "the statement is missing" rather than "the tool is missing" (26b's absent string-order theory was the first). **The laws are not owed to it**: 14b is `provedModel` over `Dag.lean`'s derivation, 15 is narrowed with C77, and 16c has its layer — so the TLA is **superseded**, not merely unrun. **Disposition**: not wired, and named as a *definitional* gap rather than a gating one — it records intent in the reference tree; the trigger to revisit is a law needing DAG-level *temporal* reasoning the register cannot state. `spec/INVENTORY.md`'s open question is corrected in place to "states the intended invariants as a reference-tree artefact", with what would be needed first |
| C80 a dropped register row is invisible to every check — a catalogue with no count of itself is checked for internal consistency, not for completeness | — **register** | **found by accident, confirmed by measurement** (2026-09-24): an edit of mine that rewrote row 26b's cells deleted the **26c record** with it, and every check stayed green — the numbering check validates *numbers* (26a survived, so 1..49 was intact) and reference integrity validates *cited declarations*, so **neither can see a dropped row**: both are checks *within* the set rather than *on* it. What surfaced it was the emitter's own summary reading **"57 entries"** against a known baseline of 58 — an artefact that failed only because a person remembered the baseline, which is C79's shape (an artefact that cannot fail) one level up. **The fix, landed with this entry**: `LawsMain.lean` gains **`entryCeiling := 58`** beside `lawCeiling`, hand-bumped and deliberately not derived for `lawCeiling`'s stated reason (a check that counted the rows would compare the register to itself, and a dropped row would take its own evidence along), so a dropped clause is a build failure naming the expected and actual counts. **Falsified before it was believed**: deleting one clause fails the build with that message, and restoring it passes. Adding a row now carries the same maintenance tax `lawCeiling` already carries. 26c itself is restored byte-identical to HEAD, so the commit that lands this carries no change to that row |
| C81 the `run` subcommand has no discoverable help, and prints a usage line that offers none | — **harness** | **confirmed end-to-end on the built binary** (2026-09-25): `rnode run --help` → `error: unexpected argument '--help' found`, and the usage line it then prints — `Usage: rnode run [OPTIONS]` — names no help flag either; `rnode run -h` → `error: a value is required for '--api-port-http <API_PORT_HTTP>' but none was supplied`. **The cause is structural** (`node/src/configuration/commandline/options.rs`): the top-level `Options` sets `disable_help_flag = true` (`:72`), defines a **long-only** custom help (`:76`, `#[arg(long = "help", action = clap::ArgAction::Help)]`), and binds `-h` **twice** — `--grpc-host` (`:79`) and `--api-port-http` (`:364`) — so the subcommands inherit no help flag at all. **The class is this pass's again: a surface that reads as present and is not** — the operator's first move (`run --help`) is the thing that fails, and the failure names a *different* flag, so it reads as a mistyped argument rather than a missing feature. **Closed at `f96600bb2`, and the fix established two things wider than the report.** (1) **The defect was not `run`-specific**: *every* subcommand lacked a help flag — `run`, `deploy`, `repl` and `status` each exited 2 — and the cause is the one structural fact, `disable_help_flag = true`, the very setting that frees `-h` for `--grpc-host` leaving no help flag for any subcommand, so no subcommand's options were discoverable from the CLI. (2) **The fix is one attribute, not four fields**: the port's long-only `--help` becomes `global = true` (`options.rs:85`, with the reason in its comment), so each command renders *its own* options, while `disable_help_flag = true` **stays** (`:72`) because it is what keeps `-h` as `--grpc-host`; a per-command help field was rejected because four commands are payload-less variants (`Status`, `Repl`, `Mvdag`, `LastFinalizedBlock`) with nowhere to put one. **And the `-h` decision is a contract choice now stated rather than left implicit**: `-h` is **unchanged in both contexts** — `--grpc-host` at the top level, `--api-port-http` under `run` — because freeing it for help would break spellings that parse today (`rnode -h <host> deploy …`), while `-h`-for-help was never available in this port; the help text states the convention. Operator-facing consequence: **`rnode run -h` still errors, and `--help` is the spelling for help** — the fix makes help *discoverable*, not `-h`-spelled. **The falsifier is test-level and exact**: with the new tests in place and only the attribute reverted, `every_command_accepts_help_for_its_own_options` (`options.rs:630`) failed `left: UnknownArgument, right: DisplayHelp` for `["run", "--help"]`; `cargo test -p rchain-node --lib` is 190 passed / 0 failed, and the previous unit's metrics refusal still fires (`rnode run --influxdb` → `Configuration error: …`, exit 1), so argument parsing is untouched |
| C82 `Descends` quantifies over every parent, and the port bounds only the unfailed ones — so the descent order is violated by a state the port admits | 15 | **found by measurement, and reachable rather than merely representable** (2026-09-25, H1b). The model's `Descends` (`spec/Rchain/Casper/Dag.lean:296-297`) requires **every** resolved parent to be lower than the message naming it, while the port enforces that only for **unfailed** justifications: `validate::block_number` skips them (`casper/src/validate.rs:156-158`, `if !meta.validation_failed`), and a failed block's recorded height is its **claimed** `block_num` (`message_from_block_metadata`, `height: block.block_num`, `casper/src/dag.rs:45`) with nothing bounding it. **The violation is constructible through the port's own acceptance path**: genesis at 0, a failed block by validator 1 claiming **999**, then a child by validator 2 claiming **1** and justifying both — `block_number` answers `Ok(())` because it skips the failed justification, and `insert` takes the block, leaving `parent.height = 999 > me.height = 1`. Witnesses: the case is `h1b_a_failed_parent_above_the_childs_height_is_refused` (`dag.rs:1287`) — the construction was `..._breaks_the_descent_order` when it *was* the violation and the falsifier inverted with the code, so the same test now asserts the refusal — and `h1b_a_justified_bonded_failed_block_is_refused_rather_than_forced` (`:1347`) is the case below. **And the corrected brief is what makes the route the right one**: `neglected_invalid_block` does **not** force a block to justify bonded invalid blocks — it **refuses** a child justifying a failed block whose sender is still **bonded** (`NeglectedInvalidBlock`, `casper/src/validate.rs:266`), faithful to the oracle (`legacy/casper/.../Validate.scala:340-360`), so the bonded route is **closed** and the open one is a peer naming an **unbonded** (or zero-stake) validator's failed block, which no rule forbids. **Enforced, not restated — the disposition this row carried is superseded by the enforcement decision, and the divergence it predicted is now real and numbered** (`46c35b545`, 2026-09-25). `block_number` bounds **every** resolved parent *before* it skips the failed ones for the maximum: `if i64::from(meta.block_num) >= i64::from(b.block_number) { return Ok(Err(BlockStatus::InvalidBlockNumber)) }` (`casper/src/validate.rs:152-154`), with the failed-skip that computes the maximum untouched at `:156-158`. So the state that made the hypothesis false — the failed parent claiming **999** with a child at **1** — is **refused at ingress** rather than admitted, and the proof's second hypothesis (`Descends`, `spec/Rchain/Casper/Dag.lean:296-297`) now holds of **every state the port admits** instead of being merely *stated* of a state that refuted it. **Which states each form covers, precisely**: before, `Descends` *covered* — was asserted of — every admitted state including the failed-parent-above ones, and the measurement at the top of this row is that it did not hold of them; the bound now **excludes exactly those and nothing else**. It does not narrow the maximum the claimed number is computed from (that still skips failed justifications, `:156-158`), so an unfailed parent at or above the child was refused before and still is — what is new is the failed parent. The witness inverted with the code: `h1b_a_failed_parent_above_the_childs_height_is_refused` (`casper/src/dag.rs:1287`) is the same construction, asserting the refusal where it asserted the old code's admission. **And the divergence the old sentence predicted is now a fact with a number: C83.** The reference validator **filters failed parents out of `blockNumber` entirely** and never bounds a resolved parent's height (`legacy/casper/src/main/scala/coop/rchain/casper/Validate.scala:178-198`), so it **admits** the block this refuses. That is a §6 deviation of the **port** — kept, because the laws are the oracle and this is the premise law 15's proof consumes — and C83 carries the operator consequence; the model obligation this row was, becomes an enforcement the port now holds |
| C84 the port refuses an equivocating block — at insert **and** at restore — where the oracle has no equivocation gate at all | 15 | **found by reading the Scala for H1c's scoping, and it closes the arc H1a opened** (2026-09-25). The gate is H-1's (`casper/src/dag.rs:244-252`: a second, distinct block by one sender reusing a `seq_num` is rejected before any partial write, pinned by `insert_rejects_equivocation_same_seq_num` at `:568`), and H1c extended it to the **restore**: `BlockDagKeyValueStorage::create` folds the persisted `height_map`, and the persisted layer checks only *contiguity*, so a store already holding a fork used to restore straight into the DAG and the H-1 stall the gate prevents was live again with nothing saying so (`a2b20554e`, pinned by `h1c_a_store_holding_an_equivocation_is_refused_on_restore`; the check is a `BTreeMap<(Validator, SeqNum), BlockHash>` rather than `insert`'s per-message scan, so the restore stays O(N) under C55). **And the oracle does not do this**: the Scala has **no equivocation gate anywhere** — a search over `legacy/casper` and `legacy/block-storage` finds none — and its `validateDagState` (`legacy/block-storage/src/main/scala/coop/rchain/blockstorage/dag/BlockMetadataStore.scala:118-124`) asserts only that the height map's numbers are contiguous, never `(sender, seq_num)`, so a forked store restores silently there. **What the refusal buys, and why the deviation is kept**: an equivocating validator can neither enter the DAG nor stall finalization. **H1a's note points at this row** for the reason law 15's no-fork premise is *guaranteed* rather than observed; this is the other half of that arc — the premise is enforced by a departure the Scala does not share. **Operator consequence: a peer's equivocating block is refused, and a node restoring a forked store refuses to start** (the Scala starts and carries the fork). **And it exposed a fixture defect, recorded here as the same finding from the test side**: six tests sharing `store_chain` built a chain in which one sender reused `seq_num 0` for every block — a state H-1 and `sequence_number` both forbid, legal to store only because the persisted layer does not look at sequence numbers — so six tests had been pinning behaviour over states the port does not admit; the repair is in `a2b20554e`, and its best evidence is that the digest constant moved (`a6632357…` → `be621d6c…`, the digest being a function of the representation's value) while `logical_bytes == 2032` and `seen_entries == 21` did not |
| C85 `visualizeDag` panics on an empty window in the oracle and returns an empty graph here | 43 | **found by the pass's live bug (H2a), settled against the oracle** (2026-09-25): `dag_as_cluster` (`casper/src/api/graph_generator.rs:51`) read `timeseries[0]` unguarded, reachable from the public `visualize_dag` (`block_api_impl.rs:534`), so an empty DAG — or a `start_block_number` above the chain's top — handed the renderer an empty slice and the index panicked. Fixed with `timeseries.first()` and an empty graph when there is no lowest height to anchor clusters to (`da6c4a440`), and the caller's filter is extracted as `view_blocks_above` — a pure function of a `DagRepresentation` and the window's lowest height — so the reachable inputs are pinned without constructing a `BlockApiImpl` over a runtime. **The oracle raises**: `GraphGenerator.scala:39` is `val lowestHeight = timeseries.head` over `acc.timeseries.toList.sorted` (`:38`), a `List` built from a `Set`, so an empty window throws `NoSuchElementException`. **Operator consequence: an empty window renders an empty graph here where the Scala's endpoint raises** | **The law column reads 43, and it is the *endpoint* law rather than a DAG one**: what `visualizeDag` answers when there is nothing to draw is a `[]`-semantics and error-precedence question — "each endpoint's serialized shape equals the schema's" (`43`) — and not a fringe or ordering one. The cell read `47`, which is the PoS withdrawal-staging law and has nothing to do with this endpoint; corrected 2026-09-25 so the column points a reader at a law that covers the incident |
| C86 the port propagates a DER decode failure on the sign path, where the oracle silently returns the empty array | 19 | **settled against the oracle, and it goes the other way from C84/C85** (2026-09-25, H2c): `Secp256k1Eth::sign` returned `Ok(certificate_helper::decode_signature_der_to_rs(&der).unwrap_or_default())` — a decode failure flattened into a value **on the sign path**, which the `silent` scan cannot see because it matches `unwrap_or(0)`-shaped calls and this is `unwrap_or_default()`. **And the oracle does exactly that**: `legacy/crypto/.../signatures/Secp256k1Eth.scala:66-72` is `decodeSignatureDERtoRS(sigDER).getOrElse(Array[Byte]())`, annotated "DER conversion error silently returns empty array", so the port was **faithful before this fix** and the fix is a *deliberate departure*. It is worth taking because the flattened value is the **empty vector** — not a zeroed 64-byte signature, which is what a brief said — so `sign` returned `Ok(vec![])`: zero bytes where `sig_length()` promises 64, indistinguishable from a result. **The defect was latent, measured rather than asserted**: the writer probed **64 key pairs** and no `sign` result was ever the flattened value, structurally — the signer emits minimal DER and the decoder strips DER's sign padding (`left_pad_32`), so decoding what this code just produced cannot fail. That probe survives as the guard `sign_returns_a_verifying_64_byte_signature_and_never_zeros` (`d01f1be2d`), so the evidence for "unreachable" is a test rather than a sentence. **Operator consequence: a malformed DER reaching `sign` now returns `CryptoError::InvalidSignatureFormat` where the oracle returns the empty array** — the port preferring an error to a value that violates its own length contract |
| C87 the genesis seeder indexed a published datum's two elements without checking its arity, so a short datum panicked the seeding on the genesis **and** the replay path | — **partiality** (no law) | **found by measurement, and settled against the oracle by finding that there is none** (2026-09-25, H2b). `rgov::published_uri` (`casper/src/genesis/rgov.rs:268`) read the published `["<name>", <uri>]` datum with `items[0]`/`items[1]` **after** `RhoList::unapply`, which returns the items at *any* arity, so a one-element or empty datum panicked with `index out of bounds: the len is 1 but the index is 1` (`afec579e6`'s falsifier reports the old `items[1]` read at `rgov.rs:267`, the pre-fix line). Both reads are `items.first()?`/`items.get(1)?` now (`:271-273`) and the caller's existing `else { continue; }` makes a short datum **skipped** rather than fatal — `None` is the right answer rather than an error, because the caller already skips what it cannot parse. **The datums are chain-side, not local**: `seed_rgov_aliases_from` runs on the genesis path (`casper/src/runtime_manager.rs:709`) and on the **replay** path (`casper/src/runtime_replay.rs:190,222`), replaying them out of the block's own genesis state, which is law 11's stake in it — the alias writes are native writes outside the deploy log, so a replay that cannot reproduce them diverges the genesis hash. **There is no Scala counterpart, and establishing that is the finding**: the whole publish-channel mechanism is this port's own — `URI_PUBLISH_CHANNEL = "rnode:genesis:rgov-uri"` (`rgov.rs:141`) is a port constant, and a search of `legacy/` for `rgov`, `readcap`, `publishChannel` and the channel name returns **nothing** (the only `rgov` hits are wallet text files). Upstream deploys this set at runtime with a script outside the node (`scripts/bootstrap-rgov.ts`, which this module's own doc names and which is not in this repo) and feeds the resulting URIs back into the dependents; it has **no node-side reader of a name/uri datum at all**, so the "index blindly, ignore the datum, or raise" question this row opened has no Scala answer to give. The closest Scala is the *registration* path, and it does not inspect a datum's shape either: `legacy/casper/src/main/resources/Registry.rho:409`'s `insertArbitrary(@data, ret)` binds the whole datum as `@data` and stores it under a freshly built URI, inspecting nothing — so **no §6 row is written**: there is no oracle behaviour here to depart from, and the fix is the type discipline's (no silent partiality) rather than a deviation from the reference. **Latent through the port's own genesis, measured rather than asserted**: both producers of the datum are this module's and both emit exactly two elements — `publish_registration` (`rgov.rs:290`) appends `@"rnode:genesis:rgov-uri"!(["<name>", uri])` and the master-directory template (`:500`) publishes `["readcap", *URI]` — and the shape is itself pinned at **build time** by the assertion that the published term is present (`:618`), so a genesis *this* binary builds cannot produce a short datum. What the guard buys is therefore at the **boundary**: `published_uri` is `pub` and reads data that came from a chain, so whether a genesis whose publish datum drifted seeds its aliases or takes the node down is decided by *which revision replays it* — and an index panic is the wrong answer for a `pub` parser in any case. The fix is pinned by `published_uri_refuses_a_list_shorter_than_two` (`:773`), which asserts both refusals **and** the two-element control, so the test can fail. **Operator consequence: a published datum shorter than two elements is skipped rather than fatal** — where before it took the genesis seeding, and the replay, down with an index panic |
| C83 the port refuses a block naming a resolved parent at or above its own number — the failed ones included — where the Scala filters failed parents out and bounds no resolved parent at all | 15 | **the enforcement decision (H1b enforced, `46c35b545`), taken because the laws outrank the Scala where the proof needs the premise** (2026-09-25). The bound is `if i64::from(meta.block_num) >= i64::from(b.block_number) { return Ok(Err(BlockStatus::InvalidBlockNumber)) }` (`casper/src/validate.rs:152-154`), applied to **every** resolved parent before the maximum's failed-skip (`:156-158`); C82 is the row it serves, and the §6 register carries the deviation. **The oracle admits the block**: `legacy/casper/src/main/scala/coop/rchain/casper/Validate.scala:178-198`'s `blockNumber` resolves the justifications through `lookupUnsafe` and then **`filter(!_.validationFailed)`** — a failed parent is discarded before the maximum, and no resolved parent's height is ever compared against the block's number — so a Scala validator receiving a chain containing such a block accepts it where this port answers `InvalidBlockNumber`. **Operator consequence: a peer sending such a block is refused** — so a port validator rejects a block a Scala validator admits, **a validator-side divergence on this check alone**, which is what a §6 row is for. **And the measurement bounds the risk to byzantine input**: no honest proposer can produce the refused block, and that is read off the proposer's own arithmetic rather than argued. A proposal's justifications *are* `latest_msgs.values()` (`casper/src/multi_parent_casper.rs:293-300`, `get_pre_state_for_new_block`, called at `casper/src/blocks/proposer/proposer.rs:423`); a validation-failed block is recorded in the message map but **kept out of `latest_msgs`** (`casper/src/dag.rs:375-381` → `block-storage/src/dag/message_state.rs:118-128`, the H-2 exclusion, whose comment says why: a failed block that became a parent "would wedge block production"); and the number claimed is `max(parent heights) + 1` over exactly those parents (`casper/src/blocks/proposer/proposer.rs:426-432`). So the failed-parent route is closed **at the proposer** and the unfailed one was already refused **by the maximum**: only a **byzantine or custom peer** can send the refused block, and the honest ones cannot. **Why it was taken anyway**: `Descends` is the premise law 15's proof consumes, and a premise the port *guarantees* is worth a divergence a byzantine peer can trigger and an honest one cannot — the reasoning C82's superseded disposition predicted as hypothetical ("a validation divergence that would need a §6 row of its own") is now the row |
| C88 the shard-id domain check was a `debug_assert!` — it vanished at `--release`, so a non-ASCII shard id reached the seed law 11's determinism rests on | 26b | **found by the audit's asymmetry rule, and settled against the oracle as a *fix* rather than a deviation** (2026-09-25, H2d). `BlockRandomSeed::new` asserted the shard id's ASCII-ness under `debug_assert!`, so in a `--release` build the check **was not there**. The falsifier states that as a run rather than an argument: `a_non_ascii_shard_id_is_refused_in_any_profile` (`casper/src/block_random_seed.rs:224`) **FAILED** under `debug_assert!` at `--release` — there was nothing to refuse — and passes in **both** profiles as `assert!` (`:49`), pinning both halves (a non-ASCII id is refused, an ASCII one accepted, so it cannot pass vacuously). **And the oracle asserts it too**: `legacy/casper/src/main/scala/coop/rchain/casper/rholang/BlockRandomSeed.scala:39` is `assert(shardId.onlyAscii, "Shard name should contain only ASCII characters")`, a plain `Predef.assert` that only `-Xelide-below ASSERTION` would elide — and no build file in `legacy/` sets it — so the Scala's check **is live in production** and the port's `debug_assert!` was a **weaker port, not a faithful one**. The change is therefore **parity, not departure**, and no §6 row is written for it: `assert!` is what the reference does. **The boundary enforcer is named rather than the assert left bare**: `validate::format_of_fields` refuses an empty or non-ASCII shard id at ingress through `ShardId::try_from` (`casper/src/validate.rs:19-23`, test `format_of_fields_rejects_non_ascii_shard_id` at `:461`, and not debug-gated), and C78 records that the port's `ShardId` is the validated newtype the oracle's plain `String` is not. **One difference is worth naming while both trees are open, and it is not a §6 row**: on a *peer* block carrying a non-ASCII shard id the port answers **invalid**, where the Scala's `formatOfFields` accepts it — it checks only `isEmpty` (`legacy/casper/src/main/scala/coop/rchain/casper/Validate.scala:56-79`) — and the refusal happens *later*, as an assert in the validation path (`deploysShardIdentifier`, `:270-273`): a throw rather than a verdict. Both trees refuse the block, by different means and with the same consensus outcome, so this is robustness where the port is the cleaner of the two. **And C78's "the oracle's shard id has no invariant to maintain" means the *type* has none** — the oracle keeps ASCII-ness by assert in two places (`Validate.scala:272` and `BlockRandomSeed.scala:39`), which is exactly why the constructor assert is parity and not an addition. **Operator consequence: a non-ASCII shard id is refused in every profile** — before, a `--release` build accepted one into the block's random seed |
| C90 the type-system audit's panic scan never ran in CI, and the 29 "violations" it reported were its own staleness check reporting that the whitelist matched nothing | — **harness** | **found by the coverage job going red, and it is a harness finding: no law covers it, because the thing that was wrong was outside the term** (`1a0fa77b8`, 2026-09-25). `scan_panic` passed its pattern as `awk -v RE="$pattern"`, and **`-v` processes backslash escapes in the value**, so the escaped literal parens in the pattern became group-openers, the alternation became an invalid regexp, awk **fataled**, and the scan matched **zero sites**. CI's log is the mechanism verbatim: `awk: warning: escape sequence '\\(' treated as plain '('` then `awk: cmd. line:21: fatal: invalid regexp: Unmatched ( or \\(: /.unwrap()|.expect(|panic!|…/`. With no sites, every whitelist entry matched nothing — and the **per-site staleness check (added last pass) correctly reported each of them as stale**: twenty-nine violations, none of them a violation. **What the disguise was, and it is why a local run could not have caught it**: the `-v` form *worked on this machine*, because this box's awk is **mawk** (measured here: with the same pattern, `-v` accepts the mangled form and matches through it, while the environment form searches the literal-paren regex the pattern actually wrote), and CI's awk is **gawk**, whose escape warnings are the log lines above. So the local green was never evidence of a working scan. **Why it surfaced only now, which is the instrument's part of the finding**: the per-site staleness check is the *only* thing that could see it, because before the per-site conversion the whitelist was **file-level** and nothing counted its matches — **a scan that returned nothing looked exactly like a clean tree**. That is C76/C79/C80's shape once more (a record believed because nothing re-derives it), here wearing the **environment** as its disguise. **How long CI's panic class had been scanning nothing is not recoverable from here** and this row does not imply a date: what is certain is that the check that finally caught it is one whose absence had been invisible for the same reason. **The fix is portability rather than a workaround**: the pattern goes through the **environment** (`RE="$pattern" awk … ENVIRON["RE"]`), which no awk escape-processes on any implementation. **Verified in this lane, not relayed**: the audit now reports `29 panic-class site(s) in production code; 29 allowlisted by 28 entr(y|ies)` and `23 narrow-refinement construction(s) in 11 file(s)`, exit 0 in 16 s here — and **the count is itself evidence**: the same run reported 30 sites / 27 allowlisted before, and the difference is exactly the prose false positive the commit before this one removed, with the two then-unallowlisted sites being the ones it fixed. **Operator consequence: none for the node — the panic class was the *instrument*, so what was lost is detection, not behaviour**: a real panic site in production code would have been invisible to CI for as long as this lasted, and the whitelist's own entries are what finally forced it into the open |
| C91 the audit can print a green summary while a class never ran — its exit code reflects the classes that *completed*, and nothing checks that one did | — **harness** | **found by the lane paying for it rather than by reading, and it is the framework's own version of C90** (2026-09-25, H3, `9bb6c3dfe`). A malformed whitelist entry — a missing field, so `parse_entry`'s `${front:0:$(( ${#front} - ${#ev} - 2 ))}` (`tools/audit-type-system.sh:262-264`) computed a **negative** length — made the audit print `OK: no hard production violations (panic/unsafe/silent/escape).` **in 0.035 s while scanning nothing**, measured by the lane that hit it. The parser bug is fixed and the guard now runs full length (13.8 s here), but **the structure that allowed it is untouched**: there is no `set -e` (the script has no `set` line at all), `hard_failures` is a global counter incremented only in `note()` (`:167-173`), the classes are a plain loop (`:891-893` over `:887`'s roster), and the summary prints **unconditionally** after that loop, exiting 1 only if that one counter is non-zero (`:895-901`) — so **a class that ends without calling `note()` leaves the exit at 0**, whether it ended because a helper errored, because a subshell took its findings with it, or because it returned early, and the result is indistinguishable from a class that found nothing. **There is no check that a class completed, and the count of classes that ran is exactly the thing nobody counts.** **The contrast that states it best, and it is a clause rather than a row**: the component the *same unit* added is stricter about its own emptiness than the framework around it — H3's guard fails hard when it derives no newtype at all (`:828-829`: "a derivation that silently finds nothing passes silently, which is the failure this guard exists to prevent") and when an exemption matches nothing derived (`:818`), while the framework that runs it has no such rule for itself. **And this is C76/C79/C80/C90's shape one level up**: those were records believed because nothing re-derived them; this is the instrument's *own summary* believed for the same reason. **Disposition: open, and it wants a fix rather than only a row** — the file is `tools/`, so the instrument lane holds it. The cheap structural remedy is a **per-class completion marker**: each class appends its name as its last act and the summary refuses unless the completed set equals `classes`, because `set -e` alone cannot catch a class that *returns* early, and the marker is the same medicine this guard already applies to its own derivations. **Operator consequence: none for the node — the loss is detection, and the row's own statement is that the gate's `OK` is a claim about the classes that finished, not about the classes that were asked for** |
| C89 the register-anchor check **aborted** on an unresolvable citation instead of reporting it — `exit 1`, no `FAIL` line, nothing after its own section header | **harness** | **found by walking into it** (2026-09-25, landing law 19's tie), and it is C37's class with the sign flipped: that was a measurement that was not a measurement, this is a **check that was not a check**. `anchor_resolve` sets a global `file` and returns it to two callers, each of which prints `does not resolve — write the path in full` when it is empty. The printing never happened. A function's status is its last command's, and both branches end on a **test**: the `for cand in …; do [[ -f "$cand" ]] && { file=…; break; }; done` leaves the failed `[[ -f ]]` as the loop's status, and the enclosing `if`/`else` propagates it — so on exactly the input the function exists to report, it returned 1, and `set -euo pipefail` killed the script at the call site. **Measured, not argued**: with `Crypto/Spec.lean:41-49` in row 19's note the audit exited 1 with its log ending at `== register anchors (every cited line still holds what the row says) ==` and **zero** `FAIL` lines; the same tree with the path written in full (`spec/Rchain/Crypto/Spec.lean:41-49`) printed `OK: the register matches the tree`. The mechanism was then isolated in six lines — `set -euo pipefail`, a function whose last command is a failing `[[ -f ]]`, one call — which reproduces `rc=1` and no output. **And the abort was reachable from a one-word mistake**: five rows of this file instruct an author to "write the path in full", and the instruction carried no verdict when ignored. **Fixed** by an explicit `return 0` at the function's end, with the contract stated (`set file, say nothing`) and C89 named at the line, because the next helper written in this style will end on a test too. **Falsified both ways after the fix**: the unresolvable path now produces `FAIL  law 19: `Crypto/Spec.lean:41` does not resolve — write the path in full (its basename is ambiguous or absent)` and the run reaches its summary; the full path is green. **Operator consequence: none for the tree, and one for the instrument** — a citation defect in the register used to stop the audit at its ninth section with no explanation, which a reader would have to distinguish from a passing run by the exit code alone |
| C92 the model's PoS state cannot hold the bonds map the finalizer's gates read — `PosState.active` is ids without stakes, so law 16d could not be stated, let alone proved | 16d | **found by asking the law what its subject is** (2026-09-25, closing law 16d's sync site). The port's bonds map *is* the `pos:active` leaf: `compute_bonds` is one read, `get_native(PREFIX_POS, pos_active_key())` (`casper/src/runtime_manager.rs:1276-1281`), decoded as `BTreeMap<Validator, NonNegI64>` — **a map with stakes** (`rholang/src/native_state.rs:796-798`). Its writers are three and none of them is `bond`: `select_active(&pool, &withdrawers, &params)` at a boundary (`:1312-1317`), genesis installation (`:1061`) and `slash` (`:1429`) — so between boundaries the pool's stakes move and the active map's do not, and the map the gates run on is a **boundary snapshot**. That is law 44's gate ("a bond pools but does not activate") seen from the finalizer's side, and it is exactly what law 16d is about. **The model has no such field**: `PosState.active : List Validator` (`spec/Rchain/Pos.lean:184`) with the stakes in `pool`. Deriving the bonds map from the model's state would answer with the *current pool stakes* — for a validator that bonded after the last boundary, a **different map from the port's** — so the derivation would be wrong exactly where law 44 says the difference is real, and `proved-model` rows 44–47 rest on that coarser state without saying so. **Consequence, and the fix's size**: law 16d moves `open` → `owed` with its sync site modelled (`Rchain/Casper/Bonds.lean`: the three sources, `the_sources_agree`, and the refusal `a_disagreeing_set_is_refused` with #73's shape as a concrete witness), and what remains is a change to `PosState` — record the stakes *selected* at the boundary, i.e. `active : List (Validator × Nat)` — which touches laws 44–47's model and their proofs. Stated here rather than in a note because the state's coarseness is what *made the row unstatable*, which is the register's own reason for existing: the law found the gap, and the gap is one field wide. **Closed (2026-09-25)**: `PosState.active` is a `List (Validator × Nat)` and `reselect` keeps the pairs instead of dropping them (`.map (·.1)` was the loss), so the model holds the map the gates read; the state's side is stated as a *pair* — `a_bond_does_not_move_the_map_the_gates_read` and `the_pool_derived_map_moves_where_the_states_does_not` (`Rchain/Casper/Bonds.lean`), because either half alone is a spelling — and law 16d moves `owed` → `proved-model`. Falsified first: making the first theorem read `pool` instead of `active` stops it being `rfl` and the build fails. The field change also made law 44's `a_boundary_activates_the_pool` **strictly stronger**, its `s'.claims = []` hypothesis going inert rather than load-bearing |
| C93 the model's min message was the **newest** same-sender ancestor where the port's is the oldest — the derivation's step 2 read the opposite end of the chain, and no instance could see it | 15 | **found by measuring the walk's order against the port's, on law 15's last conjunct** (2026-09-25). Step 2 of `next_fringe` is `chain.into_iter().last()` over `[p] ++ self_parents p` (`block-storage/src/dag/finalizer.rs:282-288`), and the port's `self_parents` builds its chain with `chain.push(m)` over a worklist it **replaces** each step (`:102-124`), so that chain is **newest-first** and `.last()` is the *oldest* non-finalized same-sender ancestor. The model's `selfParents` (`Rchain/Casper/Dag.lean`) prepends into its accumulator instead, so its list is **oldest-first** — and `minMsgs` read `getLast?`, i.e. the **newest**. **Measured**: for `10 (h 0) ← 11 (h 1) ← 12 (h 2)` the model's `selfParents` is `[10, 11]` and the port's chain is `[11, 10]`, so the model answered `11` where the port answers `10`; the transcribe-the-port version of the walk (`next.pop()`, `chain.push`, `next` replaced) returns `[11, 10]` under the model's own `sameSenderParents`, which is what pins the direction rather than a reading of the Rust's intent. **Why nothing failed, which is the finding**: every `decide`d instance in the file (`dag3`'s `minMsgs_dag3`, `a_derivation_publishes_a_layer`, `a_derivation_is_an_antichain`) gives a sender at most **one** same-sender ancestor, and with one element `getLast?` and `head?` agree — so a step of the derivation read the opposite end of the chain while `decide`, the corpora, the gate and every register check stayed green. A test that cannot distinguish the two ends is C37's class (a measurement that was not one) in the shape of an *instance*. **Fixed** to `.head?` with the measurement in the docstring, and pinned by two `decide`d theorems over a new `chain3` — `the_walk_is_oldest_first` and `a_chain_of_three_picks_the_oldest` — whose falsification is a revert: restoring `getLast?` makes the second one fail. The antichain laws above are unaffected (which of two same-sender messages the layer fold keeps does not change its keys), and law 15's *statement* is still `owed`: the step is now faithful, the per-sender sentinel comparison is not yet proved |
| C94 the register has **no model of the block requester** — the LFS ancestry walk is specified by the Scala alone, so no proof constrains it and nothing mechanical would catch a wrong cutoff | — **no law**: the requester is unmodelled | **found by sizing the §6 row that offered the Scala's cutoff as an efficiency unit** (2026-09-25, settling row 309 rather than implementing it). The walk is registered nowhere in the oracle: grepping every `.lean` under `spec/Rchain/` for `lowerBound`, `extraHeights`, `requestStream`, `LfsBlockRequester` or `cutoff` returns nothing, and `spec/Rchain/Casper/` holds exactly `Bonds`, `Dag`, `Fringe`, `Stake`, `Validate` — so this component's fidelity is anchored in `legacy/casper/src/main/scala/coop/rchain/casper/engine/LfsBlockRequester.scala` and nowhere else. **Why that settles the row rather than deferring it**: taking the cutoff needs the **truncated-ancestry DAG semantics** defined — what an absent justification *means* to `BlockDagStorage::insert`, to `finalized_blocks_set`'s seen-closure and to the finalizer's `seen.contains` gate — and a change that size has no law to be checked against; the register would have to gain a requester model first, which is the honest order of work. **Two measurements carried here rather than re-derived**: the requester's largest fixture is a **6-block chain** (`chain(6)`, `lfs_block_requester.rs`), so the §6 row's 6,300-round-trips-vs-~50 is a live/devnet figure with no unit-level before/after; and C64 already attributes the ~25 s per generation to the syncing node's shared message loop rather than to the requester's own loop, which paces faithfully (`the_walk_advances_on_responses_not_on_the_idle_timeout`: 3.85 ms for a six-block walk against a 30 s idle timeout).  **CORRECTED (2026-09-27, AUDIT C64): "the Scala's cutoff" is not a thing the Scala does.** The cutoff is inert in the oracle's production path — `LfsBlockRequester.scala`'s only `ST` construction (`:318`) never seeds `lowerBound`, so both of its gates are `>= 0` and vacuously true (the full derivation is on the §6 row above). Both trees walk the full ancestry, so the port downloads the *same* set of blocks rather than a superset, and there is no "truncated-ancestry DAG semantics" for this row to wait on. **What this row still owns is unchanged and now sharper**: grep every `.lean` under `spec/Rchain/` for `lowerBound`, `extraHeights`, `requestStream`, `LfsBlockRequester` or `cutoff` and nothing answers — the requester is unmodelled, so the port's fidelity here rests on `legacy/` and on the witnesses in `lfs_block_requester.rs` (`a_walk_longer_than_deploy_lifespan_reaches_genesis`, added for C64) rather than on a law. The row stays open for that reason alone |
| C95 the model's fold published a **stale** same-sender message — `layerInsert` dropped the port's `sender_seq` guard, and that guard is what the port's own comment calls the Law 15 monotonicity invariant | 15 | **found by lifting law 15's sentinel theorem through the derivation's fold, and chasing the counterexample until it stopped being about the port** (2026-09-25). The port's `calculate_next_layer` gives a candidate its sender's slot **only when its `sender_seq` is strictly greater** (`block-storage/src/dag/finalizer.rs:160-166`), and the port's own comment names that guard — `sender_seq` replaces the sender's latest message, *"the Law 15 monotonicity invariant"* (`message_state.rs:77`). The model's `layerInsert` applied the **seeding** rule (the port's `BTreeMap::collect`, last wins) to the candidates as well, and the justification in its doc — *"the antichain does not depend on that choice"* — is true of law 14b's **keys** and false of the **message** law 15 is about. **The difference is visible on the file's own instance**: `dag3`'s published layer was `[10, 12]` and is now `[11, 12]`, so the model was publishing a stale same-sender message that no law-14b check could see, because the keys were right either way. **Fixed** in the model by splitting the two rules (`seedLayer` for the seeding, the guarded `insertCandidate` for the candidates), with law 14b's antichain proof repaired around them (`seedLayer_nodup`, `insertCandidate_nodup`, `foldl_insertCandidate_nodup`, `nextLayer_nodup`) — law 14b's *statement* is untouched, which is the point: the simplification was sound for it and unsound for law 15. **With the faithful fold the counterexample narrows to `fold4` and nothing else**: stated per sender, `the_fold_can_publish_below_the_previous_fringe` holds of a fork-free DAG whose `101` justifies the older `99` while `100` is finalized — a shape the **sequence rule** refuses — and `fold5` refutes nothing, since with the guard it publishes `z` (height 6) for sender 0 and the comparison holds (`the_guard_keeps_the_newer_message`, `the_comparison_holds_on_fold5`). So law 15's remaining lift rests on the sequence rule, on a fold that now mirrors the port. **What this row cost, kept rather than tidied away**: four revisions in one sitting — a guess at fork-freedom (refuted by `fold4`), a guess at the sequence rule (refuted, it turned out, only by the model's own unfaithful fold), a claim that the port admitted the shape (refuted by reading `calculate_next_layer`), and this, which is what the machine says. **Operator consequence: none** — the port always had the guard; the defect was the model's, and it is the kind that only a comparison *about the published message* could ever see |
| C96 the five constructs law 1a could not state are in the model — each its own class, with the node's own interleaved tag, and seventeen new corpus rows pin them | 1a | **C58's gap, closed, and its own prediction about *why* the gap was structural is what the fix followed** (2026-09-25, the H6 unit). C58 measured eight missing `Expr`-level constructors and said the remaining gap "is structural, and its cost is the comparator block, not the constructor list"; three of the eight were conflations closed earlier, and the five left — `BIG_INT`(13), `EMETHOD`(115), `EPERCENT`(119), `EPLUSPLUS`(120), `EMINUSMINUS`(121) — are the subject here. **The plan's own framing was the thing that had to change first**: it said "a `Ground.bigint` if `BIG_INT` is to be a ground", and that is the shape C58 had already measured as blocked — `exprTag (.ground _)` would have to inspect its payload, making it a stuck `match` for a variable ground, which is exactly what breaks the cross lemmas' `simp_all` closure. The protobuf settles it the other way: `g_big_int` is an **`Expr`** variant (`models/proto/RhoTypes.proto:174`, beside `g_bool`/`g_int`/`g_string`/`g_uri`/`g_byte_array`) and the port's normalizer *converts* the parse-level ground into one (`rholang/src/normalizer.rs:74-77`, `Ground::GroundBigInt -> Expr::GBigInt`), so `ebigint` mirrors the node and `Ground` is untouched — `cmpGround` and `groundComparator` did not move, and `Ground.bytes`'s tag stays the one unpinnable divergence. **The five are five classes with literal tags, in the node's interleaved positions** (13 behind the collections and ahead of the vars; 115 between `EOR` 114 and `EMATCHES` 118; 119-121 between `EMATCHES` 118 and `EMOD` 122), not leaves of `ground` or of neighbouring operators — the "case split finer than one class per constructor" C58 named, taken where it is a class boundary rather than inside one. **The measured cost, which C58 predicted and this confirms**: the arm-lemma family grows 24 → 29 classes (5 `cmpExpr_*` pair lemmas, 5 `exprTag_eq_*` extraction lemmas, 10 tag cross lemmas) and each of `cmpExpr`'s three laws gains five arms; `emethod`'s score is three deep (`[tag, name, target, args…]`), so `Rchain.Comparator.lex_lt_trans_at3` was added to `Cmp.lean` — a three-component pointwise lex, the same shape as the two-component one `Sort.lean`'s block already used. **Two things measured on the way, recorded rather than smoothed over**: (1) `EMETHOD`'s name is a **code-point list** in the model, not a `String` — a `String` field drags its `DecidableRel` instance into the comparator's *statements* and no law of this file's shape can be stated over it consistently, which the build showed as a `String.LT'` vs `Preorder.toLT` mismatch inside `cmpExpr_eq_iff`; UTF-8 preserves code-point order, so the two orders agree. (2) `emethod`'s trailing `connective_used` score leaf is omitted because it is *derived* from the target and the arguments, so it can never separate two terms the parser can produce — the node's own `unequal_emethod_have_unequal_scores` needs hand-built ASTs. **And the guards**: the test that failed first was the *dispatch* — `exprTag_eq_<class>` succeeds vacuously when the tag is stuck, and `rw`'s failure is fatal rather than recoverable, so one mis-guarded arm aborts the whole `first` chain; the five cases are settled by `by_cases` on a literal tag *before* `cases s` generalizes `s`, with contradiction closers ahead of the chain. **The tie is the corpus**: rows 26-42, seventeen verdicts, each read off the node before the row was written (`sortCases_decide` passes on all 42 and `cargo test -p rchain-rholang --test lean_sort_corpus` agrees), plus `Surface.lean`'s five arms from `none` to emitting, which retires five `surfaceBoundaries` rows (15 → 10). **Operator consequence: none** — the model and the node agreed on every pair before the change; the five were terms the node could hold and the model could not, so law 1a was unstatable for them, and now it is stated and tied |
| C97 four ingresses where a peer's bytes reached a fixed-width constructor's length assert — R12 certified the class while these four paths were live | — (R12's ingress class; no law states it — `TYPE-SYSTEM.md` §1.7) | **A red-team audit falsified R12's closing sentence by tracing the socket to the assert, four times** (2026-09-26). Three are the hash-carrying request types: `HasBlockRequest`, `HasBlock` and `BlockRequest` each carried `pub hash: Vec<u8>` (`models/src/casper/protocol/casper_message.rs:HasBlockRequest`), their `CasperMessage::from_proto` arms cloned it unchecked beside a checked sibling arm, and the handlers' **first** statement was `BlockHash::from_slice` — `casper/src/engine/node_running.rs:handle_block_request` before `limiter.allow`, and the `HasBlockRequest`/`HasBlock` arms likewise. `from_slice` opens with `assert_eq!(bytes.len(), LENGTH)` (`models/src/block_hash.rs:from_slice`), so three bytes from any peer that completed the handshake unwound the task that owns the shard's peer-message loop; the `JoinHandle` is dropped and nothing restarts it, and the router's later `let _ = tx.send(..)` fails silently, so the shard stopped consuming every peer message with no recovery signal. **The fourth is the one no enumeration named**: `Event::from_proto` cloned its hash fields as `Vec<u8>` and `casper/src/event_converter.rs:bytes_to_hash` handed them to `Blake2b256Hash::from_byte_array`, whose length assert unwinds the same way — reached by *block replay* of a peer's `BlockMessage` (`casper/src/runtime_replay.rs`), so the same three bytes killed a validator through two unrelated doors. **Fix:** the three fields become the `BlockHash` refinement (`BlockRequest::from_proto` now `BlockHash::try_from(m.hash.as_slice())?`, the canonical `TryFrom`/`as_bytes().to_vec()` pair the seven R12 types already used), so the panic is unconstructible rather than guarded and the handlers read the hash by type; `Event`'s four hash fields are length-checked at ingress by `wire_hash`/`wire_hashes` in the same `from_proto`, with the fields left `Vec<u8>` (re-typing them would reach the `Vec<u8>`-valued helpers in `casper/src/api/block_api_impl.rs`, so that is a recorded boundary, not an omission). **Falsifiers:** `block_request_with_a_short_hash_is_refused`, `has_block_with_a_short_hash_is_refused`, `has_block_request_with_a_short_hash_is_refused`, `block_request_from_bytes_refuses_a_short_hash` and `event_with_a_short_hash_is_refused` (`models/src/casper/protocol/casper_message.rs`, each red on the pre-fix tree and green after). **What the fix deliberately did not do:** `models/src/wire.rs:bytes_to_bitset`'s `as i32` was flagged by the same audit and is **latent, not live** — the truncation needs ~256 MiB of `locallyFree` (bounded by the configured message cap), so the residue is an input-proportional (~32×) expansion rather than a panic; and the pre-existing test that pinned the defect as intended behaviour — `block_request_serde_round_trips` built `hash: vec![1, 2, 3]` and asserted it round-trips — is re-typed to a real 32-byte hash with the note at the test. |
| C98 three classes the gate could not see, the crate roster it never scanned, and four panic sites that gap was hiding | — (the instrument itself; no law states what a scanner must look for — `TYPE-SYSTEM.md` §1.6/§3.2) | **The 2026-09-26 audit's second finding was about the gate, not the tree: `tools/audit-type-system.sh`'s hard classes are panic/unsafe/silent/escape and its soft classes (cast/lax/get) were printed and never compared to anything, so variable indexing, slice ranges and division had no class at all and overflow existed only as clippy's opinion — while `TYPE-SYSTEM.md` §1.6's "no silent partiality" is the claim this gate is cited as enforcing.** Five changes, each one falsified before it was believed. **Every count in this row is that day's
measurement and is dated by the row; the live numbers are `tools/type-system-baseline.tsv`'s, by
construction, which is the point of the ratchet — a number restated in prose is a number nothing
re-checks.** (1) **`cast`/`lax`/`get` are compared in both directions against `tools/type-system-baseline.tsv`** — a rise *or* a fall fails the build until the number is changed in the same commit, the coverage floor's rule (`audit-test-register.sh`'s `floor(measured) − 2`) applied to a count: a number that can move without a commit saying so is a number nobody is watching. (2) **`index`** (313 sites) and **`div`** (49) are new classes; the first earned its keep on its first run by pointing at `rholang/src/reduce.rs`'s `EDiv`/`EMod` arms — which are *guarded* (`is_zero(&v2)` returns `ReduceError` one line above the closure), which is the honest character of a census: it points, and the guard is a judgement a grep cannot make. (3) **`overflow`** (41) is scoped to the eleven refinement files, for the reason the escape class gives its own scope — an unguarded `a - b` in an ordinary module is a design choice, and the four `self.0 - rhs` in `shared/src/refined.rs` are `NonNegI64`'s `Sub` leaving its own domain. A bare arithmetic grep was **rejected as the instrument**: it is thousands of sites and no judgement. (4) **`get`'s literal-index alternative was all of it** — accessor half 0, literal-index half 95 of 95 on this tree — so the class now means what its name says and the 95 are counted where they belong, as part of `index`. (5) **The roster was wrong in both directions**: `regex` is not a crate and was skipped silently by the `[ -d "$c/src" ] || continue` guard, and `qucalc` — a real workspace member, and the crate whose `tally_ranked` is reachable from rholang as the `ranked` system process (`rholang/src/system_processes.rs`) — was never scanned at all. Fixing it surfaced **four production panic sites** that had never been reviewed (`qucalc/src/lib.rs::tally_ranked`'s two `unwrap`s, guarded two lines above by `counts.is_empty()`, and `qucalc/src/main.rs`'s load-failure `panic!` and data-dependent `expect`); all four are now total rather than allowlisted. The gate now fails on a roster entry with no `src/`. **Verified by falsification, not by inspection**: a planted `bytes[3]` site makes `index` fail (rc=1); a baseline one above the measurement makes it fail (rc=1); a roster entry with no `src/` fails (rc=1); the clean tree passes (rc=0). **Both numbers for the class the grep cannot judge**: `clippy::arithmetic_side_effects` reports >= 33 sites in `shared/` and `qucalc/` alone before it aborts the build (lower bound — clippy never reached the dependent crates), which is the precise instrument and the named follow-up: a lint is a judgement, a grep is a census, and this row records both rather than pretending the census is the judgement. **And the counts are engine-sensitive, measured**: `[[:alpha:]]\[[^;]+\]` matched 3 of 3 under awk and **0** under this machine's `ugrep`, so the new patterns use `[a-zA-Z0-9_]` — a count compared to a committed number must not mean two things to two regex engines. |
| C99 the two parser guards do not compose: a ~4 KB deploy term aborts the node, and a 32 KB term costs 64 s of CPU before any phlo is charged | — (no law states that parse or normalize terminates in a bounded resource; `TYPE-SYSTEM.md` §1.6's totality is the nearest claim) | **Confirmed by measurement, not by reading, and the measurement is the row** (2026-09-26). `MAX_PARSE_DEPTH` (128, recursion) and `MAX_CHAIN_LENGTH` (512, one flat chain) are independent: the chain is built by a loop, so it costs a bounded number of parser frames and produces an **AST of depth `n`**, and `d` levels of `c` operators compose into `d × c` with neither guard firing. Every consumer then recurses once per level — `normalize_proc` (`rholang/src/normalizer.rs`), `sort_par`/`sort_expr`, `well_scoped_par`, `eval_expr_to_expr`, `spatial_match_core`, `build_string` — with no bound. **On the node's own 32 MiB worker stack** (`node/src/main.rs`): a debug build normalizes AST depth 732 in 739 ms and **aborts** at 994 (`fatal runtime error: stack overflow, aborting`, SIGABRT — the whole *process*, where D1/C97 killed one shard's task); a release build survives 3,000 in 3.2 s, costs **64 s** at 8,040, and aborts between 8,040 and 50,100. Cost is super-linear (157 ms at 732, 292 at 994, 537 at 1,296, 927 at 1,638, 3.2 s at 3,000). **The trigger is small and free**: 3,949 bytes is the debug abort, and the deploy path runs parse+normalize *before* any phlo or balance check (`casper/src/multi_parent_casper.rs`, `add_deploy` after `source_to_adt_with_env`), so the CPU is the attacker's to spend; a peer's replayed `BlockMessage` reaches the same path (`casper/src/runtime_replay.rs`). **Fix:** `MAX_AST_DEPTH = 768` in `rholang/src/parser.rs`, enforced by `exceeds_ast_depth` — an **iterative, early-exiting** walk over the parsed tree, deliberately not a recursive one (a recursive walk would overflow the stack it exists to protect) and deliberately not parser-side *accounting*: the first attempt charged chain links against a counter saved and restored by `with_depth`, which **under-counts**, because a chain's depth is its first operand's depth plus its links and the operand's depth is discarded on return. The measured walk was chosen after that attempt failed its own test, which is the reason the falsifier below exists. **Falsifiers:** `rejects_a_deep_ast_that_stays_inside_both_component_guards` (4 levels × 200 links: inside *both* guards, ~800 deep, refused), `accepts_a_deep_ast_under_the_budget` (the control: the same shape at ~303 deep still parses, so the refusal cannot be a generator that emits nonsense), `a_maximal_chain_still_parses` (the budget must not quietly lower `MAX_CHAIN_LENGTH`). **The constant is measured, and its safety is measured too**: 768 sits below the *debug* abort with margin — the weakest configuration that ships is CI's and a developer's, not the node's — and it is 1.5× the deepest legitimate flat chain (513), so nothing that parsed before stops parsing: sweeping every `.rho`/`.rhox` in the tree gives **36 parsed, 1 refused**, and the refusal is `Pos.rhox`, a `$`-substituted template that is not meant to be parsed raw. **The residual this row recorded is clause b's subject, and it is closed for one route only (AUDIT C100):** a *runtime*-built deep value never passes the parser, and the space now refuses a produced value deeper than `MAX_VALUE_DEPTH` (256) — so the route this row could not see is bounded at the boundary a produced datum crosses, which is where every later reader's recursion starts. **What that does not close is the *parsed* term**, and this row must not be read as closing it: a legal chain of ~350 links as send data (inside both parser guards) still overflows the stack in the evaluator *before* the space sees the datum, because `MAX_AST_DEPTH` (768) sits above the evaluator's measured abort depth (~330) — the parser's bound was measured against the normalizer's frames, and the evaluator's are larger. That is a separate finding of this pass, as is the parser's own exponential cost on nested tuples. The parser's own comments asserted the two guards bounded the AST, and the one on `MAX_PARSE_DEPTH` claimed 128 was "well within a 2 MiB async-thread stack" while `main.rs` sets 32 MiB — both corrected in place. **Two defects in this row's own clause-a accounting were found on 2026-09-28, by review, and have their own rows** (AUDIT C167 and AUDIT C168): the falsifier this register names for clause a could not fire on the witness shape it named, and the three tests this row lists as clause a's falsifiers are the guard's boundaries rather than the walk's descent. |
| C100 a **runtime-built value** aborts the process: a fold to depth 401 (a few hundred bytes of source) kills a validator, and the fold's depth costs `O(n)` reduce steps inside the step budget | **50b** (the space's bound on a value's depth; clause a is the parser's) | **Measured, not read** (2026-09-26). A contract folding its accumulator into a deeper pair reaches depth `n` in `O(n)` reduce steps — inside `DEFAULT_MAX_REDUCE_STEPS` (100,000) and the explore path's cap (10,000) — and every consumer of the stored value then recurses once per level. On the node's own 32 MiB worker (`node/src/main.rs`) in a debug build: a fold to depth **101 costs 7.3 s of CPU**, and a fold to **401 aborts the process** (`thread 'tokio-rt-worker' has overflowed its stack`, SIGABRT) inside `eval_single_expr`'s recursion over the value. No deep term is ever parsed, so C99's parser bound cannot see this route at all. **Fix:** `models/src/types.rs::exceeds_value_depth` (an iterative, early-exiting walk — iterative because the depth is what a recursive walk cannot survive) called from `rholang/src/storage.rs::ChargingRSpace::check_value_depth` at `MAX_VALUE_DEPTH = 256`, **before** the storage charge, so a refused value is one the space never accepted and never billed for. **The gap this unit's own comment hid, found by reading the claim rather than the code:** the guard's first draft sat inside `produce` under a sentence calling it "the one place a runtime value enters the space", while `produce_at` — the *scheduled* path the channel scheduler and the deferred block paths use — reaches RSpace directly and never passed through it, leaving one of two produce entries open. One shared method now, and the test fails on the old shape: with the scheduled call disabled, a 257-deep datum is *stored* (`ScheduledProduce { application: None, .. }`, no error). **Why 256 and not the parser's 768:** the frames differ — the parser route's overspill aborts at an AST depth of ~1,000, the value route's `eval_single_expr` recursion at ~401 — so one number would be a bound that never fires before the crash it prevents (which is what the first draft shipped, at 768, and the test aborted the process). **Falsifiers:** `models/src/types.rs:every_construct_that_carries_a_par_is_walked` (a depth-10 child parked in each of the 22 positions across `Par`/`Expr`/`Connective`, asking for a limit of 5, so a dropped arm is the only way a case passes — verified by disabling the `bundles` arm and watching exactly its case fail), `models/src/types.rs:the_value_walk_admits_the_limit_and_refuses_past_it` (the boundary pair on both sorts), `rholang/src/storage.rs:a_value_at_the_bound_is_stored_and_one_past_it_is_refused_on_both_produce_paths`, and `rholang/tests/deep_value_bound.rs` (the fold that aborted, now refused — 3m40s, which is why it is not a law witness). The ordering between the two bounds is carried by the definition rather than by a check: `MAX_VALUE_DEPTH` is written as the smaller of 256 and `crate::parser::MAX_AST_DEPTH`, so a parser bound lowered below 256 lowers it with it. Asserting it was tried twice and refused twice — a `#[test]` over two constants can never fail, and the `const`-block the linter suggests instead is an `assert!`, i.e. a panic site in production code, which **this** gate counts. **Recorded residues, not claimed closed:** the guard bounds the values the space *holds* and so every later reader; it does not cover a produce's **channel**, a consume's **channels/patterns**, a **continuation's body**, or state restored from persistence at boot; nor does it bound the **evaluator's** recursion while building the value, since the guard runs after that walk — the honest mechanism is that the deepest value ever built is capped at the bound + 1. **The accounting clause b's proof corrected, and the slack that was never a slack** (2026-09-28). This row said the walk gives an `Expr` node no level of its own where `parDepth` counts it, and that the gap is therefore a slack of at most `MAX_PARSE_DEPTH` (128) because no runtime path builds `Expr` nodes. Both are wrong. The walk gives **no element node** a level — `push_value_fields` pushes its fields at the depth it was handed, as do `push_value_expr` and `push_value_connective` — so `Send`/`Receive`/`New`/`Match`/`Bundle`/`MatchCase`/`Connective` are transparent along with `Expr`, and the counted quantity is the number of **`Par` nodes** on the deepest `Par`-chain (`Rchain.parNestDepth` in `Rchain/ValueDepth.lean`), not `parDepth`. And a runtime path *does* build `Expr` nodes: `(a, b)` **is** `Expr::ETuple`, built by the reducer, which is the shape `rholang/tests/deep_value_bound.rs` exists to exercise. The gap is a **factor**, not a constant, and it is exactly 3: two element nodes can sit between consecutive `Par`s (`Receive`→`ReceiveBind`, `Match`→`MatchCase`), so a `Match` ladder spends three levels per `Par` and a factor of 2 fails from two rungs — `Rchain.parDepth_le_three_mul_parNestDepth` proves the 3 and `parDepth_pairsDepth`/`parNestDepth_pairsDepth` show the shape. At this bound the two numbers are `3 × 256 = 768` and `maxAstDepth`. Two boundaries the tie carries: the guard never pushes its root (its fields go on at depth 2), so it accepts a value with no `Par` child at `limit = 0` where the model refuses it, and the two agree for every `limit ≥ 1`; and the guard walks `New.injections`, which the model's `New.mk` does not have — stricter, the safe direction. **This correction has its own row** — AUDIT C169 — and the modules it names are `spec/Rchain/ValueDepth.lean` and `Rchain/ValueDepth.lean`'s falsifier pair. |
| C101 a **parsed** term still aborts the process: a legal ~350-link operator chain as send data overflows the evaluator's stack, because `MAX_AST_DEPTH` (768) sits **above** the evaluator's abort depth | **50a** — and this row is what falsifies its constant for one of the consumers it names | **Measured 2026-09-26 while probing C100 — C99's own class left open — and fixed in the same pass (the fix, its argument and its falsifier are at the end of this row).** C99 measured its 768 against the *normalizer* (aborts ~1,000). The **evaluator** is a different, larger frame family: a flat chain as send data — inside **both** parser guards (one nesting level, `n < MAX_CHAIN_LENGTH`) — returns `Ok` at 300 and 320 links and **SIGABRTs** (`thread 'tokio-rt-worker' has overflowed its stack`) at **350, 400 and 500**; 700+ is refused by the chain guard first. `resolve_send` (`rholang/src/reduce.rs`) evaluates every datum (`eval_expr`, `substitute_par_and_charge`, `SortedProc::new`) **before** `produce` is called, so C100's guard cannot help on this route: the abort is inside `eval_single_expr`'s recursion over the *expression*, and the value the guard would inspect does not exist yet. **The trigger is ~2 KB and free**, and a peer's replayed `BlockMessage` reaches the same path. **Fix (2026-09-26, same pass): a left-nested operator chain is evaluated iteratively** (`split_left_spine` + `apply_spine_level` in `rholang/src/reduce.rs`), so the chain's length costs no stack at all. The fix is *literally* stack-only, and that is its main correctness argument: each level still runs **the same arm** in `eval_arm`, rebuilt as a one-level node with the already-evaluated left value in place, so the operator's cost, its dispatch (`Set + x`, the `++` unions, `%%` interpolation) and its errors are not transcribed anywhere — and it is *gas-exact*, because an arm's result is a value and **evaluating a value charges nothing** (`eval_expr_to_par`'s fallthrough is the leaf arm, a clone). The short-circuiting pair is the one exception, and it is shared rather than copied: `bool_short_circuit` takes its right operand as a thunk, so the rule that a decided `&&`/`\|\|` neither charges nor can raise on the right lives in one place. **What it does not do, named rather than implied:** the walk folds *left-nested binary* chains only — the shape whose depth is a spine rather than a nesting — while right-nested chains (which need parentheses, so `MAX_PARSE_DEPTH` bounds them at 128), unary chains (`not`), collections and `EMethod` keep their recursion, each bounded by the parser's guard and C100's value bound. **Falsifiers:** `rholang/tests/deep_expr_chain.rs:a_deep_parsed_chain_evaluates_without_overflowing_the_stack` — 10, 350 and 500 links, where 350 is the measured abort — which with `split_left_spine` disabled **SIGABRTs** (`thread 'tokio-rt-worker' has overflowed its stack`), and which checks the *value* as well as the absence of an abort (the contract divides by zero unless the chain evaluated to the number it should, so "no error" cannot mean "evaluated to something else"). Plus the semantic guard: `legacy_contracts` (73 corpus programs), `rho_examples`, `system_process_conformance` and the corpus layers, all unchanged. **Law 50a's note is corrected with it**: the sentence that no consumer recurses past the bound is true of the evaluator again, but *by construction* rather than by the constant. |
| C102 nested **tuples** make the *parser* exponential: 118 bytes costs 4.8 s of CPU before any phlo, and 30 levels does not finish in 45 s | — (no law states that parsing terminates in a bounded resource; the nearest claims are `TYPE-SYSTEM.md` §1.6's totality and C99's depth bound, and neither reaches this) | **Measured 2026-09-26 while probing C101, found by the control rather than by the probe, and fixed in the same pass (the fix and its two falsifiers are at the end of this row).** The `(` of rholang is ambiguous between a group (`PExprs`) and a tuple, so `parse_proc11_head` (`rholang/src/parser.rs`) tries the group *speculatively* and, on a comma, rewinds (`self.pos = save`) and lets the collection path **parse the same text again**. That re-parse re-enters the next level's speculation, so the cost is 2^n — and the comment above it asserts the opposite ("bounded by the nesting of `(`"), which measurement falsifies: parse-only, on a 32 MiB stack, in a debug build — 10 levels **7 ms**, 20 levels **4.8 s**, 30 levels **> 45 s**. The controls isolate the mechanism exactly: nested **groups** `((x))` are linear (1.6 ms at 20 levels, 4.6 ms at 60) and nested **lists** are linear, so it is the comma-carrying parens and nothing else. No depth bound can catch it (depth 20 is far below every bound in the tree), so C99's and C100's constants are irrelevant here. **Fix (2026-09-26, same pass) — and the first attempt at it was wrong in a way worth keeping.** `parse_proc11_head` now decides tuple-versus-group by **scanning the tokens** for a top-level comma (`lparen_opens_a_tuple`) and hands a tuple straight to the collection path, so the interior is parsed **exactly once, by the full grammar**; only the group case is still speculative, and its rewind costs the few tokens the `Proc4` grammar cannot continue, not a re-parse. After it, 20 levels parse in 3 ms and 28 (this shape's ceiling under `MAX_PARSE_DEPTH`) in 0.5 ms, where 20 cost 4.8 s and 30+ did not finish. **The first attempt — the obvious one — was to build the tuple from the group attempt's own partial parse, and `legacy_contracts` caught it:** `parse_proc4` reads `bundle+{*m}` as the *arithmetic* `bundle + {*m}` (the `bundle` keyword production lives at a higher grammar level), so a tuple whose first element is a bundle — `MultiSigRevVault.rho` has four, e.g. `(bundle+{*multiSig}, revAddr, revVault)` — got a **wrong tree**, and the normalizer refused the file with *Free variable bundle is used twice as a binder*. The lesson is in the function's doc comment: a speculative parse's *result* is not an oracle for what the full grammar sees, only its *position* is. **One cost is named**: deciding on tokens spends a few units of `MAX_PARSE_DEPTH` per tuple level, so this shape now tops out near 28 levels where 32+ reports `parse depth exceeded` in microseconds — against an old route whose 40 levels did not complete in 900 s. Every depth at which the old parser returned `Ok` in measurable time (20) is still accepted, and `MAX_PARSE_DEPTH` is the port's own guard, not the grammar's, so no term that worked became refused. **No parse tree moved** — `lean_parse_corpus`, `lean_normalize_corpus`, `lean_sort_corpus`, `lean_closed_corpus` and `legacy_contracts` (73 corpus programs) all pass — which is why no `Hard fork:` row accompanies it. **Falsifiers:** `rholang/src/parser.rs:nested_tuples_parse_in_linear_time` (depths 10/20/28 under a deliberately loose 2 s ceiling; with the rewind restored it fails at **4.804938383 s**, the original probe's number to the millisecond, which is what makes it a falsifier and not a benchmark), `legacy_contracts` (the wrong-tree bug's own falsifier — it failed on the first fix), and those four corpus layers. |
| C103 the C-number allocator fed `comm` a *numerically* ordered list where `comm` requires a *lexical* one — so it exits non-zero at the first three-digit finding, and every `C<n>` reference in `spec/` reads as dangling | — (the instrument itself; no law states what an allocator must do — the nearest is C91's rule that a check's exit code must reflect the work that ran) | **Found by the register gate's own guard, on the commit that added C100, which is the whole finding.** `tools/next-audit-number.sh` builds its in-use set with `sort -n -u` and pipes it into `comm`, which requires lexical order. While every number in the tree was two digits the two orders agreed wherever `comm` looked, so it exited 0 and its gap list (`66`) was right for the wrong reason; C100 made the orders disagree for the first time (`100` sorts before `99` lexically), `comm` printed `file 2 is not in sorted order` and exited 1, and `set -e` took the script down before it printed its set. The consequence was not a wrong gap but **745 failures**: the gate reads that in-use set as the oracle for every C-number cited in `spec/`, so a tool that printed nothing made C18, C38, C52, C98 — the whole register — read as dangling. It was caught in exactly the shape C91 exists for, by the gate's own vacuity guard: *listed no allocated C-numbers — every reference below would read as dangling, so this check is vacuous until that tool works*. **Fix:** both sides of the `comm` are now explicitly `LC_ALL=C sort`ed and the result restored with `sort -n` for display, so the ordering is stated rather than inherited from the catalog's length. **Falsifier:** the tool now exits 0 and prints `next free: C103` with the historical gap `66` still listed; restoring the two `sort`s reproduces the empty stdout and the exit-1. |

| C104 the port's channel lock **weakened the oracle twice**: a get-then-insert where the Scala's is atomic, and a `cleanUp` call the Scala makes and the port dropped — so one key could be locked twice at once, and the lock map was a record of every channel since start-up | — (no law states how a lock map behaves; the oracle here is the Scala, and the port was weaker than it in both halves) | **Both halves are ports of a Scala that was correct, and both were found by reading the port against it.** (1) **The race.** `MultiLock::acquire` did `locks.get(&key)` and, on a miss, a separate `locks.insert(key, new)` — two racers that both miss build *two different* `Arc<Mutex<()>>` for the same key and then lock their own, so the key's mutual exclusion is silently gone. The Scala's `locks.getOrElseUpdate(k, s)` on a `TrieMap` is **atomic**, which is exactly what the port's two steps lost. (2) **The leak.** The Scala's `MultiLock.cleanUp` (`Sync[F].delay(locks.clear)`) is composed by `TwoStepLock.cleanUp` and called from `RSpaceOps.reset` under the comment *Clean channel locks* — and the port's `RSpace::reset` did not call it, so the map kept one entry per distinct channel the process had ever touched, for the life of the process. Both are invisible in the ordinary case: the first needs true concurrency inside a window a few instructions wide, the second is a monotone map nobody reads. **Fix:** the get-or-insert is one `DashMap::entry(..).or_insert_with(..)` (one shard lock across the miss and the insert), and `clean_up` exists on both `MultiLock` and `TwoStepLock` and is called by `RSpace::reset` **and** `ReplayRSpace::reset` — the replay space acquires through a `lock_f` of its own, so the play space's call cannot reach it, which is why there are two and not one. **Falsifiers, both written before their fix and both failing on the old code:** `rspace/src/concurrent/multi_lock.rs:concurrent_misses_on_one_key_still_share_one_mutex` — eight tasks released together by a barrier over 32 rounds, each round on a **fresh key** (a key already in the map makes the `get` succeed and the race unreachable, which is how the first draft of this test passed against the broken code), which reports an overlap on the old code — and `rspace/src/rspace.rs:reset_releases_the_channel_locks` / `replay_rspace::tests:reset_releases_the_replay_spaces_own_channel_locks`, which read non-zero after a `reset` with the call disabled. |

| C105 a full routing queue **discarded the packet** while the transport acked the peer as though it had been handed on — and the ack is the node's only word on the subject | — (no law states what an ack means; the nearest is R12's ingress class, and this is the same boundary one layer in) | **The drop was one line.** `rp/handle_messages.rs`'s packet arm did `let _ = routing_queue.try_send(..)` — `mpsc::try_send` returns `Full` when the 50-slot queue is busy, the error went into `let _`, and the handler answered `HandledWithoutMessage`, so the transport's `ack` (`grpc_transport_receiver.rs`) told the peer the packet was taken. Under load, that is a peer's block or deploy quietly discarded with a success reply. (The *receiver* spawns the dispatch and acks unconditionally — `let _ = (dispatch)(protocol).await` — which is the same lie one level out, and is why an answer of "not handled" has to come from the handler at all.) **Fix:** the send is awaited, so a busy node applies **backpressure** rather than dropping — the policy the streamed path already uses (`node_runtime.rs`'s `handle_streamed` awaits the same queue) — and the queue's *closed* case (the router task gone) answers `NotHandled` instead of `HandledWithoutMessage`, because a packet that cannot be delivered at all is not one that was handled. **The decision is recorded, not implied:** an ack on this path means *taken into the pipeline*, not *processed*, which is what it meant for every other message already. The alternative — making it mean "processed" — was rejected on shape: the dispatch is `tokio::spawn`ed, so honouring its result means awaiting it inline, and that would put a **peer round-trip** (the handshake path's outbound send) inside the RPC handler's critical path. What a packet needs is not to be lost; the ack's wording was never the harm. **One hop later, the same class of silence — and a correction to this pass's own recon:** the router's per-shard `tx.send(..)` was reported as another drop-on-full site, and it is not: it already awaits (so it blocks when a shard is busy). What it did was discard the *error*, whose only remaining cause is a **closed** channel — that shard's task has exited, so the message can never be delivered — and that is now logged rather than swallowed. **Falsifier:** `comm/src/rp/handle_messages.rs:a_packet_is_not_dropped_when_the_routing_queue_is_full` — fills a one-slot queue, lets the handler reach its send, and requires that it is still *waiting* (on the old code it has already answered `HandledWithoutMessage`, and the test's own message says so), then drains a slot **while polling the handler** (a future that is not polled makes no progress — this test's first draft forgot that and failed against the fix) and requires the peer's own packet to arrive. It failed on the old code with that exact message. |

| C106 eleven hand-written law totals across nine files — every one of them outside the scope of the check that exists to find exactly this | — **the instrument itself** (no law states how a document counts its catalogue; the nearest is the coverage ledger's rule that a number must be emitted) | **The rule was right and its scope was wrong.** `tools/emit-lean-counts.sh` scans for a stale total stated in digits, and it scans the files its `FILES` list names — a list that had grown by the very rule it enforces ("a count stated outside it drifts, because nothing recomputes it", written into the list's own comments) but had never been completed. Nine pages that state the catalogue by hand were outside it, and all nine had drifted: `docs/src/contributor/why-rust.md` (twice), `laws-to-rust.md` (twice), `spec/TEST-COVERAGE.md` (twice), `contributor/spec.md` (which had **two** stale totals in one row — "49 rows: the 29 calculus laws"), `qucalc/quantum-to-rho.md`, `rholang/reference.md`, `spec/RHO-CALCULUS.md`, `spec/RUST-VS-SCALA.md`, and `docs/src/ai-entrypoint.md`, whose total sat **immediately after a correct generated span**. **Two blind spots in the check, both now closed:** (1) it skipped any paragraph that carried a `<!-- counts:… -->` marker — so a *correct* span excused a hand-written total beside it, which is how `ai-entrypoint.md` hid; the spans are now removed from the paragraph and the rest is scanned. (2) `STYLE.md`'s own prescription named the files, as a second list that could disagree with `FILES` — it was the *in* set, so the pages outside it were beyond the rule as documented; that sentence now points at `FILES` instead of enumerating. **And a limit that is named rather than papered over:** the scan is a stale *number* followed by a word boundary and a noun (`laws`, `entries`, `axioms`) within 80 characters — so a reference with no noun near it is invisible to it, which is how `spec/INVENTORY.md`'s "Each row, like the 29, ends in a check" survived both the check and this row's first sweep. It was found by hand and reworded; widening the window would trade that miss for false positives on every other number in these files, and the honest statement is that the check catches counts *stated as counts*. **Fix:** the eleven totals are reworded to point at the register (per `STYLE.md`: a status or total restated by hand is one nothing checks), the two lists are one list, and the `FILES` entry for each newly-covered page says why it is there. `--check` now passes with the *stronger* scan, which is the point. **Falsifier:** reintroducing "the 29-law invariant catalog" in a newly-covered file fails the check (run and observed), and the check's message now prints the number it matched — the first version reported the paragraph and left the reader to find which total, which is how a check gets ignored. |

| C107 three surfaces nothing measured — the coverage instrument, the benchmarks, and the peer-wire ingress — and the audit's three recommendations each turned out to need correcting before they could be carried out | — **the instruments** (no law states how a suite is measured; the nearest is the ledger's own rule that a floor must be derived from a measurement) | **Part one, the coverage instrument: `--branch` cannot be collected here, and that is measured.** The audit asked for branch coverage and a floor. `cargo llvm-cov --branch` passes `-Z coverage-options=branch` to rustc, which the pinned toolchain (1.95.0, stable, `rust-toolchain.toml`) rejects — the build fails with *1 nightly option were parsed*, so collecting branch coverage would mean taking the workspace off its pin. The closest instrument the toolchain allows is **function** coverage, and it is derivable from the artifact the ledger already reads: **8809 functions, 7249 hit — 82.29%**, which is the audit's own figure (it existed in no committed file until now) and a shape a line count cannot see (a function nothing calls has no line covered at all). So `--fail-under-functions 80` joins the line floor, derived by the same rule (`floor(measured) − 2`), emitted to `spec/COVERAGE-LEDGER.md` as a second row and a per-file ranking, and compared against its site by check 11 — which needed **two corrections to stay non-vacuous**: it read the totals row by *shape* (`^\| [0-9]+ \| …`), which silently matched nothing once a label column was added, and it knew only about the line floor, so the new floor would have been a number nothing recomputed. **Part two, the benchmarks: the audit's premise was wrong, and the correction is smaller than the recommendation.** "No bench job in CI" is true of the workflows, but `cargo clippy --all-targets` *does* type-check the benches — so the rot the item feared was already covered, and what a job adds is the build and link. `cargo bench -p rspace-bench --no-run` is that job, with the comment saying which half it adds rather than claiming the type-check was absent. **Part three, the wire ingress: in-process, not the nightly fuzzer.** The audit asked for wire-protocol fuzzing; `tools/devnet-fuzz.py` cannot reach the p2p path (every mode it has attaches to HTTP explore-deploy) and runs on a cron, not on a PR. The path a peer's bytes take is two calls the router makes in sequence — `to_casper_message_proto` then `CasperMessage::from_proto` — so `models/src/casper/protocol/casper_message_protocol.rs` gains a corpus over exactly that chain: **a packet carrying a 3-byte hash is refused end to end** for all four hash-carrying types (a *different* claim from C97's, which pinned the constructors: this pins the three layers between a socket and them), with a 32-byte control, plus garbage/truncated/wrong-message bodies asserted only to *return* — an empty `ForkChoiceTipRequest` is a valid message, so "refused" would be asserting a bug. **Falsifiers:** the function floor's — lowering `--fail-under-functions` to 79 fails check 11 with the measurement's number in the message (observed); the ingress test's — with the casper half of the chain removed it fails, reporting the short hash reaching the casper layer (observed); and the corpus half is a panic-catcher by construction. |

| C108 the review no lens covered — `crypto/`'s signature verification and PoS laws 44–47 — and one finding in it: a production doc comment claiming a verification equivalence the verifier does not have | — (the nearest law is 19's axiomatized crypto boundary; the claim that failed is about the *verifier's* S rule, which no law states) | **The surface, read and now pinned.** Every verify path is total and refusal-shaped: `Secp256k1::verify_bytes` is `let Ok(..) else { return false }` three times over (DER, SEC1 key, prehash), `Secp256k1Eth::verify` refuses a bad DER conversion, `Ed25519::verify_bytes` likewise, and `Signed::from_signed_data` answers `Some`/`None` — **fail-closed**, which is the direction that matters. The three call sites a peer or a deploy reaches agree: `validate::block_signature` and `DeployData::verify_signature` both map an *unknown* algorithm to `false` (a refusal, not a skip), and the one production consumer, `casper/src/api/block_api_impl.rs`'s `if !deploy.verify_signature()`, rejects. **What the partiality gate cannot say for us, and why this row needed a test**: the gate's hard classes are `panic!`, `unsafe`, silent conversions and `escape`, and a bare `.unwrap()` on a `Result` is none of them — so "the verify paths are total" is a claim only a test pins. `signatures_alg.rs:every_algorithms_verify_refuses_malformed_input` feeds both enabled algorithms empty, 1-byte, 63- and 65-byte, all-ones and 4 KB inputs and requires `false` from each. **The finding — a claim I set out to confirm and could not.** `normalize_signature_low_s`'s doc said the normalization "does not change verification semantics (both the high-S and low-S forms verify against the same public key)". Written as a test of exactly that (`a_signature_and_its_high_s_twin_verify_alike`, the same `r` with `s' = n − s`), it **failed**: the signer's own DER verifies and its twin does not — the verifier requires low-S, which is libsecp256k1's rule that the k256 port keeps, and the same holds on the `secp256k1:eth` path. **The behaviour is the oracle's, so nothing was changed in code**; what was wrong was the sentence, and it is replaced by what the normalizer actually is: the map from the spelling verification *refuses* to the one it *accepts*, which is precisely why it is the right key for deploy dedup (one signature, one key, and the key's spelling is the one verification would accept) — and why changing either side would be consensus-visible rather than a cleanup. Both forms on both algorithms are now pinned by `secp256k1::tests:the_verifier_refuses_the_high_s_spelling_and_the_normalizer_maps_it_back`. **Laws 44–47, and what this pass read of them**: the formal gate already runs every witness they name (78, all passing), so existence is machine-checked; what a review adds is whether each *exercises its clause*, and the honest split is that law 44's was read in full — `the_epoch_gate_does_nothing_off_a_boundary` stages a withdrawal, commits rewards, and asserts all four components (pending withdrawers, committed rewards, bonds, active) are unchanged off a boundary, which is every clause of the statement — while 45–47's were checked by **name against clause** (`an_epoch_splits_the_pot_and_keeps_the_dust` witnesses 45's split and 46's dust; `withdraw_stages_the_validator_until_the_next_boundary` and `a_released_withdrawal_pays_the_bond_plus_the_committed_rewards` witness 47's two halves) and **not** read line by line. That residual is named here rather than implied away: a deep per-clause reading of 45–47 is the part of this item that is still owed. |
| C109 **a negative `phlo_limit` minted REV into its own author's vault** — reachable from a *peer's* block, because the check that would have caught it guarded the deploy ingress and not the block path | — (no law states the sign of a charge; the nearest is law 28's ledger discipline, which is about a *record* and not about the amount's domain) | **The charge is `phlo_limit × phlo_price` and it is a debit**, so it reaches `native.pre_charge`, which subtracts it from the deployer's vault. A negative limit made that subtraction an addition — `balance - (-n) == balance + n` — and every guard on the way was directionally blind to it: `pre_charge` tested `amount == 0`, and its insufficient-funds test (`balance < amount`) is false for any non-negative balance against a negative amount. `credit_pos_vault` then no-opped on the negative (`amount <= 0` at its own guard), so the staking vault was untouched, the deployer's vault was **up by `n`**, and both halves of one transfer disagreed while nothing failed. Its sibling `refund` guarded `amount <= 0` all along — the asymmetry is the tell, and it is why the fix is a type rather than a third guard. **Measured before it was fixed:** `pre_charge(&pk, -100)` returned `Ok` and left the vault at `100` where it began at `0` (`left: 100, right: 0`), which is the unit-level probe this row's falsifiers now pin. **The path that mattered:** `BlockApiImpl::deploy` *did* reject a negative limit, so a local deploy could not carry one — but that is the deploy pool, and `total_phlo_charge` is what **block validation and replay** call, so a peer's block carried the sign straight through to the charge. `refund_amount`'s doc claimed the refund was "non-negative by construction"; it was not, because `phlo_price` is a bare `i64` off the wire and a negative price made a negative product. **Fix, structurally rather than by guard:** `total_phlo_charge` and `refund_amount` return `NonNegI64`, so the sign is refused one step after the wire and the amount is unrepresentable on the charge path — `pre_charge`, `refund`, `credit_pos_vault`, `debit_pos_vault`, `SystemDeploy::{pre_charge,refund}` and `NativeSystemDeployOp::{PreCharge,Refund}` all carry it. `NonNegI64::saturating` was added as a *total* constructor because the clamp's alternative was a `.expect(..)` in production code, which the partiality gate refuses. A `phlo_limit` check also joins the pure checks in `validate::block_summary`, so a validator rejects the block **by name** (`BlockStatus::InvalidPhloLimit`) rather than failing somewhere downstream. **Falsifiers:** `casper::validate::tests:a_negative_phlo_limit_cannot_reach_the_charge` — the charge is `None`, the block is refused, the price check is *not* what refused it, and a zero limit is valid (the control); `models::casper_message::tests:refund_amount_is_non_negative_for_every_input`; and the extended `total_phlo_charge_does_not_wrap_on_overflow`, which now asserts a negative limit and a negative price both yield `None`. **What is still owed and is named rather than implied:** a conservation invariant over *every* address the node holds. The existing `total_rev` is a test helper that sums a caller-supplied list, so a credit to an address nobody listed is invisible to it — which is the shape that let this hide. |
| C110 **a block's `Slash(victim)` was re-executed by every validator with no check that the victim had done anything** | — (the nearest is law 15's justification structure, which says what a block *justifies* and not what it may punish) | **The rule lived only on the producing side.** `proposer.rs::slashable_offenders` computed exactly the right set — bonded senders of justifications whose failure is attributable to the block — and the receiving validators took the decision on trust: `runtime_replay` rebuilt the `Slash` from the wire and ran it, and validation compared only the post-state hash and the bonds cache. A proposer could therefore name **any bonded validator**, compute the resulting state honestly, and every peer would deterministically confiscate that stake. The Scala gated this with a system auth token (`Pos.rhox`'s `slash` takes `sysAuthToken` and checks it); the port moved the protection entirely to "only the block proposer can place a system deploy", with nothing checking the placement. **Fix, as one definition and two callers:** the rule is now `validate::slashable_senders`, which `proposer` narrows by its own `bonded` filter (a policy about who is worth slashing, not part of justification) and which `validate_block_checkpoint` applies to the block's `SystemDeployData::Slash` set. The check runs against the receiving node's **own DAG metadata** — `BlockMessage.justifications` is only a list of hashes, and `slashable` is not a field of the block, which is the property wanted: the proposer's opinion of the victim carries no weight, and a node that never saw the offending block cannot be made to rubber-stamp the punishment. **This row used to say the flag was "in-memory only", and that was false in the direction that mattered** (AUDIT C122): `BlockMetadata::from_proto` hard-coded `false` and every stored metadata is read back through that codec, so `slashable_senders` had no reachable input — it returned the empty set for every block, which silenced the *producing* side (an always-empty `to_slash`) and **inverted the receiving side into a blanket refusal**: `slash_is_unjustified` is `Ok(!slashed.is_subset(&justified))` (`interpreter_util.rs:163`), so a non-empty slash set against an always-empty justified set is `Err(BlockStatus::UnjustifiedSlash)` (`:214-215`) for *every* `Slash`, justified or not. C122 carries the fix, the falsifier and the hard-fork registration; the flag now survives the store round trip. An unknown justification contributes no offender, so it licenses nothing. **Falsifier:** `casper::validate::tests:a_slash_is_justified_only_by_a_slashable_justification_from_its_victim` — a slash of the validator its evidence condemns is covered; a slash of a validator nothing holds responsible is refused; and a `validation_failed`-but-not-`slashable` justification does **not** authorize one, because a local replay failure says nothing about the sender (#70). |
| C111 **any single trusted stakeholder could confiscate any other validator's bond by revoking its trust** | — (no law states what revocation means; the nearest is law 47's withdrawal discipline, which is about a validator's own exit) | **One signature, one deploy, no evidence, no delay.** `untrust` removed the target from the trusted set and then, if the target was bonded, called `self.slash(target)` — moving the entire stake to the Coop vault. The only gate was "is the caller trusted", which the caller already was by definition. `Pos.rhox`'s `slash` required the system token; here the token has no producer, so nothing stood between a stakeholder's opinion and a peer's property. **The line also hid a second defect:** `let _ = self.slash(target).await?` discarded the inner `Result`, so a slash that failed — a staking vault that could not cover the transfer — was silently swallowed while the caller was told the revocation had succeeded. **Fix:** trust is an admission list and a bond is property; severing the first no longer transfers the second. An untrusted-but-bonded validator keeps its stake and can withdraw it, and confiscation happens only through C110's justified path. Quorum and delay were the alternatives considered and rejected on shape — both still end with one actor deciding another's property, just more slowly. **The test was the other half of the finding:** `untrust_removes_and_confiscates` asserted the behaviour it was written from, so it passed for as long as the bug lived and could not see it. It is now `untrust_removes_trust_and_leaves_the_bond_alone`, asserting the rule (trust gone, bond present, Coop untoubled, staking vault unchanged, the unbonded remainder still withdrawable). |
| C112 **the admin HTTP server published an unauthenticated `POST /api/propose` on `0.0.0.0`** | — (no law states an ingress's authentication; this is R12's ingress class, and the boundary is one layer out from C97's) | **The endpoint triggers block production and had no authentication at all.** `admin_router` mounts `/api/propose` and `/api/v1/propose` with a CORS layer and nothing else, and `admin_propose` delegates straight to `create_block`. The server bound `api-server.host` — default `0.0.0.0` — so on a node with a published port, **any host on the network could make it propose**. CORS is not authentication: it is a browser same-origin mechanism, and a non-browser client ignores it entirely, so even the restrictive default protected nothing. The *internal gRPC* propose service had the same absence but was loopback-bound, which is the asymmetry that made the omission visible. The comment at the bind said the public address was deliberate, for browser-wallet reachability, and had been since the Scala. **Fix:** the capability is preserved and the default is not — loopback unless the operator sets `api-server.enable-devnet-admin-public`, mirroring `enable-devnet-cors`'s ask-for-it shape so there is one idiom rather than two, with the key in `defaults.conf` and the flag in the CLI. **Falsifier:** `runtime::node_runtime::admin_bind_tests:the_admin_server_binds_loopback_unless_the_operator_asks_otherwise` asserts **both** arms — the default is loopback, and the opt-in actually publishes, so the flag cannot become a no-op that silently breaks the wallet path it exists for — plus a loopback host is unchanged either way and a specific interface is not widened to the wildcard. The choice was extracted from the spawned task into `admin_bind_host` precisely because, inline, the only way to observe it was to start a server and connect. |
| C113 **two ingress bounds that were not bounds** — a per-stream cap multiplied by a stream count, and a configured message limit applied on one side only | — (the nearest is §1.6's totality read as a resource claim, which is the misreading C99 records; no law bounds a peer's memory) | **Part one: the stream budget.** Each `stream` handler drains its chunks into a local `Vec<Chunk>` under a `stream_slots` permit, and the only byte cap was `max_stream_message_size` — 256 MiB — **per stream**, with `MAX_CONCURRENT_STREAMS` = 1024 of them. `blob_slots` (16) bounds *decompressed* blobs and is checked only after reassembly, so it never sees the compressed accumulation. The product, ~256 GiB of resident memory, is what one peer holding one self-signed certificate could reach: a bound whose terms multiply to a number nobody intended. **Fix:** the handler also charges an aggregate byte budget, held for the life of the stream and released on every exit path, sized `blobs × max_stream_message_size` — deliberately the figure the node already commits to for decompressed blobs, so the compressed half of one pipeline cannot be a loophole around the other half. One max-size stream still fits several times over, so legitimate traffic is unaffected. **Part two: the unary cap.** `grpc_max_recv_message_size` (256 KiB) was read on the *client* side only; the server never called `max_decoding_message_size`, so an inbound `send` was accepted up to tonic's 4 MiB default — 16× the limit the operator's configuration names. It is now set from that key. **Falsifiers:** `grpc_transport_receiver::tests:the_aggregate_stream_budget_bounds_what_the_stream_cap_multiplies` pins the product, that the budget is strictly smaller than the stream cap alone (the whole defect was that this product *was* the bound), the permit count, that a single max-size stream fits, and the degenerate configurations. The three new `div` sites (metering into `u32` permits) are raised in the ratchet baseline in the same commit with the review paragraph that file's doctrine requires. **What is not done:** the end-to-end memory measurement. The ~256 GiB figure is arithmetic from the constants, and the fix's done-condition — hold N streams open and watch RSS — is the falsifier this row does **not** yet have. |
| C118 **law 1a's first named witness cannot fail if the sorting mechanism is deleted**, because the property it asserts is satisfied by the mechanism's absence | — **the instrument** (the nearest law is 1a itself, and what is wrong is not the law but the evidence the register files for it) | **The witness is `law1_sorting_is_idempotent`, and idempotence is exactly what an identity function has.** Mutating `sort_par_term` to `par.clone()` — the canonical form becomes the input — leaves it **green** (observed). Its second assertion does not save it: `Sorted::new(p).as_par() == sort_par_term(&p)` compares two things that both became identity, so they agree. **The row is still covered**, and that is the difference between a weak witness and a gap: `law1_parallel_composition_sorts_commutatively` — the sibling named in the same row — **fails** under the same mutation (observed), because `sort(p|q)` and `sort(q|p)` stop agreeing when the order is the input's. So this is a defect in the *evidence* the register files, not in the law's coverage: a reader checking "does law 1a have a falsifier" would find one, but a reader checking *this* witness would be reading a test that cannot fail. **Found by the method the audit prescribed for all 60 rows** — delete the mechanism the row names, require green→red, restore — and it is the class round 2 reported: of 26 rows it sampled, 8 witnesses stayed green. **What is not yet done:** the other 58 rows. This row and law 8's (C119) are the first two of that sweep, not the sweep. **Fixed as a pointer, which is what it was:** law 1a's `rustWitness` now names the falsifier first — `law1_parallel_composition_sorts_commutatively`, which goes red when the sort is made the identity — and the idempotence witness (true, and unfalsifiable by that mutation) is recorded here rather than filed as this law's evidence. |",
| C140 **law 15's two named witnesses both stay green when a block is never added to the DAG state at all** — the mechanism is covered, by another law's witnesses | — **the instrument** (C118/C138/C119 again, and this one is the cleanest demonstration of the class) | **The witnesses are `law15_adding_blocks_only_grows_the_state` and `law15_insert_msg_is_monotone`, and both assert a one-directional property.** The first asserts `next.dag_set.is_superset(seen)` — "the seen set must not shrink" — and a state that never changes is a superset of itself, so it holds. Making `add_block_to_dag_state` return its input unchanged leaves **both green** (observed). **Six other tests go red under the same mutation, and not one belongs to law 15:** `dag::metadata_store::tests::add_block_to_dag_state_builds_child_map`, plus five of law 18's — `law18_height_map_with_holes_errors`, `law18_a_chain_with_a_hole_in_the_middle_is_refused`, `law18_the_empty_dag_and_a_lone_block_are_contiguous`, `law18_a_validation_failed_tip_is_not_indexed`, `law18_a_contiguous_chain_validates`. **So the DAG's growth is genuinely tested and law 15 is not what tests it** — law 18's height-map assertions are, because a block that was never added leaves a hole in the height map. That is a strange place for the coverage to live: law 15 is the row whose subject *is* monotone growth, and law 18's row is about contiguity, which catches the same defect incidentally. **C119 named this shape and got the direction wrong** ("law 8's witness does not exercise the selection"); here there is no argument about it — two witnesses, both vacuous, and the catching tests are filed under a different law entirely. **The generalisable rule this pass now has four instances of:** a witness whose assertion is one-directional (`equal ⇒ same hash` at C139, `unchanged is a superset` here, `closed stays closed` at C138, `idempotent` at C118) is satisfied by a degenerate implementation, and the register's `rustWitness` column does not distinguish "names the law's subject" from "fails when the mechanism is deleted". **Fixed by strengthening the witness rather than re-pointing it**, which is the better of the two here: law 15's subject *is* growth, so its own witness should be the one that notices a block that never arrived. `law15_adding_blocks_only_grows_the_state` now also asserts that the block just added is in the new `dag_set` and that its height is in the index — two claims a no-op violates, where `is_superset` and the `>=` counts do not. **Falsified:** with `add_block_to_dag_state` returning its input, the witness fails at the first new assertion (and `law15_insert_msg_is_monotone`, which is about the message state rather than the DAG state, stays green — the two halves of this law are separately witnessed and only one of them was vacuous). **What is not changed:** law 18's five tests and `add_block_to_dag_state_builds_child_map` still catch the mutation too, and that redundancy is left in place — the row records that the coverage *lived* there, and moving it is not the same as deleting it. |",
| C139 **`Hash for Sorted` can be made to ignore its input entirely with the whole models suite green** — law 2's named witness asserts only that *equal* values hash *alike*, which a constant function satisfies | — **the instrument** (the law is fine and so is the code; what the register claims its evidence establishes is not what the evidence says) | **The witness is `law2_canonical_equality_agrees_with_canonical_hashing`, and its hash assertion is one-directional**: `prop_assert_eq!(hash_of(&a), hash_of(&b), "equal canonical forms hash alike")` — at line `models/src/property_tests.rs:126`, with `hash_of` a `DefaultHasher` over `Hash::hash`, so the mutation does reach it. Replacing `Sorted`'s hash body with a constant leaves **both** law-2 witnesses green, and running the whole `rchain-models` lib suite under the mutation leaves **all 168 tests green** (observed). The property that would catch it is the converse — *unequal* canonical forms must hash *differently* — and no test in the crate asserts it. **This is C118's shape and it is the strongest instance of it so far:** C138 had four unnamed tests catching its mutation, so law 3's coverage existed and only the pointer was wrong. Here nothing catches it at all, which is a coverage gap as well as an evidence defect — the first of this sweep. **Severity is bounded and the bound is worth stating**: `impl Hash for Sorted` is reached in this tree only by a test helper (`models/src/par_ops.rs:436`); the consensus state hash is computed over the blake2b *encoding*, not through `std::hash::Hash`, so a degenerate `Hash` would degrade in-memory maps and make this witness meaningless without forking a chain. Recording the scope because the row's own note claims the property exists because "a `Sorted` whose `Hash` disagreed with its `Eq` would make the hash depend on insertion order" — and on this tree's paths that sentence is not load-bearing. **Fixed, and it needed a test rather than a pointer — this is the first of the class with a real coverage hole.** `models/src/property_tests.rs:law2_unequal_canonical_forms_do_not_hash_alike` states the converse (canonical forms that are not equal must not hash alike, with `prop_assume!` carrying the law's hypothesis), and it is now the first name in law 2's `rustWitness`. **Falsified with the mutation the row describes:** replacing `impl<S: Sort> Hash for Sorted<S>` with an empty body leaves both of law 2's original witnesses green and reddens the new one — 2 passed, 1 failed, which is the finding in one line. The row's severity bound stands and is unaffected: `impl Hash for Sorted` is reached by a test helper only, and the consensus state hash is over the blake2b encoding, so this closed a hole in the register's *evidence* rather than a chain risk — the two are different claims and this row made both. |",
| C138 **both of law 3's named witnesses survive a substitution that does nothing** — the row's entire evidence is vacuous under the mutation, while four tests it does not name catch it | — **the instrument** (as C118/C119: the law is fine; what the register files as its evidence is not) | **The witnesses are `law3_substituting_a_closed_value_keeps_the_term_closed` and `law3_substitution_and_sorting_commute`, and neither can fail when `substitute_par` returns its input.** The first asserts `is_closed(p)` then `is_closed(out)` — identity preserves closedness, so it holds. The second asserts substitution and sorting commute — with identity on one side, both sides reduce to `sort(p)`, so it holds. Mutating `substitute_par` to `par.clone()` leaves **both green** (observed). **The mechanism is not untested, and that is what makes this a pointer defect rather than a hole:** four tests go red under the same mutation — `property_tests::an_open_value_at_the_variable_leaves_a_free_variable`, `property_tests::substituting_a_term_level_wildcard_is_an_error`, `substitute::tests::free_var_at_depth_zero_is_illegal`, `substitute::tests::substitutes_bound_var` — and **not one of them is named by any law row**. So law 3's coverage exists and its evidence column points somewhere else entirely. **Why this is the sharpest of this pass's three:** C118 was one witness of three and the row survived on a sibling; C119 was a witness covering the wrong half of a mechanism. Here *both* named witnesses are vacuous, so a reader asking "is law 3 falsified?" would be looking at two tests that cannot answer. **The pattern across C118, C119 and this row, which is the thing worth carrying forward:** in every case the *mechanism* was covered and the *register's pointer to it* was not. That suggests the sweep's remaining 52 rows will yield more pointer defects than coverage gaps, and that the fix for the class is to name the test that fails under mutation rather than the test that mentions the law's subject. **Fixed by re-pointing the column, and re-measured first-hand rather than relayed.** Law 3's `rustWitness` now names `rholang/src/substitute.rs:substitutes_bound_var` and `rholang/src/property_tests.rs:an_open_value_at_the_variable_leaves_a_free_variable`. The mutation was run here before the change was made: with `substitute_par` returning its input, both old witnesses pass and **all four** catchers fail (`an_open_value_at_the_variable_leaves_a_free_variable`, `substituting_a_term_level_wildcard_is_an_error`, `free_var_at_depth_zero_is_illegal`, `substitutes_bound_var`), which is this row's claim reproduced from scratch. **What made the fix cheap, and is worth recording:** the row's own `falsifiable` cell *already* named `an_open_value_at_the_variable_leaves_a_free_variable` as the code-side pin — the correct pointer was in the register, one column over. The vacuous pair keeps its place in the tree as assertions about closedness and about `sort ∘ subst`; what it loses is the claim to be this law's falsifier. |",
| C119 **law 8's registered witness covers `Comm::apply`'s produce order but not the content-addressed selection the row is about** — and round 2 named this case, overstating it in both directions | — **the instrument** (as C118: the law is fine; the register's pointer is what was wrong) | **What the witness covers, established by mutation rather than by reading:** deleting `Comm::apply`'s `produce_refs.sort_by_key(..)` makes `law8_comm_sorts_produces` **fail** (observed), so the produce-order half of law 8 is genuinely falsified by it. **What it does not cover:** the witness builds `ConsumeCandidate`s by hand and calls `Comm::apply` directly, so it never touches the space matcher. Replacing `find_matching_data_candidate`'s body with `return Ok(None)` — the selector never matches anything — leaves the witness **green** (observed). **But round 2's framing of this was wrong in one direction and right in the other, and the difference matters:** the *mechanism* is not untested — `space_matcher::tests::find_matching_data_candidate_finds_first_match` **fails** under the same mutation (observed) — so the defect is that the register points law 8 at a witness that does not exercise the selection, while the test that does sits in another module and is named by no law row. Calling that "unexercised" would be wrong; calling it "the row's witness is the wrong test" is what the measurements support. **Fix is a pointer, not code:** the row's evidence should name both, which is what this row does. **And the same sweep found a witness that is *sound*, recorded here so it is not re-reported as a finding:** `law7_join_hash_commutes` stayed green under a mutation that removed the sort in `hash_hashes`, and that is *correct behaviour* — `hash_seq` sorts the same hashes upstream (`stable_hash_provider.rs`), so the removal deleted a redundant duplicate rather than the mechanism. A mutation that is masked by a second implementation of the same rule is not a deletion, and reporting it would have been a false finding. **Fixed:** law 8's `rustWitness` now names both — `law8_comm_sorts_produces` for the produce order and `rspace/src/space_matcher.rs:find_matching_data_candidate_finds_first_match` for the selection the row is about. |",
| C117 **the SSRF guard classified IP literals and nothing else, so a peer-supplied hostname reached the routing table and the node dialled it** | — (no law states what a peer address may name; this is R12's ingress class one field over from C115/C116) | **The guard's job is to stop a peer aiming the node at its own network, and a name is a way of aiming.** `is_local_address` matched `IpAddr` and returned `false` for anything else — so `localhost`, or any DNS name whose A record points at `127.0.0.1` or `10.0.0.5`, passed the Kademlia ingress, was written into the routing table, and was **dialled**. The residual was recorded honestly in the function's own comment ("a hostname that resolves to a private/loopback address is not classified here") and recording it did not make it safe: the entries in the table are what the node later connects to, so a name is exactly as good as a literal for the attack the guard exists to prevent. **Fix:** `is_local_address_resolved` applies the same rule after `lookup_host`, and the Kademlia ingress (`send_ping`, `send_lookup`) uses it. **Fail-closed at both ends:** *any* resolved address being local/private refuses the host — a name with one public and one private record reaches a private address, so it is not mostly safe — and a name that does not resolve at all is refused too, because "could not find out" must not read as "probably fine". The synchronous `is_local_address` is unchanged and still used where the caller has a literal (`check_peer_on_same_network` compares the local bind host, which is configuration rather than a peer's claim). **Falsifier:** the existing `a_ping_claiming_a_local_host_never_reaches_the_handler` gained `localhost`, `localhost.localdomain` and `this-host-does-not-exist.invalid`. Verified by reverting the handler to the synchronous guard: the test then fails with `localhost must be rejected before the handler`. **What is not closed, stated rather than implied:** the check and the dial are separate resolutions, so an attacker controlling a name with a short TTL can answer public to the check and private to the connection. Closing that means pinning the resolved address into what a `PeerNode` carries and dialling *that* — a change to the routing table's shape rather than to this function. The window narrows from "any name works, always" to "the name must lie at exactly the right moment". |",
| C116 **the Kademlia discovery service was plaintext and unauthenticated on `0.0.0.0:40404`, and its messages named a sender nothing checked** | — (no law states what discovery admits; the nearest is R12's ingress class, and C115 is the same defect one service over) | **Two residuals in one surface, and the second is the one that mattered.** The code recorded the first: `serve` was plaintext, with a rate limit and the `is_local_address` SSRF guard as the only things in front of it. Those bound *how much* a peer can do and *where* it may point the routing table; neither established **who** it is — and the routing table's key is the `sender` field of the message, which the peer writes. So a host could enter a node's table under an id it did not hold, and be dialled, gossiped onward and compared as that node. **It is C115's defect, unrecorded, on the discovery path** — which is why the fix is C115's fix rather than a second design. **Fix, in three parts.** (1) The service now serves over the **same mutual TLS** the transport uses (`hostname_trust_manager::server_config` — client certificate mandatory), through `accept_tls`, extracted from `grpc_transport_receiver::serve_with_limits` so the two services cannot drift in who they let in. `Server::tls_config` was not usable: the node's trust model is \"any self-signed P-256 certificate, with the address checked separately\", which tonic's `ServerTlsConfig` (identity + CA root) cannot express. (2) The client dials `https://` with the same connector the transport client uses, including the `ServerName` derived from the peer id that the server-side verifier compares the certificate against — it was `http://` with a bare `connect()`. (3) Both handlers check the proven identity against the claimed sender (`sender_not_proven`), refusing with `PermissionDenied`, including when nothing was proven. **One accept path, two services:** `accept_tls` carries the R14 in-flight-handshake bound the transport's inline loop already had, so extracting it did not weaken the path it came from — `cargo test -p rchain-comm` was green across the extraction before the Kademlia change landed on top. **Falsifiers:** `grpc_kademlia_rpc_server::tests:a_sender_must_be_the_identity_its_certificate_proves` — honest sender reaches the handler, spoofed sender is refused **and does not enter the routing table**, unproven sender is refused, and the lookup path carries the same check. The client's own tests build peer ids from real certificates (`peer_at_from_cert`) because a made-up id is now refused before a test could observe what it is about. **The end-to-end test arrived after the fix**, and it closes the gap this row first recorded as missing: `a_ping_round_trips_over_mutual_tls` starts the real `serve` on an ephemeral port, completes a genuine mutual-TLS handshake with a client whose certificate proves the id its `Ping` announces, and requires both that the round trip succeeds and that the ping reaches the handler. Its falsifier is the identity read itself: with `peer_id_of_tls` forced to `None`, an honest peer is refused and the test fails with `an honest peer must complete the mutual-TLS handshake and be answered` (observed). One arrangement detail is load-bearing and is stated in the test: the client **announces a public host while dialling loopback**, because the SSRF guard (C117) refuses a loopback announcement — so the test exercises the identity check rather than the address one. **And this is a wire change**: two nodes must both be on this revision to discover each other. |",
| C115 **the protocol `sender` was not bound to the TLS peer certificate** — so any host could assert any node id, and every downstream decision that keys on a sender believed it | — (no law states what an inbound connection *is*; this is R12's ingress class at the transport layer, and the boundary is the one C97 named one layer in) | **Mutual TLS proved possession of *some* P-256 key, not of *this* node's.** `NodeIdClientVerifier` accepts any self-signed certificate whose public key is a well-formed point, and the identity used for routing was read from the protocol header — a field the peer writes. So a peer could connect and claim to be any node: occupy its entry in a peer's connection table, be compared as it by equivocation detection, and be tracked as it by the DAG's per-sender rules. The code called this a residual in `handle_messages.rs` and relied on `MAX_CONNECTIONS` to bound "the resulting unbounded growth of the connection table" — which bounds a *table*, not a lie. **The asymmetry is what makes it a defect rather than a policy:** the client side has always made the mirror check (`NodeIdServerVerifier` requires the *server's* certificate address to equal the peer id it dialled), so the node verified its peers' identity outbound and accepted anything inbound. **Fix:** the identity the client certificate proves is read once at the accept, while the session is still in hand (`peer_id_of_tls` — keccak of the certificate's P-256 point, the same derivation the client side and `cert_node_id` in the tests use), carried into the request through tonic's `ConnectInfo` as `PeerId`, and compared against the header's sender. Applied to **both** paths: the unary `send` and the streamed `stream`, whose reassembled header carries its own sender that the routing layer trusts identically — a fix on one path would have left the other as a door beside it. A message whose proven identity differs is refused `PermissionDenied`, and so is one whose connection proved **nothing** (`PeerId(None)`), because client auth is mandatory and a missing proof must not read as a valid one. **Falsifiers:** `grpc_transport_receiver::tests:a_sender_must_be_the_identity_its_certificate_proves` — three arms, and the first is what makes the others mean anything: the honest request is dispatched, the spoofed one is refused *and does not reach the dispatcher*, and the unproven one is refused. Two existing tests had to be corrected rather than accommodated: `grpc_transport::tests:send_round_trips_over_socket` and `dispatch_bound_tests:…rejected_and_recovers` both sent a heartbeat whose header named the **server** while presenting the **client's** certificate — the exact assertion this check refuses — and they passed because nothing compared the two. That they now pass with the client's real identity is what shows the binding holds over a genuine TLS handshake rather than only in the unit seam. **What is not done:** the streamed path's refusal is exercised through the same `breaker` the unit tests drive, not end to end over a socket; the unary path is the one with the socket-level proof. |",
| C114 **`rho:rchain:multiSigRevVault` provided single-key custody under a multi-signature name** | — (no law states a system process's contract; this is a registry-alias fidelity gap) | **The channel had its own fixed byte channel (21) and its own `BodyRefs`, and was wired to `self.rev_vault()` — the single-signer handler.** A deploy that put funds behind the multi-signature name therefore got single-key custody: no quorum, no co-signers, no confirmation step, and **nothing said so**. The contract exists (`casper/src/genesis/resources/MultiSigRevVault.rho`, with `create`/`confirm`/quorum and a sealer-unsealer) and is deliberately not installed — `genesis/mod.rs` records that the vendored sources are "a checklist and are not installed". So the two honest options were to install it, which is a consensus change on a minted-asset path, or to stop answering to its name. **Fix:** a dedicated `multi_sig_rev_vault` handler refuses every method with a message naming exactly what is absent and pointing at `rho:rchain:revVault`, which is single-key custody and is named for it. The fix costs nothing because nothing in the port reaches this channel — only the uninstalled `.rho` sources and `legacy/` name it. **Why it is registered anyway:** a name that silently provides a weaker guarantee than it promises is the failure mode that makes a custody bug invisible until funds are gone, and this is the second such gap the audit found in the same table (see C22's account of `sys:authToken:ops`, whose `check` has no producer and always answers `false`). **What is owed:** either installing the quorum contract or deleting the alias and its registry entry — a decision for the remediation phase, not this pass. **Done 2026-09-27, by the first of the two: the contract is installed.** The remediation phase is closed and there is no genesis block yet, so the consensus change this row deferred costs nothing now. `AuthKey.rho` and `MultiSigRevVault.rho` are adapted the way `MakeMint.rho` is — both ask `rho:registry:systemContractManager` and `rho:rchain:configPublicKeyCheck` for channels this port does not have — and added to the blessed list in dependency order. The channel this row's fix made *refuse* is now answered by the contract itself, and the register's own completeness check (`missing_genesis_aliases`) is what proved the adaptation had run rather than silently not registering. The sequencing was the row's own prescription, in order: stop lying first, install when a genesis change is affordable. **The delegated spend works, and the claim that it did not was wrong — corrected here rather than quietly dropped.** `a_multisig_vault_spends_through_the_contract_that_holds_it` creates a vault through the installed contract (quorum 1, the deployer's own key), funds it, asks for a `deployerAuthKey`, and requires the **destination balance** to move — which it does, by 30,000,000 REV. What that exercises is the thing `spec/RUST-FIRST.md`'s B2 said this port did not have: the vault is spent by **the contract**, through the vault handle's `transfer` and an `unforgeableAuthKey`, not by a deployer key. An earlier version of this paragraph claimed the contract's spend was "one native→interpreted round trip away" because `MultiSigRevVault` authorises through the installed `AuthKey` contract and this port's native auth values cannot satisfy it. **That was a diagnosis of the wrong path**: the shape it wants, `(*_multiSigRevVault, pubKey)`, contains the contract's *own private name*, so `deployerAuthKey` builds it **in rholang** over the installed `AuthKey` contract and `rho:rchain:deployerId:ops` — no native code involved, and nothing missing. The attempt that produced the wrong diagnosis was driving the **sealer/unsealer** path (a contract as the confirmer, sealed authorisations compared inside `partialFold`), which is installed and **undriven by any test** — a coverage gap in this contract's second authorisation shape, not a hole in the first. |
| C167 law 50a's own falsifier could not fire: the mutation witness was `List.replicate`'s `n` **siblings** rather than `n` nested levels, so the mutation test would have passed **vacuously** — under the cell that cites law 22 | **50a** — the register's own defect, and the incident is a *document* | **Found by review 2026-09-28, by reading the witness and computing its arithmetic, and not by any test — which is the finding.** Law 50a's `falsifiable` cell and `spec/Rchain/Depth.lean` both said that a mutation dropping one arm of the walk's children function "must make soundness false at `notsDepth 768`". The witness was `Par.mk [] [] [] (List.replicate n (.enot unit)) [] [] [] []`, and `List.replicate` fills **one field with `n` siblings** — the fourth, `Par`'s `List Expr` (`spec/Rchain/Par.lean:20-22`) — not `n` nested levels. Each `enot unit` has `exprDepth` 2, so `parDepth (notsDepth n) = 3` for every `n ≥ 1`: the term never got deeper, and at the budget `maxAstDepth` (768) the real walk *and every mutant of it* accept it. The mutation test the register named would therefore have passed **vacuously** — the failure law 22's `vacuous` status exists to prevent (`Rchain/Laws.lean`), in the very cell that cites law 22 — and the doc comment beside the definition additionally claimed the depth was `n + 1`, which no arithmetic supports. **The lesson is the row, not the commit, because the falsifier was *named* in the register and *described* as verified, and nothing had evaluated it.** A witness shape whose arithmetic is asserted in a comment rather than proved is not a falsifier; it is a sentence that reads like one. **Fix (2026-09-28, in the unit that discharged clause a):** `notsDepth` now **nests** — a level costs 2, one for the `enot` node and one for the `Par` that holds it — so `parDepth (notsDepth n) = 2 * n + 1`, the falsifying instance is `notsDepth 384` (depth 769, the first term past 768), and the arithmetic is a theorem rather than a claim: `Rchain.parDepth_notsDepth` (`spec/Rchain/Depth.lean:728`), with `spec/Rchain/Depth.lean:220-222`'s comment recording why the first draft was wrong. **Falsifiers:** `Rchain.a_dropped_arm_breaks_soundness` (`spec/Rchain/Depth.lean:760`) — the walk with its `exprs` arm deleted (`Rchain.walkParDroppingExpr`, `:743`) accepts `notsDepth 384` at budget 768 while that term's depth is 769, which is the falsifier the row names, machine-checked; `Rchain.mutant_accepts_nots384` (`:752`) and `Rchain.the_walk_refuses_the_mutant_witness` (`:768`) are its two halves, so the pair is a test rather than a slogan; and `Rchain.parDepth_notsDepth` **is** the falsifier's arithmetic — the thing that was wrong here was the arithmetic, so the arithmetic is what had to become a theorem. |
| C168 law 50a's Rust half was pinned by three tests that do not test it, while the every-constructor test that does exists and belongs to the other clause | **50a** — the register's own defect, and the same class as C99's over-claimed constant | **Found by review 2026-09-28, by reading the claim against the tests it names.** Law 50a's note said the half no theorem can state — that the Rust walk descends into every `Proc` constructor — "is pinned by the three `rustWitness` tests": `rejects_a_deep_ast_that_stays_inside_both_component_guards` (`rholang/src/parser.rs:2013`), `accepts_a_deep_ast_under_the_budget` (`:2024`) and `a_maximal_chain_still_parses` (`:2036`). All three are boundary and refusal tests for the **guard**: a shape inside both component guards is refused, the same shape under the budget is accepted, and a maximal flat chain still parses. **Not one of them checks per-constructor descent**, which is the property the sentence credits them with — a walk silent about a `Proc` variant none of the three builds would leave all three green. **The every-constructor test the claim was describing does exist, one clause over:** `models/src/types.rs:1184::every_construct_that_carries_a_par_is_walked` parks a depth-10 child in each of the 22 positions across `Par`/`Expr`/`Connective` — but it exercises the **value** route (`exceeds_value_depth`, clause b, AUDIT C100), and nothing in it reaches the parser's `push_sub_procs` (`rholang/src/parser.rs:424`). So clause a's Rust half was pinned by tests that did not test it: a citation that reads as checked and is not, which is this register's own defect class and the shape C99's constant and C152–C154's pointers also take. **Fix (2026-09-28):** `rholang/src/parser.rs:2680::every_construct_in_the_parser_walk_is_descended` is the parser route's own analogue — a deep child parked in each position `push_sub_procs` must reach, so a dropped `out.push` is the only way a case can pass — and law 50a's `declarations` and `rustWitness` now name it. **Falsifiers:** `every_construct_in_the_parser_walk_is_descended`, falsified by deleting one `out.push` and watching exactly its case fail; the three tests the row used to name are kept, and are correct for what they *do* pin (the guard's two boundaries and `MAX_CHAIN_LENGTH`), which is why the defect was in the citation and not in the tests. |
| C169 law 50b's statement was not true as written: the value walk's counted quantity is `Par`-nesting rather than `parDepth`, and the gap between them is a **tight factor of 3**, not a slack of 128 | **50b** | **Found by review 2026-09-28, by re-deriving the statement against `models/src/types.rs` — the row had been `owed`, and an `owed` row whose statement nobody re-derives can be owed for the wrong theorem.** Law 50b's note said the Rust value walk gives an `Expr` node no level of its own where `parDepth` counts it, so soundness carried a slack bounded by `MAX_PARSE_DEPTH` (128) "because a value's expression nesting is *syntax* and no runtime path builds `Expr` nodes". **Three things were wrong, all verified in the code.** (1) The walk charges a level for **`Par` nodes only**: `push_value_fields` (`models/src/types.rs:695`) pushes every field element at the *same* depth `d` it was handed, and `push_value_expr` (`:739`) and `push_value_connective` (`:804`) push their `Par` children at that `d` too — so `Send`, `Receive`, `ReceiveBind`, `New`, `Match`, `MatchCase`, `Bundle` and `Connective` are transparent *along with* `Expr`, and the counted quantity is the number of **`Par` nodes on the deepest `Par`-chain**, which is not `parDepth`. (2) The premise is false: `(a, b)` **is** `Expr::ETuple`, built by the reducer (`rholang/src/reduce.rs:947`, `:953`, `:1107`), and `rholang/tests/deep_value_bound.rs` exists precisely because a program builds nested tuples at runtime — that is the shape the value route's own falsifier now uses. (3) The gap is a **factor**, not a constant, and it is exactly 3 and *tight*: two element nodes can sit between consecutive `Par`s (`Receive`→`ReceiveBind`, `Match`→`MatchCase`), so a `Match` ladder spends three levels per `Par` and `parDepth ≤ 2 * parNestDepth` fails from two rungs. At `maxValueDepth` the guard admits `pairsDepth 255`, whose `parDepth` is **511** — so the bound carried 511 where the row said 128. **Fix (2026-09-28):** clause b has its own quantity and its own walk — `Rchain.parNestDepth` (`spec/Rchain/ValueDepth.lean:55`) and `Rchain.walkValuePar` (`:214`), defined so that no element construct and no list adds a level — with a 23-member `mutual` theorem block proving `Rchain.walkValuePar_iff_parNestDepth` (`:353`), the two directions `Rchain.valueWalkExceeds_sound` (`:877`) and `Rchain.valueWalkExceeds_complete` (`:882`), the bridge `Rchain.parDepth_le_three_mul_parNestDepth` (`:577`, `parDepth p ≤ 3 * parNestDepth p`, with per-declaration offsets because an element's offset is `1 +` the largest among its children), and a machine-checked falsifier: `Rchain.a_dropped_value_arm_breaks_soundness` (`:953`) shows the `exprs`-dropped mutant `Rchain.walkValueParDroppingExpr` (`:932`) accepting `pairsDepth 256` at the bound of 256, with `Rchain.the_value_walk_refuses_the_mutant_witness` (`:961`) as the contrast, and `Rchain.parDepth_pairsDepth` (`:917`) with `Rchain.parNestDepth_pairsDepth` (`:905`) make the arithmetic theorems rather than comments. **At 256 the bound is `3 × 256 = 768`** — the same number as `maxAstDepth`, which is what makes the two clauses' constants a pair rather than two unrelated measurements. **Two boundaries of the tie are named rather than hidden**, both from the Rust: the guard never pushes its root (its fields go on at depth 2), so it accepts a value with no `Par` child at `limit = 0` where the model refuses it, and the two agree for every `limit ≥ 1`; and it walks `New.injections` (`models/src/types.rs:713-717`), which the model's `New.mk` does not have — stricter, the safe direction. **Falsifiers:** `Rchain.a_dropped_value_arm_breaks_soundness` with `Rchain.the_value_walk_refuses_the_mutant_witness` (the machine-checked mutation test, on `pairsDepth 256`); and the arithmetic **is** the falsifier here — the defect was a wrong factor, so `Rchain.parDepth_le_three_mul_parNestDepth`, `Rchain.parDepth_pairsDepth` and `Rchain.parNestDepth_pairsDepth` are what make "3, and tight" a theorem instead of the sentence that was wrong. |

**The five rows that are not laws are the ones worth keeping visible.** C97 is the first: four
ingresses where peer bytes reached a fixed-width constructor's length assert, found by a red-team
audit after R12 had certified the class — no law states "a peer-supplied hash is length-checked
before use", which is why its remedy is R12's row and `TYPE-SYSTEM.md` §1.7's Construction rule
rather than a law row, and why the fix is the refinement (`BlockHash`) at the ingress instead of a
guard at the use. C98 is the second, and it is the same lesson one level out: the *instrument* that
certifies §1.6 could not see indexing, slicing or division at all, so its green run was evidence
about four classes and was read as evidence about the claim. C99 is the third: the two parser guards
that do compose *in the parser* and not *in the term*, so a 4 KB deploy cost the process — the law
shaped nearest it (§1.6's totality) is about *closedness*, not about a resource bound, which is why
its remedy is a measured refused depth rather than a row. C37 is the fourth: a *harness* finding —
a measurement that was not a measurement — and no law would have caught it, because the thing that
was wrong was outside the term. The retired static walk over vendored text (AUDIT §17 C22) is the
fifth: it was drafted, found unable to tell a terminal consume from a deferred restore from a
permanent loss, and retired unshipped rather than shipped with an exception list. A catalogue that
claimed to cover them would be claiming something false.

---

## 21. The adversarial re-audit (pass 9), and the witness sweep it led to: C120–C137, C141–C148, C149–C155 — thirty-three findings, and four refuted

**Method, stated before the findings, because this pass's worth is its filter rather than its count.**
Seven independent lenses were run over the rosters that clause 15 of `tools/audit-test-register.sh`
records as **never read** — 342 of 350 files, all 44 ingresses, all 30 system processes, all 104 config
entries — against the committed tree at `dev@5461bbf0d`. The lenses were: ingress authentication;
consensus and economics; resource bounds; cryptography and identity; **the repo's own instruments**;
concurrency and durability; and production readiness. Every finding was then handed to a second agent
whose instruction was to **refute it and to default to `REFUTED` whenever the mechanism could not be
re-established from source**, with reachability from an untrusted source recorded as a field rather
than assumed. **All seven lenses reported: thirty raw findings became twenty-six rows, with four
refuted outright and five downgraded**, and one — C121 — was found independently by two lenses that
could not see each other's work, which is the strongest evidence this pass has for anything. **The
`ops` lens returned after the first tables were written**, so its eight findings are carried in their
own subsection below rather than merged into the ones above. **The section has since grown past that
harvest**: C149–C155 are the law-witness mutation sweep's findings, carried in the same table because
they are read against the same tree, and they are not part of the thirty-raw-to-twenty-six filter this
paragraph describes.

**These rows were all OPEN when this section was written, and that is the difference from §12 and §14.**
Those record fixes; this one records findings, so a row here starts as a report. **Both criticals are now
fixed** — C120's check, falsifier and §6 row, and C121's route move, leg bound and ledger cap; each row
says what its fix does and, where it matters, what it does not. The instrument defects (C126, C127, C128,
C130, C131) are one-line pattern changes nobody has made yet, and the rows that refute a registered claim
are, if anything, worse than a new bug — see the note after the tables.

### Critical (2)

| C120 **a deploy's `deployer` is unauthenticated on the block-validation path**, so a single bonded validator can charge and spend any account's REV vault in that account's name | — (the nearest is law 28's ledger discipline, which is about a record's *shape* and not about who may author it; this is C109's ingress-versus-block asymmetry, one field over) | **`SignedDeployData::verify_signature()` has exactly one production caller and it is the ingress** — `BlockApiImpl::deploy` at `casper/src/api/block_api_impl.rs:303`. A block arriving from a peer carries its deploys as `ProcessedDeploy`, whose `from_proto` copies `deployer` and `sig` **verbatim off the wire** (`models/src/casper/protocol/casper_message.rs:497-517`), and the pure-check list every validator runs, `block_summary` (`casper/src/validate.rs:429-476`), **contains no signature check at all**. The replay then builds the charge, the refund and the `rho:rchain:deployerId` binding from that same unverified field (`runtime_replay.rs:299-350`; `models/src/normalizer_env.rs:53-62`). **Trigger:** a bonded proposer submits a block whose `state.deploys` holds one `ProcessedDeploy` naming a **victim's** 65-byte secp256k1 key with `sig = vec![1]` and a term transferring from that `deployerId`; `block_signature` passes (the proposer signed its own block), `block_summary` passes (the forged deploy is never checked and its phlo fields are legal), the pre-charge debits the victim, the body pays the attacker, and because the proposer computed the post-state hash from the same replay, `validate_block_checkpoint` and `bonds_cache` **agree — so the block is valid and nothing is attributable.** The repo states the deferral itself: `casper/tests/consensus.rs:27-28` says signature verification "is deferred to the deploy-acceptance path", and its fixture at `:44-45` is `deployer: vec![0u8;32], sig: Vec::new();` and runs through play and replay anyway. **This refutes the unconditional claim in §6's vault row in this file** ("no deploy can spend another key's vault"): true of a deploy that arrives at the API, false of one that arrives inside a peer's block. **What is owed:** a signature check in `block_summary` beside C109's `phlo_limit` check — which is exactly where this port already chose to be stricter than the Scala for the same reason — **and a §6 row**, because `legacy/` validates no deploy signature either and binds `deploy.pk` unverified in `NormalizerEnv.scala`, so this is a shared defect rather than a port divergence. **Fixed.** `validate::deploy_signatures` is now one of `block_summary`'s pure checks and returns the new `BlockStatus::InvalidDeploySignature`, so a peer's block whose deploy names a key that did not sign it is refused before the replay — and, because `block_summary`'s refusals flow through `mark_failed_attributable` (`multi_parent_casper.rs`), refused *attributably* rather than dropped as `block_receiver`'s malformed-block pre-filter does. Falsified first: `a_block_whose_deploy_is_not_signed_by_its_named_deployer_is_refused` was red on this tree (the control validated and the forged block was accepted end-to-end), and four cases now pin it — an honest deploy, an impersonation carrying a *valid* signature made by a different key, the register's own `sig = vec![1]`, and a term changed after signing. The §6 row above records the shared defect and the hard fork; §6's vault row had claimed the spend rule unconditionally, and now says what made it true. **What the fix does not do:** it refuses the forgery but does not yet *punish* it — the attribution it produces lands on `BlockMetadata.slashable`, which C122 shows is discarded by the store round-trip, so a forged block is refused and the proposer is not slashed for it. That is C122's debt, not this one's. No fixture in `casper/tests/` needed changing: those suites drive `RuntimeManager` play/replay directly and never pass through `block_summary`, so the unsigned fixtures they build are honest about what they test — and the pool has only two ingresses, the API path (which verifies at `block_api_impl.rs:303`) and the proposer's own signed system deploy (`proposer.rs:600`), so no honest node can now build a block its peers refuse |
| C121 **`POST /api/txn` is unauthenticated and spends the node's own validator REV to a caller-chosen address** — found independently by two lenses | — (no law states an ingress's authentication; this is R12's ingress class and the sibling of C112, which hardened the *admin* capability path and left this one) | **The endpoint is a "spend my account, tell me where" primitive with no authorization boundary.** `api_txn_run` (`node/src/web/http.rs:199`) is mounted on the **public** router (`:908`, `:934`) and requires only that a gateway exists and `api-server.enable-txn-api` is true; it takes no caller identity, no rate limiter, and validates a leg's `to` only for non-emptiness. The gateway it drives signs every phase deploy with **`conf.casper.validator_private_key`**, and on the participant `rho:txn prepare` derives the escrow source from the deploy's own identity — `from = RevAddress::from_deployer_id(deployer_id)` (`rholang/src/system_processes.rs:1960-1910`) — so the escrow is taken **out of the node's own REV account** (`native_state.rs:1650-1679`) and `txn_commit` credits the caller's `to` (`:1683-1711`). **The evidence for the fix's shape is the asymmetry:** every sibling state-mutating route on the same router consults `deploy_rate_limiter` (`api_deploy`, `api_faucet`, both explore-deploy handlers, `api_get_transaction`, `reporting_trace`); this one does not, and it is the only one that moves the operator's funds. `legs.len()` is unbounded, so one request buys thousands of node-signed deploys, and `TxnLedger` has no cap or expiry while `GET /api/txn` scans all of it. **Trigger:** the documented operating configuration (`docs/src/node/operating.md:43` tells operators to set `enable-txn-api`), default `api-server.host` = `0.0.0.0`, one POST with `{"txnId":"01","legs":[{"shardId":"/root","amount":<all of it>,"to":"<attacker REV address>"}]}`. **What is owed:** the route moved to `admin_router` behind C112's opt-in, or an authentication boundary added, **plus** a bound on `legs.len()` and a cap on the ledger. **Fixed, all three halves.** The transaction family (`/api/txn`, `/api/v1/txn`, and both `{txn_id}` reads) is now mounted on `admin_router` and nowhere else, and the capability moved with it: `AdminState` holds the gateway and `HttpState` no longer does, so the fund-moving routes cannot be mounted on the public listener by a later change without moving the coordinator back into the public state. That makes the bind C112's — loopback unless `api-server.enable-devnet-admin-public` — which is the *only* authorization boundary this tree has (there is no auth middleware, token, per-request loopback check or Unix socket anywhere in `node/src`, verified), and it is the same one `/api/propose` already relies on. `legs.len()` is bounded by `MAX_TXN_LEGS = 32` at the handler, before the per-leg parse loop, and `TxnLedger::put` refuses a **new** `txn_id` past `MAX_TXN_RECORDS = 10_000` — at the single write path rather than at the RPC handler, because a bound enforced by the one caller that happens to exist is the shape this register keeps recording. Updates to a record already held are exempt by construction: a transaction in flight must always be able to reach a terminal state, or `recover_in_flight` could not finish what it started. The route also consults a rate limiter (`TXN_RATE_LIMIT_PER_SEC = 10`), which is the asymmetry this row rests on — every sibling state-mutating route did, and this was the only exception. **The falsifier is a real node, not a unit double:** `an_enabled_gateway_serves_the_txn_routes_only_on_the_admin_listener` boots a two-shard gateway **with `enable-txn-api = true`** and requires `404` from the public listener for both the list and the POST while the admin listener answers the list — and it is provably red on the tree before the fix, because the test it replaces (`gateway_node_coordinates_a_two_shard_transaction`, as committed) posted that same request to the public port and asserted `200` and a committed record. That same test now drives the whole two-shard commit through the **admin** port and passes, which is the other half: the move removed the exposure without removing the feature. The ledger's cap is pinned by `the_ledger_refuses_a_new_transaction_at_its_cap_but_still_finishes_the_ones_it_holds` and the leg bound by `a_transaction_naming_too_many_legs_is_rejected`. `docs/src/node/operating.md`, `docs/src/formal/cross-shard-transactions.md`, `defaults.conf` and the `NodeConf` field doc all now say which listener serves the routes and why publishing it is a decision. **Unchanged and deliberately so:** `enable-txn-api` remains a gate, so the operator's switch is still there — it is now a second gate in front of the bind rather than a substitute for it. |

### High (4)

| C122 **the C110 slash rule reads a flag the DAG's own store cannot return, so slashing is unreachable** — "both callers" is false of this tree | — (C110's own row is the claim this refutes) | **`slashable` is in-memory only and the store round-trip loses it.** `BlockMetadata::from_proto` hard-coded `slashable: false` (the `slashable:` line of the conversion — `models/src/block_metadata.rs:82` when this row was written, `:90` once the fix below moved it, which is why the citation names the line's *content* and not only its number) and `BlockMetadataProto` had no such field (`models/proto/casper.proto:193-205` then; the message is `:193-206` with the field added). The flag's only production writers — `validate_block_checkpoint` (`casper/src/interpreter_util.rs:239`) and `mark_failed_attributable` (`casper/src/multi_parent_casper.rs:423-428`) — hand their result to `dag.insert`, which writes through `BlockMetadataStore::add` → `BlockMetadataCodec` → `to_bytes`; every read is the mirror image, `dag.lookup` → `get` → `from_bytes`, which sets it false. So it is false for **every metadata any caller can see**, not merely after a restart as the field's own doc claims (and that doc is separately wrong that `validation_failed` is not carried — it is, at `casper.proto:201`). Both C110 call sites read only through that route, so `validate::slashable_senders` **always returns the empty set**: the proposer's `to_slash` is always empty, `block_creator` never pushes a `Slash`, and `slash_is_unjustified` treats every Slash as unjustified. **Measured on the store type the node uses** (`BlockMetadataStore::create` over `BlockMetadataCodec`, `add(slashable=true, validation_failed=true)`, `get()`): `slashable=false validation_failed=true`. C110's own unit falsifier builds `BlockMetadata` by hand with `slashable = true`, and **therefore cannot see that its input is never true in production** — the fifth instance of this register's signature failure, this time in a *falsifier*. **Why it matters beyond the mechanism:** with C111 deliberately removing confiscation-on-untrust and no other seizure rule, **nothing in this tree can take a bonded validator's stake**, so attributable failures carry no economic consequence at all. **What is owed:** carry the flag in the proto (or set `slashable: b.validation_failed` in `from_proto`) **before** any fix is measured, and correct C110's row. **Fixed, taking the proto rather than the alternative this row also offered, because the alternative would have equated two flags C110's own field doc exists to separate** — a local replay failure is not the sender's fault. `BlockMetadataProto` gained `bool slashable = 22;` beside `validationFailed = 21`, and `from_proto`/`to_proto` carry it. The metadata is the *node's own* DAG index (never transmitted, never in a state hash), so the field is node-local and cannot change the wire format; what it changes is which blocks a node *produces* and *accepts*, which is why §6 registers a hard fork. **The field's doc comment was wrong three ways and now says so:** `validation_failed` **is** carried (`casper.proto:201`), the flag was lost on *every* read rather than only at a restart, and "in memory only" was never the property at work — a hard-coded `false` in the conversion was. **Falsified in both directions:** `the_slashable_flag_survives_the_store_round_trip_and_reaches_the_slash_rule` (`casper/src/block_metadata_store.rs`) is green with the conversion reading the field and red with the pre-fix literal restored — and beside it the *existing* `add_and_lookup_round_trip` stays green in both states, which is this row's point about its predecessor's falsifier: that fixture is built with `slashable: false`, so it cannot see a flag that is false. The test asserts the consequence and not only the field — the read-back metadata fed to `validate::slashable_senders` yields the sender, which is the reachability this row was about. **What the fix changes on the consensus path, stated because no test exercises it:** a proposer can now name offenders in `to_slash` (its `bonded` filter still narrows them), and a block carrying a *justified* `Slash` is now accepted where the pre-fix tree refused **every** `Slash` with `UnjustifiedSlash`. The whole `rchain-casper` suite passes before and after, so this is registered rather than pinned, and it is a lockstep upgrade. |
| C123 **a deploy pool holding 256 valid deploys makes block production impossible** — the 10,000 pool cap and the proposer's `u8` seed index are not the same bound | — (no law bounds a deploy pool; this is C113's "a bound whose terms are set in different places" shape, and §3's "a deploy list cannot exceed 255" is the assessment it refutes) | `create_block` selects **every** pooled deploy that is not future, not expired and not already included, with no truncation (`casper/src/blocks/proposer/proposer.rs:565-575`), and passes them to `BlockCreator::create`, which converts the count to `u8` for the per-deploy randomness seed: `rand.split_byte(u8::try_from(selected.len() + to_slash.len())…)` (`block_creator.rs:120`). The pool's ingress bound is a **count of 10,000** (`casper/src/dag.rs:29`), so the whole pool becomes the per-block deploy count, and at 256 the conversion fails and `create` returns `Err` **before any state work**. **Measured:** pools of 1 and 255 return `Err(unknown root)` (a stub pre-state — i.e. they pass the seed line and reach the runtime); 256 returns `Err(out of range integral type conversion attempted)`. **Trigger:** 256 distinct signed deploys to the public deploy gRPC with `phlo_price = min_phlo_price` and `phlo_limit = 0`; no funds are needed because a deploy is charged only when a block includes it, and the attacker's deploys are in this node's pool only. The ingress rate limit is 100 req/s, so the flood is ~2.6 s, refreshed once per `DEPLOY_LIFESPAN`. Applied to every validator, block production stops network-wide. **The same missing bound has a second half:** 10,000 deploys × the 16 MiB gRPC decode cap is ~160 GiB on the same store manager as the block database. **What is owed:** cap the per-block selection (or give the pool a bound the per-block capacity implies) — **not** a wider seed. **Fixed, and on the selection as prescribed: `proposer.rs`'s `MAX_BLOCK_DEPLOYS = u8::MAX as usize` with `per_block_deploy_budget(slashes)`, and the selection extracted into `select_deploys`, which now takes the pool's valid entries in canonical order and **truncates to the budget** — a deterministic prefix, because the pool is a `BTreeMap` keyed by `DeployId`, with the remainder left pooled for the next block rather than dropped.** Three things the fix states rather than implies. (1) **The bound is the *sum***: the `close_block` seed is `split_byte(u8::try_from(selected.len() + to_slash.len())?)`, so a block with slashes carries correspondingly fewer deploys — `per_block_deploy_budget` takes the slash count for exactly that reason, and the slash side needs no cap of its own because `to_slash` comes from the block's justifications, bounded by `max-number-of-parents`. (2) **The cap is not on the seed**, per this row: widening the index would hide the bound instead of respecting it. (3) **The loop stops reading at the budget as well as collecting**, so an oversized pool costs no more DAG lookups than the block can carry — which is a smaller answer to this row's second half (`10,000 deploys × the 16 MiB decode cap` on the block store), and that half is otherwise **unchanged**: the pool's own cap stays 10,000. **Falsified in both directions:** with the pre-fix behaviour restored as a mutation (`per_block_deploy_budget → usize::MAX`, i.e. no cap), `the_per_block_budget_leaves_room_for_every_seed_the_block_needs` and `the_selection_is_capped_and_takes_the_pools_canonical_prefix` both go red — the second with `left: 300, right: 255`, which is precisely the selection the pre-fix code handed to `u8::try_from` — and both are green with the cap in place. The whole `rchain-casper` suite (295 lib + every integration suite) passes either way, so nothing pinned the old behaviour. **Not a hard fork:** a block carrying 255 deploys was always valid; this makes one *producible* where the pre-fix tree produced none, and no previously-valid block becomes invalid. |
| C124 **the matcher's subset enumeration is performed before the bound that refuses it**: a 217-byte deploy costs ~16 GB RSS and ~38 s of uncharged CPU | — (the nearest is §1.6's totality read as a resource claim, which is the misreading C99 records) | `sub_pars` (`rholang/src/matcher/spatial_matcher.rs:274`) calls `min_max_subsets` once per `Par` dimension and **only then** compares the 7-way product to `MAX_SPLIT_COMBINATIONS` (`par_spatial_matcher_utils.rs:172-183`). `min_max_subsets` checks only the element count (`> MAX_SUBSET_ITEMS = 20`) and then materializes, by recursive clone, every subset/complement pair — 2ⁿ of them. **With n = 20 that is 1,048,576 entries, which is strictly greater than `MAX_SPLIT_COMBINATIONS = 1,000,000`, so the check can never prevent the enumeration it exists to prevent** — it only converts the fully-materialized result into an error afterwards. The comment at `:12` saying "beyond this bound the enumeration is a denial-of-service, so it is rejected rather than materialized" is the claim that is false: it *is* materialized at the bound. **Measured end-to-end through the real pipeline** (`cargo test --release -p rchain-rholang --test probe_deploy_e2e`): a **217-byte** deploy term → `elapsed=38.103969589s peakRSS=15942008kB`; debug build 93.3 s / 17.9 GB. Nothing interrupts it — the deploy path has no wall-clock timeout, the exploratory path's `tokio` timeout cannot abort synchronous CPU work, and matcher work is not in the phlo table, so the deploy pays ~0 phlo and fails only after the whole burst. **The cost is multiplicative and the floor is what was measured:** 2²⁰ entries × the datum's own element size, and the element size is attacker-chosen. **Every validator replaying a block containing it pays it again**, so this is a chain-halt route and not a single-node stall. **This also refutes §12's row** ("Matcher CPU uncharged — bounded by `MAX_SPLIT_COMBINATIONS`"): the bound is not a bound on the work, and the row's two cited lines (`reduce.rs:449`, `:1491`) **do not point at matcher sites at all** (the matcher entries are `reduce.rs:790/:2196`, `storage.rs:84`). **Fixed** (`00cc0237e`, `a8f8242ac`): the counts are computed first — `subset_count` is arithmetic only — the product is checked against `MAX_SPLIT_COMBINATIONS`, and only then are the seven dimensions materialized, so a deploy that would have built ~16 GB is refused before anything is allocated. **`MAX_SUBSET_ITEMS` is 19 and not 20, because the value is derived rather than chosen**: at 20 a single dimension permits 2²⁰ = 1,048,576 pairs, which is *above* the product cap, so the per-dimension guard admitted exactly what the product check must then refuse — after it had been built. *This row's own one-line proof is now the test*, and it was watched going red on the old constant: `a_single_dimension_cannot_outgrow_the_product_cap` fails with "one dimension at MAX_SUBSET_ITEMS enumerates 2^20 = 1048576 splits, which exceeds MAX_SPLIT_COMBINATIONS = 1000000". `subset_count_predicts_the_enumeration` cross-checks the arithmetic against the enumeration itself over 240 shapes, so a drift — which would have to be *too permissive* to matter — cannot be silent. |
| C125 **`GatewayTxn::run`'s check-then-act on the durable coordinator ledger is not atomic** — two concurrent calls for one `txn_id` both open the transaction | — (law 29's ledger is the thing this breaks: the record is meant to be authoritative) | `run()` reads the ledger (`ledger.get(txn_id)`), decides "no record", builds a record **from its own argument list** and writes it (`mod.rs:298`) before driving it — with no lock, no per-`txn_id` serialization and no compare-and-set. `TxnLedger` is a bare `KeyValueTypedStore` keyed by `hash(txn_id)` with plain get/put (`casper/src/gateway/ledger.rs:359-404`). Two tasks can both pass the read before either writes. **The resume path has the same shape and needs no attacker:** the startup recovery task (`node/src/runtime/node_runtime.rs:1096`) races a client's documented "re-issue the same `txnId` after a restart" retry, and two drives of the *same* in-flight record each run their own phase one with their own 30 s timeout, so one can record `Committed` while the other records `Aborted` and each sends its own phase two. **Impact:** the durable coordinator record — the only thing `recover_in_flight` trusts after a crash — can describe a different transaction than the effects applied under that `txn_id`; `status()` returns one record while two leg sets moved funds. The participants are idempotent and terminal-absorbing (law 28), so the harm is record/world divergence rather than a double debit. **What is owed:** a per-`txn_id` guard (an in-process keyed mutex is sufficient for the single-process case, which is what the node is) or a CAS on the store, plus a test that releases two `run()` futures through a barrier and asserts the stored record's legs are the union of the legs prepared. **Fixed, taking the keyed mutex this row prescribes:** `GatewayTxn` holds one `tokio::sync::Mutex` per `txn_id` in a map behind its own lock, `run` acquires it for its whole body (the body re-homed as `run_under_lock`, so the diff is a wrapper rather than a re-indentation), and **`recover_in_flight` takes the same lock per record** — which is this row's second half, and it is not an exotic race: re-issuing a `txnId` after a restart is the documented client behaviour, so the recovery task and that retry arrive together by construction. **The map is cleaned up** (an entry is removed when the last holder releases it, checked under the map's own lock so a caller that holds a handle is counted), which is the difference from the `MultiLock` whose never-cleaned map this register already records; and the entry is created *under* the map lock, because a get-then-insert outside the guard is that same recorded race. A CAS was the row's alternative and is worse here: the ledger has no such primitive, and it would serialize the *open* without serializing the *drive*, which is the longer window. **Falsified in both directions, with the harm visible rather than asserted:** on the unfixed code (a mutation that hands out a fresh lock per call, i.e. no serialization) four barrier-released callers for one `txn_id` each open their own transaction, and the test's failure prints two `Committed` records for the same id whose legs are amount **31** and amount **30** — two opens, each driving its own leg list, which is exactly the record/world divergence this row is about. With the lock, all four converge on one record and the shards pay **one open's cost**, measured in the same fixture rather than assumed. The assertion is deliberately the *number of opens* rather than "the call succeeded", which is green on the broken code too. |

### Medium (4)

| C126 **the `silent` hard class is line-anchored, so a rustfmt-wrapped `try_from(..).unwrap_or(0)` chain is invisible** | — **harness** | `scan` feeds the stripped file through `grep -E "$pattern"` one line at a time, and `PAT_SILENT` requires the fallible call and its default on the **same line**: `try_(into\(\)|from\(.*\))\.unwrap_or(\(0\)|_default\(\))` (`tools/audit-type-system.sh:1168`). When rustfmt wraps a long chain, `.unwrap_or(0)` lands on its own line and the alternation cannot match. **Demonstrated with a two-form probe in one file:** the gate reported **only** the single-line site and printed `FAIL: 1 hard violation(s)`; the wrapped twin on the next line was absent from the output entirely. Removing the single-line call would make the class green over a live violation. **This is the same failure this file's own header records for `\bassert` versus `debug_assert!`** — a pattern that cannot match the form it exists for — and it is reachable by **ordinary formatting**, not by an attacker. **Fixed:** the `silent` class now reads the stripped file *whole* (`scan_spanning`, `perl -0777`) with the pattern allowed to span whitespace, and the note keeps the line the match *starts* on. One thing the fix had to get right and did not on the first attempt, worth recording because it is invisible in the pattern: `scan` feeds `line<TAB>code`, so a span crosses the **next line's number**, and `(?:\s|\d+\t)*` rather than `\s*` is what actually closes the hole — the probe is what showed the difference between a plausible fix and a working one. **Demonstrated in both directions on a probe file carrying three spellings** (the single-line control, a wrapped `try_from`, a wrapped `.parse`): the pre-fix gate reported **only** the control while the fixed gate reports all three. The tree holds no wrapped `silent` site today, so the verdict is unchanged and only the coverage moved. |
| C127 **the `escape` class is scoped to eleven files, so `impl Deref for <refinement>` in a sibling file is invisible** | — **harness** | `scan_escapes` iterates only `REFINEMENT_FILES` (`:658-670`, the loop at `:1127-1153`), and the `ESCAPE_GET_AWK` pass is inside that same loop. The scope's stated justification — "a refinement's invariant is carried by its private field, so a direct construction can only be written in the defining module" — **is an argument about construction**, but `impl Deref for Hash32` needs no field access (`deref` can call the type's own `as_bytes()`), and Rust's coherence rule requires only that the type be local to the **crate**, not the module. **Demonstrated with a control pair:** the byte-identical `Deref` impl in a sibling file printed `OK … rc=0` (invisible), while appended to `shared/src/refined.rs` it printed `FAIL: 1 hard violation(s)`. The same hole applies to the public `.get()` form. **One file surrendering its invariant is the rule `spec/TYPE-SYSTEM.md` §1.7 exists for, and it reports green.** **Fixed:** `scan_escapes` now walks every crate's `src/**/*.rs` rather than the eleven refinement homes — for the `Deref` form *and* the `.get()` form, since coherence requires the impl's type to be local to the **crate**, so the file was never the scope. **Demonstrated on the probe:** the byte-identical `Deref` impl in a sibling file is absent from the pre-fix gate's output and is reported by the fixed one, at its own line. The widened walk finds **no** site in this tree today, so the class's verdict is unchanged and only its coverage moved. |
| C128 **`check-rust-witnesses.sh` counts a `... ignored` line as a passing witness** | — **harness** | After `cargo test <filter>` exits 0, the witness verdict is `grep -cE "^test (.*::)?${symbol} \.\.\."` (`tools/check-rust-witnesses.sh:140`) — the pattern requires only the symbol followed by ` ...`, **not ` ... ok`** — and libtest prints `test <name> ... ignored` for a filtered-in `#[ignore]`d test, which matches. cargo exits 0 when the filter selects only ignored tests, so `matched >= 1` and the checker prints `ok`. **Demonstrated live:** a probe witness `#[test] #[ignore] fn lawZZ_probe_ignored_witness() { panic!("this witness never runs"); }` registered against the real checker printed `all 1 Rust witness(es) ran and passed`, exit 0. The file's own header states the opposite guarantee ("a registry of renamed, deleted or `#[ignore]`d tests would run green while checking nothing"). **Latent today** — no registered witness is ignored — **but the register's Rust coverage is a law-level claim, and this is the gate that makes it.** **Fixed:** the verdict pattern requires `ok$` rather than a bare ` \.\.\.`, so a filtered-in `#[ignore]`d test matches nothing and the checker **fails** on it — which is exactly what the file's own header promises. **Demonstrated at the pattern level**, which is where the defect lives and what the row measured: against a libtest line `test … ... ignored`, the pre-fix pattern counts it (1) and the fixed pattern does not (0), which turns the checker's verdict from `all N ran and passed` into a failure naming that witness. |
| C129 **R19 refuted: `exploratory_deploy` reads the non-finalized chain tip again**, on the unauthenticated explore-deploy routes | — (R19's own row is the claim this refutes) | R19 (`§13`, its row in `spec/AUDIT.md`) states "Fixed — restored `last_finalized_block` as the no-hash default". Commit `8c4bdecea` did exactly that; commit `63c535c6d` ("Fix explore-deploy to read the latest block (chain tip)…", 2026-08-26) **reverted it and never touched this register**, so the row is false of this tree. The arm is now `dag.height_map.iter().next_back().and_then(|(_, hashes)| hashes.iter().next().copied())` (`casper/src/api/block_api_impl.rs:709`) — the highest height, then an **arbitrary** hash from that height's set, and the code comment there says the tip read is deliberate. **Consequence:** an unauthenticated caller's balance and explore-deploy reads are anchored to a non-finalized block, a byzantine proposer can spoil them, and two honest nodes can answer the same request from different blocks — **and because the register says the defect is fixed, nothing tracks it.** Downgraded from the finder's `high` to `medium` on reachability: the route is rate-limited and the read is not state-mutating. **What is owed:** correct R19's row, and decide deliberately which block the no-hash default should read. **Decided: the last finalized block, and R19's row now says what is true rather than what was intended.** The arm read the chain tip — the highest height, then an **arbitrary** hash from that height's set — with a comment arguing that a wallet checking a balance after its own transfer should not see the finalized fringe lag. The decision goes the other way for two reasons, and the second is the one that makes it a decision rather than a preference: an arbitrary hash from a height's set is not a block anyone chose, so two honest nodes can answer the same unauthenticated request from *different* blocks and the read is not an oracle at all; and the anchor is a block a **byzantine proposer can pick**, on the route whose answer a wallet believes about its own balance. The finalized block is the one block in the DAG that is agreed by construction — the same read `bond_status` and `last_finalized_block` already make. A caller who wants the tip asks for it by hash (`explore-deploy-by-block-hash`), which is an explicit choice instead of this route's silent one. **Falsified without a node:** the choice is extracted as `exploration_anchor`, and the test builds the DAG directly at the two heights the decision is between — a finalized block at height 0 and an unfinalized tip at height 1 — and requires the anchor to be the height-0 block. The old read returns the height-1 tip, so the test is red on it. **And the route-level consequence is asserted rather than left for the next reader to find:** on a node that has produced only its genesis the DAG has **no finalized fringe**, so the no-hash default now answers a 400 naming "Finalized fringe is not available." — the same refusal `/api/last-finalized-block` already gives, and which `node/tests/api_surface.rs` already documented as "the behaviour to assert … and explicitly not a 200 carrying an arbitrary block". That comment was written about the sibling route; this row makes the two agree. The older assertion in the same test, which called an empty hash "the latest state", is the one that had to move. **And R19's row is corrected in place**, at `§13`, from "Fixed — restored `last_finalized_block`" to the history: `8c4bdecea` did that, `63c535c6d` reverted it without touching the register, and this row is the reversal recorded rather than the claim repeated. |

### Low (6)

| C130 **the `unsafe` class matches only `unsafe {`, so `unsafe fn` / `unsafe impl` with real unsafe operations are green** | — **harness** | The pattern is `unsafe[[:space:]]*\{` and the doc says the crate graph "must be zero" unsafe because it is "entirely safe Rust". `unsafe fn f(p: *const u8) -> u8 { *p }` contains no `unsafe {`, and in edition 2021 — the workspace's edition — `unsafe_op_in_unsafe_fn` is allow-by-default, so an unsafe fn body may dereference raw pointers with no block at all. **There is no compiler backstop either:** no `#![forbid(unsafe_code)]`/`deny(unsafe_code)` and no `[lints]` table anywhere in the workspace. `OK: no hard production violations`, exit 0, while the construct sits in a scanned file. The class verifies zero unsafe **blocks**, not a safe crate graph. Latent today (no `unsafe fn`/`unsafe impl` in the tree). **Fixed in both halves the row names.** The pattern is now `unsafe[[:space:]]*(\{|fn|impl|trait|extern)` — `unsafe_code` in an attribute does not match, because the alternation requires the keyword to be followed by whitespace and then a block or a declaration keyword. And the compiler backstop this row notes is absent now exists: **`#![forbid(unsafe_code)]` at the root of all fourteen crate roots** (twelve `lib.rs`, plus `qucalc/src/main.rs` and `node/src/main.rs`), so the pattern is the belt and the lint is the structural check. **Demonstrated on the probe:** the `unsafe fn` is absent from the pre-fix gate's output, is reported by the fixed gate, and makes `cargo check -p rchain-shared` fail with `declaration of an 'unsafe' function` naming the lint — three independent refusals where there was none. The workspace compiles with the attribute in place, which is itself the evidence that the crate graph is as safe as the class claimed. |
| C131 **the completeness guard's premise for excluding public-field newtypes is false for any name not already in `REFINEMENT_TYPES`** | — **harness** | `ESCAPE_ROSTER_AWK` derives a `pub struct Name(...)` newtype only when the text inside the parens contains no `pub`, and G1's comment (`:982-984`) justifies that with "a public field is excluded because the class flags that form directly". But the class's public-field alternative is built by iterating `REFINEMENT_TYPES` (`:1131-1133`), **so it only fires on the twenty already-rostered names** — for a new name, the form it is said to flag directly is flagged by nothing. **Demonstrated:** appending `pub struct ZzProbe2(pub u16);` to a roster file left the derived count at 28 both before and after (G1 skipped it) and the class reported nothing. **G1 exists precisely to catch new refinements**, and §1.7's canonical escape gets no check unless the name was hand-added to the roster first. **Assessed and left open deliberately, with the measurement that decides it.** This one cannot be widened the way C127's could. A `pub` field is an escape only on a type whose invariant a private field carries, and this workspace has **27 public-field newtypes that are not refinements** — `ParsingError(pub String)`, `PeerId(pub Option<String>)`, `Base16(pub Vec<u8>)`, `AlwaysEqual<T>(pub T)`, `TraceId(pub i64)`, `PrivateKey(pub Vec<u8>)`, the nine `rholang/src/proc_ast.rs` AST wrappers, and the rest — so a syntactic rule would flag 27 legitimate types to catch an unknown number of real ones, and the class is *hard*, so every one of those would have to be resolved before the gate could run. The two honest routes are a per-name exemption list (27 entries, each needing a reason, which is a decision about policy rather than a patch) and a semantic rule (a newtype carrying a `TryFrom` or a validation constructor *and* a public field); both are larger than this pass, which is why the row stays open and says so. **What the probe established, rather than assumed:** the public-field newtype added to the probe file is reported by **neither** the class nor the completeness guard — so the gap is real, unchanged, and now measured. A reader should treat "a new refinement declared with a public field" as the one case nothing covers; that is this row's finding, and it is not something this pass closed. **Closed by the semantic rule, which the policy decision chose over the 27-entry exemption list.** `ESCAPE_ROSTER_AWK` now derives a tuple newtype with a `pub` field **whose file also implements `TryFrom` for it** (`G1v`), and the completeness guard already reports such a name when it is in neither `REFINEMENT_TYPES` nor `REFINEMENT_EXEMPT`. That is the shape of the defect — a validating constructor beside a field that walks around it — and it is what the syntactic rule could not be, because the 27 legitimate public-field newtypes do not carry a `TryFrom` at all: they are wrappers, not refinements, and the rule now says so structurally rather than by listing them. **The rule is vacuous on this tree, and that is the point rather than the problem** — measured: it reports nothing, so no exemption entry was needed. **A rule that cannot fail is not evidence, so it was falsified against the real awk program, on a probe file rather than the tree:** `pub struct WithTry(pub u16);` with `impl TryFrom<u16> for WithTry` in the same file prints `G1v WithTry` — the row the guard turns into a failure — while a `pub struct Plain(pub u16);` with no `TryFrom` prints nothing, which is the negative arm and the reason the 27 stay unflagged. **What this does not do:** it does not decide the shape of a validation entry point that is not `TryFrom` (a `pub fn validate` returning `Result`, say), and it promotes per *file* rather than per type, so a same-named type in another file could promote it — a false positive lands in the completeness guard, which asks a human to look rather than deciding silently. |
| C132 **registered deviation S25 is false of this tree** — it says the admin HTTP binds `api-server.host` and "matches Scala", while C112 made it loopback-by-default | — (S25's own row is the claim this refutes) | S25 (`§12`, its row in `spec/AUDIT.md`) reads "admin HTTP binds `api-server.host` (matches Scala — reverts the earlier loopback-only bind so a browser wallet can reach `/api/propose` through a published port)". The tree does the opposite: `admin_bind_host` returns `127.0.0.1` unless the operator opts in (`node/src/runtime/node_runtime.rs:140-146`), the bind at `:493` uses it, and `defaults.conf:155-160` documents that default as C112's fix **and calls the Scala's `0.0.0.0` bind the defect**. **Two rows of this register therefore contradict each other and the code**, and S25 is the one a reader reaches from the deviation table. The risk is concrete: a future pass can "restore fidelity to the Scala" on S25's authority and re-open C112. **Fixed by correcting S25 in place.** The row now says the admin HTTP binds loopback unless `api-server.enable-devnet-admin-public` is set, cites `admin_bind_host` and `defaults.conf` as the rule and its documentation, and says in the row itself why it was wrong: it described the tree *before* C112, and the Scala's wildcard bind is the defect C112 records rather than the fidelity this row should claim. Correcting rather than deleting is deliberate — a reader needs to know that the deviation table once said the opposite, or the next pass re-derives it. |
| C133 **the admin router's CORS comment promises a protection a simple cross-origin POST needs no preflight to bypass** | — (C112 fixed the bind and assessed CORS as "not authentication" for non-browser clients; this browser simple-request residual is separate) | `admin_router` (`node/src/web/http.rs:1140-1156`) comments that the restrictive `CorsLayer` stops "a browser on another origin … trigger[ing] block production". **tower-http's `CorsLayer` never rejects a request** — verified in the dependency, `tower-http-0.6.11/src/cors/mod.rs:660-707`: only `OPTIONS` takes the preflight branch, every other method is forwarded to the inner service and the layer merely adds response headers. `admin_propose` (`:891`) has no extractor and no origin check, so a bodyless cross-origin POST is a CORS **simple request** — no preflight is sent, and CORS only stops the page reading the reply. Loopback does not help: the request comes from the operator's own browser (CSRF), and DNS rebinding resolves an attacker's name to `127.0.0.1`. **No test in the tree ever drives `admin_router` over HTTP, so the claim is unpinned as well as untrue.** Downgraded to `low` on reachability: local-operator blast radius, and the attacker picks timing rather than contents. **Fixed: the comment now says what the layer does, and the residual is pinned rather than denied.** It names the mechanism (only `OPTIONS` takes the preflight branch; every other method is forwarded with response headers added), the consequence (a bodyless cross-origin `POST` is a simple request, so the handler runs and CORS only stops the page *reading* the reply), why loopback does not help (CSRF from the operator's own browser; DNS rebinding resolves to `127.0.0.1`), and what *does* protect the route — the bind and the operator's opt-in, which is a real boundary against a remote attacker and no boundary against a page the operator visits. **And it is pinned, which it was not:** `node/tests/api_surface.rs` now drives `admin_router` over HTTP for the first time in this tree — a cross-origin `POST /api/v1/propose` with no preflight reaches the handler and returns 200, and the restrictive layer answers no `access-control-allow-origin`. The first assertion is the residual written down as a test; the second is the half the old comment got right. **Closed 2026-09-28, after this row was written.** `admin_origin_guard` (`node/src/web/http.rs`) now refuses a cross-origin request to the admin router — as a *layer* on the router rather than a check inside each handler, so every route on it inherits the refusal, including the fund-spending `/api/txn` family, and so does the next admin route added. Four arms pin it in `node/tests/api_surface.rs`, and the first two are what stop the test passing by refusing everything: an origin-less request is accepted **and** reaches the handler (the CLI's shape), a same-origin request is accepted (the admin page's shape), the cross-origin simple request is refused 403 where it used to return 200, and `Origin: null` is refused — the guard compares the origin's authority against the request's `Host`, so a value that matches no host is refused rather than inspected. **No CSRF token, and that stays deliberate:** a token needs a minting endpoint and per-session state on a listener whose legitimate clients are the operator's own tooling, and the origin is the half a browser cannot forge. `api-server.enable-devnet-cors` still means "cross-origin is allowed here", so the browser-wallet path the C112 bind exists to allow is unchanged; what changed is that the *default* no longer relies on the bind alone. |
| C134 **`Secp256k1::verify_bytes` does not bind the message it verifies**: a >32-byte input is silently truncated to its first 32 bytes | — (no law states what a verify is *over*; C108's row claims the verify paths are refusal-shaped, which is a different property) | `verify_bytes` hands `data` straight to `VerifyingKey::verify_prehash` (`crypto/src/signatures/secp256k1.rs:37-45`), which reduces the message with `bytes2scalar`, and that does `if bytes.len() > n_bytes { bytes = &bytes[..n_bytes] }` — **keeping the leftmost 32 bytes** (`ecdsa-0.17.0/src/hazmat.rs:164-172`). The rho contract `rho:crypto:secp256k1Verify` is a fixed channel of arity 4 whose handler is exactly this function (`rholang/src/system_processes.rs:548-552`), so **the verdict returned to a contract is over the message's first 32 bytes, not over the message**. **Demonstrated at the crate seam:** signing a 32-byte value and verifying `msg32 || b"ARBITRARY SUFFIX THE SIGNER NEVER SIGNED"` returns **true**. C108's instrument cannot see it — `every_algorithms_verify_refuses_malformed_input` oversizes only `sig` and `pk`, and its `data` literal is 38 bytes while its own comment calls it "a 32-byte hash", so every arm returns false for a reason unrelated to the data. **Kept at `low` rather than `medium`:** the Scala's ABI takes exactly 32 bytes and its doc warns of an assertion exception on other lengths, so the divergence's severity depends on whether the JNI assertion is enabled. **What is owed:** reject `data.len() != 32` at the contract, or hash the message before verifying. **Fixed, by the first of the two routes: a named `PREHASH_LEN` guard at the top of `verify_bytes` refuses a message that is not 32 bytes.** `false` rather than an error, because the signature is not over *this* message — which is what "does not verify" means — and because `rho:crypto:secp256k1Verify`'s handler *is* this function, so a caller's answer stays a verdict instead of becoming a differently-shaped failure. **That the guard is safe for every caller was checked, not assumed:** `signature_hash` hashes to 32 for `secp256k1` (blake2b256) *and* for `secp256k1:eth` (keccak256), so both algorithms and the `secp256k1_eth` wrapper pass a 32-byte prehash — the length was an assumption that happened to hold, which is exactly why asserting it costs no legitimate call (the whole crypto suite passes either way, so nothing pinned the old behaviour). **Falsified in both directions:** the row's own demonstration sentence is a test — the signed hash with an arbitrary suffix — and it is **red on the pre-fix tree** and green after, beside a control that the signed message itself still verifies. **The instrument this row criticises had to be fixed with it, and it was worse than the row says:** `every_algorithms_verify_refuses_malformed_input` passed a **38-byte** `data` while its own comment called it "a 32-byte hash", so with the length checked it would have become *vacuous* rather than merely misattributed; and the loop's cases all fail at the *signature* parse, so it never reached `PublicKey::from_sec1_bytes` at all — the test's name promised a key check its cases could not perform. The fixture is a real 32-byte prehash now and the test gained a **key arm** (a well-formed signature over that prehash with malformed keys, control first), so the name it has always carried is finally what its cases do. **Registered as a deviation and a hard fork in §6**, because the oracle's ABI takes exactly 32 bytes and warns of an assertion exception otherwise, so it either crashes or truncates where the port now refuses. |
| C135 **`withdraw`'s deadline multiplies the raw `epoch_length` while every other epoch site divides by `epoch_divisor`** — with `epoch_length <= 0` the quarantine is silently dropped | — (no law states the withdrawal delay; law 47 is about a validator's own exit) | `is_epoch_boundary` normalizes the divisor (`native_state.rs:632-647`, where `epoch_divisor = max(epoch_length, 1)`), but `withdraw` builds the deadline from the **raw field**: `quarantine_length + epoch_length * (1 + block_number / epoch_divisor)` (`rholang/src/native_state.rs:1179-1184`). With `epoch_length == 0` the product is zero and the deadline degenerates to the absolute constant `quarantine_length` instead of an offset — the port's own doc says a zero epoch length must *mean* a one-block epoch, and the boundary test honours that reading while the deadline does not. **This is C109's tell again: a rule applied at one of two sibling epoch sites.** **Impact:** the quarantine — the only delay between a withdrawal request and the release of the bond, and the window in which a late-detected offence could still reach the stake — is not applied at all. `PosParams::default()` uses `quarantine_length = 0`, so an explicit config pair is needed. **`PLAUSIBLE`, not `CONFIRMED`:** the trigger is reasoned from the arithmetic; no shipped config was verified to use it and the refuter did not build one. **Fixed, and confirmed by the test the refuter did not build:** the deadline now multiplies `epoch_divisor(&params)` — the same normalisation `is_epoch_boundary` divides by — so `epoch_length <= 0` cannot collapse the quarantine. The falsifier is the two-arm comparison the row's own arithmetic predicts: a withdrawal at `epoch_length = 0` and one at `epoch_length = 1` must stage the **same** deadline, and that deadline must be `quarantine_length + divisor * (1 + block_number)`. With the raw field multiplied the two arms differ (`100` against `111` for quarantine 100 at block 10), which is the quarantine being dropped for one setting and honoured for the other. The concrete value is asserted as well as the equality, so both being wrong the same way cannot satisfy it. **What the fix does not change:** the reading of `epoch_length <= 0` as a one-block epoch is the port's existing decision, recorded where `epoch_divisor` is defined — this makes the two sites agree on it rather than revisiting it. |
| C136 **`find_and_connect` dials every discovered peer serially; the routing table admits thousands and `MAX_CONNECTIONS` bounds the table, not the work** | — (likely a faithful port of `CommUtil.findAndConnect`, so this belongs in §6 rather than as a defect; C105 is the same layer but a different mechanism) | `NodeDiscovery::peers()` returns the whole Kademlia table, which holds up to `REDUNDANCY(20)` × `8 × width` entries — 5120 for a 32-byte id — and `find_and_connect` awaits `connect(…)` for **every** peer not already connected, one at a time (`comm/src/rp/connect.rs:110-123`), each bounded only by `DEFAULT_SEND_TIMEOUT = 5 s`. `add_conn`'s `MAX_CONNECTIONS = 1024` is applied to the **result** by the caller (`node_runtime.rs:324`), so it caps the table and not the loop. **Trigger:** one peer answers a Kademlia lookup with thousands of fabricated `PeerNode`s at addresses that accept no connection; the next `find_and_connect` spends up to ~5120 × 5 s ≈ **7 hours** in the RP loop, during which heartbeats, block requests and connection refresh do not run — and the node is dropped by its peers. `MAX_CONNECTIONS` gives false assurance that the work is capped. **`PLAUSIBLE`, and the finder's own impact narration was refuted** by the verifier: the stall is real but the "one response plants thousands" limb is weaker than stated, and the shape is the Scala's. **What is owed:** a decision — cap the loop at `MAX_CONNECTIONS`, or register the fidelity as a §6 deviation. **Decided: bound the round, in two directions, rather than register it.** `find_and_connect` now dials at most `dial_budget(offered, connections)` peers — the table's **room**, `MAX_CONNECTIONS` minus what it already holds — and stops the round once `CONNECT_ROUND_BUDGET` (30 s) is spent. The two answer different questions and the row's own trigger needs both: room is not a heuristic (a connection the table cannot hold has no effect on it, so dialling for one is work that cannot pay, and `MAX_CONNECTIONS` used to bound the *result* through `add_conn` and not the loop), and the clock bound is what the *count* bound cannot supply, because a dial that fails costs the full `DEFAULT_SEND_TIMEOUT` and gains nothing — 5120 dials is 7 hours whether or not any of them succeeds. An unfinished round is not lost work: the discovery loop repeats on `peers_discovery.lookup_interval` and offers the rest next time. **Falsified at the seam, with the negative arm stated:** the count bound is a pure function and its test pins both endpoints — 5120 offers against an empty table dials `MAX_CONNECTIONS` (not 5120), a table one short of the cap dials exactly one, and a full table dials none — while the clock bound is **not** pinned by a test, since a transport mock over a 30 s budget is a test of the clock and this crate's `connect.rs` has none; the row says that rather than implying the round is fully covered. |
| C137 **`coverage.yml` runs unpinned action refs with no `permissions:` block** — the supply-chain hardening the other workflows got was not applied to this one | — (no law covers the build pipeline; this is R12's ingress class turned on the repo's own CI rather than on the node) | **Two workflows were hardened and two were not, which is the asymmetry.** `ci.yml` and `build-rnode.yml` pin `actions/checkout` to a full commit SHA with a `# v7.0.1` comment, and `ci.yml`, `build-rnode.yml` and `devnet-fuzz.yml` each declare a least-privilege `permissions:` block. **`coverage.yml` declares no `permissions:` block at all** — so it runs with the repository's default token scope — and resolves **five floating refs**, of which `dtolnay/rust-toolchain@stable` is a **branch rather than a tag** and therefore moves continuously; the others are `actions/checkout@v7`, `Swatinem/rust-cache@v2`, `taiki-e/install-action@cargo-llvm-cov` and `codecov/codecov-action@v7`. It triggers on every push to `dev` and `main`, so a retagged or compromised action executes arbitrary code with that token, and one of the five tracks a branch by construction. **The split is not clean either:** `build-rnode.yml`'s `actions/upload-artifact@v4` and `devnet-fuzz.yml`'s four refs are unpinned as well, so every workflow has at least one floating ref and the hardening that was clearly intended is half-applied. Severity is kept `low` — this is supply chain and not the running node. **Fixed** (`7d13ea1ae`): **every `uses:` across all five workflows now names a commit**, with the tag in a trailing comment, and **every workflow states `permissions: contents: read`** — where the block is absent the token's scope is whatever the repository default happens to be, which is a grant nobody decided. Six distinct refs were floating, including `dtolnay/rust-toolchain@stable`, which is a *branch*. **And the check the row asked for is check 16 of `tools/audit-test-register.sh`**: a `uses:` whose ref is not a 40-hex commit fails, and so does a workflow with no top-level `permissions:`. Falsified by unpinning `actions/checkout` again and watching it refuse. |

### The `ops` lens, which reported after the tables above were written (C141–C148)

**§21's first version recorded the production-readiness sweep as incomplete, because this lens had not
returned.** It has now, and it read the operator-facing surface — the config module, the startup path,
the Dockerfile, the logger and the CLI — rather than the node's protocols. **Eight of its nine findings
survived refutation**; the ninth is in the refuted table below. Two arrived as `high` and were corrected
to `medium`, and one from `medium` to `low`, all three on the same ground: **every one of these requires
the operator or the orchestrator, so none is a remote defect.** They are carried here, in their own
subsection, so the correction is visible rather than buried in a re-ranked table.

| row | law | account |
|---|---|---|
| C141 **a malformed operator config file is silently truncated** — the node runs on defaults for everything after the first syntax error, with no error and no log line | — **harness** (no law covers configuration parsing; the nearest is the register's own doctrine that an unobservable divergence is the class it exists for) | **The parse looks total and is not.** `parse_file` is `hocon::HoconLoader::new().load_file(path)?.hocon()` (`node/src/configuration/configuration.rs:234-240`, and `parse_defaults` at `:222-231`), with `?` on both steps — but hocon 0.9's loader is **non-strict by default**: `hocon-0.9.0/src/loader_config.rs:135` is `} else if self.strict { Err(Deserialization("file could not be parsed completely")) } else { parsed }`, with `strict: false` set at `:89`. The one-line opt-in `.strict()` (`lib.rs:385`) is not called by either parse site. So the keys *before* the error apply, the keys *after* it are dropped, and nothing is returned to the operator. **The asymmetry is the tell:** a *type* error in the same file is reported legibly by the same pipeline's level 0, so the error path reads as exercised while the *syntax* path is not. **Demonstrated against the shipped image, three runs:** a file starting with `storage { data-dir = /tmp/zz-no-such-dir }` followed by junk dies with `Initialization error: No such file or directory` (the key before the junk applied); the same two lines reversed start the node on the *default* data dir with no message (the valid key after the junk was dropped); and an invalid *type* alone exits 1 with `Configuration error: invalid size value 'notanumber'` — the control. **Trigger, and it is the realistic one:** `casper.shards` and per-shard genesis data exist **only** in `rnode.conf` (`docs/src/node/operating.md`: "File-configured. The command line cannot address array entries"), so an operator editing the exported `shards = [...]` array and dropping a line, or leaving an unbalanced brace, gets a node that comes up as an ordinary single-shard `/root` node with **default genesis parameters** — a different chain, with no diagnostic, and on a fresh data dir no shard-id mismatch to catch it. **What is owed:** `.strict()` at both parse sites, which turns the silent truncation into `Err("file could not be parsed completely")`. **Fixed at both sites** — `parse_defaults` and `parse_file` — with the reason written where the next reader of that loader will meet it. **Falsified in both directions:** `a_config_file_with_a_syntax_error_is_refused_not_truncated` carries the row's own demonstration (junk first, a valid key after it, which used to be silently dropped) plus the same defect seen from the other end (a valid key first, an unterminated array after it) and a **control** — a valid file must still parse, so a refusal cannot be the loader refusing everything. Removing `.strict()` reddens it; the whole `node` configuration suite (41 tests) is green either way, so nothing depended on the truncation — including `parse_default_config`, which is the evidence that `defaults.conf` itself is strict-clean and no node fails to start. |
| C142 **a server that fails to bind is never reported** — the node runs with a dead component and no log line | — **harness** | **`tokio::join!` completes when *all* its futures do, and four of the five never complete.** `NodeProgram::serve` spawns five servers and joins them (`node/src/runtime/node_runtime.rs:513`); each `Err` is surfaced only by `??` *after* the join returns, but the other four are `axum::serve`/tonic `serve` loops that never return — so a server whose bind failed (its task ends with `Err` at once) leaves the join pending forever, the error is held in the tuple, and `main.rs:125`'s `Server error:` arm is unreachable in practice. Nothing else reports it either: the acquire functions return `Result<(), String>` and no branch logs it, and the spawn sites have no log. **Demonstrated in the shipped image:** two nodes with disjoint ports except `--api-port-http 40403`, held by node A; node B is launched with `--dev-mode`, and `curl 127.0.0.1:40403/api/status` returns `devMode:false` — B is not on it — while B's log contains no error, panic or bind line and ends at `Making a transition to Running state.` **The same shape on every listener:** `--protocol-port`, `--api-port-grpc-external`, `--api-port-admin-http`, `--discovery-port`. **Impact:** fail-open startup on all five, on the component the operator's health check most depends on — and on a validator, losing the protocol listener means no peers and no finality while the process reports itself healthy. **What is owed:** `select!`, or a per-handle await that logs; a test that drives a bind failure and requires the error to be observed fails on this tree. **Fixed with the `select!`, and the report names the listener.** The five handles are `select!`ed instead of `join!`ed and the first completion becomes the `Err` `serve` returns, so `main.rs`'s `Server error:` arm is reachable and the process exits rather than serving four fifths of itself; an `Ok(())` is treated as equally fatal, because these are accept loops and one that returns is not serving either. The naming is `listener_stopped`, which keeps a task's `JoinError` distinct from the listener's own `Err` — reporting a panic as a refused bind would hide the panic. **Falsified in both directions, in-process, against this row's own shipped-image observation:** `node/tests/listener_failure.rs` holds one port, boots an otherwise-free node, and requires the node's own `JoinHandle` to complete with an `Err` naming that listener. On the pre-fix `serve` (at `98d4a63f4`) both cases time out at 30 s and the log ends at `Making a transition to Running state.` — the row's demonstration reproduced without a second process — and with the fix they complete in under a second. Two listeners are driven, the HTTP port and the **internal gRPC** port, because what is being pinned is the row's own claim that the shape is the same on every listener rather than an HTTP path. **One claim in this row needed correcting while fixing it, and it is the same trap as C134's instrument:** `--discovery-port` is *not* silent. Its listener is spawned in `create_comm_state` (not in `serve`) and its arm already logs — `log.error(source, "Kademlia RPC server failed: {e}")` — which is the "per-handle await that logs" this row's remedy clause allows. So the tuple that needed repairing was the four-of-five in `serve`, and the sixth listener keeps its log-only disposition deliberately: a discovery listener failing does not stop this node from validating, so the comm layer reports and continues rather than taking the process down. |
| C143 **nine config keys in `defaults.conf` are parsed and never enforced** — including the one the file's longest comment tells operators to tune | — **harness** | **The fields reach `NodeConf` and are read by nothing outside the configuration module.** Verified by grepping each field name across every crate (excluding `node/src/configuration`): **zero production reads for all nine.** The load-bearing one is `protocol_server.max_message_consumers` (`defaults.conf:61`), whose value becomes `ConcurrencyLimits::default()` — the hardcoded `MAX_CONCURRENT_DISPATCH = 1024` (`comm/src/transport/grpc_transport_receiver.rs:185`, reached via `serve_with_limits(.., ConcurrencyLimits::default())` at `grpc_transport_server.rs:60`). The others are `use_random_ports` (`:35`), `dynamic_ip` (`:38`), `peers_discovery.init_wait_loop_interval` (`:119`), and the whole gRPC keepalive group `keep_alive_time` / `keep_alive_timeout` / `permit_keep_alive_time` / `max_connection_age` / `max_connection_age_grace` (`:162-172`) — there is no HTTP/2 or TCP keepalive set anywhere in the tree. **`defaults.conf` documents `max-message-consumers` in twelve lines** ("The very minimum should be {number of nodes} * {synchrony constraint}", "Recommended to make this number high") **and the keepalive group in the Scala's own words** — so the operator-facing text promises an effect the tree does not deliver. **Trigger:** set `max-message-consumers = 5000` or `keep-alive-timeout = 10 seconds`; the node starts with no note and the effective behaviour is 1024 slots and no keepalive at all. **This is the register's own recorded class twice over** (the disable-state-exporter and Kamon-reporter keys), and the disposition it chose for the metrics keys — *refused rather than accepted and ignored* — is the precedent. **What is owed:** either wire the keys or refuse them at startup; the register's existing disposition says refuse. **Fixed by that disposition, and the comparison is *derived* rather than tabulated:** `check_inert_config` compares each of the nine fields against the `NodeConf` that `defaults.conf` alone produces, so a default cannot drift out from under the refusal — a hand-written table of the nine defaults would be a *second definition* of them, which is this register's own recurring class. An operator who changed none of them is unaffected, which is every existing test: the configuration suite's forty-two tests passing *unchanged* is the control that matters most here. **Falsified in both directions:** all nine are refused through a **config file** (the operator's route), `max-message-consumers` is refused through the `--protocol-max-message-consumers` **flag** as well (the highest-priority layer, so the file spelling is not the only path), and a setting this port *does* enforce still builds. Removing the `check_inert_config` call reddens all of it. **Two things the test itself had to learn, recorded because they are one lesson from two ends:** a fixture that sets a key to *its own default* is a change that did not happen, and the comparison is right not to refuse it (`init-wait-loop-interval`'s default is `1s`, and the first version of this test set exactly that); and the test **collects** its failures and reports them in one run rather than asserting inside the loop, which is what showed it — a loop that panicked on the first body would have hidden the other eight. **What the fix does not do:** it refuses the knobs, it does not implement them. `max-message-consumers` still leaves the dispatch bound at the hardcoded 1024 for an operator who removes the setting, and no keepalive exists at all — the difference is that the node now says so *before* it runs rather than leaving the operator to infer it from behaviour. |
| C144 **SIGTERM is ignored and there is no shutdown path** — every stop is a SIGKILL after the grace period | — **harness** | **No signal handler is installed anywhere.** `grep -rnE 'tokio::signal|ctrl_c|SIGTERM|ctrlc'` over every workspace crate returns **zero hits**, and nothing in the runtime has a shutdown branch — the test harness says so in passing (`TestNode::shutdown` is `handle.abort()`, "the node has no graceful-shutdown RPC"). Combined with `ENTRYPOINT ["rnode"]` in `docker/rnode/Dockerfile`, **the node is PID 1 of the container, and the kernel does not apply the default terminate disposition to PID 1** — so SIGTERM is delivered and discarded. **Demonstrated:** `docker kill -s TERM <name>` on a running container leaves it `Running=true` twenty seconds later with no log line and no exit; a plain `docker stop` ended at `ExitCode=137`. **Every systemd `ExecStop`, compose `stop`, and pod eviction hits this.** **Impact:** no orderly drain on restart or upgrade — LMDB environments are not closed and in-flight block acceptance, state sync and proposer work are killed mid-flight — and the operator pays the full termination grace on every stop. `tools/devnet.sh down` uses `docker rm -f`, which is why the devnet hides it. **What is owed:** a SIGTERM handler (or `--init`/tini) and a drain path; the probe is `docker kill -s TERM` exiting 143 or 0 promptly. **Fixed: the handler and the drain are one path now** (`node/src/runtime/shutdown.rs`). `shutdown_signal` waits on **SIGTERM and SIGINT** and names which arrived; `main.rs` races it against a pinned `serve` future rather than moving `serve` into a `select!` arm — on a signal the future must still be *awaited*, or dropping it would detach the listeners and turn the drain back into a truncation. The word goes out on one `watch` channel, every listener awaits it (`stop_requested`), and the four that have a shutdown path drain behind it: `axum::serve(..).with_graceful_shutdown(..)` on both HTTP servers and `serve_with_shutdown(..)` on both gRPC servers, so a request in flight is answered rather than cut off. `serve` then waits for them, **bounded at `SHUTDOWN_DRAIN_TIMEOUT` (10 s)** — a listener that will not finish must not hold the process past the orchestrator's grace, which is the cost this path exists to stop paying. **Bounded means bounded:** the stop arm is in C142's `select!` for exactly that reason, since without it the wait could only end when a listener returned. **Falsified in both directions, and the first draft had a defect the falsification caught.** Removing `with_graceful_shutdown` from `acquire_http_server` reddens `node/tests/shutdown.rs` at *"the HTTP listener must be closed after the stop, not merely unreported"*; replacing `stop_requested`'s `wait_for` with a plain borrow reddens the unit test at *"a listener must not stop before the coordinator asks it to"*. And the defect: a listener that drains **because it was told to** returns `Ok(())`, which `listener_stopped` (C142's naming function) read as *"an accept loop returned"* — so `select!`'s arbitrary choice among ready arms would have made `docker stop` exit 1 on a perfectly clean shutdown, some of the time. `listener_stopped` now takes the `stopping` state and a drained listener reports `Ok`, which is the signal for `serve` to wait out the others. **What this does not do, stated rather than implied:** the **transport/protocol listener still has no shutdown path** — `TransportLayerServer::serve` reaches `grpc_transport_receiver`, which is the layer that would need one, and it is not drained here; the stop arm's bound is what keeps that from holding the process open. The end-to-end probe stays this row's own (`docker kill -s TERM` exiting promptly), because a test cannot raise SIGTERM against its own process without killing the runner: what `node/tests/shutdown.rs` pins is the half the signal is wired *to* — a handler flipping a channel nothing listened to would leave the container exactly as stuck as before. |
| C145 **the production logger emits no timestamp and has no verbosity control** | — **harness** | `StderrLog` is the only logger the node installs (`main.rs:116`, and again for the peer-message router at `node_runtime.rs:2091`). Its `info`/`warn`/`error` write `eprintln!("INFO  [{}] {}", source.class_name, msg)` — level, class and message, **no wall-clock and no date** — while `debug`/`trace` are empty bodies and `is_trace_enabled` returns a hardcoded `false`, so every debug/trace diagnostic in the tree is unreachable in the shipped binary. There is no `--log-level`/`--log-format` flag in the CLI surface (101 long options checked) and no `RUST_LOG` or tracing subscriber anywhere. **Impact:** incident forensics on one node degrade to ordering-only, and cross-node timeline reconstruction — the common case for a consensus stall or a refused block — is impossible from the logs alone; there is no way to raise verbosity short of rebuilding. **Falsifier:** grep a log line for a date-like token, or for any log-level flag in the option list — no match today. **Fixed: every line now carries the wall clock and the operator chooses the level.** `StderrLog` writes `2026-09-27T12:34:56.789Z INFO  [class] msg`, its `debug`/`trace` bodies emit instead of returning, and `is_trace_enabled` answers from the logger's own level rather than a hardcoded `false` — so the diagnostics behind them are reachable in the shipped binary for the first time. The level is a new global `--log-level` option (`error|warn|info|debug|trace`, default `info`, so an operator who sets nothing sees exactly the lines they saw before), resolved in `main` before the runtime is built, the same place and for the same reason as `--thread-pool-size`: a typo has to fail before the node starts rather than leave the operator with logs quieter than they asked for. **The calendar is pinned, not eyeballed:** `utc_timestamp` takes the instant as an argument, and the test asserts a fixed second (`1_700_000_000` → `2023-11-14T22:13:20.123Z`), the epoch itself, and a leap day (`2000-02-29`) — the two instants the March-based shift and the floor division get wrong if either is dropped. The level gate is asserted in both directions (`info` emits `info` and not `debug`; a `trace` logger traces), because getting it backwards is the defect this closed, and a typo is a refusal naming the accepted values rather than a silent fallback. **What this does not do:** there is no `--log-format`, no per-module level and no structured output — the line is a rendering choice this fixes nothing about. The arithmetic is `div_euclid`/`rem_euclid` deliberately: a calendar wants floor division (the day count is negative before 1970), and the spelling says so. |
| C146 **a dangling `C 315` pointer in the operator-facing config, invisible to the check whose job is resolving pointers** | — **harness** | `node/src/configuration/model.rs:114` and `defaults.conf:360` both cite `spec/AUDIT.md C 315` as the row recording the Kamon-switch story. **No such row exists** — the allocator's maximum is far below it, and the string `C 315` appears nowhere in `spec/`. **The check that exists to catch exactly this cannot see it:** `tools/audit-test-register.sh`'s pointer family, whose banner is "every C-number and section reference resolves", builds `pointer_sources` (`:973-977`) from `spec/*.md` and `spec/Rchain/*.lean` **only**, so a dangling citation in `node/src/**` is outside its subject. Two files an operator reads — the config module's own comments — point at a register row that is not there. **Impact:** the register's cross-reference discipline is unenforced outside `spec/`, so the source tree can accumulate citations to findings that were never allocated or were renumbered, while the pointer check prints OK. **Falsifier:** the gate passes today *with the dangling pointer present*, which is the defect; adding `node/src/**/*.rs` and the `.conf` to `pointer_sources` makes it fail, and correcting the citation makes it pass. **Fixed in both halves, and this row's own falsifier was run in both directions.** The two citations (`node/src/configuration/model.rs` and `defaults.conf`) now name **§6's metrics row** — which is where that story actually lives — instead of a `C 315` that was never allocated. And `pointer_sources` now includes the source tree: `spec/*.md`, the Lean modules, every `<crate>/src/**/*.rs` and `defaults.conf`. **Measured before widening:** the citation set in `node/src` is `C121, C112, C141, C147, C38, C81, C61, C67, C46` plus two `§8` references — every one allocated — so the widened check passes on this tree; and the resolved-pointer count goes from **998 to 1510**, i.e. 512 citations that were outside the check's subject are now inside it. **Demonstrated after widening:** appending `// AUDIT C 999` to `model.rs` makes the gate fail with "names `C 999`, which no finding claims — the allocator's in-use set is the oracle", and removing it makes the gate pass again. (The probe also demonstrated the trap this project's own conventions note records: `git checkout -- <file>` discards *all* edits to it, including the fix that was just made, so the citation correction had to be re-applied. Said here because the next reader will reach for that command.) |
| C147 **an unknown `--profile` is silently replaced by the default profile** — moving the data directory | — **harness** | `Configuration::build` resolves the flag with `profiles().into_iter().find(|p| &p.name == name).unwrap_or_else(default_profile)` (`node/src/configuration/configuration.rs:52`) — **no error and no log for an unrecognised name.** `profiles()` holds exactly two entries (`default` → `$HOME/.rnode`, `docker` → `/var/lib/rnode`), and `default_profile` further degrades to an *empty* PATH when `HOME` is unset (`std::env::var_os("HOME").unwrap_or_default()`), giving the **relative** path `.rnode`. **Trigger:** `rnode --profile docer run -s` — a typo on the operator's normal path, since the docs' own command is `rnode --profile docker run …` (`docs/src/node/running-a-public-testnet.md:46`). The node starts with no warning and writes to `$HOME/.rnode` instead: **a fresh, empty data directory**, so the restart looks like data loss or an unexpected re-genesis, with nothing naming the profile. In the container case with `HOME` unset the directory is relative to the CWD. **Falsifier:** `build()` with `--profile nosuch` returns `Ok` today; a test asserting `Err` (or a distinct unknown-profile message) fails on this tree. **Fixed: an unknown name is now refused**, naming it *and* the profiles this port has, so the operator sees both halves of the mistake. The control is deliberate — `default` and `docker` must still build — so the refusal is about the name rather than about profiles in general. **Falsified in both directions:** `an_unknown_profile_is_refused_rather_than_replaced_by_the_default` reddens when the silent `unwrap_or_else(default_profile)` is restored, and is green with the refusal. One thing the test discovered that the row does not mention: `--profile` is a **global** option and must precede the subcommand (`rnode --profile docker run -s …`, which is how the docs spell it), so a test cannot reach it through the `build_run` helper whose flags land after `run` — the argv is built directly. **Whether the oracle falls back here is not established** (typesafe-config's `--profile` selects a *file*, and the Scala's behaviour on a name with no file is unrecorded), so the refusal rests on the port's own ground: a silent change of data directory is the class this register exists for. |
| C148 **no read surface for the PoS epoch, the active validator set, or pending withdrawals** | — (no law states what an operator must be able to observe; this is the register's own three-am rule) | The public route table (`node/src/web/http.rs:898-956`) exposes version, metrics, status, the deploy/explore/data-at-name set, blocks, deploys, transactions, shards and the OpenAPI document, and the CLI's subcommands are the `Options` list. **The only validator-state read is `bond-status <public_key>` — a bool — plus `GET /api/v1/shards`'s per-shard heights.** The epoch counter, the active validator set and `pendingWithdrawers` live in native state and are projected by no route and no subcommand. **Trigger:** `docs/src/node/operating.md` explains the rule ("withdraw … only stages a deadline (pendingWithdrawers) … the boundary is `blockNumber % epochLength == 0`") and then **leaves the operator to derive the boundary from block heights** — there is no way to read the current epoch, the validator set, or how long a staged withdrawal has left, and no surface answers "why was my block refused" beyond the `/reporting/trace` replay. **Impact:** epoch and unbonding questions get answered by inference, or by hand-deploying rholang from a console on a live validator — exactly the poking the REPL's isolated eval store exists to avoid for terms, and which native state offers no alternative to. **Falsifier:** grep the route table and the subcommand list for epoch / validator set / pending withdrawals; the only `epoch` match in `node/src/web` is the block-header timestamp description, and `bond-status` returns a bool. **Fixed: `GET /api/v1/pos` — one read that answers all three questions, plus the two countdowns the operator was deriving by hand.** `node/src/web/pos_read.rs` reads the primary shard's **live native state** (`pos:params`, `pos:active`, `pos:pendingWithdrawers`) through the `RuntimeManager` that owns it, and the head's height from the same `WebApi::status` `/api/status` answers with — one definition of "latest block" rather than a second one here. The handler renders `latestBlockNumber`, `epochLength`, `epoch`, `blocksUntilEpochBoundary`, `quarantineLength`, `activeValidators` and `pendingWithdrawals[{validator, stagedAtBlock, blocksRemaining}]`. **The two derived numbers are the row's own trigger answered:** `blocksUntilEpochBoundary` is exactly the "derive the boundary from block heights" step the docs walk the operator through, and `blocksRemaining` is how long a staged withdrawal has left. Both are floored at zero rather than wrapping, and `epochLength <= 1` ("every block is a boundary", a real genesis setting) answers `0` instead of dividing by it — the shape a naive `%` would have panicked on. **The deliberate design choice, stated because a reader will ask:** the read is a trait of its own rather than a method on `BlockApi` — it needs the runtime manager and the status API, and adding a method to `BlockApi` would have meant touching every mock of it in the tree for a read no block-path caller wants. **Falsified at three layers, because the interesting failure is a *wrong* number rather than a missing route:** the arithmetic is pinned on both sides of a boundary and at the degenerate setting; the rendering is pinned through the handler with non-round values so a transposed field fails; and `node/tests/api_surface.rs` asserts against a booted node that `activeValidators` is the validator the genesis bonds file bonded — on the pre-fix tree the route does not exist. **Disclosure is deliberate and is not new:** `bond-status` already reports a validator's bond on this public API, and the active set and pending withdrawals are public chain facts. Not fixed: no CLI subcommand and no per-shard selection — the route answers for the primary shard, which is what `GET /api/v1/shards` already scopes its heights to. |
| C149 **law 46's declared witness cannot fail on the mechanism it names, because its own fixture makes the proportionality factor the identity for every validator it builds** — a fifth instance of the C118 class, and the second where the test that does catch it belongs to another law | — **the instrument** (the law is fine and `epoch_reward` is correct; what the register files as the row's evidence is not) | **The witness is `an_epoch_splits_the_pot_and_keeps_the_dust` (`rholang/src/native_state.rs:2594-2668`) and the mechanism it stands for is `epoch_reward` (`native_state.rs:689-721`): `pot * (bond / minimum_bond) / (active_bonds / minimum_bond)`. The fixture sets `minimum_bond: 3` and bonds the two validators at **4 and 5** — and `4 / 3 == 5 / 3 == 1`, so the factor under test is the identity for every validator the test builds, and two deliberately *different* bonds yield the *same* share by construction. Deleting the factor entirely (`scaled = i128::from(pot) * i128::from(bond / minimum_bond)` → `scaled = i128::from(pot)`, i.e. every active validator is paid the whole pot) leaves the witness **green** (observed): its assertions are `committed[v1] == 3`, `committed[v2] == 3`, the sum `< 10`, and a post-epoch pot of 4 — and `10 * 1 / 3 == 3` under the real formula and the mutated one alike. **The mutation is caught, and by law 47's witness**: `a_released_withdrawal_pays_the_bond_plus_the_committed_rewards` (`native_state.rs:2963-3005`) goes red, and the whole `native_state` module under the mutation reports `34 passed; 1 failed`, that test and nothing else (observed). **So law 46's evidence is vacuous and its mechanism is covered by the row beside it** — C140's shape exactly (law 15's witnesses vacuous, law 18's catching it), reached by a different route. **The route is the part worth carrying forward, because it is new to this class.** The four earlier instances are *one-directional assertions*: a property whose degenerate case satisfies it (`equal ⇒ same hash`, `unchanged is a superset`, `closed stays closed`, `idempotent`). This is not that. The assertion here is the right assertion and it is two-sided and exact — `== 3`, not `>= 0` — and the mechanism is real; what is degenerate is the **fixture's parameters**, which collapse the term under test to the identity before any assertion is evaluated. A reader auditing assertions for one-directionality will never find this one, and neither will a coverage count: the test executes the function, reaches the mutation site, and passes. Only running the mutation finds it. The sweep should therefore ask *does the fixture vary the quantity the assertion is about?* as a question separate from *is the assertion one-directional?* **Both sides carried the same degeneracy and both are fixed** — the Lean half landed separately in `7783a957f`: `Rchain/Pos.lean`'s `the_dust_is_real` was the *identical* instance (`reward 10 3 9 4 + reward 10 3 9 5 = 6`), and its doc-comment went further than the Rust ever did by *stating the degeneracy as the evidence of strictness* — "each validator's scaled share is `4/3 = 1` and `5/3 = 1`" — so the row was certified by a computation that could not see the factor it was cited for. Both sides now build bonds `[4, 8]` against `minimumBond 3`: normaliser `12 / 3 = 4`, shares `10 * 1 / 4 = 2` and `10 * 2 / 4 = 5`, sum 7, three units of dust, so the inequality stays strict and the factor is no longer invisible. One further consequence worth carrying: **law 45's row described the same fixture in its own words** ("minimum bond 3, bonds `[4, 5]`, pot 10 — each validator is paid 3"), so a reader arriving from law 45 met the degeneracy without ever meeting C149 — two rows, one fixture, and only the audit's row was about the defect. That prose moved with the Lean too. |
| C150 **law 21's declared witness cannot fail on the gate, because its fixture runs on a single-threaded runtime where the gate has no observable effect** — the mutation is *masked*, not the assertion weak, and the remedy is measured rather than argued | — **the instrument** (the gate is correct: with the runtime swapped, the intact gate still refines the sequential fold under four worker threads) | **The witness is `law21_the_gate_scheduler_refines_the_sequential_reference` (`rholang/src/property_tests.rs:412`) through `compare_mode` (`property_tests.rs:365`), and the mechanism is the Gate mode's predecessor-await chain (`rholang/src/reduce.rs`, the `EffectMode::Gate` arm): effect `i`'s task owns effect `i−1`'s handle and awaits it first. Deleting the chain (`predecessor = Some(handle)` → `predecessor = None`) so that every effect spawns ungated leaves the witness **GREEN** (observed). **The cause is the runtime, not the assertion, and separating those two was the whole of the work.** `compare_mode` builds `tokio::runtime::Builder::new_current_thread()`, and on a single-threaded runtime an ungated set of `tokio::spawn`ed tasks is still *polled in spawn order* — which is DFS order — so deleting the gate changes nothing the witness is able to see. That is law 7's trap (a mutation masked by a second mechanism) and not C149's (a fixture that collapses the term under test to the identity); the two are indistinguishable from an exit code, and a run that reported either as the other would be wrong in a different way each time. **The experiment that separated them:** with `compare_mode` swapped to `new_multi_thread().worker_threads(4)` *and* the gate still deleted, the declared witness fails **3 of 3 runs**; with the runtime swapped and the **gate intact** it passes. So the gate genuinely refines the sequential fold under real concurrency, and what was absent was not the mechanism's content but any way for the witness to observe it. **Fixed rather than merely registered:** `compare_mode` now builds the multi-threaded runtime. The trade-off is stated and not hidden — a concurrency witness is a place flakiness enters a suite, and this makes two rows (21 and 25) depend on real interleaving where previously they could not fail at all. Law 25's witness was run under the new runtime with the gate intact and passes, so the sibling did not break under the change. |
| C151 **law 25's declared witness cannot fail when the block path's validation gate is switched off, and the switch from scheduler mode to validation flag is covered by no test in either crate** — a coverage gap found by deleting the gate and watching 542 tests stay green | — **the instrument, with a coverage hole under it** (the gate itself is load-bearing and falsifiable — law 24's witness catches a mutation of the queue's validation, driving `ChannelClaimQueue` directly) | **The mechanism is one line, `rholang/src/reduce.rs:2323`: `set_validation_enabled(mode == EffectMode::RelaxedValidated)`.** Replacing it with `set_validation_enabled(false)` disables Law 24's per-commit certificate for every scheduler mode. **Nothing notices.** The whole `rchain-rholang` library — 250 tests, including law 25's declared witness — stays green, and so does the whole `rchain-casper` library, 292 tests, the crate that actually selects the mode: `casper/src/runtime_manager.rs:431,433` and `:606,609` are the two sites that switch `RelaxedValidated` around the per-deploy sequential fallback. **Counted rather than assumed** — `cargo test -p rchain-casper ::` reports 292 tests executed under the mutation and `cargo test -p rchain-rholang ::` 250, so these are not empty runs dressed as greens. **The gate is load-bearing, which is why this is a hole and not a redundancy:** law 24's row records that disabling the queue's validation makes its witness fail, and `rspace/src/concurrent/channel_queue.rs:351` is where the flag is read. But every test that exercises it sets the flag **directly on its own queue** — `rspace/src/property_tests.rs:381`, plus five sites inside `channel_queue.rs` itself (lines 701, 721, 731, 752) — which bypasses the mode switch entirely. `reduce.rs:2323` is the **only** production site that enables it. So between *the queue honours the flag* (tested) and *the scheduler sets the flag* (tested by nothing) there is a gap exactly the width of the line that matters on the block path. **What this does to the register is C149's and C150's shape a third time:** law 25's `rustWitness` names `law25_the_validated_relaxed_scheduler_refines_sequential`, that witness is green under the deletion, so the row's evidence does not establish the row. **What is owed, and what this does not claim:** law 25 needs a program whose relaxed commits *would* diverge from the sequential fold without validation. `arb_program` generates sends and receives on named channels, which are confluent under reordering, so the comparison cannot separate validated from unvalidated on the shapes it can build. Whether validation is *necessary* on shipped paths is law 24's question and is answered there; this row is that nobody can see that answer from the block path. **The coverage gap is closed, and law 25's evidence is still not** — `law24_the_effect_mode_is_what_enables_the_certificate` landed in `rholang/src/reduce.rs` (`5ee8155be`) and drives the real `set_effect_mode` rather than the flag: with `RelaxedValidated` a write stamped at the claim's own path must be refused, and under `Sequential`, `Gate` and `Relaxed` the same stamp and claim must be accepted, the three controls pinning the wiring as an *equality* with `RelaxedValidated` rather than a truthiness any non-default mode would satisfy. **Re-measured here rather than relayed:** the same `set_validation_enabled(false)` plant now reports `250 passed; 1 failed`, that test named in the failure — where before it was all-green, which was this row's measurement. So the second half of the finding is discharged: the mode→certificate switch is now witnessed. **What survives is the first half, and it should not be read as closed with it:** law 25's own declared witness is still green under the same plant, so the row that claims the validated-relaxed scheduler refines the sequential fold still rests on a witness that cannot fail on the certificate — the evidence for law 25 now exists in the tree, filed under law 24. **Registered rather than left open:** a witness for law 25 *itself* needs a program whose relaxed commits diverge from the sequential fold without validation, and `arb_program`'s send-and-receive shapes are confluent under reordering, so the comparison cannot separate validated from unvalidated on anything it can build. That is a test to write rather than a fix to make, and it is recorded as owed with its reason rather than carried as a row nobody can close. |
| C152 **law 11's machine-checked `rustWitness` names a test that cannot fail on the row's mechanism, while the row's own prose names the two that can** — a pointer defect of C119's kind, where the register knows the right answer in one column and contradicts it in another | — **the instrument** (the replay checker is falsifiable and both of its error directions are tested) | **Law 11's `rustWitness` is `rspace/src/property_tests.rs:law11_a_replayed_script_matches_its_recording`, and its `falsifiable` column says something stronger and different:** the equivalence is "refutable from both sides", and in the port "each half has its own error that the Rust's tests assert fires" — naming `a_rigged_replay_matches_its_recorded_trace` (`rspace/src/replay_rspace.rs:683`) for `ReplayCommNotInTrace` and `a_rig_whose_comm_never_happens_is_reported` (`:711`) for `Unused COMM event`. **Measured:** neutering `ReplayReplaySpace::check_replay_data` to answer `Ok` unconditionally (`if data.is_empty()` → `if true`) leaves the column's named witness **green** and reddens `a_rig_whose_comm_never_happens_is_reported` — 3 passed, 1 failed, and the one that failed is named by no law row's `rustWitness`. **This is the exact inverse of C138/C140**, and worth recording as such: there the prose was thin and the column pointed at a vacuous test; here the prose is *precise* — it names both error directions, the line ranges and the error variants — and the column, which is what the machine reads and what a sweep filters on, names neither. A reader who trusts the register's `falsifiable` prose has the right answer; a tool that reads `rustWitness` does not. **The mechanism makes the column's choice look reasonable and is the reason this is easy to miss:** the checker reports only *rigged COMM events the replay never produced*, never missing ones, so the failing shape is a rig that *expects* a match the replay does not make — which `law11_a_replayed_script_matches_its_recording` cannot construct, because its script is replayed against its own recording by construction. **What is owed:** the `rustWitness` column should carry the two side tests. That is a three-file change — `spec/laws.tsv`, `spec/Rchain/Laws.lean` and the emitted `spec/LAWS.md` — and belongs with the register work rather than with this sweep, since doing the tsv alone is the drift that was repaired in `7783a957f`. **Fixed in the register pass this row asked for, in the direction it asked for it** — `Rchain/Laws.lean` first, then `laws.tsv` and `LAWS.md` re-emitted rather than hand-edited: law 11's column 14 now names both side tests, `rspace/src/replay_rspace.rs:a_rigged_replay_matches_its_recorded_trace` and `…:a_rig_whose_comm_never_happens_is_reported`, beside the recorded-trace test. The *names* are this row's measurement, relayed as such — the sweep ran the mutation and watched which test reddened. What was established here is only that both resolve in that file and that the register's own citation check accepts them; the law-27 half of the pass was re-derived (below) and this one was not. |
| C153 **law 27's register entry points its Rust evidence at two end-to-end integration tests and at nothing else, while three tests named `law27_*` carry the row's per-guard evidence** — the column is not wrong about the headline and is silent about everything under it | — **the instrument** (the guards are implemented and tested; the register does not point at the tests) | **Law 27's `rustWitness` is `casper/tests/cross_shard_txn.rs:two_shard_2pc_commits_all` and `…:two_shard_2pc_aborts_all_when_a_leg_fails`, and its `falsifiable` column names five Lean theorems and **no Rust test at all**. In the same crate, `casper/src/property_tests.rs` holds `law27_a_legless_record_cannot_commit` (`:204`), `law27_an_abort_vote_prevents_a_later_commit` (`:~120`) and `law27_and_law29_the_state_agrees_with_the_votes` (`:173`) — tests that put the law's number in their own names — and no register field mentions one of them. **Measured:** deleting the `&& !self.legs.is_empty()` conjunct from `decision_is_deterministic` (`casper/src/gateway/ledger.rs`) — the guard that stops a legless record being vacuously all-ready — leaves **both declared witnesses green** and reddens `law27_a_legless_record_cannot_commit`. `cargo test -p rchain-casper two_shard_2pc` passes; `cargo test -p rchain-casper law27_a_legless` fails. **The general shape, which is C152's and C119's:** the `rustWitness` column is what the gates read and what a mutation sweep filters on, so a row whose column under-describes its evidence is a row a tool will clear on partial evidence. Here the column is defensible for the headline claim — the integration tests do exercise the commit-all and abort-all paths — which is precisely why it is easy to leave alone: nothing is *false*, and the sweep still ends with the wrong conclusion for this guard. **What is owed:** the same three-file column change as C152 — `spec/laws.tsv`, `spec/Rchain/Laws.lean`, the emitted `spec/LAWS.md` — and the same reason for not doing it in this sweep. **Fixed in that three-file pass**, and **this row's measurement was re-derived rather than relayed**: the `&& !self.legs.is_empty()` conjunct was deleted from `decision_is_deterministic` again, `law27_a_legless_record_cannot_commit` went **red**, and both tests the column *used to* name stayed **green** — this row's claim in both directions, on this tree. Law 27's column 14 now names the three `law27_*` property tests above the two integration tests, which is the arrangement the row describes: the headline is still the integration pair, and the per-guard evidence sits under it. **One cost of the widening, recorded because it is C149's failure mode arriving through C149's fix:** a column that declares several tests is read as *any of these went red*, and this row's three property tests cover **different guards** — so a mutation of one can be caught by the witness for another and still read as `law 27 cleared`, which is exactly the looseness this pass exists to remove. The *content* is still right (these are the tests that went red under their own mutations, which is the only reason to trust the list), so the mitigation belongs in the reading and not in the field: the mutation run records **which** declared witness fired, so the test-to-guard pairing lives in the run's record rather than in column 14. Column 14 names evidence; it is not a per-guard claim, and a sweep that treats it as one is clearing a row on the evidence for a different sentence. |
| C154 **law 19's `rustWitness` names four tests, none of which carries the merge claims the row's own statement makes** — the third instance of the C152/C153 pointer class, recorded with the class because three of the six law rows with declared witnesses turned out to be this | — **the instrument** (every claim is implemented and tested; the column does not point at the tests) | **Law 19's statement is four claims**: `Blake2b256` canonical and collision-free, the `Blake2b512Random` merge **n-ary and order-sensitive**, signatures verify what they sign, Curve25519 round-trips. Its `rustWitness` names `empty_gives_a_predictable_result`, `creates_known_ecdsa_signature`, `verifies_known_signature` and `decrypts` — the seed case and three known-answer vectors. The two claims about the *merge* are carried by `merge_with_two_children`, `merge_with_many_children` and `merge_is_order_sensitive` (`crypto/src/hash/blake2b512_random.rs:548`), and no register field names any of the three. **Measured:** making every child of the merge write to the same slot (`finalize_internal(…, i * 32)` → `finalize_internal(…, 0)`), so only the last child contributes and the merge stops being n-ary, leaves all four declared witnesses green and reddens `merge_with_two_children` and `merge_with_many_children` — the crate's whole suite runs, `2 failed` and neither is declared. **The class, now that it has three instances, is the sweep's largest single result and not the one this pass set out to find.** Law 11's column contradicts its own prose (C152); law 27's is silent on three tests that put the law's number in their own names (C153); law 19's is silent on the tests for half its statement (this row). **Three of the six law rows that declare a Rust witness have a column that does not reach the evidence**, against two rows whose fixtures were genuinely degenerate (C149, C150's runtime, C151's distribution). **Why it is easy to miss and why it matters:** the column is not *false* in any of the three — every test it names exists, passes, and is about the row — so nothing a reader checks by eye goes wrong. What goes wrong is the *sweep*: `rustWitness` is what a mutation run filters on, and a column that under-describes its row makes the run report `unchanged` or `vacuous` for a law whose evidence is sitting in the same file.  **A fourth instance, found by the sweep rather than by reading the column:** law 26b's statement is "naming a child under a valid shard yields a valid one whose path **extends** its parent's", its `rustWitness` is empty, and `law26_a_child_id_nests_under_its_parent` (`casper/src/property_tests.rs:85`) has been asserting exactly that all along — it goes red when `ShardId::child` stops extending the parent's path, measured. So the class has four members across four different flavours of the same omission: a column that contradicts its prose, a column silent on same-named tests, a column silent on half a statement, and now an **empty** column beside a witness that exists. **What is owed:** one three-file pass — `spec/laws.tsv`, `spec/Rchain/Laws.lean`, the emitted `spec/LAWS.md` — widening column 14 for the rows where the column and the statement disagree, which is the same change C152 and C153 owe and the reason none of the three was done inside this sweep. **Fixed in that pass:** law 19's column 14 now names `merge_with_two_children`, `merge_with_many_children` and `merge_is_order_sensitive` beside the seed case and the three known-answer vectors, so the two merge claims in the row's own statement have evidence named under them. Relayed from this row's measurement — the mutation that made every child write to the same slot, which reddened the first two — and not re-run here; the names were checked to resolve in `crypto/src/hash/blake2b512_random.rs`. |
| C156 **law 30's corpus does not catch the parser accepting trailing input, though the row's own `falsifiable` prose names trailing input among the shapes the corpus refuses** — the guard is real and lives in a unit test no register field names | — **the instrument** (the corpus is a genuine falsifier in other dimensions, and the trailing-input guard is tested — just not there) | **The corpus is `parse` (`spec/conformance/parse.tsv`, 11 `refused` rows) behind `rholang/tests/lean_parse_corpus.rs::the_node_parser_agrees_with_the_lean_model`, which parses every row through the node's own parser and requires its verdict to equal the model's.** The row's `falsifiable` column calls the refused half "the soundness direction made falsifiable", lists `x |`, `a.b` and `new x in` among spellings "with no separator **or with trailing input**", and concludes "a parser that grew more permissive fails one". **Measured, with the corpus test confirmed to have run:** deleting the trailing-input requirement (`p.expect(Tok::Eof)?` → `let _ = p.expect(Tok::Eof);` at `rholang/src/parser.rs:634` — the exact defect AUDIT C30 was) leaves it at `1 passed; 0 failed`. **No corpus row is refused by that rule**, because every spelling the prose lists is refused earlier: `[1 2]` and its siblings by the missing separator at `parse_collection`'s `expect(Tok::RBracket)`, and `x |`, `a.b`, `new x in` by an incomplete production. None is a *complete term followed by junk*, which is the only shape the Eof check decides. **The guard exists and is verified:** `parser::tests::trailing_input_is_not_a_term` (`parser.rs:1787`, whose own list contains `"1 2"`) FAILS under the same mutation, and no register field names it. **The corpus is not weak in general — that is the control, not the claim:** making the list terminator unreachable turns `the_node_parser_agrees_with_the_lean_model` red, so the corpus does catch a more permissive list parser; what it misses is permissiveness in the one dimension this row's prose names. Fourth instance of the C152/C153/C154 class and the first whose evidence is a unit test rather than a corpus. **Closed by registering it.** The refusal is real and witnessed — `parser::tests::trailing_input_is_not_a_term` (`rholang/src/parser.rs:1787`) fails when `p.expect(Tok::Eof)?` (`:634`) is deleted, measured — so what this row records is that the *corpus* carries no row of that shape, not that the behaviour is unwatched. The corpus is emitted from `Rchain/Parse.lean` and its four halves are counted (`PARSE_CASES`), so adding a trailing-input row is a Lean edit, a re-emit and a constant: the corpus pass's work, not this sweep's. Disposition recorded in `spec/review-ledger.tsv`'s law-30 row as a scope note, the same shape as law 33's method-argument gap and law 35's match-case fold — the row's `falsifiable` prose is narrower than the code, and nothing the register claims is unsupported. |
| C157 **`tools/audit-mutate.sh` reported `green` for runs in which no test ran** — three law-30 probes were empty runs read as evidence, and they were read that way by me | — **the instrument** (mine, written earlier this session, fixed within the hour) | **`cargo test`'s positional argument is a *filter on test names*, not a target selector, so `--test lean_parse_corpus` selects nothing: the tests in that target are named `the_node_parser_agrees_with_the_lean_model` and `the_printers_output_round_trips_through_the_node`. Every target then reports `ok. 0 passed; 0 failed; N filtered out`, cargo exits 0, and the harness read that as a witness that passes with the mechanism deleted — a vacuously-green verdict, which is the finding this audit has produced four times about *other* instruments and which the tool built to find them produced about itself. **The three readings it produced:** law 30's trailing-input removal, a list terminator made permissive, and a list terminator made `unreachable!()` — all `green`, all empty. The third is what exposed it, because `unreachable!()` on a live path cannot pass and the only other explanation was that the path was never reached. **Fixed:** the harness now greps every `test result:` line and refuses when all of them read `0 passed; 0 failed`, and it prints the per-target summaries on every run so a green can be seen to have run something. **The generalisable part, and why this is registered rather than just fixed:** the harness's own checks were all about the *plant* — that `--old` matched exactly once, that the file changed, that it restored, that the crate compiled — and none was about whether the *witness* executed. An instrument that verifies everything except its subject having run is the exact shape this register keeps recording, and it took a mutation that could not possibly pass to notice. |
| C158 **an arity drift on a catalog entry that does not reply is caught by nothing** — the corpus ties the node's arity to the Lean's *arguments* for every entry that sends back, and to nothing at all for the ones that do not | — **the register's evidence** (all nine declared arities agree with the node today, measured; the agreement is held indirectly and only where a reply exists) | **Measured two ways, and the contrast is the finding.** Planting `arity: 2 → 3` on `rho:io:stdoutAck` — kind `send`, which replies — turns `every_urn_replies_in_the_shape_the_lean_catalog_says` **FAILED**: the corpus builds the call from the row's arguments (`c!({args}, *ret)`), a three-variable bind pattern cannot match it, and the reply the corpus asserts with `data.len() == 1` never arrives. Planting `arity: 1 → 2` on `rho:io:stdout` — kind `none` — leaves the same corpus **GREEN**: a non-replying urn has no reply to lose, and the corpus's only assertion on such a row is that the probe ran. **So the chain is real where it exists and is worth stating precisely:** `replyCatalog_decide` (`spec/Rchain/Protocol.lean:136`) checks each row's `callArity` against its own `args`, the corpus checks the node's arity against those same `args` by requiring the reply, and the two together tie the implementation to the declaration — for the six rows that reply. **`callArity` itself is emitted into no artifact**: `spec/conformance/protocol.tsv` carries five columns with no arity (`layer | urn | payload | mode | slots`), and a repo-wide grep for `callArity` outside `Protocol.lean` returns nothing, though that file's own prose calls it "the one check with real teeth on the table". **Why the `none` case is not a technicality:** a wrong-arity call in rholang is *silence, not an error* — law 38's content — and the failure the Lean prose cites is C22 item 2, `extraSlots` calling `write(@key, @value, ret)` with two arguments against a three-pattern receive, so "none of the three slots was ever written". For a non-replying urn there is no slot to be missing, so the deploy simply never happens and nothing anywhere says so. **Measured today:** all nine declared arities agree with the node (`stdout` 1, `stderr` 1, `stdoutAck` 2, `stderrAck` 2, `block:data` 1, `registry:lookup` 2, `registry:insertArbitrary` 2, `rev:address` 3, `qucalc:zfa` 2), so this is an evidence gap and not a live defect. **Also from the same measurement:** the node installs 35 urns with an arity against the catalog's 9, so 26 have no declaration to compare even if the comparison existed. **Registered as owed, with the reason and a measured bound on it.** The check is real work and it belongs to the corpus pass rather than here: `callArity` lives in `Rchain/Protocol.lean`'s catalog (`:62`, the rows at `:109-131`) and is emitted into nothing, so carrying it into `spec/conformance/protocol.tsv` means editing the Lean *and* `Rchain/Corpus.lean`'s emitter and re-emitting — the same three-file discipline the register's law rows follow. **What the row is not claiming is that the node is wrong today:** all nine declared arities agree with `rholang/src/system_processes.rs` (`stdout` 1, `stderr` 1, `stdoutAck` 2, `stderrAck` 2, `block:data` 1, `registry:lookup` 2, `registry:insertArbitrary` 2, `rev:address` 3, `qucalc:zfa` 2), measured — so this is an evidence gap, not a live defect, and the consequence of the gap is stated in the row above: for a *non-replying* urn a wrong arity is silence, and nothing would report it. Registered rather than left saying only what is owed, so a reader can tell the decision from the omission. **Half closed 2026-09-27, and the half that closed was a defect rather than the gap.** `ReplyRow` has carried `callArity` since the catalog was written; this row's own prose *and* the Rust consumer's header both describe the corpus line as carrying "the call's arity"; and `Rchain/Corpus.lean`'s `protocolLine` emitted five columns with no arity — so the artifact and both of its descriptions disagreed, which is the class this register is about and is why the tie was only indirect. The column is emitted now, and the consumer reads it and checks it against the arity of the probe it builds, counted **by parsing** rather than by counting commas (`rho:qucalc:zfa`'s argument is `[0, 1]`, so a comma count would be wrong, and reading the number from the corpus is what makes the two sides independent). **Falsified both ways:** setting `rho:io:stdout`'s emitted arity to 2 reddens `every_urn_replies_in_the_shape_the_lean_catalog_says` with "the catalog declares an arity of 2 and this probe calls with 1". **What is still owed, and it is the row's original point rather than the emitter's:** that check ties the declaration to the *corpus*. Tying it to the **node** for a non-replying urn needs an observable effect per urn — `rho:io:stdout`'s is the console write — and `build_runtime_pair` exposes none, so a wrong handler arity there is still silence.  **CLOSED 2026-09-27 — the premise of the paragraph above was wrong.** Tying the declaration to the node does **not** need an observable effect per urn, and `build_runtime_pair`'s not exposing one is beside the point: the node's own arity is a **table**, not a behaviour. `SystemProcesses::definitions()` returns a `Vec<Definition>` carrying `urn` and `arity` for every installed system contract — the very `Definition.arity` this module's doc names as what `callArity` is — and `every_catalog_urn_arity_matches_the_definition_the_node_installs` compares the emitted column against it for **all nine rows, the three non-replying ones included**, with no runtime, no reply and no console. `a_drifted_catalog_arity_is_reported` shows the comparison can fail, and the fix was falsified on the row this finding named: planting `2` on the `rho:io:stdout` line of the emitted corpus reddens it with "the catalog declares arity 2, the node installs 1". So the row is `done`: the corpus-side check the paragraph describes stays (it catches an emitter that transposed or dropped the column, which is what actually happened), and the node-side tie it says is owed is now the unit test above it |
| C159 **`tools/audit-type-system.sh`'s `silent` class listed every violation it found and counted none of them** — a hard class that could not fail, and the defect it was blind to was the one it was written for | — **the instrument** (found by the instrument harness's own probe, which is what the harness exists for) | **`scan_spanning` had two defects, and the `silent` class is its only caller.** (1) **The count was lost to a subshell:** `cmd | while …; do note …; done` runs the loop in a subshell, so `note`'s `hard_failures=$((hard_failures + 1))` (`audit-type-system.sh:241`) incremented a copy that died with the loop — the class printed each site and the gate exited 0. `scan` has always used `< <(…)` and is unaffected. (2) **The reported line was the stripped stream's:** the stripper drops every `#[test]` block, so counting newlines before the match gives a position that exists in no file — the same defect the `STRIP_AWK_N` comment records for `grep -n`, reintroduced one layer up. **Measured:** planting `try_into().unwrap()` in production code made the class print `graphz/src/lib.rs:839` and the summary say `OK: no hard production violations`; after the fix the same plant is reported at the same line and the gate exits 1. **Provenance, and it bounds the exposure:** `scan_spanning` was introduced *today* in `03cafa506` as the fix for C126 — which was a real blindness, a rustfmt-wrapped `try_from(…)` `/ .unwrap_or(0)` chain being unreachable by a line-by-line grep. The fix traded that for a class that matches wrapped chains and counts nothing, and it left the comment above the dispatch arms still saying `unsafe`/`silent` keep `scan`. Measured after: the tree holds no other site of the pattern, so nothing was missed in practice — but the class would not have caught one, and the audit would have read `silent OK` while the violation sat printed one line above it. **Fixed in both directions**, with `$&` captured before the line-number match — a third bug my own first fix introduced, since `$&` is reset by every subsequent match. |
| C155 **the register's own anchor check cannot see a citation move by up to eight lines** — and the comment defining what it catches claims the opposite | — **harness** | The check is `tools/audit-test-register.sh`'s "register anchors" step, written because "the line numbers a row's prose carries are written by hand and nothing read them, so five rows had drifted silently by 2026-09-24" (`:436-443`). Its stated purpose is to catch **rot** — "a line that moved now points at unrelated code, so the symbol the row is talking about is not there" — and it asserts of itself that "it still catches the rot it exists for, which is a *moved* line: there the window holds nothing the row names at all" (`:461-462`). **It does not, for a move of up to eight lines, and the pad is why.** The window is built as `sed -n "$(( from > 8 ? from - 8 : 1 )),$(( to + 8 ))p"` (`:561`): the cited range **padded by eight lines on each side**. The test is then whether *any* backticked identifier the row names appears *anywhere* in that padded window (`:562-574`, in snake, camel or as-written spelling). A construct that moves by eight lines or fewer therefore never leaves the window — the window grew by eight on each side and still contains it — so the citation is reported valid while the lines it actually names hold different code. **Measured on this tree:** `cargo fmt --all` moved lines in ten files (`casper/src/validate.rs` by +4, `rholang/src/native_state.rs` by +4 up to line 962 and +8 after it), and **fourteen of the seventeen citations into those files then pointed at content that had changed** — counted by comparing each citation in `a86fb3b65^`'s `laws.tsv` against the code on both sides of the reformat, which is the pair that isolates the reformat from every other commit's movement; the step reported `ok 171 register citation(s) resolve, and each cited window holds what the row names` on every run of that state. Fourteen needed renumbering by hand. **Thirteen were corrected in `a86fb3b65` and the fourteenth a commit later** — `rholang/src/native_state.rs:2755`, citing law 28's test, which had moved to `:2662` and which the enumeration behind that commit missed. **That miss is this row's subject applied to itself**: the gate said `ok` for the stale citation while it stood, the count that would have caught it is the one this row measures, and what eventually found it was re-deriving that count from the two committed states rather than trusting the enumeration. **Why this is a row and not a shrug:** the permissiveness below it is deliberate and argued — the substring match exists so the check does not cry wolf on a correct citation, and the pad exists because a citation usually names the function whose *body* holds the claim — but the pad is a *window* choice, not a matching choice, and it is exactly as wide as the drift class the check was written for. The register's citations are how a reader and the audit's next pass verify a row against the code; a citation eight lines off is unchecked and reads as verified, which is C126/C127's shape — an instrument that cannot see the defect it names — reached by a pad rather than by a pattern. **Falsifier:** insert eight lines above any cited construct and re-run the step: it stays `ok` while the citation now names different code, and it fails only once the construct has moved further than the pad. **Fixed** (`2b020613b`): `tools/audit-test-register.sh` builds `tight` — the cited window without the pad — and a match that appears only in the pad is reported, in all three spellings the search accepts (the row's own, the model's camelCase, the port's snake_case), so a citation is not resolved by a name the search matched and the new test then failed to recognise. **Reported rather than fatal, and the measured number is why: 77 of the register's citations match only inside the pad**, not the eight the row's own falsifier suggested. Every one is a `file:line` in `spec/laws.tsv` and its `Rchain/Laws.lean` twin, so a `fail` here would gate the whole tree on a three-file register pass that has not been done, and a build nobody can turn green is the failure mode this register keeps recording. The summary line carries the count on every run and the first eight are named — a number that must move, visible, rather than a drift hidden by the tolerance built to absorb it. |

### Refuted, and recorded so the filter is visible (4)

**A red-team pass that reports everything it finds is reporting noise.** These three were plausible,
were written up with mechanisms, and did not survive an independent attempt to refute them:

| finding | why it is not a finding |
|---|---|
| The faucet's per-address drip budget does not bound the dev-wallet drain it claims to bound (`node/src/api/web_api_impl.rs:62`) | **Already R17, and R17 names this exact residual.** The code facts are as claimed — the budget is keyed on the caller-chosen *destination* address, so a caller rotating keypairs drains ~0.3 REV/s from one client — but R17's row already closes with "per-source IP buckets remain a devnet-only refinement", i.e. the missing bound is registered. The finder read the *comment's* claim rather than the row. |
| The two data-at-name POSTs are the only resource-spending public routes with no rate limiter (`node/src/web/http.rs:345`) | **The narrow observation is true and the load-bearing claim is false.** `api_data_at_name` and its by-block-hash sibling do skip `deploy_rate_limiter` — but `api_get_blocks_by_depth`, `api_get_blocks_by_heights`, `api_get_blocks`, `api_get_block`, `api_find_deploy` and `api_deploys` are all unmetered too, so "the only such routes" is wrong, and the asymmetry the finding rests on does not exist as stated. |
| C19's row claims a conformance test is kept `#[ignore]`d; the test is a live `#[tokio::test]` (`spec/AUDIT.md`'s `C19` row) | **Factually correct, and not a defect in a guarantee.** `grep -rn '#\[ignore' --include=*.rs .` returns zero hits and `rholang/tests/system_conformance.rs:431` is `#[tokio::test]`. But the sentence sits in C19's *diagnosis* paragraph, which is explicitly superseded by C20 — so this is a stale sentence inside a withdrawn diagnosis, not a claim anyone relies on. Recorded rather than actioned. |
| The only shipped health probe cannot distinguish a stalled node from a healthy one, and there is no readiness signal (`docker/rnode/Dockerfile:35`) | **Refuted on disposition, not on mechanism.** The code facts hold: the image's only probe is `HEALTHCHECK … curl -fsS http://localhost:40403/api/status`, and `/api/status` is liveness-shaped — `WebApiImpl::status` calls the infallible `block_api.status()` (`node/src/api/web_api_impl.rs:471-475`, which returns `Ok` always) and the DTO carries peers/nodes/latest_block_number with no sync, height or peer gate (`api/dto.rs:74-98`) — so a node with an unreachable bootstrap reports `(healthy)` indefinitely. **But the absence of a readiness signal is a documented disposition rather than an unregistered defect**, which is what this table is for: it is recorded here as a known limitation, not filed as a finding. |

**Five further findings were downgraded rather than refuted** and are carried above at their corrected
severity: C134 (`medium` → `low`, on the Scala's ABI) and C129 (`high` → `medium`, on the route being
rate-limited and read-only), plus three from the `ops` lens — C141 and C142 (`high` → `medium`) and C143
(`medium` → `low`) — where every one requires the operator or the orchestrator and none is reachable
remotely.

### What this pass did not do, stated rather than implied

- **The `ops` lens returned after the first tables and is now folded in** (C141–C148 above, with the
  ledger rows that house them), so the operator-facing surface has been read: the config module, the
  startup path, `main.rs`, `model.rs`, `log.rs` and the defaults file. The other `config` roster rows
  were not read and stay `deferred`.
- **The completeness critic's verdict on coverage, which is the finding this section most wants a reader
  to keep:** this pass read **about 5% of the file roster** — 17 of 350 file rows carry a verdict and
  **not one is `cleared`**. By crate: `rspace` **0 of 57**, `shared` 0/30, `block-storage` 0/14, `sdk`
  0/10, `crypto` 1/24, `models` 2/34, `rholang` 3/32, `node` 2/52, `comm` 3/38, `casper` 6/56. **The two
  consensus-scale subsystems — RSpace and the DAG/block-application core — were not read at all**, and
  all 30 `process` rows and all 104 `config` rows were `deferred` when the tables were written. **A
  reader should take from this pass only what it can support:** seven lenses found the defects below,
  concentrated in the ingress, consensus and instrument surfaces, and **the majority of this tree is
  still unread.** A pass that reported this coverage as reassuring would be the same defect class the
  section above is about.
- **The two criticals are fixed; the other twenty-four rows are still reports** — this bullet said "no
  finding here was fixed" when the section was written, and that was true of every row then. C120's and
  C121's rows now carry what their fixes do and, where it matters, what they do not.
- **One correction to what this section claimed about the two criticals' registration.** The bullet
  above used to say both "need §6 rows because `legacy/` shares them". **Only C120 does.** The Scala tree
  has no transaction API and no gateway — `TransactionApi`, `txnApi`, `api/v1/txn` and `enable-txn-api`
  all return nothing under `legacy/` — so the cross-shard coordinator is the port's own surface, which
  `spec/API-SCHEMA.md`'s `rho:txn` row already said. C121 owes a §6 row nothing to deviate from; C120's
  is the shared defect, and §6's vault row carried the unconditional claim it refuted.
- **Nothing was run against a live multi-validator net.** C120's falsifier is a constructed
  `BlockMessage` driven through `RuntimeManager`, not a byzantine node on the devnet; C121's was
  designed as a devnet probe and was not executed. **Its fix is pinned by a real node instead** — one
  process serving two shards, driven over HTTP — which is stronger than a unit double and still not a
  *network*: nothing here shows a byzantine proposer being refused by a peer that is actually bonded to
  it. That remains the shape neither critical's evidence reaches.
  **Run 2026-09-28 on a live two-validator devnet (`tools/devnet.sh up --validators 2 --fresh`),
  and it reaches half of the shape.** The validators are **bonded** — each reports `peers = 1,
  nodes = 1` over `/api/status` — and against that network the unauthenticated transaction route is
  **absent from the public surface**: `POST /api/v1/txn` answers `404` on both nodes' public
  listeners. That the two routers genuinely differ is shown by a control pair on the *same running
  nodes* rather than asserted: `/api/v1/propose` answers `400` on the **admin** port and `404` on the
  public one, while `/api/status` answers on the public port and `404`s on the admin one. So C121's
  boundary is now demonstrated against a network, not against a double.
  **What it does not reach, stated rather than implied.** With `api-server.enable-txn-api = true` the
  route still answers `404`, because `gateway_or_not_found` requires a *gateway* as well and a
  single-shard node has none — so the probe shows "this node does not serve transactions", not a live
  refusal of an *actual* spend, which needs a gateway-shaped node. And the admin bind's loopback
  default is **not** observable this way: the devnet image publishes the admin port, so it is
  host-reachable with and without `--dev-mode`. That default stays pinned where it was, by
  `admin_bind_host`'s own unit test (C112/C132). **C120 is untouched by this run and needs a different
  instrument**: driving it live means a node that *proposes* a block whose `ProcessedDeploy` names a
  victim's `deployer` — a deliberately byzantine binary, built and bonded to an honest peer. That is a
  patched image rather than a probe, and it remains the shape neither critical's evidence reaches.
- **The instrument findings (C124, C125, C126, C128, C129) were each demonstrated with a probe in an
  isolated worktree**, with a control pair where one was available — and every probe was removed. The
  probes are reproduced in the rows because a gate defect is only credible as a shown gate defect.
- **What the ledger changes:** twenty-six findings are housed against roster rows that were `deferred`,
  so the denominator moves for the first time since it was written. `unhousedCeiling` is unchanged
  at 107; **this pass houses its own numbers and does not lower the ceiling**, because the historical
  backlog it counts is a separate question.

### Added after the check-off was built (C163)

- **C163 — C129's fix broke a test the same commit never ran.** `node/tests/node_api.rs:146` asserts
  that `POST /api/v1/explore-deploy` answers `200` on a freshly booted node. C129 made the no-hash
  default read `dag.last_finalized_block_unsafe()` (`casper/src/api/block_api_impl.rs:111-113`), and a
  node that has produced only its genesis has **no finalized fringe**, so the handler cannot answer a
  block and the request is a `400`. C129's own row says this in as many words — "on a node that has
  produced only its genesis the DAG has no finalized fringe, so the no-hash default now answers a 400"
  — and it moved the assertion in `node/tests/api_surface.rs` to match. It did not run
  `node/tests/node_api.rs`: the commit's verification line reads `--lib` (256), `rchain-lib --lib`
  (207) and `--test api_surface`, and that file is not among them. Found by running
  `cargo test --workspace` on the check-off's own branch, where it is the only failure.
  **This is not a code defect** — C129 decided the behaviour deliberately, and a 400 for a request
  that names no block on a node with no agreed block is the honest answer. **What was owed was the
  test.** It took the second of the two routes: both calls now go to
  `explore-deploy-by-block-hash` with the genesis block's hash, which the test already had at
  `node_api.rs:111`, so every assertion it was written for — the reply channel, `replySource`, the
  reference document's value shape, and the first-`new`-bound-name convention — still runs. Asserting
  the refusal instead was the cheaper route and was rejected: it would have left the test green while
  no longer testing anything it exists for, which is the class of edit this register is about. The
  fix is pinned by the test itself (`genesis_boot_exposes_block_over_http`, green) and the whole
  suite is green behind it — the first time in this tree's recent history.

## 22. The coverage pass: reading the T1 roster

`spec/AUDIT.md`'s check-off carries two halves, and this is the second: the **89 T1 rows** — the
modules that can fork the chain or lose funds — of which 20 carried `deferred` when the pass opened.
The check-off's front page has read `Coverage 20 of 89 T1 modules unread` since it was built, and
until this pass `tools/todo.sh` could not tick one: it resolved ids against `spec/findings.tsv`, so
the larger half of the check-off was listed by a tool that refused to close it. Two verbs fixed that
(`read` and `found`), and every row below is recorded through them.

**The base rate is why this is worth doing.** Eight T1 files had been read before this pass and **all
eight produced a finding** — not one had ever been read and cleared. §21 said the same about the
roster as a whole in its own closing words: "17 of 350 file rows carry a verdict and **not one is
`cleared`**" — *not one is `cleared`*, written by the eight findings.

Each row below names the symbol read and what the read produced. A `cleared` row is a module read in
full against its oracle with nothing found; a `finding` row names the C-number it became.

### C164 — a truncated `current-root` panicked the node on the state-read path

- **C164 — `RootsStore::current_root` built a hash from whatever the store returned, and the
  constructor asserts its length.** `rspace/src/history/roots_store.rs`'s `current_root` read the
  `current-root` key and handed the value straight to `Blake2b256Hash::from_byte_array`, which is
  `assert_eq!(bytes.len(), LENGTH, "Expected 32 but got N")` (`crypto/src/hash/blake2b256_hash.rs:49-56`).
  A value that is not 32 bytes therefore **panicked** — on the path every state read goes through —
  rather than reporting what was wrong. **Found by reading the module, and the tell is one file over:**
  `rspace/src/history/codecs.rs` performs the identical conversion and *does* guard it, refusing a
  wrong length by name, with a test whose own doc says why — "a corrupted or truncated store value must
  be an error naming the length, not a hash built from whatever bytes arrived — a padded read would
  silently address the wrong node in the radix tree." The discipline existed, was pinned, and was not
  applied where the same bytes are read. **Trigger:** a truncated or corrupted `current-root` value —
  a partial write, a hand-edited store, a restore from a bad backup. The node's own writes are always
  32 bytes, so this is the corruption case and not a remote one. **Fixed** in the same shape as the
  codec's guard: `current_root` now refuses a wrong length, naming both the length it found and the
  32 it wanted. **Falsified both ways, and the failure is the defect:** with the guard removed,
  `a_truncated_current_root_is_refused_rather_than_panicking` dies inside `from_byte_array` with
  `Expected 32 but got 31` — the panic this row is about, printed by the test that now prevents it.
  Its second arm is the control: a well-formed value still reads, so the guard is a length check and
  not a read that refuses everything.

### C165 — the private key was written *before* its file was narrowed

- **C165 — `write_with_mode` wrote the secret, and only then chmodded the file.** `crypto/src/util/key_util.rs`'s
  `write_with_mode` opened with `OpenOptions::mode(0o600)`, wrote, and called `fs::set_permissions`
  **after** the write — with a comment saying that order "so the content is never briefly readable
  under the wider mode". It is exactly the reverse. `OpenOptions::mode` applies **only when the file
  is created**, so a pre-existing `0644` `rnode.key` (copied in by hand, or written by this code
  before R6 narrowed the mode) was **truncated and then written while still world-readable**, and
  narrowed only afterwards. **This is C8's fix, in the wrong place** — C8 found that the
  `set_permissions` call was *missing* and added it; the ordering was never examined, and the comment
  asserts the opposite of what the code does, which is the class of claim this register exists to
  catch. **Fixed by inverting the order**, and the open-and-narrow step is split into
  `create_owner_only` so the order is *testable*: a test can call it on a wide file and assert the
  mode is already narrow with nothing written. **Falsified both ways, and the falsification is the
  finding:** with the order put back, exactly one test reddens —
  `the_mode_is_narrowed_before_anything_is_written` — **and `the_private_key_file_is_owner_only`,
  C8's own test, still passes.** That is the whole point: C8's test asserts the mode the file *ends*
  at, and both orders satisfy it, so the instrument could not see the defect it was written for. Its
  comment said so too.

### The miss, recorded because the pass that reads must be read

**One of this pass's twenty reads is wrong, and another session caught it the same day.** This pass
read `casper/src/txn_coordinator.rs` and returned **"Nothing found"** — the ledger row it wrote said
so in those words. The other session read the same file and filed **C166**: `run_2pc` anchored every
phase deploy at block 0, because it called the 0-hardcoded `run_phase` wrapper rather than
`run_phase_at`, so on any chain past `DEPLOY_LIFESPAN = 50` every prepare and every commit was born
expired — the participant never saw it, nothing reported an error, and the transaction silently did
not happen.

**And the warning was in the text this pass read.** `run_phase`'s whole body is
`self.run_phase_at(…, 0).await`, and the doc comment on `run_phase_at` — six lines below, in the same
screen — reads: "`valid_after_block_number` must be the *target shard's* current height: a deploy
anchored at 0 is born expired once that chain is more than `DEPLOY_LIFESPAN` blocks past genesis, and
the participant would never see the phase at all." This pass quoted that sentence back in its own
note, while reporting that it had found nothing, and did not connect it to the `0` two lines above it.
There was even a test named `run_phase_anchors_at_zero` pinning the defect as intended, which this
pass read as pinning behaviour rather than as pinning the bug.

**Four things this pass did right do not excuse it.** It verified the module's 2PC phase-two loop,
`vote_from_reply`'s three-way reading, and the `?`-on-prepare recovery path — all still correct. It
established that the module is a test harness rather than the shipped coordinator, which is true and
is *why* the defect was low-consequence rather than absent: `GatewayTxn` has the durable ledger and
does anchor correctly. And it is the same class **three times over in one section** — this pass's own
§22 records two other reads whose "finding" was already registered and already fixed, and its whole
argument is that reading finds what the register missed.

**What the miss says is not that reading is worthless but that a clean read is a claim like any
other.** This pass's other nineteen rows stand on the same footing as this one did: a person read the
file and said nothing was there. One of them was wrong, it was wrong in a way the file itself
warned about in the next paragraph, and the only reason it was caught is that a second reader
existed. The pass's own doctrine applies to itself: **an independent read is evidence and a
self-reported clean is not**, which is why §21's method ran seven lenses and handed every finding to
a second agent instructed to refute it. This pass had one reader per file and no refuter, and the
one file where that mattered is this one.

### The remainder: what this pass did not read, and what is owed for it

**This pass closed the T1 roster and left everything under it exactly where it was, and #94 asks for
that remainder to be written down rather than implied.** The count is now in `spec/AUDIT.md`'s own
frame, above the generated check-off, so a reader meets it before the two headline counts rather than
after. What follows is this pass's account of it.

**The denominator, measured 2026-09-28.** `spec/review-ledger.tsv` holds **626 rows and 507
`deferred`**: `file` 306 of 352 (T1 28 with none deferred, T2 21 with 18, T3 303 with 288), `config`
104 of 105, `ingress` 40 of 45, `process` 30 of 30, `tool` 20 of 27, `class` 6 of 6, `roster` 1 of 1,
`law` 0 of 60. Two `file` rows moved out of `deferred` this same day — the ZFA prototypes deleted
under #38 — which is the difference between that 306 and the 308 an earlier count carried. **Four
crates carry no verdict at all**: `block-storage` (14), `sdk` (10), `qucalc` (2), `graphz` (1). RSpace
carries 8 verdicts of 57 — **49 rows still unread in the crate that holds the merge**.

**The two numbers that look contradictory are not, and the resolution narrows the claim twice.** This
pass's output is that every **T1 row** now carries a verdict, beside 306 deferred file rows.
`tools/audit-status.sh` counts `deferred` rows whose *tier* is T1, and the T1 tier is **28 files, 60
law-register rows and one workflow** — so the headline is a statement about a tier that happens to be
mostly law rows, and, within the file roster, about 28 of 352 rows. The line §21 wrote — "17 of 350
file rows carry a verdict and not one is `cleared`" — was about that same file roster, and the
reconciliation belongs on the register's front page precisely because a reader who takes the headline
as a statement about the tree has taken it one roster too far.

**The reading order this pass hands forward: RSpace first, then `block-storage/src/dag/*`.** RSpace
is the merge and the trie — all six `rspace/src/merger/*` rows are deferred, and ten of fifteen under
`rspace/src/history/*` — and the merge is where laws 9, 19, 20 and 21 are stated and where #83's
duplicate-action panic is assembled. `block-storage/src/dag/*` is next, with all eight rows deferred
(`finalizer.rs`, `representation.rs`, `message_state.rs` among them). **This pass's own miss is why the
order carries a rule with it**: the one file this pass read clean and got wrong was caught only
because a second reader existed, so every row in that order wants a second reader or a probe before a
`cleared` verdict is written. A pass that read the consensus core the way this one read
`txn_coordinator.rs` would be the §21 coverage defect again, with a better denominator.

**The decisions on the two kinds that are not reads, because #94 asks for a decision and not a plan to
read 134 rows.** A `process` row is an installed URN: its reachable property is the wire protocol, the
corpus at `spec/conformance/protocol.tsv` holds nine of the thirty, and C158's test ties a row's arity
to the node's own `Definition.arity`. So the remaining twenty-one are closed by **extending the
corpus**, falsified per row, rather than by twenty-one adversarial reads — and the bodies they name
live in `rholang/src/system_processes.rs`, which this pass's predecessor read as a T1 file and put a
finding on. A `config` row is a declaration in the operator-facing surface — an `#[arg(...)]` in
`node/src/configuration/commandline/options.rs`, or a `defaults.conf` key — except that the roster
records **names**, and three of its rows admit it: `depth`, `content` and `type` each carry an `@2`
twin, and each name is declared twice in `options.rs`. The seeder that produced the roster went with
the emitter on 2026-09-27. So the config rows are closed in two mechanical halves, neither a read: a
**resolving pass** of 105 names to declarations, and then the adversarial question C143 already
answered for nine keys — *enforced, or parsed and ignored?* — with that row as the worked example.

**What the note deliberately does not do.** It does not lower `unhousedCeiling` or the per-kind
`ceiling` lines. §21 already established the rule — "this pass houses its own numbers and does not
lower the ceiling, because the historical backlog it counts is a separate question" — and a pass that
shrank the ceiling while the denominator grew would be reporting coverage as reassurance, which is the
defect class §21's completeness section is about.

## 23. The attestation guard: #70's two defects, one layer below where the issue looks

[#70] is titled *"Resilience: finality needs quorum stake, not live nodes — an absent validator blocks
it, and attesting requires proposing"*, and it locates the problem in the weight layer: an absent
validator keeps full weight, and only a proposer may attest. A design pass over the tree on 2026-09-29
found the first symptom **one layer lower** — in the proposer's attestation guard, which no comment on
the issue names — and a second defect beside it. Both were red first, each against its own mutation,
and both are the *opposite* of what the guard's shipped comment and the docs said.

[#70]: https://github.com/rchain-community/rchain-rust/issues/70

### C170 — the guard counted a validator that had *ever* spoken, and could not suppress in the case it exists for

`suppress_attestation` (`casper/src/blocks/proposer/proposer.rs`) decides whether a proposer folds an
empty attestation into the block it is building. It had two independent defects, and neither is visible
from the issue's own arithmetic:

- **F4 — "moving" meant "has ever spoken".** The stake counted toward the quorum was summed over the
  senders of `pre_state.justifications`, which is `latest_msgs` — a map that **keeps a silent sender's
  last message indefinitely**. Three equal validators with one dead therefore summed 200 (the live
  peer's message *plus* the dead one's stale one), and adding `own_stake`'s 100 made it 300 of 300: a
  supermajority that does not exist. The guard was reading liveness of *the past*.
- **F3 — the supermajority clause could not suppress in the case it exists for.** Suppression was
  `nothing_to_finalize || !(new_state_transition || quorum_reachable)` with
  `new_state_transition = parents.iter().any(has_deploys)`. A deploy-bearing parent is the *ordinary*
  case on a chain with traffic, and it was OR-ed **inside** the quorum test, so it short-circuited that
  test to `false`; suppression then collapsed to `nothing_to_finalize`, which was `false` too because
  the deploy-bearing block was unfinalized. **A node that had lost over a third of its stake attested
  at every height** — the 276-blocks-in-a-minute storm recorded on the issue — and the comment at
  `attest_warranted` (`node/src/runtime/node_runtime.rs:2590`) stating that such a chain "does not
  spin" was not what the code did. The comment is corrected in the same change rather than left
  claiming more than the code does: a stale contract comment is F3's own defect class.

**The fix is two named predicates rather than inline arithmetic.** `moving_attestation_stake`
(`:1044`) drops any sender whose latest message is further than `ATTESTATION_WINDOW` (`:1024`, 5
heights) behind the tip, the tip being the newest height among the pre-state justifications — so the
recency check needs no new state leaf, no DAG scan and no genesis field, and a returning validator's
justification height jumps to the tip and its stake re-enters. `attestation_suppressed` (`:1093`) takes
`(nothing_to_finalize, new_state_transition, quorum_reachable, cadence_due)`: suppress on nothing to
finalize; attest immediately when the quorum is reachable; and while it is **un**reachable, attest only
if a state transition exists *and* this node has itself been quiet past the window. A deploy-bearing
parent still licenses an attestation; it no longer licenses an unbounded rate of them. The cadence is
what keeps the repair from being a blanket suppress, which would trap liveness: no messages → no tip
movement → no fresh justifications → a peer that came back is never seen. `cadence_due` (`:1064`) reads
our own latest message's height against the tip, so the pace bound needs no counter and no persistence.

**Falsified both ways, per defect.** Restoring the recency-free body to `moving_attestation_stake`
makes `a_silent_validators_stale_message_does_not_carry_the_quorum` fail with `left: 200, right: 100` —
the dead validator's stake, summed; restoring the old `nothing_to_finalize || !(new_state_transition ||
quorum_reachable)` makes `an_unreachable_supermajority_suppresses_even_with_a_deploy_bearing_parent`
fail on its own assertion. **The other five tests in the module stay green under both mutations**, so
each falsifier pins its own defect and neither is satisfied by the other's removal. With the fix in
place, `cargo test -p rchain-casper --lib` is 321 passed.

**What this does not fix, and it is why #70 stays open.** The storm in the case where the quorum *is*
reachable is untouched: nothing here bounds the rate at which an all-live net attests, because C170's
own repair deliberately keeps the reachable case immediate. The 2026-09-29 measurement below reproduced
that storm with autopropose **off**, so the fuel is the attestation tap and not the dummy-deploy
injector. And the weight layer the issue names — one liveness predicate shared by the proposer and the
finalizer — is a separate increment, not this one.

### C171 — an all-live net attesting on every remote block runs a block storm, and the pace bound is owed

Measured 2026-09-29, three validators at 100/100/50, `--no-autopropose --propose-on-deploy`,
`--epoch-length 10`: **four deploys produced 126 blocks in about three minutes** (~2/s) while finality
stayed at 8, and the height ran to 126. Attestation is the fuel — each attestation is itself a remote
block for the peers, which attest in turn — so it is the tap, not the dummy-`Nil` injector, that makes
the chain grow. The shape was already recorded twice on a two-validator net (23 blocks from six
deploys) and on the issue itself (276 blocks in about a minute, finalised only to 11).

`suppress_attestation` cannot bound this: the quorum *is* reachable, so its suppression clause is
deliberately inert, C170's repair included. The bound has to be on **our own quiet** — this node's
latest message at least `k` heights behind the tip — which is where `attest_warranted`
(`node/src/runtime/node_runtime.rs:2590`) already computes a per-remote-height rule that is real but
insufficient, because heights keep advancing on a chain that cannot finalise. Recorded as `todo`
rather than folded into C170 because it is open, and the `owes` cell names the falsifier that would
close it.

## 24. The DAG index and the store it indexes: the same measurement's other failure

The same 2026-09-29 run produced a second, independent finding one layer down, and it is the one that
keeps a joiner from ever catching up: the validator that stalled at the first epoch boundary never
recovered, because a block can be **in the DAG index and not in the store the index is built from**.

### C172 — `BlockMetadataStore::add` writes the index before the store, and two readers ask different sides

`add` (`casper/src/block_metadata_store.rs:41-52`) updates the in-memory `DagState` **first** and
writes the persisted store **second**, with an `await` between them. Two readers want the same fact
and ask different questions: `has_all_deps` (`casper/src/blocks/block_receiver.rs:455`) and
`not_validated` (`:245-255`) ask the **index** (`DagRepresentation::contains` → `dag_set`,
`block-storage/src/dag/representation.rs:73-75`), while the whole of `block_summary` —
`get_parents_metadata` (`casper/src/proto_util.rs:26-36`), `block_number` and `sequence_number`
(`casper/src/validate.rs:220-236`) — asks the **store** (`dag.lookup` →
`block_metadata_store.get`, `casper/src/dag.rs:419-421`). Inside that window a child's dependencies
look satisfied, the child is queued, and validation then cannot resolve its parent: `missing
justification …` → `block summary failed: …` (`casper/src/multi_parent_casper.rs:338-342`) →
`ValidateError::Internal`.

**`Internal` is terminal in a way `ValidationFailed` is not**: it is logged and `continue`d
(`casper/src/blocks/block_processor.rs:124-130`), so the block never reaches `validated_tx`, so
`BlockReceiver` never sees it *finish*, so every block waiting on it stays in `state` forever. That is
the observed cascade (three failures, each naming the previous block) and why only a restart
recovers: `BlockMetadataStore::create` rebuilds the index from the store. The retriever's log
contradicts the processor's for the same reason — `ack_received` fires on the **block-store** write
(`block_receiver.rs:452`), before validation, and removes the hash from the map `request_all`
re-requests from (`block_retriever.rs:252-263`).

**`dag.rs:260-262` makes this order the whole of a previous fix** (AUDIT F-5): *"Compute and store the
fringe data **before** the block's own metadata… The order is the whole of this fix"* — chose so a torn
write leaves orphan *data* rather than a pointer to data that is not there. `add` does the reverse for
the pointer and the contents it points at. Same class, one layer down, and this is C164's shape again:
the discipline existed, was pinned, and was not applied where the same fact is read.

**What the run establishes, and what it does not.** The observation (the log sequence, the terminal
cascade, the restart recovering) is evidence; the *mechanism* is read out of the tree and is
consistent with it, and the reading found a **deterministic** instance of it as well — see the fix
below. Whether the concurrency window is what the run hit is still not reproduced: it is one `await`
wide and needs a validation running inside it, and which of the concurrent validators (the processor's
own spawned batch, the proposer's parent validation at `proposer.rs:385`, the LFS syncer) lands in it
would take a targeted repro. That is recorded as owed rather than implied by the fix.

**Fixed in the order, with the gate moved ahead of it.** `add` now writes the **store** first, then the
index, and checks the transition **before either** — `validate_dag_state_after`
(`block-storage/src/dag/metadata_store.rs`), the contiguity predicate evaluated on the state a block
*would* produce. That second half is not decoration: the old code extended the live index in place
*before* running `validate_dag_state`, so a **refused** add returned `Err` with the index already
extended — deterministically. And the refusal is reachable: a validation-failed block is not counted
into `height_map`, so a block above one leaves a gap and the check refuses it. The old body therefore
had the divergence on a path with no concurrency in it at all, which is why this is `done` while the
window above stays owed.

Both halves are pinned by tests that were **red against the old body** —
`a_refused_height_gap_is_not_left_in_the_index` and `a_failed_store_write_is_not_left_in_the_index`,
each asserting `contains` false where it answered true — and the new gate's agreement with the check it
replaces is **pinned rather than trusted** (`validate_after_agrees_with_validating_the_extended_state`),
the same rule `recreate_in_memory_state`'s in-place rebuild follows. That equivalence test earned its
keep on its first run: the arithmetic first treated a second block at an existing height as a new
*key*, which the mutating form does not. The remaining window is closed by construction rather than by
a lock — the store-first order can only ever leave the index *under*-claiming, which is the answer both
of its callers want ("not in the DAG yet"). Filed and closed as [#103].

[#103]: https://github.com/rchain-community/rchain-rust/issues/103

## 25. One attributable failure estranges a node from the chain (#105)

Filed from the live testnet while attempting #70's recovery measurement, and **checked in the tree rather
than taken on trust**: the observed symptom — a node that fails one block, then refuses every block above
it, permanently — is two rules, and both are the oracle's.

### C173 — a failed justification cannot raise a child's height *and* cannot be justified at all, so recording one failure is terminal

- **The height rule.** `block_number` (`casper/src/validate.rs:221-241`) skips failed justifications when
  computing the maximum, so a block's number must be `(the highest *non-failed* parent's height) + 1`.
  Mark one justification failed and every later block, whose number counts that block, is refused with
  `InvalidBlockNumber` — #105's cascade, log and all.
- **The neglect rule, which fires first for a bonded sender.** `neglected_invalid_block`
  (`casper/src/validate.rs:321-340`) refuses any block justifying a failed justification whose sender is
  bonded (`b.bonds[sender] > 0`). A validator's next block always justifies its previous one, so the node
  that recorded the failure can never accept another block from that validator — nor from any peer
  building on it.

**Both are faithful, which is why this is a decision and not a bug fix.** The skip is the oracle's own
`if (!m.validationFailed)` — the port's own doc cites it as staying while registering the *descent bound*
beside it as the deliberate divergence — and `neglectedInvalidBlock` is a straight port. What no register
carried is the **consequence**: the failure is attributable (`mark_failed_attributable`,
`casper/src/multi_parent_casper.rs:433`, reached by every `ValidateError::ValidationFailed` — a rejected
status, a state-hash disagreement, a structural fault), so the block is recorded failed *and* `slashable`.
That turns the one-block-per-node divergence #70's 2026-09-24 comment describes — each node attributing
the failure to the other and proposing to slash it — into a **deterministic** outcome rather than a
coincidence: a node that attributes a failure to a bonded peer is estranged from that peer's chain for
good, and offers the slash as evidence against it.

**What it explains, and what it does not.** It explains #105's second measurement exactly — B wedged at
45 while A advanced to 50, every later rejection `InvalidBlockNumber`, and finality frozen with A holding
91 % of the pool, which #70's full-partition filter predicts (a bonded validator with no message caps the
fringe whatever the survivors hold). It does **not** explain why B failed the block in the first place:
the state-hash divergence is undetermined, and the hypothesis #105 proposes — the randomised-selection
seed is drawn a boundary ahead from the **finalised** fringe's state, so two nodes whose fringes differ
while finality lags derive different state — is checkable from the two nodes' last-finalised block hashes
at the failing height, without a re-run.

**Recorded `todo`; the fix is a fork decision and that is the `owes` cell.** Either count failed
justifications in the height maximum — the H1b descent bound still refuses a failed parent at or above the
child, so what is given up is only the protection against an *unverified* height raising a child's claim
— or give a stranded node an explicit path back (a revalidation of the failed metadata, or a bounded
re-fetch), and then the falsifier is a node that has marked one block failed and must still accept the
next block above it. Independent of C172's fix, which is `ValidateError::Internal` → dropped where this is
`ValidationFailed` → recorded, and does not touch this path.

[#105]: https://github.com/rchain-community/rchain-rust/issues/105
## 26. The fringe gate asked two questions of one map: a silent validator capped finality at any stake share (#70)

#70's measurement asked whether the survivors resume finality after a validator is killed. They did not —
finality stayed at 8 while the height ran to 126 — and the reason is not the quorum arithmetic the issue
is about.

### C174 — `calculate_fringe` required a message from **every** bonded validator, so one that stopped producing capped the fringe whatever the survivors held

`calculate_fringe` asked two questions of one map, and the map it was given was the whole bonded one:

- **who must have seen a candidate** — the full-partition filter, whose `bonded_senders` are the map's
  **keys**;
- **what the quorum is measured against** — `total_stake`, the map's **values**.

A validator that produces no message can never be "seen by every seer", so with the whole map as the
partition the filter is unsatisfiable and the fringe cannot advance — no matter how much stake the
survivors hold. Measured on a three-validator devnet at `100/100/50`: the two survivors at **80 %** of
the pool did not resume finality after the third was stopped. The same shape is visible in #105's second
run, where the survivor held 91 % and finality was stuck: A was not missing *stake*, it was missing a
*message*.

**The fix separates the two questions**, and the separation is the whole design:

- the **partition** ranges over the bonded validators whose latest message is within
  `LIVENESS_WINDOW` heights of the tip — `block-storage/src/dag/liveness.rs` — so a validator that has
  stopped is not required to have seen the candidate, and the survivors can complete a partition;
- the **quorum stays the whole bonded map**, so a minority still cannot finalise.

**The plan this replaces had the shape wrong, and the arithmetic is why.** It proposed handing the live
set to `calculate_finalization` as `bonds_map`, on the argument that "a silent validator leaves numerator
*and* denominator together". That is exactly the problem: with the live set as the denominator, the gate
is `3·F > 2·L` with `F ≤ L` over the live set, which holds for **any** self-consistent subset. Under a
network partition each side's live set is its own validators, so each side finalises its own view — two
conflicting finalisations, safety gone. The issue's own title is the requirement — *finality needs quorum
stake, not live nodes* — and it is the denominator that keeps it. So the partition shrinks and the
denominator does not: `calculate_fringe(support_map, partition_bonds, quorum_bonds)`, with
`Rchain.calculateFringe` taking the two maps and
`calculateFringeOneMap_eq_calculateFringe_self` recording that the one-map call is their identity (which
is why the boundary theorems carry over unchanged rather than being re-earned).

**Determinism and reversibility need no state.** The predicate is derived from the bonds and the heights
the DAG already records for the block's justifications — no new leaf, no genesis field, no scan — so every
node validating the same block derives the same partition; and a returning validator's message is at the
tip, so its stake is back on the next block. The two consumers that must not disagree share one function:
the block creator's fringe (`create_message`) and the validator's (`multi_parent_casper`) both go through
`liveness::calculate_finalization`, because the creator writes the fringe into the block and a validator
that derived a different one would refuse it.

**Falsified both ways.** `a_silent_bonded_validator_does_not_cap_the_fringe` (`casper/tests/finalization.rs`)
drives a four-bond fixture in which three validators speak: through the liveness rule the three-way fork
finalises, and through the raw gate with one map it does not. Deleting the rule reddens the first arm and
leaves the second, which is the pre-2026-09-29 behaviour. `the_quorum_is_measured_against_the_whole_bonded_map_not_the_live_one`
pins the denominator half as arithmetic — the same numbers are a quorum over the live set and are not one
over the bonded map — and the predicate itself is pinned by the `liveness` unit tests (window edge,
never-spoke, ahead-of-tip, return).

**What it does not fix, said plainly.** The all-live attestation storm is untouched (AUDIT C171, still
`todo`: the pace bound belongs on the guard or the tap, and the guard-side shape needs a measurement,
because a cadence that suppresses *every* round traps liveness). And a net that loses more than a third
of its stake **permanently** still cannot finalise — the honest fix there is an inactivity leak, which is
a state change (burning a silent validator's stake) and belongs with the shard-configuration and
validator-lifecycle work (#24, #39), not with a recency window.

## 27. The memory ceiling is glibc's heap, and the audit that had to break its own instruments to say so (#117)

#117 recorded that a node under fork load reaches its cgroup ceiling and is OOM-killed on a chain of
fifteen blocks. This pass is an adversarial audit of that claim and of the work done against it: eight
independent agents (four deriving blind, four attacking one seeded conclusion each), a synthesis across
three revisions, and three rounds of mechanical gates that failed twice and returned real defects both
times. Its full report is `spec/audit/evidence/n117-audit.md`; the frozen measurement protocol and its
three amendments are `spec/audit/evidence/n117-preregistration.md`; what the measurement returned is
`spec/audit/evidence/n117-stage-a-results.md`.

**Verdict: the symptom is confirmed, the fix is not, and the mechanism is now half named.** The symptom
by direct measurement (`run5`, `trim` and the three Stage A runs: anon to the ceiling, `file` flat, a
lone validator flat, 2–3 nodes OOM-killed per run). The fix is not: at a 4 GiB ceiling on the tip, after
both merge-path commits, two of three nodes still die — and the arm that died carried `MALLOC_ARENA_MAX=2`
in its environment, so it was the arm *most favourable* to the fix. The mechanism is half named because
the measurement attributed the memory but not its composition:

- **The ceiling is glibc's malloc heap.** Over 760 samples at `anon` > 1 GiB, glibc's own accounting
  (`uordblks + fordblks + hblkhd`, read by an `LD_PRELOAD` shim — `spec/audit/evidence/mallinfo-shim.c`)
  explains the cgroup's anonymous memory at **92.0–163.9 %, mean 101.0 %**. Nothing in the tree could say
  this before: a Rust heap profiler sees what passed through the global allocator and never what glibc
  kept, and `/proc` reports resident pages without saying whether an allocator call still owns them.
- **The audit's own rule could not have reached it.** The pre-registered table classifies on `R/A`, a
  `smaps`-derived proxy for the arena class. Across the same runs, on the same process, `R/A` spans
  **0.000–0.983**: 280 of 760 samples fall in the "arenas refuted" band, 377 in a band Amendment 2 had
  to add, 103 in "accepted". The verdict is decided by which instant is sampled. That is recorded rather
  than repaired, because it is the measurement's own finding about the instrument.
- **H6's accepted Θ(N²) ancestry residency is exonerated by direct measurement.** The node's own DAG
  gauges, read for the first time: at the ceiling, `logical_bytes` **46–125 MB** and `seen_entries`
  **749–2584** against a 4 GiB anonymous footprint. The register's accepted residual is real and is a
  hundredth of this defect — and it is now measured rather than argued from the N² term.
- **Left open, and named:** the composition of the *in-use* half. glibc reports 2–3.5 GiB `uordblks` at
  the ceiling while the only profile of this shape puts the live *Rust* heap at ~95 MiB — but that
  profile stopped below the ceiling, and no instrument separates Rust-side growth from C-side allocation
  from `lmdb-rkv-sys`. The subtraction needs a profile at the ceiling; the dump needs a clean exit a node
  at its ceiling does not get.

### C175 — the other half of the ingress→validate→process pipeline is still unbounded

**The check-off is R15's class, one queue over.** R15 closed "unbounded block-validation pipeline" by
bounding the processor-input channel at `MAX_PENDING_BLOCKS` with backpressure
(`node/src/runtime/node_runtime.rs:764-781`). The **validated-blocks** half of the same pipeline was not
touched: `mpsc::unbounded_channel()` at `node_runtime.rs:733` carries full `BlockMessage`s, and a second
unbounded tap channel sits at `:2557` inside `tap_validated_blocks` — which is production code; the
similarly-shaped `:2090` is inside `#[cfg(test)]` and is not a defect. `unbounded_channel::<BlockHash>()`
also appears at `casper/src/engine/lfs_block_requester.rs:226` and
`casper/src/blocks/block_receiver.rs:538`, so the count is **two in this pipeline, four in the tree**.

This is the shape the register itself records as this port's most-repeated defect — a set with a line
shared and a sibling missed — and it is the one candidate mechanism this audit could not rule out, for a
plain reason: its depth is measured by nothing. `owes`: bound it as R15 bounded its sibling, **and add a
depth gauge**, because a bound whose depth is unobservable is the same defect one level up.

### C176 — a measurement's artifacts cannot say what they measured

Three defects with one shape, all found by auditing this session's own evidence base and all of them
correcting the *record* rather than the node:

- **The arms recorded no configuration.** Six arm files in `target/n105/` share one schema with no
  configuration column; which env var produced each is carried by the *filename* only, and one of them
  (`arena-control.tsv`) contains two different runs appended, another (`trim.tsv`) three plus a restart.
  So no cross-arm comparison in that evidence base is sound, and the session's published arm comparisons
  must be read as unsupported rather than merely imprecise.
- **A killed node's peak reads 0.** A container's cgroup is removed when it exits, so a peak read through
  `/proc/<pid>/cgroup` returns nothing for exactly the nodes a death measurement exists to observe —
  `acceptance.log:746-747` records `peak=0 MiB` for both nodes that died, beside a comment claiming the
  counter "cannot miss a spike". The fix is to resolve the cgroup from the **container id** and read the
  peak while the node lives; the Stage A sampler does both.
- **A tracked page states a figure its own artifact refutes.** `docs/src/node/validator-requirements.md:120-121`
  says "29 anonymous regions of exactly 64 MiB — `HEAP_MAX_SIZE`, one per worker thread — fully resident".
  The snapshot it cites holds **29 regions of exactly 32 MiB** (the worker stacks, `thread_stack_size(32 MiB)`,
  with **5.7 MiB** of RSS between them) and **11 regions of exactly 64 MiB** which *are* the arena heaps.
  The count came from one size class and the size from another, and "fully resident" is the inverse of
  the truth for the class it names. `owes`: replace the sentence with the measured histogram.

### The record this pass corrects

The audit's claim ledger marked **6 contradicted and 8 unsupported of 43** claims made on #117 and PR
#118 — including the issue **title**'s "accumulates without bound" (2 MiB is live at exit), a published
"two seconds" with no artifact behind it, the "29 × 64 MiB" sentence above, and an issue attribution the
author had "corrected" to the wrong mechanism and which then propagated into an independent agent's
verdict as though it were evidence. The corrected record is posted on both; what this pass registers is
only the two rows above, because the rest are corrections to claims about runs, and a claim about a run
belongs where the runs are described.

## 28. The merge's conflict resolution was the memory ceiling: an ordering blow-up, then the enumeration itself (#117)

§27 named the heap's *owner* — glibc — and could not name the structure inside it, because the
instrument that names structures needs a clean exit a node at the ceiling does not get. This section
has one: a jemalloc heap profile dumped **at** the ceiling, triggered the moment the node's own cgroup
crossed a threshold rather than at exit after it has freed its world
(`spec/audit/evidence/n117-heap-profile-results.md`). It attributes **97 % of the live heap** to
`BTreeSet`/`BTreeMap` clones under `rchain_sdk::dag::merging::resolve_conflict_set`, reached from
`MergeScope::merge` via `get_pre_state_for_parents`, growing 343 -> 881 -> 1488 MiB across three dumps at
1.5 / 3.0 / 4.6 GiB of live bytes. (`BTreeSet<T>` wraps `BTreeMap<T, ()>`, so a set clone shows as a map
clone.) Two defects were inside that number, and they are separate findings because the first is a
constant factor removed and the second is the exponent.

### C177 — the rejected set was carried as state, and it is not state

`resolve_conflict_set` calls `compute_rejection_options` (`sdk/src/dag/merging.rs`), the Scala
`computeRejectionOptions` port: a BFS whose queue entries each carry the last-accepted key, the rejected
set, and the accepted set — cloning **two** sets per push. The rejected set is *derivable*: the only way a
key enters it is as the conflict of an accepted one, so after every step
`rejected = ⋃{conflicts(x) : x ∈ accepted}`. Carrying it separately does not change what the search
computes; it changes how many queue entries it takes to compute it, because a state becomes reachable by
one entry per *ordering* that reaches it.

**The census** (`spec/audit/evidence/n117-rejection-state-census.rs`, the pre-fix search kept verbatim so
the count can be reproduced) on the shape the node produces — a **fork**, where chains within a branch do
not conflict and chains across branches conflict completely:

| chains | queue entries pushed | distinct states |
|---|---|---|
| 10 | 650 | 62 |
| 16 | 219,200 | 510 |
| 20 | **19,728,200** | **2,046** |

Two registers and two cloned sets per entry, at 19.7M entries, is the gigabytes the profile sees. The
distinct count is exactly `2^(per+1) - 2` — the nonempty subsets of one branch, twice over — so the ratio
between the columns is the redundancy, and it is factorial in the branch width.

**Fixed by carrying the accepted set alone, which expands each state once.** Exact, not a heuristic: the
successor set (`all_keys \ (rejected ∪ accepted)`) and the answer (`rejected` at a terminal state) depend
on the state only through `accepted`, so skipping a state already expanded cannot remove a reachable
answer. Per the rule this project holds proofs to, the argument is prose and the check is a test:
`rejection_options_match_a_literal_enumeration` (`sdk/src/property_tests.rs`) differs the shipped
function against a **literal transcription of the old search**, over both the legally-shaped maps the
merge produces and arbitrary maps including the asymmetric ones.

Measured: 20 chains **11.3 s -> 13.4 ms**, and 40 chains, infeasible to run at all before, in 34.6 s at
351 MiB. That is where this pass first stopped, and it is worth recording why that was not the fix: 40
chains still took 34.6 s, and the enumeration behind the 2,046 states had not moved.

### C178 — the enumeration is exponential in the width, and the width was never measured

**What the search does, exactly.** A set `S ⊆ keys` is reachable iff its keys can be added one at a time
with each new one absent from the conflicts of those already accepted — equivalently, iff the conflicts
*within* `S` contain no directed cycle. So the search expands **one state per nonempty subset that induces
an acyclic subgraph**, and the number of rejection options is unrelated to that count. Pinned as four
closed forms in `the_enumeration_expands_one_state_per_acyclic_subset`, counts rather than seconds
because a stopwatch measures the machine:

| conflict map | states the enumeration expands | options reported |
|---|---|---|
| complete, `n` keys | `n` | `n` |
| **no conflicts at all** | **`2^n - 1`** | **1** |
| two branches of `p` (fork) | `2^(p+1) - 2` | `2` |
| a perfect matching of `m` pairs | `3^m - 1` | `2^m` |

Row one is why the 1000-key full-graph oracle passed while the node grew to gigabytes. **Row two is the
node's normal case** — two chains conflict only if they touch a common channel
(`DeployChainIndex::deploys_are_conflicting`, `casper/src/merging.rs:1103`), so a merge scope's conflict
relation is *sparse*, and sparse is where the enumeration is worst. And the shape had never been read off
a running node: the defect had been described with an *assumed* fork.

**So the shape was made observable before it was fixed.** `SearchCensus` (`sdk/src/dag/merging.rs`)
reports keys, conflict pairs, asymmetric pairs, self-conflicts, states expanded, frontier and options;
`resolve_conflict_set_with_census` returns it from the merge itself, `casper/src/merging.rs`'s
`search_census` accumulates the envelope process-wide, and `casper/src/interpreter_util.rs` logs it once
per five seconds on the node's own log. The casper merge fixtures now assert the one field that decides
the fix: `the_merge_search_sees_the_shape_the_merge_builds` reads **0 asymmetric pairs** — the relation
the merge hands the search is symmetric on its keys, which is the precondition of the exact rewrite.

**The exact rewrite.** With a symmetric, irreflexive relation on the keys, "reachable" is exactly
"independent set" and "terminal" is exactly "dominating", so **the terminal states are precisely the
maximal independent sets** and the option set is their image under `⋃ conflicts`. `compute_rejection_options`
now enumerates those with Bron–Kerbosch and pivoting — and touches none of the `2^n` intermediate states,
which were the entire defect. The rewrite is gated on the precondition being *checked*, not assumed: an
asymmetric map, or a key that conflicts with itself, takes the enumeration as before, and
`rejection_options_match_a_literal_enumeration` now asserts which path each generated map earned as well
as that both agree with the literal transcription.

| shape | before | after |
|---|---|---|
| 20 chains in two branches | 2,046 states / 13.4 ms | 30 states / 0.17 ms |
| 40 chains | 2^21 states / 34.6 s | 60 states / 0.7 ms |
| 200 chains | — | 300 states / 22 ms |
| 2,000 chains | — | 3,000 states / 21 s |

The counts grow linearly with width and the options are byte-identical; the falsifier that was
`#[ignore]`d red for two commits is now a gate (`rejection_options_are_bounded_on_a_fork_shape`), asserting
`expanded ≤ (keys + 1) × (options + 1)` — a bound the enumeration misses by an exponential — at every width.

**Two honest limits.** The recursion is output-sensitive but a node is not free: the pivot scan is
`O(|p| · log)` per candidate, so 2,000 chains costs 21 s of CPU (20 ms at 200). And the *options* can
themselves be exponential — a perfect matching of `m` pairs has `2^m` of them — but they are now the only
exponential thing, which is inherent to the question the merge asks and not to how this answers it.
`max_frontier` is 0 on this path: #117's memory is now the option set and nothing else.

**This is not the closure rewrite, and that refutation still stands.** Computing
`rejected ⊇ ⋃{conflicts(j) : j ∉ rejected}` to a fixed point over-approximates, because a key that has
been *rejected* can never be accepted afterwards: for `{0: {1}, 1: {0}, 2: {}}` the search yields
`{{0}, {1}}` while the closure also yields `{0, 1}`. That counter-example is pinned by
`rejection_options_are_not_the_closure` and the enumeration it refutes is still in the tree, as the
fallback path.

C175 — the unbounded ingress queue §27 could not rule out — is not this defect. Its observation half
landed (`#120`) and the depth reads 0.0 at every sample through a ramp that OOM-kills all three nodes, so
a drained queue is not what held the memory. The profile and the census agree on which path did.

[#117]: https://github.com/rchain-community/rchain-rust/issues/117
