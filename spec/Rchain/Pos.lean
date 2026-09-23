import Rchain.Casper.Stake

/-!
# Laws 44–47 — Proof-of-Stake: the epoch gate, the reward split, and the dust

The port's native PoS state (`rholang/src/native_state.rs`) implements the validator lifecycle — bond,
withdraw, slash, trust — and **the epoch gate it used to skip now exists**: where `close_block` once
refunded quarantined withdrawers and recomputed `pos:active` on *every* block, never reading
`PosParams.epoch_length`, it now runs those steps only at a boundary, matching the Scala contract it
replaces (`legacy/casper/src/main/resources/Pos.rhox`: `blockNumber % $$epochLength$$ == 0`, :517;
rewards computed at the boundary, `getCurrentEpochRewards` :241-256, and committed,
`commitCurrentEpochRewards` :568-576; only expired quarantines paid, :556-567, :592-621). The port
keeps the contract's *meaning* for a zero epoch length (`native_state.rs:577-585`: the divisor is
`max(epoch_length, 1)`, so a zero means one-block epochs rather than a division fault).

Laws 44 and 47 remain `open` for a different reason: what they name is the **state machine** the gate
guards, and until this module grows one the laws are claims about the Rust evidenced by Rust tests.
That is this file's Programme C item, and the two properties are conservation (an epoch moves value
between the vaults and mints none) and the release rule (a withdrawal is staged, then escrowed out of
the pool, then paid `bond + committed` — and only at a boundary past its quarantine).

## The reward split does not conserve, and the law has to say so

`getCurrentEpochRewards` divides **twice** with integer division:

    reward_i = pot * (bond_i / minimumBond) / (activeBonds / minimumBond)

so `Σ reward_i` is the pot *minus dust* — the sum of the floored shares, re-floored. A conservation law
written as an equality would be **false**, and the Scala's own comment does not say which it means. The
model settles it: `list_sum_div_le` and `sum_rewards_le_pot` below are the inequality, and the instance
at the end is a case where it is *strict* — which is what makes the statement falsifiable rather than a
restatement of an accounting identity.
-/

/-- The sum of a list of naturals. Defined here rather than imported: this file is about the arithmetic
of one fold, and Mathlib's big-operators import would pull a tree the spec does not otherwise need. -/
def nsum : List Nat → Nat
  | [] => 0
  | a :: rest => a + nsum rest

namespace Rchain

/-- The distributable pot of an epoch: `posBalance - totalBond - totalWithdraw - totalCommittedRewards`
(`Pos.rhox:241-256`). The Scala computes it in `Long` and the contract's guards keep it non-negative, so
the model is a `Nat` subtraction — the floor is the *stated* part, not an artefact of the type. -/
def rewardPot (posBalance totalBond totalWithdraw committed : Nat) : Nat :=
  posBalance - totalBond - totalWithdraw - committed

/-- One active validator's epoch reward, exactly as `getCurrentEpochRewards` divides it: the pot scaled
by the validator's bond over `minimumBond`, normalised by the active set's bond over `minimumBond`. -/
def reward (pot minimumBond activeBonds bond : Nat) : Nat :=
  pot * (bond / minimumBond) / (activeBonds / minimumBond)

/-- **The integer division that makes the split conservative**: `⌊a/m⌋ + ⌊b/m⌋ ≤ ⌊(a+b)/m⌋`. Two
floors never add up to more than the floor of the sum, which is why the reward formula leaves a
remainder rather than distributing the pot exactly. -/
theorem div_add_div_le (m a b : Nat) : a / m + b / m ≤ (a + b) / m := by
  rcases Nat.eq_zero_or_pos m with rfl | hm
  · simp
  -- Two floors add up to at most the floor of the sum: each division drops a remainder, and dropping
  -- two of them can only lose. Stated through `div_mul_le_self` rather than left to `omega`, because
  -- `(a / m) * m` is a product of two terms and `omega` is linear in its variables.
  · rw [Nat.le_div_iff_mul_le hm, Nat.add_mul]
    exact Nat.add_le_add (Nat.div_mul_le_self a m) (Nat.div_mul_le_self b m)

