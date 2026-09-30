#!/usr/bin/env bash
# Does finality resume after a validator is killed? — the "after" arm of the 2026-09-30 campaign.
#
# The protocol is `n127-liveness-preregistration.md`, frozen before this ran: the same rig as the campaign
# (which found finality stopping in 3 of 3 attempts, at most +4 heights), the same kill at T+120, and one
# acceptance row — finality advances on the survivors by more than +4.
#
# The merge budget (C184) is read here as a **control**, not as a reading: this run's scopes are the honest
# ones, so `SearchBudgetExceeded` must NOT appear. If it does, the budget is set too low and its number is
# wrong.
#
# Usage:  ATTEMPTS=3 spec/audit/evidence/n127-liveness-run.sh
set -u
cd /home/patrick/RNodeRust

ATTEMPTS=${ATTEMPTS:-3}
CAP=${CAP:-8g}
WINDOW_S=${WINDOW_S:-300}
DEPLOY_AT=${DEPLOY_AT:-30}
DEPLOYS=${DEPLOYS:-4}
KILL_AT=${KILL_AT:-120}
DEPLOY_TIMEOUT=${DEPLOY_TIMEOUT:-45}

TREE=$(git rev-parse --short HEAD)
STAMP=$(date -u +%Y%m%dT%H%M%SZ)
OUT="target/n127-liveness/${TREE}-${STAMP}"
declare -A PORTS=([bootstrap]=40403 [v1]=41403 [v2]=42403)
declare -A CONTAINERS=([bootstrap]=devnet-bootstrap [v1]=devnet-validator-1 [v2]=devnet-validator-2)

mkdir -p "$OUT"
IMAGE=$(docker inspect rnode:local --format '{{.Id}}' 2>/dev/null || echo "none")
FLAGS="--validators 3 --stakes 100,100,50 --epoch-length 10 --fresh"

{
  echo "# tree=$TREE image=$IMAGE"
  echo "# shape: $FLAGS, cap=$CAP, window=${WINDOW_S}s, kill=validator-2 at T+${KILL_AT}s, attempts=$ATTEMPTS"
  echo "# started $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "# binary_sha256=$(docker run --rm --entrypoint sha256sum rnode:local /usr/local/bin/rnode 2>/dev/null | cut -d' ' -f1 || echo '?')"
  echo "# rust_diff_vs_80782e184=$([ -n "$(git diff --stat 80782e184 -- '*.rs' 2>/dev/null)" ] && echo 'NON-EMPTY' || echo 'empty')"
} > "$OUT/manifest.txt"
cat "$OUT/manifest.txt"

