#!/usr/bin/env bash
#
# emit-coverage-ledger.sh — turn the coverage measurement into checked data.
#
# `spec/TEST-COVERAGE.md` is *governed*, not counted: `tools/audit-test-register.sh` recomputes its
# per-crate test counts from the tree and fails on an overstatement, its corpora counts are checked
# against the emitted `.tsv` files, and its law claims are checked against `spec/laws.tsv`. Coverage —
# the one number in the register that no gate could recompute — was the exception. Its own
# Definition-of-done item 11 said "ordered by uncovered lines", and the order it *gave* was written by
# hand, from an `lcov.info` dated **2026-09-22**: it named `node/src/api/grpc/tonic.rs` first (121
# missed lines) while the data ranked `rholang/src/reduce.rs` (648) and `rholang/src/system_processes.rs`
# (510) above it, and two files that had no register row at all sat in the top ten. A hand-written
# ranking of an emitted number is exactly the class this register exists to close, so the ranking is now
# emitted too.
#
# **What this binds, and where each half lives.**
#
#   * the per-file rows and the totals — from `lcov.info`, which is **gitignored** (a build artifact
#     the size of the workspace): the ledger is the durable half, the lcov is the input to emitting it;
#   * the floor — `--fail-under-lines` in `.github/workflows/coverage.yml`, checked here as a *derived*
#     value rather than a remembered one. The register states the rule ("two points below the measured
#     value, never to a number the plan hopes to reach") and the four raisings it has had
#     (73.68⇒71, 79.69⇒77, 81.30⇒79, 84.07⇒82) all satisfy `floor = floor(measured) − 2`. That is what
#     this script now enforces, in both directions: a floor *above* it is a tripwire the measurement does
#     not support, and a floor *below* it is a raise that was owed and not made. **One site only** — the
#     Makefile has no floor, and the register's Inventory must not restate these numbers (it points
#     here; check 11 refuses a hand-written total there).
#
# **The boundary, stated rather than implied.** Rows are the workspace members' own code: the 13 crates
# of the root `Cargo.toml`, under `src/`, `tests/` and `benches/`. Excluded, with the count printed so
# the filter is a measurement and not a promise: `legacy/` (the unported Scala tree), `spec/` (Lean and
# the registers), `target/`, and prost/tonic-generated files (their `include!`d lines are not in the
# `.rs` the compiler attributes them to). A file the compiler reports that this filter drops is not
# coverage debt; a file it *keeps* is.
#
# `--check` (what check 11 of `tools/audit-test-register.sh` runs) re-emits into a scratch copy and
# refuses a diff, the same discipline `tools/emit-lean-laws.sh` and `tools/emit-lean-counts.sh` use for
# their own emissions. It also refuses a *missing* input: an empty ledger beside a missing `lcov.info`
# would make every check below it vacuous.
#
# Usage: tools/emit-coverage-ledger.sh [--check] [--lcov <path>]
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/spec/COVERAGE-LEDGER.md"
LCOV="$ROOT/lcov.info"
check=0
while (( $# > 0 )); do
  case "$1" in
    --check) check=1 ;;
    --lcov) LCOV="${2:?--lcov needs a path}"; shift ;;
    *) echo "usage: $0 [--check] [--lcov <path>]" >&2; exit 2 ;;
  esac
  shift
done

if [[ ! -f "$LCOV" ]]; then
  echo "FAIL  $LCOV does not exist — run: cargo llvm-cov --workspace --all-features --lcov --output-path lcov.info" >&2
  exit 1
fi

CRATES='sdk|shared|crypto|graphz|models|block-storage|comm|rspace|rholang|casper|node|qucalc|rspace-bench'

# --- one row per file, from the record's own `LF:`/`LH:` ---------------------------------------------
# **The line counts are the lcov's summary, not a re-count of its `DA:` records**, and that is a decision
# with a measurement behind it. The two disagree, systematically and per record: in the `lcov.info` this
# script was first run against, 60,822 `DA:` records sat under 64,298 `LF:` (e.g. `block-storage/src/dag/
# finalizer.rs` LF=312 against 291 DA records), which is llvm-cov counting lines it maps in its own line
# model but emits no per-line record for. `cargo llvm-cov --fail-under-lines` — the floor this ledger is
# checked against — is computed from *that* model, so a ledger built on `DA:` would state a percentage the
# gate does not agree with: it read 86.52% where the summary read 85.35%. Measurement, not preference.
#
# `-F'[:,]'` is needed for the `DA:` diagnostic below (`DA:<line>,<hit-count>` splits the count into
# field 3 only when the comma is a separator); the `SF:`/`LF:`/`LH:` lines take field 2 either way.
rows="$(awk -F'[:,]' -v root="$ROOT/" -v crates="$CRATES" '
  /^SF:/ {
    f = substr($0, 4); sub("^" root, "", f)
    lf = 0; lh = 0; da = 0
    keep = (f ~ "^(" crates ")/(src|tests|benches)/") && f !~ /\.pb\.rs$/
    next
  }
  /^LF:/ { lf = $2 + 0; next }
  /^LH:/ { lh = $2 + 0; next }
  /^DA:/ { da++; next }
  /^end_of_record/ {
    if (keep && lf > 0) printf "%d\t%d\t%d\t%s\t%d\n", lf - lh, lh, lf, f, da
    keep = 0
  }