/-- The same fact over a list: the sum of the floored shares never exceeds the floor of the sum. The
induction is `div_add_div_le` once per element. -/
theorem list_sum_div_le (m : Nat) : ∀ (l : List Nat), nsum (l.map (· / m)) ≤ nsum l / m
  | [] => by simp [nsum]
  | a :: rest => by
    have ih := list_sum_div_le m rest
    have step := div_add_div_le m a (nsum rest)
    simp only [List.map_cons, nsum] at ih ⊢
    omega

/-- `Σ (pot * x_i) = pot * Σ x_i` — the factor the split multiplies each share by. -/
theorem nsum_map_mul_left (pot : Nat) : ∀ (l : List Nat), nsum (l.map (fun x => pot * x)) = pot * nsum l
  | [] => by simp [nsum]
  | a :: rest => by
    simp only [List.map_cons, nsum, nsum_map_mul_left pot rest]
    rw [Nat.mul_add]

/-- **Law 46 — the epoch's split never pays out more than the pot**, and the remainder is the dust of two
integer divisions. Stated over the list of active bonds rather than a map, because the formula only ever
reads each bond through `bond / minimumBond`.

The hypotheses are the contract's own: the active set's bond total is the sum of its members'
(`hactive`), and the normaliser is positive (`hD`) — which is the only hypothesis the proof needs, and
it is the one that carries `minimumBond > 0` with it: `activeBonds / minimumBond` is positive only if the
divisor is, so a `minimumBond` of zero is already excluded. `activeBonds / minimumBond = 0` would be a
contract that pays every validator nothing, which is the shape the hypothesis refuses. -/
theorem sum_rewards_le_pot (pot minimumBond activeBonds : Nat) (bonds : List Nat)
    (hactive : activeBonds = nsum bonds) (hD : 0 < activeBonds / minimumBond) :
    nsum (bonds.map (fun b => reward pot minimumBond activeBonds b)) ≤ pot := by
  have hstep :
      nsum (bonds.map (fun b => reward pot minimumBond activeBonds b))
        ≤ nsum (bonds.map (fun b => pot * (b / minimumBond))) / (activeBonds / minimumBond) := by
    simpa only [reward, List.map_map, Function.comp_def] using
      list_sum_div_le (activeBonds / minimumBond) (bonds.map (fun b => pot * (b / minimumBond)))
  have hsum : nsum (bonds.map (fun b => pot * (b / minimumBond)))
      = pot * nsum (bonds.map (fun b => b / minimumBond)) := by
    simpa only [List.map_map, Function.comp_def] using
      nsum_map_mul_left pot (bonds.map (fun b => b / minimumBond))
  have hfits : nsum (bonds.map (fun b => b / minimumBond)) ≤ activeBonds / minimumBond := by
    rw [hactive]
    exact list_sum_div_le minimumBond bonds
  have hmono : pot * nsum (bonds.map (fun b => b / minimumBond)) / (activeBonds / minimumBond)
      ≤ pot * (activeBonds / minimumBond) / (activeBonds / minimumBond) :=
    Nat.div_le_div_right (Nat.mul_le_mul_left pot hfits)
  calc nsum (bonds.map (fun b => reward pot minimumBond activeBonds b))
      ≤ nsum (bonds.map (fun b => pot * (b / minimumBond))) / (activeBonds / minimumBond) := hstep
    _ = pot * nsum (bonds.map (fun b => b / minimumBond)) / (activeBonds / minimumBond) := by rw [hsum]
    _ ≤ pot * (activeBonds / minimumBond) / (activeBonds / minimumBond) := hmono
    _ = pot := Nat.mul_div_cancel _ hD

