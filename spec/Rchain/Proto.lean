import Rchain.Crypto.Spec
/-!
# protobuf's wire format, enough of it to encode a block body

The primitives `Rchain/Casper/Validate.lean`'s block-body encoder is built from: the base-128 **varint**
and its inverse, the two field shapes (`varint`-valued and length-delimited), and the 64-bit narrowing
the port's `int64` fields impose on the model's `Nat`s.

The load-bearing lemma is `decodeVarint_varint`: a varint is **self-delimiting**, so a concatenation of
them is unambiguous without any length prefix — which is the whole reason a body's fields can be read
back one at a time, and why the encoder's injectivity is a proof rather than an assumption.

Nothing here is `decide`d or `rfl`-proved, and that is deliberate: `varint` recurses on a *value*
(`n / 128`), so it is a well-founded definition and not kernel-reducible.

`RSpace/Merkle.lean` has its own `byteOf` (the trie encoder's), which is why this module does not
define one: the body encoder writes its bytes through `varint`, and a second `Rchain.byteOf` — the
same name, the same body, for no caller — made `Rchain.lean` fail to import **both** modules.
-/

namespace Rchain

/-- protobuf's base-128 varint: seven bits per byte, the high bit set while another byte follows. -/
def varint (n : Nat) : Msg :=
  if h : n < 128 then [⟨n, by omega⟩]
  else ⟨n % 128 + 128, by omega⟩ :: varint (n / 128)
termination_by n
decreasing_by exact Nat.div_lt_self (by omega) (by decide)

/-- The inverse of `varint` for one field: read bytes while the high bit is set, then the final byte.
    Returns the value and the bytes that follow the field. -/
def decodeVarint : Msg → Option (Nat × Msg)
  | [] => none
  | b :: rest =>
      if (b : Nat) < 128 then some ((b : Nat), rest)
      else (decodeVarint rest).bind (fun (n, r) => some ((b : Nat) - 128 + 128 * n, r))

/-- One step of `decodeVarint` on a byte that carries the continuation bit. -/
theorem decodeVarint_cons_of_ge {b : Byte} {rest : Msg} (hb : ¬ (b : Nat) < 128) (n : Nat)
    {r : Msg} (hn : decodeVarint rest = some (n, r)) :
    decodeVarint (b :: rest) = some ((b : Nat) - 128 + 128 * n, r) := by
  simp only [decodeVarint, if_neg hb, hn]
  rfl

/-- **A varint is self-delimiting** — the bytes determine both the value and where the next field
    begins. -/
theorem decodeVarint_varint (n : Nat) (r : Msg) : decodeVarint (varint n ++ r) = some (n, r) := by
  induction n using varint.induct with
  | case1 x hx =>
    rw [varint.eq_def, dif_pos hx]
    simp only [List.singleton_append, decodeVarint, hx, ↓reduceIte]
    rfl
  | case2 x hx ih =>
    have hmod : ¬ (x % 128 + 128 : Nat) < 128 := by omega
    rw [varint.eq_def, dif_neg hx]
    simp only [List.cons_append]
    rw [decodeVarint_cons_of_ge hmod (x / 128) ih]
    rw [show (x % 128 + 128 : Nat) - 128 + 128 * (x / 128) = x from by
      have := Nat.mod_add_div x 128
      omega]

/-- A `varint` field with its protobuf tag: `(fieldNumber << 3) | wireType`, then the value. -/
def varintField (tag n : Nat) : Msg := varint tag ++ varint n

/-- A length-delimited field: the payload's length as a varint, then the payload. -/
def bytesField (tag : Nat) (payload : Msg) : Msg := varint tag ++ varint payload.length ++ payload

/-- The port's `int64` fields are 64 bits wide and the model's are `Nat`: writing such a field means
    writing its 64-bit value. -/
def int64 (n : Nat) : Nat := n % 2 ^ 64

/-- **A `Canonical` bound is what makes a value recoverable from its own bytes.** Two numbers below
    `2^63` with the same residue mod `2^64` are the same number; without the bound they need not be. -/
theorem int64_eq_of_lt {x y : Nat} (hx : x < 2 ^ 63) (hy : y < 2 ^ 63)
    (h : int64 x = int64 y) : x = y := by
  have hx' : x % 2 ^ 64 = x := Nat.mod_eq_of_lt (by omega)
  have hy' : y % 2 ^ 64 = y := Nat.mod_eq_of_lt (by omega)
  simp only [int64] at h
  rwa [hx', hy'] at h

end Rchain
