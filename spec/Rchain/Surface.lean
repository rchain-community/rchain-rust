import Rchain.Par

/-!
# Laws 30, 31 and 34 — the surface language, as data

`rholang_mercury.cf` is the grammar (BNFC, `legacy/rholang/src/main/bnfc/`), and laws 30/31 are the
two directions of the parser's relation to it: every term the Rust parser accepts is in the grammar
(30), and every grammar term is accepted modulo a list of documented deviations (31). Neither
direction can be *stated* without a type for the grammar's terms — a predicate over the flat `Par`
cannot work, because the flat `Par` is a sorted multiset: it has no way to hold "the source wrote
`(x)`", "the source wrote `!!`", or the order the source wrote things in, and the laxnesses the laws
are about (a trailing separator, a missing `in`, trailing input) are facts about *token strings* that
vanish the moment a term becomes a tree.

So this module holds the surface language:

- **`Surf`** — one constructor per `Proc` production of the `.cf`, named after its label. The
  precedence cascade (`Proc1` … `Proc16`) is **collapsed**: BNFC's `ProcN ::= ProcN+1` coercions are
  the identity on the generated tree, a group contributes no node (`PExprs` is `"(" Proc4 ")"` and the
  port asserts `parse("(3 + 5)") = parse("3 + 5")`, `parser.rs:772-780`), and the port's own AST
  collapses them too (`proc_ast.rs`). Collapsing models the *language*; keeping sixteen levels would
  model the parse table.
- **`grammarProductions`** — every production label in the `.cf`, in file order, and
  **`productionWitnesses`** — a term for each, with `witnesses_cover` deciding the table **both ways**
  (no production unwitnessed, no witness naming a production the grammar does not have).
- **`normalize`** — the desugaring into the flat `Par`, which is where laws 34 and 36 live: the
  *value-position* rule (`if E …` normalizes `E` against an empty par, and only a statement
  continuation inherits) is a property of these arms, and the output's shape is what law 36's
  closedness statement is about. Its domain is named in `surfaceBoundaries` rather than implied: a
  construct outside it returns `none`, never a sentinel (`Rchain/Json.lean`'s unforgeable leaf is the
  precedent).

**Boundary, stated:** the *lexer* and the *parser* are `Rchain/Parse.lean` and the printer is
`Rchain/Print.lean`; this module is the type they produce and the function law 33's round-trip is
stated over.
-/

namespace Rchain

-- This module's declarations are one 45-constructor mutual inductive with `List` fields, a
-- desugaring defined over it, and an 82-row witness table — Lean's default budget of 200 000
-- heartbeats is not enough for any of the three (measured: `whnf` on the inductive alone times out).
-- Raised for the whole file rather than per-declaration; it is a performance setting, so the kernel
-- still checks everything here.
set_option maxHeartbeats 8000000

/-- A variable as the *source* spells it. The surface has no de Bruijn levels: binding is the
normalizer's business, which is why this is a name and not a `Var`. -/
abbrev SVar := String

/-- A simple type (`SimpleTypeBool` … `SimpleTypeByteArray`). -/
inductive STy where
  | bool | int | bigInt | str | uri | byteArray
deriving BEq, DecidableEq, Repr

/-- The grammar's `Ground`. `bigint`, `str` and `uri` carry the **raw literal** — exactly as
`GroundBigInt String`, `GroundString String` and `GroundUri String` do in the port's `proc_ast.rs`. -/
inductive SGround where
  | bool (b : Bool)
  | bigint (digits : String)
  | int (digits : String)
  | str (raw : String)
  | uri (raw : String)
deriving BEq, DecidableEq, Repr

/-- A bundle's capability letter (`bundle+`, `bundle-`, `bundle0`, `bundle`). -/
inductive SBundle where
  | write | read | equiv | readWrite
deriving BEq, DecidableEq, Repr

/-- The kinds of `VarRefKind`: `= x` (a reference to a process variable) and `=* x`. -/
inductive SVarRefKind where
  | proc | name
deriving BEq, DecidableEq, Repr

mutual

