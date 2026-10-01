#!/usr/bin/env python3
"""Compute #149's pre-registered rows, and nothing else.

`n149-preregistration.md` fixes four quantities per attempt — `blocks_per_deploy`, `senders`,
`time_to_finality` and the idle control. This is the one implementation of them, so the number in the
results file is quoted from a program rather than read off a terminal (the defect C176 registers).

Usage:  python3 spec/audit/evidence/n149-summarise.py <run-dir>
"""

import os
import re
import sys


def read_marks(path):
    marks = {}
    with open(path) as fh:
        for line in fh:
            if "\t" in line:
                k, v = line.rstrip("\n").split("\t", 1)
                marks[k] = v
    return marks


def read_blocks(path):
    rows = []
    with open(path) as fh:
        for line in fh:
            if line.startswith("#") or not line.strip():
                continue
            t, num, sender, dc, h = line.rstrip("\n").split("\t")
            rows.append({"epoch": int(t), "number": int(num), "sender": sender,
                         "deploy_count": int(dc), "hash": h})
    return rows


def read_series(path):
    """node -> [(epoch, finalized)] in sample order, finalised numbers only."""
    out = {}
    with open(path) as fh:
        for line in fh:
            if line.startswith("#") or not line.strip() or line.startswith("utc\t"):
                continue
            parts = line.rstrip("\n").split("\t")
            if len(parts) != 6:
                continue
            _utc, epoch, node, _height, fin, _alive = parts
            if fin.isdigit():
                out.setdefault(node, []).append((int(epoch), int(fin)))
    return out


def time_to_finality(series, deploy_epoch, target_number):
    """The first second at which **every** sampled node reports finality at or past the target.

    A node missing from `series` is not a node that agreed, and a gap in sampling is not a sample: the
    search walks the sampled seconds and asks each node only for seconds it actually has, so a hole
    makes the answer later than the truth rather than earlier.
    """
    if not series:
        return None
    nodes = sorted(series)
    epochs = sorted({e for rows in series.values() for e, _f in rows if e >= deploy_epoch})
    for e in epochs:
        ok = True
        for n in nodes:
            vals = [f for se, f in series[n] if se <= e and se >= deploy_epoch - 1]
            if not vals or max(vals) < target_number:
                ok = False
                break
        if ok:
            return e - deploy_epoch
    return None


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else "target/n149-blocks"
    runs = sorted(d for d in os.listdir(out) if re.match(r"^n\d+-\w+-a\d+$", d))
    print(f"run dir: {out}")
    print(f"{'run':<18}{'idle':>6}{'deploys':>9}{'blocks':>8}{'senders':>9}{'ttf_s':>7}  note")
    rows = []
    for r in runs:
        d = os.path.join(out, r)
        marks_p, blocks_p, series_p = (os.path.join(d, f) for f in
                                       ("marks.tsv", "blocks.tsv", "series.tsv"))
        if not all(os.path.exists(p) for p in (marks_p, blocks_p, series_p)):
            print(f"{r:<18}{'—':>6}{'':>9}{'':>8}{'':>9}{'':>7}  MISSING ARTIFACT")
            continue
        m = read_marks(marks_p)
        deploy_epoch = int(m["idle_end_and_deploy"])
        settle_end = int(m["settle_end"])
        read_end = int(m["read_end"])
        blocks = read_blocks(blocks_p)

        idle = [b for b in blocks if settle_end <= b["epoch"] < deploy_epoch]
        post = [b for b in blocks if b["epoch"] >= deploy_epoch]
        carriers = [b for b in post if b["deploy_count"] >= 1]
        note = ""
        if len(carriers) != 1:
            note = f"{len(carriers)} deploy-bearing blocks (expected 1)"
        if any(b["epoch"] > read_end for b in blocks):
            note = (note + "; " if note else "") + "blocks first seen after the window"

        senders = len({b["sender"] for b in post})
        target = carriers[0]["number"] if carriers else None
        ttf = None
        if target is not None:
            ttf = time_to_finality(read_series(series_p), deploy_epoch, target)

        print(f"{r:<18}{len(idle):>6}{len(carriers):>9}{len(post):>8}{senders:>9}"
              f"{('never' if ttf is None else ttf):>7}  {note}")
        rows.append({"run": r, "idle": len(idle), "carriers": len(carriers),
                     "blocks": len(post), "senders": senders, "ttf": ttf})

    print()
    print("by N (primary arm = noauto):")
    by_n = {}
    for row in rows:
        mm = re.match(r"^n(\d+)-(\w+)-a\d+$", row["run"])
        by_n.setdefault((int(mm.group(1)), mm.group(2)), []).append(row)
    for (n, arm) in sorted(by_n):
        rs = by_n[(n, arm)]
        blocks = [r["blocks"] for r in rs]
        senders = [r["senders"] for r in rs]
        ttfs = ["never" if r["ttf"] is None else str(r["ttf"]) for r in rs]
        print(f"  N={n:<3} arm={arm:<7} attempts={len(rs)}  blocks={blocks} senders={senders} ttf_s={ttfs}")
        if any(r["idle"] for r in rs):
            print(f"    *** VOID: a run had {[r['idle'] for r in rs]} idle-window blocks — the deploy "
                  f"was not the only driver")


if __name__ == "__main__":
    main()
