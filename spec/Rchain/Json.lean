import Rchain.Par

/-!
# Law 42 — the rho-value JSON round-trip, and the envelope rule

The web API exposes rholang data as a JSON-ish tree (`node/src/api/rho_expr.rs`'s `RhoExpr`, which
clients read as `{"ExprInt":{"data":42}}`, `{"ExprBytes":{"data":"deadbeef"}}`,
`{"ExprPar":{"data":[…]}}` — the reference document's shape, arm for arm; see `render` below and
AUDIT C38). Two things about that conversion were a comment rather than a theorem, and both are AUDIT
C16's family — a shape a client depends on, pinned by nothing:

- **the envelope rule** — `expr_from_par` reads a par's fields and then decides by *count*: one value
  comes back unwrapped, none is "absent" (`None`: a response renders no value at all), and two or more
  become `ExprPar`. So `1` and `[1]` expose the same JSON, `Nil` exposes none, and `1 | 2` is
  `ExprPar [1, 2]`. That is `parToJE`/`envelope` below, and the corpus's cases are its three counts.
- **the round-trip** — `rho_expr_to_par (expr_from_par p) = p` for every JSON-able `Par`. This module
  states it as `decode_encode` over the JSON-able fragment (`flatPar`), and the corpus checks the same
  statement against the node's own codec with the model's *rendering* as the independent witness.

`JE` mirrors `RhoExpr` except in two places, named rather than hidden:

- a byte array is **raw bytes** here (`List Nat`), hex-encoded only when rendered — which is exactly
  what the wire form is, so the two sides still compare;
- an unforgeable is **not decodable** in the model: its `GUnforgeable` carries a de Bruijn *level*
  where the wire carries the name's bytes, so `unforgToJE` can render a *shape* but `jeToPar` cannot
  reconstruct one. A `Par` containing an unforgeable is therefore outside the round-trip's domain
  (its `flatPar` excludes it), and the node's own unit tests pin that leaf (`rho_expr.rs`'s
  `terminal_exprs_round_trip`)
  where this law cannot.

The model's `Ground` gained `uri` and `bytes` for this law (AUDIT C28): without them the model could
not *hold* the two leaf forms the JSON layer is largely about.
-/

namespace Rchain

/-- An unforgeable leaf: the JSON tag and the hex the wire carries. -/
structure JUnforg where
  tag : String
  hex : String
deriving DecidableEq, Repr

/-- The `RhoExpr` tree as data: the JSON the API exposes a rho value as. -/
inductive JE where
  | par   : List JE → JE
  | tuple : List JE → JE
  | list  : List JE → JE
  | set   : List JE → JE
  | map   : List (String × JE) → JE
  | bool  : Bool → JE
  | int   : Int → JE
  | str   : String → JE
  | uri   : String → JE
  | bytes : List Nat → JE
  | unforg : JUnforg → JE

/-- A `Par` holding one expression. -/
def one (e : Expr) : Par := Par.mk [] [] [] [e] [] [] [] []

