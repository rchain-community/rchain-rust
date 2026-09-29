#!/usr/bin/env python3
"""Attribute dhat heap-profile allocation to the crate that owns each program point.

**Why this file exists.** The figure "`rchain_casper` allocated 8,796.1 MiB before and 1,126.0 MiB
after" — the load-bearing leg of the claim that the #117 fix reduced allocation churn — was computed
by a rule that lived only in an interactive shell session. An audit gate then tried ~70 documented
attribution rules and could not reproduce it, and correctly called that out: a number whose derivation
rule is not recorded is prose, not a measurement, by this repository's own standard
(`spec/TEST-COVERAGE.md:412-416`).

The rule is stated here so the figure is re-derivable by anyone, and the numbers below are the ones the
rule produces. Two of them are quoted in `spec/audit/evidence/n117-preregistration.md` and in
`casper/src/merging.rs`; if this script ever stops producing them, the citation is wrong, not the
script.

**The rule, precisely** (it is one of several defensible ones, and different rules give different
totals — the choice is what has to be recorded, not the number):

- Each `pps[]` entry is one program point; its `fs` lists frame indices into the top-level `ftbl`.
- Frames are traversed **in reverse** of their listed order (`reversed(fs)`), i.e. from the outermost
  towards the innermost caller.
- Each frame name is cleaned by stripping dhat's `0x<hex>: ` address prefix.
- The **first** frame, in that reversed traversal, whose text before the first `::` is not one of the
  standard-library prefixes (`alloc`, `core`, `std`) and does not begin with `[`, `<` or `0x`, names
  the owning crate.
- Each program point's `tb` (total bytes ever allocated at that site) is credited to its owning crate.
- A program point with no qualifying frame is `unknown`.

Usage: `python3 spec/audit/evidence/dhat-crate-churn.py <profile.json> [<profile.json> ...]`
"""

import collections
import json
import re
import sys

MIB = 1 / 1048576
STD_PREFIXES = ("alloc", "core", "std")


def clean(frame: str) -> str:
    return re.sub(r"^0x[0-9a-f]+: ", "", frame)


def owner(pp, ftbl) -> str:
    for frame in reversed([clean(ftbl[i]) for i in pp.get("fs", [])]):
        head = frame.split("::")[0]
        if head and not head.startswith(STD_PREFIXES) and not head.startswith(("[", "<", "0x")):
            return head
    return "unknown"


def report(path: str) -> None:
    with open(path) as fh:
        profile = json.load(fh)
    ftbl, pps = profile["ftbl"], profile["pps"]
    by_crate: collections.Counter = collections.Counter()
    for pp in pps:
        total = pp.get("tb", 0)
        if total:
            by_crate[owner(pp, ftbl)] += total
    grand = sum(by_crate.values())
    print(f"{path}")
    print(f"  mode={profile.get('mode')}  program points={len(pps)}")
    print(f"  total allocated = {grand * MIB:,.1f} MiB")
    for crate, total in by_crate.most_common():
        print(f"    {total * MIB:10.1f} MiB  ({100 * total / grand:4.1f}%)  {crate}")


if __name__ == "__main__":
    for arg in sys.argv[1:]:
        report(arg)
        print()
