import Rchain.Par

/-!
# Law 42 — the rho-value JSON round-trip, and the envelope rule

The web API exposes rholang data as a JSON-ish tree (`node/src/api/rho_expr.rs`'s `RhoExpr`, which
clients read as `{"ExprInt":42}`, `{"ExprBytes":"deadbeef"}`, `{"ExprPar":[…]}`). Two things about that
conversion were a comment rather than a theorem, and both are AUDIT C16's family — a shape a client
depends on, pinned by nothing:

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
  /-- The wire text of a `JE`, in the shape `serde`'s derived representation of `RhoExpr` has (compact,
  externally tagged: `{"ExprInt":42}`). The corpus carries this text, and the Rust consumer compares
  the node's own JSON to it. -/
  def render : JE → String
    | .par es => "{\"ExprPar\":" ++ bracket (renderItems es) ++ "}"
    | .tuple es => "{\"ExprTuple\":" ++ bracket (renderItems es) ++ "}"
    | .list es => "{\"ExprList\":" ++ bracket (renderItems es) ++ "}"
    | .set es => "{\"ExprSet\":" ++ bracket (renderItems es) ++ "}"
    | .map kvs => "{\"ExprMap\":" ++ bracket (renderKvs kvs) ++ "}"
    | .bool b => "{\"ExprBool\":" ++ (if b then "true" else "false") ++ "}"
    | .int n => "{\"ExprInt\":" ++ toString n ++ "}"
    | .str s => "{\"ExprString\":" ++ jsonStr s ++ "}"
    | .uri s => "{\"ExprUri\":" ++ jsonStr s ++ "}"
    | .bytes l => "{\"ExprBytes\":" ++ jsonStr (hexEncode l) ++ "}"
    | .unforg u => "{\"ExprUnforg\":{" ++ jsonStr u.tag ++ ":" ++ jsonStr u.hex ++ "}}"

  def renderItems : List JE → List String
    | [] => []
    | e :: es => render e :: renderItems es

  def renderKvs : List (String × JE) → List String
    | [] => []
    | (k, v) :: kvs => ("[" ++ jsonStr k ++ "," ++ render v ++ "]") :: renderKvs kvs
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
  list) and none holds fewer than two elements (one element is the same value as its element, and none
  is no value at all). Everything else is flat by construction. -/
  def flatPar : JE → Bool
    | .par es => 2 ≤ es.length && flatListPar es
    | .tuple es => flatList es
    | .list es => flatList es
    | .set es => flatList es
    | .map kvs => flatListMap kvs
    | _ => true

  /-- A `par`'s elements: none may itself be a `par`, and each must be flat. -/
  def flatListPar : List JE → Bool
    | [] => true
    | e :: es => !isPar e && flatPar e && flatListPar es

  def flatList : List JE → Bool
    | [] => true
    | e :: es => flatPar e && flatList es

  def flatListMap : List (String × JE) → Bool
    | [] => true
    | (_, v) :: kvs => flatPar v && flatListMap kvs

  def isPar : JE → Bool
    | .par _ => true
    | _ => false
end

/-- **Law 42's core: decode what the API encoded.** If a `Par` is what the API's decoder returns for a
`JE`, then the API's encoder writes that `JE` back and the decoder reads the same `Par` again — which
is what a client depends on when it reads a value from one endpoint and sends it to another.

**Owed** and named rather than assumed quietly: the induction runs over the flat fields, and the merge
arithmetic it needs (`parMerge`'s field concatenation commuting with the encode) is the part still to
be written out. The corpus checks the statement against the node's own codec meanwhile, case by case,
with the model's `JE.render` as the independent witness. -/
axiom decode_encode (e : JE) (p : Par) (h : jeToPar e = some p) (hf : flatPar e = true) :
    (parToJE p).bind jeToPar = some p

end Rchain
