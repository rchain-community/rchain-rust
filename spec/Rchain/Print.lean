import Rchain.Surface

/-!
# Laws 31 and 33 — the printer, and the alphabet the grammar is stated over

`Surface.lean` says of its witness table that its terms "are written in the *printer's* spelling
(`Rchain/Print.lean`)" — this is that printer, and `Rchain/Parse.lean` is the grammar it is read
against. Between them they carry law 31's two directions (every grammar term is accepted, modulo a data
list of deviations; every term the parser accepts is in the grammar, modulo the same list) and law 33's
round trip (`parse (print p) ≡ p`, the C13 regression).

**What a printer can be wrong about, and which part this module pins.** C13's two defects were a
*doubled group separator* (`a |\n |\nb`, unparsable) and a *dropped keyword* (`0 { … }` where the
grammar writes `bundle0 { … }`). Both are defects of the **token sequence**, not of the layout, and
that is the line this module draws:

- **printed here**: the token sequence, as data (`printToks`), and its spelling (`printSurf`). The
  `ProcN ::= ProcN+1` coercions are re-inserted as *parentheses*, because `Surf` collapses them and a
  group contributes no node, so `mult (add 1 2) 3` must print `(1 + 2) * 3`; printing it flat is how a
  printer silently changes what a term means.
- **not printed here**, and named as data in `printBoundaries`: `rholang/src/pretty_printer.rs`'s
  *layout* — its newlines and indentation, and the eight-column `bundle` padding that Scala's
  `BundleOps.showInstance` gets from `"%-8s".format`. Law 33 is a statement about tokens; a re-parser
  does not read layout, and pinning the padding would pin a format string rather than a language.

**Two printers, tied by source text.** The port's printer takes the **core `Par`**, not this surface
type: `let`, `if`, `select`, `contract` and synchronous send are desugared by the normalizer first
(`normalizer.rs:212-222`, `:782-851`, `:1096-1123`), so `pretty_printer.rs` has no rule for them and
its parentheses are *not* precedence-driven (every binary expression is wrapped). So this printer and
the port's are not compared output-for-output; they are tied the way every layer is — by the source text
the corpus carries, which the Rust consumer feeds to the node. Law 33's *identity* half is therefore a
statement about the node and is the Rust consumer's; the model's half is that the printer's output is a
term of the grammar (`Parse.lean`'s `derives` on `printToks`), which is the decidable part of the same
claim.

**`Tok` is the shared alphabet.** The printer emits it and `Parse.lean`'s fragment consumes it, so it
lives with the printer (the module that must produce it). An `elem` is **one opaque element** — a term,
a name, a pattern, a receipt — carrying its own spelling: the fragment's grammar is stated over tokens
with the elements' interiors abstracted, which is `Parse.lean`'s boundary rather than a claim made here.

**Three warts belong to the port's printer** (`printWarts`), and this printer deliberately does *not*
reproduce them: the port prints a *one-element tuple* as a group (`(1,)` ⇒ `(1)`, the integer), `not x`
as `~(x)` (the negation), and a `new` with a urn without it. The port's own
`the_documented_warts_print_what_the_grammar_cannot_read_back` pins the first two; this layer's Rust
consumer found the third. A corpus row has to be a *grammar* term for law 31's completeness to be a
claim about the port, so this printer emits `(1,)`, `not x` and the urn — and the consumer's round trip
asserts each loss with its own detector.
-/

namespace Rchain

-- This module's declarations are one 45-constructor printer, its sub-printers, and the tables their
-- checks reduce over. Lean's default budget of 200 000 heartbeats is not enough for the `decide`d
-- checks (measured in `Surface.lean`, which holds the same kind of table); it is a performance setting,
-- so the kernel still checks everything here.
set_option maxHeartbeats 8000000
-- The `decide`d table checks below reduce `eraseDups`/`contains` over `List String`, whose equality
-- walks the character lists — the same reason `Rchain/Lex.lean` raises this for its own table. It is a
-- performance setting: the kernel still checks every reduction.
set_option maxRecDepth 10000

/-- One token of the surface, as the printer emits it and `Rchain/Parse.lean`'s grammar reads it.

- `term` — a terminal the grammar spells: a delimiter, a keyword, a separator, an operator. The spelling
  is the datum, so a production's parts are the `.cf`'s own quotation of it.
- `ellipsis` — the `"..."` that opens a remainder. Its own constructor because every remainder begins
  with it and the fragment's matcher keys on it.
- `elem` — **one opaque element**, carrying the spelling of whatever it is (a `Proc`, a `Name`, a
  `KeyValuePair`, a `Receipt`, a `Branch`). The fragment does not look inside.
- `rest` — the variable a remainder names (`...x`), again opaque. The `@` of a `NameRemainder`
  (`...@x`) is a separate `term`, because the grammar puts it there. -/
inductive Tok where
  | term (spelling : String)
  | ellipsis
  | elem (spelling : String)
  | rest (name : String)
deriving BEq, DecidableEq, Repr

/-- The characters of a token. -/
def renderTok : Tok → String
  | .term s => s
  | .ellipsis => "..."
  | .elem s => s
  | .rest s => s

/-- The terminals that take **no space before** them: a closer, an opener, a separator, and the
punctuation that binds to what precedes it (a field access, a remainder's `@`, a send's bang). `(` is
here rather than after the element that precedes it because the port writes every argument list tight
(`c!(1)`, `a.b(1)`, `contract @"c"(@x)`), which is what its own printer emits. -/
def tightBefore : Tok → Bool
  | .term s =>
    s == "]" || s == ")" || s == "}" || s == "(" || s == "," || s == ";" || s == "." || s == "@"
      || s == "!" || s == "!!" || s == "!?"
  | _ => false

/-- The terminals that take **no space after** them: an opener (and `Set`, which the grammar always
follows with its parenthesis), and the operators that bind to their operand. -/
def tightAfter : Tok → Bool
  | .term s =>
    s == "[" || s == "(" || s == "{" || s == "Set" || s == "." || s == "@" || s == "!"
      || s == "!!" || s == "!?" || s == "*" || s == "~" || s == "-" || s == "=" || s == "=*"
  | _ => false

/-- **Is there a space between two adjacent tokens?** Rholang is whitespace-insensitive, so this is not
a property of the language: it is the canonical spelling a corpus row is *rendered* from, and
`Parse.lean` checks each case's tokens render to that case's source, so a row's two halves cannot drift
apart. The rules are the ones this vocabulary needs: tight against brackets and separators, a space
before a remainder's ellipsis and none after it (so `[1 ..._]` and `{..._}` both come out the way the
grammar writes them), and a space everywhere else. -/
def spaced : Tok → Tok → Bool
  | _, .rest _ => false
  | t, .ellipsis => !tightAfter t
  | .ellipsis, _ => false
  | t, .term b => !(tightBefore (.term b) || tightAfter t)
  | t, _ => !tightAfter t

