#!/usr/bin/env bash
#
# probe-blocks-per-deploy — the instrument for #148 and #149.
#
# It submits **exactly one** deploy to an already-running net, samples every node's height and finalised
# block at a fixed cadence, and writes a transcript that says four things:
#
#   1. how many blocks were minted after the deploy (the "blocks per deploy" number #149 wants a curve of);
#   2. whether the deploy **finalised**, and after how long — the property, as opposed to the absence of a
#      storm. A chain that stops minting is only a pass if the deploy finalised;
#   3. whether the **clamp** was hit, because the hosts this is normally run on are 1 GB and an unbounded
#      storm cannot be watched to completion (the 2026-09-29 run was stopped by hand at height 368);
#   4. whether a proposer **halted** — `AUTOPROPOSE_MAX_CONSECUTIVE_FAILURES` makes the autopropose task
#      `break` after three consecutive self-validation failures, which is log-only (see #157). Without this
#      a transcript cannot distinguish "the chain went quiet" from "the proposer is dead", so a node given
#      no `ssh=` attribute is recorded as **log witness: NOT CHECKED** rather than assumed clean.
#
# It does not provision a net. Standing one up is environment-specific (hosts, ports, keys, firewall) and
# the recipe differs per environment; this script deliberately starts from a mesh that already answers
# `/api/status` on every node.
#
# Usage:
#   tools/probe-blocks-per-deploy.sh \
#     --node a=http://164.90.140.144:40403,ssh=root@164.90.140.144,unit=rnode.service \
#     --node b=http://104.131.176.164:40403,ssh=root@104.131.176.164,unit=rnode.service \
#     --deployer-key-file /etc/rnode/deployer.env \
#     --ceiling 400 --out spec/audit/evidence/probe-$(date -u +%Y%m%dT%H%M%SZ).md
#
# Node attributes (comma-separated, all but the URL optional):
#   NAME=URL              required; the HTTP API of one node (used for sampling)
#   ssh=USER@HOST         enables the halt witness on that node
#   unit=UNIT             journald unit to read for the halt witness (default rnode.service)
#   grpc=HOST:PORT        gRPC endpoint for the deploy (default 127.0.0.1:40401 on the ssh host,
#                         or the node URL's host with :40401 when there is no ssh)
#   bin=PATH              rnode binary on that host (default /usr/local/bin/rnode remote, rnode local)
#   key-file=PATH         file containing `KEY=VALUE` with the secp256k1 deployer secret
#
# The deploy goes through the **first** node's attributes. `grpc-host` is a bare host and `grpc-port` a
# bare port: the CLI builds the URI itself, and passing `host:port` to `--grpc-host` fails with
# `invalid URI` (which is how the first version of this script failed).
#
# Prereqs: curl, python3; ssh for the witness and for a remote deploy; a funded deployer key.

set -euo pipefail

cd "$(dirname "$0")/.."

CADENCE=5
CEILING=400
TIMEOUT=600
PHLO_LIMIT=90000
PHLO_PRICE=1
SHARD_ID="/root"
OUT=""
RHOLANG=""
NODES=()
DEPLOYER_KEY="${DEPLOYER_KEY:-}"

die() { printf 'probe: %s\n' "$*" >&2; exit 1; }
note() { printf '%s\n' "$*" >&2; }

# ---------------------------------------------------------------- arguments

