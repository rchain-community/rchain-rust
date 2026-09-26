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
#  10. **A layer count is that layer's count** — an `N rows|cases|verdicts|lines` phrase beside a
#      `conformance/<layer>.tsv` must equal that corpus's committed line count, so a register cell
#      cannot go on describing a corpus that has grown past it.
#  11. **The coverage measurement is emitted, and the floor is its own** — `spec/COVERAGE-LEDGER.md`
#      must be what `tools/emit-coverage-ledger.sh` emits from `lcov.info`, and CI's
#      `--fail-under-lines` must be `floor(measured) − 2`: the rule the register states, and the one
#      all four of its recorded raisings satisfy. The register itself may only *point at* the
#      measurement — a hand-written current percentage in `spec/TEST-COVERAGE.md` fails, because that
#      is precisely the shape that rots. Item 11's hand-written order ("uncovered lines") named the
#      file ranked 15th first, while the measurement's actual leaders had no register row at all.
#  12. **The status vocabulary names no law** — `spec/Rchain/Laws.lean`'s module doc defines each status
#      in one bullet, and those bullets used to carry their examples as law numbers. Measured
#      2026-09-25: `open`'s list was wrong in four of its five entries (the rows had all moved on) and
#      `orphaned`'s was right by luck. A reader goes to the bullet to *learn* the word, so a wrong list
#      there teaches the wrong word — `spec/STYLE.md`'s own rule against restating a status in prose.
#      The examples belong to the emitted register (`spec/laws.tsv`, `spec/LAWS.md`), and this check is
#      what keeps them there: a law number in a status bullet fails, and the paragraph that *quotes* the
#      two old lists does so outside a bullet, which is why the scan follows the bullets and not the file.
#  13. **A law row's claim agrees with the register's status** — `spec/INVENTORY.md`'s rows 30+ say in a
#      cell whether the law is covered, and those cells had drifted: row 47's read "**open** —
#      **proved-model**" (both words, contradicting each other and the register) and row 36's still read
#      "open" as though no model existed. The register is the oracle for a status, and the catalogue's
#      vocabulary is coarser than the register's, so what is checked is the **polarity**: the cell's
#      *first* bold word must claim coverage exactly when the register says the law is proved. The first
#      draft tested whether the cell contained *any* covering word — and passed the very drift it was
#      written for, because row 47's cell contained `proved-model` after its `open` (found by re-planting
#      the defect and reading `audit rc=0`). Later qualifiers stay allowed: a proved row's tie *is* a
#      `boundary`, and one of its forms *is* `owed`, and both are true sentences about part of a law.
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
SPEC="$ROOT/spec"
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
    # `owed` joined this vocabulary on 2026-09-26 with law 50: the register has carried that status
    # since the consolidation pass (`Rchain/Laws.lean`'s `Status.owed` — "definition exists, proof
    # missing"), and a row saying it is exactly a row saying why it does not claim coverage. The
    # checker refusing it was the checker being narrower than the vocabulary it is checking.
    printf '%s' "$status" | grep -qiE 'open|boundary|orphaned|axiomatic|deviation|owed' \
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
#
# **`return 0` is load-bearing, and it is C89.** A function ends with the status of its last command, and
# every branch here ends on a *test*: the `for` loop's last command is the `[[ -f "$cand" ]]` that failed,
# and the `if`/`else` propagates it. So when a citation does not resolve — exactly the case this function
# exists to report — `anchor_resolve` returned 1, and under this script's `set -euo pipefail` that aborted
# the run *before* the caller's `if [[ -z "$file" ]]` could print `does not resolve — write the path in
# full`. Measured 2026-09-25: an unresolvable `Crypto/Spec.lean:41-49` in row 19's note made the audit
# exit 1 with the log ending at `== register anchors …` and **zero** FAIL lines — a check that reported
# nothing rather than reporting the defect, which is the failure mode this whole pass is about. The
# function's contract is "set `file`, say nothing": it must return success in every case.
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
  return 0
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
while IFS=$'\034' read -r num clause layer status decls axioms corpus rust coq witness falsifiable statement note rustWitness; do
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
#
# **And a third shape is a *boundary*, not a check: a count written in words.** The rule above matches
# digits, so "twenty-three pairwise verdicts" — law 1a's `falsifiable` cell, describing the `sort`
# layer — was read by no check at all, and it had drifted: the layer holds **25** verdicts, which
# `INVENTORY.md`'s row for the same law already said. (Two errors, in fact: the phrase was stale, and an
# earlier reading of it called the drift "off by one" by subtracting a header the corpora do not have —
# `wc -l < spec/conformance/sort.tsv` is 25 and there is no header line to remove.) The phrase is fixed
# (it now names `conformance/sort.tsv` and carries the digit, so this check judges it).
#
# **A check for the word form was written and then withdrawn, and the measurement is why.** Firing on
# `\b(one|two|…|fifty)(-[a-z]+)? (rows|cases|verdicts|lines)\b` beside a layer token reported four
# hits in `INVENTORY.md`, and three of them were *history*: "The layer's first consumer run found three
# rows the model did not know", "a three-valued verdict" — prose recounting a run, not a claim about a
# corpus. A check that fires on prose is a check people learn to work around (the gate's own `sorry`
# scan says the same at its step 2), and the class it would catch is already covered by the rule the
# repo states: **a count is written in digits or as a generated span** (`spec/STYLE.md`, "the register is
# the oracle"). What a word-form count lacks is not a check but a *reason to exist*: the digit form is
# the one that is read.
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

