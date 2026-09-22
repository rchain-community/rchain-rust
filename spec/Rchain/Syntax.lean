import Mathlib.Data.String.Basic

/-!
# Rholang leaf types (M2 gate)

Ground scalars and variables, shared by the flat `Par` ADT in `Rchain.Par`. The Phase-0 *binary*
`Proc` fragment has been removed (its Law 1 proof lives in git history as a regression anchor); the
real `Par` is the flat record defined in `Rchain.Par`.

`Ground` and `Var` derive `LinearOrder` so that their canonical comparison is the constructor-
declaration order (`bool < int < str` (code points); `bound < free < wildcard`), which is *exactly* the Phase-0
`cmpGround`/`cmpVar` — but now lawful via Mathlib's `cmp` machinery with no axioms.

Binders use de Bruijn *levels*, matching the Scala `Var` (`bound_var`/`free_var`/`wildcard`).
-/

namespace Rchain

/-- Ground scalar values: the five `Ground` instances the protobuf has (`GBool`, `GInt`, `GString`,
`GUri`, `GByteArray`). `uri` and `bytes` were missing until law 42: the model could not *hold* the two
that the JSON layer's whole subject turns on (`ExprUri`, `ExprBytes`), so the reply-shape and envelope
laws could not be stated for them (AUDIT C28). -/
inductive Ground where
  | bool (b : Bool)
  | int  (n : Int)
  | str  (l : List Nat)  -- Unicode code points
  | uri  (l : List Nat)  -- Unicode code points
  | bytes (l : List Nat) -- raw bytes
deriving BEq, Ord, DecidableEq

/-- Variables, represented as de Bruijn levels (bound/free) or a wildcard. -/
inductive Var where
  | bound (level : Nat)  -- bound_var
  | free  (level : Nat)  -- free_var
  | wildcard             -- `_`
deriving BEq, Ord, DecidableEq

end Rchain
