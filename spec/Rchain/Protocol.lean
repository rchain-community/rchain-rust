import Rchain.Par

/-!
# Law 39 — reply shapes: the urn → reply catalog, as data

Every `rho:*`/`sys:*` system process answers in a shape a client has to know: how many values come back
(the *send arity*), and what each one is (a `Nil`, a `Bool`, a `ByteArray`, a `rho:id:` uri, a map …).
`spec/API-SCHEMA.md` states those shapes in prose and one row at a time, enforced by hand-written
probes, and the class of defect that lives here is C18: `rho:registry:lookup` wrapped its reply in
`(uri, value)` while every consumer destructured the bare value, so **it failed silently rather than
loudly** — nothing errors when a pattern does not match a shape.

This module makes the catalog *data* with decidable consistency checks, and the corpus emitted from it
(`Rchain/Corpus.lean`'s `replyCatalog`, `spec/conformance/protocol.tsv`) carries each row's **call
arguments** as rholang, so the Rust consumer can call the urn the way the law's row says and classify
what comes back. The checks here are on the table itself — they catch a row that drifted, not a reply
that did; the node disagreeing with a row is what the corpus reports.

The one check with real teeth on the *table* is `callArity` against the arguments: a row that passes
three arguments and declares an arity of two is exactly C22 item 2 (`write!(key, value)` against a
three-argument `write`), where the call is silently never matched. In Rholang that is not an error, it
is silence, so a table that does not count its own arguments cannot catch it.

**Boundary, stated rather than implied:** a row needs arguments the *surface language* can spell. The
crypto urns (`rho:crypto:*`) and `rho:rchain:deployerId:ops`, `sys:authToken:ops` take a `ByteArray`,
and the grammar has no byte-array literal (`Ground ::= BoolLiteral | "BigInt(" … ")" | LongLiteral |
StringLiteral | UriLiteral`), so those urns' shapes stay pinned by `rholang/tests/
system_process_conformance.rs`'s hand-written probes, which build the bytes in Rust. The catalog grows
as rows that can be spelled arrive; it does not pretend to cover the urns it cannot call.
-/

namespace Rchain

/-- What one value in a reply is. `any` is for a value whose shape is the caller's own (a registry
lookup returns whatever was stored), and it is the only tag that asserts nothing. -/
inductive SlotShape where
  | any | nil | bool | int | string | uri | byteArray | map | set | list
deriving DecidableEq, Repr

/-- The tag the corpus and the consumer use for a slot shape. -/
def SlotShape.tag : SlotShape → String
  | .any => "any" | .nil => "nil" | .bool => "bool" | .int => "int" | .string => "string"
  | .uri => "uri" | .byteArray => "byteArray" | .map => "map" | .set => "set" | .list => "list"

/-- How the reply arrives. A system process produces with `cc.produce(…, &[v, …], ack, …)`, so `send`
is an *n-arity send* (the receiver needs `n` patterns); `tuple` is one datum that is an n-tuple (one
pattern, `@(v0, …)`). The two are different shapes to a client and both are in the table. -/
inductive ReplyKind where
  | none | send | tuple
deriving DecidableEq, Repr

/-- The tag for a reply kind. -/
def ReplyKind.tag : ReplyKind → String
  | .none => "none" | .send => "send" | .tuple => "tuple"

/-- A row of the catalog: the urn, the call's arguments **as rholang source** (before the reply
channel), the call's arity — the node's `Definition.arity`, which counts the reply channel when there
is one — and the reply's shape. -/
structure ReplyRow where
  urn : String
  args : String
  callArity : Nat
  kind : ReplyKind
  slots : List SlotShape

/-- Does `s` begin with `p`? (`take`/`List Char` so the kernel can reduce it: the `decide`s below are
about strings, and an opaque `startsWith` would not reduce.) -/
def hasPrefix (p s : String) : Bool := s.toList.take p.length == p.toList

/-- A urn is in one of the two namespaces the fixed-channel table has. -/
def urnShaped (u : String) : Bool := hasPrefix "rho:" u || hasPrefix "sys:" u

/-- How many top-level arguments a source string spells: commas outside every bracket, paren and brace.
This is what makes the arity column a *check* on the arguments rather than a second opinion. -/
def countArgs (s : String) : Nat :=
  let step (acc : Nat × Nat) (c : Char) : Nat × Nat :=
    let (depth, n) := acc
    match c with
    | '(' | '{' | '[' => (depth + 1, n)
    | ')' | '}' | ']' => (depth - 1, n)
    | ',' => (depth, if depth == 0 then n + 1 else n)
    | _ => (depth, n)
  match s with
  | "" => 0
  | _ => (s.toList.foldl step (0, 0)).2 + 1

/-- How many arguments this row's call passes: the ones it spells, plus the reply channel — and only
when a reply is expected. `rho:io:stdout` takes one argument (the thing to print) and answers nothing,
so its arity is 1 and not 2. -/
def ReplyRow.probeArity (r : ReplyRow) : Nat :=
  countArgs r.args + (if r.kind == ReplyKind.none then 0 else 1)

/-- The row's `kind` and its slots agree: no reply has no slots, a send with no data is not a reply,
and a tuple has at least one part. -/
def kindAgrees (r : ReplyRow) : Bool :=
  match r.kind with
  | .none => r.slots.isEmpty
  | .send => !r.slots.isEmpty
  | .tuple => !r.slots.isEmpty

/-- The catalog: every row is a probe the node can be called with — `urn` called with `args` and, when
a reply is expected, the reply channel — whose reply must arrive as `kind`/`slots`. The shapes are read
off the handlers (`rholang/src/system_processes.rs`) and cross-checked with `spec/API-SCHEMA.md`'s
rows; the consumer (`rholang/tests/lean_protocol_corpus.rs`) is what makes them true of a running
node. -/
def replyCatalog : List ReplyRow :=
  [ -- Prints, replies not at all — one argument, no reply channel. Its `Ack` sibling replies `Nil`
    -- (`cc.produce(…, &[Par::default()])`).
    { urn := "rho:io:stdout", args := "\"x\"", callArity := 1, kind := .none, slots := [] }
  , { urn := "rho:io:stderr", args := "\"x\"", callArity := 1, kind := .none, slots := [] }
  , { urn := "rho:io:stdoutAck", args := "\"x\"", callArity := 2, kind := .send, slots := [.nil] }
  , { urn := "rho:io:stderrAck", args := "\"x\"", callArity := 2, kind := .send, slots := [.nil] }
  , -- `(blockNumber, sender, timestamp)` — **three** values, the documented extension over the
    -- oracle's `(blockNumber, sender)`: `docs/src/rholang/reference.md:93` specifies the timestamp and
    -- `RevVault.rho:207-209` binds all three. This row is where that divergence is pinned.
    { urn := "rho:block:data", args := "", callArity := 1, kind := .send,
      slots := [.int, .byteArray, .int] }
  , -- An unknown key answers `Nil`; a known one answers the stored value, whatever it was (`any`).
    { urn := "rho:registry:lookup", args := "\"rho:id:unknown0000000000000000000000000000000000\"",
      callArity := 2, kind := .send, slots := [.nil] }
  , { urn := "rho:registry:insertArbitrary", args := "Nil", callArity := 2, kind := .send,
      slots := [.uri] }
  , -- `"validate"` on an unparsable address answers the parse *error string* (`String`), not `Nil`:
    -- `RevAddress::parse(address).err().map(RhoString::apply).unwrap_or_default()`.
    { urn := "rho:rev:address", args := "\"validate\", \"abc\"", callArity := 3, kind := .send,
      slots := [.string] }
  , -- `cc.produce(&[RhoTupleN::apply([bool, int])])` — *one* datum that is a two-part tuple, which is
    -- why its kind is `tuple` and not `send`.
    { urn := "rho:qucalc:zfa", args := "[0, 1]", callArity := 2, kind := .tuple,
      slots := [.bool, .int] }
  ]

/-- Every row of the catalog is well-formed, `decide`d: the urns are namespaced and unique, an arity is
at least one, the **arity agrees with the arguments as written** (the C22 item 2 class), and the reply
kind agrees with the slots. A row that drifts in any of those fails `lake build`. -/
theorem replyCatalog_decide :
    (replyCatalog.all (fun r =>
        urnShaped r.urn && r.callArity ≥ 1 && r.callArity == r.probeArity && kindAgrees r)
      && (replyCatalog.map ReplyRow.urn).eraseDups.length == replyCatalog.length) = true := by
  decide

/-- The count the Rust consumer asserts it read. -/
def replyCaseCount : Nat := 9

/-- The catalog carries exactly `replyCaseCount` rows. -/
theorem replyCatalog_length : replyCatalog.length = replyCaseCount := by decide

end Rchain
