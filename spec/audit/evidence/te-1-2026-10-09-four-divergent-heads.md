# TE-1 witness — 2026-10-09: four divergent heads, and the chain cannot recover

Top event **TE-1** of [`docs/src/spec/testnet-acceptance.md`](../../src/spec/testnet-acceptance.md) —
*"The chain halts and cannot recover"* — observed on the public testnet. The specification already states the
verdict for this shape (§ line 1147: *"Recovery from the divergent finality itself: none. No mechanism drops
a divergent prefix or re-homes a fragment … The only 'recovery' documented is the operator restarting"*).
What follows is the first live artefact for that statement, rather than a new gap.

## The chain

Four bonded validators at 250 each, two per host, one binary (`dev` @ `efc1be75f`,
`sha256:675980ee95bb…`), genesis `713c0ebb0eb4ce866ae118aa1177a498b4edb2d431dd8b77d28abcdd4da9ea91`,
rebuilt 2026-10-06 and producing and finalising normally until the event.

## What happened — 2026-10-08T19:05:46Z

- Finality stopped at **101** and has not moved since (over 13 hours at the time of capture).
- **Every validator produced its own block from the same history, and each rejects the others'.**

```
A 19:05:46.336  proposed and added block #105 (seq 105)
A 19:05:46.901  state-hash disagreement on pre-state: block #104 75b98665bc52fa20…
A 19:05:46.975  Block #105 cf44cfd5… from 041ed2a2… (D) failed
A 19:05:47.029  state-hash disagreement on pre-state: block #104 3ad32f956b58b217…
A 19:05:47.185  Block #105 703255e2… from 04dce59b… (C) failed
A 19:05:47.287  state-hash disagreement on pre-state: block #104 4386635a89cfe73f…
A 19:05:47.414  Block #105 a11ba361… from 04d7707c… (B) failed
A 19:05:48.369  state-hash disagreement on rejected-deploys: block #106 58664b88…
A 19:05:48.369  ERROR Self-created block #106 (seq 106) failed validation: the block's rejected-deploy
                set does not match its parent
```

Three validators' #105 blocks rejected on a disagreement about the **pre-state of #104**, each citing a
*different* #104 hash — so the four nodes computed different states at #104, after the epoch boundary at 100.
A's own proposer could not build a child of its own state: the rejected-deploy set it derived disagreed with
the parent state it had just computed.

**The divergence is per node, not per host.** A and D share `164.90.140.144` and D rejected A's #106
(`e3e16461adb3d5b2a7c9b7f5c569cc30…`) on the same pre-state complaint about #105. Four validators, four
distinct #106 blocks. That pattern is what a nondeterministic choice inside the merge looks like from
outside; it is not claimed here as the mechanism.

## The restart experiments — measurement, not recovery

| node | restart | result |
|---|---|---|
| A | `systemctl restart rnode` | h=106 f=101 peers=3, and **silent** — the in-memory deploy pool is cleared, so the retry loop stops. One deploy → `proposed and added block #106` **accepted**, h 106 → 107 |
| D | `systemctl restart rnode-d` | same: its own #106 accepted, h 106 → 107 |

**2 of 2.** A restart restores *production* to a stuck node and adds another head; **finality stayed 101**.
The pre-restart failure was therefore transient *process* state, not a divergent state on disk — worth
knowing, and not a recovery.

## The dependency this exposes

A restoring or joining node syncs from the approved-genesis / **finalised** fringe. While finality is stuck
that path is closed — reads answer `"Finalized fringe is not available."` on a chain with no fringe, and a
restoring node has nothing to restore from. So **restarting recovers a *node*; nothing recovers the
*chain*.** The specification's line 816 (*"recovery is impossible without it"*) is the same dependency seen
from the other side.

## A write was lost — reported, not measured here

The reporter's vote-opening deploy was in **finalised** state at 101 and absent from 103. This witness does
not measure that (it is the reporter's observation, relayed on #280); it records it because it is the clause
that "recover the chain" does not cover, and the safety question that remains open: **can a later merge undo
an already-finalised write?**

## What this confirms, corrects, leaves open

- **Confirms (live)**: TE-1 for divergent finality, and the specification's verdict of *none* for recovery.
- **Corrects**: my own hypothesis, posted on #280, that the split followed the hosts. A and D share a host
  and disagree with each other; n=2 was coincidence.
- **Leaves open**: whether an already-finalised write can be undone by a later merge; and whether a diverged
  chain can be reconciled **without a genesis** — a candidate mechanism is proposed in the reconciliation
  issue (#283).
- **Falsifier**: a network in this state that converges to one head, with block hashes agreeing across
  nodes, **without a genesis**. Nothing in this capture suggests one exists today.

## Artefacts

- Capture directory and tarball (`sha256:652e21bdb8326bcbf5b4728a60da07b0c966875a76fe8500c7ae3462b5b22fa8`):
  all four nodes' logs as they were **before** any restart, each node's `/api/status` and
  `/api/last-finalized-block`, and the `ExecStart` of each unit.
- Thread: [#280](https://github.com/rchain-community/rchain-rust/issues/280), including the configuration
  asked for by the reviewers and both restart experiments.
