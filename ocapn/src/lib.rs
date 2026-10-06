#![forbid(unsafe_code)]
//! OCapN for RNode — the object-capability network (`https://ocapn.org/`), CapTP over a pluggable
//! netlayer.
//!
//! This is the transport that lets an Agoric vat hold a live reference to an RChain object and
//! invoke it, and vice versa. It is Layer 2 of the cross-shard design record
//! ([`docs/src/node/shard-invoke.md`](../../docs/src/node/shard-invoke.md)): the delegation layer
//! built over Layer 1's primitive, *a cross-shard call is a caller-signed deploy*.
//!
//! Built in stages (the design record is `docs/src/node/ocapn.md`; the OCapN implementation
//! guide numbers them):
//!
//! * **Stage 0** — the Syrup wire codec and the locators (`syrup`, `locator`, `peer`). ✓ this pass
//! * **Stage 0** — the netlayer trait, the `tcp-testing-only` netlayer, `op:start-session`.
//! * **Stage 1** — the import/export tables, `op:deliver`, the bootstrap object at position 0.
//! * **Stage 2–5** — promises/answers, `op:listen`, pipelining, GC.
//! * **Stage 6** — third-party handoffs.
//! * Then the bridge: `op:deliver` to a chain-backed export becomes a signed deploy.
//!
//! Nothing here is on-chain by itself: sessions, wire bytes, and answer bookkeeping are node-local
//! state. What reaches consensus is only the signed deploy the bridge produces.

pub mod bootstrap;
pub mod capacity;
pub mod captp;
pub mod conn;
pub mod dial_policy;
pub mod enliven;
pub mod fixtures;
pub mod handoff;
pub mod locator;
pub mod multi;
pub mod netlayer;
pub mod netstring;
pub mod owner;
pub mod par_value;
pub mod peer;
pub mod proxy;
pub mod session;
pub mod session_id;
pub mod syrup;
pub mod tcp_testing_only;
