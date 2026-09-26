# Conventions of the Lean development

Short, and only the rules that are enforced or measured somewhere. `AGENTS.md` is the entry point for
the repo; this file is for the things a person editing `spec/` needs to know and would otherwise have to
infer from 15,000 lines.

## The register is the oracle for status and counts

`spec/Rchain/Laws.lean` is the single source of truth for what each law's proof is worth, and
`spec/LAWS.md` / `spec/laws.tsv` are emitted from it. Two consequences:

- **Do not restate a status in prose.** The page that then carried the Proof-of-Stake rows (since folded
  into `docs/src/formal/laws.md`) called laws 44/47 "open" for as long as it took someone to read it;
  `ai-entrypoint.md`'s table carried a paragraph of per-law statuses that had drifted in a dozen places.
  Both now point at `spec/LAWS.md`. A status repeated by hand is a status nothing checks.
- **Do not restate a count in digits** in the reader-facing documents. Use a generated span —
  `<!-- counts:laws-entries --><!-- counts:end -->`, whose content `tools/emit-lean-counts.sh` fills
  from `spec/laws.tsv`. `--check` is step 5c of the gate and fails on a hand-written total too.
  **The scope is that script's `FILES` list, not a list written here** — this sentence used to name
  the files, and the two lists disagreed: the named set was inside the check and the pages outside it
  drifted unread (eleven hand-written totals across nine files, AUDIT C106). Adding a page is adding it
  to `FILES`; the script removes generated spans before scanning, so a page that carries a correct span
  is still checked for a hand-written total *beside* it. Historical figures are spelled out in words,
  because a historical total is still a total a reader may believe.

## Anchors name a symbol, not just a line

A row's prose cites the code it is a claim about, and a citation that carries a line number must also
name the symbol it points at — `native_state.rs:1341-1343` (`txn_prepare`'s early return), not a bare
range. Where a symbol alone suffices, prefer the symbol form (`path.rs:symbol`, the shape the `coq`
field already uses), because a name does not rot and a line does. Write paths in full whenever the
basename is not unique in the tree: `rholang/src/storage.rs`, not `storage.rs` (which `rspace/` also
has); `casper/src/genesis/resources/Pos.rhox`, not `Pos.rhox` (the legacy copy shares every cited
line).

The reason it is a rule rather than a preference: **the register's own checks verify that an anchor's
file exists, and nothing read the lines**, so five rows had drifted silently by 2026-09-24 — law 28
whole (its section moved ~450 lines), law 14a's finalizer cites, law 44's `debit_pos_vault` (cited at
a *call site*), law 9's concatenation (moved to another file), law 3's `par_concat` (imported, not
defined in the cited file). `tools/audit-test-register.sh`'s check 9 now enforces this: every
`path:line` a row carries must resolve, be inside the file, and — the clause that catches the rot —
its ±8-line window must contain an identifier the row itself names. The model's names are snake_case
and the oracle's are camelCase, so the check compares both spellings (`check_min_messages` against
`checkMinMessages`).

## Names that are load-bearing

| shape | means | examples |
|---|---|---|
| `the_*` | a positive witness: uniqueness, minimality, conservation, "the thing is real" | `the_dust_is_real`, `the_minimum_is_unique`, `the_move_does_not_disturb_the_ledger` |
| `a_*` / `an_*` | one concrete negative case — the shape that would be wrong under a weaker rule | `a_claim_before_its_deadline_is_not_paid`, `an_aborted_reply_moves_the_decision` |
| `*_is_false` | a refutation of a statement that stood as an axiom, or of the law's unrestricted form | `block_number_universal_is_false`, `fringe_antichain_is_false` |
| `*_refutes_*` | a refutation of a *named* older statement | `unforgPair_refutes_the_old_statement` |
| `*_diverges` | a divergence witness (the model admits two outcomes) | `one_hop_depth2_diverges`, `effect_reorder_diverges` |
| `*_refuses_*` / `*_rejects_*` | the boundary of a checked operation | `checkedAdd_refuses_overflow`, `block_number_rejects` |

A refuted axiom is **deleted and replaced by one of these**, never narrowed silently: a false axiom is
worse than an owed proof, because it makes everything downstream unsound. That rule is why the register
carries `*_is_false` theorems at all.

Every `provedTied`/`provedModel` row must name at least one such declaration in its `witness` field, or
carry a `corpus` — the register's check 5b. The names must exist (check 3) and must not themselves be
axioms (check 4b): a falsifier that cannot fail is not one.