# --- 11. the coverage measurement is emitted, and the floor is its own ------------------------
#
# Checks 1–10 bind every *other* number in this register to the tree: test counts are recounted, law
# citations are resolved, corpus counts are read off the committed `.tsv`. Coverage was the exception —
# a number in the register that nothing could recompute, and it showed. Definition-of-done item 11 says
# its work is "ordered by uncovered lines"; the order it carried was written by hand from an
# `lcov.info` dated 2026-09-22 and named `node/src/api/grpc/tonic.rs` (121 missed lines) first, while
# the measurement ranked `rholang/src/reduce.rs` (648) and `rholang/src/system_processes.rs` (510) above
# it and the two largest files in the top ten had no register row at all. So the ranking, the totals and
# the implied floor are emitted by `tools/emit-coverage-ledger.sh`, and this check is what keeps the
# emission honest in the two directions that matter:
#
#   * **the ledger is the emission** — its rows must be what the emitter produces from the committed
#     `lcov.info` right now, refused with a diff otherwise (the same discipline `emit-lean-laws.sh` and
#     `emit-lean-counts.sh` use for their emissions); and
#   * **the floor is derived, not remembered** — `.github/workflows/coverage.yml`'s
#     `--fail-under-lines` must equal `floor(measured) − 2`. Too high is a tripwire the measurement does
#     not support; too low is a raise that was owed. The emitter owns that comparison, so there is one
#     implementation of the rule and not two that can disagree.
#
# **And the register may not restate it.** A live percentage or floor written by hand in
# `spec/TEST-COVERAGE.md` is the rot this closes, so the scan is a shape rule rather than a value rule:
# a decimal percentage, or a `floor to <n>`, anywhere in the register fails. History is written without
# the sign — `73.68 ⇒ 71` is a record of four raisings and reads the same — and the boundary is the
# register alone: `spec/COVERAGE-LEDGER.md` is where the numbers live, and the laws' and audit
# documents are not scanned (a percentage there is about something else).
printf '\n== coverage ledger (the ranking and the floor are emitted, not remembered) ==\n'
ledger_bad_before=$failures
LEDGER="$ROOT/spec/COVERAGE-LEDGER.md"
EMITTER="$ROOT/tools/emit-coverage-ledger.sh"
FLOOR_SITE="$ROOT/.github/workflows/coverage.yml"

if [[ ! -f "$EMITTER" ]]; then
  fail "tools/emit-coverage-ledger.sh is missing — nothing owns the measurement, so this register's coverage claims are unbacked"
elif [[ ! -f "$LEDGER" ]]; then
  fail "spec/COVERAGE-LEDGER.md is missing — emit it: tools/emit-coverage-ledger.sh"
