import Rchain.Print

/-!
# Laws 30 and 31 — the grammar as data, and the parser's deviations as claims

`rholang_mercury.cf` is the grammar, and laws 30/31 are the two directions of the port's relation to
it: **every term the parser accepts is in the grammar** (30), and **every grammar term is accepted,
modulo a data list of documented deviations** (31). `Surface.lean` holds the grammar's *terms*
(`Surf`, one constructor per production, with `grammarProductions` naming every label and
`productionWitnesses` witnessing each one); this module holds the part a tree cannot state — the
**token-level** rules, which is where both laws' incidents actually live. A trailing separator, a
comma before a remainder, a missing `in`, trailing input: all of those are facts about *token
strings*, and they vanish the moment a term becomes a tree.

So `grammarFragment` is a production table over `Rchain/Print.lean`'s `Tok` alphabet, `derives`
decides derivability of a token list against it, and `parseDeviations` is the list of spellings where
the node and the grammar disagree — **each with the direction of the disagreement and a `decide`d
check that the direction is what the grammar says**. That last check is what makes the list a claim:
an `accepts` row must be genuinely *not* derivable, a `refuses` row must be derivable, so a row cannot
be a comment wearing a table's clothes.

**The fragment, and why it is one.** The table is the *delimited-list* productions — the sites where
a list, its separator and its remainder are spelled out — because that is exactly where law 30's
permissiveness lives (AUDIT C24 and C31, and Phase 1 item 5). An element of a list is **one opaque
`elem` token**: the grammar of the interior (the `Proc1`…`Proc16` precedence cascade, the literal
forms, comments) is *not* modelled, and `parseBoundaries` names every such gap as data rather than
leaving it to be inferred. This is the boundary discipline `Rchain/Lex.lean` used for law 32, which
checked the *operator* surface and named the rest.

**Item 5 is decided here, mechanically.** The grammar's list rule is `[X] ::= X | X "," [X]`
(`separator Proc ","`, `rholang_mercury.cf:78`): a separator stands **between** two elements, so a
one-element list takes the singleton rule and carries no separator at all, and `ProcRemainder` follows
the list with no terminal of its own (`:179`). `derives` therefore accepts `[1 ..._]` — the comma-less
form, which is how the Scala-era stock contracts are written (`legacy/casper/src/main/resources/
ListOps.rho:38`) — and rejects `[1, ..._]`, which is the deviation C31 registered and the port accepts
deliberately (`parser.rs:1108-1110`, pinned by `:1530`'s
`a_comma_before_a_remainder_is_the_deviation_the_contracts_use`). AUDIT C24's residual read the
derivation backwards before this module existed; the row here is the correction, as data.
-/

namespace Rchain

-- The tables below are reduced by `decide` in every check, and the `decide` for the corpus runs
-- `printToks` over every production witness and `matchParts` over every case, so the default budgets
-- are not enough (the same reason `Rchain/Lex.lean` raises the recursion depth for its own table).
-- Both are performance settings: the kernel still checks every reduction.
set_option maxHeartbeats 8000000
set_option maxRecDepth 100000

/-- An optional remainder, as the grammar has two of them: `ProcRemainder ::= "..." ProcVar | ""` and
`NameRemainder ::= "..." "@" ProcVar | ""`. The distinction is the `@` — a `contract`'s parameter list
and a bind's name list take the `NameRemainder`, a collection takes the `ProcRemainder`. -/
inductive Remainder where
  | proc
  | name
deriving BEq, DecidableEq, Repr

/-- One part of a production's right-hand side. The `.cf` is a sequence of terminals and of
parameterised list categories (`[Proc]`, `[Name]`, `[KeyValuePair]`), so a part is one of those — plus
`elem` for a single element in element position (`TupleMultiple ::= "(" Proc "," [Proc] ")"` has one
before its list) and `anyOf` for the one alternation the list sites use
(`Send ::= "!" | "!!"`). -/
inductive Part where
  | term (spelling : String)
  | anyOf (spellings : List String)
  | elem
  | remainder (r : Remainder)
  /-- `[X]` with its separator and the fewest elements it may have. Zero for a plain `[X]`, one for
  the grammar's `separator nonempty`, and **two** for a production that *is* `X sep X`
  (`PPar ::= Proc "|" Proc1`, the connectives): a one-element `|`-list would be a different production
  — one of the coercions — and letting it match would make any single token derivable. -/
  | list (kind : String) (sep : String) (minElems : Nat)
deriving BEq, DecidableEq, Repr

/-- One production of the fragment: the `.cf` labels it models, and its right-hand side as parts. The
labels are checked against `Surface.lean`'s `grammarProductions`, so a row cannot invent a production
the grammar does not have. -/
structure FragProduction where
  labels : List String
  parts : List Part
deriving BEq, Repr

/-- **A `[X]` element list**, from the list's first token: `elem`, then `sep elem` for each further
element, then whatever follows. The separator is consumed only when an element follows it — which is
the grammar's rule, "a separator stands between two elements", and is why `[1,]` and `[1, ..._]` are
not derivable while `[1 ..._]` is. Returns the elements consumed and the tokens left.

