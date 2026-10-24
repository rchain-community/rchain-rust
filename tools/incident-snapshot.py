#!/usr/bin/env python3
"""Bounded, read-only incident snapshot for Rholo #420.

Only GET requests are made. This is an observation aid, not a finality or
consensus acceptance test. Never use it as a validator health oracle.
"""

import argparse
import concurrent.futures
import datetime as dt
import hashlib
import json
import re
import socket
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

STATUS_FIELDS = (
    "latestBlockNumber", "peers", "nodes", "version", "finalityStall",
    "finalityStalled", "finalityStallEpisodes", "finalityCountersDurable",
    "sync", "poisonRecoveries", "tasksPanicked", "tasksExited",
)
FRINGE_FIELDS = ("blockNumber", "blockHash", "sender", "postStateHash")
HEALTH_FIELDS = (
    "ts", "host", "ok", "rnode_unit", "api_reachable", "blocks",
    "blocks_since_last_tick", "peers", "nodes", "finalized_fringe",
    "autopropose", "mem_available_mb", "disk_free_mb",
)
MAX_BYTES = 256 * 1024
LABEL = re.compile(r"^[a-zA-Z][a-zA-Z0-9_-]{0,31}$")


def parse_mapping(entries, purpose):
    result = {}
    for entry in entries:
        label, sep, base = entry.partition("=")
        if not sep or not LABEL.fullmatch(label) or label in result:
            raise ValueError(f"invalid or duplicate {purpose} LABEL=URL: {entry}")
        u = urllib.parse.urlsplit(base)
        if (
            u.scheme not in ("http", "https")
            or not u.hostname
            or u.username is not None
            or u.password is not None
            or u.query
            or u.fragment
            or not (u.path in ("", "/") or purpose == "health")
        ):
            raise ValueError(f"invalid {purpose} URL for {label}")
        if purpose == "health" and not u.path.endswith("/health"):
            raise ValueError(f"health URL must end in /health: {label}")
        result[label] = base.rstrip("/")
    return result


def get_json(url, timeout, fields):
    start = time.monotonic()
    request = urllib.request.Request(
        url, headers={"Accept": "application/json", "User-Agent": "rholo-incident420/1.0"},
        method="GET",
    )
    out = {"url": url, "ok": False}
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            out["status"] = response.status
            payload = response.read(MAX_BYTES + 1)
        out["bytes"] = len(payload)
        if len(payload) > MAX_BYTES:
            raise ValueError(f"response exceeds {MAX_BYTES} bytes")
        out["sha256"] = hashlib.sha256(payload).hexdigest()
        value = json.loads(payload)
        if not isinstance(value, dict):
            raise ValueError("response JSON is not an object")
        out["data"] = {k: value[k] for k in fields if k in value}
        out["ok"] = True
    except (urllib.error.URLError, socket.timeout, TimeoutError, ValueError,
            json.JSONDecodeError, OSError) as exc:
        out["error"] = str(exc)[:240]
        out["error_type"] = type(exc).__name__
    finally:
        out["elapsed_ms"] = round((time.monotonic() - start) * 1000)
    return out


def collect(nodes, health, timeout, clock=None):
    now = clock or dt.datetime.now(dt.timezone.utc)
    jobs = []
    for label, base in nodes.items():
        jobs.append((label, "status", f"{base}/api/status", STATUS_FIELDS))
        jobs.append((label, "finality", f"{base}/api/last-finalized-block", FRINGE_FIELDS))
    for label, url in health.items():
        jobs.append((label, "health", url, HEALTH_FIELDS))
    results = {label: {} for label in nodes.keys() | health.keys()}
    with concurrent.futures.ThreadPoolExecutor(max_workers=min(12, len(jobs))) as executor:
        futures = {
            executor.submit(get_json, url, timeout, fields): (label, kind)
            for label, kind, url, fields in jobs
        }
        for future in concurrent.futures.as_completed(futures):
            label, kind = futures[future]
            results[label][kind] = future.result()
    for label in results:
        results[label] = {k: results[label][k] for k in ("status", "finality", "health")
                          if k in results[label]}
    tips = {}
    warnings = []
    for label, item in sorted(results.items()):
        status = item.get("status")
        finality = item.get("finality")
        health_item = item.get("health")
        if status and not status["ok"]:
            warnings.append(f"{label}: status unavailable; no conclusion about node process")
        if finality and not finality["ok"]:
            warnings.append(f"{label}: finalized-block endpoint unavailable")
        if status and status["ok"]:
            tip = status["data"].get("latestBlockNumber")
            if type(tip) is int:
                tips[label] = tip
            else:
                warnings.append(f"{label}: latest height missing/non-integer")
        if status and status["ok"] and finality and finality["ok"]:
            tip = status["data"].get("latestBlockNumber")
            lfb = finality["data"].get("blockNumber")
            if type(tip) is int and type(lfb) is int:
                if lfb > tip:
                    warnings.append(f"{label}: finalized height exceeds local tip; investigate")
        if health_item and health_item["ok"]:
            ts = health_item["data"].get("ts")
            if type(ts) in (int, float) and (now.timestamp() - ts > 120):
                warnings.append(f"{label}: health snapshot older than 120 seconds")
            # Note: an ok=true snapshot is not an assurance of adequate memory.
    if len(tips) > 1 and min(tips.values()) != max(tips.values()):
        warnings.append(
            f"observed height spread={max(tips.values())-min(tips.values())}; "
            "not evidence of a divergent finalized state"
        )
    return {
        "sample_utc": now.isoformat(),
        "read_only": True,
        "sources": results,
        "diagnostics": {"reachable_status": sum(
            bool(it.get("status", {}).get("ok")) for it in results.values()),
            "observed_tip_heights": tips,
            "warnings": warnings},
        "scope": "Observation only: not a proof of OOM, rollback, consensus safety or recovery.",
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--node", action="append", default=[], metavar="LABEL=HTTP_URL",
                        help="Read status and last-finalized-block at the node URL")
    parser.add_argument("--health", action="append", default=[], metavar="LABEL=HEALTH_URL",
                        help="Read the separately refreshed /health snapshot")
    parser.add_argument("--timeout", type=float, default=4.0, help="Seconds per GET (default 4)")
    parser.add_argument("--samples", type=int, default=1, help="Bounded samples (default 1)")
    parser.add_argument("--interval", type=float, default=15.0, help="Seconds between samples")
    parser.add_argument("--output", help="Append JSONL instead of printing it")
    args = parser.parse_args(argv)
    try:
        nodes = parse_mapping(args.node, "node")
        health = parse_mapping(args.health, "health")
    except ValueError as error:
        parser.error(str(error))
    if not nodes and not health:
        parser.error("at least one --node or --health is required")
    if not (0.1 <= args.timeout <= 30):
        parser.error("--timeout must be between 0.1 and 30 seconds")
    if not (1 <= args.samples <= 20) or not (0 <= args.interval <= 3600):
        parser.error("--samples must be 1..20 and --interval must be 0..3600")
    if args.output:
        with open(args.output, "a", encoding="utf-8") as out:
            for number in range(args.samples):
                if number:
                    time.sleep(args.interval)
                out.write(json.dumps(collect(nodes, health, args.timeout), sort_keys=True) + "\n")
                out.flush()
    else:
        for number in range(args.samples):
            if number:
                time.sleep(args.interval)
            print(json.dumps(collect(nodes, health, args.timeout), sort_keys=True), flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
