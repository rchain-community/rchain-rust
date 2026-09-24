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
#   6. **Tier table rows are pinned** — a row's named test must exist in the named file, and no row may
#      be left with a `—` test cell (that is a registered open item).
#   7. **Every source file is tested or exempt** — each `<crate>/src/**/*.rs` either contains a test
#      attribute or is a row of the `## Exempt modules` table with a valid reason class. A file that
#      gains a test while still holding a row fails, so the table burns down instead of rotting into an
#      allowlist. Under `--deferred-ok` the still-unlisted files are reported (the burn-down list)
#      rather than failing, exactly like the deferred-gap and open-tier checks.
#   8. **Law rows are answerable** — a row of `spec/INVENTORY.md` that claims coverage (its status
#      cell says "checked") must name a Lean module that exists, a corpus that exists and a consumer
#      that exists; a row that does not claim coverage must say so in one of the closed vocabulary
#      words (`open`, `boundary`, `orphaned`, `axiomatic`, `deviation`). This is check 2's rule
#      applied to the law catalogue, and it is the check that would have caught C30-C40's gaps: each
#      was a row whose status read better than its evidence.
#   9. **The register's prose anchors still resolve** — for every `path:line` citation in a row of
#      `spec/laws.tsv`, the path must resolve, the line must be inside the file, and the cited window
#      must contain an identifier the row itself names. The register's Lean checks verify that an
#      anchor's *file* exists (a file does not rot); nothing read the lines, so five rows had drifted
#      by 2026-09-24 while every other check stayed green. The convention this enforces — and the
#      reason for it — is in `spec/STYLE.md`.
#
# The class vocabulary is **closed** because a row that can invent its own reason is not a reason:
# a `peer-bound` or `harness-bound` row must name its covering test as `path::test`, and the linter
# verifies that test exists like any other named test in this register.
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

# --- 7. every source file is tested or exempt --------------------------------
#
# The census that motivates this check: files have tests, but not every *file* does, and a per-module
# table written from memory only ever covers what it thought to list. So the register is checked
# against the tree instead of against itself.
#
# Rows look like:  | `data` | `rspace/src/checkpoint.rs` | why | — |
# A row whose file has a test is a failure (the exemption is stale — delete the row); a class outside
# the closed set is a failure (it means the vocabulary is being invented per-row); a `peer-bound` row
# must name its covering test as `path::test` and the linter checks that test exists, because
# "covered elsewhere" is a claim like any other. One cell may list several paths, comma-separated.
printf '\n== exempt modules ==\n'
exempt_section="$(awk '/^## Exempt modules/,/^## [^E]/' "$REGISTER" || true)"
if [[ -z "$exempt_section" ]]; then
  fail "no '## Exempt modules' table found (expected between '## Exempt modules' and the next '## ')"
