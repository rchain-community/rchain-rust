#!/usr/bin/env bash
#
# audit-test-register.sh — check that `spec/TEST-COVERAGE.md` tells the truth about the tree.
#
# The register is a hand-written audit, so it drifts: it has claimed an `#[ignore]`d test that no
# longer existed, "110 legacy contracts" when there were 165, and per-crate counts several releases
# out of date. This script is the antidote — it recomputes what the register asserts and fails when
# the two disagree.
#
# What it checks:
#   1. **Inventory counts** — the per-crate table must not *overstate* coverage: the live count must
#      be >= the recorded one. Understating is allowed (the register may lag as tests are added);
#      claiming tests that do not exist is not.
#   2. **Named tests exist** — every row of the "machine-checked claims" table names a file and a
#      test function; the function must be found in that file.
#   3. **No deferred gaps** — a `⏸` row means a documented, still-open gap. Every gap row was closed
#      by Stage 2 of the completion plan, so this check is now expected to pass outright; a new `⏸`
#      is a deliberate act. `--deferred-ok` also tolerates the risk-tier table's open items, which
#      is the only thing that flag still covers while the tiered work is in flight.
#   4. **Legacy contract count** — the `.rho`/`.rhox` counts must match the filesystem, so the
#      contract corpus cannot silently grow or shrink past the register.
#   5. **Tier table modules exist** — every path in the per-module tier table must be a real file.
#
# What it does NOT check: whether a named test *really pins* the behaviour it claims (only that it
# exists), and whether a module's test is a failure-arm test rather than a happy path. Those are
# human judgements; the register records them, and the plan's Stage 1 records a manual
# branch-disabled check for the highest-risk tests.
#
# Usage: tools/audit-test-register.sh [--deferred-ok]
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REGISTER="$ROOT/spec/TEST-COVERAGE.md"
DEFERRED_OK=0
[[ "${1:-}" == "--deferred-ok" ]] && DEFERRED_OK=1

failures=0
fail() { printf 'FAIL  %s\n' "$*"; failures=$((failures + 1)); }
info() { printf '      %s\n' "$*"; }
ok() { printf 'ok    %s\n' "$*"; }

[[ -f "$REGISTER" ]] || { echo "FAIL  register not found: $REGISTER" >&2; exit 1; }

# --- live counts -------------------------------------------------------------

# Unit tests in a crate's `src/`, integration tests in its `tests/`, plus property and bench counts.
#
# The property column counts **distinct laws with a property test** (`fn law7_...` ⇒ Law 7), not
# `proptest!` blocks or functions: the column exists to answer "does this law have randomized
# coverage?", and one block can hold several properties while two functions can cover one law.
# `grep` returning 1 for "no match" must not abort the script under `set -e`/`pipefail` — a crate with
# no `tests/` directory is a legitimate zero, not an error.
# The attribute pattern must absorb arguments: `#[tokio::test(flavor = "multi_thread")]` is a real
# and common form, and a pattern that stops at `test]` silently undercounts it.
count_unit() { { grep -rhoE '#\[(tokio::)?test[^]]*\]' "$ROOT/$1/src" 2>/dev/null || true; } | wc -l | tr -d ' '; }
count_integration() { { grep -rhoE '#\[(tokio::)?test[^]]*\]' "$ROOT/$1/tests" 2>/dev/null || true; } | wc -l | tr -d ' '; }
count_property() {
  { grep -rhoE 'fn law[0-9]+[a-z0-9_]*' "$ROOT/$1/src" "$ROOT/$1/tests" 2>/dev/null || true; } \
    | { grep -oE 'law[0-9]+' || true; } | sort -u | wc -l | tr -d ' '
}
count_bench() { { grep -rhoE 'bench_function|bench_with_input' "$ROOT/$1/benches" 2>/dev/null || true; } | wc -l | tr -d ' '; }

