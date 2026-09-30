#!/usr/bin/env python3
"""Read a Phase 0 campaign directory and print the three readings, mechanically.

`n127-campaign-preregistration.md` fixes what each reading reports; this computes exactly those and
nothing else, so the results file quotes a program's output rather than a person's reading of a file. It
refuses to guess: a missing artifact is reported as missing, and an empty series as empty, because an
artifact that cannot say what it measured is the defect C176 registers.

Usage: spec/audit/evidence/n127-campaign-summarise.py target/n127-campaign/<tree>-<utc>/
"""

import glob
import os
import re
import sys

# The shape's own edges (search_census::WIDTH_EDGES + the cost's) versus the registry's defaults. The
# distinction is the acceptance row PR #132's fix has to satisfy: if these are the defaults, the
# distribution collapsed into +Inf and nothing may be keyed on it.
SHAPE_EDGES = {8.0, 16.0, 32.0, 64.0, 128.0, 256.0}
DEFAULT_EDGES = {0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 0.75, 1.0, 2.5, 5.0, 7.5, 10.0}

BUCKET = re.compile(r"^rchain_merge_(\w+)_bucket\{le=\"([^\"]+)\"\}\s+(\S+)")
GAUGE = re.compile(r"^rchain_merge_(searches|max_\w+)\s+(\S+)")
STALL_VARIANTS = ("has no layer covering the partition", "supporting stake is not a supermajority",
                  "would publish the fringe that is already finalized")


def read_series(path):
    """-> [(utc, node, height, finalized, alive)] in file order."""
    rows = []
    if not os.path.exists(path):
        return rows
    with open(path) as fh:
        for line in fh:
            if line.startswith("#") or line.startswith("utc\t"):
                continue
            parts = line.rstrip("\n").split("\t")
            if len(parts) < 10:
                continue
            utc, node, _vd, _ad, _td, _anon, _peak, height, fin, alive = parts[:10]
            rows.append((utc, node, height, fin, alive))
    return rows


def rate(series, node):
    """Blocks/minute for a node, from the first and last sample that has a height."""
    pts = [(u, int(h)) for u, n, h, _f, _a in series if n == node and h.isdigit()]
    if len(pts) < 2:
        return None
    first, last = pts[0], pts[-1]
    secs = len(pts)  # one sample a second
    if secs < 2:
        return None
    return (last[1] - first[1]) * 60.0 / secs


def finality(series, node):
    """(first, last, moved_after_the_second_half) finalised heights, 'none' treated as 0."""
    vals = [(u, 0 if f == "none" else int(f)) for u, n, _h, f, _a in series if n == node and (f == "none" or f.isdigit())]
    if not vals:
        return None
    half = len(vals) // 2
    return vals[0][1], vals[-1][1], vals[-1][1] > max(v for _u, v in vals[:half])


def distribution(path):
    """-> dict for one /metrics scrape: edges, cumulative counts, median edge, envelope."""
    if not os.path.exists(path):
        return None
    buckets, gauges, count, total = {}, {}, None, None
    with open(path) as fh:
        for line in fh:
            m = BUCKET.match(line)
            if m:
                name, le, value = m.group(1), m.group(2), float(m.group(3))
                # Keyed by the *number*, not the rendered label: the renderer prints `8` for an edge
                # defined as `8.0`, so a string-keyed lookup silently finds nothing and reports a null
                # median on a perfectly readable histogram. (Found by this file's own self-test.)
                if name == "scope_width":
                    buckets[float(le) if le != "+Inf" else float("inf")] = value
                continue
            m = GAUGE.match(line)
            if m:
                gauges[m.group(1)] = float(m.group(2))
                continue
            m = re.match(r"^rchain_merge_scope_width_(count|sum)\s+(\S+)", line)
            if m:
                if m.group(1) == "count":
                    count = float(m.group(2))
                else:
                    total = float(m.group(2))
    if not buckets:
        return None
    edges = sorted(e for e in buckets if e != float("inf"))
    shape = bool(set(edges) & SHAPE_EDGES)
    default = bool(set(edges) & DEFAULT_EDGES)
    median = None
    if count:
        for e in edges:
            if buckets[e] >= count / 2:
                median = e
                break
    return {
        "edges": edges, "shape_edges": shape, "default_edges": default,
        "median": median, "count": count, "sum": total, "gauges": gauges,
    }


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    out = sys.argv[1]
    print(f"# campaign {out}")
    man = os.path.join(out, "manifest.txt")
    if os.path.exists(man):
        for line in open(man):
            if line.startswith("# "):
                print("  " + line[2:].rstrip())

    # An attempt is reported if *any* of its artifacts exists: a run that produced a distribution but no
    # series is a different failure from one that produced nothing, and the reader should see which.
    attempts = set()
    for pattern, rx in (("series-a*.tsv", r"series-a(\d+)\.tsv$"),
                        ("metrics-*-prekill.txt", r"-a(\d+)-prekill\.txt$"),
                        ("stall-*-a*.txt", r"-a(\d+)\.txt$")):
        for p in glob.glob(os.path.join(out, pattern)):
            m = re.search(rx, p)
            if m:
                attempts.add(int(m.group(1)))
    if not attempts:
        print("no artifacts at all: the run did not start, or wrote nothing")
        return
    attempts = sorted(attempts)

    for a in attempts:
        series = read_series(os.path.join(out, f"series-a{a}.tsv"))
        print(f"\n=== attempt {a} ({len(series)} samples)")
        nodes = sorted({n for _u, n, _h, _f, _av in series})
        for node in nodes:
            r, f = rate(series, node), finality(series, node)
            rtxt = f"{r:.1f} blocks/min" if r is not None else "rate: too few samples"
            ftxt = (f"finality {f[0]} -> {f[1]}" + (" (moved)" if f[2] else " (did not move)")) if f else "finality: none"
            samples = sum(1 for _u, n, _h, _f, _a in series if n == node)
            print(f"  0.1 {node:<9} {samples:>4} samples  {rtxt:<22} {ftxt}")

        for node in sorted({re.search(r"metrics-(.+)-a\d+-prekill\.txt$", p).group(1)
                            for p in glob.glob(os.path.join(out, f"metrics-*-a{a}-prekill.txt"))}):
            d = distribution(os.path.join(out, f"metrics-{node}-a{a}-prekill.txt"))
            if d is None:
                print(f"  0.2 {node:<9} no pre-kill scrape, or SCRAPE FAILED (see the artifact)")
                continue
            edges = "shape's" if d["shape_edges"] and not d["default_edges"] else (
                "REGISTRY DEFAULTS — the distribution is void" if d["default_edges"] else "unknown")
            env = d["gauges"]
            print(f"  0.2 {node:<9} edges={edges} median={d['median']} merges={d['count']:.0f} "
                  f"widest={env.get('max_scope_width')} pairs={env.get('max_conflict_pairs')} "
                  f"asym={env.get('max_asymmetric_pairs')} states={env.get('max_states_expanded')}")

        found = {}
        for path in sorted(glob.glob(os.path.join(out, f"stall-*-a{a}.txt"))):
            with open(path) as fh:
                for line in fh:
                    for v in STALL_VARIANTS:
                        if v in line:
                            found[v] = found.get(v, 0) + 1
        if found:
            for v, c in sorted(found.items(), key=lambda kv: -kv[1]):
                print(f"  0.3 {v!r} x{c}")
        else:
            print("  0.3 no stall line in this attempt (void for this reading)")


if __name__ == "__main__":
    main()