/-- **The inequality is strict, so it is not an accounting identity in disguise.** `minimumBond = 3`,
bonds `[4, 5]`: the normaliser is `9 / 3 = 3`, each validator's scaled share is `4/3 = 1` and `5/3 = 1`,
so with a pot of ten each is paid `10 * 1 / 3 = 3` and the epoch distributes **6 of 10** — four units of
dust, which is the whole content of law 46. A statement that said `Σ = pot` would be refuted by this
line. -/
theorem the_dust_is_real :
    reward 10 3 9 4 + reward 10 3 9 5 = 6 ∧ (6 : Nat) < 10 := by decide

end Rchain

/-! ## The state machine the arithmetic above sits inside (laws 44 and 47)

The first half of this file models the epoch's *arithmetic* — the pot and the split. What laws 44 and 47
name is the **state machine** around it: the gate that decides whether an epoch happens at all, the four
ordered steps `close_block` runs when it does, and the three stages a withdrawal passes through. That
machine was missing, so both laws were claims about the Rust evidenced only by Rust tests. It is here
now, mirroring `rholang/src/native_state.rs` — field for field where a rule reads the field, and step for
step in `close_block`'s order (`:1083-1158`), because the order carries meaning: the rewards are
committed **before** a leaving validator is moved out of the pool, which is what lets a validator earn in
the epoch it leaves.

## What conservation is, and what it is not

`totalRev` sums the **coin fields only**. The pool, the requests, the claims and the committed map are
*liabilities against the staking vault* — ledger entries, not coins — which is why the Rust's own
`total_rev` helper sums exactly the three coin fields (`native_state.rs:1581-1593`), and why the port's
`slash`-minted-before-`ba9e259a7` bug is this theorem's motivating counterexample: it credited the Coop
vault without debiting the staking vault. Counting the ledgers in the sum would make the theorem false,
not stronger.
-/

namespace Rchain

/-- A validator's claim on the staking vault: the bond escrowed out of the pool, and the block at which it
    may be paid (`Withdrawal`, `native_state.rs:388-392`). The struct stores the **bond only** — the
    reward is read from the committed map at payment time, which is what lets a validator be paid for the
    epoch it left in. -/
structure PosClaim where
  who : Validator
  bond : Nat
  deadline : Nat
deriving DecidableEq

/-- A staged withdrawal: the validator and the block at which its quarantine expires
    (`pos:pending_withdrawers`, `native_state.rs:90-92`). -/
structure PosRequest where
  who : Validator
  deadline : Nat
deriving DecidableEq

/-- The PoS state: the three coin fields, the four ledgers, the active set, and the two parameters the
    transition reads.

    `user` is the **total** over every user vault rather than a map: every transfer in this mechanism
    either moves coins between the staking vault and a user's vault or between the staking and Coop
    vaults, so the total is what conservation is about and a per-address map would only add lookup
    lemmas. `active` is what step 4 recomputes; `pool` is the bond pool (`pos:bonds`), which a bond joins
    immediately but which the active set only follows at a boundary. -/
structure PosState where
  /-- The staking vault (`pos:vault`) — the epoch pot's source. -/
  vault : Nat
  /-- The Coop multisig vault (`pos:coop`) — where slashing confiscates to. -/
  coop : Nat
  /-- The total over every user vault. -/
  user : Nat
  /-- The bond pool (`pos:bonds`): every pooled validator and its stake. -/
  pool : List (Validator × Nat)
  /-- The active set (`pos:active`) — recomputed by step 4, never by `bond`. -/
  active : List Validator
  /-- Staged withdrawal requests (`pos:pending_withdrawers`). -/
  requests : List PosRequest
  /-- Escrowed withdrawal claims (`pos:withdrawers`). -/
  claims : List PosClaim
  /-- Committed rewards (`pos:committed`) — a claim ledger, not coins. -/
  committed : List (Validator × Nat)
  /-- `epochLength` (`PosParams.epoch_length`). -/
  epochLength : Nat
  /-- `quarantineLength` (`PosParams.quarantine_length`). -/
  quarantineLength : Nat
deriving DecidableEq

/-- **The conserved quantity**: every user vault plus the staking vault plus the Coop vault — the sum the
    Rust's `total_rev` computes (`native_state.rs:1581-1593`). The ledgers are deliberately outside it:
    they are claims *against* `vault`, and counting them would count the same REV twice. -/
