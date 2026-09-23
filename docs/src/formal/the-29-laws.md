# The 29 laws

RChain's behavior is pinned by **laws** — one invariant per layer of the system. Each law maps a
language or system feature to its formalization: a Lean theorem, a Coq axiom, the executable K rule,
and the Rust realization. The canonical catalog is
[`spec/INVENTORY.md`](../../../spec/INVENTORY.md); this page is the reader-facing rendering of the same
mapping.

**The catalog is 49 rows.** The 29 below are the *calculus*; rows **30–43** are the *surface* the
matcher reads — grammar, lexing, normalization, matching, reply shapes, the JSON envelope — and they
are [Laws 30–43](laws-30-43.md); rows **44–47** are the native Proof-of-Stake epoch ([Laws 44–47](laws-44-47.md)), row **48**
is the fee consequence of a denied deploy — a rule neither this port nor the Scala implements — and
row **49** is what a matched deploy is charged: the gas a deploy costs, and the storage it is refunded. The surface rows exist because every defect that started that
programme (AUDIT C9–C22) lived there and **nothing errored**: the laws below constrain the calculus,
and a client never touches the calculus directly.

Legend: **proven** = a theorem with a proof; **stated** = an axiom with a precise signature (definition
deferred); **axiom** = postulated by design (a cryptographic primitive).

> **The checked rendering is [`spec/LAWS.md`](../../../spec/LAWS.md)** — emitted from
> `Rchain/Laws.lean` and refused stale by the gate. It counts clauses rather than laws and separates
> **`proved-tied`** (proved *and* tied to the node by a corpus) from **`proved-model`** (proved about a
> model a human keeps in sync); this page's plainer words are the narrative reading of the same rows.
> Its total is **<!-- counts:proved-laws -->34<!-- counts:end --> of <!-- counts:laws -->49 laws<!-- counts:end --> proved at all — <!-- counts:proved-tied-laws -->8<!-- counts:end --> tied to the node by a corpus, <!-- counts:proved-model-laws -->26<!-- counts:end --> over
> the model — with one more law proved but *vacuous* (its statement restates its own definition). Where the
> two disagree, the register is right — and it
> was right about this page's law 5, which used to call an almost-vacuous lemma "correctly stated".

## Rholang — the language (Laws 1–6)

