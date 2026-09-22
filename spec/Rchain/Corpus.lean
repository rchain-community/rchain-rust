import Rchain.Par
import Rchain.Match
import Rchain.Silence
import Rchain.Store
import Rchain.Protocol
import Rchain.Json
import Rchain.Envelope
import Rchain.Lex

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

/-! ## Laws 38 and 40 — the silence layer

Each case is a *term* — a send and a receive on a channel, the receive's body reporting on `@"out"` —
and the verdict `takesStep` gives it: does the pair form a contract step at all? A term that does not
step produces nothing but the consumer's control datum, which is what "silent" means in the port, and
what the rule in `Rchain/Silence.lean` says: the contract rule carries the match as a hypothesis.

The last six cases are **law 40's** (*protocol agreement*: every call has an accepting receive at the
target's arity), and they are here rather than in a layer of their own because the verdict they need is
this one: a call whose arity matches no receive *is* a silent step, so `takesStep` is the same oracle
and a second consumer would be a copy of this one. What law 40 changed is the *rule* — `stepsInBinds`
now requires as many patterns as data, where it used to fail closed on anything but a single datum —
and those cases are what pins the change. -/

/-- The number of cases the silence layer carries (laws 38 and 40). -/
def silenceCaseCount : Nat := 12

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
  -- A `!` send and a `for` receive: both non-persistent, which is the shape the corpus's terms are.
  parMerge (sendParP (strPar sendChan) [datum] false)
    (receiveParP (strPar recvChan) pattern stepBody false)

/-- A send with `data` and a receive with `patterns` on one string channel: the shape a *call* and a
*contract head* make. Law 40's cases are exactly these, with the arities varied. -/
def callPar (data : List Par) (patterns : List Par) : Par :=
  -- A call (`!`, non-persistent) against a **contract head** (replicated) — the flags the node gives
  -- the two shapes, and the reason `ReduceP`'s constructors take them.
  parMerge (sendParP (strPar "c") data false)
    (receiveParPs (strPar "c") patterns stepBody true)

/-- The cases. Case 3 is the shape C22 item 3 turned on, seen from the reduction side rather than the
matcher's; case 5 is a store pair, the single-step half of the C22 item 1 class; cases 4 and 6 are the
two ways a pair fails to be a redex — the wrong channel, and a pattern that does not match. Cases 7-12
are law 40's: a 2-arity call to a two-pattern receive (accepted), 1- and 3-arity calls to the same
receive (silent), the 2-arity call to a *three*-pattern receive — C22 item 2's production instance,
where `extraSlots` called the directory's `write(@key, @value, ret)` with two arguments and none of the
three slots was ever written — the 3-arity call that is accepted, and two equal arities whose
*patterns* do not match (so the arity is not the only thing the rule reads). -/
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
    -- 7. law 40: two patterns, two data — the accepting call.
  , { term := "@\"c\"!(1, 2) | for (@a, @b <- @\"c\") { @\"out\"!(\"step\") }",
      par := callPar [intPar 1, intPar 2] [namePar 0, namePar 1], steps := true }
    -- 8. one datum against two patterns: silent.
  , { term := "@\"c\"!(1) | for (@a, @b <- @\"c\") { @\"out\"!(\"step\") }",
      par := callPar [intPar 1] [namePar 0, namePar 1], steps := false }
    -- 9. three data against two patterns: silent.
  , { term := "@\"c\"!(1, 2, 3) | for (@a, @b <- @\"c\") { @\"out\"!(\"step\") }",
      par := callPar [intPar 1, intPar 2, intPar 3] [namePar 0, namePar 1], steps := false }
    -- 10. C22 item 2's instance: `write!(key, value)` against `write(@key, @value, ret)`.
  , { term := "@\"c\"!(1, 2) | for (@k, @v, @ret <- @\"c\") { @\"out\"!(\"step\") }",
      par := callPar [intPar 1, intPar 2] [namePar 0, namePar 1, namePar 2], steps := false }
    -- 11. the same receive, called at its own arity: accepted.
  , { term := "@\"c\"!(1, 2, 3) | for (@k, @v, @ret <- @\"c\") { @\"out\"!(\"step\") }",
      par := callPar [intPar 1, intPar 2, intPar 3] [namePar 0, namePar 1, namePar 2], steps := true }
    -- 12. equal arities, and a pattern that does not match: the rule reads both.
  , { term := "@\"c\"!(9, {\"x\": 1, \"y\": 2}) | for (@2, @{\"x\": 1, ..._} <- @\"c\") { @\"out\"!(\"step\") }",
      par := callPar [intPar 9, mapOf [("x", intPar 1), ("y", intPar 2)] none]
        [intPar 2, mapOf [("x", intPar 1)] wildRem],
      steps := false }
  ]

