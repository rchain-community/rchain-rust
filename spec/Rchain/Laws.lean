/-!
# The law register — all 43 laws, in one place, with what each one rests on

`spec/INVENTORY.md` is the prose catalog, and `docs/src/formal/the-29-laws.md` and
`docs/src/formal/laws-30-43.md` are its reader-facing rendering, but none of the three is *checkable*:
nothing noticed that the tree grew to 43 laws while both still
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
  Lean↔Rust link this repo has, so only laws 32, 34, 35, 37–43 can carry this status today.
- `provedModel` — proved over the model; the tie to the Rust is prose in a mapping table. Honest, and
  weaker than it sounds: a `provedModel` law is a claim about a model that a human keeps in sync. Most of
  laws 1–29 are here, and the consolidation pass moved rows *into* it by modelling the Rust's own
  definitions; the rows it could not are `owed`, and their notes say what is missing.
- `axiomByDesign` — postulated because the primitive is cryptographic (Law 19). The only status that
  should survive the work this register begins.
- `owed` — the definition exists and the proof does not. `takesStep_iff_reduces`, `decode_encode`.
- `deferred` — the `axiom` *is* the definition, so there is nothing yet to prove anything about.
  `substPar` is the remaining example; `joinKey`, `trieRoot` and `mergeChanges` left this status in the
  consolidation pass, when the Rust's own definitions were modelled.
- `open` — in the catalog, no formalization (laws 30, 31, 33, 34, 36).
- `orphaned` — out of scope because the VM it describes was not ported (laws 12, 13).
- `retired` — the port has **no rule of this shape**, with the evidence in the row's note and its `rust`
  anchor (the code that was read). A decision recorded, not an omission: the alternative is an `open` row
  that will never close, which makes the count dishonest in the direction that matters least visibly.