else
  declare -A exempt=()
  rows=0
  bad=0
  while IFS='|' read -r _ class paths why covering _; do
    class="$(printf '%s' "$class" | tr -d ' `')"
    covering="$(printf '%s' "$covering" | tr -d ' `')"
    [[ -n "$class" ]] || continue
    rows=$((rows + 1))
    case "$class" in
      data|generated|dev-tool|peer-bound|harness-bound) ;;
      *) fail "exempt row has unknown class '$class' (want data|generated|dev-tool|peer-bound|harness-bound)"; bad=$((bad + 1)); continue ;;
    esac
    # `IFS=,` splits the path cell; a single path is the common case.
    while read -r path; do
      path="$(printf '%s' "$path" | tr -d ' `')"
      [[ -n "$path" ]] || continue
      if [[ ! -f "$ROOT/$path" ]]; then
        fail "exempt row: $path does not exist"
        bad=$((bad + 1))
        continue
      fi
      if grep -qE '(^|[[:space:]{};])#\[(tokio::)?test' "$ROOT/$path"; then
        fail "$path is exempted as '$class' but has a test — delete the row"
        bad=$((bad + 1))
        continue
      fi
      # `peer-bound` and `harness-bound` both mean "covered above this file, by the test named
      # here"; the linter holds them to the same evidence.
      if [[ "$class" == "peer-bound" || "$class" == "harness-bound" ]]; then
        cfile="${covering%%::*}"
        ctest="${covering##*::}"
        if [[ -z "$cfile" || "$ctest" == "$covering" || ! -f "$ROOT/$cfile" ]]; then
          fail "peer-bound $path must name its covering test as path::test (got '$covering')"
          bad=$((bad + 1))
        elif ! grep -qE "(async )?fn[[:space:]]+$ctest\b" "$ROOT/$cfile"; then
          fail "peer-bound $path: '$ctest' not found in $cfile"
          bad=$((bad + 1))
        fi
      fi
      exempt["$path"]=1
    done < <(printf '%s\n' "$paths" | tr ',' '\n')
  done < <(printf '%s\n' "$exempt_section" | grep -E '^\| `' || true)

  # The census. `*/src/*` rather than a crate list, so a new crate is covered the day it appears.
  bare=()
  while IFS= read -r f; do
    rel="${f#"$ROOT"/}"
    grep -qE '(^|[[:space:]{};])#\[(tokio::)?test' "$f" && continue
    [[ -n "${exempt[$rel]:-}" ]] && continue
    bare+=("$rel")
  done < <(find "$ROOT" -path "$ROOT/*/src/*" -name '*.rs' -not -path '*/target/*' | sort)

  if (( ${#bare[@]} > 0 )); then
    if (( DEFERRED_OK )); then
      info "--deferred-ok: ${#bare[@]} source file(s) still have no test and no exemption row:"
      printf '      %s\n' "${bare[@]}"
    else
      fail "${#bare[@]} source file(s) have no test and no exemption row:"
      printf '      %s\n' "${bare[@]}"
    fi
  elif (( bad == 0 )); then
    ok "$rows exemption row(s) verified; every source file is tested or exempt"
  fi
fi

# --- 8. law rows are answerable ---------------------------------------------
#
# Check 2 already refuses a *claim* test that does not exist. This is the same rule for the law
# catalogue: a row of `spec/INVENTORY.md` that claims coverage (its status cell says "checked") must
# name a Lean module that exists under `spec/Rchain/`, a corpus that exists under
# `spec/conformance/`, and a Rust file that exists — the three things a reader needs to check the row
# themselves. A row that does *not* claim coverage must say so in one of the closed vocabulary words,
# so a new row cannot be added without either evidence or an explicit "open"/"boundary"/"orphaned"/
# "axiomatic" token. That is the check that would have caught every one of C30-C40's gaps: each was a
# row whose status read better than its evidence.
printf '\n== law rows (INVENTORY: every claim backed by a file) ==\n'
law_rows=0 law_bad=0
while IFS= read -r row; do
  # Only the rows 30-43 carry the corpus/consumer convention; 1-29 predate it and name Scala
  # oracles and Lean theorems instead, so they are checked by row 6's register, not here.
  num="$(printf '%s' "$row" | sed -E 's/^\| *([0-9]+) .*/\1/')"
  [[ "$num" =~ ^[0-9]+$ ]] && (( num >= 30 )) || continue
  law_rows=$((law_rows + 1))
  # `| 32 | Rholang | … | <Lean> | <corpus + consumer> | <status> |` — the leading `|` makes field 1
  # empty, so the cells are 5, 6 and 7. (Getting this wrong is not a small mistake: with `lean`=$4 the
  # check read the *law* cell, found no file, and reported "ok" for every row — the vacuous-claim
  # failure it exists to refuse, found by falsifying it.)
  status="$(printf '%s' "$row" | awk -F'|' '{print $7}')"
  lean="$(printf '%s' "$row" | awk -F'|' '{print $5}')"
  corpus="$(printf '%s' "$row" | awk -F'|' '{print $6}')"
  if printf '%s' "$status" | grep -qiE '\*\*checked\*\*|checked'; then
    # A checked row must name something this check can *verify*. Without these two counts the check
    # is vacuous for a row whose text does not match the patterns below — it would report "ok" for a
    # row naming a file that does not exist under a different spelling, which is the class of claim
    # this check exists to refuse. The character classes admit **digits**, which every layer before
    # `c21` happened not to need: `[a-zA-Z_]` rejected `spec/conformance/c21.tsv` and
    # `rholang/tests/lean_c21_corpus.rs` as if the row named nothing, which is this check reporting a
    # stale row where the row was new.
    found_lean="$(printf '%s' "$lean" | grep -coE 'Rchain/[A-Za-z]+\.lean' || true)"
    found_corpus="$(printf '%s' "$corpus" | grep -coE 'spec/conformance/[a-zA-Z0-9_]+\.tsv' || true)"
    (( found_lean > 0 )) || { fail "INVENTORY row $num claims coverage but names no spec/Rchain/*.lean module"; law_bad=$((law_bad + 1)); }
    (( found_corpus > 0 )) || { fail "INVENTORY row $num claims coverage but names no spec/conformance/*.tsv"; law_bad=$((law_bad + 1)); }
    # a checked row: the Lean module, the corpus and the consumer must all exist
    for decl in $(printf '%s' "$lean" | grep -oE 'Rchain/[A-Za-z]+\.lean' | sort -u); do
      [[ -f "$ROOT/spec/$decl" ]] || { fail "INVENTORY row $num names $decl, which does not exist"; law_bad=$((law_bad + 1)); }
    done
    for f in $(printf '%s' "$corpus" | grep -oE 'spec/conformance/[a-zA-Z0-9_]+\.tsv' | sort -u); do
      [[ -f "$ROOT/$f" ]] || { fail "INVENTORY row $num names $f, which does not exist"; law_bad=$((law_bad + 1)); }
    done
    for t in $(printf '%s' "$corpus" | grep -oE '(rholang|node)/tests/[a-z0-9_]+\.rs' | sort -u); do
      [[ -f "$ROOT/$t" ]] || { fail "INVENTORY row $num names $t, which does not exist"; law_bad=$((law_bad + 1)); }
    done
  else
    printf '%s' "$status" | grep -qiE 'open|boundary|orphaned|axiomatic|deviation' \
      || { fail "INVENTORY row $num neither claims coverage nor says why not (status: $status)"; law_bad=$((law_bad + 1)); }
  fi
done < <(grep -E '^\| *[0-9]+ \|' "$ROOT/spec/INVENTORY.md")
if (( law_bad == 0 )); then
  ok "$law_rows law row(s) 30+ name real files or state their status"
fi

# --- 9. the register's prose line-citations resolve --------------------------
#
# The register's own checks (`Rchain/LawsMain.lean`) verify that an anchor's *file* exists
# (`rustAnchorFailures`) — a file does not rot — but the line numbers a row's prose carries are written
# by hand and nothing read them, so five rows had drifted silently by 2026-09-24 (law 28 whole, law
# 14a's finalizer cites, law 44's `debit_pos_vault`, law 9's concatenation, law 3's `par_concat`)
# while every other check stayed green. This is that reader: for every `path:line` / `path:line-line`
# citation in a row's prose — and every bare `:NNN` continuation, which inherits the most recent path
# in the same cell the way a reader reads it — the path must resolve, the line must be inside the file,
# and the cited window (±8 lines) must contain an identifier the row itself names in backticks. The
# last clause is the one that catches rot: a line that moved now points at unrelated code, so the
# symbol the row is talking about is not there.
#
# **What it measures, stated because it is the trap this file's own inventory check warns about**: the
# *emitted* `spec/laws.tsv`, not `spec/Rchain/Laws.lean` and not the working tree. A row you have just
# edited in the Lean will not be read here until `tools/emit-lean-laws.sh` has run, so a citation that
# this check still calls stale after an edit means the edit is not emitted (or the register did not
# build) — not that the check is wrong. Why a shell check and not a register check: this needs only the
# TSV, so it runs in the fast audit rather than behind a `lake build`. Why the window is ±8 rather than
# exact: a citation usually names the function whose *body* holds the claim ("the early return is at
# `:1341-1343`"), so the symbol sits a few lines above it. The convention the rows follow is in
# `spec/STYLE.md`.
#
# **What the window match is, and is not**: it is a *substring* test, so a cited line whose window holds
# a test named after the symbol (`calculate_finalization_returns_none_…` for `calculate_finalization`)
# counts as holding it. That permissiveness is deliberate — this check's failure mode to avoid is crying
# wolf on a correct citation, which it did three times today before the camel/snake and dot-shape fixes —
# and it still catches the rot it exists for, which is a *moved* line: there the window holds nothing the
# row names at all. Measured both ways (2026-09-24): `finalizer.rs:99` (the symbol's own line) passes;
# `:45` and `:250` fail; `:400` passes only because a test there is named after the function.
printf '\n== register anchors (every cited line still holds what the row says) ==\n'
ANCHOR_CITE='([A-Za-z0-9_][A-Za-z0-9_./-]*\.(rs|v|lean|md|scala|toml|sh|tsv|json|rhox|rho)):([0-9]+)(-([0-9]+))?|:([0-9]+)(-([0-9]+))?'
# **A citation may name a token instead of a line** (`path:identifier`), and that is the form that
# cannot rot: a line-anchored citation into a script moves whenever a step above it is edited, and
# nothing in the register can tell. Law 30's moved **twice in one day** (AUDIT C70, C76) — once by a
# 40-line growth, once by a 13-line `ulimit` comment — and both times the audit was the only thing that
# could see it. So the symbol form is *checked*, not merely tolerated: the identifier must occur in the
# cited file, which is the direction that matters, because a renamed function must break its citation
# here rather than pass as prose.
ANCHOR_SYM='([A-Za-z0-9_][A-Za-z0-9_./-]*\.(rs|v|lean|md|scala|toml|sh|tsv|json|rhox|rho)):([A-Za-z_][A-Za-z0-9_]*)'
ANCHOR_INDEX="$(cd "$ROOT" && git ls-files | awk -F/ '{print $NF"\t"$0}' | sort)"

# resolve a citation's path to a file: as written, then by the row's own anchors (basename), then by a
# unique basename in the tree. Sets the global `file` ("" when it does not resolve) — shared by both
# citation forms, so the two cannot drift apart in how they resolve a path.
anchor_resolve() {
  file=""
  if [[ "$1" == */* ]]; then
    for cand in "$ROOT/$1" "$ROOT/spec/$1"; do [[ -f "$cand" ]] && { file="$cand"; break; }; done
  else
    local pref matches
    pref="$(printf '%s\n' "$row_anchors" | grep -E "/$1\$" | head -1 || true)"
    [[ -n "$pref" && -f "$ROOT/$pref" ]] && file="$ROOT/$pref"
    if [[ -z "$file" ]]; then
      matches="$(printf '%s\n' "$ANCHOR_INDEX" | awk -F'\t' -v b="$1" '$1 == b {print $2}')"
      [[ "$(printf '%s\n' "$matches" | grep -c . || true)" == "1" ]] && file="$ROOT/$matches"
    fi
  fi
}

# Is `$2` present in file `$1`? A citation may name the model's snake_case spelling or the port's
# camelCase one — the same asymmetry the window check below handles, and for the same reason: refusing
# a correct citation is the failure mode this check must not have.
sym_present() {
  local v="$2" camel snake
  if grep -qE "(^|[^A-Za-z0-9_])$v([^A-Za-z0-9_]|\$)" "$1"; then return 0; fi
  camel="$(printf '%s' "$v" | awk -F_ '{s=$1; for(i=2;i<=NF;i++) s=s toupper(substr($i,1,1)) substr($i,2); print s}')"
  if [[ "$camel" != "$v" ]] && grep -qE "(^|[^A-Za-z0-9_])$camel([^A-Za-z0-9_]|\$)" "$1"; then return 0; fi
  snake="$(printf '%s' "$v" | awk '{s=""; for(i=1;i<=length($0);i++){c=substr($0,i,1); if (c ~ /[A-Z]/) s=s "_" tolower(c); else s=s c} print s}')"
  if [[ "$snake" != "$v" ]] && grep -qE "(^|[^A-Za-z0-9_])$snake([^A-Za-z0-9_]|\$)" "$1"; then return 0; fi
  return 1
}
anchor_total=0
anchor_bad_before=$failures
# The delimiter is `\034`, not tab, and that is load-bearing: `read` treats tab as IFS *whitespace*,
# so it collapses an **empty field** — and every single-clause law has an empty `clause` column, which
# shifted every later variable by one and left `falsifiable` unscanned entirely. (The first draft used
# tab; the falsifier — a past-EOF line injected into `falsifiable` — reported green, which is how the
# shift was found. A check that scans the wrong column is not evidence, so `\034` it is.) That trap is
# not exotic: it is what *any* tab-separated register consumer written in shell will do, so a new one
# should start from this loop rather than from `IFS=$'\t'`.
while IFS=$'\034' read -r num clause layer status decls axioms corpus rust coq witness falsifiable statement note; do
  [[ "$num" == "number" ]] && continue
  # the row's own vocabulary: the backticked, identifier-shaped tokens it names anywhere in its prose
  # `|| true` on every pipeline whose last `grep` may find nothing: under this script's
  # `set -euo pipefail` a no-match grep inside a command substitution aborts the run, which is the trap
  # the inventory counts' own comment names — and which the first draft of this check fell into.
  row_anchors="$(printf '%s\n' "$rust" "$coq" | tr ',' '\n' | sed -e 's/^ *//' -e 's/ *$//' -e 's/:.*$//' | grep -v '^-$' || true)"
  # The token shape admits dotted and `::`-qualified *method and field paths* — a row that names
  # `produce_refs.sort_by_key` or `EventLogIndex::combine` is naming the code as precisely as one that
  # names a bare function, and a filter that dropped them reported a correct citation stale.
  row_idents="$(printf '%s\n' "$statement" "$falsifiable" "$note" "$decls" "$witness" \
    | grep -oE '`[^`]+`' | tr -d '`' | grep -E '^[A-Za-z_][A-Za-z0-9_]*([.:]{1,2}[A-Za-z_][A-Za-z0-9_]*)*(\(\))?$' | sort -u || true)"
  for cell in "$statement" "$falsifiable" "$note"; do
    text="$cell"; cite_path=""
    while [[ "$text" =~ $ANCHOR_CITE ]]; do
      cite="${BASH_REMATCH[0]}"
      if [[ -n "${BASH_REMATCH[1]:-}" ]]; then
        cite_path="${BASH_REMATCH[1]}"; from="${BASH_REMATCH[3]}"; to="${BASH_REMATCH[5]:-${BASH_REMATCH[3]}}"
      else
        from="${BASH_REMATCH[6]}"; to="${BASH_REMATCH[8]:-${BASH_REMATCH[6]}}"
      fi
      text="${text#*"$cite"}"
      [[ -z "$cite_path" ]] && continue
      anchor_total=$((anchor_total + 1))
      # A path with an ellipsis is a pointer for a reader, not a citation this can resolve.
      [[ "$cite_path" == *...* ]] && continue
      anchor_resolve "$cite_path"
      if [[ -z "$file" ]]; then
        fail "law ${num}${clause}: \`$cite_path:$from\` does not resolve — write the path in full (its basename is ambiguous or absent)"
        continue
      fi
      if (( from > $(wc -l < "$file") )); then
        fail "law ${num}${clause}: \`$cite_path:$from\` is past the file's end ($(wc -l < "$file") lines)"
        continue
      fi
      window="$(sed -n "$(( from > 8 ? from - 8 : 1 )),$(( to + 8 ))p" "$file")"
      hit=""
      while IFS= read -r id; do
        [[ -z "$id" ]] && continue
        [[ "$window" == *"$id"* ]] && { hit="$id"; break; }
        # the model's names are snake_case and the oracle's camelCase: compare both spellings
        camel="$(printf '%s' "$id" | awk -F_ '{s=$1; for(i=2;i<=NF;i++) s=s toupper(substr($i,1,1)) substr($i,2); print s}')"
        [[ "$camel" != "$id" && "$window" == *"$camel"* ]] && { hit="$id"; break; }
        # …and the other direction: a row that names the *model's* `checkMinMessages` is citing the
        # port's `check_min_messages`, so the row's own spelling must be snake-ised too. Without this
        # the rule reported a correct citation stale (found on law 14b's row, 2026-09-24).
        snake="$(printf '%s' "$id" | awk '{s=""; for(i=1;i<=length($0);i++){c=substr($0,i,1); if (c ~ /[A-Z]/) s=s "_" tolower(c); else s=s c} print s}')"
        [[ "$snake" != "$id" && "$window" == *"$snake"* ]] && { hit="$id"; break; }
      done <<< "$row_idents"
      if [[ -z "$hit" ]]; then
        fail "law ${num}${clause}: \`$cite_path:$from-$to\` holds no identifier the row names — $(sed -n "${from}p" "$file" | cut -c1-60)"
      fi
    done
    # The same row's citations in **symbol form** (`path:identifier`), separated from the line form
    # because the two have different obligations: a line citation is checked against a window, a symbol
    # citation against the whole file, and a symbol citation that names nothing must fail here rather
    # than be skipped — a silently-skipped citation is the blindness this check exists to prevent.
    symtext="$cell"
    while [[ "$symtext" =~ $ANCHOR_SYM ]]; do
      cite="${BASH_REMATCH[0]}"
      cite_path="${BASH_REMATCH[1]}"; sym="${BASH_REMATCH[3]}"
      symtext="${symtext#*"$cite"}"
      anchor_total=$((anchor_total + 1))
      [[ "$cite_path" == *...* ]] && continue
      anchor_resolve "$cite_path"
      if [[ -z "$file" ]]; then
        fail "law ${num}${clause}: \`$cite_path:$sym\` does not resolve — write the path in full (its basename is ambiguous or absent)"
        continue
      fi
      if ! sym_present "$file" "$sym"; then
        fail "law ${num}${clause}: \`$cite_path:$sym\` names no \`$sym\` in that file — the symbol form is *checked*, so a rename must land with its citation"
      fi
    done
  done
done < <(awk -F'\t' 'BEGIN { OFS="\034" } { $1 = $1; print }' "$ROOT/spec/laws.tsv")
# A check that scanned nothing is not evidence: if the TSV's columns move, this must fail loudly rather
# than report success over zero citations — the trap check 8's own comment records finding.
(( anchor_total > 0 )) || fail "no register citations found — the TSV's columns moved and this check is vacuous"
if (( failures == anchor_bad_before )); then
  ok "$anchor_total register citation(s) resolve, and each cited window holds what the row names"
fi

# --- 10. a layer count is that layer's count ---------------------------------
#
# Check 9 covers a row's *citations* against the tree; this covers its **counts**. The counts emitter
# owns the marked spans and the `N laws`/`N entries` phrasing, and `INVENTORY.md`'s cells and the
# register's notes were unchecked prose — so a cell could say "66 rows" about a corpus that had grown
# to 123 and nothing read it. Measured before writing this (2026-09-24): 15 count claims in
# `INVENTORY.md`, 13 of them layer counts, 11 correct — and the one that was wrong, `envelope.tsv`
# described as 6 rows against a file of 9, had been wrong since it was written.
#
# The rule is deliberately narrow, because a check that reads any number near any word would fire on
# arithmetic and on history. A count is judged only when it is written
# **`N rows|cases|verdicts|lines`** — the number *before* the count word — and the most recent
# `conformance/<layer>.tsv` named *earlier in the same line* supplies the layer. Two shapes are
# therefore outside it, and they are boundary rather than exception (named here so a reader knows the
# check is not claiming them): a **Lean** count (`parseDeviations`, 14 rows — the deviation table, not a
# corpus, and `INVENTORY.md` says so in the cell), and a **range** (`cases 7–12`, where the word comes
# before the number). Both are counted by nothing, and neither is silently skipped.
printf '\n== register counts (a count beside a corpus file is that file'"'"'s count) ==\n'
count_total=0
count_bad_before=$failures
for src in "$ROOT/spec/INVENTORY.md" "$ROOT/spec/laws.tsv"; do
  while IFS= read -r line || [[ -n "$line" ]]; do
    last_layer=""
    while IFS= read -r tok; do
      case "$tok" in
        conformance/*.tsv)
          last_layer="$(basename "$tok" .tsv)" ;;
        *)
          [[ -z "$last_layer" ]] && continue
          n="$(printf '%s' "$tok" | tr -dc '0-9')"
          wish="$(wc -l < "$ROOT/spec/conformance/$last_layer.tsv" 2>/dev/null || echo 0)"
          count_total=$((count_total + 1))
          if [[ "$n" != "$wish" ]]; then
            fail "$(basename "$src"): '$tok' counts $last_layer, whose committed corpus has $wish line(s)"
          fi ;;
      esac
    done < <(printf '%s' "$line" \
      | grep -oE 'conformance/[a-z0-9_]+\.tsv|[(]?[*]{0,2}[0-9]+ (rows|cases|verdicts|lines)' || true)
  done < "$src"
done
# A check that matched nothing is not evidence, exactly as in check 9.
(( count_total > 0 )) || fail "no register counts found — this check is vacuous (the count phrasing moved?)"
if (( failures == count_bad_before )); then
  ok "$count_total layer count(s) agree with their committed corpus"
fi

# --- summary -----------------------------------------------------------------
printf '\n===== summary =====\n'
if (( failures == 0 )); then
  echo "OK: the register matches the tree."
  exit 0
fi
echo "FAIL: $failures problem(s) between spec/TEST-COVERAGE.md and the tree."
exit 1
