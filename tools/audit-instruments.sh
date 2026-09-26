#!/usr/bin/env bash
# Falsify the instruments: plant the defect each gate exists to catch, and require the gate to fail.
#
# Why this exists. This repository's audit register states the rule twice —
#
#   "An instrument that cannot see the defect it names is not evidence" (spec/AUDIT.md, the 2026-09-24
#   blind-spot paragraph), and "an instrument that passes on the defect it names is not evidence, and
#   only the falsifier distinguishes" (spec/AUDIT.md, the U14 sweep).
#
# — and both times it was learned the hard way: the `panic` class could not see `debug_assert!` because
# a word boundary never occurs before `assert` after `_`, and the `escape` class could not see a public
# `.get()` because nobody had looked for that form. Each was green while the defect it named was live.
# A probe is the only thing that distinguishes "clean" from "the scan stopped looking", and a scan that
# has gone blind looks exactly like a clean tree.
#
# What it does. For each gate: plant the defect, run the gate, require a non-zero exit, restore. The
# tree is restored on every path, including a signal, and the script refuses to run on a dirty tree so
# that "restore" is always a known-good state rather than an assumption.
#
# Reading the result. A FAIL row means an instrument did not notice the defect it exists for — that is
# a finding about the instrument, and the more dangerous kind, because the gate has been reporting
# green. An OK row is the instrument working. This script's own exit code is 1 if any row is FAIL.
#
# Usage:  tools/audit-instruments.sh

set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT" || exit 1

SCRATCH="$(mktemp -d)"
FAILED=0
PROBES=0

die() { printf 'audit-instruments: %s\n' "$1" >&2; exit 2; }

# The probes edit tracked files, so a *modified* tracked file makes the restore ambiguous — and a
# probe that cannot restore is worse than no probe, because it leaves a planted defect behind.
# Untracked files are fine: a probe cannot corrupt what it does not back up, and refusing on them
# would stop this script from ever running before it is committed.
DIRTY="$(git status --porcelain | grep -v '^??' || true)"
if [[ -n "$DIRTY" ]]; then
  printf '%s\n' "$DIRTY" >&2
  die "tracked files are modified; commit or stash first (the probes need a known-good restore target)"
fi

declare -a BACKED_UP=()
restore_all() {
  local f
  for f in "${BACKED_UP[@]:-}"; do
    [[ -n "$f" && -f "$SCRATCH/$(printf '%s' "$f" | tr / _)" ]] || continue
    cp "$SCRATCH/$(printf '%s' "$f" | tr / _)" "$f"
  done
}
trap 'restore_all; rm -rf "$SCRATCH"' EXIT INT TERM

backup() {
  local f="$1"
  cp "$f" "$SCRATCH/$(printf '%s' "$f" | tr / _)"
  BACKED_UP+=("$f")
}

# probe <label> <pass|fail> <expected-nonzero>  — records the row and the tally.
report() {
  local label="$1" rc="$2"
  PROBES=$((PROBES + 1))
  if [[ "$rc" != "0" ]]; then
    printf '  OK    %-34s caught it (exit %s)\n' "$label" "$rc"
  else
    printf '  FAIL  %-34s DID NOT FAIL — the gate cannot see its own defect\n' "$label"
    FAILED=$((FAILED + 1))
  fi
}

# run <label> <gate-command...> — runs the gate quietly and reports whether it refused.
run() {
  local label="$1"; shift
  local out rc
  out="$("$@" 2>&1)" && rc=0 || rc=$?
  report "$label" "$rc"
  if [[ "$rc" == "0" ]]; then printf '%s\n' "$out" | tail -3 | sed 's/^/        /'; fi
}

printf 'Falsifying the instruments (plant → require failure → restore)\n\n'

# ---------------------------------------------------------------------------------------------
# tools/audit-type-system.sh — the four hard classes and the both-ways ratchet.
# A file in the crate roster that is not on the panic allowlist.
PROBE_FILE="graphz/src/lib.rs"

printf 'audit-type-system.sh\n'
backup "$PROBE_FILE"
printf '\npub fn probe_panic() { let _ = Some(1u8).unwrap(); }\n' >> "$PROBE_FILE"
run "panic: a production .unwrap()" bash tools/audit-type-system.sh panic
restore_all

