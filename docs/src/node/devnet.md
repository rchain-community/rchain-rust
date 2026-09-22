# Local devnet (Docker)

A local Docker **testnet** for deploying and testing rholang smart contracts, driven by the
`tools/devnet.sh` script. It brings up 1–3 bonded validators (full consensus, autopropose) plus optional
unbonded observers, seeds genesis with a funded deployer wallet, and exposes `deploy`/`query` helpers.

The same script's `up --nodes N` mode is the bare 1–5 node *network-topology* harness (no autopropose,
no deployer wallet, no deploy helpers) — see [Operating the node](operating.md).

> **Security.** The validator and deployer keys baked into the script are throwaway dev keys for a
> *local* testnet only. Never reuse them for anything with real value.

## Prereqs

- `docker`
- `openssl` (to read the bootstrap node-id from its generated TLS certificate)

## Commands

```text
tools/devnet.sh build                        build the rnode:local image
tools/devnet.sh up --validators N [--observers M]
                                             start N validators (default 1) + M observers (default 0)
tools/devnet.sh deploy <contract.rho>        signed deploy to the bootstrap (file lives in examples/)
tools/devnet.sh eval <file.rho>              thin-client REPL eval of a file on the bootstrap
tools/devnet.sh query <name>                 listen for data at a public name
tools/devnet.sh propose                      force the bootstrap to propose a block
tools/devnet.sh status                       docker ps for the devnet
tools/devnet.sh logs <node>                  tail a node's logs
tools/devnet.sh diagnose                     per-node health check (PASS/FAIL)
tools/devnet.sh down [-v]                    stop the devnet (+ drop volumes)
```

**Autopropose and friends are `up` flags, not build flags** — set them per run:

```text
tools/devnet.sh up --validators 1                    # autopropose ON (default)
tools/devnet.sh up --validators 1 --no-autopropose   # autopropose OFF — blocks only via `propose`
tools/devnet.sh up --validators 1 --no-propose-on-deploy
                                                     # deploy no longer auto-proposes
```

`--admin` (default on) publishes the admin HTTP port `40405`, which exposes only `POST /api/v1/propose`
(*force a block*) — deploys always go through the public `40403` surface or the deploy gRPC, never
`40405`. Run `tools/devnet.sh help` for the full matrix (autopropose / propose-on-deploy / admin /
deployer-key / nodes).

## Worked example

```bash
# 1. Build the image.
tools/devnet.sh build

# 2. Start a single validator (creates genesis and autoproposes).
tools/devnet.sh up --validators 1

# 3. Deploy a contract (examples/hello.rho sends "world" on the public name "hello").
tools/devnet.sh deploy hello.rho

# 4. The bootstrap autoproposes a block containing the deploy; then query the result.
tools/devnet.sh query hello          # -> "world"

# 5. Or a 3-validator + 1-observer network:
tools/devnet.sh up --validators 3 --observers 1
tools/devnet.sh deploy hello.rho
tools/devnet.sh query hello

# 6. Tear down.
tools/devnet.sh down -v
```

## Node roles

- **Validators (1–3)** — each has its own keypair, is bonded in the shared `bonds.txt`, and runs
  `--autopropose`. Validator 0 (`devnet-bootstrap`) is the genesis ceremony-master (`--standalone`);
  validators 1–2 bootstrap from it.
- **Observers (0–3)** — unbonded, no autopropose; they bootstrap and replicate the chain.

Genesis is written to a temp dir and mounted read-only into the bootstrap: `bonds.txt` (one
`<pubkey> <stake>` line per validator) and `wallets.txt` (a funded deployer vault so deploys can pay
phlo). Contracts in `examples/` are mounted read-only at `/contracts` inside every node.

Blocks are produced **on their own**: every node runs with `--dev-mode --deployer-private-key`, so
`--autopropose` injects a signed `Nil` dummy deploy whenever the pool is empty and keeps proposing.
`up` blocks until `latestBlockNumber` is advancing.

## Casper fidelity & gaps

The devnet runs the real CBC-Casper data path — genesis bonding, deploy-driven proposals, a
cross-justified block-DAG, the monotone fringe estimator, and `> 2/3`-stake finality — with these
inputs simplified (theory in [Consensus (Casper)](consensus.md)):

- **Equal stake ⇒ unanimous finality.** The validators have equal stake `100` each (total `300`), so
  the strict `> 2/3` threshold requires *all* validators to attest before a block finalizes (2 of 3 is
  exactly 2/3, not a supermajority). Production's uneven stakes let a proper subset reach `> 2/3`.
- **Dummy `Nil` deploys** stand in for real user traffic, and **autopropose is a tight loop** (no
  backoff), so the block rate is unbounded rather than a production cadence.
- **Single shard** (`root`); **no Byzantine behavior** (all validators honest — the slash path exists
  but is never exercised); **no partitions/latency** (local Docker bridge); **throwaway keys** with no
  economic security; a small fixed validator set (≤3).

## Genesis content and what a consumer can hardcode

A fresh chain's genesis installs the interpreted contracts a client reaches through
`rho:registry:lookup`, and seeds the shorthand aliases so those lookups resolve — `rho:rchain:revVault`
and `rho:rchain:pos` (native channels), `rho:rchain:makeMint`, `rho:lang:listOps` and
`rho:lang:nonNegativeNumber`. [`spec/GENESIS.md`](../../../spec/GENESIS.md) is the manifest: each
entry with the consumer it unblocks, its `rho:id`, and what is deliberately *not* installed.

