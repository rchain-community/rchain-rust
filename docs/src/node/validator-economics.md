# Validator economics

What a validator is **paid**, what it can **lose**, and how the two line up. The mechanics live
elsewhere and this page links them rather than restating them: the native PoS state and the epoch in
[`spec/RUST-FIRST.md`](../../../spec/RUST-FIRST.md), the reward arithmetic and its theorems in
[`spec/Rchain/Pos.lean`](../../../spec/Rchain/Pos.lean) (laws 44–47 — see
[`spec/LAWS.md`](../../../spec/LAWS.md) for their status, which is the authority and is never restated
here), and the active-set draw with its residuals in [`spec/RUST-VS-SCALA.md`](../../../spec/RUST-VS-SCALA.md)
§3 item 12.

**What follows is of two kinds, and each part says which.** A sentence about the protocol's behaviour
names the artifact that holds it — a `.rs` or `.lean` file. A sentence about what the rewards *should* be
is **design discussion**: not implemented, not proved, tracked on an issue rather than decided here. Per
[`AGENTS.md`](../../../AGENTS.md), plans and hypotheticals live outside this repository, so this page
keeps only the questions.

## The pot is deploy phlo, not a mint

The staking vault is funded at genesis with **exactly** the bond sum, so the epoch pot is **zero at
genesis**; from then on the pot is the phlo the deploys since the last boundary actually burned. A
deploy is charged its maximum phlo by `pre_charge` (`rholang/src/native_state.rs:1788`) and the unused
surplus returns to the deployer through `refund` (`:1838`), so what stays in the vault is what was spent.
The pot is `epoch_pot` (`:668`) — the vault less every outstanding claim — and the transfer table is in
[`spec/RUST-FIRST.md`](../../../spec/RUST-FIRST.md) § *The staking vault*.

Two consequences follow directly.

- **Rewards are paid out of fees, not minted.** Nothing creates REV at a boundary; the epoch moves it
  from the vault to the validators.
- **The validator that executed a deploy is paid exactly as one that executed nothing.** The deploy's
  phlo goes into the pot, and the pot is split by stake (below), not by who did the work.

## The formula: proportional to stake, in units of the minimum bond

At each epoch boundary every pooled validator's share is committed by `epoch_rewards`
(`:1373`), computed by `epoch_reward` (`:706`):

```
reward_i = pot * (bond_i / minimumBond) / (activeBonds / minimumBond)
```

Two integer divisions. Their consequence is proved rather than assumed: the shares do **not** sum to the
pot, and the remainder is not lost — it stays in the pot and the next epoch distributes it.
`spec/Rchain/Pos.lean` states it as the inequality `sum_rewards_le_pot` and decides a strict instance
(`the_dust_is_real`); the port pays **zero**, rather than faulting, in the two cases where the contract's
formula is undefined (a zero minimum bond, or a zero normaliser — `epoch_reward`'s early returns at
`:707`).

**Derived** (arithmetic on the two lines above, no artifact needed): the split is proportional to stake
only in whole multiples of `minimum_bond`. At the default `bond-minimum = 1`
(`node/src/configuration/defaults.conf:319`) the flooring never bites and the share is exactly
stake-proportional. Set `minimum_bond = 3` and two validators holding **4** and **5** both floor to a
factor of `1` — they are paid **the same**, whatever the pot. (That is a property of a validator's `bond`
under a configured minimum; it is not `PosParams::default()`, whose minimum is `0` and under which
`epoch_reward` returns zero for everyone.)

## What the formula does not read

