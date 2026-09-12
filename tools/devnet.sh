#!/usr/bin/env bash
#
# devnet — a single Docker script for a local RChain network.
#
# Two modes, one script:
#   • devnet  (default) — bonded validators that produce blocks + a funded deployer wallet, for
#     developing/deploying rholang contracts and reading results back.
#   • network — a bare 1..5 node topology (no autopropose, no deployer) for exercising sync/gossip;
#     drive it manually with `cli <node> propose`.
#
# Prereqs: docker (a running daemon) and openssl.
#
# SECURITY: the validator/deployer keys below are throwaway keys for a LOCAL testnet only.
# Never reuse them for anything with real value.

set -euo pipefail

cd "$(dirname "$0")/.."

IMAGE="${RNODE_IMAGE:-rnode:local}"
NETWORK="devnet"
BOOTSTRAP="devnet-bootstrap"
PREFIX="devnet"

# Throwaway validator keypairs (secp256k1, base16). validator[0] also funds the deployer wallet, so
# the deployer private key is validator[0]'s private key.
VALIDATOR_PRIV=(
  "a68a6e6cca30f81bd24a719f3145d20e8424bd7b396309b0708a16c7d8000b76"
  "b8a48b02757c0cfc9325498a93c3b28582e3967072f8fab0cdc8bd04d0d401ee"
  "d78ff60a424d71ce99d6b7d7f44a8c49b38a3757ff9e6fa9b32fcba8aa2c973b"
)
VALIDATOR_PUB=(
  "04f700a417754b775d95421973bdbdadb2d23c8a5af46f1829b1431f5c136e549e8a0d61aa0c793f1a614f8e437711c7758473c6ceb0859ac7e9e07911ca66b5c4"
  "04dbe32c2062240a4ba0bcad01d7edd98c78b51c77765d5e1e5e9fa3743d2f12a1f82f42cd7dc4f41445979117d790f23e9b3d08d0aa06d527c236172043e747fc"
  "04d8b6c325ae12e89823866b2a292a62d7acee520954761890a1621fef79dca1c8e8df79dd8519480e5c015ae6cf3ba7de8669e260561616a36eb9c308b5983ab0"
)
MAX_VALIDATORS=${#VALIDATOR_PRIV[@]}

# Deployer = validator[0] (its pubkey -> REV address is funded in genesis when DEPLOYER is on).
DEPLOYER_PRIV="${VALIDATOR_PRIV[0]}"
DEPLOYER_REV_ADDR="11112VYAt8rUGNRRZX3eJdgagaAhtWTK8Js7F7X5iqddMVqyDTtYau"
DEPLOYER_BALANCE=1000000000000

# Contract sources are mounted read-only at /contracts; deploy/eval reference them by basename.
CONTRACTS_DIR="$(pwd)/examples"

GRPC_BASE=40402   # host port mapped to the bootstrap's deploy gRPC (in-container 40401)
HTTP_BASE=40403   # host port mapped to the bootstrap's public HTTP API (in-container 40403)
ADMIN_BASE=40405  # host port mapped to the bootstrap's admin HTTP API (in-container 40405)

help() {
  cat >&2 <<'EOF'
usage: tools/devnet.sh <command> [options]

Commands:
  build [--fresh]                build the rnode:local image (--fresh: --no-cache --pull)
  up [options]                   start the network (see options below)
  down [-v]                      stop the network (+ drop data volumes with -v)
  status                         docker ps for the network
  logs <node>                    tail a node's logs
  diagnose                       per-node health check (PASS/WARN/FAIL)
  deploy <contract.rho> [--to N] signed deploy to the bootstrap (file lives in examples/)
  eval <file.rho>                thin-client REPL eval of a file on the bootstrap
  query <name>                   listen for data at a public name
  faucet <rev-address>           transfer 0.3 REV from the funded dev wallet to <rev-address>
  propose [--admin]              force the bootstrap to propose (gRPC, or admin HTTP with --admin)
  demo                           deploy the complex wallet contract and assert its round-trip
  cli <node> <rnode subcommand…> run the Rust client inside a node container
  help                           this message

`up` options:
  --validators N                 bonded validators (1..3, default 1)
  --observers M                  unbonded observers (0..3, default 0)
  --nodes N                      bare network topology: 1 bootstrap + N-1 peers (1..5),
                                 shorthand for --validators 1 --observers N-1 --no-autopropose
                                 --no-deployer --no-admin
  --autopropose | --no-autopropose
                                 continuously produce blocks on a timer (default: on for devnet,
                                 off for --nodes)
  --propose-on-deploy | --no-propose-on-deploy
                                 propose a block immediately after a deploy (default: on for devnet)
  --admin | --no-admin           publish the admin HTTP API (40405) to the host (default: on for devnet)
  --deployer-key HEX | --no-deployer
                                 fund the deployer wallet + enable dev-mode dummy-deploy keepalive
                                 (default: on for devnet, using validator[0]'s key)

When are blocks created?
  A block is created only when one of these fires: (a) --propose-on-deploy and a deploy is accepted;
  (b) --autopropose's timer/dummy-deploy; or (c) an explicit `propose`/`POST /api/v1/propose`. An idle
  node with none of these produces no blocks.
EOF
  exit 2
}

validator_name() { echo "${PREFIX}-validator-${1}"; }
observer_name()   { echo "${PREFIX}-observer-${1}"; }

cmd_build() {
  local opts=()
  if [[ "${1:-}" == "--fresh" ]]; then
    opts=(--no-cache --pull)
  elif [[ -n "${1:-}" ]]; then
    echo "unknown flag: $1" >&2; help
  fi
  docker build "${opts[@]}" -f docker/rnode/Dockerfile -t "$IMAGE" .
}

# Write genesis files (N validators + optionally a funded deployer wallet) into `$1`.
genesis_files() {
  local dir="$1" n="$2" i
  : > "$dir/bonds.txt"
  for (( i = 0; i < n; i++ )); do
    echo "${VALIDATOR_PUB[$i]} 100" >> "$dir/bonds.txt"
  done
  if $DEPLOYER; then
    echo "$DEPLOYER_REV_ADDR,$DEPLOYER_BALANCE" > "$dir/wallets.txt"
  else
    : > "$dir/wallets.txt"
  fi
}

wait_for_cert() {
  local c="$1"
  for _ in $(seq 1 60); do
    if docker exec "$c" test -f /var/lib/rnode/node.certificate.pem 2>/dev/null; then
      return 0
    fi
    sleep 1
  done
  echo "timed out waiting for $c to generate its TLS cert" >&2
  return 1
}

bootstrap_id() {
  docker exec "$BOOTSTRAP" cat /var/lib/rnode/node.certificate.pem \
    | openssl x509 -noout -subject \
    | awk -F'=' '{print $NF}'
}

# Wait until the bootstrap serves /api/v1/status and (if autopropose is on) is producing blocks.
wait_for_http() {
  local url="http://localhost:${HTTP_BASE}/api/v1/status"
  local body block_num
  for _ in $(seq 1 120); do
    if body="$(curl -fsS --max-time 5 "$url" 2>/dev/null)"; then
      block_num="$(printf '%s' "$body" | sed -n 's/.*"latestBlockNumber":\([0-9]*\).*/\1/p')"
      if ! $AUTOPROPOSE; then
        echo "==> $BOOTSTRAP serving /api/v1/status (autopropose off; block production is manual)"
        return 0
      fi
      if [[ -n "$block_num" && "$block_num" -gt 0 ]]; then
        echo "==> $BOOTSTRAP serving /api/v1/status (latestBlockNumber=$block_num)"
        return 0
      fi
    fi
    sleep 1
  done
  echo "timed out waiting for $BOOTSTRAP to serve /api/v1/status" >&2
  return 1
}

# `docker run` flags shared by every node (container name/network/ports + data + contracts mounts).
docker_opts() {
  local name="$1" grpc_host="$2" http_host="$3" admin_host="$4"
  local ports="-p ${grpc_host}:40401 -p ${http_host}:40403"
  if $ADMIN; then ports="$ports -p ${admin_host}:40405"; fi
  echo "-d --name $name --network $NETWORK $ports \
    -v ${name}-data:/var/lib/rnode \
    -v ${CONTRACTS_DIR}:/contracts:ro"
}

# `rnode run` flags shared by every node, assembled from the current flag globals.
rnode_run_common() {
  local name="$1"
  local flags="run --host $name --api-host 0.0.0.0 --data-dir /var/lib/rnode \
    --protocol-port 40400 --discovery-port 40404 \
    --api-port-grpc-external 40401 --api-port-grpc-internal 40402 \
    --api-port-http 40403 --api-port-admin-http 40405"
  if $AUTOPROPOSE; then flags="$flags --autopropose"; fi
  if $PROPOSE_ON_DEPLOY; then flags="$flags --propose-on-deploy"; fi
  if $ADMIN; then flags="$flags --api-enable-devnet-cors"; fi
  if $DEPLOYER; then flags="$flags --dev-mode --deployer-private-key ${DEPLOYER_PRIV}"; fi
  echo "$flags"
}

cmd_up() {
  # Mode globals: devnet defaults.
  local n=1 m=0
  AUTOPROPOSE=true
  PROPOSE_ON_DEPLOY=true
  ADMIN=true
  DEPLOYER=true

  while [[ $# -gt 0 ]]; do
    case "$1" in
      --validators) n="${2:?}"; shift 2 ;;
      --observers)  m="${2:?}"; shift 2 ;;
      --nodes)
        n=1; m=$((${2:?} - 1)); AUTOPROPOSE=false; PROPOSE_ON_DEPLOY=false; ADMIN=false; DEPLOYER=false
        shift 2 ;;
      --autopropose) AUTOPROPOSE=true; shift ;;
      --no-autopropose) AUTOPROPOSE=false; shift ;;
      --propose-on-deploy) PROPOSE_ON_DEPLOY=true; shift ;;
      --no-propose-on-deploy) PROPOSE_ON_DEPLOY=false; shift ;;
      --admin) ADMIN=true; shift ;;
      --no-admin) ADMIN=false; shift ;;
      --deployer-key) DEPLOYER=true; DEPLOYER_PRIV="${2:?}"; shift 2 ;;
      --no-deployer) DEPLOYER=false; shift ;;
      *) echo "unknown flag: $1" >&2; help ;;
    esac
  done
  if (( n < 1 || n > MAX_VALIDATORS )); then
    echo "--validators must be in 1..$MAX_VALIDATORS" >&2; exit 2
  fi
  if (( m < 0 || m > 3 )); then
    echo "--observers must be in 0..3" >&2; exit 2
  fi

  echo "==> devnet: $n validator(s) + $m observer(s)"
  echo "    autopropose=$AUTOPROPOSE propose-on-deploy=$PROPOSE_ON_DEPLOY admin=$ADMIN deployer=$DEPLOYER"
  docker network create "$NETWORK" >/dev/null 2>&1 || true

  local genesis_dir
  genesis_dir="$(mktemp -d)"
  genesis_files "$genesis_dir" "$n"

  # Validator 0 = bootstrap: creates + approves genesis (autopropose optional).
  echo "==> starting $BOOTSTRAP (validator 0, standalone, creates genesis)"
  # shellcheck disable=SC2046
  docker run $(docker_opts "$BOOTSTRAP" "$GRPC_BASE" "$HTTP_BASE" "$ADMIN_BASE") \
    -v "${genesis_dir}:/genesis:ro" \
    "$IMAGE" $(rnode_run_common "$BOOTSTRAP") -s \
      --bonds-file /genesis/bonds.txt --wallets-file /genesis/wallets.txt \
      --validator-private-key "${VALIDATOR_PRIV[0]}"

  wait_for_cert "$BOOTSTRAP"
  local id
  id="$(bootstrap_id)"
  echo "==> bootstrap id: $id"

  # Validators 1..n-1: bonded in genesis.
  local i name host_port http_port admin_port
  for (( i = 1; i < n; i++ )); do
    name="$(validator_name "$i")"
    host_port=$((GRPC_BASE + i * 1000))
    http_port=$((HTTP_BASE + i * 1000))
    admin_port=$((ADMIN_BASE + i * 1000))
    echo "==> starting $name (validator $i, bootstraps from $BOOTSTRAP)"
    # shellcheck disable=SC2046
    docker run $(docker_opts "$name" "$host_port" "$http_port" "$admin_port") \
      "$IMAGE" $(rnode_run_common "$name") \
        --bootstrap "rnode://${id}@${BOOTSTRAP}?protocol=40400&discovery=40404" \
        --validator-private-key "${VALIDATOR_PRIV[$i]}"
  done

  # Observers / bare peers: unbonded, replicate the chain.
  for (( i = 1; i <= m; i++ )); do
    name="$(observer_name "$i")"
    host_port=$((GRPC_BASE + (n + i) * 1000))
    http_port=$((HTTP_BASE + (n + i) * 1000))
    admin_port=$((ADMIN_BASE + (n + i) * 1000))
    echo "==> starting $name (observer $i, bootstraps from $BOOTSTRAP)"
    # shellcheck disable=SC2046
    docker run $(docker_opts "$name" "$host_port" "$http_port" "$admin_port") \
      "$IMAGE" $(rnode_run_common "$name") \
        --bootstrap "rnode://${id}@${BOOTSTRAP}?protocol=40400&discovery=40404"
  done

  wait_for_http

  echo ""
  echo "==> up. Interact with:"
  echo "    tools/devnet.sh deploy <contract.rho>   # signed deploy to $BOOTSTRAP"
  echo "    tools/devnet.sh query <name>            # listen for data at a public name"
  echo "    tools/devnet.sh propose [--admin]       # force a block"
  echo "    tools/devnet.sh status | logs <node> | diagnose | down"
  echo ""
  echo "    Public HTTP API:  http://localhost:${HTTP_BASE}/api/v1/status"
  if $ADMIN; then
    echo "    Admin HTTP API:   http://localhost:${ADMIN_BASE}/api/v1/propose"
  fi
}

