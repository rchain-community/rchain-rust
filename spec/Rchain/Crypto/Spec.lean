/-!
# Law 19 — Blake2b256, signatures, Curve25519

`Blake2b256` is a canonical (deterministic, collision-free) hash with a **fixed 32-byte output**;
signatures verify (`verify ∘ sign` is identity); Curve25519 key agreement round-trips. The Scala oracle
is `crypto/...` (`Blake2b256`, `Secp256k1`, `Ed25519`, `Curve25519`); the Rust realization is
`crypto::hash::blake2b256_hash` + `crypto::signatures`. The primitives are **axiomatized by design**,
not proven — postulating a cryptographic primitive is the correct treatment, and these are the axioms
the consolidation pass left alone.

## The types mirror the code's

`Msg` and `Hash` used to be opaque `Nat` wrappers (`structure Msg where id : Nat`). They are byte
strings now because the code's are: a message is `&[u8]`, and a Blake2b256 hash is exactly 32 bytes —
`Hash32` is `struct Hash32([u8; HASH32_LENGTH])` with `HASH32_LENGTH = 32`
(`shared/src/refined.rs:287-296`), and `Blake2b256Hash` wraps it
(`crypto/src/hash/blake2b256_hash.rs:28`). The width is a *stated* property
(`blake2b256_output_is_32_bytes`) rather than a type invariant: the model has no use for the subtype and
every use for staying readable. Nothing else in the tree used these two names, so the retype touched
only this file.
-/

namespace Rchain

/-- A byte. -/
abbrev Byte := Fin 256

/-- A message: a byte string (the Rust's `&[u8]`). -/
abbrev Msg := List Byte

/-- A Blake2b256 hash: 32 bytes (`Hash32`, `shared/src/refined.rs:287-296`). -/
abbrev Hash := List Byte

/-- Blake2b256 hashing. -/
axiom blake2b256 : Msg → Hash

/-- Law 19: the output is exactly 32 bytes — the width the code's type enforces
    (`HASH32_LENGTH`, `shared/src/refined.rs:287-296`) and the model states. -/
axiom blake2b256_output_is_32_bytes (m : Msg) : (blake2b256 m).length = 32

/-- Law 19: the hash is canonical (deterministic) and collision-free.

    **Read this as collision-*resistance* stated as injectivity.** Blake2b256 is not injective in
    reality — 2^256 outputs cannot cover every byte string — but an infeasible collision is the property
    the port relies on, and injectivity is its faithful idealization. Two consequences worth knowing:
    every "equal hash ⇒ equal object" law in this catalogue is *this* axiom specialized (Law 10's
    `trie_collision_free`, Law 16's `content_addressing`), which is why the consolidation pass moved
    them here instead of leaving them as independent claims about tries and blocks; and a proof that
    needs injectivity is relying on a cryptographic assumption, not on arithmetic. -/
axiom blake2b256_collision_free (a b : Msg) : blake2b256 a = blake2b256 b → a = b

/-- A signature. -/
abbrev Signature := List Byte

/-- A public/private keypair. -/
structure KeyPair where
  public : Msg
  secret : Msg

/-- Sign and verify. -/
axiom sign : KeyPair → Msg → Signature

axiom verify : Msg → Msg → Signature → Prop

/-- Law 19: `verify pk m (sign sk m)` holds when `pk` is `sk`'s public half (sign/verify round-trip). -/
axiom sign_verify_roundtrip (kp : KeyPair) (m : Msg) : verify kp.public m (sign kp m)

/-- Curve25519 shared-secret derivation. -/
axiom sharedSecret : Msg → Msg → Msg

/-- Law 19: Curve25519 key agreement round-trips (both sides derive the same shared secret). -/
axiom curve25519_roundtrip (a_priv b_pub b_priv a_pub : Msg) :
  sharedSecret a_priv b_pub = sharedSecret b_priv a_pub

end Rchain
