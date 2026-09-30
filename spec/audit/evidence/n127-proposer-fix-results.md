# The proposer fix on the node: the pin is gone, and the chain keeps moving

Run 2026-10-01 on tree `e7967ed35`, image `sha256:722ee686fecd…` (`image_created=2026-10-01T…`,
`rust_diff_vs_HEAD=empty`). The frozen protocol is `n127-proposer-fix-preregistration.md`, and the rig,
the perturbation and the kill offset are the campaign's own, so every arm is comparable.

```
python3 spec/audit/evidence/n127-liveness-summarise.py <rundir>
```

## The fix

A new block justifies **the round snapshot** — the `latest_msgs` as of the last round boundary — rather than
every sender's newest message. `calculate_next_fringe_support_map` derives each candidate's `seen_by` from
`parents ∖ next_layer`, so a parent set that *is* every sender's newest message has no such remainder and no
candidate is ever a full partition: the numerator is zero **before the quorum is consulted**. The chain the
node's own proposer built could not be finalised by the gate it validated against, which
`casper/tests/finalization.rs`'s module header had said since it was written.

Three designs were needed, and the two that failed are recorded because each failure is a measurement:

| design | pin fixed | equivocates against itself | keeps producing after a kill |
|---|---|---|---|
| `latest_msgs` (the state before) | no | — | no |
| pure round snapshot | **yes** | yes, and three of them halt autopropose | no |
| + refuse a second proposal in a round | yes | no | **no — deadlocks** |
| + escape, bounded by a local clock | **yes** | **no** | **yes** |

`validate.rs:236` requires `max(justifications) + 1 == block_number` and `:259` requires
`creator_latest_seq + 1 == seq_num`, so both numbers are **determined by the justification set and checked by
every validator** — which is why the tension is structural and why the numbers cannot simply be derived from
`latest_msgs` instead. The rule is one block per validator per round; the escape exists because refusing for
ever deadlocks (the round's clock is measured from a tip the refusal freezes), and it is taken only after
`LIVENESS_WINDOW` attempts, so a healthy round never sees it.

## The three rows that matter, all three attempts

| attempt | survivor | height T+120 → end | finality T+120 → end | **gap** |
|---|---|---|---|---|
| 1 | bootstrap | 84 → **96** | 80 → **81** | **4** |
| 1 | v1 | 84 → **96** | 80 → **81** | **4** |
| 2 | bootstrap | 115 → **128** | 111 → **113** | **4** |
| 2 | v1 | 116 → **128** | 112 → **113** | **4** |
| 3 | bootstrap | 131 → **143** | 127 → **128** | **4** |
| 3 | v1 | 131 → **143** | 127 → **128** | **4** |

- **The pin is gone**: finality sits 4 heights behind the tip at the kill, against 15–61 behind in the two
  earlier arms, whose last advance was 87–104 seconds *before* the kill.
- **Production continues after the kill**, in every attempt: +12 heights over the 180 seconds after it.
  The deadlock is gone, and so is the equivocation that halted autopropose.
- **Zero equivocation lines on any survivor in any attempt.** 12 / 32 / 12 stall lines against 75.
- The C184 budget control is empty: `SearchBudgetExceeded` never fired.

## One caveat, disclosed rather than smoothed

**The killed node logged ten `equivocation detected` lines, pre-kill, in one of the three attempts** —
block 27, seq 24, clustered inside three seconds, after which it recovered on its own and went on to
height 115. Traced to the guard's predicate: it compared the sender's latest *height* against
`round_height`, and a boundary set at a tip above the sender's own last block leaves that comparison false
while the snapshot's entry for it is older still. `has_advanced_past_the_round` now compares the sender's
sequence number against its snapshot entry, which is exact.

**That one-line change is covered by the fixtures and not by this run** — the binary measured above is the
one with the height comparison. Said plainly because it is the difference between "verified" and "verified
except for one predicate".

## What this does not settle

- Three attempts of one configuration on one machine. Not a proof.
- **A mixed network is untested.** §6 registers this as a proposer divergence from the oracle — the Scala's
  `BlockCreator` justifies `latestMessages` — and the classification (an unpatched node accepts a patched
  node's blocks) is argued from the validation rules, not measured.
- Nothing here measures the merge shape (C182) or sets N.
