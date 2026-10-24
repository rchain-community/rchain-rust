# Mainnet acceptance evidence: proposed M1–M6 gate

Tracking: [#428](https://github.com/rhovision-org/rholo/issues/428). Incident provenance: [#420](https://github.com/rhovision-org/rholo/issues/420).

**This is a proposed, fail-closed evidence evaluator. It is not an authorized release policy or a runner that can certify Mainnet.** It neither runs live-node workloads nor performs a merge or deployment. Maintainers must approve the policy, verify the witnesses and make the release decision separately.

## Execution

```sh
python3 -m unittest discover -s tools/tests -p 'test_mainnet_acceptance.py' -v
python3 tools/mainnet-acceptance.py \
  --manifest spec/audit/evidence/mainnet-acceptance/manifest.json \
  --json-out /tmp/mainnet-acceptance.json \
  --md-out /tmp/mainnet-acceptance.md
# Returns 2 while any release gate is BLOCKED/FAIL; use only for a real release gate:
python3 tools/mainnet-acceptance.py \
  --manifest spec/audit/evidence/mainnet-acceptance/manifest.json --strict
```

The ordinary report command returns zero when the *evaluator worked*, **not when Mainnet passed**. In GitHub Actions, manual **Run workflow** defaults to \`mode=report\` to generate a non-certifying report; selecting \`mode=strict\` deliberately makes the job fail (exit 2) until every M1–M6 evidence contract passes. Even a strict pass is **PENDING_MAINTAINER_SIGNOFF** and explicitly **NOT_CERTIFIED**. A PR's green validation badge is **never** a claim that the strict acceptance gate has passed.

## Evidence contract (v1)

The manifest names an explicit 40-character candidate source revision and exactly six gate keys. Every gate requires all the criteria listed in `tools/mainnet-acceptance.py:CRITERIA`, with precisely one evidence descriptor for each criterion:

```json
{
  "criterion": "supported_capacity",
  "path": "proofs/isolated-supported-tier.json",
  "sha256": "<SHA-256 of the referenced JSON file>"
}
```

Evidence paths are relative to the manifest directory; absolute paths, traversal, external symlinks, empty/oversized files (maximum 256 KiB) and hash mismatches are blocked. Files are parsed from the **same bounded bytes** used for their SHA-256. Duplicate JSON members, non-standard \`NaN\`/\`Infinity\`, extra manifest/witness fields, invalid UTF-8 and noncanonical UTC timestamps fail closed. Each referenced JSON witness must identify the *same* `candidate_sha`, `gate` and `criterion`, a reproducible `command`, an `environment`, a unique `run_id`, UTC `started_at`/`finished_at` and a `checks` mapping containing **exactly** the named required affirmative assertions. Example shape (illustrative only; do not submit fabricated records):

```json
{
  "gate": "M1",
  "criterion": "supported_capacity",
  "candidate_sha": "<40-character release source SHA>",
  "run_id": "isolated-capacity-20261024-01",
  "command": "./tools/your-measured-acceptance-runner --profile supported-tier",
  "environment": "independent-hosts; topology and memory profile attached separately",
  "started_at": "2026-10-24T10:00:00Z",
  "finished_at": "2026-10-24T11:00:00Z",
  "checks": {
    "supported_tier_declared": true,
    "memory_headroom_measured": true,
    "no_unexpected_oom": true
  }
}
```

The sample above is illustrative and **must not** be submitted as evidence. The v1 schema accepts only the documented fields and exactly the published affirmative \`checks\` for each criterion. These fields are **not a cryptographic proof of the measurements**. The evaluator verifies file integrity and schema consistency, not the authenticity of external hosts, identities, timestamps, reported booleans, or any particular consensus invariant. The owner must preserve raw run artifacts, hardware/cgroup measurements, block/state hashes and reviewer evidence **outside the public fixture**, with a verifiable chain of custody and independent review. A manually edited JSON report must not be treated as an independently executed acceptance run.

## Gate meaning

| Gate | Required criteria | Necessary independent campaign |
| --- | --- | --- |
| M1 — capacity/fault domains | `supported_capacity`, `failure_domains`, `restart_replay` | Supported-tier, independently hosted validators; measured RSS/cgroup, fault-domain topology and bounded restart/replay. |
| M2 — admission/overload | `http_ingress`, `grpc_ingress`, `overload_recovery` | Direct HTTP **and gRPC**, byte/concurrency limits, rejection semantics, bounded recovery. |
| M3 — external bootstrap | `external_observer`, `invalid_peer_negative`, `bootstrap_failover` | Off-host routable observer, response **delivery and application**, rejected loopback control, alternate bootstrap. |
| M4 — finality/recovery | `n4_finality`, `n8_finality`, `partition_rejoin`, `host_loss_recovery` | Named supported-tier N=4/N=8 finalization, partition/rejoin, fault-domain loss and approved-state safety. |
| M5 — state integrity | `merge_840_841`, `merge_accounting`, `hot_cell_regression` | Source-pinned 840→841 merge replay; final-scope accepted/rejected effects and no silent accepted-write loss. |
| M6 — provenance | `release_provenance`, `audit_trail`, `independent_review` | Binary/source/config/bond pin; original raw witnesses and commands; independent maintainer review. |

**The current manifest intentionally carries no evidence and yields six BLOCKED gates.** Its `candidate_sha` is the source baseline at implementation, **not an assertion that this candidate has passed acceptance**. Replace it with the actual release-candidate SHA only after running acceptance against that binary/config. Every item must be regenerated for the exact same candidate revision.

## Status semantics and release authority

- **PASS:** all required assertions are present, affirmative, and bound to matching SHA-256 / candidate metadata. This says nothing by itself about evidence authenticity or mainnet approval.
- **FAIL:** a complete, integrity-checked witness contains a failed required assertion.
- **BLOCKED:** no witness, duplicate witness, missing/invalid assertions, path escape, corrupted digest or mismatched revision. A missing test is never PASS.
- **Release decision:** `BLOCKED` unless all six gate contracts pass, then `PENDING_MAINTAINER_SIGNOFF`.
- **Certification:** always `NOT_CERTIFIED`. External, authorized maintainers must decide whether actual raw evidence supports a release; the evaluator cannot self-authorize.

The CLI tests use synthetic fixtures and do not count toward runtime acceptance. No secret, signing key, original private host log, IP-bearing access record or private deploy key belongs in the committed manifest.

## Release review checklist

The reviewer must confirm the exact source/binary version on every host; stake/topology and supported hardware; test isolation; externally routable peer identity; original raw logs with hash/UTC provenance; accepted/finalized-state trace and merge rejection accounting; negative/fault-injection controls; reproducibility on independent infrastructure; absence of undisclosed environmental differences; and a documented sign-off decision. CI checks evaluate the **format and declared assertions**, not these external attestations.

Related draft changes [#421](https://github.com/rhovision-org/rholo/pull/421) (diagnostics), [#426](https://github.com/rhovision-org/rholo/pull/426) (HTTP only) and [#427](https://github.com/rhovision-org/rholo/pull/427) (fringe logs only) are **not sufficient to pass M1–M6**. No live load or restart is necessary to run the evaluator.
