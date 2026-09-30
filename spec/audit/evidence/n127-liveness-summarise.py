#!/usr/bin/env python3
"""Read a liveness run directory and print the pre-registered rows, mechanically.

`n127-liveness-preregistration.md` fixes the reading: **the finalised height at T+120 (the kill) and at
T+300, per node per attempt, beside the height at the same instants.** This computes exactly those, plus the
one quantity the pre-registration did not name and the run turned out to need — **the instant finality
stopped**, which is not the kill — and nothing else, so the results file quotes a program rather than a
person's reading of a file.

It exists because the harness computed these rows at run time and printed them to a terminal: the numbers in
the tracker and in the register came from someone reading `target/`, which is gitignored, and the series they
were read from were never committed. An instrument whose output is not an artifact is C176's class.

Refuses to guess: a missing series is reported as missing, and a node with no sample at the kill instant is
reported as such rather than interpolated.

Usage: n127-liveness-summarise.py <rundir> [<rundir> ...]
"""

import glob
import os
import re
import sys

KILL_AT_DEFAULT = 120
STALL = re.compile(r"(\d{2}:\d{2}:\d{2})\S*\s+WARN.*finality did not advance at tip (\d+): (.*)")
SUPPORT = re.compile(r"— (\d+) of (\d+) \((\d+) full partition\(s\) among (\d+) candidate\(s\)\)")


def secs(t):
    h, m, s = (int(x) for x in t.split(":"))
    return h * 3600 + m * 60 + s


def rows(path):
    """-> [(t, node, height, finalized, alive)] in file order, None for a blank/non-numeric cell."""
    out = []
    with open(path) as fh:
        for line in fh:
            if line.startswith("#") or line.startswith("utc\t"):
                continue
            p = line.rstrip("\n").split("\t")
            if len(p) < 10:
                continue
            def num(v):
                return int(v) if v.isdigit() else None
            out.append((p[0], p[1], num(p[7]), num(p[8]), num(p[9])))
    return out


def kill_offset(manifest):
    for line in manifest:
        m = re.search(r"kill=\S* at T\+(\d+)s", line)
        if m:
            return int(m.group(1))
    return KILL_AT_DEFAULT


def freeze_instant(pts, key):
    """The last instant the value moved, and how long it had been frozen by the end.

    A pin is not "finality is behind the height" — it is "finality stopped moving while time ran". The
    quantity that separates them is when it last moved, so that is what is reported.
    """
    moved = [(t, v) for t, v in pts if v is not None]
    if not moved:
        return None
    last_t, last_v = moved[0]
    for t, v in moved:
        if v != last_v:
            last_t, last_v = t, v
    return last_t, last_v, moved[-1][0] - last_t


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
        kill = kill_offset(manifest)
        attempts = sorted(
            int(m.group(1))
            for p in glob.glob(os.path.join(out, "series-a*.tsv"))
            if (m := re.search(r"series-a(\d+)\.tsv$", p))
        )
        if not attempts:
            print("  no series at all: the run did not start, or wrote nothing")
            continue

        for a in attempts:
            series = rows(os.path.join(out, f"series-a{a}.tsv"))
            if not series:
                print(f"  attempt {a}: series present but empty")
                continue
            t0 = secs(series[0][0])
            span = secs(series[-1][0]) - t0
            print(f"  attempt {a} — window {span:.0f}s, kill scheduled at T+{kill}s")
            for node in sorted({r[1] for r in series}):
                pts = [(secs(r[0]) - t0, r[2], r[3]) for r in series if r[1] == node]
                at_kill = max((p for p in pts if p[0] <= kill), key=lambda p: p[0], default=None)
                end = pts[-1]
                fr = freeze_instant([(p[0], p[2]) for p in pts], None)
                def s(p, i):
                    return "?" if p is None or p[i] is None else str(p[i])
                print(
                    f"    {node:<9} height {s(at_kill,1)}@{kill}s -> {s(end,1)}@{int(end[0])}s"
                    f"   finality {s(at_kill,2)}@{kill}s -> {s(end,2)}@{int(end[0])}s"
                    f"   finality last moved at T+{int(fr[0])}s, frozen {int(fr[2])}s"
                    if fr
                    else f"    {node:<9} no numeric finality sample"
                )

            stalls = []
            for path in sorted(glob.glob(os.path.join(out, f"stall-*-a{a}.txt"))):
                for line in open(path):
                    m = STALL.search(line)
                    if m:
                        stalls.append((secs(m.group(1)) - t0, int(m.group(2)), m.group(3)))
            if not stalls:
                print("    stalls: none in this attempt (void for this reading)")
                continue
            pre = [s for s in stalls if s[0] <= kill]
            print(
                f"    stalls: {len(stalls)} lines, {len(pre)} before the kill and "
                f"{len(stalls) - len(pre)} after it"
            )
            for label, group in (("before", pre), ("after", stalls[len(pre):])):
                counts = {}
                for _t, _tip, text in group:
                    m = SUPPORT.search(text)
                    key = f"{m.group(1)} of {m.group(2)} ({m.group(3)} full among {m.group(4)})" if m else text
                    counts[key] = counts.get(key, 0) + 1
                for key, c in sorted(counts.items(), key=lambda kv: -kv[1]):
                    print(f"      {label:<6} x{c:<3} {key}")


if __name__ == "__main__":
    main()
