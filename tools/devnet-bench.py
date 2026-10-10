#!/usr/bin/env python3
"""Benchmark a running devnet: block cost, deploy throughput and residency growth.

    python3 tools/devnet-bench.py --validators 1 --blocks 300 --deploys 200

Three measurements, in the order they run:

* **Block cost** — force blocks through the admin `propose` route and time them. This is the node's
  own cost for a block: propose, validate, replay-check, insert. The block rate is the reciprocal of
  that, and it is what an idle chain's throughput is bounded by.
* **Residency growth** — sample `/metrics`' five DAG gauges and the container's RSS as the chain
  grows. The point is the *shape*: the audit measured the message map at N(N+1)/2 entries for N
  blocks (17 319 555 entries at 5 885 blocks) from a long-running node's own metric, and a benchmark
  is how that curve gets confirmed on a chain this harness built, at a known block count, rather
  than trusted from a scraped number.
* **Deploy throughput and latency** — fire signed deploys at a concurrency, then time each one from
  submission to the block that processed it, read from `/api/v1/deploy-status/{id}`.

**The benchmark remains externally driven.** DAG residency comes from the node's existing gauges,
RSS from `docker stats`, and rates from wall-clock observation. #144 adds one deliberately narrow
exception: monotone `rchain_runtime_*` gauges expose the count and time of already-existing soft
checkpoints, because attributing that internal copy cost from outside the process is impossible. The
metrics are observation-only and are read as before/after deltas; they do not drive block execution.

Three caveats, stated because they bound what the numbers mean:

1. **The deploy rate includes the client's cost.** Each deploy is a real `rnode deploy` client: a
   process spawn, a TLS-less gRPC round trip and a signature inside the container, and its wall time
   is reported separately (`client_ms`) so the node's share is visible rather than assumed. The rate
   is "deploys per second this ingress sustains under K clients", not the node's internal ceiling.
2. **One validator.** The devnet's multi-validator path does not converge on this tree (a
   pre-existing participation/state-staleness defect: a joining validator's view of the active set
   lags, and with a cap below the validator count the draw can hand an epoch to a node that cannot
   propose — see `docs/src/node/security-audit.md` §8). So this measures a single node doing all the
   work: propose, validate and insert. That is the honest ceiling for *one* validator and says
   nothing about consensus overhead across three.
3. **Block rate here is propose-driven, and autopropose must be off** (`tools/devnet.sh up
   --validators 1 --no-autopropose`). With autopropose on, the node produces a block every
   `AUTOPROPOSE_INTERVAL` (2 s) on its own timer, so a height that moves proves nothing about the
   proposal this harness just made, and a measured "blocks/s" would be the timer's period divided
   into 1. Off, the rate is this loop's, so read the *distribution* of per-block time rather than the
   mean: the harness reports p50/p95/max for that reason.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import statistics
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request

# --- the network ------------------------------------------------------------------------------

GRPC_BASE = 40402
HTTP_BASE = 40403
ADMIN_BASE = 40405

# The deployer key. `tools/devnet.sh` funds validator 0's key as the devnet's deployer
# (`--dev-mode --deployer-private-key`), and it is the only key with a REV balance to pay phlo from,
# so it is the only one a signed deploy can be made with. A devnet started with a different
# `--deployer-key` needs the same key passed here.
DEPLOYER_PRIV = "a68a6e6cca30f81bd24a719f3145d20e8424bd7b396309b0708a16c7d8000b76"
DEPLOY_ID_RE = re.compile(r"DeployId is: ([0-9a-fA-F]+)")

# `rchain_dag_*` — the five DAG-residency gauges. `messages` is the one the residency claim is
# about; `seen_entries` is the sum of the per-message ancestry sets, which is where N(N+1)/2 shows
# up. #144 adds a separate `rchain_runtime_*` census below; neither family changes node behaviour.
DAG_GAUGES = ("rchain_dag_messages", "rchain_dag_seen_entries", "rchain_dag_fringe_states",
              "rchain_dag_index_entries", "rchain_dag_logical_bytes")

# #144 Stage 1: play-path soft-checkpoint census. The four user-deploy sites are intentionally
# separate from the block-level system-deploy site so the campaign can compute the actual
# snapshots/deploy and checkpoint-time/deploy ratios rather than infer "up to four" from source.
CHECKPOINT_SITES = ("deploy_fallback", "deploy_log", "cost_unit_fallback", "pre_charge_log")
RUNTIME_GAUGES = (
    "rchain_runtime_deploy_units",
    "rchain_runtime_deploy_unit_total_ns",
    "rchain_runtime_deploy_unit_max_ns",
    *(f"rchain_runtime_soft_checkpoint_{site}_{suffix}"
      for site in (*CHECKPOINT_SITES, "system_deploy_log")
      for suffix in ("count", "total_ns", "max_ns")),
)
GAUGES = DAG_GAUGES + RUNTIME_GAUGES

# The default image name `devnet.sh up` produces, i.e. the container the measurements are taken from.
CONTAINER = "devnet-bootstrap"


def http_json(url, method="GET", body=None, timeout=20):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(url, data=data, method=method,
                                 headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        raw = resp.read().decode()
    return json.loads(raw) if raw.strip() else None


def status(port=HTTP_BASE, timeout=5):
    return http_json(f"http://localhost:{port}/api/v1/status", timeout=timeout)


def height(port=HTTP_BASE):
    return status(port).get("latestBlockNumber")


def gauges(port=HTTP_BASE):
    """Scrape the benchmark's DAG and runtime gauges. Returns {} if the route is unreachable,
    so a sample is never silently read as zero growth or zero checkpoint work."""
    try:
        req = urllib.request.Request(f"http://localhost:{port}/metrics")
        with urllib.request.urlopen(req, timeout=5) as resp:
            text = resp.read().decode()
    except Exception:
        return {}
    out = {}
    for line in text.splitlines():
        for name in GAUGES:
            if line.startswith(name + " "):
                try:
                    out[name] = float(line.split()[1])
                except (IndexError, ValueError):
                    pass
    return out


_MEM_RE = re.compile(r"([0-9.]+)\s*([KMGT]i?B)")


def rss_mib(container):
    """The container's resident memory in MiB, from `docker stats`.

    One-shot (`--no-stream`) rather than a streaming session: a sample has to be taken *at* the
    block count it is reported beside, and `docker stats`' stream emits its own timing that would
    have to be reconciled with the height samples anyway.
    """
    try:
        out = subprocess.run(
            ["docker", "stats", "--no-stream", "--format", "{{.MemUsage}}", container],
            capture_output=True, text=True, timeout=15,
        ).stdout
    except Exception:
        return None
    m = _MEM_RE.search(out)
    if not m:
        return None
    value, unit = float(m.group(1)), m.group(2)
    factor = {"B": 1 / 1048576, "KiB": 1 / 1024, "MiB": 1, "GiB": 1024, "TiB": 1048576}
    return value * factor.get(unit, 1)


# --- measurement 1: block cost ------------------------------------------------------------------


def bench_blocks(args, container):
    """Force `--blocks` blocks and time each one.

    The proposal is forced through the admin route because that is the only way to make a block
    without a deploy, and forcing it is what makes the *rate* a property of the node rather than of
    the client: the loop does nothing between proposals but wait for the height to move.
    """
    print(f"==> blocks: forcing {args.blocks} through the admin propose route")
    samples = []
    times = []
    start = time.monotonic()
    start_height = height()
    for i in range(args.blocks):
        before = height()
        t0 = time.monotonic()
        try:
            resp = http_json(f"http://localhost:{ADMIN_BASE}/api/v1/propose", method="POST", timeout=30)
        except urllib.error.HTTPError as e:
            print(f"    propose refused ({e.code}): {e.read().decode()[:200]}", file=sys.stderr)
            break
        except Exception as e:  # a timeout here is a *result*: the block took longer than the limit
            print(f"    propose failed: {e}", file=sys.stderr)
            break
        # A proposal whose body does not say it created a block is a *result*, not noise: the route
        # reports `Success! Block <hash> created and added`, so a refusal ("not bonded", "no new
        # deploys") arrives as a 200 with a different sentence and must not be timed as a block.
        if isinstance(resp, str) and "created and added" not in resp:
            print(f"    propose declined: {resp[:160]}", file=sys.stderr)
            break
        # Wait for the height to move, bounded: a proposal that returns without producing a block is
        # a fact about the node and must not stall the run.
        deadline = time.monotonic() + args.block_timeout
        while height() <= before and time.monotonic() < deadline:
            time.sleep(0.01)
        dt = time.monotonic() - t0
        times.append(dt)
        if (i + 1) % args.sample_every == 0 or i == 0:
            samples.append({
                "blocks": i + 1,
                "seconds": time.monotonic() - start,
                "height": height(),
                "rss_mib": rss_mib(container),
                **gauges(),
            })
            s = samples[-1]
            print(f"    block {i + 1:5d}  {dt * 1000:7.1f} ms  height={s['height']}  "
                  f"messages={s.get('rchain_dag_messages')}  rss={s['rss_mib']:.0f} MiB"
                  if s["rss_mib"] is not None else
                  f"    block {i + 1:5d}  {dt * 1000:7.1f} ms  height={s['height']}")
    elapsed = time.monotonic() - start
    end_height = height()
    return {
        "requested": args.blocks,
        "produced": len(times),
        "height_delta": (end_height - start_height) if (end_height and start_height) else None,
        "seconds": elapsed,
        "blocks_per_sec": (len(times) / elapsed) if elapsed else None,
        "ms_p50": statistics.median(times) * 1000 if times else None,
        "ms_p95": (sorted(times)[int(len(times) * 0.95)] * 1000) if times else None,
        "ms_max": max(times) * 1000 if times else None,
        "samples": samples,
    }


# --- measurement 2: deploy throughput and latency -------------------------------------------


def one_deploy(container, deployer_key, scratch, term, valid_after, shard, native_rnode=None):
    """Submit one signed deploy and return (deploy_id, client_seconds).

    The normal arm uses `docker exec rnode deploy`. `--native-rnode` is the same Rust client against
    the same external gRPC API, used when the node itself is running directly on the host (for
    example Termux, where Docker is unavailable). No deploy bytes are reimplemented in Python.
    """
    if native_rnode:
        local_scratch = os.path.join(os.environ.get("TMPDIR", "/tmp"), f"rchain-{os.path.basename(scratch)}")
        try:
            Path(local_scratch).write_text(term)
        except Exception as e:
            return None, None, str(e)
        cmd = [native_rnode, "--grpc-host", "localhost", "--grpc-port", str(GRPC_BASE), "deploy",
               "--phlo-limit", "1000000", "--phlo-price", "1",
               "--private-key", deployer_key, "--shard-id", shard,
               "--valid-after-block-number", str(valid_after), local_scratch]
    else:
        # **One scratch file per worker, not one for the run.** Shared scratch paths corrupt terms.
        write = subprocess.run(
            ["docker", "exec", "-i", container, "sh", "-c", f"cat > {scratch}"],
            input=term, text=True, capture_output=True,
        )
        if write.returncode != 0:
            return None, None, write.stderr
        cmd = ["docker", "exec", container, "rnode", "--grpc-host", "localhost", "deploy",
               "--phlo-limit", "1000000", "--phlo-price", "1",
               "--private-key", deployer_key, "--shard-id", shard,
               "--valid-after-block-number", str(valid_after), scratch]
    t0 = time.monotonic()
    proc = subprocess.run(cmd, text=True, capture_output=True, timeout=120)
    dt = time.monotonic() - t0
    m = DEPLOY_ID_RE.search(proc.stdout or "")
    if not m:
        return None, dt, (proc.stdout or "") + (proc.stderr or "")
    return m.group(1), dt, None


def deploy_status(deploy_id):
    """(processed?, block number) for a deploy id — the node's own verdict on where it landed."""
    try:
        doc = http_json(f"http://localhost:{HTTP_BASE}/api/v1/deploy-status/{deploy_id}", timeout=10)
    except Exception:
        return False, None
    if not isinstance(doc, dict):
        return False, None
    for key, value in doc.items():
        if key.startswith("Processed") and isinstance(value, dict):
            return True, (value.get("block") or {}).get("blockNumber")
    return False, None