/-- Every silence case's verdict holds of the model, `decide`d against `takesStep`. -/
theorem silenceCases_decide : silenceCases.all (fun c => takesStep c.par == c.steps) = true := by
  decide

/-- The layer carries exactly `silenceCaseCount` cases. -/
theorem silenceCases_length : silenceCases.length = silenceCaseCount := by decide

/-- One silence corpus line: layer, the term, whether it steps. -/
def silenceLine (c : SilenceCase) : String :=
  "silence\t" ++ c.term ++ "\t" ++ (if c.steps then "true" else "false")

/-! ## Law 41 — the store layer

Each case is a *term* whose store is read **twice** — a `contract` reader called two times, which is
what makes it replicable — and the number of answers that must come back: `2` when the reader restores
what it consumed, `1` when it has consumed the store for good. The model's side of the case is the same
question asked of `Rchain/Store.lean`: `storeSurvives` after one read. The two sides are deliberately
different shapes — the term has the surface syntax and two calls; the model has the flat `Par` and the
replication — and they have to agree on the verdict, which is the corpus's premise.

Case 2 is the defect that started this (AUDIT C22 item 1: `Inbox.rho`'s zero-argument `read`), case 3 is
its repair (`box!(Nil)`: the read is documented to remove the *messages*, not the container), case 4 is
the shape C22 item 3 taught from the matching side — a restore that lives in only one branch of a
`match` is not a restore, and the term makes the datum take the other branch so the model's conservative
verdict and the node's behaviour are the same claim — and case 5 is the restore that goes to the wrong
channel, which is what makes the model compare channels rather than ask whether *any* send is there.

A *peek* is deliberately absent, for the reason the Directory's reads were never this law's defect:
`for (map <<- mapCh)` consumes nothing, so it cannot consume the store. The model's `Receive` has a
*persistent* flag and no peek one, so a peek case would have to state its own verdict rather than derive
it.

Every term declares the reader's name in the `new` (`new box, read in …`), because a name that is
*not* bound may be used only **once**: the second use of a free name is `UnexpectedReuseOfNameContextFree`
— the Scala's `NameNormalizeMatcher.scala:52` and the port's `normalizer.rs:111`, faithfully the same
rule. A `contract`'s own name is not bound by its definition, so a reader called twice must have its
channel in scope. -/

/-- The number of cases the store layer carries. -/
def storeCaseCount : Nat := 5

/-- A store case: the term as rholang spells it, the model's view of it, and whether the store answers
the second read. -/
structure StoreCase where
  /-- The term: a store on `@"box"`, a reader, and exactly two calls of it. Its answers land on
  `@"out"`, one per call that fired. -/
  term : String
  /-- The model's view: the store's datum beside the replicated reader and its continuation. -/
  par : Par
  /-- Does the store answer the second read? -/
  survives : Bool

/-- A store's datum on `"box"`, beside a replicated reader of `"box"` whose continuation is `body`. -/
def storeWith (datum body : Par) : Par :=
  parMerge (sendPar (strPar "box") [datum]) (replicatedRead (strPar "box") (namePar 0) body)

/-- A `match` on the reader's datum whose case body restores the store — a restore that is only in one
branch, kept in `matches` rather than `sends`, which is why the model does not count it. (`Rchain.`-
qualified: this module's own `MatchCase` is the corpus's case record, a different type.) -/
def restoreInOneBranch (datum : Par) : Par :=
  Par.mk [] [] [] [] [Rchain.Match.mk (one (.evar (.free 0)))
    [Rchain.MatchCase.mk (strPar "m") (sendPar (strPar "box") [datum]) 0]] [] [] []

/-- The cases. -/
def storeCases : List StoreCase :=
  [ -- 1. the reader restores the datum it took: the store answers again.
    { term := "new box, read in { box!(\"m\") | contract read(_) = { for (x <- box) { @\"out\"!(*x) | box!(*x) } } | read!(Nil) | read!(Nil) }"
    , par := storeWith (strPar "m") (sendPar (strPar "box") [strPar "m"])
    , survives := true }
  , -- 2. the reader that consumes: AUDIT C22 item 1, `Inbox.rho`'s zero-argument `read`.
    { term := "new box, read in { box!(\"m\") | contract read(_) = { for (x <- box) { @\"out\"!(*x) } } | read!(Nil) | read!(Nil) }"
    , par := storeWith (strPar "m") (sendPar (strPar "out") [strPar "m"])
    , survives := false }
  , -- 3. the repair: the read removes the messages, and puts the emptied container back.
    { term := "new box, read in { box!([\"m\"]) | contract read(_) = { for (items <- box) { @\"out\"!(*items) | box!(Nil) } } | read!(Nil) | read!(Nil) }"
    , par := storeWith (listPat [strPar "m"] none) (sendPar (strPar "box") [listPat [] none])
    , survives := true }
  , -- 4. the restore is in one branch of a match, and the datum takes the other.
    { term := "new box, read in { box!(\"z\") | contract read(_) = { for (x <- box) { @\"out\"!(*x) | match *x { \"m\" => { box!(*x) } _ => { Nil } } } } | read!(Nil) | read!(Nil) }"
    , par := storeWith (strPar "z")
        (parMerge (sendPar (strPar "out") [strPar "z"]) (restoreInOneBranch (strPar "z")))
    , survives := false }
  , -- 5. the restore goes somewhere else: a datum put back on a *different* channel is not a restore,
    --    which is why the model compares channels rather than asking whether *any* send is there.
    { term := "new box, read in { box!(\"m\") | contract read(_) = { for (x <- box) { @\"out\"!(*x) | @\"boxx\"!(*x) } } | read!(Nil) | read!(Nil) }"
    , par := storeWith (strPar "m") (sendPar (strPar "boxx") [strPar "m"])
    , survives := false }
  ]

/-- Every store case's verdict holds of the model, `decide`d against `storeSurvives`. -/
theorem storeCases_decide :
    storeCases.all (fun c => storeSurvives (strPar "box") c.par == c.survives) = true := by
  decide

/-- The layer carries exactly `storeCaseCount` cases. -/
theorem storeCases_length : storeCases.length = storeCaseCount := by decide

/-- One store corpus line: layer, the term, and how many reads of the store the node must answer — two
calls in every term, so a store that survives answers both. -/
def storeLine (c : StoreCase) : String :=
  "store\t" ++ c.term ++ "\t" ++ (if c.survives then "2" else "1")

/-! ## Law 39 — the protocol layer

The catalog is the law (`Rchain/Protocol.lean`: `replyCatalog`, with its `decide`d consistency checks);
this layer is its rendering. Each row is a *probe*: call the urn with the arguments the row spells, and
the reply must arrive in the row's kind and slots. `args` and `slots` render as `-` when empty, so a
column is never blank and the consumer never has to guess what a missing column means. -/

/-- One protocol corpus line: layer, urn, the call's arguments, the reply's kind, its slots. -/
def protocolLine (r : ReplyRow) : String :=
  "protocol\t" ++ r.urn ++ "\t" ++ (if r.args.isEmpty then "-" else r.args) ++ "\t"
    ++ r.kind.tag ++ "\t" ++ (if r.slots.isEmpty then "-" else
      String.intercalate "," (r.slots.map SlotShape.tag))

/-! ## Law 42 — the JSON layer

Each case is a *value* — a rholang expression the API would receive as a datum — the model's view of the
`Par` it becomes, and what the encode must produce (`none` is the envelope's "absent": the API renders
no value at all). The verdict is `decide`d by comparing the model's rendering of the encode against the
rendering of the declared `JE`, so a case that drifted fails `lake build`; the Rust consumer runs the
same source through the node's own codec and compares the JSON, so a case that disagrees with the node
fails the corpus. The cases cover the envelope's three counts (one → unwrapped, none → absent, two or
more → `ExprPar`) and every `JE` arm except the unforgeable leaf, which the model cannot decode back
(`Rchain/Json.lean`'s boundary note). -/

/-- A JSON case: the value as rholang spells it, the model's view of it, and the JSON the API must
expose — `none` for "no value at all" (the envelope's zero case). -/
structure JsonCase where
  /-- The value, as the API receives it: a datum in a term's send. -/
  source : String
  /-- The model's view of the `Par` that value becomes. -/
  par : Par
  /-- What the encode must produce. -/
  je : Option JE

/-- An integer value. -/
def intOf (n : Int) : Par := one (.ground (.int n))

/-- A boolean value. -/
def boolOf (b : Bool) : Par := one (.ground (.bool b))

/-- A `Par` whose only field is a bundle around `p` (the `expr_from_bundle` arm). -/
def bundleOf (p : Par) : Par := Par.mk [] [] [] [] [] [] [Bundle.mk p true false] []

/-- The cases. -/
def jsonCases : List JsonCase :=
  [ -- 1. one field: the envelope unwraps it.
    { source := "42", par := intOf 42, je := some (.int 42) }
  , { source := "true", par := boolOf true, je := some (.bool true) }
  , { source := "\"hello\"", par := strPar "hello", je := some (.str "hello") }
  , { source := "`rho:id:abc`", par := one (.ground (.uri (codePointsOf "rho:id:abc"))),
      je := some (.uri "rho:id:abc") }
  , -- 2. no field at all: the envelope is "absent", which a response renders as no value.
    { source := "Nil", par := Par.mk [] [] [] [] [] [] [] [], je := none }
  , -- 3. two or more: the envelope is an `ExprPar`.
    { source := "1 | 2", par := parMerge (intOf 1) (intOf 2), je := some (.par [.int 1, .int 2]) }
  , -- 4. the collections, and a tuple inside a list (the nesting the JSON exposes as nesting).
    { source := "[1, true]", par := listPat [intOf 1, boolOf true] none,
      je := some (.list [.int 1, .bool true]) }
  , { source := "Set(1, 2)", par := setPat [intOf 1, intOf 2] none,
      je := some (.set [.int 1, .int 2]) }
  , { source := "(1, \"a\")", par := one (.etuple [intOf 1, strPar "a"]),
      je := some (.tuple [.int 1, .str "a"]) }
  , { source := "[[1]]", par := listPat [listPat [intOf 1] none] none,
      je := some (.list [.list [.int 1]]) }
  , { source := "{\"a\": 1}", par := mapOf [("a", intOf 1)] none,
      je := some (.map [("a", .int 1)]) }
  , -- 5. a bundle exposes its body (one field, so the envelope unwraps that too).
    { source := "bundle+{1}", par := bundleOf (intOf 1), je := some (.int 1) }
  ]

/-- Every JSON case's verdict holds of the model: the encode of the case's `Par` renders to the same
text as the case's declared `JE`. `decide`d, so a case that drifted from `parToJE`/`render` fails the
build rather than being trusted. -/
theorem jsonCases_decide :
    jsonCases.all (fun c => (parToJE c.par).map render == c.je.map render) = true := by
  decide

/-- **Law 42 instantiated on the layer's cases, discharged by computation.** The general statement
(`Rchain/Json.lean`'s `decode_encode`) is an induction over the flat fields and is still owed; what this
theorem adds is that *these* cases are not owed: for each one the kernel reduces the encode, the decode
and the encode again, and the wire text is unchanged. `decide` closes it because every function involved
is structural recursion — the same reason the other layers' verdicts are `decide`d — so a case that
stopped round-tripping fails `lake build` rather than waiting for the induction. -/
theorem jsonCases_round_trip :
    jsonCases.all (fun c =>
      (((parToJE c.par).bind jeToPar).bind parToJE).map render
        == (parToJE c.par).map render) = true := by
  decide

/-- The count the Rust consumer asserts it read. -/
def jsonCaseCount : Nat := 12

/-- The layer carries exactly `jsonCaseCount` cases. -/
theorem jsonCases_length : jsonCases.length = jsonCaseCount := by decide

/-- One JSON corpus line: layer, the value, and the JSON the API must expose (`-` when the envelope is
"absent", so no column is ever blank). -/
def jsonLine (c : JsonCase) : String :=
  "json\t" ++ c.source ++ "\t" ++ (match c.je with | none => "-" | some j => render j)

/-! ## Law 43 — the envelope layer

The catalog is the law (`Rchain/Envelope.lean`: `envelopeCatalog`, with its `decide`d checks); this
layer renders it. Each row is a response type — its name, the endpoint a reader should think of, and
either its struct keys or its tagged union's `tag → keys`. The Rust consumer holds *both* parties to
it: the DTO's own serialization and the served OpenAPI schema. -/

/-- One envelope corpus line: layer, the type, the endpoint, the keys, and the union's tags with their
own keys (`-` where a row has none, so a column is never blank). -/
def envelopeLine (r : EnvelopeRow) : String :=
  "envelope\t" ++ r.name ++ "\t" ++ r.endpoint ++ "\t"
    ++ (if r.keys.isEmpty then "-" else String.intercalate "," r.keys) ++ "\t"
    ++ (if r.variants.isEmpty then "-" else
      String.intercalate ";" (r.variants.map (fun v =>
        v.1 ++ ":" ++ String.intercalate "," v.2)))

/-! ## Law 32 — the lexeme layer

The table is the law (`Rchain/Lex.lean`: `lexemes`, with `lexemes_decide` checking distinctness, the
punctuation class and maximal munch); this layer renders it. Each row is a spelling, the token it lexes
to, what its sample must observe, and the sample itself — so the Rust consumer runs the sample and
reports; nothing about the meaning is the consumer's to decide. -/

/-- One lexeme corpus line: layer, the spelling, its token, the expectation, and the sample term. -/
def lexLine (l : Lexeme) : String :=
  "lex\t" ++ l.spelling ++ "\t" ++ l.token ++ "\t" ++ l.expected ++ "\t" ++ l.sample

end Corpus
end Rchain

open Rchain

/-- `rchain-corpus --layer {flags|match|silence|store|protocol} [--out FILE]` — print the corpus
(stdout by default). -/
def main (args : List String) : IO UInt32 := do
  let want :=
    (args.find? (fun a => a == "flags" || a == "match" || a == "silence" || a == "store"
      || a == "protocol" || a == "json" || a == "envelope" || a == "lex")).getD "flags"
  let (lines, count) :=
    if want == "match" then (Corpus.matchCases.map Corpus.matchLine, Corpus.matchCaseCount)
    else if want == "silence" then
      (Corpus.silenceCases.map Corpus.silenceLine, Corpus.silenceCaseCount)
    else if want == "store" then
      (Corpus.storeCases.map Corpus.storeLine, Corpus.storeCaseCount)
    else if want == "protocol" then
      (replyCatalog.map Corpus.protocolLine, replyCaseCount)
    else if want == "json" then
      (Corpus.jsonCases.map Corpus.jsonLine, Corpus.jsonCaseCount)
    else if want == "envelope" then
      (envelopeCatalog.map Corpus.envelopeLine, envelopeCaseCount)
    else if want == "lex" then
      (lexemes.map Corpus.lexLine, lexemeCount)
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