else
  # **The check does not need the lcov, and that is deliberate.** `lcov.info` is gitignored (a build
  # artifact the size of the workspace), and CI's register gate runs *before* the coverage step that
  # would produce it — so a check that demanded the lcov would fail in CI for want of a file the
  # register deliberately does not keep. What is durable is the ledger: it holds the measured totals,
  # and everything below is recomputed from them.
  #
  # Read the rows **by label**: `| lines | <found> | <hit> | <missed> | <pct>% | <implied floor> |`, and
  # the same row for `functions`. The first version matched the row by *shape* (`^\| [0-9]+ \| …`), which
  # found nothing at all once a label column was added — a check that goes quietly vacuous, which is
  # exactly the failure this register exists to refuse, so the labels are named (AUDIT C107).
  read_coverage_row() { # $1 = label; prints "found hit missed pct floor"
    awk -F'|' -v label="$1" '
      $2 ~ "^ *" label " *$" { for (i = 3; i <= 7; i++) gsub(/ /, "", $i); print $3, $4, $5, $6, $7; exit }
    ' "$LEDGER"
  }
  lines_row="$(read_coverage_row lines)"
  functions_row="$(read_coverage_row functions)"
  if [[ -z "$lines_row" ]]; then
    fail "spec/COVERAGE-LEDGER.md has no lines totals row — this check would be vacuous (the table shape moved?)"
  elif [[ -z "$functions_row" ]]; then
    fail "spec/COVERAGE-LEDGER.md has no functions totals row — the function floor CI enforces would be checked nowhere"
  else
    read -r l_found l_hit l_missed l_pct l_floor <<<"$lines_row"
    read -r f_found f_hit f_missed f_pct f_floor <<<"$functions_row"
    l_pct="${l_pct%\%}"   # the cell carries the sign; the messages below add their own
    l_pct100="$(printf '%s' "$l_pct" | tr -d '%' | awk -F. '{ printf "%d%02d\n", $1, $2 }')"
    # (a) the ledger's own arithmetic.
    if (( l_found - l_hit != l_missed )); then
      fail "the ledger's totals disagree with themselves: $l_found found − $l_hit hit ≠ $l_missed missed"
    fi
    want_pct100=$(( l_hit * 10000 / l_found ))
    if (( l_pct100 != want_pct100 )); then
      fail "the ledger says $l_pct% ($l_found/$l_hit) — the ratio its own totals give is $(( want_pct100 / 100 )).$(printf '%02d' $(( want_pct100 % 100 )))%"
    fi
    want_floor=$(( (want_pct100 - 200) / 100 ))
    if (( l_floor != want_floor )); then
      fail "the ledger's implied floor is $l_floor but its measurement implies $want_floor (floor = floor(measured) − 2)"
    fi
    # (b) CI's floor is that floor.
    ci_floor="$(grep -oE 'fail-under-lines [0-9]+' "$FLOOR_SITE" 2>/dev/null | grep -oE '[0-9]+' || true)"
    if [[ -z "$ci_floor" ]]; then
      fail "no --fail-under-lines in .github/workflows/coverage.yml — the floor this ledger implies has no site"
    elif (( ci_floor != want_floor )); then
      fail "CI's floor is $ci_floor; the committed measurement ($l_pct%) implies $want_floor"
    fi
    # (b1) the same three claims for the **function** row, and CI's flag for it (AUDIT C107). The
    # audit's item asked for branch coverage and the pinned toolchain refuses to collect it, so the
    # second instrument is functions — which makes this the check that keeps the second floor from
    # being a number nobody recomputes, the thing every other floor here was written to avoid.
    if (( f_found - f_hit != f_missed )); then
      fail "the ledger's function totals disagree with themselves: $f_found found − $f_hit hit ≠ $f_missed missed"
    fi
    f_want_pct100=$(( f_hit * 10000 / f_found ))
    f_want_floor=$(( (f_want_pct100 - 200) / 100 ))
    if (( f_floor != f_want_floor )); then
      fail "the ledger's implied function floor is $f_floor but its measurement implies $f_want_floor (floor = floor(measured) − 2)"
    fi
    fn_ci_floor="$(grep -oE 'fail-under-functions [0-9]+' "$FLOOR_SITE" 2>/dev/null | grep -oE '[0-9]+' || true)"
    if [[ -z "$fn_ci_floor" ]]; then
      fail "no --fail-under-functions in .github/workflows/coverage.yml — the function floor this ledger implies has no site"
    elif (( fn_ci_floor != f_want_floor )); then
      fail "CI's function floor is $fn_ci_floor; the committed measurement ($f_pct) implies $f_want_floor"
    fi
    # (b2) and the *raise history* the ledger states is the one its site records. The template used to
    # say "the four times it has been raised" while `coverage.yml` held six — a hand-written count among
    # machine-checked numbers, which is why the emitter now derives the sentence from the site and this
    # compares the two. Without it, a ledger that was not re-emitted after a raising would read as
    # current: the pairs are the record of the rule being applied, and a stale list is a stale ledger.
    ledger_raises="$(grep -oE '[0-9]+\.[0-9]+⇒[0-9]+' "$LEDGER" 2>/dev/null | tr '\n' ' ' | sed 's/ $//' || true)"
    site_raises="$(grep -oE '[0-9]+\.[0-9]+ *=> *[0-9]+' "$FLOOR_SITE" 2>/dev/null | sed 's/ *=* *> */⇒/' | tr '\n' ' ' | sed 's/ $//' || true)"
    if [[ -z "$site_raises" ]]; then
      fail "no raise history in .github/workflows/coverage.yml — the ledger's own history sentence has no site to be checked against"
    elif [[ "$ledger_raises" != "$site_raises" ]]; then
      fail "the ledger's raise history is '$ledger_raises'; the floor's site records '$site_raises' — the ledger is stale, or the site moved without it"
    fi
    # (c) and, when the lcov is present (locally, or in the coverage job), the ledger is the emission.
    # Absent, this is skipped **with a note** rather than silently — the difference between "checked and
    # agreed" and "not checked here" is the whole reason this register exists.
    if [[ -f "$ROOT/lcov.info" ]]; then
      if ledger_out="$("$EMITTER" --check 2>&1)"; then
        ok "coverage ledger matches lcov.info ($l_found lines, $l_missed missed; $f_found functions, $f_missed missed) and the CI floors are $ci_floor / $fn_ci_floor"
      else
        printf '%s\n' "$ledger_out" | sed 's/^/      /'
        fail "the coverage ledger is not what lcov.info emits, or a CI floor is not floor(measured) − 2"
      fi
    else
      info "no lcov.info here: the ledger's rows were not compared against a measurement (CI's coverage job has no copy either — the file is not committed)"
      ok "coverage ledger is self-consistent ($l_found lines, $l_missed missed; $f_found functions, $f_missed missed) and CI's floors $ci_floor / $fn_ci_floor are the ones it implies"
    fi
  fi
fi

