# OCapN interoperability

[OCapN](https://ocapn.org/) (the Object Capability Network, *CapTP*) is the capability transport
Agoric's stack speaks. It lets a foreign peer — an Agoric vat, an `@endo/ocapn` client, any
implementation of the protocol — open a session to a node and hold **live references** to objects on
it. A reference to a Rholang capability is a real remote object: the peer calls it, and the call is
answered from the chain.

This is a *peer* protocol, not a client API. A peer does not post deploys and poll; it holds a
capability and invokes it. What makes that useful here is that the capability can name anything on the
chain — an ERTP issuer ([ERTP](ertp.md)), the REV vault, a contract of your own.

The crate is `ocapn/` (package `rchain-ocapn`). Sessions, wire bytes and answer bookkeeping are
node-local; the only thing that reaches consensus is the signed deploy the bridge produces.

## Turning it on

The listener is **off unless `api-server.ocapn-listen` names an address**.

| Config key | CLI flag | Meaning |
|---|---|---|
| `api-server.ocapn-listen` | `--ocapn-listen` | `host:port` to bind. Unset: no listener. |
| `api-server.ocapn-deny-local-dial` | — | Refuse to dial loopback and private addresses a peer names. Off by default. |

```hocon
api-server {
  ocapn-listen = "127.0.0.1:22045"
}
```

The netlayers implemented are the OCapN project's `tcp-testing-only` — plain TCP, **no encryption and
no authentication**, which the project's own README flags "HIGHLY INSECURE — DO NOT USE IN
PRODUCTION" — and `unix`, a Unix domain socket whose authentication is the socket's file mode
(`0600`). Either or both may be bound, on separate keys:

```hocon
api-server {
  ocapn-listen = "127.0.0.1:22045"            # tcp-testing-only
  ocapn-listen-unix = "/run/rnode/ocapn.sock" # unix
}
```

A node may bind **neither**, in which case it is **dial-only**: it starts no listener but can still be
driven to dial out. **Noise is not implemented** — until a reference implementation speaks it, a Noise
netlayer here would talk only to itself.

**A peer that completes a handshake can do two things, and both are the node's own authority:**

- **Make the node submit a signed deploy.** Every call to a chain-backed capability becomes a deploy
  signed by the node's deployer key. It spends the node's REV, and the chain records the *node* as the
  caller. A peer can therefore reach every arm that trusts `rho:rchain:deployerId`, with the node's
  identity.
- **Make the node dial an address of the peer's choosing.** A sturdyref and a handoff give both carry
  a peer-chosen location, and the node connects to it.

Bind loopback unless the interop genuinely needs otherwise. `ocapn-deny-local-dial = true` closes the
second one for a listener whose peers are not on the same host; link-local (`169.254.0.0/16`,
`fe80::/10`) and the unspecified address are refused whatever this is set to, and a hostname is
resolved and judged by what it resolves to before any connection is attempted.

## What a peer can reach

A peer dials, the two sides handshake, and the peer is given the node's **bootstrap object** at export
0. `fetch(swiss)` on it resolves a swiss number to a capability. The node publishes:

| Swiss number | Object |
|---|---|
| `rho:rchain:revVault/getBalance` | the REV vault balance capability, read from the deployer's own address |
| `rho:rchain:ertp` | the ERTP object API — `makeIssuerKit` and `getRevIssuer` |
| the OCapN conformance fixtures | the test suite's objects: echo, the car factory, the promise resolver, the greeter, the sturdyref enlivener |

The first two are published **only when the node has a deployer key to sign with**
(`dev.deployer-private-key`). Without one the bridge cannot reach the chain, and the bootstrap carries
the fixtures alone.

## The bridge

A delivery to a chain-backed capability becomes a **deploy**. The node builds a Rholang term that looks
the capability up by its registry name and calls the method the delivery named, signs it with the
node's key, submits it, and answers the CapTP promise with whatever the call wrote to the deploy's
reply channel.

Four consequences a client feels:

- **Every call costs a block.** The reply arrives when a block carrying the deploy is produced. On a
  node with autopropose off, nothing answers until something proposes.
- **Capabilities cross as descriptors, never as data.** A reply that *returns* an object — a brand, a
  mint, an issuer — arrives at the peer as a CapTP import it can call. The underlying Rholang name
  never leaves the node. A capability can also be passed *as an argument* to an arm that takes one.
- **The node is the caller on chain.** The deploy is signed by the node's deployer key, so the node's
  vault pays the phlo and the chain records the node as `deployerId` for every peer. There is no
  binding between a CapTP session and a deployer key, so the chain cannot tell one peer from another.
- **Bridged deploys are rate-limited** to 4 per second, across every session and capability. Each one
  spends the node's REV, so the bound is on the node's spend.

## Interop notes

These are the places where the OCapN prose and the reference implementations disagree, and where the
port had to choose. They matter to anyone integrating a new peer.

- **`op:start-session` carries four fields**, not the five the CapTP draft lists: the draft names a
  `crypto-version` that appears nowhere on the wire. The port follows the reference implementations.
- **Messages are netstring-framed.** The netlayer is described as "pure Syrup, no length prefix", but
  every implementation tested — the Python suite and `@endo/ocapn` — wraps each message as
  `<length>:<payload>`. Bare Syrup interoperates with nothing.
- **The swiss number is a byte array to the suite and a string to Endo.** The two reference
  implementations disagree, so the bootstrap accepts either and keys its directory by bytes.
- **A tuple crosses Syrup as a list.** A Syrup record is *labelled*, and a Rholang tuple has no label,
  so `(true, 0)` arrives at a peer as `[true, 0]`. It does not convert back: a list from the peer does
  not match a contract's `(brand, value)` tuple pattern, which is what keeps the ERTP arms that take an
  amount out of reach over OCapN (see [ERTP](ertp.md)).
- **Struct members are ordered by their encoded key bytes**, not by the key string — the reference's
  own sort. The session signature covers a struct, so this is load-bearing.
- The session Public Identifier is two SHA-256 rounds over the session public key; the Session ID is
  `SHA256(SHA256("prot0" ‖ sorted(PI_a, PI_b)))`. Session keys are **Ed25519**, ephemeral and
  off-chain; Ed25519 stays disabled as an on-chain signature algorithm.

## The code

| Module | What it holds |
|---|---|
| `syrup.rs`, `netstring.rs` | the Syrup codec and the length-prefixed framing |
| `locator.rs`, `peer.rs` | the URI and in-band locator forms |
| `session.rs`, `session_id.rs` | `op:start-session`, Public Identifier, Session ID, `op:abort` |
| `netlayer.rs`, `tcp_testing_only.rs`, `unix.rs` | the netlayer trait and the two transports |
| `multi.rs` | the dialing dispatcher: a locator's transport name picks the layer |
| `captp.rs`, `conn.rs` | the import/export and answer tables, `op:deliver`, `op:listen`, GC |
| `bootstrap.rs`, `fixtures.rs` | the bootstrap object and the conformance fixtures |
| `owner.rs`, `proxy.rs` | session ownership (a handle and the loop that owns the socket) and cross-session forwarding |
| `handoff.rs`, `enliven.rs` | third-party handoffs and the sturdyref enlivener that dials out |
| `capacity.rs`, `dial_policy.rs` | the peer-facing tables' caps and the dial policy |
| `par_value.rs` | the Rholang `Par` ↔ Syrup translation |

`node/src/api/ocapn.rs` is the node's end: the listener, the chain-backed capability, and the bridge.
`casper/src/shard_invoke.rs` builds the deploy terms.

The wire encodings are pinned by known-answer tests in `ocapn/tests/reference_vectors.rs` against
vectors produced by the reference suite's own encoder, because a codec that round-trips but orders
itself differently from a peer passes every local test and fails every handshake.

## Limits

- **The transports are `tcp-testing-only` and `unix`.** `tcp-testing-only` is the OCapN project's own
  test transport, unauthenticated by design; `unix` authenticates by the socket's file mode. **Noise is
  not built** (it would talk only to itself until a reference speaks it), and a production netlayer
  (Tor, libp2p, IBC) implements the same two-function trait; nothing above it changes.
- **A bridged call's reply is written to the permanent registry**, because a Rholang value returned to
  a peer has no source literal and must be registered to be reachable. Nothing deletes those entries.
- **The node cannot yet name the peer on chain.** Until a session is bound to a deployer key, binding
  the listener publishes the node's own authority, not identified callers.

## See also

- [Talking to a node from another implementation](../developer/ocapn.md) — the client-side how-to.
- [ERTP](ertp.md) — the object API most peers want to reach.
- [Cross-shard invoke](shard-invoke.md) — the caller-signed deploy the bridge is built on.