def soft_checkpoint_delta(before, after):
    """Derive #144's Stage-1 measurements from two process-counter scrapes.

    Only monotone count/total gauges are subtracted. Missing or decreasing counters make the reading
    unavailable rather than silently turning it into zero: a process restart or a partial scrape is a
    void arm, not a cheap checkpoint. Maxima are process-wide and are therefore diagnostic only.
    """
    required = ["rchain_runtime_deploy_units", "rchain_runtime_deploy_unit_total_ns"]
    for site in CHECKPOINT_SITES:
        required.extend((
            f"rchain_runtime_soft_checkpoint_{site}_count",
            f"rchain_runtime_soft_checkpoint_{site}_total_ns",
        ))
    missing = [name for name in required if name not in before or name not in after]
    if missing:
        return {"available": False, "reason": f"missing runtime gauge(s): {', '.join(missing)}"}
    backwards = [name for name in required if after[name] < before[name]]
    if backwards:
        return {"available": False, "reason": f"runtime gauge(s) moved backwards: {', '.join(backwards)}"}

    def delta(name):
        return after[name] - before[name]

    deploy_units = delta("rchain_runtime_deploy_units")
    deploy_ns = delta("rchain_runtime_deploy_unit_total_ns")
    sites = {}
    checkpoint_count = 0.0
    checkpoint_ns = 0.0
    for site in CHECKPOINT_SITES:
        count = delta(f"rchain_runtime_soft_checkpoint_{site}_count")
        total_ns = delta(f"rchain_runtime_soft_checkpoint_{site}_total_ns")
        sites[site] = {"count": count, "total_ns": total_ns}
        checkpoint_count += count
        checkpoint_ns += total_ns

    return {
        "available": True,
        "deploy_units": deploy_units,
        "checkpoint_count": checkpoint_count,
        "snapshots_per_deploy": checkpoint_count / deploy_units if deploy_units else None,
        "checkpoint_total_ms": checkpoint_ns / 1_000_000,
        "checkpoint_ms_per_deploy": (checkpoint_ns / deploy_units / 1_000_000)
        if deploy_units else None,
        "deploy_unit_total_ms": deploy_ns / 1_000_000,
        "deploy_unit_ms_per_deploy": (deploy_ns / deploy_units / 1_000_000)
        if deploy_units else None,
        "checkpoint_fraction_of_deploy_unit": checkpoint_ns / deploy_ns if deploy_ns else None,
        "sites": sites,
    }