/-- The space to emit between two adjacent tokens: the characters, not the decision. -/
def spaceBetween (t u : Tok) : String := if spaced t u then " " else ""

/-- **The spelling of a token list.** Structural, so `decide` sees through it: `Parse.lean`'s corpus
rows are rendered from their token lists by this function, which is what stops a row's source from
disagreeing with the tokens its verdict was decided from. -/
def renderTokens : List Tok → String
  | [] => ""
  | [t] => renderTok t
  | t :: u :: ts => renderTok t ++ spaceBetween t u ++ renderTokens (u :: ts)

/-- The precedence of a term, on the `.cf`'s own scale: **larger binds tighter**. `par` is `Proc` (1)
and an atom is `Proc16` (17); the arithmetic and connective levels are `Proc9`…`Proc4`, and the atom
forms are all one level. A term in a position requiring level `p` is parenthesised when its own level is
lower — the group production, `PExprs ::= "(" Proc4 ")"`. -/
def prec : Surf → Nat
  | .par _ _ => 1
  | .ifThen _ _ | .ifElse _ _ _ | .newIn _ _ | .sendSynch _ _ _ => 2
  | .contr _ _ _ _ | .input _ _ | .choice _ | .match _ _ | .bundle _ _ | .letIn _ _ _ => 3
  | .send _ _ _ => 4
  | .or _ _ | .shortOr _ _ => 5
  | .and _ _ | .shortAnd _ _ => 6
  | .matches _ _ | .eq _ _ | .neq _ _ => 7
  | .lt _ _ | .lte _ _ | .gt _ _ | .gte _ _ => 8
  | .add _ _ | .sub _ _ | .plusPlus _ _ | .minusMinus _ _ => 9
  | .mult _ _ | .div _ _ | .mod _ _ | .pctPct _ _ => 10
  | .not _ | .negNum _ => 11
  | .method _ _ _ => 12
  | .eval _ => 13
  | .disj _ _ | .varRef _ _ => 14
  | .conj _ _ => 15
  | .neg _ => 16
  | _ => 17


