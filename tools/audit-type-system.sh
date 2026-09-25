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
#   escape  — a refinement newtype surrendering its invariant. Four forms, all scoped to the files
#             that hold the refinements (`REFINEMENT_FILES`): an `impl … Deref … for`, a public tuple
#             field, a public `.get()`, and — since 2026-09-24 — a **construction** of a narrow
#             refinement that does not run its validator (`ShardId(format!(…))` in `ShardId::child`
#             before C78, `KeySegment::new` before C61). `spec/TYPE-SYSTEM.md` §1.7 is the rule; the
#             construction sites are reviewed one by one in `ESCAPE_CTOR_ALLOW`, whose entries are
#             keyed by text and each carry the guard their soundness rests on.
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
# The two entries added 2026-09-24 with R2a's C61 fix (`64ea1350f`) are **invariant assertions on
# already-proven values** — the same category as the fixed-size-array length checks above, and the
# category this list exists for ("a deliberate decision where the expression is the right one"):
#
#   `rspace/src/history/key_segment.rs:38` — `from_slice_of_valid` calls
#   `KeySegment::try_from(value).expect("a slice of a valid segment is at most 127 bytes")`. The
#   function is private and its name is its precondition: the argument is only ever a *slice of a
#   valid segment*, and a slice is never longer than its source, so `len <= 127` holds by
#   monotonicity and the `Err` arm is unreachable. The proof is the function's own doc comment; what
#   the `expect` buys is that a future caller who breaks the precondition gets a panic at the
#   construction rather than a segment past the wire invariant.
#
#   `rspace/src/history/history_repository.rs:48` — `key_segment(prefix, hash)` builds
#   `vec![prefix] ++ hash.as_bytes()` and converts it. 1 + 32 = 33 <= 127 arithmetically, and
#   `Blake2b256Hash::as_bytes` returns `&[u8; 32]` *by type*, so the length is fixed at the call site
#   rather than measured. The comment above the call states the same measurement.
#
# Neither assertion is removed to satisfy this class: an invariant check that cannot fire is worth
# keeping, and deleting it would trade a check for a quieter log.
#
# **What the file granularity costs, stated plainly.** This list is suffix-matched per *file*, so
# these two entries suppress the panic class over `key_segment.rs` and `history_repository.rs`
# *entirely*: a new `unwrap` anywhere in either file would be silent. Measured today, each file
# carries exactly one flagged expression — the one quoted above — and nothing else, so the entry
# hides nothing now; but a reader must not take an entry as evidence that the file was swept. The
# escape class's `ESCAPE_CTOR_ALLOW` is the per-site shape this list should eventually take: keyed by
# text with an evidence regex, so an entry states what it covers and fails when that thing moves.
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
  # R2a (C61): the two invariant assertions whose proofs are named above.
  '/rspace/src/history/key_segment.rs'
  '/rspace/src/history/history_repository.rs'
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
  "shared/src/refined.rs"                      # BlockHeight, SeqNum, Port, Hash32, ShardId, NonNegI64, WireLen
  "crypto/src/hash/blake2b512_random.rs"       # SerializedRandom
  "crypto/src/hash/blake2b256_hash.rs"         # Blake2b256Hash
  "models/src/block_hash.rs"                   # BlockHash
  "models/src/block/state_hash.rs"             # StateHash
  "models/src/validator.rs"                    # Validator
  "models/src/sorted.rs"                       # Sorted
  "models/src/types.rs"                        # Closed, WellScoped, FreeCount
  "rspace/src/history/radix_tree.rs"           # SerializedNode
  "rspace/src/history/key_segment.rs"          # KeySegment
  "rholang/src/util/rev_address.rs"            # Address, RevAddress
)

