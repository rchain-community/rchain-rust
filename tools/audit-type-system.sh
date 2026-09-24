#!/usr/bin/env bash
# Audit the Rust port for type-system violations — the authoritative gate.
#
# Supersedes `audit-partiality.sh` (a candidate finder). This script *strips* `#[cfg(test)]` test
# blocks (brace-depth aware) so it classifies production vs test correctly, applies an explicit
# whitelist of justified sites, and exits non-zero when a hard violation is found.
#
# Hard violations (exit 1):
#   panic   — `.unwrap()`, `.expect(`, `panic!`, `unreachable!`, `todo!`, `unimplemented!` in
#             production code. Whitelisted: `sdk/src/primitive.rs` (the Scala `getUnsafe` escape
#             hatch) and `regex/src/regex_pattern.rs` (the Scala `NotImplementedError("TODO")` stubs).
#             The rholang parser's `expect(Tok::…)` method is
#             excluded (a method, not `Result::expect`; the receiver may be `self` or the parser
#             parameter `p` inside `with_depth`).
#   unsafe  — `unsafe {` (must be zero: the crate graph is entirely safe Rust).
#   silent  — silent defaulting of a fallible numeric conversion: `try_into().unwrap()`,
#             `try_into().expect(`, `try_into().unwrap_or(`, `try_from(..).unwrap_or(`,
#             `..parse(..).unwrap_or(`. A fallible conversion must not be flattened to 0/Default.
#   escape  — a refinement newtype surrendering its invariant: `impl … Deref … for`, or a public
#             tuple field. Scoped to the files holding the refinements (see `REFINEMENT_FILES`);
#             `spec/TYPE-SYSTEM.md` §1.7 is the rule.
#
# Soft reports (exit 0, informational — refined by `cargo clippy` + manual review):
#   cast    — narrowing / signedness-changing numeric casts (`as i8/i32/i64/u8/u32/..`).
#   lax     — silent parse/hex escapes: `from_str_radix(..).unwrap_or(..)` and `base16::unsafe_decode`
#             (a hex decode that skips non-hex and never length-checks).
#   get     — index access and `.get(..).unwrap()`-style lookups.
#
# Usage: tools/audit-type-system.sh [panic|unsafe|silent|escape|cast|lax|get]   (default: all)

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CRATES=(sdk shared crypto graphz models block-storage comm rspace rholang casper regex node)

# ---------------------------------------------------------------------------
# Brace-depth-aware test-block stripper.
#
# A `#[cfg(test)]` attribute in this codebase is *always* followed by a
# `mod <name> { .. }` or a file-based `mod <name>;`. We skip from the attribute until the brace
# depth (tracked from the attribute line) returns to zero; a brace-less item (`mod property_tests;`)
# is exactly the one following line. Standalone `#[test]` / `#[tokio::test]` fns are skipped the
# same way.
# ---------------------------------------------------------------------------
STRIP_AWK='
function braces(s,   o,c,i,ch){ o=0; c=0; for(i=1;i<=length(s);i++){ ch=substr(s,i,1); if(ch=="{")o++; else if(ch=="}")c++ } return o-c }
BEGIN { skip=0; depth=0 }
{
  if (skip == 0) {
    if ($0 ~ /^[[:space:]]*#\[cfg\(test\)\]/) { skip=1; depth=0; next }
    if ($0 ~ /^[[:space:]]*#\[(tokio::)?test\]/) { skip=1; depth=braces($0); if (depth<=0) skip=0; next }
    print
    next
  }
  depth += braces($0)
  if (depth <= 0) skip=0
  next
}
'

# Files that are entirely test-only (not gated by an inline `mod` in another file).
TEST_ONLY_FILE_RE='(_tests?|test_)\.rs$|/property_tests\.rs$'

# Panic-class whitelist (suffix-matched against the file path). These are the deliberate escapes:
# get_unsafe (Scala `getUnsafe`) and the Scala-oracle `TODO`/`NotImplementedError` stubs.
#
# The following are `assert!`/`assert_eq!`/`debug_assert!` **internal invariants** on
# internally-produced data (fixed-size-array constructor length checks, radix-tree corrupt-node
# detection, empty-channels / channels==patterns, DAG-state contiguity, config buffer-size). The
# `from_slice` length asserts in block_hash/state_hash/validator (and
# `Blake2b256Hash::from_byte_array`) are now reachable only from internally-produced data: untrusted
# wire/API bytes use the checked `TryFrom<&[u8]>`/`try_from_hex` constructors (validate-on-ingress,
# see spec/AUDIT.md §11 R12).
#
# `block-storage/src/dag/message_state.rs` is the one entry added by the `debug_assert!` half of this
# class becoming visible (2026-09-24, U2 of Programme F): `latest_msgs` is a subset of `msg_map`,
# maintained by `insert_msg_mut` over two fields of one struct. That is the category above — a
# development-time self-consistency check on internally-produced data, with no value constructed from
# untrusted input and nothing invalid produced in release. Reviewed rather than restructured because
# the site is another writer's file and the check reports a *bug* rather than carrying a value; a
# reader who wants it enforced in release should give `insert_msg_mut`'s callers an error channel,
# which is a change to that state's API, not to this class. `casper/src/block_random_seed.rs`'s
# `debug_assert!(shard_id.is_ascii())` is the same shape and was already listed.
#
# Note the asymmetry this class now states plainly: an *assert* that a value is in its domain
# (`FreeCount::from_nonneg`'s `debug_assert!(n >= 0)`, deleted 2026-09-24) is **not** whitelisted —
# it silently accepted an invalid value in release. A `debug_assert!` on a *relation between two
# internally-produced structures* is whitelisted. Neither is invisible any more.
WHITELIST_PANIC=(
  '/sdk/src/primitive.rs'
  '/models/src/block_hash.rs'
  '/models/src/block/state_hash.rs'
  '/models/src/validator.rs'
  '/crypto/src/hash/blake2b256_hash.rs'
  '/crypto/src/hash/blake2b512_random.rs'
  '/block-storage/src/dag/metadata_store.rs'
  '/block-storage/src/dag/message_state.rs'
  '/comm/src/transport/buffer/limited_buffer.rs'
  '/rspace/src/history/radix_tree.rs'
  '/rspace/src/history/export.rs'
  '/rspace/src/history/instances/radix_history.rs'
  '/rspace/src/rspace.rs'
  '/rspace/src/replay_rspace.rs'
  '/casper/src/block_random_seed.rs'
)

hard_failures=0

is_whitelisted() {
  # $1 = file path; true if it matches any panic whitelist suffix.
  local f="$1" w
  for w in "${WHITELIST_PANIC[@]}"; do
    case "$f" in
      *"$w") return 0 ;;
    esac
  done
  return 1
}

note() {
  local kind="$1" file="$2" line="$3" text="$4"
  printf '  %s\n' "$file:$line: $text"
  case "$kind" in
    panic|unsafe|silent|escape) hard_failures=$((hard_failures + 1)) ;;
  esac
}

