/-!
# The law register — all 43 laws, in one place, with what each one rests on

`spec/INVENTORY.md` is the prose catalog and `docs/src/formal/the-43-laws.md` is its reader-facing
rendering, but neither is *checkable*: nothing noticed that the tree grew to 43 laws while both still
said 29, that the two tables contradicted each other on Laws 5 and 24, or that Law 1's "30 residual
axioms" were really 12. This module is the single source of truth those documents are generated from,
and `Rchain/LawsMain.lean` (the `rchain-laws` executable) is what enforces it:

1. **Numbering** — every law 1..43 is present, with no gaps, so a law cannot be quietly dropped.
2. **Reference integrity** — every declaration a row names actually exists in `Rchain`. A renamed or
   deleted theorem fails this, instead of leaving a row that cites a proof that is gone.
3. **Axiom accounting** — the set of axioms cited by these rows is *exactly* the set of `axiom`
   declarations in the tree. This is the ratchet the `sorry` scan cannot be: `tools/check-lean-conformance.sh`
   catches a new `sorry`, but an `axiom` is how a law is usually assumed, and until this register existed
   a 63rd axiom was invisible.

## The status vocabulary, and why it is not "proven / stated"

The old vocabulary (`spec/INVENTORY.md`'s legend) had one word for two very different things. A law
proved *about the Lean model* and a law proved *and checked against the running node* both read
"proven", and the difference between them is exactly the difference that let C21 ship: the model's
`normalize_if` was never the question — the node's was.

- `provedTied` — proved, **and** tied to the node by a conformance corpus whose Rust consumer runs the
  same source text through the real thing (`spec/conformance/*.tsv`). The corpus is the only mechanical
  Lean↔Rust link this repo has, so only laws 32, 35, 37–43 can carry this status today.
- `provedModel` — proved over the model; the tie to the Rust is prose in a mapping table. Honest, and
  weaker than it sounds: a `provedModel` law is a claim about a model that a human keeps in sync. Laws
  1–11 and 14–29 are here.
- `axiomByDesign` — postulated because the primitive is cryptographic (Law 19). The only status that
  should survive the work this register begins.
- `owed` — the definition exists and the proof does not. `takesStep_iff_reduces`, `decode_encode`.
- `deferred` — the `axiom` *is* the definition (`joinKey`, `trieRoot`, `mergeChanges`, `substPar`), so
  there is nothing yet to prove anything about.
- `open` — in the catalog, no formalization (laws 30, 31, 33, 34, 36).
- `orphaned` — out of scope because the VM it describes was not ported (laws 12, 13).

`falsifiable` records what would have to be true for the law to be *false* — a witness, a negative case,
or the reason it cannot fail. A law that cannot fail constrains nothing: `numeric_channels_nonneg` was
`0 ≤ b.number` on a `Nat` (`Nat.zero_le`), and `finality_iff_supermajority` restated its own definition.
`none` means the witness is owed, and the count of those is the measure of how much of this catalog is
still unfalsifiable.
-/

-- The register is data, and `Name` is the one type it needs from outside the prelude. `autoImplicit`
-- off because the alternative is what happened on this file's first draft: `List Name` silently became
-- an *implicitly bound* `Name✝`, so the structure was accepted with a field type nobody wrote. A file
-- whose whole purpose is to be checkable should not accept a typo as a binder.
set_option autoImplicit false

namespace Rchain
namespace Laws

/-- What a law's proof is worth. See the module doc — this vocabulary is the point of the register. -/
inductive Status where
  /-- Proved, and tied to the node by a conformance corpus. -/
  | provedTied
  /-- Proved over the Lean model; the model↔Rust tie is prose, not machine-checked. -/
  | provedModel
  /-- Postulated by design: a cryptographic primitive (Law 19). -/
  | axiomByDesign
  /-- The definition exists; the proof is missing. -/
  | owed
  /-- The `axiom` is the definition — nothing to prove yet. -/
  | deferred
  /-- In the catalog; no formalization. -/
  | open
  /-- Out of scope: the VM it describes was not ported. -/
  | orphaned
  /-- **Proved, but the statement restates its own definition and so cannot fail.** This is the status
  the consolidation pass exists for: `next_step_closure_computable` was `rfl`,
  `validated_speculation_refines_apply` a disjunction whose second arm holds for any run, and
  `finality_iff_supermajority` two `Nat.mul_comm`s away from `isSuperMajority`'s own body. Calling such
  a row `provedModel` would be true and useless; `vacuous` says the proof is real and the *law* is not
  yet. A `vacuous` row must carry a note naming the re-scoping it needs, so the word cannot become a
  resting place. -/
  | vacuous
  deriving BEq, DecidableEq, Repr

def Status.wire : Status → String
  | .provedTied    => "proved-tied"
  | .provedModel   => "proved-model"
  | .axiomByDesign => "axiom-by-design"
  | .owed          => "owed"
  | .deferred      => "deferred"
  | .open          => "open"
  | .orphaned      => "orphaned"
  | .vacuous       => "vacuous"

/-- One law, or one clause of a law whose clauses differ in status or in what they rest on. Law 16
carries four clauses and Law 1 two, because "Law 1 is proved, residually 30 axioms" was the sentence
that hid the truth (it is 12). -/
structure Law where
  /-- The law number: 1–43. A law with several clauses repeats its number. -/
  number : Nat
  /-- The clause letter — `""` for a single-clause law, else `"a"`, `"b"`, … -/
  clause : String := ""
  /-- Which layer of the system the law belongs to. -/
  layer : String
  /-- The law, in one line. The long form is `spec/INVENTORY.md`'s. -/
  statement : String
  status : Status
  /-- The declarations this law's status rests on: its headline theorem(s) or definition(s). -/
  declarations : List Lean.Name := []
  /-- The axioms this law rests on. The union over all rows must equal the tree's axiom set. -/
  axioms : List Lean.Name := []
  /-- The conformance corpus layer that ties this law to the node, if there is one. -/
  corpus : Option String := none
  /-- **The Rust this law models**, as `path` or `path:line` anchors — the field that makes "the model
  models the code" checkable rather than promised. `Rchain/LawsMain.lean` refuses a row that claims a
  model (`proved-tied`, `proved-model`, `axiom-by-design`, `vacuous`) without one, and refuses any
  anchor whose file does not exist. The anchor names the *code*, not the test: where a conformance
  corpus exists it is already in `corpus`, and where a property test exists it is named in the note. -/
  rust : List String := []
  /-- What would have to hold for this law to be false: a witness, a negative case, or why it cannot
  fail. `none` = owed. -/
  falsifiable : Option String := none
  /-- A note where the status needs qualifying. -/
  note : String := ""

/-- Every law in the catalog — 1..43, the orphaned ones and the open ones included, because a register
that lists only the formalized laws cannot notice a law that was dropped.

Statements are the one-line form; `spec/INVENTORY.md` carries the long form and the Rust realization.
`falsifiable` is `none` where the witness is owed, which is most of the non-corpus laws: naming that gap
is the purpose, not a defect of this file. -/
def laws : List Law := [
  -- ── Rholang: the language (Laws 1–6) ────────────────────────────────────────────────────────────
  { number := 1, clause := "a", layer := "Rholang",
    statement := "`Par`/`ESet`/`EMap` are commutative and canonicalization is idempotent and \
      commutative: `sort (sort p) = sort p`, `sort (p | q) = sort (q | p)`",
    status := .provedModel,
    declarations := [`Rchain.sortPar_idempotent, `Rchain.sortPar_comm, `Rchain.sortPar,
      `Rchain.parMerge],
    rust := ["models/src/sorter.rs"],
    axioms := [],
    falsifiable := some "`sortPar_idempotent`/`sortPar_comm` are theorems; `spec/INVENTORY.md`'s Law 1 \
      claim of idempotence is falsified by any leaf type whose comparator is not a total order — see \
      clause b, where exactly that is assumed rather than proved",
    note := "`sortPar_idempotent` is proved only *for* a comparator whose element laws hold; the \
      element-law half is clause b, and it is axioms" },
  { number := 1, clause := "b", layer := "Rholang",
    statement := "Each element comparator (`cmpPar`, `cmpSend`, …, `cmpConnective`) is a lawful total \
      order: `eq_iff`, `swap`, `lt_trans`",
    status := .owed,
    declarations := [`Rchain.cmpPar, `Rchain.cmpSend, `Rchain.cmpExpr],
    axioms := [`Rchain.cmpExpr_eq_iff, `Rchain.cmpExpr_swap,
      `Rchain.cmpPar_lt_trans, `Rchain.cmpSend_lt_trans, `Rchain.cmpReceiveBind_lt_trans,
      `Rchain.cmpReceive_lt_trans, `Rchain.cmpNew_lt_trans, `Rchain.cmpMatchCase_lt_trans,
      `Rchain.cmpMatch_lt_trans, `Rchain.cmpExpr_lt_trans, `Rchain.cmpBundle_lt_trans,
      `Rchain.cmpConnective_lt_trans],
    falsifiable := some "`cmpGUnforgeable_lt_trans` is proved while its ten siblings are axioms, and \
      `cmpListSend_lt_trans` is proved from them — so a counterexample to any element law would be a \
      counterexample to those proofs. No witness is published for the axioms themselves, which is the \
      gap: nothing would notice if one of them were false",
    note := "12 axioms, not 30: `Sort.lean`'s own header says 33 and `spec/INVENTORY.md` says 30, and \
      both are stale. The list comparators' laws were discharged by induction on the list" },
  { number := 2, layer := "Rholang",
    statement := "α/name equivalence = par order + `| Nil` + top-level arithmetic + α + added \
      eval/quote",
    status := .provedModel,
    declarations := [`Rchain.StrCong, `Rchain.strCong_equivalence, `Rchain.strCong_comm,
      `Rchain.strCong_assoc, `Rchain.strCong_ident, `Rchain.strCong_nil_left],
    rust := ["models/src/ast.rs"],
    falsifiable := some "`reduce_not_deterministic` (`Rchain/Concurrent.lean`) exhibits two distinct \
      reductions of one term, which is what makes `≡` — rather than syntactic identity — the relation \
      reduction needs",
    note := "the deep α half is Coq's obligation (`spec/coq/Laws.v`), where it is an `Axiom`" },
  { number := 3, layer := "Rholang",
    statement := "Capture-avoiding de Bruijn substitution; `sort (subst t) = subst (sort t)`",
    status := .deferred,
    declarations := [`Rchain.substPar, `Rchain.sort_subst, `Rchain.subst_closed],
    axioms := [`Rchain.substPar, `Rchain.sort_subst, `Rchain.subst_closed],
    falsifiable := none,
    note := "`substPar` is an `axiom`, so the two laws are statements about an undefined function; the \
      metatheory lives in Coq" },
  { number := 4, clause := "a", layer := "Rholang",
    statement := "Reduction (COMM): a send and a matching receive on one channel reduce to the \
      receive's body",
    status := .provedModel,
    declarations := [`Rchain.Reduce, `Rchain.reduce_closed, `Rchain.reduce_not_deterministic],
    rust := ["rholang/src/reduce.rs"],
    falsifiable := some "`reduce_not_deterministic` proves confluence is **false** on the flat `Par`, \
      so the law's statement is bounded by a published disproof rather than an assertion"
    },
  { number := 4, clause := "b", layer := "Rholang",
    statement := "`new` yields fresh unforgeable names: reduction introduces no free variables it did \
      not already have",
    status := .owed,
    declarations := [`Rchain.reduce_freeVars_subset],
    axioms := [`Rchain.reduce_freeVars_subset],
    falsifiable := none },
  { number := 5, layer := "Rholang",
    statement := "Spatial matching; a free variable is bound at most once",
    status := .owed,
    declarations := [`Rchain.spatialMatch, `Rchain.spatialMatch_implies_linear,
      `Rchain.spatialMatchCore],
    axioms := [`Rchain.concrete_matches_iff_eq, `Rchain.fuel_saturation],
    corpus := some "match",
    falsifiable := some "the corpus's three-valued verdicts (`true`/`false`/`rejected`) include the \
      rejected case a twice-bound pattern produces — the shape the previous law-5 axiom *denied* and \
      which `spec/conformance/match.tsv` now pins (AUDIT C26)",
    note := "`spatialMatch_implies_linear` is `h.2` of a conjunct inside `spatialMatch`'s own \
      definition, so it holds by construction: it is not yet a statement about the matcher's clauses" },
  { number := 6, layer := "Rholang",
    statement := "No globally free variables in a program",
    status := .owed,
    declarations := [`Rchain.Closed, `Rchain.closed, `Rchain.Closed_parMerge_iff,
      `Rchain.Closed_receivePar_iff, `Rchain.closed_anyPat],
    axioms := [`Rchain.freeVarOf, `Rchain.closed_iff_no_freeVars],
    falsifiable := some "`Closed` is a `decide`d `Bool` checker with a proved agreement lemma \
      (`closed_eq_Closed`), so both directions of the predicate are testable on concrete terms",
    note := "`Closed` itself is proved; the tie to `freeVarOf` is the axiom, which is what makes the \
      law rest on a defined-but-undefined-elsewhere predicate" },

  -- ── RSpace: the tuple space (Laws 7–11) ─────────────────────────────────────────────────────────
  { number := 7, layer := "RSpace",
    statement := "Join commutativity: channel keys are hashed in sorted order, so the join key is \
      invariant under permutation",
    status := .deferred,
    declarations := [`Rchain.joinKey, `Rchain.joinKey_perm],
    axioms := [`Rchain.joinKey, `Rchain.joinKey_perm],
    falsifiable := none },
  { number := 8, layer := "RSpace",
    statement := "Deterministic COMM: candidate selection is sorted-first by content hash and produce \
      refs are sorted, so the event trace is content-addressed",
    status := .deferred,
    declarations := [`Rchain.produceRefs, `Rchain.comm_content_addressed],
    axioms := [`Rchain.produceRefs, `Rchain.comm_content_addressed],
    falsifiable := none },
  { number := 9, layer := "RSpace",
    statement := "Merge is a monoid and non-conflicting logs commute — strengthened for effect \
      scheduling: disjoint **closure** (not footprint) implies commutation",
    status := .provedModel,
    declarations := [`Rchain.mergeChanges, `Rchain.mergeChanges_assoc, `Rchain.NonConflicting,
      `Rchain.mergeChanges_comm, `Rchain.effect_commute_of_disjoint_closure,
      `Rchain.effect_reorder_diverges],
    rust := ["rspace/src/merger/state_change.rs", "rspace/src/merger/event_log_merging_logic.rs"],
    axioms := [`Rchain.mergeChanges, `Rchain.mergeChanges_assoc, `Rchain.NonConflicting,
      `Rchain.mergeChanges_comm],
    falsifiable := some "`effect_reorder_diverges` is a proved counterexample to the weaker \
      footprint-disjointness reading: there is a published schedule that the naive rule would allow and \
      that changes the result, which is why the law was strengthened to closures",
    note := "the effect-level strengthening is a theorem; the state-change monoid it is stated over is \
      `deferred` (the four axioms)" },
  { number := 10, layer := "RSpace",
    statement := "Merkle determinism: the radix trie is content-addressed, collision-free, with a \
      defined empty root",
    status := .deferred,
    declarations := [`Rchain.trieRoot, `Rchain.trie_collision_free, `Rchain.emptyRoot,
      `Rchain.trie_empty_root],
    axioms := [`Rchain.trieRoot, `Rchain.trie_collision_free, `Rchain.emptyRoot,
      `Rchain.trie_empty_root],
    falsifiable := none },
  { number := 11, layer := "RSpace",
    statement := "Replay determinism: recomputed COMM is a subset of the recorded trace",
    status := .deferred,
    declarations := [`Rchain.replayEvents, `Rchain.replay_comm_subset],
    axioms := [`Rchain.replayEvents, `Rchain.replay_comm_subset],
    falsifiable := none },

  -- ── Rosette: the actor VM (Laws 12–13) ──────────────────────────────────────────────────────────
  { number := 12, layer := "Rosette",
    statement := "Actor atomicity (single-threaded `mbox.nextMsg`)",
    status := .orphaned,
    falsifiable := none,
    note := "`rosette`/`roscala` are not wired into the build and the Rust reducer replaces the VM" },
  { number := 13, layer := "Rosette",
    statement := "Reflection: everything is an `Ob`; meta/parent chain; fork-join barrier",
    status := .orphaned,
    falsifiable := none },

  -- ── Casper / Storage / Crypto (Laws 14–19) ──────────────────────────────────────────────────────
  { number := 14, clause := "a", layer := "Casper",
    statement := "Finality requires more than 2/3 of bonded stake, as the exact integer comparison \
      `3·stake > 2·total` (no float rounding)",
    status := .owed,
    declarations := [`Rchain.isSuperMajority, `Rchain.finality_iff_supermajority],
    axioms := [`Rchain.finality_iff_supermajority],
    falsifiable := some "the integer form is falsifiable where the float form is not: `3·stake = \
      2·total` is a boundary case the `>` excludes, and `isSuperMajority` is a `Prop` over `Nat`, so a \
      boundary instance can be checked",
    note := "as stated the axiom restates its own definition (`simp [isSuperMajority, Nat.mul_comm]` \
      closes it) — it ties finality to nothing. Re-scoping it is Task 2's" },
  { number := 14, clause := "b", layer := "Casper",
    statement := "A fringe holds one message per bonded validator (an antichain)",
    status := .deferred,
    declarations := [`Rchain.fringe_antichain],
    axioms := [`Rchain.fringe_antichain],
    falsifiable := none },
  { number := 15, layer := "Casper",
    statement := "The fringe is monotone by height and the seen-set is monotone (no regression)",
    status := .deferred,
    declarations := [`Rchain.fringe_monotone, `Rchain.seen_monotone],
    axioms := [`Rchain.fringe_monotone, `Rchain.seen_monotone],
    falsifiable := none },
  { number := 16, clause := "a", layer := "Casper",
    statement := "Block number = max(parent) + 1",
    status := .deferred,
    declarations := [`Rchain.Block, `Rchain.block_number_max_parent_plus_one],
    axioms := [`Rchain.block_number_max_parent_plus_one],
    falsifiable := none },
  { number := 16, clause := "b", layer := "Casper",
    statement := "`seqNum` strictly increases: the sender's next block is exactly one more than its \
      previous",
    status := .deferred,
    declarations := [`Rchain.seq_num_strictly_increases],
    axioms := [`Rchain.seq_num_strictly_increases],
    falsifiable := none,
    note := "as stated the axiom quantifies over **any** two blocks (`prev.seqNum + 1 = next.seqNum`), \
      which is false of two unrelated blocks: the sender relation is missing from the statement" },
  { number := 16, clause := "c", layer := "Casper",
    statement := "Content addressing: `hash = Blake2b256(block − {hash, sig})`, so the hash determines \
      the body",
    status := .deferred,
    declarations := [`Rchain.content_addressing],
    axioms := [`Rchain.content_addressing],
    falsifiable := none,
    note := "over the model's `hash : Nat` field the axiom is not true — an injective `Nat → Nat` does \
      not exist; a byte-string hash model is needed before this can be more than a postulate" },
  { number := 16, clause := "d", layer := "Casper",
    statement := "The bonds cache equals the PoS state",
    status := .open,
    falsifiable := none,
    note := "no Lean declaration; the Rust side is `BTreeMap<S, NonNegI64>` bonds" },
  { number := 17, clause := "a", layer := "Casper",
    statement := "Merge determinism: a rejection resolves to a unique minimum-cost candidate",
    status := .open,
    falsifiable := none,
    note := "no Lean statement, and the Rust suggests why: the merge takes the *branch set* \
      (`casper/src/merging.rs`'s `compute_merged_state` over the conflict predicate at \
      `rspace/src/merger/event_log_merging_logic.rs:100-158`) — it does not choose among candidates, so \
      a claim about a unique minimum-cost candidate has nothing in the code to be stated against. What \
      the code does have is Law 9's non-conflict condition, which is modelled there" },
  { number := 17, clause := "b", layer := "Casper",
    statement := "The merge's arithmetic is the checked 64-bit one — a value that would leave `i64` is \
      **refused, not wrapped** — and the merged RNG is a function of the *set* of branch generators",
    status := .provedModel,
    declarations := [`Rchain.checkedAdd, `Rchain.checkedSub, `Rchain.mergeRandoms,
      `Rchain.checkedAdd_refuses_overflow, `Rchain.checkedSub_refuses_overflow,
      `Rchain.merge_diff_round_trip, `Rchain.mergeRandoms_perm],
    rust := ["rholang/src/merging.rs", "rspace/src/merger/event_log_index.rs"],
    axioms := [],
    falsifiable := some "the refusal is a witness rather than a remark: `checkedAdd i64Max 1 = none` \
      and `checkedAdd i64Min (-1) = none` (`checkedAdd_refuses_overflow`), which a `checkedAdd` that \
      wrapped would fail; `mergeRandoms_perm` is the statement that the merged RNG does not depend on \
      the order branches arrived in, false the moment the caller's sort is removed",
    note := "**the law that stood here was false**: `numeric_channels_nonneg` claimed numeric channels \
      are non-negative, and they are signed `i64` with ordinary negative diffs \
      (`rholang/src/merging.rs:161-166`, tests at `:349,370` with `diff: -5`). The non-negativity that \
      *is* true belongs to Law 14's bonds and to `NonNegI64` (`shared/src/refined.rs:64`), which types \
      bonds and heights, never numeric channels. **And the arithmetic is only half checked**: the merge \
      result uses `checked_add` (`merging.rs:102`) while the diff accumulator uses a plain `i64 +=` \
      (`rspace/src/merger/event_log_index.rs:151`, `casper/src/merging.rs:758`) — a debug panic, a \
      release wrap. That half is a code finding, recorded as AUDIT §17 C41, not a law" },
  { number := 18, clause := "a", layer := "Storage",
    statement := "The height map is contiguous: no holes in block heights",
    status := .deferred,
    declarations := [`Rchain.height_map_contiguous],
    axioms := [`Rchain.height_map_contiguous],
    falsifiable := none,
    note := "as stated it claims a property of *any* `List Block`, which is false of an arbitrary \
      list — the DAG/fringe structure it depends on is not a hypothesis" },
  { number := 18, clause := "b", layer := "Storage",
    statement := "The fringe identity is order-independent (a set, not a list)",
    status := .deferred,
    declarations := [`Rchain.fringe_identity_order_independent],
    axioms := [`Rchain.fringe_identity_order_independent],
    falsifiable := none,
    note := "`Perm → f = g` needs `messages` sorted and deduplicated as an invariant of `Fringe`; \
      without it the axiom is false of a `Fringe` built by hand" },
  { number := 19, layer := "Crypto",
    statement := "Blake2b256 is canonical and collision-free; the `Blake2b512Random` merge is n-ary \
      and **order-sensitive**; signatures verify what they sign; Curve25519 round-trips",
    status := .axiomByDesign,
    declarations := [`Rchain.blake2b256, `Rchain.blake2b256_collision_free, `Rchain.sign,
      `Rchain.verify, `Rchain.sign_verify_roundtrip, `Rchain.sharedSecret,
      `Rchain.curve25519_roundtrip, `Rchain.mergeRandom],
    rust := ["crypto/src/hash/blake2b256_hash.rs", "crypto/src/hash/blake2b512_random.rs"],
    axioms := [`Rchain.blake2b256, `Rchain.blake2b256_collision_free, `Rchain.sign, `Rchain.verify,
      `Rchain.sign_verify_roundtrip, `Rchain.sharedSecret, `Rchain.curve25519_roundtrip,
      `Rchain.mergeRandom],
    falsifiable := some "the Rust side is pinned by known-answer tests rather than by these axioms: \
      `crypto/`'s `Blake2b512Random`/`Secp256k1`/`Curve25519` vectors are what would catch a wrong \
      primitive, so the axioms are a *boundary* of the model, not a claim the model establishes",
    note := "the only `axiomByDesign` law, and where a **false axiom** was found: `mergeRandom_comm` \
      claimed the RNG merge commutes and the code's own test refutes it \
      (`crypto/src/hash/blake2b512_random.rs:548`, `merge_is_order_sensitive`). The merge is modelled \
      n-ary (`List Random → Random`) because that is its Rust signature, and *no* algebraic law of it \
      is claimed — the code has none to claim, so `mergeRandom_assoc` went with the commutativity. Law \
      17's RNG clause duplicated this one and is merged into it" },

  -- ── Scheduler: the effect scheduler (Laws 20–25) ─────────────────────────────────────────────────
  { number := 20, layer := "Scheduler",
    statement := "Channel-task linearization (\"1 channel = 1 logical task\"): same-channel ops commit \
      in DFS path order through a per-channel claim queue, and the path-smallest pending claim is \
      always committable",
    status := .owed,
    declarations := [`Rchain.queue_commit_path_ordered, `Rchain.law20_deadlock_freedom],
    axioms := [`Rchain.law20_deadlock_freedom],
    falsifiable := some "`queue_commit_path_ordered` is proved, so the ordering half is falsifiable \
      by construction; the deadlock-freedom half is an axiom with no published witness — a queue \
      configuration that never commits would be the counterexample, and none is exhibited",
    note := "ordering proved; the bakery argument for liveness is the axiom" },
  { number := 21, layer := "Scheduler",
    statement := "DFS-gate linearization: the gate scheduler refines sequential `Effect.apply`; one-hop \
      next-step pruning is unsound",
    status := .provedModel,
    declarations := [`Rchain.gate_exec_refines_apply, `Rchain.one_hop_depth2_diverges],
    rust := ["rholang/src/reduce.rs", "rholang/src/scheduler.rs"],
    falsifiable := some "`one_hop_depth2_diverges` is a proved depth-2 counterexample to the pruning \
      rule the law forbids — this is the catalog's model of what a falsifiable law looks like: the law \
      and the disproof of its tempting weakening are published together",
    note := "the strongest-status law in the catalog: a refinement proof plus a disproof of the \
      competing design, both machine-checked" },
  { number := 22, layer := "Scheduler",
    statement := "Next-step closure is computable at dispatch (the matched datum is concrete); \
      computability does not make cross-channel pruning sound",
    status := .provedModel,
    declarations := [`Rchain.next_step_closure_computable, `Rchain.depth2_next_step_disjoint],
    rust := ["rholang/src/reduce.rs"],
    falsifiable := some "`depth2_next_step_disjoint` bounds the law: the closure is computable yet \
      cross-channel pruning by it is still unsound, and that is proved rather than asserted",
    note := "`next_step_closure_computable` itself is `rfl` — it restates the definition's unfolding, \
      so as a *law* it is currently unfalsifiable even though the claim beside it is not" },
  { number := 23, layer := "Scheduler",
    statement := "Read-determinism: an effect's chosen candidate, commit outcome and event trace are a \
      deterministic function of the state it reads",
    status := .provedModel,
    declarations := [`Rchain.read_state_determines_outcome],
    rust := ["rspace/src/space_matcher.rs", "rspace/src/rspace.rs"],
    falsifiable := some "stated as an implication from two states agreeing on the effect's closure, so \
      it is falsifiable by a state pair that agrees on the closure yet yields different traces — the \
      depth-2 stale read of Law 24 is exactly the shape that would produce one",
    note := "the property test `law23_read_state_determines_outcome` (`rspace/src/property_tests.rs`) \
      names it on the Rust side (commit 35dd13b62)" },
  { number := 24, layer := "Scheduler",
    statement := "DFS-order serializability: a concurrent execution is sound for the block path iff \
      every commit read exactly the state the DFS-earlier effects produced (the versioned \
      write-record layer)",
    status := .provedModel,
    declarations := [`Rchain.prefixVisible, `Rchain.ValidCommit, `Rchain.DFSSerializable,
      `Rchain.s3_pair_fails_validation, `Rchain.serializable_writer_chain,
      `Rchain.dfs_serializable_implies_log_equal,
      `Rchain.certificate_blind_late_writer_diverges],
    rust := ["rspace/src/concurrent/channel_queue.rs", "casper/src/runtime_manager.rs"],
    falsifiable := some "the certificate's blind spot is *published* as a theorem \
      (`certificate_blind_late_writer_diverges`): a late-writer run passes the certificate and still \
      diverges, which is why the oracle backstop stays load-bearing",
    note := "`dfs_serializable_implies_log_equal` does not use its `DFSSerializable` hypothesis — \
      `set_option linter.unusedVariables false` (`SchedulerOnchain.lean:1205`) hides that, and the \
      theorem holds of any dispatched, path-nodup, pinned run. Either use the hypothesis or restate \
      what is proved — Task 2's" },
  { number := 25, layer := "Scheduler",
    statement := "Validated speculation: commits may reorder iff each validates Law 24; invalidated \
      runs fall back to the whole-run gate re-run, so the published state is the sequential fold's",
    status := .owed,
    declarations := [`Rchain.validated_speculation_refines_apply, `Rchain.gate_replay_terminates,
      `Rchain.fallback_rerun_published, `Rchain.Published],
    falsifiable := some "`fallback_rerun_published` proves the fallback reaches the gate fold for \
      `Published` runs; `validated_speculation_refines_apply` is the disjunction \
      `oracleClean run ∨ gateRerun (pathSortedRun run) = gateFold run`, whose second disjunct is proved \
      for **any** run — so as stated the refinement holds without the validation premise, and the law \
      is weaker than its name: `gate_replay_terminates` is `⟨gateRerun ops st, rfl⟩`",
    note := "`spec/INVENTORY.md` says 'no axioms remain', which is true of the axioms and not of the \
      claim: two of these three statements are true by construction. Task 2's" },

  -- ── Cross-shard: two-phase commit (Laws 26–29) ───────────────────────────────────────────────────
  { number := 26, clause := "a", layer := "Cross-shard",
    statement := "A deploy/block's effects bind to exactly one shard",
    status := .deferred,
    declarations := [`Rchain.shard_scope_deterministic],
    axioms := [`Rchain.shard_scope_deterministic],
    falsifiable := none },
  { number := 26, clause := "b", layer := "Cross-shard",
    statement := "The shard id is a validated, ordered value",
    status := .open,
    falsifiable := none,
    note := "no Lean declaration; the Rust side is the `ShardId` newtype with its parent-shard-id \
      hierarchy (`shared/src/refined.rs`)" },
  { number := 26, clause := "c", layer := "Cross-shard",
    statement := "The RNG seed and unforgeable names are shard-scoped",
    status := .open,
    falsifiable := none },
  { number := 27, layer := "Cross-shard",
    statement := "Cross-shard atomicity (2PC): a transaction commits on every participant or aborts on \
      every one — no run leaves a strict subset committed",
    status := .deferred,
    declarations := [`Rchain.txn_atomic],
    axioms := [`Rchain.txn_atomic],
    falsifiable := none },
  { number := 28, layer := "Cross-shard",
    statement := "Leg idempotency: `prepare`/`commit`/`abort` are idempotent under the transaction id",
    status := .owed,
    declarations := [`Rchain.applyEffect, `Rchain.leg_idempotent],
    axioms := [`Rchain.leg_idempotent],
    falsifiable := none,
    note := "provable by construction — `applyEffect` is a pointwise update \
      (`if s = l.shard then l.effect else st s`), so the law follows by `funext`; it is an axiom only \
      because nobody wrote the one-line proof. Task 2's" },
  { number := 29, layer := "Cross-shard",
    statement := "Decision durability and record determinism: the coordinator's decision is a durable, \
      content-addressed record; a prepared participant can always recover it",
    status := .deferred,
    declarations := [`Rchain.commit_record_deterministic],
    axioms := [`Rchain.commit_record_deterministic],
    falsifiable := none },

  -- ── Laws 30–43: the surface the ten silent defects live in ──────────────────────────────────────
  { number := 30, layer := "Rholang",
    statement := "Every term the parser accepts is in the BNFC grammar (`rholang_mercury.cf`)",
    status := .open,
    falsifiable := none,
    note := "`parser.rs` is more permissive in places (AUDIT C24's residual); needs `Rchain/Parse.lean` \
      and the grammar as data. The gate already reserves a `parse` corpus consumer \
      (`tools/check-lean-conformance.sh:132`) with nothing behind it yet" },
  { number := 31, layer := "Rholang",
    statement := "Every BNFC term is accepted, modulo a data list of documented deviations",
    status := .open,
    falsifiable := none },
  { number := 32, layer := "Rholang",
    statement := "Lexical determinism: comments, the `_`/`_ident` rule, `bundle0`, number forms and \
      the operator spellings each lex one way",
    status := .provedTied,
    declarations := [`Rchain.lexemes, `Rchain.lexemes_decide, `Rchain.longestMatchIn],
    corpus := some "lex",
    rust := ["rholang/src/parser.rs", "node/tests/lean_lex_corpus.rs"],
    falsifiable := some "the `decide`d `lexemes_decide` fails if two spellings collide or if a row's \
      spelling is not its own longest match — a table where `<` shadowed `<=` breaks the `<=` row; the \
      Rust consumer (`node/tests/lean_lex_corpus.rs`) runs each sample through the real lexer, so a \
      source that drifted from the model fails instead of being trusted",
    note := "checked for the *operator* surface only; comments, `_`/`_ident`, `bundle0` and the literal \
      forms are named as the boundary and belong to laws 30/31/33" },
  { number := 33, layer := "Rholang",
    statement := "`parse (print p) ≡ p` on `Par` (the C13 round-trip)",
    status := .open,
    falsifiable := none,
    note := "needs `Rchain/Print.lean`, which does not exist" },
  { number := 34, layer := "Rholang",
    statement := "Normalization is a function; a *value* position (operand, condition, target, datum, \
      element, pattern, name) is normalized against an empty `par` — only a statement continuation \
      inherits",
    status := .open,
    declarations := [`Rchain.normalizeAt, `Rchain.Surf],
    falsifiable := some "this is AUDIT C21: with the rule violated, `x!([\"a\"]) | if (1 == 1) { … }` \
      produces nothing at all — the defect was observed on a node and the Rust fix is `f6477eba3`, so \
      the negative case is not hypothetical, it is history",
    note := "**the highest-value open law**, and the model cannot yet state it: `normalizeAt` threads \
      only the binder stack `Γ` and takes no accumulated `par`, so the very parameter whose misuse was \
      C21 does not exist here. The defect lived in the Rust normalizer seeding a *value* position with \
      the par that preceded it; a model with no accumulator cannot express that, so the law needs the \
      accumulator modelled (or, sharper and cheaper, a corpus layer carrying the C21 repro — the \
      mechanism that would actually tie it to the node)" },
  { number := 35, layer := "Rholang",
    statement := "`connective_used` is sound: it holds iff the term contains a connective, free \
      variable, wildcard or remainder, per collection form",
    status := .provedTied,
    declarations := [`Rchain.connectiveUsed, `Rchain.Var.isConnective, `Rchain.cmpOptionVar],
    corpus := some "flags",
    rust := ["rholang/src/normalizer.rs", "rholang/tests/lean_normalize_corpus.rs"],
    falsifiable := some "`flagCases_decide` (`Rchain/Corpus.lean`, 17 cases) is `decide`d, so a model \
      whose flag disagreed with its own case would not compile; \
      `rholang/tests/lean_normalize_corpus.rs` then reads each pattern's verdict out of the real node. \
      This layer found AUDIT C24",
    note := "the layer whose first line — `connective_used` short-circuiting to structural equality — \
      is where C19/C20/C22 item 3 all lived" },
  { number := 36, layer := "Rholang",
    statement := "The normalizer's output is well-scoped and closed (Law 6 through every path)",
    status := .open,
    falsifiable := none,
    note := "reuses `TotalOn`/`Refined` (`Rchain/Ty.lean`)" },
  { number := 37, layer := "Rholang",
    statement := "Match soundness and completeness (Law 5 strengthened: partial collections, \
      wildcards, remainders)",
    status := .owed,
    declarations := [`Rchain.spatialMatchCore, `Rchain.spatialMatchExprs, `Rchain.spatialMatchExpr,
      `Rchain.matchListPar, `Rchain.matchMap],
    axioms := [`Rchain.concrete_matches_iff_eq, `Rchain.fuel_saturation],
    corpus := some "match",
    falsifiable := some "15 cases with three-valued verdicts; the once-false law-5 axiom was replaced \
      *because* a corpus case contradicted it (AUDIT C26), and the fuel bound was one step short until \
      the `decide` refused to compile — the mechanism caught both",
    note := "shares its two axioms with Law 5" },
  { number := 38, layer := "Rholang",
    statement := "Silence is specified: an unmatched receive or `match` yields no reduction **and no \
      error**",
    status := .owed,
    declarations := [`Rchain.ReduceP, `Rchain.takesStep, `Rchain.receiveParP],
    axioms := [`Rchain.takesStep_iff_reduces, `Rchain.takesStep_sound],
    corpus := some "silence",
    falsifiable := some "`ReduceP`'s only datum-consuming rule carries `spatialMatches data pattern` \
      as a hypothesis, so silence is a consequence of the rule; the corpus's 6 cases each run on their \
      own runtime with a control datum, and a case that stepped when it should not would fail",
    note := "the rule is the law; the tie from the rule to the search is owed and named. It was \
      *false* as first stated — `takesStep p = true ↔ ∃ q', ReduceP p q'` for every `p`, refuted by \
      `chan = nilPar` (AUDIT C40) — so it is now domain-restricted to `allStringChans p`, with the sound \
      direction split out unconditionally as `takesStep_sound`, which is the half the corpus leans on. \
      A false axiom is worse than an owed proof: anything follows from it" },
  { number := 39, layer := "Protocol",
    statement := "Every `rho:*` urn's reply arity and shape equals its `spec/API-SCHEMA.md` row",
    status := .provedTied,
    declarations := [`Rchain.replyCatalog, `Rchain.replyCatalog_decide],
    corpus := some "protocol",
    rust := ["rholang/src/system_processes.rs", "spec/API-SCHEMA.md"],
    falsifiable := some "`replyCatalog_decide` requires namespaced unique urns, the reply kind \
      agreeing with its slots, and the declared arity agreeing with the arguments as written; the gate \
      additionally requires a `spec/API-SCHEMA.md` row per catalog urn, so a urn without a documented \
      shape fails",
    note := "checked for the rows the surface language can spell; `ByteArray`-taking urns (crypto, \
      `deployerId:ops`, `authToken:ops`) have no grammar literal and stay pinned by hand-written probes" },
  { number := 40, layer := "Protocol",
    statement := "Every call in the protocol catalog has an accepting receive at the target's arity",
    status := .provedTied,
    declarations := [`Rchain.stepsInBinds, `Rchain.receiveParPs],
    rust := ["rholang/src/system_processes.rs"],
    corpus := some "silence",
    falsifiable := some "case 10 is `write!(key, value)` against a three-argument `write` — a call at \
      the wrong arity has no step *by the rule*, so the corpus's arity-varying cases are the negative \
      cases, and this is AUDIT C22 item 2's check",
    note := "shares the silence layer with Law 38, which the gate documents as deliberate" },
  { number := 41, layer := "Protocol",
    statement := "Channel balance: a replicable reader restores what it consumes",
    status := .provedTied,
    declarations := [`Rchain.replicatedRead, `Rchain.readStore, `Rchain.storeSurvives],
    corpus := some "store",
    rust := ["rspace/src/rspace.rs", "casper/tests/genesis_registry.rs"],
    falsifiable := some "`storeCases_decide` (5 cases) plus \
      `casper/tests/genesis_registry.rs`'s `a_read_does_not_destroy_the_inbox` on a real chain: a read \
      that consumed its datum would fail both, and C25 (`Group`'s `new` reading a dictionary nobody \
      writes) was found by the second",
    note := "the static walk over the vendored text was retired unshipped rather than committed with \
      an exception list (AUDIT §17 C22 item 1)" },
  { number := 42, layer := "JSON",
    statement := "`rho_expr_to_par (expr_from_par p) = p`, and the `0 → absent`, `1 → unwrapped`, \
      `n → ExprPar` envelope rule",
    status := .owed,
    declarations := [`Rchain.parToJE, `Rchain.jeToPar, `Rchain.render, `Rchain.flatPar],
    axioms := [`Rchain.decode_encode],
    corpus := some "json",
    falsifiable := some "`jsonCases_decide` (12 cases) is `decide`d, and \
      `node/tests/lean_json_corpus.rs` runs each case through the node's own `expr_from_par` *and* its \
      `rho_expr_to_par` round-trip, so an envelope rule that was wrong for `n = 2` fails on a case",
    note := "the unforgeable leaf is outside the model's domain (its `GUnforgeable` carries a level, \
      not the wire bytes) and stays pinned by `rho_expr.rs`'s unit tests" },
  { number := 43, layer := "JSON",
    statement := "Each endpoint's serialized shape equals the schema's (camelCase fields, `[]` \
      semantics, error precedence)",
    status := .provedTied,
    declarations := [`Rchain.envelopeCatalog, `Rchain.envelopeCatalog_decide],
    corpus := some "envelope",
    rust := ["node/src/api/grpc/tonic.rs", "node/src/api/dto.rs"],
    falsifiable := some "`envelopeCatalog_decide`: no key contains an underscore (C16's rule), keys \
      distinct, union tags capitalized, names unique; `node/tests/lean_envelope_corpus.rs` holds *both* \
      parties to the catalog — the DTOs' serialization and the served `OPENAPI_JSON` document — so a \
      row that no longer matches either one fails",
    note := "checked for the envelope's keys and tags; the types behind them and the document's \
      coverage are named as the boundary (AUDIT C29 fixed the two rows the check found stale)" }
]

/-- Every law number the catalog defines. Laws with clauses repeat. -/
def numbers : List Nat := laws.map (·.number)

/-- The axioms claimed by some law. `Rchain/LawsMain.lean` requires this to equal the tree's `axiom`
set exactly — the ratchet that makes a new axiom impossible to add without a row that justifies it. -/
def claimedAxioms : List Lean.Name := (laws.map (·.axioms)).join

/-- The declarations every row cites, for the reference-integrity check. -/
def citedDeclarations : List Lean.Name := (laws.map (·.declarations)).join

end Laws
end Rchain