The inputs are the pot, `minimum_bond`, `active_bonds` and the validator's own `bond` — and nothing
else. Not blocks proposed, not attestations made, not deploys executed, and not whether the validator
was **live at all**. An absent validator is paid on the same terms as one that ran all epoch, because
there is **no inactivity leak, no decay and no eviction** in this tree: a stopped validator's stake stays
in the pool and counts in the finality denominator for ever. That is measured and written up in
[The public testnet](testnet.md) (the *Do not onboard a validator yet* measurements), and
[#149](https://github.com/rchain-community/rchain-rust/issues/149) owns the consequence — its
predecessor #148 was closed into it with the close condition carried verbatim: a validator that bonds
and goes offline makes production unbounded and freezes finality, **and still collects its share**.

So the protocol's only performance input is the liveness hypothesis finality carries — `Participation`,
named in [Progress: the shapes of non-progress](../formal/progress.md) — and liveness affects whether the
chain *finalises*, not what a validator *earns*. Nothing in the reward is a function of the work a node
did.

## Who is paid: the drawn set, and where "pro-rata" stops holding

`epoch_rewards` pays the **active** set. It writes an entry for **every** pooled validator and the entry
is `0` for anyone outside the active set, so a validator that is bonded but not drawn for the epoch earns
nothing in it.

The active set is not a ranking. It is recomputed only at an epoch boundary (`select_active`, `:580`),
and when the cap bites it is a **seeded uniform sample without replacement** from the eligible pool —
positive stake, not withdrawing — with the cap set by `number-of-active-validators`
(`node/src/configuration/defaults.conf:334`, default `100`). The seed comes from the last **finalised**
fringe, one boundary ahead. The reason, the residuals, and the alternative are in
[`spec/RUST-VS-SCALA.md`](../../../spec/RUST-VS-SCALA.md) §3 item 12; the residuals are named **O1–O4**
there and not repeated here.

**There are two regimes, and they differ in the answer to "does the largest stake earn most?"**

- **The cap does not bite** — the eligible pool is within `number-of-active-validators`, which on a
  bonded network of ≤100 validators is simply all of it. Every pooled validator is active, nobody is
  undrawn, and the split is stake-proportional: the largest bond takes the largest share.
- **The cap bites.** Membership is a uniform draw, so expected income is close to **uniform across the
  drawn members** whatever their stake, and a member that is not drawn earns nothing that epoch. In this
  regime "the largest stake earns most" is false, and what is true instead is the residual **O3**:
  per-unit-of-stake income *favours* a stake split across several keys, since each key draws
  independently.

**How large that reversal is, on the shipped rule** (derived from `select_active`'s uniform draw; pot 1,
six rival keys at stake 10, cap 4, 200,000 trials):

| how one stake of 40 is held | expected income |
|---|---|
| one key of 40 | **0.326** — *below* the flat pro-rata benchmark of 0.400 |
| four keys of 10 | 0.400 |
| twenty keys of 2 | **0.528** — **62 % more** than holding it whole |

So above the cap the operative lever is **the number of keys, not the size of the stake** — per-unit
income falls as one's own stake grows, because a large drawn key inflates the normaliser the share
divides by. This is O3 read from the validator's side rather than the network's, and it is a property of
the rule as shipped, not of any proposal.

Which regime a net is in is a property of its size, not a policy — and the two are the same code path
(`select_active` returns the whole eligible pool when the cap does not bite).

## The asymmetry: everything at risk, one epoch's phlo at reward

What is at stake is **everything the validator holds in the PoS system** — its bond, its accrued and
unwithdrawn rewards, and an escrowed withdrawal claim — and what is earned is its share of one epoch's
burned phlo. The two exits from the active set are not symmetric, and the asymmetry is deliberate rather
than an accident of the code — [Running a public testnet of your own](running-a-public-testnet.md) states
it:

> A silent validator is not slashed; it is a drag instead. […] The protocol is asymmetric: going offline
> is free, while a block that *fails validation on another node* costs the sender its stake.

So a validator that never proposes keeps its bond and (if drawn) still earns; a validator that commits an
offence loses a **stated share** of what it holds, and the share is set by what it did (below). Nothing in
this tree **underwrites** that risk — no insurance, no reward floor, and (below) no delegation to spread
it — and there is no programme that shares the income either, which is the risk/reward shape the page
returns to. That is an observation about the code, not a theorem about incentives: it says what the
protocol does, not that the balance is the one a rational operator would choose.

## Slashing: behavioural, trustless, and graded

A slash is decided by **what the block did**, not by who is watching, and every node checks the
producer's work.

**Behavioural.** A failure is recorded in one place, `mark_failed`
(`casper/src/multi_parent_casper.rs`), which takes the refusing status and derives **both** the cause and
whether it is an offence from it:

```rust
validated: true,
validation_failed: true,
slashable: status.is_slashing_offence(),
slash_severity: status.slash_severity().unwrap_or(SlashSeverity::Unspecified),
failure_cause: Some(status.failure_cause()),
```

The cause is `FailureCause` (`models/src/block_metadata.rs`) — `Attributable` (the block's own fault,
together with the DAG's structure), `Divergence` (this node's state or replay disagreed — the measured
`InvalidStateHash`, [#105](https://github.com/rchain-community/rchain-rust/issues/105)), or `Cascade` (a
justification failed). **Only `Attributable` is the block's fault** — one transient failure must not
fabricate slash evidence against every validator above it
([#125](https://github.com/rchain-community/rchain-rust/issues/125)) — and the partition is
`BlockStatus::failure_cause` (`casper/src/block_status.rs:129`).

**But the offence set is narrower than the blame set**, which is a separate question and a separate
predicate: `BlockStatus::is_slashing_offence` (`casper/src/block_status.rs`). Three `Attributable`
refusals read an input that belongs to the **receiving node** rather than to the block — the fee floor
`casper.min-phlo-price`, the width `casper.max-number-of-parents`, and the compiled version set
`SUPPORTED` — and none of the three is committed at genesis, so two correctly-configured operators may
hold different values with no committed value to be wrong about. They are still refused, and they are no
longer offences: a node that reached the opposite verdict about blame would not merely disagree, it would
**refuse the block carrying the slash** — a permanent split, over a local setting, with the sender's whole
bond gone.

**Two ways a slash is justified, and neither is the proposer's word.** The first is a failed block: the
offence set is the senders of justifications whose metadata is `slashable`, intersected with the bonded
set — `slashable_senders` (`casper/src/validate.rs`), called through `slashable_offenders`
(`casper/src/blocks/proposer/proposer.rs`). The second is an **equivocation**.

**Equivocation: the one fault that needs no judgement, and it used to be free.** The H-1 gate refuses a
second block at a `(sender, seq_num)` the DAG already holds, and it refuses it **before any write** — so
the refused block was stored nowhere, was never a `BlockStatus`, and was therefore never an offence.
Double-signing cost a validator nothing while a stale deploy cost it its bond. Now the gate records the
refused block's **header**, the proposer attaches it to the slash of every recorded equivocation by a
bonded sender, and the header travels in the block's own state (`SystemDeployData::Slash`). It is a
*header* — sender, reused sequence number, the conflicting hash and its signature — and not the block,
because it lands in consensus data and a block refused at that gate has passed no check at all, so its
size would be whatever its sender chose.

**Trustless.** What makes either arm the *protocol's* rule rather than the proposer's is the receiving
side: `slash_is_unjustified` (`casper/src/interpreter_util.rs:170`) refuses a block whose slashes it
cannot re-derive from **its own** view. For the first arm, that is the slashable senders in the receiving
node's own DAG. For the second it is a signature and a conflict the receiver checks itself:
`validate::equivocation_is_proved` requires that the named offender's **own** signature covers the
evidence's hash, and that the receiver's DAG already holds a **different** block by that sender at that
sequence number. So a receiver needs neither to have seen the refused block nor to trust the proposer
that showed it — a forged, relabelled or non-conflicting payload is refused. The proposer's opinion of
the victim carries no weight, and the slash service is not auth-gated — that per-node check is the guard
(AUDIT C110 and C200, [`spec/audit/passes.md`](../../../spec/audit/passes.md)).

**Graded, and bounded.** `slash` (`rholang/src/native_state.rs`) removes the validator from the pool, the
active set, the withdrawers and the pending withdrawers — confiscation, not deactivation — and what it
takes is a **share of everything that validator holds in the PoS system**: its bond, its accrued and
unwithdrawn rewards, and an escrowed withdrawal claim. The share is set by the **tier of the offence**
(`SlashSeverity`, and the table is `BlockStatus::slash_severity`): a **forged** deploy — a signature that
does not verify against the key it names — or a **proved equivocation** takes all of it; a rule the
author's own block breaks takes a **quarter**; and a bound a stale deploy pool or a clock skew explains
takes a **tenth**. **The remainder
returns to the validator's own vault**, so the loss is exactly the tier and the worst case is one an
operator can read before bonding. A validator that offended more than once answers for the worst tier it
committed. (Before 2026-10-02 the rule took the *whole* bond whatever the offence, which is the exposure
the tiers exist to bound.)

That is the whole of the answer to "does slashing punish a software fault?" — the protocol already
declines to. A node that cannot replay a block, or replays it to a *different* state, records
`Divergence`, not `Attributable`, and a `Divergence` is never an offence.

**One divergence from the oracle, and it is not the bond.** `Pos.rhox`'s slash also **deletes** the
offender's accrued rewards from the committed map, in the same write as the bond zeroing — and this port
once did not, which stranded the balance in the vault rather than returning it to the pot. That was
**C197** in [`spec/AUDIT.md`](../../../spec/AUDIT.md), and it is **closed**: `slash` now removes the entry
with the rest of the validator's records, so the accrued amount goes back to the distributable pot, which
is what the oracle does. The bond is confiscated either way; what changed is only where the offender's
*unpaid* rewards end up.

## Admission: trust, self-bond, staged exit

The economics above are closed by who may take part.

- **Bonding is permissioned.** `bond` (`:1086`) refuses a key that is not in the **trusted** set
  (`pos:trusted`); a trusted stakeholder admits a key with `pos!("trust", …)`. A genesis validator is
  trusted by construction.
- **A key can only bond itself.** The bond takes the caller's own unforgeable `GDeployerId`, so a
  byte-array argument cannot bond another key (`rholang/src/system_processes.rs:1714`). **There is no
  delegation and no delegated-stake market in this tree** — no staking pool, no restaking, no
  liquid-staking mechanism anywhere in it.
- **Exiting is staged and quarantined.** `withdraw` (`:1164`) only *stages* a request: the validator
  stays bonded and active and keeps earning until the next epoch boundary, when the bond leaves the pool
  and is escrowed until `quarantine-length` has passed; it is then paid `bond + committed` rewards. See
  [Operating the node](operating.md) and [`spec/RUST-FIRST.md`](../../../spec/RUST-FIRST.md) § *Dynamic
  validators* for the lifecycle.

The practical shape of that: the party that can be slashed is the party that staked, and it is the same
party — the operator's own capital, with no one between the operator and the bond.

## Open questions (tracked, not decided)

**Everything in this section is design discussion, not implemented and not proved.** Each item lives on
the issue named with it; per [`AGENTS.md`](../../../AGENTS.md) the plans and hypotheticals behind them
belong outside this repository, and none of it is policy.

**A review of these questions from fresh concluded that neither of the two obvious improvements works as
stated.** Both conclusions are arguments about the code above, not preferences.

1. **Should the reward be validator-agnostic and network-weighted — a pooled staking contract?** *Not on
   this protocol, and it would make risk worse.* A pool exists to bond stake that is not the operator's,
   and the two primitives that needs are not both present: `bond` takes only the deploy signer's own
   unforgeable `deployerId` (`rholang/src/system_processes.rs:1707`), and a reward is paid only to the
   vault derived from `fromPublicKey(validator)` (`rholang/src/native_state.rs:1086`). So **every pool
   reachable today is one more validator key** — which *concentrates* the whole-bond slash on the operator
   rather than spreading it, and leaves members with no on-chain claim at all. Above the cap it is worse:
   merging many small stakes into one key **forfeits** precisely the key-count income the draw pays
   (above). A pool that would actually spread risk needs a new primitive — bonding from a named vault, or
   a delegation leaf — which is a genesis-plus-hard-fork change, not a contract. Tracked on
   [#150](https://github.com/rchain-community/rchain-rust/issues/150).
2. **Should the pot be weighted by participation as well as stake?** *It cannot do what it looks like it
   does.* The pot is a fixed pie of phlo already burned, so a multiplier is pure **reallocation** — with
   `p` in `[0,1]` it is exactly a haircut on absent stake, not a new reward axis. A uniform multiplier is
   a no-op, and a **correlated** absence (a cartel offline, a partition) is unpunishable by any
   per-validator score. The only signal available without new consensus state — the live weight set
   (`block-storage/src/dag/liveness.rs`) — is derived from the block's **own chosen justification set**,
   and nothing requires a block to carry every message it has seen, so the proposer of the boundary block
   could make a rival read absent and take the withheld share; that is the same lever that forced the
   epoch seed onto the last *finalised* fringe (residual O1). An epoch-accurate or volume-based score
   would need a counter leaf written on the block path, where native writes today ride only system
   deploys, breaking play/replay symmetry; either way it is hard-fork class. The claim
   [#150](https://github.com/rchain-community/rchain-rust/issues/150) attributes to an external result (a
   `welfare_game_potential`) does **not** survive stake-weighting: the ratio form stake-weighting forces
   has no exact potential. That external file is not in this repository and is not restated here.
3. **Should a deploy's phlo be shared with the block that executed it?** Today it is not: the phlo joins
   the pot and is split by stake, so the executor is paid like every other active validator. Also on
   [#150](https://github.com/rchain-community/rchain-rust/issues/150).

**And what the economy actually suffers from, which none of the three touches.** The honest loser above is
a validator with a *small* stake and one key, whose expected income is neither large nor stable; the
liveness defects are [#149](https://github.com/rchain-community/rchain-rust/issues/149) and
[#172](https://github.com/rchain-community/rchain-rust/issues/172); and **absence** is not a
reward-formula question at all. The instrument that prices a validator which stops is an **inactivity
leak** — a state change that burns a silent validator's stake — and this tree has neither the state nor
any issue that owns the design (the citations this page and the law register used to carry, #24 and #39,
are both closed and one of them is about something else).

(The stranded-rewards divergence above is *not* an open question of this kind — the oracle states the
intent, so it is a registered finding (C197) rather than a design choice.)

Any accepted change to any of the above is **hard-fork class**: rewards and slashing are consensus state,
so a change is a deviation from `Pos.rhox` registered in
[`spec/audit/passes.md`](../../../spec/audit/passes.md) §6 and classified on
[#51](https://github.com/rchain-community/rchain-rust/issues/51) category A.

## See also

- [Consensus (Casper)](consensus.md) — the fringe, the `> 2/3` rule, and the weight set the draw feeds.
- [Running a validator: hardware requirements](validator-requirements.md) — what a node *costs*; this
  page is what it *earns*.
- [Running a public testnet of your own](running-a-public-testnet.md) — the slash rule and the
  offline-is-free asymmetry stated for an operator.
- [Progress: the shapes of non-progress](../formal/progress.md) — the `Participation` hypothesis, and why
  an inactivity leak is out of scope.
- [Blockchain multi-stakeholder governance](../qucalc/multi-stakeholder-governance.md) — the governance
  framing around validator-only decision-making.

> **Formal.** The epoch gate, the reward split and the three-stage withdrawal are laws 44–47, modelled in
> [`spec/Rchain/Pos.lean`](../../../spec/Rchain/Pos.lean) and emitted to
> [`spec/LAWS.md`](../../../spec/LAWS.md) — read the status there, not here. The active-set draw and its
> residuals (**O1–O4**) are [`spec/RUST-VS-SCALA.md`](../../../spec/RUST-VS-SCALA.md) §3 item 12.
