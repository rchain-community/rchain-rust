#!/usr/bin/env bash
#
# check-lean-conformance.sh — the formal gate.
#
# The repo's standard is "the oracle is the mathematical specification, not the Scala code"
# (`AGENTS.md:55-57`), but until this script existed the specification was checked by nothing: no CI
# job built the Lean, and the Coq had never been built outside a developer's shell. A theorem that
# stopped holding, or a module that quietly fell out of the library, was noticed by no one.
#
# What it checks, in order:
#   1. **The library builds** — `lake build` in `spec/`.
#   2. **Nothing is admitted** — no `sorry` anywhere under `spec/Rchain/`. The tree is `sorry`-free
#      today, so this is a ratchet: it can only stay that way.
#   3. **The library is complete** — every `spec/Rchain/**/*.lean` file is imported by
#      `spec/Rchain.lean`. Two modules (`Concurrent`, `Tree`) had been compiled but left outside the
#      library, which is exactly the kind of quiet omission this file exists to make loud.
#   4. **The Coq builds** — `make` in `spec/coq/` (the second formalization of laws 2, 3, 5, 6).
#   5. **The conformance corpora are current** — `lake exe rchain-corpus` re-emits them and
#      `git diff --exit-code` proves the committed corpora are exactly what the Lean definitions
#      produce. A stale corpus is a check that stopped checking.
#   6. **The Rust agrees 1:1** — the consumer tests read the corpora and fail on any disagreement.
#   7. **The static audits** — protocol agreement and channel balance over the vendored sources.
#
# Usage: tools/check-lean-conformance.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SPEC="$ROOT/spec"
failures=0
fail() { printf 'FAIL  %s\n' "$*"; failures=$((failures + 1)); }
ok() { printf 'ok    %s\n' "$*"; }

# --- 1. the library builds -----------------------------------------------------
# The executable targets are built too, and that is load-bearing: a module that is an `lean_exe` root
# (the corpus emitter) is *not* pulled in by the library target, so `lake build` alone would compile
# the corpus module's theorems never — a `decide`d case could rot unread. Verified by breaking the
# model on purpose and watching this fail.
if command -v lake >/dev/null 2>&1; then
  if (cd "$SPEC" && lake build >/tmp/lean-build.log 2>&1 && lake build rchain-corpus >>/tmp/lean-build.log 2>&1); then
    ok "lake build (spec/, library + executables)"
  else
    fail "lake build (spec/) — see /tmp/lean-build.log"
  fi
else
  fail "lake is not installed (elan: https://github.com/leanprover/elan)"
fi