cmd_down() {
  local remove_volumes=false
  [[ "${1:-}" == "-v" ]] && remove_volumes=true
  local names=("$BOOTSTRAP")
  local i
  for (( i = 1; i <= MAX_VALIDATORS; i++ )); do names+=("$(validator_name "$i")"); done
  for (( i = 1; i <= 3; i++ )); do names+=("$(observer_name "$i")"); done
  for c in "${names[@]}"; do
    docker rm -f "$c" >/dev/null 2>&1 || true
    if $remove_volumes; then docker volume rm "${c}-data" >/dev/null 2>&1 || true; fi
  done
  docker network rm "$NETWORK" >/dev/null 2>&1 || true
}

cmd_status() {
  docker ps --filter "network=$NETWORK" --format 'table {{.Names}}\t{{.Status}}\t{{.Ports}}'
}

cmd_logs() {
  docker logs -f "${1:?node name required}"
}

# Extract the host port mapped to a container port (e.g. "0.0.0.0:40403" -> "40403").
host_port_for() {
  local c="$1" container_port="$2"
  docker port "$c" "$container_port" 2>/dev/null | head -n1 | sed -n 's/.*:\([0-9]*\)$/\1/p'
}

# check <label> <ok?> — PASS/FAIL; warn <label> — WARN. Both track globals.
check() {
  local label="$1" ok="$2"
  if [[ "$ok" == "0" ]]; then
    echo "  PASS  $label"
  else
    echo "  FAIL  $label"
    FAILURES=$((FAILURES + 1))
  fi
}
warn() {
  echo "  WARN  $1"
  WARNINGS=$((WARNINGS + 1))
}