' "$LCOV" | sort -t$'\t' -k1,1rn -k4,4)"

# The gap between the two definitions, kept visible: a reader comparing this ledger against a
# hand-counted `grep -c '^DA:'` should find this number, not a mystery.
da_records="$(printf '%s\n' "$rows" | awk -F'\t' '{s += $5} END {print s + 0}')"

if [[ -z "$rows" ]]; then
  echo "FAIL  no file rows survived the filter in $LCOV — this emission would be vacuous" >&2
  exit 1
fi

# The filter's own count, so a dropped file is visible rather than silent.
excluded="$(awk -v root="$ROOT/" -v crates="$CRATES" '
  /^SF:/ { f = substr($0, 4); sub("^" root, "", f); next }
  /^end_of_record/ { total++; if (!(f ~ "^(" crates ")/(src|tests|benches)/") || f ~ /\.pb\.rs$/) skip++ }
  END { printf "%d\t%d\n", total + 0, skip + 0 }
' "$LCOV")"
n_total="${excluded%%$'\t'*}"
n_skipped="${excluded##*$'\t'}"

# --- the totals, and the floor they imply ----------------------------------------------------------
tot="$(printf '%s\n' "$rows" | awk -F'\t' '{m+=$1; h+=$2; f+=$3} END {printf "%d\t%d\t%d\n", m+0, h+0, f+0}')"
missed="${tot%%$'\t'*}"; rest="${tot#*$'\t'}"; hit="${rest%%$'\t'*}"; found="${rest##*$'\t'}"
# Percent in hundredths, so the comparison below is integer arithmetic and not a float's opinion.
pct100=$(( hit * 10000 / found ))
max_floor=$(( (pct100 - 200) / 100 ))

floor_line="$(grep -nE '^\s*run: .*--fail-under-lines' "$ROOT/.github/workflows/coverage.yml" || true)"
floor="$(printf '%s' "$floor_line" | grep -oE 'fail-under-lines [0-9]+' | grep -oE '[0-9]+' || true)"
if [[ -z "$floor" ]]; then
  echo "FAIL  no --fail-under-lines in .github/workflows/coverage.yml — the floor this ledger names has no site" >&2
  exit 1
fi

floor_verdict="ok"
if (( floor != max_floor )); then
  floor_verdict="MISMATCH"
fi

# Provenance. `lcov.info` is **gitignored** — it is a build artifact the size of the workspace, and the
# measurement is recorded in the ledger rather than by committing the file — so the date comes from the
# lcov's own mtime (the machine that measured it) and the tree it was measured against is named by the
# commit. Both live on this one line, which `--check` compares out: the date is provenance for a
# reader, not part of the claim, and a fresh clone's checkout mtime would otherwise make the ledger
# unverifiable everywhere but the machine that emitted it.
measured_on="$(date -r "$LCOV" -u +%Y-%m-%d 2>/dev/null || echo unknown)"
head_short="$(git -C "$ROOT" rev-parse --short=9 HEAD 2>/dev/null || echo '-')"

# The raise history is **derived from its own site**, not restated. The pairs live in the comment above
# the floor in `.github/workflows/coverage.yml` — which is where a raising is made — so the sentence below
# cannot disagree with the record. It did: this template said "the four times it has been raised" while
# the site held **six** (found 2026-09-25 by counting one against the other, which nothing was doing).
# That is a hand-written count sitting among machine-checked numbers, the shape check 10 exists for in the
# register's corpus counts.
raise_list="$(grep -oE '[0-9]+\.[0-9]+ *=> *[0-9]+' "$ROOT/.github/workflows/coverage.yml" | sed 's/ *=* *> */⇒/')"
raise_pairs="$(printf '%s\n' "$raise_list" | paste -sd, - | sed 's/,/, /g')"
if [[ -z "$raise_list" ]]; then
  echo "FAIL  no raise history in .github/workflows/coverage.yml — the ledger's history sentence would be vacuous" >&2
  exit 1
fi

