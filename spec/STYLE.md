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
- **Do not restate a count in digits** in the reader-facing documents (`spec/README.md`,
  `spec/TYPE-SYSTEM.md`, `spec/INVENTORY.md`, `spec/coq/README.md`, `docs/src/formal/*.md`,
  `docs/src/ai-entrypoint.md`). Use a generated span —
  `<!-- counts:laws-entries -->49 laws and 58 entries<!-- counts:end -->` — and
  `tools/emit-lean-counts.sh` fills it from `spec/laws.tsv`. `--check` is step 5c of the gate and fails
  on a hand-written total too. Historical figures are spelled out in words, because a historical total is
  still a total a reader may believe.

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
- **No `opaque`, `unsafe`, `partial`, `extern`, `@[implemented_by]`** — verified absent, and the register's
  axiom-accounting check is what would notice an assumption arriving by another route.
