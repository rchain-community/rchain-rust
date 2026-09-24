# The laws

RChain's behaviour is pinned by **laws** — one invariant per layer of the system. Each law maps a
language or system feature to its formalization: a Lean theorem, a Coq axiom, the executable K rule, and
the Rust realization. This page is the full set, in the order the register numbers them, with each set
saying **what it is about**.

> **The checked rendering is [`spec/LAWS.md`](../../../spec/LAWS.md)** — emitted from `Rchain/Laws.lean`
> and refused stale by the gate. It is the authority for every law's **status** (and for the per-row
> history: `spec/LAWS.md`, [`spec/INVENTORY.md`](../../../spec/INVENTORY.md) and `spec/AUDIT.md` each
> carry a part of it). It counts *clauses* rather than laws and separates **`proved-tied`** (proved *and*
> tied to the node by a conformance corpus) from **`proved-model`** (proved about a model a human keeps
> in sync); the prose on this page is the narrative reading of the same rows, and where the two
> disagree, the register is right. Its total is **<!-- counts:proved-laws -->42<!-- counts:end --> of
> <!-- counts:laws -->49 laws<!-- counts:end --> proved at all — <!-- counts:proved-tied-laws -->14<!-- counts:end --> tied to the node by a corpus,
> <!-- counts:proved-model-laws -->28<!-- counts:end --> over the model — with one more law proved but *vacuous* (its statement restates its own
> definition, and the register has a word for that).

## The set, and what each part of it is about

| Laws | Set | What it is about | The narrative |
|---|---|---|---|
| 1–6 | **Rholang** | the calculus itself: canonicalization, α-equivalence, substitution, reduction, matching, closedness | [Substitution and matching](substitution-matching.md) |
| 7–11 | **RSpace** | the tuple space: joins, the comm event, merge, the Merkle trie, replay | [The concurrency model](concurrency.md) |
| 12–13 | **Rosette** | the actor VM — orphaned: `rosette`/`roscala` is out of scope for this port | — |
| 14–19 | **Casper, storage, crypto** | finality and the fringe, the DAG's monotonicity, block validity, the merge's arithmetic, the height map, the cryptographic primitives | [Determinism of the block state transition](determinism.md) |
| 20–25 | **The scheduler** | effect scheduling: the per-channel claim queue, the DFS gate, and the on-chain validated-speculation layer | [Effect scheduling](scheduling.md) |
| 26–29 | **Cross-shard** | two-phase commit across shards: scope, atomicity, idempotency, the durable decision record | [Cross-shard transactions](cross-shard-transactions.md) |
| 30–43 | **The surface** | what a client writes and a matcher reads: grammar, lexing, normalization, matching, reply shapes, the JSON envelope | *(below)* |
| 44–47 | **The validator lifecycle** | the Proof-of-Stake epoch: the boundary gate, the reward split, its conservation, staged withdrawal | *(below)* |
| 48–49 | **Charging** | what a denied deploy does to the merged state, and what a matched deploy is charged | *(below)* |

The last three sets are the ones a graph of the *calculus* does not reach. Rows 30–43 exist because every
defect that started the formalisation programme (AUDIT C9–C22) lived there and **nothing errored**: an
unmatched `for` is not an error, a wrong reply shape is not an error, a swapped connective is not an
error — each read as a client bug, and each was found on a running node. Rows 44–49 exist because the
port had a validator lifecycle, a merge and a gas account and no law for any of them.

---

## Rholang — the language (Laws 1–6)

