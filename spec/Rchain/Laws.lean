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
  the consolidation pass exists for: three statements have already been removed under it —
  `next_step_closure_computable` (a `rfl`), `validated_speculation_refines_apply` (a disjunction whose
  second arm held for any run), and `gate_replay_terminates` (`∃ st', f st = st'`, which is totality of a
  Lean function) — and `finality_iff_supermajority` is still here, two `Nat.mul_comm`s away from
  `isSuperMajority`'s own body. Calling such a row `provedModel` would be true and useless; `vacuous`
  says the proof is real and the *law* is not yet. A `vacuous` row must carry a note naming the
  re-scoping it needs, so the word cannot become a resting place. -/
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
    statement := "Spatial matching; a free variable is bound at most once — **on the aggregation path**, \
      which is the only place the port checks it (`aggregate_updates` raises a `BugFoundError`), while \
      the element-pair and conjunction paths overwrite silently",
    status := .owed,
    declarations := [`Rchain.spatialMatch, `Rchain.spatialMatches, `Rchain.spatialMatchCore,
      `Rchain.aggregateUpdates, `Rchain.aggregateUpdates_rejects_double_bind,
      `Rchain.freeMapMerge_overwrites],
    axioms := [`Rchain.concrete_matches_iff_eq, `Rchain.fuel_saturation],
    corpus := some "match",
    rust := ["rholang/src/matcher/spatial_matcher.rs"],
    falsifiable := some "the corpus's three-valued verdicts (`true`/`false`/`rejected`) include the \
      rejected case a twice-bound pattern produces — the shape the previous law-5 axiom *denied* and \
      which `spec/conformance/match.tsv` now pins (AUDIT C26) — and `freeMapMerge_overwrites` is the \
      counterexample inside the model: the same repeated level the aggregation path refuses is silently \
      overwritten on the fold path",
    note := "`spatialMatch_implies_linear` is `h.2` of a conjunct inside `spatialMatch`'s own \
      definition, so it holds by construction — and it is **not the port's predicate**. The port checks \
      linearity in exactly one place, `aggregate_updates` (`spatial_matcher.rs:644-665`), reached only \
      from the collection path (`list_match`'s tail, `:800`); the element-pair path (`fold_match`, \
      `:595-629`) and the conjunction path (`ConnAnd`, `:325-334`) thread their binding maps with no \
      check, and a binding is a plain `insert` (`:477-480`), so a level bound twice **overwrites**, \
      right-biased, with no error. The model carries both halves now — \
      `aggregateUpdates_rejects_double_bind` for the checked path, `freeMapMerge_overwrites` for the \
      unchecked ones — which is what makes the law a statement about the matcher's clauses rather than \
      about its own definition. **Owed**: the owed proofs above, and a node probe of the silent path \
      (`x!(a, a)`, and a twice-bound pattern through each path) before the divergence is *called* a \
      defect" },
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
    status := .provedModel,
    declarations := [`Rchain.joinKey, `Rchain.joinKey_perm],
    axioms := [`Rchain.hashHashes],
    rust := ["rspace/src/hashing/stable_hash_provider.rs"],
    falsifiable := some "`joinKey_perm` follows from `Cmp.sortList_perm` — the join key is *defined* as \
      hash-of-sorted-hashes, mirroring `hash_seq` + `hash_hashes` — so removing the sort from the \
      definition would falsify it: two permutations of one channel list would then hash differently",
    note := "**this row's two axioms are gone.** `joinKey : List Channel → Nat` was opaque and \
      `joinKey_perm` a claim about it; the Rust's join key is *defined* (`hash_seq` sorts the channel \
      hashes, `hash_hashes` sorts again and hashes — `stable_hash_provider.rs:22-46`), so the invariance \
      is Law 1's canonicalization applied to a join key rather than an independent postulate. \
      `hashHashes` is the one primitive that stays axiomatized, in Law 19's class" },
  { number := 8, layer := "RSpace",
    statement := "Deterministic COMM: candidate selection is sorted-first by content hash and produce \
      refs are sorted, so the event trace is content-addressed",
    status := .provedModel,
    declarations := [`Rchain.Produce, `Rchain.Consume, `Rchain.Comm, `Rchain.produceRefs,
      `Rchain.commId, `Rchain.comm_content_addressed],
    rust := ["rspace/src/trace/event.rs", "rspace/src/space_matcher.rs", "rspace/src/rspace.rs"],
    axioms := [],
    falsifiable := some "`comm_content_addressed` is proved from `sortList_perm`: two comms whose \
      produces are permutations of one another have the same identity. Dropping the sort from \
      `produceRefs` — which is what `Comm::apply` would be without `produce_refs.sort_by_key` — would \
      falsify it, and the Rust pins the ordering directly (`event.rs:165`, `space_matcher.rs:98-102`, \
      `rspace.rs:154-157`)",
    note := "**both axioms are gone.** `produceRefs : Comm → List Nat` was opaque and \
      `comm_content_addressed` a claim about it; the refs are a *definition* now, mirroring \
      `Comm::apply`'s sort, and the content-addressing is Law 1's canonicalization applied to the event \
      log. The model keeps the arrival order in `Comm.produces` precisely so the sort has something to \
      remove" },
  { number := 9, layer := "RSpace",
    statement := "Merge is a monoid and non-conflicting logs commute — strengthened for effect \
      scheduling: disjoint **closure** (not footprint) implies commutation",
    status := .provedModel,
    declarations := [`Rchain.mergeChanges, `Rchain.mergeChanges_assoc, `Rchain.NonConflicting,
      `Rchain.mergeChanges_comm, `Rchain.nonConflicting_not_necessary, `Rchain.join_last_wins,
      `Rchain.effect_commute_of_disjoint_closure, `Rchain.effect_reorder_diverges],
    rust := ["rspace/src/merger/state_change.rs", "rspace/src/merger/event_log_merging_logic.rs"],
    axioms := [],
    falsifiable := some "`mergeChanges_assoc` is structural (concatenation of the added/removed lists, \
      right-biased overwrite of the join map); `mergeChanges_comm` *needs* the disjointness hypothesis — \
      without it the theorem is false twice over: the added/removed lists concatenate in operand order \
      (which is why the Rust's own test compares sorted multisets rather than lists, \
      `state_change.rs:224-238`), and a contested **join** is won by whichever side the fold reaches last \
      (`state_change.rs:186-189`, pinned by `:502-544` and stated as `join_last_wins`; the fold is \
      `casper/src/merging.rs:752-755`). `nonConflicting_not_necessary` proves the relation is sufficient \
      and not necessary, and `effect_reorder_diverges` remains the disproof of the weaker footprint \
      reading",
    note := "**all four axioms are gone.** `mergeChanges` and `NonConflicting` were axioms over \
      `StateChange = { id : Nat }` — and a claim about an undefined relation is a claim about nothing, \
      so the two laws could have been true of any relation one cared to imagine. The Rust gives the \
      definition: the added/removed lists concatenate (`state_change.rs:18-38`) and the join map is a \
      **right-biased overwrite** (`joins.insert(k, v)` in a loop over the right operand, \
      `:186-189` — the Scala's `x.map ++ y.map`, right-biased too, `StateChange.scala:152`), so the model \
      overwrites and `join_last_wins` states which side wins — which the code's own \
      `combine_has_an_identity_and_a_right_biased_join_map` (`:502-544`, \"the later change's join body \
      wins\") pins. **Two findings are recorded here.** \
      (1) `NonConflicting` is *not* `are_conflicting` read negatively, and saying so was wrong: \
      `are_conflicting` is over two `EventLogIndex`es with three checks, one of which (a potential COMM) \
      is a shared-channel interaction and one of which (produces touching base joins) no state diff can \
      see (`event_log_merging_logic.rs:105-158`), and the predicate the merge branches on is broader \
      again (`casper/src/merging.rs:177-181`). What the model needs is the sufficient condition for \
      commutation, and it is named for that. (2) The Rust test named `combine_is_associative` \
      (`state_change.rs:203-238`) does **not** test associativity — its own comment says the law it pins \
      is empty-is-identity — so the associativity the merge fold relies on (`casper/src/merging.rs:752-755`) \
      is **untested on the Rust side**, while the identity, the right-biased join and the inner \
      `ChannelChange` monoid all are (`:502-544`, `channel_change.rs:35-50`); owed: an AUDIT §17 entry" },
  { number := 10, layer := "RSpace",
    statement := "Merkle determinism: the radix trie is content-addressed, collision-free, with a \
      defined empty root",
    status := .provedModel,
    declarations := [`Rchain.Item, `Rchain.Node, `Rchain.encodeNode, `Rchain.nodeHash,
      `Rchain.emptyNode, `Rchain.emptyRoot, `Rchain.root_collision_free,
      `Rchain.nodeHash_eq_emptyRoot],
    axioms := [`Rchain.encodeNode, `Rchain.encodeNode_injective],
    rust := ["rspace/src/history/radix_tree.rs"],
    falsifiable := some "`root_collision_free` composes Law 19's `blake2b256_collision_free` with \
      `encodeNode_injective`; `nodeHash_eq_emptyRoot` pins the empty root as a fixed point with nothing \
      else hashing to it. A serializer that dropped a field would falsify the first, and a second node \
      hashing to the empty root the second — which is why the store *refuses* a colliding write \
      (`radix_tree.rs:208-223,226-258`) rather than tolerating one",
    note := "**the ghost constant is gone.** `trieRoot : NodeHash` had no arguments — a constant — which \
      is why nothing could be proved about it; the root is now `nodeHash ∘ encodeNode` over the node \
      type the code has (`[Item; 256]`, `radix_tree.rs:15-35`), with `emptyRoot` the empty node's hash. \
      The two collision statements, which were statements about the *hash* wearing a trie's name, are \
      now theorems composing Law 19's axiom with `encodeNode_injective` — the serializer's canonicity, \
      which a hash cannot supply, so it is stated as this row's own axiom" },
  { number := 11, layer := "RSpace",
    statement := "Replay determinism: a recomputed COMM agrees with the recorded trace, and the port \
      checks membership **both** ways — a recomputed COMM absent from the trace fails, and a recorded \
      COMM the replay never consumes fails too",
    status := .vacuous,
    declarations := [`Rchain.Comm, `Rchain.produceRefs, `Rchain.commId, `Rchain.Trace],
    axioms := [],
    rust := ["rspace/src/replay_rspace.rs", "rspace/src/space_matcher.rs"],
    falsifiable := some "the port's check has a negative case on each side, so the law is falsifiable in \
      both directions: `ReplayCommNotInTrace` when the recomputed COMM is not in the record, and \
      `Unused COMM event` when the record keeps an entry the replay never consumed \
      (`rspace/src/replay_rspace.rs:580-590`). The Rust's own tests assert each fires — \
      `a_rig_whose_comm_never_happens_is_reported` (`:663-696`) for the second, \
      `a_rigged_replay_matches_its_recorded_trace` (`:635-651`) for the first",
    note := "`replayEvents` and `replay_comm_subset` are **deleted**, and the reason they are is the \
      finding: in this model the replay runs the same matcher over the same recorded producers, so \
      \"recomputed\" and \"recorded\" would be the *same function* and the subset claim would be `rfl` \
      (argued in `Rchain/RSpace/Comm.lean`'s header). What would give the law content is a model of the \
      recorded store as a structure *distinct* from the recomputation, so that the two must be shown to \
      agree; the proof is owed to that modelling step, not to a tactic. The law is real in the code and \
      **stronger than this register used to say** — the Rust checks membership in both directions, \
      forward at `replay_rspace.rs:330-332` and reverse at `:580-590` — so the re-scoping is to \
      `Rchain.Trace` plus that bidirectional check, which also stops Law 11 from being Law 8 restated" },

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
    statement := "Content addressing: `hash_block` clears `block_hash` and `sig` and hashes every other \
      proto field canonically, so equal hashes determine equal bodies",
    status := .provedModel,
    declarations := [`Rchain.Block, `Rchain.BlockBody, `Rchain.Block.body, `Rchain.encodeBody,
      `Rchain.blockHash, `Rchain.content_addressing, `Rchain.blockHash_changes_with_header],
    axioms := [`Rchain.encodeBody, `Rchain.encodeBody_injective],
    rust := ["casper/src/proto_util.rs", "models/src/casper/protocol/casper_message.rs"],
    falsifiable := some "`content_addressing` composes Law 19's `blake2b256_collision_free` with \
      `encodeBody_injective`, so a serializer that dropped a field would falsify it — and \
      `blockHash_changes_with_header` is the code's own `hash_block_changes_with_timestamp` \
      (`proto_util.rs:155-162`) derived: two blocks differing in **any** hashed field must hash \
      differently, which is why the informational timestamp is in the body even though no consensus rule \
      reads it. A `hash_block` that stopped clearing `sig` would falsify \
      `hash_block_is_deterministic_and_ignores_sig` (`:138-144`) and, transitively, this",
    note := "**the old row was a postulate about a field no function computed** — `hash = \
      Blake2b256(block − {hash, sig})` over `Block.hash : Nat`, which the note below it admitted was not \
      true even of its own model (an injective `Nat → Nat` does not exist). `Block.hash` is gone; the \
      hash is *computed* (`blockHash`), and the content addressing is a **theorem**: the composition of \
      Law 19's collision-freedom with the serializer's canonicity. The two axioms that remain are Law \
      10's shape and for the same stated reason — a hash can make an encoding collision-resistant but \
      never canonical, so `encodeBody`'s injectivity is the serializer's assumption, not the hash's. \
      The body is the **hashed** body: the port hashes every field except `block_hash` and `sig` \
      (`proto_util.rs:58-64`, `BlockMessage` at `casper_message.rs:563-585`), so the model carries the \
      timestamp and the rest of the header, and `blockHash_changes_with_header` is the statement a \
      narrower body could not make" },
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
    declarations := [`Rchain.blake2b256, `Rchain.blake2b256_output_is_32_bytes,
      `Rchain.blake2b256_collision_free, `Rchain.sign, `Rchain.verify,
      `Rchain.sign_verify_roundtrip, `Rchain.sharedSecret, `Rchain.curve25519_roundtrip,
      `Rchain.mergeRandom],
    rust := ["crypto/src/hash/blake2b256_hash.rs", "crypto/src/hash/blake2b512_random.rs"],
    axioms := [`Rchain.blake2b256, `Rchain.blake2b256_output_is_32_bytes,
      `Rchain.blake2b256_collision_free, `Rchain.sign, `Rchain.verify,
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
      17's RNG clause duplicated this one and is merged into it. **The types were retyped to mirror the \
      code's**: `Msg`/`Hash` are byte strings rather than opaque `Nat` wrappers, and the 32-byte width \
      is `blake2b256_output_is_32_bytes` (`Hash32`, `shared/src/refined.rs:287-296`) — which is what \
      lets Law 7's and Law 10's models be built out of hashes instead of guessed numbers" },

  -- ── Scheduler: the effect scheduler (Laws 20–25) ─────────────────────────────────────────────────
  { number := 20, layer := "Scheduler",
    statement := "Channel-task linearization (\"1 channel = 1 logical task\"): same-channel ops commit \
      in DFS path order through a per-channel claim queue, and the path-smallest pending claim is \
      always committable",
    status := .provedModel,
    declarations := [`Rchain.queue_commit_path_ordered, `Rchain.pathSorted_head_minimal],
    rust := ["rspace/src/concurrent/channel_queue.rs"],
    falsifiable := some "`queue_commit_path_ordered` is an induction on the run — a commit rule that \
      removed a claim which was not head-on-all would falsify it — and `pathSorted_head_minimal` is \
      the bakery argument's core: an unordered queue, or a head with a smaller path behind it, would \
      falsify that",
    note := "the axiom that stood here (`law20_deadlock_freedom`) is **deleted**, and the reason is a \
      finding: it quantified over every `Chan → Queue` and claimed some claim is head-on-all, which is \
      unprovable as stated — `PathLt` is not well-founded on paths (`[0] > [0,0] > [0,0,0] > …`), so an \
      unbounded pending set need have no minimum. The real system's paths are bounded by the depth of \
      the term being reduced and its pending set is finite, so the corrected statement carries that \
      finiteness as a hypothesis; what is proved here is the queue-level core, which needs none. The \
      Rust's liveness evidence is stress tests plus the queue's own deferral of cross-channel cycles \
      (`rspace/src/concurrent/channel_queue.rs:24-28`, risk R3) — a tested property, not a theorem. \
      **Also found while proving it**: `queue_commit_path_ordered` carried an unused \
      `hinit : ∀ ch, PathSorted (q0 ch)` — the same unused-hypothesis shape as Law 24's — and it is \
      gone, because the suffix property comes from the commit rule rather than from the initial sorting" },
  { number := 21, layer := "Scheduler",
    statement := "DFS-gate linearization: the gate scheduler refines sequential `Effect.apply`; one-hop \
      next-step pruning is unsound",
    status := .provedModel,
    declarations := [`Rchain.gate_await_closure_orders, `Rchain.one_hop_depth2_diverges],
    rust := ["rholang/src/reduce.rs", "rholang/src/scheduler.rs"],
    falsifiable := some "`one_hop_depth2_diverges` is a proved depth-2 counterexample to the pruning \
      rule the law forbids — the law and the disproof of its tempting weakening are published together \
      — and `gate_await_closure_orders` is false for a chain with a missing link (a dependency of \
      `j + 2 = i` would leave odd-indexed tasks unordered)",
    note := "`gate_exec_refines_apply` is **gone**: it defined `gateApply` as the sequential fold and \
      then proved the fold is the fold. The Rust's own comment above the gate says the same — 'Not a \
      speedup — the sound, sequential-equivalent carrier' (`reduce.rs:2324-2327`) — so the content was \
      never in the identification. It is in the **dependency structure**, and that is what \
      `gate_await_closure_orders` proves: the immediate-predecessor await chain is transitively \
      complete, which is exactly the Rust's 'a linear chain of awaits, not the quadratic \
      all-predecessors join' (`reduce.rs:2341-2345`)" },
  { number := 22, layer := "Scheduler",
    statement := "Next-step closure is computable at dispatch (the matched datum is concrete); \
      computability does not make cross-channel pruning sound",
    status := .vacuous,
    declarations := [`Rchain.depth2_next_step_disjoint],
    rust := ["rholang/src/reduce.rs"],
    falsifiable := some "`depth2_next_step_disjoint` is the half with content: the closure is \
      computable yet cross-channel pruning by it is still unsound, and that is proved rather than \
      asserted",
    note := "the positive half is a fact about a **signature**, not a theorem, which is why this row is \
      `vacuous` rather than a proof claim: `resolve_children` takes no store \
      (`reduce.rs:2228-2234`, and its comment names the purity), so stating it in Lean would prove that \
      a function ignores a parameter nobody passes. The theorem that stood here \
      (`next_step_closure_computable`) was `by rfl` and is **deleted**. **And the tempting positive law \
      is false**: resolving `p | q` term-wise does *not* give `resolve p ++ resolve q`, because the Rust \
      flattens all sends before all receives — one send and one receive in each half resolves as \
      `[sp,sq,rp,rq]` merged but `[sp,rp,sq,rq]` concatenated. Found by trying to prove it, which is \
      the argument for proving things" },
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
      `Rchain.pinned_run_publication, `Rchain.certificate_blind_late_writer_diverges],
    rust := ["rspace/src/concurrent/channel_queue.rs", "casper/src/runtime_manager.rs"],
    falsifiable := some "the certificate's blind spot is *published* as a theorem \
      (`certificate_blind_late_writer_diverges`): a late-writer run passes the certificate and still \
      diverges, which is why the oracle backstop stays load-bearing",
    note := "the unused `DFSSerializable` hypothesis is **gone**, and with it the \
      `set_option linter.unusedVariables false` that suppressed the warning — the theorem \
      (`pinned_run_publication`, renamed to state what it proves) is dispatched + path-nodup + pinned \
      ⇒ the gate fold, with **no** certificate hypothesis, because a pinned run *is* the \
      serializability that matters. The certificate *finds* such runs; it is not why the fold is reached" },
  { number := 25, layer := "Scheduler",
    statement := "Validated speculation: commits may reorder iff each validates Law 24; invalidated \
      runs fall back to the whole-run gate re-run, so the published state is the sequential fold's",
    status := .provedModel,
    declarations := [`Rchain.published, `Rchain.published_state_is_the_oracles,
      `Rchain.fallback_rerun_published, `Rchain.Published],
    rust := ["casper/src/runtime_manager.rs", "rspace/src/concurrent/channel_queue.rs"],
    falsifiable := some "the rule is the Rust's own (`casper/src/runtime_manager.rs:1013,1044`): accept \
      the speculative run only when it agrees with the oracle, and otherwise ship the oracle's result. \
      A publication rule that shipped the speculative state unconditionally would falsify \
      `published_state_is_the_oracles`, which is why the theorem is stated over the *rule* rather than \
      over the fold. `fallback_rerun_published` separately pins that the path-sorted re-run commits \
      each path at most once",
    note := "`validated_speculation_refines_apply` is **gone**, and so is `gate_replay_terminates`: the \
      first was a disjunction whose second arm held for *every* run by the definition of `gateFold` (so \
      it said nothing about the certificate, the fallback, or the code), and the second was \
      `⟨gateRerun ops st, rfl⟩` — totality of a Lean function, which is why `∃ st', f st = st'` is \
      never a law. What replaces them is the port's actual publication rule, which is what makes the \
      sequential-oracle backstop load-bearing rather than decorative" },

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