Structural on the token list, so `decide` can reduce it (the reason `Surface.lean`'s `normalize`
carries no `termination_by`). -/
def elemsFrom (sep : String) : List Tok → Option (Nat × List Tok)
  | [] => some (0, [])
  | .elem _ :: ts =>
    match ts with
    | .term s :: ts' =>
      if s == sep then
        match elemsFrom sep ts' with
        | some (n, ts'') => if n == 0 then none else some (n + 1, ts'')
        | none => none
      else some (1, ts)
    | _ => some (1, ts)
  | ts => some (0, ts)

/-- A list whose separator is **empty** (`separator nonempty Branch ""`: a `select`'s branches, a
`match`'s cases) is its elements juxtaposed, so every adjacent element is another element. Structural,
like `elemsFrom`. -/
def elemsJuxt : List Tok → Nat × List Tok
  | [] => (0, [])
  | .elem _ :: ts =>
    match elemsJuxt ts with
    | (n, ts') => (n + 1, ts')
  | ts => (0, ts)

/-- One list slot: the elements, with the slot's minimum enforced — fewer than `minElems` is not the
grammar's `[X]`. -/
def matchList (sep : String) (minElems : Nat) (ts : List Tok) : Option (List Tok) :=
  match sep.isEmpty, elemsFrom sep ts, elemsJuxt ts with
  | true, _, (n, ts') => if n < minElems then none else some ts'
  | false, some (n, ts'), _ => if n < minElems then none else some ts'
  | false, none, _ => none

/-- A remainder, or (for the caller) nothing: `ProcRemainder` is `...x` and `NameRemainder` is
`...@x`. -/
def matchRemainder : Remainder → List Tok → Option (List Tok)
  | .proc, .ellipsis :: .rest _ :: ts => some ts
  | .name, .ellipsis :: .term mark :: .rest _ :: ts => if mark == "@" then some ts else none
  | _, _ => none

/-- **A production's parts against a token list**, returning what is left. Structural on the parts,
which is what lets `decide` run it. -/
def matchParts : List Part → List Tok → Option (List Tok)
  | [], ts => some ts
  | .term s :: ps, ts =>
    match ts with
    | .term u :: ts' => if s == u then matchParts ps ts' else none
    | _ => none
  | .anyOf ss :: ps, ts =>
    match ts with
    | .term u :: ts' => if ss.contains u then matchParts ps ts' else none
    | _ => none
  | .elem :: ps, ts =>
    match ts with
    | .elem _ :: ts' => matchParts ps ts'
    | _ => none
  | .list _ sep minElems :: ps, ts =>
    match matchList sep minElems ts with
    | some ts' => matchParts ps ts'
    | none => none
  | .remainder r :: ps, ts =>
    match matchRemainder r ts with
    | some ts' => matchParts ps ts'
    | none => matchParts ps ts

/-- The productions of the fragment, as data. Each row is one list-bearing production of the `.cf`,
with the `.cf`'s own terminals, and the label(s) of the production it models. -/
def grammarFragment : List FragProduction :=
  [ ⟨["CollectList"], [.term "[", .list "Proc" "," 0, .remainder .proc, .term "]"]⟩
  , ⟨["CollectSet"], [.term "Set", .term "(", .list "Proc" "," 0, .remainder .proc,
      .term ")"]⟩
  , ⟨["CollectMap"], [.term "{", .list "KeyValuePair" "," 0, .remainder .proc, .term "}"]⟩
    -- `TupleSingle ::= "(" Proc ",)"` and `TupleMultiple ::= "(" Proc "," [Proc] ")"`: one element
    -- then a comma, then (for the multiple form) a list that may be empty.
  , ⟨["TupleSingle"], [.term "(", .elem, .term ",", .term ")"]⟩
  , ⟨["TupleMultiple"], [.term "(", .elem, .term ",", .list "Proc" "," 0, .term ")"]⟩
    -- `PSend ::= Name Send "(" [Proc] ")"`, and `Send ::= "!" | "!!"`.
  , ⟨["PSend"], [.elem, .anyOf ["!", "!!"], .term "(", .list "Proc" "," 0, .term ")"]⟩
    -- `PMethod ::= Proc11 "." Var "(" [Proc] ")"`.
  , ⟨["PMethod"], [.elem, .term ".", .elem, .term "(", .list "Proc" "," 0, .term ")"]⟩
    -- `PNew ::= "new" [NameDecl] "in" Proc1`, `separator nonempty NameDecl ","`.
  , ⟨["PNew"], [.term "new", .list "NameDecl" "," 1, .term "in", .elem]⟩
    -- `PContr ::= "contract" Name "(" [Name] NameRemainder ")" "=" "{" Proc "}"`.
  , ⟨["PContr"], [.term "contract", .elem, .term "(", .list "Name" "," 0, .remainder .name,
      .term ")", .term "=", .term "{", .elem, .term "}"]⟩
    -- `PInput ::= "for" "(" [Receipt] ")" "{" Proc "}"`, `separator nonempty Receipt ";"`. A receipt
    -- is one opaque element, which is what puts C31's `for (x <- c;)` site inside the fragment.
  , ⟨["PInput"], [.term "for", .term "(", .list "Receipt" ";" 1, .term ")", .term "{", .elem,
      .term "}"]⟩
    -- The connectives: `PPar ::= Proc "|" Proc1`, `PConjunction ::= Proc14 "/\\" Proc15`,
    -- `PDisjunction ::= Proc13 "\\/" Proc14`. Each is a nonempty list under its separator.
  , ⟨["PPar"], [.list "Proc" "|" 2]⟩
  , ⟨["PConjunction"], [.list "Proc" "/\\" 2]⟩
  , ⟨["PDisjunction"], [.list "Proc" "\\/" 2]⟩
    -- `Proc16`: the **atom** forms — `PGround ::= Ground`, `PVar ::= ProcVar`, `PNil ::= "Nil"`,
    -- `PSimpleType ::= SimpleType`. Each is one opaque element (they have no terminals of their own),
    -- so their row is the element slot itself: a single-element token list derives *by construction*,
    -- which is the fragment's stated abstraction (`parseBoundaries`' `element-interior` row) and not a
    -- claim about the element.
  , ⟨["PGround", "PVar", "PNil", "PSimpleType"], [.elem]⟩
    -- `PVarRef ::= VarRefKind Var` and `VarRefKind ::= "=" | "=*"`: two tokens, which is why the
    -- printer emits the kind's spelling separately from the variable.
  , ⟨["PVarRef"], [.anyOf ["=", "=*"], .elem]⟩
    -- `PNegation ::= "~" Proc15`, `PEval ::= "*" Name`, `PNot ::= "not" Proc10`,
    -- `PNeg ::= "-" Proc10`: a terminal and an element.
  , ⟨["PNegation"], [.term "~", .elem]⟩
  , ⟨["PEval"], [.term "*", .elem]⟩
  , ⟨["PNot"], [.term "not", .elem]⟩
  , ⟨["PNeg"], [.term "-", .elem]⟩
    -- `Proc9`…`Proc4`: the binary operators. Each production is `X op X` (`PMult ::= Proc9 "*" Proc10`
    -- …), i.e. a list of **two or more** under its separator — a one-element `op`-list would be one of
    -- the coercions and no operator at all, which is why the minimum is 2 rather than 1.
  , ⟨["PMult"], [.list "Proc" "*" 2]⟩
  , ⟨["PDiv"], [.list "Proc" "/" 2]⟩
  , ⟨["PMod"], [.list "Proc" "%" 2]⟩
  , ⟨["PPercentPercent"], [.list "Proc" "%%" 2]⟩
  , ⟨["PAdd"], [.list "Proc" "+" 2]⟩
  , ⟨["PMinus"], [.list "Proc" "-" 2]⟩
  , ⟨["PPlusPlus"], [.list "Proc" "++" 2]⟩
  , ⟨["PMinusMinus"], [.list "Proc" "--" 2]⟩
  , ⟨["PLt"], [.list "Proc" "<" 2]⟩
  , ⟨["PLte"], [.list "Proc" "<=" 2]⟩
  , ⟨["PGt"], [.list "Proc" ">" 2]⟩
  , ⟨["PGte"], [.list "Proc" ">=" 2]⟩
  , ⟨["PMatches"], [.list "Proc" "matches" 2]⟩
  , ⟨["PEq"], [.list "Proc" "==" 2]⟩
  , ⟨["PNeq"], [.list "Proc" "!=" 2]⟩
  , ⟨["PAnd"], [.list "Proc" "and" 2]⟩
  , ⟨["PShortAnd"], [.list "Proc" "&&" 2]⟩
  , ⟨["POr"], [.list "Proc" "or" 2]⟩
  , ⟨["PShortOr"], [.list "Proc" "||" 2]⟩
    -- `PBundle ::= Bundle "{" Proc "}"` with the four `Bundle` spellings.
  , ⟨["PBundle"], [.anyOf ["bundle+", "bundle-", "bundle0", "bundle"], .term "{", .elem,
      .term "}"]⟩
    -- `PIf` / `PIfElse`, and `PSendSynch ::= Name "!?" "(" [Proc] ")" SynchSendCont` with
    -- `EmptyCont ::= "."` and `NonEmptyCont ::= ";" Proc1` — one row each, because the continuation
    -- decides which production the spelling is.
  , ⟨["PIf"], [.term "if", .term "(", .elem, .term ")", .elem]⟩
  , ⟨["PIfElse"], [.term "if", .term "(", .elem, .term ")", .elem, .term "else", .elem]⟩
  , ⟨["PSendSynch", "EmptyCont"],
      [.elem, .term "!?", .term "(", .list "Proc" "," 0, .term ")", .term "."]⟩
  , ⟨["PSendSynch", "NonEmptyCont"],
      [.elem, .term "!?", .term "(", .list "Proc" "," 0, .term ")", .term ";", .elem]⟩
    -- `PChoice ::= "select" "{" [Branch] "}"` and `PMatch ::= "match" Proc4 "{" [Case] "}"`: their
    -- lists are `separator nonempty Branch ""` / `Case ""`, so the elements are **juxtaposed** — the
    -- engine's empty-separator case.
  , ⟨["PChoice"], [.term "select", .term "{", .list "Branch" "" 1, .term "}"]⟩
  , ⟨["PMatch"], [.term "match", .elem, .term "{", .list "Case" "" 1, .term "}"]⟩
    -- `PLet ::= "let" Decl Decls "in" "{" Proc "}"`, **one row per `Decls` production**:
    -- `EmptyDeclImpl` is no separator at all, and `LinearDeclsImpl`/`ConcDeclsImpl` put their separator
    -- *before* their list (`Decls ::= ";" [LinearDecl]`). That is why the label repeats.
  , ⟨["PLet", "EmptyDeclImpl"],
      [.term "let", .list "Name" "," 0, .remainder .name, .term "<-", .list "Proc" "," 0,
        .term "in", .term "{", .elem, .term "}"]⟩
  , ⟨["PLet", "LinearDeclsImpl"],
      [.term "let", .list "Name" "," 0, .remainder .name, .term "<-", .list "Proc" "," 0,
        .term ";", .list "Decl" ";" 1, .term "in", .term "{", .elem, .term "}"]⟩
  , ⟨["PLet", "ConcDeclsImpl"],
      [.term "let", .list "Name" "," 0, .remainder .name, .term "<-", .list "Proc" "," 0,
        .term "&", .list "Decl" "&" 1, .term "in", .term "{", .elem, .term "}"]⟩
  ]

/-- **Law 30's soundness direction, decidable**: a token list is derivable when some fragment
production consumes it *exactly*, leaving nothing — the full match is what makes a term followed by
trailing input a rejection (`parser.rs`'s `trailing_input_is_not_a_term`, AUDIT C30). -/
def derives (ts : List Tok) : Bool :=
  grammarFragment.any (fun p =>
    match matchParts p.parts ts with
    | some [] => true
    | _ => false)

/-- Is this token list one of the *derivable* spellings of the fragment? -/
def inFragment (ts : List Tok) : Bool := derives ts

/-- Each row's labels are real `.cf` productions and every row carries parts — a row for a production
nobody wrote, or an empty row, is a table that has stopped describing the grammar. `decide`d. -/
theorem fragment_rows_decide :
    (grammarFragment.all (fun p =>
        !p.labels.isEmpty && !p.parts.isEmpty && p.labels.all grammarProductions.contains)
      -- A label may repeat (`PLet` has one row per `Decls` production), so the uniqueness the table
      -- owes is over the *rows*: two rows that agree on their labels and their parts are one row
      -- written twice.
      && (grammarFragment.map (fun p => (p.labels, p.parts))).eraseDups.length
          == grammarFragment.length) = true := by
  decide

/-- **The fragment is not degenerate**: it carries a remainder-bearing production, which is the shape
laws 30/31 and item 5 are about — a fragment of lists without a remainder could not state them. -/
theorem fragment_has_a_remainder :
    (grammarFragment.any (fun p =>
      p.parts.any (fun part => match part with | .remainder _ => true | _ => false))) = true := by
  decide

/-- How many productions the fragment carries. -/
def fragmentCount : Nat := 48

/-- The table carries exactly `fragmentCount` rows. -/
theorem fragment_length : grammarFragment.length = fragmentCount := by decide

/-! ## The deviations — law 31's list, as claims

A deviation is a spelling where the node and the grammar disagree. There are two possible directions —
the **parser** accepts a spelling the grammar does not derive, or it refuses one the grammar does —
and every row states which, because `deviations_decide` checks the statement against `derives`: an
`accepts` row must be genuinely non-derivable, a `refuses` row genuinely derivable. That is what makes
the list a set of claims rather than a table of comments.

The `accepts` rows are AUDIT C31's trailing separators and comma-before-remainder sites. The
`refuses` direction has one row, and it was *found by the corpus's own Rust consumer rather than by
reading*: a `NameRemainder` in a contract's parameter list (`contract f(x ...@rest) = { Nil }`) is
derived by `PContr` and refused by the port's parameter loop (`rholang/src/parser.rs:426-431`), which
has no ellipsis arm. The direction was empty before that run, and the constructor was kept for exactly
this case — a spelling the grammar derives that `parse` refuses.

One candidate that is **not** a row here, deliberately: a process-position connective (`x /\ y`) is
refused by the **normalizer** (`rholang/src/normalizer.rs:1762`,
`RholangError::TopLevelLogicalConnectivesNotAllowedError`), and the *parser* accepts it — that is a
fact about law 34/35's layer, not laws 30/31's. -/

/-- Which way the node differs from the grammar, at the **parser**. -/
inductive Direction where
  | accepts
  | refuses
deriving BEq, DecidableEq, Repr

/-- One row of law 31's deviation list. -/
structure Deviation where
  /-- The `.cf` production the spelling is (or would be) an instance of. -/
  label : String
  /-- The direction of the disagreement. -/
  direction : Direction
  /-- The spelling, as its tokens. -/
  tokens : List Tok
  /-- Why the grammar derives it, or does not. -/
  why : String
  /-- Where the port's behaviour is pinned. -/
  rust : String
deriving Repr

/-- Law 31's list: AUDIT C31's trailing-separator sites (nine of its eleven; the tenth the port's own
test records carries a comment inside the spelling, which `parseBoundaries` names as the lexer's), the
comma-before-remainder form, and the one `refuses` row — the contract-parameter remainder the port
cannot read. -/
def parseDeviations : List Deviation :=
  [ -- A separator with nothing after it, at each list site the fragment can spell.
    ⟨"CollectList", .accepts, [.term "[", .elem "1", .term ",", .term "]"],
      "`[X] ::= X | X \",\" [X]` puts a separator *between* two elements, so a separator with no \
       element after it has no derivation", "`parser.rs:1496`'s \
       `a_trailing_separator_is_accepted_as_a_deviation` (AUDIT C31)"⟩
  , ⟨"CollectSet", .accepts, [.term "Set", .term "(", .elem "1", .term ",", .term ")"],
      "same list rule, at `CollectSet ::= \"Set\" \"(\" [Proc] ProcRemainder \")\"`",
      "`parser.rs:1499`, AUDIT C31"⟩
  , ⟨"CollectMap", .accepts, [.term "{", .elem "a : 1", .term ",", .term "}"],
      "same list rule, at `CollectMap ::= \"{\" [KeyValuePair] ProcRemainder \"}\"`",
      "`parser.rs:1500`, AUDIT C31"⟩
  , ⟨"PSend", .accepts, [.elem "c", .term "!", .term "(", .elem "1", .term ",", .term ")"],
      "same list rule, at `PSend ::= Name Send \"(\" [Proc] \")\"`", "`parser.rs:1501`, AUDIT C31"⟩
  , ⟨"PContr", .accepts,
      [.term "contract", .elem "c", .term "(", .elem "@x", .term ",", .term ")", .term "=",
        .term "{", .elem "Nil", .term "}"],
      "same list rule, at `PContr`'s `[Name]` — and the spelling the vendored contracts use \
       (`rchain-community/rgov`'s `CrowdFund.rho` ends each parameter with a comma before a comment)",
      "`parser.rs:1502`, AUDIT C31"⟩
  , ⟨"PNew", .accepts,
      [.term "new", .elem "x", .term ",", .term "in", .elem "Nil"],
      "same list rule, at `PNew`'s `separator nonempty NameDecl \",\"`",
      "`parser.rs:1503`, AUDIT C31"⟩
  , ⟨"TupleMultiple", .accepts,
      [.term "(", .elem "1", .term ",", .elem "2", .term ",", .term ")"],
      "same list rule, at `TupleMultiple ::= \"(\" Proc \",\" [Proc] \")\"` (the *single* trailing \
       comma of `(1,)` **is** derivable — `TupleSingle ::= \"(\" Proc \",)\"`)",
      "`parser.rs:1504`, AUDIT C31"⟩
  , ⟨"PMethod", .accepts,
      [.elem "a", .term ".", .elem "b", .term "(", .elem "1", .term ",", .term ")"],
      "same list rule, at `PMethod ::= Proc11 \".\" Var \"(\" [Proc] \")\"`",
      "`parser.rs:1505`, AUDIT C31"⟩
  , ⟨"PInput", .accepts,
      [.term "for", .term "(", .elem "x <- @\"c\"", .term ";", .term ")", .term "{", .elem "Nil",
        .term "}"],
      "same list rule, at `PInput`'s `separator nonempty Receipt \";\"`",
      "`parser.rs:1506`, AUDIT C31"⟩
    -- A separator followed by a *remainder*: the same shape, and the shape the vendored contracts use.
  , ⟨"CollectList", .accepts,
      [.term "[", .elem "a", .term ",", .ellipsis, .rest "rest", .term "]"],
      "the grammar separates a remainder from the list by no terminal \
       (`CollectList ::= \"[\" [Proc] ProcRemainder \"]\"`), so a comma before it is underivable — \
       and it is how `Issue.rho:110` and `Ballot.rho:106` are written (31 sites, AUDIT C31)",
      "`parser.rs:1108-1110` (the break before `parse_proc` meets the ellipsis), AUDIT C31"⟩
  , ⟨"CollectMap", .accepts,
      [.term "{", .elem "name : *voter", .term ",", .ellipsis, .rest "tail", .term "}"],
      "`{name: *voter, ...tail}` — the same row at the map site",
      "`parser.rs:1179-1181`, `Issue.rho:110`, AUDIT C31"⟩
  , ⟨"CollectSet", .accepts,
      [.term "Set", .term "(", .elem "a", .term ",", .ellipsis, .rest "rest", .term ")"],
      "`Set(a, ...rest)` — the same row at the set site", "`parser.rs:1201-1203`, AUDIT C31"⟩
    -- The other direction, and the row that closed it: a `NameRemainder` in a *contract's* parameter
    -- list is derived by the grammar and refused by the port. Found by the Rust consumer on its first
    -- run — the `refuses` direction was empty until then, which is why the constructor was kept.
  , ⟨"PContr", .refuses,
      [.term "contract", .elem "f", .term "(", .elem "x", .ellipsis, .term "@", .rest "rest",
        .term ")", .term "=", .term "{", .elem "Nil", .term "}"],
      "`PContr ::= \"contract\" Name \"(\" [Name] NameRemainder \")\" ...` derives a remainder in the parameter list, and the port's parameter loop has no ellipsis arm: it calls `parse_name` until `)`, so `...@rest` reaches `parse_name` and the parse fails `expected variable, got At`",
      "`rholang/src/parser.rs:426-431` (the parameter loop) — the finding, pinned by `rholang/tests/lean_parse_corpus.rs`"⟩
    -- And the two capability bind sources, also found by the consumer's first run: the receipt parser
    -- reads the source `Name` and then expects the receipt to end, so it has no arm for either.
  , ⟨"ReceiveSendSource", .refuses,
      [.term "for", .term "(", .elem "x <- c ?!", .term ")", .term "{", .elem "Nil", .term "}"],
      "`ReceiveSendSource ::= Name \"?!\"` derives a send-receive source, and the port's receipt parser has no arm for it: `?` lexes (`Tok::QMark`) and `!` lexes, but after the source `Name` the parser expects the receipt's `)`, so `c ?!` fails `expected RParen, got Bang`. (Its sibling `SendReceiveSource` (`Name \"!?\"`) **is** implemented — `Tok::BangQ`, read by `parse_name_source` — so only this one is a gap.)",
      "`rholang/src/parser.rs:1271-1315` (the receipt parser) — the finding, pinned by `rholang/tests/lean_parse_corpus.rs`"⟩
  ]

/-- **The deviation list is a claim, not a comment.** Every row states the direction of the
disagreement, and this decides each one against `derives`: an `accepts` row must be genuinely *not*
derivable (the node is laxer than the grammar there), a `refuses` row must be derivable (the node is
stricter). A row whose direction the grammar contradicts does not compile. -/
theorem deviations_decide :
    (parseDeviations.all (fun d =>
        !d.why.isEmpty && !d.rust.isEmpty && grammarProductions.contains d.label
          && ((d.direction == Direction.accepts && !inFragment d.tokens)
            || (d.direction == Direction.refuses && inFragment d.tokens)))
      && (parseDeviations.map Deviation.tokens).eraseDups.length
          == parseDeviations.length) = true := by
  decide

/-- The list carries no duplicate spelling — two rows for one spelling would be two claims about one
fact. (Discharged inside `deviations_decide`; named so the register can cite it.) -/
theorem deviations_distinct :
    (parseDeviations.map Deviation.tokens).eraseDups.length = parseDeviations.length := by decide

/-- How many deviations the list carries. -/
def deviationCount : Nat := 14

/-- The list carries exactly `deviationCount` rows. -/
theorem deviations_length : parseDeviations.length = deviationCount := by decide

/-- **What the node must answer for a spelling**: the grammar's verdict, unless a deviation row says
otherwise. This is the model's per-case verdict, and it is what the Rust consumer compares the node
against. -/
def nodeExpects (ts : List Tok) : Bool :=
  match parseDeviations.find? (fun d => d.tokens == ts) with
  | some d => d.direction == Direction.accepts
  | none => derives ts

/-! ## What the fragment does not model

Each row is a decision taken deliberately, named rather than left to be inferred — the form
`Surface.lean`'s `surfaceBoundaries` and `Rchain/Lex.lean`'s boundary note use. The largest of them is
the one this module was scoped around: the `Proc1`…`Proc16` cascade *inside* an element. -/

/-- A boundary of the fragment: what is outside it, and what a later pass would have to add. -/
structure ParseBoundary where
  id : String
  reason : String

/-- What `grammarFragment` deliberately does not model. -/
def parseBoundaries : List ParseBoundary :=
  [ ⟨"element-interior", "an element is one opaque `elem` token, so the `Proc1`…`Proc16` precedence \
      cascade, the operator grammar and the atom forms are **not** modelled: `derives [elem \"1 + 2\"]` \
      is false, not because the spelling is wrong but because `PAdd` is not a row. Modelling them is a \
      row per production — cheap once this engine exists, and deliberately not done in this pass, \
      which scoped itself to the list sites where laws 30/31's incidents live (AUDIT C24, C31)"⟩
  , ⟨"literal-forms", "`LongLiteral`, `StringLiteral`, `UriLiteral` and the `_`/`_ident` identifier \
      rule are the lexer's (law 32), and an element's spelling is not re-lexed here"⟩
  , ⟨"comments", "`//` and `/* */` are stripped by the lexer, so a case's tokens never carry one — \
      AUDIT C31's `contract c(@x, // a comment \\n @y,)` spelling is therefore in the deviation list's \
      prose rather than as a corpus case"⟩
  , ⟨"empty-separator-lists", "`separator nonempty Branch \"\"` (a `select`'s branches) and the `Case` \
      list juxtapose their elements with *no* separator, which this engine's `sep` cannot express; \
      `PChoice` and `PMatch` are therefore outside the fragment"⟩
  , ⟨"nested-list-sites", "`PLet`'s `Decls ::= \";\" [LinearDecl] | \"&\" [ConcDecl] | ε` puts its \
      separator *before* the list, and a bind's `[Name]` sits inside a receipt; the fragment's parts \
      are flat, so those sites are reached through their enclosing list slot (`PInput`'s `[Receipt]`) \
      rather than modelled themselves"⟩
  , ⟨"group", "`PExprs ::= \"(\" Proc4 \")\"` contributes no node — the AST collapses it and the port \
      asserts `parse(\"(3 + 5)\") = parse(\"3 + 5\")` (`parser.rs:772-780`) — so a group is not a token \
      here: `Print.lean` re-inserts parentheses inside an element's spelling, and `derives` never sees \
      one"⟩
  ]

/-- The boundary rows are non-empty and distinct. -/
theorem parseBoundaries_decide :
    (parseBoundaries.all (fun b => !b.id.isEmpty && !b.reason.isEmpty)
      && (parseBoundaries.map ParseBoundary.id).eraseDups.length == parseBoundaries.length) = true := by
  decide

/-- How many boundaries the fragment names. -/
def parseBoundaryCount : Nat := 6

/-- The table carries exactly `parseBoundaryCount` rows. -/
theorem parseBoundaries_length : parseBoundaries.length = parseBoundaryCount := by decide

/-! ## The corpus

One case is a spelling and what the node must do with it — `accept` or `reject`. Two halves, one
source of truth each: the model's verdict is `decide`d here, and the Rust consumer
(`rholang/tests/lean_parse_corpus.rs`) runs the node's parser on `renderTokens c.tokens` and compares.
The cases come from four places, all of them checked: the derivable spellings, the spellings the
grammar refuses outright, the deviation list, and the **printer's output for every witness the
fragment can read** — that last one is law 31's completeness direction, tied to law 33's printer
instead of hand-written. -/

/-- Where a case came from. The corpus carries this because the two halves are *different claims*:
`derivable`/`refused` are the grammar's verdict (law 30's soundness direction is the `refused` rows),
`deviation` is a row of law 31's list, and `printer` is law 31's completeness direction tied to law
33's printer — and because law 33's round trip is only meaningful for rows whose source the fragment
can read. -/
inductive Kind where
  | derivable
  | refused
  | deviation
  | printer
deriving BEq, DecidableEq, Repr

/-- The name the corpus line carries for a kind. -/
def Kind.tag : Kind → String
  | .derivable => "derivable"
  | .refused => "refused"
  | .deviation => "deviation"
  | .printer => "printer"

/-- One corpus case: the tokens, what the node must answer, and which half of the layer it belongs
to. -/
structure ParseCase where
  tokens : List Tok
  expect : Bool
  kind : Kind
deriving BEq, Repr

/-- A case holds when the model's verdict for its tokens is the verdict the row states — so a
hand-written row whose expectation disagrees with the grammar and the deviation list does not
compile. -/
def parseHolds (c : ParseCase) : Bool := nodeExpects c.tokens == c.expect

/-- A terminal token, for writing cases readably. -/
def tk (s : String) : Tok := .term s

/-- An element token. -/
def tkE (s : String) : Tok := .elem s

/-- A `ProcRemainder` (`...x`). -/
def tkRem (x : String) : List Tok := [.ellipsis, .rest x]

/-- A `NameRemainder` (`...@x`). -/
def tkNameRem (x : String) : List Tok := [.ellipsis, .term "@", .rest x]

/-- The spellings the grammar derives, including the ones item 5 is about: `[1 ..._]` is derivable
(and is how the Scala-era stock contracts are written) beside `[1, ..._]`, which is not. -/
def derivableCases : List ParseCase :=
  [ ⟨[tk "[", tkE "1"] ++ tkRem "_" ++ [tk "]"], true, .derivable⟩
  , ⟨[tk "[", tk "]"], true, .derivable⟩
  , ⟨[tk "[", tkE "1", tk "]"], true, .derivable⟩
  , ⟨[tk "[", tkE "1", tk ",", tkE "2", tk "]"], true, .derivable⟩
  , ⟨[tk "[", .ellipsis, .rest "rest", tk "]"], true, .derivable⟩
  , ⟨[tk "Set", tk "(", .ellipsis, .rest "_", tk ")"], true, .derivable⟩
  , ⟨[tk "{", .ellipsis, .rest "_", tk "}"], true, .derivable⟩
  , ⟨[tk "{", tkE "a : 1", tk "}"], true, .derivable⟩
  , ⟨[tk "(", tkE "1", tk ",", tk ")"], true, .derivable⟩
  , ⟨[tk "(", tkE "1", tk ",", tkE "2", tk ")"], true, .derivable⟩
  , ⟨[tkE "c", tk "!", tk "(", tkE "1", tk ")"], true, .derivable⟩
  , ⟨[tkE "c", tk "!!", tk "(", tkE "1", tk ")"], true, .derivable⟩
  , ⟨[tk "new", tkE "x", tk "in", tkE "Nil"], true, .derivable⟩
  , ⟨[tk "for", tk "(", tkE "x <- c", tk ")", tk "{", tkE "Nil", tk "}"], true, .derivable⟩
  , ⟨[tkE "x", tk "|", tkE "y"], true, .derivable⟩
    -- a multi-element list, so a table that only matched singletons fails here
  , ⟨[tk "[", tkE "1", tk ",", tkE "2", tk ",", tkE "3", tk "]"], true, .derivable⟩
    -- The bind/param remainder is comma-less too: `[Name] NameRemainder`, no terminal between them.
    -- **Its verdict is `reject`, not `accept`**, and that is the deviation list at work: the grammar
    -- derives the spelling and the port's parameter loop refuses it (`parseDeviations`'s `PContr`
    -- `refuses` row) — the row is here, in the derivable half, precisely so the qualification is
    -- visible beside the derive.
  , ⟨[tk "contract", tkE "f", tk "(", tkE "x"] ++ tkNameRem "rest" ++
      [tk ")", tk "=", tk "{", tkE "Nil", tk "}"], false, .derivable⟩
  ]

/-- The spellings the grammar refuses and the node refuses with it: two elements with no separator, a
term where a separator's element should be, and trailing input (AUDIT C30). These are law 30's
*soundness* direction made falsifiable — a port that grew more permissive fails one of them. -/
def refusedCases : List ParseCase :=
  [ ⟨[tk "[", tkE "1", tkE "2", tk "]"], false, .refused⟩
  , ⟨[tk "(", tkE "1", tkE "2", tk ")"], false, .refused⟩
  , ⟨[tk "[", tkE "1", tk ",", tkE "2", tkE "3", tk "]"], false, .refused⟩
  , ⟨[tk "{", tkE "a : 1", tkE "b : 2", tk "}"], false, .refused⟩
  , ⟨[tk "Set", tk "(", tkE "1", tkE "2", tk ")"], false, .refused⟩
  , ⟨[tk "(", tkE "1", tk ",", tkE "2", tkE "3", tk ")"], false, .refused⟩
  , ⟨[tkE "a", tk ".", tkE "b", tk "(", tkE "1", tkE "2", tk ")"], false, .refused⟩
  , ⟨[tkE "c", tk "!", tk "(", tkE "1", tkE "2", tk ")"], false, .refused⟩
  , ⟨[tkE "x", tk "|"], false, .refused⟩
  , ⟨[tkE "a", tk ".", tkE "b"], false, .refused⟩
  , ⟨[tk "new", tkE "x", tk "in"], false, .refused⟩
  ]

/-- The deviation list, as cases: each row's spelling with the verdict its direction implies. -/
def deviationCases : List ParseCase :=
  parseDeviations.map (fun d => ⟨d.tokens, nodeExpects d.tokens, .deviation⟩)

/-- **Law 31's completeness direction, tied to law 33's printer**: every witness whose printed tokens
the fragment can read is a case the node must accept — unless it is a deviation, which is how the
process-position connectives come out as `reject` without a hand-written row. A witness the fragment
cannot read is a `parseBoundaries` row rather than a case. -/
def printerCases : List ParseCase :=
  (productionWitnesses.filter (fun w => derives (printToks w.term))).map
    (fun w => ⟨printToks w.term, nodeExpects (printToks w.term), .printer⟩)

/-- The corpus. -/
def parseCases : List ParseCase :=
  derivableCases ++ refusedCases ++ deviationCases ++ printerCases

/-- **Every case holds of the model** — the `decide`d half of every layer. The Rust consumer is the
other half: it parses the rendered source and must get `expect`. -/
theorem parseCases_decide : parseCases.all parseHolds = true := by decide

/-- **A spelling cannot be both accepted and rejected.** Two rows for one spelling with different
expectations would be a corpus contradicting itself, and it would fail the Rust consumer in whichever
direction it read last — so it is a compile failure instead. -/
theorem parseCases_consistent :
    parseCases.all (fun c =>
      (parseCases.filter (fun c' => c'.tokens == c.tokens)).all (fun c' => c'.expect == c.expect))
      = true := by decide

/-- **The layer is not degenerate**: both verdicts appear, so a corpus that answered one of them
everywhere could not pass — the non-vacuity ratchet, as in `Corpus.sortCases_verdicts`. -/
theorem parseCases_both_ways :
    (parseCases.any (fun c => c.expect) && parseCases.any (fun c => !c.expect)) = true := by decide

/-- How many cases the derivable half of the layer carries. -/
def derivableCaseCount : Nat := 17

/-- How many cases the refused half carries (law 30's soundness direction). -/
def refusedCaseCount : Nat := 11

/-- How many cases the deviation half carries (law 31's list). -/
def deviationCaseCount : Nat := 14

/-- How many cases the printer half carries (law 31's completeness direction): one per production
witness whose printed tokens the fragment reads — which is now every witness but two, the ones this
printer spells as the port's warts (`(1,)` ⇒ `(1)` and `not x` ⇒ `~(x)`), plus `[1]` twice because
`Surface.lean`'s table witnesses `CollectList`/`ProcRemainderEmpty` with the same term. -/
def printerCaseCount : Nat := 79

/-- How many cases the layer carries, as the sum of its four halves. -/
def parseCaseCount : Nat :=
  derivableCaseCount + refusedCaseCount + deviationCaseCount + printerCaseCount

/-- Each half carries exactly its count. `decide`d, so a half that silently shrinks fails the
build — the corpus's own version of the ratchet the Rust consumer applies to the total. -/
theorem parseCases_by_kind :
    (derivableCases.length == derivableCaseCount
      && refusedCases.length == refusedCaseCount
      && deviationCases.length == deviationCaseCount
      && printerCases.length == printerCaseCount) = true := by
  decide

/-- **The two printer warts, decided**: the printer's output for a one-element tuple and for `not x`
are not terms of the grammar at all, which is what makes them unreadable rather than merely
mis-spelled — the model's half of the port's own
`the_documented_warts_print_what_the_grammar_cannot_read_back`. -/
theorem warts_are_not_derivable :
    (derives (printToks (Surf.collect (.tuple (.ground (.int "1")) []))) == false
      && derives (printToks (Surf.not (.var "x"))) == false) = true := by
  decide

/-- The layer carries exactly `parseCaseCount` cases. -/
theorem parseCases_length : parseCases.length = parseCaseCount := by decide


