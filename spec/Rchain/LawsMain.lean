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

**Run time** (`main`). The catalog itself: `spec/laws.tsv` (data) and `spec/LAWS.md` (the tables, via
`--format md`). Both are committed and the gate re-emits them and refuses a diff — the same discipline the
conformance corpora already follow, which is what stops the law count drifting (two documents once said
29 while the tree held 43). The reader-facing documents that *used* to restate those totals by hand now
take them from `spec/laws.tsv` through `tools/emit-lean-counts.sh`, whose markers the same gate checks.

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

/-- **The ceiling the numbering check requires: deliberately hand-maintained, and deliberately not
derived from the rows.** `lawCount` above counts what is *there*; a check that compared that count to
the rows would be comparing the register to itself, and a law dropped from the list would take its own
evidence with it — which is exactly the drift this register was built to stop (43 laws in the tree while
two documents said 29). So the ceiling is a number a person bumps when a law is added, the check below
requires the rows to be exactly `1..lawCeiling`, and adding a law without bumping it is a build failure
rather than a quietly larger catalog.

It was 43 — the catalog of laws 1–43 — until Programme B's implementation gaps were opened as their own
laws rather than as prose in `spec/RUST-FIRST.md`: 44–47 are the Proof-of-Stake epoch, its split, its
conservation and its withdrawal — the four the port did not implement when the ceiling was written —
48 is the fee consequence of a denied deploy, which the plan's Programme B opened as a row because it
is a rule neither this port nor the Scala it was ported from implements, and 49 is the gas a matched
deploy is charged — the first row about cost rather than state." -/
def lawCeiling : Nat := 49

/-- **The ceiling the completeness check requires: the number of *entries*, hand-maintained for
`lawCeiling`'s own reason.** The numbering check above validates *numbers* (1..`lawCeiling`) and
reference integrity validates *cited declarations*; neither can see a row **dropped**, because a
surviving sibling keeps its number and nothing cites a row by identity. So a deleted clause is a
quietly smaller catalog — measured: 26c's record was deleted by an edit of row 26b's cells and every
check stayed green, while the emitter's summary read "57 entries" against a baseline of 58 (AUDIT
C80). Pinned by hand rather than derived, because a check that counted the rows would compare the
register to itself and a dropped row would take its own evidence along. **Adding a row means bumping
this** — the same tax `lawCeiling` already carries. -/
def entryCeiling : Nat := 58

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

/-- Read a file, trying the path as given and then relative to the repo root (the parent of `spec/`). -/
def readAnchorFile (p : String) : IO (Option String) := do
  try
    return some (← IO.FS.readFile p)
  catch _ =>
    try
      return some (← IO.FS.readFile (".." / p))
    catch _ =>
      return none

/-- **The Coq anchors, checked against the files themselves.** `spec/coq/Laws.v:substPar` must be a file
that exists *and* a file that mentions `substPar`. That is a weaker claim than the `rust` field makes of
a Rust file — it resolves a symbol, not a line — and weaker than a proof: whether that symbol is a
theorem or an `Axiom` is not this field's question, because for most of this catalog's Coq half the
answer is "an axiom" and step 4b of the gate is the thing that counts those. What it does catch is the
register saying a law is stated in Coq when the declaration is not there at all, which no check could
see before. -/
def coqAnchorFailures : IO (List String) := do
  let mut failures : List String := []
  for l in Laws.laws do
    for a in l.coq do
      match a.splitOn ":" with
      | [path, sym] =>
        if sym.isEmpty then
          failures := failures ++ [s!"law {l.number}{l.clause} cites Coq `{a}` with an empty symbol"]
        else
          match (← readAnchorFile path) with
          | none =>
            failures := failures ++ [s!"law {l.number}{l.clause} cites Coq `{a}`, whose file does not \
              exist"]
          | some content =>
            if (content.splitOn sym).length < 2 then
              failures := failures ++ [s!"law {l.number}{l.clause} cites Coq `{a}`, but `{path}` never \
                mentions `{sym}`"]
      | _ =>
        failures := failures ++ [s!"law {l.number}{l.clause} cites Coq `{a}`, which is not \
          `path:symbol`"]
  return failures

