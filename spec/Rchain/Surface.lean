import Rchain.Par
import Rchain.Ty

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
  | .pctPct a b, acc, Γ =>
    match normalizeAt a nilPar Γ, normalizeAt b nilPar Γ with
    | some p, some q => some (parMerge acc (parOf (.epercentPercent p q)))
    | _, _ => none
  | .plusPlus a b, acc, Γ =>
    match normalizeAt a nilPar Γ, normalizeAt b nilPar Γ with
    | some p, some q => some (parMerge acc (parOf (.eplusPlus p q)))
    | _, _ => none
  | .minusMinus a b, acc, Γ =>
    match normalizeAt a nilPar Γ, normalizeAt b nilPar Γ with
    | some p, some q => some (parMerge acc (parOf (.eminusMinus p q)))
    | _, _ => none
  -- `PMethod` is `Proc11 "." Var "(" [Proc] ")"` and its name is a *code-point list* in the model
  -- (`charsOf`, the convention `Ground.str` uses) — mirroring `normalizer.rs:973-979`, which builds
  -- `Expr::EMethod` with the target normalised and each argument normalised in place.
  | .method t v args, acc, Γ =>
    match normalizeAt t nilPar Γ, procsPar args Γ with
    | some p, some qs => some (parMerge acc (parOf (.emethod (charsOf v) p qs)))
    | _, _ => none
  -- Outside the modelled fragment, each with a row in `surfaceBoundaries`: a simple type has no flat
  -- leaf, `select` and `let` need the normalizer's own machinery, `PSendSynch` and `PVarRef` have no
  -- flat constructor, and the pattern connectives in *process* position are refused by the port.
  | .simpleType _, _, _ => none
  | .eval _, _, _ => none
  | .choice _, _, _ => none
  | .letIn _ _ _, _, _ => none
  | .sendSynch _ _ _, _, _ => none
  | .varRef _ _, _, _ => none
  | .conj _ _, _, _ => none
  | .disj _ _, _, _ => none

/-- A ground as a flat `Par`. `bigint` is an **`Expr`** leaf here and not a `Ground` one — the
protobuf's `g_big_int` is an `Expr` variant (`models/proto/RhoTypes.proto:174`) and the port's
normalizer *converts* the parse-level ground into it (`normalizer.rs:74-77`,
`Ground::GroundBigInt -> Expr::GBigInt`), which is exactly this arm. `intOfDigits` is unsigned, and
that is faithful: the node's `BigInt(…)` production reads a signed `i64` token, so a negative literal
arrives as `PNeg` over a positive one and never reaches here. -/
def groundPar : SGround → Option Par
  | .bool b => some (parOf (.ground (.bool b)))
  | .int d => (intOfDigits d).map (fun n => parOf (.ground (.int n)))
  | .str raw => some (parOf (.ground (.str (charsOf (unquote raw)))))
  | .uri raw => some (parOf (.ground (.uri (charsOf (unquote raw)))))
  | .bigint d => (intOfDigits d).map (fun n => parOf (.ebigint n))

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

/-- **One bind's result**, given its already-desugared patterns and tail: a `SimpleSource` becomes a
    `ReceiveBind` whose free count is the pattern count, every other source shape is outside the domain.

    It is a `def` of its own rather than the arm's inline `match`, and the reason is mechanical:
    **Lean cannot generate an equation lemma for `bindsPar` with that `match` inline** — "failed to
    generate equational theorem for `Rchain.bindsPar`", measured 2026-09-25 — because the matched value
    is a *pattern-bound field* of the bind and the match is nested inside another match on two
    `Option`s. Nothing can unfold such a function: `simp only [bindsPar]` fails, `rw [bindsPar]` fails,
    and `exact`-style unfolding has nothing to use. That is a definitional obstruction to the
    closedness induction law 36 owes (`ScopedIn Γ e → normalizeAt e acc Γ = some p → Closed p`), which
    has to *unfold the arms* to apply its induction hypotheses, so the shape is part of the proof's
    cost rather than a stylistic choice. Lifted out, `bindsPar`'s own equation is a plain match on its
    arguments and generates.

    Behaviour is unchanged, and in the direction that matters: the arm answers `none` exactly when the
    source is not `SimpleSource` or either sub-result is `none`, which is what both forms compute (the
    sub-computations are total, so evaluating them in either order is the same value). The
    source-side boundary is `surfaceBoundaries`' `name-source` row. -/
def bindResult (src : SNameSource) (pats : List Par) (bs : List ReceiveBind)
    (Γ : List SVar) : Option (List ReceiveBind) :=
  match src with
  | .simple n => (namePar n Γ).map (fun c => ReceiveBind.mk pats c pats.length :: bs)
  | _ => none

/-- A receipt's binds. Only a `SimpleSource` name is modelled: `Name ?!` and `Name !?( data )` are the
send-receive forms whose flat shape the port builds in the normalizer, not in the parser. -/
def bindsPar : List SBind → List SVar → Option (List ReceiveBind)
  | [], _ => some []
  | ⟨ns, rest, src⟩ :: tl, Γ =>
    match namesPar ns rest Γ, bindsPar tl Γ with
    | some pats, some bs => bindResult src pats bs Γ
    | _, _ => none

/-- A name in a *value* position: a quoted process (`@p`) desugars, a bare name is a channel. -/
def namePar : SName → List SVar → Option Par
  | .wild, _ => some (parOf (.evar .wildcard))
  | .var x, Γ => some (parOf (.evar (nameVar Γ x)))
  | .quote p, Γ => normalizeAt p nilPar Γ

end

/-! ## Law 36's subject: the source term is *in scope for its binder stack*

