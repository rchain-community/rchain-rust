# Operating the node: REPL and the Docker multi-node network

Two thin-client surfaces ship with the `rnode` binary: the interactive rholang **REPL** and the
scripted **Docker multi-node network**. This page is the operation guide for both.

---

## Shard memberships

A node validates one shard by default (`casper.shard-name = root` under `casper.parent-shard-id = /`,
giving the full id `/root`). A **gateway** node validates several — it produces and validates each
independently, routes every request to the shard that owns it, and can coordinate a cross-shard
transaction across its own shards ([cross-shard transactions](../formal/cross-shard-transactions.md)).

```hocon
casper {
  # One entry per membership; the FIRST is the primary shard (the default target for requests that
  # do not name a shard). Each entry may override `genesis-block-data` field by field.
  shards = [
    { shard-name = root, parent-shard-id = / }
    { shard-name = child, parent-shard-id = /root,
      genesis-block-data { wallets-file = /var/lib/rnode/genesis-child/wallets.txt } }
  ]
}
```

Notes for operators:

- **File-configured.** The command line cannot address array entries, so per-shard genesis data
  (bonds and wallets) is expressed in `rnode.conf`; `--shard-name`/`--parent-shard-id` still work and
  define the one-element default. Setting both the array and those scalar keys is an error.
- **Storage.** The primary membership keeps the data directory itself; each additional shard nests
  under `<data-dir>/shard/<shard-id segments>/`. A directory records the shard that owns it
  (`shard-id`), and a mismatch is a startup error rather than a node silently running the wrong
  chain — so keep a shard's position in the list stable.
- **Sync.** LFS sync is not shard-aware, so a multi-shard node must reach its shards from their own
  genesis (genesis master / `--standalone`) or from existing local state.
- **Cross-shard transactions.** Set `api-server.enable-txn-api = true` on a gateway to serve
  `POST /api/v1/txn` (plus `GET /api/v1/txn` and `/api/v1/txn/{txnId}`). A single-shard node, or one
  without a validator key, answers 404. The legs are ordinary deploys, so they take effect when a
  block includes them — enable `--propose-on-deploy` or `--autopropose`, or the transaction will
  time out waiting.
- `GET /api/v1/shards` lists the memberships, primary first, each with its own chain height. The
  membership is deliberately *not* folded into `/api/status`, whose `shardId` field existing tooling
  parses on its own.

---

## The REPL

`rnode repl` is a thin gRPC client: the interactive loop runs on your machine and forwards each line
to a running node, which parses, normalizes, and evaluates it and returns the rendered result. The
evaluation happens on the node — inside its **isolated `eval-*` store** — so REPL terms never touch
the node's live chain state (`rspace-*`).

### Start a node

```sh
# Local standalone node (creates genesis on first boot):
cargo run --release -p rchain-node --bin rnode -- run -s
```

The node serves **Deploy** on the external gRPC port **40401** and **Propose + Repl** on the internal
loopback port **40402** (propose/repl are not network-reachable — `spec/AUDIT.md` C1).

### Standalone genesis prerequisites

`run -s` makes the node the **genesis master**, and it needs two things the bare command above does
not set up for you:

1. **A validator key.** The genesis block must be signed, so provide a secp256k1 private key (32
   bytes, base16) through the *hidden* `--validator-private-key` flag:

   ```sh
   rnode run -s --validator-private-key 67e56582298859ddae725f972992a07c6c4fb9f62a8fff58ce3ca926a1063530
   ```

   Without it the node exits with `To create genesis block node must provide validator private key`.
   (`--validator-private-key-path` accepts a PEM file, but the Rust runtime currently reads only the
   hex form.)

2. **A wallets file.** The genesis ceremony parses `~/.rnode/genesis/wallets.txt` *strictly*, so the
   file must exist — an empty file is fine. `bonds.txt` is auto-generated when absent, but to be
   bonded as a validator, provide a `<public_key> <stake>` line (public key = uncompressed 65-byte
   point, base16) matching your key:

   ```sh
   mkdir -p ~/.rnode/genesis
   touch ~/.rnode/genesis/wallets.txt
   echo "04c591a8ff19ac9c4e4e5793673b83123437e975285e7b442f4ee2654dffca5e2d2103ed494718c697ac9aebcfd19612e224db46661011863ed2fc54e71861e2a6 100" \
     > ~/.rnode/genesis/bonds.txt
   ```

   A missing wallets file exits with `FAILED PARSING WALLETS FILE: … No such file or directory`.

### Bind to loopback (localhost-only)

The API server builds its listen address with `SocketAddr::from_str`, so `--api-host` must be a
**literal IP**, not a hostname — `--api-host localhost` fails with `invalid socket address syntax`.
For a localhost-only node, use `127.0.0.1` and skip the UPnP probe and external-IP guessing:

```sh
rnode run -s \
  --validator-private-key 67e56582298859ddae725f972992a07c6c4fb9f62a8fff58ce3ca926a1063530 \
  --host 127.0.0.1 --api-host 127.0.0.1 --no-upnp
```

`--host 127.0.0.1` sets the advertised protocol address to loopback and also stops the node probing
external services to guess its public IP (otherwise it logs `guessing your external IP address…`);
`--no-upnp` disables the gateway probe. All node data lives under `~/.rnode` by default.

### Run the REPL

```sh
# In a second terminal, against the local node (defaults: localhost:40402):
cargo run --release -p rchain-node --bin rnode -- repl

# Against a remote node:
rnode --grpc-host <host> --grpc-port 40402 repl
```

