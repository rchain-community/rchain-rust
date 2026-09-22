import Rchain.Par
import Rchain.Match
import Rchain.Silence

/-!
# The conformance corpus, generated from the specification

`lake exe rchain-corpus --layer flags --out …` prints the corpus the Rust is checked against, 1:1.
Each case carries its own source text and the *pattern the model believes that text denotes*: the
verdict is the model's (`decide`d here, so `lake build` fails if the model and the case disagree), and
the Rust consumer parses the same source and computes its own verdict, so a source that drifted from the
model fails the check instead of being trusted. Two independent parties, one source of truth each.

Law 35 (concreteness soundness) is the first layer, because the silent defects of AUDIT C9-C22 all pass
through one predicate: `spatial_match`'s first line consults `pattern.connective_used` and short-circuits
to structural equality when it is false. So a collection pattern whose only non-concreteness is a
`..._`/`...rest` remainder either matches partially or never matches at all, depending on a flag set far
away in the normalizer — which is exactly C22 item 3, and C19/C20's root cause.
-/

namespace Rchain
namespace Corpus

/-- The number of cases the flags layer carries, stated here so the Rust consumer can assert it read
every one (a corpus that silently shrinks is a check that stopped checking). -/
def flagCaseCount : Nat := 17

/-- A corpus case: the receive bind's source, the pattern the model believes it denotes, and the
verdict `connectiveUsed` must return for that pattern. -/
structure FlagCase where
  /-- The bind as rholang spells it — `@<pattern>` for a collection (a collection is not a *name*), bare
  for a wildcard or a name. The Rust consumer wraps it as `for (<bind> <- chan) { Nil }` and reads the
  pattern's verdict. -/
  source : String
  /-- The pattern, as the model sees it. -/
  par : Par
  /-- What `connectiveUsed` must return. -/
  expected : Bool

/-- A `Par` holding one expression. -/
def one (e : Expr) : Par := Par.mk [] [] [] [e] [] [] [] []

/-- A map pattern. -/
def mapPat (kvs : List (Par × Par)) (r : Option Var) : Par := one (.emap kvs r)

/-- A list pattern. -/
def listPat (ps : List Par) (r : Option Var) : Par := one (.elist ps r)

/-- A set pattern. -/
def setPat (ps : List Par) (r : Option Var) : Par := one (.eset ps r)

/-- An integer. -/
def intPar (n : Int) : Par := one (.ground (.int n))

/-- A string. -/
def strPar (s : String) : Par := one (.ground (.str (s.toList.map Char.toNat)))

/-- A name, as a pattern variable. -/
def namePar (n : Nat) : Par := one (.evar (.free n))

/-- The wildcard. -/
def wildPar : Par := one (.evar .wildcard)

/-- The wildcard remainder (`..._`). -/
def wildRem : Option Var := some .wildcard

/-- A named remainder (`...rest`). -/
def namedRem : Option Var := some (.free 0)

/-- The cases. The first three are the same shape with and without a tail: a *map* was the one form the
port forgot to mark non-concrete (C22 item 3), and it hid behind the cases that all carried a free
variable as well as a remainder. The last two are the production shapes the audit chased:
`Inbox.rho:141`'s `[type, subtype, map /\ {a: b, ..._}]` and `Group.rho:140`'s nested partial maps. -/
def flagCases : List FlagCase :=
  [ { source := "@{}", par := mapPat [] none, expected := false }
  , { source := "@{\"x\": 1}", par := mapPat [(strPar "x", intPar 1)] none, expected := false }
  , { source := "@{\"x\": 1, \"y\": 2}",
      par := mapPat [(strPar "x", intPar 1), (strPar "y", intPar 2)] none, expected := false }
  , { source := "@{\"x\": 1, ..._}", par := mapPat [(strPar "x", intPar 1)] wildRem,
      expected := true }
  , { source := "@{\"x\": 1, ...rest}", par := mapPat [(strPar "x", intPar 1)] namedRem,
      expected := true }
  , { source := "@{..._}", par := mapPat [] wildRem, expected := true }
  , { source := "@[]", par := listPat [] none, expected := false }
  , { source := "@[1, 2]", par := listPat [intPar 1, intPar 2] none, expected := false }
  , { source := "@[1, ..._]", par := listPat [intPar 1] wildRem, expected := true }
  , { source := "@[1, ...rest]", par := listPat [intPar 1] namedRem, expected := true }
  , { source := "@Set()", par := setPat [] none, expected := false }
  , { source := "@Set(1, 2)", par := setPat [intPar 1, intPar 2] none, expected := false }
  , { source := "@Set(1, ..._)", par := setPat [intPar 1] wildRem, expected := true }
  , { source := "@{\"x\": v7}", par := mapPat [(strPar "x", namePar 7)] none, expected := true }
  , { source := "_", par := wildPar, expected := true }
  , { source := "@[{\"x\": 1, ..._}]",
      par := listPat [mapPat [(strPar "x", intPar 1)] wildRem] none, expected := true }
  , { source := "@{\"g\": {\"info\": {\"x\": 1, ..._}, ..._}, ..._}",
      par := mapPat
        [(strPar "g", mapPat [(strPar "info", mapPat [(strPar "x", intPar 1)] wildRem)] wildRem)]
        none,
      expected := true }
  ]