# The types the rule is about, **named** rather than pattern-matched: the same files hold error
# types with public fields (`RefineError(pub String)`) and service structs (`AddressTools`,
# `RadixTreeImpl`), which are not refinements and would be false positives. Adding a refinement means
# adding it here, which is the point — a new invariant should touch its audit.
#
# 2026-09-24: the roster was 3 files and 7 names, and it was **stale** — eight refinement homes were
# outside it, so all three forms below were green partly because they could not see the types they
# exist for. The measure that matters: every one of these names is a struct with a private field
# whose invariant is established by a constructor, and each home is the module that defines it
# (`spec/TYPE-SYSTEM.md` §1.7; the roster is what makes "the rule is complete over its scope" true,
# since a private field can only be written in the defining module). Two of the names — `NonNegI64`,
# `WireLen` — are macro-generated in `shared/src/refined.rs` (`non_neg_signed!`, `len_newtype!`) and
# therefore have no literal `pub struct` line of their own; the completeness guard in step G1 knows
# them by their macro invocation.
REFINEMENT_TYPES=(BlockHeight SeqNum Port Hash32 ShardId NonNegI64 WireLen SerializedRandom \
  SerializedNode Blake2b256Hash BlockHash StateHash Validator Sorted Closed WellScoped FreeCount \
  KeySegment Address RevAddress)