| Law | Invariant | Surface feature | Lean | Coq | K rule |
|---|---|---|---|---|---|
| **1** | canonicalization is idempotent & commutative: `sort(sort p)=sort p`, `sort(p\|q)=sort(q\|p)` | the sorted `Par`, commutative `ESet`/`EMap` | `Sort.lean` — `sortPar_idempotent` / `sortPar_comm` (**proven**, mod the 4 element-comparator axioms the register counts — `cmpExpr`'s `eq_iff`/`swap`/`lt_trans` and `cmpPar`'s `lt_trans`; the other eight element laws, the list laws and `cmpGUnforgeable_lt_trans` are discharged) | `Sort.v` — `sortPar_idempotent` / `sortPar_comm` (axioms) | normalization (α + canonical `\|` sort) |
| **2** | α / name equivalence: par order, `\| Nil`, associativity, top-level arithmetic, α, `@`/`*` | `@`/`*`, `@{P\|Q}=@{Q\|P}` | `Rho.lean` — `StrCong` `≡` (**proven** core) | `Laws.v` — `alpha_equiv` (axiom) | `name-equivalence.k`, `alpha-equivalence.k` |
| **3** | capture-avoiding de Bruijn substitution; `sort(subst t)=subst(sort t)` | variable binding | `Subst.lean` — `substPar` (**defined**: a `mutual` family mirroring `rholang/src/substitute.rs`), `sort_subst`, `subst_closed` (**axioms**, owed) | `Laws.v` — `substPar`, `subst_commutes_sort` (axiom) | `free.k` (substitution; `substitution.k` referenced) |
| **4** | reduction (COMM), first-match-wins, `new` freshness | send/receive, `match` | `Rho.lean` `Reduce` ⟶ (**proven** core); `Concurrent.lean` `reduce_redex_unique` (**proven** — an isolated redex reduces to its body, uniquely up to `≡`) and `reduce_not_deterministic` (**proven** — the flat `Par` is *not* confluent, which is why the block path's determinism comes from the scheduler, Laws 20–25); `Reduce.lean` `reduce_freeVars_subset` (**proven** — `Reduce`'s freshness clause, over `FreeVars.lean`'s definition) | `Laws.v` — `reduce` (axiom) | `processes-semantics.k`, `sending-receiving.k`, `persistent-sending-receiving.k` |
| **5** | spatial matching, defined; an accepted match binds each free level at most once | patterns, `_`, `~`, `/\`, `\/` | `Match.lean` — `spatialMatchCore`/`spatialMatch` (**defined**: a fuel-carrying matcher, so "does it match" is computed), `spatialMatches` (**defined**), `spatialMatches_decidable` (**an instance**, not an axiom), `spatialMatch_implies_linear` (**proved**, with the register's caveat: it is a conjunct inside `spatialMatch`'s own definition, so it holds *by construction* rather than as a statement about the matcher's clauses). The old statement of this row ("`BindsAtMostOnce` … **stated**") was an axiom that was *false* as written: AUDIT C26. Law 37's tie — `spatialMatch t p = (t = p)` for a connective-free pattern, which is what makes the port's `connective_used` fast path sound — is stated there as `concrete_matches_iff_eq` and is **owed** | `Laws.v` — `spatial_matches` (axiom). `binds_at_most_once`, which this cell used to list, was **deleted** (2026-09-23): it stated as an axiom the proposition Lean found *false* as written (AUDIT C26), which is worse than no statement — an axiom for a false proposition makes the file unsound for the row it claims. Lean's replacement (`linear`) is a definition, and the Coq copy is owed with the rest of the matcher | `matching-function.k`, `specific-matching-rules.k`, `exact-matching-function.k`, `matching-with-par.k` |
| **6** | no globally free variables | `Closed` | `Ty.lean` `Closed` (**proven**) + `FreeVars.lean` `freeVarOf` (**defined** — a `mutual` block mirroring the checker) and `closed_iff_no_freeVars` (**proven**); both were axioms | `Laws.v` — `closed`, `closed_decidable` (axiom) | `free.k`, `program-restrictions.k` |

## RSpace — the tuple space (Laws 7–11)

| Law | Invariant | Feature | Lean |
|---|---|---|---|
| **7** | join commutativity (channel keys hashed in sorted order) | multi-channel receive | `RSpace/Join.lean` — `joinKey` is *defined* as hash-of-sorted-channel-hashes, so `joinKey_perm` (**proved**) is law 1's canonicalization applied to a join key |
| **8** | deterministic COMM (produce refs sorted; content-addressed events) | the comm event | `RSpace/Comm.lean` — `produceRefs`/`commId` defined over the Rust's own sort, `comm_content_addressed` (**proved**) |
| **9** | merge is a monoid; non-conflicting logs commute | state merging | `RSpace/Merge.lean` — `mergeChanges` defined (added/removed concatenate, the join map is right-biased overwrite), `NonConflicting` defined, `mergeChanges_assoc`/`_comm` (**proved**) |
| **10** | Merkle determinism (content-addressed radix trie, collision-free, empty root) | history | `RSpace/Merkle.lean` — `nodeHash`/`emptyNode`/`emptyRoot` defined over the node type the code has, `encodeNode_injective` **proved** over `WellFormed` (it was an axiom, and a false one before the invariant was found), `root_collision_free`/`nodeHash_eq_emptyRoot` (**proved**) |
| **11** | replay determinism (recomputed COMM ⊆ recorded trace) | replay | `RSpace/Comm.lean` — the recorded trace is an **input** now (`Replays recomputed recorded`), and the check is **proved** to hold exactly when the two agree on occurrences (`replays_iff_same_occurrences`); neither half suffices, each with a witness, and the second witness is the port's own guard — `check_replay_data_with_fix` drops the reverse half on a failed deploy (RCHAIN-3505) |

## Rosette — the actor VM (Laws 12–13)

| Law | Invariant | Status |
|---|---|---|
| **12** | actor atomicity (single-threaded `mbox.nextMsg`) | **orphaned** — `rosette`/`roscala` VM is out of scope |
| **13** | reflection (everything is an `Ob`; meta/parent chain; fork-join barrier) | **orphaned** |

## Casper / Storage / Crypto (Laws 14–19)

| Law | Invariant | Feature | Lean |
|---|---|---|---|
| **14** | finality requires **> 2/3** bonded stake; fringe = one message per validator | the fringe | `Casper/Stake.lean` — `calculateFringe` is the port's `calculate_fringe` (full-partition filter, non-bonded skip, exact-integer comparison) and `Fringe.lean`'s `finality_iff_supermajority` is the advance gate (**proved**, with the 2/3, 2⁵³ and `i64` boundary tests); `fringe_antichain` was **false of a bare `Fringe`** and is refuted in the tree — the antichain is a property of the *derived* fringe, and that derivation is owed |
| **15** | fringe monotone by height; seen-set monotone | the DAG | `Casper/Fringe.lean` — both axioms were **false of a bare `Fringe`/`Message`** and are refuted in the tree; the port *constructs* a message's seen set (`seenOf`, with `seenOf_contains_justifications`/`mem_seenOf_self` **proved**), and the transitive closure the finalizer leans on is owed to a DAG model |
| **16** | block number = max(parent)+1; seqNum strictly +1; content addressing; bonds cache = PoS | blocks | `Casper/Validate.lean` — the two number laws are now the port's own checks (`BlockNumberValid`/`SeqNumValid`, folds over the justifications seeded `-1`) with the laws as their elimination (**proved**; both old universal forms were **false** and are refuted in the tree); `content_addressing` is **proved** from `blake2b256_collision_free` and the serializer's canonicity |
| **17** | the merge's arithmetic is the checked 64-bit one — a value that would leave `i64` is **refused, not wrapped** — and the merged RNG is a function of the *set* of branch generators | merges | `Merging.lean` — `checkedAdd`/`checkedSub` with their refusal witnesses, `mergeRandoms_perm` (**proved**). The `numeric_channels_nonneg` this row used to cite was **false of the code**: numeric channels are signed `i64` and negative diffs are ordinary (`rholang/src/merging.rs:161-166`) |
| **18** | height map contiguous; fringe identity order-independent | storage | `Casper/Validate.lean` — the two axioms were **false as stated** (any `List Block`; `Perm → f = g`) and are refuted in the tree; `Contiguous` is the store's invariant with `contiguous_insert_succ`/`contiguous_skip_leaves_hole` **proved**, and `fringeId`/`fringeId_perm` prove the order-independence the `BTreeSet` supplies |
| **19** | Blake2b256 canonical; the `Blake2b512Random` merge is n-ary and **order-sensitive**; sig verify/sign; Curve25519 round-trip | crypto | `Crypto/Random.lean` `mergeRandom` (n-ary, mirroring `merge(children: &[Self])`); `Crypto/Spec.lean` `blake2b256_collision_free`, `blake2b256_output_is_32_bytes` (the 32-byte width the code's `Hash32` types), `sign_verify_roundtrip`, `curve25519_roundtrip` (**axiom**, by design; `Msg`/`Hash` are byte strings, not opaque `Nat`s, so the width is stateable). The `mergeRandom_comm` this row used to cite was **false of the code** — `crypto/src/hash/blake2b512_random.rs:548` asserts order-sensitivity — so order-independence is stated where it holds: at the *call site*, `Merging.lean`'s `mergeRandoms_perm` |

## Scheduler — the effect scheduler (Laws 20–25)

| Law | Invariant | Feature | Lean |
|---|---|---|---|
| **20** | channel-task linearization ("1 channel = 1 logical task"): a per-channel claim queue keeps same-channel commits in DFS path order; the path-smallest pending claim is always committable | the claim queue | `Scheduler.lean` — `queue_commit_path_ordered` (**proven**; its statement no longer carries an unused sortedness hypothesis), `pathSorted_head_minimal` (**proven** — the bakery argument's core). The *global* liveness claim the old `law20_deadlock_freedom` axiom made is unprovable as stated (`PathLt` is not well-founded on paths, so an unbounded pending set need have no minimum) and needs the finiteness the real system has |
| **21** | DFS-gate linearization: running effect `i` only after effects `0..i−1` complete is exactly the sequential apply fold; the one-hop (next-step-footprint) variant is unsound | the gate scheduler | `Scheduler.lean` — `gate_await_closure_orders` (**proven**: the immediate-predecessor await chain is transitively complete, which is the Rust's "not the quadratic all-predecessors join"), `one_hop_depth2_diverges` (**proven** counterexample). `gate_exec_refines_apply` defined the fold as the fold and is gone |
| **22** | next-step closure is computable at dispatch (the matched datum is concrete); computability does *not* make cross-channel pruning sound | dispatch-time closure | `Scheduler.lean` — `depth2_next_step_disjoint` (**proven**, the half with content). The positive half is a *signature* fact (`resolve_children` takes no store), so `next_step_closure_computable` was `by rfl` and is gone; the tempting distributivity law is false for the Rust's flattening order |
| **23** | read-determinism: an effect's outcome and event trace are a deterministic function of the state it reads | on-chain speculation | `SchedulerOnchain.lean` — `read_state_determines_outcome` (**proven**) |
| **24** | DFS-order serializability: a concurrent run is sound iff every commit read the state the DFS-earlier effects produced (versioned write-record layer; prefix visibility) | the validation certificate | `SchedulerOnchain.lean` — all **proven**: the witnesses, `record_determines_value`/`dispatched_record_at`, `serializable_writer_chain` and `pinned_run_publication` (the file has no axioms left). The publication theorem carries **no** certificate hypothesis — the unused `DFSSerializable` argument and the `linter.unusedVariables` suppression that hid it are both gone |
| **25** | validated speculation: commits may reorder iff each validates Law 24; invalidated subtrees abort and gate-re-run, so the published log is the sequential fold's | the abort/re-run coordinator | `SchedulerOnchain.lean` — `published` + `published_state_is_the_oracles` (**proven**: accept only on agreement with the oracle, else ship the oracle's result — so the certificate decides *how much* is re-run, never *what* is published), `Published` + `fallback_rerun_published` (**proven**). `validated_speculation_refines_apply` and `gate_replay_terminates` are gone: one was a disjunction true for any run, the other was totality |

The reader-facing story — the claim queue, the DFS gate, the relaxed mode, and the unsound one-hop
variant — is [The channel scheduler](channel-scheduler.md); the on-chain extension is
[On-chain scheduling: validated speculation](onchain-scheduling.md).

## Cross-shard — two-phase commit (Laws 26–29)

| Law | Invariant | Feature | Lean |
|---|---|---|---|
| **26** | shard scope determinism: a deploy/block's effects bind to exactly one shard; the shard id is a validated, ordered value; the RNG seed + unforgeable names are shard-scoped | the shard boundary | `CrossShard.lean` — `shard_scope_deterministic` (**stated**) |
| **27** | cross-shard atomicity (2PC): a transaction commits on every participant or aborts on every one — no run leaves a strict subset committed | the coordinator | `CrossShard.lean` — `txn_atomic` (**stated**) |
| **28** | leg idempotency: `prepare`/`commit`/`abort` are idempotent under the transaction id | re-delivery / retry | `CrossShard.lean` — `leg_idempotent` (**stated**) |
| **29** | decision durability & record determinism: the coordinator's decision is a durable, content-addressed record; a prepared participant recovers it; re-derivable on replay | the merge/close record | `CrossShard.lean` — `commit_record_deterministic` (**stated**) |

The reader-facing story — the Git pull-request flow, the roles, the state machine, and the 2PC
recovery caveat — is [Cross-shard transactions: two-phase commit](cross-shard-transactions.md).

## Reading the formalization

- **Lean** (`spec/Rchain/*.lean`) owns the algebraic/order laws and the type-system fundamentals. Build:
  `cd spec && lake build`. An `axiom` with a named reason is a *stated* obligation, not a hole: the gate
  below refuses a `sorry`, so an unproven claim has to be visible in the source as an axiom and in
  `spec/INVENTORY.md` as a status.
- **Coq** (`spec/coq/*.v`) owns the substitution / α-equivalence metatheory (Laws 2–6). Build:
  `make -C spec/coq`.
- **K** (`legacy/rholang/src/main/k/rholang/*.k`) is the executable reference semantics of the language.
- **Rust** carries each invariant *structurally* (refinement newtypes, no silent partiality).
- **The corpora** (`spec/conformance/*.tsv`) are the bridge: they are *emitted* from the Lean
  definitions (`lake exe rchain-corpus --layer <l>`), committed, and read by a Rust consumer that runs
  the same cases through the node and must agree 1:1. A law whose definition changes without its
  corpus being re-emitted fails the gate rather than drifting quietly.

### The gates

| Command | What it refuses |
|---|---|
| `tools/check-lean-conformance.sh` | a failed Lean or Coq build, a `sorry`/`admit`, a module `Rchain.lean` does not import, a **stale or untracked corpus**, a corpus with no consumer, a consumer that disagrees with its corpus, and a law-39 catalog urn with no row in `spec/API-SCHEMA.md`. It is the `formal` job in CI. |
| `tools/audit-type-system.sh` | production `panic!`/`unsafe`/silent conversion — the no-silent-partiality discipline |
| `tools/audit-test-register.sh` | a register that overstates the tree, a named test that does not exist, **a law row claiming coverage without naming a Lean module, a corpus and a consumer that exist** |

The third of those is the one that answers "which law would have caught the eleventh failure?":
`spec/AUDIT.md` §20 maps every incident to its law and its case.

Per-law status, source-of-truth pointers, and Rust realization are in
[`spec/INVENTORY.md`](../../../spec/INVENTORY.md).