def totalRev (s : PosState) : Nat := s.vault + s.coop + s.user

/-- A ledger lookup — `0` for an absent key, the port's `unwrap_or(NonNegI64::zero())`
    (`native_state.rs:1104-1107`, `:1137`). -/
def lookup : List (Validator × Nat) → Validator → Nat
  | [], _ => 0
  | (w, x) :: rest, v => if w = v then x else lookup rest v

/-- Set a ledger key (insert or replace) — the port's `committed.insert`. -/
def setKey (l : List (Validator × Nat)) (v : Validator) (x : Nat) : List (Validator × Nat) :=
  if l.any (fun p => p.1 = v) then l.map (fun p => if p.1 = v then (v, x) else p)
  else (v, x) :: l

/-- **The epoch divisor**: `max(epoch_length, 1)` (`epoch_divisor`, `native_state.rs:584-590`). The
    contract divides by `$$epochLength$$` directly and faults on zero; the port reads a zero as "every
    block is a boundary", which is what `epoch_length = 1` means to the contract. -/
def divisor (s : PosState) : Nat := max s.epochLength 1

/-- **The gate** (`is_epoch_boundary`, `native_state.rs:594-596`): an epoch runs exactly when the block
    number is a multiple of the divisor. -/
def isBoundary (s : PosState) (n : Nat) : Bool := n % divisor s == 0

/-- The deadline a withdrawal staged at block `n` is given:
    `quarantineLength + epochLength * (1 + n / divisor)` (`native_state.rs:1046-1052`, `Pos.rhox:381`). -/
def withdrawDeadline (s : PosState) (n : Nat) : Nat :=
  s.quarantineLength + s.epochLength * (1 + n / divisor s)

/-- **The withdrawal request** (`withdraw`, `native_state.rs:1037-1057`): the request records a deadline
    and changes **nothing else** — the validator stays in the pool and in the active set, still earning.
    This is law 47's first stage. -/
def stage (s : PosState) (v : Validator) (n : Nat) : PosState :=
  { s with requests := ⟨v, withdrawDeadline s n⟩ :: s.requests }

/-- **A bond** (`bond`, `native_state.rs:962-1022`): the stake moves from the user's vault into the
    staking vault and the validator joins the **pool** — and the active set is untouched. Activation is
    the boundary's step 4, which is law 44's "pooled but not activated". -/
def bond (s : PosState) (v : Validator) (stake : Nat) : PosState :=
  { s with user := s.user - stake, vault := s.vault + stake, pool := (v, stake) :: s.pool }

/-- Step 1 of `close_block`: the epoch's rewards are written into the **committed** ledger
    (`native_state.rs:1099-1112`). The amounts are `Rchain.reward`'s (modelled above); the machine takes
    them as given, because no amount moves a coin — and *that* is what conservation claims. -/
def commitRewards (r : Validator → Nat) (s : PosState) : PosState :=
  { s with
    committed := s.pool.foldl
      (fun (l : List (Validator × Nat)) (wb : Validator × Nat) =>
        setKey l wb.1 (lookup l wb.1 + r wb.1)) s.committed }

/-- Step 2: every staged request becomes an escrowed claim — its bond leaves the pool and its deadline is
    recorded (`native_state.rs:1114-1125`). No coin moves: the escrowed bond is still in the vault and the
    validator's own vault is still empty. Law 47's second stage. -/
def movePending (s : PosState) : PosState :=
  { s with
    claims := s.claims ++ s.requests.filterMap (fun r =>
      match s.pool.find? (fun wb => wb.1 = r.who) with
      | some wb => some ⟨r.who, wb.2, r.deadline⟩
      | none => none),
    pool := s.pool.filter (fun wb => !(s.requests.any (fun r => r.who = wb.1))),
    requests := [] }

/-- The claims a boundary pays: those whose quarantine has elapsed (`native_state.rs:1127-1132`, the
    filter `w.deadline <= block_number`). -/