# diagnose: per-node health report (state, ports, sockets, reachability, syncing, block number);
# non-zero exit on any FAIL.
cmd_diagnose() {
  FAILURES=0
  WARNINGS=0
  local names=("$BOOTSTRAP") i
  for (( i = 1; i <= MAX_VALIDATORS; i++ )); do names+=("$(validator_name "$i")"); done
  for (( i = 1; i <= 3; i++ )); do names+=("$(observer_name "$i")"); done

  for c in "${names[@]}"; do
    docker inspect "$c" >/dev/null 2>&1 || continue
    echo "== $c =="

    local state
    state="$(docker inspect -f '{{.State.Status}}' "$c" 2>/dev/null || true)"
    [[ "$state" == "running" ]]; check "container running" $?

    local grpc_host http_host admin_host
    grpc_host="$(host_port_for "$c" 40401)"
    http_host="$(host_port_for "$c" 40403)"
    admin_host="$(host_port_for "$c" 40405)"
    [[ -n "$grpc_host" ]]; check "deploy gRPC published (40401)" $?
    [[ -n "$http_host" ]]; check "public HTTP published (40403)" $?
    [[ -n "$admin_host" ]]; check "admin HTTP published (40405)" $?

    # Host HTTP reachability + block number + peer counts.
    local body block peers nodes
    if body="$(curl -fsS --max-time 5 "http://localhost:${http_host}/api/v1/status" 2>/dev/null)"; then
      check "GET /api/v1/status reachable" 0
      block="$(printf '%s' "$body" | sed -n 's/.*"latestBlockNumber":\([0-9]*\).*/\1/p')"
      peers="$(printf '%s' "$body" | sed -n 's/.*"peers":\([0-9]*\).*/\1/p')"
      nodes="$(printf '%s' "$body" | sed -n 's/.*"nodes":\([0-9]*\).*/\1/p')"
      if [[ -n "$block" && "$block" -gt 0 ]]; then
        check "block production (latestBlockNumber=$block)" 0
      else
        warn "no blocks yet (latestBlockNumber=0) — idle, still syncing, or not autoproposing"
      fi
      if [[ "$peers" == "0" && "$nodes" == "0" ]]; then
        warn "no peers discovered (peers=0 nodes=0)"
      else
        check "peer connectivity (peers=$peers nodes=$nodes)" 0
      fi
    else
      check "GET /api/v1/status reachable" 1
    fi
  done

  echo ""
  if (( FAILURES > 0 )); then
    echo "==> $FAILURES check(s) FAILED${WARNINGS:+ ($WARNINGS warning(s))}"
    return 1
  fi
  echo "==> all checks passed${WARNINGS:+ ($WARNINGS warning(s))}"
  return 0
}