while [ $# -gt 0 ]; do
    case "$1" in
        --node)          NODES+=("$2"); shift 2 ;;
        --deployer-key)  DEPLOYER_KEY="$2"; shift 2 ;;
        --deployer-key-file)
            DEPLOYER_KEY="$(grep -E '^[A-Z_]*KEY=' "$2" | head -1 | cut -d= -f2-)"
            [ -n "$DEPLOYER_KEY" ] || die "$2 has no KEY= line"
            shift 2 ;;
        --rholang)       RHOLANG="$2"; shift 2 ;;
        --cadence)       CADENCE="$2"; shift 2 ;;
        --ceiling)       CEILING="$2"; shift 2 ;;
        --timeout)       TIMEOUT="$2"; shift 2 ;;
        --phlo-limit)    PHLO_LIMIT="$2"; shift 2 ;;
        --phlo-price)    PHLO_PRICE="$2"; shift 2 ;;
        --shard-id)      SHARD_ID="$2"; shift 2 ;;
        --out)           OUT="$2"; shift 2 ;;
        -h|--help)       sed -n '2,45p' "$0"; exit 0 ;;
        *)               die "unknown argument: $1 (try --help)" ;;
    esac
done

[ "${#NODES[@]}" -ge 1 ] || die "at least one --node is required (try --help)"
[ -n "$DEPLOYER_KEY" ] || die "a deployer key is required: --deployer-key or --deployer-key-file"
command -v curl >/dev/null || die "curl is required"
command -v python3 >/dev/null || die "python3 is required"

attr() {  # attr <node-spec> <key> -> value or empty
    printf '%s' "$1" | tr ',' '\n' | awk -F= -v k="$2" '$1==k {print substr($0, index($0,"=")+1)}' | head -1
}

node_names=(); node_urls=(); node_ssh=(); node_unit=(); node_grpc=(); node_bin=()
for spec in "${NODES[@]}"; do
    name_url="${spec%%,*}"
    name="${name_url%%=*}"
    url="${name_url#*=}"
    [ "$name" != "$url" ] || die "--node needs NAME=URL, got: $spec"
    [ -n "$url" ] || die "--node $name has an empty URL"
    node_names+=("$name"); node_urls+=("$url")
    node_ssh+=("$(attr "$spec" ssh)")
    node_unit+=("$(attr "$spec" unit)"); [ -n "${node_unit[-1]}" ] || node_unit[-1]="rnode.service"
    node_grpc_host+=("$(attr "$spec" grpc-host)"); [ -n "${node_grpc_host[-1]}" ] || node_grpc_host[-1]=127.0.0.1
    node_grpc_port+=("$(attr "$spec" grpc-port)"); [ -n "${node_grpc_port[-1]}" ] || node_grpc_port[-1]=40401
    node_bin+=("$(attr "$spec" bin)")
done

# ---------------------------------------------------------------- http helpers

get() { curl -s --max-time 10 "$1"; }

status_field() {  # status_field <url> <field>
    get "$1/api/status" | python3 -c "
import sys, json
try: print(json.load(sys.stdin).get('$2', ''))
except Exception: print('')"
}

finalized() {  # finalized <url> -> height, or 'none'
    get "$1/api/last-finalized-block" | python3 -c "
import sys, json
raw = sys.stdin.read()
try:
    d = json.loads(raw)
    print(d['blockInfo']['blockNumber'] if isinstance(d, dict) else 'none')
except Exception:
    print('none')"
}

explore() {  # explore <url> <json-payload-file>
    curl -s --max-time 25 -X POST "$1/api/explore-deploy" \
        -H 'Content-Type: application/json' --data-binary @"$2"
}

deploy_block_of() {  # deploy_block_of <url> <signature> -> block number or empty
    get "$1/api/v1/deploy-status/$2" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    print(d.get('blockNumber', ''))
except Exception: print('')"
}

halt_count() {  # halt_count <ssh> <unit> -> lines seen, or 'n/a'
    local ssh="$1" unit="$2"
    [ -n "$ssh" ] || { printf 'n/a'; return; }
    ssh -o BatchMode=yes -o ConnectTimeout=8 "$ssh" \
        "journalctl -u $unit --no-pager 2>/dev/null | grep -c 'halted after' || true" 2>/dev/null || printf 'n/a'
}

