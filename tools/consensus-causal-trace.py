#!/usr/bin/env python3
"""Capture and compare accepted DAG views; diagnostics, not consensus proof."""
import hashlib
import json
from pathlib import Path
import re
import subprocess
import time


def hex_value(value, size, label):
    if not isinstance(value, str) or len(value) != size * 2:
        raise ValueError(f"invalid {label}")
    bytes.fromhex(value)
    return value.lower()


def index_blocks(blocks):
    if not isinstance(blocks, list):
        raise ValueError("block inventory is not a list")
    result = {}
    for block in blocks:
        key = hex_value(block.get("blockHash"), 32, "block hash")
        hex_value(block.get("sender"), 65, "validator identity")
        if type(block.get("blockNumber")) is not int or block["blockNumber"] < 0:
            raise ValueError("invalid block height")
        if type(block.get("seqNum")) is not int or block["seqNum"] < 0:
            raise ValueError("invalid sender sequence")
        hex_value(block.get("preStateHash"), 32, "pre-state hash")
        hex_value(block.get("postStateHash"), 32, "post-state hash")
        parents = block.get("justifications")
        if not isinstance(parents, list):
            raise ValueError("missing justifications")
        for parent in parents:
            hex_value(parent, 32, "justification")
        if len(set(parents)) != len(parents):
            raise ValueError("duplicate justification")
        if key in result and result[key] != block:
            raise ValueError("contradictory observations for one block hash")
        result[key] = block
    return result


def unique_bonds(rows):
    if not isinstance(rows, list) or not rows:
        raise ValueError("missing bonded set")
    bonds = {}
    for row in rows:
        sender = hex_value(row.get("validator"), 65, "bonded identity")
        weight = row.get("stake")
        if type(weight) is not int or weight < 0:
            raise ValueError("invalid stake")
        if sender in bonds:
            raise ValueError("duplicate bonded identity")
        bonds[sender] = weight
    return bonds


def summarize(blocks):
    index = index_blocks(blocks)
    if not index:
        raise ValueError("empty DAG inventory")
    latest = {}
    for block in index.values():
        sender = block["sender"]
        if sender not in latest or block["seqNum"] > latest[sender]["seqNum"]:
            latest[sender] = block
    tips = sorted((b for b in index.values()
                   if b["blockNumber"] == max(x["blockNumber"] for x in index.values())),
                  key=lambda b: b["blockHash"])
    missing = sorted({p for b in index.values() for p in b["justifications"] if p not in index})
    # Keep each tip's bonded context separate. A union across epochs would be
    # misleading, and these weights are never treated as signed quorum evidence.
    contexts = []
    for tip in tips:
        bonds = unique_bonds(tip["bonds"])
        contexts.append({"block_hash": tip["blockHash"], "bonds": bonds,
                         "total_stake": sum(bonds.values())})
    return {"accepted_ids": sorted(index), "missing_dependencies": missing,
            "latest_by_sender": {s: {"hash": b["blockHash"], "height": b["blockNumber"],
                                      "seq": b["seqNum"]} for s, b in sorted(latest.items())},
            "tip_bond_contexts": contexts,
            "complete_inventory": not missing,
            "boundary": "accepted DAG observations; not proof of finality or stake support"}


def compare(views):
    sets = {str(i): set(v["accepted_ids"]) for i, v in views.items()}
    union = set().union(*sets.values()) if sets else set()
    return {"nodes": len(sets), "union_ids": sorted(union),
            "missing_by_node": {i: sorted(union - ids) for i, ids in sets.items()},
            "same_accepted_ids": bool(sets) and all(ids == union for ids in sets.values()),
            "boundary": "same accepted IDs does not establish finality or independently signed quorum"}


def decisions(log):
    result = []
    for line in log.splitlines():
        match = re.search(r"attestation at tip (\d+): (licensed|withheld).*?round=\[([^]]*)\]", line)
        if match:
            fields = dict(re.findall(r"(\w+)=(true|false|\d+)", line))
            result.append({"height": int(match[1]), "decision": match[2],
                           "round_sender_prefixes": match[3].split(), "inputs": fields,
                           "source_line": line})
    return result


def verify_raw(block, raw):
    if raw.get("blockHash") != block["blockHash"] or raw.get("sender") != block["sender"]:
        raise ValueError("raw block identity disagrees with inventory")
    encoded = raw.get("encoded")
    if not isinstance(encoded, str):
        raise ValueError("raw block has no preimage")
    digest = hashlib.blake2b(bytes.fromhex(encoded), digest_size=32).hexdigest()
    if digest != block["blockHash"]:
        raise ValueError("raw block preimage hash mismatch")


def capture(run, get, node_name):
    out = run.out / "causal"
    out.mkdir(exist_ok=True)
    views, errors = {}, {}
    manifest = {"utc": time.time(), "source_sha": subprocess.check_output(
        ["git", "rev-parse", "HEAD"], text=True).strip(),
        "validators": run.args.validators, "nodes": {},
        "unavailable": ["internal round ID", "signature verification of captured raw bytes",
                        "candidate fringe internals; use gate verdict in operator status"]}
    for i in range(run.args.validators):
        node = node_name(i)
        try:
            status = get(i, "/api/v1/status/operator", admin=True)
            height = status["latestBlockNumber"]
            if type(height) is not int or not 0 <= height <= 256:
                raise ValueError("capture height outside bounded 0..256 range")
            # Height-range reads include accepted unfinalized blocks. Do not
            # substitute a wallet/name read that now selects finalized state.
            blocks = get(i, f"/api/blocks/0/{height}")
            index = index_blocks(blocks)
            raw_blocks = {}
            for key, block in sorted(index.items()):
                raw = get(i, f"/api/v1/block/{key}/raw")
                verify_raw(block, raw)
                raw_blocks[key] = raw
            log = subprocess.run(["docker", "logs", node], text=True,
                                 capture_output=True, timeout=15)
            if log.returncode:
                raise RuntimeError("container log capture failed")
            text = log.stdout + log.stderr
            (out / f"node-{i}.log").write_text(text)
            binary = subprocess.check_output(
                ["docker", "exec", node, "sha256sum", "/usr/local/bin/rnode"],
                text=True, timeout=10).strip().split()[0]
            state = subprocess.check_output(["docker", "inspect", "--format",
                                             "{{json .State}}", node], text=True, timeout=10)
            view = summarize(blocks)
            views[i] = view
            payload = {"node": i, "operator_status": status, "blocks": blocks,
                       "raw_blocks": raw_blocks, "summary": view,
                       "attestation_decisions": decisions(text)}
            (out / f"node-{i}.json").write_text(json.dumps(payload, indent=2) + "\n")
            manifest["nodes"][str(i)] = {"binary_sha256": binary, "container_state": json.loads(state)}
        except Exception as error:
            errors[str(i)] = f"{type(error).__name__}: {error}"
    comparison = compare(views)
    comparison.update({"errors": errors,
                       "complete_capture": not errors and len(views) == run.args.validators})
    (out / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    (out / "comparison.json").write_text(json.dumps(comparison, indent=2) + "\n")
    print("CAUSAL_CAPTURE " + json.dumps(comparison), flush=True)
    # The existing runner retains logs but has no artifact-upload step. Keep
    # each capture there too, with delimiters for mechanical extraction.
    for path in sorted(out.glob("*.json")):
        print(f"=== BEGIN {path} ===", flush=True)
        print(path.read_text(), flush=True)
        print(f"=== END {path} ===", flush=True)
    if not comparison["complete_capture"]:
        raise RuntimeError("causal capture is incomplete; see causal/comparison.json")
