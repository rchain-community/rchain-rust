# Blocks per deploy as a function of the validator count — the reading

Protocol: `n149-preregistration.md`, frozen before the run. Artifacts:
**`spec/audit/evidence/n149-blocks/cf3945045-20261001T152307Z/`** — committed, because every number below
is otherwise uncheckable, which is C176's class and the defect this file's own instrument section
records. Per run: `series.tsv` (the 1 Hz height/finality samples, whose header carries the node's argv),
`blocks.tsv` (the block-hash union with each block's sender, deploy count and parent count),
`marks.tsv` (the window boundaries), the budget control and the log extractions. Tree `cf3945045`, image
`sha256:d7fd75fb7175…`, cap `4g`. 13 runs, **0 void attempts**, the C184 budget control empty in all of
them.

## The answer to #149, in one table

Primary arm, `--no-autopropose` — the guard-live configuration (see the pre-registration for why):

| N | blocks after one deploy, all three attempts | senders | parent count | time to finality |
|---|---|---|---|---|
| 2 | 8 · 8 · 8 | 2 | 2 | 5 s · 5 s · 48 s |
| 3 | 3 · 3 · 3 | 3 | 1 | never · never · never |
| 5 | 5 · 5 · 5 | 5 | 1 | never · never · never |
| 8 | 8 · 8 · 8 | 8 | 1 | never · never · never |

**Blocks per deploy is exactly the validator count** — one block per bonded validator, in every attempt
at N ≥ 3, with every validator among the senders. #149's inference is **confirmed and quantified**: the
growth is linear in the validator count. What it is not is a storm. The issue's title imagines every
validator proposing *repeatly* until the deploy finalises; what the node actually does is one round of N
attestations and then nothing, which is the round rule doing its job
(`proposer.rs:531-547`, the one-block-per-validator-per-round veto).

The structure says it plainly. At N = 8, the nine blocks in the DAG are genesis plus **eight blocks all
at height 1** — the deploy-bearing block from validator 0 and one empty attestation from each of the
other seven. There is no height 2.

## The finding that is larger than the cost: finality does not advance at N ≥ 3

| N | tip the chain reached | highest finalised |
|---|---|---|
| 2 | height 4 | **1** |
| 3, 5, 8 | height 1 | **none — nothing finalised at all** |

At N ≥ 3 no node finalises *anything* in the 180 s window: `/api/last-finalized-block` reports no block
for the whole run. At N = 2 the chain runs four rounds, reaches height 4 and finalises height 1.

So on this rig the deploy is not merely expensive — with the guard live it **is not finalised**, and the
chain stops producing. That is a liveness result, and it is the opposite shape from the one the issue
was written against. It also matches what the #148 session measured independently ("an idle chain does
not close its last blocks on its own"): a fringe does not advance over a static DAG, so finalising needs
*new* blocks, and after the first round nothing asks for any.

**This is not a storm and not a stall of the same kind as #148's.** #148 is unbounded production with
finality frozen; this is *bounded* production — exactly N blocks — with finality frozen. Both end with a
frozen finality; only one of them grows.

**Why N = 2 differs is not settled here.** It runs four rounds where N ≥ 3 runs one, and this rig does
not say which mechanism — the tap's trigger, the round veto's reset, or the finality advance itself —
produces the second, third and fourth round. It is the first thing a follow-up should measure, and it is
recorded as open rather than explained.

## The control arm reproduces #148's storm rate with every validator live

The control is the devnet default (`--autopropose`, which `devnet.sh:80` passes as `--autopropose`), N = 3,
one attempt:

| quantity | value |
|---|---|
| blocks in the 300 s window | **857** |
| blocks carrying a deploy | **856** (the 857th is genesis) |
| tip reached | height 286 |
| rate | **≈ 2.9 blocks/s** |

**#148's storm figure is 351 blocks in ~2 minutes ≈ 2.9 blocks/s — the same rate — and this run reached
it with all three validators live and no kill at all.** The storm is a property of the shipped default
configuration, not of an absent validator: with `--autopropose` and the dev-mode deployer key, every
propose attempt finds an empty pool, injects a dummy `Nil` deploy (`proposer.rs:711`), and produces a
block; `deploys` is therefore never empty, `new_state_transition` stays true, and the suppression clause
has nothing to suppress. That is Finding 1 of the pre-registration, now with a number attached.

Two cautions on reading it. It is **one attempt**, and its rig differs from #148's in stakes
(100/100/100 here, 100/100/50 there) and in the absence of a kill. So it does not by itself re-attribute
#148's measurement; it shows the rate is *reachable without the kill*, which is the part that matters for
attribution and which is why it is written here rather than left to inference.

It also does **not** reproduce the 2026-09-30 campaign's "12–16 blocks/min with all three validators
live" (`n127-campaign-results.md:165-169`) — but that row does not say what it appears to say, and the
first version of this paragraph read it as a refutation when it is mostly a **units artifact**.
Checked at the source rather than taken on report:

- The campaign's table column is **"height at T+119 s"** and its rows are 27, 32, 23; the rate is
  `height ÷ 119 × 60`, so the numbers printed as "blocks/min" are **heights/min**. Its own gloss, "one
  block per 4–5 s", inherits the same substitution.
- Heights are not blocks on that tree either — the same `max_block_number + 1 == b.block_number`
  derivation is in `c5442ee1f:validate.rs:221-236` — and this rig measures **2.99 blocks per height** at
  N=3. So the row is at least ~3× larger than printed: ≈ 40 blocks/min, not 12–16.
- Against this control's ≈ 173 blocks/min that is a factor of ~4, not ~10, and the remainder is **not
  attributable from here**. The likeliest cause is the *tree*, not the rig: the campaign's trees predate
  `AlreadyProposedThisRound`, which landed with #126 (`6c37a23f5`), so they are a different regime — one
  where a validator could propose repeatedly inside a round. Nobody has measured both regimes with one
  instrument; this control is the first block-level measurement of either.

So the frozen row is **not reproduced**, and the honest statement is that it is partly a units artifact
and partly an unmeasured regime difference — not a refutation. (The related figure on the scaling page,
`~38 blocks/min` cited to `n127-proposer-fix-results.md`, has since been found to appear *nowhere* in
that file, which reports heights and finality and no rate at all; that is the #148 session's finding and
their correction, noted here because it is the same measurement spelled the same wrong way.)

## The idle control, and the number the #148 session asked for

**0 blocks in the idle window, all 12 primary-arm runs.** With autopropose off and nothing to finalise,
an N-validator chain sits perfectly still. That is the control working — it is what makes "one deploy"
the only driver in the reading window — and it is the answer the #148 session wanted for their part 2.

The proof that the guard was reachable is in the artifacts rather than in the flags, and both are here.
The flags, verbatim from the run's own header (`n149-sample.py` writes the node's argv into every
`series.tsv`):

```
--propose-on-deploy … --dev-mode --deployer-private-key <hex> --epoch-length 10
```

with **no `--autopropose`**. And the stronger half: of the nine blocks at N = 8, **seven carry
`deployCount = 0`** and one carries `deployCount = 1` (the real deploy). An injected dummy deploy would
make a zero impossible, so the blocks are attestations and not dummy-deploy carriers. A reader of the
artifact alone can tell which configuration produced it.

## Measurements against the frozen acceptance rows

| row | outcome |
|---|---|
| senders reaches N and blocks per deploy grows with N, in all three attempts | **confirmed** — senders = N and blocks = N exactly, 3 of 3 attempts, at N ∈ {3, 5, 8} |
| the primary arm's idle window is non-zero → rig void | **did not fire** — 0 in every run |
| the control arm reproduces 12–16 blocks/min at N = 3 | **not reproduced**, and the comparison is not like-for-like: the campaign's row is heights/min, so it is ≈ 40 blocks/min against this rig's ≈ 173, and the residual ~4× is most likely the pre-#126 round rule. Partly a units artifact, partly an unmeasured regime difference — see the caution above |
| the control arm's idle window mints, and is not a void attempt | as expected — 165 blocks in its 60 s idle window |

## The instrument, and the defect the pre-flight caught

A short N = 2 pre-flight (20/20/40 s windows) ran first, because a sampler exercised only by the real run
has failures indistinguishable from a quiet chain. It paid for itself on the first execution: the sampler
asked `/api/blocks/2000`, the node's `max-blocks-limit` is **50** (`defaults.conf:143`, and `--dev-mode`
does not raise it), the 400 body parsed as an empty list, and a chain whose heights plainly advanced
1 → 2 → 4 → 5 was reported as **0 blocks, 0 senders, never finalised**. Fixed in `cf3945045`: the depth is
`min(50, height)`, and a failed read is now recorded in `blocks-read-errors.txt` and surfaced by the
summariser rather than counted as zero.

`maxpar` — the largest justification count among the post-deploy blocks — was added for the issue
author's #153 caveat, and it reads 1 at N ∈ {3, 5, 8} and 2 at N = 2. The round snapshot is one message
per sender, so the parent count tracks the *round*, not the validator count: an N = 8 block justifies
exactly one parent. The merge's width is not the justification count, and the column is what shows it.

## What this does not settle

- **Why N = 2 finalises and N ≥ 3 does not.** One rig, one configuration, 180 s. The mechanism is open.
- **#148's attribution.** The control shows the rate is reachable all-live; it does not re-measure #148's
  arm, whose stakes and kill differ.
- **The consensus half.** The issue's step 3 — attestations that are not blocks — is untouched and remains
  hard-fork-class (#51).
- **The 12–16 blocks/min comparison.** Not reproduced, not like-for-like: the campaign's row is a height
  rate, so it is ≈ 40 blocks/min against this rig's ≈ 173, and the residual is unattributed — most
  likely the pre-#126 round rule, which no one has measured against the current one with one instrument.
- Four validator counts, three attempts, one machine. This is the measurement the issue asked for before
  its claim was written down, not a proof of a bound.
