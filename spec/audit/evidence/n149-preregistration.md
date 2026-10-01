# Blocks per deploy as a function of the validator count — pre-registered

**Status: FROZEN before the run.** Issue [#149](https://github.com/rchain-community/rchain-rust/issues/149).

## The claim under test

#149 infers, without measuring, that one deploy obliges **every** validator to mint blocks until that
deploy is finalised, so the chain's growth per deploy is a function of the validator count rather than
of the work submitted. Three code facts behind it, re-read on `dev` (`8118001fe`) before freezing:

- the attestation guard is `attestation_suppressed` (`casper/src/blocks/proposer/proposer.rs:1132`),
  and its first clause returns `true` on `nothing_to_finalize` alone (`:1138`);
- an attestation *is* a block (`casper/src/blocks/proposer/block_creator.rs:146-149`, the
  `!suppress_attestation` branch builds an empty state transition);
- the tap is on by default and reacts to any remote block, attestations included
  (`node/src/runtime/node_runtime.rs:1553-1560`).

The number has never been taken. Taking it is this unit.

## The finding that shapes the rig: the guard is dead in the devnet's default configuration

`has_deploys` is not the only thing that can make `should_propose` true. The dev-mode dummy deploy is
injected whenever the pool is empty (`proposer.rs:711-743`), and it is pushed into the very `deploys`
vector that `should_propose` reads (`block_creator.rs:87`). `dummy_deploy_key` requires
`autopropose` **and** a deployer key (`node_runtime.rs:2575`), and `tools/devnet.sh` sets both by
default. So on a default devnet:

| configuration | `deploys` at `create` | branch taken | is `attestation_suppressed` consulted? |
|---|---|---|---|
| `--autopropose` (the default) | non-empty — the dummy | the full-transition branch | **no** |
| `--no-autopropose` | empty unless a real deploy is pooled | `!suppress_attestation` → empty attestation, else `NoNewDeploys` | **yes** |

The guard and the empty-attestation branch it gates are therefore **only live with autopropose off**.
A measurement of "#149's mechanism" taken on the default devnet measures the dummy-deploy path instead.
The primary arm is `--no-autopropose`; the default configuration is carried as a **control arm**, and
the contrast between the two is itself a result of this unit.

This also retires a number already in the tree: the 2026-09-30 campaign's "12–16 blocks/min with all
three validators live" was taken with autopropose on (`n127-campaign-results.md:165-169`), so it is a
rate for the dummy-deploy path, not for attestation. The idle arm below re-measures it.

## The rig

`tools/devnet.sh up --validators N --fresh --epoch-length 10 [--no-autopropose]`, all N validators
live, one devnet at a time, `DEVNET_NODE_MEMORY` stated per run. N ∈ {2, 3, 5, 8} — the top of the
range exists because 8 is the largest set `devnet.sh` can bond, its key table having been extended from
3 to 8 for this unit.

Per N, one devnet start carries **three windows**, sampled at 1 Hz by `n149-sample.py`:

1. **settle** (60 s): joining, genesis sync and the first attestation rounds. Not read.
2. **idle** (60 s): *no deploy.* The baseline. The node's documented idle contract is that an idle
   chain does not grow (`proposer.rs:1115`), so this is expected to be **0 new blocks** on the primary
   arm — and it is the control that isolates the deploy as the only driver.
3. **reading** (180 s): **exactly one deploy**, submitted at the first second of the window.

One deploy only, from the bootstrap (validator 0) — `tools/devnet.sh deploy` targets it — so the
deploy-bearing block is always block-0's successor by the same sender. Stated because it is an
asymmetry the rig does not remove.

**Attempts: 3 per (N, arm)**, unfiltered, one devnet at a time. Artifacts under
`target/n149-blocks/<tree>-<utc>/`, every file carrying the tree, the image id and the flags.

### Arms

| arm | flags | what it is for |
|---|---|---|
| **primary** | `--no-autopropose` | the guard is live; this is #149's mechanism |
| **control** | defaults (`--autopropose`) | the same rig as the campaign's 12–16/min, so the two numbers are comparable; expected to show the dummy-deploy path |

The control arm is run at **N = 3 only, 1 attempt** — it is a bridge to the existing number, not a
second sweep.

## The reading

Per attempt, computed by `n149-summarise.py` from the sampler's TSVs and from the union of block
hashes the sampler saw:

| quantity | definition |
|---|---|
| `blocks_per_deploy` | distinct block hashes first seen in the reading window, i.e. **not** present at the deploy second — the union over all N nodes, so a block on a fork branch counts once |
| `senders` | distinct `sender` values among those blocks — the count #149 predicts grows to N |
| `time_to_finality` | seconds from the deploy to the first sample where, on **every** node, the finalised block number is ≥ the deploy-bearing block's number; `never` if it does not happen inside the window |
| `idle_blocks` | distinct block hashes in the idle window — the control, expected 0 |

`blocks_per_deploy` counts blocks, not heights: two validators can mint at the same height, and a
height delta would report them as one.

## The acceptance rows, frozen

| observation | verdict |
|---|---|
| the primary arm's `senders` reaches N and `blocks_per_deploy` grows with N across all three attempts | **#149's inference is confirmed and quantified** — the stated function is the measured curve, and step 2 of the issue is the next unit |
| the primary arm's `senders` is N but `blocks_per_deploy` does not grow with N | **#149 is half right**: every validator does speak, but the count is bounded by a constant the issue does not name |
| the primary arm's `idle_blocks` is non-zero | **the rig is void** — the deploy is not the only driver and nothing below can be read |
| the control arm reproduces 12–16 blocks/min at N = 3 | the campaign's number is the dummy-deploy path, as this unit infers |

**The single number to report:** `blocks_per_deploy` at N = 2, 3, 5, 8 on the primary arm, with its
`time_to_finality` beside it — because a count that grows while finality keeps pace is a bandwidth
cost, and a count that grows while finality stalls is #148's class.

## Void conditions

- Fewer than N containers up, or any node not reporting `latestBlockNumber > 0` before the settle
  window ends, or nodes not agreeing on the genesis hash: **void**, the network was not formed.
- `SearchBudgetExceeded` (C184) appearing in the logs: **void** — these scopes are honest, so the
  budget must refuse nothing.
- Any `WARN`/`ERROR` about validation failures in the settle window: recorded, and the attempt is read
  only if the settle window ends quiescent.

## What this does not settle

- It does not change the attestation guard. Step 2 of the issue is a separate change, and this unit
  only produces the before-number it must move.
- It does not test the consensus/wire options (issue step 3). Those are a hard-fork-class change and
  belong on #51 and in the §6 register.
- It does not measure #148's absent-validator storm, which is a different rig (one validator stopped).
- Four validator counts, three attempts, one machine: this is the measurement the issue asks for before
  its claim is written down, not a proof of a bound.
