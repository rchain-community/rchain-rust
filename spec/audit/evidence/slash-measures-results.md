# The slash measures: A2's equivocation and A1's attributability, over the real DAG

**What this is.** The risk plan (`~/.claude/plans/lexical-gliding-hopcroft.md`) asked for a measurement
for A1 and A2 that its own standard requires of a consensus change — "the whole suite passes before and
after" is not a measurement. A1 (C198) and A2 (C200) both landed with unit falsifiers only. This is the
measurement.

**The run.** `cargo test -p rchain-casper --test slash_measures` — the suite is
`casper/tests/slash_measures.rs`, so the run is in the tree rather than described here.

```
running 2 tests
test a_local_knob_changes_the_refusal_and_never_the_offence_across_a_store ... ok
test an_equivocation_is_proved_on_a_node_that_never_saw_the_offending_block ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
```

**What it drives.** Both arms use the *real* `BlockDagKeyValueStorage` — the same four stores
`casper/src/dag.rs`'s own tests build — so everything goes through the production `insert`, the
production H-1 gate and the production proof check.

## A2 — `an_equivocation_is_proved_on_a_node_that_never_saw_the_offending_block`

Two stores, standing for two nodes. A validator with a **real secp256k1 key** signs one block at
`seq_num 0`; both nodes hold it. It then signs a second, differently-hashed block at the same
`seq_num 0` — and **only the first node is shown it**.

| step | what it establishes |
|---|---|
| `n1.insert(second)` | refused, and refused *as an equivocation* (`EQUIVOCATION_PREFIX`) |
| `n1.recorded_equivocations()` | exactly one entry, keyed by the offender — the gate kept the header of the only copy of the offence that will ever exist |
| `equivocation_is_proved(&n2, offender, header)` | **true** — `n2` never held the offending block and still proves the offence, from the offender's own signature plus the first block in its own DAG |

Three controls, each a way the proof could be too permissive, all of them false: the *first* block's own
header (the DAG holds it, so it conflicts with nothing); evidence signed by a different key; and a fresh
node whose DAG holds no first block at all.

**Red under two mutations, each aimed at one half of the mechanism:**

| mutation | result |
|---|---|
| `recorded_equivocations` returns `Vec::new()` (the gate keeps nothing) | FAILED — `left: 0, right: 1`, the offence was never recorded |
| the DAG conflict check removed from `equivocation_is_proved` (`return Ok(true)` after the signature) | FAILED — the "lonely node" control, which is the arm that makes the check a *proof* rather than a signature test |

**What it does not cover, stated rather than implied: the wire.** The two nodes are two `Arc`s in one
process, so a block here reaches the second node by a direct call. Propagation is generic block gossip
and is exercised by `casper/tests/multinode.rs` and the devnet scripts; what is *not* exercised is the
particular path of a slashing block that carries evidence. A live two-node rig would close that, and it
remains the honest residue of this measurement.

## A1 — `a_local_knob_changes_the_refusal_and_never_the_offence_across_a_store`

One block, one deploy priced at `phlo_price = 3`. Under a strict node's floor of 5 the pure checks
return `ContainsLowCostDeploy`; under a permissive node's floor of 3 they return `Valid`. The two
verdicts differ, which is the control that there is anything here to measure.

What is measured is the chain after that: the strict node's record for the block is marked failed with
`slashable` taken from `is_slashing_offence`, **stored in the real metadata store and read back**, and
then handed to the rule the proposer uses (`slashable_senders`). The flag survives the round trip as
false and the rule finds **nobody to slash** — so the strict node proposes no `Slash`, and the permissive
node never has to decide whether to refuse one. That is the mechanism by which C198 removed the split:
the two nodes still disagree about the *block*, and can no longer disagree about each other's
*slashes*.

**What it does not cover, and it matters for how this is read.** It does not make two differently
configured nodes agree about validity, and it does not claim to: a node whose `min-phlo-price` is below
the network's will still refuse blocks the network accepts, and will still lag. What C198 removed is the
**permanent, bond-destroying** half — a slash one node could not re-derive from its own config. A
two-node devnet arm with `--min-phlo-price` differing would show the lag directly and would show that no
stake moves; that arm is still owed, and the plan's text ("the chain must not split") should be read as
the narrower claim above until it is run.

## Where this is registered

Pass §55's B4 section and §54's A2 section, and the §6 rows for the graded slash (C199) and the
equivocation (C200). The plan's "OWED" note for A1 and A2 is closed by this file, with the wire arm and
the lag measurement named as the residue.