binary_identity() {  # binary_identity <ssh> <bin> -> what the binary calls itself, or 'unknown'
    local ssh="$1" bin="$2" out=""
    [ -n "$ssh" ] || { printf 'unknown (no ssh)'; return; }
    [ -n "$bin" ] || bin=/usr/local/bin/rnode
    out="$(ssh -o BatchMode=yes -o ConnectTimeout=8 "$ssh" "$bin --version 2>/dev/null | head -1" 2>/dev/null)" || out=""
    [ -n "$out" ] && printf '%s' "$out" || printf 'unknown'
}

# ---------------------------------------------------------------- preflight

WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT

note "==> preflight"
bonds_json="$WORK/bonds.json"
python3 - "$bonds_json" <<'PY'
import json, sys
term = ('new return, rl(`rho:registry:lookup`), posCh in { rl!(`rho:rchain:pos`, *posCh) | '
        'for (@(_, PoS) <- posCh) { @PoS!("getBonds", *return) } }')
open(sys.argv[1], 'w').write(json.dumps(term))
PY

start_heights=(); start_finalized=(); halt_before=(); versions=()
net_id=""
for i in "${!node_names[@]}"; do
    n="${node_names[$i]}"; u="${node_urls[$i]}"
    h="$(status_field "$u" latestBlockNumber)"
    [ -n "$h" ] || die "$n ($u) does not answer /api/status"
    start_heights+=("$h")
    start_finalized+=("$(finalized "$u")")
    versions+=("$(binary_identity "${node_ssh[$i]}" "${node_bin[$i]}")")
    nid="$(status_field "$u" networkId)"
    if [ -z "$net_id" ]; then net_id="$nid"; elif [ "$nid" != "$net_id" ]; then
        die "nodes disagree on networkId: '$net_id' vs '$nid' on $n — this is not one net"
    fi
    halt_before+=("$(halt_count "${node_ssh[$i]}" "${node_unit[$i]}")")
    note "    $n [${versions[-1]}] height=$h finalised=${start_finalized[-1]} halt-lines-before=${halt_before[-1]}"
done

bonds_summary="$(explore "${node_urls[0]}" "$bonds_json" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    m = d['expr'][0]['ExprMap']['data']
    stakes = [v['ExprInt']['data'] for v in m.values()]
    print(f'{len(m)} validator(s), total stake {sum(stakes)}, stakes {sorted(stakes)}')
except Exception as e:
    print('(bond map unavailable)')")"
note "    bonds: $bonds_summary"

# ---------------------------------------------------------------- deploy

note "==> submitting exactly one deploy"
if [ -n "$RHOLANG" ]; then
    term_src="$(cat "$RHOLANG")"
else
    term_src='new stdout(`rho:io:stdout`) in { stdout!("probe-blocks-per-deploy") }'
fi
printf '%s\n' "$term_src" > "$WORK/probe.rho"

i=0  # the deploy goes through the first node
ssh0="${node_ssh[0]}"; bin0="${node_bin[0]}"
ghost0="${node_grpc_host[0]}"; gport0="${node_grpc_port[0]}"
url0="${node_urls[0]}"
[ -n "$bin0" ] || { [ -n "$ssh0" ] && bin0=/usr/local/bin/rnode || bin0=rnode; }
if [ -z "$ssh0" ]; then
    # no ssh: the deploy runs from here, so "localhost" means this machine, not the node
    [ "$ghost0" = "127.0.0.1" ] && ghost0="$(printf '%s' "$url0" | sed -E 's#^https?://##; s#[:/].*$##')"
fi

deploy_out="$WORK/deploy.out"
if [ -n "$ssh0" ]; then
    scp -q -o BatchMode=yes "$WORK/probe.rho" "$ssh0:/tmp/probe-blocks-per-deploy.rho" 2>/dev/null \
        || die "could not copy the rholang source to $ssh0"
    ssh -o BatchMode=yes "$ssh0" \
        "$bin0 --profile docker --grpc-host $ghost0 --grpc-port $gport0 deploy \
         --phlo-limit $PHLO_LIMIT --phlo-price $PHLO_PRICE --shard-id '$SHARD_ID' \
         --private-key $DEPLOYER_KEY /tmp/probe-blocks-per-deploy.rho" \
        >"$deploy_out" 2>&1 || true