`falsifiable` records what would have to be true for the law to be *false* — a witness, a negative case,
or the reason it cannot fail. A law that cannot fail constrains nothing: `numeric_channels_nonneg` was
`0 ≤ b.number` on a `Nat` (`Nat.zero_le`), and `finality_iff_supermajority` restated its own definition
(the pass has since deleted the first and re-scoped the second onto the finalizer's gate).
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
  the consolidation pass exists for: statements that are true and useless. Four have been removed under
  it — `next_step_closure_computable` (a `rfl`), `validated_speculation_refines_apply` (a disjunction
  whose second arm held for any run), `gate_replay_terminates` (`∃ st', f st = st'`, which is totality of
  a Lean function), and `finality_iff_supermajority` (two `Nat.mul_comm`s away from `isSuperMajority`'s
  own body, now re-scoped onto the finalizer's gate). Two remain, each with the re-scoping it needs
  written in its note. Calling such a row `provedModel` would be true and useless; `vacuous` says the
  proof is real and the *law* is not yet, and the note requirement is what stops the word becoming a
  resting place. -/
  | vacuous
  /-- **Retired: the port has no rule of this shape to state the law against**, evidenced rather than
  assumed. Distinct from `orphaned` (a VM that was not ported) and from `open` (work pending): a
  `retired` row is one whose own investigation found nothing in the code for the law to be *about* — the
  merge "does not choose among candidates", so a law about a unique minimum-cost candidate has nothing to
  be stated against, and the honest close is to say so rather than leave the row owed forever. The
  requirement is what keeps the word from becoming a resting place: a `retired` row must carry the
  evidence in its note **and** a `rust` anchor naming the code that was read, because the claim being
  made is a claim about that code. Retiring is a *decision taken*, recorded — the distinction this
  register exists to keep. -/
  | retired
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
  | .retired       => "retired"

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
    status := .provedModel,
    declarations := [`Rchain.reduce_freeVars_subset, `Rchain.freeVarOf_receivePar,
      `Rchain.freeVarOf_parMerge],
    rust := ["rholang/src/reduce.rs"],
    falsifiable := some "a `comm` whose receive body mentions a level free in neither the send nor the \
      receive would refute it — the shape a body that is not a function of the datum it consumed \
      produces, and the one the port's capture-avoiding `substitute_par` (`rholang/src/substitute.rs`) \
      exists to prevent",
    note := "proved in `Rchain/Reduce.lean` by induction on the derivation, against `freeVarOf`, which \
      is a definition (law 6) — so this is a statement about a defined predicate rather than about an \
      `axiom`, which is what the row used to be. `comm` is the arm with content (the reduct is the \
      receive's body, a component of the redex); the two congruence arms are `freeVarOf_parMerge` in \
      both directions. **The `new` half is stated in `Closed`'s terms, not here**: the base-sort \
      `Reduce` has no `new` rule — freshness of the bound name is a property of the model's \
      `GUnforgeable`, not of the relation — and what carries it is `Ty.lean`'s `closedNew` (a binder's \
      body is checked and nothing else) with `reduce_closed` composed in. So the row proves the \
      free-variable half in as many words, and names the half it does not." },
  { number := 5, layer := "Rholang",
    statement := "Spatial matching; a free variable is bound at most once — enforced by the port's \
      **normalizer** before any matcher runs, and inside the matcher only on the aggregation path \
      (`aggregate_updates`), while its element-pair and conjunction paths overwrite silently",
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
      definition, so it holds by construction — and it is **not the port's predicate**. The port's \
      *enforcing* check is the **normalizer's**, not the matcher's: a pattern that binds a name twice is \
      refused before any matcher runs, in both contexts (`normalizer.rs:111,289,590,1325`, \
      `UnexpectedReuseOfNameContextFree`/`…ProcContextFree`). **Probed on a devnet** (AUDIT C42): \
      `for (@[v, v] <- x)`, `for (v <- x & v <- y)` and `for (@{\"k\": v, ...v} <- x)` each return 400 \
      with `Free variable v is used twice as a binder …`, while a duplicated *datum* \
      (`x!([*a, *a])` against `for (@[p, q] <- x)`) returns 200 with the same unforgeable hash twice. \
      Inside the matcher the check exists in exactly one place — `aggregate_updates` \
      (`spatial_matcher.rs:644-665`), reached only from the collection path (`list_match`'s tail, \
      `:800`) — while the element-pair path (`fold_match`, `:595-629`) and the conjunction path \
      (`ConnAnd`, `:325-334`) thread their maps with no check, a binding being a plain `insert` \
      (`:477-480`), so a level bound twice would overwrite right-biased with no error. The model \
      carries both halves (`aggregateUpdates_rejects_double_bind`, `freeMapMerge_overwrites`), which is \
      what makes the law a statement about the matcher's clauses — and the probe is what says the \
      matcher half is defence-in-depth rather than a defect, exactly as C42 records. **Owed**: the two \
      proofs above" },
  { number := 6, layer := "Rholang",
    statement := "No globally free variables in a program",
    status := .provedModel,
    declarations := [`Rchain.Closed, `Rchain.closed, `Rchain.closed_eq_Closed, `Rchain.freeVarOf,
      `Rchain.freeVarOf_iff_closed, `Rchain.closed_iff_no_freeVars, `Rchain.Closed_parMerge_iff,
      `Rchain.Closed_receivePar_iff, `Rchain.closed_anyPat],
    rust := ["models/src/types.rs"],
    falsifiable := some "both sides are `decide`d on concrete terms: `bound_var_is_closed` (a `.bound` \
      occurrence is closed — the model reads it as a back-reference the normalizer resolves), \
      `free_var_is_not_closed` (a `.free 3` occurrence is not, and the **level** is what the predicate \
      is about: `freeVarOf p 4` is false of it), and `free_var_is_free_under_par` (a level free \
      inside a `|` is free in the whole). Two mutations were checked while writing it: making \
      `freeVarAt` accept `.bound k` as free breaks the tie's `Expr.evar` arm, and dropping one \
      disjunct from `freeVarOf`'s `Par` arm breaks its `Par` arm",
    note := "**the tie is a proof now.** `freeVarOf` is a `mutual` block mirroring `Ty.lean`'s \
      `closed*` block type for type (`∨` where the checker has `&&`), and `freeVarOf_iff_closed` is \
      the corresponding induction. It discharges two axioms — the opaque predicate and the tie — which \
      the row used to record as 'a defined-but-undefined-elsewhere predicate'. What makes the \
      mirroring exact is the model's de Bruijn *levels*: `closedVar` reads `.bound` as bound and \
      `.free` as open, so the checker refuses exactly the occurrences the predicate accepts" },

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
      wins\") pins. **Two findings are recorded here, one now fixed.** \
      (1) `NonConflicting` is *not* `are_conflicting` read negatively, and saying so was wrong: \
      `are_conflicting` is over two `EventLogIndex`es with three checks, one of which (a potential COMM) \
      is a shared-channel interaction and one of which (produces touching base joins) no state diff can \
      see (`event_log_merging_logic.rs:105-158`), and the predicate the merge branches on is broader \
      again (`casper/src/merging.rs:177-181`). What the model needs is the sufficient condition for \
      commutation, and it is named for that. (2) The Rust test named `combine_is_associative` \
      (`state_change.rs:203-238`) did **not** test associativity — its own comment said the law it pinned \
      was empty-is-identity — so the associativity the merge fold relies on \
      (`casper/src/merging.rs:752-755`) was **untested on the Rust side** (AUDIT C43). It is fixed: the \
      misnamed test is renamed to what it asserts, and \
      `property_tests.rs`'s `law9_state_change_combine_is_associative` is the test — over arbitrary state \
      changes including the join map, and falsified before it was believed (a left-side-dropping \
      `combine` makes it fail in 0.01s)" },
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
    statement := "Finality is the fringe's advance gate: the fringe advances iff the supporting stake is \
      a strict supermajority of the bonded stake, as the exact integer comparison `3·stake > 2·total` \
      (no float rounding)",
    status := .provedModel,
    declarations := [`Rchain.isSuperMajority, `Rchain.bondedSenders, `Rchain.stakeOf,
      `Rchain.stakeOf_eq_none, `Rchain.allBonded, `Rchain.bondedSupport,
      `Rchain.fullPartitionStake, `Rchain.totalStake, `Rchain.calculateFringe,
      `Rchain.finality_iff_supermajority, `Rchain.two_thirds_is_not_supermajority,
      `Rchain.above_two_thirds_is_supermajority, `Rchain.below_two_thirds_is_not_supermajority,
      `Rchain.large_stake_just_above_two_thirds_is_exact,
      `Rchain.i64_overflowing_stakes_do_not_wrap],
    axioms := [],
    rust := ["block-storage/src/dag/finalizer.rs", "sdk/src/consensus.rs"],
    falsifiable := some "each boundary is an independent witness, and each names the port's own test: \
      `two_thirds_is_not_supermajority` fails the moment the comparison is `≥` (`consensus.rs:24`); \
      `large_stake_just_above_two_thirds_is_exact` is false for the `f64` form the Scala oracle uses \
      (`stake.toDouble / totalStake > 2d / 3`, `legacy/sdk/.../consensus/Stake.scala:8`), which cannot \
      represent `2·2⁵³+1` (`sdk/src/consensus.rs:40`); `i64_overflowing_stakes_do_not_wrap` is the \
      case the port's `i128` exists for (`:50`); and `stakeOf_eq_none` is false for a gate that \
      indexed the bonds map by every support sender — the panic the port's `calculate_fringe` skips \
      instead, pinned by `calculate_fringe_ignores_non_bonded_sender` \
      (`block-storage/src/dag/finalizer.rs:282`, and `law14_fringe_requires_supermajority`       at `:268`)",
    note := "**the axiom that stood here was `Nat.mul_comm` twice** — `isSuperMajority s t ↔ s * 3 > \
      t * 2` restated the definition's own body, which is why the row was `vacuous` and why it tied \
      finality to nothing. It is a **theorem** now, and the law is the **gate**: `calculateFringe` is \
      the port's `calculate_fringe` (the full-partition filter, the skip for a non-bonded sender, the \
      exact integer comparison — `finalizer.rs:153-171`) and `nextFringe` is `next_fringe`'s decision \
      with `calculate_finalization`'s progress guard (`:174-197`, `:202-215`). **Where the content is, stated \
      plainly**: the `↔`'s shape is the gate's own `if`, so the weight sits in *what the gate computes*, \
      and that is what the boundary theorems falsify — the strict `>`, the exact `3·stake > 2·total` at \
      the 2⁵³ boundary, and the non-bonded skip. Each is pinned by a named Rust test, which is what \
      makes the row a claim about code rather than arithmetic. Not modelled: `check_min_messages`' \
      arity check ahead of the gate (`finalizer.rs:87`, called at `:188`), which rejects a layer before the stake \
      question is asked" },
  { number := 14, clause := "b", layer := "Casper",
    statement := "A fringe holds one message per bonded validator (an antichain) — **of the fringe the \
      finalizer derives**; over a bare `Fringe` the claim is false and its refutation is proved",
    status := .owed,
    declarations := [`Rchain.Fringe, `Rchain.fringe_antichain_is_false],
    axioms := [],
    rust := ["block-storage/src/dag/finalizer.rs"],
    falsifiable := some "the refutation is in the tree: `fringe_antichain_is_false` exhibits two messages \
      from one sender with different ids in one `Fringe`, which is a value the model can build and the \
      finalizer cannot produce",
    note := "**the axiom that stood here was false**: it quantified over a *bare* `Fringe`, and a \
      `Fringe` is freely constructed, so the refutation is three lines. What it is missing is not a \
      hypothesis on the value but the **derivation** — the fringe the port publishes comes from \
      `calculate_finalization`, which advances only on the support gate and only to a strictly new layer \
      (`finalizer.rs:174-197`, `:202-215`) — and the model has no DAG from which to derive it. Owed: the derivation, \
      at which point the statement becomes provable rather than falsified" },
  { number := 15, layer := "Casper",
    statement := "The fringe is monotone by height and the seen set is monotone (no regression) — the \
      **derived** fringe and the **constructed** seen set; over bare values both claims are false and \
      their refutations are proved",
    status := .owed,
    declarations := [`Rchain.Message, `Rchain.seenOf, `Rchain.seen_monotone_is_false,
      `Rchain.fringe_monotone_is_false, `Rchain.seenOf_contains_justifications, `Rchain.mem_seenOf_self],
    axioms := [],
    rust := ["block-storage/src/dag/message_state.rs", "block-storage/src/dag/finalizer.rs"],
    falsifiable := some "`fringe_monotone_is_false` exhibits two overlapping fringes (one at 5 and 1, one \
      at 3) where both arms of the disjunction fail — so the axiom was false as written; \
      `seen_monotone_is_false` exhibits two unrelated messages where `b` sees `a` and `a` sees `2` but \
      `b` does not. The constructive half is falsifiable too: `seenOf_contains_justifications` fails for \
      a `seenOf` that dropped the justifications' sets, which is the port's `new_seen` \
      (`message_state.rs:54-59`)",
    note := "**two more false axioms, both refuted in the tree.** The content is the *derivation*: a \
      message's seen set is **constructed** as the union of its justifications' seen sets plus its own id \
      (`message_state.rs:54-59`), which the model now has (`seenOf`, with both halves proved: \
      `seenOf_contains_justifications` and `mem_seenOf_self`); and height monotonicity relates \
      *successive* fringes of one validator, which the finalizer's advance gate produces \
      (`finalizer.rs:174-197`, `:202-215`). What remains owed is the **transitive** closure the finalizer leans on — \
      `a ∈ b.seen → a.seen ⊆ b.seen` — which follows from the construction by induction over the DAG, \
      and the DAG is not modelled here. The old row's claim that the seen set is monotone \"(no \
      regression)\" was true of the port and false of the value the axiom quantified over" },
  { number := 16, clause := "a", layer := "Casper",
    statement := "Block number = max(parent) + 1 — as the port's check, which **rejects** a block whose \
      number is not one more than the maximum of its non-failed justifications (`0` when there is none \
      live)",
    status := .provedModel,
    declarations := [`Rchain.Parent, `Rchain.maxParentNumber, `Rchain.BlockNumberValid,
      `Rchain.block_number_max_parent_plus_one, `Rchain.block_number_rejects,
      `Rchain.block_number_universal_is_false],
    axioms := [],
    rust := ["casper/src/validate.rs"],
    falsifiable := some "`block_number_rejects` is the case the port returns `InvalidBlockNumber` for \
      (`validate.rs:135-140`): an off-by-one — `max + 2`, or `max` itself — fails it. The `-1` seed is \
      falsifiable on its own: a block with no live justification must be numbered `0`, so a model that \
      folded a maximum from `0` would demand `1` and reject the genesis-shaped case. And \
      `block_number_universal_is_false` exhibits the refutation of the axiom that stood here — which \
      quantified over every `Block` with no hypothesis at all",
    note := "**the axiom was false, not merely unproven**: it quantified over every `Block`, and a \
      `Block` is freely constructed, so one line refutes it (`block_number_universal_is_false`). The law \
      is re-scoped to the check the code has — a fold over the block's justifications, skipping the \
      failed ones and seeded `-1` (`validate.rs:123-140`) — and the proof is that predicate's \
      elimination, which is the honest shape: the port enforces this by *refusing blocks*, not by \
      maintaining an invariant it states. The model's `Block` carries `justifications` because the check \
      reads them; the `parents : List Nat` field this row's model used does not exist in the port" },
  { number := 16, clause := "b", layer := "Casper",
    statement := "`seqNum` strictly increases **under the sender's justification**: the port requires the \
      block's `seqNum` to be one more than the maximum `seqNum` among the justifications whose sender is \
      this block's sender (`0` when there are none)",
    status := .provedModel,
    declarations := [`Rchain.senderLatestSeq, `Rchain.SeqNumValid, `Rchain.seq_num_strictly_increases,
      `Rchain.seq_num_universal_is_false],
    axioms := [],
    rust := ["casper/src/validate.rs"],
    falsifiable := some "a block whose `seqNum` skips or repeats the sender's latest justification is \
      rejected (`InvalidSequenceNumber`, `validate.rs:158-162`), and the `-1` seed is a case of its own: \
      a sender's first block must be `0`. `seq_num_universal_is_false` is the published refutation of \
      the axiom this replaces — and it refutes it **for a single sender**, which is why the re-scoping \
      is the justification relation and not the sender relation",
    note := "**the axiom was false as stated** (`prev.seqNum + 1 = next.seqNum` over any two blocks), and \
      the diagnosis in the row it replaces — \"the sender relation is missing\" — was wrong: a same-sender \
      pair with a non-consecutive `seqNum` refutes it just as well \
      (`seq_num_universal_is_false`). What the check folds over is the block's justifications **whose \
      sender matches**, against their maximum (`validate.rs:146-161`); the law is re-scoped to that, and \
      the proof is the predicate's elimination. The old model also carried a `seqNum`-ordering axiom over \
      any two blocks, which no port rule states" },
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
      release wrap. That half was a code finding (AUDIT §17 C41) and is **fixed**: the accumulation is
      checked now and its error reaches the merge, with `combining_refuses_a_diff_that_leaves_i64`
      (`rspace/src/merger/event_log_index.rs`) failing on a `wrapping_add`" },
  { number := 18, layer := "Storage",
    statement := "The store's own invariants: the height map is **contiguous** — no holes in block \
      heights — and the fringe identity is **order-independent**, because what the code keys on is a \
      `BTreeSet`, not a list",
    status := .provedModel,
    declarations := [`Rchain.HeightSet, `Rchain.Contiguous, `Rchain.contiguous_insert_succ,
      `Rchain.contiguous_skip_leaves_hole, `Rchain.height_map_universal_is_false, `Rchain.Fringe,
      `Rchain.fringeId, `Rchain.fringeId_perm,
      `Rchain.fringe_identity_order_independent_is_false],
    axioms := [],
    rust := ["block-storage/src/dag/metadata_store.rs", "models/src/fringe_data.rs",
      "block-storage/src/dag/finalizer.rs"],
    falsifiable := some "`contiguous_skip_leaves_hole` is the negative case the store's check exists for: \
      a block whose number is not the successor of the current maximum leaves a hole (what \
      `validate_dag_state` reports, `metadata_store.rs:83-86`) — and the store **cannot derive** the \
      positive direction for itself, since it inspects only the keys it is handed: that step runs through \
      Law 16a's block-number check. `fringeId_perm` fails for an identity that dropped the sort, which \
      is what `fringe_hash_of` would be without its `BTreeSet` input (`models/src/fringe_data.rs:38-43`), \
      and `height_map_universal_is_false` / `fringe_identity_order_independent_is_false` publish the \
      refutations of the two axioms this row replaces",
    note := "**two axioms gone, and neither was a law.** `height_map_contiguous` claimed a property of \
      *any* `List Block` — false, and refuted in the tree (a one-element list numbered 1 has an element \
      above 0 and no element at 0); what the port has is an invariant its *store* checks \
      (`metadata_store.rs:77-87`), so the model states contiguity as the store's meaning and proves the \
      two steps that matter: a successor insert preserves it, a skipped number breaks it. \
      `fringe_identity_order_independent` claimed `Perm → f = g`, also false of a hand-built `Fringe` \
      (refuted in the tree) — in the port the property is **structural**: `fringe` is a \
      `BTreeSet<BlockHash>` wherever it appears, so there is no list order to be invariant under, and the \
      model keeps a list only so the claim can be stated at all, then sorts (`fringeId`) and proves the \
      invariance the type supplies. **Clause split removed**: 18a and 18b now share a status, an empty \
      axiom set and a file" },
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
    statement := "Leg idempotency: `txn_prepare`/`txn_commit`/`txn_abort` are idempotent under the \
      transaction id — a retried leg returns the record it already has — and the two terminal verbs \
      refuse each other",
    status := .provedModel,
    declarations := [`Rchain.applyEffect, `Rchain.leg_idempotent, `Rchain.TxnRecord, `Rchain.Ledger,
      `Rchain.txnPrepare, `Rchain.txnCommit, `Rchain.txnAbort, `Rchain.txnPrepare_idempotent,
      `Rchain.txnCommit_idempotent, `Rchain.txnAbort_idempotent, `Rchain.commit_after_abort_is_an_error,
      `Rchain.abort_after_commit_is_an_error, `Rchain.prepare_refuses_overdraft],
    axioms := [],
    rust := ["rholang/src/native_state.rs"],
    falsifiable := some "the negatives are the witnesses: a second `prepare` that re-escrowed would fail \
      `txnPrepare_idempotent`, and the port's own test pins the balance after a repeated \
      `txn_prepare`/`txn_commit` (`native_state.rs:1442-1479`, and \
      `law28_txn_prepare_rejects_overdraw_and_is_idempotent` at `:1512`); dropping the early return \
      would let a retry fail on insufficient balance *after* the first call had already succeeded, which \
      the port's ordering (`:881-883`, before the balance check) forbids. `commit_after_abort_is_an_error` \
      fails for a verb that allowed the transition — the port returns \
      `Err(\"txn commit: already aborted\")` (`:912`), and `abort_after_commit_is_an_error` the mirror \
      (`:944`). `prepare_refuses_overdraft` is the refusal with the port's own message (`:886`)",
    note := "`leg_idempotent` is **proved now** — `funext` on a pointwise update, which is all it ever \
      needed — but it is the per-*shard-state* view, and the port's verbs are not pointwise updates: \
      they read a record, decide, and write a vault balance *and* a record. So the law is re-modelled on \
      the ledger the port keeps, where idempotence is the **early return on an existing record** \
      (`native_state.rs:881-883`) rather than a coincidence of the arithmetic, and where the **fences** \
      are stated too — commit after abort is an error and abort after commit is an error (`:912`, \
      `:944`), which idempotence alone would permit. A fidelity note: the model's `TxnState` carried a \
      fourth constructor (`proposed`) that the code does not have (`:106-111`); a transaction with no \
      record is `none`, which is how the verbs spell it" },
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
    statement := "A *value* position (a condition, target, datum, element, pattern, name) is normalized \
      against an **empty** `par` — only a statement continuation inherits what precedes it. **Checked on \
      the shapes C21 broke**, not universally: the model threads no accumulator, so the universal \
      statement is not statable over it",
    status := .provedTied,
    declarations := [`Rchain.normalizeAt, `Rchain.Surf],
    corpus := some "c21",
    rust := ["rholang/src/normalizer.rs", "rholang/tests/lean_c21_corpus.rs"],
    falsifiable := some "**the layer is the witness.** Reintroducing C21's defect in `normalize_if` — \
      normalizing the condition against `input.par` rather than `Par::default()` — fails case 1 at once, \
      with the target reported as `@\"c\"!([\"a\"])` beside `(1 == 1)` instead of the condition alone \
      (verified before the corpus was believed). The corpus has a second failure mode of its own: its \
      non-degeneracy half (`c21IsProbe`, decided with the rest) fails for a case whose term normalizes \
      to its own condition, so a case that proved nothing would fail rather than pass quietly",
    note := "**the corpus is the check here, and the model's half is a check rather than a proof of the \
      rule** — which is the honest shape, not a weakness to hide. `normalizeAt` threads only the binder \
      stack `Γ` and takes no accumulated `par`, so the parameter whose misuse was C21 does not exist in \
      the model: \"a value position is normalized against an empty par\" holds of it by construction and \
      there is nothing to falsify. What the two sides do is normalize the same source text \
      independently — the model `decide`s (through `cmpPar`, whose `eq_iff` is proved) that the \
      desugared `Match`'s target is the condition alone, and `lean_c21_corpus.rs` asserts the node's \
      target equals its own normalization of the condition — so what breaks under a regression is the \
      **tie**, and it does. Case 3 is the explicit-`match` control (the desugaring that was never \
      broken) and case 5 a ground condition, so the layer does not rest on the arithmetic clauses \
      agreeing. AUDIT C21 is the history: the target became the preceding par, the pattern cases are \
      `true`/`false`, an unmatched `match` is not an error, and the `if` reduced to nothing at \
      `processedWithSuccess` — invisible for an `if` in first position, which is the idiom contracts \
      mostly use. **Still owed**: the accumulator modelled, which would make the *universal* rule \
      statable; the corpus ties the shape, it does not replace the statement" },
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
