import Rchain.Rho

/-!
# The concurrency model (parallel reduction)

Specifies the *parallel* reduction `⟹` and the soundness theorems of the concurrency model
(`docs/src/formal/concurrency-model.md`). The sequential `Reduce` and structural congruence `StrCong`
are in `Rchain.Rho`.

**Proven here:**

- `parStep_comm` — two independent redexes (one on each side of `parMerge`) commute.
- `parStep_to_reduce` — linearization: every parallel step is a finite sequence of sequential
  `Reduce` steps.
- `List.append_eq_singleton`, `parMerge_eq_nilPar`, `sendPar_eq_parMerge`, `receivePar_eq_parMerge`,
  `redex_eq_parMerge` — the field-wise decomposition of the flat `Par` under `parMerge`.
- `reduce_nilPar_impossible`, `reduce_sendPar_impossible`, `reduce_receivePar_impossible` — inertness:
  a nil/send-only/receive-only process cannot reduce (the `comm`-vs-`parLeft` cases are vacuous).
- `reduce_redex_unique` — an *isolated* COMM redex reduces to its body, uniquely up to `StrCong`.
- `strCong_sends_perm` — `≡` preserves the multiset of sends (Law 2: `≡` reorders, never changes).

**Not confluent (flat `Par`)** — `reduce_not_deterministic` shows `Reduce` is not even single-step
deterministic up to `StrCong`: a term with one receive and two sends on one channel is a redex in two
ways. Hence `reduce_confluent` / `parStep_diamond` (confluence up to `StrCong`) are **false** on the
flat `Par`. Full confluence is a property of the **tree model** (`Rchain.Tree`, explicit `par` nodes):
`reduceT_confluent` there proves the diamond, and `flatten` maps it soundly onto the flat `Par`.
-/

namespace Rchain

/-- A parallel step `⟹`: reduce a finite number of pairwise-independent redexes at once. `refl`
    reduces nothing, `comm` contracts a COMM redex, and `par` reduces both sides of `parMerge`. -/
inductive ParStep : Par → Par → Prop where
  | refl {p : Par} : ParStep p p
  | comm (chan data body : Par) :
      ParStep (parMerge (sendPar chan [data]) (receivePar chan body)) body
  | par {p q p' q' : Par} : ParStep p p' → ParStep q q' →
      ParStep (parMerge p q) (parMerge p' q')

/-- Two independent redexes, one on each side of `parMerge`, commute: their common reduct is the
    merge of the two reducts. The foundation of the diamond property. -/
theorem parStep_comm {p p' q q' : Par} (hp : Reduce p p') (hq : Reduce q q') :
    Reduce (parMerge p' q) (parMerge p' q') ∧ Reduce (parMerge p q') (parMerge p' q') :=
  ⟨Reduce.parRight hq, Reduce.parLeft hp⟩