scan() {
  # $1 = kind; $2 = grep -E pattern; $3 = "panic" to apply the panic whitelist, else "".
  local kind="$1" pattern="$2" whitelist="$3"
  local c f line text
  for c in "${CRATES[@]}"; do
    local dir="$ROOT/$c/src"
    [ -d "$dir" ] || continue
    while IFS= read -r f; do
      if [ "$whitelist" = "panic" ] && is_whitelisted "$f"; then
        continue
      fi
      while IFS=: read -r line text; do
        [ -n "$line" ] || continue
        note "$kind" "$f" "$line" "$text"
      done < <(awk "$STRIP_AWK" "$f" \
                 | grep -nE "$pattern" \
                 | grep -vE 'self\.expect\(|\.expect\(Tok::')
    done < <(find "$dir" -name '*.rs' | grep -vE "$TEST_ONLY_FILE_RE")
  done
}

# The refinement newtypes must not surrender the invariant they exist to carry.
#
# `spec/TYPE-SYSTEM.md` §1.7 states the rule — no `Deref` (which drops `P` mid-domain), no public
# accessor returning the raw inner (`.get()`), and no public tuple field (which is the same escape by
# construction) — and until this class existed nothing checked it: it was a promise in prose, unlike
# the panic/unsafe/silent classes. The tree satisfies it today (no `impl … Deref` on any refinement,
# no `.get()`, every refinement field private), so this is a ratchet that can only stay green.
#
# **All three forms are scanned.** The first two were, from the start; the public `.get()` — the
# third form `spec/TYPE-SYSTEM.md:112-115` names — was not until 2026-09-24, so the count below was
# green partly because the form was unscanned. That is the same class of defect as a tripwire whose
# bound passes on the defect it names: an instrument that cannot see what it claims to check is not
# evidence (see the header's note on `debug_assert!`).
#
# Scoped to the files that hold the refinements rather than to the workspace: a `Deref` on some
# other wrapper type is a design choice, not a type escape, and a check that fired on those would be
# switched off within a week.
REFINEMENT_FILES=(
  "shared/src/refined.rs"                      # BlockHeight, SeqNum, Port, Hash32, ShardId
  "crypto/src/hash/blake2b512_random.rs"       # SerializedRandom
  "rspace/src/history/radix_tree.rs"           # SerializedNode
)

# The types the rule is about, **named** rather than pattern-matched: the same files hold error
# types with public fields (`RefineError(pub String)`), which are not refinements and would be false
# positives. Adding a refinement means adding it here, which is the point — a new invariant should
# touch its audit.
REFINEMENT_TYPES=(BlockHeight SeqNum Port Hash32 ShardId SerializedRandom SerializedNode)

