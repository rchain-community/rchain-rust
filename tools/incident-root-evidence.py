#!/usr/bin/env python3
"""Offline Root-state witness verifier for Rholo #420.

Consumes operator-supplied raw API responses. Does not contact the network.
Ancestry alone is not accepted-write provenance; a failed read is not an empty set.
"""
from __future__ import annotations
import argparse
from collections import deque
import hashlib
import json
from pathlib import Path
import re
import sys

MAX_BYTES = 4 * 1024 * 1024
HASH = re.compile(r"^[0-9a-f]{64}$")
NODE = re.compile(r"^[A-Za-z][A-Za-z0-9_-]{0,31}$")
MAX_HEADERS = 2000

class WitnessError(ValueError):
    pass

def require_hash(value):
    if not isinstance(value, str) or not HASH.fullmatch(value):
        raise WitnessError("expected complete 64-digit lowercase block hash")
    return value

def extract_keys(raw):
    if not isinstance(raw, dict) or not isinstance(raw.get("expr"), list):
        raise WitnessError("missing or invalid explore response; never interpret as empty")
    expr = raw["expr"]
    if len(expr) != 1 or not isinstance(expr[0], dict):
        raise WitnessError("expected one Root.keys result")
    tup = expr[0].get("ExprTuple")
    data = tup.get("data") if isinstance(tup, dict) else None
    if not isinstance(data, list) or len(data) != 2:
        raise WitnessError("expected (true, key-set) tuple")
    flag = data[0].get("ExprBool") if isinstance(data[0], dict) else None
    if not isinstance(flag, dict) or flag.get("data") is not True:
        raise WitnessError("Root lookup/keys did not return true")
    s = data[1].get("ExprSet") if isinstance(data[1], dict) else None
    items = s.get("data") if isinstance(s, dict) else None
    if not isinstance(items, list):
        raise WitnessError("missing ExprSet (not a valid empty set)")
    keys = []
    for item in items:
        string = item.get("ExprString") if isinstance(item, dict) else None
        if not isinstance(string, dict) or not isinstance(string.get("data"), str):
            raise WitnessError("non-string Root key")
        keys.append(string["data"])
    if len(keys) != len(set(keys)):
        raise WitnessError("duplicate keys in ExprSet")
    return sorted(keys)

def read_records(entries):
    if not isinstance(entries, list) or len(entries) > 64:
        raise WitnessError("reads must be a bounded list")
    pairs = {}
    invalid_pairs = set()
    problems = []
    for idx, entry in enumerate(entries):
        try:
            if not isinstance(entry, dict):
                raise WitnessError("read record is not an object")
            node = entry.get("node")
            if not isinstance(node, str) or not NODE.fullmatch(node):
                raise WitnessError("invalid node label")
            h = require_hash(entry.get("blockHash"))
            request = entry.get("request")
            if not isinstance(request, dict) or request.get("blockHash") != h or request.get("usePreStateHash") is not False:
                raise WitnessError("request hash or post-state selector not verified")
            pair = (node, h)
            if pair in pairs or pair in invalid_pairs:
                pairs.pop(pair, None)
                invalid_pairs.add(pair)
                raise WitnessError("duplicate node/hash read; both records excluded")
            pairs[pair] = extract_keys(entry.get("response"))
        except WitnessError as exc:
            problems.append({"index": idx, "error": str(exc)})
    return pairs, problems

def headers_from(entries):
    if not isinstance(entries, list) or len(entries) > MAX_HEADERS:
        raise WitnessError("block records missing or unbounded")
    headers, errors = {}, []
    invalid_headers = set()
    for index, entry in enumerate(entries):
        try:
            if not isinstance(entry, dict):
                raise WitnessError("block record is not an object")
            requested = require_hash(entry.get("blockHash"))
            raw = entry.get("response")
            if not isinstance(raw, dict):
                raise WitnessError("missing raw block response")
            header = raw.get("blockInfo", raw)
            if not isinstance(header, dict) or header.get("blockHash") != requested:
                raise WitnessError("response blockHash does not match requested hash")
            just = header.get("justifications")
            if not isinstance(just, list) or any(not isinstance(j, str) or not HASH.fullmatch(j) for j in just):
                raise WitnessError("missing/invalid justifications")
            rejects = header.get("rejectedDeploys")
            if rejects is not None and (not isinstance(rejects, list) or any(not isinstance(x, str) for x in rejects)):
                raise WitnessError("invalid rejectedDeploys")
            state = header.get("postStateHash")
            if state is not None and not isinstance(state, str):
                raise WitnessError("invalid postStateHash")
            if requested in invalid_headers or (requested in headers and headers[requested] != header):
                headers.pop(requested, None)
                invalid_headers.add(requested)
                raise WitnessError("conflicting headers for same hash; hash excluded")
            headers[requested] = header
        except WitnessError as exc:
            errors.append({"index": index, "error": str(exc)})
    return headers, errors

