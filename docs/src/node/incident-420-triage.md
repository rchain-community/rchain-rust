# Issue #420: bounded read-only incident evidence

**Scope:** [validator resource-pressure incident #420](https://github.com/rhovision-org/rholo/issues/420), rhobot public testnet, 23 October 2026. This note is a diagnostic plan, not a root-cause verdict, a release gate, authorization to change the testnet, or proof that finalized writes were lost.

## Confirmed observations and limits

- Four bonded validators, two per 1 GB host, each with a quarter of bonded stake; one host failure removes **half** the active stake. This is a correlated-failure topology, not four independent failure domains.
- Deployed binary reported by node A: **71c6a513d9785e45c3fbb919125c3314c2d8bf6e** (11 October), **not** merged N=8 [#408](https://github.com/rhovision-org/rholo/pull/408) or restored-state [#411](https://github.com/rhovision-org/rholo/pull/411). Do not apply incompatible post-merge code to the existing genesis.
- The testnet's /health snapshot reports host available memory, but is refreshed only about once per minute. **ok: true** is not a minimum-memory or finality-progress guarantee. Observe snapshot **ts** beside every reading.
- Under the reported r-cash load, available memory fell to 16 then 1 MB, API requests timed out, and an observer could not obtain a finalized fringe. The observed height difference (1264 versus 1271) is **not by itself a divergent-finalized-state witness**.
- The 0.25 MB/block and 0.2 s/block replay figures cited in the issue are from [#60](https://github.com/rhovision-org/rholo/issues/60)'s earlier build. [#68](https://github.com/rhovision-org/rholo/issues/68) and [#154](https://github.com/rhovision-org/rholo/issues/154) explicitly flag later measurements as unreproduced after the metadata-rebuild change. Do not size the current binary by linearly extrapolating those numbers.
- An API deploy rate limiter already exists on the deployed version. A rate limiter is not an in-flight memory budget. A load-sensitive admission gate would be a separate change requiring evidence and tests.
- The finalized-fringe handler can deliberately decline a reply for an unavailable anchor or absent stored state. **Do not replace that with an unrelated genesis or ordinary fringe**; first distinguish transport timeout, handler starvation, state lookup failure, and a legitimate missing anchor.
- The r-cash Root registration anomaly is **unclassified** pending deploy/block IDs, finalized-state evidence and a comparable cross-node read. An application response alone does not prove loss of a finalized write.

## Read-only witness collection

The probe uses only bounded GETs; one sample requests status and last finalized block from each supplied node and a separate health snapshot. It neither submits deploys nor initiates peer sync.

Run from an authorized client with endpoint reachability:

~~~sh
python3 tools/incident-snapshot.py \
  --node A=https://testnet.rhobot.net \
  --node B=http://104.131.176.164:40403 \
  --node C=http://104.131.176.164:41403 \
  --node D=http://164.90.140.144:41403 \
  --health A=https://testnet.rhobot.net/health \
  --timeout 4 --samples 3 --interval 60 \
  --output incident-420-public-observations.jsonl
~~~

The JSONL records UTC sample time, response latency, HTTP result, selected fields, SHA-256 of response bytes and errors. A failed GET means **unobservable from that client**, not a dead process. A health snapshot older than 120 seconds is flagged. Tip-height differences are observational only. A single response from /api/last-finalized-block does not prove finality is advancing: compare successive samples from each responsive node.

Read-only checks **on each operator-controlled host** (Jim/operator; do not share service environment files, keys or tokens):

~~~sh
date -u
free -m
uptime
systemctl show rnode -p ActiveState -p SubState -p MainPID -p NRestarts -p MemoryCurrent
journalctl -k --since '2026-10-23 18:45:00 UTC' --until '2026-10-23 19:45:00 UTC' --no-pager |
  grep -Ei 'out of memory|oom-kill|killed process|memory cgroup'
~~~

Repeat the systemctl show query for the actual unit names of the other co-located validators (e.g., rnode-d / rnode-c). Collect per-PID RSS and cgroup memory.current / memory.events if supported, with UTC timestamps; protect validator keys and private deploy credentials. Preserve original logs **before** any reset, host resize, or binary replacement. A restart count or low free RAM is not itself proof of an OOM kill.

## 24 October follow-up: kernel OOM evidence from the B/C host

An authorized read-only inspection of the original Linux kernel journal on the shared B/C physical host found four paired OOM-kill and killed-process records involving rnode:

| UTC on 2026-10-23 | task_memcg service | Victim |
| --- | --- | --- |
| 18:37:41 | rnode.service | rnode |
| 18:46:21 | rnode-c.service | rnode |
| 18:52:02 | rnode.service | rnode |
| 22:15:49 | rnode-c.service | rnode |

**Established:** kernel OOM termination occurred on the B/C host, including before the 18:55–19:10 UTC reported API/fringe degradation. This corrects the earlier unverified OOM status for this host only.

**Not established:** whether those terminations were the exclusive cause of API/fringe stalls or any consensus-finality defect; the causal mechanism behind the Root-key observation; kernel OOM on the separate A/D host. Different event windows must not be conflated.

An operator-provided, bounded kernel-journal excerpt (1,644 bytes) was captured privately and pinned to SHA-256 dcc20c578a7e96dff65de68cd22934766dd0732ff0d1db27058ff24bac135e8f. Only sanitized UTC/service/process outcomes appear here; raw lines, process IDs, host credentials and private configuration are **not committed**. This digest identifies bytes but is not independent host attestation.

A later read-only inspection found approximately 18 MB available on B/C at 22:48 UTC with both services running. Current service cgroup memory.events counts were zero; counters observed after restarting services do not negate earlier kernel OOM evidence.

To classify authorized operator-held raw kernel text offline, run:

~~~sh
sha256sum incident420-hostB-kernel.log
python3 tools/incident-oom-evidence.py \
  --journal incident420-hostB-kernel.log \
  --host-label HostBC \
  --expect-sha256 REVIEWED_64_CHARACTER_SHA256 \
  --output incident420-hostB-oom-summary.json
python3 -m unittest discover -s tests -p 'test_incident_oom_evidence.py'
~~~

The parser never contacts validators, submits a deploy or embeds raw kernel log lines in the result. It fails closed on malformed and oversized inputs. No matching event in an incomplete supplied log means **not observed**, not no-OOM proof. Host A/D **kernel** evidence, independently authenticated raw Root reads and a disposable source-pinned replay remain outstanding. The [A/D private-artifact custody index](incident-420-host-ad-evidence.md) pins the available application and Nginx files without disclosing raw logs. Neither file substitutes for the missing A/D kernel journal. The offline classifier now recognizes \`rnode-d.service\` and refuses ambiguous same-window kernel kill attribution.

## Discriminating acceptance gates

1. **Host pressure:** replay the same signed, bounded ~40-deploy workload on an isolated, disposable copy of the exact deployed binary and data/genesis class. Compare measured memory, process exits, API readiness and time to service under low versus sufficient memory. Do not run load against the fragile public testnet.
2. **Fringe availability:** record exact request/response events on both sender and responder, matching source SHA, peer and anchor. If a reply is absent, distinguish a timed-out transport, a starved event loop, and refusal for an unavailable stored root. Preserve fail-closed behavior.
3. **Finalized-write integrity:** for each allegedly absent Root key, record deploy ID, inclusion block hash, whether that block is finalized, the finalized post-state, and reads from at least two independently responding validators at the **same finalized state**. A disagreement in same-pinned-state reads is a read-integrity witness. But unanimous agreement does not exclude a lost accepted write between finalized states (see K8/#280). Prove inclusion in an earlier finalized post-state, absence in a later finalized post-state, and the absence of legitimate rejection/removal before claiming accepted-finalized-write loss.
4. **Topology:** model a whole-host failure (two of four equal-stake validators) separately from a single-node failure. Adding an observer can offload some API work but does not add voting stake or repair a missing quorum.
5. **Deployment:** if the operator resizes hosts, collect pre- and post-change samples on the same binary and genesis before attributing improvement to memory. Any new-genesis transition or protocol upgrade needs separate maintainer approval and its own acceptance.

**Exit condition:** repeatably functioning reads and finality progress under the bounded application workload, successful ordinary and anchored fringe recovery on available nodes, a validated finalized-write invariant, and no unexplained process exits. If some gates cannot be run, retain #420 as open with the exact missing evidence.


## Root-key claim: offline discriminator (separate from host OOM)

The r-cash installer reports **17 deploys from 18:00:03 to 18:03:06 UTC**, before the severe-memory window at about 18:55–19:10 UTC. The B/C host OOM kills are real; they are **not yet a causal explanation for this earlier Root observation**. A `ProcessedWithSuccess` inclusion status is not evidence that every register effect remained accepted across later multi-parent merges.

The current [r-cash Root template](https://github.com/PatrickMockridge/r-cash/blob/main/contracts/build/deploy/v3/00_root.rho) inserts keys on the success path and exposes `entries.keys()`. The exact installed term/source commit remains to be pinned. Because Rholo's [multi-parent merge](https://github.com/rhovision-org/rholo/blob/71c6a513d9785e45c3fbb919125c3314c2d8bf6e/docs/src/node/block-merge.md) explicitly resolves conflicting deploy chains, `keys(A) ⊆ keys(B)` is **conditional on accepted effects**; justification ancestry alone is insufficient.

| Reported pin | Full hash | `component:governor` |
| --- | --- | --- |
| Height 835 | `3c12c6e02aec52b44c8f0953e1a60f9dedd5a1ed5ebc40d76a0df4f4200bd6d6` | present |
| Height 845 | `083e118904f2fdf27e87e03ee2523d7c805dd960a29c013b1d5220dbb8a51950` | absent |
| Height 1254, reported finalized | `dfed8a269ed35d893e64b61abb6fcfbe2c1112a38ea5cb449cdfeec1fd3e79db` | compare finalized post-state |

Do not use `bounds` at 830→845 as an equivalent witness: the published 830 post-state already lacks `bounds`.

With operator-supplied **raw** B/C `POST /api/v1/explore-deploy-by-block-hash` responses and block headers, run the classifier entirely offline:

~~~sh
python3 tools/incident-root-evidence.py root-witness-input.json \
  --output root-witness-classification.json
python3 -m unittest discover -s tests -p 'test_incident_root_evidence.py'
~~~

Input is one JSON object with `pair: {ancestor, descendant}` (full 64-character lowercase block hashes), `nodes: ["B","C"]`, `key: "component:governor"`, `reads` and `blocks`. Each read records `node`, `blockHash`, `request: {blockHash, usePreStateHash:false}`, and the raw `response` including `expr`. Each block record has `blockHash` and raw `response` (the actual `blockInfo` header). Include every intermediate header to verify a transitive justification path. No network calls are made; input is bounded at 4 MiB.

Output distinguishes invalid/missing reads from empty key sets; compares B/C at the same anchored state; evaluates whether the named key is present/absent; and labels ancestry as `VERIFIED`, `INCOMPLETE_HEADERS` or `NOT_IN_SUPPLIED_CLOSURE`. The output is SHA-256 bound to input. It **does not** claim finalized write loss or OOM causation. Use actual merge-rejection/acceptance provenance and finalized pre-/post-states for attribution.

A same-state B/C agreement can still coexist with a **unanimous regression**; the distinct earlier [#280 witness](https://github.com/rhovision-org/rholo/blob/71c6a513d9785e45c3fbb919125c3314c2d8bf6e/spec/audit/evidence/n280-merge-loses-a-write-results.md) demonstrates why. The other suggested features (capacity preflight, load shedding, peer-degraded signaling, multi-bootstrap failover, structured fringe refusal) require separately scoped changes and isolated acceptance. Do not substitute an unrelated root for an unavailable anchored fringe, do not conflate existing rate-limiting with memory backpressure, and do not use the unreproduced historical 0.25 MiB/block estimate to size the deployed binary.


## Disposition of every #420 proposal

| Proposal / concern | Present status | Required gate before claiming it is fixed |
| --- | --- | --- |
| Kernel memory exhaustion | **Confirmed on B/C host** by four matched kernel kill records; A/D unknown | Preserve host A/D window, kernel/cgroup records, PIDs/restarts and binary identity; explain any time offset |
| API timeout and load shedding | API degradation **reported**; causal attribution not isolated | Measure latency, in-flight memory/work, queue limits, accurate 429/503 behavior and recovery under bounded isolated load |
| Startup memory preflight | **Not implemented** by this diagnostic PR | Specify enforceable minima per topology and workload; default/override behavior and startup refusal tests; do not reuse historical replay-per-block estimate |
| Term size / deploy budget | Existing API deploy rate limit is **not** a memory budget | Establish measured resource accounting, bounded in-flight capacity and deterministic safe failure; size-only caps cannot substitute for state/merge costs |
| Peer degraded state / failover | **Design open** | Define authenticated readiness metrics, degraded semantics and compatibility; ensure a host-level failure does not masquerade as four independent failures |
| Fringe responder silence | Observer no-reply **reported**; source permits no reply for unavailable anchor or missing state | Correlate request ID, peer, requested anchor, block-store outcome and response logs on both ends; never substitute an unrelated root |
| Multi-bootstrap list | **Design open** | Specify deterministic candidate selection and retry/failover, negative response interoperability, no stale-state substitution, and isolated first-peer-silent test |
| Finality / tip spread alert | Tip heights differed; **no finalized-fork witness** | Capture timestamped per-node finalized (height, hash, state) progress; distinguish stale snapshots, stalled finality and conflicting finalized states |
| Append-only Root key loss | 835→845 key discontinuity **reported** on B/C; no authenticated raw JSON in this PR | Independently verify pinned reads, installed contract revision, full 835→845 justification chain, merged accepted/rejected deploy effects and finalized post-state membership |
| Register-then-read under burst | **Unclassified** | Disposable source-pinned replay of controlled sequential/parallel registers; two independent reads at a matched finalized state; record rejected deploys |
| 1 GB vs adequate-memory comparison | **Not run** | Isolated two-arm replay of a fixed workload with separate sources of measurement; no live testnet stress |
| Topology / 50% co-location risk | **Confirmed**: A+D share one host, B+C another, equal 250 stakes | Document 2 × 250 bonded stake in each failure domain; ensure operator sizing/failure-domain acceptance before public workload |
| Release / testnet changes | **Out of scope** | Separate maintainer authorization for restart, resize, genesis migration, protocol upgrade, or merging this Draft |

This table intentionally separates **observed failure**, **diagnosed mechanism**, **candidate guardrail**, and **tested fix**. The present PR provides evidence tooling, not the proposed runtime changes. Approval of this diagnostics PR would not close #420, authorize a new genesis or warrant a mainnet-readiness claim.
