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

# The shape's own edges are **read out of the source that publishes them**, not written down here. The
# previous version of this file carried `{8, 16, 32, 64, 128, 256}` labelled as the shape's — which was the
# endpoint's old *hand-written* superset, the defect PR #132 removed — and tested it by **intersection**, so
# a published set sharing one edge with it passed. An acceptance row that cannot fail on the axis it exists
# for is the defect this file was written to avoid, one level up.
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
RUST = os.path.join(ROOT, "casper", "src", "merging.rs")


def shape_edges(const):
    """`WIDTH_EDGES`/`EXPANDED_EDGES` as a set of floats, from `search_census`'s own declarations."""
    m = re.search(rf"const {const}: \[usize; \d+\] = \[([^\]]*)\]", open(RUST).read())
    if not m:
        sys.exit(f"cannot find {const} in {RUST}: the edges are not derivable, so no row may be checked")
    return {float(v.replace("_", "")) for v in m.group(1).split(",")}


SHAPE_EDGES = shape_edges("WIDTH_EDGES")
# The registry's defaults, which the published edges must *not* be: if these are what came out, the
# distribution collapsed into a range that says nothing about chains.
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
    """**Heights**/minute for a node, from the first and last sample that has a height.

    It is *not* a block rate, and the name and the printed label said it was from the day this was
    written — "Blocks/minute … from … a height" in one sentence, with the body reading `h`. The
    quantity is a `latestBlockNumber` delta, and `latestBlockNumber` is `max_height + 1`, so what this
    returns is a **round** rate: one height is one round, and a round carries one block per bonded
    sender (measured at 2.99 blocks per height on a 3-validator net, `n149-results.md`).

    Every rate in `n127-campaign-results.md` and every document quoting them inherits this, which is
    why the campaign's "12–16 blocks/min" is a height figure. Converting is not a multiplication by 3
    for those runs: the block-per-height ratio was never recorded, the series here keeps only
    `latestBlockNumber`, and the sender count differs between arms. So the honest repair is to label
    the number for what it is and let a block-level instrument re-derive it — which is what
    `n149-sample.py`'s block-hash union does.
    """
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
    # **Equality, not intersection**: the published boundary set must *be* the shape's, because the
    # acceptance row is "the histogram's boundaries are the shape's own". A superset or a subset is a
    # different rendering of the same quantity, which is exactly what went unnoticed once already.
    published = set(edges)
    shape = published == SHAPE_EDGES
    default = bool(published & DEFAULT_EDGES)
    median = None
    if count:
        for e in edges:
            if buckets[e] >= count / 2:
                median = e
                break
    return {
        "edges": edges, "shape_edges": shape, "default_edges": default,
        "extra": sorted(published - SHAPE_EDGES), "missing": sorted(SHAPE_EDGES - published),
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
            # `heights/min`, not `blocks/min` — see `rate`'s docstring. The label is the whole defect:
            # every number below was read as a block rate for two days.
            rtxt = f"{r:.1f} heights/min" if r is not None else "rate: too few samples"
            ftxt = (f"finality {f[0]} -> {f[1]}" + (" (moved)" if f[2] else " (did not move)")) if f else "finality: none"
            samples = sum(1 for _u, n, _h, _f, _a in series if n == node)
            print(f"  0.1 {node:<9} {samples:>4} samples  {rtxt:<22} {ftxt}")

        for node in sorted({re.search(r"metrics-(.+)-a\d+-prekill\.txt$", p).group(1)
                            for p in glob.glob(os.path.join(out, f"metrics-*-a{a}-prekill.txt"))}):
            d = distribution(os.path.join(out, f"metrics-{node}-a{a}-prekill.txt"))
            if d is None:
                print(f"  0.2 {node:<9} no pre-kill scrape, or SCRAPE FAILED (see the artifact)")
                continue
            edges = "the shape's own" if d["shape_edges"] else (
                f"MISMATCH — extra {d['extra']}, missing {d['missing']}"
                + (" (the registry's defaults: void)" if d["default_edges"] else ""))
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
