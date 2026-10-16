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
- **Part of what a deploy burned is paid to the producer of the block that carried it**, before the rest
  reaches the pot — `executor-share`, a quarter of it by default, so the epoch pot is the burned phlo
  *minus* the producer's shares. That is the one place the protocol's payout reads who did the work, and
  it is a payment out of the same vault rather than a second pot: the pie is what was burned, and this
  decides who gets a slice of it (below).

## The developer allocation is a genesis vault, not a levy

One REV address is credited at genesis that **no wallets file names**: an allocation to the developer, of
`max(20 000 000 REV, 2% of the total the wallets file allocates)`. The two arms cross at exactly one
billion REV, so below that allocation the floor governs and at or above it the rate does — and on the
allocation this network has (908 962 714 REV, dated 13 May 2026 — see [the history chain](history-chain.md))
2% is 18 179 254 REV, under the floor, so the allocation is **20 000 000 REV** and the
wallets-plus-allocation total becomes **928 962 714 REV**.

Two things follow from *where* it sits, and both are worth stating because the same numbers read very
differently as a different mechanism:

- **It is not a levy on fees, and it is not a mint.** REV's issuer has no mint arm, so nothing here
  creates REV: the vault is written once, at genesis, by the same installation that writes every other
  wallet. `executor-share` and the epoch pot are untouched, and no boundary pays the allocation anything.
- **It is consensus state, and it is permanent.** The balance is installed outside the genesis
  block's deploys and content-addressed into the genesis post-state hash (AUDIT C46), so the amount must
  be one function of the wallets file — which is why it is *derived* rather than written into that file.
  The function is of the total the genesis will **install**, which is why the allocation boundary
  refuses a wallets file that repeats an address: `set_vault_balance` overwrites a repeated key, so the
  rows would not be a total the chain holds. Only the recipient address is a setting
  (`casper.genesis-block-data.dev-tax-address`), and a genesis created outside `--dev-mode` without one
  is refused at the launch branch. After genesis it cannot change without a hard fork.

- **It unlocks on a schedule, and that is a separate rule.** The allocation is credited to a *vesting
  account*, not a spendable vault — nothing is spendable at genesis — and each **epoch boundary** moves
  what has vested into the recipient's own vault, where it is an ordinary balance. The schedule is
  linear, with no cliff: zero at genesis, total after `dev-tax-vesting-blocks`, truncated toward the
  network so the released amount never runs ahead. That term is a **genesis parameter**, not a constant
  — 36 months at the cadence `defaults.conf` states — because the block cadence is not a constant in
  this tree ([scaling](scaling.md)), so a "month" is an assumption and the parameter is where it is
  written down. The rule is Law 79 (`spec/Rchain/Vesting.lean`); the release is granular to the epoch
  rather than the block, because native state is written at boundaries.

**The boundary of the "creator and joiner agree" claim.** Two nodes that both **replay** the genesis
derive the same vaults and so the same post-state hash — that is the property the derivation buys. A node
that instead **restores** the chain over LFS does *not* replay genesis (`insert_genesis` marks it
validated as it stands), so its local configuration is not checked against what it is restoring: on that
path the bootstrap's genesis is taken as given. That is the restore path's trust model, it predates this
allocation, and it is stated here because the claim is narrower than "a disagreeing joiner is refused"
would suggest.