For a client:

- A seeded lookup answers **`(9223372036854775807, bundle+{dispatcher})`** — the signed-registration
  shape, destructured `for (@(_, X) <- ch)` (or `for (@(_, *X) <- ch)`), after which `X` is callable.
  A `Nil` reply is what an *unseeded* shorthand looks like; it is silent, since an unmatched `for` is
  not an error.
- The `rho:id`s of the blessed contracts are **constants of this port** (they derive from fixed
  deployer keys) and are listed in the manifest, so they are safe to hardcode. They are not the
  54-char mainnet ids.
- `down -v && up` **regenerates genesis**, which produces a new genesis *address* and invalidates any
  URI recorded from a *deployed* bootstrap (e.g. a governance master URI obtained by deploying the
  contract set at runtime). With the blessed set installed at genesis, that no longer applies to the
  node's own contracts — but anything a deploy registers is still per-chain, because
  `rho:registry:insertArbitrary` derives its URI from a random seed.

## Governance on a devnet (the rgov set)

A fresh chain from `tools/devnet.sh up` already has the rgov governance set installed — no bootstrap
script, no per-chain URI to record. `spec/GENESIS.md` is the manifest; the constants a client
hardcodes are there, and the one a governance client needs first is the master directory's read cap
(`ReadcapURI`, what the wallet's `master-uri.ts` holds).

What is installed, and by whom:

- the eight classes (`Kudos`, `Inbox`, `Directory`, `roll`, `Issue`, `Ballot`, `Chat`, `Group`) with
  upstream's registration shape, each published under a chosen constant key;
- the master directory (upstream's seven slots, rendered with those constants) plus the three slots
  upstream omits (`Chat`, `Ballot`, `Group`);
- the `GetMe`/`SendThem` feature — the first call a governance client makes.

The last three are deployed by **one fixed dummy key** (`rgov::testnet_governance_key()`), because the
template publishes its admin capability for its own deployer and the feature's registration reads it
back. That is a testnet arrangement: see "Testnet vs mainnet" in `spec/GENESIS.md` before reusing any
of it on a public network.

Two foot-guns worth knowing when you exercise this by hand:

- **Pass the feature a *drain* as its log channel, never `rho:io:stdout`.** It logs multi-element
  lines (`["getMe", you, …]`) and stdout takes one datum, so a stdout log channel makes the contract
  error mid-flow — which looks exactly like the stall you are trying to diagnose.
- **`rho:rchain:deployerId` must be a real public key** for anything that derives a REV address from
  it (`getMe` does). A placeholder byte array makes `RevAddress!("fromPublicKey", …)` match nothing
  and the call stalls.

A hand probe of the handshake (deploy it, then read the node's own log for the stages — the feature's
contract logs are where a stall is legible):

```rholang
new rl(`rho:registry:lookup`), deployerId(`rho:rchain:deployerId`), out(`rho:io:stdout`),
    capCh, getMeCh, stuffCh in {
  out!("stage:1 lookup sent") |
  rl!(`<ReadcapURI from spec/GENESIS.md>`, *capCh) |
  for (MCAread <- capCh) {
    out!("stage:2 readcap resolved") |
    MCAread!("GetMe", *getMeCh) |
    for (GetMe <- getMeCh) {
      out!("stage:3 directory answered GetMe") |
      new logCh in {
        for (@_line <= logCh) { Nil } |          // a *repeated* drain, not stdout
        GetMe!(*deployerId, *stuffCh, *logCh) |
        for (@_reply <- stuffCh) { out!("stage:4 getMe answered") }
      }
    }
  }
}
```

Current state: stages 1–3 pass on a fresh devnet and `getMe` runs, then it stops inside the feature's
own `createMe` before answering — see the open item in `spec/GENESIS.md`.

## The consumer's call order

From the wallet's integration suite (which drives these contracts): `newInbox` **first** — it creates
`@[*deployerId, "inbox"]` and `@[*deployerId, "dictionary"]`, and eighteen other snippets read one of
those two lockers. Then, per family: `newChat` → `sendChat`/`readChat`; `newBallot` → `castBallot`;
`newIssue` → `addVoterToIssue`/`castVote`/`displayVote`/`delegateVote`/`tallyVotes`; `newGroup` →
`joinGroup`/`addMember`. `newMemberDirectory` needs the `MasterContractAdmin` locker, which only the
key that deployed the master directory has — the dummy key here, or a client's own deploy on a public
network. Reads (`getRoll`, `peekKudos`, `checkRegistration`, `checkBalance`) go anywhere after their
inputs exist.

## Ports


Deploy is served on gRPC `40401` and Propose+Repl on `40402`; the helpers run the Rust `rnode` client
*inside* a node container (`docker exec`) so they reach both via `localhost`. The public HTTP API
(in-container `40403`) and the admin HTTP API (in-container `40405`) are also published to the host,
so a browser can reach them directly.

The host maps each node's deploy gRPC port to `40402 + 1000·i`, its public HTTP port to
`40403 + 1000·i`, and its admin HTTP port to `40405 + 1000·i` — the bootstrap is `i = 0`, so it
publishes `40402`/`40403`/`40405` directly.
