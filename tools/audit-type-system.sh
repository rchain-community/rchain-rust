#!/usr/bin/env bash
# Audit the Rust port for type-system violations — the authoritative gate.
#
# Supersedes `audit-partiality.sh` (a candidate finder). This script *strips* `#[cfg(test)]` test
# blocks (brace-depth aware) so it classifies production vs test correctly, applies an explicit
# whitelist of justified sites, and exits non-zero when a hard violation is found.
#
# Hard violations (exit 1):
#   panic   — `.unwrap()`, `.expect(`, `panic!`, `unreachable!`, `todo!`, `unimplemented!` in
#             production code. Every site is reviewed **one by one** in `WHITELIST_PANIC`, each entry
#             keyed on the site's own text with the guard or invariant it rests on; an entry that
#             matches no site, or whose evidence has gone, is a hard failure. The rholang parser's
#             `expect(Tok::…)` method is excluded (a method, not `Result::expect`; the receiver may be
#             `self` or the parser parameter `p` inside `with_depth`).
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
# **This list was file-suffix-matched until 2026-09-25, and that was a hole in the instrument.** A
# file-level entry suppresses the class over the *whole* file: a new `unwrap` in
# `rspace/src/history/key_segment.rs` or `history_repository.rs` was invisible to the gate, and
# `history_repository.rs` is where C61's repair put a fresh `expect` (R2a) — the worst place for a
# blind spot, since a file under active repair is the one whose next line most needs watching. Two
# other defects came out of the same survey: `block-storage/src/dag/metadata_store.rs` was listed with
# **no production panic site at all** (its `unwrap`s are inside `mod tests`), an entry that suppresses
# nothing while reading as "this file was reviewed"; and the header above named
# `regex/src/regex_pattern.rs`, which is not in the array and not in the tree (`regex/` does not
# exist). The **comment** was corrected rather than the array, because an entry for a directory that
# is not there is exactly what the staleness check below now fails on.
#
# The shape is the escape class's: `<file-suffix>|<site-regex>|<evidence-regex>|<reason>`, keyed on
# **text, never line numbers** (a line-keyed entry fails *open* — insert a line above and it points at
# its neighbour). The site key is the site's *whole* expression with a multi-line assertion joined
# into one string, which is what makes it unique: `assert!(` occurs three times in `radix_tree.rs` and
# `assert_eq!(channels.len(), patterns.len(), …)` three times across two files. For an `assert!` the
# guard *is* the assertion, so its evidence names the condition checked; for an `expect` whose
# unreachability rests on a proof written nearby (a comment, a type fact, a caller-side measurement),
# the evidence names that proof — and deleting it stales the entry and fails the gate, which is the
# property that makes the entry a judgement rather than a silencer.
#
# The panic class **scans one awk pass itself** now (`PANIC_SITES_AWK`) instead of `grep -n` over a
# stripped stream: the joined site needs both, and the reported line is the file's own rather than the
# stripped stream's (the two differ once a `#[cfg(test)]` block precedes a site).
WHITELIST_PANIC=(
  'sdk/src/primitive.rs;;\.unwrap_or_else\(\|\| panic!\("No key \{key:\?\} in a map\."\)\);;fn get_unsafe;;port of the Scala `getUnsafe` escape hatch: the panic *is* the API (the Scala throws), which is why the whole file is a declared escape'
  'sdk/src/primitive.rs;;self\.unwrap\(\);;fn get_unsafe;;`TryOps::get_unsafe`, the same declared escape hatch on the Result side'
  'crypto/src/hash/blake2b256_hash.rs;;assert_eq!\( bytes\.len\(\), LENGTH, "Expected \{\} but got \{\}", LENGTH, bytes\.len\(\) \);;;requiring it to be exactly 32 bytes;;`from_byte_array`: a length assert on a fixed-size-array construction, and the doc says so'
  'crypto/src/hash/blake2b512_random.rs;;assert!\( children\.len\(\) >= 2, "Blake2b512Random should have at least 2 inputs to merge, received \{\}\.", children\.len\(\) \);;;Merge two or more states;;`merge`: the state merge is defined for two or more inputs; the doc states it and the assert is the boundary'
  'models/src/validator.rs;;assert_eq!\(bytes\.len\(\), LENGTH, "expected \{LENGTH\} bytes"\);;;pub fn from_slice\(bytes;;`Validator::from_slice` asserts the 32-byte length; untrusted bytes use the checked `TryFrom<&[u8]>` (validate-on-ingress, AUDIT R12)'
  'models/src/block_hash.rs;;assert_eq!\(bytes\.len\(\), LENGTH, "expected \{LENGTH\} bytes"\);;;panics if not exactly;;`BlockHash::from_slice`: an internal length assert, with the doc naming the panic'
  'models/src/block/state_hash.rs;;assert_eq!\(bytes\.len\(\), LENGTH, "expected \{LENGTH\} bytes"\);;;pub fn from_slice\(bytes;;`StateHash::from_slice`: the same internal length assert as `BlockHash`/`Validator`'
  'block-storage/src/dag/message_state.rs;;debug_assert!\( self\.latest_msgs \.values\(\) \.all\(\|m\| self\.msg_map\.contains_key\(&m\.id\)\), "latest_msgs must be a subset of msg_map" \);;;latest_msgs must be a subset of msg_map;;a development-time self-consistency check between two fields of one struct, maintained together by `insert_msg_mut`; the invariant is the message'
  'comm/src/transport/buffer/limited_buffer.rs;;assert!\( buffer_size > 0, "bufferSize must be a strictly positive number" \);;;bufferSize must be a strictly positive number;;a buffer of size 0 cannot exist; the Scala `require` this ports is in the message'
  'rspace/src/replay_rspace.rs;;assert!\(!channels\.is_empty\(\), "channels can'\''t be empty"\);;;channels can'\''t be empty;;a consume with no channels is unconstructible at every call site; the assert is the boundary, ported from the Scala `require`'
  'rspace/src/replay_rspace.rs;;assert_eq!\( channels\.len\(\), patterns\.len\(\), "channels\.length must equal patterns\.length" \);;;channels.length must equal patterns.length;;channels and patterns are zipped, so their lengths must agree; the assert states the pairing invariant'
  'rspace/src/rspace.rs;;assert!\(!channels\.is_empty\(\), "channels can'\''t be empty"\);;;channels can'\''t be empty;;the same consume boundary as `replay_rspace.rs`'\''s `consume_result`'
  'rspace/src/rspace.rs;;assert_eq!\( channels\.len\(\), patterns\.len\(\), "channels\.length must equal patterns\.length" \);;;channels.length must equal patterns.length;;the channels/patterns pairing at `consume` **and** at `install` (the same text occurs twice in this file): they are zipped, so their lengths must agree'
  'rspace/src/history/key_segment.rs;;KeySegment::try_from\(value\)\.expect\("a slice of a valid segment is at most 127 bytes"\);;provably a .slice of a valid segment;;`from_slice_of_valid`: private, and its name is its precondition — the argument is only ever a slice of a valid segment, so `len <= 127` holds by monotonicity and the `Err` arm is unreachable (C61'\''s fix; the proof is the doc above)'
  'rspace/src/history/key_segment.rs;;\.expect\("a segment'\''s head is read only where the segment is known non-empty"\);;;;head reads through head_option and refuses by name; every caller argues its segment is non-empty, and the doc above names each enforcer (H2e, 7ec9e16d5)'
  'rspace/src/history/export.rs;;assert!\( ptr_prefix_rest\.is_empty\(\), "Export error: node with prefix \{expected_prefix\} not found\." \);;;ptr_prefix_rest\.is_empty\(\);;`export`: a node whose prefix is not a prefix of the searched one is a corrupt tree, and the diagnostic is rendered fallibly just above so the panic cannot fire while formatting'
  'rspace/src/history/history_repository.rs;;KeySegment::try_from\(bytes\)\.expect\("1 \+ 32 = 33 bytes is at most 127 bytes"\);;well inside the 127-byte invariant;;`key_segment`: one prefix byte plus a 32-byte hash is 33 bytes by type (`as_bytes()` is `&[u8; 32]`), so the constructor cannot refuse — the comment above states the measurement'
  'rspace/src/history/radix_tree.rs;;\.expect\("a 7-bit size field is at most 127"\);;;a 7-bit size field is at most 127;;C61'\''s decode site: the length prefix is 7 bits, so the slice re-wrapped here is at most 127 bytes; the comment above names the measurement'
  'rspace/src/history/radix_tree.rs;;assert!\( no_assert, "Missing node in database\. ptr=\{\}", node_ptr\.to_hex\(\) \);;;Missing node in database;;a node referenced by a node ptr must be in the store: a corrupt tree, not a reachable statespace condition'
  'rspace/src/history/radix_tree.rs;;assert!\( existing == node, "Collision in cache: record with key = \{\} has already existed\.", hash\.to_hex\(\) \);;;Collision in cache;;a hash collision between a cached record and a new one: a corrupt-cache check, not a data condition'
  'rspace/src/history/radix_tree.rs;;assert!\(!prefix\.is_empty\(\), "LeafPrefix should be non empty\."\);;;LeafPrefix should be non empty;;`create_node_from_item` is defined on a non-empty prefix (the Scala `require`); the leaf is stored under its head byte, so an empty prefix has no place to go'
  'rspace/src/history/radix_tree.rs;;assert!\(!prefix\.is_empty\(\), "NodePtrPrefix should be non empty\."\);;;NodePtrPrefix should be non empty;;the NodePtr arm of the same constructor, same invariant'
  'rspace/src/history/radix_tree.rs;;\.expect\("a single byte is at most 127 bytes"\);;;a single byte is at most 127 bytes;;a re-wrap of a one-byte segment: 1 <= 127 arithmetically, stated in the message'
  'rspace/src/history/radix_tree.rs;;assert_eq!\( leaf_prefix\.len\(\), ins_prefix\.len\(\), "The length of all prefixes in the subtree must be the same\." \);;;The length of all prefixes in the subtree must be the same;;the subtree prefix invariant of the radix tree, stated in the message'
  'rspace/src/history/radix_tree.rs;;assert!\( ptr_prefix\.len\(\) < ins_prefix\.len\(\), "Radix key should be longer than NodePtr key\." \);;;Radix key should be longer than NodePtr key;;an insert key must be strictly longer than a node ptr key — the tree'\''s structural invariant, stated in the message'
  'rspace/src/history/instances/radix_history.rs;;assert!\( has_no_duplicates\(actions\), "Cannot process duplicate actions on one key\." \);;;Cannot process duplicate actions on one key;;one action per key per commit; duplicates are rejected upstream, and the assert is the boundary'
  'casper/src/block_random_seed.rs;;assert!\( shard_id\.is_ascii\(\), "Shard name should contain only ASCII characters" \);;;Shard name should contain only ASCII characters;;`ShardId` is ASCII by invariant (the refinement), and H2d changed this from a `debug_assert!` to a plain `assert!` so the port'\''s check is live in every profile, as the Scala'\''s `Predef.assert` is — the rename is what staled the previous key, which is this staleness check earning its keep on a real commit rather than on a planted probe'
  'casper/src/block_random_seed.rs;;\.expect\("block random seed components always fit a 1-byte length prefix"\);;;the length prefix cannot overflow;;`random_generator`: `var_size`'\''s inputs are far below 256 bytes — the comment above states the measurement the `expect` documents'
)