# Run `rnode` inside a node container (reaches deploy 40401 + propose/repl 40402 via localhost).
node_cli() {
  local node="$1"; shift
  local tty=""
  [[ "${1:-}" == "repl" ]] && tty="-t"
  docker exec -i $tty "$node" rnode --grpc-host localhost "$@"
}

cmd_cli() {
  local node="${1:?node name required}"; shift
  node_cli "$node" "$@"
}

cmd_deploy() {
  local file="${1:?contract file required (relative to examples/)}"; shift
  local node="$BOOTSTRAP"
  if [[ "${1:-}" == "--to" ]]; then node="$(validator_name "${2:?}")"; shift 2; fi
  local base; base="$(basename "$file")"
  # Anchor the deploy to the current height: a `valid_after_block_number = -1` deploy is "valid from
  # genesis" and expires after DEPLOY_LIFESPAN blocks, so on a long-running devnet it would be dropped
  # before it can be proposed.
  local height
  height="$(node_cli "$node" status 2>/dev/null | sed -n 's/.*"latestBlockNumber": *\([0-9]*\).*/\1/p')"
  echo "==> deploying $base to $node (validAfterBlockNumber=${height:-0})"
  node_cli "$node" deploy \
    --phlo-limit 1000000 --phlo-price 1 \
    --private-key "$DEPLOYER_PRIV" \
    --shard-id root \
    --valid-after-block-number "${height:-0}" \
    "/contracts/$base"
}

