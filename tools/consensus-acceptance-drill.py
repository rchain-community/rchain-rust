#!/usr/bin/env python3
"""Measure finality and retention of one finalized write on a disposable devnet.

No quorum or consensus parameter is changed. The write is observed in finalized
post-state on every validator, then checked through two further epoch boundaries.
This is a finite execution witness, not a universal rollback-safety proof.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("devnet_fuzz", ROOT / "tools/devnet-fuzz.py")
fuzz = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fuzz)
PREFIX = os.environ.get("DEVNET_PREFIX", "acceptance")


def node_name(i):
    return f"{PREFIX}-bootstrap" if i == 0 else f"{PREFIX}-validator-{i}"


def get(i, path, body=None):
    return fuzz.http_json(f"http://127.0.0.1:{40403 + i * 1000}{path}",
                          "POST" if body is not None else "GET", body, timeout=10)[1]


def block_identity(value):
    if not isinstance(value, dict):
        raise ValueError("block response is not an object")
    height, block_hash = value.get("blockNumber"), value.get("blockHash")
    if type(height) is not int or height < 0:
        raise ValueError("block response has no nonnegative height")
    if not isinstance(block_hash, str) or len(block_hash) != 64:
        raise ValueError("block response has no 32-byte hash")
    bytes.fromhex(block_hash)
    return height, block_hash


def marker_present(response, expected):
    if not isinstance(response, dict) or not isinstance(response.get("expr"), list):
        raise ValueError("exploration did not return an expression list")
    values = response["expr"]
    return len(values) == 1 and values[0] == {"ExprInt": {"data": expected}}


def probe(i, name, block_hash):
    term = f'new return in {{ for (@v <- @{json.dumps(name)}) {{ return!(v) }} }}'
    return get(i, "/api/v1/explore-deploy-by-block-hash",
               {"term": term, "blockHash": block_hash, "usePreStateHash": False})


class Drill:
    def __init__(self, args):
        self.args = args
        self.out = Path(args.out)
        self.out.mkdir(parents=True, exist_ok=True)
        self.events = self.out / "observations.jsonl"
        self.marker = "acceptance:" + uuid.uuid4().hex
        self.value = 42
        self.next_drive = 0

    def record(self, **event):
        with self.events.open("a") as f:
            f.write(json.dumps({"utc": time.time(), **event}) + "\n")

    def snapshot(self):
        results = []
        for i in range(self.args.validators):
            status = get(i, "/api/v1/status")
            finalized = get(i, "/api/last-finalized-block")
            self.record(node=i, status=status, finalized=finalized)
            results.append((status, finalized))
        return results

    def deploy(self, term):
        status = get(0, "/api/v1/status")
        result = fuzz.signed_deploy(node_name(0), term, status["latestBlockNumber"], status["shardId"])
        self.record(deploy_term=term, exit_code=result.returncode,
                    stdout=result.stdout, stderr=result.stderr)
        if result.returncode:
            raise RuntimeError("signed deploy submission failed; see observations.jsonl")

    def drive(self):
        if time.monotonic() >= self.next_drive:
            self.deploy("Nil")
            self.next_drive = time.monotonic() + 5

    def wait(self, description, condition, drive=True):
        deadline = time.monotonic() + self.args.window
        last_error = None
        while time.monotonic() < deadline:
            try:
                value = condition()
                if value is not None:
                    print(f"PASS {description}: {value}", flush=True)
                    return value
            except (OSError, ValueError) as error:
                last_error = repr(error)
                self.record(wait=description, read_error=last_error)
            if drive:
                self.drive()
            time.sleep(2)
        raise RuntimeError(f"{description} did not pass within {self.args.window}s; last read error={last_error}")

    def finalized_marker(self, min_height):
        identities = []
        for i, (_, finalized) in enumerate(self.snapshot()):
            try:
                height, block_hash = block_identity(finalized)
            except ValueError:
                return None
            if height < min_height:
                return None
            answer = probe(i, self.marker, block_hash)
            self.record(node=i, finalized_marker_at=block_hash, response=answer)
            if not marker_present(answer, self.value):
                raise RuntimeError(f"validator {i} lost or changed the marker in finalized post-state")
            identities.append({"node": i, "height": height, "hash": block_hash})
        return identities

    def run(self):
        self.record(tree=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
                    validators=self.args.validators, epoch_length=10, window=self.args.window,
                    marker=self.marker, expected=self.value)
        self.record(binary_sha256=subprocess.check_output(
            ["docker", "exec", node_name(0), "sha256sum", "/usr/local/bin/rnode"], text=True).strip())
        for i in range(self.args.validators):
            inspect = subprocess.check_output(["docker", "inspect", node_name(i)], text=True)
            (self.out / f"container-{i}.json").write_text(inspect)

        # An absent marker must be absent before the write; empty/malformed API
        # responses are not accepted as evidence that the marker is present.
        for i in range(self.args.validators):
            blocks = get(i, "/api/blocks/1")
            if not isinstance(blocks, list) or not blocks:
                raise RuntimeError(f"validator {i} has no anchor block")
            _, block_hash = block_identity(blocks[0])
            answer = probe(i, self.marker, block_hash)
            self.record(node=i, absent_marker_at=block_hash, response=answer)
            if not isinstance(answer, dict) or answer.get("expr") != []:
                raise RuntimeError("the absent-marker control did not return an empty expression list")

        self.deploy(f'@{json.dumps(self.marker)}!({self.value})')

        def visible():
            tips = []
            for i in range(self.args.validators):
                blocks = get(i, "/api/blocks/1")
                if not isinstance(blocks, list) or not blocks:
                    return None
                height, block_hash = block_identity(blocks[0])
                answer = probe(i, self.marker, block_hash)
                self.record(node=i, latest_marker_at=block_hash, response=answer)
                if not marker_present(answer, self.value):
                    return None
                tips.append(height)
            return max(tips)

        observed_height = self.wait("marker observed on every validator", visible)
        initial = self.wait("marker present in finalized state on every validator",
                            lambda: self.finalized_marker(observed_height))
        first_height = max(x["height"] for x in initial)
        # Cross two *further* boundaries after the finalized-write observation.
        target = (first_height // 10 + 2) * 10 + 4
        final = self.wait("finalized marker retained through two further epoch boundaries",
                          lambda: self.finalized_marker(target))
        summary = {"result": "pass", "validators": self.args.validators,
                   "initial_finalized": initial, "final_finalized": final,
                   "target_height": target,
                   "boundary": "finite devnet witness; no universal safety theorem"}
        (self.out / "result.json").write_text(json.dumps(summary, indent=2) + "\n")
        print(json.dumps(summary), flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--validators", type=int, choices=[4, 8], required=True)
    parser.add_argument("--window", type=int, default=240)
    parser.add_argument("--out", required=True)
    args = parser.parse_args()
    if not 30 <= args.window <= 600:
        parser.error("window must be between 30 and 600 seconds")
    drill = Drill(args)
    try:
        drill.run()
    except Exception as error:
        (drill.out / "result.json").write_text(json.dumps({"result": "fail", "reason": str(error)}) + "\n")
        raise


if __name__ == "__main__":
    main()
