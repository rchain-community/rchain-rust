#!/usr/bin/env python3
"""Exercise `n149-summarise.py` on synthetic artifacts, so the reading program has a check that fails.

The four shapes are the ones the run can actually produce and the summariser must get right: the
textbook run, a non-quiescent idle window (which voids the attempt), a deploy that is not the only
deploy-bearing block, and finality that never catches up. A summariser only ever exercised on a good
run is one whose failure modes are unmeasured.

Usage:  python3 spec/audit/evidence/n149-summarise-selftest.py
"""

import os
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
BASE = "target/n149-blocks/SELFTEST"


def run(name, settle_end, deploy_epoch, read_end, blocks, finals, nodes, parents=2):
    d = os.path.join(BASE, name)
    os.makedirs(d, exist_ok=True)
    n = name.split("-")[0][1:]
    arm = name.split("-")[1]
    with open(os.path.join(d, "marks.tsv"), "w") as fh:
        fh.write(f"arm\t{arm}\nn\t{n}\nattempt\t1\nt0\t{settle_end}\n")
        fh.write(f"settle_end\t{settle_end}\nidle_end_and_deploy\t{deploy_epoch}\n")
        fh.write(f"read_end\t{read_end}\n")
    with open(os.path.join(d, "blocks.tsv"), "w") as fh:
        fh.write("# first_seen_epoch\tblock_number\tsender\tdeploy_count\tparents\tblock_hash\n")
        for i, (e, sender, dc) in enumerate(blocks):
            fh.write(f"{e}\t{i + 1}\t{sender}\t{dc}\t{parents}\th{i + 1}\n")
    with open(os.path.join(d, "series.tsv"), "w") as fh:
        fh.write("# header\nutc\tepoch\tnode\theight\tfinalized\talive\n")
        for e in range(settle_end, read_end + 1, 5):
            for nd in nodes:
                fh.write(f"00:00\t{e}\t{nd}\t1\t{finals(e)}\t1\n")


def main():
    shutil.rmtree(BASE, ignore_errors=True)
    # 1. Textbook: one deploy, two attestations, finality ten seconds later.
    run("n3-noauto-a1", 1000, 1060, 1240, [(1061, "A", 1), (1062, "B", 0), (1063, "C", 0)],
        lambda e: 1 if e >= 1070 else "none", ["bootstrap", "v1", "v2"])
    # 2. An idle window that minted: the rig is void and the summariser must say so.
    run("n3-noauto-a2", 1000, 1060, 1240, [(1020, "A", 0), (1061, "A", 1), (1062, "B", 0)],
        lambda e: 1 if e >= 1070 else "none", ["bootstrap", "v1", "v2"])
    # 3. Two deploy-bearing blocks: the deploy is not unique, which the note must flag.
    run("n3-noauto-a3", 1000, 1060, 1240, [(1061, "A", 1), (1062, "B", 1)],
        lambda e: 1 if e >= 1070 else "none", ["bootstrap", "v1", "v2"])
    # 4. Finality that never catches up.
    run("n3-auto-a1", 1000, 1060, 1240, [(1061, "A", 1), (1062, "B", 0), (1063, "C", 0)],
        lambda e: "none", ["bootstrap", "v1", "v2"])
    # 5. The control arm, idle window minting: expected here (the autopropose dummy deploy is the
    #    driver), so it must **not** void the attempt — the distinction the void rule turns on.
    run("n3-auto-a2", 1000, 1060, 1240, [(1020, "A", 0), (1061, "A", 1), (1062, "B", 0)],
        lambda e: 1 if e >= 1070 else "none", ["bootstrap", "v1", "v2"])

    out = subprocess.run([sys.executable, os.path.join(HERE, "n149-summarise.py"), BASE],
                         capture_output=True, text=True).stdout
    print(out)
    want = {
        "n3-noauto-a1": (0, 3, 3, "10"),
        "n3-noauto-a2": (1, 2, 2, "never"),
        "n3-noauto-a3": (0, 2, 2, "10"),
        "n3-auto-a1": (0, 3, 3, "never"),
        # `never`: its deploy-bearing block is #2 and the fixture's finality only reaches #1. The case
        # exists for the idle/void distinction, not for its latency.
        "n3-auto-a2": (1, 2, 2, "never"),
    }
    bad = []
    for name, (idle, blocks, senders, ttf) in want.items():
        line = next((ln for ln in out.splitlines() if ln.startswith(name)), None)
        if line is None:
            bad.append(f"{name}: no row")
            continue
        cols = line.split()
        if (int(cols[1]), int(cols[3]), int(cols[4]), cols[6]) != (idle, blocks, senders, ttf):
            bad.append(f"{name}: got {cols[1:7]}, want {[idle, blocks, senders, ttf]}")
    if "2 deploy-bearing blocks" not in out:
        bad.append("the non-unique-deploy note is missing")
    if "*** VOID" not in out:
        bad.append("the non-quiescent idle window did not void the primary arm")
    if "expected on this arm" not in out:
        bad.append("the control arm's idle window was not distinguished from a void primary attempt")
    if bad:
        print("SELFTEST FAILED:")
        for b in bad:
            print("  " + b)
        return 1
    print(f"selftest ok: {len(want)} of {len(want)} shapes read as the pre-registration says they must")
    shutil.rmtree(BASE, ignore_errors=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