/-- Unicode code points as a `String` (the model's `Ground` carries code points). -/
def codePointsToString (l : List Nat) : String := (l.map Char.ofNat).asString

/-- A string's code points. -/
def codePointsOf (s : String) : List Nat := s.toList.map Char.toNat

/-- Lower-case hex, as `base16::encode` writes it. -/
def hexEncode (bs : List Nat) : String :=
  let digits := "0123456789abcdef".toList
  (bs.map (fun b => digits.getD (b / 16) '0') ++ bs.map (fun b => digits.getD (b % 16) '0')).asString

/-- JSON string text: quoted, with `"`/`\` and the three common control escapes. Other control
characters are out of scope here rather than silently mangled, and the corpus's cases avoid them. -/
def jsonStr (s : String) : String :=
  let esc (c : Char) : String :=
    match c with
    | '"' => "\\\"" | '\\' => "\\\\" | '\n' => "\\n" | '\r' => "\\r" | '\t' => "\\t"
    | c => [c].asString
  "\"" ++ String.join (s.toList.map esc) ++ "\""

/-- `[a,b,…]`. -/
def bracket (xs : List String) : String := "[" ++ String.intercalate "," xs ++ "]"

mutual
  /-- The wire text of a `JE`: **the reference document's shape, arm for arm** — every arm wraps its
  payload in a field named `data`, a map's payload is a JSON *object* keyed by string, and an
  unforgeable nests one level (`{"ExprUnforg":{"data":{"UnforgPrivate":{"data":"ab"}}}}`). The shapes
  are copied from `legacy/docs/rnode-api/rnode-openapi-schema.ts`, the file the Scala node's own
  OpenAPI document generates for clients: `ExprInt: { ExprInt: { data: number } }` and so on.

  This is AUDIT C38. The model used to render `{"ExprInt":42}` — because it was written from the
  *port's* `#[derive]`d serde output rather than from the contract, so law and code agreed with each
  other and both disagreed with every client. The corpus could not catch that: it compares the node
  to the model, and the two were wrong together. -/
  def render : JE → String
    | .par es => data "ExprPar" (bracket (renderItems es))
    | .tuple es => data "ExprTuple" (bracket (renderItems es))
    | .list es => data "ExprList" (bracket (renderItems es))
    | .set es => data "ExprSet" (bracket (renderItems es))
    | .map kvs => data "ExprMap" ("{" ++ String.intercalate "," (renderKvs kvs) ++ "}")
    | .bool b => data "ExprBool" (if b then "true" else "false")
    | .int n => data "ExprInt" (toString n)
    | .str s => data "ExprString" (jsonStr s)
    | .uri s => data "ExprUri" (jsonStr s)
    | .bytes l => data "ExprBytes" (jsonStr (hexEncode l))
    | .unforg u => data "ExprUnforg" (data u.tag (jsonStr u.hex))

  /-- One arm: `{"<tag>":{"data":<payload>}}`. The tag is the bare name (`ExprInt`), quoted here. -/
  def data (tag payload : String) : String := "{\"" ++ tag ++ "\":{\"data\":" ++ payload ++ "}}"

  def renderItems : List JE → List String
    | [] => []
    | e :: es => render e :: renderItems es

  /-- A map's entries as a JSON object's members, in the canonical order law 1 gives them. -/
  def renderKvs : List (String × JE) → List String
    | [] => []
    | (k, v) :: kvs => (jsonStr k ++ ":" ++ render v) :: renderKvs kvs
end

/-- The envelope rule, on its own: `expr_from_par`'s `match exprs.len()` — one value comes back
unwrapped, none is absent, two or more become an `ExprPar` (`node/src/api/rho_expr.rs:57-62`). -/
def envelope (jes : List JE) : Option JE :=
  match jes.length with
  | 0 => none
  | 1 => jes.head?
  | _ => some (.par jes)

mutual
  /-- The **encode**: a `Par` to the JSON the API exposes. The three fields `RhoExpr` reads are
  `exprs`, `unforgeables` and `bundles`; everything else in the flat `Par` has no JSON arm, and the
  rule above decides by the count of what is left. -/
  def parToJE : Par → Option JE
    | .mk _ _ _ e _ u b _ => envelope (exprsToJE e ++ unforgsToJE u ++ bundlesToJE b)

  def exprsToJE : List Expr → List JE
    | [] => []
    | e :: es => (match exprToJE e with | some j => [j] | none => []) ++ exprsToJE es

  def exprToJE : Expr → Option JE
    | .ground (.bool b) => some (.bool b)
    | .ground (.int n) => some (.int n)
    | .ground (.str l) => some (.str (codePointsToString l))
    | .ground (.uri l) => some (.uri (codePointsToString l))
    | .ground (.bytes l) => some (.bytes l)
    | .etuple ps => some (.tuple (parsToJE ps))
    | .elist ps _ => some (.list (parsToJE ps))
    | .eset ps _ => some (.set (parsToJE ps))
    | .emap kvs _ => some (.map (mapKvsToJE kvs))
    | _ => none

  def parsToJE : List Par → List JE
    | [] => []
    | p :: ps => (match parToJE p with | some j => [j] | none => []) ++ parsToJE ps

  def mapKvsToJE : List (Par × Par) → List (String × JE)
    | [] => []
    | (kp, vp) :: kvs =>
      (match keyToJE kp, parToJE vp with
       | some k, some v => [(k, v)]
       | _, _ => []) ++ mapKvsToJE kvs

  /-- A map key as text: `key_to_string`'s rule. A key that is not one of the stringifiable leaves
  drops the entry — a lossiness the law has to carry, since the JSON map is string-keyed. -/
  def keyToJE : Par → Option String
    | .mk _ _ _ es _ _ _ _ =>
      match es with
      | [.ground (.str l)] => some (codePointsToString l)
      | [.ground (.int n)] => some (toString n)
      | [.ground (.bool b)] => some (toString b)
      | [.ground (.uri l)] => some (codePointsToString l)
      | [.ground (.bytes l)] => some (hexEncode l)
      | _ => none

  def unforgsToJE : List GUnforgeable → List JE
    | [] => []
    | u :: us => (match unforgToJE u with | some j => [j] | none => []) ++ unforgsToJE us

  /-- An unforgeable's JSON leaf. `gSysAuthToken` has no `RhoExpr` arm in the port (`unforg_from_proto`
  ends in `_ => None`), so it encodes to nothing here too. The hex is the model's level, not the wire's
  bytes — the boundary note in the module doc. -/
  def unforgToJE : GUnforgeable → Option JE
    | .gPrivate n => some (.unforg ⟨"UnforgPrivate", hexEncode [n % 256]⟩)
    | .gDeployId n => some (.unforg ⟨"UnforgDeploy", hexEncode [n % 256]⟩)
    | .gDeployerId => some (.unforg ⟨"UnforgDeployer", hexEncode []⟩)
    | .gSysAuthToken => none

  def bundlesToJE : List Bundle → List JE
    | [] => []
    | b :: bs => (match bundleToJE b with | some j => [j] | none => []) ++ bundlesToJE bs

  /-- A bundle exposes its body (`expr_from_bundle`). -/
  def bundleToJE : Bundle → Option JE
    | .mk body _ _ => parToJE body