/-- Lift `Reduce*` under the left side of `parMerge`. -/
lemma reflTransGen_parLeft {p p' q : Par} (h : Relation.ReflTransGen Reduce p p') :
    Relation.ReflTransGen Reduce (parMerge p q) (parMerge p' q) := by
  induction h with
  | refl => exact Relation.ReflTransGen.refl
  | tail _ hstep ih =>
      exact Relation.ReflTransGen.tail ih (Reduce.parLeft hstep)

/-- Lift `Reduce*` under the right side of `parMerge`. -/
lemma reflTransGen_parRight {p q q' : Par} (h : Relation.ReflTransGen Reduce q q') :
    Relation.ReflTransGen Reduce (parMerge p q) (parMerge p q') := by
  induction h with
  | refl => exact Relation.ReflTransGen.refl
  | tail _ hstep ih =>
      exact Relation.ReflTransGen.tail ih (Reduce.parRight hstep)

/-- Linearization: every parallel step is a finite sequence of sequential `Reduce` steps. -/
theorem parStep_to_reduce {p q : Par} (h : ParStep p q) :
    Relation.ReflTransGen Reduce p q := by
  induction h with
  | refl => exact Relation.ReflTransGen.refl
  | comm chan data body =>
      exact Relation.ReflTransGen.tail Relation.ReflTransGen.refl (Reduce.comm chan data body)
  | par hp hq ihp ihq =>
      exact Relation.ReflTransGen.trans (reflTransGen_parLeft ihp) (reflTransGen_parRight ihq)

/-- `p ++ q = [s]` splits as `[s] ++ []` or `[] ++ [s]`. -/
lemma List.append_eq_singleton {α : Type} {p q : List α} {s : α} (h : p ++ q = [s]) :
    (p = [s] ∧ q = []) ∨ (p = [] ∧ q = [s]) := by
  cases p with
  | nil => simp at h; exact Or.inr ⟨rfl, h⟩
  | cons a ps =>
      cases ps with
      | nil =>
          cases q with
          | nil => exact Or.inl ⟨by simpa using h, rfl⟩
          | cons b qs => cases h
      | cons b pss => cases h

/-- `parMerge p q = nilPar` forces both summands to `nilPar`. -/
lemma parMerge_eq_nilPar {p q : Par} (h : parMerge p q = nilPar) : p = nilPar ∧ q = nilPar := by
  cases p <;> cases q <;> simp [parMerge, nilPar] at h ⊢
  · simp_all [nilPar]

/-- `sendPar chan data = parMerge p q` splits as `sendPar | nilPar` (or `nilPar | sendPar`). -/
lemma sendPar_eq_parMerge {chan : Par} {data : List Par} {p q : Par}
    (h : sendPar chan data = parMerge p q) :
    (p = sendPar chan data ∧ q = nilPar) ∨ (p = nilPar ∧ q = sendPar chan data) := by
  cases p <;> cases q <;> simp [sendPar, parMerge] at h
  rcases List.append_eq_singleton h.1.symm with ⟨hps, hqs⟩ | ⟨hps, hqs⟩
  · left; subst hps; subst hqs; simp_all [sendPar, nilPar, List.append_eq_nil]
  · right; subst hps; subst hqs; simp_all [sendPar, nilPar, List.append_eq_nil]

/-- `receivePar chan body = parMerge p q` splits as `receivePar | nilPar` (or `nilPar | receivePar`). -/
lemma receivePar_eq_parMerge {chan body : Par} {p q : Par}
    (h : receivePar chan body = parMerge p q) :
    (p = receivePar chan body ∧ q = nilPar) ∨ (p = nilPar ∧ q = receivePar chan body) := by
  cases p <;> cases q <;> simp [receivePar, parMerge] at h
  rcases List.append_eq_singleton h.2.1.symm with ⟨hpr, hqr⟩ | ⟨hpr, hqr⟩
  · left; subst hpr; subst hqr; simp_all [receivePar, nilPar, List.append_eq_nil]
  · right; subst hpr; subst hqr; simp_all [receivePar, nilPar, List.append_eq_nil]

/-- `nilPar` cannot reduce. -/
lemma reduce_nilPar_impossible {r : Par} (h : Reduce nilPar r) : False :=
  reduce_nilPar_impossible_aux h rfl
where
  reduce_nilPar_impossible_aux : ∀ {p r : Par}, Reduce p r → p = nilPar → False := by
    intro p r h hp
    induction h with
    | comm c d b =>
        simp [parMerge, sendPar, receivePar, nilPar] at hp
    | parLeft hp1 ih =>
        rcases parMerge_eq_nilPar hp with ⟨hp_eq, _⟩
        subst hp_eq
        exact ih rfl
    | parRight hq1 ih =>
        rcases parMerge_eq_nilPar hp with ⟨_, hq_eq⟩
        subst hq_eq
        exact ih rfl

/-- A send-only process cannot reduce. -/
lemma reduce_sendPar_impossible {chan : Par} {data : List Par} {r : Par}
    (h : Reduce (sendPar chan data) r) : False :=
  reduce_sendPar_impossible_aux h rfl
where
  reduce_sendPar_impossible_aux : ∀ {p r : Par}, Reduce p r → p = sendPar chan data → False := by
    intro p r h hp
    induction h with
    | comm c d b =>
        simp [parMerge, sendPar, receivePar] at hp
    | parLeft hp1 ih =>
        rcases sendPar_eq_parMerge hp.symm with ⟨hp_eq, hq_eq⟩ | ⟨hp_eq, hq_eq⟩
        · subst hp_eq; subst hq_eq; exact ih rfl
        · subst hp_eq; subst hq_eq; exact reduce_nilPar_impossible hp1
    | parRight hq1 ih =>
        rcases sendPar_eq_parMerge hp.symm with ⟨hp_eq, hq_eq⟩ | ⟨hp_eq, hq_eq⟩
        · subst hp_eq; subst hq_eq; exact reduce_nilPar_impossible hq1
        · subst hp_eq; subst hq_eq; exact ih rfl

/-- A receive-only process cannot reduce. -/
lemma reduce_receivePar_impossible {chan body r : Par}
    (h : Reduce (receivePar chan body) r) : False :=
  reduce_receivePar_impossible_aux h rfl
where
  reduce_receivePar_impossible_aux : ∀ {p r : Par}, Reduce p r → p = receivePar chan body → False := by
    intro p r h hp
    induction h with
    | comm c d b =>
        simp [parMerge, sendPar, receivePar] at hp
    | parLeft hp1 ih =>
        rcases receivePar_eq_parMerge hp.symm with ⟨hp_eq, hq_eq⟩ | ⟨hp_eq, hq_eq⟩
        · subst hp_eq; subst hq_eq; exact ih rfl
        · subst hp_eq; subst hq_eq; exact reduce_nilPar_impossible hp1
    | parRight hq1 ih =>
        rcases receivePar_eq_parMerge hp.symm with ⟨hp_eq, hq_eq⟩ | ⟨hp_eq, hq_eq⟩
        · subst hp_eq; subst hq_eq; exact reduce_nilPar_impossible hq1
        · subst hp_eq; subst hq_eq; exact ih rfl

/-- `Par` is determined by its eight fields. -/
lemma Par.eta (p : Par) :
    p = Par.mk p.sends p.receives p.news p.exprs p.matches p.unforgeables p.bundles p.connectives := by
  cases p; rfl

/-- Reconstruct `p = sendPar chan data` from its field values. -/
lemma eq_sendPar_of_fields {p chan : Par} {data : List Par}
    (hs : p.sends = [Send.mk chan data false]) (hr : p.receives = [])
    (hn : p.news = []) (he : p.exprs = []) (hm : p.matches = [])
    (hu : p.unforgeables = []) (hb : p.bundles = []) (hc : p.connectives = []) :
    p = sendPar chan data := by
  rw [Par.eta p, hs, hr, hn, he, hm, hu, hb, hc]; rfl

/-- Reconstruct `p = nilPar` from its field values. -/
lemma eq_nilPar_of_fields {p : Par}
    (hs : p.sends = []) (hr : p.receives = []) (hn : p.news = []) (he : p.exprs = [])
    (hm : p.matches = []) (hu : p.unforgeables = []) (hb : p.bundles = []) (hc : p.connectives = []) :
    p = nilPar := by
  rw [Par.eta p, hs, hr, hn, he, hm, hu, hb, hc]; rfl

/-- Reconstruct `p = receivePar chan body` from its field values. -/
lemma eq_receivePar_of_fields {p chan body : Par}
    (hs : p.sends = [])
    (hr : p.receives = [Receive.mk [ReceiveBind.mk [anyPat] chan 1] body false 1])
    (hn : p.news = []) (he : p.exprs = []) (hm : p.matches = [])
    (hu : p.unforgeables = []) (hb : p.bundles = []) (hc : p.connectives = []) :
    p = receivePar chan body := by
  rw [Par.eta p, hs, hr, hn, he, hm, hu, hb, hc]; rfl

/-- Reconstruct `p = parMerge (sendPar chan [data]) (receivePar chan body)` from its field values. -/
lemma eq_commRedex_of_fields {p chan data body : Par}
    (hs : p.sends = [Send.mk chan [data] false])
    (hr : p.receives = [Receive.mk [ReceiveBind.mk [anyPat] chan 1] body false 1])
    (hn : p.news = []) (he : p.exprs = []) (hm : p.matches = [])
    (hu : p.unforgeables = []) (hb : p.bundles = []) (hc : p.connectives = []) :
    p = parMerge (sendPar chan [data]) (receivePar chan body) := by
  rw [Par.eta p, hs, hr, hn, he, hm, hu, hb, hc]; rfl

/-- A COMM redex split as `parMerge p q` decomposes into four cases, determined by which summand
    carries the send and which carries the receive. -/
lemma redex_eq_parMerge {chan data body p q : Par}
    (h : parMerge (sendPar chan [data]) (receivePar chan body) = parMerge p q) :
    (p = parMerge (sendPar chan [data]) (receivePar chan body) ∧ q = nilPar)
    ∨ (p = sendPar chan [data] ∧ q = receivePar chan body)
    ∨ (p = receivePar chan body ∧ q = sendPar chan [data])
    ∨ (p = nilPar ∧ q = parMerge (sendPar chan [data]) (receivePar chan body)) := by
  have hsends : p.sends ++ q.sends = [Send.mk chan [data] false] := by
    simpa [parMerge, sendPar, receivePar] using (congrArg Par.sends h).symm
  have hrecvs : p.receives ++ q.receives =
      [Receive.mk [ReceiveBind.mk [anyPat] chan 1] body false 1] := by
    simpa [parMerge, sendPar, receivePar] using (congrArg Par.receives h).symm
  have hnews : p.news = [] ∧ q.news = [] := by
    exact List.append_eq_nil.mp (by simpa [parMerge, sendPar, receivePar] using (congrArg Par.news h).symm)
  have hexprs : p.exprs = [] ∧ q.exprs = [] := by
    exact List.append_eq_nil.mp (by simpa [parMerge, sendPar, receivePar] using (congrArg Par.exprs h).symm)
  have hmatches : p.matches = [] ∧ q.matches = [] := by
    exact List.append_eq_nil.mp (by simpa [parMerge, sendPar, receivePar] using (congrArg Par.matches h).symm)
  have hunforgeables : p.unforgeables = [] ∧ q.unforgeables = [] := by
    exact List.append_eq_nil.mp (by simpa [parMerge, sendPar, receivePar] using (congrArg Par.unforgeables h).symm)
  have hbundles : p.bundles = [] ∧ q.bundles = [] := by
    exact List.append_eq_nil.mp (by simpa [parMerge, sendPar, receivePar] using (congrArg Par.bundles h).symm)
  have hconnectives : p.connectives = [] ∧ q.connectives = [] := by
    exact List.append_eq_nil.mp (by simpa [parMerge, sendPar, receivePar] using (congrArg Par.connectives h).symm)
  rcases List.append_eq_singleton hsends with ⟨hps, hqs⟩ | ⟨hps, hqs⟩
  · rcases List.append_eq_singleton hrecvs with ⟨hpr, hqr⟩ | ⟨hpr, hqr⟩
    · left
      exact ⟨eq_commRedex_of_fields hps hpr hnews.1 hexprs.1 hmatches.1 hunforgeables.1 hbundles.1 hconnectives.1,
             eq_nilPar_of_fields hqs hqr hnews.2 hexprs.2 hmatches.2 hunforgeables.2 hbundles.2 hconnectives.2⟩
    · right; left
      exact ⟨eq_sendPar_of_fields hps hpr hnews.1 hexprs.1 hmatches.1 hunforgeables.1 hbundles.1 hconnectives.1,
             eq_receivePar_of_fields hqs hqr hnews.2 hexprs.2 hmatches.2 hunforgeables.2 hbundles.2 hconnectives.2⟩
  · rcases List.append_eq_singleton hrecvs with ⟨hpr, hqr⟩ | ⟨hpr, hqr⟩
    · right; right; left
      exact ⟨eq_receivePar_of_fields hps hpr hnews.1 hexprs.1 hmatches.1 hunforgeables.1 hbundles.1 hconnectives.1,
             eq_sendPar_of_fields hqs hqr hnews.2 hexprs.2 hmatches.2 hunforgeables.2 hbundles.2 hconnectives.2⟩
    · right; right; right
      exact ⟨eq_nilPar_of_fields hps hpr hnews.1 hexprs.1 hmatches.1 hunforgeables.1 hbundles.1 hconnectives.1,
             eq_commRedex_of_fields hqs hqr hnews.2 hexprs.2 hmatches.2 hunforgeables.2 hbundles.2 hconnectives.2⟩

/-- A COMM redex has a unique reduct up to `StrCong`. -/
lemma reduce_redex_unique {chan data body q' : Par}
    (h : Reduce (parMerge (sendPar chan [data]) (receivePar chan body)) q') :
    StrCong q' body :=
  reduce_redex_unique_aux h rfl
where
  reduce_redex_unique_aux : ∀ {p q' : Par},
      Reduce p q' → p = parMerge (sendPar chan [data]) (receivePar chan body) → StrCong q' body := by
    intro p q' h hp
    induction h with
    | comm c d b =>
        have hrecvs : [Receive.mk [ReceiveBind.mk [anyPat] c 1] b false 1] =
            [Receive.mk [ReceiveBind.mk [anyPat] chan 1] body false 1] := by
          simpa [parMerge, sendPar, receivePar] using congrArg Par.receives hp
        have hb : b = body := by
          simpa using congrArg Receive.body (List.cons.inj hrecvs).1
        rw [hb]
        exact StrCong.refl body
    | parLeft hp1 ih =>
        rcases redex_eq_parMerge hp.symm with
          ⟨hp_redex, hq_nil⟩ | ⟨hp_send, hq_recv⟩ | ⟨hp_recv, hq_send⟩ | ⟨hp_nil, hq_redex⟩
        · subst hq_nil
          exact StrCong.trans (StrCong.ident _) (ih hp_redex)
        · subst hp_send
          exact False.elim (reduce_sendPar_impossible hp1)
        · subst hp_recv
          exact False.elim (reduce_receivePar_impossible hp1)
        · subst hp_nil
          exact False.elim (reduce_nilPar_impossible hp1)
    | parRight hq1 ih =>
        rcases redex_eq_parMerge hp.symm with
          ⟨hp_redex, hq_nil⟩ | ⟨hp_send, hq_recv⟩ | ⟨hp_recv, hq_send⟩ | ⟨hp_nil, hq_redex⟩
        · subst hq_nil
          exact False.elim (reduce_nilPar_impossible hq1)
        · subst hq_recv
          exact False.elim (reduce_receivePar_impossible hq1)
        · subst hq_send
          exact False.elim (reduce_sendPar_impossible hq1)
        · subst hp_nil
          exact StrCong.trans (strCong_nil_left _) (ih hq_redex)

/-- Structural congruence preserves the multiset of sends: `≡` (Law 2) only reorders and
    reassociates the eight field lists, never changing their contents. -/
lemma strCong_sends_perm {p q : Par} (h : StrCong p q) : List.Perm p.sends q.sends := by
  induction h with
  | refl p => exact List.Perm.refl _
  | symm hp ih => exact ih.symm
  | trans hp hq ihp ihq => exact ihp.trans ihq
  | comm p q =>
      simpa [parMerge] using (List.perm_append_comm (l₁ := p.sends) (l₂ := q.sends))
  | assoc p q r =>
      simp [parMerge, List.append_assoc]
  | ident p => simpa [parMerge, nilPar] using (List.Perm.refl p.sends)
  | par hp hq ihp ihq => simpa [parMerge] using (List.Perm.append ihp ihq)

/-- Two `sendPar`s carrying different data are never `≡`. -/
lemma sendPar_ne_strCong {c d1 d2 : Par} (hne : d1 ≠ d2) :
    ¬ StrCong (sendPar c [d1]) (sendPar c [d2]) := by
  intro h
  have hperm : List.Perm [Send.mk c [d1] false] [Send.mk c [d2] false] := by
    simpa [sendPar] using strCong_sends_perm h
  have hmem : Send.mk c [d1] false ∈ [Send.mk c [d2] false] :=
    (List.Perm.mem_iff hperm).1 (by simp)
  have hsend : Send.mk c [d1] false = Send.mk c [d2] false := by simpa using hmem
  have hdata : [d1] = [d2] := congrArg Send.data hsend
  exact hne ((List.cons.inj hdata).1)

/-! ## The flat `Par` is **not** confluent

The flat `parMerge` is a field-wise monoid, so a single flat term has several decompositions into
`parMerge p q` summands. A term with one receive and two sends on the same channel

    `sendPar c [d₁] | receivePar c nilPar | sendPar c [d₂]`

is a COMM redex in **two** ways: the receive pairs with `d₁` (via `parLeft`) or with `d₂` (via
`parRight`), reducing to the inert, non-`≡` terms `sendPar c [d₂]` and `sendPar c [d₁]`
(`sendPar_ne_strCong`). Hence single-step confluence (`Reduce p q → Reduce p r → ∃ s t,
q ⟶* s ∧ r ⟶* t ∧ s ≡ t`) and the `ParStep` diamond are **false** on the flat `Par`: `parMerge`
erases the tree structure that would say *which* send pairs with the receive.

What *does* hold is `reduce_redex_unique` above — an *isolated* redex is deterministic up to
`StrCong`. Full confluence is a property of the tree model (explicit `par` nodes); the flat `Par`
is its field-wise quotient, and the two are not interchangeable for confluence. -/

/-- The flat `Reduce` is not even single-step deterministic up to `StrCong`: one term reduces (via
    two different `parMerge` decompositions) to two non-`≡` send-only terms. -/
theorem reduce_not_deterministic :
    ∃ p q r, Reduce p q ∧ Reduce p r ∧ ¬ StrCong q r := by
  let c := nilPar
  let d1 := nilPar
  let d2 := sendPar nilPar [nilPar]
  have hne : d1 ≠ d2 := by
    intro h
    have := congrArg Par.sends h
    simp [d1, d2, nilPar, sendPar] at this
  refine ⟨parMerge (sendPar c [d1]) (parMerge (receivePar c nilPar) (sendPar c [d2])),
      sendPar c [d2], sendPar c [d1], ?_, ?_, ?_⟩
  · have hp : parMerge (sendPar c [d1]) (parMerge (receivePar c nilPar) (sendPar c [d2])) =
        parMerge (parMerge (sendPar c [d1]) (receivePar c nilPar)) (sendPar c [d2]) := by
      simp [parMerge, sendPar, receivePar]
    rw [hp]
    exact Reduce.parLeft (Reduce.comm c d1 nilPar)
  · have hp : parMerge (sendPar c [d1]) (parMerge (receivePar c nilPar) (sendPar c [d2])) =
        parMerge (sendPar c [d1]) (parMerge (sendPar c [d2]) (receivePar c nilPar)) := by
      simp [parMerge, sendPar, receivePar]
    rw [hp]
    exact Reduce.parRight (Reduce.comm c d2 nilPar)
  · intro h
    exact sendPar_ne_strCong hne (StrCong.symm h)

end Rchain
