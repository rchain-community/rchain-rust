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
#   8. **The law register is current** — `lake exe rchain-laws` re-emits `spec/laws.tsv` and
#      `spec/LAWS.md`, and `git status` proves they are what `Rchain/Laws.lean` says. The register's own
#      checks — numbering 1..`lawCeiling`, reference integrity, axiom accounting, and that every
#      *proved* law carries a falsifiability witness — run in step 1, because they need the elaborated
#      environment. The documents that quote the register's totals are re-derived from `spec/laws.tsv`
#      here too (`tools/emit-lean-counts.sh`), so a number in prose cannot drift from it.
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
#
# `rchain-laws` is built here for the same reason and one more: its `run_cmd` checks (the register's
# numbering, reference integrity and axiom accounting) only run when the module is elaborated, so
# *building* it is the check. A `sorry` scan cannot see an `axiom`; this step is what can.
if command -v lake >/dev/null 2>&1; then
  if (cd "$SPEC" && lake build >/tmp/lean-build.log 2>&1 \
      && lake build rchain-corpus >>/tmp/lean-build.log 2>&1 \
      && lake build rchain-laws >>/tmp/lean-build.log 2>&1); then
    ok "lake build (spec/, library + executables + the law-register checks)"
  else
    fail "lake build (spec/) — see /tmp/lean-build.log"
  fi
else
  fail "lake is not installed (elan: https://github.com/leanprover/elan)"
fi