def bench_deploys(args, container):
    print(f"==> deploys: {args.deploys} signed deploys, concurrency {args.concurrency}")
    shard = (status().get("shardId") or "/root")
    lock = threading.Lock()
    submitted = []          # (id, t_submit, client_seconds)
    failures = []
    counter = {"n": 0}

    def worker(w):
        while True:
            with lock:
                if counter["n"] >= args.deploys:
                    return
                n = counter["n"]
                counter["n"] += 1
            # A unique term per deploy: the node refuses a replayed deploy id, so a repeated term
            # would be rejected by the pool rather than measured.
            term = f'@"bench-{w}-{n}"!({n})'
            # Anchor each deploy to the chain height at submission time. A single height captured
            # before a long load ages the earliest deploys across DEPLOY_LIFESPAN while the chain is
            # still processing later submissions; #144's first preregistered arm exposed the node's
            # separate pool/validation boundary bug that way. Per-submit anchoring is also what
            # `tools/devnet.sh deploy` does, and keeps this performance arm from manufacturing stale
            # deploys as client-side load progresses. The checkpoint decision thresholds are unchanged.
            valid_after = height() or 0
            did, client_dt, err = one_deploy(
                container, args.deployer_key, f"bench-{w}.rho", term, valid_after, shard,
                native_rnode=args.native_rnode)
            with lock:
                if did is None:
                    failures.append(err or "no deploy id")
                else:
                    submitted.append((did, time.monotonic(), client_dt))

    threads = [threading.Thread(target=worker, args=(w,)) for w in range(args.concurrency)]
    runtime_before = gauges()
    t_start = time.monotonic()
    for t in threads:
        t.start()
    # Sample while the load runs, not only after: the residency curve is a property of the chain as
    # it grows, and a single before/after pair cannot show whether the growth is linear in blocks.
    samples = [{"seconds": 0.0, "height": height(), "submitted": 0, "rss_mib": rss_mib(container),
                **gauges()}]
    while any(t.is_alive() for t in threads):
        time.sleep(args.sample_seconds)
        with lock:
            n = len(submitted)
        samples.append({"seconds": time.monotonic() - t_start, "height": height(), "submitted": n,
                        "rss_mib": rss_mib(container), **gauges()})
        print(f"    t={samples[-1]['seconds']:6.1f}s  submitted={n:4d}  height={samples[-1]['height']}"
              f"  messages={samples[-1].get('rchain_dag_messages')}"
              f"  rss={samples[-1]['rss_mib']:.0f} MiB"
              if samples[-1]["rss_mib"] is not None else
              f"    t={samples[-1]['seconds']:6.1f}s  submitted={n:4d}  height={samples[-1]['height']}")
    for t in threads:
        t.join()
    submit_seconds = time.monotonic() - t_start

    print(f"    submitted {len(submitted)}, failed {len(failures)} in {submit_seconds:.1f}s")

    # Wait out the backlog, **proposing while we do**, and that is not a fudge: the devnet's
    # production configuration has autopropose on, so a real node keeps making blocks while deploys
    # sit in its pool. With autopropose off, the block path only proposes on a *deploy arrival*, so
    # once this harness stops submitting, nothing makes a block and the pool never drains — measured:
    # the chain froze at height 51 with 133 accepted deploys stranded, which reads as a node that
    # cannot keep up when it is really a harness that stopped asking.
    latencies = []
    proposals = 0
    deadline = time.monotonic() + args.deploy_timeout
    pending = list(submitted)
    while pending and time.monotonic() < deadline:
        if args.drain:
            try:
                http_json(f"http://localhost:{ADMIN_BASE}/api/v1/propose", method="POST", timeout=30)
                proposals += 1
            except Exception:
                pass
        still = []
        for did, t_submit, _ in pending:
            done, block = deploy_status(did)
            if done:
                latencies.append((time.monotonic() - t_submit, block))
            else:
                still.append((did, t_submit, 0.0))
        pending = still
        if pending:
            time.sleep(0.1)

    lats = [l for l, _ in latencies]
    blocks_hit = {b for _, b in latencies if b is not None}
    elapsed = time.monotonic() - t_start
    # The load window, from the first sample to the last *during submission* — not the whole run: the
    # drain below is this harness proposing, and counting it would credit the node with blocks it was
    # asked for after the load stopped.
    first, last = samples[0], samples[-1]
    d_blocks = (last["height"] or 0) - (first["height"] or 0)
    load_seconds = (last["seconds"] - first["seconds"]) or elapsed
    runtime_after = gauges()
    checkpoint = soft_checkpoint_delta(runtime_before, runtime_after)
    return {
        "requested": args.deploys,
        "submitted": len(submitted),
        "failed": len(failures),
        "submit_seconds": submit_seconds,
        "submit_per_sec": len(submitted) / submit_seconds if submit_seconds else None,
        "client_ms_p50": statistics.median([c for _, _, c in submitted if c]) * 1000
                        if any(c for _, _, c in submitted) else None,
        "processed": len(latencies),
        "unprocessed": len(pending),
        "ms_p50": statistics.median(lats) * 1000 if lats else None,
        "ms_p95": (sorted(lats)[int(len(lats) * 0.95)] * 1000) if lats else None,
        "ms_max": max(lats) * 1000 if lats else None,
        "deploys_per_block": (len(latencies) / len(blocks_hit)) if blocks_hit else None,
        "blocks_hit": len(blocks_hit),
        "drain_proposals": proposals,
        "seconds": load_seconds,
        "blocks_per_sec": (d_blocks / load_seconds) if load_seconds else None,
        "height_delta": d_blocks,
        "soft_checkpoints": checkpoint,
        "samples": samples,
        "errors": failures[:5],
    }


