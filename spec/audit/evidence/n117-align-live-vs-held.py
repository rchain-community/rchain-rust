#!/usr/bin/env python3
"""Join jemalloc's accounting to the cgroup's, and apply the pre-registered verdict bands.

Two 1 Hz series from two independent mechanisms:

  * `anon`  — the cgroup's `memory.stat`, by the queue-depth sampler, stamped `HH:MM:SS` UTC;
  * jemalloc's `allocated`/`active`/`resident`/`mapped`/`retained`, by the LD_PRELOAD shim, stamped in
              epoch seconds.

Neither can see what the other sees: the cgroup counts every resident page and attributes it to nothing;
jemalloc attributes every byte it owns and cannot see anything it does not own. #117 turns on the
difference between them, read at the moment the node holds the most.

Verdict bands, fixed before the run (a = anon at peak, A = allocated, R = resident):
    A/a >= 0.7                        LIVE
    A/a <  0.3  and  R/a >= 0.7       HELD
    otherwise                         AMBIGUOUS

If the shim could not resolve `mallctl`, its header says so and this script reports NO DATA rather than
printing zeros — the failure mode that made two earlier instruments look like negative results.

usage: align-live-vs-held.py <YYYY-MM-DD> <live-vs-held-attempt<N>.tsv>... <stats-<node>-a<N>.txt>...
"""
import datetime
import re
import sys

MIB = 1 / 1048576
run_date = sys.argv[1]
rest = sys.argv[2:]
anon_files = [a for a in rest if a.endswith(".tsv")]
stats_files = [a for a in rest if a.endswith(".txt")]


def epoch_of(hhmmss, date):
    t = datetime.datetime.strptime(hhmmss, "%H:%M:%S")
    d = datetime.datetime.strptime(date, "%Y-%m-%d")
    return int((d.replace(hour=t.hour, minute=t.minute, second=t.second)
                .replace(tzinfo=datetime.timezone.utc)).timestamp())


def read_anon(path, date):
    out = {}
    for line in open(path):
        if line.startswith("#"):
            continue
        f = line.rstrip("\n").split("\t")
        if len(f) < 6 or f[0] == "utc" or not f[5]:
            continue
        out.setdefault(f[1], {})[epoch_of(f[0], date)] = float(f[5])
    return out


def read_stats(path):
    header, rows = [], {}
    for line in open(path, errors="replace"):
        if line.startswith("#"):
            header.append(line.rstrip())
            continue
        f = line.split()
        if len(f) >= 7:
            try:
                rows[int(float(f[0]))] = [int(x) * MIB for x in f[1:7]]
            except ValueError:
                pass
    return header, rows


anon = {}
for path in anon_files:
    a_num = re.search(r"-attempt(\d+)\.tsv$", path)
    if not a_num:
        continue
    for node, series in read_anon(path, run_date).items():
        anon[(a_num.group(1), node)] = series
print(f"anon series: {sum(len(v) for v in anon.values())} samples over {len(anon)} (attempt, node) "
      f"pairs ({run_date})\n")

name_re = re.compile(r"stats-(?P<node>[a-z0-9]+)-a(?P<attempt>\d+)\.txt$")
print(f"{'att':>3} {'node':10} {'anon':>8} {'alloc':>9} {'active':>8} {'resident':>9} {'mapped':>9} "
      f"{'retain':>7} {'A/a':>6} {'R/a':>6}  verdict")
for path in sorted(stats_files, key=lambda p: (re.search(r"-a(\d+)", p).group(1),
                                               re.search(r"stats-([a-z0-9]+)-", p).group(1))):
    m = name_re.search(path)
    if not m:
        continue
    node, attempt = m.group("node"), m.group("attempt")
    header, rows = read_stats(path)
    if not rows:
        print(f"{attempt:>3} {node:10} {'--':>8}  NO SHIM DATA (header: "
              f"{header[0] if header else 'empty file'})")
        continue
    series = anon.get((attempt, node), {})
    if not series:
        print(f"{attempt:>3} {node:10} {'--':>8}  no anon samples for this node")
        continue
    peak_t = max(series, key=lambda t: series[t])
    a = series[peak_t]
    near = min(rows, key=lambda t: abs(t - peak_t))
    if abs(near - peak_t) > 5:
        print(f"{attempt:>3} {node:10} {a:8.0f}  shim sample is {abs(near-peak_t)} s from the anon peak "
              f"— too far to join")
        continue
    alloc, active, resident, mapped, retained, _meta = rows[near]
    ratio_a, ratio_r = (alloc / a if a else 0), (resident / a if a else 0)
    if ratio_a >= 0.7:
        verdict = "LIVE"
    elif ratio_a < 0.3 and ratio_r >= 0.7:
        verdict = "HELD"
    else:
        verdict = "AMBIGUOUS"
    print(f"{attempt:>3} {node:10} {a:8.0f} {alloc:9.1f} {active:8.1f} {resident:9.1f} {mapped:9.1f} "
          f"{retained:7.1f} {ratio_a:6.3f} {ratio_r:6.3f}  {verdict}")
