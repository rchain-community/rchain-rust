# Cross-shard transactions: two-phase commit (Laws 26–29)

[Cross-shard invoke](shard-invoke.md) specifies the *primitive*: a remote deploy is an ordinary
caller-signed deploy submitted to a **different shard**'s deploy service, and `deployerId` is
shard-independent, so the same key is the same account on every shard. That primitive can *reach*
another shard, but it cannot make a pair of legs **atomic**: the escrow example
([`qucalc/examples/shard_exchange.rho`](../../../qucalc/examples/shard_exchange.rho)) is explicitly
non-atomic — two idempotent legs plus a joint-check monitor, with the client orchestrating the order
and retrying on crash.

This page specifies the **transaction layer** that lifts that primitive to atomicity: a
**two-phase commit (2PC)**, framed as a **Git pull request**. It is formalized in
[`spec/Rchain/CrossShard.lean`](../../../spec/Rchain/CrossShard.lean) as Laws 26–29.

## The Git pull-request analogy

A cross-shard transaction is a PR that spans two (or more) branches — one per shard.

| Git | Transaction |
|-----|-------------|
| open a PR | the initiator opens a transaction (`txn_id`), naming the shards and legs |
| push commits to each branch | each participant runs **PREPARE** — it locks its resources and votes |
| CI / review | the coordinator collects the votes (all `ready` = green) |
| merge | the coordinator records **COMMITTED** and sends COMMIT to every participant — the atomic merge commit |
| close without merge | the coordinator records **ABORTED** and sends ABORT; each participant compensates |

The merge commit is a single, content-addressed decision: after it exists, every participant has the
same terminal outcome. That is the atomicity the primitive lacks.

## Roles

- **Coordinator** — a gateway that can sign deploys to every participant shard (the "merger"). It
  owns a durable, content-addressed **transaction record**.
- **Initiator** — the client that opens the transaction (the "author").
- **Participant** — a shard whose state the transaction spans (a "branch"); usually two: source and
  target.

## States

```lean
inductive TxnState where
  | proposed | prepared | committed | aborted
```

- `proposed` — the coordinator recorded the transaction's legs (the PR is open).
- `prepared` — every participant has locked its resources and voted (both branches staged).
- `committed` — the coordinator decided COMMIT (the merge).
- `aborted` — the coordinator decided ABORT (the close).

## Messages

Each message is a signed remote deploy (reusing the
[`shard_invoke`](shard-invoke.md) primitive), **idempotent under `txn_id`** so re-delivery and client
retry are safe (Law 28).

| Message | Effect on the participant |
|---------|---------------------------|
| `PREPARE(txn_id, leg)` | lock the leg's resources; write a vote (`ready`/`abort`) into the shard's content-addressed state |
| `VOTE_READY(txn_id)` / `VOTE_ABORT(txn_id)` | the vote the coordinator observes |
| `COMMIT(txn_id)` | apply the effect, release the lock |
| `ABORT(txn_id)` | run the compensation, release the lock |

## The decision and recovery

The coordinator collects votes under a timeout:

- **all `ready` ⇒ COMMIT** — record `committed`, send COMMIT to every participant.
- **any `abort` or timeout ⇒ ABORT** — record `aborted`, send ABORT to every participant.

A `prepared` participant that loses the coordinator **holds its lock** and recovers the decision from
the coordinator's durable record. That is the classic 2PC **blocking** caveat — the model is *atomic*
(no mixed outcome) at the cost of a blocking recovery window; there is deliberately no non-blocking
3PC or presumed-abort, because those trade atomicity for liveness. The honest consequence is Law 29:
the record must be durable, so recovery always terminates.

## The four laws

- **Law 26 — shard scope determinism.** A deploy/block's effects bind to exactly one shard; the shard
  id is a validated, ordered value, and the RNG seed + unforgeable names are shard-scoped. This is
  the *sharding* half: the id is the typed `ShardId` newtype (`shared/src/refined.rs`) — non-empty
  ASCII, ordered, with the `parent-shard-id`/`shard-name` hierarchy realized by
  `CasperConf::full_shard_id` (`/root` for the default `root` shard, `/root/rootchild` for its child)
  — and the boundary is enforced at deploy admission and block validation (`casper/src/validate.rs`).