/-- Every case's verdict holds of the model. `decide` discharges all of them because the predicate is
mutually *structural* recursion — each call takes a field of its argument — so the kernel reduces it by
computation. That is why `connectiveUsed` carries no `termination_by` clause: a well-founded definition
would not be reducible, and this theorem could not be closed by `decide`. -/
theorem flagCases_decide : flagCases.all (fun c => connectiveUsed c.par == c.expected) = true := by
  decide

/-- The layer carries exactly `flagCaseCount` cases. -/
theorem flagCases_length : flagCases.length = flagCaseCount := by decide

/-- One corpus line: layer, the source, the verdict. Tab-separated rather than JSON so the consumer
needs no dependency, and so a pattern's punctuation never has to be escaped. -/
def flagLine (c : FlagCase) : String :=
  "flags\t" ++ c.source ++ "\t" ++ (if c.expected then "true" else "false")

/-! ## Law 37 — the matching layer

Each case is a *pattern* (as a receive bind spells it — `@` for a collection, since a collection is
not a name) and a *target*, both as rholang source, plus the verdict `spatialMatch` gives them. The
Rust consumer runs `target` into a channel, receives with `pattern`, and must get the same verdict.
The two shapes the ten incidents lived in are here: a partial map with a ground entry (C19/C20) and a
partial map whose only non-concreteness is the remainder (C22 item 3), each with the shape that must
*not* match beside it. -/

/-- The number of cases the matching layer carries. -/
def matchCaseCount : Nat := 15

/-- One matching case: the bind's source, the target's source, the model's view of both, and the
verdict. The verdict is `decide`d against `spatialMatch` (`matchCases_decide`), which is what makes
this corpus a law's corpus rather than a table of opinions. -/
structure MatchCase where
  /-- The receive bind, as rholang spells it. -/
  bind : String
  /-- The datum, as rholang spells it. -/
  target : String
  /-- The pattern, as the model sees it. -/
  patternPar : Par
  /-- The target, as the model sees it. -/
  targetPar : Par
  /-- What `spatialMatch` must say. -/
  expected : Bool
  /-- Whether the row is really about *rejection* rather than matching: a pattern that binds a level
  twice is refused by the normalizer (loudly, `UnexpectedReuseOfProcContextFree`) before any match
  happens, so its honest verdict is neither `true` nor `false`. The Rust consumer asserts the refusal
  for these rows — that is law 5 in the port, where the Scala's `addedVars.distinct` runs. -/
  rejected : Bool := false

/-- A map pattern with string keys. -/
def mapOf (kvs : List (String × Par)) (r : Option Var) : Par :=
  mapPat (kvs.map (fun kv => (strPar kv.1, kv.2))) r

