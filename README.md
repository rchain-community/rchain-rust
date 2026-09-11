# RChain — Rust rewrite

A faithful Rust rewrite of the [RChain](https://rchain.coop) node. The Rust implementation is a
Cargo workspace at the top level (one crate per original sbt module); the entire upstream Scala
fork is preserved for reference under [`legacy/`](legacy/).

## Provenance

This repository was developed through commit `14b8b77` on
[`PatrickMockridge/rchain-rust`](https://github.com/PatrickMockridge/rchain-rust) — a fork of
[`rchain/rchain`](https://github.com/rchain/rchain). As of that commit, the repository lives at
[`rchain-community/rchain-rust`](https://github.com/rchain-community/rchain-rust). A snapshot of the
code at that state is timestamped on Arweave — transaction
[`MK4WA8w3NTIIFd6iaD06EPyxVbFhoV9MtfhMNcHWWMw`](https://arweave.net/MK4WA8w3NTIIFd6iaD06EPyxVbFhoV9MtfhMNcHWWMw),
22 August 2026 7:15pm.

## Documentation

The documentation is served as a book (`mdbook serve docs`). It is organized software-first:

- **Part I — Rholang & the ρ-calculus** ([`docs/src/rholang/`](docs/src/rholang/)) — the language:
  what it is, why it fits a blockchain, and from processes and names through object-capability smart
  contracts.
- **Part II — The ρ-calculus, formally** ([`docs/src/formal/`](docs/src/formal/)) — the grammar, the
  sorts, and the 22 laws — including the channel scheduler (Laws 20–22) — mapped to their
  machine-checked proofs.
- **Part III — The node** ([`docs/src/node/`](docs/src/node/)) — consensus, the tuple space, storage,
  and operation.
- **Part IV — Contributor / port** ([`docs/src/contributor/`](docs/src/contributor/)) — why Rust, and
  the per-module status.

The entry point for the book is [`docs/src/introduction.md`](docs/src/introduction.md); the
goal-indexed map for readers and AI agents is
[`docs/src/ai-entrypoint.md`](docs/src/ai-entrypoint.md).

## Why Rust

Three reasons drive the rewrite.

**Memory safety.** The Scala/JVM node leaked memory and paused on garbage collection — it shipped
`Memory`/`GarbageCollector` diagnostics and needed `SBT_OPTS="-Xmx4g -Xss2m"` to run. Rust's ownership
model and lack of a tracing GC make the leak and the stop-the-world pause unrepresentable.

**Decentralization.** ~69,000 lines of Rust compile to a single tight native binary — no JVM, no GC,
no heap tuning — so a validator runs on any modern desktop or laptop with an NVMe SSD. Validator
operation sits within consumer-grade hardware, which is what makes the network genuinely decentralized
(see [hardware requirements](docs/src/node/validator-requirements.md)).

**The calculus hierarchy.** Rust natively expresses the λ-calculus (closures), the π-calculus
(channels and `Send`/`Sync` name mobility), and the ρ-calculus (the reflective π-calculus: a name is a
quoted process, expressed here as the sortable `Par` value). The port's type discipline embeds ρ as
the base sort of a Calculus of Constructions, constructible and provable in Lean 4 and Coq.

The full argument — including the co-op lesson and the Rust → calculus → formalization correspondence
table — is in [docs/src/contributor/why-rust.md](docs/src/contributor/why-rust.md). The prose
documentation is also served as a book: `mdbook serve docs`.

## Governance

The rewrite is governed by [`AGENTS.md`](AGENTS.md) — the binding intent + formal specification —
and the machine-checked formalizations in [`spec/`](spec/) (the 22-law invariant inventory in
[`spec/INVENTORY.md`](spec/INVENTORY.md), plus the Lean/Coq tracks). The prime directive is a
**faithful implementation of the ρ-calculus**: the 22 laws are the oracle; the Scala node was the
port reference.

## Funding

Development is funded via [OpenCollective](https://opencollective.com/rho-vision-community), under
the **Rho Vision (formerly RChain Community)** collective:

- [Rholang – Rust Implementation](https://opencollective.com/rholang-rust) — this rewrite.
- [RhoGOV: EIES3](https://opencollective.com/eies3) — electronic information exchange / governance.
- [RHO Tools in Rust](https://opencollective.com/rho-tools-in-rust).

## Layout

The Cargo workspace has thirteen members — twelve crates ported from the original sbt modules
(`sdk`, `shared`, `crypto`, `graphz`, `models`, `block-storage`, `comm`, `rspace`, `rholang`,
`casper`, `node`, `rspace-bench`) plus `qucalc`, the Rust-first native AI + governance crate
(Part V of the book). The per-crate status, the layer map, the rewrite order, and the remaining work
are consolidated in
[docs/src/contributor/architecture.md](docs/src/contributor/architecture.md).

## Build & test

```sh
cargo build --release -p rchain-node --bin rnode   # the `rnode` binary
cargo test --workspace                              # the full test suite
```

Build and serve the documentation book:

```sh
mdbook serve docs   # or: mdbook build docs
```

## REPL

The `rnode` binary doubles as a thin gRPC client. Run the interactive rholang REPL against a node:

```sh
# Start a local standalone node (creates genesis), then in another terminal:
cargo run --release -p rchain-node --bin rnode -- run -s

# Interactive REPL (prompt `rholang $ `, history, tab-completion):
cargo run --release -p rchain-node --bin rnode -- repl

# …or point the client at a remote node (Repl is on the internal port 40402; Deploy is on 40401):
rnode --grpc-host <host> --grpc-port 40402 repl
```

Each term is parsed, normalized (an `Evaluating:` line is echoed on the node console), evaluated
against the node's isolated `eval-*` store, and printed as `Deployment cost:` + `Storage Contents:`.
`:q` quits. Evaluate files non-interactively with `rnode eval <file>...`.

## Docker

`tools/devnet.sh` is the single script for a local Docker network, with two modes:

- **devnet** (default) — 1–3 bonded validators with autopropose + a funded deployer wallet, for
  deploying/testing rholang contracts and reading results back.
- **network** (`up --nodes N`) — a bare 1–5 node topology with no autopropose/deployer, for exercising
  sync/gossip, driven manually via `cli <node> propose`.

```sh
tools/devnet.sh build                 # build the rnode:local image (cached)
tools/devnet.sh build --fresh         # force a clean rebuild (--no-cache --pull)

# contract devnet:
tools/devnet.sh up --validators 1     # single validator, autoproposing
tools/devnet.sh deploy hello.rho      # signed deploy (examples/hello.rho sends "world")
tools/devnet.sh query hello           # -> "world"

# bare network topology:
tools/devnet.sh up --nodes 3          # bootstrap + 2 peers, manual propose
tools/devnet.sh cli devnet-bootstrap propose

tools/devnet.sh down -v               # stop + drop volumes
```

A block is created only when one of these fires: (a) a deploy arrives and `--propose-on-deploy` is set,
(b) `--autopropose`'s timer fires, or (c) someone calls `propose`/`POST /api/v1/propose`. An idle node
with none of these produces no blocks.

See [docs/src/node/devnet.md](docs/src/node/devnet.md) (contract devnet),
[docs/src/node/operating.md](docs/src/node/operating.md) (network topology), and `tools/devnet.sh help`
for the full flag reference.

## Where the Scala went

The upstream Scala fork — the sbt modules (`node/`, `sdk/`, `shared/`, `crypto/`, `models/`,
`rspace/`, `comm/`, `casper/`, `rholang/`, `block-storage/`, `regex/`, `graphz/`, `roscala/`,
`rosette/`, `rspace-bench/`), the sbt build (`build.sbt`, `project/`), configuration, CI, docs,
tooling, and data files — now lives under [`legacy/`](legacy/), with the original directory names.

## License

The Rust rewrite is licensed under the [GNU Affero General Public License, version 3](LICENSE)
(AGPL-3.0). The upstream Scala fork preserved for reference under [`legacy/`](legacy/) retains its
original [Apache License 2.0](legacy/LICENSE.TXT).