while IFS=: read -r line_no rest; do
  [[ -z "$line_no" ]] && continue
  fail "spec/TEST-COVERAGE.md:$line_no states a coverage figure by hand ('$(printf '%s' "$rest" | sed 's/^[[:space:]]*//' | cut -c1-56)…') — the measurement lives in spec/COVERAGE-LEDGER.md; point at it, and write history without the sign"
done < <(grep -nE '([0-9]+\.[0-9]+%|floor to [*]{0,2}[0-9]+)' "$REGISTER" || true)

if (( failures == ledger_bad_before )); then
  ok "the register points at the measurement and states no coverage figure of its own"
fi

# --- 13. a law row's claim agrees with the register's status --------------------
#
# The register (`spec/laws.tsv`, emitted from `Rchain/Laws.lean`) is the oracle for a law's status, and
# `spec/INVENTORY.md`'s rows 30+ say in a *cell* whether the law is covered. Those two drifted: measured
# 2026-09-25, row 47's cell read "**open** — **proved-model**" — both words, in one cell, contradicting
# each other and the register — and row 36's still read "open" as though the model did not exist, four
# hours after the subject for its proof landed. A cell is prose a reader trusts, so the *polarity* has to
# be checked even where the vocabulary cannot: `INVENTORY.md`'s closed word set (`open`, `boundary`,
# `orphaned`, `axiomatic`, `deviation`) is coarser than the register's (`owed`, `vacuous`, `retired`,
# `axiomByDesign`), so this checks one direction only — a row whose status claims *coverage* may not be
# one the register calls unproved, and a row the register calls proved may not read as a gap.
printf '\n== law rows vs the register (a claim agrees with the status) ==\n'
polarity_bad_before=$failures
polarity_checked=0
while IFS= read -r row; do
  num="$(printf '%s' "$row" | sed -E 's/^\| *([0-9]+) .*/\1/')"
  [[ "$num" =~ ^[0-9]+$ ]] && (( num >= 30 )) || continue
  status_cell="$(printf '%s' "$row" | awk -F'|' '{print $7}')"
  # The register's statuses for this law, as the comma-separated set its clauses carry.
  reg="$(awk -F'\t' -v n="$num" 'NR>1 && $1==n {printf "%s ", $4}' "$ROOT/spec/laws.tsv")"
  [[ -n "$reg" ]] || continue
  polarity_checked=$((polarity_checked + 1))
  # **The cell's *first* bold word is its status**, and that is the whole rule. A first draft tested
  # whether the cell contained *any* covering word, and it passed the very drift it was written for —
  # row 47's "**open** — **proved-model**", whose second half satisfied it (measured 2026-09-25 by
  # re-planting the defect: `audit rc=0`). Later qualifiers are legitimate and must stay allowed: a
  # proved row's tie *is* a `boundary` (rows 45/49), and row 47's general `foldl` form is `owed` while
  # the row is proved — both are true sentences about part of a law, and neither is a status.
  # `proved` *and* `proven`: the catalogue uses both spellings, and a pattern that knows only one of
  # them reports a covering row as a gap — a false positive this check must not have either.
  first_bold="$(printf '%s' "$status_cell" | grep -oE '\*\*[^*]+\*\*' | head -1 | tr -d '*')"
  [[ -n "$first_bold" ]] || { fail "INVENTORY row $num has no status word in its cell — a reader cannot tell whether the law is covered"; continue; }
  claims="$(printf '%s' "$first_bold" | grep -ciE '^(checked|proved|proven|proved-model|proved-tied)' || true)"
  proved="$(printf '%s' "$reg" | grep -coE 'proved-tied|proved-model' || true)"
  if (( claims > 0 )) && (( proved == 0 )); then
    fail "INVENTORY row $num claims coverage — its cell opens '**$first_bold**' — while the register's status is '$reg' (nothing proved). The register is the oracle: fix the cell, or fix the row it disagrees with"
  fi
  if (( claims == 0 )) && (( proved > 0 )); then
    fail "INVENTORY row $num reads as a gap — its cell opens '**$first_bold**' — while the register says '$reg'. Either the cell or the register is wrong, and the register is the emitted one"
  fi
done < "$ROOT/spec/INVENTORY.md"
# A check that matched nothing is not evidence.
(( polarity_checked > 0 )) || fail "no law rows were compared against the register — this check is vacuous (the row shape moved?)"
if (( failures == polarity_bad_before )); then
  ok "$polarity_checked law row(s) claim coverage exactly when the register says they have it"
fi

# --- 12. the status vocabulary names no law ------------------------------------
#
# `spec/Rchain/Laws.lean`'s module doc defines each status in one bullet, and the bullets used to carry
# their examples as law numbers — "(laws 30, 31, 33, 34, 36)" for `open`, "(laws 12, 13)" for
# `orphaned`. Measured 2026-09-25: the first was wrong in four of its five entries (every one of those
# rows had moved on) and the second was right by luck; the bullet is where a reader goes to *learn* the
# vocabulary, so a wrong list there teaches the wrong word. That is `spec/STYLE.md`'s own rule — do not
# restate a status in prose — and the ruled fix is that the examples live in the emitted register
# (`spec/laws.tsv`, `spec/LAWS.md`), where they cannot drift. This check is what keeps them out: a law
# number in a status bullet is a failure, and the paragraph that quotes the two old lists quotes them
# *outside* a bullet, which is why the scan follows the bullets rather than the file.
printf '\n== status vocabulary (no status bullet names a law number) ==\n'
vocab_bad_before=$failures
while IFS=: read -r line_no rest; do
  [[ -z "$line_no" ]] && continue
  fail "Rchain/Laws.lean:$line_no names a law number in a status bullet ('$(printf '%s' "$rest" | sed 's/^[[:space:]]*//' | cut -c1-48)…') — the examples are the emitted register's, not the vocabulary's"
