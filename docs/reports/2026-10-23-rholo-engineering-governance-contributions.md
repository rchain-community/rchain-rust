# Rholo — Engineering, Governance & Contribution Status

**Reporting window:** 18–23 October 2026  
**Repository:** [rhovision-org/rholo](https://github.com/rhovision-org/rholo)  
**Reference branch:** `dev`  
**Observed branch head:** `4d3c7b716ca370bf8e6751e4479829c42fecaa31` (23 October, 16:16 UTC)  
**Method:** GitHub PR/issue metadata, review history, PR descriptions, merge states, and Actions results. Historical acceptance is attributed to its exact source, not automatically to current `dev`.
**Metadata cutoff:** 23 October 2026, 18:46 UTC (UTC calendar window 18–23 October, inclusive). The activity counts are time-bounded, not lifetime rankings.

## Executive summary

The project made substantial, independently reviewable progress in consensus liveness, finalized-state recovery, genesis protocol infrastructure, exact decimal semantics, dependency security, and release tracking. The key achievement is the integration of the canonical **N=8 validator partition/rejoin correction** through [#408](https://github.com/rhovision-org/rholo/pull/408), supported by multiple real-node campaigns and an independent maintainer retest. Follow-on restore hardening [#410](https://github.com/rhovision-org/rholo/pull/410) and [#411](https://github.com/rhovision-org/rholo/pull/411) is also merged.

This is **not** evidence of mainnet readiness. The repository describes itself as pre-testnet, [#51](https://github.com/rhovision-org/rholo/issues/51) remains open, historical REV reconciliation is still a decision/evidence gate, and next-genesis rollout and compatibility require explicit authorization and acceptance.

## 1. Snapshot of recent activity

The **41 pull requests updated** in the UTC reporting window are distributed as follows. The unit is a unique PR with an `updatedAt` date from 18 through 23 October, counted by the GitHub **PR author**; these are **not** percentages of code ownership, engineering value or compensation.

| PR author | Updated PRs | Merged PRs | Closed without merge | Still open |
|---|---:|---:|---:|---:|
| [aixaria0](https://github.com/aixaria0) | **27** | **15** | 11 | 1 |
| [PatrickMockridge](https://github.com/PatrickMockridge) | 9 | 6 | 1 | 2 |
| [jimscarver](https://github.com/jimscarver) | 5 | 3 | 2 | 0 |
| **Total** | **41** | **24** | **14** | **3** |

**Counting boundary:** The query covers PRs updated in this window, including work opened earlier and PRs #418 and #419, which were opened after the original report draft. Merged status is determined from `mergedAt`, `CLOSED` means closed without merge, and `OPEN` includes Draft PRs. Closed-but-unmerged experimental PRs can be deliberately superseded. The counts may change if a PR receives another update; the cutoff above is binding.

### Source history, identity and attribution

- **Same repository, new address:** `rchain-community/rchain-rust` and `rhovision-org/rholo` resolve to the same GitHub repository ID `1343088956`, originally created on 22 August 2026. [#413](https://github.com/rhovision-org/rholo/pull/413) renamed the project and updated documentation and links. This was not a separate clean-slate codebase, nor a transfer of existing commit authorship.
- **Three different identities:** PR **author** identifies who opened the review; Git **commit author** identifies a change as recorded in Git history; **merge-commit author** or a file's **latest editor** may instead reflect review, integration or renaming. None of these alone measures who identified a defect, reproduced a failure, wrote the final fix, or approved a change.
- **Verified authored source:** the constituent commits of the merged [N=8 #408](https://github.com/rhovision-org/rholo/pull/408) are attributed to `aixaria0` (with Git author names Aria/AixAria). Its merge commit is attributed to Jim; the merge does not change those original authors. The merged [restore #411](https://github.com/rhovision-org/rholo/pull/411) contains source commits [`fb3aecd5d7`](https://github.com/rhovision-org/rholo/commit/fb3aecd5d776347e1cf32105d63a6999da677cdf) and [`29daf5c0ef`](https://github.com/rhovision-org/rholo/commit/29daf5c0ef0770051793283de78a485d1665ba01), both attributed to `aixaria0`.
- **What the rename changes:** the latest README/file history includes #413 naming edits and may highlight those newer edits. That does not erase the underlying N=8, recovery, formal, genesis, and other commits. Nor does it establish that any one contributor authored the entire current implementation.
- **Limits:** This is a reviewable engineering record. The GitHub `write` permission currently observed for `aixaria0` is not, by itself, a specification of organizational rank, exclusive code ownership, copyright assignment, token entitlement or integration authority. Claims beyond the traceable repository record require separate decisions.

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
- The original `aixaria0` commit identities remain visible on [#408](https://github.com/rhovision-org/rholo/pull/408) and [#411](https://github.com/rhovision-org/rholo/pull/411); later merge and rename commits do not supersede this authorship. This is not a claim of sole ownership over prior architecture or others' independent review.
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
- [#413 CI](https://github.com/rhovision-org/rholo/actions/runs/39981247312) was **3/3 PASS** before merge. Earlier separate [formal run](https://github.com/rhovision-org/rholo/actions/runs/39981247409) and [nightly formal job](https://github.com/rhovision-org/rholo/actions/runs/39983358232) terminated with exit 143 and runner-shutdown messages; this is not proof of a Lean/Coq theorem failure or of an OOM event. Patrick's [#419](https://github.com/rhovision-org/rholo/pull/419) isolates `Rchain.Laws` and records memory on the runner. The two **independent, exact-head** formal runs [40001293363](https://github.com/rhovision-org/rholo/actions/runs/40001293363) and [40001348627](https://github.com/rhovision-org/rholo/actions/runs/40001348627) both **PASS** on `dd764fe7443ec1e34cb0134f9ea172f0a7a5b179`. The logged largest sampled process RSS values were **14,103,368 KiB** and **14,097,784 KiB**, respectively, on runners reporting approximately **15,989 MiB** physical RAM. This satisfies the two-run observation but does not establish the original SIGTERM cause or guarantee future runner availability. [#417](https://github.com/rhovision-org/rholo/issues/417) remains open and #419 remains unmerged at this snapshot.
- [#416 CI](https://github.com/rhovision-org/rholo/actions/runs/39976029669) failed the dependency-audit job on its then-current head; `dev` subsequently received #415. Rebase/retest before interpreting its current readiness.

## 5. Outstanding launch gates

1. **Hard-fork disposition — [#51](https://github.com/rhovision-org/rholo/issues/51): OPEN.** The merged [#401](https://github.com/rhovision-org/rholo/pull/401) ledger records 42 *overlapping* obligations: 10 landed references, 12 owner-and-reason deferrals, **20 open**. The complete inventory includes 90 Section-6 rows and 43 conservative category-A candidates. These counts are not 42 independent bugs.
2. **Must-land or independently decided architecture:** crypto-agility [#274](https://github.com/rhovision-org/rholo/issues/274), genesis contract upgrade path (RCHIP #43), governance-controlled schedule/parameters (H2), state preservation and exact provenance/replay for next Genesis; plus storage-rent policy. See #51.
3. **REV migration entitlement correctness — [#412](https://github.com/rhovision-org/rholo/issues/412): OPEN.** Historical snapshots, finalized cut-off, liquid/bonded/locked semantics, duplicate claims, supply reconciliation, and a safe read-only holder verification path need authenticated sources and an approved decision. A green N=8 run does not validate a wallet allocation.
4. **Recipient and economics:** [#416](https://github.com/rhovision-org/rholo/pull/416) remains draft; the distinct fixed allocation, dynamic pool RFC history, address custody and governance authority must not be conflated.
5. **Deployment / fork safety:** #408's deterministic consensus change is authorized for next new genesis, not a silent swap onto historical chains; current-source N=8, full recovery, state preservation, matching binaries and rollback/restore policies still need full release-level sign-off.
6. **Migration and formal verification:** [#413](https://github.com/rhovision-org/rholo/pull/413) is merged; verify external rename dependencies. Formal-gate #419 has two passing exact-head runs but the previous runner-shutdown cause is unproven; retain [#417](https://github.com/rhovision-org/rholo/issues/417) as an independently dispositioned reliability issue. Resolve [#416](https://github.com/rhovision-org/rholo/pull/416) CI separately. Closed test drafts should not be reopened solely for their PR numbers.

## Conclusion

The project is in a stronger engineering state than it was on 18 October: the original N=8 one-validator-down blocker now has a merged correction backed by independent real-node evidence; restore trust boundaries are stricter; the protocol release base, exact Decimal type, dependency patching and release accounting are more explicit.

The remaining risk has shifted from isolated implementation defects toward **release integration, genesis authorization, historical entitlement reconciliation, compatibility, and governance decision closure**. The next valuable milestone is not an unqualified “ready for mainnet” declaration. It is a **single source-pinned release-candidate evidence packet** with final binary identity, N=8/recovery pass criteria, Genesis/REV reconciliation, #51 dispositions and named approval authority.

*Evidence snapshot compiled from GitHub repository metadata, reviews and Actions records with an activity-count cutoff of 23 October 2026, 18:46 UTC. It is an informational status report, not an authorization to merge, migrate historical state or deploy.*
