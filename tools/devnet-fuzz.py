#!/usr/bin/env python3
"""Devnet fuzz harness — robustness + determinism.

Drives a running Docker devnet (started by tools/devnet.sh) and asserts:

  * robustness  — random, syntactically-valid rholang terms sent through the reducer
                   (POST /api/v1/explore-deploy, which forks a throwaway runtime) never crash,
                   hang, or 5xx the node; and signed deploys through the real block path
                   (docker exec rnode deploy) keep every validator healthy;
  * determinism — every validator stays in lock-step (latestBlockNumber within a small spread).
                   The equal-stake devnet never reaches >2/3 finality, so content-hash determinism
                   is covered in-process (casper/tests/determinism.rs + scheduler.rs).

Stdlib only (urllib + json + subprocess). Run against an already-up devnet:

    tools/devnet.sh up --validators 3
    python3 tools/devnet-fuzz.py --validators 3 --seed 1 --iterations 100
"""

import argparse
import json
import random
import subprocess
import sys
import time
import urllib.error
import urllib.request

BOOTSTRAP = "devnet-bootstrap"
HTTP_BASE = 40403   # host HTTP port for validator 0; validator i is HTTP_BASE + 1000*i

# Max block-number spread (latestBlockNumber) that still counts as "in lock-step". The equal-stake
# devnet never reaches finality, so this convergence is the network-level determinism signal.
LOCKSTEP_TOLERANCE = 3

# Throwaway devnet deployer key (validator[0]'s private key, see tools/devnet.sh). Dev-only.
DEPLOYER_PRIV = "a68a6e6cca30f81bd24a719f3145d20e8424bd7b396309b0708a16c7d8000b76"


def http_json(url, method="GET", body=None, timeout=20):
    """Return (status, decoded_json_or_None). Raises on connection errors."""
    data = None
    headers = {}
    if body is not None:
        data = json.dumps(body).encode()
        headers["Content-Type"] = "application/json"
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        raw = resp.read().decode()
        return resp.status, json.loads(raw) if raw else None


def node_http(i):
    return f"http://localhost:{HTTP_BASE + 1000 * i}"


def fetch_status(i):
    _, body = http_json(f"{node_http(i)}/api/v1/status")
    return body


def all_nodes_running(validators):
    names = [BOOTSTRAP] + [f"devnet-validator-{i}" for i in range(1, validators)]
    for name in names:
        p = subprocess.run(
            ["docker", "inspect", "-f", "{{.State.Running}}", name],
            capture_output=True, text=True,
        )
        if p.returncode != 0 or p.stdout.strip() != "true":
            return False, name
    return True, ""


def dump_diagnostics(validators):
    subprocess.run(["docker", "ps"], check=False)
    names = [BOOTSTRAP] + [f"devnet-validator-{i}" for i in range(1, validators)]
    for name in names:
        subprocess.run(["docker", "logs", "--tail", "40", name], check=False)


def fail(msg, validators):
    print(f"FAIL: {msg}", file=sys.stderr)
    dump_diagnostics(validators)
    sys.exit(1)


def gen_term(rng):
    """A syntactically-valid, closed (free-var-free) rholang term."""
    n = rng.randint(0, 1000)
    s = rng.choice(["foo", "bar", "hello", "world", "fuzz"])
    b = rng.choice(["true", "false"])
    templates = [
        "Nil",
        f'@"fuzz"!({n})',
        f'@"fuzz"!("{s}")',
        f'@"fuzz"!({b})',
        f'new x in {{ x!({n}) | for (@v <- x) {{ @"fuzzout"!(v) }} }}',
        f'new x in {{ x!({n}) | for (@v <= x) {{ @"fuzzout"!(v) }} }}',
        f'new x in {{ x!!({n}) | for (@v <= x) {{ @"fuzzout"!(v) }} }}',
        f'@"fuzz"!([{n}, {n + 1}, {n + 2}].nth(0))',
        f'@"fuzz"!( {{ "k": {n} }}.get("k") )',
        f'@"fuzz"!({n} + {rng.randint(0, 9)} * {rng.randint(0, 9)})',
        f'if ({b}) {{ @"fuzz"!("yes") }} else {{ @"fuzz"!("no") }}',
        f'match {n} {{ 0 => @"fuzz"!("zero") _ => @"fuzz"!("nonzero") }}',
        f'@"fuzz"!({n}) | @"fuzz2"!("{s}")',
        f'@"fuzz"!("{s}".length())',
        f'@"fuzz"!(({n}, "{s}").nth(1))',
    ]
    return rng.choice(templates)


def heights_in_lockstep(numbers):
    """Validators count as in lock-step when their block-number spread is small.

    The equal-stake devnet never reaches `> 2/3` finality (2/3 is not a supermajority), so there is
    no finalized fringe to compare; latest-block-number convergence is the network-level determinism
    signal (content-hash determinism is covered by casper/tests/determinism.rs + scheduler.rs).
    """
    return bool(numbers) and (max(numbers) - min(numbers)) <= LOCKSTEP_TOLERANCE