/-- The cases. Case 3 is the rgov gate's own shape (`{"read": *MCAread, ..._}` against a three-key
dictionary); case 13 is law 5's linearity — the shape the old axiom `pattern_binds_at_most_once`
declared impossible (AUDIT C26); case 14 is the same shape with *two* distinct variables, which that
axiom denied outright. -/
def matchCases : List MatchCase :=
  [ { bind := "@{\"x\": 1}", target := "{\"x\": 1}",
      patternPar := mapOf [("x", intPar 1)] none,
      targetPar := mapOf [("x", intPar 1)] none, expected := true }
  , { bind := "@{\"x\": 1}", target := "{\"x\": 1, \"y\": 2}",
      patternPar := mapOf [("x", intPar 1)] none,
      targetPar := mapOf [("x", intPar 1), ("y", intPar 2)] none, expected := false }
  , { bind := "@{\"x\": 1, ..._}", target := "{\"x\": 1, \"y\": 2}",
      patternPar := mapOf [("x", intPar 1)] wildRem,
      targetPar := mapOf [("x", intPar 1), ("y", intPar 2)] none, expected := true }
  , { bind := "@{\"x\": 1, ...rest}", target := "{\"x\": 1, \"y\": 2}",
      patternPar := mapOf [("x", intPar 1)] namedRem,
      targetPar := mapOf [("x", intPar 1), ("y", intPar 2)] none, expected := true }
  , { bind := "@{\"x\": 9, ..._}", target := "{\"x\": 1, \"y\": 2}",
      patternPar := mapOf [("x", intPar 9)] wildRem,
      targetPar := mapOf [("x", intPar 1), ("y", intPar 2)] none, expected := false }
  , { bind := "@{\"read\": v7, ..._}", target := "{\"read\": 1, \"write\": 2, \"grant\": 3}",
      patternPar := mapOf [("read", namePar 7)] wildRem,
      targetPar := mapOf [("read", intPar 1), ("write", intPar 2), ("grant", intPar 3)] none,
      expected := true }
  , { bind := "@[1, ..._]", target := "[1, 2, 3]",
      patternPar := listPat [intPar 1] wildRem,
      targetPar := listPat [intPar 1, intPar 2, intPar 3] none, expected := true }
  , { bind := "@[1]", target := "[1, 2]",
      patternPar := listPat [intPar 1] none,
      targetPar := listPat [intPar 1, intPar 2] none, expected := false }
  , { bind := "@Set(1, ..._)", target := "Set(1, 2)",
      patternPar := setPat [intPar 1] wildRem,
      targetPar := setPat [intPar 1, intPar 2] none, expected := true }
  , { bind := "@[]", target := "[]",
      patternPar := listPat [] none, targetPar := listPat [] none, expected := true }
  , { bind := "@{}", target := "{\"a\": 1}",
      patternPar := mapPat [] none,
      targetPar := mapOf [("a", intPar 1)] none, expected := false }
  , { bind := "@{..._}", target := "{\"a\": 1}",
      patternPar := mapPat [] wildRem,
      targetPar := mapOf [("a", intPar 1)] none, expected := true }
  , { bind := "@{\"a\": {\"b\": 1, ..._}, ..._}", target := "{\"a\": {\"b\": 1, \"c\": 2}, \"d\": 3}",
      patternPar := mapOf [("a", mapOf [("b", intPar 1)] wildRem)] wildRem,
      targetPar := mapOf [("a", mapOf [("b", intPar 1), ("c", intPar 2)] none), ("d", intPar 3)] none,
      expected := true }
  , { bind := "@{\"a\": v1, \"b\": v1}", target := "{\"a\": 1, \"b\": 2}",
      patternPar := mapOf [("a", namePar 1), ("b", namePar 1)] none,
      targetPar := mapOf [("a", intPar 1), ("b", intPar 2)] none, expected := false,
      rejected := true }
  , { bind := "@{\"a\": v1, \"b\": v2}", target := "{\"a\": 1, \"b\": 2}",
      patternPar := mapOf [("a", namePar 1), ("b", namePar 2)] none,
      targetPar := mapOf [("a", intPar 1), ("b", intPar 2)] none, expected := true }
  ]