/-- **The Rust witnesses, checked against the files themselves.** `rholang/src/storage.rs:refund_storage`
must be a file that exists *and* a file that contains `fn refund_storage`. Weaker than running the test
— `tools/check-rust-witnesses.sh` does that, and refuses a name that matches no test — and much weaker
than a proof: this resolves a symbol, exactly as the `coq` field does. What it catches, and the reason
running alone is not enough, is that a witness whose symbol is not in its file is a *typo or a stale
name*, and cargo would be paid for before anyone found out. -/
def rustWitnessFailures : IO (List String) := do
  let mut failures : List String := []
  for l in Laws.laws do
    for a in l.rustWitness do
      match a.splitOn ":" with
      | [path, sym] =>
        if sym.isEmpty then
          failures := failures ++ [s!"law {l.number}{l.clause} cites Rust witness `{a}` with an empty \
            symbol"]
        else
          match (← readAnchorFile path) with
          | none =>
            failures := failures ++ [s!"law {l.number}{l.clause} cites Rust witness `{a}`, whose file \
              does not exist"]
          | some content =>
            if (content.splitOn s!"fn {sym}").length < 2 then
              failures := failures ++ [s!"law {l.number}{l.clause} cites Rust witness `{a}`, but \
                `{path}` has no `fn {sym}`"]
      | _ =>
        failures := failures ++ [s!"law {l.number}{l.clause} cites Rust witness `{a}`, which is not \
          `path.rs:symbol`"]
  return failures

/-- One line of `spec/laws.tsv`. Columns: number, clause, layer, status, declarations, axioms, corpus,
rust, coq, witness, falsifiable, statement, note, rustWitness. Tab-separated with a header, so a consumer
can read it by column. Newlines and tabs inside a field would break the format, so they are folded to
spaces. **A new column goes at the end**: `tools/audit-test-register.sh`'s check 9 reads this file with
one `read` variable per column, so inserting one in the middle silently shifts `falsifiable` into `note`
and the citation check starts reading prose. (That trap is recorded at check 9 — it was found by
falsifying the check, not by reading it.) -/
def tsvRow (l : Law) : String :=
  let flat (s : String) : String := (s.replace "\t" " ").replace "\n" " "
  String.intercalate "\t" [
    toString l.number, l.clause, l.layer, l.status.wire, joinNames l.declarations,
    joinNames l.axioms, l.corpus.getD "-", joinStrs l.rust, joinStrs l.coq, joinNames l.witness,
    flat (l.falsifiable.getD "-"), flat l.statement, flat l.note, joinStrs l.rustWitness]

def tsv : String :=
  String.intercalate "\n" <|
    ("number\tclause\tlayer\tstatus\tdeclarations\taxioms\tcorpus\trust\tcoq\twitness\tfalsifiable\tstatement\tnote\trustWitness"
      :: ordered.map tsvRow)