cmd_eval() {
  local file="${1:?file required (relative to examples/)}"
  local base; base="$(basename "$file")"
  node_cli "$BOOTSTRAP" eval "/contracts/$base"
}

cmd_query() {
  local name="${1:?public name required}"
  # The name is a public (forgeable) name; quote it so the client normalizes it as a rholang
  # *ground string* (matching `@"hello"!("world")`), not as a free variable.
  node_cli "$BOOTSTRAP" listen-data-at-name -t pub -c "\"$name\""
}

# One-shot run of the second (complex) contract: deploy examples/wallet.rho, which derives the
# deployer's REV address, keeps a per-address codeDict map, round-trips save/load, and publishes the
# loaded value on the public name "wallet"; assert the round-trip landed.
cmd_demo() {
  echo "==> deploying the complex wallet contract (examples/wallet.rho)"
  cmd_deploy wallet.rho

  echo "==> waiting for the round-tripped value on the public name 'wallet'"
  local out
  if ! out="$(timeout 120 docker exec -i "$BOOTSTRAP" rnode --grpc-host localhost \
      listen-data-at-name -t pub -c '"wallet"')"; then
    echo "ERROR: query timed out or failed" >&2
    return 1
  fi
  echo "$out"
  if ! grep -q 'world' <<<"$out"; then
    echo "ERROR: expected the wallet contract to round-trip 'world', got:" >&2
    echo "$out" >&2
    return 1
  fi
  echo "==> wallet contract round-trip OK"
}

