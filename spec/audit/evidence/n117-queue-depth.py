#!/usr/bin/env python3
"""Does the validated-blocks queue actually grow under the fork storm? (C175)

The audit could not rule this queue out as #117's mechanism for one plain reason: **nothing measured its
depth**. PR #120 added that observable (`rchain_block_pipeline_shard_<n>_<stage>_depth` on `/metrics`), so
the question is now answerable in a single run. This sampler reads the depth beside the cgroup's `anon`
and the node's height and finality, once a second, per node.

The configuration goes into the header, because a measurement whose artifact cannot say what it measured
is the defect C176 registers — and one arm of the earlier work was labelled "no arena cap" while its
container had one.
"""

import csv
import os
import re
import subprocess
import sys
import time

NODES = {"bootstrap": 40403, "v1": 41403, "v2": 42403}
CONTAINERS = {"bootstrap": "devnet-bootstrap", "v1": "devnet-validator-1", "v2": "devnet-validator-2"}
OUT = sys.argv[1] if len(sys.argv) > 1 else "target/n117-audit/queue-depth.tsv"
WINDOW_S = int(os.environ.get("WINDOW_S", "300"))
MIB = 1 / 1048576
DEPTH = re.compile(r"rchain_block_pipeline_shard_(\d+)_(validated|autopropose|attestation)_depth\s+(\d+)")


def sh(cmd, timeout=8):
    try:
        return subprocess.run(cmd, shell=True, capture_output=True, text=True, timeout=timeout).stdout
    except Exception:
        return ""


def cgroup_of(container):
    cid = sh(f"docker inspect {container} --format '{{{{.Id}}}}'").strip()
    if not cid:
        return None
    path = f"/sys/fs/cgroup/system.slice/docker-{cid}.scope"
    return path if sh(f"test -r {path}/memory.stat && echo yes").strip() == "yes" else None


def main():
    # The header is read out of the running container, not restated by the caller. It used to hardcode
    # "env=none (DEVNET_NODE_ENV empty)" and "cap=4g", which made the artifact unable to say what it
    # measured — C176's defect, and one this same file committed while the run it described passed a
    # `_RJEM_MALLOC_CONF` that the header would have denied.
    cid = sh(f"docker inspect {CONTAINERS['bootstrap']} --format '{{{{.Id}}}}'").strip()
    envs = sh(
        f"docker inspect {CONTAINERS['bootstrap']} "
        "--format '{{range .Config.Env}}{{println .}}{{end}}'"
    ).splitlines()
    allocator = [e for e in envs if e.startswith(("_RJEM_MALLOC_CONF=", "LD_PRELOAD="))]
    limits = sh(
        f"docker inspect {CONTAINERS['bootstrap']} "
        "--format '{{.HostConfig.Memory}} {{.HostConfig.MemorySwap}}'"
    ).strip()
    config = [
        f"cap=memory.max={limits.split()[0] if limits.split() else '?'} "
        f"memory.swap.max={limits.split()[1] if len(limits.split()) > 1 else '?'} (bytes)",
        "env=" + (", ".join(allocator) if allocator else "none (no allocator env override)"),
        "shape=--validators 3 --stakes 100,100,50 --epoch-length 10 --fresh",
        "tree=" + sh("git rev-parse HEAD").strip(),
        "image=" + sh("docker inspect rnode:local --format '{{.Id}} {{.Created}}'").strip(),
        "container=" + cid,
        "binary_sha256="
        + (sh(f"docker exec {CONTAINERS['bootstrap']} sha256sum /usr/local/bin/rnode").split() or ["?"])[0],
    ]
    with open(OUT, "w") as fh:
        for line in config:
            fh.write(f"# {line}\n")
        fh.write("utc\tnode\tvalidated_depth\tautopropose_depth\tattestation_depth\tanon\tpeak\theight\tfinalized\talive\n")

    cgroups = {n: cgroup_of(c) for n, c in CONTAINERS.items()}
    print(f"cgroups: {cgroups}", flush=True)
    started = time.time()
    while time.time() - started < WINDOW_S:
        alive = 0
        now = time.strftime("%H:%M:%S", time.gmtime())
        for node, container in CONTAINERS.items():
            cg = cgroups.get(node)
            pid = sh(f"docker inspect {container} --format '{{{{.State.Pid}}}}'").strip()
            if not cg or not pid or pid == "0":
                continue
            alive += 1
            metrics = sh(f"curl -s --max-time 3 http://localhost:{NODES[node]}/metrics 2>/dev/null", timeout=6)
            depths = {"validated": "", "autopropose": "", "attestation": ""}
            for _shard, stage, value in DEPTH.findall(metrics):
                depths[stage] = value
            stat = sh(f"cat {cg}/memory.stat 2>/dev/null")
            anon = re.search(r"^anon (\d+)", stat, re.M)
            peak = sh(f"cat {cg}/memory.peak 2>/dev/null").strip()
            status = sh(f"curl -s --max-time 3 http://localhost:{NODES[node]}/api/v1/status 2>/dev/null", timeout=6)
            height = re.search(r'"latestBlockNumber":(\d+)', status)
            fin = sh(f"curl -s --max-time 3 http://localhost:{NODES[node]}/api/last-finalized-block 2>/dev/null", timeout=6)
            fin_n = re.search(r'"blockNumber":(\d+)', fin)
            with open(OUT, "a") as fh:
                fh.write("\t".join([
                    now, node,
                    depths["validated"], depths["autopropose"], depths["attestation"],
                    f"{int(anon.group(1)) * MIB:.1f}" if anon else "",
                    f"{int(peak) * MIB:.1f}" if peak.isdigit() else "",
                    height.group(1) if height else "",
                    fin_n.group(1) if fin_n else "none",
                    "1",
                ]) + "\n")
        if alive == 0:
            print("all nodes dead", flush=True)
            break
        time.sleep(1)
    else:
        print("window elapsed with a node still alive", flush=True)
    print(f"wrote {OUT}", flush=True)


if __name__ == "__main__":
    main()