/-- The surface terms: one constructor per `.cf` `Proc` production, named after its label. -/
inductive Surf where
  | ground : SGround → Surf
  | collect : SCollect → Surf
  | var : SVar → Surf
  | varWild : Surf
  | varRef : SVarRefKind → SVar → Surf                    -- `PVarRef`
  | nil : Surf                                            -- `PNil`
  | simpleType : STy → Surf                               -- `PSimpleType`
  | neg : Surf → Surf                                     -- `PNegation` (`~`)
  | conj : Surf → Surf → Surf                             -- `PConjunction` (`/\`)
  | disj : Surf → Surf → Surf                             -- `PDisjunction` (`\/`)
  | eval : SName → Surf                                   -- `PEval` (`*Name`)
  | method : Surf → SVar → List Surf → Surf               -- `PMethod`
  | not : Surf → Surf                                     -- `PNot`
  | negNum : Surf → Surf                                  -- `PNeg` (`-`)
  | mult : Surf → Surf → Surf                             -- `PMult`
  | div : Surf → Surf → Surf                              -- `PDiv`
  | mod : Surf → Surf → Surf                              -- `PMod`
  | pctPct : Surf → Surf → Surf                           -- `PPercentPercent`
  | add : Surf → Surf → Surf                              -- `PAdd`
  | sub : Surf → Surf → Surf                              -- `PMinus`
  | plusPlus : Surf → Surf → Surf                         -- `PPlusPlus`
  | minusMinus : Surf → Surf → Surf                       -- `PMinusMinus`
  | lt : Surf → Surf → Surf                               -- `PLt`
  | lte : Surf → Surf → Surf                              -- `PLte`
  | gt : Surf → Surf → Surf                               -- `PGt`
  | gte : Surf → Surf → Surf                              -- `PGte`
  | matches : Surf → Surf → Surf                          -- `PMatches`
  | eq : Surf → Surf → Surf                               -- `PEq`
  | neq : Surf → Surf → Surf                              -- `PNeq`
  | and : Surf → Surf → Surf                              -- `PAnd`
  | shortAnd : Surf → Surf → Surf                         -- `PShortAnd`
  | or : Surf → Surf → Surf                               -- `POr`
  | shortOr : Surf → Surf → Surf                          -- `PShortOr`
  | send : SName → Bool → List Surf → Surf                -- `PSend` (true = `!!`)
  | contr : SName → List SName → Option SVar → Surf → Surf -- `PContr` (params, `...rest`, body)
  | input : List SReceipt → Surf → Surf                   -- `PInput`
  | choice : List SBranch → Surf                          -- `PChoice` (`select`)
  | match : Surf → List SCase → Surf                      -- `PMatch`
  | bundle : SBundle → Surf → Surf                        -- `PBundle`
  | letIn : SDecl → SDecls → Surf → Surf                  -- `PLet`
  | ifThen : Surf → Surf → Surf                           -- `PIf`
  | ifElse : Surf → Surf → Surf → Surf                    -- `PIfElse`
  | newIn : List SNameDecl → Surf → Surf                  -- `PNew`
  | sendSynch : SName → List Surf → SSynchCont → Surf     -- `PSendSynch`
  | par : Surf → Surf → Surf                              -- `PPar`

/-- `Collection`: the four forms, each with its optional remainder. -/
inductive SCollect where
  | list  : List Surf → Option SVar → SCollect            -- `CollectList`
  | tuple : Surf → List Surf → SCollect                   -- `CollectTuple` (`TupleSingle` and `TupleMultiple`)
  | set   : List Surf → Option SVar → SCollect            -- `CollectSet`
  | map   : List SKeyValuePair → Option SVar → SCollect   -- `CollectMap`

/-- `Name`: a wildcard, a variable, or a quoted process (`@`). -/
inductive SName where
  | wild : SName
  | var : SVar → SName
  | quote : Surf → SName

/-- `NameSource`: `Name`, `Name ?!`, or `Name !?( data )`. -/
inductive SNameSource where
  | simple : SName → SNameSource
  | receiveSend : SName → SNameSource
  | sendReceive : SName → List Surf → SNameSource

/-- One bind of a receipt: names, their remainder, and the source. -/
inductive SBind where
  | mk : List SName → Option SVar → SNameSource → SBind

/-- `KeyValuePairImpl`: a map entry. A named record rather than a `Surf × Surf` pair, because the
grammar has the production and because Lean rejects `List (α × β)` inside a mutual block of
inductives (nested-inductive parameters may not contain local variables). -/
inductive SKeyValuePair where
  | mk : Surf → Surf → SKeyValuePair

/-- `CaseImpl`: a `match` case. -/
inductive SCase where
  | mk : Surf → Surf → SCase

/-- `BranchImpl`: a `select` branch — a linear receipt and the body it leads to. -/
inductive SBranch where
  | mk : List SBind → Surf → SBranch

/-- A receipt (`ReceiptLinear` / `ReceiptRepeated` / `ReceiptPeek`), each a list of binds separated by
`&`. Which of the three it is decides the receive's persistence, and the peek form has no flat
representation — see `surfaceBoundaries`. -/
inductive SReceipt where
  | linear   : List SBind → SReceipt
  | repeated : List SBind → SReceipt
  | peek     : List SBind → SReceipt

/-- A `Decl` (`DeclImpl`): names, remainder, source, and the values. -/
inductive SDecl where
  | mk : List SName → Option SVar → List Surf → SDecl

/-- `Decls`: the leading `;`/`&` separator decides which list follows, and neither is `EmptyDeclImpl`. -/
inductive SDecls where
  | empty : SDecls
  | linear : List SDecl → SDecls
  | conc : List SDecl → SDecls

/-- `SynchSendCont`: `"."` is empty, `";" Proc1` is not. -/
inductive SSynchCont where
  | empty : SSynchCont
  | nonEmpty : Surf → SSynchCont

/-- `NameDecl`: `Var` or `Var ( UriLiteral )`. -/
inductive SNameDecl where
  | simple : SVar → SNameDecl
  | urn : SVar → String → SNameDecl

end

/-- The production labels of `rholang_mercury.cf`, in file order — the `Proc` level first, then the
auxiliary sorts. This list is the table's *whole* claim, so `witnesses_cover` checks it both ways:
every label here has a witness below, and no witness names a label that is not here. -/
def grammarProductions : List String :=
  [ -- Proc16 … Proc1, Proc (the 45 of the `.cf`)
    "PGround", "PCollect", "PVar", "PVarRef", "PNil", "PSimpleType", "PNegation", "PConjunction",
    "PDisjunction", "PEval", "PMethod", "PExprs", "PNot", "PNeg", "PMult", "PDiv", "PMod",
    "PPercentPercent", "PAdd", "PMinus", "PPlusPlus", "PMinusMinus", "PLt", "PLte", "PGt", "PGte",
    "PMatches", "PEq", "PNeq", "PAnd", "PShortAnd", "POr", "PShortOr", "PSend", "PContr", "PInput",
    "PChoice", "PMatch", "PBundle", "PLet", "PIf", "PIfElse", "PNew", "PSendSynch", "PPar",
    -- the auxiliary sorts
    "DeclImpl", "LinearDeclImpl", "ConcDeclImpl", "EmptyDeclImpl", "LinearDeclsImpl",
    "ConcDeclsImpl", "EmptyCont", "NonEmptyCont", "ProcVarWildcard", "ProcVarVar", "NameWildcard",
    "NameVar", "NameQuote", "BundleWrite", "BundleRead", "BundleEquiv", "BundleReadWrite",
    "ReceiptLinear", "ReceiptRepeated", "ReceiptPeek", "LinearSimple", "LinearBindImpl",
    "SimpleSource", "ReceiveSendSource", "SendReceiveSource", "RepeatedSimple", "RepeatedBindImpl",
    "PeekSimple", "PeekBindImpl", "SendSingle", "SendMultiple", "BranchImpl", "CaseImpl",
    "NameDeclSimpl", "NameDeclUrn", "BoolTrue", "BoolFalse", "GroundBool", "GroundBigInt",
    "GroundInt", "GroundString", "GroundUri", "CollectList", "CollectTuple", "CollectSet",
    "CollectMap", "KeyValuePairImpl", "TupleSingle", "TupleMultiple", "ProcRemainderVar",
    "ProcRemainderEmpty", "NameRemainderVar", "NameRemainderEmpty", "VarRefKindProc",
    "VarRefKindName", "SimpleTypeBool", "SimpleTypeInt", "SimpleTypeBigInt", "SimpleTypeString",
    "SimpleTypeUri", "SimpleTypeByteArray" ]

/-- One witness: the term, and the production label(s) it demonstrates. A production whose shape is
the same as another's gets both labels on one witness (the two `if` forms share nothing, but
`TupleSingle` and `TupleMultiple` are one constructor and two productions). -/
structure ProductionWitness where
  productions : List String
  term : Surf

/-- A witness for **every** production the grammar has. The terms are written in the *printer's*
spelling (`Rchain/Print.lean`), so a corpus row generated from one is the printer's own rendering —
which makes the row check the printer against the parser against the port, all three. -/
def productionWitnesses : List ProductionWitness :=
  [ ⟨["PGround", "GroundInt"], .ground (.int "42")⟩
  , ⟨["PGround", "GroundBool", "BoolTrue"], .ground (.bool true)⟩
  , ⟨["PGround", "GroundBool", "BoolFalse"], .ground (.bool false)⟩
  , ⟨["PGround", "GroundBigInt"], .ground (.bigint "1234")⟩
  , ⟨["PGround", "GroundString"], .ground (.str "\"s\"")⟩
  , ⟨["PGround", "GroundUri"], .ground (.uri "`rho:id:x`")⟩
  , ⟨["PCollect", "CollectList"], .collect (.list [.ground (.int "1")] none)⟩
  , ⟨["PCollect", "CollectSet"], .collect (.set [.ground (.int "1")] none)⟩
  , ⟨["PCollect", "CollectMap", "KeyValuePairImpl"],
      .collect (.map [⟨.ground (.int "1"), .ground (.int "2")⟩] none)⟩
  , ⟨["PCollect", "CollectTuple", "TupleSingle"], .collect (.tuple (.ground (.int "1")) [])⟩
  , ⟨["PCollect", "CollectTuple", "TupleMultiple"],
      .collect (.tuple (.ground (.int "1")) [.ground (.int "2")])⟩
  , ⟨["PVar", "ProcVarVar"], .var "x"⟩
  , ⟨["PVar", "ProcVarWildcard"], .varWild⟩
  , ⟨["PVarRef", "VarRefKindProc"], .varRef .proc "x"⟩
  , ⟨["PVarRef", "VarRefKindName"], .varRef .name "x"⟩
  , ⟨["PNil"], .nil⟩
  , ⟨["PSimpleType", "SimpleTypeBool"], .simpleType .bool⟩
  , ⟨["PSimpleType", "SimpleTypeInt"], .simpleType .int⟩
  , ⟨["PSimpleType", "SimpleTypeBigInt"], .simpleType .bigInt⟩
  , ⟨["PSimpleType", "SimpleTypeString"], .simpleType .str⟩
  , ⟨["PSimpleType", "SimpleTypeUri"], .simpleType .uri⟩
  , ⟨["PSimpleType", "SimpleTypeByteArray"], .simpleType .byteArray⟩
  , ⟨["PNegation"], .neg (.var "x")⟩
  , ⟨["PConjunction"], .conj (.var "x") (.var "y")⟩
  , ⟨["PDisjunction"], .disj (.var "x") (.var "y")⟩
  , ⟨["PEval", "NameVar"], .eval (.var "c")⟩
  , ⟨["PEval", "NameQuote"], .eval (.quote (.ground (.int "1")))⟩
  , ⟨["PEval", "NameWildcard"], .eval .wild⟩
  , ⟨["PMethod"], .method (.var "x") "get" [.ground (.int "1")]⟩
  -- `PExprs` is the group, and the AST collapses it: the witness is the interior, and the printer
  -- must re-insert the parentheses from the shape (that is law 33's job, not this table's).
  , ⟨["PExprs"], .add (.ground (.int "1")) (.ground (.int "2"))⟩
  , ⟨["PNot"], .not (.var "x")⟩
  , ⟨["PNeg"], .negNum (.ground (.int "1"))⟩
  , ⟨["PMult"], .mult (.ground (.int "2")) (.ground (.int "3"))⟩
  , ⟨["PDiv"], .div (.ground (.int "6")) (.ground (.int "2"))⟩
  , ⟨["PMod"], .mod (.ground (.int "5")) (.ground (.int "2"))⟩
  , ⟨["PPercentPercent"], .pctPct (.ground (.int "5")) (.ground (.int "2"))⟩
  , ⟨["PAdd"], .add (.ground (.int "1")) (.ground (.int "2"))⟩
  , ⟨["PMinus"], .sub (.ground (.int "1")) (.ground (.int "2"))⟩
  , ⟨["PPlusPlus"], .plusPlus (.collect (.list [] none)) (.collect (.list [] none))⟩
  , ⟨["PMinusMinus"], .minusMinus (.collect (.set [] none)) (.collect (.set [] none))⟩
  , ⟨["PLt"], .lt (.ground (.int "1")) (.ground (.int "2"))⟩
  , ⟨["PLte"], .lte (.ground (.int "1")) (.ground (.int "2"))⟩
  , ⟨["PGt"], .gt (.ground (.int "1")) (.ground (.int "2"))⟩
  , ⟨["PGte"], .gte (.ground (.int "1")) (.ground (.int "2"))⟩
  , ⟨["PMatches"], .matches (.var "x") (.simpleType .bool)⟩
  , ⟨["PEq"], .eq (.ground (.int "1")) (.ground (.int "2"))⟩
  , ⟨["PNeq"], .neq (.ground (.int "1")) (.ground (.int "2"))⟩
  , ⟨["PAnd"], .and (.var "x") (.var "y")⟩
  , ⟨["PShortAnd"], .shortAnd (.var "x") (.var "y")⟩
  , ⟨["POr"], .or (.var "x") (.var "y")⟩
  , ⟨["PShortOr"], .shortOr (.var "x") (.var "y")⟩
  , ⟨["PSend", "SendSingle", "SendMultiple"],
      .send (.var "c") true [.ground (.int "1")]⟩
  , ⟨["PSend", "SendSingle"], .send (.var "c") false [.ground (.int "1")]⟩
  , ⟨["PContr", "NameRemainderEmpty"],
      .contr (.var "f") [.var "x"] none (.nil)⟩
  , ⟨["PContr", "NameRemainderVar"],
      .contr (.var "f") [.var "x"] (some "rest") (.nil)⟩
  , ⟨["PInput", "ReceiptLinear", "LinearSimple", "LinearBindImpl", "SimpleSource"],
      .input [.linear [⟨[.var "x"], none, .simple (.var "c")⟩]] .nil⟩
  , ⟨["PInput", "ReceiptRepeated", "RepeatedSimple", "RepeatedBindImpl"],
      .input [.repeated [⟨[.var "x"], none, .simple (.var "c")⟩]] .nil⟩
  , ⟨["PInput", "ReceiptPeek", "PeekSimple", "PeekBindImpl"],
      .input [.peek [⟨[.var "x"], none, .simple (.var "c")⟩]] .nil⟩
  , ⟨["ReceiveSendSource"], .input [.linear [⟨[.var "x"], none, .receiveSend (.var "c")⟩]] .nil⟩
  , ⟨["SendReceiveSource"],
      .input [.linear [⟨[.var "x"], none, .sendReceive (.var "c") [.ground (.int "1")]⟩]] .nil⟩
  , ⟨["PChoice", "BranchImpl"], .choice [⟨[⟨[.var "x"], none, .simple (.var "c")⟩], .nil⟩]⟩
  , ⟨["PMatch", "CaseImpl"],
      .match (.var "x") [⟨.ground (.int "1"), .nil⟩]⟩
  , ⟨["PBundle", "BundleWrite"], .bundle .write .nil⟩
  , ⟨["PBundle", "BundleRead"], .bundle .read .nil⟩
  , ⟨["PBundle", "BundleEquiv"], .bundle .equiv .nil⟩
  , ⟨["PBundle", "BundleReadWrite"], .bundle .readWrite .nil⟩
  , ⟨["PLet", "DeclImpl", "EmptyDeclImpl"],
      .letIn (⟨[.var "x"], none, [.var "c"]⟩) .empty .nil⟩
  , ⟨["PLet", "LinearDeclImpl", "LinearDeclsImpl"],
      .letIn (⟨[.var "x"], none, [.var "c"]⟩) (.linear [⟨[.var "y"], none, [.var "c"]⟩]) .nil⟩
  , ⟨["PLet", "ConcDeclImpl", "ConcDeclsImpl"],
      .letIn (⟨[.var "x"], none, [.var "c"]⟩) (.conc [⟨[.var "y"], none, [.var "c"]⟩]) .nil⟩
  , ⟨["PIf"], .ifThen (.ground (.bool true)) .nil⟩
  , ⟨["PIfElse"], .ifElse (.ground (.bool true)) .nil .nil⟩
  , ⟨["PNew", "NameDeclSimpl"], .newIn [.simple "x"] .nil⟩
  , ⟨["PNew", "NameDeclUrn"], .newIn [.urn "x" "`rho:id:y`"] .nil⟩
  , ⟨["PSendSynch", "EmptyCont"], .sendSynch (.var "c") [.ground (.int "1")] .empty⟩
  , ⟨["PSendSynch", "NonEmptyCont"],
      .sendSynch (.var "c") [.ground (.int "1")] (.nonEmpty .nil)⟩
  , ⟨["PPar"], .par (.var "x") (.var "y")⟩
  , ⟨["CollectList", "ProcRemainderVar"], .collect (.list [] (some "rest"))⟩
  , ⟨["CollectSet", "ProcRemainderVar"], .collect (.set [] (some "rest"))⟩
  , ⟨["CollectMap", "ProcRemainderVar"], .collect (.map [] (some "rest"))⟩
  , ⟨["CollectList", "ProcRemainderEmpty"], .collect (.list [] none)⟩
  , ⟨["ProcRemainderEmpty"], .collect (.list [.ground (.int "1")] none)⟩
  ]

/-- **Law 31's table is complete, `decide`d, in both directions.** Every label of the `.cf` is
witnessed by some row, and no row names a label the grammar does not have — a table with a witness
for a production nobody wrote, or a production with no witness, is a table that has stopped
describing the grammar. -/
theorem witnesses_cover :
    (grammarProductions.all (fun g => productionWitnesses.any (fun w => w.productions.contains g))
      && productionWitnesses.all (fun w => w.productions.all grammarProductions.contains)) = true := by
  decide

/-- How many witnesses the table carries. -/
def productionWitnessCount : Nat := 81

/-- The table carries exactly `productionWitnessCount` rows. -/
theorem productionWitnesses_length :
    productionWitnesses.length = productionWitnessCount := by decide

/-! ## The desugaring (laws 34 and 36)

`normalize` is the function the *value-position* rule is a property of: an operand, an `if`'s
condition, a `match`'s target, a method's target, a send's datum, a collection's element, a pattern
and a name are each normalized against an **empty** par, and only a statement continuation inherits
the accumulated one. Stating the rule over this function rather than over the Rust is what makes it
checkable: the arms below thread a binder stack (`Γ`) and nothing else, so "no value position carries
an accumulator" is a property of the definition, not a convention.

The output is a flat `Par` — the model's `Par`, whose fields are sorted by law 1. `normalize` returns
`none` for the constructs outside its domain, each named in `surfaceBoundaries`. -/

/-- A `Par` holding one expression. (Named `parOf` rather than `one` because `Rchain/Json.lean`
defines an `one` with the same type, and Lean's library rejects a duplicate name outright — the
clash surfaced the moment both modules were in the library.) -/
def parOf (e : Expr) : Par := Par.mk [] [] [] [e] [] [] [] []

/-- A string literal's contents: the grammar's `StringLiteral` includes its delimiters, and the flat
`Ground.str` carries the characters, so the two ends come off. -/
def unquote (raw : String) : String :=
  match raw.toList with
  | [] => ""
  | _ :: rest =>
    match rest.reverse with
    | [] => ""
    | _ :: inner => inner.reverse.asString

/-- The number of names a `new` block binds. -/
def namesOfDecls : List SNameDecl → Option Nat
  | [] => some 0
  | _ :: rest => (namesOfDecls rest).map (fun n => n + 1)

/-- The names a `new` block binds, so the body's references to them are bound. -/
def declNames : List SNameDecl → List SVar
  | [] => []
  | .simple x :: rest => x :: declNames rest
  | .urn x _ :: rest => x :: declNames rest

/-- Code points, so a source string becomes the model's `Ground.str`. -/
def charsOf (s : String) : List Nat := s.toList.map Char.toNat

/-- The head and tail of a `String`-encoded integer, as an `Int`. `none` for the empty string. -/
def intOfDigits (d : String) : Option Int :=
  match d.toList with
  | [] => none
  | _ => some (d.toList.foldl (fun acc c => acc * 10 + (c.toNat - '0'.toNat)) 0)

/-- A source name under a binder stack: `Γ` is the stack of bound names, head innermost, so a name
found at distance `i` from the head is `Var.bound i` — the model's level convention. A name not in
`Γ` is free. -/
def nameVar (Γ : List SVar) (x : SVar) : Var :=
  match Γ.findIdx? (fun y => y == x) with
  | some i => .bound i
  | none => .free 0

/-- The bundling flags of each spelling: `bundle+` is write-only, `bundle-` read-only, `bundle0`
neither, `bundle` both. (`bundle0` is lexed as `bundle` + `0` and read by `parse_bundle`.) -/
def bundleFlags : SBundle → Bool × Bool
  | .write => (true, false)
  | .read => (false, true)
  | .equiv => (false, false)
  | .readWrite => (true, true)

-- The recursion below is structural on the *term*, so every member takes its term **first** and the
-- binder stack `Γ` last: Lean's mutual structural recursion looks for a common parameter to
-- eliminate on, and with `Γ` first it tried `Γ` (which is not an inductive) and gave up
-- ("Cannot use parameters Γ of …: failed to eliminate recursive application"). No member returns a
-- closure either — `receiptPar` takes the body as an argument instead of yielding a `Par → Receive`,
-- the same reason `Rchain/Json.lean` spells out `renderItems` rather than mapping a lambda.
mutual

/-- **The desugaring**, mirroring the port's `normalize_proc` (`normalizer.rs:152`): one arm per
constructor, threading the binder stack **and the accumulated `par`** — the accumulator, which is
`ProcVisitInputs.par` there and is what makes law 34's rule a claim rather than a construction:

- a **value** position (a condition, target, datum, element, pattern, name) is normalized against the
  *empty* par — `normalizeAt … nilPar Γ` — which is the rule, and the defect C21 shipped was seeding
  the condition in a desugared `if` with the accumulator instead;
- a **statement continuation** is the only thing that inherits: the sequencing arm (`.par`) hands the
  left side's result to the right, the port's `PPar(l, r)` normalizing `r` with `par: result.par`
  (`normalizer.rs:188-199`);
- every construct's own result is merged into the accumulator — the port's `prepend_expr` /
  `prepend_send` shape (`:157`, `:930`), with `parMerge` (a field-wise append, so the corpus's
  `decide`d verdicts stay `decide`-able). -/
def normalizeAt : Surf → Par → List SVar → Option Par
  | .ground g, acc, _ => (groundPar g).map (fun p => parMerge acc p)
  | .collect c, acc, Γ => (collectPar c Γ).map (fun p => parMerge acc p)
  | .var x, acc, Γ => some (parMerge acc (parOf (.evar (nameVar Γ x))))
  | .varWild, acc, _ => some (parMerge acc (parOf (.evar .wildcard)))
  | .nil, acc, _ => some acc
  | .neg p, acc, Γ => (normalizeAt p nilPar Γ).map (fun q => parMerge acc (parOf (.eneg q)))
  | .not p, acc, Γ => (normalizeAt p nilPar Γ).map (fun q => parMerge acc (parOf (.enot q)))
  | .negNum p, acc, Γ => (normalizeAt p nilPar Γ).map (fun q => parMerge acc (parOf (.eneg q)))
  | .mult a b, acc, Γ =>
    match normalizeAt a nilPar Γ, normalizeAt b nilPar Γ with
    | some p, some q => some (parMerge acc (parOf (.emult p q)))
    | _, _ => none
  | .div a b, acc, Γ =>
    match normalizeAt a nilPar Γ, normalizeAt b nilPar Γ with
    | some p, some q => some (parMerge acc (parOf (.ediv p q)))
    | _, _ => none
  | .mod a b, acc, Γ =>
    match normalizeAt a nilPar Γ, normalizeAt b nilPar Γ with
    | some p, some q => some (parMerge acc (parOf (.emod p q)))
    | _, _ => none
  | .add a b, acc, Γ =>
    match normalizeAt a nilPar Γ, normalizeAt b nilPar Γ with
    | some p, some q => some (parMerge acc (parOf (.eplus p q)))
    | _, _ => none
  | .sub a b, acc, Γ =>
    match normalizeAt a nilPar Γ, normalizeAt b nilPar Γ with
    | some p, some q => some (parMerge acc (parOf (.eminus p q)))
    | _, _ => none
  | .lt a b, acc, Γ =>
    match normalizeAt a nilPar Γ, normalizeAt b nilPar Γ with
    | some p, some q => some (parMerge acc (parOf (.elt p q)))
    | _, _ => none
  | .lte a b, acc, Γ =>
    match normalizeAt a nilPar Γ, normalizeAt b nilPar Γ with
    | some p, some q => some (parMerge acc (parOf (.ele p q)))
    | _, _ => none
  | .gt a b, acc, Γ =>
    match normalizeAt a nilPar Γ, normalizeAt b nilPar Γ with
    | some p, some q => some (parMerge acc (parOf (.egt p q)))
    | _, _ => none
  | .gte a b, acc, Γ =>
    match normalizeAt a nilPar Γ, normalizeAt b nilPar Γ with
    | some p, some q => some (parMerge acc (parOf (.ege p q)))
    | _, _ => none
  | .eq a b, acc, Γ =>
    match normalizeAt a nilPar Γ, normalizeAt b nilPar Γ with
    | some p, some q => some (parMerge acc (parOf (.eeq p q)))
    | _, _ => none
  | .neq a b, acc, Γ =>
    match normalizeAt a nilPar Γ, normalizeAt b nilPar Γ with
    | some p, some q => some (parMerge acc (parOf (.eneq p q)))
    | _, _ => none
  | .and a b, acc, Γ =>
    match normalizeAt a nilPar Γ, normalizeAt b nilPar Γ with
    | some p, some q => some (parMerge acc (parOf (.eand p q)))
    | _, _ => none
  | .shortAnd a b, acc, Γ =>
    match normalizeAt a nilPar Γ, normalizeAt b nilPar Γ with
    | some p, some q => some (parMerge acc (parOf (.eshortand p q)))
    | _, _ => none
  | .or a b, acc, Γ =>
    match normalizeAt a nilPar Γ, normalizeAt b nilPar Γ with
    | some p, some q => some (parMerge acc (parOf (.eor p q)))
    | _, _ => none
  | .shortOr a b, acc, Γ =>
    match normalizeAt a nilPar Γ, normalizeAt b nilPar Γ with
    | some p, some q => some (parMerge acc (parOf (.eshortor p q)))
    | _, _ => none
  | .matches a b, acc, Γ =>
    match normalizeAt a nilPar Γ, normalizeAt b nilPar Γ with
    | some p, some q => some (parMerge acc (parOf (.ematches p q)))
    | _, _ => none
  | .par a b, acc, Γ =>
    match normalizeAt a acc Γ with
    | some p => normalizeAt b p Γ
    | none => none
  | .send n persistent data, acc, Γ =>
    match namePar n Γ, procsPar data Γ with
    | some c, some d => some (parMerge acc (Par.mk [Send.mk c d persistent] [] [] [] [] [] [] []))
    | _, _ => none
  | .contr n params rest body, acc, Γ =>
    match namePar n Γ, namesPar params rest Γ, normalizeAt body nilPar Γ with
    | some c, some pats, some b =>
      some (parMerge acc (Par.mk [] [Receive.mk [ReceiveBind.mk pats c pats.length] b true 1] [] [] [] [] [] []))
    | _, _, _ => none
  | .input receipts body, acc, Γ =>
    match normalizeAt body nilPar Γ with
    | some b =>
      match receiptsPar receipts b Γ with
      | some rs => some (parMerge acc (Par.mk [] rs [] [] [] [] [] []))
      | none => none
    | none => none
  | .match target cases, acc, Γ =>
    match normalizeAt target nilPar Γ, casesPar cases Γ with
    | some t, some cs => some (parMerge acc (Par.mk [] [] [] [] [Match.mk t cs] [] [] []))
    | _, _ => none
  | .bundle b body, acc, Γ =>
    match normalizeAt body nilPar Γ with
    | some q => some (parMerge acc (Par.mk [] [] [] [] [] [] [Bundle.mk q (bundleFlags b).1 (bundleFlags b).2] []))
    | none => none
  | .ifThen c t, acc, Γ =>
    match normalizeAt c nilPar Γ, normalizeAt t nilPar Γ with
    | some cond, some thenB =>
      some (parMerge acc (Par.mk [] [] [] [] [Match.mk cond [MatchCase.mk (parOf (.ground (.bool true))) thenB 0]] [] [] []))
    | _, _ => none
  | .ifElse c t e, acc, Γ =>
    match normalizeAt c nilPar Γ, normalizeAt t nilPar Γ, normalizeAt e nilPar Γ with
    | some cond, some thenB, some elseB =>
      some (parMerge acc (Par.mk [] [] [] [] [Match.mk cond
        [MatchCase.mk (parOf (.ground (.bool true))) thenB 0,
         MatchCase.mk (parOf (.ground (.bool false))) elseB 0]] [] [] []))
    | _, _, _ => none
  | .newIn decls body, acc, Γ =>
    match namesOfDecls decls, normalizeAt body nilPar (declNames decls ++ Γ) with
    | some d, some b => some (parMerge acc (Par.mk [] [] [New.mk d b] [] [] [] [] []))
    | _, _ => none
  -- Outside the modelled fragment, each with a row in `surfaceBoundaries`: a simple type has no flat
  -- leaf, a method call is a native dispatch, `select` and `let` need the normalizer's own
  -- machinery, `PSendSynch` and `PVarRef` have no flat constructor, and the pattern connectives in
  -- *process* position are refused by the port itself.
  | .simpleType _, _, _ => none
  | .method _ _ _, _, _ => none
  | .eval _, _, _ => none
  | .choice _, _, _ => none
  | .letIn _ _ _, _, _ => none
  | .sendSynch _ _ _, _, _ => none
  | .varRef _ _, _, _ => none
  | .conj _ _, _, _ => none
  | .disj _ _, _, _ => none
  | .plusPlus _ _, _, _ => none
  | .minusMinus _ _, _, _ => none
  | .pctPct _ _, _, _ => none

/-- A ground as a flat `Par`. **`bigint` is outside the domain**: the model's `Ground` has no bigint
leaf (`Rchain/Syntax.lean`) while the protobuf's `Expr` has `GBigInt` — the same boundary
`Rchain/Json.lean` records for the unforgeable. -/
def groundPar : SGround → Option Par
  | .bool b => some (parOf (.ground (.bool b)))
  | .int d => (intOfDigits d).map (fun n => parOf (.ground (.int n)))
  | .str raw => some (parOf (.ground (.str (charsOf (unquote raw)))))
  | .uri raw => some (parOf (.ground (.uri (charsOf (unquote raw)))))
  | .bigint _ => none

/-- A list of processes, in order. -/
def procsPar : List Surf → List SVar → Option (List Par)
  | [], _ => some []
  | p :: ps, Γ =>
    match normalizeAt p nilPar Γ, procsPar ps Γ with
    | some q, some qs => some (q :: qs)
    | _, _ => none

/-- A receiver's patterns, in order. A bind's `...rest` is **outside the domain**: the flat
`ReceiveBind` carries a pattern list and no remainder (`surfaceBoundaries`). -/
def namesPar : List SName → Option SVar → List SVar → Option (List Par)
  | _, some _, _ => none
  | [], none, _ => some []
  | n :: ns, none, Γ =>
    match namePar n Γ, namesPar ns none Γ with
    | some p, some ps => some (p :: ps)
    | _, _ => none

/-- A collection as a flat `Par`. -/
def collectPar : SCollect → List SVar → Option Par
  | .list ps rem, Γ =>
    (procsPar ps Γ).map (fun qs => parOf (.elist qs (rem.map (fun r => nameVar Γ r))))
  | .set ps rem, Γ =>
    (procsPar ps Γ).map (fun qs => parOf (.eset qs (rem.map (fun r => nameVar Γ r))))
  | .tuple first rest, Γ =>
    match normalizeAt first nilPar Γ, procsPar rest Γ with
    | some f, some rs => some (parOf (.etuple (f :: rs)))
    | _, _ => none
  | .map kvs rem, Γ =>
    match kvsPar kvs Γ with
    | some ps => some (parOf (.emap ps (rem.map (fun r => nameVar Γ r))))
    | none => none

/-- A map's pairs, in order. -/
def kvsPar : List SKeyValuePair → List SVar → Option (List (Par × Par))
  | [], _ => some []
  | ⟨k, v⟩ :: rest, Γ =>
    match normalizeAt k nilPar Γ, normalizeAt v nilPar Γ, kvsPar rest Γ with
    | some kp, some vp, some rp => some ((kp, vp) :: rp)
    | _, _, _ => none

/-- A `match`'s cases. -/
def casesPar : List SCase → List SVar → Option (List MatchCase)
  | [], _ => some []
  | ⟨pat, body⟩ :: rest, Γ =>
    match normalizeAt pat nilPar Γ, normalizeAt body nilPar Γ, casesPar rest Γ with
    | some p, some b, some r => some (MatchCase.mk p b 0 :: r)
    | _, _, _ => none

/-- Each receipt as a receive for `body`. Takes the body rather than returning a builder: a closure
here is what the termination checker refuses. -/
def receiptsPar : List SReceipt → Par → List SVar → Option (List Receive)
  | [], _, _ => some []
  | r :: rest, body, Γ =>
    match receiptPar r body Γ, receiptsPar rest body Γ with
    | some f, some fs => some (f :: fs)
    | _, _ => none

/-- One receipt as a receive. `linear` is `<-` (not persistent), `repeated` is `<=` (persistent),
`peek` is `<<-` and is out of the domain. -/
def receiptPar : SReceipt → Par → List SVar → Option Receive
  | .linear binds, body, Γ => (bindsPar binds Γ).map (fun bs => Receive.mk bs body false bs.length)
  | .repeated binds, body, Γ => (bindsPar binds Γ).map (fun bs => Receive.mk bs body true bs.length)
  | .peek _, _, _ => none

/-- A receipt's binds. Only a `SimpleSource` name is modelled: `Name ?!` and `Name !?( data )` are the
send-receive forms whose flat shape the port builds in the normalizer, not in the parser. -/
def bindsPar : List SBind → List SVar → Option (List ReceiveBind)
  | [], _ => some []
  | ⟨ns, rest, src⟩ :: tl, Γ =>
    match src, namesPar ns rest Γ, bindsPar tl Γ with
    | .simple n, some pats, some bs =>
      (namePar n Γ).map (fun c => ReceiveBind.mk pats c pats.length :: bs)
    | _, _, _ => none

/-- A name in a *value* position: a quoted process (`@p`) desugars, a bare name is a channel. -/
def namePar : SName → List SVar → Option Par
  | .wild, _ => some (parOf (.evar .wildcard))
  | .var x, Γ => some (parOf (.evar (nameVar Γ x)))
  | .quote p, Γ => normalizeAt p nilPar Γ

end

/-- The constructs `normalize` does not model, each with the reason — so "outside the domain" is a
row to read rather than a `none` to wonder about. -/
structure SurfaceBoundary where
  id : String
  reason : String

/-- The boundary table. Each row is a named gap, and law 31's deviation list is where the *parser*
side of the same gaps is recorded. -/
def surfaceBoundaries : List SurfaceBoundary :=
  [ ⟨"simple-type", "`Proc16 ::= SimpleType` has no flat leaf: the model's `Expr` has no type leaf"⟩
  , ⟨"method-call", "`PMethod` dispatches to a native method at reduce time; the flat `Par` has no node for it"⟩
  , ⟨"eval", "`PEval` (`*Name`) needs the normalizer's environment, which this function does not model"⟩
  , ⟨"select", "`PChoice`'s branches carry their own receipts; the flat `Receive` has one body each"⟩
  , ⟨"let", "`PLet` desugars through the normalizer's declaration machinery, not in one step"⟩
  , ⟨"send-synch", "`PSendSynch` has no flat constructor (the port's `proc_ast` has one, the model does not)"⟩
  , ⟨"var-ref", "`PVarRef` has no flat constructor"⟩
  , ⟨"connective", "`/\\` and `\\/` in *process* position are refused by the port itself (law 32's table)"⟩
  , ⟨"plus-plus", "`++` is a collection operation the model's `Expr` does not carry"⟩
  , ⟨"minus-minus", "`--` is a set operation the model's `Expr` does not carry"⟩
  , ⟨"pct-pct", "`%%` is the map/dictionary operation; the model's `Expr` does not carry it"⟩
  , ⟨"peek-receipt", "`<<-` has no flat representation: the model's `Receive` has a persistent flag and no peek one"⟩
  , ⟨"bigint-ground", "the model's `Ground` has no bigint leaf (AUDIT C28's boundary, as in `Rchain/Json.lean`)"⟩
  , ⟨"bind-remainder", "`[Name] NameRemainder` in a bind: the flat `ReceiveBind` carries patterns and no remainder"⟩
  , ⟨"name-source", "`Name ?!` / `Name !?( data )` binds: the flat shape is the normalizer's, not the parser's"⟩
  ]

/-- The boundary table's rows are non-empty and distinct — a table that can say nothing is not a
boundary list. -/
theorem surfaceBoundaries_nonempty :
    (surfaceBoundaries.all (fun b => !b.id.isEmpty && !b.reason.isEmpty)
      && (surfaceBoundaries.map SurfaceBoundary.id).eraseDups.length
          == surfaceBoundaries.length) = true := by
  decide

/-- How many boundaries the model names. -/
def surfaceBoundaryCount : Nat := 15

/-- The boundary table carries exactly `surfaceBoundaryCount` rows. -/
theorem surfaceBoundaries_length :
    surfaceBoundaries.length = surfaceBoundaryCount := by decide

end Rchain
