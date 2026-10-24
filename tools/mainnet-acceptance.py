#!/usr/bin/env python3
"""Offline, fail-closed evidence-contract evaluation for proposed mainnet gates.

A PASS is evidence-contract completeness, NOT independently authenticated proof
of runtime behaviour or permission to release.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import re
import sys

HEX40 = re.compile(r"^[0-9a-f]{40}$")
HEX64 = re.compile(r"^[0-9a-f]{64}$")
UTC_Z = re.compile(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{1,6})?Z$")
TOKEN = re.compile(r"^[A-Za-z0-9._-]{1,100}$")
LIMIT = 256 * 1024

# All checks are affirmative statements. A missing/false assertion can never pass.
CRITERIA = {
    "M1": {
        "supported_capacity": ("supported_tier_declared", "memory_headroom_measured", "no_unexpected_oom"),
        "failure_domains": ("hosts_independent", "host_loss_within_declared_quorum_model"),
        "restart_replay": ("replay_bounded", "no_restart_loop"),
    },
    "M2": {
        "http_ingress": ("http_concurrency_bounded", "http_request_bytes_bounded", "http_overload_explicit"),
        "grpc_ingress": ("grpc_concurrency_bounded", "grpc_request_bytes_bounded", "grpc_overload_explicit"),
        "overload_recovery": ("queues_bounded", "no_ambiguous_acceptance", "service_recovers"),
    },
    "M3": {
        "external_observer": ("advertised_endpoint_routable", "fringe_received", "approved_state_applied"),
        "invalid_peer_negative": ("loopback_refused", "refusal_observable"),
        "bootstrap_failover": ("alternate_peer_tried", "approved_root_preserved"),
    },
    "M4": {
        "n4_finality": ("deploy_finalized", "production_bounded", "finalized_roots_consistent"),
        "n8_finality": ("deploy_finalized", "production_bounded", "finalized_roots_consistent"),
        "partition_rejoin": ("partition_exercised", "rejoin_completed", "no_safety_violation"),
        "host_loss_recovery": ("host_loss_exercised", "within_declared_fault_model", "recovery_observed"),
    },
    "M5": {
        "merge_840_841": ("all_parent_events_pinned", "merged_state_recomputed", "state_hash_matches"),
        "merge_accounting": ("final_scope_rejections_recorded", "accepted_effects_accounted", "unapplied_writes_accounted"),
        "hot_cell_regression": ("sequential_case_checked", "parallel_case_checked", "accepted_effects_preserved"),
    },
    "M6": {
        "release_provenance": ("source_binary_pinned", "environment_pinned", "bond_set_pinned"),
        "audit_trail": ("raw_results_preserved", "evidence_digests_verified", "run_commands_recorded"),
        "independent_review": ("reviewer_independent", "review_traceable", "exceptions_disposed"),
    },
}


class EvidenceError(ValueError):
    pass


def _no_duplicate_keys(pairs: list[tuple[str, object]]) -> dict:
    obj = {}
    for key, value in pairs:
        if key in obj:
            raise EvidenceError("duplicate JSON member")
        obj[key] = value
    return obj


def _reject_constant(_value: str) -> None:
    raise EvidenceError("non-standard JSON numeric value")


def load_json(path: Path) -> tuple[dict, str]:
    """Read bounded bytes once; parse and digest exactly the same payload."""
    if path.is_symlink() or not path.is_file():
        raise EvidenceError("evidence must be a regular file, not a symlink")
    try:
        with path.open("rb") as handle:
            raw = handle.read(LIMIT + 1)
        if not raw or len(raw) > LIMIT:
            raise EvidenceError("JSON file must be 1 byte to 256 KiB")
        obj = json.loads(
            raw.decode("utf-8"),
            object_pairs_hook=_no_duplicate_keys,
            parse_constant=_reject_constant,
        )
    except (OSError, UnicodeError, json.JSONDecodeError, RecursionError) as err:
        raise EvidenceError(f"unreadable/invalid JSON: {type(err).__name__}") from err
    if not isinstance(obj, dict):
        raise EvidenceError("JSON root must be an object")
    return obj, hashlib.sha256(raw).hexdigest()

def valid_utc(value: object) -> datetime:
    if not isinstance(value, str) or UTC_Z.fullmatch(value) is None:
        raise EvidenceError("timestamps must be UTC RFC3339 with seconds and Z")
    try:
        d = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as e:
        raise EvidenceError("invalid timestamp") from e
    if d.utcoffset() != timezone.utc.utcoffset(d):
        raise EvidenceError("timestamp is not UTC")
    return d


def resolve_under(base: Path, name: object) -> Path:
    if not isinstance(name, str) or not name or name.startswith("/") or "\\" in name:
        raise EvidenceError("artifact path must be relative")
    parts = Path(name).parts
    if ".." in parts or "." in parts:
        raise EvidenceError("artifact path traversal denied")
    candidate = base.joinpath(*parts)
    if candidate.is_symlink() or not candidate.resolve().is_relative_to(base.resolve()):
        raise EvidenceError("artifact path escapes evidence directory")
    return candidate


def verify_entry(root: Path, item: object, gate: str, criterion: str, candidate: str) -> tuple[str, str]:
    if not isinstance(item, dict) or set(item) != {"path", "sha256"}:
        return "BLOCKED", "missing/invalid evidence descriptor"
    digest = item.get("sha256")
    if not isinstance(digest, str) or not HEX64.fullmatch(digest):
        return "BLOCKED", "missing/invalid evidence SHA-256"
    try:
        path = resolve_under(root, item["path"])
        evidence, real_digest = load_json(path)
        if real_digest != digest:
            return "BLOCKED", "evidence digest mismatch"
        if set(evidence) != {
            "gate", "criterion", "candidate_sha", "run_id", "command",
            "environment", "started_at", "finished_at", "checks",
        }:
            return "BLOCKED", "unexpected or missing evidence fields"
        if evidence.get("gate") != gate or evidence.get("criterion") != criterion:
            return "BLOCKED", "evidence gate/criterion mismatch"
        if evidence.get("candidate_sha") != candidate:
            return "BLOCKED", "candidate revision mismatch"
        if not isinstance(evidence.get("run_id"), str) or not TOKEN.fullmatch(evidence["run_id"]):
            return "BLOCKED", "missing deterministic run identifier"
        if not isinstance(evidence.get("command"), str) or not evidence["command"].strip():
            return "BLOCKED", "missing reproduction command"
        if not isinstance(evidence.get("environment"), str) or not evidence["environment"].strip():
            return "BLOCKED", "missing execution environment"
        start, end = valid_utc(evidence.get("started_at")), valid_utc(evidence.get("finished_at"))
        if start > end:
            return "BLOCKED", "evidence timestamps reversed"
        checks = evidence.get("checks")
        expected = set(CRITERIA[gate][criterion])
        if not isinstance(checks, dict) or set(checks) != expected:
            return "BLOCKED", "missing or unexpected required checks"
        if any(type(v) is not bool for v in checks.values()):
            return "BLOCKED", "check outcomes must be booleans"
        failed = sorted(k for k, v in checks.items() if v is False)
        if failed:
            return "FAIL", "failed checks: " + ", ".join(failed)
        return "PASS", "digest/metadata/check contract satisfied (not independent certification)"
    except EvidenceError as e:
        return "BLOCKED", str(e)
    except OSError:
        return "BLOCKED", "unreadable evidence file"


def evaluate(manifest_path: Path) -> dict:
    manifest, _manifest_sha256 = load_json(manifest_path)
    if set(manifest) != {"schema_version", "candidate_sha", "gates"}:
        raise EvidenceError("unexpected or missing manifest fields")
    if type(manifest.get("schema_version")) is not int or manifest["schema_version"] != 1:
        raise EvidenceError("unsupported manifest version")
    candidate = manifest.get("candidate_sha")
    if not isinstance(candidate, str) or not HEX40.fullmatch(candidate):
        raise EvidenceError("candidate_sha must be a 40-character lowercase Git SHA")
    gates = manifest.get("gates")
    if not isinstance(gates, dict) or set(gates) != set(CRITERIA):
        raise EvidenceError("manifest must list exactly M1-M6")
    evidence_root = manifest_path.parent
    output = {}
    for gate, expectations in CRITERIA.items():
        entries = gates[gate]
        if not isinstance(entries, list):
            raise EvidenceError(f"{gate} evidence must be a list")
        found: dict[str, list] = {}
        for entry in entries:
            if not isinstance(entry, dict) or set(entry) != {"criterion", "path", "sha256"}:
                raise EvidenceError(f"{gate}: invalid evidence entry")
            criterion = entry["criterion"]
            if not isinstance(criterion, str) or criterion not in expectations:
                raise EvidenceError(f"{gate}: unknown criterion")
            found.setdefault(criterion, []).append({"path": entry["path"], "sha256": entry["sha256"]})
        results = {}
        for criterion in expectations:
            hits = found.get(criterion, [])
            if not hits:
                status, reason = "BLOCKED", "no evidence"
            elif len(hits) != 1:
                status, reason = "BLOCKED", "duplicate evidence for criterion"
            else:
                status, reason = verify_entry(evidence_root, hits[0], gate, criterion, candidate)
            results[criterion] = {"status": status, "reason": reason}
        statuses = [result["status"] for result in results.values()]
        gate_status = "FAIL" if "FAIL" in statuses else "BLOCKED" if "BLOCKED" in statuses else "PASS"
        output[gate] = {"status": gate_status, "criteria": results}
    all_passed = all(entry["status"] == "PASS" for entry in output.values())
    return {
        "schema_version": 1,
        "candidate_sha": candidate,
        "gates": output,
        "release_decision": "PENDING_MAINTAINER_SIGNOFF" if all_passed else "BLOCKED",
        "certification": "NOT_CERTIFIED",
        "warning": "A PASS validates only supplied evidence contracts, never independent authenticity or mainnet approval.",
    }


def markdown(report: dict) -> str:
    lines = [
        "# Proposed mainnet acceptance evidence",
        "",
        f"Candidate revision: `{report['candidate_sha']}`",
        f"Release decision: **{report['release_decision']}** — **{report['certification']}**",
        "",
        "| Gate | Status | Passing criteria | Total |",
        "| --- | --- | ---: | ---: |",
    ]
    for key, gate in report["gates"].items():
        n = sum(x["status"] == "PASS" for x in gate["criteria"].values())
        lines.append(f"| {key} | **{gate['status']}** | {n} | {len(gate['criteria'])} |")
    lines.extend(["", "## Criteria and blockers", ""])
    for key, gate in report["gates"].items():
        lines.append(f"### {key} — {gate['status']}")
        for name, criterion in gate["criteria"].items():
            lines.append(f"- **{name}**: {criterion['status']} — {criterion['reason']}")
        lines.append("")
    lines.extend(["**This is not an authorization to merge, deploy, or declare mainnet readiness.**", ""])
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--json-out", type=Path)
    parser.add_argument("--md-out", type=Path)
    parser.add_argument("--strict", action="store_true", help="exit 2 unless all evidence contracts pass")
    args = parser.parse_args()
    try:
        report = evaluate(args.manifest)
    except (EvidenceError, OSError) as e:
        print(f"INVALID MANIFEST: {e}", file=sys.stderr)
        return 1
    serialized = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if args.json_out:
        args.json_out.parent.mkdir(parents=True, exist_ok=True)
        args.json_out.write_text(serialized, encoding="utf-8")
    if args.md_out:
        args.md_out.parent.mkdir(parents=True, exist_ok=True)
        args.md_out.write_text(markdown(report), encoding="utf-8")
    print(" ".join(f"{key}={value['status']}" for key, value in report["gates"].items()))
    print(f"release_decision={report['release_decision']} certification={report['certification']}")
    return 2 if args.strict and report["release_decision"] != "PENDING_MAINTAINER_SIGNOFF" else 0


if __name__ == "__main__":
    raise SystemExit(main())