| Law | Invariant | Surface feature | Lean | Coq | K rule |
|---|---|---|---|---|---|
| **1** | canonicalization is idempotent & commutative: `sort(sort p)=sort p`, `sort(p\|q)=sort(q\|p)` | the sorted `Par`, commutative `ESet`/`EMap` | `Sort.lean` — `sortPar_idempotent` / `sortPar_comm` (the element-comparator laws behind them are theorems, not axioms) | `Sort.v` — `sortPar_idempotent` / `sortPar_comm` (axioms) | normalization (α + canonical `\|` sort) |
| **2** | α / name equivalence: par order, `\| Nil`, associativity, top-level arithmetic, α, `@`/`*` | `@`/`*`, `@{P\|Q}=@{Q\|P}` | `Rho.lean` — `StrCong` `≡` | `Laws.v` — `alpha_equiv` (an inductive mirroring `StrCong`; with de Bruijn *levels* deep α is equality, so no track owes it) | `name-equivalence.k`, `alpha-equivalence.k` |
| **3** | capture-avoiding de Bruijn substitution; `sort(subst t)=subst(sort t)` | variable binding | `Subst.lean` — `substPar` (a `mutual` family mirroring `rholang/src/substitute.rs`), `sort_subst`, `subst_closed` (given the closed image) | `Laws.v` — `substPar`, `subst_commutes_sort` (axioms) | `free.k` (substitution; `substitution.k` referenced) |
| **4** | reduction (COMM), first-match-wins, `new` freshness | send/receive, `match` | `Rho.lean` `Reduce`; `Concurrent.lean` `reduce_redex_unique` (an isolated redex reduces to its body, uniquely up to `≡`) and `reduce_not_deterministic` (the flat `Par` is *not* confluent, which is why the block path's determinism comes from the scheduler, Laws 20–25); `Reduce.lean` `reduce_freeVars_subset` | `Laws.v` — `reduce` (axiom) | `processes-semantics.k`, `sending-receiving.k`, `persistent-sending-receiving.k` |
| **5** | spatial matching, defined; an accepted match binds each free level at most once | patterns, `_`, `~`, `/\`, `\/` | `Match.lean` — `spatialMatchCore`/`spatialMatch` (a fuel-carrying matcher, so "does it match" is *computed*), `spatialMatches`, `spatialMatches_decidable` (an instance, not an axiom), `spatialMatch_implies_linear`, `linear_of_pathPar`. Law 37's tie — that the clauses decide equality on the shapes they have an arm for — is that row's theorem (`spatialMatches_iff_eq`, over `pathPar`) | `Laws.v` — `spatial_matches` (axiom); `linear` is a **definition** with `linear_decidable` and the witness `a_double_binding_is_not_linear` | `matching-function.k`, `specific-matching-rules.k`, `exact-matching-function.k`, `matching-with-par.k` |
| **6** | no globally free variables | `Closed` | `Ty.lean` `Closed` + `FreeVars.lean` `freeVarOf` (a `mutual` block mirroring the checker) and `closed_iff_no_freeVars` | `Laws.v` — `closed`, `closed_decidable` (axiom) | `free.k`, `program-restrictions.k` |

The reader-facing story — substitution, the matcher, and what silence means — is
[Substitution and matching](substitution-matching.md); the sort discipline the parser feeds is
[Grammar and sorts](grammar-sorts.md).

## RSpace — the tuple space (Laws 7–11)

| Law | Invariant | Feature | Lean |
|---|---|---|---|
| **7** | join commutativity (channel keys hashed in sorted order) | multi-channel receive | `RSpace/Join.lean` — `joinKey` is *defined* as hash-of-sorted-channel-hashes, so `joinKey_perm` is law 1's canonicalization applied to a join key |
| **8** | deterministic COMM (produce refs sorted; content-addressed events) | the comm event | `RSpace/Comm.lean` — `produceRefs`/`commId` defined over the Rust's own sort, `comm_content_addressed` |
| **9** | merge is a monoid; non-conflicting logs commute | state merging | `RSpace/Merge.lean` — `mergeChanges` defined (added/removed concatenate, the join map is right-biased overwrite), `NonConflicting` defined, `mergeChanges_assoc`/`_comm` |
| **10** | Merkle determinism (content-addressed radix trie, collision-free, empty root) | history | `RSpace/Merkle.lean` — `nodeHash`/`emptyNode`/`emptyRoot` defined over the node type the code has, `encodeNode_injective` over `WellFormed`, `root_collision_free`/`nodeHash_eq_emptyRoot` |
| **11** | replay determinism (recomputed COMM ⊆ recorded trace) | replay | `RSpace/Comm.lean` — the recorded trace is an **input** (`Replays recomputed recorded`), and the check holds exactly when the two agree on occurrences (`replays_iff_same_occurrences`); neither half suffices, each with a witness, and the second witness is the port's own guard — `check_replay_data_with_fix` drops the reverse half on a failed deploy (RCHAIN-3505) |

## Rosette — the actor VM (Laws 12–13)

| Law | Invariant | Status |
|---|---|---|
| **12** | actor atomicity (single-threaded `mbox.nextMsg`) | **orphaned** — `rosette`/`roscala` VM is out of scope |
| **13** | reflection (everything is an `Ob`; meta/parent chain; fork-join barrier) | **orphaned** |

## Casper / Storage / Crypto (Laws 14–19)

| Law | Invariant | Feature | Lean |
|---|---|---|---|
| **14** | finality requires **> 2/3** bonded stake; fringe = one message per validator | the fringe | `Casper/Stake.lean` — `calculateFringe` is the port's `calculate_fringe` (full-partition filter, non-bonded skip, exact-integer comparison) and `Fringe.lean`'s `finality_iff_supermajority` is the advance gate, with the 2/3, 2⁵³ and `i64` boundary tests; the antichain is proved **over the derivation** (`Casper/Dag.lean`: the walk, the min messages, the count gate and the layer fold, in the port's own order) — stated *about the fold* rather than about the `BTreeMap` the port stores, so that a one-word mutation (`layerInsert`'s filter dropped) makes it false rather than merely unproved |
| **15** | fringe monotone by height; seen-set monotone | the DAG | `Casper/Fringe.lean` — the port *constructs* a message's seen set (`seenOf`, with `seenOf_contains_justifications`/`mem_seenOf_self`); the closure the finalizer leans on is proved along justification reachability (`Reaches`/`seen_monotone_of_reaches`); what is owed is the *height* half, and the id-level form needs the DAG's own closure invariant stated |
| **16** | block number = max(parent)+1; seqNum strictly +1; content addressing; bonds cache = PoS | blocks | `Casper/Validate.lean` — the two number laws are the port's own checks (`BlockNumberValid`/`SeqNumValid`, folds over the justifications seeded `-1`) with the laws as their elimination; `content_addressing` follows from `blake2b256_collision_free` and the serializer's canonicity, and the serializer is a **definition** (`encodeBody` over protobuf's wire format, `Rchain/Proto.lean`) with `Canonical` load-bearing. The ties to `prost`'s bytes stay prose (no crate is vendored): the `body` conformance layer is the named follow-up |
| **17** | the merge's arithmetic is the checked 64-bit one — a value that would leave `i64` is **refused, not wrapped** — and the merged RNG is a function of the *set* of branch generators | merges | `Merging.lean` — `checkedAdd`/`checkedSub` with their refusal witnesses, `mergeRandoms_perm`. Numeric channels are signed `i64` and negative diffs are ordinary (`rholang/src/merging.rs:161-166`), which is why the non-negativity form was not the law |
| **18** | height map contiguous; fringe identity order-independent | storage | `Casper/Validate.lean` — `Contiguous` is the store's invariant with `contiguous_insert_succ`/`contiguous_skip_leaves_hole`, and `fringeId`/`fringeId_perm` give the order-independence the `BTreeSet` supplies |
| **19** | Blake2b256 canonical; the `Blake2b512Random` merge is n-ary and **order-sensitive**; sig verify/sign; Curve25519 round-trip | crypto | `Crypto/Random.lean` `mergeRandom` (n-ary, mirroring `merge(children: &[Self])`); `Crypto/Spec.lean` `blake2b256_collision_free`, `blake2b256_output_is_32_bytes` (the width the code's `Hash32` types), `sign_verify_roundtrip`, `curve25519_roundtrip` — **the register's axiom set is this row's cryptographic boundary, by design**. Order-independence is stated where it holds: at the *call site*, `Merging.lean`'s `mergeRandoms_perm` |

The state transition these laws constrain, and what determinism means for it, is
[Determinism of the block state transition](determinism.md).

## The scheduler — effect scheduling (Laws 20–25)

| Law | Invariant | Feature | Lean |
|---|---|---|---|
| **20** | channel-task linearization ("1 channel = 1 logical task"): a per-channel claim queue keeps same-channel commits in DFS path order; the path-smallest pending claim is always committable | the claim queue | `Scheduler.lean` — `queue_commit_path_ordered`, `pathSorted_head_minimal` (the bakery argument's core). The *global* liveness claim needs the finiteness the real system has: `PathLt` is not well-founded on paths, so an unbounded pending set need have no minimum |
| **21** | DFS-gate linearization: running effect `i` only after effects `0..i−1` complete is exactly the sequential apply fold; the one-hop (next-step-footprint) variant is unsound | the gate scheduler | `Scheduler.lean` — `gate_await_closure_orders` (the immediate-predecessor await chain is transitively complete, which is the Rust's "not the quadratic all-predecessors join"), `one_hop_depth2_diverges` (counterexample) |
| **22** | next-step closure is computable at dispatch (the matched datum is concrete); computability does *not* make cross-channel pruning sound | dispatch-time closure | `Scheduler.lean` — `depth2_next_step_disjoint` (the half with content). The positive half is a *signature* fact (`resolve_children` takes no store); the tempting distributivity law is false for the Rust's flattening order |
| **23** | read-determinism: an effect's outcome and event trace are a deterministic function of the state it reads | on-chain speculation | `SchedulerOnchain.lean` — `read_state_determines_outcome` |
| **24** | DFS-order serializability: a concurrent run is sound iff every commit read the state the DFS-earlier effects produced (versioned write-record layer; prefix visibility) | the validation certificate | `SchedulerOnchain.lean` — the witnesses, `record_determines_value`/`dispatched_record_at`, `serializable_writer_chain` and `pinned_run_publication`. The publication theorem carries **no** certificate hypothesis: the unused argument and the linter suppression that hid it are both gone |
| **25** | validated speculation: commits may reorder iff each validates Law 24; invalidated subtrees abort and gate-re-run, so the published log is the sequential fold's | the abort/re-run coordinator | `SchedulerOnchain.lean` — `published` + `published_state_is_the_oracles` (accept only on agreement with the oracle, else ship the oracle's result — so the certificate decides *how much* is re-run, never *what* is published), `Published` + `fallback_rerun_published` |

The reader-facing story — the claim queue, the DFS gate, the relaxed mode, the unsound one-hop variant,
and the on-chain extension — is [Effect scheduling](scheduling.md).

## Cross-shard — two-phase commit (Laws 26–29)

| Law | Invariant | Feature | Lean |
|---|---|---|---|
| **26** | shard scope determinism: a deploy/block's effects bind to exactly one shard; the shard id is a validated, ordered value; the RNG seed + unforgeable names are shard-scoped | the shard boundary | `CrossShard.lean` — the law's real content is the **ingress**: `admitLeg` mirrors `node/src/web/http.rs:203-218` (non-empty and ASCII shard id, non-blank recipient) and `the_admitted_leg_carries_a_validated_shard_id` states all three conjuncts, with the two boundaries — the amount is deliberately *not* checked at the ingress, and the recipient predicate is over code points — as theorems |
| **27** | cross-shard atomicity (2PC): a transaction commits on every participant or aborts on every one — no run leaves a strict subset committed | the coordinator | `CrossShard.lean` — the decision half is proved: `allReady`/`coordinatorDecision` mirror `txn_coordinator.rs:177-179` and `coordinator_decision_committed_iff` gives `committed` iff every vote is ready. The universal form over a freely constructible `Run` was **false** and is refuted (`txn_atomic_is_false`) |
| **28** | leg idempotency: `prepare`/`commit`/`abort` are idempotent under the transaction id | re-delivery / retry | `CrossShard.lean` — `leg_idempotent` |
| **29** | decision durability & record determinism: the coordinator's decision is a durable, content-addressed record; a prepared participant recovers it; re-derivable on replay | the merge/close record | `CrossShard.lean` — the decision half is proved (`coordinator_decision_committed_iff`); durability is the half with a record, and it is named rather than assumed |

The reader-facing story — the Git pull-request flow, the roles, the state machine, and the 2PC recovery
caveat — is [Cross-shard transactions: two-phase commit](cross-shard-transactions.md).

## The surface a client writes and a matcher reads (Laws 30–43)

Fourteen rows, `spec/INVENTORY.md` 30–43, covering the grammar, the normalizer, the matcher's patterns,
the reply shapes and the JSON. They close the same way the calculus rows do — a Lean declaration, a
corpus emitted from it, a Rust consumer held to the same cases — with one addition that matters: the
gate ([`tools/check-lean-conformance.sh`](https://github.com/rchain-community/rchain-rust/blob/dev/tools/check-lean-conformance.sh))
refuses a stale corpus, an unimported module, a `sorry`, and a corpus with no consumer, so a law that
stops holding is noticed by a machine rather than by a user.

| # | Law | Where it lives |
|---|---|---|
| 30 | every term the parser accepts is in the grammar | `Rchain/Parse.lean` — the grammar as **data** (`grammarFragment`, `derives`), the deviations as a list of named claims (`parseDeviations`), and a `decide`d corpus row per production |
| 31 | every grammar term is accepted, modulo a data list of deviations | the same: the fragment reads every production, and each deviation is a row with its own detector |
| 32 | each spelling lexes one way | `Rchain/Lex.lean` (`lexemes`, maximal munch) — the operator surface is checked |
| 33 | `parse (print p) ≡ p` | `Rchain/Print.lean` (`printToks`, `printSurf`, `renderTokens`) — checked in **two halves**: the model's, that the printer's output is a grammar term, and the identity itself, which runs on the node in the Rust consumer, modulo the named warts (`printWarts`) |
| 34 | a value position is normalized against an empty par; only a statement continuation inherits | `Surface.lean`'s `normalizeAt` + the `c21` corpus layer (`Corpus.lean`'s `c21Cases`) — both halves checked on the shapes C21 broke, the accumulator itself now **modelled** (`normalizeAt` threads the port's `ProcVisitInputs.par`, and only the sequencing arm hands it on) |
| 35 | concreteness is sound (connective, free var, wildcard **or remainder**) | `Rchain/Par.lean`'s `connectiveUsed` + the `flags` corpus |
| 36 | the normalizer's output is well-scoped and closed | `Surface.lean`'s `normalize` — modelled; the corpus that would hold it to the node does not exist |
| 37 | matching is sound and complete, over the shapes the clauses cover | `Rchain/Match.lean` — `pathPar` is that domain and `spatialMatches_iff_eq` is the tie; `match.tsv` ties the model to the node |
| 38 | silence is specified: no reduction, no error | `Rchain/Silence.lean`'s `ReduceP` — the rule is the law and the `silence` corpus ties it; the tie between the rule and the search is a theorem in both directions (`takesStep_sound`, `takesStep_complete`, so `takesStep_iff_reduces` is not an axiom). What is left is the model's boundary rather than a proof debt: it has no join rule, so a join with one channel filled is silent here and in the node, while a fully matched join is a step in the node and none here |
| 39 | every `rho:*` urn's reply shape equals its `API-SCHEMA.md` row | `Rchain/Protocol.lean`'s `replyCatalog` — checked for the rows the surface can spell |
| 40 | every call has an accepting receive at the target's arity | `Silence.lean`'s `stepsInBinds` + `ReduceP.commPs` (the rule clause that makes it a rule) |
| 41 | a replicable reader restores what it consumes | `Rchain/Store.lean` — the `store` corpus ties it (measured on the live contract) |
| 42 | the rho-value round-trip, and the envelope rule | `Rchain/Json.lean` — the corpus ties the wire form and `decode_encode` is a theorem; the axiom it replaced was not merely owed but **false**, because its domain predicate admitted a par of two unforgeables, which the decoder drops and the encoder writes as nothing at all (`unforgPair_refutes_the_old_statement`; the predicate now refuses one) |
| 43 | each endpoint's serialized shape equals the schema's | `Rchain/Envelope.lean`'s catalog — checked for the envelope's keys and tags |

### What each layer buys, in one line each

- **Grammar (30, 31, 32, 33)** — a term is what the grammar derives, and nothing else. C30's finding is
  the shape of it: `parse` returned `Ok` for a valid *prefix* (`null )` parsed), and because
  `source_to_adt_with_env` runs on `deploy.data.term`, a deploy could execute part of what a client wrote
  and report success.
- **Normalization (34, 36)** — a value position is normalized against an *empty* par; only a statement
  continuation inherits the accumulated one. C21 lived here: a non-first `if` reduced to nothing.
- **Matching (35, 37, 38)** — an unmatched receive performs no step and reports no error. That is the law,
  stated, and it is why the defects were expensive: the silence *is* the correct behaviour, so a broken
  pattern is indistinguishable from a pattern that legitimately matched nothing.
- **Replies and protocols (39, 40, 41)** — a reply's arity and shape are a row in a table, and a call at
  the wrong arity has no accepting receive *by the rule*.
- **JSON (42, 43)** — the wire form is the reference document's, arm for arm, and the served document is
  held to the same catalog the DTOs are. C38 is the finding: the code and the law agreed with each other
  and neither with a client, because the law had been written from the code.

**A law written from the code is a mirror.** The corpus cannot catch a wrong shape when the model and the
node were derived from each other; only the contract can, and it has to be read rather than assumed.

## The validator lifecycle — the Proof-of-Stake epoch (Laws 44–47)

The port had the lifecycle — bond, withdraw, slash, trust — and no epoch, so a membership change took
effect the moment its deploy ran and a reward law had no reward to be about. Every law below is anchored
to `rholang/src/native_state.rs`, the native replacement for `legacy/casper/src/main/resources/Pos.rhox`.
The Scala contract is the **differential reference** — the port must satisfy the law, and where the
contract's behaviour is the law, the row cites its line. The design decisions are in
[`spec/RUST-FIRST.md`](../../../spec/RUST-FIRST.md) (`## The staking vault`, `## The epoch`) and the
epoch-boundary deviations in `spec/AUDIT.md` §6.

