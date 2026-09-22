import Rchain.Par

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

end Corpus
end Rchain

open Rchain

/-- `rchain-corpus --layer flags [--out FILE]` — print the corpus (stdout by default). -/
def main (args : List String) : IO UInt32 := do
  let lines := Corpus.flagCases.map Corpus.flagLine
  if lines.length != Corpus.flagCaseCount then
    IO.eprintln "rchain-corpus: the case list and flagCaseCount disagree"
    return 1
  let out := String.intercalate "\n" lines
  match args.findIdx? (fun a => a == "--out") with
  | some i =>
    match args[i + 1]? with
    | some path => IO.FS.writeFile path (out ++ "\n")
    | none => IO.eprintln "rchain-corpus: --out needs a path"; return 1
  | none => IO.println out
  return 0