# --- emit ------------------------------------------------------------------------------------------
emit() {
  cat <<EOF
# The coarse coverage ledger

<!-- Generated by tools/emit-coverage-ledger.sh from lcov.info. Do not edit by hand. -->

Generated, never written: \`tools/emit-coverage-ledger.sh\` reads \`lcov.info\` — the committed measurement,
\`cargo llvm-cov --workspace --all-features --lcov --output-path lcov.info\` — and emits the per-file
ranking that \`spec/TEST-COVERAGE.md\`'s Definition-of-done item 11 is worked from. Check 11 of
\`tools/audit-test-register.sh\` refuses a ledger that disagrees with the lcov it names, and refuses a CI
floor that is not the one this measurement implies.

**Measurement**: $measured_on (the lcov's own date); emitted from a tree at $head_short.

| lines found | hit | missed | line coverage | CI floor (implied) |
|---:|---:|---:|---:|---:|
| $found | $hit | $missed | $(( pct100 / 100 )).$(printf '%02d' $(( pct100 % 100 )))% | $max_floor |

The floor's rule, machine-checked above rather than remembered: **\`floor = floor(measured) − 2\`** — two
points below the measurement, never a number a plan hopes to reach. The raisings are read off the floor's
own site, not restated here: ($raise_pairs). **No count of them is written in this
sentence**, because the count is what rotted — this template once said "the four times it has been raised"
while the site already held more — and the list carries its own length. Every raising satisfies the rule,
which is why it is a rule and not a convention, and check 11 compares this list against the site, so a
ledger not re-emitted after a raising reads as stale rather than as current. A floor the measurement does
not support fails check 11 in either direction: too high is a tripwire nothing justifies, too low is a
raise that was owed.

Rows are the workspace members' own code under \`src/\`, \`tests/\` and \`benches/\`. Of the $n_total file
records in the lcov, **$n_skipped are excluded** here — \`legacy/\` (the unported Scala tree), \`spec/\`,
\`target/\`, and prost/tonic-generated files, whose \`include!\`d lines are not the \`.rs\` the compiler
attributes them to. The remaining **$(printf '%s\n' "$rows" | wc -l | tr -d ' ')** are below.

Each row's line counts are its record's own \`LF:\`/\`LH:\` summary — the line model
\`--fail-under-lines\` scores — **not** a re-count of its \`DA:\` records: the two disagree here by a
little under 5% ($da_records \`DA:\` records against $found \`LF\`), which is llvm-cov mapping lines it
emits no per-line record for. A ledger built on \`DA:\` would state a percentage CI's own gate does not
agree with, so the summary wins and the gap is printed rather than hidden.

## Files by missed lines

| missed | hit | found | % covered | file |
|---:|---:|---:|---:|---|
EOF
  printf '%s\n' "$rows" | awk -F'\t' '{ printf "| %d | %d | %d | %.1f | `%s` |\n", $1, $2, $3, ($3 ? $2 * 100 / $3 : 100), $4 }'
  cat <<EOF

## Files by missed fraction (≥150 executable lines)

The same data, ranked the other way: the files where a *proportion* is what is missing. Small files
dominate the missed-line table only when they are large in absolute terms; this one finds the modules
where a third or more of the behaviour is unpinned.

| % missed | missed | found | file |
|---:|---:|---:|---|
EOF
  printf '%s\n' "$rows" | awk -F'\t' '$3 >= 150 { printf "| %.1f | %d | %d | `%s` |\n", $1 * 100 / $3, $1, $3, $4 }' \
    | sort -t'|' -k2,2rn
}

if (( check )); then
  scratch="$(mktemp)"
  trap 'rm -f "$scratch"' EXIT
  emit > "$scratch"
  # The provenance line is compared out, and that is not a loophole — it is the one line whose value
  # *cannot* be stable across the commit that lands this file. It names the commit that last touched
  # `lcov.info`; committing the ledger commits `lcov.info` too, so the very next `--check` would read a
  # newer date than the ledger carries and fail on the commit that fixed it. The measured claim is the
  # numbers; the date is provenance for a reader, and it is the same date on both sides of this diff
  # whenever the lcov itself has not moved.
  filter_provenance() { grep -v '^\*\*Measurement\*\*:' "$1"; }
  if ! diff <(filter_provenance "$OUT") <(filter_provenance "$scratch") >/dev/null 2>&1; then
    echo "FAIL  $OUT is not what tools/emit-coverage-ledger.sh emits — re-emit it (the diff follows)" >&2
    diff <(filter_provenance "$OUT") <(filter_provenance "$scratch") | head -30 >&2 || true
    exit 1
  fi
  if (( floor != max_floor )); then
    echo "FAIL  CI's floor is $floor; the committed measurement implies $max_floor (floor = floor(measured) − 2)" >&2
    exit 1
  fi
  echo "ok    coverage ledger matches lcov.info ($found lines, $missed missed) and the CI floor is $floor"
else
  emit > "$OUT"
  echo "wrote $OUT ($found lines, $missed missed, $n_skipped of $n_total records excluded)"
  if (( floor != max_floor )); then
    echo "WARN  CI's floor is $floor but this measurement implies $max_floor — raise it after committing the measurement"
  fi
fi
