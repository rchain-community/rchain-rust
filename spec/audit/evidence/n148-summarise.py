#!/usr/bin/env python3
"""Read an #148 probe run directory and print the pre-registered rows, mechanically.

`n148-preregistration.md` fixes the reading: **height and finalised height at T+120 (the kill), at
T+180 (the single deploy), and at the end of the window, per node per attempt**, plus the height gained
after the deploy and its rate — because #148's claim is about *production running away while finality
does not move*, and those are the two columns that show it.

The parsing is the sibling's, imported rather than copied, so there is one implementation of "read a
series" and one of "when did it stop moving" (C176's class; and law 51b's `Split` — a reading computed
two ways is two readings).

Refuses to guess: a missing series is reported as missing, and a node with no sample at an instant is
reported as such rather than interpolated.

Usage: n148-summarise.py <rundir> [<rundir> ...]
"""

import glob
import importlib.util
import os
import re
import sys

_spec = importlib.util.spec_from_file_location(
    "n127ls", os.path.join(os.path.dirname(os.path.abspath(__file__)), "n127-liveness-summarise.py")
)
n127ls = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(n127ls)


def deploy_offset(manifest):
    """The T+ offset of the post-kill deploy, from the manifest line the harness writes for #148."""
    for line in manifest:
        m = re.search(r"after the kill: (\d+) at T\+(\d+)s", line)
        if m:
            return (int(m.group(2)) if int(m.group(1)) > 0 else None)
    return None


def at(pts, t):
    """The sample in force at T+t — the last one at or before t, or None if the series starts later."""
    return max((p for p in pts if p[0] <= t), key=lambda p: p[0], default=None)


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    for out in sys.argv[1:]:
        print(f"# run {out}")
        man_path = os.path.join(out, "manifest.txt")
        manifest = open(man_path).read().splitlines() if os.path.exists(man_path) else []
        for line in manifest:
            if line.startswith("# "):
                print("  " + line[2:])
        kill = n127ls.kill_offset(manifest)
        deploy = deploy_offset(manifest)
        if deploy is None:
            print("  no post-kill deploy in this run (the arm was off) — nothing to read here")
            continue
        attempts = sorted(
            int(m.group(1))
            for p in glob.glob(os.path.join(out, "series-a*.tsv"))
            if (m := re.search(r"series-a(\d+)\.tsv$", p))
        )
        if not attempts:
            print("  no series at all: the run did not start, or wrote nothing")
            continue

        for a in attempts:
            series = n127ls.rows(os.path.join(out, f"series-a{a}.tsv"))
            if not series:
                print(f"  attempt {a}: series present but empty")
                continue
            t0 = n127ls.secs(series[0][0])
            span = n127ls.secs(series[-1][0]) - t0
            print(
                f"  attempt {a} — window {span:.0f}s, kill scheduled at T+{kill}s, "
                f"deploy at T+{deploy}s" + ("" if deploy < span else "  *** deploy after the window: VOID")
            )
            for node in sorted({r[1] for r in series}):
                pts = [(n127ls.secs(r[0]) - t0, r[2], r[3]) for r in series if r[1] == node]
                bef, dep, end = at(pts, deploy - 1), at(pts, deploy), pts[-1]

                def s(p, i):
                    return "?" if p is None or p[i] is None else str(p[i])

                gained = "?"
                rate = "?"
                if dep and end and dep[1] is not None and end[1] is not None and end[0] > dep[0]:
                    gained = str(end[1] - dep[1])
                    rate = f"{(end[1] - dep[1]) / (end[0] - dep[0]):.2f}/s"
                fr = n127ls.freeze_instant([(p[0], p[2]) for p in pts], None)
                print(
                    f"    {node:<9} height  {s(bef,1)}@{int(bef[0]) if bef else '?'}s"
                    f" -> {s(dep,1)}@{int(dep[0]) if dep else '?'}s(deploy)"
                    f" -> {s(end,1)}@{int(end[0])}s"
                    f"   |  finality {s(bef,2)} -> {s(dep,2)} -> {s(end,2)}"
                    f"   |  after deploy: +{gained} at {rate}"
                )
                if fr:
                    print(
                        f"    {'':<9} finality last moved at T+{int(fr[0])}s "
                        f"(value {fr[1]}), frozen {int(fr[2])}s by the end"
                    )


if __name__ == "__main__":
    main()
