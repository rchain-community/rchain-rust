# The proposer fix on the node: the pin is gone, and a second stop is now isolated

Two runs on tree `de4e9af02`, image `sha256:f948f92b15c6…` (`image_created=2026-09-30T21:41:37+01:00`,
`rust_diff_vs_HEAD=empty`). The protocol is `n127-proposer-fix-preregistration.md`, frozen before the first
of them; the rig, the perturbation and the kill offset are identical to the campaign's, so all three arms
are comparable.

```
python3 spec/audit/evidence/n127-liveness-summarise.py <rundir>
```

## 0.1 — the row the two earlier arms failed, and it passes decisively

Each arm's survivors, at the kill instant (T+120) and at the end of the window:

| arm | finality @T+120 | height @T+120 | **gap** | finality last advanced |
|---|---|---|---|---|
| before-arm (campaign, `c5442ee1f`) | 14, 17, 10 | 27, 32, 23 | 13–15 | T+111s, T+55s, T+17s |
| broken after-arm (`ef412ef84`) | 13, 4, 18 | 28, 65, 32 | **15–61** | T+33s, T+19s, T+128s |
| **fixed (`de4e9af02`)** | **72, 90, 105** | 76, 94, 109 | **3–4** | **T+120s — i.e. still advancing at the kill** |

| attempt | survivor | height T+120 → end | finality T+120 → end |
|---|---|---|---|
| 1 | bootstrap | 76 → 77 | **72 → 73** |
| 1 | v1 | 76 → 77 | **72 → 73** |
| 2 | bootstrap | 94 → 95 | **90 → 91** |
| 2 | v1 | 94 → 95 | **90 → 91** |
| 3 | bootstrap | 109 → 109 | **105 → 105** |
| 3 | v1 | 109 → 109 | **105 → 105** |

**Finality tracks the height at a gap of three to four, in all three attempts.** In the two earlier arms
the gap opened to 15 and then to 61 and the last advance was 87–104 seconds *before* the kill. Here the
last observed advance is at T+120 — the kill instant — because that is when the chain stops (§0.2); before
it, finality was moving continuously. The chain also runs at ~38 blocks/min against the campaign's 12–16,
which is what a chain that is no longer pinned looks like.

`n127-proposer-fix-preregistration.md`'s **0.1 row is met.**

**0.3, the stall line, has effectively disappeared**: six lines per attempt against seventy-five, and all
six are the boundary warming up at genesis (`0 of 250 (0 full among 0)` ×3 and `(0 full among 3)` ×3, one
pair per node). No stall line carries the support value the two earlier arms pinned on.

**0.4, the C184 control, passed**: `budget-*.txt` is empty in both runs.

## 0.2 — and the second stop, which this run isolates but does not explain

**The chain stops when a validator is killed, in every attempt, and it stops immediately** — height 76 → 77,
94 → 95, 109 → 109 across the 180 seconds after the kill, with both survivors alive and serving.

That is *not* "there is nothing left to finalise", which is the proposer's documented idle contract and
would be correct behaviour. The discriminator is a deploy **after** the kill, and it was run:

| run | perturbation | result |
|---|---|---|
| `de4e9af02-20260930T210137Z` | 4 deploys at T+201, after the T+120 kill | **4 of 4 submitted**; the height is 100 at T+120 and still 100 at T+300 |

**Work available, and nothing produced for 180 seconds.** So there is a second stop, it is downstream of
the kill rather than of the parent set, and the fix neither caused nor cured it.

**What is owed for it, and why it is not a diagnosis yet.** The proposer does not log the path that
suppresses it (`proposer.rs` logs only a self-created block that fails its own validation), so the artifact
that would explain the halt did not exist. Capturing node logs is now part of the harness
(`logs-<node>-a<N>.txt`, WARN and above, verbatim) — that is the instrument, and the missing log line is the
next unit's first change. **This file claims the halt, the discriminator, and nothing about the cause.**

## What this does not settle

- Three attempts of one configuration on one machine, plus one diagnostic. Not a proof.
- **A mixed network is untested.** The fix makes the port's proposer diverge from the oracle's (`§6`), and
  the classification — an unpatched node accepts a patched node's blocks — is argued from the validation
  rules, not measured. A patched node alongside an unpatched one is the measurement that would test it.
- Nothing here measures the merge shape (C182) or sets N.