end

mutual
  /-- The **decode**: `rho_expr_to_par`. A `par` is the merge of its parts, which is what
  `rho_expr_to_par`'s `ExprPar` arm folds. -/
  def jeToPar : JE → Option Par
    | .par es => some (parsToPar es)
    | .tuple es => some (one (.etuple (parsToParList es)))
    | .list es => some (one (.elist (parsToParList es) none))
    | .set es => some (one (.eset (parsToParList es) none))
    | .map kvs => some (one (.emap (kvsToPars kvs) none))
    | .bool b => some (one (.ground (.bool b)))
    | .int n => some (one (.ground (.int n)))
    | .str s => some (one (.ground (.str (codePointsOf s))))
    | .uri s => some (one (.ground (.uri (codePointsOf s))))
    | .bytes l => some (one (.ground (.bytes l)))
    | .unforg _ => none

  def parsToPar : List JE → Par
    | [] => Par.mk [] [] [] [] [] [] [] []
    | j :: js => parMerge ((jeToPar j).getD (Par.mk [] [] [] [] [] [] [] [])) (parsToPar js)

  def parsToParList : List JE → List Par
    | [] => []
    | j :: js => (jeToPar j).toList ++ parsToParList js

  def kvsToPars : List (String × JE) → List (Par × Par)
    | [] => []
    | (k, v) :: kvs =>
      match jeToPar v with
      | some p => (one (.ground (.str (codePointsOf k))), p) :: kvsToPars kvs
      | none => kvsToPars kvs
end

mutual
  /-- A `JE` the API round-trips, in the sense the *envelope rule* allows: no `par` node holds another
  `par` (the decoder **merges** what it sees, so a nested one would flatten into its parent's field
  list), none holds fewer than two elements (one element is the same value as its element, and none is
  no value at all), and none holds an **unforgeable** — the model's `GUnforgeable` carries a de Bruijn
  level where the wire carries the name's bytes, so `jeToPar` cannot reconstruct one and drops it. The
  unforgeable arm is the one this predicate got wrong: the `_ => true` catch-all covered it, so the
  domain admitted `JE.par [unforg, unforg]`, which decodes to `nilPar` and then encodes to nothing —
  the round trip false of the model rather than merely unproved. See the refutation below. -/
  def flatPar : JE → Bool
    | .par es => 2 ≤ es.length && flatListPar es
    | .tuple es => flatList es
    | .list es => flatList es
    | .set es => flatList es
    | .map kvs => flatListMap kvs
    | .unforg _ => false
    | _ => true

  termination_by e => sizeOf e

  /-- A `par`'s elements: none may itself be a `par`, and each must be flat. -/
  def flatListPar : List JE → Bool
    | [] => true
    | e :: es => !isPar e && flatPar e && flatListPar es
  termination_by l => sizeOf l

  def flatList : List JE → Bool
    | [] => true
    | e :: es => flatPar e && flatList es
  termination_by l => sizeOf l

  def flatListMap : List (String × JE) → Bool
    | [] => true
    | (_, v) :: kvs => flatPar v && flatListMap kvs
  termination_by l => sizeOf l

  def isPar : JE → Bool
    | .par _ => true
    | _ => false
end

/-! ## The domain predicate was too wide, and the row said the round trip held on it

`flatPar` is the round trip's *domain*: the statement is quantified over the `JE`s it accepts, so an
arm it accepts too much of is not a slack hypothesis — it is a counterexample generator. As first
written the catch-all `| _ => true` covered `.unforg`, and an unforgeable cannot survive the
round trip: `jeToPar` has no arm for one (`unforgeable`'s wire form is the name's bytes; the model
carries a de Bruijn level), so it **drops** it, and `parToJE` then writes what is left. Two of them in
one par therefore decoded to `nilPar` — whose encoding is `none`, "no value at all":

    flatPar (JE.par [unforg, unforg]) = true     -- of the old predicate
    jeToPar  (JE.par [unforg, unforg]) = some nilPar
    ((parToJE nilPar).bind jeToPar) = none

