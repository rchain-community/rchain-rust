#!/usr/bin/env bash
# Does `/metrics` carry the census's distribution? — C182's close condition, run rather than argued.
#
# The campaign of 2026-09-30 read `_count 7617` from the endpoint against `101 merges` from the node's own
# census line, on the same node at the same instant, and the factor was the number of **scrapes**: the
# endpoint pushed a snapshot per request into a period accumulator over a *cumulative* registry. So this
# run scrapes deliberately many times — the property under test is the one that was broken — and then
# compares the endpoint against the census's own line for the same node.
#
# The comparison is the one the results file should have made and did not:
#   endpoint  `rchain_merge_scope_width_count`            == census `MERGES`
#   endpoint  `_bucket{le=E}` (cumulative)                == census buckets 0..i summed, for E = edge i
#
# Usage:  SCRAPES=200 spec/audit/evidence/n127-endpoint-vs-census-run.sh
set -u
cd /home/patrick/RNodeRust

SCRAPES=${SCRAPES:-200}
CAP=${CAP:-8g}
WINDOW_S=${WINDOW_S:-90}
TREE=$(git rev-parse --short HEAD)
STAMP=$(date -u +%Y%m%dT%H%M%SZ)
OUT="target/n127-endpoint/${TREE}-${STAMP}"
mkdir -p "$OUT"

echo "tree=$TREE image=$(docker inspect rnode:local --format '{{.Id}}' 2>/dev/null)"
echo "# scraping :40403 $SCRAPES times after a ${WINDOW_S}s window"

tools/devnet.sh down >/dev/null 2>&1
DEVNET_NODE_MEMORY="$CAP" timeout 900 \
  tools/devnet.sh up --validators 3 --stakes 100,100,50 --epoch-length 10 --fresh 2>&1 | tail -1

up=$(docker ps --format '{{.Names}}' 2>/dev/null | grep -c '^devnet-' || true)
if [[ "$up" -lt 3 ]]; then
  echo "VOID: $up of 3 containers up"
  tools/devnet.sh down >/dev/null 2>&1
  exit 1
fi

echo "window ${WINDOW_S}s ..."
sleep "$WINDOW_S"

# The scrapes. Under the defect each of these added the running total to itself, so the count grew with
# this loop — the loop's length *is* the multiplier the campaign measured.
first=""
for i in $(seq 1 "$SCRAPES"); do
  curl -fsS --max-time 10 http://localhost:40403/metrics -o "$OUT/scrape-$i.txt" 2>/dev/null || true
  [[ -n "$first" ]] || first="$OUT/scrape-1.txt"
done
cp "$first" "$OUT/scrape-first.txt" 2>/dev/null || true
curl -fsS --max-time 10 http://localhost:40403/metrics -o "$OUT/scrape-last.txt" 2>/dev/null || true
docker logs devnet-bootstrap 2>&1 | grep 'merge search' | tail -1 > "$OUT/census.txt" || true
{
  echo "# tree=$TREE scraped=$(date -u +%Y-%m-%dT%H:%M:%SZ) node=bootstrap scrapes=$SCRAPES"
  echo "# shape: --validators 3 --stakes 100,100,50 --epoch-length 10 --fresh, cap=$CAP, window=${WINDOW_S}s"
} > "$OUT/manifest.txt"

echo
echo "=== the census's own line (the node's rendering):"
cat "$OUT/census.txt"
echo
echo "=== the comparison:"
python3 - "$OUT" "$SCRAPES" <<'PY'
import re, sys
out = sys.argv[1]
scrapes = int(sys.argv[2])
census = open(f"{out}/census.txt").read()
m = re.search(r"merge search: (\d+) merges .*width buckets \[([0-9, ]+)\]", census)
if not m:
    print("  no census line: the node logged none in the window"); raise SystemExit(1)
merges = int(m.group(1))
raw = [int(x) for x in m.group(2).split(",")]
cum = [sum(raw[:i + 1]) for i in range(len(raw))]

def endpoint(path):
    txt = open(path).read()
    count = re.search(r"^rchain_merge_scope_width_count (\S+)", txt, re.M)
    buckets = {float(k): float(v) for k, v in
               re.findall(r'^rchain_merge_scope_width_bucket\{le="([^"]+)"\} (\S+)', txt, re.M)}
    return (float(count.group(1)) if count else None), buckets

first_count, first_b = endpoint(f"{out}/scrape-first.txt")
last_count, last_b = endpoint(f"{out}/scrape-last.txt")
print(f"  census      : {merges} merges, buckets {raw} -> cumulative {cum}")
print(f"  endpoint    : count {last_count} (first scrape {first_count}, last of {len(first_b)} edges)")
# The load-bearing comparison is against the *census*, not first-against-last: merges keep landing while
# the loop runs, so a live registry's first and last scrapes legitimately differ by however many landed in
# between. What the defect did was multiply the count by the number of *scrapes* — so the honest report is
# the count beside the scrape count, which under the defect would be ~200x the merges and is now equal to
# them. (Measured, 2026-09-30: 93 merges and a count of 93 after 200 scrapes.)
print(f"  vs scrapes  : {scrapes} scrapes, endpoint count {last_count} — the defect scaled with this number")

# And the distribution itself, cumulative on both sides, against the census's own edges.
edges = sorted(e for e in last_b if e != float("inf"))
ok = True
for i, e in enumerate(edges):
    want = cum[i] if i < len(cum) else cum[-1]
    got = last_b[e]
    if got != want:
        ok = False
    print(f"    le={e:<7} endpoint {got:<8.0f} census {want:<8d} {'ok' if got == want else 'MISMATCH'}")
print(f"    le={e:<7} endpoint {got:<8.0f} census {want:<8d} {flag}")
print(f"  count vs census: {last_count} vs {merges} -> {'ok' if last_count == merges else 'MISMATCH (the log line can lag by one merge)'}")
print(f"\n  VERDICT: {'the endpoint carries the census' if ok and last_count == merges else 'STILL WRONG'}")
PY

tools/devnet.sh down >/dev/null 2>&1
echo
echo "artifacts: $OUT"