else
    $bin0 --profile docker --grpc-host "$ghost0" --grpc-port "$gport0" deploy \
        --phlo-limit "$PHLO_LIMIT" --phlo-price "$PHLO_PRICE" --shard-id "$SHARD_ID" \
        --private-key "$DEPLOYER_KEY" "$WORK/probe.rho" >"$deploy_out" 2>&1 || true
fi

# `|| true` on the substitution: under `set -e` a grep that finds nothing aborts the shell *before* the
# check below can say why, which is how this script first failed silently. The check is the message.
sig="$( { grep -oE 'DeployId is: [0-9a-f]+' "$deploy_out" || true; } | awk '{print $3}' | head -1)"
[ -n "$sig" ] || { cat "$deploy_out" >&2; die "deploy did not return a DeployId (output above)"; }
note "    deploy signature: ${sig:0:16}…"

# ---------------------------------------------------------------- sample

note "==> sampling every ${CADENCE}s (ceiling ${CEILING} blocks, timeout ${TIMEOUT}s)"
samples="$WORK/samples.md"
printf '| t (s) |' > "$samples"
for n in "${node_names[@]}"; do printf ' %s height | %s finalised |' "$n" "$n" >> "$samples"; done
printf -- '\n|---|' >> "$samples"; for _ in "${node_names[@]}"; do printf -- '---|---|' >> "$samples"; done
printf -- '\n' >> "$samples"

t0=$(date +%s)
deploy_height=""; finalised_at=""; start_ns=$(date +%s%N)
clamped=0
while :; do
    elapsed=$(( $(date +%s) - t0 ))
    row="| $elapsed |"; max_delta=0; all_fin=1
    for i in "${!node_names[@]}"; do
        u="${node_urls[$i]}"
        h="$(status_field "$u" latestBlockNumber)"; [ -n "$h" ] || h="?"
        f="$(finalized "$u")"
        row="$row $h | $f |"
        if [ "$h" != "?" ]; then
            d=$(( h - start_heights[i] )); [ "$d" -gt "$max_delta" ] && max_delta=$d
        fi
        if [ "$f" = "none" ] || [ -z "$deploy_height" ] || [ "$f" -lt "$deploy_height" ] 2>/dev/null; then all_fin=0; fi
    done
    printf '%s\n' "$row" >> "$samples"

    if [ -z "$deploy_height" ]; then
        deploy_height="$(deploy_block_of "$url0" "$sig")"
        if [ -n "$deploy_height" ]; then
            note "    deploy included at block $deploy_height"
        fi
    fi
    if [ "$all_fin" = "1" ] && [ -n "$deploy_height" ]; then
        finalised_at=$elapsed; break
    fi
    if [ "$max_delta" -ge "$CEILING" ]; then clamped=1; break; fi
    if [ "$elapsed" -ge "$TIMEOUT" ]; then break; fi
    sleep "$CADENCE"
done
end_ns=$(date +%s%N)

# ---------------------------------------------------------------- verdict

halt_after=(); for i in "${!node_names[@]}"; do
    halt_after+=("$(halt_count "${node_ssh[$i]}" "${node_unit[$i]}")")
done

end_heights=(); end_finalized=()
for i in "${!node_names[@]}"; do
    end_heights+=("$(status_field "${node_urls[$i]}" latestBlockNumber)")
    end_finalized+=("$(finalized "${node_urls[$i]}")")
done

witness_ok=1
for i in "${!node_names[@]}"; do [ "${halt_after[$i]}" = "n/a" ] && witness_ok=0; done