The prompt is `rholang $ ` with line-editing, history, and tab-completion over the REPL keywords
(`stdout`, `stdoutack`, `stderr`, `stderrack`, `for`, `!!`). Each submitted term:

1. is parsed and normalized; a syntax error surfaces as `Error: …` without evaluating;
2. on success, the normalized term is echoed on the **node console** as an `Evaluating:` line;
3. is evaluated, and the result is printed:

```
Deployment cost: 33
Storage Contents:
@{Unforgeable(0x…)}!(0) |
for( … ) { Nil } | …
```

`:q` (or EOF/`Ctrl-D`) quits. A single evaluation error stops the loop — matching the Scala
thin-client REPL.

### Evaluate files

```sh
rnode eval file.rho [--print-unmatched-sends-only]
```

`eval` reads the files client-side, sends each whole program to the node, and prints a `Result for
<file>:` header per file.

### REPL internals (short)

| Piece | File |
|---|---|
| client loop (`:q`, coloring) | `node/src/runtime/repl_runtime.rs` |
| rustyline console (prompt, history, completion) | `node/src/effects/console_io.rs` |
| gRPC client | `node/src/effects/repl_client.rs` |
| server eval (`Evaluating:` echo + result) | `node/src/api/grpc/repl_grpc_service.rs` |
| isolated eval store | `node/src/runtime/node_runtime.rs` (`"eval"` prefix → `rspace/src/factory.rs`) |

`rho:io:stdout` / `rho:io:stderr` print on the **node** process (server side), matching the Scala
thin-client model — not on the REPL client.

---

## The Docker multi-node network (bare topology)

`tools/devnet.sh up --nodes N` boots a bare **1–5 node** network (one bootstrap + `N-1` unbonded peers)
with no autopropose and no deployer wallet — the network-*topology* mode for exercising sync/gossip,
driven manually via `cli`. (The same script's default mode is the contract *devnet* — see
[Local devnet](devnet.md).) The image is built from
[`docker/rnode/Dockerfile`](../../../docker/rnode/Dockerfile).

### Prereqs

- `docker` (a running daemon)
- `openssl` (to read the bootstrap node-id from its generated TLS cert)

### Commands

```sh
tools/devnet.sh build                       # build the rnode:local image
tools/devnet.sh up --nodes 3                # bootstrap + 2 peers (N in 1..5)
tools/devnet.sh status                      # docker ps for the network
tools/devnet.sh logs <node>                 # tail a node's logs
tools/devnet.sh cli <node> <subcommand…>    # run the Rust client against <node>
tools/devnet.sh down                        # stop the network
tools/devnet.sh down -v                     # stop + delete the data volumes
```

### Topology and genesis

`up --nodes N` starts:

1. **`devnet-bootstrap`** — `rnode run -s` (standalone). It generates its own TLS cert, **creates the
   genesis block**, and is bonded as the sole validator via a fixed validator key and a generated
   `bonds.txt`/`wallets.txt` mounted read-only.
2. **`devnet-observer-1` … `devnet-observer-N-1`** — `rnode run --bootstrap rnode://<bootstrap-id>@devnet-bootstrap?protocol=40400&discovery=40404`.
   Each peer gets its own data volume (so its node identity — the TLS cert — is stable), connects to
   the bootstrap over the **TLS protocol transport** (port 40400), and syncs the finalized fringe.

The bootstrap's node-id is read from its generated certificate (the cert CommonName is the base16
keccak-20 address). The script waits for the cert, extracts the id with `openssl`, and passes it into
each peer's bootstrap URL.

### Ports

Each node binds the same in-container ports; `up` maps each node's **deploy** gRPC port to a distinct
host port so you can also reach a node from the host. Propose/repl bind loopback-only
(`127.0.0.1:40402`), so they are not host-mapped. In bare (`--nodes`) mode the admin HTTP port is not
published (`--no-admin`):

| Service | In-container | Host (bootstrap / peerN) |
|---|---|---|
| protocol (TLS peer transport) | 40400 | not mapped (network-internal) |
| discovery (Kademlia) | 40404 | not mapped |
| gRPC API — Deploy | 40401 | 40402 / 40402 + 1000·N |
| gRPC API — Propose/Repl | 40402 | not mapped (loopback-only) |
| HTTP | 40403 | 40403 / 40403 + 1000·N |
| admin HTTP | 40405 | not published |

### Interacting from the CLI

`cli <node> <subcommand…>` runs the Rust client **inside the node container** via `docker exec`, so it
can reach both the deploy server (external port **40401**) and the loopback-only propose/repl server
(`127.0.0.1:40402`):

```sh
tools/devnet.sh cli devnet-bootstrap status
tools/devnet.sh cli devnet-bootstrap repl       # interactive REPL against the bootstrap
tools/devnet.sh cli devnet-observer-1 status
tools/devnet.sh cli devnet-bootstrap propose   # force a block (bare mode has no autopropose)
```

No `--grpc-port` is passed: the client defaults to the right port per subcommand (`deploy`/`status`/
`show-block`/… → 40401; `repl`/`eval`/`propose` → 40402), matching the node's port split.

`deploy` / `eval` read a file at a path inside the container; copy a local file in first:

```sh
docker cp demo.rho devnet-bootstrap:/tmp/demo.rho
tools/devnet.sh cli devnet-bootstrap deploy /tmp/demo.rho
```
