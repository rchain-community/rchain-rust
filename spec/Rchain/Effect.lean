import Mathlib.Data.Finset.Basic

/-!
# The effect level of the concurrency model — Law 9 is not enough

`Rchain.Rho`/`Rchain.Concurrent` formalize reduction at the **process** level (contiguous COMM redexes
`send | receive → body`). The reducer, however, schedules **effects** — `produce`/`consume` operations
against the tuple space — where a produce *stores* a datum, a *later* consume matches it, and a
consume's continuation emits its own effects only **after** the trigger matches. This is the level at
which the channel-sharded scheduler (`docs/src/formal/effect-scheduling.md`) wants to run disjoint
effects concurrently.

The natural reading of **Law 9** (`mergeChanges_comm` in `Rchain.RSpace.Merge`: "merging non-conflicting
changes commutes") licenses commuting two effects whose **footprints** (the channels they directly touch)
are disjoint. This module shows that reading is **insufficient**:

* `footprint e` is the trigger channels; `closure e` is the channels reachable through `e`'s transitive
  continuation descent.
* `footprint_disjoint` / `closure_overlap`: a pair of effects with *disjoint footprints* but *overlapping
  closures* exists (the `d`-receive's body receives again on `c`).
* `effect_reorder_diverges`: applying that pair in the two possible orders reaches **different** states —
  so they do **not** commute, and a scheduler that partitions a `Par`'s effects by *static footprint* and
  runs the parts concurrently is **unsound**.

The sound condition is **disjoint closure**, not disjoint footprint — `effect_commute_of_disjoint_closure`
(the *strengthened* Law 9). Because a continuation's closure is discovered only by running the trigger, it
is not statically decidable; consequently no static footprint partition is sound, and the sound maximum
is Level 1 (pure-resolution parallelism only). This is the formal ground for the finding that the
channel-sharded effect scheduler is unsound.
-/

namespace Rchain

/-- A channel. The counterexample uses `0 = c`, `1 = d`, `2 = "join"`, `3 = "out"`. -/
abbrev Chan := Nat

/-- The tuple-space state: `s c` is `true` iff channel `c` holds a datum. At most one datum per channel
    suffices to exhibit the divergence, and keeps the model minimal. -/
abbrev State := Chan → Bool

/-- A produce/consume effect tree. `consume c k` receives on `c`; on a match it runs the continuation
    `k`, whose own effects are emitted only *after* the trigger matches. `stop` is the inert leaf. -/
inductive Effect where
  | produce (c : Chan)
  | consume (c : Chan) (k : Effect)
  | stop

/-- Apply an effect tree depth-first — the sequential reducer's order. -/
def Effect.apply : Effect → State → State
  | produce c, s => fun x => if x = c then true else s x
  | consume c k, s => if s c then k.apply (fun x => if x = c then false else s x) else s
  | stop, s => s

/-- The channels an effect directly touches (its footprint). -/
def Effect.footprint : Effect → Finset Chan
  | produce c => {c}
  | consume c _ => {c}
  | stop => ∅

/-- The channels reachable through the transitive continuation descent (its closure). -/
def Effect.closure : Effect → Finset Chan
  | produce c => {c}
  | consume c k => insert c k.closure
  | stop => ∅

/-! ## The counterexample: disjoint footprint, overlapping closure, non-commuting -/

/-- `receive d { receive c { @"join"!() } }` — footprint `{1}`, closure `{1,0,2}`. -/
def effectA : Effect := Effect.consume 1 (Effect.consume 0 (Effect.produce 2))

/-- `receive c { @"out"!() }` — footprint `{0}`, closure `{0,3}`. -/
def effectB : Effect := Effect.consume 0 (Effect.produce 3)

/-- Initial state: channels `c = 0` and `d = 1` each hold one datum. -/
def state0 : State := fun x => x = 0 ∨ x = 1

/-- The two effects have **disjoint footprints** (Law 9's `NonConflicting` holds). -/
theorem footprint_disjoint : effectA.footprint ∩ effectB.footprint = ∅ := by
  native_decide

/-- ... but **overlapping closures** — so Law 9's footprint reading is too weak. -/
theorem closure_overlap : effectA.closure ∩ effectB.closure ≠ ∅ := by
  native_decide

/-- The counterexample: applying the two effects in opposite orders reaches different states. Hence
    they do not commute, and a static footprint partition that runs them concurrently is unsound. -/
theorem effect_reorder_diverges :
    effectA.apply (effectB.apply state0) ≠ effectB.apply (effectA.apply state0) := by
  intro h
  have h2 : effectA.apply (effectB.apply state0) 2 = effectB.apply (effectA.apply state0) 2 :=
    congrFun h 2
  have hab : effectA.apply (effectB.apply state0) 2 = false := by native_decide
  have hba : effectB.apply (effectA.apply state0) 2 = true := by native_decide
  rw [hab, hba] at h2
  cases h2

/-! ## The sound condition: disjoint **closure** -/

/-- `apply` only touches the closure: outside it, the state is unchanged. -/
theorem apply_unchanged_outside_closure {e : Effect} {s : State} {c : Chan}
    (h : c ∉ e.closure) : e.apply s c = s c := by
  induction e generalizing c s with
  | produce c' =>
      by_cases hc : c = c'
      · subst hc
        simp [Effect.closure] at h
      · simp [Effect.apply, hc]
  | consume c' k ih =>
      by_cases hsc : s c'
      · have hk : c ∉ k.closure := by
          intro hck
          apply h
          simp [Effect.closure, hck]
        have hcc : c ≠ c' := by
          intro hcc
          apply h
          rw [hcc]
          simp [Effect.closure]
        simp [Effect.apply, hsc]
        rw [ih hk]
        simp [hcc]
      · simp [Effect.apply, hsc]
  | stop => simp [Effect.apply]

/-- `apply` is determined by the state on the effect's closure: if two states agree on the
    closure, the results agree **on the closure** (outside it they may differ — the closure is
    exactly the reachable channels, so `e.apply s1 c` outside it is just `s1 c`). This on-closure
    form is what the strengthened Law 9 (`effect_commute_of_disjoint_closure`) needs: there the
    channel is always a member of the first effect's closure. -/
theorem apply_determined_by_closure {e : Effect} {s1 s2 : State} {c : Chan}
    (h : ∀ c, c ∈ e.closure → s1 c = s2 c) (hc : c ∈ e.closure) :
    e.apply s1 c = e.apply s2 c := by
  induction e generalizing s1 s2 c with
  | produce c' =>
      have hcc : c = c' := by simpa [Effect.closure] using hc
      subst hcc
      simp [Effect.apply]
  | consume c' k ih =>
      have hmem : c = c' ∨ c ∈ k.closure := by simpa [Effect.closure] using hc
      by_cases hsc1 : s1 c'
      · have hsc2 : s2 c' = true := by
          simpa [hsc1] using (h c' (by simp [Effect.closure])).symm
        have hagree : ∀ y, y ∈ k.closure →
            (fun x => !decide (x = c') && s1 x) y = (fun x => !decide (x = c') && s2 x) y := by
          intro y hy
          by_cases hyc : y = c'
          · subst y
            simp
          · have := h y (by simp [Effect.closure, hy, hyc])
            simp [hyc, this]
        simp [Effect.apply, hsc1, hsc2]
        rcases hmem with hcc | hck
        · rw [hcc]
          by_cases hkc : c' ∈ k.closure
          · exact ih hagree hkc
          · rw [apply_unchanged_outside_closure (e := k)
                  (s := fun x => !decide (x = c') && s1 x) (c := c') hkc,
                apply_unchanged_outside_closure (e := k)
                  (s := fun x => !decide (x = c') && s2 x) (c := c') hkc]
            simp
        · exact ih hagree hck
      · have hsc2 : s2 c' = false := by
          simpa [hsc1] using (h c' (by simp [Effect.closure])).symm
        simp [Effect.apply, hsc1, hsc2]
        rcases hmem with hcc | hck
        · rw [hcc]
          exact h c' (by simp [Effect.closure])
        · exact h c (by simp [Effect.closure, hck])
  | stop => simp [Effect.apply, Effect.closure] at hc ⊢

/-- **Strengthened Law 9** (the sound condition): effects with disjoint *closures* commute, for
    every state. This is the criterion a concurrent effect scheduler must enforce; disjoint
    *footprints* are insufficient (`effect_reorder_diverges`). Proven from the locality of `apply`
    (`apply_unchanged_outside_closure` / `apply_determined_by_closure`): an effect reads and
    writes only its closure channels, so two effects whose closures are disjoint act on disjoint
    state and commute. It is the effect-level refinement of
    `Rchain.RSpace.Merge.mergeChanges_comm` that the scheduler needs. -/
theorem effect_commute_of_disjoint_closure (e1 e2 : Effect) (s : State) :
    e1.closure ∩ e2.closure = ∅ → e1.apply (e2.apply s) = e2.apply (e1.apply s) := by
  intro h
  have h12 : ∀ c, c ∈ e1.closure → (e2.apply s) c = s c := by
    intro c hc
    have hc2 : c ∉ e2.closure := by
      intro hc2
      have hm : c ∈ e1.closure ∩ e2.closure := Finset.mem_inter.mpr ⟨hc, hc2⟩
      exact (Finset.eq_empty_iff_forall_not_mem.mp h) c hm
    exact apply_unchanged_outside_closure (e := e2) (s := s) (c := c) hc2
  have h21 : ∀ c, c ∈ e2.closure → (e1.apply s) c = s c := by
    intro c hc
    have hc1 : c ∉ e1.closure := by
      intro hc1
      have hm : c ∈ e1.closure ∩ e2.closure := Finset.mem_inter.mpr ⟨hc1, hc⟩
      exact (Finset.eq_empty_iff_forall_not_mem.mp h) c hm
    exact apply_unchanged_outside_closure (e := e1) (s := s) (c := c) hc1
  funext c
  by_cases hc1 : c ∈ e1.closure
  · have hc2 : c ∉ e2.closure := by
      intro hc2
      have hm : c ∈ e1.closure ∩ e2.closure := Finset.mem_inter.mpr ⟨hc1, hc2⟩
      exact (Finset.eq_empty_iff_forall_not_mem.mp h) c hm
    have hl : e1.apply (e2.apply s) c = e1.apply s c :=
      apply_determined_by_closure (e := e1) h12 hc1
    rw [hl, apply_unchanged_outside_closure (e := e2) (s := e1.apply s) (c := c) hc2]
  · by_cases hc2 : c ∈ e2.closure
    · have hr : e2.apply (e1.apply s) c = e2.apply s c :=
        apply_determined_by_closure (e := e2) h21 hc2
      rw [apply_unchanged_outside_closure (e := e1) (s := e2.apply s) (c := c) hc1]
      exact hr.symm
    · rw [apply_unchanged_outside_closure (e := e1) (s := e2.apply s) (c := c) hc1,
          apply_unchanged_outside_closure (e := e2) (s := s) (c := c) hc2,
          apply_unchanged_outside_closure (e := e2) (s := e1.apply s) (c := c) hc2,
          apply_unchanged_outside_closure (e := e1) (s := s) (c := c) hc1]

end Rchain