# --- 2. nothing is admitted ----------------------------------------------------
# `sorry` (and `admit`, its tactic spelling) is how a proof obligation becomes a promise. The tree
# holds zero today, so this is a ratchet: it can only stay that way. Comments are stripped first —
# the words occur in prose ("the grammar allows one", "admit one"), and a gate that fires on prose is
# a gate people learn to work around.
if awk '
  { line = $0
    # drop `--` line comments (naive: the corpus has no string containing `--`)
    sub(/--.*/, "", line)
    # track /- -/ block comments
    if (inblock) { if (line ~ /-\//) { sub(/.*-\//, "", line); inblock = 0 } else next }
    while (line ~ /\/-/) {
      if (line ~ /-\//) { sub(/\/-.*-\//, "", line) } else { sub(/\/-.*/, "", line); inblock = 1; break }
    }
    if (line ~ /(^|[^A-Za-z_])(sorry|admit)([^A-Za-z_]|$)/) printf "%s:%d:%s\n", FILENAME, FNR, line
  }' "$SPEC/Rchain.lean" $(find "$SPEC/Rchain" -name '*.lean') >/tmp/lean-sorry.log 2>/dev/null; then
  if [[ -s /tmp/lean-sorry.log ]]; then
    fail "a 'sorry'/'admit' appeared under spec/Rchain — see /tmp/lean-sorry.log"
  else
    ok "no sorry/admit under spec/Rchain"
  fi
else
  fail "the sorry scan could not run"
fi

# --- 3. the library is complete ------------------------------------------------
expected="$(cd "$SPEC" && find Rchain -name '*.lean' | sed -e 's/\.lean$//' -e 's#/#.#g' | sort)"
# `lean_exe` roots (e.g. the corpus emitter) are executables, not library content: exclude them.
exe_roots="$(grep -oE 'root = "[^"]+"' "$SPEC/lakefile.toml" | sed -e 's/root = "//' -e 's/"//' \
  | sed -e 's/\./\//g' | sed -e 's#$#.lean#')"
if [[ -n "$exe_roots" ]]; then
  while read -r root; do
    [[ -n "$root" ]] || continue
    expected="$(printf '%s\n' "$expected" | grep -v "^$(printf '%s' "$root" | sed -e 's/\.lean$//' -e 's#/#.#g')$")"
  done <<< "$exe_roots"
fi
actual="$(grep -oE '^import Rchain[.A-Za-z]*' "$SPEC/Rchain.lean" | sed -e 's/^import //' | sort)"
if [[ "$expected" == "$actual" ]]; then
  ok "Rchain.lean imports every module ($(printf '%s\n' "$expected" | wc -l) files)"
else
  fail "Rchain.lean's imports differ from the tree:"
  diff <(printf '%s\n' "$expected") <(printf '%s\n' "$actual") | sed 's/^/      /' || true
fi

# --- 4. the Coq builds ---------------------------------------------------------
if [[ -f "$SPEC/coq/Makefile" ]]; then
  if ! command -v coqc >/dev/null 2>&1; then
    fail "spec/coq exists but coqc is not installed"
  elif (cd "$SPEC/coq" && make >/tmp/coq-build.log 2>&1); then
    ok "coq build (spec/coq/)"
  else
    fail "coq build (spec/coq/) — see /tmp/coq-build.log"
  fi
fi

# --- 5/6/7. the corpora and their consumers ------------------------------------
# Each of these steps is added with the artefacts it needs, in the same change: a step whose inputs do
# not exist would make this script vacuously green, which is the failure mode it is here to prevent.
# The consumer list is driven by the corpora on disk, so a new layer cannot ship without its consumer
# (a corpus nothing reads is not a check) — while a layer not yet started is simply absent.
if [[ -x "$ROOT/tools/emit-lean-corpus.sh" ]]; then
  if "$ROOT/tools/emit-lean-corpus.sh" >/tmp/lean-corpus.log 2>&1; then
    ok "corpora emitted from the Lean definitions"
  else
    fail "corpus emission failed — see /tmp/lean-corpus.log"
  fi
  # `git status --porcelain` rather than `git diff`: an untracked corpus (a layer just added, not yet
  # committed) has no diff to show, and a check that passes on an uncommitted file is the same check
  # that would pass on a stale one.
  dirty="$(cd "$ROOT" && git status --porcelain -- spec/conformance)"
  if [[ -z "$dirty" ]]; then
    ok "committed corpora match the Lean definitions"
  else
    fail "spec/conformance is not what the Lean defines — re-emit and commit:"
    printf '%s\n' "$dirty" | sed 's/^/      /'
  fi
fi

if [[ -d "$ROOT/spec/conformance" ]]; then
  for corpus in "$ROOT"/spec/conformance/*.tsv; do
    [[ -e "$corpus" ]] || continue
    layer="$(basename "$corpus" .tsv)"
    case "$layer" in
      flags)    test_name="lean_normalize_corpus"; crate="rchain-rholang" ;;
      parse)    test_name="lean_parse_corpus";     crate="rchain-rholang" ;;
      match)    test_name="lean_match_corpus";     crate="rchain-rholang" ;;
      silence)  test_name="lean_silence_corpus";   crate="rchain-rholang" ;;
      json)     test_name="lean_json_corpus";      crate="rchain-node" ;;
      *)        fail "corpus spec/conformance/$layer.tsv has no consumer mapping"; continue ;;
    esac
    if [[ ! -f "$ROOT/rholang/tests/$test_name.rs" && ! -f "$ROOT/node/tests/$test_name.rs" ]]; then
      fail "spec/conformance/$layer.tsv exists but no consumer test ($test_name) does — the corpus has no consumer"
      continue
    fi
    # Consumers live in `rholang/tests` or `node/tests`; find which, so the case below stays honest.
    if [[ -f "$ROOT/rholang/tests/$test_name.rs" ]]; then
      dir=rholang
    else
      dir=node
    fi
    if (cd "$ROOT" && cargo test -p "$crate" --test "$test_name" >/tmp/"$test_name".log 2>&1); then
      ok "$dir/tests/$test_name.rs agrees with spec/conformance/$layer.tsv"
    else
      fail "$dir/tests/$test_name.rs disagrees with the corpus — see /tmp/$test_name.log"
    fi
  done
fi

if [[ -x "$ROOT/tools/audit-protocols.sh" ]]; then
  if "$ROOT/tools/audit-protocols.sh" >/tmp/audit-protocols.log 2>&1; then
    ok "protocol agreement + channel balance"
  else
    fail "protocol/channel-balance audit failed — see /tmp/audit-protocols.log"
  fi
fi

echo ""
if (( failures > 0 )); then
  echo "===== $failures formal check(s) FAILED ====="
  exit 1
fi
echo "===== the formal gate is green ====="