cmd_faucet() {
  local addr="${1:?REV address required}"
  if ! docker ps --format '{{.Names}}' | grep -q "^${BOOTSTRAP}$"; then
    echo "ERROR: $BOOTSTRAP is not running — start it with 'up --validators 1'." >&2
    exit 2
  fi
  # The faucet is served on the public HTTP API (dev-mode only); it transfers 0.3 REV from the
  # genesis-funded deployer wallet to the requested address.
  curl -s -X POST "http://localhost:${HTTP_BASE}/api/v1/faucet" \
    -H 'Content-Type: application/json' \
    -d "{\"address\":\"${addr}\"}"
  echo
}

cmd_propose() {
  if [[ "${1:-}" == "--admin" ]]; then
    if ! docker port "$BOOTSTRAP" 40405 >/dev/null 2>&1; then
      echo "ERROR: admin HTTP (40405) is not published — restart with 'up --admin'." >&2
      exit 2
    fi
    curl -s -X POST "http://localhost:${ADMIN_BASE}/api/v1/propose"
  else
    node_cli "$BOOTSTRAP" propose
  fi
}

case "${1:-}" in
  build) shift; cmd_build "$@" ;;
  up) shift; cmd_up "$@" ;;
  down) cmd_down "${2:-}" ;;
  status) cmd_status ;;
  logs) cmd_logs "${2:-}" ;;
  diagnose) cmd_diagnose ;;
  deploy) shift; cmd_deploy "$@" ;;
  eval) shift; cmd_eval "$@" ;;
  query) shift; cmd_query "$@" ;;
  faucet) shift; cmd_faucet "$@" ;;
  demo) cmd_demo ;;
  propose) shift; cmd_propose "${1:-}" ;;
  cli) shift; cmd_cli "$@" ;;
  help|--help|-h) help ;;
  *) help ;;
esac
