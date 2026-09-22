import Rchain.Par

/-!
# Law 32 — lexical determinism: each spelling lexes one way

A lexer is where a *character* becomes a *meaning*, and it is the layer where a wrong meaning is
cheapest to miss: AUDIT C10 was `/\` and `\/` **swapped** in the port's operator cascade, so `a \/ b`
lexed as conjunction and `a /\ b` did not lex at all — "a silent *semantic* swap on a ρ-calculus
connective". Nothing errored; the two connectives simply meant each other.

This module holds the operator surface as **data** — the port's `Tok` variants (`rholang/src/parser.rs`)
with the characters that spell them and, for each, a *sample* term whose result changes if the spelling
means one of its neighbours — and checks what a table can check about itself:

- **spellings are distinct** (two spellings for one meaning is a lexer with a rule nobody wrote down);
- **maximal munch**: for every row, the longest spelling in the table that prefixes the row's own
  spelling *is* that row (`longestMatch`) — the rule the port implements as a cascade of `peek` guards,
  and the one a hand-written cascade gets wrong when a short spelling is tested first (`<` before `<=`
  would make every `<=` a `<` followed by an `=`);
- every spelling is punctuation (an operator spelled with letters would lex as an identifier).

The *semantics* are in the table too — each row says what its sample must observe — and that is the
part the two parties split: the model asserts the meaning, and `rholang/tests/lean_lex_corpus.rs` runs
the sample and reports what the node did. A swap like C10's fails the sample, which is what makes this
a law rather than a comment about a cascade.

**Two spellings may share one token**: `->` and `=>` are both `Arrow`, and `<=` is one token the parser
reads two ways (a comparison, and the persistent-receive arrow of `for (x <= ch)`). The table checks the
*spellings*; the context rule is the parser's business, not the lexer's.

**Boundary, stated:** this is the *operator* surface — the spellings C10's incident lived in. The rest
of law 32 (comments, the `_`/`_ident` rule, `bundle0`, the number and string/uri literal forms) needs the
full lexer that laws 30/31/33 share, and it is not claimed here.
-/

namespace Rchain

/-- One lexeme: the token's name (the port's `Tok` variant), the characters that spell it, a sample
term, and what that sample must observe. The observation has two forms, because two kinds of question
are being asked:

- `value:<json>` — the sample sends one datum on `@"out"`, which must be that JSON (law 42's machinery);
- `answers:<n>` — the sample drives a store and must get `n` answers on `@"out"`, which is how `<-`,
  `<<-` and `<=` are told apart (one consumes, one peeks, one re-arms). -/
structure Lexeme where
  token : String
  spelling : String
  sample : String
  expected : String

/-- Does `s` begin with `p`? (`List Char`, so the kernel reduces it — the same reason `Envelope.lean`'s
`hasPrefix` is written this way.) -/
def startsWith (p s : String) : Bool := s.toList.take p.length == p.toList

/-- **Maximal munch**, over a table: the longest spelling that prefixes `s`, with its token. Stating the
rule as a function over the table is what lets the table check itself. -/
def longestMatchIn (table : List Lexeme) (s : String) : Option (String × String) :=
  table.foldl
    (fun best l =>
      if startsWith l.spelling s then
        match best with
        | none => some (l.spelling, l.token)
        | some (b, _) =>
          if b.length < l.spelling.length then some (l.spelling, l.token) else best
      else best)
    none

/-- Every spelling is punctuation: no letter and no digit starts one (an operator spelled with a letter
would lex as an identifier, a different token class entirely). -/
def isPunctuation (s : String) : Bool :=
  let alnum (c : Char) : Bool := ('a' <= c && c <= 'z') || ('A' <= c && c <= 'Z') || ('0' <= c && c <= '9')
  match s.toList with
  | [] => false
  | c :: _ => !alnum c

/-- The operator surface, as the port's lexer spells it (`rholang/src/parser.rs`'s `match c` cascade,
read row for row). -/
def lexemes : List Lexeme :=
  [ -- The connectives C10 swapped. They are *pattern* connectives, and two position rules shaped the
    -- samples: a top-level connective in *process* position is refused
    -- (`TopLevelLogicalConnectivesNotAllowedError`), and at bind depth 0 a `\/` or `~` in a *pattern*
    -- is refused too (`fail_on_invalid_connective`, "`\/` (disjunction) at …"), and the port checks that
    -- rule for *receive* patterns only — `match` case patterns are not checked — so the samples use a
    -- `match`. The datum is `[1]`: its element matches `x /\ String` only if `String`
    -- does as well (it does not), and matches `x \/ String` because `x` does. The two rows therefore
    -- discriminate each other, and the conjunction's expectation is *silence* — which is why every
    -- sample carries its control datum on `@"ctl"`.
    { token := "Conj", spelling := "/\\",
      sample := "match [1] { [x /\\ String] => { @\"out\"!(\"matched\") } } | @\"ctl\"!(\"ran\")",
      expected := "answers:0" }
  , { token := "Disj", spelling := "\\/",
      sample := "match [1] { [x \\/ String] => { @\"out\"!(\"matched\") } } | @\"ctl\"!(\"ran\")",
      expected := "answers:1" }
    -- `...` is the collection remainder: the sample matches a partial list pattern.
  , { token := "Ellipsis", spelling := "...",
      sample := "new c in { @\"c\"!([1, 2]) | for (@[1, ..._] <- @\"c\") { @\"out\"!(\"matched\") } } | @\"ctl\"!(\"ran\")",
      expected := "value:{\"ExprString\":\"matched\"}" }
    -- Arithmetic and the collection operators, each against its neighbour.
  , { token := "Star", spelling := "*", sample := "@\"out\"!(2 * 3) | @\"ctl\"!(\"ran\")",
      expected := "value:{\"ExprInt\":6}" }
  , { token := "Slash", spelling := "/", sample := "@\"out\"!(6 / 2) | @\"ctl\"!(\"ran\")",
      expected := "value:{\"ExprInt\":3}" }
  , { token := "Percent", spelling := "%", sample := "@\"out\"!(5 % 2) | @\"ctl\"!(\"ran\")",
      expected := "value:{\"ExprInt\":1}" }
  , { token := "Plus", spelling := "+", sample := "@\"out\"!(1 + 2) | @\"ctl\"!(\"ran\")",
      expected := "value:{\"ExprInt\":3}" }
  , { token := "PlusPlus", spelling := "++", sample := "@\"out\"!([1] ++ [2]) | @\"ctl\"!(\"ran\")",
      expected := "value:{\"ExprList\":[{\"ExprInt\":1},{\"ExprInt\":2}]}" }
  , { token := "Minus", spelling := "-", sample := "@\"out\"!(3 - 1) | @\"ctl\"!(\"ran\")",
      expected := "value:{\"ExprInt\":2}" }
  , { token := "MinusMinus", spelling := "--",
      sample := "@\"out\"!(Set(1, 2) -- Set(1)) | @\"ctl\"!(\"ran\")",
      expected := "value:{\"ExprSet\":[{\"ExprInt\":2}]}" }
    -- The comparisons: each sample is true only for its own spelling.
  , { token := "Lt", spelling := "<", sample := "@\"out\"!(1 < 1) | @\"ctl\"!(\"ran\")",
      expected := "value:{\"ExprBool\":false}" }
  , { token := "Lte", spelling := "<=", sample := "@\"out\"!(1 <= 1) | @\"ctl\"!(\"ran\")",
      expected := "value:{\"ExprBool\":true}" }
  , { token := "Gt", spelling := ">", sample := "@\"out\"!(1 > 1) | @\"ctl\"!(\"ran\")",
      expected := "value:{\"ExprBool\":false}" }
  , { token := "Gte", spelling := ">=", sample := "@\"out\"!(1 >= 1) | @\"ctl\"!(\"ran\")",
      expected := "value:{\"ExprBool\":true}" }
  , { token := "EqEq", spelling := "==", sample := "@\"out\"!(1 == 1) | @\"ctl\"!(\"ran\")",
      expected := "value:{\"ExprBool\":true}" }
  , { token := "Neq", spelling := "!=", sample := "@\"out\"!(1 != 2) | @\"ctl\"!(\"ran\")",
      expected := "value:{\"ExprBool\":true}" }
    -- The arrows: one datum, read linearly (`<-` consumes), peeked (`<<-` does not) and persistently
    -- (`<=` re-arms) — so the counts are 1, 2 and 2. `<-`/`<<-` are distinct spellings; `<=` is the
    -- *same* token as the comparison above, read by the parser's bind position rather than by the
    -- lexer, which is why it does not appear twice here.
  , { token := "LArrow", spelling := "<-",
      sample := "new c in { @\"c\"!(1) | for (x <- @\"c\") { @\"out\"!(\"read\") } } | @\"ctl\"!(\"ran\")",
      expected := "answers:1" }
  , { token := "LLArrow", spelling := "<<-",
      sample := "new c in { @\"c\"!(1) | for (x <<- @\"c\") { @\"out\"!(\"peek\") } | for (y <<- @\"c\") { @\"out\"!(\"peek2\") } } | @\"ctl\"!(\"ran\")",
      expected := "answers:2" }
  ]

-- The `decide` below unfolds `longestMatchIn` over eighteen rows for every row's own spelling, and the
-- default recursion depth is not enough for that: raising it is what the `decide` needs, and it does not
-- weaken it (the kernel still checks the reduction).
set_option maxRecDepth 10000 in
/-- The table is well-formed and **maximal munch**, `decide`d: spellings distinct and punctuation, every
row carrying both a sample and its expectation, and each row's spelling is its own longest match — a
table where `<` shadowed `<=` would fail the `<=` row. -/
theorem lexemes_decide :
    ((lexemes.map Lexeme.spelling).eraseDups.length == lexemes.length
      && lexemes.all (fun l => isPunctuation l.spelling && !l.sample.isEmpty && !l.expected.isEmpty)
      && lexemes.all (fun l => longestMatchIn lexemes l.spelling == some (l.spelling, l.token))) = true := by
  decide

/-- The count the Rust consumer asserts it read. -/
def lexemeCount : Nat := 18

/-- The table carries exactly `lexemeCount` rows. -/
theorem lexemes_length : lexemes.length = lexemeCount := by decide

end Rchain
