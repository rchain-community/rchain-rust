#!/usr/bin/env bash
# Print the next free AUDIT C-number, so no writer has to guess one.
#
# Why this exists: the finding numbers are the one part of the audit's structure that nothing
# allocates. Every other cell is owned (`tools/emit-lean-counts.sh` owns the counts, the register owns
# the laws, `tools/audit-test-register.sh` owns the citations), but the C-number is chosen by whoever
# writes the finding — and in a checkout with four writers that raced three times in one hour
# (2026-09-24: C64, then C66, then C67, then C68, with C66 left as a gap and one commit message
# citing a number that now names a different finding).
#
# Usage, immediately before the edit and in the same working stretch as the commit:
#
#     n=$(tools/next-audit-number.sh --number)
#
# The scan is over *both* shapes a finding number takes: the entry (`- **C61 — …`) and the §20
# back-sweep table row (`| C61 … |`), so a number that is only in the table still counts as used.

set -euo pipefail

audit="$(cd "$(dirname "$0")/.." && pwd)/spec/AUDIT.md"
[[ -f "$audit" ]] || { echo "no spec/AUDIT.md at $audit" >&2; exit 1; }

# Only the two shapes a *finding* number takes: the entry and the §20 table row. A prose mention
# (`… traced to C66`) is not an allocation — C66 is a gap the numbering race left, and it must read as
# a gap here too, or the tool would disagree with the register about what is allocated.
used="$(
  { grep -oE '^- \*\*C[0-9]+' "$audit"; grep -oE '^\| C[0-9]+ ' "$audit"; } \
    | grep -oE '[0-9]+' | sort -n -u
)"
max="$(printf '%s\n' "$used" | tail -1)"

# Gaps: numbers below the maximum that no finding claims.
gaps="$(seq 1 "$max" | comm -13 <(printf '%s\n' "$used") - | tr '\n' ' ')"

if [[ "${1:-}" == "--number" ]]; then
  echo $((max + 1))
  exit 0
fi

echo "highest C-number in use: C$max"
printf 'in use: %s\n' "$(printf '%s\n' "$used" | tr '\n' ' ')"
if [[ -n "$gaps" ]]; then
  printf 'unused below the maximum (do not reuse — a gap is how a retired number looks): %s\n' "$gaps"
fi
echo "next free: C$((max + 1))"
