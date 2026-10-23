# Rholo — Engineering, Governance & Contribution Status

**Reporting window:** 18–23 October 2026  
**Repository:** [rhovision-org/rholo](https://github.com/rhovision-org/rholo)  
**Reference branch:** `dev`  
**Observed branch head:** `4d3c7b716ca370bf8e6751e4479829c42fecaa31` (23 October, 16:16 UTC)  
**Method:** GitHub PR/issue metadata, review history, PR descriptions, merge states, and Actions results. Historical acceptance is attributed to its exact source, not automatically to current `dev`.

## Executive summary

The project made substantial, independently reviewable progress in consensus liveness, finalized-state recovery, genesis protocol infrastructure, exact decimal semantics, dependency security, and release tracking. The key achievement is the integration of the canonical **N=8 validator partition/rejoin correction** through [#408](https://github.com/rhovision-org/rholo/pull/408), supported by multiple real-node campaigns and an independent maintainer retest. Follow-on restore hardening [#410](https://github.com/rhovision-org/rholo/pull/410) and [#411](https://github.com/rhovision-org/rholo/pull/411) is also merged.

This is **not** evidence of mainnet readiness. The repository describes itself as pre-testnet, [#51](https://github.com/rhovision-org/rholo/issues/51) remains open, historical REV reconciliation is still a decision/evidence gate, and next-genesis rollout and compatibility require explicit authorization and acceptance.

## 1. Snapshot of recent activity

The **39 pull requests updated** during the reporting window are distributed as follows (counts by PR author, **not** a measure of code ownership or total project value):

| PR author | Updated PRs | Merged PRs | Closed without merge | Still open |
|---|---:|---:|---:|---:|
| [aixaria0](https://github.com/aixaria0) | 26 | **15** | 11 | 0 |
| [PatrickMockridge](https://github.com/PatrickMockridge) | 8 | **6** | 1 | 1 |
| [jimscarver](https://github.com/jimscarver) | 5 | **3** | 2 | 0 |
| **Total** | **39** | **24** | **14** | **1** |

**Counting boundary:** this is a snapshot of PRs *updated*, not necessarily first created, within 18–23 October. Closed-but-unmerged drafts may represent deliberately superseded experiments rather than unsuccessful work. Merge counts do not capture review, independent testing, architecture, governance, or historical contributions.

## 2. Engineering milestones

| Area | Merged work | Evidence-backed value |
|---|---|---|
| N=8 liveness and recovery | [#408](https://github.com/rhovision-org/rholo/pull/408) | Quorum-backed progress view, sender-local round boundary, bounded announcements and transport retry, missing-parent recovery; original N=8 failure scenario passed source-pinned acceptance. |
| Finality/recovery assurance | [#399](https://github.com/rhovision-org/rholo/pull/399), [#392](https://github.com/rhovision-org/rholo/pull/392), [#402](https://github.com/rhovision-org/rholo/pull/402) | Disposable multi-node harness, bootstrap identity distinct from endpoint, bounded initial LFS reply retry and fail-closed behavior. |
| Restore-state integrity | [#410](https://github.com/rhovision-org/rholo/pull/410), [#411](https://github.com/rhovision-org/rholo/pull/411) | Read committed parameters from newest DAG block post-state; reject Running if a nonempty DAG references a newest block missing from BlockStore. |
| Stall-time correctness | [#379](https://github.com/rhovision-org/rholo/pull/379), [#393](https://github.com/rhovision-org/rholo/pull/393), [#395](https://github.com/rhovision-org/rholo/pull/395) | Remove attempt-count bypass of 15 s stall threshold, align model/tests, use monotonic elapsed time. |
| Protocol and genesis | [#373](https://github.com/rhovision-org/rholo/pull/373), [#376](https://github.com/rhovision-org/rholo/pull/376), [#388](https://github.com/rhovision-org/rholo/pull/388) | Height-based version schedule, genesis-committed validation bounds, format-aware params, `fundPot`, format-2 developer-vesting implementation and restore checks. |
| Language and formal methods | [#389](https://github.com/rhovision-org/rholo/pull/389), [#390](https://github.com/rhovision-org/rholo/pull/390) | Canonical exact Decimal type with deterministic wire/order/arithmetic and Lean comparator/conformance tie. Scope is not a universal proof of all Decimal operations. |
| Security and release gates | [#404](https://github.com/rhovision-org/rholo/pull/404), [#415](https://github.com/rhovision-org/rholo/pull/415), [#391](https://github.com/rhovision-org/rholo/pull/391), [#401](https://github.com/rhovision-org/rholo/pull/401) | Dependency advisory fixes; fail-closed release preflight; auditable hard-fork obligation/decision ledger. |

### N=8 — what passed, exactly

[#408](https://github.com/rhovision-org/rholo/pull/408) provides two initially independent canonical N=8 acceptance campaigns on identical source content: **four successful partition/rejoin cycles**. Seven connected validators advanced finality by at least three heights inside the 300-second bound; the eighth rejoined; all eight converged on finalized height/hash; and the tested durable markers/snapshots were retained. The original failing campaigns remain identified as failures. The evidence is bounded to those workloads and commits.

Jim's [independent retest](https://github.com/rhovision-org/rholo/pull/408#issuecomment-6328812889) also passed the Rust suites and original two-cycle N=8 drill. An initial maintainer review requested activation/stack changes; [final approval](https://github.com/rhovision-org/rholo/pull/408) on `69bf868` confirmed the required **next-new-genesis-only** route, full CI, and exact-head N=8 acceptance. #408 merged on 22 October; the predecessor [#407](https://github.com/rhovision-org/rholo/pull/407), workload discriminator [#406](https://github.com/rhovision-org/rholo/pull/406), and separate proposer-policy [#403](https://github.com/rhovision-org/rholo/pull/403) were closed without merge to keep one canonical integration path.

**Important limit:** changing the deterministic fringe selection is a protocol-compatibility change. It must not be silently installed as a binary replacement on an existing chain.

### Restore hardening — #410 → #411

Patrick's [#410](https://github.com/rhovision-org/rholo/pull/410) fixed a restored-parameters read that incorrectly used the approved fringe root rather than the newest block post-state. Aria's [#411](https://github.com/rhovision-org/rholo/pull/411) subsequently closed the remaining missing-body fallback: a nonempty DAG that names a block whose body is absent from storage must **refuse Running**, even if the fringe root happens to contain matching parameters. Jim [approved the exact correction](https://github.com/rhovision-org/rholo/pull/411), which merged on 23 October.

The #411 PR recorded green three-job CI on both upstream and fork heads; it correctly distinguished prior merged-`dev` real-node acceptance from exact-head acceptance. This is a precise startup/restore admission fix, not a blanket claim of recovery under arbitrary corruption.

## 3. Contribution and value by person

### Aria / `aixaria0` — fault isolation, consensus integration, recovery assurance

- Authored **15 merged PRs** among PRs updated in this window, including [#408](https://github.com/rhovision-org/rholo/pull/408), [#411](https://github.com/rhovision-org/rholo/pull/411), [#399](https://github.com/rhovision-org/rholo/pull/399), [#392](https://github.com/rhovision-org/rholo/pull/392), [#402](https://github.com/rhovision-org/rholo/pull/402), [#379](https://github.com/rhovision-org/rholo/pull/379), [#395](https://github.com/rhovision-org/rholo/pull/395), [#376](https://github.com/rhovision-org/rholo/pull/376), [#401](https://github.com/rhovision-org/rholo/pull/401), and [#404](https://github.com/rhovision-org/rholo/pull/404).
- Traced failures from real-node symptoms through quorum liveness, DAG/parent availability, transport retry, recovery and finalized-state retention; supplied pinned positive **and negative** campaigns and executable acceptance controls.
- Identified and reviewed Genesis/PosParams wire collision, duplicate-holder accounting, restore semantics, creator/replay requirements and audit-register collisions in [#371](https://github.com/rhovision-org/rholo/pull/371)/[#388](https://github.com/rhovision-org/rholo/pull/388). These review findings influenced the reconciled implementation; they are **not** a claim of sole authorship of Patrick's code.
- Added [#412](https://github.com/rhovision-org/rholo/issues/412) to separate historical REV entitlement correctness from validator consensus readiness.

**Value:** converted recurring consensus/recovery failures into reviewable production changes, source-pinned acceptance evidence, release constraints and independently reproducible tests. Primary recent impact: operational safety, liveness and integration assurance.

### Jim / `jimscarver` — architecture, release decisions, independent review

- Authored [#373](https://github.com/rhovision-org/rholo/pull/373), which defines the next-fork version/committed-parameter foundation, and [#415](https://github.com/rhovision-org/rholo/pull/415), the `ctutils` security patch.
- Diagnosed review blockers in [#408](https://github.com/rhovision-org/rholo/pull/408), independently reran its real N=8 workload, and approved the corrected exact head with an explicit genesis-only activation constraint.
- Identified [#410](https://github.com/rhovision-org/rholo/pull/410) as a separately mergeable restore defect, reviewed [#411](https://github.com/rhovision-org/rholo/pull/411), and structured [#51](https://github.com/rhovision-org/rholo/issues/51) decisions, order and accountable deferrals.
- Led the repository rename/migration through merged [#413](https://github.com/rhovision-org/rholo/pull/413) and initiated the outreach plan [#405](https://github.com/rhovision-org/rholo/issues/405).

**Value:** establishes architecture and integration boundaries, provides independent verification, and makes decisions that prevent passing tests from being mistaken for release authorization.

### Patrick / `PatrickMockridge` — protocol implementation, genesis economics, formal semantics

- Authored **six merged PRs** in the recent updated set: [#388](https://github.com/rhovision-org/rholo/pull/388), [#410](https://github.com/rhovision-org/rholo/pull/410), [#389](https://github.com/rhovision-org/rholo/pull/389), [#390](https://github.com/rhovision-org/rholo/pull/390), [#143](https://github.com/rhovision-org/rholo/pull/143), and [#299](https://github.com/rhovision-org/rholo/pull/299).
- Implemented the reconciled format-2 allocation/vesting structure, replay and production-genesis tests, LFS restore matrices, and the distinct post-state restore-root fix.
- Added exact Decimal semantics and formal comparator correspondence; recorded proof boundaries instead of asserting complete formal verification.
- Accepted documented review corrections and kept the superseded [#371](https://github.com/rhovision-org/rholo/pull/371) discussion history. Opened draft [#416](https://github.com/rhovision-org/rholo/pull/416) for a **provisional** recipient address; it is not approved for merge.

**Value:** provides deep protocol, language, formalization and Genesis implementation, with substantial reconciliation between review findings and shipped code.

### Peter / `peteratomic` — community and distribution perspective

- Raised the practical question of whether the proposed additive developer allocation could also support a broader contributor pool in [#371](https://github.com/rhovision-org/rholo/pull/371#issuecomment-6267777897).
- Is assigned the communication/marketing plan in [#405](https://github.com/rhovision-org/rholo/issues/405). Jim and Patrick supplied the initial positioning and audience/implementation feedback.

**Value:** surfaces distribution fairness and prospective user/community needs. Within this window, the retrieved repository record **does not establish a completed marketing rollout or a merged protocol PR attributable to Peter**; assignment is not delivery.

**Attribution principle:** merged implementation, causal defect discovery, independent review, acceptance engineering and governance are different kinds of contribution. This report does not reduce them to commit counts, assign token percentages or assert contractual rights.

## 4. Governance, project transition and current CI

- The repository is now [rhovision-org/rholo](https://github.com/rhovision-org/rholo). [#413](https://github.com/rhovision-org/rholo/pull/413) **merged on 23 October**, updating naming, repository references and documentation; [#414](https://github.com/rhovision-org/rholo/pull/414) closed **without merge**. External integrations and GitHub Pages links still require independent migration verification.
- [#388](https://github.com/rhovision-org/rholo/pull/388) merged on 23 October, and [#371](https://github.com/rhovision-org/rholo/pull/371) closed **superseded, unmerged**. [#372](https://github.com/rhovision-org/rholo/issues/372), the distinct dynamic rank/reserve RFC, is also **closed**, but closure/implementation of #388 does not itself demonstrate approval of its allocation formula, beneficiary, distribution or custody policy.
- [#416](https://github.com/rhovision-org/rholo/pull/416) explicitly says **Draft — do not merge** until recipient/address and default-versus-overlay policy are confirmed. A genesis with that default would fix the address for the chain unless a hard fork occurs.
- Latest observed `dev` head `4d3c7b7` has **3/3 core CI jobs passing** on [run 39985380481](https://github.com/rhovision-org/rholo/actions/runs/39985380481): tests/static gates, formatting/lints and dependency audit. This does not supersede independent formal and release-acceptance requirements.
- [#413 CI](https://github.com/rhovision-org/rholo/actions/runs/39981247312) was **3/3 PASS** before merge. Its separate [formal run](https://github.com/rhovision-org/rholo/actions/runs/39981247409) and the [nightly formal job](https://github.com/rhovision-org/rholo/actions/runs/39983358232) terminated on runner shutdown with exit 143; neither establishes a successful formal gate nor proves a source-level proof failure. An independent clean rerun remains required.
- [#416 CI](https://github.com/rhovision-org/rholo/actions/runs/39976029669) failed the dependency-audit job on its then-current head; `dev` subsequently received #415. Rebase/retest before interpreting its current readiness.

## 5. Outstanding launch gates

1. **Hard-fork disposition — [#51](https://github.com/rhovision-org/rholo/issues/51): OPEN.** The merged [#401](https://github.com/rhovision-org/rholo/pull/401) ledger records 42 *overlapping* obligations: 10 landed references, 12 owner-and-reason deferrals, **20 open**. The complete inventory includes 90 Section-6 rows and 43 conservative category-A candidates. These counts are not 42 independent bugs.
2. **Must-land or independently decided architecture:** crypto-agility [#274](https://github.com/rhovision-org/rholo/issues/274), genesis contract upgrade path (RCHIP #43), governance-controlled schedule/parameters (H2), state preservation and exact provenance/replay for next Genesis; plus storage-rent policy. See #51.
3. **REV migration entitlement correctness — [#412](https://github.com/rhovision-org/rholo/issues/412): OPEN.** Historical snapshots, finalized cut-off, liquid/bonded/locked semantics, duplicate claims, supply reconciliation, and a safe read-only holder verification path need authenticated sources and an approved decision. A green N=8 run does not validate a wallet allocation.
4. **Recipient and economics:** [#416](https://github.com/rhovision-org/rholo/pull/416) remains draft; the distinct fixed allocation, dynamic pool RFC history, address custody and governance authority must not be conflated.
5. **Deployment / fork safety:** #408's deterministic consensus change is authorized for next new genesis, not a silent swap onto historical chains; current-source N=8, full recovery, state preservation, matching binaries and rollback/restore policies still need full release-level sign-off.
6. **Migration and formal verification:** [#413](https://github.com/rhovision-org/rholo/pull/413) is merged; verify external rename dependencies and obtain a clean formal-gate rerun after the recorded runner shutdowns. Resolve [#416](https://github.com/rhovision-org/rholo/pull/416) CI separately. Closed test drafts should not be reopened solely for their PR numbers.

## Conclusion

The project is in a stronger engineering state than it was on 18 October: the original N=8 one-validator-down blocker now has a merged correction backed by independent real-node evidence; restore trust boundaries are stricter; the protocol release base, exact Decimal type, dependency patching and release accounting are more explicit.

The remaining risk has shifted from isolated implementation defects toward **release integration, genesis authorization, historical entitlement reconciliation, compatibility, and governance decision closure**. The next valuable milestone is not an unqualified “ready for mainnet” declaration. It is a **single source-pinned release-candidate evidence packet** with final binary identity, N=8/recovery pass criteria, Genesis/REV reconciliation, #51 dispositions and named approval authority.

*Evidence snapshot compiled from GitHub repository metadata, reviews and Actions records as observed on 23 October 2026. It is an informational status report, not an authorization to merge, migrate historical state or deploy.*