for attempt in $(seq 1 "$ATTEMPTS"); do
  echo "=================== attempt $attempt of $ATTEMPTS  $(date -u +%H:%M:%S) UTC ==================="
  tools/devnet.sh down >/dev/null 2>&1
  DEVNET_NODE_MEMORY="$CAP" timeout 900 tools/devnet.sh up $FLAGS 2>&1 | tail -1

  up=$(docker ps --format '{{.Names}}' 2>/dev/null | grep -c '^devnet-' || true)
  if [[ "$up" -lt 3 ]]; then
    echo "  VOID: $up of 3 containers up"
    tools/devnet.sh down >/dev/null 2>&1
    continue
  fi
  echo "  3 of 3 up at $(date -u +%H:%M:%S)"

  python3 spec/audit/evidence/n117-queue-depth.py "$OUT/series-a${attempt}.tsv" \
    > "$OUT/series-a${attempt}.log" 2>&1 &
  sampler=$!

  t0=$(date +%s)
  wait_until() { local d=$(( $1 - $(date +%s) )); (( d > 0 )) && sleep "$d" || true; }

  wait_until $((t0 + DEPLOY_AT))
  submitted=0
  for _ in $(seq 1 "$DEPLOYS"); do
    timeout "$DEPLOY_TIMEOUT" tools/devnet.sh deploy examples/hello.rho >/dev/null 2>&1 \
      && submitted=$((submitted + 1))
  done
  echo "  deploys: $submitted of $DEPLOYS"

  wait_until $((t0 + KILL_AT))
  tools/devnet.sh stop 2 >/dev/null 2>&1
  echo "  killed validator-2 at T+$(( $(date +%s) - t0 ))s (target ${KILL_AT}s)"

  wait "$sampler"

  # **The node's own reason**, read before the containers go: the stall line now fires on a change of
  # variant as well as per 100 heights (`interpreter_util.rs`), because the previous campaign measured the
  # pin in 3 of 3 attempts and could not say why — the instrument that explains it was gated out of range.
  for n in bootstrap v1 v2; do
    docker logs "${CONTAINERS[$n]}" 2>&1 | grep 'finality did not advance' \
      > "$OUT/stall-${n}-a${attempt}.txt" || true
    docker logs "${CONTAINERS[$n]}" 2>&1 | grep 'exceeded its budget' \
      >> "$OUT/budget-${n}-a${attempt}.txt" || true
  done
  echo "  stall lines: $(cat "$OUT"/stall-*-a${attempt}.txt 2>/dev/null | wc -l)"
  tools/devnet.sh down >/dev/null 2>&1
  echo "  attempt $attempt done at $(date -u +%H:%M:%S) UTC"
done

echo
echo "=== finality after the kill, per attempt ==="
python3 - "$OUT" "$KILL_AT" <<'PY'
import glob, os, re, sys
out, kill = sys.argv[1], int(sys.argv[2])

def secs(t):
    h, m, s = (int(x) for x in t.split(":"))
    return h * 3600 + m * 60 + s

for path in sorted(glob.glob(os.path.join(out, "series-a*.tsv"))):
    attempt = re.search(r"series-a(\d+)\.tsv$", path).group(1)
    rows = []
    for line in open(path):
        if line.startswith("#") or line.startswith("utc\t"):
            continue
        p = line.rstrip("\n").split("\t")
        if len(p) >= 10:
            rows.append(p)
    if not rows:
        print(f"  attempt {attempt}: no series"); continue
    t0 = secs(rows[0][0])
    span = secs(rows[-1][0]) - t0
    print(f"  --- attempt {attempt} ({span:.0f}s window)")
    for node in sorted({r[1] for r in rows}):
        rs = [r for r in rows if r[1] == node]
        def at(t):
            return min(rs, key=lambda r: abs(secs(r[0]) - t0 - t))
        b, e = at(kill - 1), at(span)
        hb, fb = b[7], b[8]
        he, fe = e[7], e[8]
        def n(v):
            return 0 if v == "none" else (int(v) if v.isdigit() else None)
        fb_n, fe_n, hb_n, he_n = n(fb), n(fe), n(hb), n(he)
        moved = (fe_n is not None and fb_n is not None) and fe_n > fb_n
        delta = fe_n - fb_n if (fe_n is not None and fb_n is not None) else None
        grew = he_n - hb_n if (he_n is not None and hb_n is not None) else None
        print(f"      {node:<9} finality {fb_n} -> {fe_n} (+{delta})   height {hb_n} -> {he_n} (+{grew})   "
              f"{'FINALITY ADVANCED' if moved else 'finality did not move'}")
PY

echo
echo "=== the budget control: SearchBudgetExceeded must not appear on honest scopes ==="
hits=0
for n in bootstrap v1 v2; do
  c=$(docker logs "${CONTAINERS[$n]}" 2>&1 | grep -c 'exceeded its budget' || true)
  hits=$((hits + c))
done
if [[ "$hits" -eq 0 ]]; then
  echo "  ok: the budget refused nothing — these are the honest scopes it must not touch"
else
  echo "  *** $hits budget refusal(s): SearchBudget::NODE is set too low, and its number is wrong"
fi
echo
echo "artifacts: $OUT"
