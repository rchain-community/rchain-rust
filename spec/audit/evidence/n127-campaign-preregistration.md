# Phase 0 — the campaign: the storm, the shape distribution, and the stall's cause

**Status: FROZEN before the run.** The configuration and the acceptance rows below were fixed while no
result was known. A later change is a new commit saying why it was necessary and what it invalidated.

## What is measured, and what it is for

Three readings off **one** rig, because they want the same devnet:

| # | reading | what it decides |
|---|---|---|
| 0.1 | the block rate and the non-finalised region's growth, with a validator stopped mid-window | C171's baseline: is the attestation tap the fuel, and does finality recover |
| 0.2 | the merge scope-width and state-count **distributions** | C182's owed re-run, and the number Stage 2's threshold **N** is read from |
| 0.3 | which `NoAdvance` variant the stall line carries | #70's second stop: `Coverage` and `Support` imply different fixes |

The plan's record is `~/.claude/plans/partitioned-hugging-noodle.md`; the change order is
`spec/audit/passes.md` §30.

## The reproduction (fixed)

Three validators, `--stakes 100,100,50 --epoch-length 10 --fresh`, devnet defaults — **autopropose on,
propose-on-deploy on, attest-on-new-blocks on** (`tools/devnet.sh:411`; `configuration.rs:443-446`). This
is deliberately the *same* configuration as the Stage 1 shape run, so "before" and "after" are comparable
and comparable with the published 43-chain envelope. An 8 GiB cgroup with swap off, a 300 s window.

The perturbations, both at fixed offsets and neither optional:

- **four deploys** (`examples/hello.rho`) at T+30 s, so the chain has a deploy-bearing round;
- **`tools/devnet.sh stop v2`** at T+120 s — the 50-stake validator, so the survivors hold 80 %, which is
  the case #70 turns on.

Artifacts go in `target/n127-campaign/<tree>-<utc>/`, and **every filename carries the tree**: a run's
directory is unique, so a file from an earlier run cannot be read as this one's. `/metrics` is scraped
twice — at T+110, **before** the kill, so all three nodes contribute a distribution, and at the end of the
window, where only the survivors can. Each scrape carries the tree, the image id, the node, the attempt and
the scrape time in its header; a scrape from a node that is not serving writes *that fact* rather than a
stale body (C176's class — an artifact that cannot say what it measured).

> **Amendment 2026-09-30, after the run: that is not what happened, and it mattered.** "Scraped twice"
> describes this harness's own scrapes *only*. The sampler launched four lines earlier in the same script
> reads `/metrics` on every node **once a second for the whole window**, so the endpoint was scraped ~200
> times per node — and the endpoint accumulated per request over a cumulative registry, so those scrapes
> inflated the very histogram this run was reading. The measurement was inflated by the measurement; the
> exact protocol above is false as a description of the run. The instrument is fixed and the comparison
> that would have caught it on the spot is now the close condition
> (`n127-endpoint-vs-census-results.md`).

**Discipline:** one devnet at a time; ≥ 3 attempts, unfiltered; a node that dies is recorded, not dropped.

## The acceptance rows, frozen

**0.1 — the rate.**

| observation | verdict | consequence |
|---|---|---|
| the height advances faster than the 2 s autopropose timer alone can explain, and finality stays flat | the attestation tap is the fuel | C171's baseline stands; Phase 1's pace term targets it |
| the height advances at about the timer's rate | the timer is the fuel, not the tap | the reading is **void for C171**, and the configuration is recorded rather than reasoned about |
| finality advances with the height | no storm in this configuration | recorded; the 0.2 distribution is then an honest envelope and N may be read from it directly |
| the survivors do not resume finality after the kill | the partition, not the quorum, is binding | C174's territory; recorded, and the stall's variant comes from 0.3 |

**0.2 — the shape distribution.**

| observation | verdict | consequence |
|---|---|---|
| the histogram's boundaries are 16/32/64/128 (not 0.005…10) | the instrument is fixed (PR #132) | the distribution is readable and N may be keyed on it |
| the boundaries are the registry's defaults | the fix did not take | **void**: C182 reopens, and nothing may be gated on the quantity |
| two nodes on the same history report identical envelopes | deterministic | N may key on it at all |
| every bucket but the first is empty *and* the envelope is small | the run never reached a wide scope | **void for Stage 2**, and said so |

**0.3 — the stall's cause.**

| observation | verdict | consequence |
|---|---|---|
| the WARN line names `Coverage { missing, senders }` | a validator has no message in the unfinalized region | the requirement is unsatisfiable — 52b's repair, one state further |
| it names `Support { supporting, total, … }` | the partition is satisfiable and the quorum is short | `candidate:inactivity-leak` — a **decision** (#24/#39), not a fix |
| ⚠️ **`Support` was reached, and this row's reading of it is wrong** (corrected 2026-09-30, after the run) | the outcome that occurred is `0 of 250 (0 full partition(s) among 3 candidate(s))` — **no candidate was seen by the whole partition at all**, which is not "the partition is satisfiable". `NoAdvance::Support`'s own doc says it means the candidates *were* seen by the whole partition, so the variant is returned for two different situations and renders both as a shortfall. The dominant pre-kill value in the instrumented run is this one (`0 of 250` with no full partition is 39 of its 73 pre-kill lines); `150 of 250` is 10 of the 73 pre-kill lines | the consequence is **not** `candidate:inactivity-leak`, and that slug is not a row in `spec/findings.tsv` — it is named here and in `passes.md:4942` and defined nowhere. The consequence is C185: the gate's *seeing* relation, and the in-process fixture that separates its two mechanisms. A row that maps an outcome to a decision which does not exist is the class this file exists to avoid |
| it names `AlreadyPublished` | the fringe is at the tip and the height ran on | the production side, not finality |
| no line at all | finality never stalled inside the window | **void for this reading**, recorded |

## The single number each reading reports

- **0.1:** blocks per minute, and the last finalised height at the end of the window, beside the height at
  that moment. A rate is reported *with* the region it accumulates into, never alone.
- **0.2:** the **median scope width** — the smallest edge whose cumulative bucket count is at least half
  the merges — beside the widest scope and the largest `states_expanded` on any single merge. Reported with
  its envelope, as Stage 1's run was.
- **0.3:** the variant name, verbatim, quoted from the node's own log beside the tip it fired at.

## What this does not settle

- **It does not bound or price anything.** It produces the numbers those decisions are read from.
- **It is not C171's own falsifier.** That one is "block growth *per deploy* is bounded", and it needs the
  configuration C171 was reproduced in (`--no-autopropose --propose-on-deploy`, four deploys, an
  idle-timer-free chain) run **before and after** the pace term. This campaign's autopropose timer makes
  "per deploy" unattributable, which is why the reading above is blocks per *minute*.
- **It cannot see a scope a node never had to resolve.** A quiet chain understates what a flooded one
  produces; the flood is C175's territory.
- **It does not decide what a correct N is.** N must exceed the honest envelope; this campaign measures
  the envelope of a chain whose rate nothing bounds, so a *paced* re-read (Phase 1, same rig) is what N is
  finally read from. This run is the "before" that makes that comparison mean something.
- **It does not attribute the width to the load.** That was Stage 1's finding (43 chains → 899,236 states
  where 35 → 1,663,395); this run measures `states_expanded` beside width so the relation can be re-checked
  rather than assumed.
