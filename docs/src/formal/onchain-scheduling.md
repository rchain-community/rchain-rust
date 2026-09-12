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
  validates Law 24; a run that fails validation falls back to the whole-run gate re-run, so the
  published state is the sequential fold's (`validated_speculation_refines_apply`, proven). The
  gate re-run is total (`gate_replay_terminates`), a published run commits each path at most once
  (`Published`: each path validates, or aborts at most once and then commits via the path-ordered
  re-run queue), and the fallback's sorted re-run queue preserves that invariant
  (`fallback_rerun_published`, proven).

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
construction. The Rust realization computes exactly this certificate on the block path: the
committed effect's read set is its claimed footprint, and the record layer is the write-record
layer of `rspace::concurrent::channel_queue::ChannelQueue` — `record_write`/`last_write` over a
per-channel `WriteRecord` (writer path = claim path, version = write count, polarity = post-value),
armed by `set_validation_enabled` for `RelaxedValidated` only. `try_acquire` performs the prefix
visibility check under the held channel locks (the gap-free linearization point), rejecting with
`AcquireError::ValidationFailed` — the fail-fast certificate.

The certificate is **not** the acceptance gate by itself: it is a per-commit detector, and the
sequential-oracle legs remain on the accept path. The two checks are complementary, each with a
decided witness: the certificate is the only detector for the C/D pair and the persistent-produce
COMM inversion (oracle-blind — identical multisets and state), while the oracle is the only
detector for a DFS-earlier writer committing *after* a DFS-later reader
(`certificate_blind_late_writer_diverges` — both commits validate, yet the commit-order fold
diverges from the gate fold). A certificate failure fails *fast* into the sequential fallback
without running the oracle; certificate-clean runs still pass all three oracle legs.

## The witnesses

The witness theorems pin the check's shape, all proven in
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

Three further boundary witnesses — each a decided counterexample — settle the corrected
statements below:

- `dispatched_serializable_log_inequality` — literal log equality between the commit-order fold
  and the path-order fold is false even for dispatched, serializable runs (two independent
  produces committed out of path order). The published log is the *path-ordered drain*; the
  publication theorem must relate each commit's trace to the gate's per-path trace.
- `certificate_blind_late_writer_diverges` — the certificate's blind spot above.
- `writer_chain_needs_nodup` — one path committing twice on different channels is serializable,
  yet the writer path list repeats; the writer chain needs the path-nodup hypothesis (the
  coordinator's `Published` invariant).

## The publication theorems (proven)

- `serializable_writer_chain` — prefix visibility forces each channel's writers to commit in
  strictly increasing path order, from the initial (empty) record layer and with path-nodup.
- `dfs_serializable_implies_log_equal` — the pinned publication theorem: a dispatched,
  path-nodup run whose commits are **per-channel path-pinned** (`Pinned`: each channel's commits
  already appear in path order — what dispatch-time pre-claiming gives within one dispatch list)
  reaches the gate fold's state, and each commit emits exactly the gate's trace at its path; the
  published log — the path-ordered drain — is the gate's trace, event for event. The pin is the
  load-bearing hypothesis.
- `validated_speculation_refines_apply` — the coordinator refinement: on the accept path the
  oracle's own verdict is the state equality, on the fallback path the deploy set re-runs under
  the gate from the start state (which is the gate fold by construction); either way the
  sequential fold ships. The supporting lemmas — `record_determines_value`, `dispatched_record_at`
  (a dispatched commit only ever writes its own channel), `gate_replay_terminates`, and
  `fallback_rerun_published` (the path-sorted re-run queue keeps `Published`) — are proven. No
  axioms remain.

## Rust realization map

| Law element | Lean | Rust |
|---|---|---|
| write-record layer | `SpecState` / `applyAt` | per-channel `WriteRecord` (path, version = write count, polarity) in `rspace::concurrent::channel_queue.rs`: `record_write`/`last_write`/`reset_write_record`, armed by `set_validation_enabled` |
| prefix visibility | `prefixVisible` / `ValidCommit` | the certificate in `try_acquire` — `AcquireError::ValidationFailed` rejects a commit whose read is not prefix-visible, under the held channel locks (the gap-free linearization point) — **plus** the sequential-oracle legs on the accept path (`RuntimeManager::validate_relaxed_block`: post-state hash + per-channel COMM multisets + Law 11 rig-replay) |
| DFS-serializability | `DFSSerializable` | the certificate + oracle pair above: the certificate catches the C/D pair and the persistent-produce COMM inversion (oracle-blind); the oracle catches the late-earlier-writer divergence (certificate-blind) |
| abort / inverse-replay / gate re-run | Law 25 (`Published`, `gate_replay_terminates`, `fallback_rerun_published`) | per-deploy fallback: `SpeculationInvalid` reverts to the deploy's soft checkpoint and re-runs it sequentially; whole-set safety net: `compute_state` re-runs the deploy set on `fork_play_runtime` from `start_hash` |
| published log = sequential fold | `validated_speculation_refines_apply` | block carries the sequential trace either way: the oracle-verified relaxed trace on accept, the gate re-run's trace on fallback |

The block path therefore runs: speculate under the claim queue (Law 20) with dispatch-time
pre-claiming pinning per-channel commit order, fail fast into the sequential fallback on a
certificate rejection (Law 24's check), validate certificate-clean runs against the sequential
reference (the oracle legs), and on divergence ship the reference's results (Law 25's gate
re-run) — pure `Relaxed` remains off-chain only. The realization (the `RelaxedValidated`
block-path mode, the write-record layer with its `try_acquire` certificate, dispatch-time
pre-claiming, per-deploy fail-fast with the whole-set safety net, and the `validate_relaxed_block`
oracle) is implemented on `feature/channel-scheduler`; the phase-by-phase account is
[the on-chain validation phase plan](../contributor/onchain-validation-phases.md).
