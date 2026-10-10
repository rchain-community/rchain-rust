#!/usr/bin/env python3
"""Offline full-script reconciliation regressions with deterministic API fixtures.

Each case slices one section out of `tools/reconcile-network.sh` by its own banner and runs it with the
API surface stubbed, so the assertion is about the *section's* arithmetic rather than about a live net.
The sections are located by exact banner text, so a banner that moves breaks a case here rather than
passing silently — the anchors are listed at the end of the file."""
import json
import os
from pathlib import Path
import subprocess
import tempfile

SCRIPT = Path(__file__).resolve().parents[1] / "tools/reconcile-network.sh"
def run_case(name, finalized, expected, required):
    # Exercise the production anchor selection in isolation with injected API snapshots.
    source = SCRIPT.read_text()
    a = source.index('echo "== 3. finalized anchor')
    b = source.index("# --- 4. what is above", a)
    fragment = source[a:b]
    shell = """set -uo pipefail
NAMES=(A B)
declare -A LFB LFH
LFB[A]="$A_HEIGHT"; LFH[A]="$A_HASH"
LFB[B]="$B_HEIGHT"; LFH[B]="$B_HASH"
""" + fragment
    def fields(v):
        if isinstance(v, dict):
            return str(v.get("blockNumber", "")), v.get("blockHash", "")
        return "", ""
    ah, ab = fields(finalized[0]); bh, bb = fields(finalized[1])
    env = {**os.environ, "A_HEIGHT": ah, "A_HASH": ab, "B_HEIGHT": bh, "B_HASH": bb}
    p = subprocess.run(["bash", "-c", shell], env=env, capture_output=True, text=True, timeout=10)
    combined = p.stdout + p.stderr
    assert p.returncode == expected, f"{name}: exit={p.returncode}, expected={expected}\n{combined}"
    assert required in combined, f"{name}: missing {required!r}\n{combined}"
    print(f"PASS {name} (exit {p.returncode})")