{
    printf '# Blocks per deploy — probe transcript\n\n'
    printf -- '- **When:** %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    printf -- '- **Nodes and binaries:**\n'
    for i in "${!node_names[@]}"; do
        printf -- '  - `%s` %s — %s\n' "${node_names[$i]}" "${node_urls[$i]}" "${versions[$i]}"
    done
    printf -- '- **Network:** %s   **Bonds:** %s\n' "$net_id" "$bonds_summary"
    printf -- '- **Deploy:** `%s` at phlo-limit %s, phlo-price %s; signature `%s`\n' \
        "${RHOLANG:-built-in no-op}" "$PHLO_LIMIT" "$PHLO_PRICE" "$sig"
    printf -- '- **Cadence:** %ss   **Ceiling:** %s blocks   **Timeout:** %ss\n\n' "$CADENCE" "$CEILING" "$TIMEOUT"

    printf '## Curve\n\n'; cat "$samples"; printf '\n'

    printf '## Verdict\n\n'
    printf '| node | height | finalised | blocks minted | halt lines before → after |\n|---|---|---|---|---|\n'
    for i in "${!node_names[@]}"; do
        printf '| %s | %s → %s | %s → %s | %s | %s → %s |\n' \
            "${node_names[$i]}" "${start_heights[$i]}" "${end_heights[$i]}" \
            "${start_finalized[$i]}" "${end_finalized[$i]}" \
            "$(( end_heights[i] - start_heights[i] ))" "${halt_before[$i]}" "${halt_after[$i]}"
    done
    printf '\n'

    if [ -n "$finalised_at" ]; then
        printf -- '- **The deploy finalised** at block %s, %ss after submission.\n' "$deploy_height" "$finalised_at"
    else
        printf -- '- **The deploy did NOT finalise** within %ss' "$TIMEOUT"
        [ -n "$deploy_height" ] && printf ' (included at block %s)' "$deploy_height"
        printf '.\n'
    fi
    printf -- '- **Blocks minted after the deploy:** %s\n' \
        "$(for i in "${!node_names[@]}"; do
              printf '%s: %s; ' "${node_names[$i]}" "$(( end_heights[i] - start_heights[i] ))"
          done | sed 's/; *$//')"
    minted_max=0
    for i in "${!node_names[@]}"; do
        d=$(( end_heights[i] - start_heights[i] )); [ "$d" -gt "$minted_max" ] && minted_max=$d
    done
    printf -- '- Production after the deploy: %s\n' \
        "$( [ "$minted_max" -le 2 ] && printf 'stopped immediately (%s block(s))' "$minted_max" \
           || printf 'continued (%s blocks)' "$minted_max" )"
    if [ -z "$finalised_at" ] && [ "$clamped" = "0" ] && [ "$minted_max" -le 2 ]; then
        printf -- '  - **Read this as the shape of the net, not as a finality defect, until the net has more than\n    one proposer.** With `--propose-on-deploy` and no autopropose, a deploy produces one block, and the\n    block that carries it cannot finalise until further blocks exist: attestation is what supplies them,\n    and a single node has no remote block to attest to. The same run on two or more attesting validators\n    is the measurement that discriminates.\n'
    fi
    if [ "$clamped" = "1" ]; then
        printf -- '- **CLAMPED** at the %s-block ceiling: production was still running when the probe stopped.\n' "$CEILING"
    else
        printf -- '- Not clamped.\n'
    fi
    if [ "$witness_ok" = "1" ]; then
        printf -- '- **Halt witness:** checked on every node (journald). %s\n' \
            "$(for i in "${!node_names[@]}"; do printf '%s: %s new line(s); ' "${node_names[$i]}" "$(( halt_after[i] - halt_before[i] ))"; done | sed 's/; *$//')"
    else
        printf -- '- **Halt witness: NOT CHECKED on every node** — a node without `ssh=` cannot rule out a halted\n  proposer (#157), and a chain that stops minting may be a dead proposer rather than a converged one.\n'
    fi
    printf '\n'
} | { [ -n "$OUT" ] && tee "$OUT" || cat; }

if [ "$witness_ok" != "1" ]; then
    note "probe: WARNING — the halt witness was not checked on every node; see the transcript"
fi