# --- 1. inventory table ------------------------------------------------------
#
# Rows look like:  | `rspace` | 66 | — | 7 | — |
printf '\n== inventory (recorded vs live) ==\n'
while IFS='|' read -r _ crate recorded_unit recorded_integration recorded_property recorded_bench _; do
  crate="$(printf '%s' "$crate" | tr -d ' `')"
  [[ -d "$ROOT/$crate" ]] || continue
  for pair in "unit:$recorded_unit" "integration:$recorded_integration" "property:$recorded_property" "bench:$recorded_bench"; do
    kind="${pair%%:*}"; recorded="$(printf '%s' "${pair#*:}" | tr -d ' ')"
    [[ "$recorded" =~ ^[0-9]+$ ]] || continue # "—" means the crate has none
    case "$kind" in
      unit) live="$(count_unit "$crate")" ;;
      integration) live="$(count_integration "$crate")" ;;
      property) live="$(count_property "$crate")" ;;
      bench) live="$(count_bench "$crate")" ;;
    esac
    if (( live < recorded )); then
      fail "$crate $kind: register claims $recorded, the tree has $live"
    elif (( live > recorded )); then
      info "$crate $kind: register says $recorded, tree has $live (stale — update the table)"
    fi
  done
done < <(grep -E '^\| `[a-z-]+` \|' "$REGISTER" | head -20)
ok "inventory counts do not overstate the tree"

# --- 2. machine-checked claims ----------------------------------------------
#
# Rows look like:  | G1 | casper/src/dag.rs | insert_rejects_equivocation_same_seq_num |
printf '\n== named tests ==\n'
# A claim is any row of the table whose *file* cell looks like a path. Keying on the id column was
# fragile in both directions: it silently ignored rows whose id has no digit (`| corpus | …`), and
# it needed an alphabet-specific pattern to avoid parsing the header row. A path in the second cell
# is what makes a row a claim, and the header's second cell is the literal `File`.
claims="$(awk '/^## Machine-checked claims/,/^## [^M]/' "$REGISTER" | grep -E '^\| [^|]*\| [^|]*[./][^|]*\|' || true)"
if [[ -z "$claims" ]]; then
  fail "no machine-checked claims table found (expected between '## Machine-checked claims' and the next '## ')"
else
  checked=0
  while IFS='|' read -r _ id file test _; do
    id="$(printf '%s' "$id" | tr -d ' ')"
    file="$(printf '%s' "$file" | tr -d ' `')"
    test="$(printf '%s' "$test" | tr -d ' `')"
    [[ -n "$file" && -n "$test" ]] || continue
    if [[ ! -f "$ROOT/$file" ]]; then
      fail "$id: $file does not exist"
    elif ! grep -qE "(async )?fn[[:space:]]+$test\b" "$ROOT/$file"; then
      fail "$id: '$test' not found in $file"
    else
      checked=$((checked + 1))
    fi
  done < <(printf '%s\n' "$claims" || true)
  # Every claim row must have been checked: a row this loop skipped is a claim nobody verifies.
  rows=$(printf '%s\n' "$claims" | grep -c '^|' || true)
  if (( checked != rows )); then
    fail "$((rows - checked)) claim row(s) were not checked (a malformed row is silently skipped)"
  fi
  ok "$checked named test(s) verified"
fi

# --- 3. deferred gaps --------------------------------------------------------
printf '\n== deferred gaps ==\n'
# Only bullets and table rows count as deferred *claims*; the prose that defines the marker and the
# legend that explains it are not gaps (counting them inflated this list by three).
deferred="$(grep -nE '^(- |\| ).*⏸' "$REGISTER" || true)"
if [[ -n "$deferred" ]]; then
  if (( DEFERRED_OK )); then
    info "--deferred-ok: $(printf '%s\n' "$deferred" | wc -l | tr -d ' ') still-deferred row(s):"
    printf '%s\n' "$deferred" | sed 's/^/      /'
  else
    fail "$(printf '%s\n' "$deferred" | wc -l | tr -d ' ') deferred gap row(s) remain; close them, or pass --deferred-ok while the work is open:"
    printf '%s\n' "$deferred" | sed 's/^/      /'
  fi
else
  ok "no deferred gaps"
fi

