/-!
# Law 49 — the gas a matched op costs, and why the refund's *order* is the rule

`rholang/src/storage.rs` charges a produce's or consume's storage up front and, when the op **matches**,
refunds what the match consumed: the continuation's consume storage, and the produce storage of every
datum the op removed (`ChargingRSpace.scala:105-127` — `refundForConsume` and
`refundForRemovingProduces`, charged as *negative* `Cost`s). The port did not; it recorded the gap as a
"safe over-charge" (safe in that no deployer is ever under-charged, which is why it survived a look).

What makes the refunds more than arithmetic is **where they land**. `CostAccounting::charge` refuses a
step that would take the balance negative, so what a deploy needs is not the *sum* of its charges but
the largest **prefix** total — a refund credited before a later charge lowers that peak, and one
credited after it does not. The Scala charges both refunds immediately after the match and *before* the
event and COMM costs; so does the port now, and
`a_matched_produce_refunds_its_storage_before_the_event_costs` pins both halves: the total (which is
order-independent) and a balance lying between the two peaks (which is not).

**What is proved here** is the instance the charge sequence has — two storage charges and two refunds,
refunds first (`peak_refunds_first`), plus the fact that the two orders cost the same in total
(`itotal_refunds_first`), which is exactly why the peak and not the sum is what the placement changes.
The fully general form — "moving any non-positive charge earlier past a charge cannot raise the peak" —
is the same argument iterated, and is *not* stated: the iteration needs a permutation lemma over `List`
that carries no further content, and this module would rather say what it proves than prove a general
statement no caller uses.
-/

namespace Rchain

/-- The running total after each charge, in order. `Int`, because a refund is a negative charge and the
whole point of the module is that the *placement* of those negatives matters. -/
def prefixes : List Int → List Int
  | [] => []
  | a :: rest => a :: (prefixes rest).map (· + a)

/-- The smallest balance a deploy can start with and pay this sequence of charges, given that each step
is refused once the balance would go negative: the largest running total, or zero if nothing is ever
positive. This is what `CostAccounting::charge` enforces one step at a time. -/
def peak (l : List Int) : Int := (0 :: prefixes l).foldr max 0

/-- The sequence's total, which the refunds change if and only if they are *missing*. -/
def itotal : List Int → Int
  | [] => 0
  | a :: rest => a + itotal rest

/-- The prefixes of the four-charge shape, unfolded: this is what turns the instance below from a
statement about lists into a comparison of two `max` chains over four named running totals. -/
theorem prefixes_four (a b c d : Int) :
    prefixes [a, b, c, d] = [a, a + b, a + b + c, a + b + c + d] := by
  simp only [prefixes, List.map_cons, List.map_nil, List.foldr_nil, List.foldr_cons]
  simp only [Int.add_comm, Int.add_left_comm, Int.add_assoc]

/-- The peak as the `max` chain it is, for the four-charge shape — so the instance below is a
comparison of members rather than an induction. (`peak` carries a zero seed *and* is a fold with a zero
seed, which is why the innermost `max … 0` is there: neither is decoration, both are what make the peak
a lower bound a balance can meet.) -/
theorem peak_four (a b c d : Int) :
    peak [a, b, c, d] =
      max 0 (max a (max (a + b) (max (a + b + c) (max (a + b + c + d) 0)))) := by
  simp only [peak, prefixes_four, List.foldr_cons, List.foldr_nil, List.map_cons, List.map_nil]

/-- **The Scala's order, at the shape a charge sequence has**: two storage charges and two refunds,
refunds first. The two sequences have the *same total* (`itotal_refunds_first`) — so what the order
changes is the peak, which is the balance a deploy must start with. This is the statement
`a_matched_produce_refunds_its_storage_before_the_event_costs` builds against the Rust: a balance
between the two peaks completes the refunds-first order and fails the other.

The hypotheses are the charges' own: `s` non-negative (a storage cost) and `r` non-positive (a refund).
Both are needed — with a negative `s` the comparison can go the other way. -/
theorem peak_refunds_first (s₁ s₂ r₁ r₂ : Int)
    (hs₁ : 0 ≤ s₁) (hs₂ : 0 ≤ s₂) (hr₁ : r₁ ≤ 0) (hr₂ : r₂ ≤ 0) :
    peak [r₁, r₂, s₁, s₂] ≤ peak [s₁, s₂, r₁, r₂] := by
  rw [peak_four, peak_four]
  -- The right-hand chain, spelled out once: every member of the left-hand chain is below it.
  -- `Int.max_le` splits each `max` into its two arguments, so the proof is five such splits.
  have hzero :
      0 ≤ max 0 (max s₁ (max (s₁ + s₂) (max (s₁ + s₂ + r₁) (max (s₁ + s₂ + r₁ + r₂) 0)))) :=
    Int.le_max_left 0 _
  have hS :
      s₁ + s₂ ≤ max 0 (max s₁ (max (s₁ + s₂) (max (s₁ + s₂ + r₁) (max (s₁ + s₂ + r₁ + r₂) 0)))) :=
    Int.le_trans (Int.le_max_left (s₁ + s₂) _)
      (Int.le_trans (Int.le_max_right s₁ _) (Int.le_max_right 0 _))
  have hSR :
      s₁ + s₂ + r₁ + r₂ ≤ max 0 (max s₁ (max (s₁ + s₂) (max (s₁ + s₂ + r₁) (max (s₁ + s₂ + r₁ + r₂) 0)))) :=
    Int.le_trans (Int.le_max_left (s₁ + s₂ + r₁ + r₂) 0)
      (Int.le_trans (Int.le_max_right (s₁ + s₂ + r₁) _)
        (Int.le_trans (Int.le_max_right (s₁ + s₂) _)
          (Int.le_trans (Int.le_max_right s₁ _) (Int.le_max_right 0 _))))
  refine Int.max_le.mpr ⟨hzero, Int.max_le.mpr ⟨Int.le_trans hr₁ hzero, ?_⟩⟩
  refine Int.max_le.mpr ⟨Int.le_trans (by omega : r₁ + r₂ ≤ 0) hzero, ?_⟩
  refine Int.max_le.mpr ⟨Int.le_trans (by omega : r₁ + r₂ + s₁ ≤ s₁ + s₂) hS, ?_⟩
  -- The last running total is the *same number* in both orders (`s₁ + s₂ + r₁ + r₂`), so it is the one
  -- place the orders cannot differ — and the innermost zero is in both chains.
  exact Int.max_le.mpr ⟨Int.le_trans (by omega : r₁ + r₂ + s₁ + s₂ ≤ s₁ + s₂ + r₁ + r₂) hSR, hzero⟩

/-- The two orders cost the same in total. This is the lemma that says the *sum* cannot see the change:
a test asserting only the total passes with the refunds charged last, and one asserting the peak
cannot. -/
theorem itotal_refunds_first (s₁ s₂ r₁ r₂ : Int) :
    itotal [r₁, r₂, s₁, s₂] = itotal [s₁, s₂, r₁, r₂] := by
  simp only [itotal]
  omega

end Rchain
