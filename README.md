# RChain in Rust: a language with a theorem, not just a test suite

Rholang's <!-- counts:laws -->53 laws<!-- counts:end --> are machine-checked in Lean 4 and Coq, and
its authority cannot be forged — there is no syntax that writes a private name. Pre-testnet.

**Whole classes of defect are absent by construction**: no `unsafe` in any of the thirteen crates, so
memory-safety bugs are unwritable rather than mitigated. An adversarial audit read this node and
Solana, Sui and Bitcoin SV from source: no chain split, no fund loss, no remote code execution here.

**A small artifact, deliberately**: ~138,000 lines of Rust across 400 files compile to a single **37 MB**
native binary — nothing to boot but the process, no garbage collector to pause, no heap to tune — so a
validator runs on a commodity laptop with an NVMe.

## Provenance

This repository was developed through commit `14b8b77` on
[`PatrickMockridge/rchain-rust`](https://github.com/PatrickMockridge/rchain-rust) — a fork of
[`rchain/rchain`](https://github.com/rchain/rchain). As of that commit, the repository lives at
[`rchain-community/rchain-rust`](https://github.com/rchain-community/rchain-rust). A snapshot of the
code at that state is timestamped on Arweave — transaction
[`MK4WA8w3NTIIFd6iaD06EPyxVbFhoV9MtfhMNcHWWMw`](https://arweave.net/MK4WA8w3NTIIFd6iaD06EPyxVbFhoV9MtfhMNcHWWMw),
22 August 2026 7:15pm.

The node is a Cargo workspace at the top level (one crate per original sbt module); the upstream Scala
fork is preserved for reference at
[`1b7583649`](https://github.com/rchain-community/rchain-rust/tree/1b7583649/legacy) — see
[Where the Scala went](#where-the-scala-went) for why it is a revision rather than a directory.

## Documentation

The documentation is served as a book (`mdbook serve docs`). It is organized software-first:

- **Part I — Rholang & the ρ-calculus** ([`docs/src/rholang/`](docs/src/rholang/)) — the language:
  what it is, why it fits a blockchain, and from processes and names through object-capability smart
  contracts.
- **Part II — The ρ-calculus, formally** ([`docs/src/formal/`](docs/src/formal/)) — the grammar, the
  sorts, and all <!-- counts:laws -->53 laws<!-- counts:end --> — the calculus (1–29), the surface a
  client writes and a matcher reads (30–43), the Proof-of-Stake epoch (44–47), the fee and charging
  rows (48–49) and the depth guards (50) — mapped to their machine-checked proofs.
- **Part III — The node** ([`docs/src/node/`](docs/src/node/)) — consensus, the tuple space, storage,
  and operation.
- **Part IV — Building applications** ([`docs/src/developer/`](docs/src/developer/)) — building
  rholang applications on the local devnet.
- **Part V — Contributor / port** ([`docs/src/contributor/`](docs/src/contributor/)) — why Rust, and
  the per-module status.
- **Part VI — QuCalc: native AI & governance** ([`docs/src/qucalc/`](docs/src/qucalc/)) — the
  Rust-first quantum-to-ρ operators, multi-stakeholder governance, and the concurrent-reducer
  experiments.

The entry point for the book is [`docs/src/introduction.md`](docs/src/introduction.md); the
goal-indexed map for readers and AI agents is
[`docs/src/ai-entrypoint.md`](docs/src/ai-entrypoint.md).

## Why Rust

Three reasons drive the rewrite.

**Memory safety.** The Scala/JVM node leaked memory and paused on garbage collection — it shipped
`Memory`/`GarbageCollector` diagnostics and needed `SBT_OPTS="-Xmx4g -Xss2m"` to run. Rust's ownership
model and lack of a tracing GC make the leak and the stop-the-world pause unrepresentable.

**Decentralization.** ~138,000 lines of Rust compile to a single 37 MB native binary — no JVM, no GC,
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

## Comparative security audit

**A pre-testnet node, measured against live mainnet chains — and ahead on structure.**

Solana, Sui and Bitcoin SV are live, at a combined market capitalisation north of $75bn (September
2026), with years of adversarial exposure, audit budgets and production hardening behind them. This
node is **pre-testnet** and has had none of that. So the September 2026 review read all four from
source at pinned revisions and probed them with the *same* adversarial tests. It found **no chain
split, no fund loss and no remote code execution** here. On the structural axes below, the pre-testnet
node leads.

| | **This node** (pre-testnet) | **Solana** (~$70bn) | **Sui** (~$5bn) | **Bitcoin SV** (~$400m) |
|---|---|---|---|---|
| **Memory safety** | No `unsafe` anywhere — `#![forbid(unsafe_code)]` in all thirteen crates | 52 `unsafe` blocks in the loader alone; forbidden in one crate | `unsafe` in `sui-types`; not forbidden | C++ — assertions cannot be compiled out |
| **Authority cannot be forged** | **Unrepresentable** — no syntax writes a private name; a name exists only to be received | Checked — a program-derived address has no key, but granting the privilege is a runtime check | **Unrepresentable** — a linear `UID`, minted from the transaction digest | Absent as a concept — authority is a private key |
| **Authority cannot be captured** | **Unrepresentable, and proved** — a COMM moves only the datum sent, into the receiver's own environment; closedness is a theorem | Checked at runtime | Unrepresentable — no dynamic dispatch | Moot — there are no calls to capture through |
| **Machine-checked semantics** | Lean 4 + Coq — <!-- counts:laws -->53 laws<!-- counts:end -->; every registered witness resolves; zero `#[ignore]`d tests; the gate refuses `sorry`/`admit`/`opaque` | `frozen-abi` digests, which are checksums | Static bytecode verifier; **Move Prover absent from its tree and CI** | Nothing |
| **Peer identity** | Mutual TLS — the certificate's key is bound to the identity the peer claims in the message | Signed shreds, but no binding to a claimed identity | P2P layer is a git dependency — uninspectable in-tree | Plaintext; the handshake signs nothing |
| **Supermajority arithmetic** | Exact `i128` integer — no floating point | `2f64/3f64` | Integer | Proof of work |
| **Cost model bounds attacker work** | ✓ **closed out the same day** — a per-block phlo cap, and the five operations that were charged less than their work now charge for it | ✗ account copies are unbilled | Instruction tiers, 128 KiB transaction bound | ✗ no step budget at all |

**The last row is the one this node did not lead, and it is the one it fixed within the day** — a defect
class **all four nodes share**, and the close-out is not claimed to be more than it is: those charges
were found by reading the cost table, so the claim is "these five" rather than "the cost model is now
sound".

Two limits bound the table. Two of the four trees could not be fully read — unvendored dependencies —
so those columns are marked as traced rather than cleared; and this node's assurance ends where the
review says it does: the language and calculus are safe-by-structure and partly proved, the chain
layer's authority distribution is access control by another name, and unforgeability is axiomatised at
the crypto boundary (law 19) rather than proved.

**Full report, every citation, the reverted fixes, the open questions — and the ten candidate findings
the pass refuted, including the one it opened expecting to lead with:**
[**docs/src/node/security-audit.md**](docs/src/node/security-audit.md).

## Governance

The rewrite is governed by [`AGENTS.md`](AGENTS.md) — the binding intent + formal specification —
and the machine-checked formalizations in [`spec/`](spec/) (the law register
[`spec/INVENTORY.md`](spec/INVENTORY.md) — <!-- counts:laws -->53 laws<!-- counts:end -->, counted and
emitted by `Rchain/Laws.lean` so the number cannot drift — plus the Lean/Coq tracks). The prime
directive is a **faithful implementation of the ρ-calculus**: the laws are the oracle; the Scala node
was the port reference.

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
(Part VI of the book). The per-crate status, the layer map, the rewrite order, and the remaining work
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
tools/devnet.sh demo                  # deploy examples/wallet.rho + assert its save/load round-trip
python3 tools/devnet-fuzz.py --validators 3   # robustness + determinism fuzz against a live devnet

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
tooling, and data files — lived under `legacy/` from the start of the rewrite.

**It was archived out of the working tree on 2026-09-28.** Two reasons: it was 32 MB and 1,596 files
that no CI job built, tested or scanned, and it carried its own 2020–21 dependency manifest
(`build.sbt`, `project/Dependencies.scala` — netty, logback, protobuf, lz4) that `cargo-deny` cannot
see because it walks only the Rust lockfile.

**It is still there, and every citation to it still resolves** — at the commit that froze it:

| | |
|---|---|
| Last commit carrying `legacy/` | [`1b7583649`](https://github.com/rchain-community/rchain-rust/tree/1b7583649/legacy) |
| Read a file | `https://github.com/rchain-community/rchain-rust/blob/1b7583649/legacy/<path>` |
| Restore it locally | `git checkout 1b7583649 -- legacy` |

That is the point of recording the revision rather than just deleting the tree: this repository cites
the Scala in around 150 places, as the port's reference implementation, and a citation that cannot be
followed is not a citation. Every `legacy/...` path in a code comment, in `spec/`, or in the book
resolves at `1b7583649` — including the ones written after the archive, which is why the revision is
here and not just in the commit that removed it.

## License

The Rust rewrite is licensed under the [GNU Affero General Public License, version 3](LICENSE)
(AGPL-3.0). The upstream Scala fork — archived out of the working tree on 2026-09-28 and readable at
[`1b7583649`](https://github.com/rchain-community/rchain-rust/tree/1b7583649/legacy) — retains its
original [Apache License 2.0](https://github.com/rchain-community/rchain-rust/blob/1b7583649/legacy/LICENSE.TXT).