def ancestry(headers, ancestor, descendant):
    """Return VERIFIED, NOT_IN_SUPPLIED_CLOSURE, or INCOMPLETE_HEADERS."""
    seen, queue, incomplete = set(), deque([descendant]), False
    while queue:
        h = queue.popleft()
        if h in seen:
            continue
        seen.add(h)
        if h == ancestor:
            return "VERIFIED"
        if h not in headers:
            incomplete = True
            continue
        queue.extend(headers[h]["justifications"])
        if len(seen) > MAX_HEADERS:
            return "INCOMPLETE_HEADERS"
    return "INCOMPLETE_HEADERS" if incomplete else "NOT_IN_SUPPLIED_CLOSURE"

def classify(raw_bytes, doc):
    if not isinstance(doc, dict):
        raise WitnessError("evidence manifest must be an object")
    pair = doc.get("pair")
    if not isinstance(pair, dict):
        raise WitnessError("missing pinned pair")
    ancestor = require_hash(pair.get("ancestor"))
    descendant = require_hash(pair.get("descendant"))
    if ancestor == descendant:
        raise WitnessError("ancestor and descendant must differ")
    nodes = doc.get("nodes")
    if not isinstance(nodes, list) or len(nodes) != 2 or len(set(nodes)) != 2 or any(
        not isinstance(n, str) or not NODE.fullmatch(n) for n in nodes
    ):
        raise WitnessError("exactly two distinct node labels required")
    wanted = doc.get("key")
    if not isinstance(wanted, str) or not wanted or len(wanted) > 256:
        raise WitnessError("invalid expected key")
    reads, problems = read_records(doc.get("reads", []))
    headers, header_errors = headers_from(doc.get("blocks", []))
    checks = {}
    for h, label in [(ancestor, "ancestor"), (descendant, "descendant")]:
        a, b = (reads.get((n, h)) for n in nodes)
        if a is None or b is None:
            checks[label] = {"status": "MISSING_VALID_READ"}
        else:
            checks[label] = {
                "status": "MATCH" if a == b else "CROSS_NODE_READ_MISMATCH",
                "key_present": {nodes[0]: wanted in a, nodes[1]: wanted in b},
                "key_counts": {nodes[0]: len(a), nodes[1]: len(b)},
            }
    coherent = all(checks[s]["status"] == "MATCH" for s in ("ancestor", "descendant"))
    discontinuity = (
        coherent
        and checks["ancestor"]["key_present"][nodes[0]] is True
        and checks["descendant"]["key_present"][nodes[0]] is False
    )
    return {
        "schema": 1,
        "input_sha256": hashlib.sha256(raw_bytes).hexdigest(),
        "offline_only": True,
        "pair": pair,
        "key": wanted,
        "cross_validator_checks": checks,
        "reported_discontinuity_reproduced_in_supplied_json": bool(discontinuity),
        "ancestry_from_supplied_headers": ancestry(headers, ancestor, descendant),
        "descendant_rejected_deploys": headers.get(descendant, {}).get("rejectedDeploys"),
        "read_errors": problems,
        "block_errors": header_errors,
        "finalized_effect_loss_proven": False,
        "kernel_oom_causation_proven": False,
        "explanation": (
            "Pinned state observations alone do not prove accepted/finalized write loss. "
            "Trace merge scope, rejected deploy chains, the actual installed contract and "
            "finalized post-state independently."
        ),
    }

def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("evidence_json", help="bounded operator-supplied raw response manifest")
    ap.add_argument("--output", help="write sanitized JSON verdict (default stdout)")
    args = ap.parse_args(argv)
    with Path(args.evidence_json).open("rb") as source:
        raw = source.read(MAX_BYTES + 1)
    if not raw or len(raw) > MAX_BYTES:
        ap.error("input must be 1 byte to 4 MiB")
    try:
        result = classify(raw, json.loads(raw))
    except (WitnessError, ValueError, TypeError) as exc:
        ap.error(str(exc))
    rendered = json.dumps(result, sort_keys=True, indent=2) + "\n"
    if args.output:
        Path(args.output).write_text(rendered, encoding="utf-8")
    else:
        sys.stdout.write(rendered)
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