**The status word is bound to the `corpus` cell — check 5c.** `provedTied` *means* "proved and tied to
the node by a conformance corpus", and until 2026-09-25 nothing related the word to the field: 5b accepts
either a witness or a corpus, and the anchor check asks every proved row for a `rust` anchor, so a row
could call itself tied with no corpus and the build stayed green. That distinction is the register's whole
subject (it is the difference that let C21 ship while its model was fine), so it is the one word that must
not be authorial. A `provedModel` row may still carry a corpus — laws 1a/1b do, because the model's
algebra is coarser than the node's — but then its `note` has to say why the corpus is not the tie the
stronger word would claim.

**What is checked, and what is convention — measured 2026-09-24 (AUDIT C74).** The *rule* above is
machine-checked and holds everywhere: no `proved*` row is without a witness **and** a corpus (0 of 49).
The *vocabulary* is not a predicate over the register and cannot be one: of the 126 entries in the
`witness` field, **63 match one of the six shapes and 63 do not** — and the ones that do not are the
model's own declarations, which the field exists to name (`joinKey_perm`, `mergeChanges_assoc`,
`encodeNode_injective`, `reduce_not_deterministic`). Renaming those to fit a style would rename the
mathematics; a check asserting the shapes over the field fails on 63 legitimate rows, which is how the
claim was falsified rather than argued. The `falsifiable` prose is no better as a source: it mentions
**275** names, of which **232** are declarations under discussion rather than witnesses.

So the shapes are what an author writing a new falsifier should reach for, and the *falsifier-specific*
half of the rule — the one the gate can see — is that the name exists, is not an axiom, and that the row
carries a witness or a corpus at all. A reader who wants to know which shape a given witness takes
should read the name, not expect a check to have done it.

## Options that are set deliberately

- **`maxHeartbeats` is measured, not inherited.** `Rchain/Sort.lean` carries a budget of 4,000,000 with
  the measurement in the comment: 1,000,000 fails there with 80 declarations over it, 2,000,000 passes.
  The 100,000,000 that stood before was a rumour — it would have hidden any regression. If you raise a
  budget, measure the floor the way that file's comment describes and record both numbers.
- **`autoImplicit` stays on, and this was measured too.** Turning it off library-wide (`leanOptions` in
  `lakefile.toml`) fails the build on the *first* module: 53 errors in `Rchain/Cmp.lean` alone, from
  implicitly bound universe variables (`unknown universe level 'u'`) and implicitly bound structure
  fields (`invalid field 'le'`). The development uses the feature, so the switch is a whole-development
  edit rather than a hygiene setting — and `Rchain/Laws.lean` keeps `set_option autoImplicit false`
  locally, which is what caught its first draft's `List Name` silently becoming an implicitly bound
  `Name`.
- **A `termination_by` clause costs reducibility.** A well-founded definition is not kernel-reducible, so
  `rfl`/`decide` stop working on it (`Rchain/Corpus.lean`'s docstring records the case: `connectiveUsed`
  carries no clause *so that* the corpus verdicts stay `decide`-able). Prefer structural recursion where a
  corpus depends on the definition's reduction.

## Things that are deliberately absent

- **No `sorry`, no `admit`** anywhere under `spec/Rchain/` — a hard gate step, and the tree holds zero.
- **No `opaque`, `unsafe`, `partial`, `extern`, `@[implemented_by]`** — verified absent.

**And what notices an assumption arriving by another route — corrected 2026-09-25.** The sentence that
stood here said the register's axiom-accounting check was that thing. It was not, on either of the two
routes the tree actually uses:

- **`native_decide`**, which proves by evaluation in compiled code and admits the result through the axiom
  `Lean.ofReduceBool`: "the Lean compiler and interpreter become part of your trusted code base"
  (`Init/Core.lean`). The word is legal, so the gate's scan did not refuse it, and the accounting folds the
  environment for axioms named `Rchain`, so it could not see it either. **Measured 2026-09-25**: 34 sites in
  six modules, of which **30 became `decide`** (the kernel reduces those goals; the emitted corpora did not
  move a byte, so the conversions are verdict-preserving). `Rchain/LawsMain.lean`'s **check 6b** now
  computes each register-named declaration's transitive axiom set with `Lean.collectAxioms` and holds the
  compiler-resting set at *zero* — six were found, all six converted, and the list is empty and exact in
  both directions — and the gate's **step 2b** counts `native_decide` across `spec/` against a ceiling of
  **4**: the four corpus verdicts in `Rchain/Corpus.lean`, where the kernel does not reduce the goal at all.
- **Lean's own logic** — `propext`, `Quot.sound`, `Classical.choice`, which **225** of the 456
  declarations the register names rest on transitively, carried in by Mathlib's lemmas. The tree
  references none of them in source; the transitive set says otherwise, and check 6b reports the count on
  every build rather than failing on it (a Mathlib development uses them).

The general lesson is the register's own: an assumption is worth what the check that counts it is worth,
and "no `opaque`/`unsafe`/`partial`" is a list of *spellings*, not of routes.
