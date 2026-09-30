#!/usr/bin/env python3
"""Rank a jemalloc heap dump by live bytes, and name the allocating stacks.

Dump format (jemalloc 5.3, `heap_v2/<lg_quantum>`):
    @ 0x… 0x… 0x…        <- one frame-address list per entry, space-separated
      t*: <MiB>: <count> [<deallocated MiB>: <count>]
      t<N>: …

The first value is in units of 2^lg_quantum bytes — here 1 MiB — so `t*: 4571` is 4571 MiB live, which
cross-checks against the shim's own `PROF_DUMP … alloc_mib=` line taken at the same instant.

Addresses are runtime addresses (the binary is PIE), so the executable's load base must be subtracted
before symbolising. The base is recorded by the shim next to the dump; the node is PID 1 in the container,
so its header is the first one in the file — every helper process the image runs also gets the preload and
writes its own, with its own ASLR base.

usage: parse-jemalloc-dump.py <dump> <binary> <exe_base_hex> [top_n]
"""
import re
import subprocess
import sys

MIB = 1048576
dump, binary = sys.argv[1], sys.argv[2]
base = int(sys.argv[3], 16)
top_n = int(sys.argv[4]) if len(sys.argv) > 4 else 8

entries = []
frames = None
tot = None
for line in open(dump, errors="replace"):
    if line.startswith("@"):
        frames = [int(a, 16) for a in line[1:].split() if a.startswith("0x")]
        continue
    m = re.match(r"\s+t\*:\s+(\d+):\s+(\d+)", line)
    if m and frames is not None:
        mib, count = int(m.group(1)), int(m.group(2))
        entries.append((mib, count, frames))
        frames = None
        continue
    m = re.match(r"\s+t\*:\s+(\d+):", line)
    if m and frames is None:
        tot = int(m.group(1))

print(f"{dump}\n  executable base 0x{base:x}  (the node is PID 1; its header is the first in the stats file)")
print(f"  stacks in dump: {len(entries)}   total live: {tot if tot is not None else '?'} MiB")
print(f"  live bytes accounted by these stacks: {sum(e[0] for e in entries)} MiB\n")

entries.sort(key=lambda e: -e[0])
for mib, count, f in entries[:top_n]:
    offs = [f"0x{a - base:x}" for a in f if a - base > 0]
    syms = []
    if offs:
        out = subprocess.run(["addr2line", "-Cfipe", binary, *offs[:14]],
                             capture_output=True, text=True).stdout.splitlines()
        seen = set()
        for o, s in zip(offs[:14], out):
            name = s.split(" at ")[0]
            if name in seen or not name or name == "??":
                continue
            seen.add(name)
            syms.append(f"      {o}  {name[:120]}")
    print(f"  {mib:>7} MiB  samples={count}")
    print("\n".join(syms[:8]))
    print()