/-- **A term's spelling at a position requiring level `p`**: parenthesised when the term binds looser,
which is where the printer re-inserts the group production the AST collapses (`PExprs ::= "(" Proc4
")"`). It takes the spelling rather than printing, and that is deliberate: a helper that printed would
call the printer on its own argument, and Lean's structural checker — correctly — refuses a mutual
recursion whose call is not on a *subterm*. Callers hold the term as a field of the constructor they
are printing, so the recursion stays structural. -/
def parenAt (t : Surf) (p : Nat) (spelling : String) : String :=
  if prec t < p then "(" ++ spelling ++ ")" else spelling

/-- One element, as a token: the same, wrapped. Every element slot of every production goes through
one of these two, at level 0 (no parentheses — a list element binds looser than nothing). -/
def elemOf (t : Surf) (p : Nat) (spelling : String) : Tok := .elem (parenAt t p spelling)

/-- The spelling of a ground. `int`/`bigint` carry their digits and `str`/`uri` their delimiters —
`Surface.lean`'s `SGround` records that these are the *raw literals*, as the port's `proc_ast.rs` does —
so this is the identity on all four. -/
def groundSpelling : SGround → String
  | .bool b => if b then "true" else "false"
  | .bigint d => "BigInt(" ++ d ++ ")"
  | .int d => d
  | .str raw => raw
  | .uri raw => raw

/-- A simple type's keyword (`SimpleTypeBool ::= "Bool"`, …). -/
def typeSpelling : STy → String
  | .bool => "Bool"
  | .int => "Int"
  | .bigInt => "BigInt"
  | .str => "String"
  | .uri => "Uri"
  | .byteArray => "ByteArray"

/-- A bundle's keyword. C13 item 2 was this keyword dropped from the output, which is why it is data
here rather than a string built at the print site. -/
def bundleSpelling : SBundle → String
  | .write => "bundle+"
  | .read => "bundle-"
  | .equiv => "bundle0"
  | .readWrite => "bundle"

-- The recursion below is structural on the *term*, so every member takes its term **first** and the
-- remaining arguments after it, and no member returns a closure — the shape `Surface.lean`'s
-- `normalize` has, and for its two reasons: Lean's mutual structural recursion looks for a term to
-- eliminate on, and a closure would not reduce, so `decide` could not see through it.
/-- A remainder (`ProcRemainder ::= "..." ProcVar | ""`). -/
def remToks : Option SVar → List Tok
  | none => []
  | some x => [.ellipsis, .rest x]

/-- A `NameRemainder` (`NameRemainderVar ::= "..." "@" ProcVar | ""`) — the `@` is the grammar's. -/
def nameRemToks : Option SVar → List Tok
  | none => []
  | some x => [.ellipsis, .term "@", .rest x]

mutual

/-- A `Name`'s spelling, for the positions that take one (a send's channel, a contract's name, a bind's
source). A `Name` is opaque to the fragment, so this yields a `String` for an `elem` to carry; a quoted
name spells its term, which is why it is in this block. -/
def nameSpelling : SName → String
  | .wild => "_"
  | .var x => x
  | .quote p => "@" ++ parenAt p 12 (renderTokens (printToksAt p 0))

