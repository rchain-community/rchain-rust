# Type-safety audit — the census, the classification, and the rules it yields

**Reviewed revision.** `dev` at `b625eef06`, plus whatever the census header records — the census is
`spec/audit/evidence/type-safety-audit/census.txt`, taken by `tools/audit-type-system.sh --sites` at the
moment the audit opened, and it carries the tree hash it was taken at.

**Why this study exists.** Not a new suspicion — the repository's own audit record names the gap twice.
`docs/src/node/security-audit.md` §5 ("What a green register does not cover") says it outright:

> **The computed gates are weaker than they read.** The type-system gate's counted classes — `cast`,
> `lax`, `get`, `index`, `div`, `overflow` — are reported and **not enforced**; the ratchet that once
> failed the build is no longer wired.
> …
> **Nothing in the register asks what an attacker pays.** Cost is not a row type the audit can express.

And this session's own precedent says *why* the instrument has to read shapes rather than behaviour: the
rule motivated by worksheet row F-U8-01 — "an awaited call whose error is erased into an absence" — found
**five** production sites, and **four of them were invisible to the failure-mode disposition pass**,
because that pass read what a node *does* and this is what the code *says*.

**The ask, and the standard.** A multi-agent audit of every non-type-safe construct — the `unwrap` family
and its relatives — producing a classified inventory, and, for every class that survives, a rule that
**fails the build** rather than a paragraph that reads well. `spec/TYPE-SYSTEM.md` §1.6 states the policy
being enforced: *"every partial operation is either proven total on a refinement, or returns
`Option`/`Except` at a declared boundary."*

## Method — the house's three stages, reused

`docs/src/node/security-audit.md` §1 records the method this project's audit history converged on, with
its refutation stage rebuilt after the first version failed. All three stages are taken unchanged:

1. **Lenses** read the tree adversarially. Each must produce **either a reproduction or nothing** — no
   "this looks risky".
2. **Refutation.** Every candidate goes to a *second* agent instructed to refute it. The refuter's
   instruction is the **inverse** of the lens's: it may return `REFUTED` **only** with a positive artifact
   — a named line, a test, or a measurement — because under budget pressure the failure mode is not a
   missed finding, it is an **invented artifact**.
3. **Grading.** Survivors are graded and given a falsifier.

Three verdicts, and only one removes a finding: **`CONFIRMED` / `NOT-RE-ESTABLISHED` / `REFUTED`**. An
empty-handed refutation is a self-reported clean, and this project's record already holds that *"an
independent read is evidence and a self-reported clean is not"*.

**Artifact form** (`spec/audit/evidence/n117-audit.md`): adjudications, a claim ledger, a merged table
**with a pair index for mechanical completeness**, a fault tree, a verdict.

## 0. The census — taken, not re-derived

The gate already enumerates the candidate universe mechanically, keyed on site text. Re-deriving it would
spend agents to rebuild an artifact the repository owns, so the census is the lenses' **input**:

| class | verdict kind | sites | the gate's baseline says | drift |
|---|---|---|---|---|
| `panic` | hard | 30 | — | all 30 allow-listed by 29 entries |
| `unsafe` | hard | 0 | — | `#![forbid(unsafe_code)]` at 15 crate roots |
| `silent` | hard | 0 | — | the awaited-erasure rule (C251) |
| `escape` | hard | 0 live | — | 25 constructions allow-listed, 30 roster newtypes derived |
| `cast` | **counted** | **358** | 331 | **+27** |
| `index` | **counted** | **397** | 316 | **+81** |
| `div` | **counted** | **74** | 54 | **+20** |
| `overflow` | **counted** | 41 | 41 | — |
| `lax` | **counted** | 10 | 10 | — |
| `get` | **counted** | 0 | 0 | — |

**Finding TSA-1 — the census has been printing into the void.** `tools/type-system-baseline.tsv`'s own
header says it is *"a dated measurement, not a ratchet"*, and the gate confirms it on every run: the
ratchet was deleted 2026-09-27, `ratchet_failures` is declared and never incremented, and `ratchet()`
prints a comparison and returns. So **128 sites of drift** (cast +27, index +81, div +20) have accumulated
with a message that reads *"count that moved is worth a look"* and nothing that acts on it. The gate
notices; nothing enforces. This is the §5 bullet, measured.

**Finding TSA-2 — the panic family's zero is an allow-list, and an allow-list cannot see its callers.**
The `panic` class reports zero violations because all **30** sites are allow-listed by **29** entries. The
gate's own record holds why that is not the same as "no reachable panic": AUDIT C97 — four sites were
green *"because an allowlist entry keyed on the *site* cannot see whether a caller supplies wire bytes."*
The gate's header states its own boundary in the same breath: *"a green `OK` here is evidence about the
sites this file lists, never about the ingress discipline."* Whether any of the 30 is peer-reachable is a
question the census cannot answer and this audit must — it is lens L1's first half.

**Finding TSA-4 — the gate's site key is coarser than a site, and 59 of them are shared.** The census
prints `path:line: text`, and the gate keys an allow-list entry on **`(path, text)`** — deliberately, so an
expression repeated in a file is one entry. But a *site* is a line, and the census shows the cost:
`crypto/src/hash/blake2b512_random.rs:42` and `:240` are the same text, **198 lines apart**, and **59 of the
census's keys are shared by two or more rows**. Two consequences, and the second is the dangerous one:

