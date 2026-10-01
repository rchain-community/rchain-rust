# Running a validator: hardware requirements

A validator is **storage-bound and uptime-bound**, not CPU-bound. The compute envelope is modest; the
binding constraints are the synchronous-write storage layer (LMDB) and being online continuously. This
page gives a grounded sizing estimate derived from the code's storage envelope and execution model —
there is no in-repo production benchmark, so treat the numbers as an estimate, not a measurement.

This page answers **"how big a box"**. For **"how fast, and what stops it"** — the throughput ceiling,
finality latency, merge cost and the Θ(N²) residency that sets a validator's time-to-ceiling — see
[Scaling and performance limits](scaling.md).

## What drives each dimension

### Storage — the biggest variable

The LMDB environments are sized in `casper/src/storage.rs` (`rnode_db_mapping`):

| Store | LMDB map size |
|---|---|
| `blockstorage` (blocks) | 1 TB |
| `dagstorage` (DAG / metadata / deploy index) | 100 GB |
| `rspace/history` + `rspace/cold` (tuple-space trie) | 1 TB each |
| `eval/history` + `eval/cold` (REPL eval store) | 1 TB each |
| `reporting` (event log) | 10 TB |
| `deploypoolstorage` / `transaction` | 1 GB each |

These are **sparse mmap reservations**, not actual usage — an LMDB file only grows as data is written.
Real disk usage scales with chain length, on-chain state size (the RSpace trie grows with contracts,
vaults, and unforgeable names), and deploy event-log volume. The `reporting` store is the largest
reservation but is gated off by default (`enable-reporting = false` in `node/src/configuration/defaults.conf`).

### CPU — replay determinism is the real cost

Every block a node validates is **re-executed**: `replay_compute_state` runs each deploy + system
deploy against the recorded COMM trace, and proposing also executes deploys. Block validation is
parallelized across blocks (cross-block replay parallelism), so this scales with core count — it is the
main reason to give a validator more vCPUs than a first estimate suggests.

### RAM — mmap'd LMDB + in-memory DAG/hot-store

LMDB is mmap-backed (large virtual address space, but resident memory is bounded by the working set).
The genuinely in-memory pieces are the hot store and the DAG representation. The worst case is protocol
message buffering — `max-message-consumers = 400` with blocks up to
`grpc-max-recv-stream-message-size = 256M` is ~100 GB in theory, but steady-state is far lower.

### RAM again — start-up replay is the floor, and it is not a steady-state number

What a node needs *to start* is a different quantity from what it needs *while running*, and it scales
with how long the chain is. Measured on a chain of **1142 blocks whose persisted state is 4.5 MB**
(21 files under the data dir):

| | RSS | API reachable after |
|---|---|---|
| fresh chain (≤6 blocks) | 18–20 MB | 15 s |
| replay of the 1142-block state, read-only | 57 → 284 MB (oscillating, not monotone) | 225 s |
| the same replay with `-s --dev-mode --propose-on-deploy` | 52 → 285 MB | 210 s |
| an isolated chain *producing* blocks (`--autopropose` + dummy deploy) | 20 → 23 MB across 131 produced blocks | — |

So start-up costs roughly **0.25 MB of RSS and ~0.2 s per existing block**, while *producing* blocks
costs almost nothing (~0.02 MB/block). The start-up cost is the in-memory view being rebuilt:
`BlockMetadataStore::create` (`casper/src/block_metadata_store.rs`) reads every block's metadata and
rebuilds the DAG state, `BlockDagKeyValueStorage::create` (`casper/src/dag.rs`) rebuilds the child map,
height map, message graph and fringe states, and `BlockIndex` entries accumulate in a process-global
cache (`casper/src/merging.rs`).

What this means for sizing:

- **A 1 GB host is fine for a few hundred blocks but can be unable to restart a ~1000-block chain** while
  anything else runs on the box: the kernel OOM-kills rnode mid-replay, and since every restart replays
  the same DAG it loops rather than recovering. The unit still reports `active`, so the visible symptom
  is an API that never answers.
- Budget **≥ 2 GB for ~1k blocks, ≥ 4 GB to be comfortable**, and expect roughly **30 s of unavailability
  per 150 blocks** after every restart before the API answers at all.
- Replay is silent: no progress output, no readiness signal. If a node has not answered shortly after
  start on a long chain, check whether the process's RSS is *growing* before treating it as hung — and do
  not tighten the restart loop, which only repeats the replay.