# --- measurement 3: the residency curve, read off the samples -------------------------------


def residency(block_result):
    """Where the residency went, and whether it is the N(N+1)/2 the metric claims.

    The claim to check is `messages = N` and `seen_entries = N(N+1)/2` for N blocks, so the check is
    *arithmetic against the sampled pair*, not against a remembered number: both gauges are read from
    the same scrape, and a chain whose blocks are not a single chain (a fork, or a proposal that
    produced nothing) breaks the identity exactly as it should.
    """
    samples = [s for s in block_result["samples"] if s.get("rchain_dag_messages")]
    if len(samples) < 2:
        return {"available": False}
    first, last = samples[0], samples[-1]
    d_blocks = (last["height"] or 0) - (first["height"] or 0)
    d_messages = last["rchain_dag_messages"] - first["rchain_dag_messages"]
    d_seen = (last.get("rchain_dag_seen_entries", 0) - first.get("rchain_dag_seen_entries", 0))
    d_bytes = (last.get("rchain_dag_logical_bytes", 0) - first.get("rchain_dag_logical_bytes", 0))
    n = last["rchain_dag_messages"]
    predicted = n * (n + 1) / 2
    rss_first, rss_last = first.get("rss_mib"), last.get("rss_mib")
    return {
        "available": True,
        "blocks": d_blocks,
        "messages_per_block": d_messages / d_blocks if d_blocks else None,
        "seen_per_block": d_seen / d_blocks if d_blocks else None,
        "bytes_per_block": d_bytes / d_blocks if d_blocks else None,
        "seen_entries": last.get("rchain_dag_seen_entries"),
        "messages": n,
        "n_n_plus_1_over_2": predicted,
        "curve_holds": last.get("rchain_dag_seen_entries") == predicted,
        "rss_first_mib": rss_first,
        "rss_last_mib": rss_last,
        "rss_per_block_kib": ((rss_last - rss_first) * 1024 / d_blocks)
                             if (rss_first is not None and rss_last is not None and d_blocks) else None,
    }


