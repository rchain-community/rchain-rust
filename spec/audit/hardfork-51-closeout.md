# Issue #51 — Hard-fork closeout ledger

This packet follows the [close-clause decision of 21 October 2026](https://github.com/rhovision-org/rholo/issues/51#issuecomment-6316299117): each required row must be landed or deferred with a named decision owner and a one-line reason. A release name is optional. The pinned source is upstream `dev@93dbdf364f14d728a27455b4e05282eace2e3c47`.

## Current result

**TRACKER_MUST_REMAIN_OPEN.** [hardfork-51-closeout.json](hardfork-51-closeout.json) records 42 overlapping tracker, audit and release obligations: **10 landed implementation references, 12 maintainer-recorded deferrals and 20 open obligations**. These counts are not counts of distinct defects. Landed implementation and outstanding deployment evidence are recorded separately.

| Landed implementation | Merge reference | Remaining boundary |
|---|---|---|
| Timestamp, #44 | `228b66742f` | Release coordination remains distinct |
| String functions, #47 | `f07a2628a7` | Client and historical behavior remain distinct |
| Attachments, #53 | `9a04285a6b` | Appears in both §A and §B; uniform validator/client rollout is separate |
| BigInt/Decimal, #49/#389/#390 | #389: `666ddb75e2` | #373 and #391 are also merged; archive scans, uniform validators and the new-genesis ceremony remain open |
| Activation schedule, committed bounds, params format and fundPot, #373/#376 | `c42440716d` | Integrated distributed acceptance remains separate |
| Parent-bound integration, #160/#373 | `c42440716d` | Historical and distributed release evidence remains separate |
| Live finality partition, #108 | `a4dd4ede66` | Full bonded-stake denominator remains required |
| Stake-weighted active-set draw, #186 | `37ed6f5068` | Replaced the historical uniform draw; residual incentives are not certified |
| C207 cost merge, #209 | `5f0763eb4d` | Concurrent native-ledger conflicts and partition liveness remain separate |

#404, #373, #391, #392 and #402 are all in the pinned dev ancestry. The checker verifies every ledger merge reference against that source when `--verify-local-git` is supplied. #379 remains a proposer timing correction outside category A.

## Complete source inventory

[section6-gap-inventory.json](section6-gap-inventory.json) covers **all 90 current §6 rows**, with 43 conservative category-A candidates. The refresh accounts for C334, the revised Decimal activation decision, the inert height schedule, committed bounds, fundPot and C335. C334/C335 are node-local recovery changes and the shipped `(1, 0)` schedule is inert; they are recorded as non-A. Committed bounds and fundPot are A candidates. These classifications remain reviewable dispositions, not an independent safety attestation.

The [gap matrix](final-gap-matrix.md) gives each obligation its invariant, existing evidence, missing proof, next action and acceptance condition. Source-row hashes, line numbers and evidence URLs must agree with the pinned register. A documented deferral must agree between the matrix and the validated ledger. Neither file can invent an independent sign-off or a passed execution.

## Maintainer-recorded deferrals

The [owner decision](https://github.com/rhovision-org/rholo/issues/51#issuecomment-6317072399) records 11 distinct deferred proposal rows, represented by 12 ledger entries because RCHIP #32 has an interface cross-reference. C173, #127 stage 4, RCHIPs #4, #32, #31, #47, #46, #7/#8, #38 and #30 are owned by @PatrickMockridge; the inactivity leak is owned by @aixaria0. Each entry retains the specific one-line reason, decision author and source URL. These are actual maintainer dispositions, not inferred approvals.

The two newly explicit technical proposals and the separate allocation-track choice were added to the required inventory, bringing it from 39 to 42 obligations. [The integrated run evidence](evidence/hardfork-51-integrated-20261021.md) records successful scoped arms alongside the failed partition arms.

## Decisions and acceptance still open

- **Crypto-agility (#274 §14):** @jimscarver owns the track. Freeze the registry/wire, key identity, deploy hash, activation, pricing and sunset policy before implementation. The already-landed seed-length typed error is not the complete agility fork.
- **RCHIP #43:** decide the governed blessed-contract upgrade shape, then implement its state/replay and authorization tests. The A and B rows share this obligation.
- **Decisions still open:** choose one #371/#388 developer-allocation track and disposition separate RFC #372; #40 storage rent remains open. Compatibility translation, attestation scope and other uncovered A/B proposals remain as listed in the matrix. The recorded deferral of RCHIP #47 does not choose an allocation PR.
- **Plan H2 and state preservation:** governance-settable rules, authenticated old-state exports, genesis hashes, validator binary hashes and the reviewed rollout remain open. #391's merged preflight checks evidence consistency; it cannot supply missing operator evidence.
- **Integrated acceptance (#339/#399):** preserve independent N=8 finality/partition and anchored-recovery outcomes on one exact image. #399 also carries the unmerged #403 experiment, so its result cannot be represented as evidence for bare dev without identifying that additional source.
- **Historical deviations:** the current Law 53 inventory already records the C173 inverses as shipped. Reconcile their exact ancestry and replay/rejoin evidence rather than implementing them again. Independently review all §6 category dispositions.
- **Faucet (#247):** the recorded resolution uses an already-funded key; the live key/finalized-drip attestation remains outstanding. Do not invent another genesis allocation.

## Reproduce

```sh
python3 -m unittest discover -s tests -p test_hardfork_51_closeout.py -v
python3 -m unittest discover -s tests -p test_section6_inventory.py -v
python3 tools/check-section6-inventory.py
python3 tools/check-hardfork-51-closeout.py --verify-local-git
python3 tools/check-hardfork-51-closeout.py --verify-local-git --assert-closeable
# expected final exit: 3, TRACKER_MUST_REMAIN_OPEN
```

The controls cover owner-and-reason deferrals without release names, absent/generic owners, absent/placeholder reasons, actual and escaped line breaks, malformed optional decision links, omitted or duplicated rows, mismatched matrix deferrals, stale source links/line numbers and invented sign-off. An eligibility result only requests independent closeout review; it never closes the issue or authorizes a network change.
