#!/usr/bin/env bash
#
# devnet-test.sh — integration test for the Docker devnet (see tools/devnet.sh).
#
#   tools/devnet-test.sh [--validators N]
#
# Against a fresh `tools/devnet.sh up`, asserts:
#   - the expected host ports are published (deploy gRPC, public HTTP, admin HTTP);
#   - GET /api/status returns 200 with shardId/minPhloPrice and an advancing latestBlockNumber;
#   - POST /api/explore-deploy returns 200 and a well-formed result;
#   - POST /api/deploy (signed) returns 200, and a real deploy reaches ProcessedWithSuccess;
#   - admin POST /api/propose returns 200 from the host;
#   - permissive CORS headers are present on /api/status and /api/propose.
#
# On any failure it dumps `docker logs` + `docker inspect` for every node and exits non-zero.

set -uo pipefail

cd "$(dirname "$0")/.."

VALIDATORS=1
while [[ $# -gt 0 ]]; do
  case "$1" in
    --validators) VALIDATORS="${2:?}"; shift 2 ;;
    *) echo "unknown flag: $1" >&2; exit 2 ;;
  esac
done

HTTP=http://localhost:40403
ADMIN=http://localhost:40405
BOOTSTRAP=devnet-bootstrap
DEPLOYER_PRIV="a68a6e6cca30f81bd24a719f3145d20e8424bd7b396309b0708a16c7d8000b76"

# Precomputed signed DeployRequest (term "Nil", timestamp 1700000000000, phloPrice 1, phloLimit
# 1000000, validAfterBlockNumber 0, shardId "/root"), signed by the devnet deployer key. Generated
# once via `cargo run -p rchain-node --example gen_deploy_fixture` (deleted); the signature is valid
# regardless of block number because the HTTP deploy endpoint does not check future/expired at submit.
DEPLOY_FIXTURE='{"data":{"term":"Nil","timestamp":1700000000000,"phloPrice":1,"phloLimit":1000000,"validAfterBlockNumber":0,"shardId":"/root"},"deployer":"04f700a417754b775d95421973bdbdadb2d23c8a5af46f1829b1431f5c136e549e8a0d61aa0c793f1a614f8e437711c7758473c6ceb0859ac7e9e07911ca66b5c4","signature":"3044022041ebc32bae0195d167310362dd6539d3e2a750512d03ca075c0308bc75744e1002206338157bde1b385ff76ebc1840cdccb942158cff6e377b3053a1a2e9389d6ad8","sigAlgorithm":"secp256k1"}'

FAILURES=0

check() {
  local label="$1" cond="$2"
  if eval "$cond"; then
    echo "PASS  $label"
  else
    echo "FAIL  $label"
    FAILURES=$((FAILURES + 1))
  fi
}

dump_diagnostics() {
  echo "" >&2
  echo "--- diagnostics ---" >&2
  docker ps -a --filter "network=devnet" >&2 || true
  local c
  for c in $(docker ps -aq --filter "network=devnet" 2>/dev/null); do
    echo "--- docker logs $c (tail) ---" >&2
    docker logs "$c" 2>&1 | tail -40 >&2 || true
    echo "--- docker inspect $c (ports/binds) ---" >&2
    docker inspect -f '{{.Name}} state={{.State.Status}} ports={{json .NetworkSettings.Ports}}' "$c" >&2 || true
  done
}

# http_get <url> [extra curl args...]  → sets HTTP_CODE and HTTP_BODY.
http_get() {
  local url="$1"; shift
  local tmp
  tmp="$(mktemp)"
  HTTP_CODE="$(curl -s -o "$tmp" -w '%{http_code}' --max-time 10 "$@" "$url")"
  HTTP_BODY="$(cat "$tmp")"
  rm -f "$tmp"
}

json_num() { printf '%s' "$1" | sed -n "s/.*\"$2\":\([0-9]*\).*/\1/p"; }

echo "==> starting a fresh devnet (--validators $VALIDATORS)"
tools/devnet.sh down -v >/dev/null 2>&1 || true
tools/devnet.sh up --validators "$VALIDATORS" >/dev/null