# --- 4. legacy contract count -----------------------------------------------
printf '\n== legacy contract corpus ==\n'
rho_live="$(find "$ROOT/legacy" -name '*.rho' 2>/dev/null | wc -l | tr -d ' ')"
rhox_live="$(find "$ROOT/legacy" -name '*.rhox' 2>/dev/null | wc -l | tr -d ' ')"
rho_recorded="$(grep -oE '[0-9]+ `\.rho`' "$REGISTER" | head -1 | grep -oE '^[0-9]+' || echo '')"
if [[ -z "$rho_recorded" ]]; then
  fail "the register does not state a '.rho' contract count (expected e.g. '165 \`.rho\`')"
elif (( rho_live < rho_recorded )); then
  fail "register claims $rho_recorded .rho contracts, the tree has $rho_live"
else
  ok "$rho_recorded recorded .rho contracts, $rho_live live ($rhox_live .rhox)"
fi

# --- 5. every ✅ claim cites a verified test ---------------------------------
#
# A ✅ with no named test is how G2's `RateLimiter` leg came to be marked covered when nothing
# exercised it — the failure this rule exists to prevent.
printf '\n== remediation claims ==\n'
claim_ids="$(printf '%s\n' "${claims:-}" | awk -F'|' '{gsub(/ /,"",$2); print $2}' | sort -u)"
remediation="$(awk '/^## Remediation status/,/^## [^R]/' "$REGISTER" | grep -E '^\| G[0-9]+ ' || true)"
if [[ -z "$remediation" ]]; then
  fail "no remediation-status table found (expected rows starting '| G1 ', '| G2 ', …)"
else
  missing=0
  while IFS='|' read -r _ name status _; do
    id="$(printf '%s' "$name" | grep -oE 'G[0-9]+' | head -1)"
    [[ -n "$id" ]] || continue
    if [[ "$status" == *"✅"* ]] && ! printf '%s\n' "$claim_ids" | grep -qx "$id"; then
      fail "$id is marked ✅ but names no verified test in the machine-checked claims table"
      missing=$((missing + 1))
    fi
  done < <(printf '%s\n' "$remediation" || true)
  (( missing == 0 )) && ok "every ✅ claim cites a verified test"
fi

# --- 6. tier table ----------------------------------------------------------
#
# Rows look like:  | T1 | `casper/src/conf.rs` | `shard_spec_rejects_an_illegal_name` |
# A `—` test cell means the module is a registered open item: the module exists but has no test yet,
# so the table doubles as the todo list for the tiered coverage work.
printf '\n== risk-tier table ==\n'
tier_rows="$(awk '/^## Risk tiers/,/^## [^R]/' "$REGISTER" | grep -E '^\| T[0-9] ' || true)"
if [[ -z "$tier_rows" ]]; then
  fail "no risk-tier table rows found (expected rows starting '| T1 ', '| T2 ' or '| T3 ')"
else
  open=0
  checked=0
  while IFS='|' read -r _ tier path test _; do
    path="$(printf '%s' "$path" | tr -d ' `')"
    test="$(printf '%s' "$test" | tr -d ' `')"
    [[ -n "$path" ]] || continue
    if [[ ! -f "$ROOT/$path" ]]; then
      fail "$tier: $path does not exist"
      continue
    fi
    if [[ -z "$test" || "$test" == "—" ]]; then
      open=$((open + 1))
      info "$tier open: $path has no test recorded yet"
    elif ! grep -qE "(async )?fn[[:space:]]+$test\b" "$ROOT/$path"; then
      fail "$tier: '$test' not found in $path"
    else
      checked=$((checked + 1))
    fi
  done < <(printf '%s\n' "$tier_rows" || true)
  ok "$checked tier-table test(s) verified"
  if (( open > 0 )); then
    if (( DEFERRED_OK )); then
      info "--deferred-ok: $open tier module(s) still without a recorded test"
    else
      fail "$open tier module(s) have no test recorded; close them, or pass --deferred-ok while the work is open"
    fi
  fi
fi

# --- summary -----------------------------------------------------------------
printf '\n===== summary =====\n'
if (( failures == 0 )); then
  echo "OK: the register matches the tree."
  exit 0
fi
echo "FAIL: $failures problem(s) between spec/TEST-COVERAGE.md and the tree."
exit 1