# A public `.get()` on a refinement: matched *inside* an `impl` block that names one of
# `REFINEMENT_TYPES`, not file-wide — these files also hold error types with public fields
# (`RefineError(pub String)`) whose accessors are not escapes and would be false positives.
ESCAPE_GET_AWK='
BEGIN { n = split(TYPES, t, ","); in_impl = 0; depth = 0 }
{
  # `match()` rather than `$0 ~ /…/`: only `match` is required to set `RSTART`/`RLENGTH` (mawk
  # leaves them untouched for `~`, which silently produced an empty header and a check that never
  # fired — measured 2026-09-24 with a probe).
  #
  # `[^{]*` and not `([^;{]*)`: the header ends at the body brace, and it may contain a `;` inside a
  # generic argument. `impl From<[u8; HASH32_LENGTH]> for Hash32` is exactly that shape — the old
  # class stopped at the `;`, so the header was the string `impl From<[u8;`, `Hash32` was never seen,
  # and a `pub fn get` inside that impl was invisible to the form that exists for it. (Measured
  # 2026-09-24; the file is in the roster, so this was a live blind spot, not a hypothetical one.)
  if (!in_impl && match($0, /^[[:space:]]*impl[^{]*/)) {
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

# --- the fourth form: a refinement rebuilt without its validator --------------------------------
#
# The three forms above are *accesses* (a `Deref`, a `.get()`, a public field). The fourth is a
# **construction**: `ShardId(format!(…))` inside `ShardId::child` before C78, `KeySegment::new`
# before C61 — a public method or constructor that mints a value of a refinement type without
# running the validator, so the invariant holds only if the author of that method got the argument
# right. The class could not see it at all until 2026-09-24: it printed green for C78 and for C61.
#
# **The rule is complete over its scope, and that is why the scope is a roster.** A refinement's
# invariant is carried by its **private field**, so a direct construction can only be written in the
# defining module — the 11 files in `REFINEMENT_FILES` (plus their child modules, which do not exist
# today: see the blind spots). Over those files, every construction of a narrow type is found, not
# sampled.
#
# **Narrow vs total is what keeps it quiet.** The inner type of a *total* refinement *is* its domain
# (`Port(u16)`, `Hash32([u8;32])`, `Validator([u8;65])`), so any inner-typed argument satisfies
# `spec/TYPE-SYSTEM.md`'s "total constructor on already-valid input" and a construction is not a
# claim. For a *narrow* refinement (`ShardId(String)`, `FreeCount(i32)`, `KeySegment(Vec<u8>)`) the
# inner type is strictly wider than the domain, so every construction is a judgement that needs a
# written reason. Measured with the split: 23 sites in 5 files; without it, the total-inner homes add
# `Self(Hash32::new(…))`-shaped false positives.
REFINEMENT_NARROW=(ShardId BlockHeight SeqNum NonNegI64 FreeCount KeySegment Sorted Closed WellScoped \
  SerializedRandom SerializedNode Address)
REFINEMENT_TOTAL=(Port Hash32 WireLen BlockHash StateHash Blake2b256Hash Validator RevAddress)

# The forms: `T(…)` (C78's), `Self(…)` (the same thing one token later — a legal rewrite that must
# not evade), `T { … }` on one line (KeySegment's), `T {` with the first field on the next line
# (every construction in `rholang/src/util/rev_address.rs`), and `.map(T)` (construction by
# fn-item reference). The exclusions are each a verified false positive: a `TryFrom` impl — whose
# *body* is the validator, where the invariant is established by definition, and which is also where
# the macro-generated `NonNegI64`/`WireLen` construct — a line containing `struct|enum|trait` (a
# definition), a line containing `->` (a return type: `fn tail(&self) -> KeySegment {`), and `//`
# lines (these files' doc comments quote constructors). `Self(…)` is scoped to the impl of a narrow
# type, so a `Self(…)` inside a service struct's impl is not this class's business.
#
# Evidence is checked **at the site**, not in the file: `CTOR_EVIDENCE_WINDOW` lines above the
# construction through the construction itself. An entry's evidence regex must match there — so
# deleting the guard the entry's reason names makes the entry fail, rather than passing on a guard
# that still exists elsewhere in the file (`is_closed` is called at more than one site).
#
# **What this rule cannot see, measured rather than imagined.** Each shape below is real; the ones
# marked *absent* were searched for in the roster and are listed so that a reader knows the boundary
# of the claim instead of assuming it:
#
#  1. **A caller of a growth path.** The rule reads constructions, not call sites. `KeySegment::concat`
#     and `append` *can* exceed 127 bytes (`a.len() + b.len()`), but measured 2026-09-25 they are
#     fallible and route through `KeySegment::try_from`, and every production call site propagates
#     with `?` (`radix_tree.rs:396`, `export.rs:119-125,187,219`) — so nothing mints an out-of-domain
#     segment silently. C61 was this shape with a *total* `new`; that constructor is gone.
#  2. **A construction inside a macro.** The macro body is scanned only if it is written in one of
#     these files (`non_neg_signed!` is, so its `Ok(Self(v))` is inside a `TryFrom` and suppressed as
#     the validator). A macro defined elsewhere that expands to a construction is invisible here.
#  3. **A derive-generated construction.** `#[derive(Default)]` on `ShardId` would mint `""` without
#     one line of construction text. Nothing checks derives; `KeySegment` derives `Default`, which is
#     why its `Default` is hand-written (the allowlist entry at `key_segment.rs:44`).
#  4. **A `TryFrom` that does not actually validate.** The validator's body is suppressed by
#     definition, so a `TryFrom` that returns `Ok` unconditionally is invisible — the class trusts the
#     name the way the type system does.
#  5. **`Name::from(x)` as a fn item, or a builder delegating to a validating `TryFrom`.** The `.map(T)`
#     form catches fn-item construction; a `From` impl that wraps an unchecked `Self(..)` would be
#     caught as that construction *if* the impl is in a roster file. A builder's chain is not followed.
#  6. **A brace-form refinement in a new file nobody added to the roster.** This is why the
#     completeness guard (a follow-up unit: G1/G2/G3 per the design) exists; until it lands, a new
#     refinement home is outside every form of this class.
#  7. **A non-`pub` newtype.** The roster is public refinements; a crate-private one is not named.
#  8. **Child modules of a defining file.** A private field is visible to descendants, so a
#     `mod inner` inside a roster file could construct without being seen. Measured: none exist today
#     (`mod tests` only, and test blocks are stripped).
#  9. **A construction sharing a line with `->`.** The `->` exclusion drops the whole line, so
#     `fn tail(&self) -> KeySegment { KeySegment { value: v } }` written on one line is invisible.
#     Measured: every `->`-carrying match in the roster is a *signature* today (rustfmt's rule puts
#     bodies on their own lines), so the exclusion costs nothing now — but it is a real hole, not a
#     hypothetical one, and narrowing it means parsing the item rather than the line.
# 10. **Evidence further than the window.** A guard more than `CTOR_EVIDENCE_WINDOW` lines above its
#     construction cannot satisfy its entry; spell the guard locally or raise the window.
CTOR_EVIDENCE_WINDOW=25

ESCAPE_CTOR_AWK='
BEGIN {
  # A blank element must not become a name: the empty regex matches any line carrying a `(`, and an
  # empty *last* name silently swallowed the site it had just found (measured 2026-09-25 while
  # falsifying this class — the empty string was the last element, so `hit` was reset to "" and the
  # construction was dropped). Roster entries are names; anything else is a defect in the roster.
  k = split(TYPES, raw, ","); n = 0
  for (i = 1; i <= k; i++) if (raw[i] != "") { n++; t[n] = raw[i] }
  ne = 0
  while ((getline e < ENTRIES) > 0) {
    if (e == "" || e ~ /^#/) continue
    ne++
    efile[ne] = e; sub(/\|.*/,       "", efile[ne])
    ector[ne] = e; sub(/^[^|]*\|/,   "", ector[ne]); sub(/\|.*/, "", ector[ne])
    eev[ne]   = e; sub(/^[^|]*\|[^|]*\|/, "", eev[ne]);  sub(/\|.*/, "", eev[ne])
  }
  close(ENTRIES)
  sites = 0; in_test = 0; tdepth = 0; cur = ""; mode = ""; idepth = 0
}
function is_narrow(name,   i) { for (i = 1; i <= n; i++) if (t[i] == name) return 1; return 0 }
function braces(s,   o, c, i, ch) { o = 0; c = 0; for (i = 1; i <= length(s); i++) { ch = substr(s, i, 1); if (ch == "{") o++; else if (ch == "}") c++ } return o - c }
function strip_generics(s) { sub(/<.*/, "", s); return s }
function last_token(s,   a, k) { gsub(/[[:space:]]+$/, "", s); k = split(s, a, /[.:[:space:]]+/); return a[k] }
function report(kind, file, line, text, extra) {
  gsub(/\t/, " ", text)
  printf "%s\t%s\t%s\t%s\t%s\n", kind, file, line, text, extra
}
FNR == 1 { in_test = 0; tdepth = 0; cur = ""; mode = ""; idepth = 0 }
{
  line = $0
  # Every line is kept, including the ones the rules below skip: the evidence window reads them.
  held[FNR] = line
  if (in_test == 0 && line ~ /^[[:space:]]*#\[cfg\(test\)\]/) { in_test = 1; tdepth = 0; next }
  if (in_test) { tdepth += braces(line); if (tdepth <= 0) in_test = 0; next }

  if (match(line, /^[[:space:]]*impl[^{]*/)) {
    hdr = substr(line, RSTART, RLENGTH)
    mode = (hdr ~ /TryFrom/) ? "validator" : "impl"
    if (hdr ~ / for /) { sub(/.* for /, "", hdr); cur = strip_generics(last_token(hdr)) }
    else { sub(/^[[:space:]]*impl([[:space:]]*<[^>]*>)?[[:space:]]*/, "", hdr); cur = strip_generics(last_token(hdr)) }
    idepth = braces(line)
    next
  }
  if (idepth > 0) idepth += braces(line)
  if (idepth <= 0) { cur = ""; mode = "" }

  if (line ~ /^[[:space:]]*\/\//) next
  if (line ~ /(struct|enum|trait)/) next
  if (line ~ /->/) next
  if (mode == "validator") next

  hit = ""
  for (i = 1; i <= n; i++) if (line ~ ("(^|[^[:alnum:]_])" t[i] "[[:space:]]*[({]")) hit = t[i]
  if (hit == "" && is_narrow(cur) && line ~ /(^|[^[:alnum:]_])Self[[:space:]]*[({]/) hit = "Self:" cur
  if (hit == "") for (i = 1; i <= n; i++) if (line ~ ("\\.map[[:space:]]*\\([[:space:]]*" t[i] "[[:space:]]*\\)")) hit = "map:" t[i]
  if (hit == "") next

  sites++
  # Which allowlist entry claims this site? File suffix and construction regex, both text.
  claims = 0; who = ""
  for (j = 1; j <= ne; j++) {
    if (FILENAME !~ (efile[j] "$")) continue
    if (line ~ ector[j]) { claims++; who = j; eclaims[j]++ }
  }
  if (claims == 0) { report("UNCLAIMED", FILENAME, FNR, line); next }
  if (claims > 1)  { report("MULTI", FILENAME, FNR, line, claims); next }
  # The evidence must be present at the site (see CTOR_EVIDENCE_WINDOW).
  ev = 0
  for (k = (FNR - WINDOW > 0 ? FNR - WINDOW : 1); k <= FNR; k++) if (held[k] ~ eev[who]) ev = 1
  if (!ev) { report("NOEVIDENCE", FILENAME, FNR, line, eev[who]); next }
  report("ALLOWED", FILENAME, FNR, line, who)
}
END {
  for (j = 1; j <= ne; j++) {
    if (eclaims[j] == 0) report("STALE", "-", "-", efile[j] "|" ector[j])
  }
  printf "SITES\t%d\n", sites
}
'

# Every site above is a judgement, and its *reason* is the artefact — so the reasons live here, one
# entry per site, keyed by **text, never by line number**: the files move under this pass (every line
# in `models/src/types.rs` moved while the design was being measured), and a line-keyed entry fails
# *open* — insert a line above it and it silently points at its neighbour. The four fields are
#   <file-suffix>|<construction-regex>|<evidence-regex>|<reason>
# and an entry matching no site, two sites, or whose evidence has gone is a hard failure.
ESCAPE_CTOR_ALLOW=(
  # --- shared/src/refined.rs -------------------------------------------------------------------
  'shared/src/refined.rs|NonNegI64\(1\)|total: .1. is non-negative|the literal 1 is in the domain; the doc states the totality argument'
  'shared/src/refined.rs|NonNegI64\(0\)|total: .0. is non-negative|the literal 0 is in the domain'
  'shared/src/refined.rs|BlockHeight\(0\)|total: .0. is non-negative|genesis height: the literal 0 is in the domain'
  'shared/src/refined.rs|BlockHeight\(self\.0\.saturating_add\(i64::from\(rhs\)\)\)|Add<NonNegI64> for BlockHeight|sum of a non-negative height and a NonNegI64; saturating, so a hypothetical overflow stays in the domain rather than wrapping negative'
  'shared/src/refined.rs|SeqNum\(0\)|total: .0. is non-negative|the literal 0 is in the domain'
  'shared/src/refined.rs|SeqNum\(self\.0\.saturating_add\(i64::from\(rhs\)\)\)|Add<NonNegI64> for SeqNum|sum of a non-negative sequence number and a NonNegI64; saturating, same argument as BlockHeight'
  'shared/src/refined.rs|ShardId\("/"\.to_string\(\)\)|non-empty ASCII|the root id: "/" is the literal its own doc calls non-empty ASCII'
  'shared/src/refined.rs|ShardId\(format!\("/\{name\}"\)\)|name\.is_ascii\(\)|C78: the appended name is validated by the same predicates TryFrom applies, so the join is ASCII and non-empty'
  'shared/src/refined.rs|ShardId\(format!\("\{\}/\{name\}", self\.0\)\)|name\.is_ascii\(\)|C78: same, on the non-root arm'
  'shared/src/refined.rs|Some\(ShardId\(self\.0\[\.\.cut\]\.to_string\(\)\)\)|rfind\(.\/.\)|a strict prefix of a valid id, cut at a `/` byte: ASCII by the invariant, non-empty because the `Some(0)` arm took the root case'
  # --- models/src/sorted.rs --------------------------------------------------------------------
  'models/src/sorted.rs|Sorted\(sort_par_term\(&par\)\)|Invariant: .self\.0 == sort_par_term|the canonicalizer runs on the way in — the construction *is* the sort'
  'models/src/sorted.rs|Sorted\(Par::default\(\)\)|The empty process is already canonical|the empty `Par` is a fixed point of the canonicalizer'
  # --- models/src/types.rs ---------------------------------------------------------------------
  'models/src/types.rs|Some\(Closed\(par\)\)|is_closed\(&par\)|the `Closed::new` guard: the construction is inside the branch that proved closure'
  'models/src/types.rs|Some\(WellScoped \{ ctx, par \}\)|well_scoped\(&ctx, &par\)|the `WellScoped::new` guard, same shape as `Closed`'
  'models/src/types.rs|FreeCount\(0\)|The empty-pattern count|zero binds, so the free-variable count is 0 — in the domain'
  'models/src/types.rs|FreeCount\(1\)|The single-binding count|one binding, so the count is 1 — in the domain'
  'models/src/types.rs|Some\(FreeCount\(n\)\)|if n >= 0|the `FreeCount::new` guard: the construction is inside the `n >= 0` branch'
  'models/src/types.rs|FreeCount\(i32::try_from\(n\)\.unwrap_or\(i32::MAX\)\)|cannot be negative|`from_len`: the argument is a collection length, so the sign is impossible rather than checked, and saturation keeps width out of the domain'
  'models/src/types.rs|FreeCount\(self\.0\.saturating_add\(rhs\.0\)\)|impl std::ops::Add for FreeCount|sum of two non-negative counts; saturating, like the `BlockHeight`/`SeqNum` additions'
  # --- rspace/src/history/key_segment.rs -------------------------------------------------------
  'rspace/src/history/key_segment.rs|KeySegment \{ value: Vec::new\(\) \}|Total: .0. bytes|the empty segment: 0 <= 127, and the doc states it'
  # --- rholang/src/util/rev_address.rs ---------------------------------------------------------
  'rholang/src/util/rev_address.rs|Some\(Address \{|stripped\.len\(\) == ETH_ADDRESS_LENGTH|`from_eth_address`: the length guard on the branch the construction is in'
  'rholang/src/util/rev_address.rs|^[[:space:]]*Address \{|pub fn from_unforgeable|`from_unforgeable`: every field is derived from `gprivate.id` and `self.prefix` by a hash of fixed width, so the record is total on its input'
  'rholang/src/util/rev_address.rs|Ok\(Address \{|decoded\.len\(\) != address_length|`parse`: length, checksum and prefix are checked on the lines above the construction'
)

scan_ctor_escapes() {
  local narrow out kind a b c d
  narrow="$(IFS=,; echo "${REFINEMENT_NARROW[*]}")"
  # The roster must be non-empty *and* every file in it must exist, before awk is invoked: awk reads
  # **stdin** when it is given no file argument, so an empty roster would block on the terminal (or
  # consume whatever is on stdin) instead of reporting its own emptiness — measured 2026-09-25, the
  # falsifier hung here. The `[ -f ] || continue` the older loops use is the other half of the same
  # hazard: a renamed file would drop silently out of the scan and the class would stay green over a
  # home it never read.
  local roster=() rel
  for rel in "${REFINEMENT_FILES[@]}"; do
    if [ -f "$ROOT/$rel" ]; then
      roster+=("$ROOT/$rel")
    else
      note escape "$ROOT/tools/audit-type-system.sh" "-" "REFINEMENT_FILES names $rel and no such file exists — the scan would be green over a home it never read"
    fi
  done
  if (( ${#REFINEMENT_NARROW[@]} == 0 )); then
    note escape "$ROOT/tools/audit-type-system.sh" "-" "REFINEMENT_NARROW names no type — the construction scan would match nothing and report silence as safety"
    return
  fi
  if (( ${#roster[@]} == 0 )); then
    note escape "$ROOT/tools/audit-type-system.sh" "-" "the roster names no file that exists — the construction scan has nothing to read (a scan that reviewed nothing is not evidence)"
    return
  fi
  local entries_file
  entries_file="$(mktemp)"
  printf '%s\n' "${ESCAPE_CTOR_ALLOW[@]}" > "$entries_file"
  # `held[]` is the file's lines, needed for the evidence window; `window` in awk needs the value.
  out="$(awk -v TYPES="$narrow" -v ENTRIES="$entries_file" -v WINDOW="$CTOR_EVIDENCE_WINDOW" "$ESCAPE_CTOR_AWK" \
           "${roster[@]}" 2>&1)"
  rm -f "$entries_file"
  while IFS=$'\t' read -r kind a b c d; do
    case "$kind" in
      UNCLAIMED)  note escape "$a" "$b" "construction of a narrow refinement with no allowlist entry: $c" ;;
      MULTI)      note escape "$a" "$b" "construction claimed by $d allowlist entries (an entry must name one site): $c" ;;
      NOEVIDENCE) note escape "$a" "$b" "allowlist entry's evidence ($d) is not at the site: $c" ;;
      STALE)      note escape "$ROOT/tools/audit-type-system.sh" "-" "allowlist entry claims no site (stale): $c" ;;
      ALLOWED)    printf '  (allowed) %s:%s — %s\n' "${a#"$ROOT/"}" "$b" "${ESCAPE_CTOR_ALLOW[$((d - 1))]##*|}" ;;
      SITES)      ctor_sites="$a" ;;
      *)          [ -n "$kind" ] && printf '  (ctor scan) %s\n' "$kind" ;;
    esac
  done <<<"$out"
  # The clause that keeps this class honest about itself: a scan that reviewed nothing is not
  # evidence, and neither is one that reviewed nothing *because* it failed to run.
  printf '  (reviewed) %s narrow-refinement construction(s) in %s file(s)\n' "${ctor_sites:-0}" "${#REFINEMENT_FILES[@]}"
  if (( ${ctor_sites:-0} == 0 )); then
    note escape "$ROOT/tools/audit-type-system.sh" "-" "the construction scan reviewed no sites at all — the awk found nothing to look at (bad roster, bad extraction, or a regex that cannot match)"
  fi
}

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
  scan_ctor_escapes
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