so `decode_encode` was **false** of the model, not merely owed — the same shape as law 5's axiom (C26)
and law 38's (C40), and found the same way: by asking what the statement says on a term the model
actually has. The predicate now refuses an unforgeable, which is what the module doc above already
*claimed* it did ("A `Par` containing an unforgeable is therefore outside the round-trip's domain (its
`flatPar` excludes it)") — a documentation claim that was true of the intent and false of the code.

The three facts above are kept as checked fixtures rather than prose: the first is what the fix
changes, and the second and third are the refutation of the old statement, which is why the domain had
to change rather than the statement's quantifier. -/

/-- A par holding two unforgeables — the term the old domain admitted. -/
def unforgPair : JE := JE.par [JE.unforg ⟨"UnforgPrivate", "ab"⟩, JE.unforg ⟨"UnforgPrivate", "ab"⟩]

/-- **The fix**: the predicate now refuses it. -/
theorem unforgPair_is_outside_the_domain : flatPar unforgPair = false := by
  simp [flatPar, flatListPar, isPar, unforgPair]

/-- **The refutation of the old statement**: the decoder drops both unforgeables and the encoder has
    nothing left to write, so the round trip returns *no value* for a term it was quantified over. -/
theorem unforgPair_refutes_the_old_statement :
    jeToPar unforgPair = some nilPar ∧ ((parToJE nilPar).bind jeToPar).isNone = true :=
  ⟨rfl, rfl⟩

/-! ## The round trip, proved

The merge arithmetic the row above called "still to be written out" is this: `parToJE` is the envelope
rule applied to a par's three JSON-able fields, and `parMerge` concatenates those fields **one field at
a time** — all the exprs of both sides, then all the unforgeables, then all the bundles. That is why the
proof cannot be about `parToJE` alone: the envelope *collapses* (three exprs become one `ExprPar`), so a
proof that reasons one `parToJE` at a time destroys the very list it needs to reconstruct a merge. The
decomposition below keeps the uncollapsed **item list** (`rawItems`) and splits the merge per field.

Then a `mutual` block of five statements, one per shape the decoder's recursion descends through (a
`JE`, a `par`'s element list, a collection's element list, a map's entries), and `decode_encode` is the
first of them read at the stated hypothesis.

