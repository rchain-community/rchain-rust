# Incident #420 — Engineering disposition, provenance and release gates

**Record date:** 2026-10-24 (UTC)  
**Incident:** [rhovision-org/rholo#420](https://github.com/rhovision-org/rholo/issues/420)  
**Deployed source under investigation:** `71c6a513d9785e45c3fbb919125c3314c2d8bf6e`  
**Runtime development baseline:** `dev@150d21d7775ab2e8d22e8d904270a97a53bd9878`  
**Change policy:** Three independent review tracks; no merge, validator restart, testnet deployment, genesis rebuild, or public network stress was performed.

## 1. Work delivered

| Workstream | Change set | Verified scope | Unverified scope |
| --- | --- | --- | --- |
| Incident provenance | [Draft #421](https://github.com/rhovision-org/rholo/pull/421) | Bounded read-only HTTP snapshots; offline kernel-OOM parser; strict pinned Root witness parser; 30 Python regression tests; triage runbook; dedicated and upstream CI **SUCCESS** at `ce34948e1a66e1528846142f380bde082eabd2e0` | Independent raw B/C reads, Root accepted-write provenance, host A/D journal, operator recovery and network soak |
| HTTP concurrent admission | [Draft #426](https://github.com/rhovision-org/rholo/pull/426) | Opt-in in-flight bounds for deploy/explore requests; shared budget per legacy/v1 alias, `503` and `Retry-After: 1` before JSON extraction, cancellation-safe permit lifetime, admission status route, OpenAPI and operator runbook. Five new targeted Rust unit tests **PASS** in the connected environment; upstream CI **SUCCESS** at `adebec68a06224bfccf910daa71d91b303f3a5e8` | Formal gate, isolated capacity/latency trial; actual cgroup-aware backpressure; P2P/consensus workload limits |
| Fringe refusal diagnostics | [Draft #427](https://github.com/rhovision-org/rholo/pull/427) | Logs received request, peer/anchor, absent BlockStore anchor, absent genesis and unreadable fringe metadata. Fail-closed response suppression preserved; regression coverage expanded. Formatting, dependency audit and full upstream CI **SUCCESS** at `57974f4cbd7184ac6473ba97e7513ce7301d5344` | No independent observer bootstrap acceptance, multi-peer failover or versioned negative reply |

The regressions in #426 test runtime concurrency admission; they are **not** a demonstration that an under-provisioned node cannot run out of memory. #427 changes refusal visibility but not the requested state root or the wire protocol. #421 is evidence tooling only.

## 2. Incident facts and attribution boundary

- **Failure domain:** two 1 GiB DigitalOcean hosts, two 250-stake validators per host; either single-host failure can make 500/1000 bonded stake unavailable. Stake distribution by logical validator does not erase host co-location risk.
- **Confirmed B/C host OOM:** kernel journal contains matched OOM-kill records for `rnode.service` at `2026-10-23T18:37:41Z` and `18:52:02Z`, and `rnode-c.service` at `18:46:21Z` and `22:15:49Z`. The preserved *private* 1,644-byte excerpt is identified by SHA-256 `dcc20c578a7e96dff65de68cd22934766dd0732ff0d1db27058ff24bac135e8f`. A digest alone is not an independent operator attestation; the excerpt and credentials are not published.
- **Reported network effects:** health `mem_available_mb` reached 16 then 1; HTTP request timeouts, observer fringe acquisition failures, divergent tip heights and lagging finality were observed in the incident period. Per-request attribution to a particular OOM kill, specific handler failure or safety violation is **not** established. A/D kernel evidence was not independently checked.
- **Separate client vantage:** on 2026-10-23 at 22:43 UTC, six bounded GET-only probes of public/direct health or last-finalized-block endpoints timed out at the 2-second connect deadline from the connected client. A GitHub HTTPS reachability control succeeded. These probes show *client-path unreachability*, not that validators were definitely down. No deploy or additional expensive historical Root read was sent.
- **Historical comparator:** [#280](https://github.com/rhovision-org/rholo/issues/280) documents a distinct accepted/finalized-write merge regression and its subsequent fix path. It demonstrates why *matching reads from all validators* cannot by itself exclude a shared regression; it does not prove #420 reproduces #280 or carries the same unfixed code.

## 3. Root-state witness — precise unresolved discriminator

The r-cash installation reported 17 deploys at **18:00:03–18:03:06 UTC**, preceding the later severe-memory window. Do not infer OOM as its cause from timing.

| Reported post-state | Full block hash | Reported `component:governor` |
| --- | --- | --- |
| Block 835 | `3c12c6e02aec52b44c8f0953e1a60f9dedd5a1ed5ebc40d76a0df4f4200bd6d6` | present |
| Block 845 | `083e118904f2fdf27e87e03ee2523d7c805dd960a29c013b1d5220dbb8a51950` | absent |
| Reported finalized height 1254 | `dfed8a269ed35d893e64b61abb6fcfbe2c1112a38ea5cb449cdfeec1fd3e79db` | evaluate matched finalized post-state |

B/C reads and a justification ancestry walk are reported in [Patrick's evidence](https://github.com/rhovision-org/rholo/issues/420#issuecomment-6375656151) and [reproducer](https://github.com/rhovision-org/rholo/issues/420#issuecomment-6375688758), but original raw responses were not attached for independent re-parsing. The 830→845 `bounds` comparison is **not** an equivalent witness, because the published 830 post-state already lacks `bounds`.

The current [Root implementation](https://github.com/PatrickMockridge/r-cash/blob/main/contracts/build/deploy/v3/00_root.rho) inserts keys on its accepted success path. Nevertheless a justification ancestor is not proof that all the ancestor's deploy effects were accepted after a *multi-parent merge*. `ProcessedWithSuccess` at inclusion is not sufficient either. Classify the merged accepted/rejected effects and `rejectedDeploys` before asserting a lost **accepted, finalized** write. A genuine accepted and finalized effect missing from a later finalized post-state without an admissible state transition **would** be a material integrity defect.

The #421 offline parser distinguishes failed reads from valid empty sets, same-hash B/C parity, verified versus incomplete ancestry and observed post-state key discontinuity. It cannot infer finalized write loss from those fields alone.

**Independent evidence required:** sanitized `.testnet/install.json`, exact historical r-cash installer and deployed contract source hashes/URI, original B/C `Root.keys` JSON at identical full block hashes with `usePreStateHash: false`, complete justification path, before/after state hashes, accepted/rejected deploy chains, and a genuinely finalized post-state witness. A separate bounded, disposable sequential/parallel register-and-read replay must produce the result before classifying root cause. Do not publish private keys or operator credentials.

**Third-party source provenance:** current `r-cash/docs/TESTNET.md` describes per-component `Root.resolve` verification/repair by `harness/install_testnet.py`, but the current visible installer file does not include that verification pass. Patrick separately reports a repair pass whose deploy IDs were not retained. This is a source/documentation discrepancy requiring a pinned historical revision, **not** evidence that a separate pass did not happen.

## 4. Disposition of every mitigation requested in #420

| Requested mitigation | Status after this work | Next bounded acceptance gate |
| --- | --- | --- |
| OOM/event correlation | Confirmed B/C kills; diagnostic parser in #421 | Preserve A/D host journal, cgroup `memory.events`, RSS, unit restart count, binary SHA and synchronized UTC clocks |
| API overload control | Opt-in concurrent HTTP gate in #426 | Measure tail latency, `429` vs `503` rejection rate, queue/cgroup high-water marks and finality under controlled isolated load |
| Startup minimum-memory preflight and override | Not implemented | Calibrated host/cgroup v1/v2 and shared-host budget; define explicit operator override and startup refusal semantics |
| Memory-aware backpressure | Not implemented | Attribute memory to Rholang interpreter, block replay, message processing, in-flight queues and merge; measure before setting thresholds |
| Bound in-flight deploys and term bytes | Only HTTP concurrency bound; no per-block consensus limit | Profile memory/phlo/term costs and review validity/consensus effects of per-block bounds |
| HTTP `degraded` and peer gossip | Not implemented | Define readiness semantics and versioned, authenticated peer signal with false-positive handling |
| Missing fringe and storage failure observability | Source-safe logging in #427 | Correlate receipt/refusal/sent response by peer/anchor in a disposable joiner campaign |
| Structured fringe negative reply | Not implemented | Protocol schema/version compatibility and requester handling; never substitute an unrequested genesis or root |
| Multi-bootstrap peer list and failover | Not implemented | Deterministic peer selection, anchored state verification, silent-first-peer replay and partition/rejoin acceptance |
| Finality-height/divergence alerts | Snapshot diagnostics only | Timed multi-validator finalized height, hash, state and lag semantics, not merely tip-height comparisons |
| Root insert-only regression | Offline witness tooling only | Original pinned B/C JSON, accepted merge provenance and actual Rholang integration replay |
| Memory cgroup soak at 1 GiB versus adequate capacity | Not executed | Disposable isolated hosts, exact binary/workload, bounded stop conditions; no live testnet load |
| Validator topology remediation | Co-location risk documented | Operator-approved host/stake isolation, sizing plan and restart/deploy authorization |
| Release, merge or mainnet declaration | Not authorized | Green gated CI, review, isolated acceptance and an explicit release decision |

## 5. Review/rollout control

1. Keep all three PRs in **Draft** until their independent test gates and maintainer review are complete. A passing diagnostics test is not a protocol acceptance certificate.
2. Do not deploy #426 with arbitrary limits. Both environment variables are **unset by default**; instrument a disposable node before selecting `RNODE_HTTP_MAX_INFLIGHT_DEPLOY` and `RNODE_HTTP_MAX_INFLIGHT_EXPLORE`. Rollback removes the settings through the approved operator restart procedure.
3. Do not permit #427 to replace a missing anchored state with another root. The response remains fail-closed; stronger negative replies and peer failover require a separately reviewed protocol change.
4. Reproduce the Root witness with immutable inputs and distinguish *rejected merger effects*, *stale/nonfinalized reads*, and *verified accepted-finalized write loss*. A finding in one class must not be reported as another.
5. Keep #420 **OPEN** until operational reproduction, remediation evidence, affected-host recovery and state-integrity attribution have been separately completed.

## 6. Evidence index

- [Incident and third-party witness](https://github.com/rhovision-org/rholo/issues/420)
- [Diagnostics and local triage, #421](https://github.com/rhovision-org/rholo/pull/421)
- [HTTP concurrency admission, #426](https://github.com/rhovision-org/rholo/pull/426)
- [Fail-closed fringe diagnostics, #427](https://github.com/rhovision-org/rholo/pull/427)
- [Merge semantics at deployed revision](https://github.com/rhovision-org/rholo/blob/71c6a513d9785e45c3fbb919125c3314c2d8bf6e/docs/src/node/block-merge.md)
- [Preserved prior merge regression #280](https://github.com/rhovision-org/rholo/blob/71c6a513d9785e45c3fbb919125c3314c2d8bf6e/spec/audit/evidence/n280-merge-loses-a-write-results.md)

**Disposition:** partial runtime hardening is ready for review, while the production incident's memory/finality recovery and Root accepted-effect attribution remain explicitly unclosed. This record makes no claim of a validated mainnet or testnet fix.
