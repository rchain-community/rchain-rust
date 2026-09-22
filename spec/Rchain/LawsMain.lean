import Rchain

/-!
# `rchain-laws` — emit the law register, and check the tree against it

Two halves, because they run at different times and each catches what the other cannot:

**Compile time** (`run_cmd` below). Because this module imports all of `Rchain`, the checks run over the
real environment: every declaration a register row names must exist, and the axioms the rows cite must
be *exactly* the tree's axioms. So `lake build rchain-laws` fails on a register that has drifted from the
tree — a renamed theorem, a dropped law, an axiom added without a row that justifies it. That last one is
the gap `tools/check-lean-conformance.sh`'s `sorry` scan cannot cover: `sorry` is rare in this tree (zero),
`axiom` is how a law is actually assumed, and there were 62 of them with nothing counting.

**Run time** (`main`). The catalog itself: `spec/laws.tsv` (data) and `--format md` (the tables
`spec/INVENTORY.md` and `docs/src/formal/the-43-laws.md` are generated from). Both are committed and the
gate re-emits them and refuses a diff — the same discipline the conformance corpora already follow, which
is what stops the law count drifting from 43 to 29 again.

**How this relates to `spec/INVENTORY.md`.** That file is the reader-facing catalogue, and
`tools/audit-test-register.sh`'s check 8 already verifies that its rows name files which exist (for rows
30–43, the ones carrying the corpus/consumer convention). This register is the machine-side half a shell
check cannot do, because it is written in Lean and therefore checkable against the *elaborated
environment*: every declaration a row cites must exist, and the axioms the rows cite must be exactly the
tree's axioms. The two should converge on one vocabulary. The words here are the ones no file-existence
check can distinguish — a law proved *and tied to the node by a corpus* (`provedTied`) versus one proved
over the model with a prose tie (`provedModel`) — and that distinction is the whole reason C21 could ship
while its model was fine.

Usage:
  lake exe rchain-laws --out spec/laws.tsv
  lake exe rchain-laws --format md --out spec/LAWS.md
-/

open Lean

namespace Rchain
namespace Laws