**One Lean limitation is worth naming**, because it shaped the proof rather than merely slowing it:
`JE` is a *nested* inductive (`List JE` is a field), and the equation compiler emits unusable equation
lemmas for a `mutual` block over one — asking for `exprToJE`'s arm reports `invalid projection
⟨head_ih, tail_ih⟩.1.2.2` — while the kernel still reduces the definition at a constructor, so `rfl`
proves each arm. The arms below are therefore stated as `rfl`-proved `@[simp]` lemmas, which is kernel
reduction rather than the equation lemma. The domain predicate hit the same thing and is fixed the same
way at one remove: its block carries explicit `termination_by` clauses (well-founded rather than
structural recursion), which is enough for its equations to work. -/

/-- The items a `Par` contributes to the envelope: its three JSON-able fields, in the order
    `parToJE` reads them. `parToJE p = envelope (rawItems p)`, and the envelope collapses the list —
    which is why this is defined separately rather than inlined. -/
def rawItems (p : Par) : List JE :=
  exprsToJE p.exprs ++ unforgsToJE p.unforgeables ++ bundlesToJE p.bundles

/-- The encoder *is* the envelope rule applied to the item list. -/
theorem parToJE_eq_envelope (p : Par) : parToJE p = envelope (rawItems p) := by
  cases p; rfl

/-- The decoder's result, with the `none` case replaced by `nilPar` — `parsToPar`'s own reading. Used
    as an abbreviation because the round-trip statements mention it on every line. -/
def decoded (e : JE) : Par := (jeToPar e).getD nilPar

/-- What a `JE` that is **not** a `par` decodes to: one item's worth of content, and no unforgeable or
    bundle — the decoder has no arm that produces either, which is why the domain refuses a term
    containing one. The field-level form (rather than "the item list is `[e]`") is what the merge
    arithmetic needs: `parMerge` concatenates each field separately, so a merge's item list is only
    additive when the sides' unforgeable and bundle lists are empty. -/
def DecodesTo (p : Par) (e : JE) : Prop :=
  exprsToJE p.exprs = [e] ∧ p.unforgeables = [] ∧ p.bundles = []

/-- The pieces of a **merged** par: its expr items are the element list itself (not `[e]`, and not
    `[.par es]` — the empty list's merge is `Nil`, whose item list is empty, which is why this is
    stated over the list rather than as `DecodesTo` at `.par es`) and the two fields the decoder never
    fills stay empty. -/
def MergedInto (p : Par) (es : List JE) : Prop :=
  exprsToJE p.exprs = es ∧ p.unforgeables = [] ∧ p.bundles = []

/-! ### The encoder's arms, as `rfl` equations (see the note above) -/

@[simp] theorem exprToJE_bool (b : Bool) : exprToJE (.ground (.bool b)) = some (.bool b) := rfl
@[simp] theorem exprToJE_int (n : Int) : exprToJE (.ground (.int n)) = some (.int n) := rfl
@[simp] theorem exprToJE_str (l : List Nat) :
    exprToJE (.ground (.str l)) = some (.str (codePointsToString l)) := rfl
@[simp] theorem exprToJE_uri (l : List Nat) :
    exprToJE (.ground (.uri l)) = some (.uri (codePointsToString l)) := rfl
@[simp] theorem exprToJE_bytes (l : List Nat) :
    exprToJE (.ground (.bytes l)) = some (.bytes l) := rfl
@[simp] theorem exprToJE_etuple (ps : List Par) :
    exprToJE (.etuple ps) = some (.tuple (parsToJE ps)) := rfl
@[simp] theorem exprToJE_elist (ps : List Par) (r : Option Var) :
    exprToJE (.elist ps r) = some (.list (parsToJE ps)) := rfl
@[simp] theorem exprToJE_eset (ps : List Par) (r : Option Var) :
    exprToJE (.eset ps r) = some (.set (parsToJE ps)) := rfl
@[simp] theorem exprToJE_emap (kvs : List (Par × Par)) (r : Option Var) :
    exprToJE (.emap kvs r) = some (.map (mapKvsToJE kvs)) := rfl

/-! ### The decoder's arms, likewise -/

@[simp] theorem jeToPar_par (es : List JE) : jeToPar (.par es) = some (parsToPar es) := rfl
@[simp] theorem jeToPar_tuple (es : List JE) :
    jeToPar (.tuple es) = some (one (.etuple (parsToParList es))) := rfl
@[simp] theorem jeToPar_list (es : List JE) :
    jeToPar (.list es) = some (one (.elist (parsToParList es) none)) := rfl
@[simp] theorem jeToPar_set (es : List JE) :
    jeToPar (.set es) = some (one (.eset (parsToParList es) none)) := rfl
@[simp] theorem jeToPar_map (kvs : List (String × JE)) :
    jeToPar (.map kvs) = some (one (.emap (kvsToPars kvs) none)) := rfl
@[simp] theorem jeToPar_bool (b : Bool) : jeToPar (.bool b) = some (one (.ground (.bool b))) := rfl
@[simp] theorem jeToPar_int (n : Int) : jeToPar (.int n) = some (one (.ground (.int n))) := rfl
@[simp] theorem jeToPar_str (s : String) :
    jeToPar (.str s) = some (one (.ground (.str (codePointsOf s)))) := rfl
@[simp] theorem jeToPar_uri (s : String) :
    jeToPar (.uri s) = some (one (.ground (.uri (codePointsOf s)))) := rfl
@[simp] theorem jeToPar_bytes (l : List Nat) :
    jeToPar (.bytes l) = some (one (.ground (.bytes l))) := rfl
@[simp] theorem jeToPar_unforg (u : JUnforg) : jeToPar (.unforg u) = none := rfl

@[simp] theorem parsToParList_nil : parsToParList ([] : List JE) = [] := rfl
@[simp] theorem parsToParList_cons (j : JE) (js : List JE) :
    parsToParList (j :: js) = (jeToPar j).toList ++ parsToParList js := rfl
@[simp] theorem parsToPar_nil : parsToPar ([] : List JE) = nilPar := rfl
@[simp] theorem parsToPar_cons (j : JE) (js : List JE) :
    parsToPar (j :: js) = parMerge (decoded j) (parsToPar js) := rfl

@[simp] theorem exprsToJE_nil : exprsToJE ([] : List Expr) = [] := rfl
@[simp] theorem exprsToJE_cons (e : Expr) (es : List Expr) :
    exprsToJE (e :: es) = (match exprToJE e with | some j => [j] | none => []) ++ exprsToJE es := rfl

@[simp] theorem unforgsToJE_nil : unforgsToJE ([] : List GUnforgeable) = [] := rfl
@[simp] theorem bundlesToJE_nil : bundlesToJE ([] : List Bundle) = [] := rfl

@[simp] theorem exprsToJE_append (l l' : List Expr) :
    exprsToJE (l ++ l') = exprsToJE l ++ exprsToJE l' := by
  induction l with
  | nil => simp [exprsToJE]
  | cons e es ih => simp [exprsToJE, ih]

@[simp] theorem unforgsToJE_append (l l' : List GUnforgeable) :
    unforgsToJE (l ++ l') = unforgsToJE l ++ unforgsToJE l' := by
  induction l with
  | nil => simp [unforgsToJE]
  | cons u us ih => simp [unforgsToJE, ih]

@[simp] theorem bundlesToJE_append (l l' : List Bundle) :
    bundlesToJE (l ++ l') = bundlesToJE l ++ bundlesToJE l' := by
  induction l with
  | nil => simp [bundlesToJE]
  | cons b bs ih => simp [bundlesToJE, ih]

/-- `rawItems` of a merge, **per field**: the merge concatenates each of the three fields separately,
    so this is what the item list of `p | q` is — not `rawItems p ++ rawItems q`, which would put
    `p`'s bundles before `q`'s exprs and is false. This is the fact that forces the elements' empty
    unforgeable and bundle lists into the induction. -/
theorem rawItems_parMerge (p q : Par) :
    rawItems (parMerge p q)
      = exprsToJE (p.exprs ++ q.exprs)
        ++ (unforgsToJE (p.unforgeables ++ q.unforgeables)
            ++ bundlesToJE (p.bundles ++ q.bundles)) := by
  cases p; cases q
  simp [rawItems, parMerge, List.append_assoc]

/-- The envelope's collapse rule: two or more items become a `par`. -/
theorem envelope_of_two_le {es : List JE} (h : 2 ≤ es.length) : envelope es = some (.par es) := by
  cases es with
  | nil => simp at h
  | cons a t =>
    cases t with
    | nil => simp at h
    | cons b t => rfl

/-- A par whose item list is the singleton `e` encodes back to `e`: `rawItems` written out and the
    envelope's one-item rule. Every arm of the round trip below is this step read at its shape's
    `DecodesTo` fact, so the work is in producing those facts, not in the envelope. -/
theorem parToJE_of_decodesTo {p : Par} {e : JE} (h : DecodesTo p e) : parToJE p = some e := by
  obtain ⟨h1, h2, h3⟩ := h
  have h2' : unforgsToJE p.unforgeables = [] := by rw [h2]; rfl
  have h3' : bundlesToJE p.bundles = [] := by rw [h3]; rfl
  rw [parToJE_eq_envelope, rawItems, h1, h2', h3']
  rfl

/-- The decoder accepts every `flatPar` JE. This is the leaf the whole development rests on, and the
    reason the *domain* had to change rather than the theorem's quantifier: `.unforg` is the only arm
    the decoder refuses, so a predicate that admits one admits a term with no decoding at all. -/
theorem jeToPar_isSome (e : JE) (hf : flatPar e = true) : (jeToPar e).isSome = true := by
  cases e with
  | unforg u => simp [flatPar] at hf
  | par es => rfl
  | tuple es => rfl
  | list es => rfl
  | set es => rfl
  | map kvs => rfl
  | bool b => rfl
  | int n => rfl
  | str s => rfl
  | uri s => rfl
  | bytes l => rfl

/-- A `Some` is its own `toList` at `getD`: the bridge between the decoder's `toList` (which drops a
    `none`) and `parsToPar`'s `getD` (which keeps a default), used wherever a list of elements is
    walked. -/
theorem Option.toList_eq_getD {α : Type} (o : Option α) (d : α) (h : o.isSome = true) :
    o.toList = [o.getD d] := by
  cases o with
  | none => simp at h
  | some a => rfl

/-- The model's code points round-trip: what makes `ExprString`/`ExprUri` a lossless leaf. -/
@[simp] theorem codePointsToString_codePointsOf (s : String) :
    codePointsToString (codePointsOf s) = s := by
  simp only [codePointsToString, codePointsOf, List.map_map, Function.comp_def, Char.ofNat_toNat,
    List.map_id']
  exact String.asString_toList s

/-- A string-keyed map key, as the decoder builds it, reads back to the same string. -/
theorem keyToJE_of_str (l : List Nat) :
    keyToJE (one (.ground (.str l))) = some (codePointsToString l) := by
  simp [keyToJE, one]

/-- `parsToJE` as a `bind`: the encoder's element-wise drop is `toList`. -/
theorem parsToJE_eq_bind (ps : List Par) :
    parsToJE ps = ps.bind (fun p => (parToJE p).toList) := by
  induction ps with
  | nil => rfl
  | cons p ps ih =>
    have h : (match parToJE p with | some j => [j] | none => []) = (parToJE p).toList := by
      cases parToJE p <;> rfl
    simp [parsToJE, h, ih]

/-- `parsToJE` distributes over append. -/
theorem parsToJE_append (ps qs : List Par) : parsToJE (ps ++ qs) = parsToJE ps ++ parsToJE qs := by
  rw [parsToJE_eq_bind, parsToJE_eq_bind, parsToJE_eq_bind, List.bind_append]

/-! ### The five statements the round trip decomposes into

One per shape the decoder's recursion descends through. They are mutual because the decoder is: a
`par`'s element list holds JEs, whose own round trip goes back through the element lemmas. -/
mutual
  /-- A `flatPar` JE encodes back to itself: decode then encode is the identity on the domain. -/
  theorem parToJE_getD_round : (e : JE) → flatPar e = true → parToJE (decoded e) = some e
    | .unforg u, hf => by simp [flatPar] at hf
    | .par es, hf => by
        simp only [flatPar, Bool.and_eq_true] at hf
        obtain ⟨hlen, hpar⟩ := hf
        have htwo : 2 ≤ es.length := by simpa using hlen
        obtain ⟨he, hu, hb⟩ := parsToPar_merged es hpar
        rw [decoded, jeToPar_par, Option.getD_some, parToJE_eq_envelope, rawItems, he, hu, hb]
        simp [List.append_nil, envelope_of_two_le htwo]
    | .tuple es, hf => parToJE_of_decodesTo (decodesTo_leaf (.tuple es) hf (by rfl))
    | .list es, hf => parToJE_of_decodesTo (decodesTo_leaf (.list es) hf (by rfl))
    | .set es, hf => parToJE_of_decodesTo (decodesTo_leaf (.set es) hf (by rfl))
    | .map kvs, hf => parToJE_of_decodesTo (decodesTo_leaf (.map kvs) hf (by rfl))
    | .bool b, hf => parToJE_of_decodesTo (decodesTo_leaf (.bool b) hf (by rfl))
    | .int n, hf => parToJE_of_decodesTo (decodesTo_leaf (.int n) hf (by rfl))
    | .str s, hf => parToJE_of_decodesTo (decodesTo_leaf (.str s) hf (by rfl))
    | .uri s, hf => parToJE_of_decodesTo (decodesTo_leaf (.uri s) hf (by rfl))
    | .bytes l, hf => parToJE_of_decodesTo (decodesTo_leaf (.bytes l) hf (by rfl))

  /-- What a `flatPar` JE that is not a `par` decodes to: the item list is the singleton itself, and
      neither of the fields the decoder never fills is filled. The second and third conjuncts are
      load-bearing — they are what makes a merge's item list additive (see `rawItems_parMerge`). -/
  theorem decodesTo_leaf : (e : JE) → flatPar e = true → isPar e = false → DecodesTo (decoded e) e
    | .par es, _, hp => by simp [isPar] at hp
    | .unforg u, hf, _ => by simp [flatPar] at hf
    | .tuple es, hf, _ => by
        simp only [flatPar] at hf
        refine ⟨?_, rfl, rfl⟩
        simp only [decoded, one, Par.exprs, jeToPar, Option.getD_some, exprsToJE_cons,
          exprsToJE_nil, exprToJE_etuple, List.append_nil, parsToJE_parsToParList es hf]
    | .list es, hf, _ => by
        simp only [flatPar] at hf
        refine ⟨?_, rfl, rfl⟩
        simp only [decoded, one, Par.exprs, jeToPar, Option.getD_some, exprsToJE_cons,
          exprsToJE_nil, exprToJE_elist, List.append_nil, parsToJE_parsToParList es hf]
    | .set es, hf, _ => by
        simp only [flatPar] at hf
        refine ⟨?_, rfl, rfl⟩
        simp only [decoded, one, Par.exprs, jeToPar, Option.getD_some, exprsToJE_cons,
          exprsToJE_nil, exprToJE_eset, List.append_nil, parsToJE_parsToParList es hf]
    | .map kvs, hf, _ => by
        simp only [flatPar] at hf
        refine ⟨?_, rfl, rfl⟩
        simp only [decoded, one, Par.exprs, jeToPar, Option.getD_some, exprsToJE_cons,
          exprsToJE_nil, exprToJE_emap, List.append_nil, mapKvsToJE_kvsToPars kvs hf]
    | .bool b, _, _ => ⟨by simp [decoded, one, Par.exprs], rfl, rfl⟩
    | .int n, _, _ => ⟨by simp [decoded, one, Par.exprs], rfl, rfl⟩
    | .str s, _, _ => ⟨by simp [decoded, one, Par.exprs], rfl, rfl⟩
    | .uri s, _, _ => ⟨by simp [decoded, one, Par.exprs], rfl, rfl⟩
    | .bytes l, _, _ => ⟨by simp [decoded, one, Par.exprs], rfl, rfl⟩

  /-- A `par`'s elements, merged, contribute exactly the element list: each element contributes one
      item (`decodesTo_leaf`), the merge arithmetic puts them in order, and the **empty** unforgeable
      and bundle fields are what make the concatenation the element list rather than an interleaving
      of fields. -/
  theorem parsToPar_merged : (es : List JE) → flatListPar es = true → MergedInto (parsToPar es) es
    | [], _ => ⟨by simp [parsToPar, exprsToJE], rfl, rfl⟩
    | j :: js, hf => by
        simp only [flatListPar, Bool.and_eq_true] at hf
        obtain ⟨⟨hnotpar, hflatj⟩, hflatjs⟩ := hf
        have hnot : isPar j = false := by simpa using hnotpar
        obtain ⟨hj_e, hj_u, hj_b⟩ := decodesTo_leaf j hflatj hnot
        obtain ⟨hs_e, hs_u, hs_b⟩ := parsToPar_merged js hflatjs
        refine ⟨?_, ?_, ?_⟩
        · rw [parsToPar_cons]
          have h1 : (parMerge (decoded j) (parsToPar js)).exprs
              = (decoded j).exprs ++ (parsToPar js).exprs := rfl
          rw [h1, exprsToJE_append, hj_e, hs_e, List.singleton_append]
        · rw [parsToPar_cons]
          have h1 : (parMerge (decoded j) (parsToPar js)).unforgeables
              = (decoded j).unforgeables ++ (parsToPar js).unforgeables := rfl
          rw [h1, hj_u, hs_u]
          rfl
        · rw [parsToPar_cons]
          have h1 : (parMerge (decoded j) (parsToPar js)).bundles
              = (decoded j).bundles ++ (parsToPar js).bundles := rfl
          rw [h1, hj_b, hs_b]
          rfl

  /-- A flat list of JEs round-trips element-wise: encoding the decoded elements gives the list back.
      This is what a collection form (`ExprTuple`/`ExprList`/`ExprSet`) needs, since the decoder keeps
      its elements as separate `Par`s. -/
  theorem parsToJE_parsToParList :
      (es : List JE) → flatList es = true → parsToJE (parsToParList es) = es
    | [], _ => rfl
    | j :: js, hf => by
        simp only [flatList, Bool.and_eq_true] at hf
        obtain ⟨hflatj, hflatjs⟩ := hf
        have htl : (jeToPar j).toList = [decoded j] :=
          Option.toList_eq_getD _ _ (jeToPar_isSome j hflatj)
        have hround : parToJE (decoded j) = some j := parToJE_getD_round j hflatj
        rw [parsToParList_cons, htl, parsToJE_append, parsToJE_eq_bind, List.bind_cons, List.bind_nil,
          hround, Option.toList_some, List.append_nil, parsToJE_parsToParList js hflatjs,
          List.singleton_append]

  /-- A flat map round-trips entry-wise: keys are strings by construction and values round-trip as
      values, so the drop-on-`none` arms of both directions never fire. -/
  theorem mapKvsToJE_kvsToPars :
      (kvs : List (String × JE)) → flatListMap kvs = true → mapKvsToJE (kvsToPars kvs) = kvs
    | [], _ => rfl
    | (k, v) :: kvs, hf => by
        simp only [flatListMap, Bool.and_eq_true] at hf
        obtain ⟨hf_v, hf_rest⟩ := hf
        obtain ⟨pv, hpv⟩ := Option.isSome_iff_exists.mp (jeToPar_isSome v hf_v)
        have hround : parToJE pv = some v := by
          have h := parToJE_getD_round v hf_v
          rwa [decoded, hpv, Option.getD_some] at h
        have hkey : keyToJE (one (.ground (.str (codePointsOf k)))) = some k := by
          simpa using keyToJE_of_str (codePointsOf k)
        simp [kvsToPars, hpv, mapKvsToJE, hkey, hround, mapKvsToJE_kvsToPars kvs hf_rest]
end

/-- **Law 42's core: decode what the API encoded.** If a `Par` is what the API's decoder returns for a
`JE`, then the API's encoder writes that `JE` back and the decoder reads the same `Par` again — which
is what a client depends on when it reads a value from one endpoint and sends it to another.

Proved over the **corrected** domain: the hypothesis is `flatPar`, which refuses an unforgeable
precisely because the decoder cannot reconstruct one (`unforgPair_refutes_the_old_statement`). -/
theorem decode_encode (e : JE) (p : Par) (h : jeToPar e = some p) (hf : flatPar e = true) :
    (parToJE p).bind jeToPar = some p := by
  have h1 := parToJE_getD_round e hf
  rw [decoded, h, Option.getD_some] at h1
  rw [h1]
  simp [h]

end Rchain
