# Incident #420 — A/D host evidence custody and remaining gate

**Purpose:** Public, sanitized custody index for the A/D physical host. This record does **not** represent complete operator journal evidence, attribute API outages to a single crash, or certify recovery. Original host logs remain private.

## Source and coverage matrix

The following **private local copies** were inspected for filenames, byte counts and SHA-256. They are *not* included in this repository or in GitHub Actions artifacts.

| Artifact | Available | Bytes | SHA-256 | What it can establish |
| --- | --- | ---: | --- | --- |
| A RNode application log | Preserved privately | 10,184,908 | `dd556dbc3fa607ec8a7945bd83a289132be3ae2bf0e4829f23a80b05df1a1266` | Only what the A process logged within its captured coverage |
| A/D Nginx rotated access log | Preserved privately | 1,630,725 | `45db2232e602959b991db74bdcb6c182e5bd0f1cdc0ad7804f44aaa8b9d2a839` | Only traffic traversing this specific Nginx ingress |
| Redacted Nginx ingress summary | Preserved privately | 597 | `7931c9aa61055a32a3de18ccac117963d3f9fa698bcc47e6d41a1234bd9c019c` | A secondary derived account of the same ingress scope; not independent evidence |
| A/D `journalctl -k` / kernel OOM excerpt | **Not archived** | — | — | Cannot establish a source-pinned kernel OOM event on A/D |
| D RNode application log | **Not archived in this evidence bundle** | — | — | No independent D application-level event trace in this bundle |
| A/D `memory.events`, RSS and service restart counts during incident | **Not archived** | — | — | No per-cgroup high-water mark or restart timeline |

The earlier operator discussion mentions **possible A/D kernel-OOM events**. Without the original timestamped kernel lines, unit identity, matching killed-process records, an input digest and a custody record, this public evidence package cannot independently corroborate those reports. **Do not replace missing A/D raw evidence with synthetic test fixtures or B/C evidence.**

A read-only SSH reachability attempt from the current diagnostic client did not complete the SSH banner exchange. This is a connectivity limitation of the client path, not evidence that A/D was down or that it did not log the event. No validator process or configuration was modified.

## Independently confirmed comparator: B/C

The B/C physical host **does** have an operator-held 1,644-byte private kernel excerpt, SHA-256 `dcc20c578a7e96dff65de68cd22934766dd0732ff0d1db27058ff24bac135e8f`. Reprocessing it with `tools/incident-oom-evidence.py` yields **four** paired `oom-kill` / `Killed process` records for `rnode.service` and `rnode-c.service`. That is confirmation **on B/C only**. It does not prove any A/D event, API-request causality or Casper finality violation.

## Minimal operator-only collection gate for A/D

On the **authorized A/D host**, the operator should preserve the entire relevant kernel journal window before any rotation or repair. A minimal bounded **private** excerpt for the existing offline parser can be produced as follows, while retaining the original unfiltered journal in protected custody:

```sh
umask 077
journalctl -k -o short-iso-precise \
  --since '2026-10-23 18:00:00 UTC' \
  --until '2026-10-23 23:30:00 UTC' --no-pager \
  | grep -E 'oom-kill:|Out of memory: Killed process|Memory cgroup out of memory:' \
  > incident420-hostAD-kernel.private.log
sha256sum incident420-hostAD-kernel.private.log
python3 tools/incident-oom-evidence.py \
  --journal incident420-hostAD-kernel.private.log \
  --host-label HostAD \
  --expect-sha256 VERIFIED_INPUT_DIGEST \
  --output incident420-hostAD-oom-summary.private.json
```

The parser refuses empty or oversized evidence and never publishes raw lines. It now explicitly recognizes `rnode-d.service` as well as other reviewed A–D unit names and only pairs a kernel signal to a killed `rnode` when its PID (where available) and timestamp agree or an unambiguous temporal match exists. The output deliberately strips raw lines and PIDs. **No output is an attestation by the host operator on its own.**

Collect a separate authorized read-only service/cgroup snapshot for both `rnode.service` and `rnode-d.service`: unit `MainPID`/`NRestarts`, `MemoryCurrent`, cgroup `memory.events`, UTC host clock, exact deployed revision, source hostname identity under private custody, collection command, capture timestamps and hashes. Check journal persistence, time coverage, reboot/rotation boundaries, and whether the journal predates the current boot. Only then distinguish `VERIFIED OOM`, `NOT OBSERVED IN COMPLETE WINDOW` or `INSUFFICIENT EVIDENCE`.

Do not publish raw Nginx lines, IP-bearing application logs, private service environment, keys, privileged host access information or unredacted Root payloads. Host-level OOM evidence still does not identify which request caused exhaustion; only a separately synchronized response/transport trace could support that attribution.

## Review disposition

- **Ready for review:** bounded offline parser, strict matching with no PID disclosure, private-file custody hashes and separate coverage report.
- **Blocked by original evidence:** A/D kernel OOM attestation and full service-level fault timeline.
- **Independent blockers:** source-pinned Root merge replay, accepted/rejected deploy-chain accounting, isolated finality/restore campaigns and maintainer release approval.

This is a diagnostic evidence gate, not approval to merge, resize, restart, load or deploy validators.
