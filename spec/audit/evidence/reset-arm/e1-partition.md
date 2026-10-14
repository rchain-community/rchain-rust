# E1 — a network partition stages nothing (measured 2026-10-13)

**What this is.** Phase 1 of the R3 plan's first experiment: a design pass argued that a network partition
cannot stage a `FailureCause::Divergence`, because the merge is now a pure function of the parent set
(`casper/tests/merge_determinism.rs`) — *a different view is not a different computation*. This is the
measurement that confirms it, so the refutation rests on a run rather than on a reading.

**The run.** One host, one run. `tools/devnet.sh down -v`, then `tools/devnet.sh up --validators 4 --fresh`
(the bootstrap plus three = four bonded). Then:

```
docker network disconnect devnet devnet-validator-1     # v1 off-net, still running
… hold 60 s …
docker network connect    devnet devnet-validator-1     # reconnect
… wait 75 s …
```

**What happened — the two heads formed, and then healed themselves.**

| moment | bootstrap | v1 (isolated) | v2 | v3 |
|---|---|---|---|---|
| baseline | 69 | 83 | 85 | 83 |
| +60 s isolated | 84 | **95 → 96 → 98** (still producing) | 144 | 142 |
| +75 s after reconnect | 116 | 148 | 192 | 190 |

The isolated node **kept producing** (`autoproposeTimerHalted` false, `consecutiveSelfValidationFailures` 0)
while the survivors ran ahead — the two-heads shape the plan predicted. On reconnect it **simply caught up**
(98 → 148), which is what the merge's determinism buys.

**The falsifier, run as designed** (the plan's Phase 1, E1):

```
$ docker logs devnet-validator-1 2>&1 | grep -cE \
  'state-hash disagreement|failed validation: (Invalid(State|PreState)Hash|InvalidRejectedDeploy|InvalidBondsCache|UnjustifiedSlash)'
0
$ docker logs devnet-validator-1 2>&1 | grep -cE \
  'cannot restore block|reset requested|rewound [0-9]+ block|cleared the failure record'
0
```

**Expect 0; got 0.** No `Divergence` status, no record, no restore, no reset — and `unrestorableRecords`
stayed **0** on every node throughout (one reading per node at the end of the window).

**The conclusion, and it is the plan's.** The partition is **not** an instrument: a node with a *different
block set* computes the *same* merge for a delivered block, so nothing it validates disagrees. That is
`merge_determinism.rs`'s property measured on a live net, and it agrees with `n223-rejoin-results.md` R3
(*"nothing is refused"*) — the repo ran this drill before and got the same non-result. Recorded so the route
is not proposed again: the two refutations now each have a measurement behind them (this one; and `#345`'s
read of the revalidation for a *record* edit).

**What it does not touch.** The route the plan's E2 probes is different in kind: an edit of the *state* the
node computes from, not of the record it writes. This run says nothing about that.
