# Wargame run — the human roll-up (`dev` at `1419b127d`)

The record of the production-readiness wargame. The rendered section is
`docs/src/spec/adversary.md` → *The wargame — production readiness, from an apriori standpoint*. This file
is the run's own summary and the pointer to the machine artifacts beside it.

## What ran

140 agents, zero errors, in three phases — reachability (one agent per node), propose (one per node ×
reachable persona), referee (one per fresh finding, plus the seeded controls). The referee was given the
finding and repo read access and nothing else; it was not told the proposer's reasoning, the persona, the
other findings, or whether a register row existed. The join to the register is separate (below).

| | |
|---|---|
| Nodes | 14 (N1–N14) |
| Findings proposed | 136 |
| After cross-persona dedup by (node, guide word) | 97 |
| Vacuity | 47% (119 guide-word cells with a structural reason) |
| Referee verdicts | 85 `CONFIRMED`, 6 `REFUTED`, 6 `NOT-RE-ESTABLISHED` |
| Register-leaks in referee artifacts | 0 |
| **Calibration** | **6 of 6 seeded controls** |

## The instrument was falsified twice before it was trusted

This is the load-bearing part of the record, because the referee's independence is a prompt-level claim
and nothing *proves* it — only the seeded controls measure it.

- **Pilot 1** (one node): calibration **2/3**. The referee **read the register** — two findings were
  dispositioned by citing `spec/findings.tsv` / `AUDIT.md` rows, and were downgraded to
  `NOT-RE-ESTABLISHED` *because a register row existed*. That is the register acting as oracle, the exact
  failure the join stage exists to prevent. Also: dedup broke on the node's prose id, and vacuity was a
  suspicious 14%.
- **Pilot 2** (one node, resumed): calibration **2/3**, register-leaks **0**, dedup fixed, vacuity 27%.
  New miss: the referee **over-confirmed** the out-of-model control, citing an *adjacent* line (that a key
  *can* be loaded) to confirm a claim about *who can reach it*. The control was also mis-specified (key
  compromise is *in* the statement).
- **Pilot 3** (resumed): calibration **3/3**. The out-of-model control was replaced with a genuinely
  unreachable one (a compromised image registry).
- **Full run**: calibration **5/6** — the referee over-confirmed a **false** control (that the public HTTP
  server requires a bearer token), citing a bare router line. The CONFIRMED bar was tightened to require
  the *specific site where the claimed mechanism runs*, and the referees were re-run.
- **Full run, re-judged**: calibration **6/6**; the tighter bar moved 8 `CONFIRMED` rows to
  `REFUTED`/`NOT-RE-ESTABLISHED` (93→85 confirmed). This is the run rendered in the page.

**The lesson, stated because the next run will be tempted to skip it:** without the seeded controls the
verdict column is indistinguishable from self-review. Three of the five referee passes above would have
passed a "looks rigorous" bar while being wrong. The controls cost budget, produce no findings, and are the
first thing an orchestrator drops — they are the entire load-bearing member.

## Table F — the join (first pass)

Each `CONFIRMED` finding is matched against the register by its cited source path:

| | Count |
|---|---|
| `CONFIRMED` | 85 |
| **new** (no register row) | **≈36** |
| **known** (a register row covers it) | **≈49** |

This is a **first-pass** join by file path; it does not adjudicate *which* row. A later pass should read
each match. The near-half that are `known` is the expected shape for a surface the September audit and the
OCapN HAZOP already crossed — and it is why the page presents this section as a *test of the statement*,
not as a list of discoveries.

## What the statement test found (Table E of the page)

Two census rows in `docs/src/spec/adversary.md` are **too narrow** for the surface the node runs on:

- **operator error during recovery** — omits the durable on-disk reset record that overrides the
  operator's `--sync-anchor` at boot (W-N11-01).
- **key compromise** — the statement bounds it by the stake assumption and never mentions the *host* path:
  an unencrypted PEM accepted at the path, the hex on `argv`, the password in the environment, the key
  file's mode unchecked on read (W-N5, W-N6).

Both are rows where the **statement** should widen, not rows where the node is broken.

## What this run cannot claim

It is **`read`-class evidence**: no live multi-node attack was run, so every falsifier is `owed:` unless a
committed test arms it, and the **rate** that criterion 3 wants is the register's, not this run's. The
wargame supplies the **class vocabulary** and a **statement verdict**; it does not supply a rate.

## Artifacts

| file | what |
|---|---|
| `run.sh` | the workflow script (the run is reproducible from it) |
| `journal.jsonl` | every agent's return value |
| `results.json` | the full run result |
| `reachability.tsv` | Phase 1: who reaches each node, cited |
| `findings.tsv` | the 97 findings with their verdicts |
| `verdicts.tsv` | every verdict, including the six seeded controls |
| `../preregistration.md` | the node set, persona model, referee protocol and falsifiers, committed **before** the run |