if __name__ == "__main__":
    anchor = {"blockNumber": 0, "blockHash": "anchor"}
    run_case("unanimous anchor", [anchor, anchor], 0, "unanimously reported finalized anchor")
    run_case("divergent finalized hashes",
             [anchor, {"blockNumber": 0, "blockHash": "other"}],
             4, "finalized heads differ")
    run_case("unavailable finality",
             [anchor, "Finalized fringe is not available."],
             4, "no usable finalized block")
    run_case("same hash at different finalized heights",
             [anchor, {"blockNumber": 1, "blockHash": "anchor"}],
             4, "finalized heads differ")
    # Same first block but a different second block must not be reported as convergence.
    # Verification is behind --apply, so test the actual set-comparison fragment below.
    fragment = SCRIPT.read_text()
    start = fragment.index('  for (( h=MEET; h<=MAXH; h++ )); do', fragment.index('== 9. verification'))
    end = fragment.index('  echo "  [$i]', start)
    verify = fragment[start:end]
    with tempfile.TemporaryDirectory(prefix="reconcile-dag-") as d:
        for case, left, right, expected in [
            ("equal sets different order", "a\\nb", "b\\na", 1),
            ("extra sibling block", "a\\nb", "a", 0),
            ("different second block", "a\\nb", "a\\nc", 0),
            ("missing height", "a", "", 0),
        ]:
            script = """set -uo pipefail
MEET=0; MAXH=0; ok=1
NAMES=(A B)
declare -A HOST PORT
HOST[A]=A; HOST[B]=B; PORT[A]=1; PORT[B]=2
blocks_at() { if [ "$1" = A ]; then printf '%b\\n' "$LEFT"; else printf '%b\\n' "$RIGHT"; fi; }
""" + verify + "\nprintf '%s' \"$ok\"\n"
            p = subprocess.run(["bash", "-c", script], env={**os.environ, "LEFT": left, "RIGHT": right},
                               text=True, capture_output=True, timeout=10)
            assert p.returncode == 0 and p.stdout == str(expected), (case, p.stdout, p.stderr)
            print(f"PASS {case}")

    # --- section 2: the report fires, and nothing is consumed on it ------------------------------
    # The equivocation report had never been exercised — the drill's own note says so — and after the
    # #308 rename the section's contract is that it *prints* and decides nothing. Two distinct blocks
    # by one sender at one height is the shape it looks for; the assertion is that the report appears
    # and that no line claims a stake was dropped, reweighted or decided.
    src = SCRIPT.read_text()
    # The slice must start at the declarations above the banner, not at the banner: `REPORTED` is an
    # associative array, and a fragment that begins below the `declare -A` re-reads it as an
    # arithmetic index — the failure mode this slice-by-marker style has.
    a = src.index("declare -A REPORTED=() REPORTED_WHY=()")
    b = src.index("# --- 3. conservative finalized anchor", a)
    section2 = src[a:b]
    script = """set -uo pipefail
MAXH=1
NAMES=(A)
declare -A HOST PORT
HOST[A]=A; PORT[A]=1
blocks_at() { printf '%s\\n' 'aa11 sender9 post 1 []' 'bb22 sender9 post 1 []'; }
""" + section2
    p = subprocess.run(["bash", "-c", script], capture_output=True, text=True, timeout=10)
    assert p.returncode == 0, p.stdout + p.stderr
    assert "REPORTED sender9" in p.stdout, p.stdout
    assert "two distinct blocks at height 0" in p.stdout, p.stdout
    assert "no stake is weighed, dropped or decided" in p.stdout, p.stdout
    print("PASS section 2 reports an equivocation and consumes nothing")

    # --- section 4: one block served by several nodes is one block -------------------------------
    # RED before the fix: three records for two blocks and a total of 8, because the section walked
    # every node's answer and counted each *observation* — the #302 double-count, one section down.
    # `/api/blocks/h/h` is answered per node, so this is what a four-validator net does at every
    # height: each node serves the sibling blocks it holds, and the same block comes back more than
    # once. The stub is that shape in miniature: A serves two blocks, B serves one of A's and one of
    # its own.
    a = src.index('echo "== 4. what is above the point =="')
    b = src.index('if [ "$APPLY" != "1" ]; then', a)
    section4 = src[a:b]
    with tempfile.TemporaryDirectory(prefix="reconcile-s4-") as d:
        report = os.path.join(d, "deploys.jsonl")
        script = """set -uo pipefail
MEET=0; MAXH=1; MASTER=A
NAMES=(A B)
declare -A HOST PORT
HOST[A]=A; PORT[A]=1; HOST[B]=B; PORT[B]=2
blocks_at() {   # the five fields the section reads, last one never empty
  [ "$3" = "1" ] || return 0
  printf '%s\\n' 'h1 sender1 post1 3 ["deadbeef"]'
  [ "$1" = B ] && printf '%s\\n' 'h2 sender2 post2 2 []'
  return 0
}
""" + section4
        p = subprocess.run(["bash", "-c", script], env={**os.environ, "RECONCILE_REPORT": report},
                           capture_output=True, text=True, timeout=10)
        assert p.returncode == 0, f"section 4 exited {p.returncode}\n{p.stdout}{p.stderr}"
        recs = [json.loads(l) for l in Path(report).read_text().splitlines() if l.strip()]
        assert len(recs) == 2, f"two unique blocks served by two nodes produced {len(recs)} records"
        by_hash = {r["blockHash"]: r for r in recs}
        assert by_hash["h1"]["observers"] == ["A", "B"], by_hash["h1"]
        assert by_hash["h2"]["observers"] == ["B"], by_hash["h2"]
        assert by_hash["h1"]["rejectedDeploys"] == ["deadbeef"], by_hash["h1"]
        # 3 + 2, once each — the observation count of the same record is 3 answers, and the old
        # section totalled 8 by adding h1's deploys a second time.
        assert "5 deploy(s) above the point across 2 unique block(s)" in p.stdout, p.stdout
        print("PASS section 4 counts a block once and names the nodes that served it")

    # --- section 10: it must assert, not print ---------------------------------------------------
    # RED before the fix on every negative case below: the old section printed each node's finalised
    # block and exited 0 whatever it said — which is exactly what the committed drill transcript
    # (`spec/audit/evidence/n-reconcile-drill/restore-run.txt`) shows, four `finalised ?` lines and a
    # zero exit, under a closing sentence claiming to be #287's falsifier. Each node is
    # `name -> (finalised height, finalised hash, tip height, hashes its block API serves)`.
    a = src.index('echo "== 10. finality past the point =="')
    b = src.index("echo \"  (this section plus section 9 is #287's falsifier", a)
    section10 = src[a:b]

    def run_finality(name, nodes, expect_rc, required):
        lines = ["set -uo pipefail", "MEET=0", "RECONCILE_FINALITY_ROUNDS=1",
                 "sleep() { :; }", "trigger_block() { :; }",
                 "NAMES=(%s)" % " ".join(nodes), "declare -A HOST PORT"]
        for n in nodes:
            lines.append("HOST[%s]=%s; PORT[%s]=1" % (n, n, n))
        for fn, i in (("lfb_num", 0), ("lfb_hash", 1), ("height_of", 2)):
            lines.append('%s() { case "$1" in' % fn)
            for n, spec in nodes.items():
                lines.append("  %s) printf '%%s' '%s' ;;" % (n, spec[i]))
            lines.append("esac; }")
        lines.append('blocks_at() { case "$1" in')
        for n, spec in nodes.items():
            body = " ; ".join("printf '%%s\\n' '%s'" % h for h in spec[3]) or ":"
            lines.append("  %s) %s ;;" % (n, body))
        lines.append("esac; }")
        p = subprocess.run(["bash", "-c", "\n".join(lines) + "\n" + section10],
                           capture_output=True, text=True, timeout=10)
        combined = p.stdout + p.stderr
        assert p.returncode == expect_rc, f"{name}: exit={p.returncode}, expected={expect_rc}\n{combined}"
        assert required in combined, f"{name}: missing {required!r}\n{combined}"
        print(f"PASS {name} (exit {p.returncode})")

    # A — the transcript's own shape: nothing finalised anywhere, so every node reads unusable.
    run_finality("finality unavailable everywhere",
                 {"A": ("", "", "3", []), "B": ("", "", "4", [])}, 9, "no usable finalized block")
    # B — the anchor itself is not progress: the acceptance clause is *past* the point.
    run_finality("finalized at the anchor, not past it",
                 {"A": ("0", "anchor", "3", ["anchor"]), "B": ("0", "anchor", "3", ["anchor"])},
                 9, "not past the point")
    # C — two finalised siblings at one height, **both** in the common DAG: the pairwise rule alone
    # accepts this, and two finalised heads at one height is the opposite of one head.
    run_finality("two finalized blocks at one height",
                 {"A": ("1", "hxA", "2", ["hxA", "hxB"]), "B": ("1", "hxB", "2", ["hxA", "hxB"])},
                 9, "two finalized blocks at height 1")
    # D — a finalised hash that a node which reached its height does not hold: a reported string,
    # not a block.
    run_finality("finalized block absent from a node that reached its height",
                 {"A": ("1", "hxA", "2", ["hxA"]), "B": ("1", "hxA", "2", ["hxB"])},
                 9, "does not hold it")
    run_finality("a taller node missing a shorter node's finalized block",
                 {"A": ("3", "hxA", "3", ["hxA"]), "B": ("1", "hxB", "1", ["hxB"])},
                 9, "does not hold it")
    # GREEN — differing finalised heights are a lag, not a divergence, and a node below a finalised
    # height is exempt from holding that block.
    run_finality("advanced, differing heights, each in the common DAG",
                 {"A": ("3", "hxA", "3", ["hxA", "hxB"]), "B": ("1", "hxB", "1", ["hxA", "hxB"])},
                 0, "finality advanced past")