def wait_for_lockstep(validators, timeout=120):
    """Poll until every validator's latestBlockNumber is within LOCKSTEP_TOLERANCE."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            numbers = [fetch_status(i)["latestBlockNumber"] for i in range(validators)]
        except (urllib.error.HTTPError, urllib.error.URLError):
            time.sleep(2)
            continue
        if heights_in_lockstep(numbers):
            return max(numbers)
        time.sleep(2)
    return None


def assert_lockstep(validators):
    """Assert every validator's latestBlockNumber is within LOCKSTEP_TOLERANCE (no divergence)."""
    numbers = []
    for i in range(validators):
        try:
            numbers.append(fetch_status(i)["latestBlockNumber"])
        except (urllib.error.HTTPError, urllib.error.URLError) as e:
            fail(f"validator {i} status unreachable: {e}", validators)
    if not heights_in_lockstep(numbers):
        fail(f"validators diverged in latestBlockNumber: {numbers}", validators)
    return max(numbers)


def signed_deploy(node, term, height):
    """Deploy a term through the real block path (docker exec rnode deploy)."""
    write = subprocess.run(
        ["docker", "exec", "-i", node, "sh", "-c", "cat > /tmp/fuzz.rho"],
        input=term, text=True, capture_output=True,
    )
    if write.returncode != 0:
        return write
    return subprocess.run(
        ["docker", "exec", node, "rnode", "--grpc-host", "localhost", "deploy",
         "--phlo-limit", "1000000", "--phlo-price", "1",
         "--private-key", DEPLOYER_PRIV, "--shard-id", "root",
         "--valid-after-block-number", str(height), "/tmp/fuzz.rho"],
        text=True, capture_output=True,
    )


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--validators", type=int, default=3, help="bonded validators (default 3)")
    ap.add_argument("--iterations", type=int, default=100, help="explore-deploy fuzz iterations")
    ap.add_argument("--deploy-burst", type=int, default=10, help="signed deploys through the block path")
    ap.add_argument("--seed", type=int, default=None, help="RNG seed (reproducible)")
    ap.add_argument("--sleep", type=float, default=0.05, help="pause between explore-deploys (s)")
    args = ap.parse_args()

    rng = random.Random(args.seed)

    # Pre-flight: every validator must be up and healthy.
    running, bad = all_nodes_running(args.validators)
    if not running:
        fail(f"node '{bad}' is not running (start it with tools/devnet.sh up)", args.validators)

    print(f"==> fuzz: {args.iterations} explore-deploys + {args.deploy_burst} signed deploys "
          f"across {args.validators} validators (seed={args.seed})")

    print("==> waiting for cross-validator lock-step ...")
    common_height = wait_for_lockstep(args.validators)
    if common_height is None:
        fail("timed out waiting for validators to reach lock-step", args.validators)
    print(f"==> pre-fuzz lock-step height: {common_height}")

    # --- robustness: read-only reducer fuzz ------------------------------------------------
    for it in range(args.iterations):
        node = it % args.validators
        term = gen_term(rng)
        url = f"{node_http(node)}/api/v1/explore-deploy"
        try:
            status, _ = http_json(url, method="POST", body=term)
        except urllib.error.HTTPError as e:
            status = e.code
        except (urllib.error.URLError, OSError) as e:
            fail(f"explore-deploy #{it} on validator {node} connection error: {e}", args.validators)

        if status >= 500:
            fail(f"explore-deploy #{it} on validator {node} returned {status} for term: {term}",
                 args.validators)
        # 200 = reduced, 400 = reduce error (fine — exercises error paths), 429 = rate limit (warn).
        if status == 429 and (it + 1) % 50 == 0:
            print(f"    (rate-limited at iteration {it})")
        time.sleep(args.sleep)

    # --- state-mutating fuzz through the block path ---------------------------------------
    height = fetch_status(0)["latestBlockNumber"]
    for j in range(args.deploy_burst):
        term = f'@"fuzzstate{j}"!({rng.randint(0, 10 ** 6)})'
        p = signed_deploy(BOOTSTRAP, term, height)
        if p.returncode != 0:
            fail(f"signed deploy #{j} failed: {p.stderr.strip() or p.stdout.strip()}", args.validators)
        height += 1

    # Wait for the deploy-driven blocks to propagate across validators.
    time.sleep(5)

    # --- determinism re-check --------------------------------------------------------------
    running, bad = all_nodes_running(args.validators)
    if not running:
        fail(f"node '{bad}' stopped during the fuzz run", args.validators)
    final_height = assert_lockstep(args.validators)

    print(f"==> post-fuzz: {args.validators} validators in lock-step at height {final_height}")
    print("==> robustness + determinism OK")


if __name__ == "__main__":
    main()
