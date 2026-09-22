# Running a validator: hardware requirements

A validator is **storage-bound and uptime-bound**, not CPU-bound. The compute envelope is modest; the
binding constraints are the synchronous-write storage layer (LMDB) and being online continuously. This
page gives a grounded sizing estimate derived from the code's storage envelope and execution model —
there is no in-repo production benchmark, so treat the numbers as an estimate, not a measurement.

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

Tracked as [#60](https://github.com/rchain-community/rchain-rust/issues/60), which carries a
self-contained reproduction (a memory-capped `systemd-run` unit) and separates what is measured from what
is still unattributed.

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