def git_head():
    try:
        return subprocess.run(
            ["git", "rev-parse", "HEAD"], capture_output=True, text=True, timeout=5, check=True
        ).stdout.strip()
    except Exception:
        return None


def container_image_id(container):
    try:
        return subprocess.run(
            ["docker", "inspect", "--format", "{{.Image}}", container],
            capture_output=True, text=True, timeout=5, check=True,
        ).stdout.strip() or None
    except Exception:
        return None


def file_sha256(path):
    if not path:
        return None
    try:
        h = hashlib.sha256()
        with open(path, "rb") as f:
            for chunk in iter(lambda: f.read(1024 * 1024), b""):
                h.update(chunk)
        return h.hexdigest()
    except Exception:
        return None


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--validators", type=int, default=1, help="expected bonded validators (a check)")
    ap.add_argument("--blocks", type=int, default=200, help="blocks to force")
    ap.add_argument("--deploys", type=int, default=0, help="signed deploys to submit")
    ap.add_argument("--concurrency", type=int, default=4, help="parallel deploy clients")
    ap.add_argument("--sample-every", type=int, default=25, help="sample every N blocks")
    ap.add_argument("--sample-seconds", type=float, default=2.0, help="sample every N seconds (deploys)")
    ap.add_argument("--no-drain", dest="drain", action="store_false",
                    help="do not propose the backlog out; measures the chain's own rate instead")
    ap.add_argument("--block-timeout", type=float, default=30.0, help="seconds to wait for a block")
    ap.add_argument("--deploy-timeout", type=float, default=120.0, help="seconds to wait for inclusion")
    ap.add_argument("--only", choices=["blocks", "deploys"], help="run one measurement")
    ap.add_argument("--json", help="write the full result, samples included, to this path")
    ap.add_argument("--deployer-key", default=DEPLOYER_PRIV,
                    help="private key of a deployer funded in the devnet's wallets file")
    ap.add_argument("--container", default=CONTAINER,
                    help="container the node under test runs in, for `docker stats` and exec")
    ap.add_argument("--native-rnode",
                    help="host path to the rnode binary; use the native Rust client instead of docker exec")
    args = ap.parse_args()

    container = args.container
    try:
        st = status()
    except Exception as e:
        sys.exit(f"no devnet on :{HTTP_BASE} ({e}) — start one with tools/devnet.sh up")
    print(f"    node {st.get('address')} shard {st.get('shardId')} height {st.get('latestBlockNumber')}")

    result = {
        "tree": git_head(),
        "node": st.get("address"),
        "shard": st.get("shardId"),
        "image_id": container_image_id(container),
        "node_status": {
            "devMode": st.get("devMode"),
            "autopropose": st.get("autopropose"),
            "proposeOnDeploy": st.get("proposeOnDeploy"),
            "peers": st.get("peers"),
            "nodes": st.get("nodes"),
        },
        "configuration": {
            "validators": args.validators,
            "blocks": args.blocks,
            "deploys": args.deploys,
            "concurrency": args.concurrency,
            "drain": args.drain,
            "container": args.container,
            "native_rnode": args.native_rnode,
            "native_rnode_sha256": file_sha256(args.native_rnode),
            "sample_every": args.sample_every,
            "sample_seconds": args.sample_seconds,
            "block_timeout": args.block_timeout,
            "deploy_timeout": args.deploy_timeout,
        },
    }
    if args.only != "deploys":
        result["blocks"] = bench_blocks(args, container)
        result["residency"] = residency(result["blocks"])
    if args.only != "blocks" and args.deploys:
        result["deploys"] = bench_deploys(args, container)

    print("\n=== summary ===")
    if "blocks" in result:
        b = result["blocks"]
        print(f"blocks         {b['produced']}/{b['requested']} produced in {b['seconds']:.1f}s"
              f"  -> {b['blocks_per_sec']:.2f}/s")
        if b["ms_p50"] is not None:
            print(f"per block      p50 {b['ms_p50']:.0f} ms   p95 {b['ms_p95']:.0f} ms   "
                  f"max {b['ms_max']:.0f} ms")
        r = result["residency"]
        if r.get("available"):
            print(f"residency      {r['messages_per_block']:.2f} messages/block, "
                  f"{r['seen_per_block']:.2f} ancestry entries/block, "
                  f"{r['bytes_per_block']:.0f} bytes/block")
            print(f"               at {r['messages']:.0f} messages, seen_entries "
                  f"{r['seen_entries']:.0f} vs N(N+1)/2 {r['n_n_plus_1_over_2']:.0f}  -> "
                  f"{'matches' if r['curve_holds'] else 'DOES NOT MATCH'}")
            if r["rss_per_block_kib"] is not None:
                print(f"               rss {r['rss_first_mib']:.0f} -> {r['rss_last_mib']:.0f} MiB "
                      f"({r['rss_per_block_kib']:.1f} KiB/block)")
    if "deploys" in result:
        d = result["deploys"]
        print(f"deploys        {d['submitted']} submitted, {d['processed']} processed, "
              f"{d['failed']} failed")
        if d["submit_per_sec"]:
            print(f"ingress        {d['submit_per_sec']:.1f} deploys/s across "
                  f"{args.concurrency} clients (client itself {d['client_ms_p50']:.0f} ms p50)")
        if d["ms_p50"] is not None:
            print(f"inclusion      p50 {d['ms_p50']:.0f} ms   p95 {d['ms_p95']:.0f} ms   "
                  f"max {d['ms_max']:.0f} ms")
            print(f"               {d['deploys_per_block']:.2f} deploys/block over "
                  f"{d['blocks_hit']} blocks")
            if d["unprocessed"]:
                print(f"               {d['unprocessed']} still unprocessed after "
                      f"{args.deploy_timeout:.0f}s (drain proposals: {d['drain_proposals']})")
        if d.get("blocks_per_sec"):
            print(f"chain          {d['height_delta']} blocks in {d['seconds']:.1f}s "
                  f"-> {d['blocks_per_sec']:.2f} blocks/s")
            r = residency({"samples": d["samples"]})
            if r.get("available"):
                print(f"residency      {r['messages_per_block']:.2f} messages/block, "
                      f"{r['seen_per_block']:.2f} ancestry entries/block  ->  at "
                      f"{r['messages']:.0f} messages seen_entries {r['seen_entries']:.0f} vs "
                      f"N(N+1)/2 {r['n_n_plus_1_over_2']:.0f}  "
                      f"{'matches' if r['curve_holds'] else 'DOES NOT MATCH'}")
                if r["rss_per_block_kib"] is not None:
                    print(f"               rss {r['rss_first_mib']:.0f} -> "
                          f"{r['rss_last_mib']:.0f} MiB ({r['rss_per_block_kib']:.1f} KiB/block)")
        c = d.get("soft_checkpoints", {})
        if c.get("available"):
            print(f"soft checkpoints {c['checkpoint_count']:.0f} across "
                  f"{c['deploy_units']:.0f} deploy units -> "
                  f"{c['snapshots_per_deploy']:.2f}/deploy")
            print(f"               {c['checkpoint_ms_per_deploy']:.3f} ms/deploy, "
                  f"{100 * c['checkpoint_fraction_of_deploy_unit']:.1f}% of measured deploy-unit time")
            for site, sample in c["sites"].items():
                per = sample["total_ns"] / c["deploy_units"] / 1_000_000 if c["deploy_units"] else 0
                print(f"               {site:18s} {sample['count']:.0f} calls, {per:.3f} ms/deploy")
        elif c:
            print(f"soft checkpoints unavailable: {c.get('reason', 'unknown runtime-census error')}")
        for e in d["errors"]:
            print(f"    error: {e.strip()[:160]}")

    if args.json:
        with open(args.json, "w") as fh:
            json.dump(result, fh, indent=2)
        print(f"\nwritten: {args.json}")


if __name__ == "__main__":
    main()