/-- The laws, stable-sorted by number then clause, so `1a` precedes `1b` and the emitted files are
byte-identical between runs (an unstable order would make the gate's "re-emit and diff" check noise). -/
def ordered : List Law :=
  (laws.toArray.qsort (fun a b =>
    if a.number != b.number then a.number < b.number else a.clause < b.clause)).toList

/-- Entries, not laws: a law whose clauses differ in status or in what they rest on contributes several.
Deliberately *not* called rows — `spec/INVENTORY.md` has one row per law and
`tools/audit-test-register.sh`'s check 8 parses those rows, so calling these rows too would make "43 rows"
mean two different things in the same repo. -/
def entryCount : Nat := laws.length

/-- The number of distinct laws — the number the documents were disagreeing about (19, 29, 43). -/
def lawCount : Nat := (laws.map (·.number)).eraseDups.length

/-- How many laws carry a given status, for the summary line. -/
def statusCount (s : Status) : Nat := (laws.filter (·.status == s)).length

def joinNames (ns : List Name) : String :=
  if ns.isEmpty then "-" else String.intercalate ", " (ns.map (·.toString))

def joinStrs (ss : List String) : String :=
  if ss.isEmpty then "-" else String.intercalate ", " ss

/-! ## The Rust anchors, checked against the filesystem

Not part of the compile-time checks: these need `IO`, and `run_cmd` has no `IO`. They run in `main`
*before* anything is written, so a register that cites code which does not exist emits nothing. -/

/-- The anchor's file: `"rholang/src/merging.rs:102"` → `"rholang/src/merging.rs"`. -/
def anchorPath (a : String) : String := (a.splitOn ":").head!

/-- Whether a row's status *claims something about the code*. Exactly these must name the code they are
about: a `owed`, `deferred`, `open` or `orphaned` row has no model yet, so there is nothing to anchor,
and requiring one would be busywork that teaches people to fill the column in. A `retired` row is the
case worth stating: it claims *the port has no rule of this shape*, which is a claim about code, so it
names the code that was read. -/
def claimsModel (s : Status) : Bool :=
  match s with
  | .provedTied | .provedModel | .axiomByDesign | .vacuous | .retired => true
  | .owed | .deferred | .open | .orphaned => false

/-- Every anchor must be a file that exists, resolved from the repo root (the parent of `spec/`, since
the executable is run from there) or from the working directory. Returns the failures. -/
def rustAnchorFailures : IO (List String) := do
  let mut failures : List String := []
  for l in Laws.laws do
    if claimsModel l.status && l.rust.isEmpty then
      failures := failures ++ [s!"law {l.number}{l.clause} is `{l.status.wire}` and cites no Rust \
        anchor — say which code the model stands for"]
    for a in l.rust do
      let p := anchorPath a
      let here ← System.FilePath.pathExists p
      let up ← if here then pure true else System.FilePath.pathExists (".." / p)
      unless up do
        failures := failures ++ [s!"law {l.number}{l.clause} cites `{a}`, which does not exist"]
  return failures

/-- One line of `spec/laws.tsv`. Columns: number, clause, layer, status, declarations, axioms, corpus,
rust, falsifiable, statement, note. Tab-separated with a header, so a consumer can read it by column.
Newlines and tabs inside a field would break the format, so they are folded to spaces. -/
def tsvRow (l : Law) : String :=
  let flat (s : String) : String := (s.replace "\t" " ").replace "\n" " "
  String.intercalate "\t" [
    toString l.number, l.clause, l.layer, l.status.wire, joinNames l.declarations,
    joinNames l.axioms, l.corpus.getD "-", joinStrs l.rust, flat (l.falsifiable.getD "-"),
    flat l.statement, flat l.note]

def tsv : String :=
  String.intercalate "\n" <|
    ("number\tclause\tlayer\tstatus\tdeclarations\taxioms\tcorpus\trust\tfalsifiable\tstatement\tnote"
      :: ordered.map tsvRow)

/-- The summary sentence both documents open with — the one number that replaces the three competing
counts (19 in a stale note, 29 in two documents, 43 in the tree). -/
def summary : String :=
  s!"{lawCount} laws, {entryCount} entries: {statusCount .provedTied} proved and tied to the node by a \
conformance corpus, {statusCount .provedModel} proved over the model, \
{statusCount .vacuous} proved but vacuous (the statement restates its own definition), \
{statusCount .axiomByDesign} axiomatized by design (the cryptographic primitives), \
{statusCount .owed} owed, {statusCount .deferred} deferred, {statusCount .open} open, \
{statusCount .retired} retired (the port has no rule of that shape — the row says what was read), \
{statusCount .orphaned} orphaned."

/-- A Markdown table cell. A literal `|` ends the cell, and this catalog is full of them — Law 1 is
`sort (p | q) = sort (q | p)` — so it is escaped; GFM honours `\|` inside a code span, which is where ours
sit. The note is wrapped in `<em>` rather than `_…_` so its own underscores cannot pair with the
delimiters. -/
def mdCell (s : String) : String := (s.replace "|" "\\|").replace "\n" " "

/-- The register as Markdown: a table per layer, one row per law, with the axioms it rests on and what
would falsify it. This is the generated half of `docs/src/formal/the-43-laws.md`. -/
def markdown : String :=
  let layers := (laws.map (·.layer)).eraseDups
  let one (l : Law) : String :=
    let num := if l.clause.isEmpty then s!"**{l.number}**" else s!"**{l.number}{l.clause}**"
    let statement :=
      if l.note.isEmpty then l.statement else s!"{l.statement} <br/> <em>{l.note}</em>"
    let cells := [num, statement, l.status.wire, joinNames l.declarations, joinNames l.axioms,
      l.corpus.getD "-", joinStrs l.rust, l.falsifiable.getD "*owed*"]
    "| " ++ String.intercalate " | " (cells.map mdCell) ++ " |"
  let table (layer : String) : String :=
    let rows := ordered.filter (·.layer == layer)
    "| Law | Invariant | Status | Lean | Rests on | Tied by | Models | Falsified by |\n\
     |---|---|---|---|---|---|---|---|\n"
      ++ String.intercalate "\n" (rows.map one)
  String.intercalate "\n\n" (layers.map fun layer => s!"### {layer}\n\n{table layer}")

end Laws
end Rchain

/-- The executable's entry point — a **root-level** `main`, not `Rchain.LawsMain.main`. That is the shape
`Rchain/Corpus.lean` uses (it closes its namespaces at 545-546 and defines `main` at 552), and it is what
lake's `lean_exe` links. With `main` under a namespace the build still *succeeds* — it links some other
`main` — and the binary then rejects `--out` with "unknown long option": a silent mismatch, caught only by
running the thing. -/
def main (args : List String) : IO UInt32 := do
  -- Check 9 first, and before writing anything: a register whose anchors do not resolve must not be
  -- able to emit. The gate re-emits the register and diffs it, so a failing check here fails the gate.
  let anchorFailures ← Rchain.Laws.rustAnchorFailures
  unless anchorFailures.isEmpty do
    for f in anchorFailures do
      IO.eprintln s!"rchain-laws: {f}"
    IO.eprintln s!"rchain-laws: {anchorFailures.length} Rust anchor(s) do not resolve — the register \
      claims to model code that is not there"
    return 1
  let format :=
    match args.findIdx? (fun a => a == "--format") with
    | some i => args[i + 1]?.getD "tsv"
    | none => "tsv"
  if format != "tsv" && format != "md" then
    IO.eprintln s!"rchain-laws: unknown --format {format} (expected tsv or md)"
    return 1
  let body :=
    if format == "md" then s!"{Rchain.Laws.summary}\n\n{Rchain.Laws.markdown}" else Rchain.Laws.tsv
  match args.findIdx? (fun a => a == "--out") with
  | some i =>
      match args[i + 1]? with
      | some path => IO.FS.writeFile path (body ++ "\n")
      | none => do
          IO.eprintln "rchain-laws: --out needs a path"
          return 1
  | none => IO.println body
  return 0

/-! ## The checks, at compile time, against the real environment -/

namespace Rchain
namespace Laws

open Lean Elab Command

/-- Every `axiom` declared in the `Rchain` namespace, by name. Deliberately the *declared* axioms rather
than the transitive ones: this is the trust surface as the tree states it, which is what a register row
must account for. The transitive dependency of an individual theorem is a different (also useful)
question — that is `#print axioms`, one level down. -/
def rchainAxioms : CommandElabM (List Name) := do
  let env ← getEnv
  -- By the name's own components, not `Name.isPrefixOf`: that takes the receiver as the prefix, so
  -- `n.isPrefixOf `Rchain` asks whether the *declaration* is a prefix of `Rchain` — false for every
  -- one of them, which is how this check first reported an empty tree (61 axioms "missing").
  let underRchain (n : Name) : Bool :=
    match n.components with
    | `Rchain :: _ => true
    | _ => false
  return env.constants.fold (init := ([] : List Name)) fun acc n ci =>
    -- `isInternalDetail` skips the compiler's own auxiliaries: local lambdas (`_elambda`, `_lambda`)
    -- and hygiene names are stored as axioms in the environment, and they are not assumptions about the
    -- calculus — counting them would make this check permanently red with five false positives (the
    -- first run reported exactly that: `Comparator.listComparator._elambda_1` and friends).
    if underRchain n && !n.isInternalDetail then
      match ci with
      | .axiomInfo _ => n :: acc
      | _ => acc
    else acc

/-- Which of `ns` are *not* declared in the environment. -/
def missing (env : Environment) (ns : List Name) : List Name :=
  ns.filter (fun n => !env.contains n)

open Lean Elab Command in
run_cmd do
  let env ← getEnv
  let register := laws
  let mut failures : Array String := #[]

  -- 1. Numbering: 1..43, no gaps. A law silently dropped is a `FAIL` here rather than a quietly
  -- smaller catalog.
  let nums := (register.map (·.number)).eraseDups.erase 0 |>.mergeSort (· ≤ ·)
  let expected := List.range 43 |>.map (· + 1)
  if nums != expected then
    failures := failures.push s!"numbering: the register has {nums.length} distinct numbers, expected \
      1..43; missing {expected.filter (fun n => !nums.contains n)}, \
      unexpected {nums.filter (fun n => !expected.contains n)}"

  -- 2. Clauses: a law's clause letters are distinct, so `16a`/`16b` cannot collide.
  for n in nums do
    let cs := (register.filter (·.number == n)).map (·.clause)
    if cs.eraseDups.length != cs.length then
      failures := failures.push s!"law {n}: duplicate clause letter in {cs}"

  -- 3. Reference integrity: a row citing a theorem that no longer exists is a row that stopped meaning
  -- anything, and no grep can see that.
  let cited := (register.map (·.declarations)).join ++ claimedAxioms
  let gone := missing env cited
  if !gone.isEmpty then
    failures := failures.push s!"reference integrity: {gone.length} cited declaration(s) do not exist \
      in `Rchain`: {gone.map (·.toString)}"

  -- 4. Axiom accounting: the axioms the rows cite are exactly the tree's axioms. Two failures in one —
  -- an axiom in the tree that no row cites (a law assumed silently), and a row citing an axiom that is
  -- gone (discharged, but not removed from the register).
  let tree ← rchainAxioms
  let claimed := claimedAxioms.eraseDups
  let unclaimed := tree.filter (fun n => !claimed.contains n)
  let absent := claimed.filter (fun n => !tree.contains n)
  if !unclaimed.isEmpty then
    failures := failures.push s!"axiom accounting: {unclaimed.length} axiom(s) in the tree that no \
      register row cites: {unclaimed.map (·.toString)}. Attribute it to the law it serves, or discharge \
      it."
  if !absent.isEmpty then
    failures := failures.push s!"axiom accounting: {absent.length} axiom(s) cited by a row but not \
      declared in the tree (discharged? then delete the citation): {absent.map (·.toString)}"

  -- 5. A proved law must be falsifiable — the non-vacuity ratchet. `numeric_channels_nonneg`
  -- (`0 ≤ b.number` on a `Nat`) and `finality_iff_supermajority` (which restates its own definition)
  -- were both "proven" and neither could fail. A law nobody can imagine being false is not yet a law.
  for l in register do
    if (l.status == .provedTied || l.status == .provedModel) && l.falsifiable.isNone then
      failures := failures.push s!"non-vacuity: law {l.number}{l.clause} is `{l.status.wire}` with no \
        `falsifiable` witness — say what would make it false, or why it cannot be"

  -- 6. A proved law that rests on an axiom must say so in its own row. Law 1a is the case this exists
  -- for: "proven, residually 30 axioms" in a document, with the qualification nowhere near the claim.
  for l in register do
    if (l.status == .provedTied || l.status == .provedModel) && !l.axioms.isEmpty && l.note.isEmpty then
      failures := failures.push s!"law {l.number}{l.clause} is `{l.status.wire}` and rests on \
        {l.axioms.length} axiom(s) with no `note` saying so"

  -- 7. An axiom may not be attributed to a law with no formalization.
  for l in register do
    if !l.axioms.isEmpty && (l.status == .open || l.status == .orphaned || l.status == .retired) then
      failures := failures.push s!"law {l.number}{l.clause} is `{l.status.wire}` yet cites axioms"

  -- 8. A `vacuous` row must say what the law needs in order to stop being vacuous. The word is an
  -- admission — "proved, but the statement restates its own definition" — and an admission with no plan
  -- is how a gap becomes permanent. A `retired` row must say what was read and why it is not a law: the
  -- word is a *decision*, and a decision with no evidence is an omission wearing a status.
  for l in register do
    if l.status == .vacuous && l.note.isEmpty then
      failures := failures.push s!"law {l.number}{l.clause} is `vacuous` with no note — name the \
        re-scoping it needs, or the word is a resting place rather than a finding"
    if l.status == .retired && l.note.isEmpty then
      failures := failures.push s!"law {l.number}{l.clause} is `retired` with no note — name the code \
        that was read and why the port has no rule of this shape (its `rust` anchor says where)"

  if failures.isEmpty then
    logInfo m!"rchain-laws: the register is consistent — {lawCount} laws, {entryCount} entries, \
      {tree.length} axioms in the tree, all cited"
  else
    for f in failures do
      logError m!"rchain-laws: {f}"
    throwError "rchain-laws: the law register does not match the tree ({failures.size} failure(s), \
      listed above)"

end Laws
end Rchain