- A chain that produces blocks continuously (`--autopropose` plus the dev-mode injector, as the devnet
  does) reaches a given height in an afternoon, and therefore reaches this limit quickly; a chain that
  only grows with real traffic stays in the safe zone much longer. Steady-state production is cheap — the
  cost is all in start-up.

**Correction, 2026-09-28: the per-block figures above are measurements, but the rule drawn from them is
not linear, and the ceiling it implies is wrong at height.** Everything above is stated as a rate — "~0.25
MB RSS and ~0.2 s per block" — which reads as a constant to multiply by a block count. It is not: the
DAG's per-message ancestry sets are Θ(N²) in total, so the *marginal* cost of a block rises with how many
blocks are already in the DAG. Three measured points, and they do not lie on a line:

| measured | per block, resident |
|---|---|
| 131 produced blocks (the table above) | ~0.02 MB |
| 100–300 blocks, fresh devnet, 2026-09-28 (`tools/devnet-bench.py`) | 0.16–0.23 MB |
| **3 200 blocks** ([#68](https://github.com/rchain-community/rchain-rust/issues/68)) | **~1.8 GB peak, ~17 minutes before the API opens** — *pre-`b37410b2e`, and **unreproduced** on this lineage; see below* |

**Correction, 2026-09-29: the third row is an upper bound, not a measurement of the current build.**
There were **two** Θ(N²) terms in the start-up path and one of them is gone: the metadata-index rebuild
(`recreate_in_memory_state`) used to fold through `add_block_to_dag_state` — a full `DagState` clone per
block — and now extends the index in place, which is what `b37410b2e` landed. The ancestry-set term above
remains, and it is the one this page's rule is about, so the figure still bounds the cost from *above* —
but the peak and the start-up time are both now unknown at 3,200 blocks.

**Decision, 2026-10-01 (#154): the row is *unreproduced*, not re-measured — and the statement replaces the
sizing point rather than waiting for one.** Two pre-registered rounds ran that day and neither reached the
ceiling:

| rig | crossing | widest merge scope | peak |
|---|---|---|---|
| no-load, both trees (`n127-directed-results.md`) | **0 of 3**, on the control *and* the fixed arm | **9 chains** | 49–66 MiB against a 3000 MiB threshold |
| loaded, four bounded deploys + `stop 2` (`n127-loaded-results.md`) | **0 of 3**, on either arm | 161–174 chains after the fix, 29 pre-quotient | 647–689 MiB control vs 31–45 MiB fixed |

The no-load rig cannot reach the ceiling because **the input it needs no longer occurs**: the 32–43 chain
scopes the 1.8 GB figure was measured at were a DAG shape the proposer fix (#138) moved, and on this
lineage the widest scope is 9 chains. The loaded rig does reach 161–174 chains — and still peaks 647–689 MiB,
4–5× below the 3000 MiB threshold. The honest reading is therefore **not "the ramp is fixed" but "there is
no ramp to be gone on this build"**, and the 3,200-block row is a property of a tree this repository no
longer builds. It is kept above as history, marked as such, and **it is no longer a figure to size from.**

What survives as advice is only what a host has to hold for a *start-up* on a chain of the height intended:
the ancestry term is still Θ(N²) and still unbounded, so **expect start-up to get worse superlinearly and
treat a few thousand blocks as the point where a small host stops fitting** — but the ~1.8 GB figure is not
evidence for where that point is, because nobody has measured it since the tree changed.

**The two statements are not in tension, and the difference is worth keeping straight.** The 2026-09-29
correction's *upper bound* still stands: a term was removed, so the current cost is **at most** the old one.
That is a ceiling on a number nobody has re-taken — it is not a sizing point, and "provision for 1.8 GB at
3,200 blocks" reads a bound as a measurement. The row above is kept because it is the last thing anyone
measured on this path; it is marked unreproduced because that is now all it is.

**And one line of the list above is now stale: there *is* progress output.** A node indexing its store
logs a line every 250 blocks (`casper/src/merging.rs`, commit `43a549e9c`), and a node speaks as it starts
serving — but **readiness is still only implicit**: the API binds after replay completes, so a client sees
connection-refused, which is indistinguishable from a crash. Bounding the rebuild is tracked as #68; the
start-up cost is otherwise unbounded and this section is the only place it is written down.

Tracked as [#60](https://github.com/rchain-community/rchain-rust/issues/60), which carries a
self-contained reproduction (a memory-capped `systemd-run` unit) and separates what is measured from what
is still unattributed.

### Resident memory is not only what the node holds — glibc's arenas

glibc gives each thread its own malloc **arena**, and an arena keeps the high-water mark of what it has
held; it is not returned to the OS. A node under fork load spreads allocation across its worker threads,
so what the process retains is the *sum* of the per-thread peaks rather than any one peak. Measured on a
3-validator devnet ([#117](https://github.com/rchain-community/rchain-rust/issues/117)), the anonymous
regions are **29 of exactly 32 MiB** — the worker **stacks** (`thread_stack_size(32 MiB)`), holding
5.7 MiB of RSS between them — and **11 of exactly 64 MiB**, which *are* the arena heaps, while the main
`[heap]` sat at 1.9 MiB and file-backed RSS at 26.8 MiB.

**That paragraph used to read the same histogram backwards**, as "29 regions of exactly 64 MiB, one per
worker thread, fully resident": a count taken from one size class and a size from another, with the
residency inverted — 29 × 64 MiB is 1.86 GiB, more than the whole anonymous RSS of that run. It is
corrected here because the audit of #117 says so — C176 of that audit's register, §27 of its pass record.

**And what reaches the ceiling is held, not returned.** Read from inside the running node by the
allocator's own accounting, `retained` was **0** in every one of ~1,700 samples across nine node-runs,
with allocated bytes explaining 97–99 % of the cgroup's `anon` at peaks of 3.1–8.2 GiB. So a node that
dies at its limit is dying of memory it still holds, not of a leak and not of purge lag — which is why
the fix space is the code, and why the arena cap below is not the answer. The audit's §27 carries the
instruments, and the one composition question they leave open.

**Capping the arenas is unproven and is deliberately not shipped.** `MALLOC_ARENA_MAX=2` was tried, and
the survival counts at a 4 GiB ceiling do not separate it from noise — nor are they reproducible from the
artifacts: the earlier version of this paragraph quoted "1, 2 and 1 surviving nodes of three", of which
one figure is on disk and the other two are not, while at the same setting an arm *carrying* the cap was
the one that lost 2 of 3 nodes. Do not set it on the strength of this page. What is worth knowing is the
mechanism — and its measurement limit: a probe intended to read the arena count directly could not sample
the nodes that mattered, because a killed node's cgroup is gone and the peak then reads 0 (that audit's C176); a node
cannot set the cap for itself, because `mallopt` and `malloc_trim` are `unsafe` and this
crate graph is `#![forbid(unsafe_code)]`, so the only lever is a runtime environment variable — and
whatever is done about it has to be measured against a reproduction whose run-to-run spread is first
characterised, because single runs of this shape disagree with each other.

### Network — low bandwidth, latency-sensitive

gRPC + Kademlia discovery over ~20 batch peer connections; blocks are capped at 256 MB streams and
deploys at 16 MB. Bandwidth is modest; consensus wants **stable, low-latency** connectivity more than
raw throughput.

## Recommendation

| | Minimal (testnet / low traffic) | Comfortable (mainnet validator) |
|---|---|---|
| CPU | 2–4 vCPU | 8+ vCPU |
| RAM | 8 GB | 16–32 GB |
| Disk | 100–250 GB SSD | 1 TB+ NVMe SSD |
| Network | 10 Mbit+ stable | 100 Mbit+ low-latency |

The single most important choice is **NVMe SSD for the data dir**: LMDB is synchronous-write-heavy and
the RSpace trie + block store perform many small writes, so a spinning disk dominates the cost.

## In practice

Any modern general-purpose desktop or laptop with an NVMe SSD is enough to *run* a validator — the
CPU/RAM needs are modest. Four caveats matter more than raw specs:

1. **Uptime, not horsepower.** A validator that sleeps, hibernates, or is shut down falls out of sync
   and (once slashing/epochs are enforced) is penalized. A laptop runs it fine; a laptop you carry
   around and close is a bad *validator*. This is the real reason for an always-on box or VPS — not
   because the CPU is insufficient.
2. **Disk capacity grows.** The 1 TB reservations are not needed up front, but a busy shard's RSpace
   trie + block store + deploy event logs accumulate. A 128–256 GB SSD is fine to start and eventually
   fills on a heavily used chain.
3. **RAM has to cover the restart, not just the run.** Sizing on steady-state RSS alone produces a node
   that works fine until it is restarted, and then cannot come back on a small host. See the start-up
   replay section above.
4. **Bare-metal storage beats a nested container.** A Chromebook-class machine (8 threads / 8 GB)
   would *run* the node, but only inside its Linux container behind a hypervisor and an I/O layer —
   exactly the wrong substrate for LMDB's many-small-writes pattern, and it may not expose the NVMe
   directly. A cheap always-on box running bare Linux is a better validator than a faster laptop in a
   container.