# A public `.get()` on a refinement: matched *inside* an `impl` block that names one of
# `REFINEMENT_TYPES`, not file-wide — these files also hold error types with public fields
# (`RefineError(pub String)`) whose accessors are not escapes and would be false positives.
ESCAPE_GET_AWK='
BEGIN { n = split(TYPES, t, ","); in_impl = 0; depth = 0 }
{
  # `match()` rather than `$0 ~ /…/`: only `match` is required to set `RSTART`/`RLENGTH` (mawk
  # leaves them untouched for `~`, which silently produced an empty header and a check that never
  # fired — measured 2026-09-24 with a probe).
  if (!in_impl && match($0, /^[[:space:]]*impl([^;{]*)/)) {
    header = substr($0, RSTART, RLENGTH)
    for (i = 1; i <= n; i++) {
      if (header ~ ("(^|[^[:alnum:]_])" t[i] "([^[:alnum:]_]|$)")) { in_impl = 1 }
    }
  }
  if (in_impl && $0 ~ /pub (const )?fn get[[:space:]]*(<[^>]*>)?\(/) { print NR ": " $0 }
  depth += gsub(/\{/, "{") - gsub(/\}/, "}")
  if (depth <= 0) { depth = 0; in_impl = 0 }
}
'

scan_escapes() {
  local rel f name pattern line text
  # Any `Deref` impl in a refinement home, whatever it is on.
  pattern='impl[^;{]*Deref[^;{]*for'
  for name in "${REFINEMENT_TYPES[@]}"; do
    pattern="$pattern|pub struct ${name}\(pub "
  done
  local types
  types="$(IFS=,; echo "${REFINEMENT_TYPES[*]}")"
  for rel in "${REFINEMENT_FILES[@]}"; do
    f="$ROOT/$rel"
    [ -f "$f" ] || continue
    # `grep -n` on the file directly, so the reported line is the file's: filtering through more
    # pipes first (as the `scan` classes do with their comment stripper) renumbers the stream.
    while IFS=: read -r line text; do
      [ -n "$line" ] || continue
      note escape "$f" "$line" "$text"
    done < <(awk "$STRIP_AWK" "$f" | grep -nE "$pattern")
    # The public `.get()` form, scope-tracked (see `ESCAPE_GET_AWK`).
    while IFS=: read -r line text; do
      [ -n "$line" ] || continue
      note escape "$f" "$line" "$text"
    done < <(awk -v TYPES="$types" "$ESCAPE_GET_AWK" "$f")
  done
}

run_class() {
  local cls="$1"
  echo "===== class: $cls ====="
  case "$cls" in
    # `(debug_)?assert` rather than `\bassert`: a word boundary is a boundary between a word and a
    # non-word character, and `_` *is* a word character, so `\bassert` never matched `debug_assert!` —
    # the four production `debug_assert!`s were invisible to the class that exists for them until
    # 2026-09-24 (see the header). The preceding-character class keeps `xassert!` out.
    panic)   scan panic '\.unwrap\(\)|\.expect\(|panic!|unreachable!|todo!|unimplemented!|(^|[^[:alnum:]_])(debug_)?assert(_eq|_ne)?!\(|unwrap_or_else\(\s*\|\|\s*panic!' panic ;;
    unsafe)  scan unsafe 'unsafe[[:space:]]*\{' '' ;;
    silent)  scan silent 'try_into\(\)\.(unwrap|expect)\(|try_(into\(\)|from\(.*\))\.unwrap_or(\(0\)|_default\(\))|\.parse(::<[^>]+>)?\(\)\.unwrap_or(\(0\)|_default\(\))' '' ;;
    cast)    scan cast '\bas (i8|i16|i32|i64|u8|u16|u32|u64|usize|isize|f32|f64)\b' '' ;;
    lax)     scan lax 'from_str_radix\([^)]*\)\.(unwrap_or|unwrap|expect)\(|unsafe_decode\(' '' ;;
    get)     scan get '\.get\([^)]*\)\.(unwrap|expect)\(|\.(next|last|first|pop)\(\)\.(unwrap|expect)\(|\b[a-zA-Z_]+\[[0-9]+\]' '' ;;
    escape)  scan_escapes ;;
    *)
      echo "unknown class: $cls (expected panic|unsafe|silent|escape|cast|lax|get)" >&2
      exit 2
      ;;
  esac
}

if [ "$#" -eq 0 ]; then
  classes=(panic unsafe silent escape cast lax get)
else
  classes=("$@")
fi
for cls in "${classes[@]}"; do
  run_class "$cls"
done

echo
echo "===== summary ====="
if [ "$hard_failures" -gt 0 ]; then
  echo "FAIL: $hard_failures hard violation(s) (panic/unsafe/silent/escape) in production code."
  exit 1
fi
echo "OK: no hard production violations (panic/unsafe/silent/escape)."