/-- **The printer.** A production emits its own terminals and its element slots as `elem` tokens, so the
token sequence mirrors the grammar's shape: the list-bearing productions come out as their skeleton with
opaque elements, which is exactly what `Parse.lean`'s fragment reads back. -/
def printToksAt : Surf → Nat → List Tok
  | .ground g, _ => [.elem (groundSpelling g)]
  | .collect c, _ => collectToks c
  | .var x, _ => [.elem x]
  | .varWild, _ => [.elem "_"]
  | .varRef k x, _ => [.term (if k == SVarRefKind.proc then "=" else "=*"), .elem x]
  | .nil, _ => [.elem "Nil"]
  | .simpleType ty, _ => [.elem (typeSpelling ty)]
  | .neg p, _ => [.term "~", elemOf p 16 (renderTokens (printToksAt p 0))]
  | .conj a b, _ => [elemOf a 15 (renderTokens (printToksAt a 0)), .term "/\\", elemOf b 16 (renderTokens (printToksAt b 0))]
  | .disj a b, _ => [elemOf a 14 (renderTokens (printToksAt a 0)), .term "\\/", elemOf b 15 (renderTokens (printToksAt b 0))]
  | .eval n, _ => [.term "*", .elem (nameSpelling n)]
  -- The port prints `PNot` as `~(x)` — the *negation* spelling (`pretty_printer.rs`'s ENot,
  -- `PrettyPrinter.scala:75`), which is one of `printWarts`' rows and is asserted by the consumer's
  -- round trip. This printer emits the grammar's own spelling (`PNot ::= "not" Proc10`), because a
  -- corpus row has to be a grammar term for law 31's completeness to be a claim about the port.
  | .not p, _ => [.term "not", elemOf p 10 (renderTokens (printToksAt p 0))]
  | .negNum p, _ => [.term "-", elemOf p 10 (renderTokens (printToksAt p 0))]
  | .mult a b, _ => [elemOf a 10 (renderTokens (printToksAt a 0)), .term "*", elemOf b 11 (renderTokens (printToksAt b 0))]
  | .div a b, _ => [elemOf a 10 (renderTokens (printToksAt a 0)), .term "/", elemOf b 11 (renderTokens (printToksAt b 0))]
  | .mod a b, _ => [elemOf a 10 (renderTokens (printToksAt a 0)), .term "%", elemOf b 11 (renderTokens (printToksAt b 0))]
  | .pctPct a b, _ => [elemOf a 10 (renderTokens (printToksAt a 0)), .term "%%", elemOf b 11 (renderTokens (printToksAt b 0))]
  | .add a b, _ => [elemOf a 9 (renderTokens (printToksAt a 0)), .term "+", elemOf b 10 (renderTokens (printToksAt b 0))]
  | .sub a b, _ => [elemOf a 9 (renderTokens (printToksAt a 0)), .term "-", elemOf b 10 (renderTokens (printToksAt b 0))]
  | .plusPlus a b, _ => [elemOf a 9 (renderTokens (printToksAt a 0)), .term "++", elemOf b 10 (renderTokens (printToksAt b 0))]
  | .minusMinus a b, _ => [elemOf a 9 (renderTokens (printToksAt a 0)), .term "--", elemOf b 10 (renderTokens (printToksAt b 0))]
  | .lt a b, _ => [elemOf a 8 (renderTokens (printToksAt a 0)), .term "<", elemOf b 9 (renderTokens (printToksAt b 0))]
  | .lte a b, _ => [elemOf a 8 (renderTokens (printToksAt a 0)), .term "<=", elemOf b 9 (renderTokens (printToksAt b 0))]
  | .gt a b, _ => [elemOf a 8 (renderTokens (printToksAt a 0)), .term ">", elemOf b 9 (renderTokens (printToksAt b 0))]
  | .gte a b, _ => [elemOf a 8 (renderTokens (printToksAt a 0)), .term ">=", elemOf b 9 (renderTokens (printToksAt b 0))]
  | .matches a b, _ => [elemOf a 8 (renderTokens (printToksAt a 0)), .term "matches", elemOf b 8 (renderTokens (printToksAt b 0))]
  | .eq a b, _ => [elemOf a 7 (renderTokens (printToksAt a 0)), .term "==", elemOf b 8 (renderTokens (printToksAt b 0))]
  | .neq a b, _ => [elemOf a 7 (renderTokens (printToksAt a 0)), .term "!=", elemOf b 8 (renderTokens (printToksAt b 0))]
  | .and a b, _ => [elemOf a 6 (renderTokens (printToksAt a 0)), .term "and", elemOf b 7 (renderTokens (printToksAt b 0))]
  | .shortAnd a b, _ => [elemOf a 6 (renderTokens (printToksAt a 0)), .term "&&", elemOf b 7 (renderTokens (printToksAt b 0))]
  | .or a b, _ => [elemOf a 5 (renderTokens (printToksAt a 0)), .term "or", elemOf b 6 (renderTokens (printToksAt b 0))]
  | .shortOr a b, _ => [elemOf a 5 (renderTokens (printToksAt a 0)), .term "||", elemOf b 6 (renderTokens (printToksAt b 0))]
  | .method t v args, _ => [elemOf t 12 (renderTokens (printToksAt t 0)), .term ".", .elem v, .term "("] ++ elemsToks args ++ [.term ")"]
  | .send n persistent args, _ =>
      [.elem (nameSpelling n), .term (if persistent then "!!" else "!"), .term "("]
        ++ elemsToks args ++ [.term ")"]
  | .contr n params rem body, _ =>
      [.term "contract", .elem (nameSpelling n), .term "("]
        ++ nameElemsToks params ++ nameRemToks rem
        ++ [.term ")", .term "=", .term "{", elemOf body 0 (renderTokens (printToksAt body 0)), .term "}"]
  | .input receipts body, _ =>
      [.term "for", .term "("] ++ receiptToks receipts
        ++ [.term ")", .term "{", elemOf body 0 (renderTokens (printToksAt body 0)), .term "}"]
  | .choice branches, _ => [.term "select", .term "{"] ++ branchToks branches ++ [.term "}"]
  | .match target cases, _ =>
      [.term "match", elemOf target 0 (renderTokens (printToksAt target 0)), .term "{"] ++ caseToks cases ++ [.term "}"]
  | .bundle b body, _ => [.term (bundleSpelling b), .term "{", elemOf body 0 (renderTokens (printToksAt body 0)), .term "}"]
  | .letIn d ds body, _ =>
      [.term "let"] ++ declToks d ++ declsToks ds
        ++ [.term "in", .term "{", elemOf body 0 (renderTokens (printToksAt body 0)), .term "}"]
  | .ifThen c t, _ => [.term "if", .term "(", elemOf c 1 (renderTokens (printToksAt c 0)), .term ")", elemOf t 2 (renderTokens (printToksAt t 0))]
  | .ifElse c t e, _ =>
      [.term "if", .term "(", elemOf c 1 (renderTokens (printToksAt c 0)), .term ")", elemOf t 2 (renderTokens (printToksAt t 0)), .term "else", elemOf e 2 (renderTokens (printToksAt e 0))]
  | .newIn decls body, _ => [.term "new"] ++ nameDeclToks decls ++ [.term "in", elemOf body 2 (renderTokens (printToksAt body 0))]
  | .sendSynch n args cont, _ =>
      [.elem (nameSpelling n), .term "!?", .term "("] ++ elemsToks args ++ [.term ")"]
        ++ (match cont with
            | .empty => [.term "."]
            | .nonEmpty body => [.term ";", elemOf body 2 (renderTokens (printToksAt body 0))])
  | .par a b, _ => [elemOf a 1 (renderTokens (printToksAt a 0)), .term "|", elemOf b 2 (renderTokens (printToksAt b 0))]

