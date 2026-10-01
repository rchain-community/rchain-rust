# Does finality resume after a validator is killed? — pre-registered

**Status: FROZEN before the run.**

> **Correction, 2026-09-30 after the run: the premise this was frozen on is false, and the run's own series
> says so.** "Once the 50-stake validator is killed, finality stops" is the wrong reading of the campaign
> and of this arm. Finality stops **16 to 130 seconds into the run, with every validator live**: in this
> arm's three attempts it last moved at T+33s, T+30s, T+19s, T+16s and T+24s against a kill at T+120, and
> across both arms 14 of 16 survivor node-runs had finished finalising before the kill
> (`n127-liveness-results.md`). The kill neither causes the pin nor changes it, so the acceptance row below
> — "finality advances on the survivors after the kill, by more than the +4" — measures **when finality
> naturally pins**, and reading it as a verdict on the §32 fix is what produced "the fix did not work on the
> node". The rows are left frozen as written; this block is the correction, not a rewrite.
>
> The reproduction itself stands, and one thing in it is now known to be wrong: the **+4 the before-arm
> reached** was that arm's own late increment, not a head start over a working build.
>
> **Correction, added 2026-10-01: the "12–16 blocks/min" below is a *height* rate.** It comes from
> `n127-campaign-summarise.py`'s `rate()`, which is documented as "Blocks/minute … from … a height" and
> reads `latestBlockNumber` — `max_height + 1`, a **round** count, one block per bonded sender per round.
> The label at the source is fixed; the figure quoted here is left as written and read as 12–16
> **heights**/min (~3 blocks each, measured block-level in `n149-results.md`).

## Why this is the measurement

The campaign of 2026-09-30 ran this exact rig and found the opposite of what the register expected: with all
three validators live the chain is quiescent (12–16 blocks/min) and finality keeps up, and **once the
50-stake validator is killed, finality stops** — in **3 of 3 attempts**, two of them freezing completely
(height and finality both stopped for the remaining 180 s) and one advancing **+4 heights** and then
stopping. That +4 is the number to beat, and it is the number `LIVENESS_WINDOW = 5` predicts.

The cause was found and fixed (`80782e184`): the fringe derivation ranged over the *whole* justification set
while the partition is the *live* weight set, so a stopped validator's last message — which `latest_msgs`
keeps for ever — held the coverage gate at four messages against three senders. With the fix the derivation
ranges over the partition, and a 2-of-3 survivor set holding 80 % of the stake is a supermajority.

## The reproduction (fixed, and identical to the campaign's)

Three validators, `--stakes 100,100,50 --epoch-length 10 --fresh`, devnet defaults (autopropose on,
propose-on-deploy on, attest-on-new-blocks on), an 8 GiB cgroup with swap off, a 300 s window, four deploys
at T+30 s, and `tools/devnet.sh stop 2` (the 50-stake validator) at T+120 s. ≥ 3 attempts, unfiltered, one
devnet at a time. Artifacts under `target/n127-liveness/<tree>-<utc>/`, every file carrying the tree.

## The acceptance row, frozen

| observation | verdict |
|---|---|
| finality advances on the survivors after the kill, in **every** attempt, by more than the **+4** the before-arm reached | **the fix works** — the survivors finalise without the departed validator |
| finality advances in some attempts and freezes in others | **partially fixed**, and the freezing attempts are reported individually rather than averaged away |
| finality does not advance after the kill in any attempt | **the fix does not work on the node**: the in-process falsifier passes and this rig does not, which is a finding about the *other* stop, not about this one |

**The single number to report:** the finalised height at T+120 (the kill) and at T+300, per node per
attempt, beside the height at the same instants — because a height that runs while finality is pinned is a
different failure from a chain that stops, and the campaign saw both.

## What this does not settle

- It does not measure the merge budget (C184). That bound refuses work; this run's scopes are the honest
  ones, so the budget is expected **not** to engage, and its `SearchBudgetExceeded` line is read as a
  control: if it fires here, the budget is set too low and the number is wrong.
- It does not bound the exponential, and it is not a claim about an attacker.
- Three attempts of one configuration on one machine: a fix that holds here is not a proof, it is the
  measurement the register asks for before the claim is written down.
