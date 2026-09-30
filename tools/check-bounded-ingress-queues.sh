#!/usr/bin/env bash
#
# check-bounded-ingress-queues — every queue on the ingress -> validate -> process path is bounded.
#
# **Why a source check and not a test.** The defect it guards (C175) is visible in a *type*:
# `wire_block_processing` returns `mpsc::UnboundedSender<BlockMessage>` for the validated-blocks queue,
# so a unit test could only assert the boundedness by failing to compile against today's signature, and
# a test that does not compile is not a test. The function's other arguments need `CommState`/`ShardParts`
# fixtures, which makes a runtime test expensive for a fact that is textual. So the assertion lives here,
# in the repo's `tools/check-*.sh` idiom, and it is deliberately **red until C175 is fixed**: this is the
# gate that fix has to satisfy, not a check looking for work.
#
# What it looks for, and why each is on this path: a peer streams valid-signed blocks, so any unbounded
# queue between the transport and the processor lets a peer fill memory faster than the node drains it.
# R15 (`spec/findings.tsv`) closed exactly this for the processor-input channel; C175 is the half it
# missed, and the tap channel beside it.
#
# Usage: tools/check-bounded-ingress-queues.sh          # report
#        tools/check-bounded-ingress-queues.sh --gate   # exit non-zero on a violation (for CI)
set -uo pipefail
cd "$(dirname "$0")/.."

SRC=node/src/runtime/node_runtime.rs
[[ -r "$SRC" ]] || { echo "check-bounded-ingress-queues: no $SRC" >&2; exit 2; }

violations=0
report() {
  printf '  %-28s %-14s %s\n' "$1" "$2" "$3"
  [[ "$2" == "unbounded" ]] && violations=$((violations + 1))
}

echo "check-bounded-ingress-queues: queues on the ingress -> validate -> process path in $SRC"
echo
printf '  %-28s %-14s %s\n' QUEUE BOUND SITE
printf '  %-28s %-14s %s\n' ----- ----- ----

# Each queue is identified by the binding that creates it, not by its constructor call — the
# constructor text alone appears three times (a test's channel among them) and would report the wrong
# site for two of the three queues.
declare -a QUEUES=(
  "incoming_blocks|let (incoming_blocks_tx"
  "processor_input|let (processor_input_tx"
  "validated_blocks|let (validated_blocks_tx"
  "tap_validated_blocks|let (tap_tx"
)
while IFS='|' read -r name binding; do
  site="$(grep -nF "$binding" "$SRC" | head -1)"
  if [[ -z "$site" ]]; then
    report "$name" "absent" "no binding matching '$binding' — has the pipeline been renamed?"
    continue
  fi
  line="${site%%:*}"
  if [[ "$site" == *unbounded_channel* ]]; then
    report "$name" "unbounded" "${SRC}:${line}"
  else
    report "$name" "bounded" "${SRC}:${line}"
  fi
done < <(printf '%s\n' "${QUEUES[@]}")

echo
if (( violations > 0 )); then
  cat >&2 <<EOF
check-bounded-ingress-queues: $violations unbounded queue(s) on the ingress -> validate -> process path.

This is C175 (spec/findings.tsv), and it is the defect this check exists to gate. The fix is R15's:
bound the queue with backpressure -- \`mpsc::channel(N)\` and an awaiting \`send\` -- and change
\`wire_block_processing\`'s return type from \`mpsc::UnboundedSender<BlockMessage>\` to
\`mpsc::Sender<BlockMessage>\` so the type carries the bound. Add a depth gauge at the same time: the
audit could not rule this queue out as #117's mechanism for the plain reason that nothing measures its
depth.

When the fix lands, wire this script into the lint job with --gate.
EOF
  [[ "${1:-}" == "--gate" ]] && exit 1
  exit 0
fi

echo "check-bounded-ingress-queues: every queue on the path is bounded."