def dueClaims (s : PosState) (n : Nat) : List PosClaim := s.claims.filter (fun c => c.deadline ≤ n)

/-- What one claim is paid: its bond **plus** its committed reward, read from the ledger at payment time
    (`native_state.rs:1137-1138`; the contract's `bonds + committedRewards.getOrElse(pk, 0)`,
    `Pos.rhox:604`). -/
def payoutOf (s : PosState) (c : PosClaim) : Nat := c.bond + lookup s.committed c.who

/-- The total a boundary pays out. -/
def duePayout (s : PosState) (n : Nat) : Nat := nsum ((dueClaims s n).map (payoutOf s))

/-- Step 3: pay every due claim — debit the staking vault, credit the validator's vault, remove the claim
    and its committed entry (`native_state.rs:1133-1148`). Written as one batch rather than a fold, which
    is the same function whenever a validator has at most one claim: the port's `withdrawers` is a *map*,
    so that is an invariant of the mechanism rather than a coincidence.

    **`none` is the port's refusal, not a silent half-payment.** `debit_pos_vault` returns an error when
    the vault cannot cover the transfer and `close_block` propagates it, having persisted nothing (its
    writes come after all four steps, `native_state.rs:1151-1157`). An unguarded `Nat` subtraction would
    instead truncate the debit and *mint* the difference — a model that quietly does the wrong thing,
    which is the failure mode this project's Rust avoids structurally and its models are supposed to as
    well. So the payment is partial on purpose, and the conservation theorem below carries no hypothesis:
    it is stated about whatever the step returned. -/
def payDue (s : PosState) (n : Nat) : Option PosState :=
  if duePayout s n ≤ s.vault then
    some { s with
      vault := s.vault - duePayout s n
      user := s.user + duePayout s n
      claims := s.claims.filter (fun c => !(c.deadline ≤ n))
      committed := s.committed.filter (fun p => !((dueClaims s n).any (fun c => c.who = p.1))) }
  else none

/-- Step 4: the active set for the epoch that starts now — the pool's members, minus anyone whose bond is
    escrowed in a claim (`select_active`, `native_state.rs:542-561`). -/
def reselect (s : PosState) : PosState :=
  { s with
    active := (s.pool.filter (fun wb => !(s.claims.any (fun c => c.who = wb.1)))).map (·.1) }

/-- **The epoch transition**, in `close_block`'s order (`native_state.rs:1083-1158`): commit the rewards,
    move the staged withdrawals into claims, pay the claims whose quarantine elapsed, re-select the active
    set. The gate is *outside* this function (`closeBlock`), because a transition that branched on it
    would make "off a boundary nothing changes" a restatement of its own definition. -/
def epochStep (r : Validator → Nat) (s : PosState) (n : Nat) : Option PosState :=
  (payDue (movePending (commitRewards r s)) n).map reselect

/-- `close_block`: at a boundary the epoch runs; off one, **nothing is written at all** — the port returns
    before touching state (`native_state.rs:1083-1092`). -/
def closeBlock (r : Validator → Nat) (s : PosState) (n : Nat) : Option PosState :=
  if isBoundary s n then epochStep r s n else some s

/-! ### Conservation -/

/-- **A payout is a transfer, not a mint** — the property the port's `total_rev` helper exists to assert.
    No hypothesis: the guard *is* the port's refusal (`debit_pos_vault` fails a transfer the vault cannot
    cover, `native_state.rs:1139`, rather than half-paying it), so whatever the step returned conserves. -/
theorem payDue_conserves (s : PosState) (n : Nat) {s' : PosState} (h : payDue s n = some s') :
    totalRev s' = totalRev s := by
  unfold payDue at h
  split at h
  · simp only [Option.some.injEq] at h
    subst h
    simp only [totalRev]
    omega
  · exact absurd h (by simp)

