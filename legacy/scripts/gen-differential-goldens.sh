#!/usr/bin/env bash
# Regenerate the Scala ground-truth golden vectors consumed by the Rust differential tests.
#
# The vectors are the *oracle*: produced by the Scala node and committed under
# `<crate>/testdata/differential/` at the **repository root** (the crates moved out of `crates/` when
# the repo was restructured — an older version of this script still wrote to `crates/…`). The Rust
# `#[cfg(test)] differential` modules assert the port reproduces them byte-for-byte, and each file's
# rows carry a `provenance` column so a regression pin is never mistaken for ground truth:
#
#   scala-ground-truth      produced by an oracle below, run via sbt
#   scala-rule-transcribed  computed from a proto schema plus a documented Scala rule, not run
#   rust-regression-pinned  the port's own behaviour; NOT evidence of Scala agreement
#
# Prerequisite: **sbt** and a working `legacy/` build. CI does not install sbt (the workflows run the
# Rust side only), so this is a local/legacy-environment tool: without sbt it fails here, loudly,
# rather than writing an empty golden file that would silently break every differential test.
#
# Usage: legacy/scripts/gen-differential-goldens.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
LEGACY="$ROOT/legacy"

if ! command -v sbt >/dev/null 2>&1; then
  cat >&2 <<'EOF'
error: sbt is not on PATH.

This script regenerates the golden vectors by *running the Scala oracles* in `legacy/`, so it needs
sbt and a working legacy build. It is deliberately not part of CI: the Rust workflows have no JVM.

Install sbt (https://www.scala-sbt.org/download) and build the legacy project first:

    cd legacy && sbt compile

Nothing was written. If you need only to *check* the committed goldens, run the guards instead:

    cargo test -p rchain-models --lib differential
    cargo test -p rchain-rspace --lib stable_hash_provider
    cargo test -p rchain-rspace --lib scodec_serialize
    cargo test -p rchain-rholang --test execution every_execution_golden_row
EOF
  exit 2
fi

oracle() { # <crate-dir> <file> <sbt-main-class>
  local dir="$1" file="$2" main="$3"
  local out="$ROOT/$dir/testdata/differential/$file"
  echo "==> $file ($main)"
  {
    printf '# id\tvalue\tprovenance\n'
    printf '# scala-ground-truth: produced by %s (run via sbt).\n' "$main"
    (cd "$LEGACY" && sbt "rspace/Test/runMain $main") \
      | sed -e 's/[[:space:]]*$//' -e '/^$/d' \
      | awk -F'\t' '{ printf "%s\t%s\tscala-ground-truth\n", $1, $2 }'
  } > "$out.tmp"
  mv "$out.tmp" "$out"
  echo "    wrote $out ($(grep -cv '^#' "$out") rows)"
}

oracle rspace stable_hash.tsv coop.rchain.rspace.differential.StableHashOracle
oracle rspace scodec.tsv      coop.rchain.rspace.differential.ScodecOracle

cat <<'EOF'

Not regenerated here, on purpose:

  models/testdata/differential/wire.tsv   `scala-rule-transcribed`: derived from the shared
                                          RhoTypes.proto schema plus the scalapb TypeMapper's
                                          bitSetToByteString rule. The wire format is pinned by the
                                          proto, so there is nothing to run.

  rholang/testdata/differential/execution.tsv
                                          `rust-regression-pinned`: **no Scala oracle exists** for the
                                          rholang execution pipeline. The rows record the Rust port's
                                          own behaviour and are a regression pin, not a differential
                                          vector. Writing an `ExecutionOracle.scala` under
                                          legacy/rholang/src/test/... would let those rows become
                                          ground truth; until then this script must not touch the file.

Re-run the Rust side to confirm the port still matches:
    cd "$ROOT" && cargo test -p rchain-rspace --lib differential
EOF