- **Law 27 — cross-shard atomicity (2PC).** A transaction commits on every participant or aborts on
  every participant — no run leaves a strict subset committed.

- **Law 28 — leg idempotency.** `prepare`/`commit`/`abort` are idempotent under `txn_id`.

- **Law 29 — decision durability & record determinism.** The coordinator's decision is a durable,
  content-addressed record; a `prepared` participant can always recover it, and the merge/close
  record is re-derivable on replay (the Laws 11/16 content-addressing analog).

## The formalization

The Lean model defines the shard id, the lifecycle, a leg (a shard's effect + its compensating
inverse), the transaction, the coordinator record, and a run's terminal configuration:

```lean
abbrev ShardId := String
def ValidShardId (s : ShardId) : Prop := s ≠ ""

inductive TxnState where
  | proposed | prepared | committed | aborted

inductive Vote where
  | ready | abort

structure Leg where
  shard : ShardId
  effect : Nat          -- opaque content-addressed effect handle
  compensation : Nat    -- opaque content-addressed compensation handle

structure Txn where
  txn_id : Nat
  legs : List Leg

inductive Outcome where
  | committed | aborted

structure CoordRecord where
  txn : Txn
  state : TxnState
  votes : List (ShardId × Vote)

abbrev Run := List Outcome
```

The four laws are **stated** (their proofs are a later obligation, as Laws 20–25 were before their
proofs):

```lean
axiom shard_scope_deterministic (l : Leg) : ValidShardId l.shard

def uniform (r : Run) : Prop :=
  (∀ o ∈ r, o = Outcome.committed) ∨ (∀ o ∈ r, o = Outcome.aborted)

axiom txn_atomic (r : Run) : uniform r

def applyEffect (st : ShardId → Nat) (l : Leg) : ShardId → Nat :=
  fun s => if s = l.shard then l.effect else st s

axiom leg_idempotent (st : ShardId → Nat) (l : Leg) :
  applyEffect (applyEffect st l) l = applyEffect st l

axiom commit_record_deterministic (r : CoordRecord) :
  r.state = TxnState.committed ↔ (∀ v ∈ r.votes, v.2 = Vote.ready)
```

`txn_atomic` is the load-bearing statement: every run is *uniform* — all committed or all aborted —
which is exactly "no partial merge". `commit_record_deterministic` pins the decision rule (commit iff
every vote is `ready`) and makes the record a deterministic function of the votes, so recovery and
replay re-derive it.

## Rust realization (status)

- **Participant** (`rholang/src/system_processes.rs` `rho:txn`, `rholang/src/native_state.rs`
  `PREFIX_TXN`): the per-shard REV escrow — `prepare` (lock + vote), `commit` (apply), `abort`
  (compensate), `recover` (query), idempotent under `txn_id` (Law 28) and coordinator-gated via
  `RhoDeployerId` (the `bond` capability pattern).
- **Coordinator** (`casper/src/txn_coordinator.rs`): the client-side driver — `txn_term` builds the
  phase terms and `TxnCoordinator::run_2pc` signs → submits → collects the `prepare` votes → commits
  all or aborts the prepared legs (Law 27). The transport is the existing
  `casper/src/shard_invoke.rs` primitive (`invoke_term` → `signed_invoke` → submit → `await_reply`).
- **Shard scoping** (`shared/src/refined.rs` `ShardId`, `casper/src/conf.rs` `full_shard_id`,
  `casper/src/block_random_seed.rs`, `casper/src/validate.rs`,
  `casper/src/api/block_api_impl.rs`): the shard id is a validated, ordered newtype in the RNG seed
  and the unforgeable names, the full id is derived from the `parent-shard-id`/`shard-name`
  hierarchy, and the boundary is enforced at admission/validation.
- **Deferred** (specified here, not yet built): only the *multi-shard gateway*. The coordinator is an
  *off-chain client* that signs deploys to each shard; a node that is itself a member of multiple
  shards (the "gateway") remains out of scope for `rnode`.

The end-to-end two-shard path — uniform commit, and abort on partial failure — is exercised by
`casper/tests/cross_shard_txn.rs` over a `DeployService` test double.