/-- Steps 1, 2 and 4 leave the coin fields exactly as they were: they are ledger steps. -/
theorem the_ledger_steps_leave_the_coins (r : Validator → Nat) (s : PosState) (n : Nat) :
    totalRev (commitRewards r s) = totalRev s ∧
    totalRev (movePending s) = totalRev s ∧
    totalRev (reselect s) = totalRev s :=
  ⟨rfl, rfl, rfl⟩

/-- **Law 44/47 — conservation of an epoch**: whatever the transition pays, it pays out of the staking
    vault, so the REV held by users, the staking vault and the Coop vault is invariant across an epoch.
    This is the theorem the file could not state before it had a state machine, and the one whose absence
    let the port mint in `slash` until `ba9e259a7`. -/
theorem epochStep_conserves (r : Validator → Nat) (s : PosState) (n : Nat) {s' : PosState}
    (h : epochStep r s n = some s') :
    totalRev s' = totalRev s := by
  -- `reselect` is the outermost step and a ledger step, so it comes off first; then the payment, then
  -- the two ledger steps beneath it. Each `have` is the lemma instantiated at the state it is applied
  -- to, which is what `rw` needs — `totalRev (reselect ((…)))` does not contain `totalRev (payDue …)`
  -- as a subterm, so the rewrites have to peel one layer at a time.
  unfold epochStep at h
  cases hpay : payDue (movePending (commitRewards r s)) n with
  | none => simp [hpay] at h
  | some paid =>
      simp only [hpay, Option.map_some, Option.some.injEq] at h
      cases h
      have hcon := payDue_conserves (movePending (commitRewards r s)) n hpay
      have hres := (the_ledger_steps_leave_the_coins r paid n).2.2
      have hmov := (the_ledger_steps_leave_the_coins r (commitRewards r s) n).2.1
      have hcom := (the_ledger_steps_leave_the_coins r s n).1
      rw [hres, hcon, hmov, hcom]

/-- Off a boundary the transition writes nothing — stated of `closeBlock`, so it is a fact about the gate
    rather than a restatement of a branch inside the transition. -/
theorem closeBlock_off_a_boundary (r : Validator → Nat) (s : PosState) (n : Nat)
    (h : isBoundary s n = false) : closeBlock r s n = some s := by
  simp [closeBlock, h]

/-! ### The release rule (law 47) and the gate's two halves (law 44) -/

/-- **Law 47, stage one — staged, not moved.** A withdrawal records a deadline and changes nothing else:
    the validator is still in the pool, still active, and no coin has moved. -/
theorem a_staged_withdrawal_moves_no_coins (s : PosState) (v : Validator) (n : Nat) :
    (stage s v n).requests = ⟨v, withdrawDeadline s n⟩ :: s.requests ∧
    totalRev (stage s v n) = totalRev s ∧
    (stage s v n).pool = s.pool ∧
    (stage s v n).active = s.active :=
  ⟨rfl, rfl, rfl, rfl⟩

/-- **Law 47, stage two — escrowed, not paid.** At the boundary after the request the bond leaves the pool
    and becomes a claim carrying the recorded deadline, and the payer's vault is still empty: the claim
    stores the **bond only** (the contract stores `allBonds.get(pk)` at `Pos.rhox:582` and adds
    `committedRewards` at `:604`, which is the order the port keeps). -/
theorem the_move_escrows_the_bond_and_pays_nothing (s : PosState) (v : Validator) (stake d : Nat)
    (h : s.pool.find? (fun wb => wb.1 = v) = some (v, stake)) :
    (movePending { s with requests := [⟨v, d⟩] }).claims = s.claims ++ [⟨v, stake, d⟩] ∧
    (movePending { s with requests := [⟨v, d⟩] }).user = s.user := by
  constructor
  · simp [movePending, h, List.filterMap_cons]
  · rfl

/-- **Law 47, stage three — paid `bond + committed`.** What a due claim is paid is its bond plus the
    reward committed to it (the first conjunct is the definition, so a payout that dropped either part
    would fail to type-check here), and a successful payment credits the validator's vault by exactly
    that (the second, under the same guard the port applies). -/
theorem a_due_claim_is_paid_its_bond_plus_its_committed (s : PosState) (c : PosClaim) {s' : PosState}
    (h : payDue s c.deadline = some s') :
    payoutOf s c = c.bond + lookup s.committed c.who ∧ s'.user = s.user + duePayout s c.deadline := by
  refine ⟨rfl, ?_⟩
  unfold payDue at h
  split at h
  · cases h
    rfl
  · exact absurd h (by simp)

/-- **…and a claim before its deadline is not paid**: it is still in the ledger after the boundary's step
    3, which is what "only the expired quarantines are paid" means for the claim itself. -/
theorem a_claim_before_its_deadline_is_not_paid (s : PosState) (c : PosClaim) (n : Nat)
    {s' : PosState} (hc : c ∈ s.claims) (h : ¬ c.deadline ≤ n) (hp : payDue s n = some s') :
    c ∈ s'.claims := by
  unfold payDue at hp
  split at hp
  · simp only [Option.some.injEq] at hp
    subst hp
    simp only [List.mem_filter]
    exact ⟨hc, by simp [h]⟩
  · exact absurd hp (by simp)

/-- **Law 44 — pooled, not activated.** A bond moves the stake into the staking vault and joins the pool,
    and leaves the active set exactly as it was; activation is the boundary's step 4, which is why the
    port's own test must call `close_block` before the validator is in the consensus set. -/
theorem a_bond_pools_but_does_not_activate (s : PosState) (v : Validator) (stake : Nat) :
    (bond s v stake).pool = (v, stake) :: s.pool ∧
    (bond s v stake).active = s.active ∧
    (bond s v stake).vault = s.vault + stake ∧
    (bond s v stake).user = s.user - stake :=
  ⟨rfl, rfl, rfl, rfl⟩

/-- **Law 44 — and the boundary is what activates it.** With no claim escrowed, step 4 makes the pool —
    the newly bonded validator included — the active set, so a bond and a boundary in the same block do
    activate it. This is the port's `bond_escrows_the_stake_and_activates_at_the_boundary` in the model. -/
theorem a_boundary_activates_the_pool (r : Validator → Nat) (s : PosState) (n : Nat)
    {s' : PosState} (hp : payDue (movePending (commitRewards r s)) n = some s')
    (h : s'.claims = []) :
    ∃ s'', epochStep r s n = some s'' ∧ s''.active = s'.pool.map (·.1) :=
  ⟨reselect s', by simp [epochStep, hp], by simp [reselect, h]⟩

/-- **The ordering the release rule depends on**: the move of step 2 leaves the committed ledger
    exactly as step 1 wrote it, so a validator that leaves the pool at this boundary is still paid
    against a reward committed for the epoch it was in. The two steps are ordered commit-then-move in
    `epochStep` for this reason (`native_state.rs:1099-1125`, `Pos.rhox:568-588`). -/
theorem the_move_does_not_disturb_the_ledger (r : Validator → Nat) (s : PosState) (v : Validator) :
    lookup (movePending (commitRewards r s)).committed v
      = lookup (commitRewards r s).committed v := by
  simp [movePending]

/-- **…and the reward is *in* that entry**: the same ordering checked on the run the port's own test
    builds — one pooled validator, a withdrawal staged at block 3, the epoch reward committed at the
    boundary. The value is read back from the ledger **after** the move, so the theorem fails if the
    commitment is dropped, reordered after the move, or keyed to the wrong validator. (The general form
    of this — for any pool and any reward function — is a `foldl` induction over `setKey` and is owed;
    this is the instance the port's test exercises, decided rather than described.) -/
theorem the_reward_is_committed_before_the_leave :
    lookup (movePending (commitRewards (fun _ => 5)
      { vault := 40, coop := 0, user := 0, pool := [(⟨0⟩, 40)], active := [⟨0⟩],
        requests := [⟨⟨0⟩, 9⟩], claims := [], committed := [(⟨0⟩, 0)],
        epochLength := 1, quarantineLength := 0 })).committed ⟨0⟩ = 5 := by
  decide

end Rchain
