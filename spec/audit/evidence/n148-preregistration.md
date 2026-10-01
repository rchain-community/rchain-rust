# Does one deploy re-arm unbounded production when a validator is absent? — pre-registered

**Status: FROZEN before the run.**

> **Correction, 2026-10-01 after the run: arm B as written was VOID, and the run says so.** The arm was
> specified as "`KILL_AT` beyond the window", but the harness waits for `KILL_AT` *before* the after-kill
> deploy, so pushing the kill past the window pushes the **deploy** past it too: the first arm B attempt
> deployed at T+421, after sampling ended at 420 s, and read nothing. Arm B was re-run with a `NO_KILL=1`
> control (skip the kill, keep the timeline) — a knob this freeze should have named. The rows are left
> frozen as written; this block is the correction, not a rewrite. Arm A ran before the change.
>
> **And one claim in "Why" is now known to be wrong on this rig.** The n127-liveness correction ("the kill
> is not the variable") does not hold here: with no pre-kill deploy, finality is healthy until the kill and
> freezes exactly at it (`n148-results.md`, arm A). #148 is reproduced, and the deploy in its narrative is
> incidental to the freeze.

## Why this is the measurement

[#148](https://github.com/rchain-community/rchain-rust/issues/148) reports that with a validator
absent, block production runs unbounded while finality freezes: bonds A 100 / B 100 / C 50, C killed,
**351 blocks in about two minutes** after a single deploy, finality frozen at 14. Three things make it
worth re-running rather than accepting:

1. **Its measurement predates the fix it is about.** The probe is dated `f36312a55` (2026-09-29). The
   `#126` round-snapshot fix — which changed what a block justifies so the fringe gate can find a full
   partition and finality can advance again — landed on `dev` on 2026-09-30/10-01. #148 says so itself:
   "nothing on the tracker re-runs this probe on a build that carries those changes."
2. **The arm that tests it has never been run.** `n127-liveness-run.sh` carries a `DEPLOY_AGAIN_AT`
   knob whose own comment calls it "the discriminator" for exactly this — deploy *after* the kill. It
   defaults off, and neither `n127-liveness-results.md` nor its pre-registration sets it.
3. **The n127-liveness correction says the kill may not be the variable.** That arm found finality
   pinning **with every validator live**, before the kill (its own correction block). If that still
   holds on this tree, a storm re-armed by a deploy is a per-deploy phenomenon wearing the kill's
   clothes — so a second arm, with no kill, is carried to attribute it.

## The reproduction (the campaign's rig, unchanged except where noted)

Three validators, `--stakes 100,100,50 --epoch-length 10 --fresh`, devnet defaults (autopropose on,
propose-on-deploy on, attest-on-new-blocks on), an 8 GiB cgroup with swap off, `WINDOW_S=420` so the
post-deploy window is 240 s (wider than the ~120 s in which #148 saw 351 blocks), no deploy before the
kill, and **one** deploy at T+180 s. `tools/devnet.sh stop 2` (the 50-stake validator) at T+120 s.
≥ 3 attempts, unfiltered, one devnet at a time. Artifacts under `target/n127-liveness/<tree>-<utc>/`,
every file carrying the tree, the image, and the deploy parameters (`manifest.txt`).

**The image is built from the tree under test.** `tools/devnet.sh build` builds `rnode:local` from the
current directory, and the harness runs with `REPO` pointed at the `dev` worktree, so the binary under
test is `dev`'s. `.dockerignore` excludes `spec/`, so the harness and pre-registration are read from the
checkout, not the image.

## The arms

| arm | kill at T+120 | deploy at T+180 | purpose |
|---|---|---|---|
| **A** | yes (`stop 2`) | one | #148's shape: does one deploy re-arm unbounded production? |
| **B** | **no** (`KILL_AT` beyond the window) | one | **the attribution control** — run only if A reproduces: is the absence the cause, or does any deploy do it? |

Arm B is conditional and its condition is stated here so it is not chosen after seeing the result.

## The acceptance row, frozen

| observation | verdict |
|---|---|
| height runs away after the single deploy — far above any function of the 2 surviving validators — while finality does not advance past its pre-deploy value | **#148 reproduced on `dev`**: part 2 (a bound on post-deploy production, held by a test) is owed |
| height grows, then stops within a function of the validator count, and finality resumes | **not reproduced**: the `#126` round-snapshot fix subsumed it, and #148 reduces to its regression guard |
| no production after the deploy at all | the chain is stopped for a reason other than the one under test; reported as that, not as "fixed" |
| finality is already frozen before T+180, with no deploy | a different failure (the n127-liveness pin); reported as a finding about the pin, not about #148 |

## The numbers to report

Per node per attempt, mechanically: `height` and `finalized` at T+120 (kill), T+180 (the deploy), and
the end of the window; the height gained after T+180 and its rate; and the instant finality last moved.
Read from the sampler's own series by `n148-summarise.py`, which refuses to interpolate a missing sample.

## What this does not settle

- **The all-live minting per deploy** is #149, a separate issue; this does not measure blocks-per-deploy
  as a function of the validator count.
- **The merge budget** (C184) is read as a *control*, not a reading: this run's scopes are the honest
  ones, so `SearchBudgetExceeded` must be absent. If it fires, the bound is set too low.
- It is not a claim about an attacker, and it does not bound the exponential.
- Three attempts of one configuration on one machine: a result here is the measurement the register asks
  for, not a proof.
