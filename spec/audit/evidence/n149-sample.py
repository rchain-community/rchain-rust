#!/usr/bin/env python3
"""Sample an N-validator devnet for #149's blocks-per-deploy sweep.

One sampler for the whole sweep, parameterised by the validator count, because the question is a
*function* of that count and a per-N copy of this file would be four places for the reading to drift.

Two artifacts, both under the run directory:

- `series.tsv` — one row per node per second: height (`latestBlockNumber`), finalised block number, and
  whether the node answered at all. This is the n127 sampler's shape, reduced to the columns this
  measurement reads.
- `blocks.tsv` — the **union** of block hashes ever seen, with the second each was first seen. A block
  on a fork branch would be miscounted by a height delta (two validators can mint at the same height),
  and the union over all nodes counts it once, from whichever node saw it first.

The configuration is read out of the running container rather than restated by the caller: an artifact
that cannot say what it measured is the defect C176 registers, and the earlier sampler committed it by
hardcoding a cap and a shape.

Usage:  N149_N=3 N149_WINDOW_S=300 python3 spec/audit/evidence/n149-sample.py <outdir>
"""

import json
import os
import re
import subprocess
import sys
import time

HTTP_BASE = 40403
PREFIX = os.environ.get("DEVNET_PREFIX", "devnet")
N = int(os.environ.get("N149_N", "3"))
WINDOW_S = int(os.environ.get("N149_WINDOW_S", "300"))
# Deep enough to hold every block the run can mint: the node's own cap is `api_server.max-blocks-limit`,
# which is 111111 under `--dev-mode` (the devnet's default) — so this is a bound on *our* request, not on
# the node's retention, and it is stated rather than silently clamped.
DEPTH = int(os.environ.get("N149_DEPTH", "2000"))

OUTDIR = sys.argv[1] if len(sys.argv) > 1 else "target/n149-blocks"
SERIES = os.path.join(OUTDIR, "series.tsv")
BLOCKS = os.path.join(OUTDIR, "blocks.tsv")


def nodes():
    """`bootstrap` is validator 0; validator i publishes HTTP on `HTTP_BASE + i*1000` (devnet.sh:132)."""
    out = [("bootstrap", f"{PREFIX}-bootstrap", HTTP_BASE)]
    for i in range(1, N):
        out.append((f"v{i}", f"{PREFIX}-validator-{i}", HTTP_BASE + i * 1000))
    return out


def sh(cmd, timeout=8):
    try:
        return subprocess.run(cmd, shell=True, capture_output=True, text=True, timeout=timeout).stdout
    except Exception:
        return ""


def get(url, timeout=6):
    return sh(f"curl -s --max-time 3 {url} 2>/dev/null", timeout=timeout)


def header():
    nodes_ = nodes()
    cid = sh(f"docker inspect {nodes_[0][1]} --format '{{{{.Id}}}}'").strip()
    limits = sh(f"docker inspect {nodes_[0][1]} "
                "--format '{{.HostConfig.Memory}} {{.HostConfig.MemorySwap}}'").strip()
    cmd = sh(f"docker inspect {nodes_[0][1]} --format '{{{{.Config.Cmd}}}}'").strip()
    envs = sh(f"docker inspect {nodes_[0][1]} "
              "--format '{{range .Config.Env}}{{println .}}{{end}}'").splitlines()
    return [
        f"validators={N} nodes=" + ",".join(f"{n}:{p}" for n, _c, p in nodes_),
        f"cap=memory.max={limits.split()[0] if limits.split() else '?'} "
        f"memory.swap.max={limits.split()[1] if len(limits.split()) > 1 else '?'} (bytes)",
        "env=" + (", ".join(e for e in envs if e.startswith(("_RJEM_MALLOC_CONF=", "LD_PRELOAD=")))
                  or "none"),
        f"container_cmd={cmd}",
        "tree=" + sh("git rev-parse HEAD").strip(),
        "image=" + sh("docker inspect rnode:local --format '{{.Id}} {{.Created}}'").strip(),
        "container=" + cid,
        "binary_sha256="
        + (sh(f"docker exec {nodes_[0][1]} sha256sum /usr/local/bin/rnode").split() or ["?"])[0],
    ]


def main():
    os.makedirs(OUTDIR, exist_ok=True)
    seen = {}
    with open(SERIES, "w") as fh:
        for line in header():
            fh.write(f"# {line}\n")
        fh.write("utc\tepoch\tnode\theight\tfinalized\talive\n")

    started = time.time()
    while time.time() - started < WINDOW_S:
        now = time.time()
        utc = time.strftime("%H:%M:%S", time.gmtime(now))
        alive = 0
        rows = []
        for name, container, port in nodes():
            pid = sh(f"docker inspect {container} --format '{{{{.State.Pid}}}}'").strip()
            if not pid or pid == "0":
                rows.append((name, "", "", "0"))
                continue
            alive += 1
            status = get(f"http://localhost:{port}/api/v1/status")
            m = re.search(r'"latestBlockNumber":(\d+)', status)
            height = m.group(1) if m else ""
            fin = get(f"http://localhost:{port}/api/last-finalized-block")
            fm = re.search(r'"blockNumber":(\d+)', fin)
            rows.append((name, height, fm.group(1) if fm else "none", "1"))

            # The union: first sighting wins, so `first_seen_epoch` is the earliest node's answer.
            body = get(f"http://localhost:{port}/api/blocks/{DEPTH}")
            try:
                blocks = json.loads(body)
            except Exception:
                blocks = []
            for b in blocks if isinstance(blocks, list) else []:
                h = b.get("blockHash")
                if h and h not in seen:
                    seen[h] = (int(now), b.get("blockNumber", -1), b.get("sender", "?"),
                               b.get("deployCount", -1))
        with open(SERIES, "a") as fh:
            for name, height, fin, al in rows:
                fh.write("\t".join([utc, f"{now:.0f}", name, str(height), str(fin), al]) + "\n")
        if alive == 0:
            print("all nodes dead", flush=True)
            break
        time.sleep(1)
    else:
        print("window elapsed with a node still alive", flush=True)

    with open(BLOCKS, "w") as fh:
        fh.write("# first_seen_epoch\tblock_number\tsender\tdeploy_count\tblock_hash\n")
        for h, (t, num, sender, dc) in sorted(seen.items(), key=lambda kv: (kv[1][0], kv[1][1])):
            fh.write(f"{t}\t{num}\t{sender}\t{dc}\t{h}\n")
    print(f"wrote {SERIES} ({len(seen)} distinct blocks) and {BLOCKS}", flush=True)


if __name__ == "__main__":
    main()