/-- How many rows carry a Rust witness — the Rust counterpart of `witness`, and the number that says how
much of the register's Rust half is *run* rather than described. It is not a status: a witness is not a
proof, and the corpus rung still outranks it (`spec/LAWS.md`'s header says so beside the count). -/
def rustWitnessCount : Nat := (laws.filter (·.rustWitness != [])).length

/-- The Rust witnesses as **entries**: every `rustWitness` every row carries, across all rows. A
test may witness two laws — law 45 and law 46 both cite
`rholang/src/native_state.rs:an_epoch_splits_the_pot_and_keeps_the_dust` — so the entry count
exceeds the distinct-test count, and the summary states both rather than one number a reader would
take as 68 distinct tests. Derived, so it cannot rot: the prose version of this number is exactly
what the counts emitter exists to prevent. -/
def rustWitnessEntries : List String := (laws.map (·.rustWitness)).join

/-- How many **distinct** tests those entries name. -/
def rustWitnessDistinct : Nat := rustWitnessEntries.eraseDups.length

/-- The summary sentence both documents open with — the one number that replaces the three competing
counts (19 in a stale note, 29 in two documents, 43 in the tree). -/
def summary : String :=
  s!"{lawCount} laws, {entryCount} entries: {statusCount .provedTied} proved and tied to the node by a \
conformance corpus, {statusCount .provedModel} proved over the model — of which {rustWitnessCount} carry \
a Rust witness the gate runs ({rustWitnessEntries.length} witness entries, {rustWitnessDistinct} \
distinct tests, a test being able to witness two laws) — \
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
would falsify it. This is `spec/LAWS.md`, which the reader-facing pages link to rather than restate. -/
def markdown : String :=
  let layers := (laws.map (·.layer)).eraseDups
  let one (l : Law) : String :=
    let num := if l.clause.isEmpty then s!"**{l.number}**" else s!"**{l.number}{l.clause}**"
    let statement :=
      if l.note.isEmpty then l.statement else s!"{l.statement} <br/> <em>{l.note}</em>"
    let cells := [num, statement, l.status.wire, joinNames l.declarations, joinNames l.axioms,
      l.corpus.getD "-", joinStrs l.rust, joinStrs l.coq, joinNames l.witness,
      l.falsifiable.getD "*owed*"]
    "| " ++ String.intercalate " | " (cells.map mdCell) ++ " |"
  let table (layer : String) : String :=
    let rows := ordered.filter (·.layer == layer)
    "| Law | Invariant | Status | Lean | Rests on | Tied by | Models | Coq | Witness | Falsified by |\n\
     |---|---|---|---|---|---|---|---|---|---|\n"
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
  let coqFailures ← Rchain.Laws.coqAnchorFailures
  let witnessFailures ← Rchain.Laws.rustWitnessFailures
  let anchorFailures := anchorFailures ++ coqFailures ++ witnessFailures
  unless anchorFailures.isEmpty do
    for f in anchorFailures do
      IO.eprintln s!"rchain-laws: {f}"
    IO.eprintln s!"rchain-laws: {anchorFailures.length} anchor(s) do not resolve — the register \
      claims to model code, or to be stated in Coq, that is not there"
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

-- Whether `n` is a declaration under the `Rchain` namespace, by the name's own components and not
-- `Name.isPrefixOf`: that takes the receiver as the prefix, so `n.isPrefixOf `Rchain` asks whether the
-- *declaration* is a prefix of `Rchain` — false for every one of them, which is how the axiom check first
-- reported an empty tree (61 axioms "missing").
def underRchain (n : Name) : Bool :=
  match n.components with
  | `Rchain :: _ => true
  | _ => false

def rchainAxioms : CommandElabM (List Name) := do
  let env ← getEnv
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

/-- The tree's `theorem`/`lemma` declarations. Check 4c uses it: a falsifiability `witness` has to be one
of these, because a *definition* cannot fail. -/
def rchainTheorems : CommandElabM (List Name) := do
  let env ← getEnv
  return env.constants.fold (init := ([] : List Name)) fun acc n ci =>
    if underRchain n && !n.isInternalDetail then
      match ci with
      | .thmInfo _ => n :: acc
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
  let expected := List.range lawCeiling |>.map (· + 1)
  if nums != expected then
    failures := failures.push s!"numbering: the register has {nums.length} distinct numbers, expected \
      1..{lawCeiling}; missing {expected.filter (fun n => !nums.contains n)}, \
      unexpected {nums.filter (fun n => !expected.contains n)} — bump `lawCeiling` when a law is \
      added, or the new law is uncounted (that is what this constant is for)"

  -- 1b. Completeness: the *entries* against a hand-maintained ceiling, not against themselves. A
  -- dropped clause keeps its law's number alive in a sibling and is cited by no one, so nothing above
  -- can see it (AUDIT C80).
  if entryCount != entryCeiling then
    failures := failures.push s!"entries: the register has {entryCount} entries, expected \
      {entryCeiling} — a row dropped from the list takes its own evidence with it, which is what this \
      ceiling exists to catch; bump `entryCeiling` when a row is added (the tax `lawCeiling` carries)"

  -- 2. Clauses: a law's clause letters are distinct, so `16a`/`16b` cannot collide.
  for n in nums do
    let cs := (register.filter (·.number == n)).map (·.clause)
    if cs.eraseDups.length != cs.length then
      failures := failures.push s!"law {n}: duplicate clause letter in {cs}"

  -- 3. Reference integrity: a row citing a theorem that no longer exists is a row that stopped meaning
  -- anything, and no grep can see that. `witness` is here because it is the one field that *became*
  -- checkable rather than being written checkable: the `falsifiable` sentence it is extracted from also
  -- names Rust tests and `file:line` references, which no check can resolve, so those stay prose and the
  -- Lean half is a declaration list.
  let cited := (register.map (·.declarations)).join ++ claimedAxioms
    ++ (register.map (·.witness)).join
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

  -- 4b. An axiom may not be a falsifiability witness. A row that names the assumption it rests on as the
  -- thing that would falsify it is naming decoration: an axiom cannot fail.
  let axiomDeco := (register.map (·.witness)).join.filter (fun n => tree.contains n)
  if !axiomDeco.isEmpty then
    failures := failures.push s!"non-vacuity: {axiomDeco.eraseDups.length} `witness` name(s) are axioms \
      — a falsifier that cannot fail is not one: {axiomDeco.eraseDups.map (·.toString)}"

  -- 4c. A falsifiability witness must be a **theorem**. 4b refuses an axiom — a falsifier that cannot fail
  -- is not one — and its sibling is the *definition*: row 44's `falsifiable` cell named
  -- `PosState`/`isBoundary`/`epochStep` before the witness ratchet landed, a structure and two functions,
  -- none of which can fail either. Existence is check 3's job (`missing`); this is the *shape*, which a
  -- machine can see and a reader has to remember. The measured state when this was added: all 107 witness
  -- names are theorems, so it is a ratchet that starts green.
  let theorems ← rchainTheorems
  for l in register do
    for w in l.witness do
      if env.contains w && !theorems.contains w then
        failures := failures.push s!"non-vacuity: law {l.number}{l.clause}'s `witness` {w} is declared \
          but is not a theorem — a definition cannot fail, so naming one is decoration"

  -- 5. A proved law must be falsifiable — the non-vacuity ratchet. `numeric_channels_nonneg`
  -- (`0 ≤ b.number` on a `Nat`) and `finality_iff_supermajority` (which restates its own definition)
  -- were both "proven" and neither could fail. A law nobody can imagine being false is not yet a law.
  --
  -- 5b extends it from prose to declarations: the `falsifiable` sentence must be anchored on something the
  -- environment has, either a named witness or the corpus that pins the row to the node. Row 3's
  -- sentence had been naming a declaration that did not exist; a sentence nothing checks is how that
  -- happens.
  for l in register do
    if (l.status == .provedTied || l.status == .provedModel) && l.falsifiable.isNone then
      failures := failures.push s!"non-vacuity: law {l.number}{l.clause} is `{l.status.wire}` with no \
        `falsifiable` witness — say what would make it false, or why it cannot be"
    if (l.status == .provedTied || l.status == .provedModel)
        && l.witness.isEmpty && l.corpus.isNone then
      failures := failures.push s!"non-vacuity: law {l.number}{l.clause} is `{l.status.wire}` with \
        neither a `witness` declaration nor a `corpus` — the falsifiability claim is prose nothing \
        checks; name the declaration it rests on"

  -- 5c. The status word is bound to the `corpus` cell — the one place the vocabulary could still be
  -- authorial. `provedTied` *means* "proved and tied to the node by a conformance corpus" (this module's
  -- doc, and the `Status` constructors below), and nothing related the two: `claimsModel` requires a
  -- `rust` anchor of every proved row and 5b accepts *either* a witness *or* a corpus, so a row could call
  -- itself tied with no corpus and the build stayed green. That distinction — proved against the *running
  -- node* versus proved about a model a human keeps in sync — is the register's whole subject (it is the
  -- difference that let C21 ship while its model was fine), so it is the one word that must not be a
  -- promise. The converse is the same discipline in the other direction: a `proved-model` row may carry a
  -- corpus (laws 1a/1b do — the model's algebra is coarser than the node's), but then its `note` has to
  -- say why the corpus is not the tie the stronger word would claim.
  for l in register do
    if l.status == .provedTied && l.corpus.isNone then
      failures := failures.push s!"tie vocabulary: law {l.number}{l.clause} is `proved-tied` with no \
        `corpus` — that status means tied to the node by a conformance corpus, so either name the layer \
        that pins it or write `proved-model`, which is the word for a prose tie"
    if l.status == .provedModel && l.corpus.isSome && l.note.isEmpty then
      failures := failures.push s!"tie vocabulary: law {l.number}{l.clause} is `proved-model` while \
        carrying a `corpus` — say in its `note` why the corpus is not the tie that word means (the layer \
        may be partial, as law 1a's is: the model's algebra is coarser than the node's)"

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