done < <(awk '
  /^- `[a-zA-Z-]+` —/ { inb = 1; if ($0 ~ /laws? [0-9]/) print FNR":"$0; next }
  inb && /^  [^ ]/ { if ($0 ~ /laws? [0-9]/) print FNR":"$0; next }
  { inb = 0 }
' "$SPEC/Rchain/Laws.lean" || true)
if (( failures == vocab_bad_before )); then
  ok "the status vocabulary names no law (the emitted register carries the examples)"
fi

# --- 14. the register's pointers resolve ---------------------------------------
#
# Check 9 resolves the citations a row carries in `path:line` form, and checks 1–13 bind every other
# number in the register to the tree. What none of them reads is the **pointer**: a `Cnn` mentioned in
# prose, a `§N` section reference, a `§NNN-MMM` line range, or a `.lean:NNN` citation written inside a
# Rust doc comment. Those are how the record refers to *itself*, and they rot silently — three live
# instances, all measured 2026-09-25:
#
#   * `spec/Rchain/Laws.lean`'s law 16d note says "AUDIT **C90** records it", and the row that records
#     the `PosState.active` gap is **C92**. The note was written when C90 was the next free number and
#     the allocation moved under it — the same shape as law 30's line citation moving twice in one day
#     (AUDIT C70, C76), one level up: not a line that moved but a *number* that did.
#   * `spec/API-SCHEMA.md:92` sends the reader to "AUDIT **§337-343**" for the z-base-32/CRC14 divergence.
#     Those lines are the §7/§8 boundary; the divergence is at `AUDIT.md:455-457`.
#   * `spec/AUDIT.md`'s C56 row says "**§20 below** states it with numbers" — a self-reference: the
#     numbers are in §19, and the design the §5 H6 row promises is "recorded in C56's §20 row" is
#     recorded nowhere at all.
#
# The oracle for "is this number allocated" is the allocator itself (`tools/next-audit-number.sh`), so
# there is one implementation of that question and not two that can disagree — and the numbers that tool
# calls **gaps** (C66) are exactly the ones prose may name without a row to point at, because a gap is
# what a coined-but-unlanded number looks like. That is why the rule is derived rather than exempting
# C66 by hand.
#
# **What this check cannot decide, measured rather than left for the next reader to assume**: a pointer
# whose sentence *describes* the row ("the shape C45 records", "AUDIT C40 records the false statement")
# would need the description read against the row, and a rule loose enough to do that guesses — the six
# correct ones in the tree today share tokens with their rows by coincidence of vocabulary, not by
# construction, and a token rule that "passed" them would pass a wrong number too. So the subject tier
# fires only on the **reflexive** form — "`Cnn` records it/this", where the pointer claims the row
# records the enclosing claim — which is the shape that was wrong, and the wider forms are counted and
# reported as undecided rather than decided badly. The line-range tier is the same discipline in the
# other direction: `§337-343` has no identifier to hold, so its rule is a *distinctive token* of the
# citing sentence, which is what makes the correct window (`CRC14`, `ZBase32`) pass and the §7/§8
# boundary fail.
#
# The emitted files are excluded on purpose: `spec/laws.tsv` and `spec/LAWS.md` are `Rchain/Laws.lean`
# rendered, so scanning them would report every defect three times and make the *source* look optional.
# That is the opposite of check 9's choice (which reads the emitted TSV because a citation is only
# checked once it is emitted), and the reason differs: a citation lives in the row, a pointer lives in
# the source.
printf '\n== pointers (every C-number and section reference resolves) ==\n'
pointer_bad_before=$failures
pointer_checked=0       # every C-number and section reference examined
pointer_subject=0       # references the reflexive (subject) tier decided
pointer_gap_refs=0      # references to a number the allocator calls a gap
pointer_undecided=0     # references this check reads but cannot decide (stated, never skipped silently)
pointer_doc=0           # `.lean` citations inside Rust doc comments

# --- the oracle: what the allocator says is allocated, and what it calls a gap --------------------
alloc_out="$("$ROOT/tools/next-audit-number.sh" 2>/dev/null || true)"
in_use=" $(printf '%s\n' "$alloc_out" | sed -n 's/^in use: //p') "
gap_list=" $(printf '%s\n' "$alloc_out" | sed -n 's/^unused below the maximum.*: //p') "
if [[ -z "${in_use// /}" ]]; then
  fail "tools/next-audit-number.sh listed no allocated C-numbers — every reference below would read as dangling, so this check is vacuous until that tool works"
fi

# A finding's text: its `- **Cnn — …` entry if it has one (the long form), else its `| Cnn … |` §20 row.
# The entry is preferred because it is where the subject words are; the row is the fallback for a number
# that only ever landed in the table, which `tools/next-audit-number.sh`'s own header says happens.
pointer_row_text() {
  local t
  t="$(awk -v n="C$1" '
    /^- \*\*C[0-9]+ / { if (grab) exit; if ($0 ~ ("^- \\*\\*" n " ")) grab = 1 }
    grab { print }
  ' "$ROOT/spec/AUDIT.md")"
  [[ -n "$t" ]] || t="$(grep -m1 -E "^\| C$1 " "$ROOT/spec/AUDIT.md" || true)"
  printf '%s' "$t"
}

# The distinctive vocabulary of a passage: words long enough to be a subject rather than glue. The
# stoplist is not tidiness — `audit`, `records`, `entry`, `register` and `finding` appear in a pointer
# *and* in every row it might point at, so leaving them in would make the reflexive tier pass the very
# defect it exists to catch (measured against C90's row, which contains "audit" twice).
pointer_tokens() {
  tr 'A-Z' 'a-z' | grep -oE '[a-z][a-z0-9]{4,}' \
    | grep -vEx 'audit|record|records|entry|entries|finding|findings|register|noted|notes|about|which|there|where|their|these|those|other|first|second|since|after|before|every|never|still|would|could|should|makes|made|make|must|does|done|from|with|that|this|then|than|when|have|been|also|into|over|only|both|them|they|because' \
    | sort -u
}

# Does the row a pointer names share a distinctive token with the pointer's own sentence? One shared
# token is enough; zero means the row is about something else.
pointer_subject_ok() {
  local n="$1" line="$2" rowtext shared
  rowtext="$(pointer_row_text "$n")"
  [[ -n "$rowtext" ]] || return 1
  shared="$(comm -12 <(printf '%s\n' "$line" | pointer_tokens) <(printf '%s\n' "$rowtext" | pointer_tokens) | head -1)"
  [[ -n "$shared" ]]
}

# The sources: `spec/`'s prose and the Lean modules — not the two emitted renderings.
pointer_sources=()
while IFS= read -r f; do pointer_sources+=("$f"); done < <(
  { ls "$ROOT"/spec/*.md 2>/dev/null; find "$ROOT/spec/Rchain" -name '*.lean' 2>/dev/null; } \
    | grep -vE '/(LAWS\.md|COVERAGE-LEDGER\.md)$' | sort -u
)

# --- family A, tiers 1 and 2: C-number references in prose ------------------------
while IFS=: read -r pfile pline ptext; do
  [[ -z "$pfile" ]] && continue
  # **A word boundary, and it is not decoration**: `C[0-9]+` alone matches inside `RFC1918` and
  # `RFC2253`, which are protocol numbers in two §6 rows — the first run of this check reported both as
  # unallocated findings, which is the same defect the `§337-343` misparse was: an instrument that
  # misreads its subject reports findings about a text nobody wrote.
  for num in $(printf '%s' "$ptext" | grep -oE '(^|[^A-Za-z0-9_])C[0-9]+' | sed 's/^[^C]*//' | sort -u); do
    n="${num#C}"
    pointer_checked=$((pointer_checked + 1))
    if [[ "$in_use" != *" $n "* ]]; then
      if [[ "$gap_list" == *" $n "* ]]; then
        pointer_gap_refs=$((pointer_gap_refs + 1))
        continue
      fi
      fail "${pfile#"$ROOT"/}:$pline names \`$num\`, which no finding claims — the allocator's in-use set (tools/next-audit-number.sh) is the oracle, and a number it does not list has no row to resolve to"
      continue
    fi
    # the reflexive form only: the pointer claims the row records the enclosing claim
    if printf '%s' "$ptext" | grep -qE "(^|[^A-Za-z0-9_])C$n[^.]{0,40}records (it|this)([^a-z]|\$)"; then
      pointer_subject=$((pointer_subject + 1))
      if ! pointer_subject_ok "$n" "$ptext"; then
        fail "${pfile#"$ROOT"/}:$pline says \`$num\` records it, and C$n's own text shares nothing distinctive with that sentence — the pointer names a row about something else"
      fi
    fi
  done
done < <(grep -nHE 'C[0-9]+' "${pointer_sources[@]}" 2>/dev/null || true)

# --- family A, tier 3: section references and line ranges -------------------------
# A `§N` of one or two digits is a section of `spec/AUDIT.md` (it has twenty), and it must name a
# heading. A `§NNN(-NNN)` of three or more cannot be a section, so it is a *line range* into that file —
# and the window must contain a distinctive token of the citing sentence. The split is measured rather
# than assumed: the first draft of this check read `§337-343` as "section 33" and reported a section
# that does not exist, which is a false positive of the check's own making.
while IFS=: read -r pfile pline ptext; do
  [[ -z "$pfile" ]] && continue
  for ref in $(printf '%s' "$ptext" | grep -oE '§[0-9]+(-[0-9]+)?' | sort -u); do
    body="${ref#§}"
    pointer_checked=$((pointer_checked + 1))
    if [[ "$body" =~ ^[0-9]{1,2}$ ]]; then
      if [[ "$pfile" == "$ROOT/spec/AUDIT.md" ]]; then
        # inside AUDIT.md itself a `§N` is the enclosing document's own structure
        grep -qE "^## $body\. " "$ROOT/spec/AUDIT.md" \
          || fail "AUDIT.md:$pline cites §$body, which is not a section of this file (it has 20, and the numbers are checked)"
      else
        grep -qE "^## $body\. " "$ROOT/spec/AUDIT.md" \
          || fail "${pfile#"$ROOT"/}:$pline cites AUDIT §$body, which is not a section of that file"
      fi
    elif [[ "$body" =~ ^([0-9]{3,4})(-([0-9]{3,4}))?$ ]]; then
      pfrom="${BASH_REMATCH[1]}"; pto="${BASH_REMATCH[3]:-$pfrom}"
      if (( pfrom > $(wc -l < "$ROOT/spec/AUDIT.md") )); then
        fail "${pfile#"$ROOT"/}:$pline cites AUDIT §$body, which is past that file's end ($(wc -l < "$ROOT/spec/AUDIT.md") lines)"
        continue
      fi
      pwindow="$(sed -n "$(( pfrom > 3 ? pfrom - 3 : 1 )),$(( pto + 3 ))p" "$ROOT/spec/AUDIT.md")"
      if ! printf '%s\n' "$ptext" | pointer_tokens | while IFS= read -r tok; do
             [[ -n "$tok" ]] || continue
             printf '%s' "$pwindow" | grep -qiF "$tok" && { echo "$tok"; break; }
           done | grep -q .; then
        fail "${pfile#"$ROOT"/}:$pline cites AUDIT §$body and the lines there hold nothing that sentence names — $(sed -n "${pfrom}p" "$ROOT/spec/AUDIT.md" | cut -c1-50)"
      fi
    fi
  done
done < <(grep -nHE '§[0-9]' "${pointer_sources[@]}" 2>/dev/null || true)

# --- family B: the `.lean` line-citations inside Rust doc comments ----------------
# Invisible to every other gate: check 9 reads the register's rows and the type-system audit reads Rust
# *expressions*, so a citation in a `///` comment is checked by nothing. The population is seven
# (re-derived 2026-09-25), and the rule is check 9's window rule with the arrow reversed: the *sentence*
# names an identifier in backticks and the cited range must contain it. A doc comment naming no
# identifier is counted as undecided — the check says how many it could not decide rather than reading
# them as clean.
# `-not -path '*/.lake/*'` is load-bearing: `.lake` holds the vendored Mathlib packages, whose
# `Match.lean`/`Array/Match.lean` make a bare basename ambiguous — and a basename that resolves to two
# files is how this family reported a *correct* citation (`models/src/types.rs:80`'s `Match.lean:402`)
# as one that does not resolve. Third false positive of the first run, and the same shape as the other
# two: the check measured the wrong subject.
lean_index="$(cd "$ROOT" && find spec -name '*.lean' -not -path '*/.lake/*' | awk -F/ '{print $NF"\t"$0}' | sort)"
while IFS= read -r hit; do
  [[ -z "$hit" ]] && continue
  rfile="${hit%%:*}"; rest="${hit#*:}"; rline="${rest%%:*}"; rtext="${rest#*:}"; stext="${rest#*:}"
  while [[ "$rtext" =~ ([A-Za-z0-9_/]*\.lean):([0-9]+)(-([0-9]+))? ]]; do
    cite_path="${BASH_REMATCH[1]}"; from="${BASH_REMATCH[2]}"; to="${BASH_REMATCH[4]:-$from}"
    rtext="${rtext#*"${BASH_REMATCH[0]}"}"
    pointer_doc=$((pointer_doc + 1))
    lfile=""
    if [[ "$cite_path" == */* ]]; then
      for cand in "$ROOT/$cite_path" "$ROOT/spec/$cite_path"; do [[ -f "$cand" ]] && { lfile="$cand"; break; }; done
    else
      matches="$(printf '%s\n' "$lean_index" | awk -F'\t' -v b="$cite_path" '$1 == b {print $2}')"
      [[ "$(printf '%s\n' "$matches" | grep -c . || true)" == "1" ]] && lfile="$ROOT/$matches"
    fi
    if [[ -z "$lfile" ]]; then
      fail "$rfile:$rline cites \`$cite_path:$from\`, which does not resolve — write the path in full (its basename is ambiguous or absent)"
      continue
    fi
    # **Resolvability first, and the order is load-bearing.** A citation past the end of its file is
    # decidable whether or not the sentence names an identifier, and the first draft put the "names
    # nothing, so this check cannot decide it" branch above this test — which made a citation to line
    # 99999 of a 40-line file report as *undecided* rather than as broken. Found by probing it
    # (`Fringe.lean:99999`: `ok … 1 outside what this check reads`), which is the falsifier that keeps
    # "undecided" from becoming a place a defect can hide.
    if (( from > $(wc -l < "$lfile") )); then
      fail "$rfile:$rline cites \`$cite_path:$from\`, which is past that file's end ($(wc -l < "$lfile") lines)"
      continue
    fi
    # the identifiers the sentence names: the citation line and its doc-comment neighbours (a citation
    # often closes a sentence whose symbol was named the line before — property_tests.rs:66's
    # `validShardId` is on `:65`)
    idents="$(sed -n "$(( rline > 2 ? rline - 2 : 1 )),$(( rline + 2 ))p" "$ROOT/$rfile" \
      | grep -oE '`[A-Za-z_][A-Za-z0-9_]*`' | tr -d '`' | sort -u || true)"
    if [[ -z "$idents" ]]; then
      pointer_undecided=$((pointer_undecided + 1))
      continue
    fi
    lwindow="$(sed -n "$(( from > 8 ? from - 8 : 1 )),$(( to + 8 ))p" "$lfile")"
    lhit=""
    while IFS= read -r id; do
      [[ -z "$id" ]] && continue
      [[ "$lwindow" == *"$id"* ]] && { lhit="$id"; break; }
      lcamel="$(printf '%s' "$id" | awk -F_ '{s=$1; for(i=2;i<=NF;i++) s=s toupper(substr($i,1,1)) substr($i,2); print s}')"
      [[ "$lcamel" != "$id" && "$lwindow" == *"$lcamel"* ]] && { lhit="$id"; break; }
      lsnake="$(printf '%s' "$id" | awk '{s=""; for(i=1;i<=length($0);i++){c=substr($0,i,1); if (c ~ /[A-Z]/) s=s "_" tolower(c); else s=s c} print s}')"
      [[ "$lsnake" != "$id" && "$lwindow" == *"$lsnake"* ]] && { lhit="$id"; break; }
    done <<< "$idents"
    if [[ -z "$lhit" ]]; then
      fail "$rfile:$rline cites \`$cite_path:$from-$to\`, whose window holds no identifier that comment names — $(sed -n "${from}p" "$lfile" | cut -c1-50)"
    fi
  done
  # The **symbol form** — `Dag.lean:Descends` — is the durable one (a line moves, a name does not), and
  # it is the form the register's own convention prefers (check 9's `ANCHOR_SYM`, added for exactly this
  # reason). It is checked here too, and the reason is the one this whole family exists for: a citation
  # form the scanner does not know is a citation form nobody reads. Measured while writing it — the two
  # `Dag.lean:296-297` citations the line tier had just found stale were re-anchored to
  # `Dag.lean:Descends` by the lane that owns those files, and that symbol citation is now subject to
  # `sym_present` like any other.
  while [[ "$stext" =~ ([A-Za-z0-9_/]*\.lean):([A-Za-z_][A-Za-z0-9_]*) ]]; do
    cite_path="${BASH_REMATCH[1]}"; sym="${BASH_REMATCH[2]}"
    stext="${stext#*"${BASH_REMATCH[0]}"}"
    pointer_doc=$((pointer_doc + 1))
    lfile=""
    if [[ "$cite_path" == */* ]]; then
      for cand in "$ROOT/$cite_path" "$ROOT/spec/$cite_path"; do [[ -f "$cand" ]] && { lfile="$cand"; break; }; done
    else
      matches="$(printf '%s\n' "$lean_index" | awk -F'\t' -v b="$cite_path" '$1 == b {print $2}')"
      [[ "$(printf '%s\n' "$matches" | grep -c . || true)" == "1" ]] && lfile="$ROOT/$matches"
    fi
    if [[ -z "$lfile" ]]; then
      fail "$rfile:$rline cites \`$cite_path:$sym\`, which does not resolve — write the path in full (its basename is ambiguous or absent)"
      continue
    fi
    sym_present "$lfile" "$sym" \
      || fail "$rfile:$rline cites \`$cite_path:$sym\`, and that file names no \`$sym\` — the symbol form is *checked*, so a rename must land with its citation"
  done
done < <(cd "$ROOT" && grep -rnE '///.*[A-Za-z0-9_/]*\.lean:[0-9A-Za-z_]' --include='*.rs' . 2>/dev/null | sed 's|^\./||' || true)

# A check that scanned nothing is not evidence (the trap check 9's own comment records).
(( pointer_checked > 0 )) || fail "no C-number or section references found in spec/ — this check is vacuous (the sources list moved?)"
(( pointer_doc > 0 )) || fail "no \`.lean\` citations found in Rust doc comments — this family is vacuous (the doc-comment shape moved?)"
if (( failures == pointer_bad_before )); then
  ok "$pointer_checked pointer(s) resolve, $pointer_doc of them doc-comment citations; $pointer_subject decided by subject, $pointer_gap_refs naming a gap the allocator lists, $pointer_undecided outside what this check reads"
fi

# --- summary -----------------------------------------------------------------
printf '\n===== summary =====\n'
if (( failures == 0 )); then
  echo "OK: the register matches the tree."
  exit 0
fi
echo "FAIL: $failures problem(s) between spec/TEST-COVERAGE.md and the tree."
exit 1