backup "$PROBE_FILE"
printf '\npub fn probe_unsafe() { unsafe { let _p: *const u8 = std::ptr::null(); } }\n' >> "$PROBE_FILE"
run "unsafe: an unsafe block" bash tools/audit-type-system.sh unsafe
restore_all

backup "$PROBE_FILE"
printf '\npub fn probe_silent() { let _x: u8 = 300u16.try_into().unwrap(); }\n' >> "$PROBE_FILE"
run "silent: a flattened fallible conversion" bash tools/audit-type-system.sh silent
restore_all

# The escape class matches inside an `impl` naming a refinement, in the files that hold them.
backup shared/src/refined.rs
printf '\nimpl std::ops::Deref for BlockHeight {\n    type Target = i64;\n    fn deref(&self) -> &i64 { &self.0 }\n}\n' >> shared/src/refined.rs
run "escape: a Deref on a refinement" bash tools/audit-type-system.sh escape
restore_all

backup "$PROBE_FILE"
printf '\npub fn probe_cast() { let _x = 1u64 as u32; }\n' >> "$PROBE_FILE"
run "cast ratchet: a site arrives" bash tools/audit-type-system.sh cast
restore_all

# The ratchet is *bidirectional* (AUDIT C98): a site leaving fails too, which is what stops the number
# moving without a commit saying so. Raise the baseline and the unchanged tree must fail.
backup tools/type-system-baseline.tsv
perl -0pi -e 's/^cast\t336$/cast\t337/m' tools/type-system-baseline.tsv
run "cast ratchet: a site leaves" bash tools/audit-type-system.sh cast
restore_all

# ---------------------------------------------------------------------------------------------
printf '\naudit-test-register.sh\n'

# Check 14: every `C<n>` pointer anywhere in spec/ must resolve to an allocated finding.
backup spec/RUST-VS-SCALA.md
printf '\nSee AUDIT C999 for the detail.\n' >> spec/RUST-VS-SCALA.md
run "check 14: a C-pointer to nothing" bash tools/audit-test-register.sh
restore_all

# Check 2: a named test in the `## Machine-checked claims` table must exist. Probe the table the check
# actually reads — the law-property matrix is a *different* table and is covered by no check (see the
# ledger's `tool` roster).
backup spec/TEST-COVERAGE.md
perl -0pi -e 's/`insert_rejects_equivocation_same_seq_num`/`insert_rejects_equivocation_same_seq_num_XYZ`/' spec/TEST-COVERAGE.md
run "check 2: a named test that is not there" bash tools/audit-test-register.sh
restore_all

# Check 9: a `path:line` citation in the *emitted* register must resolve and be inside the file.
backup spec/laws.tsv
perl -0pi -e 's{([a-z-]+/src/[a-z_/]+\.rs):\d+}{$1:99999}' spec/laws.tsv
run "check 9: an anchor past EOF" bash tools/audit-test-register.sh
restore_all

# ---------------------------------------------------------------------------------------------
printf '\ncheck-rust-witnesses.sh\n'
# The tool's whole point is the zero-match refusal: `cargo test <filter>` exits 0 when nothing
# matches, so a renamed witness would silently pass. A real `fn` that is not a test must fail.
printf 'comm/src/transport/chunker.rs:chunk_it\n' > "$SCRATCH/witness.txt"
out="$(timeout 900 tools/check-rust-witnesses.sh "$SCRATCH/witness.txt" 2>&1)" && rc=0 || rc=$?
report "zero-match refusal" "$rc"
if [[ "$rc" == "0" ]]; then printf '%s\n' "$out" | tail -3 | sed 's/^/        /'; fi

# ---------------------------------------------------------------------------------------------
printf '\naudit-vendored-sources.sh\n'
# This gate is in **no workflow** (only `make check-register` runs it), so it is reported here even
# though a probe cannot tell whether anyone would run it.
if grep -rq 'audit-vendored-sources' .github/workflows/ 2>/dev/null; then
  printf '  OK    %-34s reachable from CI\n' "vendored sources in CI"
else
  printf '  FAIL  %-34s %s\n' "vendored sources in CI" "in no workflow — make check-register is the only entry point"
  FAILED=$((FAILED + 1))
fi
PROBES=$((PROBES + 1))

printf '\n%d probe(s), %d instrument(s) did not catch their own defect.\n' "$PROBES" "$FAILED"
[[ "$FAILED" == "0" ]] || exit 1
