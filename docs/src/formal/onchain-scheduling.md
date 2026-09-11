# On-chain scheduling: validated speculation (Laws 23–25)

[The channel scheduler](channel-scheduling.md) (Laws 20–22) ships the sound schedulers — the
per-channel claim queue and the DFS gate — and the *relaxed* mode, whose free cross-channel
interleaving may diverge from the sequential reducer's final state and event log. That is why the
casper block paths hard-reject `Relaxed`: a relaxed trace must never reach a block's event log.
This page specifies the extension that makes effect-level concurrency *sound on-chain*, formalized
in [`spec/Rchain/SchedulerOnchain.lean`](../../../spec/Rchain/SchedulerOnchain.lean):

- **Law 23 — read-determinism.** An effect's chosen candidate, commit outcome, and event trace
  are a deterministic function of the state it reads: states agreeing on the effect's *closure*
  yield identical traces (`read_state_determines_outcome`). This is the executable counterpart of
  Law 8's content-addressed COMM and the strengthened Law 9 (`effect_commute_of_disjoint_closure`):
  it is what lets a *checked* interleaving stand in for the sequential run.

- **Law 24 — DFS-order serializability.** A concurrent execution is sound for the block path iff
  every commit read exactly the state the DFS-earlier effects produced. The Bool-presence `State`
  of `Rchain.Effect` cannot state this — the depth-2 pair's stale read has the same Bool value as
  the correct one — so the law adds the **versioned write-record layer**
  `SpecState := Chan → Option (DfsPath × Nat × Bool)`: each write records its writer's DFS path,
  a per-channel version, and the value the channel held *after* the write (the polarity makes the
  record witness the data: `record_determines_value`). A commit validates when every channel it
  reads is **prefix-visible** — its newest write was recorded by a DFS-earlier path.

- **Law 25 — validated speculation.** A scheduler may commit effects in any order iff each commit
  validates Law 24; invalidated subtrees abort, inverse-replay their writes, and re-run under the
  gate, so the published log is the sequential fold's (`validated_speculation_refines_apply`).
  The gate re-run is total (`gate_replay_terminates`), and a published run commits each path at
  most once (`Published`: each path validates, or aborts at most once and then commits via the
  path-ordered re-run queue).

## The versioned write-record layer

The data layer (`State := Chan → Bool`) tracks datum presence. The record layer tracks, per
channel, its newest write — writer path, version, and post-value:

```lean
abbrev Write     := DfsPath × Nat × Bool
abbrev SpecState := Chan → Option Write
```

A produce records an addition write (post-value `true`); a matched consume records a removal write
(post-value `false`); a missing consume writes nothing. Versions increment per channel, so the
newest write is the latest committer. The two layers are kept coherent by construction:

- `applyAt_preserves_invariant` / `record_determines_value` (proven): in any reachable state, a
  channel's value **is** its newest write's recorded post-value (or the initial value if
  unwritten). This is what makes the record a *witness* — where two Bool-equal reads differ in
  which data they matched, their records differ in writer path.

## Prefix visibility: the check

```lean
def prefixVisible (s : SpecState) (p : DfsPath) (c : Chan) : Prop :=
  (s c).elim True (fun w => PathLt w.1 p)

def ValidCommit (sRead : SpecState) (p : DfsPath) (reads : Finset Chan) : Prop :=
  ∀ c ∈ reads, prefixVisible sRead p c

def DFSSerializable (st0 : State × SpecState) : List (DfsPath × Effect) → Prop
  | [] => True
  | (p, e) :: rest =>
      ValidCommit st0.2 p (Effect.footprint e) ∧ DFSSerializable (applyAt p e st0) rest
```

`DFSSerializable` steps the state in *commit* order and validates each commit against the record
the commit actually read — no separate snapshot is carried, so the check is faithful by
construction. The Rust realization *target* computes exactly this certificate on the block path: the
committed effect's read set is its claimed footprint, and the record layer is the per-channel
version counter of `rspace::concurrent::channel_queue::ChannelQueue` (writer path = claim path,
version = commit counter).

## The witnesses

The four witness theorems pin the check's shape, all proven in
`spec/Rchain/SchedulerOnchain.lean`:

- `s3_pair_fails_validation` — the depth-2 pair's B-first interleaving is **not** serializable:
  `[0,0,0]`'s produce reads channel `0` whose newest write is `[1]`'s removal — a DFS-later path.
  The scheduler aborts A's subtree and re-runs it under the gate.
- `s3_sequential_order_valid` — the DFS order itself validates: this is the run the sequential
  reducer produces, and the run the gate re-run restores.
- `later_write_pollution_unsound` — the draft "every DFS-earlier write is reflected" check is
  blind to a DFS-earlier read observing a DFS-*later* write that committed first (the C/D pair);
  only prefix visibility rejects it.
- `trace_equality_without_serializability` — the publication check is **one-directional**: the
  produce-only pair publishes the same trace as the path-order fold yet fails validation. Log
  equality is a necessary consequence, never a sufficient certificate.

## The publication theorem

`dfs_serializable_implies_log_equal` (stated): a serializable run publishes the sequential fold's
trace — the commit-order fold equals the path-order fold. The proof obligation is the writer-chain
lemma (`serializable_writer_chain`, stated): prefix visibility forces each channel's writers to
commit in strictly increasing path order, so the commit-time data state equals the path-order
fold's, and Law 23's read-determinism lifts state equality to trace equality. The supporting
lemmas — `record_determines_value` and `dispatched_record_at` (a dispatched commit only ever
writes its own channel) — are proven.

## Rust realization map

| Law element | Lean | Rust |
|---|---|---|
| write-record layer | `SpecState` / `applyAt` | per-channel version counter in `rspace::concurrent::channel_queue.rs` |
| prefix visibility | `prefixVisible` / `ValidCommit` | certificate check at the block-path gate (`casper::runtime_manager::RuntimeManager`, the `RelaxedValidated` mode) |
| DFS-serializability | `DFSSerializable` | per-deploy validation: recorded commit order vs path order |
| abort / inverse-replay / gate re-run | Law 25 (`Published`, `gate_replay_terminates`) | soft-checkpoint rollback (`RSpace::revert_to_soft_checkpoint`) + sequential re-run on `fork_play_runtime` |
| published log = sequential fold | `validated_speculation_refines_apply` | block carries the sequential trace on fallback |

The block path therefore runs: speculate under the claim queue (Law 20), check the certificate
(Law 24), and on failure re-run under the gate (Law 25) — pure `Relaxed` remains off-chain only.
The Rust realization of the certificate (the `RelaxedValidated` block-path mode, the per-channel
version counters, and the abort/re-run coordinator) is the implementation target of this spec.