| # | Law | Where it lives |
|---|---|---|
| 44 | membership takes effect at an **epoch boundary**: the sequence runs only when `blockNumber % epochLength = 0`; a bond is pooled but not activated, a withdrawal is staged but not moved, a claim is not paid | `rholang/src/native_state.rs`'s `close_block` gate (`is_epoch_boundary`); the machine is `Rchain/Pos.lean` (`isBoundary`, `epochStep`, `closeBlock`: commit → move → pay → reselect, in `close_block`'s order, with the gate *outside* the transition), witnessed by `closeBlock_off_a_boundary`, `a_bond_pools_but_does_not_activate` and `a_boundary_activates_the_pool` |
| 45 | the split: `pot * (bondᵢ / minimumBond) / (activeBonds / minimumBond)` per active validator, out of `pot = posBalance − totalBond − totalWithdraw − committedRewards`, committed per validator and paid only when the validator leaves | `Rchain/Pos.lean` (`rewardPot`, `reward`) — the formula is the Scala's `getCurrentEpochRewards` (`Pos.rhox:241-256`) and the tie to the Rust is named (`an_epoch_splits_the_pot_and_keeps_the_dust`). Where the contract is undefined — `minimumBond` 0, or a normaliser of 0 — it divides by zero and faults; the port pays zero, registered in `spec/AUDIT.md` §6 |
| 46 | the split **does not conserve**: `Σ rewards ≤ pot`, the difference being the dust of two integer divisions — which stays in the pot for the next epoch | `Rchain/Pos.lean` (`sum_rewards_le_pot`, `list_sum_div_le`, `div_add_div_le`, `the_dust_is_real`), with a strict instance `decide`d: minimum bond 3, bonds `[4, 5]`, pot 10 — **six distributed of ten** |
| 47 | a withdrawal is **staged**: the request records `quarantineLength + epochLength * (1 + blockNumber / epochLength)` and changes nothing else; the validator leaves the pool at the next boundary and is paid `bond + committed rewards` at the first boundary past its quarantine | `rholang/src/native_state.rs` (`withdraw`, `close_block`'s move and pay steps); `Rchain/Pos.lean`'s `stage`, `movePending`, `dueClaims`, `payoutOf`, with the three stages witnessed separately (`a_staged_withdrawal_moves_no_coins`, `the_move_escrows_the_bond_and_pays_nothing`, `a_due_claim_is_paid_its_bond_plus_its_committed`, `a_claim_before_its_deadline_is_not_paid`) and the ordering by `the_reward_is_committed_before_the_leave` |

### Why 46 is the row worth having

The contract's reward divides **twice**:

```text
reward_i = (posBalance - totalBond - totalWithdraw - totalCommittedRewards) * (bonds / minimumBond)
                                                                     / (activeBonds / minimumBond)
```

so `Σ reward_i` is the pot *minus* dust — the sum of the floored shares, re-floored. A law like "the
epoch distributes the pot" is therefore not merely unproved but **false**, and the difference is not
noise to be waved away: it is a quantity the next epoch redistributes, because the dust stays in the
staking vault and the next pot is the vault less the same claims. `Pos.lean`'s `the_dust_is_real` is
minimum bond 3, bonds `[4, 5]`, pot 10: each validator's scaled share is `4 / 3 = 5 / 3 = 1`, so each is
paid `10 * 1 / 3 = 3`, and the epoch moves **6 of 10 units**. The Rust test builds exactly that state —
the two bonds, the minimum bond, a pot of 10 paid in as phlo — and reads the split back, so the
implementation is checked against the arithmetic rather than against a remembered number.

### The order of the sequence is the law

An epoch boundary runs four steps, and their order carries meaning (`Pos.rhox:528-551`):

1. compute the epoch's rewards **from the state as it stands** and add them to `committedRewards`;
2. move each staged withdrawal into the claims map, taking it out of the pool;
3. pay every claim whose quarantine has elapsed — `bond + committed[pk]` — and clear both entries;
4. re-select the active set from what is left of the pool.

Because step 1 runs before step 2, a validator that asked to leave **earns the epoch it left in**, and
because its reward is paid at step 3 of a *later* boundary, the claim must store the bond and read the
reward out of the committed map at payment time rather than storing the sum. That is why the contract's
header comment describing the stored pair as `(bond + reward, quantunue)` is the *payee's* sum and not
the record: the code at `Pos.rhox:582` stores the bond alone, and `:604` adds the reward.

**The sequence is not idempotent** — a second call at the same height would redistribute the previous
epoch's dust, since the committed claims now cover the rest. The system deploy calls it once per block.

### What these laws rest on, and what is still owed

`close_block`'s *arithmetic* is modelled; its *transition* is not. The formalization worth writing for
laws 44 and 47 is not the gate itself — `if boundary then … else s` restates its own definition, which is
the `vacuous` shape this register has a word for — but the **conservation an epoch preserves**: the
staking vault plus the Coop vault plus every user vault is invariant, which is what makes a payout a
transfer rather than a mint. The row that found this the hard way is the port's `slash`, which credited
the Coop vault without debiting anything: the staking vault (law 45's pot source, `pos:vault`) is what
turned that into a transfer, and `total_rev` in the Rust tests is what holds it.

Two deviations from the contract are recorded in `spec/AUDIT.md` §6 and matter here: the active set is
chosen **top-N by descending stake** (the contract takes the first N entries of the bonds map in key
order, under a `TODO` for a random selection), and a slashed validator with a staged withdrawal is
removed from the pending map rather than left as a permanent zero-claim tombstone.

## Charging — the denied deploy, and what a matched one costs (Laws 48–49)

The last two rows are about **gas and consequences**, not about state, and both were decided rather than
ported.

| # | Law | Where it lives |
|---|---|---|
| 48 | a **denied** deploy's effects are excluded from the merged state, and the merge's objective counts its cost exactly as an included deploy's — so the fee consequence RCHIP-02 proposes (the deployer is not charged, the validator is not rewarded) holds in neither tree | `casper/src/merging.rs`, `rholang/src/native_state.rs`, `docs/src/node/block-merge.md` — **an open design question shared with the Scala**, not a port divergence: `MergeScope.scala:87` defaults `rejectionCost` to `DeployChainIndex.deployChainCost`, and `DeployChainIndex.scala:73` defines that as `deploysWithCost.map(_.cost).sum` — the same objective this port computes. Nothing in either tree refunds a denied deploy. Closing it means choosing an objective and a refund path the Scala does not have, which is a consensus change without an oracle; the decision taken is to leave it open *with the reason* |
| 49 | for a matched produce/consume the charged gas is the Scala's: the storage is charged up front and what the match consumed is **refunded** — the continuation's consume storage and the produce storage of every removed datum — *before* the event and COMM costs | `rholang/src/storage.rs`; the model is `Rchain/Charging.lean` (`prefixes`, `peak`, `itotal`, `peak_refunds_first`, `itotal_refunds_first`), and the Rust test asserts both halves: the matched call's total against the same op *without* a match plus the COMM cost minus the two refunds, and the balance at which the deploy completes versus one phlo below it. The second half matters because the *peak* is what refuses a step: a test that checked only the total would pass with the refunds charged last |

Law 49 is also a **hard fork**: the refund changes the recorded `PCost` of every deploy that matches, so a
chain that accepted such a block under the old (over-charging) rule diverges on it. Registered in
`spec/AUDIT.md` §6 with the reason the old value was already wrong.

---

## Reading the formalization

- **Lean** (`spec/Rchain/*.lean`) owns the algebraic/order laws and the type-system fundamentals. Build:
  `cd spec && lake build`. An `axiom` with a named reason is a *stated* obligation, not a hole: the gate
  refuses a `sorry`, so an unproven claim has to be visible in the source as an axiom and in
  `spec/INVENTORY.md` as a status.
- **Coq** (`spec/coq/*.v`) owns the substitution / α-equivalence metatheory (Laws 2–6). Build:
  `make -C spec/coq`.
- **K** (`legacy/rholang/src/main/k/rholang/*.k`) is the executable reference semantics of the language.
- **Rust** carries each invariant *structurally* (refinement newtypes, no silent partiality).
- **The corpora** (`spec/conformance/*.tsv`) are the bridge: they are *emitted* from the Lean definitions
  (`lake exe rchain-corpus --layer <l>`), committed, and read by a Rust consumer that runs the same cases
  through the node and must agree 1:1. A law whose definition changes without its corpus being re-emitted
  fails the gate rather than drifting quietly.

### The gates

| Command | What it refuses |
|---|---|
| `tools/check-lean-conformance.sh` | a failed Lean or Coq build, a `sorry`/`admit`, a module `Rchain.lean` does not import, a **stale or untracked corpus**, a corpus with no consumer, a consumer that disagrees with its corpus, and a law-39 catalog urn with no row in `spec/API-SCHEMA.md`. It is the `formal` job in CI. |
| `tools/audit-type-system.sh` | production `panic!`/`unsafe`/silent conversion — the no-silent-partiality discipline |
| `tools/audit-test-register.sh` | a register that overstates the tree, a named test that does not exist, **a law row claiming coverage without naming a Lean module, a corpus and a consumer that exist** |

The third of those is the one that answers "which law would have caught the eleventh failure?":
`spec/AUDIT.md` §20 maps every incident to its law and its case.

Per-law status, source-of-truth pointers, and Rust realization are in
[`spec/INVENTORY.md`](../../../spec/INVENTORY.md); the machine-readable rows are
[`spec/laws.tsv`](../../../spec/laws.tsv).
