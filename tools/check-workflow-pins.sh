#!/usr/bin/env bash
# check-workflow-pins.sh — every workflow action pinned to a commit, every workflow declaring its
# permissions, and every container base image pinned by digest.
#
# **The same supply-chain default, three places it fails quietly.**
#
# A `uses:` that names a tag or a branch (`actions/checkout@v7`, `@stable`) is a dependency whose
# content can change after review and without a commit here — the pin is the only thing that makes
# "the workflow we audited" the workflow that runs. A workflow with no `permissions:` gets GitHub's
# default token, which is read-write across the repository unless the org has tightened it: a
# compromised step then has more than the job needs. And a `FROM rust:1.95.0` is the *same* mutable
# name one layer down — whoever controls the upstream image can repoint the tag at different content,
# and the artifact this repository ships changes with no commit here (W-N6-07). None of the three is
# visible in review, because all three *look* like ordinary YAML.
#
# This is the check that issue #91 owed and never got. It is deliberately small and single-purpose:
# the gate it replaces (`tools/audit-test-register.sh`, check 16) was 1,453 lines and was deleted on
# 2026-09-27 for being a document-versus-tree linter whose failures all meant "a document disagrees".
# The logic below is that check's, re-derived rather than restored. The Dockerfile half was added
# 2026-10-13 (W-N7-05, which noted this script never opened one) — here rather than in a new script
# because this one is already wired into CI, so the extension needs no workflow edit; a token without
# the `workflow` scope cannot merge one.
#
# What it checks:
#   1. for each `.github/workflows/*.yml`: every `uses:` ref is a **40-hex commit** — a tag, a branch,
#      or any other ref is floating; `uses: ./local-action` is allowed, because an in-repo path is not
#      a supply-chain input;
#   2. each such file declares a **top-level `permissions:`**;
#   3. for each `Dockerfile*` under `docker/` or the repository root: every `FROM` ref ends in
#      `@sha256:<64 hex>`. `FROM <stage>` — a reference to an earlier `FROM … AS <stage>` — is not a
#      registry ref and is allowed.
#
# **A run that inspected nothing fails rather than passing.** `grep` over an empty glob exits non-zero
# and an unread directory prints nothing, either of which would otherwise read as "all clean" — the
# failure mode `tools/check-rust-witnesses.sh` names for its own zero-match case.
#
# Usage: tools/check-workflow-pins.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORKFLOWS="$ROOT/.github/workflows"
failures=0
fail() { printf 'FAIL  %s\n' "$*"; failures=$((failures + 1)); }
ok() { printf 'ok    %s\n' "$*"; }

shopt -s nullglob globstar
files=("$WORKFLOWS"/*.yml)
if [ "${#files[@]}" -eq 0 ]; then
  fail "no workflow files under $WORKFLOWS — a check with nothing to read is not a check"
fi

pinned=0
for f in "${files[@]}"; do
  name="$(basename "$f")"

  # 1. the refs. `uses:` appears both as a list item (`- uses: …`) and as a mapping key
  # (`        uses: …`), so this matches the key anywhere on the line and takes the value up to the
  # first whitespace — which is where a trailing `# v7` comment begins.
  #
  # **Comment lines are dropped first, and that is not tidiness.** The phrase `uses:` also occurs in
  # prose — including this file's own step comment in `ci.yml`, which explains what the gate looks
  # for — and matching there parses the sentence as a ref and fails on it. Found by running the gate
  # on the commit that added it, which is the cheapest possible place to find it.
  while IFS= read -r ref; do
    case "$ref" in
      ./*)
        # A local action: an in-repo path is not a supply-chain input, so it is not a floating ref.
        pinned=$((pinned + 1))
        ;;
      *)
        if printf '%s' "$ref" | grep -qE '@[0-9a-f]{40}$'; then
          pinned=$((pinned + 1))
        elif printf '%s' "$ref" | grep -q '@'; then
          fail "$name: '$ref' is not pinned to a commit (a tag or a branch can change under us)"
        else
          fail "$name: '$ref' has no '@ref' at all — malformed, or the parse missed it"
        fi
        ;;
    esac
  done < <(grep -vE '^[[:space:]]*#' "$f" \
           | grep -oE 'uses:[[:space:]]*[^[:space:]#]+' \
           | sed -E 's/uses:[[:space:]]*//' || true)

  # 2. the permissions block. Top-level only: an indented `permissions:` inside a job scopes that job
  # and leaves the workflow default untouched, which is the case this is here to catch.
  if ! grep -qE '^permissions:' "$f"; then
    fail "$name: no top-level 'permissions:' block — the token defaults to more than the job needs"
  fi

  ok "$name"
done

# 3. container base images, pinned by digest. `FROM <stage>` is allowed: it names an earlier stage in
# the same file, which is not a registry ref, and the way to tell one from a registry short name is
# that it has neither a `:` tag nor an `@` digest and has already been declared by an `AS`.
maps=()
for f in "$ROOT"/Dockerfile* "$ROOT"/docker/**/Dockerfile*; do
  if [ -f "$f" ]; then maps+=("$f"); fi
done
if [ "${#maps[@]}" -eq 0 ]; then
  fail "no Dockerfile found — a check with nothing to read is not a check"
fi

digests=0
for f in "${maps[@]}"; do
  name="${f#"$ROOT"/}"
  stages=()
  while IFS= read -r line; do
    read -r -a tok <<< "${line%%#*}"
    i=1
    # `FROM --platform=… <ref> AS <stage>`: skip any flags before the ref.
    while [ "$i" -lt "${#tok[@]}" ] && [[ "${tok[$i]}" == --* ]]; do i=$((i + 1)); done
    ref="${tok[$i]:-}"
    if [ -z "$ref" ]; then
      fail "$name: 'FROM' with no image reference"
      continue
    fi
    if [[ ! "$ref" =~ [:@] ]] && [[ " ${stages[*]-} " == *" $ref "* ]]; then
      continue # a stage reference, not an image
    fi
    if printf '%s' "$ref" | grep -qE '@sha256:[0-9a-f]{64}$'; then
      digests=$((digests + 1))
    else
      fail "$name: 'FROM $ref' is not pinned by digest (a tag can be repointed under us)"
    fi
    for ((j = i + 1; j < ${#tok[@]}; j++)); do
      if [[ "${tok[$j]^^}" == "AS" ]]; then stages+=("${tok[$((j + 1))]}"); break; fi
    done
  done < <(grep -viE '^[[:space:]]*#' "$f" | grep -iE '^[[:space:]]*FROM[[:space:]]')
  ok "$name"
done

echo ""
if [ "$failures" -gt 0 ]; then
  echo "===== $failures supply-chain pin check(s) FAILED ====="
  exit 1
fi
echo "ok    workflows: ${#files[@]} file(s), $pinned pinned action ref(s), every workflow declaring permissions"
echo "ok    containers: ${#maps[@]} Dockerfile(s), $digests digest-pinned base image(s)"
echo "===== every workflow action and every container base image is pinned ====="