/-- Every matching case's verdict holds of the model. `decide`, because the clauses are structurally
recursive on fuel — the reason `spatialMatch` carries fuel at all (`Rchain/Match.lean`'s note). -/
theorem matchCases_decide :
    matchCases.all (fun c =>
      if c.rejected then decide (linear c.patternPar = false) else spatialMatch c.targetPar c.patternPar == c.expected)
      = true := by
  decide

/-- The layer carries exactly `matchCaseCount` cases. -/
theorem matchCases_length : matchCases.length = matchCaseCount := by decide

/-- One matching corpus line: layer, the bind, the datum, the verdict. -/
def matchLine (c : MatchCase) : String :=
  "match\t" ++ c.bind ++ "\t" ++ c.target ++ "\t"
    ++ (if c.rejected then "rejected" else if c.expected then "true" else "false")

/-! ## Law 38 — the silence layer

Each case is a *term* — a send and a receive on a channel, the receive's body reporting on `@"out"` —
and the verdict `takesStep` gives it: does the pair form a contract step at all? A term that does not
step produces nothing but the consumer's control datum, which is what "silent" means in the port, and
what the rule in `Rchain/Silence.lean` says: the contract rule carries the match as a hypothesis. -/

/-- The number of cases the silence layer carries. -/
def silenceCaseCount : Nat := 6

/-- A silence case: the term as rholang spells it, the model's view of it, and whether it steps. -/
structure SilenceCase where
  /-- The term's source: a send, a receive whose body reports on `out`, and nothing else. -/
  term : String
  /-- The model's view of that term. -/
  par : Par
  /-- Does it form a contract step? -/
  steps : Bool

/-- The body a silence case's receive runs when it fires: `@"out"!("step")`. -/
def stepBody : Par := sendPar (strPar "out") [strPar "step"]

/-- A send-and-receive term on string channels, with `pattern` as the receive's bind. -/
def pairPar (sendChan : String) (datum : Par) (recvChan : String) (pattern : Par) : Par :=
  parMerge (sendPar (strPar sendChan) [datum]) (receiveParP (strPar recvChan) pattern stepBody)

/-- The cases. Case 3 is the shape C22 item 3 turned on, seen from the reduction side rather than the
matcher's; case 5 is a store pair, the single-step half of the C22 item 1 class; cases 4 and 6 are the
two ways a pair fails to be a redex — the wrong channel, and a pattern that does not match. -/
def silenceCases : List SilenceCase :=
  [ { term := "@\"c\"!(1) | for (x <- @\"c\") { @\"out\"!(\"step\") }",
      par := pairPar "c" (intPar 1) "c" (namePar 0), steps := true }
  , { term := "@\"c\"!(1) | for (@{\"k\": 9} <- @\"c\") { @\"out\"!(\"step\") }",
      par := pairPar "c" (intPar 1) "c" (mapOf [("k", intPar 9)] none), steps := false }
  , { term := "@\"c\"!({\"x\": 1, \"y\": 2}) | for (@{\"x\": 1, ..._} <- @\"c\") { @\"out\"!(\"step\") }",
      par := pairPar "c" (mapOf [("x", intPar 1), ("y", intPar 2)] none) "c"
        (mapOf [("x", intPar 1)] wildRem),
      steps := true }
  , { term := "@\"c\"!(1) | for (x <- @\"d\") { @\"out\"!(\"step\") }",
      par := pairPar "c" (intPar 1) "d" (namePar 0), steps := false }
  , { term := "@\"s\"!({}) | for (@m <- @\"s\") { @\"out\"!(\"step\") }",
      par := pairPar "s" (mapPat [] none) "s" (namePar 0), steps := true }
  , { term := "@\"c\"!(1) | for (@2 <- @\"c\") { @\"out\"!(\"step\") }",
      par := pairPar "c" (intPar 1) "c" (intPar 2), steps := false }
  ]

/-- Every silence case's verdict holds of the model, `decide`d against `takesStep`. -/
theorem silenceCases_decide : silenceCases.all (fun c => takesStep c.par == c.steps) = true := by
  decide

/-- The layer carries exactly `silenceCaseCount` cases. -/
theorem silenceCases_length : silenceCases.length = silenceCaseCount := by decide

/-- One silence corpus line: layer, the term, whether it steps. -/
def silenceLine (c : SilenceCase) : String :=
  "silence\t" ++ c.term ++ "\t" ++ (if c.steps then "true" else "false")

end Corpus
end Rchain

open Rchain

/-- `rchain-corpus --layer {flags|match} [--out FILE]` — print the corpus (stdout by default). -/
def main (args : List String) : IO UInt32 := do
  let want := (args.find? (fun a => a == "flags" || a == "match" || a == "silence")).getD "flags"
  let (lines, count) :=
    if want == "match" then (Corpus.matchCases.map Corpus.matchLine, Corpus.matchCaseCount)
    else if want == "silence" then
      (Corpus.silenceCases.map Corpus.silenceLine, Corpus.silenceCaseCount)
    else (Corpus.flagCases.map Corpus.flagLine, Corpus.flagCaseCount)
  if lines.length != count then
    IO.eprintln s!"rchain-corpus: {want}: the case list and the declared count disagree"
    return 1
  let out := String.intercalate "\n" lines
  match args.findIdx? (fun a => a == "--out") with
  | some i =>
    match args[i + 1]? with
    | some path => IO.FS.writeFile path (out ++ "\n")
    | none => IO.eprintln "rchain-corpus: --out needs a path"; return 1
  | none => IO.println out
  return 0