# --- 2. nothing is admitted, and nothing is assumed by another name ------------
# `sorry` (and `admit`, its tactic spelling) is how a proof obligation becomes a promise. The tree
# holds zero today, so this is a ratchet: it can only stay that way. Comments are stripped first —
# the words occur in prose ("the grammar allows one", "admit one"), and a gate that fires on prose is
# a gate people learn to work around.
#
if awk ' BEGIN { SQ = sprintf("%c", 39) }
  { out = ""; i = 1
    while (i <= length($0)) { c = substr($0, i, 1)
      if (depth > 0) { if (substr($0,i,2)=="/-") { depth++; i+=2 } else if (substr($0,i,2)=="-/") { depth--; i+=2 } else i++ }
      else if (instring) { if (c=="\\") i+=2; else { if (c=="\"") instring=0; i++ } }
      else {
        if (substr($0,i,2)=="--") break
        if (substr($0,i,2)=="/-") { depth++; i+=2 } else if (c=="\"") { if (substr($0,i-1,1)==SQ) { i++ } else { instring=1; i++ } } else { out=out c; i++ }
      } }
    if (out ~ /(^|[^A-Za-z_])(sorry|admit|opaque|unsafe|partial|extern|implemented_by)([^A-Za-z_]|$)/) printf "%s:%d:%s\n", FILENAME, FNR, out
  }' $(find "$SPEC" -name '*.lean' -not -path '*/.lake/*') >/tmp/lean-sorry.log 2>/dev/null; then
  if [[ -s /tmp/lean-sorry.log ]]; then
    fail "an assumption appeared under spec/Rchain (sorry/admit/opaque/unsafe/partial/extern/implemented_by) — see /tmp/lean-sorry.log"
  else
    ok "no sorry/admit/opaque/unsafe/partial/extern/implemented_by under spec/Rchain"
  fi
else
  fail "the assurance scan could not run"
fi

# --- 3. the library is complete ------------------------------------------------
# The library root is the import list itself, and is not imported by anything (`grep -v` here).
expected="$(cd "$SPEC" && find . -name '*.lean' -not -path './.lake/*' | sed -e 's#^\./##' -e 's/\.lean$//' -e 's#/#.#g' | grep -v '^Rchain$' | sort)"
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
# The version is pinned in `spec/coq/coq-version` and asserted here rather than installed by the gate:
# CI used to take whatever `apt-get install coq` gave the runner image, so the tested and the deployed
# Coq agreed by luck, and a Coq upgrade could have changed what the files mean without anything
# noticing. The file is the checkable half; how CI provision that version is CI's business.
COQ_AXIOM_CEILING=8    # the trust surface of spec/coq/, counted by step 4b below. Lower it with a discharge.
if [[ -f "$SPEC/coq/Makefile" ]]; then
  if ! command -v coqc >/dev/null 2>&1; then
    fail "spec/coq exists but coqc is not installed"
  else
    want="$(tr -d '[:space:]' <"$SPEC/coq/coq-version" 2>/dev/null || true)"
    have="$(coqc --version 2>/dev/null | head -1 | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1)"
    if [[ -n "$want" && "$want" != "$have" ]]; then
      fail "Coq is $have but spec/coq/coq-version pins $want — the definitions are checked against one Coq, not 'a Coq 8.x'"
    else
      ok "coq version ($have, pinned)"
    fi
    if (cd "$SPEC/coq" && make >/tmp/coq-build.log 2>&1); then
      ok "coq build (spec/coq/)"
    else
      fail "coq build (spec/coq/) — see /tmp/coq-build.log"
    fi
  fi
fi

# --- 4b. the Coq's own trust surface -------------------------------------------
# Nothing read the Coq's axioms before this step: the Lean `sorry` scan covers `spec/Rchain/**` and no
# check looked at a `.v` file at all, so the 15 `Axiom`s there — one of them *false* as written
# (`binds_at_most_once`, deleted 2026-09-23; AUDIT C26 said so and Coq still carried it) — were
# invisible. Counted rather than forbidden, and printed on *every* run: all of them are there today, so
# a zero-tolerance rule would be red on arrival and switched off on day one, and an unchanged ceiling
# that nobody can see is a ratchet in name only. `Admitted` counts too, because it is an axiom in the
# kernel; `Unset Guard|Positivity Checking` counts because that is how a non-terminating recursion or an
# ill-founded inductive is smuggled past the checker.
if [[ -f "$SPEC/coq/Makefile" ]]; then
  # The comment stripper uses `index` and never re-scans from the front of a shortened string. The
  # `match`-based version looked equivalent and was not: after stripping `(* … *)` from a whole-line
  # comment it left a shorter line, and `match("", /\(\*/)` in mawk *re-entered* the loop, so every
  # paragraph after the first such comment was treated as comment text and silently dropped from the
  # count (it reported 5 of 14). A count that under-reports is worse than none. This loop consumes at
  # least two characters per iteration, so it terminates.
  coq_axioms="$(awk '
    function strip(line,   i, j, out) {
      out = ""
      while (1) {
        i = index(line, "(*")
        if (i == 0) return out line
        out = out substr(line, 1, i - 1)
        line = substr(line, i + 2)
        j = index(line, "*)")
        if (j == 0) { inblock = 1; return out }
        line = substr(line, j + 2)
      }
    }
    { line = $0
      if (inblock) {
        j = index(line, "*)")
        if (j == 0) next
        line = strip(substr(line, j + 2)); inblock = 0
        if (line == "") next
      } else { line = strip(line) }
      if (line ~ /(^|[^A-Za-z_])(Axiom|Axioms|Parameter|Parameters|Conjecture|Admitted|admit|Unset[ \t]+(Guard|Positivity)[ \t]+Checking)([^A-Za-z_]|$)/) {
        printf "%s:%d: %s\n", FILENAME, FNR, line
      }
    }' "$SPEC"/coq/*.v)"
  n_coq="$(printf '%s\n' "$coq_axioms" | grep -c . || true)"
  if ((n_coq > COQ_AXIOM_CEILING)); then
    fail "the Coq trust surface grew: $n_coq axiom(s)/admit(s), ceiling $COQ_AXIOM_CEILING:"
    printf '%s\n' "$coq_axioms" | sed 's/^/      /'
  else
    ok "coq trust surface: $n_coq axiom(s)/admit(s), ceiling $COQ_AXIOM_CEILING"
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
      store)    test_name="lean_store_corpus";     crate="rchain-rholang" ;;
      c21)      test_name="lean_c21_corpus";       crate="rchain-rholang" ;;
      sort)     test_name="lean_sort_corpus";      crate="rchain-rholang" ;;
      protocol) test_name="lean_protocol_corpus";  crate="rchain-rholang" ;;
      envelope) test_name="lean_envelope_corpus";  crate="rchain-node" ;;
      lex)      test_name="lean_lex_corpus";       crate="rchain-node" ;;
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

# **The scope of the two steps above (the token scan and the completeness check) is every `.lean`
# under `spec/` except the build tree, and that is a decision rather than an accident of where the walk
# starts** (2026-09-24, AUDIT C75). Both used to look at `Rchain/**` only, so a module at the top of
# `spec/` — which nothing imports and `lake build` therefore never compiles — was invisible to the
# completeness check, to the token scan, and to the build: a `sorry` written in such a file was
# invisible to the ratchet that exists to find it. Falsified with two probes (`spec/ProbeSorry.lean`
# with `sorry`, `spec/ProbeOpaque.lean` with an `opaque def`): the scan reported 0 hits and
# `expected`/`actual` compared equal with both present. The policy the widened scope enforces: a Lean
# file under `spec/` is either the library root, a module the root imports, or a declared `lean_exe`
# root — there is no third kind, and a scratch file belongs outside `spec/`. `.lake/` is excluded
# because it holds 5,668 generated `.lean` files, which is why the old scope looked reasonable.
#
# This note sits BELOW the corpus-to-test mapping on purpose: `Laws.lean` cites a *line* of this
# script for law 30, and a line-anchored citation into a script moves whenever a step above it grows.

# --- 5b. the law register is current -------------------------------------------
# `spec/laws.tsv` and `spec/LAWS.md` are generated from `Rchain/Laws.lean`, and the documents' law counts
# and statuses are supposed to come from them. Committed and re-emitted here for the same reason the
# corpora are: a register that no longer matches the Lean is a catalog that stopped being true — which is
# exactly how both documents came to say "29 laws" while the tree held 43, and how they came to disagree
# with each other on Laws 5 and 24.
if [[ -x "$ROOT/tools/emit-lean-laws.sh" ]]; then
  if "$ROOT/tools/emit-lean-laws.sh" >/tmp/lean-laws.log 2>&1; then
    ok "law register emitted from the Lean"
  else
    fail "law register emission failed — see /tmp/lean-laws.log"
  fi
  dirty="$(cd "$ROOT" && git status --porcelain -- spec/laws.tsv spec/LAWS.md)"
  if [[ -z "$dirty" ]]; then
    ok "committed law register matches the Lean"
  else
    fail "spec/laws.tsv / spec/LAWS.md are not what the Lean defines — re-emit and commit:"
    printf '%s\n' "$dirty" | sed 's/^/      /'
  fi

  # --- 5c. and the documents that quote the register still agree with it ---------
  # `tools/emit-lean-counts.sh` rewrites the totals inside `<!-- counts:KEY -->…<!-- counts:end -->`
  # spans in the reader-facing documents from `spec/laws.tsv`, and fails on a register total written by
  # hand outside one. It is the same emit-and-diff discipline as above, one layer out: the *numbers* in
  # `TYPE-SYSTEM.md`'s and `laws.md`'s prose now come off the register, which is what stops the
  # class of defect this pass found there (three "30 element-comparator axioms" claims, a page saying
  # "48 laws and 57 entries" beside its own "49 laws, 58 entries", a gate comment reading "1..43").
  if [[ -x "$ROOT/tools/emit-lean-counts.sh" ]]; then
    if "$ROOT/tools/emit-lean-counts.sh" --check >/tmp/lean-counts.log 2>&1; then
      tail -1 /tmp/lean-counts.log | sed 's/^/ok    /'
    else
      fail "the documents' law counts are not the register's — see /tmp/lean-counts.log:"
      sed 's/^/      /' /tmp/lean-counts.log
    fi
  fi
fi

# --- the reply catalog is the schema's table ---------------------------------
#
# Law 39's catalog is data in Lean (`spec/Rchain/Protocol.lean`'s `replyCatalog`) and a probe per row
# in Rust; what it must not become is a *second* copy of `spec/API-SCHEMA.md` free to drift from it.
# Every urn the catalog names must have a row there — by name, or by its family, since the schema
# tables the qucalc/gov urns as one row per family.
if [[ -f "$ROOT/spec/conformance/protocol.tsv" ]]; then
  missing=0
  while IFS=$'\t' read -r layer urn _args _kind _slots; do
    [[ "$layer" == "protocol" ]] || continue
    family="${urn%:*}"
    if grep -qF -- "$urn" "$ROOT/spec/API-SCHEMA.md" \
      || grep -qF -- "$family" "$ROOT/spec/API-SCHEMA.md"; then
      :
    else
      fail "law 39's catalog names $urn and spec/API-SCHEMA.md has no row for it (nor for $family)"
      missing=$((missing + 1))
    fi
  done <"$ROOT/spec/conformance/protocol.tsv"
  if ((missing == 0)); then
    ok "every urn in law 39's catalog has a row in spec/API-SCHEMA.md"
  fi
fi

# (`tools/audit-protocols.sh` was to be a *static* audit of the vendored protocol content — a
# "linear consume with no paired produce" walk for law 41 and a call-shape table for law 40. Law 41's
# half was retired unshipped rather than committed with an exception list over vendored content; the
# reasoning is AUDIT §17 C22 item 1. Law 40's call-shape table is still owed, and it lands as a corpus
# layer like the others, not as a hook here: a conditional check for a file that does not exist is a
# green that means nothing.)

echo ""
if (( failures > 0 )); then
  echo "===== $failures formal check(s) FAILED ====="
  exit 1
fi
echo "===== the formal gate is green ====="