The desugaring above is closedness-*preserving* — a term whose every name occurrence is bound normalizes
to a term with no free variable — and until this block existed nothing said what "bound" means for a
*surface* term, so law 36's row had no subject ("nothing computes a well-scopedness or closedness
predicate over the desugared output"). `Scoped*` are that predicate: one per arity of the desugaring,
mirroring its binder discipline exactly, so that the induction over `normalizeAt` (whose statement is
owed, in law 36's row) has something to induct on.

Three decisions, each measured rather than chosen (2026-09-25):

1. **The hypothesis is `Γ`, and it has to be.** `nameVar` resolves an occurrence against the binder stack
   and answers `.free 0` when it is absent, and `Ty.Closed` counts a `free` variable as open — so
   `for (x <- c) { x!(1) }` normalizes to an **open** term (`closed = false`, `#eval`d) while the same
   term under `new c, x in { … }` normalizes to a **closed** one (`closed = true`). The un-scoped case is
   not a smaller theorem, it is a false statement, and the mechanism is checked in both directions below
   (`an_unscoped_name_occurrence_is_open` / `a_scoped_name_occurrence_is_closed`).
2. **A name occurrence is whatever the desugaring *reads* through `nameVar`**, which includes positions
   the *grammar* treats as binders: a receive's patterns, a `match`'s patterns and a bind's names are all
   normalized against `Γ` by the arms above rather than pushed onto it. So `Scoped*` mirrors the
   function, not the grammar — a predicate mirroring the grammar would demand variables the desugaring
   never looks up, and would make the theorem false wherever a pattern's name is not in `Γ`.
3. **`Γ` grows in exactly one place**: a `new`'s declarations (`declNames decls ++ Γ`), because that is
   the only arm that changes the stack. Every other arm passes `Γ` through unchanged.

The un-modelled connectives and collection operators are scoped as the constructs they are (their names
are checked) even though their arms return `none`: the statement is vacuous for them either way, and
stating the real thing means the predicate does not have to be revisited if one is ever brought into the
domain. -/
mutual
  /-- Every name the desugaring reads out of a process is in `Γ`. -/
  def ScopedIn (Γ : List SVar) : Surf → Prop
    | .ground _ => True
    | .collect c => ScopedCollect Γ c
    | .var x => x ∈ Γ
    | .varWild => True
    | .varRef _ x => x ∈ Γ
    | .nil => True
    | .simpleType _ => True
    | .neg p => ScopedIn Γ p
    | .conj a b => ScopedIn Γ a ∧ ScopedIn Γ b
    | .disj a b => ScopedIn Γ a ∧ ScopedIn Γ b
    | .eval n => ScopedName Γ n
    | .method t x ds => ScopedIn Γ t ∧ x ∈ Γ ∧ ScopedProcs Γ ds
    | .not p => ScopedIn Γ p
    | .negNum p => ScopedIn Γ p
    | .mult a b => ScopedIn Γ a ∧ ScopedIn Γ b
    | .div a b => ScopedIn Γ a ∧ ScopedIn Γ b
    | .mod a b => ScopedIn Γ a ∧ ScopedIn Γ b
    | .pctPct a b => ScopedIn Γ a ∧ ScopedIn Γ b
    | .add a b => ScopedIn Γ a ∧ ScopedIn Γ b
    | .sub a b => ScopedIn Γ a ∧ ScopedIn Γ b
    | .plusPlus a b => ScopedIn Γ a ∧ ScopedIn Γ b
    | .minusMinus a b => ScopedIn Γ a ∧ ScopedIn Γ b
    | .lt a b => ScopedIn Γ a ∧ ScopedIn Γ b
    | .lte a b => ScopedIn Γ a ∧ ScopedIn Γ b
    | .gt a b => ScopedIn Γ a ∧ ScopedIn Γ b
    | .gte a b => ScopedIn Γ a ∧ ScopedIn Γ b
    | .matches a b => ScopedIn Γ a ∧ ScopedIn Γ b
    | .eq a b => ScopedIn Γ a ∧ ScopedIn Γ b
    | .neq a b => ScopedIn Γ a ∧ ScopedIn Γ b
    | .and a b => ScopedIn Γ a ∧ ScopedIn Γ b
    | .shortAnd a b => ScopedIn Γ a ∧ ScopedIn Γ b
    | .or a b => ScopedIn Γ a ∧ ScopedIn Γ b
    | .shortOr a b => ScopedIn Γ a ∧ ScopedIn Γ b
    | .send n _ ds => ScopedName Γ n ∧ ScopedProcs Γ ds
    | .contr n ps rest body => ScopedName Γ n ∧ ScopedNames Γ ps ∧ ScopedRemainder Γ rest
        ∧ ScopedIn Γ body
    | .input rs body => ScopedReceipts Γ rs ∧ ScopedIn Γ body
    | .choice bs => ScopedBranches Γ bs
    | .match t cs => ScopedIn Γ t ∧ ScopedCases Γ cs
    | .bundle _ body => ScopedIn Γ body
    | .letIn d ds body => ScopedDecl Γ d ∧ ScopedDecls Γ ds ∧ ScopedIn Γ body
    | .ifThen c t => ScopedIn Γ c ∧ ScopedIn Γ t
    | .ifElse c t e => ScopedIn Γ c ∧ ScopedIn Γ t ∧ ScopedIn Γ e
    | .newIn decls body => ScopedIn (declNames decls ++ Γ) body
    | .sendSynch n ds _ => ScopedName Γ n ∧ ScopedProcs Γ ds
    | .par a b => ScopedIn Γ a ∧ ScopedIn Γ b

  /-- Every process of the list is in scope. -/
  def ScopedProcs (Γ : List SVar) : List Surf → Prop
    | [] => True
    | p :: ps => ScopedIn Γ p ∧ ScopedProcs Γ ps

  /-- Every name of the list is in scope. -/
  def ScopedNames (Γ : List SVar) : List SName → Prop
    | [] => True
    | n :: ns => ScopedName Γ n ∧ ScopedNames Γ ns

  /-- A name occurrence: a bare name is read through `nameVar`, a quoted process is desugared. -/
  def ScopedName (Γ : List SVar) : SName → Prop
    | .wild => True
    | .var x => x ∈ Γ
    | .quote p => ScopedIn Γ p

  /-- A collection's `...rest`, which the desugaring reads through `nameVar` too. -/
  def ScopedRemainder (Γ : List SVar) : Option SVar → Prop
    | none => True
    | some r => r ∈ Γ

  /-- A bind's source. -/
  def ScopedSource (Γ : List SVar) : SNameSource → Prop
    | .simple n => ScopedName Γ n
    | .receiveSend n => ScopedName Γ n
    | .sendReceive n ds => ScopedName Γ n ∧ ScopedProcs Γ ds

  /-- A receipt's binds. -/
  def ScopedBinds (Γ : List SVar) : List SBind → Prop
    | [] => True
    | ⟨ns, rest, src⟩ :: tl => ScopedNames Γ ns ∧ ScopedRemainder Γ rest ∧ ScopedSource Γ src
        ∧ ScopedBinds Γ tl

  /-- One receipt. A `peek` is out of the desugaring's domain; it is scoped all the same. -/
  def ScopedReceipt (Γ : List SVar) : SReceipt → Prop
    | .linear bs => ScopedBinds Γ bs
    | .repeated bs => ScopedBinds Γ bs
    | .peek bs => ScopedBinds Γ bs

  /-- A receipt list, in order. -/
  def ScopedReceipts (Γ : List SVar) : List SReceipt → Prop
    | [] => True
    | r :: rs => ScopedReceipt Γ r ∧ ScopedReceipts Γ rs

  /-- A `match`'s cases: both the pattern and the body are desugared against `Γ`. -/
  def ScopedCases (Γ : List SVar) : List SCase → Prop
    | [] => True
    | ⟨pat, body⟩ :: cs => ScopedIn Γ pat ∧ ScopedIn Γ body ∧ ScopedCases Γ cs

  /-- A map's pairs: both sides are desugared against `Γ`. -/
  def ScopedKvs (Γ : List SVar) : List SKeyValuePair → Prop
    | [] => True
    | ⟨k, v⟩ :: rest => ScopedIn Γ k ∧ ScopedIn Γ v ∧ ScopedKvs Γ rest

  /-- A collection. -/
  def ScopedCollect (Γ : List SVar) : SCollect → Prop
    | .list ps rem => ScopedProcs Γ ps ∧ ScopedRemainder Γ rem
    | .set ps rem => ScopedProcs Γ ps ∧ ScopedRemainder Γ rem
    | .tuple first rest => ScopedIn Γ first ∧ ScopedProcs Γ rest
    | .map kvs rem => ScopedKvs Γ kvs ∧ ScopedRemainder Γ rem

  /-- A `let` declaration: names, remainder and the values. (`let` is outside the domain.) -/
  def ScopedDecl (Γ : List SVar) : SDecl → Prop
    | ⟨ns, rest, vals⟩ => ScopedNames Γ ns ∧ ScopedRemainder Γ rest ∧ ScopedProcs Γ vals

  /-- A list of declarations. -/
  def ScopedDeclList (Γ : List SVar) : List SDecl → Prop
    | [] => True
    | d :: ds => ScopedDecl Γ d ∧ ScopedDeclList Γ ds

  /-- A `let`'s declaration block, in either of its two shapes. -/
  def ScopedDecls (Γ : List SVar) : SDecls → Prop
    | .empty => True
    | .linear ds => ScopedDeclList Γ ds
    | .conc ds => ScopedDeclList Γ ds

  /-- A `select`'s branches. (`select` is outside the domain.) -/
  def ScopedBranches (Γ : List SVar) : List SBranch → Prop
    | [] => True
    | ⟨bs, body⟩ :: rest => ScopedBinds Γ bs ∧ ScopedIn Γ body ∧ ScopedBranches Γ rest
end

/-- **The hypothesis is not decoration: the statement is false without it.** `for (x <- c) { x!(1) }`
    with nothing in scope desugars to a term with a free occurrence — `nameVar` answers `.free 0` for
    the pattern's `x`, its body's `x`, and the channel `c` — so the output is *open*. This is the
    counterexample law 36's row cites for the un-scoped reading, and it is what makes `ScopedIn` the
    statement's shape rather than a convenience. -/
theorem an_unscoped_name_occurrence_is_open :
    closed (parOf (.evar (nameVar [] "x"))) = false := by rfl

/-- **And with the name in scope the same occurrence is closed** — the positive half, one line: `nameVar`
    answers `.bound i` for a name in `Γ`, and `closedVar` counts a bound variable as closed. The pair
    fixes law 36's shape: the hypothesis is exactly "the names the desugaring reads are in `Γ`", and both
    directions of it are kernel-checked here. Measured on whole terms as well (`#eval`, 2026-09-25):
    `for (x <- c) { x!(1) }` alone desugars to `closed = false`, the same term under
    `new c, x in { … }` to `closed = true` — the statement the owed induction generalizes from one path
    to every path. -/
theorem a_scoped_name_occurrence_is_closed :
    closed (parOf (.evar (nameVar ["x"] "x"))) = true := by rfl

/-! ## The induction's leaves

Law 36's owed induction (`∀ e acc Γ, Closed acc → ScopedIn Γ e → ∀ p, normalizeAt e acc Γ = some p →
Closed p`) reduces every arm to two things: `Closed_parMerge` (`Ty.lean:277`) and the closedness of the
`Par` that arm *builds*. These are the second things — one lemma per shape `normalizeAt` constructs.

They are stated in the **`Bool` form** (`closed … = true`), which is what makes them usable: an arm
`unfold Closed` and hands one of these over, and the companion functions' own statements (`procsPar`'s,
`bindsPar`'s, …) conclude in the same form — `closedListPar qs = true` rather than "every element is
closed" — so nothing has to convert between the two. That is a deliberate shape, not a convenience: the
first draft stated the leaves in the `Closed` form and needed four bridging lemmas between the `&&`-style
list checkers and `∀ x ∈ l` (`closedListPar` is a `&&`-fold, not a `List.all`), each of which then fought
the `Closed`-unfolding simp lemmas. With the `Bool` form the leaves are one `simp` each.

The shapes, read off the arms: a **send**, a **receive**, a **new**, a **match** and a **bundle** (each a
`Par.mk` with exactly one field populated); a **value in expression position** (`parOf`, and the four
collection expressions, whose children are a list and an optional remainder); and — the one that makes
the hypothesis do its work — `nameVar Γ x` for `x ∈ Γ`. -/

/-- A **send**: one `Send` in the sends field, nothing else. -/
theorem closed_sendPar (c : Par) (d : List Par) (b : Bool) (hc : closed c = true)
    (hd : closedListPar d = true) :
    closed (Par.mk [Send.mk c d b] [] [] [] [] [] [] []) = true := by
  simp [closed, closedListSend, closedListPar, closedSend, hc, hd]

/-- A **receive**: binds and body, in the receives field. -/
theorem closed_receivePar (bs : List ReceiveBind) (body : Par) (p : Bool) (n : Nat)
    (hbs : closedListReceiveBind bs = true) (hbody : closed body = true) :
    closed (Par.mk [] [Receive.mk bs body p n] [] [] [] [] [] []) = true := by
  simp [closed, closedListReceive, closedReceive, hbs, hbody]

/-- A **new**: the identifier count and its body. -/
theorem closed_newPar (n : Nat) (body : Par) (h : closed body = true) :
    closed (Par.mk [] [] [New.mk n body] [] [] [] [] []) = true := by
  simp [closed, closedListNew, closedNew, h]

/-- A **match**: the target and the cases, in the matches field. (The `if` arms build one of these, with
    a `MatchCase` per branch, which is why `MatchCase` itself needs no arm — its closedness comes from
    the cases' checker, which `casesPar`'s statement produces.) -/
theorem closed_matchPar (t : Par) (cs : List MatchCase) (ht : closed t = true)
    (hcs : closedListMatchCase cs = true) :
    closed (Par.mk [] [] [] [] [Match.mk t cs] [] [] []) = true := by
  simp [closed, closedListMatch, closedMatch, hcs, ht]

/-- A **bundle**: the body, with its two capability flags. -/
theorem closed_bundlePar (q : Par) (w r : Bool) (h : closed q = true) :
    closed (Par.mk [] [] [] [] [] [] [Bundle.mk q w r] []) = true := by
  simp [closed, closedListBundle, closedBundle, h]

/-- A **value in expression position**: `parOf` of any closed expression. -/
theorem closed_parOf (e : Expr) (h : closedExpr e = true) : closed (parOf e) = true := by
  simp [closed, parOf, closedListExpr, h]

/-- **The one that makes the hypothesis do its work**: a name the binder stack holds is a *bound*
    variable, and a bound variable is closed. An occurrence outside `Γ` is `nameVar`'s `free 0` arm,
    which is open — which is why `ScopedIn` is the statement's hypothesis and not decoration, and why
    `an_unscoped_name_occurrence_is_open` above is its counterpart.

    The lookup half first: `findIdx?` finds a member. (This toolchain carries no
    `List.findIdx?_eq_none`-style lemma to do it in one step.) -/
theorem findIdx?_some_of_mem : ∀ (Γ : List SVar) {x : SVar}, x ∈ Γ →
    ∃ i, Γ.findIdx? (fun y => y == x) = some i
  | [], _, h => absurd h (List.not_mem_nil _)
  | y :: ys, x, h => by
    rw [List.findIdx?]
    by_cases hyx : y == x
    · exact ⟨0, by simp [hyx]⟩
    · obtain ⟨i, hi⟩ := findIdx?_some_of_mem ys (by
        rcases List.mem_cons.mp h with heq | hmem
        · exact absurd (by rw [heq]; exact beq_self_eq_true y) hyx
        · exact hmem)
      exact ⟨i + 1, by simp [hyx, hi]⟩

/-- And the closedness: a found index makes `nameVar` answer `.bound i`, which `closedVar` counts as
    closed. -/
theorem closedVar_nameVar {Γ : List SVar} {x : SVar} (h : x ∈ Γ) :
    closedVar (nameVar Γ x) = true := by
  obtain ⟨i, hi⟩ := findIdx?_some_of_mem Γ h
  unfold nameVar
  rw [hi]
  rfl

/-- A **list collection** — `[a, b, ...rest]`. The remainder is `Option Var`, and its closedness is
    `closedRemainder`: the *scoped* hypothesis is what makes it a bound variable rather than a free
    one (`closedVar_nameVar`). -/
theorem closed_parOf_elist (ps : List Par) (rem : Option Var) (hps : closedListPar ps = true)
    (hrem : closedRemainder rem = true) : closed (parOf (.elist ps rem)) = true := by
  simp [closed, parOf, closedListExpr, closedExpr, hps, hrem]

/-- A **set collection** — the same shape with `eset`'s checker. -/
theorem closed_parOf_eset (ps : List Par) (rem : Option Var) (hps : closedListPar ps = true)
    (hrem : closedRemainder rem = true) : closed (parOf (.eset ps rem)) = true := by
  simp [closed, parOf, closedListExpr, closedExpr, hps, hrem]

/-- A **map collection** — whose children are pairs, checked by `closedListParPair`. -/
theorem closed_parOf_emap (kvs : List (Par × Par)) (rem : Option Var)
    (hkvs : closedListParPair kvs = true) (hrem : closedRemainder rem = true) :
    closed (parOf (.emap kvs rem)) = true := by
  simp [closed, parOf, closedListExpr, closedExpr, hkvs, hrem]

/-- A **tuple**: `TupleSingle` and `TupleMultiple` are one constructor here, and the checker is the list
    with the first element consed on. -/
theorem closed_matchCasePar (pat body : Par) (n : Nat) (hp : closed pat = true)
    (hb : closed body = true) : closedMatchCase (MatchCase.mk pat body n) = true := by
  simp [closedMatchCase, hp, hb]

theorem closed_parOf_etuple (first : Par) (rest : List Par) (h1 : closed first = true)
    (hr : closedListPar rest = true) : closed (parOf (.etuple (first :: rest))) = true := by
  simp [closed, parOf, closedListExpr, closedExpr, closedListPar, h1, hr]



/-! ## Law 36's induction

The theorem the row owes, and the ten companions that its arms call. They are proved by the block's own
induction — `termination_by` on each, with the calls into a list's *head fields* (a bind's `src`, a
case's `pat`/`body`) written as **equations**, because that is what exposes the subterm relation to the
termination checker: with `induction`/`cases` inside the proof the measure generalizes and the obligation
becomes false (`sizeOf pat < sizeOf cs` rather than `… < sizeOf (⟨pat, body⟩ :: cs)`), which is how the
first draft of these two failed. The statement is law 36's: a source term whose every name occurrence the
binder stack holds normalizes to a closed `Par`. -/

theorem closed_iff_Closed (p : Par) : closed p = true ↔ Closed p := by
  simp [Closed, closed, Bool.and_eq_true]

theorem closed_parMerge (p q : Par) :
    closed (parMerge p q) = true ↔ closed p = true ∧ closed q = true := by
  rw [closed_iff_Closed, Closed_parMerge_iff, closed_iff_Closed, closed_iff_Closed]

theorem closed_parOf_ground_b (g : Ground) : closed (parOf (.ground g)) = true := by
  cases g <;> simp [closed, parOf, closedListExpr, closedExpr]

theorem closed_groundPar (g : SGround) : ∀ p, groundPar g = some p → closed p = true := by
  intro p hp
  cases g with
  | bool b => simp only [groundPar, Option.some.injEq] at hp; rw [← hp]; exact closed_parOf_ground_b _
  | int d => simp only [groundPar] at hp
             rw [Option.map_eq_some'] at hp
             obtain ⟨n, -, hp'⟩ := hp
             rw [← hp']
             exact closed_parOf_ground_b _
  | str raw => simp only [groundPar, Option.some.injEq] at hp; rw [← hp]; exact closed_parOf_ground_b _
  | uri raw => simp only [groundPar, Option.some.injEq] at hp; rw [← hp]; exact closed_parOf_ground_b _
  | bigint d =>
    simp only [groundPar] at hp
    rw [Option.map_eq_some'] at hp
    obtain ⟨n, -, hp'⟩ := hp
    rw [← hp']
    simp [closed, parOf, closedListExpr, closedExpr]

theorem closed_receiveBindPar (pats : List Par) (c : Par) (n : Nat)
    (hp : closedListPar pats = true) (hc : closed c = true) :
    closedReceiveBind (ReceiveBind.mk pats c n) = true := by
  simp [closedReceiveBind, hp, hc]

theorem closed_receiveOneList (rb : ReceiveBind) (h : closedReceiveBind rb = true) :
    closedListReceiveBind [rb] = true := by
  simp [closedListReceiveBind, h]

theorem closed_matchOneList (mc : MatchCase) (h : closedMatchCase mc = true) :
    closedListMatchCase [mc] = true := by
  simp [closedListMatchCase, h]

theorem closed_matchTwoList (mc1 mc2 : MatchCase) (h1 : closedMatchCase mc1 = true)
    (h2 : closedMatchCase mc2 = true) : closedListMatchCase [mc1, mc2] = true := by
  simp [closedListMatchCase, h1, h2]

theorem closed_receiveListPar (rs : List Receive) (hrs : closedListReceive rs = true) :
    closed (Par.mk [] rs [] [] [] [] [] []) = true := by
  simp [closed, closedListReceive, hrs]

theorem closed_receiveOnePar (rb : ReceiveBind) (body : Par) (p : Bool) (n : Nat)
    (hbs : closedReceiveBind rb = true) (hbody : closed body = true) :
    closed (Par.mk [] [Receive.mk [rb] body p n] [] [] [] [] [] []) = true := by
  simp [closed, closedListReceive, closedReceive, closedListReceiveBind, hbs, hbody]

theorem closed_matchOnePar (cond : Par) (mc : MatchCase) (ht : closed cond = true)
    (hmc : closedMatchCase mc = true) :
    closed (Par.mk [] [] [] [] [Match.mk cond [mc]] [] [] []) = true := by
  simp [closed, closedListMatch, closedMatch, closedListMatchCase, ht, hmc]

theorem closed_matchTwoPar (cond : Par) (mc1 mc2 : MatchCase) (ht : closed cond = true)
    (h1 : closedMatchCase mc1 = true) (h2 : closedMatchCase mc2 = true) :
    closed (Par.mk [] [] [] [] [Match.mk cond [mc1, mc2]] [] [] []) = true := by
  simp [closed, closedListMatch, closedMatch, closedListMatchCase, ht, h1, h2]

mutual
  theorem closed_normalizeAt : ∀ (e : Surf) (acc : Par) (Γ : List SVar), Closed acc → ScopedIn Γ e →
      ∀ p, normalizeAt e acc Γ = some p → closed p = true := by
    intro e
    cases e with
    | ground g =>
      intro acc Γ hacc hs p hp
      simp only [normalizeAt] at hp
      rw [Option.map_eq_some'] at hp
      obtain ⟨q, hq, hp'⟩ := hp
      rw [← hp']
      exact (closed_parMerge acc q).mpr ⟨(closed_iff_Closed acc).mpr hacc, closed_groundPar g q hq⟩
    | collect c =>
      intro acc Γ hacc hs p hp
      simp only [normalizeAt] at hp
      rw [Option.map_eq_some'] at hp
      obtain ⟨q, hq, hp'⟩ := hp
      rw [← hp']
      exact (closed_parMerge acc q).mpr ⟨(closed_iff_Closed acc).mpr hacc, closed_collectPar c Γ hs q hq⟩
    | var x =>
      intro acc Γ hacc hs p hp
      simp only [normalizeAt] at hp
      rw [Option.some.injEq] at hp
      rw [← hp]
      exact (closed_parMerge acc _).mpr ⟨(closed_iff_Closed acc).mpr hacc,
        closed_parOf _ (by simp [closedExpr, closedVar_nameVar hs])⟩
    | varWild =>
      intro acc Γ hacc hs p hp
      simp only [normalizeAt] at hp
      rw [Option.some.injEq] at hp
      rw [← hp]
      exact (closed_parMerge acc _).mpr ⟨(closed_iff_Closed acc).mpr hacc,
        closed_parOf _ (by simp [closedExpr, closedVar])⟩
    | nil =>
      intro acc Γ hacc hs p hp
      simp only [normalizeAt] at hp
      rw [Option.some.injEq] at hp
      rw [← hp]
      exact (closed_iff_Closed acc).mpr hacc
    | neg p' =>
      intro acc Γ hacc hs p hp
      simp only [normalizeAt] at hp
      rw [Option.map_eq_some'] at hp
      obtain ⟨q, hq, hp'⟩ := hp
      rw [← hp']
      exact (closed_parMerge acc _).mpr ⟨(closed_iff_Closed acc).mpr hacc,
        closed_parOf _ (by simp [closedExpr, (closed_iff_Closed q).mp (closed_normalizeAt p' nilPar Γ Closed_nil (by simpa using hs) q hq)])⟩
    | not p' =>
      intro acc Γ hacc hs p hp
      simp only [normalizeAt] at hp
      rw [Option.map_eq_some'] at hp
      obtain ⟨q, hq, hp'⟩ := hp
      rw [← hp']
      exact (closed_parMerge acc _).mpr ⟨(closed_iff_Closed acc).mpr hacc,
        closed_parOf _ (by simp [closedExpr, (closed_iff_Closed q).mp (closed_normalizeAt p' nilPar Γ Closed_nil (by simpa using hs) q hq)])⟩
    | negNum p' =>
      intro acc Γ hacc hs p hp
      simp only [normalizeAt] at hp
      rw [Option.map_eq_some'] at hp
      obtain ⟨q, hq, hp'⟩ := hp
      rw [← hp']
      exact (closed_parMerge acc _).mpr ⟨(closed_iff_Closed acc).mpr hacc,
        closed_parOf _ (by simp [closedExpr, (closed_iff_Closed q).mp (closed_normalizeAt p' nilPar Γ Closed_nil (by simpa using hs) q hq)])⟩
    | par a b =>
      intro acc Γ hacc hs p hp
      simp only [normalizeAt] at hp
      split at hp
      · rename_i q hq
        exact closed_normalizeAt b q Γ ((closed_iff_Closed q).mp (closed_normalizeAt a acc Γ hacc hs.1 q hq)) hs.2 p hp
      · exact absurd hp (by simp)
    | send n persistent data =>
      intro acc Γ hacc hs p hp
      simp only [normalizeAt] at hp
      split at hp
      · rename_i c d h1 h2
        rw [Option.some.injEq] at hp
        rw [← hp]
        exact (closed_parMerge acc _).mpr ⟨(closed_iff_Closed acc).mpr hacc,
          closed_sendPar c d persistent (closed_namePar n Γ hs.1 c h1) (closed_procsPar data Γ hs.2 d h2)⟩
      · exact absurd hp (by simp)
    | contr n params rest body =>
      intro acc Γ hacc hs p hp
      simp only [normalizeAt] at hp
      split at hp
      · rename_i c pats b h1 h2 h3
        rw [Option.some.injEq] at hp
        rw [← hp]
        exact (closed_parMerge acc _).mpr ⟨(closed_iff_Closed acc).mpr hacc,
          closed_receiveOnePar (ReceiveBind.mk pats c pats.length) b true 1
            (closed_receiveBindPar pats c pats.length (closed_namesPar params rest Γ hs.2.1 pats h2) (closed_namePar n Γ hs.1 c h1))
            (closed_normalizeAt body nilPar Γ Closed_nil hs.2.2.2 b h3)⟩
      · exact absurd hp (by simp)
    | input receipts body =>
      intro acc Γ hacc hs p hp
      simp only [normalizeAt] at hp
      split at hp
      · rename_i b hb
        split at hp
        · rename_i rs hrs
          rw [Option.some.injEq] at hp
          rw [← hp]
          exact (closed_parMerge acc _).mpr ⟨(closed_iff_Closed acc).mpr hacc,
            closed_receiveListPar rs (closed_receiptsPar receipts b Γ
              (closed_normalizeAt body nilPar Γ Closed_nil hs.2 b hb) hs.1 rs hrs)⟩
        · exact absurd hp (by simp)
      · exact absurd hp (by simp)
    | «match» target cases =>
      intro acc Γ hacc hs p hp
      simp only [normalizeAt] at hp
      split at hp
      · rename_i t cs h1 h2
        rw [Option.some.injEq] at hp
        rw [← hp]
        exact (closed_parMerge acc _).mpr ⟨(closed_iff_Closed acc).mpr hacc,
          closed_matchPar t cs (closed_normalizeAt target nilPar Γ Closed_nil hs.1 t h1) (closed_casesPar cases Γ hs.2 cs h2)⟩
      · exact absurd hp (by simp)
    | bundle b body =>
      intro acc Γ hacc hs p hp
      simp only [normalizeAt] at hp
      split at hp
      · rename_i q hq
        rw [Option.some.injEq] at hp
        rw [← hp]
        exact (closed_parMerge acc _).mpr ⟨(closed_iff_Closed acc).mpr hacc,
          closed_bundlePar q (bundleFlags b).1 (bundleFlags b).2
            (closed_normalizeAt body nilPar Γ Closed_nil (by simpa using hs) q hq)⟩
      · exact absurd hp (by simp)
    | ifThen c t =>
      intro acc Γ hacc hs p hp
      simp only [normalizeAt] at hp
      split at hp
      · rename_i cond thenB h1 h2
        rw [Option.some.injEq] at hp
        rw [← hp]
        exact (closed_parMerge acc _).mpr ⟨(closed_iff_Closed acc).mpr hacc,
          closed_matchOnePar cond (MatchCase.mk (parOf (.ground (.bool true))) thenB 0)
            (closed_normalizeAt c nilPar Γ Closed_nil hs.1 cond h1)
            (closed_matchCasePar (parOf (.ground (.bool true))) thenB 0
              (closed_groundPar (.bool true) _ rfl)
              (closed_normalizeAt t nilPar Γ Closed_nil hs.2 thenB h2))⟩
      · exact absurd hp (by simp)
    | ifElse c t e' =>
      intro acc Γ hacc hs p hp
      simp only [normalizeAt] at hp
      split at hp
      · rename_i cond thenB elseB h1 h2 h3
        rw [Option.some.injEq] at hp
        rw [← hp]
        exact (closed_parMerge acc _).mpr ⟨(closed_iff_Closed acc).mpr hacc,
          closed_matchTwoPar cond
            (MatchCase.mk (parOf (.ground (.bool true))) thenB 0)
            (MatchCase.mk (parOf (.ground (.bool false))) elseB 0)
            (closed_normalizeAt c nilPar Γ Closed_nil hs.1 cond h1)
            (closed_matchCasePar (parOf (.ground (.bool true))) thenB 0
              (closed_groundPar (.bool true) _ rfl)
              (closed_normalizeAt t nilPar Γ Closed_nil hs.2.1 thenB h2))
            (closed_matchCasePar (parOf (.ground (.bool false))) elseB 0
              (closed_groundPar (.bool false) _ rfl)
              (closed_normalizeAt e' nilPar Γ Closed_nil hs.2.2 elseB h3))⟩
      · exact absurd hp (by simp)
    | newIn decls body =>
      intro acc Γ hacc hs p hp
      simp only [normalizeAt] at hp
      split at hp
      · rename_i d b hd hb
        rw [Option.some.injEq] at hp
        rw [← hp]
        exact (closed_parMerge acc _).mpr ⟨(closed_iff_Closed acc).mpr hacc,
          closed_newPar d b (closed_normalizeAt body nilPar (declNames decls ++ Γ) Closed_nil (by simpa using hs) b hb)⟩
      · exact absurd hp (by simp)
    | simpleType ty => intro acc Γ hacc hs p hp; simp only [normalizeAt] at hp; exact absurd hp (by simp)
    | method t x ds =>
      intro acc Γ hacc hs p hp
      simp only [normalizeAt] at hp
      split at hp
      · rename_i r1 qs h1 h2
        rw [Option.some.injEq] at hp
        rw [← hp]
        exact (closed_parMerge acc _).mpr ⟨(closed_iff_Closed acc).mpr hacc,
          closed_parOf _ (by simp [closedExpr,
            (closed_iff_Closed r1).mp (closed_normalizeAt t nilPar Γ Closed_nil (by simpa using hs.1) r1 h1),
            closed_procsPar ds Γ hs.2.2 qs h2])⟩
      · exact absurd hp (by simp)
    | eval n => intro acc Γ hacc hs p hp; simp only [normalizeAt] at hp; exact absurd hp (by simp)
    | choice bs => intro acc Γ hacc hs p hp; simp only [normalizeAt] at hp; exact absurd hp (by simp)
    | letIn d ds body => intro acc Γ hacc hs p hp; simp only [normalizeAt] at hp; exact absurd hp (by simp)
    | sendSynch n ds c => intro acc Γ hacc hs p hp; simp only [normalizeAt] at hp; exact absurd hp (by simp)
    | varRef k x => intro acc Γ hacc hs p hp; simp only [normalizeAt] at hp; exact absurd hp (by simp)
    | conj a b => intro acc Γ hacc hs p hp; simp only [normalizeAt] at hp; exact absurd hp (by simp)
    | disj a b => intro acc Γ hacc hs p hp; simp only [normalizeAt] at hp; exact absurd hp (by simp)
    | plusPlus a b =>
      intro acc Γ hacc hs p hp
      simp only [normalizeAt] at hp
      split at hp
      · rename_i r1 r2 h1 h2
        rw [Option.some.injEq] at hp
        rw [← hp]
        exact (closed_parMerge acc _).mpr ⟨(closed_iff_Closed acc).mpr hacc,
          closed_parOf _ (by simp [closedExpr,
            (closed_iff_Closed r1).mp (closed_normalizeAt a nilPar Γ Closed_nil (by simpa using hs.1) r1 h1),
            (closed_iff_Closed r2).mp (closed_normalizeAt b nilPar Γ Closed_nil (by simpa using hs.2) r2 h2)])⟩
      · exact absurd hp (by simp)
    | minusMinus a b =>
      intro acc Γ hacc hs p hp
      simp only [normalizeAt] at hp
      split at hp
      · rename_i r1 r2 h1 h2
        rw [Option.some.injEq] at hp
        rw [← hp]
        exact (closed_parMerge acc _).mpr ⟨(closed_iff_Closed acc).mpr hacc,
          closed_parOf _ (by simp [closedExpr,
            (closed_iff_Closed r1).mp (closed_normalizeAt a nilPar Γ Closed_nil (by simpa using hs.1) r1 h1),
            (closed_iff_Closed r2).mp (closed_normalizeAt b nilPar Γ Closed_nil (by simpa using hs.2) r2 h2)])⟩
      · exact absurd hp (by simp)
    | pctPct a b =>
      intro acc Γ hacc hs p hp
      simp only [normalizeAt] at hp
      split at hp
      · rename_i r1 r2 h1 h2
        rw [Option.some.injEq] at hp
        rw [← hp]
        exact (closed_parMerge acc _).mpr ⟨(closed_iff_Closed acc).mpr hacc,
          closed_parOf _ (by simp [closedExpr,
            (closed_iff_Closed r1).mp (closed_normalizeAt a nilPar Γ Closed_nil (by simpa using hs.1) r1 h1),
            (closed_iff_Closed r2).mp (closed_normalizeAt b nilPar Γ Closed_nil (by simpa using hs.2) r2 h2)])⟩
      · exact absurd hp (by simp)
    | mult a b =>
      intro acc Γ hacc hs p hp
      simp only [normalizeAt] at hp
      split at hp
      · rename_i r1 r2 h1 h2
        rw [Option.some.injEq] at hp
        rw [← hp]
        exact (closed_parMerge acc _).mpr ⟨(closed_iff_Closed acc).mpr hacc,
          closed_parOf _ (by simp [closedExpr,
            (closed_iff_Closed r1).mp (closed_normalizeAt a nilPar Γ Closed_nil (by simpa using hs.1) r1 h1),
            (closed_iff_Closed r2).mp (closed_normalizeAt b nilPar Γ Closed_nil (by simpa using hs.2) r2 h2)])⟩
      · exact absurd hp (by simp)
    | div a b =>
      intro acc Γ hacc hs p hp
      simp only [normalizeAt] at hp
      split at hp
      · rename_i r1 r2 h1 h2
        rw [Option.some.injEq] at hp
        rw [← hp]
        exact (closed_parMerge acc _).mpr ⟨(closed_iff_Closed acc).mpr hacc,
          closed_parOf _ (by simp [closedExpr,
            (closed_iff_Closed r1).mp (closed_normalizeAt a nilPar Γ Closed_nil (by simpa using hs.1) r1 h1),
            (closed_iff_Closed r2).mp (closed_normalizeAt b nilPar Γ Closed_nil (by simpa using hs.2) r2 h2)])⟩
      · exact absurd hp (by simp)
    | mod a b =>
      intro acc Γ hacc hs p hp
      simp only [normalizeAt] at hp
      split at hp
      · rename_i r1 r2 h1 h2
        rw [Option.some.injEq] at hp
        rw [← hp]
        exact (closed_parMerge acc _).mpr ⟨(closed_iff_Closed acc).mpr hacc,
          closed_parOf _ (by simp [closedExpr,
            (closed_iff_Closed r1).mp (closed_normalizeAt a nilPar Γ Closed_nil (by simpa using hs.1) r1 h1),
            (closed_iff_Closed r2).mp (closed_normalizeAt b nilPar Γ Closed_nil (by simpa using hs.2) r2 h2)])⟩
      · exact absurd hp (by simp)
    | add a b =>
      intro acc Γ hacc hs p hp
      simp only [normalizeAt] at hp
      split at hp
      · rename_i r1 r2 h1 h2
        rw [Option.some.injEq] at hp
        rw [← hp]
        exact (closed_parMerge acc _).mpr ⟨(closed_iff_Closed acc).mpr hacc,
          closed_parOf _ (by simp [closedExpr,
            (closed_iff_Closed r1).mp (closed_normalizeAt a nilPar Γ Closed_nil (by simpa using hs.1) r1 h1),
            (closed_iff_Closed r2).mp (closed_normalizeAt b nilPar Γ Closed_nil (by simpa using hs.2) r2 h2)])⟩
      · exact absurd hp (by simp)
    | sub a b =>
      intro acc Γ hacc hs p hp
      simp only [normalizeAt] at hp
      split at hp
      · rename_i r1 r2 h1 h2
        rw [Option.some.injEq] at hp
        rw [← hp]
        exact (closed_parMerge acc _).mpr ⟨(closed_iff_Closed acc).mpr hacc,
          closed_parOf _ (by simp [closedExpr,
            (closed_iff_Closed r1).mp (closed_normalizeAt a nilPar Γ Closed_nil (by simpa using hs.1) r1 h1),
            (closed_iff_Closed r2).mp (closed_normalizeAt b nilPar Γ Closed_nil (by simpa using hs.2) r2 h2)])⟩
      · exact absurd hp (by simp)
    | lt a b =>
      intro acc Γ hacc hs p hp
      simp only [normalizeAt] at hp
      split at hp
      · rename_i r1 r2 h1 h2
        rw [Option.some.injEq] at hp
        rw [← hp]
        exact (closed_parMerge acc _).mpr ⟨(closed_iff_Closed acc).mpr hacc,
          closed_parOf _ (by simp [closedExpr,
            (closed_iff_Closed r1).mp (closed_normalizeAt a nilPar Γ Closed_nil (by simpa using hs.1) r1 h1),
            (closed_iff_Closed r2).mp (closed_normalizeAt b nilPar Γ Closed_nil (by simpa using hs.2) r2 h2)])⟩
      · exact absurd hp (by simp)
    | lte a b =>
      intro acc Γ hacc hs p hp
      simp only [normalizeAt] at hp
      split at hp
      · rename_i r1 r2 h1 h2
        rw [Option.some.injEq] at hp
        rw [← hp]
        exact (closed_parMerge acc _).mpr ⟨(closed_iff_Closed acc).mpr hacc,
          closed_parOf _ (by simp [closedExpr,
            (closed_iff_Closed r1).mp (closed_normalizeAt a nilPar Γ Closed_nil (by simpa using hs.1) r1 h1),
            (closed_iff_Closed r2).mp (closed_normalizeAt b nilPar Γ Closed_nil (by simpa using hs.2) r2 h2)])⟩
      · exact absurd hp (by simp)
    | gt a b =>
      intro acc Γ hacc hs p hp
      simp only [normalizeAt] at hp
      split at hp
      · rename_i r1 r2 h1 h2
        rw [Option.some.injEq] at hp
        rw [← hp]
        exact (closed_parMerge acc _).mpr ⟨(closed_iff_Closed acc).mpr hacc,
          closed_parOf _ (by simp [closedExpr,
            (closed_iff_Closed r1).mp (closed_normalizeAt a nilPar Γ Closed_nil (by simpa using hs.1) r1 h1),
            (closed_iff_Closed r2).mp (closed_normalizeAt b nilPar Γ Closed_nil (by simpa using hs.2) r2 h2)])⟩
      · exact absurd hp (by simp)
    | gte a b =>
      intro acc Γ hacc hs p hp
      simp only [normalizeAt] at hp
      split at hp
      · rename_i r1 r2 h1 h2
        rw [Option.some.injEq] at hp
        rw [← hp]
        exact (closed_parMerge acc _).mpr ⟨(closed_iff_Closed acc).mpr hacc,
          closed_parOf _ (by simp [closedExpr,
            (closed_iff_Closed r1).mp (closed_normalizeAt a nilPar Γ Closed_nil (by simpa using hs.1) r1 h1),
            (closed_iff_Closed r2).mp (closed_normalizeAt b nilPar Γ Closed_nil (by simpa using hs.2) r2 h2)])⟩
      · exact absurd hp (by simp)
    | eq a b =>
      intro acc Γ hacc hs p hp
      simp only [normalizeAt] at hp
      split at hp
      · rename_i r1 r2 h1 h2
        rw [Option.some.injEq] at hp
        rw [← hp]
        exact (closed_parMerge acc _).mpr ⟨(closed_iff_Closed acc).mpr hacc,
          closed_parOf _ (by simp [closedExpr,
            (closed_iff_Closed r1).mp (closed_normalizeAt a nilPar Γ Closed_nil (by simpa using hs.1) r1 h1),
            (closed_iff_Closed r2).mp (closed_normalizeAt b nilPar Γ Closed_nil (by simpa using hs.2) r2 h2)])⟩
      · exact absurd hp (by simp)
    | neq a b =>
      intro acc Γ hacc hs p hp
      simp only [normalizeAt] at hp
      split at hp
      · rename_i r1 r2 h1 h2
        rw [Option.some.injEq] at hp
        rw [← hp]
        exact (closed_parMerge acc _).mpr ⟨(closed_iff_Closed acc).mpr hacc,
          closed_parOf _ (by simp [closedExpr,
            (closed_iff_Closed r1).mp (closed_normalizeAt a nilPar Γ Closed_nil (by simpa using hs.1) r1 h1),
            (closed_iff_Closed r2).mp (closed_normalizeAt b nilPar Γ Closed_nil (by simpa using hs.2) r2 h2)])⟩
      · exact absurd hp (by simp)
    | and a b =>
      intro acc Γ hacc hs p hp
      simp only [normalizeAt] at hp
      split at hp
      · rename_i r1 r2 h1 h2
        rw [Option.some.injEq] at hp
        rw [← hp]
        exact (closed_parMerge acc _).mpr ⟨(closed_iff_Closed acc).mpr hacc,
          closed_parOf _ (by simp [closedExpr,
            (closed_iff_Closed r1).mp (closed_normalizeAt a nilPar Γ Closed_nil (by simpa using hs.1) r1 h1),
            (closed_iff_Closed r2).mp (closed_normalizeAt b nilPar Γ Closed_nil (by simpa using hs.2) r2 h2)])⟩
      · exact absurd hp (by simp)
    | shortAnd a b =>
      intro acc Γ hacc hs p hp
      simp only [normalizeAt] at hp
      split at hp
      · rename_i r1 r2 h1 h2
        rw [Option.some.injEq] at hp
        rw [← hp]
        exact (closed_parMerge acc _).mpr ⟨(closed_iff_Closed acc).mpr hacc,
          closed_parOf _ (by simp [closedExpr,
            (closed_iff_Closed r1).mp (closed_normalizeAt a nilPar Γ Closed_nil (by simpa using hs.1) r1 h1),
            (closed_iff_Closed r2).mp (closed_normalizeAt b nilPar Γ Closed_nil (by simpa using hs.2) r2 h2)])⟩
      · exact absurd hp (by simp)
    | or a b =>
      intro acc Γ hacc hs p hp
      simp only [normalizeAt] at hp
      split at hp
      · rename_i r1 r2 h1 h2
        rw [Option.some.injEq] at hp
        rw [← hp]
        exact (closed_parMerge acc _).mpr ⟨(closed_iff_Closed acc).mpr hacc,
          closed_parOf _ (by simp [closedExpr,
            (closed_iff_Closed r1).mp (closed_normalizeAt a nilPar Γ Closed_nil (by simpa using hs.1) r1 h1),
            (closed_iff_Closed r2).mp (closed_normalizeAt b nilPar Γ Closed_nil (by simpa using hs.2) r2 h2)])⟩
      · exact absurd hp (by simp)
    | shortOr a b =>
      intro acc Γ hacc hs p hp
      simp only [normalizeAt] at hp
      split at hp
      · rename_i r1 r2 h1 h2
        rw [Option.some.injEq] at hp
        rw [← hp]
        exact (closed_parMerge acc _).mpr ⟨(closed_iff_Closed acc).mpr hacc,
          closed_parOf _ (by simp [closedExpr,
            (closed_iff_Closed r1).mp (closed_normalizeAt a nilPar Γ Closed_nil (by simpa using hs.1) r1 h1),
            (closed_iff_Closed r2).mp (closed_normalizeAt b nilPar Γ Closed_nil (by simpa using hs.2) r2 h2)])⟩
      · exact absurd hp (by simp)
    | «matches» a b =>
      intro acc Γ hacc hs p hp
      simp only [normalizeAt] at hp
      split at hp
      · rename_i r1 r2 h1 h2
        rw [Option.some.injEq] at hp
        rw [← hp]
        exact (closed_parMerge acc _).mpr ⟨(closed_iff_Closed acc).mpr hacc,
          closed_parOf _ (by simp [closedExpr,
            (closed_iff_Closed r1).mp (closed_normalizeAt a nilPar Γ Closed_nil (by simpa using hs.1) r1 h1),
            (closed_iff_Closed r2).mp (closed_normalizeAt b nilPar Γ Closed_nil (by simpa using hs.2) r2 h2)])⟩
      · exact absurd hp (by simp)
  termination_by e => sizeOf e
  theorem closed_procsPar : ∀ (ps : List Surf) (Γ : List SVar), ScopedProcs Γ ps →
      ∀ qs, procsPar ps Γ = some qs → closedListPar qs = true
    | [], Γ, hs, qs, hq => by
      simp only [procsPar] at hq; injection hq with h; subst h; rfl
    | p :: ps, Γ, hs, qs, hq => by
      simp only [procsPar] at hq
      split at hq
      · rename_i q qs' h1 h2
        rw [Option.some.injEq] at hq
        rw [← hq]
        simp only [closedListPar, closed_procsPar ps Γ hs.2 qs' h2,
          closed_normalizeAt p nilPar Γ Closed_nil hs.1 q h1, Bool.and_self, Bool.true_and]
      · exact absurd hq (by simp)
  termination_by ps => sizeOf ps
  theorem closed_namesPar : ∀ (ns : List SName) (rest : Option SVar) (Γ : List SVar),
      ScopedNames Γ ns → ∀ ps, namesPar ns rest Γ = some ps → closedListPar ps = true
    | [], none, Γ, hs, ps, hp => by
      simp only [namesPar] at hp; injection hp with h; subst h; rfl
    | [], some r, Γ, hs, ps, hp => by
      simp only [namesPar] at hp; exact absurd hp (by simp)
    | n :: ns, some r, Γ, hs, ps, hp => by
      simp only [namesPar] at hp; exact absurd hp (by simp)
    | n :: ns, none, Γ, hs, ps, hp => by
      simp only [namesPar] at hp
      split at hp
      · rename_i q qs h1 h2
        rw [Option.some.injEq] at hp
        rw [← hp]
        simp only [closedListPar, closed_namesPar ns none Γ hs.2 qs h2,
          closed_namePar n Γ hs.1 q h1, Bool.and_self, Bool.true_and]
      · exact absurd hp (by simp)
  termination_by ns => sizeOf ns
  theorem closed_namePar : ∀ (n : SName) (Γ : List SVar), ScopedName Γ n →
      ∀ p, namePar n Γ = some p → closed p = true := by
    intro n
    cases n with
    | wild => intro Γ hs p hp; simp only [namePar] at hp; injection hp with h; subst h
              exact closed_parOf _ (by simp [closedExpr, closedVar])
    | var x => intro Γ hs p hp; simp only [namePar] at hp; injection hp with h; subst h
               exact closed_parOf _ (by simp [closedExpr, closedVar_nameVar hs])
    | quote p' => intro Γ hs p hp; simp only [namePar] at hp
                  exact closed_normalizeAt p' nilPar Γ Closed_nil hs p hp

  termination_by n => sizeOf n
  theorem closed_collectPar : ∀ (c : SCollect) (Γ : List SVar), ScopedCollect Γ c →
      ∀ p, collectPar c Γ = some p → closed p = true := by
    intro c
    cases c with
    | list ps rem =>
      intro Γ hs p hp
      simp only [collectPar] at hp
      rw [Option.map_eq_some'] at hp
      obtain ⟨qs, hqs, hp'⟩ := hp
      rw [← hp']
      exact closed_parOf_elist qs (rem.map (fun r => nameVar Γ r))
        (closed_procsPar ps Γ hs.1 qs hqs)
        (by cases rem with
            | none => rfl
            | some r => simp [closedRemainder, closedVar_nameVar hs.2])
    | set ps rem =>
      intro Γ hs p hp
      simp only [collectPar] at hp
      rw [Option.map_eq_some'] at hp
      obtain ⟨qs, hqs, hp'⟩ := hp
      rw [← hp']
      exact closed_parOf_eset qs (rem.map (fun r => nameVar Γ r))
        (closed_procsPar ps Γ hs.1 qs hqs)
        (by cases rem with
            | none => rfl
            | some r => simp [closedRemainder, closedVar_nameVar hs.2])
    | tuple first rest =>
      intro Γ hs p hp
      simp only [collectPar] at hp
      split at hp
      · rename_i f rs h1 h2
        rw [Option.some.injEq] at hp
        rw [← hp]
        exact closed_parOf_etuple f rs (closed_normalizeAt first nilPar Γ Closed_nil hs.1 f h1)
          (closed_procsPar rest Γ hs.2 rs h2)
      · exact absurd hp (by simp)
    | map kvs rem =>
      intro Γ hs p hp
      simp only [collectPar] at hp
      split at hp
      · rename_i ps hps
        rw [Option.some.injEq] at hp
        rw [← hp]
        exact closed_parOf_emap ps (rem.map (fun r => nameVar Γ r))
          (closed_kvsPar kvs Γ hs.1 ps hps)
          (by cases rem with
              | none => rfl
              | some r => simp [closedRemainder, closedVar_nameVar hs.2])
      · exact absurd hp (by simp)

  termination_by c => sizeOf c
  theorem closed_kvsPar : ∀ (kvs : List SKeyValuePair) (Γ : List SVar), ScopedKvs Γ kvs →
      ∀ r, kvsPar kvs Γ = some r → closedListParPair r = true
    | [], Γ, hs, r, hr => by
      simp only [kvsPar] at hr; injection hr with h; subst h; rfl
    | SKeyValuePair.mk k v :: kvs, Γ, hs, r, hr => by
      simp only [kvsPar] at hr
      split at hr
      · rename_i kp vp rp h1 h2 h3
        rw [Option.some.injEq] at hr
        rw [← hr]
        simp [closedListParPair, closed_kvsPar kvs Γ hs.2.2 rp h3,
          (closed_iff_Closed kp).mp (closed_normalizeAt k nilPar Γ Closed_nil hs.1 kp h1),
          (closed_iff_Closed vp).mp (closed_normalizeAt v nilPar Γ Closed_nil hs.2.1 vp h2)]
      · exact absurd hr (by simp)
  termination_by kvs => sizeOf kvs
  theorem closed_casesPar : ∀ (cs : List SCase) (Γ : List SVar), ScopedCases Γ cs →
      ∀ r, casesPar cs Γ = some r → closedListMatchCase r = true
    | [], Γ, hs, r, hr => by
      simp only [casesPar] at hr; injection hr with h; subst h; rfl
    | SCase.mk pat body :: cs, Γ, hs, r, hr => by
      simp only [casesPar] at hr
      split at hr
      · rename_i ppar bpar rp h1 h2 h3
        rw [Option.some.injEq] at hr
        rw [← hr]
        simp only [closedListMatchCase, closed_casesPar cs Γ hs.2.2 rp h3,
          closed_matchCasePar ppar bpar 0
            (closed_normalizeAt pat nilPar Γ Closed_nil hs.1 ppar h1)
            (closed_normalizeAt body nilPar Γ Closed_nil hs.2.1 bpar h2), Bool.and_self, Bool.true_and]
      · exact absurd hr (by simp)
  termination_by cs => sizeOf cs
  theorem closed_receiptsPar : ∀ (rs : List SReceipt) (body : Par) (Γ : List SVar),
      closed body = true → ScopedReceipts Γ rs →
      ∀ r, receiptsPar rs body Γ = some r → closedListReceive r = true
    | [], body, Γ, hb, hs, r, hr => by
      simp only [receiptsPar] at hr; injection hr with h; subst h; rfl
    | rec :: rs, body, Γ, hb, hs, r, hr => by
      simp only [receiptsPar] at hr
      split at hr
      · rename_i f fs h1 h2
        rw [Option.some.injEq] at hr
        rw [← hr]
        simp only [closedListReceive, closed_receiptsPar rs body Γ hb hs.2 fs h2,
          closed_receiptPar rec body Γ hb hs.1 f h1, Bool.and_self, Bool.true_and]
      · exact absurd hr (by simp)
  termination_by rs => sizeOf rs
  theorem closed_receiptPar : ∀ (rec : SReceipt) (body : Par) (Γ : List SVar),
      closed body = true → ScopedReceipt Γ rec →
      ∀ r, receiptPar rec body Γ = some r → closedReceive r = true := by
    intro rec
    cases rec with
    | linear binds =>
      intro body Γ hb hs r hr
      simp only [receiptPar] at hr
      rw [Option.map_eq_some'] at hr
      obtain ⟨bs, hbs, hr'⟩ := hr
      rw [← hr']
      simp only [closedReceive, hb, closed_bindsPar binds Γ hs bs hbs, Bool.and_self, Bool.true_and]
    | repeated binds =>
      intro body Γ hb hs r hr
      simp only [receiptPar] at hr
      rw [Option.map_eq_some'] at hr
      obtain ⟨bs, hbs, hr'⟩ := hr
      rw [← hr']
      simp only [closedReceive, hb, closed_bindsPar binds Γ hs bs hbs, Bool.and_self, Bool.true_and]
    | peek binds => intro body Γ hb hs r hr; simp only [receiptPar] at hr; exact absurd hr (by simp)

  termination_by rec => sizeOf rec
  theorem closed_bindsPar : ∀ (bs : List SBind) (Γ : List SVar), ScopedBinds Γ bs →
      ∀ r, bindsPar bs Γ = some r → closedListReceiveBind r = true
    | [], Γ, hs, r, hr => by
      simp only [bindsPar] at hr; injection hr with h; subst h; rfl
    | SBind.mk ns rest src :: bs, Γ, hs, r, hr => by
      simp only [bindsPar] at hr
      split at hr
      · rename_i pats bs' h1 h2
        exact closed_bindResult src pats bs' Γ hs.2.2.1
          (closed_namesPar ns rest Γ hs.1 pats h1) (closed_bindsPar bs Γ hs.2.2.2 bs' h2) r hr
      · exact absurd hr (by simp)
  termination_by bs => sizeOf bs
  decreasing_by all_goals (simp_wf <;> omega)
  theorem closed_bindResult : ∀ (src : SNameSource) (pats : List Par) (bs : List ReceiveBind)
      (Γ : List SVar), ScopedSource Γ src → closedListPar pats = true →
      closedListReceiveBind bs = true →
      ∀ r, bindResult src pats bs Γ = some r → closedListReceiveBind r = true := by
    intro src
    cases src with
    | simple n =>
      intro pats bs Γ hs hp hbs r hr
      simp only [bindResult] at hr
      rw [Option.map_eq_some'] at hr
      obtain ⟨c, hc, hr'⟩ := hr
      rw [← hr']
      simp only [closedListReceiveBind, hbs, closed_receiveBindPar pats c pats.length hp (closed_namePar n Γ hs c hc),
        Bool.and_self, Bool.true_and]
    | receiveSend n => intro pats bs Γ hs hp hbs r hr; simp only [bindResult] at hr; exact absurd hr (by simp)
    | sendReceive n ds => intro pats bs Γ hs hp hbs r hr; simp only [bindResult] at hr; exact absurd hr (by simp)
  termination_by src => sizeOf src
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
  , ⟨"eval", "`PEval` (`*Name`) needs the normalizer's environment, which this function does not model"⟩
  , ⟨"select", "`PChoice`'s branches carry their own receipts; the flat `Receive` has one body each"⟩
  , ⟨"let", "`PLet` desugars through the normalizer's declaration machinery, not in one step"⟩
  , ⟨"send-synch", "`PSendSynch` has no flat constructor (the port's `proc_ast` has one, the model does not)"⟩
  , ⟨"var-ref", "`PVarRef` has no flat constructor"⟩
  , ⟨"connective", "`/\\` and `\\/` in *process* position are refused by the port itself (law 32's table)"⟩
  , ⟨"peek-receipt", "`<<-` has no flat representation: the model's `Receive` has a persistent flag and no peek one"⟩
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
def surfaceBoundaryCount : Nat := 10

/-- The boundary table carries exactly `surfaceBoundaryCount` rows. -/
theorem surfaceBoundaries_length :
    surfaceBoundaries.length = surfaceBoundaryCount := by decide

end Rchain
