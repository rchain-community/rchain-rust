# Production-readiness wargame — pre-registration

**Reviewed revision.** `dev` at `1419b127d` (the merge of #347). Every `file:line` below is
**re-derived at run, never remapped** — the repository's rule (`failure-hazop.md` §7, hypothesis R4).

**What this is.** A HAZOP-led tabletop wargame over the node's **production surface**, run as a real
multi-agent exercise: personas act deviations against study nodes, and every candidate finding is handed to
a **separate referee** that confirms or refutes it from source. The register is **bracketed** — the
proposers work from the code alone, and the join to `spec/findings.tsv` happens in a later, separate stage.

**Falsifiers are stated before the run**, so the run cannot be read as a success by default. This file is
committed *before* any agent runs.

---

## 1. Study nodes (function-grain; fixed before the sweep)

Each node is **one function**, with the `adversary.md` census row it tests — the tie that makes the exercise
a test of the bounded-adversary statement rather than a worksheet beside it.

| id | node | design intent | census row it tests |
|---|---|---|---|
| **N1** | peer transport + discovery | a peer's message is authenticated and bounded in size and rate | equivocation, withholding |
| **N2** | deploy ingress | a deploy is admitted once; admitting is bounded per unit time *per reachable identity* | deploy flooding |
| **N3** | public read surface | the reads disclose only what a public reader is meant to see | — (new surface) |
| **N4** | admin actuator | only the trusted operator can make the node act with its own authority | operator error |
| **N5** | validator key lifecycle | the signing key exists only where the operator intends and can be replaced | key compromise |
| **N6** | host and container | a compromise of the node process is contained by the host | — (new surface) |
| **N7** | build and release provenance | the artefact an operator runs is what the source builds, verifiably | — (new surface) |
| **N8** | OCapN transports | the capability wire lets a stranger do only what a stranger is meant to do | — (new surface) |
| **N9** | validator powers | **the statement under test**: a bounded validator can neither split safety nor stall liveness | *all census rows* |
| **N10** | observability and alerting | an operator can see production, finality and halt state | — (new surface) |
| **N11** | upgrade, migration and DR | the operator can advance and restore the node without a wipe | silence of a quorum; operator error |
| **N12** | DoS as a dimension | no reachable action costs the node superlinearly in attacker input | deploy flooding |
| **N13** | entropy and randomness sourcing | the names a deploy can forge are unpredictable | (adjacent to key compromise) |
| **N14** | clock and time | the node's notion of time is bounded and observable | a validator rejecting its own block |

**Reframes locked in** (where a ready-made checklist would have been theater): N2 does **not** re-file the
plaintext/0.0.0.0 fact; N3's property is **disclosure**; N6 asks *what a missing control lets an attacker
do*; N7 does not re-litigate the signing/SBOM decision; N8 does not re-file the OCapN injection/SSRF
(findings recorded as `known`, not new); N9 tests the statement rather than hunting defects.

## 2. Personas — capabilities, not voices

A persona is an **axis of the table**, not a prompt. Each carries a capability model; a persona that
**cannot cite a reachability fact for a deviation returns `UNSTATABLE`, not a row**. The cited facts are
derived in Phase 1 (one agent per node), not asserted here.

| persona | capability model |
|---|---|
| **Alice** | honest validator — the non-adversary control |
| **Bob** | honest client / deployer / public reader — the control |
| **Eve** | passive network adversary — reads plaintext (40401 deploy, 40403 HTTP), observes timing; cannot modify |
| **Mallory** | active network adversary — injects / modifies / replays on unauthenticated surfaces |
| **Oscar** | Byzantine participant — the statement's ≤1 validator, probing the **whole** stack |
| **Root** | host / supply-chain intruder — reads the data dir and key files, swaps the image or the artefact |
| **Operator** | the trusted operator — **a fault source, not an adversary** (R1–R4 ladder, `In-model? = outside`) |
| *substrate* | **no persona** — entropy/RNG sourcing; the model breaks here, and saying so is a finding about the method |

**Persona × node (seed; Phase 1 refines with citations):**

| persona | reaches |
|---|---|
| Alice | N1, N9 |
| Bob | N2, N3 |
| Eve | N1, N2, N3 |
| Mallory | N1, N2, N4, N8, N12 |
| Oscar | N1, N2, N9, N14 |
| Root | N5, N6, N7, N11 |
| Operator | N4, N10, N11 |

## 3. The referee

Four verdicts; only two are the referee's.

| verdict | who | requires | lands |
|---|---|---|---|
| `UNSTATABLE` | proposer | no bounding property / no reachability fact / out of model | page + structural reason |
| `CONFIRMED` | referee | re-derived the mechanism **and** named the falsifier that would fire | page → join |
| `NOT-RE-ESTABLISHED` | referee | mechanism plausible, not re-provable by reading (the dynamic class) | page as a named residual **+ the measurement it would need** |
| `REFUTED` | referee | a **positive artifact** (named line / test / measurement) | page only |

- **`CONFIRMED` is not "I agree"** — it is "I re-derived it, here is the artifact". A CONFIRMED whose
  falsifier does not yet exist is downgraded to `owed:` and enters the register `in progress`, never `done`.
- **A `REFUTED` with no positive artifact is forced to `NOT-RE-ESTABLISHED`.** An empty-handed refutation is
  a self-reported clean (`security-audit.md` §1).
- **What the referee is NOT told** (independence is prompt construction): the proposer's reasoning; the
  persona's identity; the proposer's confidence/severity; the other findings; their count; the node's design
  intent; whether the finding is already a register row. Its input is the **bare claim**
  `{node, guide word, deviation, class, falsifier}` plus repo read access.

## 4. Seeded controls — the anti-theater device

The least-enforceable claim is that the referee never saw the proposer's reasoning. The only thing that
**measures** it is planting findings with known ground truth, mixed into the referee's queue unlabelled:

| kind | planted claim | required verdict |
|---|---|---|
| `known-true` | a register mechanism, re-worded | `CONFIRMED` |
| `false` | a deviation citing a line that does not exist / a visibly-bounded property | `REFUTED` |
| `out-of-model` | an attack by an actor outside `adversary.md` | `NOT-RE-ESTABLISHED` or `UNSTATABLE` (never `CONFIRMED`) |

**Calibration = controls matching their required verdict / controls planted.** It is reported on the page.
A referee that CONFIRMS the `false` control or REFUTES the `known-true` one is uncalibrated; the run is
redone. **Report the calibration number, or do not report a verdict column.**

## 5. Falsifiers — the run is void unless

1. **Calibration holds** on the pilot (Phase 0): every seeded control returns its required verdict. If not,
   the referee is fixed before the red cell is scaled.
2. **Every `CONFIRMED` carries a re-derived `file:line` artifact** (checked by the leader).
3. **Every `REFUTED` carries a positive artifact**; a `REFUTED` without one is downgraded to
   `NOT-RE-ESTABLISHED`.
4. **The join is separate**: no proposer cites a register row, and the registrar labels each `CONFIRMED`
   `new` or `known (C<n>)`.
5. **Vacuity is reported** — the guide-word sweep's vacuity rate is a calibration number, not a hidden
   silent drop (T2).

## 6. Evidence shape (this directory)

```
spec/audit/evidence/wargame/
  preregistration.md   # this file
  pilot/               # Phase 0: the one-node calibration run
  run-<rev>-<ts>/      # the full run
    run.sh             # the workflow script, copied in
    journal.jsonl      # every agent's return value
    reachability.tsv   # Phase 1
    findings.tsv       # Phase 2-4 rows, pre-register
    verdicts.tsv       # Phase 4-5 incl. the seeded controls and the calibration
    results.md         # the human roll-up
    provenance         # HEAD, model, budget, agents-per-phase, truncations
```

Timestamps are stamped **after** the workflow returns (scripts cannot call `Date.now()`).

## 7. What this cannot claim

A wargame is **`read`-class evidence** and **cannot supply a rate** (`security-audit.md` §1 states this of
itself). It supplies a **class vocabulary** and a **statement verdict**. Every row is labelled `read`; every
falsifier `owed` unless a committed run arms it; the rate stays the register's. If a reader concludes
"every vector is handled because the referee confirmed the statement's rows", the page lies.
