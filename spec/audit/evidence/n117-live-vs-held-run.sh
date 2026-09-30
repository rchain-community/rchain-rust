#!/usr/bin/env bash
# Is the memory that reaches the 4 GiB ceiling **live** or **held**? One number decides it.
#
# `stats.allocated` is what jemalloc believes the application holds; `stats.resident` is what it has
# resident on the application's behalf. Read at the ceiling:
#
#   allocated ~ anon                  -> live. A structure, and a heap profile can name it.
#   allocated << anon, resident ~ anon -> held. Purge rate, and allocator configuration is the fix space.
#
# This is the measurement that two earlier attempts *appeared* to make and did not: `tikv-jemalloc-sys`
# passes `--enable-stats` and `--enable-prof` only for the `stats` / `profiling` Cargo features, and
# neither was on, so every jemalloc option in the earlier `MALLOC_CONF`s was ignored without complaint and
# the runs read a never-compiled-in instrument as one that found nothing. `node/Cargo.toml` now enables
# `stats`; `profiling` stays off on purpose (it samples every allocation and the defect is
# timing-sensitive).
#
# `stats_interval` (not just `stats_print`) is the capture of record, because the node that matters is the
# one that gets OOM-killed: SIGKILL runs no exit handler, so a dump-at-exit would be lost on exactly the
# node the measurement exists for. The interval writes to stderr, which is the container log, which
# survives the kill.
set -u
cd /home/patrick/RNodeRust
OUT=${OUT:-target/n117-audit/live-vs-held.tsv}
# `;` so that the comma-containing MALLOC_CONF is not shredded by DEVNET_NODE_ENV's separator.
CONF="_RJEM_MALLOC_CONF=background_thread:true,dirty_decay_ms:0,muzzy_decay_ms:0,retain:false,stats_print:true,stats_print_opts:g,stats_interval:1000,stats_interval_opts:g;RUST_BACKTRACE=1"

echo "config: cap=4g stats=on profiling=off tree=$(git rev-parse --short HEAD)"
echo "image: $(docker inspect rnode:local --format '{{.Id}} {{.Created}}' 2>/dev/null)"
tools/devnet.sh down >/dev/null 2>&1
DEVNET_NODE_MEMORY=4g DEVNET_NODE_ENV="$CONF" \
  timeout 900 tools/devnet.sh up --validators 3 --stakes 100,100,50 --epoch-length 10 --fresh 2>&1 | tail -1

echo "--- the env reached the container? ---"
docker inspect devnet-bootstrap --format '{{range .Config.Env}}{{println .}}{{end}}' 2>/dev/null | grep -o '_RJEM_MALLOC_CONF=[^ ]*stats_interval:1000' | head -1 || echo "  NOT SET"

echo "--- sampling cgroup anon alongside the node's own gauges ---"
python3 target/n117-audit/queue-depth.py "$OUT"

echo "=== outcome ==="
for c in devnet-bootstrap devnet-validator-1 devnet-validator-2; do
  docker inspect "$c" --format '  {{.Name}} {{.State.Status}} oom={{.State.OOMKilled}} exit={{.State.ExitCode}}' 2>/dev/null
done

echo "=== jemalloc's own accounting, per node (last interval block before death) ==="
for c in devnet-bootstrap devnet-validator-1 devnet-validator-2; do
  echo "--- $c ---"
  docker logs "$c" 2>&1 | grep -a -E '^Allocated:' | tail -2 || echo "  no stats block (feature off? node killed before the first interval?)"
done
echo "(kept: the containers are left up so the logs can be read again; run 'tools/devnet.sh down' after)"