- An audit that dedups on the gate's key collapses two distinct sites into one row. So this record carries
  **two keys**: `site_key` (the gate's `(path, text)`, so the allow-list can reuse it) and the line, kept
  beside it for ranking.
- An allow-list entry's evidence window is `[line−25, line+8]` around the site — *one* line. When the key
  matches two distant rows, the evidence can be true of one and unreachable for the other, so the entry can
  read as evidenced while a site it claims to cover is never actually checked. **A key coarser than a site
  makes its own evidence check vacuous for the second site.** Any rule this audit adopts must key on
  something that distinguishes the rows it means to cover, or state that it covers a *shape* rather than a
  line.

**The mass the gate does *not* census** — measured on stripped production code, and the stripping matters:
a raw scan reads `.ok()` 76 and `let _ = ` 166, while the gate's own `#[cfg(test)]`-aware strip reads
**62** and **80**. These are the stripped numbers:

| family | sites | worst files |
|---|---|---|
| `.unwrap_or(` | 164 | `rholang/src/native_state.rs` 30, `qucalc/src/lib.rs` 9, `node/src/runtime/node_main.rs` 7 |
| `.unwrap_or_else(` | 106 | `rspace/src/native_store.rs` 11, `ocapn/src/owner.rs` 7, `comm/src/discovery/peer_table.rs` 7 |
| `.unwrap_or_default(` | 47 | `comm/src/upnp/gateway.rs` 8, `sdk/src/dag/merging.rs` 5 |
| `.ok()` | 62 | `comm/src/upnp/gateway.rs` 13, `rholang/src/system_processes.rs` 10 |
| `let _ = ` | 80 | `node/src/runtime/node_runtime.rs` 10, `ocapn/src/conn.rs` 6 |
| `.get(` (bare, no direct unwrap) | 321 | `rholang/src/native_state.rs` 31, `casper/src/merging.rs` 22 |
| `into_inner` | 122 | `node/src/api/grpc/tonic.rs` 18, `casper/src/protocol/client.rs` 14 |
| `try_from` / `try_into` | 260 / 36 | `rholang/src/native_state.rs` 40 / 20 |
| `std::mem::` | 15 | — |
| binary `+ - *` | **914** (907 unguarded) | `rholang/src/native_state.rs` 65, `casper/.../proposer.rs` 29 |

**So the un-censused surface is roughly 700 sites** — the 459 of the defaulting/discard family (L2), the 321
bare `.get(`, the 122 `into_inner`, the 15 `std::mem::` — on top of the counted 880. And the arithmetic gap
is the sharpest of them: the gate's `overflow` class covers `+ - *` in **the 11 refinement files only**, where
it reads 41, while the tree has **914 such lines**.

**The panic zero, precisely.** 30 sites, all allow-listed, **0 un-reviewed** — and the composition is worth
stating because "the panic family is gated" hides how thin it is. `unwrap` is **one** site
(`sdk/src/primitive.rs`'s `get_unsafe`, the declared Scala escape hatch). Real `Result::expect`s are
**seven**: five in `rspace` (radix-tree and key-segment invariants), one in `ocapn/src/owner.rs`, one in
`casper/src/block_random_seed.rs`. `assert!`-family 21, `panic!` 1 (inside the allow-listed
`unwrap_or_else(|| panic!(…))`), and `unreachable!`/`todo!`/`unimplemented!` **0**. The 50
`self.expect(Tok::…)` calls in `rholang/src/parser.rs` are a *method* on a tokenizer, not `Result::expect`,
and the gate excludes them by name.

**The reachable sets, as the lenses' input.** The wasm32 chain CI builds is `rchain-rspace` and
`rchain-rholang` plus `shared`, `crypto`, `models`, `qucalc` — and that six-crate set holds **no `unwrap`**,
the five allow-listed `rspace` expects, zero `unsafe`, and the bulk of the census (cast 292, index 299,
`try_from` 156). The request paths — `node/src/api/**` and `node/src/web/**`, 24 handler files — hold
**`unwrap` 0, `expect` 0, `panic!` 0, `unsafe` 0**; what remains there is `into_inner` 20, arithmetic 25,
`.get(` 10. Both facts are *refutations in advance* for candidates the lenses might otherwise raise.

**A near-miss this pass records rather than hides.** The census listing shows 355 `cast` rows where the
gate counts 358, and this audit first read that as an instrument defect of exactly the class the gate's own
history is made of (`SITE_AWK` not derived from `COUNT_AWK`). It is not: **three census rows have no space
after the colon** — `models/src/murmur_hash3.rs:7:const PRODUCT_SEED: i32 = 0xcafebabeu32 as i32;` — because
the site text begins a `const`, and a parser requiring `: ` loses them. 355 + 3 = 358 ✓. The lesson is the
one the refutation stage exists for, applied to the auditor: *a claim about an instrument needs the
instrument's own output as its artifact*, and the naive parse was mine, not the gate's.

**The scope boundary the lenses inherit** (stated rather than left silent): `rspace-bench` is in
`NOT_SCANNED` and is never read; files matching `TEST_ONLY_FILE_RE` are excluded **by name**; `#[cfg(test)]`
blocks are stripped; and the `unsafe`/`silent`/`escape` scans do **not** strip comment lines, so a doc
comment quoting a pattern would false-positive. The lenses use this same boundary deliberately — a
different one would make their counts incomparable with the gate's. Three further holes: **macro-generated
code is never seen** (`prost`'s `OUT_DIR` output is in `target/`, and the checked-in generated
`models/src/casper/protocol/*.rs` *are* scanned); `/* … */` block comments are **not** stripped (the gate's
own baseline records `HTTP/2` reading as a `div` hit); and a `#[cfg(test)]` on an unusual shape can leak
test code into the census.

## 1. The fatality policy, written down for the first time

No document states what a production partiality *costs*, so no one can say whether a given `unwrap` is a
node kill or a shrug. Four facts, each with its anchor:

- **There is no `panic = "abort"`.** No `[profile]` section exists in any `Cargo.toml`; `.cargo/config.toml`
  sets no panic strategy. The Rust default — **unwind** — applies in dev and release alike.
- **There is no panic hook and no production `catch_unwind`.** The tree's only `catch_unwind` is inside a
  `#[test]` (`casper/src/block_random_seed.rs:225`, `:239`).
- **A fire-and-forget task's panic vanishes.** There are **88 `tokio::spawn` sites in production code** —
  `node/src/runtime/node_runtime.rs` 19, `rspace/src/concurrent/channel_queue.rs` 12, `node/src/api/ocapn.rs`
  7, `ocapn/src/noise.rs` 5, `casper/src/blocks/block_receiver.rs` 5, `comm/src/transport/grpc_transport_receiver.rs`
  4, and more — and all but the awaited listener set are fire-and-forget. tokio captures the panic into a
  `JoinHandle` nobody reads: the task is gone and the node keeps reporting itself healthy. One of those
  twelve is in the *reducer's* channel queue.
- **A listener task's panic is a node kill.** The five or six listener tasks are awaited via `select!`; a
  `JoinError` goes to `listener_stopped` (`node/src/runtime/node_runtime.rs:1551-1566`), is returned by
  `serve` (`:706-709`), and `node/src/main.rs:201-204` turns it into `std::process::exit(1)`. The code says
  so itself (`:652-657`): *"a panicking task"* is one of the ways an accept loop returns.

**TSA-3 — the policy, as a rule.** A production partiality is a **node kill** if and only if it can unwind
through one of the awaited listener tasks; everywhere else it is a **dropped task** that leaves the node
reporting itself healthy. So "how bad is this `unwrap`" is not a property of the `unwrap` — it is a
property of the *task it runs in*, which is why reachability is a field of this audit's record and not an
afterthought.

**And the corollary that costs the most.** The dangerous class is not the node kill — a dead node is loud.
It is the **dropped task**: a lookup, a re-request, a proposer loop, or a metrics push that dies and leaves
a node answering `/api/status` with a healthy-looking face. That is the same shape as the incident this
programme started from (C248: a merge lost a write with no counter and no line anywhere moving), and it is
what the highest-severity tier of this audit is for.

## 2. The lenses

Six lenses read the tree adversarially, each required to produce **a reproduction or nothing** and to state
its own input domain. Their candidates are §3; what matters here is what each *is* and what the set of them
turned out to be.

| lens | family | candidates | the shape of its result |
|---|---|---|---|
| **L1** | panics and the counted mass | **1** | the panic zero **holds** (29 allow-list entries re-walked with C97's question, each closed) and ~520 counted sites classify as **contained** — with **one ingress panic** in the `index` class |
| **L2** | silent defaulting and erasure | **15** | the class C251 named is **wider than C251's rule**, and two of the sites are on the validation path |
| **L3** | numeric conversion and width | **9** | four of nine are *wasm-only* and L3 says so; the rest are narrowing/sign/float on the x86-64 path |
| **L4** | refinement escape hatches | **6** | the checked class's zero **holds**; what is broken is the **roster**, not a type |
| **L5** | concurrency partiality | **18** | 13 are `dropped-task` — a **task** discipline the tree does not have, where the HAZOP had rowed a *lock* discipline |
| **L6** | the wasm32 target | **8** | the target's divergences, with a **bypassed guard** as their shape (`restrict_to_int` exists; four siblings skip it) |

**57 candidates before dedup**, and the audit is not a list of 57 things. Three structural products:

- **The counted classes are mostly contained, and the exceptions are severe.** L1 classified div, overflow,
  lax and 397 indexings and found guards in nearly every case — and the one exception (`hexToBytes` on
  non-ASCII deploy content) is an **ingress panic**. That is the audit's answer to "what is in the 880": not
  a mountain of risk, but a counted mass with a handful of genuine holes, found only by *reading each site's
  guard* — which is exactly what a census cannot do.
- **The classes cluster, and the clusters are the deliverable.** Two lenses independently reached the poison
  counter (L5 by reading `rspace`, L2 by sweeping the pattern) and *disagreed on the count* (8 vs 49) —
  routed to refutation, as the design requires. Two reached the width family from opposite doors (L3
  deferring wasm explicitly, L6 owning it) and *agreed*. Dedup by site key is what makes "57 candidates" not
  mean 57 defects.
- **Three of the six lenses end in a hole in a *boundary* rather than a defect in a site**: L4's is the
  roster (a new file nobody added), L5's is the absence of any supervisor for a detached task, and L3/L6's
  is whether a second target is *shipped*. An audit that only listed sites would have missed all three.

## 3. The candidates

**The record, per candidate.**

| field | meaning |
|---|---|
| `site_key` | `path :: the joined site text` — the **gate's** key, so dedup is a `GROUP BY` and the allow-list can reuse it |
| `construct` | the family, from a closed vocabulary |
| `claim` | the sentence the lens asserts |
| `repro_or_nothing` | a command, a named test, or a named line |
| `reachability` | `ingress` · `process` · `config` · `tool` · `test-only` · `dead` (the register's existing vocabulary) |
| `failure_mode` | `panic-kill` · `dropped-task` · `silent-wrong-value` · `contained` |
| `class` | `Void` · `Terminal` · `Drift` · `Split` · `Historic` |
| `disposition` | one rung: `R1` · `R2` · `R3` · `R4` |
| `verdict` | `CONFIRMED` · `NOT-RE-ESTABLISHED` · `REFUTED` |
| `falsifier_direction` | what would overturn the verdict |
| `lenses` | the set of lens ids that named the site |

**Each lens's rows as emitted, before adjudication.** `verdict` is filled by the refutation stage, and a
candidate's identity is its `site_key` — so two lenses naming one site produce **one** row carrying a
two-element `lenses` set rather than two rows, and a disagreement between them goes to refutation instead
of being averaged.

### L5 — concurrency partiality

**L5's thesis, and it is a class the failure-mode pass did not name.** Of its 18 rows, **13 are
`dropped-task`**: the HAZOP read what the node *does* while holding a lock (its U9 rows owe a **bound** or a
**lint** — a *lock* discipline), and this family is what the node *stops doing* when a detached task dies —
which owes a **supervisor** or a **watched handle**, a *task* discipline entirely absent from the tree.

| # | site_key | construct | claim | repro | reach | failure | class | disp |
|---|---|---|---|---|---|---|---|---|
| 1 | `node/src/runtime/node_runtime.rs :: tokio::spawn(processor)` (`:976`) | dropped-task | the block-validation + insert loop is fire-and-forget; a panic drops it and every later validated block is never inserted — the chain stops advancing while `/api/status` stays green | line only | ingress | dropped-task | lifecycle | R2 |
| 2 | `… :: tokio::spawn(load_blocks)` (`:923`) | dropped-task | `pump_validated_blocks` is the validated→processor hand-off; its panic closes `processor_input_tx` and the processor exits `Ok` | line only | ingress | dropped-task | lifecycle | R2 |
| 3 | `casper/src/engine/node_syncing.rs :: tokio::spawn(… loop { … })` (`:325`) | dropped-task | the LFS-sync retry task is the only thing that can re-arm a failed sync (its own comment says so); a panic drops it and `finished`/`terminal` never notify → the node reports "syncing" for ever | `node_syncing.rs`'s own fixture | ingress | dropped-task | lifecycle | **R2** |
| 4 | `node/src/runtime/node_runtime.rs :: tokio::spawn(request_deps)` (`:1261`) | dropped-task | the `request_all` loop that pulls missing dependencies dies silently; the node keeps producing blocks with an unresolved dependency set | line only | process | dropped-task | lifecycle | R2 |
| 5 | `… :: tokio::spawn(peer-message router)` (`:1027`) | dropped-task | the single hop from the transport's routing queue to each shard's `NodeLaunch`; its panic makes every peer packet answer `InternalCommunicationError` — the node looks partitioned, cause only in the dropped task | `comm/.../handle_messages.rs:190` is the symptom | ingress | dropped-task | lifecycle | R2 |
| 6 | `… :: tokio::spawn(discover)` (`:380`), `tokio::spawn(clear_connections)` (`:403`) | dropped-task | the two comm maintenance loops (discover+connect, ping+prune); a panic stops peer discovery or leaves dead peers in `connections`, unreported | lines only | ingress | dropped-task | lifecycle | R3 |
| 7 | `… :: tokio::spawn(kademlia_serve)` (`:355`) | dropped-task | its `Err` is logged, but a **panic** is a `JoinError` on a dropped handle — the one discovery failure the `select!` does not see | line only | ingress | dropped-task | lifecycle | R3 |
| 8 | `… :: tokio::spawn(node_launch)` (`:1880`) | dropped-task | `NodeLaunch` carries the block stream for the node; its `Err` arm is logged and tested, its panic arm is neither | line only | process | dropped-task | lifecycle | R2 |
| 9 | `… :: tokio::spawn(proposer stream)` (`:1969`) | dropped-task | the proposer stream driver dies silently on panic; block production stops | line only | process | dropped-task | lifecycle | R3 |
| 10 | `… :: tokio::spawn(autopropose timer)` (`:1738`) | dropped-task | the `--autopropose` timer has deliberate halt logic (#157) — but only for the *failure threshold*; a panic drops the timer with **no flag set** | `--autopropose` on a quiescent net | config | dropped-task | lifecycle | R3 |
| 11 | `casper/src/blocks/block_receiver.rs :: tokio::spawn(incoming_blocks)` (`:737`), `tokio::spawn(validated_blocks)` (`:748`) | dropped-task | the two block-intake pipelines are detached; a panic stops ingestion from peers and `out_rx` simply goes quiet | lines only | ingress | dropped-task | lifecycle | R2 |
| 12 | `comm/src/transport/grpc_transport_receiver.rs :: tokio::spawn(accept)` (`:106`) | dropped-task | the per-connection TLS handshake task is detached; its panic drops one peer's connection while the accept loop continues — the peer sees a truncated handshake, the node sees nothing | line only | ingress | dropped-task | lifecycle | R3 |
| 13 | `casper/src/dag.rs :: let _guard = self.lock.lock().await` (`:489`) | guard-across-await | **the reachability F-U9-01 states as behaviour only**: `insert` is called from `block_processor`, which *spawns per-block validations in parallel* (`block_processor.rs:129`), so N peer-driven validations serialize on one global lock held across sixteen awaits and both representation locks. A slow store turns admission from "slow" into "stopped", and the trigger is a peer's block | `block_processor.rs:129` + `:62`/`:179` | ingress | silent-wrong-value (liveness) | lock-discipline | R2 |
| 14 | `casper/src/dag.rs :: self.representation.write().await` (`:729`) → `height_map().await` (`:736`) | guard-across-await | **the reachability of F-U9-02**: `update_metadata` is reached from the C193/C190 **revalidation** path (`multi_parent_casper.rs:736`) and holds the representation *write* lock across an LMDB read, blocking every `representation.read()` in `insert`'s equivocation check (`dag.rs:502`); the discipline `insert` documents at `:547` holds only in `insert` | `multi_parent_casper.rs:736`; `dag.rs:547` | process | silent-wrong-value (stall) | lock-discipline | R2 |
| 15 | `rspace/src/native_store.rs :: self.overlay.lock().unwrap_or_else(\|p\| p.into_inner())` (`:340`, `:361`, `:397`, `:427`, `:500`) | poison-accounting | **a poison recovery that bypasses the counted accessors.** `NativeStore` holds `std::sync::Mutex`/`RwLock` directly and recovers with the raw `into_inner`, **not** through `rspace/src/lock.rs`'s counted `rlock`/`wlock`/`mlock`. So the `poison_recoveries()` value published at `node/src/api/web_api_impl.rs:577` **under-reports**, and any deploy that writes native values is on the path | poisons a `NativeStore` lock and asserts the counter moved — **it does not, today** | ingress (any deploy) | silent-wrong-value | **poison-accounting** | **R3** |
| 16 | `rspace/src/concurrent/channel_queue.rs :: arc.lock().unwrap_or_else(\|p\| p.into_inner())` (`:213`, `:335`, `:390`) | poison-accounting | the same bypass in the **reducer's per-channel claim queue** — `:213` is `insert`, `:335` the head-lease check, `:390` the `ClaimGuard::drop` release, which runs on unwind paths. Produce/consume claim poisoning is recovered and uncounted | as #15, against a channel queue | ingress (deploy produce/consume) | silent-wrong-value | poison-accounting | R3 |
| 17 | `casper/src/engine/node_launch.rs :: tokio::select! { handle_loop = … }` (`:304`) | cancel-unsafe | `handle_loop` holds `engine.lock().await` and calls `guard.handle(…).await` — a mutating await; when the `finished`/`terminal` arm wins, the future is **dropped mid-`handle`**, so the engine can go running with a message half-applied | `node_launch.rs:296-303` (the held lock + mutating await in the arm) | process | silent-wrong-value | cancel-safety | R3 |
| 18 | `casper/src/engine/lfs_block_requester.rs :: tokio::select! { request_loop / response_loop }` (`:332`), `lfs_tuple_space_requester.rs:397` | cancel-unsafe | when one loop finishes the sibling is cancelled inside `process_block`, which mutates `st` under `st.lock()`; the walk then clones `st` as its result — a cancelled mid-update `st` is published | `lfs_block_requester.rs:287-330` | process | silent-wrong-value | cancel-safety | R3 |

**Checked and contained — stated so the sweep is not read as a miss.**

| site | verdict |
|---|---|
| the listener `select!` (`node_runtime.rs:685-701`) | **contained** — every arm polls a `&mut JoinHandle`, so a losing arm leaves its task Pending rather than dropping it; the one inline future is a watch read. Cancel-safe |
| `node_main.rs:45`, `:69` (the `spawn_blocking`s) | **contained** — the `Result` **is** read (`.await.map_err(…)`), so a `JoinError` surfaces as a CLI error. (This refutes an item the lens opened with) |
| `block_processor.rs:129` | **contained** — the per-block validation handles are collected and awaited |
| `block_receiver.rs:640-660` | **contained** — `work` is `tokio::pin!`-ed, so `interval.tick()` re-polls rather than drops it |
| `rspace/src/lock.rs:75`, `:86` and `node_syncing.rs:768-787`'s `.expect(…)`s | **test-only** — inside `#[cfg(test)]` |

**L5's input domain.** (a) It established the *accounting* gap in #15/#16 (the counter under-reports) but
**not** a reachable panic inside those critical sections — "a remote can poison the store" is
`NOT-RE-ESTABLISHED`, and must not be graded as if it were. (b) For #17/#18 it established that the future
*is* dropped mid-mutating-await (a positive artifact) but not that the drop leaves an *observable*
inconsistency — the honest claim is "cancel-unsafe shape". (c) Three further comm spawns
(`grpc_transport.rs:201`, `:319`, `grpc_transport_client.rs:142`) were not confirmed either way and are
**not rows** — a second pass owns them. (d) Nothing was run: every `repro` is a named `file:line`, which is
what a lens under budget can honestly give.

**L5 count: 18 candidates (13 `dropped-task`, 2 `guard-across-await`, 2 `poison-accounting`, 2 `cancel-unsafe`), 5 refuted inline, R2 × 8 · R3 × 9.**

### L6 — the wasm32 target

**The load-bearing fact.** `usize` is 32 bits on `wasm32-unknown-unknown`, so a positive `i64`/`u64` that
fits a 64-bit `usize` but not a 32-bit one **silently truncates mod 2³²**. Casts to `u32`/`i32` do **not**
diverge (both targets give the low 32 bits), so the divergent subset is exactly `{i64, u64} → usize`. The
wasm-reachable crate closure is `shared`, `crypto`, `models`, `rspace`, `rholang`, `qucalc`; everything else
has no wasm build at all.

**The finding's shape, and it is the best kind.** `rholang/src/reduce.rs`'s `restrict_to_int` (`:122-130`)
**is** the tree's i32 boundary — it refuses a value outside `[i32::MIN, i32::MAX]` before the cast, and
`nth` (`:1141`) routes through it. **`slice`, `substring`, `take` and `indexOf` do not.** So this is not
"the tree has no width guard"; it is "the guard exists and four sibling methods bypass it".

| site_key | construct | claim | repro | reach | failure | class | disp |
|---|---|---|---|---|---|---|---|
| `rholang/src/reduce.rs :: let n = if n_i <= 0 { 0 } else { n_i as usize };` (`:1492`) | width/usize | `take(n)` accepts an unguarded `i64`; on wasm32 `4294967296 as usize == 0`, so the method returns **empty and charges `take_cost(0) == 0`** where native takes the whole list and charges the full cost — **cost and result diverge** | a new `#[wasm_bindgen_test]` under CI's *Reducer wasm32 tests* | ingress | silent-wrong-value | cast/width | **R1** |
| `rholang/src/reduce.rs :: let start = from_i.max(0) as usize;` (`:1647`) | width/usize | `indexOf`'s cost depends only on the string lengths, so no cost gate masks it: `"hello".indexOf("l", 4294967296)` is `-1` on native (start past the end) and `2` on wasm (start truncates to 0) | as above | ingress | silent-wrong-value | cast/width | **R1** |
| `rholang/src/reduce.rs :: (until - from) as usize`, `from as usize` (`:1457`, `:1464`) and the identical pair in `substring` (`:1624`, `:1628`) | width/usize | same unguarded narrowing, and **not masked**: `slice_cost(to)` charges exactly `from`, and the per-deploy ceiling `MAX_BLOCK_PHLO = 25_500_000_000` (`casper/src/validate.rs:627`) **exceeds** 2³² ≈ 4.29e9 — so `slice(4294967296, 4294967297)` is affordable and returns the *whole-string prefix* on wasm vs the *empty suffix* on native | as above | ingress | silent-wrong-value | cast/width | **R1** |
| `rspace/src/serializers/scodec_serialize.rs :: r.read_bits(64) as usize` (`:136`) + `rspace/src/history/cold_store.rs :: r.read_bits(64) as usize` (`:46`) | width/untrusted-length | a 64-bit big-endian length read from a **persisted leaf** is narrowed with **no check against the remaining buffer**; on wasm a length ≥ 2³² truncates, so the same bytes decode to a *short payload* on the target while the host attempts a ≥4 GiB allocation — the two targets fail *differently* (silent short read vs exhaustion) | CI's *Reducer wasm32 check*; `rspace/tests/wasm_roundtrip.rs` exercises the path | process | silent-wrong-value | cast/untrusted-length | **R1** |
| `rspace/src/hot_store.rs :: (u64::from_le_bytes(word) as usize) % shards.max(1)` (`:38`) | width/usize | the shard hash truncates **before** the modulo. At the default `SHARDS = 64` (a power of two) the low 6 bits survive, so nothing is observable; `with_shards(m)` for a non-power-of-two `m` puts a channel in a *different* shard on wasm32. **Contained today, a latent landmine** | CI's *Reducer wasm32 check* | process | contained | cast/width | **R1** |
| `shared/src/time.rs :: #[cfg(target_arch = "wasm32")] pub fn sleep(_: Duration) {}` (`:41`) | clock/no-op | on the target `sleep` is a **silent no-op**; the doc says the reducer must not depend on it and there is **no wasm-reachable caller** today. A future reducer path that paces with it will silently not pace — a trap, not a live bug | the artifact *is* the cfg arm | dead | contained | clock | R2 |
| `crypto/src/util/secure_random_util.rs :: pub use rand::rngs::SysRng as OsRng;` (`:10`) | rand/partial | the *missing-backend* case is a **compile-time refusal** (`getrandom` emits `compile_error!` without `wasm_js`, which is what CI gates). The *backend-present-but-host-lacks-crypto* case is not: `getrandom` returns `Err(WEB_CRYPTO)` rather than panicking, and `rand`'s `SysRng::fill_bytes` **panics** on it — a dead browser tab (non-secure context, a worker with no `globalThis.crypto`), not a compile error | `crypto/tests/wasm_law19.rs` already exercises the working path | process | panic-kill | rand/partiality | R2 |

**L6's input domain — and it is a refutation set as much as a limit.** The crates with **no** wasm build
(`comm`, `casper`, `node`, `block-storage`, `ocapn`, `graphz`, `sdk`, `rspace-bench`) cannot produce an L6
finding *by construction*, so the census's casts there — `comm/src/transport/*.rs`'s `…as i64`,
`block_api_impl.rs`'s `…as usize`/`as i32`, `interpreter_util.rs:549`'s `as_millis() as u64`,
`ocapn/src/syrup.rs:262`'s `len as u64` — are **refuted for this lens**. `models/src/wire.rs` compiles for
the target but is *not executed* by the reducer (the reducer parses rholang source, not protobuf). And the
lens did not execute the target: the casts and cfgs are read, not observed; the wasm *host* (browser tab vs
worker vs Node) decides whether the RNG row bites in practice. One further site
(`shared/src/time.rs`'s `Instant::now().elapsed().as_nanos() as i64`) was **refuted**: a `u128 → i64`
narrowing is target-independent, and its coarser `performance.now()` resolution is documented, not a defect.

**L6 count: 8 rows covering 11 sites; the semantics-changing core is rows 1–4 (six sites); rows 5–6 are latent; rows 7–8 are seams. R1 × 5 · R2 × 2 · R4 × 0 (one refuted R4).**

### L3 — numeric conversion and width

**The buckets, over the census's 358 rows** (not a tree-wide scan, which reads 20–30 % higher because it
does not strip `#[cfg(test)]`): `as i64` 128, `as usize` **98**, `as i32` 62, `as u64` 26, `as u8` 25,
`as f64` 17, `as u32` 13, `as u16` 8, `as char` 8, tail. **125 rows are size-derived** (103 of them
`.len() as`) and only **7** cast a literal-adjacent value. The priority order is the width tier first, then
the size-derived tier.

| # | site_key | construct | claim | repro | reach | failure | class | disp |
|---|---|---|---|---|---|---|---|---|
| 1 | `rholang/src/merging.rs :: uint16_be(data.channels.len() as u16)` (`:222`) | narrowing | a deploy's **mergeable number-channel count** is written as a `u16` with no bound on `channels.len()`; above 65 535 the decoder reads back `len mod 65536` (`read_u16` at `:261`) and **silently drops the rest — the merge applies a partial diff set** | a round-trip beside `merging.rs:794` with 65 537 `NumberChannel`s: it returns 1, not 65 537 | process | silent-wrong-value | **Split** | **R1** |
| 2 | `rholang/src/merging.rs :: uint16_be(seq.len() as u16)` (`:232`) | narrowing | the same `u16` prefix for the **sequence length**; a block whose mergeable-data sequence exceeds 65 535 truncates the count | as #1 with 65 537 `DeployMergeableData` | process | silent-wrong-value | Split | R1 |
| 3 | `rspace/src/serializers/scodec_serialize.rs :: read_bits(64) as usize` (`:136`) **+** `rspace/src/history/cold_store.rs` (`:46`) | width | a 64-bit length prefix truncated to `usize`; on wasm32 `2^32 + k` reads as `k`, so **the same bytes decode to a different payload per target**, and nothing bounds the value first. (L6 rows it too — see the convergence note) | `read_size_head` fed `0x1_0000_0005` on a 32-bit target | ingress | silent-wrong-value | Split | R1 |
| 4 | `rholang/src/native_state.rs :: u32::from_le_bytes(len) as usize` (`:6971`) | width | `brand_len` (up to 2³²−1) is added to 9 for a `rest.len() < brand_len + 9` check; on a 32-bit `usize` that add overflows and **the check inverts** for a bogus brand | a 32-bit-target test with brand length `0xFFFF_FFF8` | ingress | silent-wrong-value | Split | R1 |
| 5 | `casper/src/event_converter.rs :: times_repeated.insert(p, pe.times_repeated as usize)` (`:53`), `.map(\|p\| p.channel_index as usize)` (`:60`) | sign | the wire fields are `i32`; a **negative** `times_repeated` from a peer's block event log becomes ~2⁶⁴, so `ReplayRSpace::matches` compares a huge `expected` against the real counter, never matches, and **the replay of an otherwise-valid block diverges** — reaching replay, the block API and the merger | `event_converter.rs`'s own module: build a `Comm` proto with `times_repeated = -1` | ingress | contained | **Drift** | R2 |
| 6 | `rspace/src/state/exporters.rs :: chunk_size as i32` (`:104`) **+** `rspace/src/state/mod.rs :: history_items.len() as i32` (`:247`) | narrowing | a `usize` chunk size narrowed to `i32`; a chunk > `i32::MAX` turns negative and **inverts the `received < chunk_size` end-of-stream test**. Distinct from R16, which bounds the *request* | `validate_state_items` with `chunk_size = usize::MAX` | process | silent-wrong-value | Drift | R1 |
| 7 | `node/src/configuration/hocon.rs :: Hocon::Real(x) => Ok(*x as i64)` (`:136`), `(n * mult) as i64` (`:55`), `Duration::from_nanos((n * nanos) as u64)` (`:86`) | float | an out-of-range or **NaN** real in config saturates silently to `i64::MAX`/`0`, while the sibling `to_i32` **refuses** via `i32::try_from` — so `phlo-price = 1e30` is accepted where the same value spelled as an integer is rejected | `to_i64(&Hocon::Real(1e30)) == i64::MAX` | config | silent-wrong-value | Void | R2 |
| 8 | `node/src/configuration/commandline/config_mapper.rs :: Hocon::Integer(v.as_nanos() as i64)` (`:55`) | narrowing | a CLI duration's `as_nanos()` (`u128`) truncated to `i64`; > ~292 years yields a wrong, possibly negative, nanosecond count | round-trip `Duration::from_secs(1<<62)` through `parse_duration` | config | silent-wrong-value | Void | R2 |
| 9 | `models/src/wire.rs :: bitset.iter().map(\|e\| *e as usize).max()` (`:26`), `let e = e as usize;` (`:34`) | sign | `BitSet` positions are `i32` cast to `usize` with no sign check; a negative position makes `num_words = max/64+1` a huge allocation and `words[e/64]` an out-of-range index — bounded only by an unenforced "positions ≥ 0" invariant | `wire.rs`: `bitset_to_bytes(&[-1])` | process | **panic-kill** | Terminal | R1 |

**Rows 1–2 were refuted, and the refutation is worth more than the rows were.** Neither `u16` count can be
large — the outer is capped at `MAX_BLOCK_DEPLOYS = 255` and **the inner is identically 0** — so the
truncation cannot fire (E7). But the reason the inner is 0 is a finding of its own: **the mergeable-channel
mechanism is inert** (E8) — `EvaluateResult.mergeable` is empty at every construction site and the reducer's
own collection is **dead code**, a divergence from the Scala oracle. So the codec's missing trailing-bytes
check is a live trap behind a dormant feature: wire the collection up and E7 becomes real.

**L3's second mandate returned a negative result, and it is worth as much as a finding.** It swept the
**unflattened** fallible-conversion family — `try_into`/`try_from`/`.parse` whose error is discarded by a
*other* spelling (`.ok()`, `unwrap_or(MAX)`) — found **~20 sites and no defect**: every `.ok()` is mapped to
an explicit refusal at its call site, and every `unwrap_or(i64::MAX)` is a deliberate **saturation
sentinel**, not an erasure to zero. Stated as a negative result for L2 rather than as a miss — and it is the
kind of result the refutation stage exists to protect.

**L3's input domain.** It read ~70 of the 358 sites to adjudication and judged the rest from the census
plus a target-width/leaf-source heuristic, naming each heuristic bucket (byte-masked casts; `enumerate` over
a fixed 256-array; `usize → i64/f64` from a parsed term's length; the 21-site `len() as i32` block bounded
by the parser's guards). It **refutes** the guarded ones explicitly so the sweep is not read as a miss
(`reduce.rs`'s index block, `block_api_impl.rs`'s clamps, `par_spatial_matcher_utils.rs`'s early return,
`system_processes.rs`'s `MAX_GOV_UNIVERSE`, `rspace/state/instances.rs`'s already-validated `skip`, the
LEB128 and 32-byte-input cases). Unread in budget: `pretty_printer.rs:279-732`, `compiler.rs`,
`murmur_hash3.rs` (intentional wrapping), the diagnostics crates, `ocapn`'s guarded lengths, and the
`casper/genesis` · `interpreter_util` · `qucalc` tail. **No repro was executed** — each is a named test or
`file:line`, so every row is a candidate. And the load-bearing qualification: **the width rows (#3, #4) do
not fire on x86-64 at all** — `usize` is 64 bits there.

**L3 count: 9 candidates over 16 cast sites — 4 narrowing, 3 width, 3 sign, 1 float. R1 × 6 · R2 × 3.**

**Convergence, recorded because two lenses reached one family from different doors.** L3 rows the width
sites *and* states they cannot fire on the shipping x86-64 target; L6 owns the wasm32 target where they do,
and its `take`/`slice`/`substring`/`indexOf` rows are the same `usize` narrowing seen from the target's
side. **They agree, and the agreement is the adjudication question:** whether wasm32 is a *shipped* target
decides whether the width family is live (R1) or latent (R2/R4). L3 asserts it is supported (PR #187/#188
closed the reducer seams); nothing in the tree says the reducer *runs* anywhere yet. That is the refutation
stage's first job.

### L2 — silent defaulting and erasure

**The filter that *is* the lens, and the size it removed.** `unwrap_or(x)` on an **`Option`** is total —
there is no failure to erase — while on a **`Result`** it destroys one, and only the type says which. L2
resolved that by **reading each receiver**, and removed **~220 of 270** `unwrap_or*` sites (map `.get()`s,
`Iter::next`, `.first()`) plus every `.ok()` that feeds a subsequent `ok_or_else` (an explicit error
survives). Production counts after the gate's own `#[cfg(test)]` strip: `.ok()` **62**,
`unwrap_or_default()` **47**, `unwrap_or*` **270**, `let _ = `/`let _x = ` **96**, `drop(…)` 10,
`if let Err(_)`/`Ok(_)` **0**.

| # | site_key | construct | claim | repro | reach | failure | class | disp |
|---|---|---|---|---|---|---|---|---|
| 1 | `casper/src/multi_parent_casper.rs :: revalidated_record(&stored, outcome.ok())` (`:601`) | erasure/ok | `outcome` is `Result<BlockMetadata, ValidateError>`, and `ValidateError` carries an **`Internal`** (a failed checkpoint replay) *beside* `ValidationFailed`. `.ok()` collapses both to `None`, which `revalidated_record` (`:640`) reads as "the block still fails validation" — so **an internal fault is recorded as a validation verdict**, the failure record is retained, the proposer stays stuck past the spent sequence number, and the `Internal` reason is **never logged**. The branch is acted on | `:1241`'s existing test drives only `revalidated_record(stored, None)`; propose `a_revalidation_internal_error_is_not_a_validation_verdict` | process | silent-wrong-value | **Terminal** | **R2** |
| 2 | `casper/src/multi_parent_casper.rs :: outcome.ok()` (`:723`) | erasure/ok | the same erasure on the **restore** path: an internal error silently means "the restore did not take", **spending an attempt against `RESTORE_ATTEMPT_LIMIT`** | named line; #1's falsifier covers it | process | silent-wrong-value | Terminal | R2 |
| 3 | `casper/src/bonds_parser.rs :: let _ = std::fs::write(bonds_file_path, content);` (`:83`), `let _ = write_private_key(&sk_file, …)` (`:64`) | discard/let_ | the write error is discarded and `new_validators` still returns `Ok(bonds)`. A node that cannot write the bonds file proceeds as if it had — and on the next boot the missing file sends `parse_or_generate` (`:46`) down its `Err(_)` arm and **mints a fresh validator set with different keys**. The sibling discards the **secret-key** write the same way | named lines; `parses_bonds_file` covers the read path only | config | silent-wrong-value | **Split** | **R2** |
| 4 | `node/src/runtime/node_runtime.rs :: std::fs::read_to_string(&marker).ok()` (`:2321`) | erasure/ok | F-U8-01's exact shape on a **non-async** read, so the gate's `.await` pattern cannot see it: a marker that exists but cannot be read is read as *absent*, and the `None` arm then adopts the directory on weaker evidence — or **writes the marker** asserting the dir is the configured shard's | named line; the module's own `FailingBlockStore` (`:2368`) is the harness idiom | config | silent-wrong-value | Split | R2 |
| 5 | **49 sites** — `rholang/src/system_processes.rs:1740`, `casper/src/merging.rs:1198`, `rspace/src/native_store.rs:334`, … (rspace 10 · rholang 14 · ocapn 12 · casper 5 · node 6 · shared 3) | default/Result | **the poison counter sees 3 of 49.** `rspace`'s counted `rlock`/`wlock`/`mlock` are `pub(crate)`, and the direct `.lock()/.read()/.write().unwrap_or_else(\|p\| p.into_inner())` pattern occurs **49 times in production** — every one recovering a torn state with no log and no increment — so the surface reads `0` on a node that took a panic inside, e.g. `system_processes.rs`'s block-data lock or `merging.rs`'s merge cache. **Converges with L5's rows 15–16, which measured 8; this measures 49** | poison a direct site and assert `poison_recoveries()` moves — it does not | process | silent-wrong-value | Drift | **R2** |
| 6 | `casper/src/genesis/mod.rs :: if let Some(validator) = parse_validator_hex(key) { trusted.insert(…) }` (`:60`) | erasure/ok | `parse_validator_hex` ends in `ModelsValidator::try_from(..).ok()`, so a `pos-multi-sig-public-keys` entry that is non-hex or the wrong length is **silently dropped from the genesis `trusted` set** — no log, no startup error — and that set decides who may be trusted and bonded | named line; config flows from `configuration/hocon.rs:343` | config | silent-wrong-value | Split | R2 |
| 7 | `comm/src/transport/transport_layer_syntax.rs :: let _ = transport.send(peer, msg).await;` (`:28`) | discard/let_ | `send_to_peer` returns `()`, so an outbound peer-send failure is **unobservable to every caller** (`send_to_bootstrap`, `node_running.rs:242`, `:273`) — the error cannot surface at all | the existing test asserts the success path only | process | silent-wrong-value | Drift | R2 |
| 8 | `casper/src/gateway/mod.rs :: self.deployed.lock().ok()?.get(sig).cloned()?` (`:98`) | erasure/ok | a **poisoned** `deployed` lock is erased into "no recorded shard", so a phase deploy is **routed by an absence the lock never asserted** — a routing decision, not a guard recovery | named line | process | silent-wrong-value | Drift | R2 |
| 9 | `node/src/effects/console_io.rs :: let _ = stdin().read_line(&mut line);` (`:51-54`, `:148-151`) | discard/let_ | a failed `read_line` leaves `line` empty, so `read_password` returns `""` and the caller cannot tell a read failure from an empty entry | named lines | tool | silent-wrong-value | Drift | R3 |
| 10 | `shared/src/store.rs :: entries().map(\|e\| e.len()).unwrap_or(0)` (`:21`) + `shared/src/lmdb.rs :: let Ok(txn) = begin_ro_txn() else { return 0 }` (`:101`) | default/Result | three erasures all producing **`0` for `num_records()`**, reached through `typed_store.rs:135`'s `count()` — a diagnostic, so the acted-on-ness is a metric | named lines | process | silent-wrong-value | Drift | R3 |
| 11 | `comm/src/transport/grpc_transport_client.rs :: let _ = timeout(..).await.map_err(\|_\| CommError::TimeOut);` (`:151`), `let _ = task.await;` (`:167`) | discard/let_ | the code **builds a `CommError::TimeOut`** on the failure arm and then throws the whole `Result` away; each spawned stream task's join is discarded too | named lines | process | dropped-task | Drift | R3 |
| 12 | `casper/src/protocol/client.rs :: serde_json::to_string_pretty(value).unwrap_or_default()` (`:166`) | default/Result | the callers (`:290`, `:322`, `:385`, `:449`, `:502`) wrap it in `Ok(..)`, so a serialization failure is served as an **empty-string success** | named line | tool | silent-wrong-value | Historic | R3 |
| 13 | `casper/src/blocks/block_processor.rs :: available_parallelism().map(\|n\| n.get()).unwrap_or(4)` (`:23`) | default/Result | an `io::Result` error silently defaults the per-batch validation width to 4 — a perf knob, not a decision | named line | process | contained | Historic | R4 |
| 14 | `ocapn/src/conn.rs :: let _ = conn.send(&abort.to_syrup().to_bytes()).await;` (`:1289`) | discard/let_ | the failure to tell a peer we are aborting is dropped; the peer is likely gone | named line | process | contained | Historic | R4 |
| 15 | `casper/src/blocks/block_receiver.rs` (`:542`, `:688`), `lfs_block_requester.rs` (`:154`, `:192`, `:229`), `rholang/src/reduce.rs:2320` | discard/let_ | `let _ = out_tx.send(hash)` / `request_tx.send(false)` / `tx.send(…)` discard an mpsc/oneshot send's `Result`; a dropped receiver is indistinguishable from success, and the task exits anyway | named lines | process | contained | Historic | R4 |

**Checked and deliberately not filed** (so they are not re-found): `.ok()` feeding a subsequent
`ok_or_else`, where an explicit error survives (`comm/src/peer_node.rs:154`, `ocapn/src/netstring.rs:42`,
`rholang/src/system_processes.rs` ×6, `casper/src/merging.rs:1398`); documented-and-contained
(`shared/src/compression.rs:26`, `shared/src/base16.rs:31`, `models/.../casper_message.rs:116`);
**`Option` receivers that appear on the census list but are total** (`sdk/src/dag/merging.rs` all,
`models/src/block_metadata.rs:293`, `casper/src/blocks/block_receiver.rs:206`, `comm/src/upnp/gateway.rs`,
`rspace/src/runtime_manager.rs:1061`); and comments that merely *quote* a pattern
(`block_receiver.rs:274`, `lfs_block_requester.rs:176`), dropped as prose.

**L2's input domain, and the honest gap.** There is **no census for this family**, so "15 candidates" has no
denominator: nothing says the gate's awaited shape is the *whole* class, and any erasure spelled outside
these forms — `.map_err(|_| Default::default())`, a `match` arm returning a fabricated value, a derived
`Default` *constructed* to mean "absent", a `#[serde(default)]` on a consensus field — is invisible to grep
and to this lens. `Result`-vs-`Option` was resolved by **reading each receiver**, so a `Result` behind a
re-export or an `impl Future` boundary could be mis-filed. Reachability is behavioural, not a graph: whether
the resulting `trusted` set (#6) can differ in effect, or whether the `tool`/`config` sites ever run on a
public testnet, is **unresolved** — and `dead` vs `config` for a partially-wired surface is exactly what
`-A dead-code` (workspace-wide) makes the compiler unable to say. The async/`spawn` boundary is not
modelled: several discards sit inside spawned closures whose handles are awaited elsewhere.

**L2 count: 15 rows (two aggregating 49 sites), 13 refuted/contained sites stated, R2 × 8 · R3 × 4 · R4 × 3.
The two to ship first are #1/#2 (validation and restore) and #3 (the genesis write).**

**And the lens's own correction to *this audit's* rule — a finding about C251.** The `silent` class C251
added matches `\.await…\.ok\(\)` as a **chain**. Rows #1 and #2 are `.ok()` on a binding **two lines below**
the `.await`, in `multi_parent_casper.rs` — the *same file* C251 was written from — and both are on the
validation path. So the rule is narrower than the class it names, which is precisely the false clean §5 of
this artifact predicts.

### L4 — refinement escape hatches

**The verdict on the checked class is negative, and reproduced.** No `impl … Deref … for` exists anywhere in
the workspace, no `pub fn get` sits inside an impl naming a rostered type, no rostered name has a public
tuple field, and no un-allow-listed narrow construction exists. The `escape` class's zero **holds**.

**But the tree is clean in places *by absence, not by check*** — and that distinction is the lens's real
product:

| form | finding |
|---|---|
| `into_inner` / `into_*` on a refinement | **none exist.** The gate does not check this form, so the green here is *the absence of the method*, not a rule. The only `into_*` are on non-refinements (`native_store::into_map`, `shard_invoke::into_value`, `graphz::into_string`) |
| `pub fn get` on a non-rostered type | 14 sites, all containers or services (`key_value_cache`, `maybe_cell`, `env`, `compiler`, `capacity::Bounded`, `accounting::CostAccounting::get`). None carries an invariant |
| `#[allow(…)]` in this family | none; the 50+ `#[allow]`s are `too_many_arguments`/`dead_code`/`non_snake_case`. No `arithmetic_side_effects` allow |
| `Default` on a refinement | only `KeySegment`, `FreeCount` and `Sorted`, all in-domain |
| `Deref`/`AsRef`/`Borrow`/`Index` on a refinement | none workspace-wide; no mutable accessor in any roster file |
| derived `Deserialize` | a real construction-bypass hazard, and **closed**: `FreeCount` uses `#[serde(try_from = "i32")]`, and `Validator`/`StateHash`/`Sorted`/`Blake2b512Random` hand-write it through the validator |

| # | site_key | construct | claim | repro | reach | failure | disp |
|---|---|---|---|---|---|---|---|
| 1 | `comm/src/peer_node.rs :: pub struct NodeIdentifier { key: Vec<u8> }` + `pub fn new(key: Vec<u8>) -> Self` (`:20-26`) | escape/roster-blind-brace | a private-field wrapper the escape gate **cannot name at all** — brace form (G1 no), non-roster file (G2 is roster-scoped), no same-file `TryFrom` (G1v no), not in `REFINEMENT_TYPES` nor `REFINEMENT_EXEMPT`. Its **unvalidated `new` is the constructor on every wire path**; the one validating constructor (`from_hex`) is config-only | `grep TryFrom comm/src/peer_node.rs` → rc 1; the gate's own lists do not name it | ingress | contained | R3 |
| 2 | `comm/src/discovery/mod.rs :: pub fn to_peer_node(node: &Node) -> Result<PeerNode, CommError>` (`:24-34`) | ctor-no-validator | wraps `NodeIdentifier::new(node.id.clone())` and `String::from_utf8_lossy(&node.host)` **unvalidated**, checking only the ports — while its sibling `PeerNode::from_node` applies the host bound (**R32**). Reached from a **remote Kademlia `Lookup` response**, with the per-entry size uncapped (only the *count* is, at `MAX_CONNECTIONS = 1024`). **The under-validation is pinned as intent by a test**: `mod.rs:87`'s `.expect("a bad host is not an error")` | `grpc_kademlia_rpc.rs:123`'s `filter_map(\|n\| to_peer_node(n).ok())`; contrast `peer_node.rs:113`'s `MAX_HOST_BYTES` and its `a_host_over_the_bound_is_refused_at_the_wire` | ingress | contained → memory-amplification | **R2** |
| 3 | `crypto/src/private_key.rs :: pub struct PrivateKey(pub Vec<u8>)` (`:19`) | public-field-unrostered | a public tuple field on a key wrapper — invisible to the name-scoped public-field pattern (unrostered), to G1v (no same-file `TryFrom`) and **absent from `REFINEMENT_EXEMPT`** | `grep "pub struct PrivateKey(pub"` | process | contained | R4 |
| 4 | `crypto/src/public_key.rs :: PublicKey::new(bytes: Vec<u8>) -> Self` (`:19`) | ctor-no-validator | a **total constructor on wider-than-domain input** (§1.7 allows a total constructor only on *already-valid* input); a public key is length-constrained everywhere else (`Validator` = 65) — already registered in `REFINEMENT_EXEMPT` | `grep "pub fn new" crypto/src/public_key.rs` | ingress | contained | R4 |
| 5 | `crypto/src/hash/blake2b512_block.rs :: pub struct Blake2b512Block { … }` + `from_bytes` (`:58`, `:281`) | roster-blind-brace | a private-field state type in a **non-roster file** with a validating `from_bytes`, while its sibling `Blake2b512Random` **is** exempted — **coverage is asymmetric** | the gate's lists name `blake2b512_random` and not `blake2b512_block` | process | contained | R4 |
| 6 | `casper/src/dag.rs :: i64::from(m.height)` (`:1750`) | from-in-domain | §1.7 permits the `From<R> for T` discharge **only at a declared boundary** (prost/wire, FFI, external API) — and **nothing checks where it is used**; here a `BlockHeight` is discharged mid-domain | `grep "impl From<BlockHeight> for i64" shared/src/refined.rs` (=`:118`) | process | contained | R4 |

**The roster is the boundary, and the checked forms cannot name a type the roster does not name.** That is
the lens's structural finding, and it enumerated the blind set **four ways** rather than asserting it: G1
(private tuple, workspace-wide) — all accounted, zero unrostered, and the only three non-`pub` tuple
newtypes in the tree are test helpers; G1v (public tuple + same-file `TryFrom`) — no extra hits, because the
only files implementing `TryFrom` for a named type *are* the 11 roster files; **G2 (private brace,
non-roster) — the real blind set**, ~125 types filtered to those carrying a *fallible* constructor and hand-
read, which is how rows 1 and 5 were found; and macro-generated types plus `pub(crate) fn get`, searched and
absent.

**What no form can reach, stated plainly**: a refinement whose validator returns `Ok` unconditionally (the
scan trusts the `TryFrom` *name*), a construction inside a macro defined elsewhere, a construction sharing
a line with `->`, **a brace refinement in a new file nobody added to `REFINEMENT_FILES`**, and the *use* of
`From<R> for T`. **Rows 1 and 5 are exactly the "new file nobody added to the roster" hole, realized** — so
what the lens found is not a defect in a type but a hole in the *roster*.

**L4 count: 6 rows — 1 R2, 1 R3, 4 R4. No R1: no reachable silent invariant-drop was found.** The row to
ship first is #2, and its sibling relationship to R32 is prior art, not a re-find.

### L1 — panics and the counted mass

**Part 1: the zero holds, and the check is the C97 question per allow-listed site.** L1 re-walked all 29
`WHITELIST_PANIC` entries asking, for each, *does a caller supply peer bytes or deploy content?* — and every
one closes with a named guard: the `from_slice` length asserts are fed by `as_bytes()` of typed newtypes or
have a checked `TryFrom` on the wire path (verified in `casper_message.rs`, `event_converter.rs`,
`interpreter_util.rs`); `blake2b256_hash::from_byte_array`'s one `Vec<u8>` ingress is length-checked in
`Event::from_proto`; the `rspace` consume asserts cannot fire from a parsed term because the parser
**refuses a zero-receipt `for()`** (`parser.rs:729-733`); the rest are declared escapes or structural
boundaries. The zero is honest.

**One imprecision recorded, and it is not called a wire defect.** The `Validator::from_slice` entry's reason
says callers pass "fixed-width `PublicKey::bytes()`" — but **`PublicKey` is a `Vec<u8>`
(`crypto/src/public_key.rs:14`)**, so the length is *not* type-guaranteed. Its callers are local-identity,
genesis and `validator_identity` — reachability **`config`**, not ingress. A node configured with a
malformed key panics at propose or genesis: the **C78 shape** (an invariant held by caller discipline rather
than by the type). It earns a row; it does not re-open C97.

**Part 2: the counted mass is almost entirely contained — and one site is a remote panic.**

| site_key | construct | claim | repro | reach | failure | class | disp |
|---|---|---|---|---|---|---|---|
| `rholang/src/reduce.rs :: .map(\|i\| u8::from_str_radix(&s[i..i + 2], 16).ok())` (`:1102`) | index (str byte-range) | `hex_decode` splits the UTF-8 `&str` at byte offsets `i..i+2` for even `i`, guarded only by `s.len() % 2 == 0`. When `s` is **non-ASCII with an even byte length** and a multi-byte char straddles an even offset, the slice is not a char boundary and **panics**. Reached by `"…".hexToBytes()` (`reduce.rs:1211-1222`) on **deploy content** — and no ASCII guard exists anywhere on the path (`hex_to_bytes_cost` only charges) | the deploy `"aéa".hexToBytes()`: bytes `[61 C3 A9 61]`, so index 2 is interior to `é` → `byte index 2 is not a char boundary`. The lexer accepts arbitrary `char`s (`parser.rs:125-136`), so this is reachable — **and now OBSERVED, see §4's E1** | **ingress** | **panic-kill** | **Terminal** | **R1** |

**The classification of the rest, which is the audit's answer for this half.**

- **`div` (74) — contained.** `EDiv`/`EMod` guard `is_zero(&v2)` one line above (`reduce.rs:626`, `:643`);
  `epoch_divisor` clamps `<= 0 → 1`; `minimum_bond`/`normaliser`/`total` each guard zero; the rest are
  `.len() / k`.
- **`overflow` (41) — contained by design.** The four `refined.rs` sites are the `Sub for BlockHeight/SeqNum`
  impls that deliberately return the raw signed difference (the refinement leaving its domain, documented);
  the others' operands are validated lengths.
- **`lax` (10) — contained, reachability `config`/constant.** `unsafe_decode`'s only production callers are
  constants (`interpreter_util.rs:142`'s hard-coded empty-state hash, `rev_address.rs:135`) and genesis key
  material; the one caller that could take an untrusted string is reached only from a `base16::encode` output.
- **`index` (397) — one candidate, the rest guarded**, with the guards named: rspace's radix/export decode
  (via `SerializedNode`/`KeySegment`), the merge decoders' bounds, `syrup`/`netstring`/`locator` peer bytes,
  `native_state`'s length-guarded decoders, the matcher's empty-guards, `blake2b512_random` via
  `SerializedRandom`, fixed arrays, `peer_table`/`hot_store`'s `index < buckets`, `ip.octets()`.

**Two non-candidates named so a refuter need not re-derive them.** `block-storage/src/dag/representation.rs:174`
(`&truncated_hash[..len-1]`) is the **same char-boundary shape** and *would* panic on `find("abcé")` — but its
only production caller runs `base16::decode(hash)` first, which refuses non-hex, so `hash` is proven
ASCII-hex before the slice: **caller-guarded (C78 class), no ingress reproduction — and it becomes a
candidate the moment that guard moves.** And `models/src/wire.rs:25-35`'s `bitset_to_bytes` is **already
registered as latent in C97's pass record**; it is not re-found as new.

**L1's input domain.** Nothing was executed: the candidate's panic is argued from Rust's char-boundary rule
and a byte-layout computation, **not observed** — which is why it is the highest-priority refutation target.
The store-backed and network-backed states were read but not driven (a *live* corrupt store could reach the
"the store is consistent" asserts in `export.rs:105`, `radix_tree.rs:223`, `radix_history.rs:69` — not
claimed, not constructed). Grammar coverage was checked for the string methods
(`slice`/`substring`/`indexOf`/`split`/`replace`/`format`/`capitalize`/`hexToBytes`) and only `hexToBytes`
misaligns, but not every one of the ~200 method arms. The `cast` class was sampled where it touched slicing.
`rspace-bench` and the wasm target are outside the gate's scan and inherited as such.

**L1 count: 1 candidate, 1 recorded imprecision, ~520 counted sites classified contained.**

## 4. The claim ledger

Columns as `n117-audit.md` §6: **claim · source · deciding artifact · verdict · falsifier (direction
stated) · tracking**. `CONFIRMED` names the artifact that decides it; `REFUTED` names the mechanism that
kills it; `NOT-RE-ESTABLISHED` is *not* a clean.

### The instrument claims (this pass's own measurement, not a lens's)

| # | claim | source | deciding artifact | verdict | falsifier (direction) | trk |
|---|---|---|---|---|---|---|
| I1 | the gate's counted baseline is **stale and unread** — 128 sites of drift | §0 | the gate's own run at `b625eef06`: `cast` 358 (base 331), `index` 397 (316), `div` 74 (54); `ratchet_failures` declared and never incremented | **CONFIRMED** | a run whose counts equal the baseline — i.e. a ratchet that is wired | — |
| I2 | the `panic` class's zero is an **allow-list**, and by C97 an allow-list cannot see its callers | §0 | 30 sites, 29 entries; the gate's header: *"a green `OK` here is evidence about the sites this file lists, never about the ingress discipline"* | **CONFIRMED** | an entry whose reason names the caller's provenance, not only the site's shape | — |
| I3 | the gate's site key is **coarser than a site** — 59 keys shared by ≥2 rows, so an entry's evidence window can be vacuous for the second row | §0 | `blake2b512_random.rs:42` and `:240` are the same text 198 lines apart; the census's collision count | **CONFIRMED** | a key that distinguishes the two rows, or an entry whose evidence holds at both sites | — |
| I4 | *(a near-miss, recorded)* the listing/count gap of 3 is **not** a gate defect | §0 | three census rows have no space after the colon (`7:const PRODUCT_SEED: …`); 355 + 3 = 358 | **REFUTED** — the *parser* was wrong, not the instrument | a census row of that shape that the gate also fails to count | — |

### The refuted-or-established cluster claims

| # | claim | source | deciding artifact | verdict | falsifier (direction) | trk |
|---|---|---|---|---|---|---|
| R1 | the **wasm32 width family is live** (so its dispositions stay R1, not latent) | L3/L6 | CI runs the reducer on the target: `.github/workflows/ci.yml:222-233`'s `Reducer wasm32 tests` step (`cargo test -p rchain-crypto/rspace/rholang --target wasm32-unknown-unknown`), with `.cargo/config.toml:83-88`'s configured `wasm-bindgen-test-runner`; `rholang/tests/wasm_reduce.rs` drives `rt.evaluate(…)`, the same module holding the four bypassed methods, with **no cfg gate** | **CONFIRMED** | deleting the `Reducer wasm32 tests` step and leaving only `cargo check` (then the family is latent) | — |
| R1a | the four bypassed methods are reachable on the target but **not exercised by it** | L3/L6 | the wasm corpus's `PRODUCE_AND_MATCH` calls none of `take`/`slice`/`substring`/`indexOf` | **CONFIRMED** (a statement about test input, not reachability) | a wasm test that calls one of them | — |

*(The remaining clusters — the erasure sites, the poison-accounting count, the `u16` mergeable prefix, the
dropped-task set — are in refutation; their verdicts land here.)*

### The confirmed defects, and the claims refutation narrowed

| # | claim | source | deciding artifact | verdict | falsifier (direction) | trk |
|---|---|---|---|---|---|---|
| **E2** | **the poison-accounting gap** — recoveries bypass the counter the surface publishes | L5 (8) **vs** L2 (49) — *disagreeing* | **both counted wrong; the true population is 77 production sites** (80 non-test, 133 tree-wide). L5's 8 = *rspace only* **and** the inline `.lock().unwrap_or_else(\|p\| …)` spelling; L2's 49 = the same narrow regex workspace-wide — its own breakdown sums to **50**, not 49, and it has **no `comm` rows at all**. The narrowness both missed: the **multiline** form, `.write()`/`.read()`/`.wait()` (Condvar), and the `\|poisoned\|`/`\|e\|` closure variants. **No site is counted**: `rlock`/`wlock`/`mlock` are `pub(crate)` (`rspace/src/lock.rs:35`, `:46`, `:57`), so no non-`rspace` site *can* increment; within `rspace` the accessor-using files and the raw-site files are **disjoint**; and no lock *object* is reached by both | **CONFIRMED (77)** | routing any of them through the accessors moves the counter — and the falsifier is red today: poison a `NativeStore` lock and assert `poison_recoveries()` moves; it does not | its own C-row |
| E2a | a poison is *reachable* at the named sites | L5/L2 | the in-guard bodies at every named site are pure in-memory operations (`Vec::push/pop`, map `get`/`insert`, `clone`, `len`) with no index, arithmetic, `expect` or `panic!`, and the heavy work runs **before** or **after** the guard | **NOT-RE-ESTABLISHED** — and deliberately *not* folded into E2 | an in-guard panic source at any named site | — |
| **E3** | the erasure on `revalidated_record(&stored, outcome.ok())` records an **internal fault as a validation verdict** | L2 | `ValidateError::Internal` is constructible **four ways** (`casper/src/multi_parent_casper.rs:855` block summary, `:865` checkpoint replay, `:894` bondsCache, `:907` neglected-invalid), and none of those helpers logs before returning `Err`; `.ok()` collapses all four with `ValidationFailed`; the `None` arm (`:640`) provably carries `validation_failed: true`, so the stall is not cleared at `:601`; and `:723` **spends an attempt before** the record is built | **CONFIRMED** | a log of the `Internal` before the erasure at `:601`/`:723` | one clause **narrowed**: "never logged" fails for the replay sub-case — its cause *is* logged at `interpreter_util.rs:685` (`describe_replay_failure`) |
| **E4** | the genesis bonds/private-key write is discarded and the next boot mints **fresh keys** | L2 | `bonds_parser.rs:42`'s `parse_or_generate` is `match parse(path) { Ok(b) => Ok(b), Err(_) => new_validators(…) }` — total over a missing, unreadable *or* malformed file — and `new_validators` calls `Secp256k1.new_key_pair()` and returns `Ok(bonds)` regardless of the two discarded writes (`:83` the bonds file, `:64` the **private key**) | **CONFIRMED — and scoped narrower than the lens claimed**: the hazard is the **standalone/ceremony** node (`conf.standalone`); a non-ceremony node is protected **twice** (`genesis/mod.rs:128` returns `Ok(None)` when the file is absent, `:130` is strict) | a verified write, or an operator-visible failure | reachability narrowed to `config`, ceremony only |
| **E5** | the `silent` rule is **narrower than the class it names** | L2 | the pattern (`tools/audit-type-system.sh:1322`) is `\.await(?:\s\|\d+\t)*\.ok\(\)`: `(?:\s\|\d+\t)*` consumes whitespace and the scanner's line prefixes but **no intervening code**. Tested both ways: `.await` ending a line with `.ok()` starting the next → **matches**; the two-line binding form → **does not**. `bash tools/audit-type-system.sh silent` reports **green** over `:601`/`:723`. And the C251 commit fixed the *chain* form in those very two functions | **CONFIRMED** | the narrowest correct extension, per the refuter: `let\s+([A-Za-z_]\w*)\s*=[^;]*\.await(?:\s\|\d+\t)*;[\s\S]*?\b\1\.ok\(\)` — a **backreference** that keeps `.await` provenance as the discriminator, additive to the chain alternative, and matching **none** of the other ~60 production `.ok()` sites | — |

**Two lens errors caught by refutation, recorded because the discipline is the point.** L2's breakdown
*sums to 50 while its headline says 49*, and it omits `comm` entirely; and it cited "`interpreter_util.rs:485`'s
`map_err`" as a log that does not exist (`:485` is inside `describe_no_advance`, a pure formatter, and the
file contains no `map_err` at all). Both were found by a refuter doing arithmetic on the claim rather than
reading it — which is what the stage is for.

| **E6** | 13 detached tasks whose death goes unnoticed | L5 | **partly refuted site-by-site, and 6 of 13 stand.** *Refuted with a named observer each*: the block pipeline, via `consume_observed_queue`'s Drop guard reaching the `…_consumer_active` gauge (`block_receiver.rs:574-579` → `node/src/diagnostics/effects.rs:77`); the peer-message router via its consumer's WARN (`node_runtime.rs:1183`) and the unary path's `not_handled("the peer-message router is not running")` (`comm/src/rp/handle_messages.rs:192`); the shard's ingress (`node_running.rs:665`, `:747`); the propose queue (`node_runtime.rs:1713`, `:1827`). **Four of those seven need a *later event* to fire**, which is a caveat, not a refutation. *Live*: `:1261` (`request_all` — no channel that matters), `:380` (discover), `:403` (clear_connections), `:355` (`kademlia_serve`'s **panic** path — its `Err` *is* logged), `:1738` (the autopropose timer's panic path — the designed halt is loud, a panic sets nothing), `comm/src/transport/grpc_transport_receiver.rs:106` (the handshake, indistinguishable from a normal timeout) | **CONFIRMED (6 of 13)** | an observer at any of the six | |
| **E6a** | **there is no mechanism whose job is to notice a detached task's death** | L5 | the absence, established positively rather than asserted: no `JoinSet`/`TaskTracker`/`task::Builder` in `node`, `casper` or `comm` (the only `JoinSet`s are a *local* accept-task set inside the already-awaited `serve_ocapn`, `node/src/api/ocapn.rs:871`, and rholang's reducer); no `is_finished()` outside tests; no `std::panic::set_hook` anywhere; no production `catch_unwind`; no `panic = "abort"` in any `Cargo.toml` | **CONFIRMED** — and it is the cluster's real finding, stronger than any row | a supervisor, a task registry, or a health poll | its own unit |
| **E6b** | whether a dead downstream is *noticed* is a **per-edge accident** | L5 | sibling producers discard the **identical** error where others log it: `out_tx` (`block_receiver.rs:542`, `:688`), `processor_input_tx` (`node_runtime.rs:791`), `validated_tx` (`block_processor.rs:193`) and `tap_tx` (`node_runtime.rs:2994`) are bare `let _ = `, while the router and the shard's ingress log `is_err()`. So the signal exists on some edges and is thrown away on their siblings | **CONFIRMED** | a convention or lint that a closed-channel send is always surfaced | folds into E6a's unit |

### The refutation that found a dormancy instead

| # | claim | source | deciding artifact | verdict | falsifier (direction) | trk |
|---|---|---|---|---|---|---|
| E7 | a `u16` mergeable-length prefix can truncate into a **silent partial merge** | L3 (#1/#2) | **REFUTED — and not by economics.** The codec *has* the property: its decode loop (`rholang/src/merging.rs:259-278`) reads `count` then consumes what it can with **no trailing-bytes check** (unlike the `u32` native-changes sidecar, which refuses them at `:459`). But neither count can be large: the **outer** is bounded by `MAX_BLOCK_DEPLOYS = 255` (`casper/src/blocks/proposer/proposer.rs:622`, enforced in `casper/src/validate.rs:642-651`, and the merge refuses a mismatch at `merging.rs:767-771`), and the **inner is identically 0** — see E8 | **REFUTED** | **wiring the dead `merge_chs` into `EvaluateResult.mergeable`** (E8's one-line port of the Scala reducer's `mergeableChannels`): the per-deploy count then becomes user-controlled, and the refuter's own arithmetic puts 65 536 inside one deploy's budget (holding it out at `MAX_BLOCK_PHLO = 25_500_000_000` would need ≥ 389 099 phlo per channel; the actual cost is a few dozen), at which point the truncation is live and the missing trailing-bytes check makes it **silent** | |
| **E8** | **the mergeable-channel mechanism is inert — a divergence from the oracle** | refutation of E7 | `DeployMergeableData.channels` is `[]` for every deploy: `EvaluateResult.mergeable` (`rholang/src/evaluate_result.rs:16`) is `BTreeSet::new()` at **every one of its 15 construction sites** and is never mutated (play, replay, reporting, casper's two managers), and the reducer's own collection — `update_mergeable_channels`'s `merge_chs` (`rholang/src/reduce.rs:3089`) — is **written and never read anywhere in the workspace** (its only three occurrences are the push sites, and there is no getter). So `get_number_channels_data` returns an empty map and the merge-side fold `for (k, v) in &b.event_log_index.number_channels_data` (`casper/src/merging.rs:2010`) iterates nothing | **CONFIRMED** | a deploy that reports a mergeable channel — which today is impossible | **its own C-row** — it is a `spec/AUDIT.md` §6 **Scala deviation** (the oracle's reducer returns `mergeableChannels`; this port collects them and drops them) *and* the precondition of E7 |

### The confirmed defect, with an observed artifact

| # | claim | source | deciding artifact | verdict | falsifier (direction) | trk |
|---|---|---|---|---|---|---|
| **E1** | **`hexToBytes` panics on non-ASCII deploy content — a deploy panics the node** | L1 | **observed, not argued**: a probe test reduced `"aéa".hexToBytes()` through the real evaluator (`source_to_adt → eval_single_expr`, the same parse→normalize→dispatch the runtime crosses) and **panicked at `rholang/src/reduce.rs:1102:39`** — *"end byte index 2 is not a char boundary; it is inside 'é' (bytes 1..3) of `aéa`"* — unwinding out of a `Result`-returning path with nothing catching it. The trace is complete: deploy `term` → `casper/src/runtime_manager.rs:466` → `rholang/src/runtime.rs:326-355` → `eval_expr`'s `EMethod` arm → `eval_method:1130` → `:1211`'s `hexToBytes` arm → `hex_decode:1096` → **panic**. **Neither gas nor arity guards it**: `hex_to_bytes_cost` is `s.len()` phlo (*4* for `"aéa"`), charged **before** the call and refusing nothing, and the arity check passes at 0. The lexer collects arbitrary `char`s (`parser.rs:122-137`) and the normalizer only strips quotes (`normalizer.rs:78`), so nothing upstream refuses it | **CONFIRMED (observed)** | an ASCII guard on the `hexToBytes` path, or a lexer that refuses non-ASCII in a string literal — either turns the panic into `hex_decode`'s **existing** `ReduceError` arm, which is already the function's own contract | **deserves its own C-row and fix unit** |
| E1a | the twin shape at `block-storage/src/dag/representation.rs:174` is **latent, not live** | L1 | the guard is real and upstream: its sole production caller (`block_api_impl.rs:711`, inside `get_block`) runs `base16::decode(hash)` at `:701` **first** and refuses any non-hex input, so every offset is a char boundary; a workspace-wide grep finds **no second unguarded caller** (the others are unit tests passing hex) | **CONFIRMED (latent)** | a second caller that skips the `base16::decode` guard — the finding's own condition | — |

## 5. What this pass could not see

Each lens's own boundary is stated **with its rows** in §3; this is the list that crosses all of them, and
the common scope is §0's.

- **Almost nothing was executed.** One artifact in this audit is a *run* — E1's probe, which panicked. Every
  other "contained" verdict is a **reading of the guard**, not a re-run of a falsifier: L1 (and so the
  classification of ~520 counted sites), L3's 9 rows, L5's and L6's are all read. That is the honest
  difference between this section and the security audit's *measured* / *read* labels: this pass is
  overwhelmingly *read*, with one exception, and the exception is the one finding that is beyond argument.
- **`-A dead-code` is workspace-wide, so a green build says nothing about whether a knob is wired.** L2
  named three rows (#6 the genesis `trusted` set, #9 the console password, #12 the JSON body) whose
  reachability cannot be settled — is it `config` (live on some deployment) or `dead` (a surface nothing
  calls)? The compiler is explicitly told not to say, and `spec/TYPE-SYSTEM.md` §3.2 records that as a
  standing blind spot. **This is the single gap most likely to change a verdict**, and it is why several
  rows carry a reachability that is a judgement rather than a measurement.
- **No lens drove a live system.** No peer socket, no corrupted LMDB store, no genesis file, no browser tab.
  The sites whose safety is "the store is consistent" (`export.rs:105`'s prefix assert, `radix_tree.rs:223`,
  `radix_history.rs:69`), the races that are only arguable (L5's cancel-safety rows), and L6's "the host
  decides whether the RNG row bites" are all in that class — a corrupt store or a hostile host could
  activate them, and this pass could not construct one.
- **The census's own boundary is inherited by every lens** (§0): `rspace-bench` is never scanned; files are
  excluded from the type gate **by name** (`TEST_ONLY_FILE_RE`); `#[cfg(test)]` blocks are stripped;
  `unsafe`/`silent`/`escape` do not strip comments (so a doc-comment quoting a pattern false-positives);
  `/* … */` is not stripped at all; and **macro-generated code is invisible** (`prost`'s `OUT_DIR` output is
  in `target/`, though the checked-in generated `models/src/casper/protocol/*.rs` *are* scanned).
- **The roster is L4's boundary and it is now the audit's**: for a brace-form refinement in a file nobody
  added to `REFINEMENT_FILES`, *no* form can name the type — which is how `NodeIdentifier` and
  `Blake2b512Block` stayed invisible. L4 enumerated the blind set four ways rather than asserting it, but
  the enumeration itself has the property C97 warns about: **it is a claim about a sweep, not about the
  tree.**
- **Specific per-lens limits, already stated with their rows**: L1 could not enumerate all ~200 method arms
  (only the string methods, of which one misaligns); L2 could not type a receiver behind a generic or an
  `impl Future` boundary and did not model the `spawn` boundary; L3 read ~70 of 358 sites to adjudication
  and judged the rest by named heuristics; L4 read the fallible-constructor subset of ~125 brace-types;
  L5 left three comm spawns unconfirmed rather than refuted; L6 did not compile or run the target.

## 6. The rules it yields

The point of the audit is not the list; it is what becomes **enforced**. Each surviving class takes one of
three routes, and the audit's most important finding about itself is that **one class cannot take the
cheapest route**.

### Adoptable now — hard classes, each with a text-keyed, at-site-evidence allow-list

- **The raw poison recovery.** Forbid `.unwrap_or_else(… into_inner)` on a lock in production, in the
  `WHITELIST_PANIC` entry shape (`<file-suffix>;;<site-regex>;;<evidence-regex>;;<reason>`, text-keyed
  because a line-keyed entry **fails open**). This is the class C249/F-U9-03 *thought* it had closed — the
  counter went on the accessors and the direct pattern was left — and the refutation adjudicated the
  population at **77 production sites** (E2), so the rule is the missing half. **Sequence matters**: route
  the sites through `rlock`/`wlock`/`mlock` (so they are *counted*) in the same unit that adopts the rule,
  or the rule lands with a 77-entry allow-list.
- **The roster's blind scope (L4's finding).** The escape gate's completeness guard derives
  refinement-shaped newtypes in **roster files only** for the brace form (`G2`), which is why
  `comm/src/peer_node.rs`'s `NodeIdentifier` and `crypto/src/hash/blake2b512_block.rs`'s `Blake2b512Block`
  are nameless to it. Extending `G2` to every crate — with the same staleness discipline the other forms
  have — turns "a new file nobody added to the roster" from a hole into a failure. **This is the audit's
  cheapest structural win**: it is a scope change to an existing guard, not a new instrument.
- **The erasure's unambiguous spellings — and the refutation *specified* the extension rather than
  sketching it.** C251's rule matches `\.await…\.ok\(\)` as a **chain**; the same erasure occurs on a
  **binding** two lines below the `.await`, in the file C251 was written from. The narrowest correct form
  (E5) is a **backreference**:

  ```
  let\s+([A-Za-z_]\w*)\s*=[^;]*\.await(?:\s|\d+\t)*;[\s\S]*?\b\1\.ok\(\)
  ```

  It keeps `.await` provenance as the discriminator, is **additive** to the chain alternative, and matches
  **none** of the other ~60 production `.ok()` sites — so it needs **no allow-list** at all. A rule that
  catches two live defects on the validation path with an empty allow-list is the shape this section exists
  to produce.

### The one class that cannot take that route, and what it takes instead

**The defaulting family — `.unwrap_or_default()`, `.unwrap_or(…)`, `let _ = ` on a `Result` — is not
text-gateable, and the audit should say so rather than ship a rule that reads well and fails open.** The
reason is the lens's own filter: `unwrap_or` on an `Option` is **total**; on a `Result` it destroys an
error; and **only the type tells them apart**. L2 removed ~220 of 270 candidates by *reading each receiver*,
which is precisely what a regex cannot do. A pattern over those spellings would either be vacuous or carry
hundreds of allow-list entries — the "gate satisfied by bookkeeping" this repo has already deleted once.

Two levers exist, and the audit assesses them as an **option, not a commitment**:

- **The compiler already has these lints.** `unwrap_used`, `expect_used`, `clippy::panic`,
  `clippy::let_underscore_must_use` exist, are **enabled nowhere** in this tree, and have **never been told
  to look away** either (no `#[allow(unwrap_used)]` anywhere in `src/`). The precedent is `unsafe`: the
  gate's pattern is the *belt* and `#![forbid(unsafe_code)]` at 15 crate roots is the structural half. A
  crate- or module-level lint plus an allow-list is cheaper and cannot go vacuous — but it is a *build*
  change that will surface sites the lenses called contained, so it wants its own unit and its own
  measurement.
- **Per-site adjudication**, which is what this artifact is: 15 rows with artifacts, of which the two on the
  validation path (#1/#2) and the genesis write (#3) are the ones worth a unit now.

### Structural rules (the `check-bounded-ingress-queues.sh` model)

- **A spawned task with no observer.** L5's 13 `dropped-task` rows share one cause: the tree has **no
  supervisor** — no `JoinSet`, no registry, no `is_finished()` poll — so a panic in a detached task is a
  `JoinError` nobody reads and the node reports itself healthy. The rule shape is a **source check**: every
  `tokio::spawn` on the ingress path is either awaited, in a `JoinSet`, or listed with a reason. Stated with
  its cost: there are **88 spawns** in production, so the allow-list is large unless the check is scoped to
  the ingress path (which is where the five worst rows live).
- **`rspace-bench` is never scanned** by the type gate (`NOT_SCANNED`). That is defensible — a bench panic
  is harmless — but it is currently a *silence*, so the boundary belongs in the gate's own output rather
  than in a reader's head.

### Not rules: sites

The **`hexToBytes` char-boundary panic** (L1) and the **`u16` mergeable-length prefix** (L3) are *sites*,
not shapes: each gets a code fix and a named test, not a pattern. The general shape behind L1's — a `&str`
sliced by a byte offset — is real but too noisy to gate (a pattern over `[i..i` would match half the
tree's legitimate byte work); the honest disposition is the ASCII guard plus the test, and the *twin* site
`representation.rs:174` named as the reason.

### Every rule gets a probe pair, run by someone else

No rule from this audit lands on a green run. Each gets a **positive control** (a planted site the rule must
report) and a **negative control** (a near-miss it must not), **executed and recorded**, by an agent that
did not write the rule — because this repo has paid for the absence twice and says so in
`spec/audit/passes.md` §1: *"an instrument that cannot see the defect it names is not evidence — this pass
has now paid for that lesson twice."* The `panic` pattern's `\bassert` never matched `debug_assert!`, and
`scan_escapes` missed the public `get()` the spec names beside `Deref`; both were green, and neither was
found by reading the pattern.

## 7. The verdict

**The counted mass is overwhelmingly contained, the panic zero holds, and what is not contained is small,
severe, and mostly about *boundaries* rather than sites.**

What the six lenses established, in one line each:

- **L1** re-walked all 29 `panic` allow-list entries with C97's question and **the zero holds**; it then
  classified div, overflow, lax and 397 indexings and found guards in nearly every case. The one exception
  is the audit's worst finding.
- **L2** found the erasure class **wider than the rule C251 shipped for it**, with two sites on the
  validation path and one in the genesis ceremony.
- **L3** classified the 358 casts, of which four of nine candidates are wasm-only, and returned a **negative
  result worth as much as a finding**: the unflattened conversion family has no defect.
- **L4** verified the escape class's zero — and found the hole is the **roster**, not a type.
- **L5** found the tree has no supervisor, so a detached task's death is noticed only **by accident**.
- **L6** found the width family, whose shape is a **guard that exists and four siblings that bypass it**.

**The confirmed defects, by severity.**

| # | defect | severity | artifact |
|---|---|---|---|
| **E1** | `hexToBytes` **panics on non-ASCII deploy content** — a deploy panics the node, with gas and arity both cleared | **ingress / DoS** | **observed**: a probe panicked at `rholang/src/reduce.rs:1102:39`, `end byte index 2 is not a char boundary` |
| **E3** | `revalidated_record(&stored, outcome.ok())` records an **internal fault as a validation verdict** — the stall is not cleared, and on the restore path an attempt is spent | validation path | `ValidateError::Internal` constructible four ways (`multi_parent_casper.rs:855`, `:865`, `:894`, `:907`); the `None` arm at `:640` |
| **E4** | the genesis **bonds and private-key writes are discarded**, and the next boot mints fresh keys — **standalone/ceremony only** | genesis ceremony | `bonds_parser.rs:42`, `:83`, `:64` |
| **E2** | the published `poison_recoveries()` sees **0 of 77** production recoveries, so the surface added to end the silence itself under-reports | accounting | the refutation's count, and `rlock`/`wlock`/`mlock` being `pub(crate)` |
| **E6** | **six** detached tasks die silently (of thirteen claimed; seven have a named observer) | process | per-site observers, and their absence |
| **E6a** | **there is no supervisor at all** — the observation that exists is a per-edge accident of whether an author wrote `is_err()` or `let _ =` | structural | the absence: no `JoinSet`, no registry, no `is_finished()`, no hook, no `catch_unwind` |
| **E8** | **the mergeable-channel mechanism is inert** — `EvaluateResult.mergeable` is empty everywhere and the reducer's collection is dead code, a **divergence from the Scala oracle**, and the reason E7 is refuted | dormant feature / oracle divergence | `evaluate_result.rs:16`, `reduce.rs:3089`, `merging.rs:2010` |

**What the audit corrects in the record — its own and the repository's.**

1. **C251's rule is narrower than the class it names** (E5). It matches the *chain* form and misses the
   *binding* form, in the same file it was written from, over two live defects on the validation path. The
   extension is specified and needs no allow-list.
2. **The poison counter C249/F-U9-03 added sees 0 of 77** (E2). The finding was "the recovery is invisible";
   the fix made it visible only on the accessors, and the recoveries do not use them.
3. **The gate's baseline is stale by 128 sites and read by nothing** (I1); its `panic` zero is an
   **allow-list** whose own record says an allow-list cannot see its callers (I2); and its site key is
   **coarser than a site**, so an entry's evidence window can be vacuous for the second row it covers (I3).

**What this pass does not claim.**

- **~520 "contained" verdicts are readings of guards, not runs.** One artifact in this audit is a run (E1's
  probe). The rest is read code, and the honest label is *read*, not *measured*.
- **`dead` vs `config` is unresolved** for three L2 rows, because `-A dead-code` is workspace-wide and a
  green build says nothing about whether a knob is wired. That is the single gap most likely to change a
  verdict.
- **Nothing was driven live** — no peer socket, no corrupted store, no hostile host, no browser tab.
- **The wasm family is reachable but unexercised**: the CI corpus crosses the four methods' module without
  calling them.
- **Three of six lenses found a boundary, not a site** — the roster (L4), the absent supervisor (L5), and
  whether the second target ships (L3/L6). An audit that listed only sites would have reported a shorter,
  wrong document.
