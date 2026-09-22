# Porting an app from rnode

If your app was written against the **old `rnode`**, most of it needs no change: the routes, the JSON
envelope, the `deploy`/`explore-deploy`/`deploy-status` responses and the served OpenAPI document are
the same surface. This page is what *is* different, in the two places it bites — **the shapes you will
receive**, and **the terms this node will refuse** — plus the four traps that read like bugs the first
time you point a working app at it.

## 1. What a result looks like

Every rho value is tagged and carries its payload in a field named `data`, exactly as `rnode` sent it:

| Value | JSON |
|---|---|
| an integer | `{"ExprInt":{"data":42}}` |
| a string | `{"ExprString":{"data":"b"}}` |
| a boolean | `{"ExprBool":{"data":true}}` |
| a list / tuple / set / par | `{"ExprList":{"data":[…]}}` (and `ExprTuple` / `ExprSet` / `ExprPar`) |
| a map | `{"ExprMap":{"data":{"k":{…}}}}` — a JSON **object**, keys in canonical (sorted) order |
| an unforgeable | `{"ExprUnforg":{"data":{"UnforgPrivate":{"data":"<hex>"}}}}` |

Two facts about the response *around* the value, both of which trip hand-written parsers:

- **A result is a list.** `explore-deploy` and `deploy-status` return `expr` / `deployResult` as an
  array of values, because the result channel is a `Par` — `[42]`, not `42`. An empty array is a term
  that sent nothing (see the traps below).
- **The whole response is an object with `block`**, plus `replySource` on the exploratory path.

Accessor:

```js
const isEnvelope = (p) => p && typeof p === "object" && !Array.isArray(p)
  && Object.keys(p).length === 1 && "data" in p;

function unwrap(node) {
  const [tag, payload] = Object.entries(node)[0];
  const body = isEnvelope(payload) ? payload.data : payload;
  switch (tag) {
    case "ExprPar": case "ExprTuple": case "ExprList": case "ExprSet":
      return body.map(unwrap);
    case "ExprMap":
      return Object.fromEntries(Object.entries(body).map(([k, v]) => [k, unwrap(v)]));
    case "ExprBool": case "ExprInt": case "ExprString": case "ExprUri": case "ExprBytes":
      return body;
    case "ExprUnforg": {
      const [tag2, payload2] = Object.entries(body)[0];
      return { unforgeable: tag2, hex: isEnvelope(payload2) ? payload2.data : payload2 };
    }
    default:
      throw new Error(`unknown RhoExpr arm: ${tag}`);
  }
}
```

The machine-readable version is in the served document (`GET /api/v1/openapi`): the `RhoExpr` schema
describes every arm, so a generated client is right by construction.

## 2. What the parser now refuses

**The parser here is held to the grammar** — the BNFC file the language is defined by
([`legacy/rholang/src/main/bnfc/rholang_mercury.cf`](https://github.com/rchain-community/rchain-rust/blob/dev/legacy/rholang/src/main/bnfc/rholang_mercury.cf))
— so it is stricter than a permissive one, and it never accepts a *prefix* of what you sent. Two
consequences for a term your app already has: it may be **refused**, or it may have been doing
something other than what it looked like. Check these against the terms you store or deploy:

| Refused, with the reason | |
|---|---|
| `[1, 2,]`, `c!(1,)`, `{a: 1,}`, `Set(1,)`, `(1, 2,)`, `contract c(@x,) = { Nil }`, `new x,) in { Nil }` | a **trailing separator**: the grammar's list is `[X] ::= X \| X "," [X]`, so nothing may follow the comma |
| `new x Nil`, `let x <- 1 { Nil }` | **`in` omitted**: `PNew`/`PLet` require it |
| `x.m` — a method call with no argument list | `PMethod ::= Proc11 "." Var "(" [Proc] ")"` requires the parentheses |
| `Nil )`, `c!(1) garbage` | **trailing input**: the whole token stream must be the term, not a term followed by anything |
| `"abc`, `` `abc `` | an **unterminated literal** (a lexer error, where a sliced index used to be a panic) |

And these now parse as what they *say*, where a permissive parser read them as something else —
silently:

| Parsed now as | Where it used to read as |
|---|---|
| `x!?(1); P` — a synchronous send, with its continuation | the bare variable `x`, discarding `!?(1); P` |
| `BigInt(42)` — the bigint ground the grammar names | the *type* `BigInt` with `(42)` discarded |

**One deliberate exception, kept because the contracts here use it:** the *comma* form of a collection
remainder — `[head, ...tail]`, `{"a": 1, ...rest}` — is **accepted**, though the grammar separates a
remainder from the list by no terminal at all. The shipped genesis sources spell it both ways
(`ListOps.rho:38` `[head ...tail]`, `MemberDirectory.rho:124` `[themRevAddr, ...rest]`), so refusing it
would stop them loading; the acceptance is a registered deviation
([`spec/API-SCHEMA.md`](../../../spec/API-SCHEMA.md), and law 31's deviation list).

If a term of yours is refused now, the fix is on the term: the grammar is the contract, and the
deploy's failure names the position.

## 3. Traps that read like bugs

1. **Silence is not failure.** A `for` whose pattern matches no datum does no step and reports **no
   error** — that is the calculus, not a dropped reply. If a term answers nothing, the pattern, the
   arity or the channel is wrong; nothing will tell you which. (Law 38.)
2. **An empty `expr` is ambiguous; `replySource` is not.** The exploratory path reads its reply from
   the term's first `new`-bound name, then from `@"out"` — and the response's `replySource` says which
   one answered (`firstPrivateName`, `out`, or `none`). A term that produces `{"expr":[]}` with
   `"none"` produced nothing on either channel. On the deploy path the reply is read from
   `rho:rchain:deployId`.
3. **A call at the wrong arity does nothing.** `write!(key, value)` against a contract head
   `write(@key, @value, ret)` matches no receive, so the branch never runs and nothing errors
   (law 40). Count your arguments against the contract's head.
4. **`rho:rchain:deployId` / `deployerId` are not bound under `explore-deploy`.** A term that reads
   them fails with `No value set for …` there; substitute a fresh name when probing a gate that uses
   them.

## 4. Two deliberate differences from `rnode`

- **`rho:registry:ops` uri encoding.** This node's `rho:id:` is z-base-32 of the 32-byte blake2b256
  hash; the Scala's was CRC14 plus a 270-bit z-base-32 (`rholang/src/registry.rs:3-4`). The JSON
  shape is identical and the string is opaque, so this only matters if your app *computes* or
  validates a uri rather than storing one. Registered in
  [`spec/API-SCHEMA.md`](../../../spec/API-SCHEMA.md).
- **An exploratory deploy is bounded here**: a phlo limit, a reduction-step budget and a 60-second
  deadline, where the Scala ran it unbounded. A long-running probe that returned on `rnode` may return
  a *timeout* here; that is a documented deviation, not a hang.

Everything else in the surface — including the reply shapes above — follows the reference, and
`spec/API-SCHEMA.md` is the register: every row cites the oracle it follows and the test that enforces
it. `spec/RUST-VS-SCALA.md` covers the differences that are *not* on the API surface (internal
invariants the rewrite makes structural), which your app should never see.