/-- A `,`-separated element list (a collection's elements, a send's data, a declaration's values): each
element one opaque token. The empty list is empty — the slot's own delimiters belong to the
production. -/
def elemsToks : List Surf → List Tok
  | [] => []
  | [p] => [elemOf p 0 (renderTokens (printToksAt p 0))]
  | p :: q :: ps => elemOf p 0 (renderTokens (printToksAt p 0)) :: .term "," :: elemsToks (q :: ps)

/-- A `,`-separated list of *names* (a contract's parameters, a bind's patterns) — the same shape as
`elemsToks`, over `SName`. -/
def nameElemsToks : List SName → List Tok
  | [] => []
  | [n] => [.elem (nameSpelling n)]
  | n :: m :: ns => .elem (nameSpelling n) :: .term "," :: nameElemsToks (m :: ns)

/-- A collection: the four forms, each with its delimiters and its optional remainder. A *one-element
tuple* prints its trailing comma (`TupleSingle ::= "(" Proc ",)"`); the port's printer drops it
(`pretty_printer.rs`'s ETuple, `PrettyPrinter.scala:117`), which is `printWarts`' first row and is
asserted by the consumer's round trip. -/
def collectToks : SCollect → List Tok
  | .list ps rem => [.term "["] ++ elemsToks ps ++ remToks rem ++ [.term "]"]
  | .set ps rem => [.term "Set", .term "("] ++ elemsToks ps ++ remToks rem ++ [.term ")"]
  | .map kvs rem => [.term "{"] ++ kvToks kvs ++ remToks rem ++ [.term "}"]
  | .tuple first rest =>
      match rest with
      -- `TupleSingle ::= "(" Proc ",)"` — the trailing comma is the grammar's, and the port's printer
      -- drops it (`printWarts`' first row, asserted by the consumer's round trip).
      | [] => [.term "(", elemOf first 0 (renderTokens (printToksAt first 0)), .term ",", .term ")"]
      | _ => [.term "(", elemOf first 0 (renderTokens (printToksAt first 0)), .term ","] ++ elemsToks rest ++ [.term ")"]

/-- One map pair, as an element. The pair's own spelling (`k : v`, with spaces) is the port's
(`pretty_printer.rs`'s EMap); the pair is opaque, so it is one element. -/
def pairTok : SKeyValuePair → Tok
  | ⟨k, v⟩ => .elem (renderTokens (printToksAt k 0) ++ " : " ++ renderTokens (printToksAt v 0))

/-- A map's pairs. -/
def kvToks : List SKeyValuePair → List Tok
  | [] => []
  | [⟨k, v⟩] => [pairTok ⟨k, v⟩]
  | ⟨k, v⟩ :: ⟨k', v'⟩ :: rest => pairTok ⟨k, v⟩ :: .term "," :: kvToks (⟨k', v'⟩ :: rest)

/-- A receipt list (`separator nonempty Receipt ";"`): each receipt is one opaque element. -/
def receiptToks : List SReceipt → List Tok
  | [] => []
  | [r] => [.elem (renderTokens (bindToks r))]
  | r :: s :: rs => .elem (renderTokens (bindToks r)) :: .term ";" :: receiptToks (s :: rs)

/-- A receipt's binds, as the tokens that spell them inside the receipt's opaque element. -/
def bindToks : SReceipt → List Tok
  | .linear bs => bindListToks bs "<-"
  | .repeated bs => bindListToks bs "<="
  | .peek bs => bindListToks bs "<<-"

/-- A `&`-separated bind list. -/
def bindListToks : List SBind → String → List Tok
  | [], _ => []
  | [b], arrow => bindOneToks b arrow
  | b :: c :: bs, arrow =>
      bindOneToks b arrow ++ [.term "&"] ++ bindListToks (c :: bs) arrow

/-- One bind: its names, its remainder, its arrow, and its source. -/
def bindOneToks : SBind → String → List Tok
  | ⟨ns, rem, src⟩, arrow =>
    nameElemsToks ns ++ nameRemToks rem ++ [.term arrow] ++
      (match src with
       | .simple n => [.elem (nameSpelling n)]
       | .receiveSend n => [.elem (nameSpelling n), .term "?!"]
       | .sendReceive n args => [.elem (nameSpelling n), .term "!?", .term "("]
           ++ elemsToks args ++ [.term ")"])

/-- One branch, as an element (`BranchImpl ::= ReceiptLinearImpl "=>" Proc3`). -/
def branchTok : SBranch → Tok
  | ⟨bs, body⟩ =>
    .elem (renderTokens (bindListToks bs "<-" ++ [.term "=>"] ++ printToksAt body 3))

/-- A `select`'s branches (`separator nonempty Branch ""`) — one opaque element each. -/
def branchToks : List SBranch → List Tok
  | [] => []
  | [⟨bs, body⟩] => [branchTok ⟨bs, body⟩]
  | ⟨bs, body⟩ :: c :: cs => branchTok ⟨bs, body⟩ :: branchToks (c :: cs)

/-- One case, as an element (`CaseImpl ::= Proc13 "=>" Proc3`). -/
def caseTok : SCase → Tok
  | ⟨pat, body⟩ =>
    .elem (renderTokens (printToksAt pat 14 ++ [.term "=>"] ++ printToksAt body 3))

/-- A `match`'s cases (`separator nonempty Case ""`) — one opaque element each. -/
def caseToks : List SCase → List Tok
  | [] => []
  | [⟨pat, body⟩] => [caseTok ⟨pat, body⟩]
  | ⟨pat, body⟩ :: c :: cs => caseTok ⟨pat, body⟩ :: caseToks (c :: cs)

/-- A single `Decl` (`DeclImpl ::= [Name] NameRemainder "<-" [Proc]`). -/
def declToks : SDecl → List Tok
  | ⟨ns, rem, values⟩ => nameElemsToks ns ++ nameRemToks rem ++ [.term "<-"] ++ elemsToks values

/-- A `Decls`: the leading separator decides which list follows (`LinearDeclsImpl` / `ConcDeclsImpl`),
and neither is `EmptyDeclImpl`. -/
def declsToks : SDecls → List Tok
  | .empty => []
  | .linear ds => [.term ";"] ++ declListToks ds ";"
  | .conc ds => [.term "&"] ++ declListToks ds "&"

/-- A declaration list, whose separator the `Decls` production names. Each element is one opaque
element — the way a receipt is its list's — and the *caller* renders it, because a helper taking the
`Decl` would call the printer on its own argument and the mutual block would lose its structural
descent. -/
def declListToks : List SDecl → String → List Tok
  | [], _ => []
  | [d], _ => [.elem (renderTokens (declToks d))]
  | d :: e :: ds, sep =>
      .elem (renderTokens (declToks d)) :: .term sep :: declListToks (e :: ds) sep

/-- A `new`'s name declarations (`separator nonempty NameDecl ","`), including the `urn` form
(`NameDeclUrn ::= Var "(" UriLiteral ")"`). -/
def nameDeclToks : List SNameDecl → List Tok
  | [] => []
  | [.simple x] => [.elem x]
  | [.urn x u] => [.elem (x ++ "(" ++ u ++ ")")]
  | .simple x :: d :: ds => .elem x :: .term "," :: nameDeclToks (d :: ds)
  | .urn x u :: d :: ds => .elem (x ++ "(" ++ u ++ ")") :: .term "," :: nameDeclToks (d :: ds)

end

/-- The printer's tokens, from the loosest position. -/
def printToks (t : Surf) : List Tok := printToksAt t 0

/-- The printer's spelling: the token sequence, rendered. -/
def printSurf (t : Surf) : String := renderTokens (printToks t)

/-! ## The warts, and what is not printed here

Both tables are **data**, for the reason the rest of this tree keeps its gaps in tables: a wart that
lives in a comment is discovered again by the next reader, and a boundary that lives nowhere is mistaken
for an omission. The warts are the **port's** printer's: this printer emits the grammar's own spelling
for each of them, because a corpus row has to be a grammar term for law 31's completeness to be a claim
about the port. So each claim is checked where it can be — the consumer's round trip, by a detector for
each wart. -/

/-- A wart of the **port's** printer: a spelling it emits for a term that is not the term — either not a
grammar term at all, or a different one. This printer does *not* reproduce them (see the section note),
so the table's own checks are only its shape; the claim is the consumer's. -/
structure PrintWart where
  id : String
  /-- The production it happens on. -/
  production : String
  /-- What the port's printer emits, and what that is read as instead. -/
  emits : String
  /-- Why it is the port's, and how the consumer asserts the loss. -/
  why : String

/-- The warts. The first two were inherited from `PrettyPrinter.scala` and are pinned in the port
(`pretty_printer.rs`'s `the_documented_warts_print_what_the_grammar_cannot_read_back`); the third — the
`new`-urn drop — is a de-facto wart the port's comments do not record and this layer's Rust consumer
found. Each is asserted *by its own detector* in the consumer's round trip: the urn one requires the
cleared `New.uri`/`injections` to be the only difference, the tuple one requires the reparse to be the
element the tuple wrapped, and the `not` one requires it to be a negation. An exemption that cannot
fail would be this repo's most expensive kind of defect. -/
def printWarts : List PrintWart :=
  [ ⟨"singleton-tuple", "CollectTuple",
      "the port prints a one-element tuple as a group: `(1,)` ⇒ `(1)`, which the grammar reads as the \
       integer 1",
      "`PrettyPrinter.scala:117` prints `\"(\" buildSeq \")\"` with no trailing comma and the port \
       reproduces it. This printer emits `(1,)` — the grammar's `TupleSingle` — and the consumer's round \
       trip asserts the port's loss: the reparsed term is the element the tuple wrapped"⟩
  , ⟨"not-spelling", "PNot",
      "the port prints `not x` as `~(x)`, the *negation* spelling: `~` is process negation over a \
       `Proc15`, not the `not` of `PNot ::= \"not\" Proc10`",
      "`PrettyPrinter.scala:75` prints the negation spelling for a `PNot` node. This printer emits \
       `not x` — the grammar's own spelling — and the consumer asserts the port's reparse is a negation \
       rather than a `not`"⟩
  , ⟨"urn-in-new", "PNew",
      "the port prints a `new` with a urn without it: `new x(`rho:id:y`) in Nil` ⇒ `new x0 in { Nil }`, \
       so the reparsed term's `New.uri` is empty — the printer *loses* the urn (and `injections`)",
      "`pretty_printer.rs`'s New arm renders `bind_count` and the body and ignores `uri` and \
       `injections`, as `PrettyPrinter.scala` does. Found by this layer's Rust consumer rather than by \
       reading; the consumer asserts the urn really was lost"⟩
  ]

/-- Each wart names a real production, says what the port emits, and says how the loss is asserted — a
wart with no reason is a bug that has been left in place quietly. `decide`d. -/
theorem printWarts_decide :
    (printWarts.all (fun w =>
        !w.id.isEmpty && !w.emits.isEmpty && !w.why.isEmpty
          && grammarProductions.contains w.production)
      && (printWarts.map PrintWart.id).eraseDups.length == printWarts.length) = true := by
  decide

/-- How many warts the table carries. -/
def printWartCount : Nat := 3

/-- The table carries exactly `printWartCount` rows. -/
theorem printWarts_length : printWarts.length = printWartCount := by decide

/-- A boundary of the printer: something law 33's statement does not reach, and why. -/
structure PrintBoundary where
  id : String
  reason : String

/-- What this printer does **not** model. Each row is a decision taken deliberately rather than a gap
found later — the form `Surface.lean`'s `surfaceBoundaries` uses. -/
def printBoundaries : List PrintBoundary :=
  [ ⟨"layout", "the port's printer emits newlines and two-space indentation (`pretty_printer.rs`'s \
      `INDENT`, and its `\" |\\n\"` group separator). Layout is not a token: a re-parser does not read \
      it, so law 33's statement — `parse (print p) ≡ p` — is about the token sequence, and a model of \
      the layout would pin the Scala's whitespace rather than the language"⟩
  , ⟨"bundle-padding", "`BundleOps.showInstance` is `\"%-8s\".format(s\"bundle$sign\")`, so the port \
      emits `bundle0 { `, `bundle  { ` and friends — the eight-column padding IS where its space before \
      `{` comes from. The model emits `bundle0 {`, which is the same token sequence"⟩
  , ⟨"core-vs-surface", "the port prints the core `Par`: `let`, `if`, `select`, `contract` and \
      synchronous send are desugared by the normalizer before the printer sees them, so it has no rule \
      for them (`normalizer.rs:212-222`, `:782-851`, `:1096-1123`) — a `contract` round-trips through a \
      persistent `Receive`. This printer is over the *surface*, so the two are tied by source text and \
      re-parse (the Rust consumer's half), not by identical output"⟩
  , ⟨"no-precedence-in-port", "the port's parentheses are not precedence-driven: every binary \
      expression is wrapped (`wrap_with_braces`), and a method's target is always parenthesised. This \
      printer re-inserts parentheses from the precedence cascade, because a model of the grammar must \
      print terms the grammar derives — the two agree on what a term means, not on where the brackets \
      fall"⟩
  , ⟨"element-interior", "an element is opaque: its interior is not modelled by `Parse.lean`'s \
      fragment, so nothing here claims an element's spelling is itself in the grammar. `Surface.lean`'s \
      `normalize` is where a term's interior is modelled, and `Ty.lean` is where its type is"⟩
  , ⟨"unforgeable-leaves", "the model's `Ground` has no bigint *value* and no unforgeable leaf — \
      `Rchain/Json.lean` and `Rchain/Syntax.lean` record those boundaries (AUDIT C28) — so an \
      unforgeable in a value position has no spelling here"⟩
  ]

/-- The boundary table's rows are non-empty and distinct — a table that can say nothing is not a
boundary list. -/
theorem printBoundaries_decide :
    (printBoundaries.all (fun b => !b.id.isEmpty && !b.reason.isEmpty)
      && (printBoundaries.map PrintBoundary.id).eraseDups.length == printBoundaries.length) = true := by
  decide

/-- How many boundaries the printer names. -/
def printBoundaryCount : Nat := 6

/-- The table carries exactly `printBoundaryCount` rows. -/
theorem printBoundaries_length : printBoundaries.length = printBoundaryCount := by decide

end Rchain