The rule, its two crossover directions and its non-vacuity witness are Law 78
([`spec/Rchain/Genesis.lean`](../../../spec/Rchain/Genesis.lean)); the deviation from the Scala oracle is
recorded in [`spec/audit/passes.md`](../../../spec/audit/passes.md) §138. It **increases the allocated
total by 20 000 000 REV (2.2003%)**, which dilutes the existing holders' proportional share by 2.1529% —
the 13 946 addresses the spreadsheet lists keep their balances, and the total they are a share *of* grows.
The 928 962 714 REV above is the **wallets-plus-allocation** subtotal: the genesis also funds the staking
vault with the bond sum (`install_genesis`), so it is not all of the REV the genesis writes.

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
else. Not attestations made, not how many blocks it proposed, and not whether it was **live at all**. An
absent validator is paid on the same terms as one that ran all epoch, because there is **no inactivity
leak, no decay and no eviction** in this tree: a stopped validator's stake stays in the pool and counts
in the finality denominator for ever. That is measured and written up in
[The public testnet](testnet.md) (the *Do not onboard a validator yet* measurements), and
[#149](https://github.com/rchain-community/rchain-rust/issues/149) owns the consequence — its
predecessor #148 was closed into it with the close condition carried verbatim: a validator that bonds
and goes offline makes production unbounded and freezes finality, **and still collects its share** of
whatever the epoch's pot does hold.

**The producer's share is the exception, and it is outside this formula.** It is not part of
`epoch_rewards` at all: `pay_executor` (`rholang/src/native_state.rs`) runs at the end of each deploy's
cost accounting and pays the **block's own signed sender** a share of what that deploy burned, so the
money never reaches the pot the formula divides. That makes production the one thing the payout reads
about a node's work — and it is coarse rather than graduated: a block's producer is paid for the deploys
it carried, weighted by what they burned, and nothing else it did (attesting, relaying, staying up)
pays anything at all.

So the protocol's liveness signal remains the one finality carries — `Participation`, named in
[Progress: the shapes of non-progress](../formal/progress.md) — and liveness decides whether the chain
*finalises*. What a validator earns is a function of two things only: **how much stake it holds** (the
formula above, if it is drawn) and **how much phlo the blocks it signs burn** (the producer's share).
Everything else a node does for the network is unpaid.

## What the block's *producer* is paid, and what an absent validator forgoes

Two payments sit outside the epoch formula, and both read the block rather than the boundary.

**The producer's share.** A deploy's phlo is charged into the staking vault by `pre_charge` and the
unconsumed part returned by `refund`, so what the vault keeps for a deploy is exactly what that deploy
burned — and that is what the epoch pot is made of. `pay_executor` takes `executor-share` basis points of
it (**a quarter** by default, set at genesis) and pays it to the block's **own signed `sender`**, at the
end of that deploy's cost accounting. The producer is the one party the protocol can identify without new
state and that nobody but the signer can steer, so this is the *only* place a payout reads who did the
work — and it is coarse: a block's producer is paid for the deploys it carried, weighted by what they
burned. Attesting, relaying and staying up pay nothing.

It is a **reallocation inside a fixed pie**, not an emission: nothing is minted and the epoch pot is
smaller by exactly what was paid out. `executor-share = 0` is `Pos.rhox`'s behaviour — the contract pays
a producer nothing beyond its share of the pot — and it is what a params record written before the key
existed decodes to. See [`spec/RUST-FIRST.md`](../../../spec/RUST-FIRST.md) for the transfer table, and
`payExecutor_conserves` in [`spec/Rchain/Pos.lean`](../../../spec/Rchain/Pos.lean) for the conservation
that holds across it.

**The absence rule, off by default.** A validator's epoch reward is scaled by its **participation**: how
far behind the last finalised fringe its latest *message* sits. Each node derives that itself from its own
DAG — the same object the epoch's seed is anchored to, and for the same reason, since it is agreed by
more than two thirds and a lone proposer does not move it. Nothing about it is carried on the wire.

Two heights parameterise the weight, and the reason there are two is a property rather than a taste: a
rule that pays an honest validator *in full* and reaches zero somewhere else cannot be a function of a
single threshold, because the ramp would have to begin at lag zero and would put a haircut on every live
validator.

- `participation-grace` — inside this many heights of the boundary, the share is whole.
- `absence-slack` — the **knee**: at this lag the weight reaches zero, with a linear ramp between.

`participation-grace = absence-slack` is a simple threshold rule — full inside, nothing past it — and
that is what a configuration written before the grace existed resolves to.

Three properties, and each is why the rule is shaped as it is. It **cannot touch a bond** — it scales the
rewards a boundary is about to commit, and a stake is not reachable from there. It **is not a slash** —
nothing about it appears in `mark_failed`, the offence predicate, or any exemption ledger. And it
**recovers in full**: the weight is a function of the fringe, so one message puts a validator back inside
the grace and the next boundary pays it exactly as if it had never been away.

**The shipped default is `absence-slack = 0`, which is off.** That is `Pos.rhox`'s behaviour — in the
contract absence costs nothing and an absent validator is paid for being drawn — and it is a rule the
contract does not have, so a network chooses it rather than inheriting it. It is also **not a fix for the
liveness defect**: a validator that goes offline still freezes finality and still counts in the
denominator ([#149](https://github.com/rchain-community/rchain-rust/issues/149)); this rule only decides
that it is paid less for the boundary it sat out.

**Arming it is a genesis decision, and the numbers are the network's to make.** Both parameters live in
the genesis params record, so two nodes that disagree compute different post-states — and the value a
network picks has to come from how far behind the fringe its *live* validators actually sit on its own
chain, which is a property of its block rate and its active-set size rather than something this port can
hand it. **A boundary logs exactly that**, whether or not the rule is armed, so a network can measure
before it chooses:

```
[pos] participation lag at boundary 40000: drawn=100 never_spoke=0 min=4 median=6 p90=31 max=88
```

**And the first thing that log will tell you is not what you would guess: a validator that missed
*nothing* does not read as zero.** The lag is measured against the last **finalised** fringe, so it
carries the finality lag — on the rig this was developed on, a constant 4, which is that chain's own
all-live finality gap. So the knee is a *sum*: the finality lag, plus the silence you want to price.
Picking `absence-slack 5` because "a validator that missed a round or two should be cut" would tax
validators that missed nothing at all. Measure first; the number is yours and nobody else's
(`spec/audit/evidence/participation-lag-results.md` carries the run and its limits).

## Who is paid: the drawn set, and where "pro-rata" stops holding

`epoch_rewards` pays the **active** set. It writes an entry for **every** pooled validator and the entry
is `0` for anyone outside the active set, so a validator that is bonded but not drawn for the epoch earns
nothing in it.

The active set is not a ranking. It is recomputed only at an epoch boundary (`select_active`), and when
the cap bites it is a **seeded stake-weighted draw without replacement** from the eligible pool —
positive stake, not withdrawing — with the cap set by `number-of-active-validators`
(`node/src/configuration/defaults.conf:334`, default `100`). The seed comes from the last **finalised**
fringe, one boundary ahead. The reason, the residuals, and the alternative are in
[`spec/RUST-VS-SCALA.md`](../../../spec/RUST-VS-SCALA.md) §3 item 12; the residuals are named **O1–O4**
there and not repeated here.

**There are two regimes, and they differ in the answer to "does the largest stake earn most?"**

- **The cap does not bite** — the eligible pool is within `number-of-active-validators`, which on a
  bonded network of ≤100 validators is simply all of it. Every pooled validator is active, nobody is
  undrawn, and the split is stake-proportional: the largest bond takes the largest share.
- **The cap bites.** Membership is a weighted draw, so a larger stake is drawn in proportion to its size
  and a member that is not drawn earns nothing that epoch. Stake still predicts income — but not exactly
  pro-rata, and which way it misses is worth knowing (below).

**How far from pro-rata, on the shipped rule** (pot 1, six rival keys at stake 10, cap 4 — the same
measurement before and after the draw was weighted on 2026-10-02; 4 096 fixed seeds, so these are pins
rather than samples):

| how one stake of 40 is held | uniform draw (before) | weighted draw (now) |
|---|---|---|
| one key of 40 | **0.326** — *below* the flat pro-rata 0.400 | **0.5328** — *above* it |
| four keys of 10 | 0.400 — exactly pro-rata | 0.4000 — exactly pro-rata |
| twenty keys of 2 | **0.528** — 62 % *more* than whole | **0.1770** — 56 % *less* |

Each pair is the same pool at the same total stake, differing only in how the stake is held. **Under the
uniform rule the lever was the number of keys, and splitting was worth 62 %** — a per-key rule mints
finality weight (and income) for free, which is a consensus-safety problem and not just an economic
quirk. **Under the weighted rule the lever is the size of the key, and splitting costs 56 %.** Neither
regime is exactly pro-rata; the cap is what breaks proportionality, and the two rules differ only in
which side of it a staker lands on. Weighting is the side chosen: it is the side on which **stake buys
weight**, which is what the cap exists to bound — a bond is bounded above by `maximum_bond`, and the
concentration is the operator's own risk, borne per validator since everything at risk is per-validator.

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
  byte-array argument cannot bond another key (`rholang/src/system_processes.rs:1714`). **A key can,
  however, stake on another key**: `rho:rchain:pos!("delegate", *deployerId, operatorPubKey, amount,
  *ret)` moves the principal out of the *delegator's* own vault and adds it to the operator's bond
  (2026-10-02, [#193](https://github.com/rchain-community/rchain-rust/issues/193), law 57). There is
  still **no staking pool, no restaking and no liquid-staking mechanism** in this tree: delegation
  attributes stake to an operator's key and gives the delegator no transferable claim, no share and no
  vote — see item 1 below for what it does and does not buy.
- **Exiting is staged and quarantined.** `withdraw` (`:1164`) only *stages* a request: the validator
  stays bonded and active and keeps earning until the next epoch boundary, when the bond leaves the pool
  and is escrowed until `quarantine-length` has passed; it is then paid `bond + committed` rewards. See
  [Operating the node](operating.md) and [`spec/RUST-FIRST.md`](../../../spec/RUST-FIRST.md) § *Dynamic
  validators* for the lifecycle.

The practical shape of that: the party that can be slashed is the party that staked, and it is the same
party — the operator's own capital, with no one between the operator and the bond.

## Open questions (tracked, not decided)

**Everything in this section is design discussion, not implemented and not proved** — with one qualified
exception, marked in place: item 1's mechanism is now **decided, its arithmetic proved and its Rust
landed** (law 57, 2026-10-02), and only its end-to-end evidence is owed. Each item lives on the issue
named with it; per [`AGENTS.md`](../../../AGENTS.md) the plans and hypotheticals behind them belong
outside this repository, and none of it is policy.

**A review of these questions from fresh concluded that neither of the two obvious improvements works as
stated.** Both conclusions are arguments about the code above, not preferences.

1. **Should the reward be validator-agnostic and network-weighted — a pooled staking contract?** *Not on
   this protocol, and it would make risk worse.* A pool exists to bond stake that is not the operator's,
   and the two primitives that needs are not both present: `bond` takes only the deploy signer's own
   unforgeable `deployerId` (`rholang/src/system_processes.rs:1707`), and a reward is paid only to the
   vault derived from `fromPublicKey(validator)` (`rholang/src/native_state.rs:1086`). So **every pool
   reachable today is one more validator key** — which *concentrates* the slash on the operator rather
   than spreading it, and leaves members with no on-chain claim at all. Above the cap it is the wrong
   direction economically too: merging many small stakes into one key **raises** income under the
   weighted draw (0.1770 for twenty keys of 2 against 0.5328 for one key of 40, above) — a pool
   *concentrates* the stake it claims to spread, and it concentrates the cap's reward too. A pool that
   would actually spread risk needs a new primitive — bonding from a named vault, or a delegation leaf —
   which is a genesis-plus-hard-fork change, not a contract.

   **Decided 2026-10-02, and it is the second of those two: a delegation leaf**
   ([#193](https://github.com/rchain-community/rchain-rust/issues/193)). `pos:bonds` becomes the
   **aggregate** stake on a key — the operator's own plus every delegation to it — and a new ledger
   `pos:delegations` records who owns which part, so the stake counts in the draw and in what a slash
   reaches exactly as the operator's own does while the attribution has an on-chain home. Direction 2 was
   rejected for a reason worth stating: bonding from a named vault needs a third party to spend a named
   account, the capability [`spec/RUST-FIRST.md`](../../../spec/RUST-FIRST.md)'s B2 decision declined,
   while the ledger needs no capability at all — a delegator spends from **its own** vault through **its
   own** `deployerId` and the protocol attributes the stake.

   **The economics of the primitive, stated as it actually is rather than as a pool would be sold.**
   *It does not spread risk.* A delegation joins the operator's aggregate entry, which is what the
   stake-weighted draw weights — so a delegator's REV **does** buy draw weight through the operator's
   key, and the cap regime decides what that weight is worth: the measurement above (**0.5328 / 0.4000 /
   0.1770** for one key of 40, four of 10, twenty of 2) says **delegating into one large operator earns
   above pro-rata**. The primitive gives a small staker access to a concentration it could not assemble
   alone; it does not give it a diversified position, and no one should describe it as spreading stake. A
   delegator also takes the operator's behaviour with the operator's key: its principal is inside the
   bond a slash reaches (pro-rata, at the same tier as the operator's own stake), and the operator's
   participation weight scales the reward the delegators' share is drawn from, so an operator that stops
   attesting withholds its delegators' income with its own. No commission is charged and none is
   specified, and the operator can neither refuse a delegation nor set terms — all of which is the shape
   [#193](https://github.com/rchain-community/rchain-rust/issues/193) settled on and RCHIP #32's
   staking-pool interface does not.

   **What is landed and what is owed, because this section's own rule is that a claim names its
   artifact.** The mechanism is specified in
   [`spec/RUST-FIRST.md`](../../../spec/RUST-FIRST.md) § *Delegated stake*, modelled as **law 57** in
   [`spec/Rchain/Pos.lean`](../../../spec/Rchain/Pos.lean) — the split's exactness, its identity with no
   delegators, the aggregate reaching the slash, and the clearing of the ledgers — and implemented in
   `rholang/src/native_state.rs` + `rholang/src/system_processes.rs`, with twelve `rustWitness` entries
   on law 57. **What is owed is the end-to-end evidence** and the read surface: the in-process
   measurement, a devnet run of delegate → boundary → undelegate → quarantine payout, and the
   delegator-scoped view on `GET /api/v1/pos` are the change's third unit, tracked as **C204** in
   [`spec/AUDIT.md`](../../../spec/AUDIT.md) — which stays open until they land, so the economics above
   are a decision, a proof about the arithmetic and a working primitive, but not yet a measured one.
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
> [`spec/LAWS.md`](../../../spec/LAWS.md) — read the status there, not here. **Delegated stake is law
> 57**, in the same file, and its deviations are `spec/audit/passes.md` §6. The active-set draw and its
> residuals (**O1–O5**) are [`spec/RUST-VS-SCALA.md`](../../../spec/RUST-VS-SCALA.md) §3 item 12.
