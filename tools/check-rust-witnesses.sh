#!/usr/bin/env bash
#
# check-rust-witnesses.sh — run every registered Rust witness *by name*.
#
# The law register (`spec/Rchain/Laws.lean` → `spec/laws.tsv`) names, for each proved row, the Lean
# declarations its status rests on (`witness`, checked by name against the elaborated environment).
# The Rust half of the same claim had no machine half at all: a row's `rust` cell names a *file*, and
# a file does not rot but it is also not evidence — the tests that actually assert a row's case were
# named in prose (`falsifiable`/`note`) where nothing read them. `rustWitness` is that half, entries
# of the shape `path.rs:symbol`, and this is what makes it *executable*: the named test is run, and a
# name that matches no test fails.
#
# **The zero-match refusal is the point, not a detail.** `cargo test <filter>` exits 0 when the filter
# matches nothing, so a registry of renamed, deleted or `#[ignore]`d tests would run green while
# checking nothing — the fifth blind instrument this pass has found, and the same clause
# `tools/audit-test-register.sh`'s checks 9 and 10 already carry ("a check that matched nothing is not
# evidence"). A witness whose symbol is not even *in* its file is refused before cargo runs.
#
# Usage:  tools/check-rust-witnesses.sh [file]      (default: stdin; `-` also means stdin)
#         the input is one `path.rs:symbol` per line; blank lines and `#` comments are skipped.
#
# The register-side caller is `tools/check-lean-conformance.sh` once the field is populated:
#   awk -F'\t' 'NR>1 { n=split($N, a, ", "); for (i=1;i<=n;i++) if (a[i] != "-") print a[i] }' \
#     spec/laws.tsv | tools/check-rust-witnesses.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
INPUT="${1:--}"

# --- the crate a path belongs to ---------------------------------------------------------------
# Derived from the path's first segment rather than a per-witness field: the workspace is one crate
# per sbt module, so the path already says which crate the symbol compiles into, and a second copy
# of that mapping in the register would be a second thing to drift.
crate_of() {
  case "${1%%/*}" in
    sdk)           echo rchain-sdk ;;
    shared)        echo rchain-shared ;;
    crypto)        echo rchain-crypto ;;
    graphz)        echo rchain-graphz ;;
    models)        echo rchain-models ;;
    block-storage) echo rchain-block-storage ;;
    comm)          echo rchain-comm ;;
    rspace)        echo rchain-rspace ;;
    rholang)       echo rchain-rholang ;;
    casper)        echo rchain-casper ;;
    node)          echo rchain-node ;;
    *)             echo "" ;;
  esac
}

total=0
failures=0
skip_note=""

while IFS= read -r line || [[ -n "$line" ]]; do
  [[ -z "${line// }" || "${line:0:1}" == "#" ]] && continue
  total=$((total + 1))
  path="${line%%:*}"
  symbol="${line#*:}"
  if [[ -z "$path" || -z "$symbol" || "$path" == "$line" ]]; then
    printf 'FAIL  `%s` is not `path.rs:symbol`\n' "$line"
    failures=$((failures + 1))
    continue
  fi
  if [[ ! -f "$ROOT/$path" ]]; then
    printf 'FAIL  %s: no such file\n' "$path"
    failures=$((failures + 1))
    continue
  fi
  # The symbol must be *in* the file: cheap, and it catches a rename before cargo is paid for.
  if ! grep -qE "fn ${symbol}[^A-Za-z0-9_]" "$ROOT/$path"; then
    printf 'FAIL  %s:%s — no `fn %s` in that file\n' "$path" "$symbol" "$symbol"
    failures=$((failures + 1))
    continue
  fi

  crate="$(crate_of "$path")"
  if [[ -z "$crate" ]]; then
    printf 'FAIL  %s: %s is not a workspace crate\n' "$path" "${path%%/*}"
    failures=$((failures + 1))
    continue
  fi
  # An integration test lives in `<crate>/tests/<file>.rs` and is named by its target; a unit test
  # is in the library, and `--lib` keeps cargo out of the integration targets' build.
  if [[ "$path" == */tests/* ]]; then
    target="${path##*/tests/}"
    target="${target%.rs}"
    cargo_args=(--test "$target")
  else
    cargo_args=(--lib)
  fi

  log="/tmp/rust-witness-$(printf '%s' "$path:$symbol" | tr -c 'A-Za-z0-9' '-').log"
  if ! (cd "$ROOT" && cargo test -p "$crate" "${cargo_args[@]}" "$symbol" >"$log" 2>&1); then
    printf 'FAIL  %s:%s — the test failed (see %s)\n' "$path" "$symbol" "$log"
    grep -E '^(test .* FAILED|assertion|thread .* panicked)' "$log" | head -3 | sed 's/^/      /'
    failures=$((failures + 1))
    continue
  fi
  # The clause that makes the rest evidence: the run must have *run* something. Counted from the
  # harness's own per-test lines, and the symbol's own name must be among them (a substring filter
  # may match a family, which is fine — a witness that matches only others is not).
  matched="$(grep -cE "^test (.*::)?${symbol} \.\.\." "$log" || true)"
  if (( matched == 0 )); then
    printf 'FAIL  %s:%s — matched no test (`cargo test` exits 0 on an empty filter)\n' "$path" "$symbol"
    failures=$((failures + 1))
    continue
  fi
  printf 'ok    %s:%s (%s, %s test(s))\n' "$path" "$symbol" "$crate" "$matched"
done < <(if [[ "$INPUT" == "-" ]]; then cat; else cat "$INPUT"; fi)

# A check that scanned nothing is not evidence, exactly as in the register's checks 9 and 10.
if (( total == 0 )); then
  echo "FAIL  no witnesses on the input — this check is vacuous${skip_note}"
  exit 1
fi
echo ""
if (( failures > 0 )); then
  echo "===== $failures of $total Rust witness(es) FAILED ====="
  exit 1
fi
echo "all $total Rust witness(es) ran and passed"