echo ""
echo "==> 1. published ports"
check "deploy gRPC published (40402->40401)" \
  '[[ -n "$(docker port "$BOOTSTRAP" 40401 2>/dev/null | head -n1)" ]]'
check "public HTTP published (40403->40403)" \
  '[[ -n "$(docker port "$BOOTSTRAP" 40403 2>/dev/null | head -n1)" ]]'
check "admin HTTP published (40405->40405)" \
  '[[ -n "$(docker port "$BOOTSTRAP" 40405 2>/dev/null | head -n1)" ]]'

echo ""
echo "==> 2. status + block production"
HTTP_CODE=""
HTTP_BODY=""
block=""
shard=""
minphlo=""
for _ in $(seq 1 90); do
  http_get "$HTTP/api/status"
  if [[ "$HTTP_CODE" == "200" ]]; then
    block="$(json_num "$HTTP_BODY" latestBlockNumber)"
    shard="$(printf '%s' "$HTTP_BODY" | sed -n 's/.*"shardId":"\([^"]*\)".*/\1/p')"
    minphlo="$(json_num "$HTTP_BODY" minPhloPrice)"
    [[ -n "$block" ]] && break
  fi
  sleep 1
done
check "GET /api/status returns 200" '[[ "$HTTP_CODE" == "200" ]]'
check "shardId == /root" '[[ "$shard" == "/root" ]]'
check "minPhloPrice set" '[[ -n "$minphlo" ]]'

start="$block"
for _ in $(seq 1 60); do
  sleep 1
  http_get "$HTTP/api/status"
  block="$(json_num "$HTTP_BODY" latestBlockNumber)"
  if [[ -n "$block" && -n "$start" && "$block" -gt "$start" ]]; then break; fi
done
check "latestBlockNumber advances ($start -> $block)" \
  '[[ -n "$start" && -n "$block" && "$block" -gt "$start" ]]'

echo ""
echo "==> 3. explore-deploy"
# A trivial term without `!` (shell history expansion mangles `!` in some environments). The endpoint
# runs it and returns a well-formed RhoDataResponse (`expr` + `block`). Poll briefly: on a fresh node
# the finalized fringe isn't available until the first block finalizes (~1 block into the run).
for _ in $(seq 1 30); do
  http_get "$HTTP/api/v1/explore-deploy" -X POST -H 'Content-Type: application/json' -d '"1 + 1"'
  [[ "$HTTP_CODE" == "200" ]] && break
  sleep 1
done
check "POST /api/v1/explore-deploy returns 200" '[[ "$HTTP_CODE" == "200" ]]'
check "explore-deploy response has expr" \
  'printf "%s" "$HTTP_BODY" | grep -q "\"expr\""'

echo ""
echo "==> 3b. partial collection patterns (C20)"
# A pattern may name only part of a collection: `..._` absorbs the rest, `...rest` binds it. A *map*
# pattern that could not match a map with further keys made every rgov governance contract
# unreachable, silently — an unmatched `for` is not an error, it just never fires — so the family
# returned `[]` with no diagnostic. This is the reproduction, run on a real node: the two patterns
# sit on separate channels, so both must fire.
#
# The first `new`-bound name of the term is the deploy's return channel (see
# `RuntimeSyntax.playExploratoryDeploy`), so the endpoint's response carries `result`'s data: read
# it, and read it from the *node*, which is the only surface that showed the defect — the in-process
# conformance harness was green throughout, because its cases shared one runtime and one output
# channel (C20).
# The endpoint takes the term as a JSON *string*; escape it with `sed`, like `json_num` parses JSON
# with `sed`, so the script keeps its dependencies.
json_string() { sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' | tr -d '\n'; }
c20_term='new result, a, b in {
  a!({"x": 1, "y": 2}) |
  b!({"x": 1, "y": 2}) |
  for (@{"x": *v, ..._} <- a)    { result!(["partial-with-remainder", "matched"]) } |
  for (@{"x": *w, "y": *z} <- b) { result!(["exact", "matched"]) }
}'
c20_body="\"$(printf '%s' "$c20_term" | json_string)\""
http_get "$HTTP/api/v1/explore-deploy" -X POST -H 'Content-Type: application/json' \
  --data-binary "$c20_body"
check "POST /api/v1/explore-deploy (C20 reproduction) returns 200" \
  '[[ "$HTTP_CODE" == "200" ]]'
check "the partial map pattern matched (a partial map pattern fires)" \
  'printf "%s" "$HTTP_BODY" | grep -q "partial-with-remainder"'
check "the exact map pattern still matched" \
  'printf "%s" "$HTTP_BODY" | grep -q "\"exact\""'

echo ""
echo "==> 4. deploy + deploy-status"
# (a) HTTP deploy endpoint accepts a signed deploy.
http_get "$HTTP/api/deploy" -X POST -H 'Content-Type: application/json' -d "$DEPLOY_FIXTURE"
check "POST /api/deploy returns 200" '[[ "$HTTP_CODE" == "200" ]]'

# (b) a real deploy (submitted with a current validAfterBlockNumber so it is not expired) is
# included in a block and reaches ProcessedWithSuccess via the HTTP deploy-status endpoint.
docker exec -i "$BOOTSTRAP" sh -c 'printf "Nil\n" > /tmp/nil.rho' || true
deploy_out="$(docker exec -i "$BOOTSTRAP" rnode --grpc-host localhost deploy \
  --phlo-limit 1000000 --phlo-price 1 \
  --valid-after-block-number "$block" \
  --private-key "$DEPLOYER_PRIV" --shard-id /root /tmp/nil.rho 2>&1 || true)"
deploy_id="$(printf '%s' "$deploy_out" | sed -n 's/.*DeployId is: \([0-9a-f]*\).*/\1/p')"
check "gRPC deploy accepted (DeployId captured)" '[[ -n "$deploy_id" ]]'

deploy_status=""
for _ in $(seq 1 60); do
  http_get "$HTTP/api/v1/deploy-status/$deploy_id"
  if printf '%s' "$HTTP_BODY" | grep -q 'ProcessedWithSuccess'; then
    deploy_status="ProcessedWithSuccess"
    break
  fi
  sleep 1
done
check "deploy-status reaches ProcessedWithSuccess" '[[ "$deploy_status" == "ProcessedWithSuccess" ]]'

echo ""
echo "==> 4b. the governance handshake (C21)"
# `MemberDirectory.rho`'s `getMe` (the wallet's first governance call) logged its way to its third
# statement and then neither created the member nor answered: line 78's
# `if (everyone.contains(you) == false)` is not the first term of its `par` (line 77 logs first), and
# the port normalized an `if`'s condition against the `par` preceding it, so the condition *was* that
# `par` — a non-Bool `Match` target matches neither `true` nor `false`, and an unmatched `match` is
# not an error, so the `if` reduced to nothing, silently (AUDIT C21). Two node-level signatures of
# it, both read from the bootstrap's own log:
#
#   1. the feature's *deploy-time epilogue* calls `getMe` for the key that deployed it
#      (`MemberDirectory.rho:167`), so with the defect the log carries the feature's first lines and
#      stops — `["creating your stuff", …]` (line 27) never appears. It is upstream's own log line,
#      not a probe's.
#   2. a client's staged handshake reaches `stage:4 getMe answered`.
#
# The devnet is restarted with `down -v` at the top of this script, so the log is empty of earlier
# runs: a `grep` for a stage cannot be satisfied by a previous run's output. The deploy is signed by
# the devnet's *genesis* key (= the ceremony key, the only funded deployer here), so this leg covers
# the answer path and the member-exists branch; the fresh-member branch (which runs `createMe` for a
# client's own key) is pinned in-process by `a_fresh_chain_serves_the_wallets_new_inbox_handshake`.
#
# `stage:3` prints the value the directory holds, but no assertion reads it: `stage:4` already proves
# it. A `GetMe` entry that was `Nil` (an absent key) would take the send at the next statement
# nowhere, so a reply can only come from a real channel — and asserting "not Nil" by grepping a
# pretty-printed par is the kind of shape-matching that stops being true when the printer changes.
# The term is written with a *quoted* heredoc: backticks in the rho source (`rho:io:stdout`) would
# otherwise be command-substituted by the shell that expands it.
docker exec -i "$BOOTSTRAP" sh -c 'cat > /tmp/handshake.rho' <<'RHO'
new rl(`rho:registry:lookup`), deployerId(`rho:rchain:deployerId`), out(`rho:io:stdout`),
    capCh, getMeCh, stuffCh in {
  out!("stage:1 lookup sent") |
  rl!(`rho:id:wxc4mwdh7otq4fd6iuxt84inepssyz5tugojf7ao68dkh4ebbncy`, *capCh) |
  for (MCAread <- capCh) {
    out!("stage:2 readcap resolved") |
    MCAread!("GetMe", *getMeCh) |
    for (GetMe <- getMeCh) {
      out!(["stage:3 getme-entry", *GetMe]) |
      new logCh in {
        // Forward the feature's own log lines instead of draining them: they are the only trace of
        // where its control flow went, and a drain is what made a stall look like a client bug.
        for (@line <= logCh) { out!(["feature-log", line]) } |
        GetMe!(*deployerId, *stuffCh, *logCh) |
        for (@_reply <- stuffCh) { out!("stage:4 getMe answered") }
      }
    }
  } |
  // Peeks (`<<-`), so the lockers survive for the next reader — the wallet reads exactly these.
  for (@inboxValue <<- @[*deployerId, "inbox"])      { out!("stage:5 inbox locker") } |
  for (@dictValue <<- @[*deployerId, "dictionary"])  { out!("stage:5 dictionary locker") }
}
RHO
handshake_out="$(docker exec -i "$BOOTSTRAP" rnode --grpc-host localhost deploy \
  --phlo-limit 1000000 --phlo-price 1 \
  --valid-after-block-number "$block" \
  --private-key "$DEPLOYER_PRIV" --shard-id /root /tmp/handshake.rho 2>&1 || true)"
handshake_id="$(printf '%s' "$handshake_out" | sed -n 's/.*DeployId is: \([0-9a-f]*\).*/\1/p')"
check "handshake deploy accepted (DeployId captured)" '[[ -n "$handshake_id" ]]'
for _ in $(seq 1 60); do
  http_get "$HTTP/api/v1/deploy-status/$handshake_id"
  printf '%s' "$HTTP_BODY" | grep -q 'ProcessedWithSuccess' && break
  sleep 1
done
check "handshake deploy reaches ProcessedWithSuccess" \
  'printf "%s" "$HTTP_BODY" | grep -q "ProcessedWithSuccess"'

node_log="$(docker logs "$BOOTSTRAP" 2>&1 || true)"
check "the feature's own createMe ran at genesis ([\"creating your stuff\"] in the node log)" \
  'printf "%s" "$node_log" | grep -q "creating your stuff"'
check "the client's handshake reaches stage:4" \
  'printf "%s" "$node_log" | grep -q "stage:4 getMe answered"'
check "the deployer's inbox and dictionary lockers are present" \
  'printf "%s" "$node_log" | grep -q "stage:5 inbox locker" && printf "%s" "$node_log" | grep -q "stage:5 dictionary locker"'

echo ""
echo "==> 5. admin propose"
http_get "$ADMIN/api/propose" -X POST
check "POST /api/propose returns 200" '[[ "$HTTP_CODE" == "200" ]]'

echo ""
echo "==> 6. CORS"
cors_status="$(curl -s -D - -o /dev/null -H 'Origin: http://wallet.example' "$HTTP/api/status" \
  | tr -d '\r' | awk -F': ' 'tolower($1)=="access-control-allow-origin"{print $2}')"
check "GET /api/status CORS allow-origin *" '[[ "$cors_status" == "*" ]]'
cors_propose="$(curl -s -D - -o /dev/null -X POST -H 'Origin: http://wallet.example' "$ADMIN/api/propose" \
  | tr -d '\r' | awk -F': ' 'tolower($1)=="access-control-allow-origin"{print $2}')"
check "POST /api/propose CORS allow-origin *" '[[ "$cors_propose" == "*" ]]'

echo ""
if (( FAILURES > 0 )); then
  echo "==> $FAILURES check(s) FAILED"
  dump_diagnostics
  exit 1
fi
echo "==> all checks passed"
exit 0