hard_failures=0

note() {
  local kind="$1" file="$2" line="$3" text="$4"
  printf '  %s\n' "$file:$line: $text"
  case "$kind" in
    panic|unsafe|silent|escape) hard_failures=$((hard_failures + 1)) ;;
  esac
}

# --- the panic class: one awk pass, then the per-site allowlist -------------------------------
#
# The awk emits each site's *whole* expression — a multi-line assertion joined into one string — with
# the file's own line number, which is what lets an entry key on it (`assert!(` alone occurs three
# times in `radix_tree.rs`). Doing test-stripping, matching and joining in one pass also keeps the
# reported line the file's own; the older `grep -n` over a stripped stream renumbered once a
# `#[cfg(test)]` block preceded a site.
PANIC_SITES_AWK='
function braces(s,   o, c, i, ch) { o = 0; c = 0; for (i = 1; i <= length(s); i++) { ch = substr(s, i, 1); if (ch == "{") o++; else if (ch == "}") c++ } return o - c }
function balance(s,   o, c, i, ch) { o = 0; c = 0; for (i = 1; i <= length(s); i++) { ch = substr(s, i, 1); if (ch == "(" || ch == "[") o++; else if (ch == ")" || ch == "]") c++ } return o - c }
function norm(s) { gsub(/[[:space:]]+/, " ", s); sub(/^ /, "", s); sub(/ $/, "", s); return s }
BEGIN { skip = 0; depth = 0 }
{
  if (skip == 0) {
    if ($0 ~ /^[[:space:]]*#\[cfg\(test\)\]/) { skip = 1; depth = 0; next }
    if ($0 ~ /^[[:space:]]*#\[(tokio::)?test\]/) { skip = 1; depth = braces($0); if (depth <= 0) skip = 0; next }
  } else {
    depth += braces($0)
    if (depth <= 0) skip = 0
    next
  }
  # A comment line is not a site — `///` and `//!` included, which is why the test is `//` and not
  # `// `: the doc comments in this tree quote `assert!`/`expect`/`panic!` constantly (the file-level
  # whitelist hid that surface until the per-site conversion exposed `key_segment.rs:59`, a doc line
  # naming a caller'\''s `assert!`), and a class that fires on prose is one people learn to work around.
  # The earlier `sorry`/`opaque` ratchet strips comments for the same reason.
  if ($0 ~ /^[[:space:]]*\/\//) next
  if ($0 !~ ENVIRON["RE"]) next
  if ($0 ~ /self\.expect\(|\.expect\(Tok::/) next
  start = FNR
  text = norm($0)
  bal = balance($0)
  while (bal > 0) { if ((getline nxt) <= 0) break; text = text " " norm(nxt); bal += balance(nxt) }
  print start "\t" text
}
'

# How far the evidence may sit from the site: a doc comment, a signature or a guard *above* it, and
# the assertion's own argument list *below* it (a joined site's text is on the site's line for the
# first fragment only).
PANIC_EVIDENCE_ABOVE=25
PANIC_EVIDENCE_BELOW=8

PANIC_CLAIMS=()
PANIC_EVIDENCE_OK=()

scan_panic() {
  local pattern="$1" c dir f line text sites=0 i n=${#WHITELIST_PANIC[@]}
  PANIC_CLAIMS=(); PANIC_EVIDENCE_OK=()
  for ((i = 0; i < n; i++)); do PANIC_CLAIMS[i]=0; PANIC_EVIDENCE_OK[i]=0; done
  for c in "${CRATES[@]}"; do
    dir="$ROOT/$c/src"
    [ -d "$dir" ] || continue
    while IFS= read -r f; do
      while IFS=$'\t' read -r line text; do
        [ -n "$line" ] || continue
        sites=$((sites + 1))
        panic_site "$f" "$line" "$text"
      done < <(RE="$pattern" awk "$PANIC_SITES_AWK" "$f")
    done < <(find "$dir" -name '*.rs' | grep -vE "$TEST_ONLY_FILE_RE")
  done
  panic_guard "$sites"
}

# The fields are separated by `;;` and parsed **from both ends**: `file;;<site>;;<evidence>;;<reason>`.
# Left-anchored parsing is wrong here twice over — a site is a Rust *statement*, so it can contain the
# separator itself (`assert_eq!(bytes.len(), LENGTH, "expected {LENGTH} bytes");` ends in `;`, and the
# site text `...");` followed by `;;` is three semicolons), and the sdk's escape hatch contains `|`
# (`unwrap_or_else(\|\| panic!`), which is why `|` cannot be the separator either. So: the file is the
# first field, the reason and then the evidence are taken from the right, and the site is what is
# left. Measured 2026-09-25: both left-anchored spellings reported three sites unclaimed while their
# entries were reported stale — the entries were never wrong, the parser was.

parse_entry() {
  # $1 = entry, $2..$5 = names to assign file, site, evidence, reason to.
  #
  # Right-anchored, and every field but the first is **cut by length, never by a pattern**: a
  # `%pattern` removal treats the pattern as a glob, and an evidence regex like `TryFrom<&[u8]>`
  # contains `[u8]` — a character class — so `"${rest%";;"$reason}"` matched a *shorter* suffix and
  # left half the reason inside the site (measured 2026-09-25: four entries parsed into nonsense
  # while their text was correct). The only patterns used here are `*;;`, which carry no user data.
  local e="$1" rest reason front ev site
  local f="${e%%;;*}"
  rest="${e#*;;}"
  reason="${rest##*;;}"
  front="${rest:0:$(( ${#rest} - ${#reason} - 2 ))}"
  ev="${front##*;;}"
  site="${front:0:$(( ${#front} - ${#ev} - 2 ))}"
  printf -v "$2" '%s' "$f"
  printf -v "$3" '%s' "$site"
  printf -v "$4" '%s' "$ev"
  printf -v "$5" '%s' "$reason"
}

panic_site() {
  # $1 = file, $2 = line, $3 = the site's whole (joined) text.
  local f="$1" line="$2" text="$3"
  local i entry wfile wsite wevid wreason rest mid claims=0 claim=-1
  for ((i = 0; i < ${#WHITELIST_PANIC[@]}; i++)); do
    entry="${WHITELIST_PANIC[i]}"
    parse_entry "$entry" wfile wsite wevid wreason
    case "$f" in *"$wfile") ;; *) continue ;; esac
    [[ "$text" =~ $wsite ]] || continue
    claims=$((claims + 1)); claim=$i
  done
  if (( claims == 0 )); then
    note panic "$f" "$line" "$text"
    return
  fi
  if (( claims > 1 )); then
    note panic "$f" "$line" "$text — claimed by $claims allowlist entries; a site must be named by one"
    return
  fi
  # The loop above ends on the last entry, not the claiming one: re-read the winner's fields.
  parse_entry "${WHITELIST_PANIC[claim]}" wfile wsite wevid wreason
  # The evidence must be *at the site*: the guard the entry names, not a guard that still exists
  # somewhere else in the file.
  local from=$(( line > PANIC_EVIDENCE_ABOVE ? line - PANIC_EVIDENCE_ABOVE : 1 ))
  local win
  win="$(sed -n "${from},$((line + PANIC_EVIDENCE_BELOW))p" "$f")"
  if [[ "$win" =~ $wevid ]]; then
    PANIC_CLAIMS[claim]=$(( ${PANIC_CLAIMS[claim]:-0} + 1 ))
    PANIC_EVIDENCE_OK[claim]=1
    printf '  (allowed) %s:%s%s — %s\n' "${f#"$ROOT/"}" "$line" \
      "$( (( ${PANIC_CLAIMS[claim]} > 1 )) && printf ' (+%s more site(s) of the same text)' "$(( ${PANIC_CLAIMS[claim]} - 1 ))" )" \
      "$(wreason_of "$claim")"
  else
    note panic "$f" "$line" "$text — allowlist evidence ($wevid) is not at the site"
  fi
}

wreason_of() {
  local e="${WHITELIST_PANIC[$1]}"
  printf '%s' "${e##*;;}"
}

panic_guard() {
  # The clause that keeps this table honest about itself: every entry must name a site the scan
  # found, and the scan must have found something. A whitelist that has stopped matching suppresses
  # nothing while reading as a review, and a scan that found nothing is not evidence either way.
  local sites="$1" i entry allowed=0 named=0
  for ((i = 0; i < ${#WHITELIST_PANIC[@]}; i++)); do
    if (( ${PANIC_CLAIMS[i]:-0} > 0 )); then
      allowed=$((allowed + 1)); named=$((named + ${PANIC_CLAIMS[i]}))
    else
      local ef es ee er
      parse_entry "${WHITELIST_PANIC[i]}" ef es ee er
      note panic "$ROOT/tools/audit-type-system.sh" "-" \
        "allowlist entry claims no site (stale): $ef — $(printf '%s' "$es" | cut -c1-70)"
    fi
  done
  printf '  (reviewed) %s panic-class site(s) in production code; %s allowlisted by %s entr(y|ies)\n' "$sites" "$named" "$allowed"
  if (( sites == 0 )); then
    note panic "$ROOT/tools/audit-type-system.sh" "-" \
      "the panic class found no site at all — an allowlist that suppresses nothing is not evidence, and neither is a scan that found nothing"
  fi
}

scan() {
  # $1 = kind; $2 = grep -E pattern.
  local kind="$1" pattern="$2"
  local c f line text
  for c in "${CRATES[@]}"; do
    local dir="$ROOT/$c/src"
    [ -d "$dir" ] || continue
    while IFS= read -r f; do
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
# and an entry matching no site, or whose evidence has gone, is a hard failure — as is a site named
# by two entries. An entry *may* name several sites of the same text (the key is a text, and
# `assert_eq!(…, …)` is written twice in one file); "one entry, one site" there would be a rule about
# line numbers dressed up as a rule about text. Note the field separator is `|` and a *construction*
# pattern must therefore not contain one: today none does (measured), but that is a latent trap this
# class shares with the panic list's history — see the panic entries' note on separators.
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
    # `[[:space:]]` and not `\s`: the pattern is a *dynamic* regex to awk, and `\s` is a GNU
    # extension to it (grep accepts it, mawk does not) — a site matched by grep and missed by awk
    # would be a check that went quiet on one implementation.
    panic)   scan_panic '\.unwrap\(\)|\.expect\(|panic!|unreachable!|todo!|unimplemented!|(^|[^[:alnum:]_])(debug_)?assert(_eq|_ne)?!\(|unwrap_or_else\([[:space:]]*\|\|[[:space:]]*panic!' ;;
    unsafe)  scan unsafe 'unsafe[[:space:]]*\{' ;;
    silent)  scan silent 'try_into\(\)\.(unwrap|expect)\(|try_(into\(\)|from\(.*\))\.unwrap_or(\(0\)|_default\(\))|\.parse(::<[^>]+>)?\(\)\.unwrap_or(\(0\)|_default\(\))' ;;
    cast)    scan cast '\bas (i8|i16|i32|i64|u8|u16|u32|u64|usize|isize|f32|f64)\b' ;;
    lax)     scan lax 'from_str_radix\([^)]*\)\.(unwrap_or|unwrap|expect)\(|unsafe_decode\(' ;;
    get)     scan get '\.get\([^)]*\)\.(unwrap|expect)\(|\.(next|last|first|pop)\(\)\.(unwrap|expect)\(|\b[a-zA-Z_]+\[[0-9]+\]' ;;
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
