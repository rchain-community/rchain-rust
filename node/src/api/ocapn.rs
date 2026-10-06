//! The node's OCapN listener (issue #249).
//!
//! OCapN is the object-capability network an Agoric vat speaks: a peer dials, the two sides
//! handshake over a netlayer, and the peer works against objects the node exports. This module is
//! the node's end of that — a listener that serves a fresh session per connection — and the
//! **bridge**: a chain-backed capability whose deliveries become signed deploys.
//!
//! **A node may listen on either of two transports, both, or (dial-only) neither.**
//! `tcp-testing-only` is the OCapN project's own, which its README flags as "HIGHLY INSECURE — DO NOT
//! USE IN PRODUCTION": plain TCP, no encryption, no authentication, so it is off unless
//! `api-server.ocapn-listen` names an address and a node reachable from anywhere it does not control
//! should leave it unset. `unix` (`api-server.ocapn-listen-unix`) takes a socket path and
//! authenticates by the socket's file mode — what admitted a peer is the filesystem's permission,
//! which is what makes it more than a testing transport. **A Noise netlayer is deliberately not
//! offered**: it is not built, because it would talk only to itself (see `ocapn/src/netlayer.rs`).
//!
//! **The bridge reuses Layer 1 rather than re-implementing it.** `docs/src/node/shard-invoke.md`
//! established that a cross-shard call *is* a caller-signed deploy whose reply arrives on
//! `` `rho:rchain:deployId` ``; a CapTP delivery is the same object with a different caller, so this
//! builds the term with [`invoke_term`], signs it with [`signed_invoke`], submits it through
//! `BlockApi::deploy` exactly as the faucet does, and reads the reply with [`reply_outcome`].
//!
//! **The reply path is the whole round trip.** A delivery is built into a term, signed, submitted,
//! and the value the deployed contract writes to the deploy's reply channel comes back as the CapTP
//! promise's fulfilment: `node/tests/ocapn_listener.rs` reads a REV balance out of a block that way.
//! **The reply channel being *bound* rather than written as a bare URI is load-bearing** (AUDIT
//! C218): a backticked `` `rho:rchain:deployId` `` is an ordinary, guessable name that nothing
//! reads, while the unforgeable per-deploy channel is what a `deployId(`rho:rchain:deployId`)`
//! binding introduces. Writing the URI instead sent every reply into a channel nobody watched and
//! the deploy still reported success.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use rchain_casper::api::block_api::BlockApi;
use rchain_casper::shard_invoke::Arg;
use rchain_casper::shard_invoke::{
    invoke_member_term, invoke_term, reply_outcome, signed_invoke, ShardOutcome,
};
use rchain_crypto::private_key::PrivateKey;
use rchain_models::casper::protocol::casper_message::SignedDeployData;
use rchain_models::rholang::RhoType::RhoString;
use rchain_ocapn::captp::{
    Desc, EXPORT_LABEL, IMPORT_OBJECT_LABEL as EXPORT_IMPORT_OBJECT_LABEL,
    IMPORT_PROMISE_LABEL as EXPORT_IMPORT_PROMISE_LABEL,
};
use rchain_ocapn::conn::{Act, Export, ExportView, Identity, Named};
use rchain_ocapn::dial_policy::{DialPolicy, PolicyNetlayer};
use rchain_ocapn::fixtures;
use rchain_ocapn::locator::PeerLocator;
use rchain_ocapn::multi::MultiNetlayer;
use rchain_ocapn::netlayer::{NetConn, Netlayer};
use rchain_ocapn::par_value;
use rchain_ocapn::syrup::Value;
use rchain_ocapn::tcp_testing_only::TcpTestingOnly;
use rchain_ocapn::unix::UnixNetlayer;
use rchain_rholang::pretty_printer::PrettyPrinter;
use rchain_shared::base16;
use tokio::sync::watch;

use crate::runtime::shutdown::stop_requested;

/// The swiss number of the chain-backed capability.
pub const REV_VAULT_BALANCE_SWISS: &[u8] = b"rho:rchain:revVault/getBalance";

/// The swiss number of the **ERTP object API** — the contract's own registry shorthand, looked up on
/// chain. Unlike the REV balance capability this one takes **the method from the message**, because
/// it is an object with arms (`makeIssuerKit`, `getRevIssuer`) and what it *returns* are capabilities
/// (a brand, a mint, an issuer), which is what issue #249's clause 4 asks a peer to be able to hold.
pub const ERTP_SWISS: &[u8] = b"rho:rchain:ertp";

/// The ERTP contract as a chain capability: every delivery names its own method, and a reply that
/// holds capabilities comes back as descriptors.
pub fn ertp_capability(
    block_api: Arc<dyn BlockApi>,
    key: PrivateKey,
    shard_id: String,
    limiter: Arc<rchain_shared::rate_limiter::RateLimiter>,
) -> ChainCapability {
    ChainCapability {
        block_api,
        key,
        shard_id,
        target_uri: "rho:rchain:ertp".to_string(),
        pattern: None,
        method: None,
        limiter,
        reply_timeout: CHAIN_REPLY_TIMEOUT,
        reply_interval: CHAIN_REPLY_INTERVAL,
    }
}

/// How many bridged deploys the node will submit per second, **across every capability and session**.
///
/// Each one is a signed deploy paid for out of the node's own REV, so the quantity being bounded is
/// the node's spend, and the honest calibration is the chain's own cadence rather than a round number:
/// a bridged call cannot be included faster than a block, so a rate near the block interval is the
/// most that can *matter*, and a small burst above it keeps an interactive peer (a wallet making a few
/// calls) unaffected while a loop is throttled to a known ceiling (HAZOP row A3).
///
/// **Not a per-peer bound.** `Export::deliver` is handed the arguments, not the session they arrived
/// on, so a chain capability cannot tell one peer from another; the fairness refinement — a limiter
/// keyed by peer, so one peer cannot spend the whole allowance — needs the caller's identity threaded
/// to the capability, which is the same work as binding a session to a deployer key.
pub const BRIDGED_DEPLOYMENTS_PER_SEC: u64 = 4;

/// Phlo for a bridged deploy — the same budget the faucet and the 2PC coordinator use.
const CHAIN_PHLO_LIMIT: i64 = 1_000_000;
const CHAIN_PHLO_PRICE: i64 = 1;
/// How long a delivery waits for its deploy's reply, and how often it looks.
const CHAIN_REPLY_TIMEOUT: Duration = Duration::from_secs(30);
const CHAIN_REPLY_INTERVAL: Duration = Duration::from_millis(250);
/// How far back to look for the reply datum, in blocks. Matches the socket-level callers.
const CHAIN_REPLY_DEPTH: i32 = 50;

/// How many OCapN sessions the node serves at once.
///
/// Every session is a task, a socket, and its own export/answer tables, and the tables' size is the
/// peer's to choose, so the ceiling is what makes the per-session bounds add up to a node bound. 64 is
/// ~10× the conformance suite's heaviest case (it opens about six, serially) and far below any file
/// descriptor ceiling; the parameter that matters for an operator is that it is *finite*.
const MAX_SESSIONS: usize = 64;

/// Where the node listens for OCapN peers (issue #249).
///
/// **A fixed struct rather than a list, because the accept loop's `select!` is fixed-arity**: each
/// configured transport gets an arm, and one that is not configured gets an arm over a
/// never-completing future rather than a conditionally-built one. A node with **no** transport
/// configured is not an error — it is a **dial-only** node, and the task still runs so the surfaces
/// that dial out have somewhere to live.
///
/// **`noise` is deliberately absent.** The Noise netlayer is not built (see `ocapn/src/netlayer.rs`
/// for the gate that would unblock it), so a key naming it would configure a transport this node
/// cannot construct.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OcapnListeners {
    /// `host:port` for the `tcp-testing-only` transport.
    pub tcp: Option<String>,
    /// A socket path for the `unix` transport, whose authentication is the socket's file mode.
    pub unix: Option<String>,
}

impl OcapnListeners {
    /// Whether the node listens on any transport. A node that listens on none is dial-only.
    pub fn any(&self) -> bool {
        self.tcp.is_some() || self.unix.is_some()
    }
}

/// The node's **outbound** OCapN surface (issue #249): what a dial this node starts itself needs.
///
/// Built by [`serve_ocapn`] once its listeners are bound — a transport that can dial is one that has
/// bound a layer — and shared with the admin route that starts a dial, so a session the node opens
/// lands in the **same** [`rchain_ocapn::owner::SessionRegistry`] the listener consults, which the
/// crossed-hello rule requires: two registries would not see each other's sessions.
#[derive(Clone)]
pub struct OcapnDialer {
    /// The dispatcher over the node's transports: a locator naming `unix` reaches the unix layer.
    netlayer: Arc<dyn Netlayer>,
    /// The live-session registry, shared with the listener.
    registry: Arc<rchain_ocapn::owner::SessionRegistry>,
    /// The location this node advertises in a dial's `op:start-session` — one of the transports it
    /// listens on, so the peer can dial back.
    location: PeerLocator,
}

impl OcapnDialer {
    /// Build a dialer over an already-bound layer. [`serve_ocapn`] publishes one through an
    /// [`OcapnDialSlot`]; this is for a caller that has its own.
    pub fn new(
        netlayer: Arc<dyn Netlayer>,
        registry: Arc<rchain_ocapn::owner::SessionRegistry>,
        location: PeerLocator,
    ) -> OcapnDialer {
        OcapnDialer {
            netlayer,
            registry,
            location,
        }
    }

    /// Dial the peer `peer` names (reusing a live session if there is one), fetch the object at
    /// `swiss`, and return the session that owns it, how to address it there, and the raw value.
    ///
    /// **The origin is `None` and the target policy is the only guard.** This dial is the node's own,
    /// not a peer's request, so there is no peer origin for Law 62's rule to judge; what decides
    /// whether it may run is `ocapn-deny-local-dial` (applied by the layer's policy) and the route's
    /// own `enable-ocapn-dial` gate.
    pub async fn dial_and_fetch(
        &self,
        peer: &PeerLocator,
        swiss: &[u8],
    ) -> Result<(rchain_ocapn::owner::SessionHandle, Desc, Value), String> {
        // An empty slot, so `session_origin` is `None` — see the doc above. The dial is deferred and
        // its loop runs in its own task, so the fetch below does not block on the handshake.
        let enlivener = rchain_ocapn::enliven::Enlivener::new(
            self.netlayer.clone(),
            self.location.clone(),
            self.registry.clone(),
            rchain_ocapn::owner::session_slot(),
        );
        enlivener.dial_and_fetch(peer, swiss).await
    }
}

/// Where the listener publishes the [`OcapnDialer`] it built, for the admin route to read.
///
/// A slot rather than a value threaded through the builder: the dialer wraps the netlayers the
/// listener **binds**, and binding is the listener's own first step — so a bad address is still
/// reported by the listener task, as it was, and the route sees "not ready" rather than dialing with
/// no transport.
pub type OcapnDialSlot = Arc<std::sync::OnceLock<OcapnDialer>>;

/// One transport the node listens on: the policy-wrapped netlayer, and the location a connection it
/// accepts advertises. `None` for a transport the node does not listen on.
///
/// The location is carried **per listener** because it is the honest answer only for the transport
/// that accepted: `owner::peer_key` is `(designator, transport)`, so a session dialled over unix and
/// the same peer dialled back over TCP are two peers, and a connection advertising the wrong
/// transport would advertise a location no peer can use.
struct Listener {
    inner: Option<(Arc<dyn Netlayer>, PeerLocator)>,
}

impl Listener {
    fn new(inner: Option<(Arc<dyn Netlayer>, PeerLocator)>) -> Listener {
        Listener { inner }
    }

    /// Accept one connection and the location to advertise for it — or never, for a transport the
    /// node does not listen on. Never completing keeps the accept loop's arm count fixed without a
    /// conditionally-built future.
    async fn accept(&self) -> std::io::Result<(Box<dyn NetConn>, PeerLocator)> {
        match &self.inner {
            Some((layer, location)) => {
                Ok((layer.accept_incoming_connection().await?, location.clone()))
            }
            None => std::future::pending().await,
        }
    }
}

/// Bind the `tcp-testing-only` listener and build the location a peer reaches it at.
async fn listen_tcp(
    address: &str,
    policy: DialPolicy,
    designator: &str,
    chain: usize,
    log: &Arc<dyn rchain_shared::log::Log>,
    source: rchain_shared::log::LogSource,
) -> Result<(Arc<dyn Netlayer>, PeerLocator), String> {
    let bound = TcpTestingOnly::bind(address)
        .await
        .map_err(|e| e.to_string())?;
    // `local_addr` belongs to the concrete netlayer, not the trait: take it before the Arc.
    let local = bound.local_addr().map_err(|e| e.to_string())?;
    // **The line an operator needs first** (HAZOP row E6): this surface spends money and writes
    // consensus state with no local trace of its own, so "is it serving?" had no answer short of
    // reading chain state. `local` rather than the configured string, because `:0` is legal and the
    // chosen port is the only useful thing to print.
    log.info(
        source,
        &format!(
            "OCapN listener serving tcp-testing-only on {local} ({chain} chain-backed capability/ies)"
        ),
    );
    // **The designator is this node's, not a shared constant** (C224 item 2). Peer identity *is*
    // `(designator, transport)` (`owner::peer_key`), so with every node calling itself `"rnode"` two
    // nodes were one peer: a sturdyref to one resolved at the other, and the crossed-hello registry
    // conflated their sessions. Derived from the node's key (or its node id when there is no key) in
    // `node_designator`, so it is stable across restarts and distinct per node.
    let location = PeerLocator {
        designator: designator.to_string(),
        transport: "tcp-testing-only".to_string(),
        hints: BTreeMap::from([
            ("host".to_string(), local.ip().to_string()),
            ("port".to_string(), local.port().to_string()),
        ]),
    };
    Ok((Arc::new(PolicyNetlayer::new(bound, policy)), location))
}

/// Bind the `unix` listener and build the location a peer reaches it at.
///
/// The location carries a `path` hint and **no host**, which is what the dial policy reads as
/// "cannot be judged" — the right answer for a peer admitted by the socket's file mode rather than by
/// an address (`dial_policy`'s note).
async fn listen_unix(
    path: &str,
    policy: DialPolicy,
    designator: &str,
    chain: usize,
    log: &Arc<dyn rchain_shared::log::Log>,
    source: rchain_shared::log::LogSource,
) -> Result<(Arc<dyn Netlayer>, PeerLocator), String> {
    let bound = UnixNetlayer::bind(path).await.map_err(|e| e.to_string())?;
    let local = bound.local_path().display().to_string();
    log.info(
        source,
        &format!("OCapN listener serving unix on {local} ({chain} chain-backed capability/ies)"),
    );
    let location = PeerLocator {
        designator: designator.to_string(),
        transport: "unix".to_string(),
        hints: BTreeMap::from([("path".to_string(), local)]),
    };
    Ok((Arc::new(PolicyNetlayer::new(bound, policy)), location))
}

/// Serve OCapN on every configured transport until the node is asked to stop.
///
/// The task is spawned whether or not any transport is configured — a node that listens on none is
/// **dial-only**, and its surfaces still need a task to live in — so the node's listener set stays
/// uniform and its drain slot does not need a second optional arm.
///
/// `chain` is the chain-backed capabilities to publish on each session's bootstrap — a swiss number
/// and the object it names — empty when the node has no deployer key to sign with. They are shared
/// across sessions: they hold no per-session state.
pub async fn serve_ocapn(
    listeners: OcapnListeners,
    chain: Vec<(Vec<u8>, Arc<dyn Export>)>,
    designator: String,
    deny_local_dial: bool,
    dial_slot: OcapnDialSlot,
    log: Arc<dyn rchain_shared::log::Log>,
    stop: watch::Receiver<bool>,
) -> Result<(), String> {
    let source = rchain_shared::log::LogSource::new("coop.rchain.node.api.ocapn");
    // **The dial policy wraps each transport**, so every dial this surface makes — the enlivener's and
    // the greeter's, both to peer-named addresses — is measured before a connection exists (HAZOP row
    // B4). The policy is configuration, not a property of the test netlayer: the fixture peer wraps
    // nothing, because the conformance suite must be able to dial whatever it names.
    let policy = DialPolicy {
        deny_local: deny_local_dial,
        allow: Vec::new(),
    };
    // **Bind every configured listener before the accept loop**, so a bad address is an error the node
    // reports (this task returns `Err`, which `listener_stopped` names) rather than a listener that
    // dies quietly inside the loop.
    let tcp = match listeners.tcp.as_deref() {
        Some(address) => Some(
            listen_tcp(
                address,
                policy.clone(),
                &designator,
                chain.len(),
                &log,
                source,
            )
            .await?,
        ),
        None => None,
    };
    let unix = match listeners.unix.as_deref() {
        Some(path) => {
            Some(listen_unix(path, policy.clone(), &designator, chain.len(), &log, source).await?)
        }
        None => None,
    };
    // **The dialing netlayer is the dispatcher over every transport the node has**: a fixture that
    // dials `ocapn://peer.unix?path=…` reaches the unix layer and one that names `tcp-testing-only`
    // reaches the TCP layer. Accepting stays per listener below, because a session has to advertise
    // the transport it arrived on, which the dispatcher cannot say.
    let mut dialer = MultiNetlayer::new();
    if let Some((layer, _)) = &tcp {
        dialer = dialer.with("tcp-testing-only", layer.clone());
    }
    if let Some((layer, _)) = &unix {
        dialer = dialer.with("unix", layer.clone());
    }
    let dialer: Arc<dyn Netlayer> = Arc::new(dialer);
    // The location this node advertises in a dial it starts itself: the first configured transport in
    // a fixed order (tcp, then unix). A node listening on both still has one location to put in a
    // start-session, and it must be one a peer can dial back.
    let outward = tcp
        .as_ref()
        .or(unix.as_ref())
        .map(|(_, location)| location.clone());
    let tcp = Listener::new(tcp);
    let unix = Listener::new(unix);
    // One registry and one gift store for the node's whole OCapN surface: the crossed-hello rule
    // compares the sessions *this node* has with a peer, and a handoff is deposited on one session
    // and withdrawn on another.
    let registry = Arc::new(rchain_ocapn::owner::SessionRegistry::default());
    let handoffs = Arc::new(rchain_ocapn::handoff::Handoffs::default());
    // **Publish the dialer** for the admin route that starts a dial of the node's own. A node with no
    // transport does not publish one: there is no layer to dial with, and the route says so rather
    // than dialing into "this node speaks nothing".
    if let Some(location) = outward {
        let _ = dial_slot.set(OcapnDialer {
            netlayer: dialer.clone(),
            registry: registry.clone(),
            location,
        });
    }
    // **The accept loop is bounded, and the permit is taken before the task exists** (HAZOP row B1).
    // An unbounded accept loop let one peer hold 3 000 idle connections (+41 MB, RSS never returned)
    // and one task each; the handshake bound now stops a *silent* connection from holding its task,
    // and this stops there being arbitrarily many. Taken before the spawn, so a refused connection
    // costs nothing but the socket — which is dropped here.
    let sessions = Arc::new(tokio::sync::Semaphore::new(MAX_SESSIONS));

    loop {
        // **One arm per transport.** A transport the node does not listen on is `pending()`, so its
        // arm never fires — and with no transport at all the loop waits on the stop word alone, which
        // is what keeps a dial-only node's task alive.
        let (connection, location) = tokio::select! {
            // The operator's word, and a dropped coordinator, both land here.
            _ = stop_requested(stop.clone()) => return Ok(()),
            accepted = tcp.accept() => accepted.map_err(|e| e.to_string())?,
            accepted = unix.accept() => accepted.map_err(|e| e.to_string())?,
        };
        let Ok(permit) = sessions.clone().try_acquire_owned() else {
            // At the ceiling: close the socket rather than queue it. A peer that keeps connecting
            // gets refusals, not a growing backlog, and the operator's own sessions keep working.
            // Said out loud, at `warn`: a node whose sessions are all held is either under attack or
            // has a stuck peer, and the operator cannot tell either from silence.
            log.warn(
                source,
                &format!("at the session ceiling ({MAX_SESSIONS}); refusing a new connection"),
            );
            drop(connection);
            continue;
        };
        let chain = chain.clone();
        let netlayer = dialer.clone();
        let registry = registry.clone();
        let handoffs = handoffs.clone();
        let log = log.clone();
        tokio::spawn(async move {
            // Held for the session's life and released when this task ends: the task's own lifetime
            // *is* the session's, so the permit needs no name beyond this binding.
            let _permit = permit;
            // A fresh session key per session, as OCapN requires.
            let Ok(identity) = Identity::fresh(location.clone()) else {
                return;
            };
            // The per-session objects that dial find their session through this slot, which is
            // filled in once the session exists — `accept` needs the bootstrap before that.
            let slot = rchain_ocapn::owner::session_slot();
            // The fixtures are rebuilt per session (they hold per-session promise state); the
            // chain-backed capability is shared, because it does not.
            let mut bootstrap =
                fixtures::conformance_bootstrap_with(handoffs, slot.clone(), registry.clone());
            for (swiss, capability) in chain {
                bootstrap.publish(swiss, capability);
            }
            fixtures::publish_dialing_fixtures(
                &mut bootstrap,
                netlayer,
                location,
                registry.clone(),
                slot.clone(),
            );
            // **Book, then answer** — see `owner::accept_and_book`: a session has to be in the
            // registry before the peer can act on it, or a delivery that arrives on the first round
            // trip after our start-session (a handoff, a fetch) reaches an object that cannot find
            // the session it belongs to.
            let (handle, loop_, context, peer_location) =
                match rchain_ocapn::owner::accept_and_book(
                    connection,
                    &identity,
                    Arc::new(bootstrap),
                    &registry,
                )
                .await
                {
                    Ok(parts) => parts,
                    Err(reason) => {
                        // A refused handshake, and a session that lost its crossing, have both already
                        // been answered with `op:abort` where one was owed. Logged at **debug**, not
                        // warn: a crossing is a legitimate outcome of the protocol, and a peer-driven
                        // stream of them would make a warning meaningless (the log-flood the
                        // operations lens warned about).
                        log.debug(source, &format!("session not served: {reason}"));
                        return;
                    }
                };
            *slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(context);
            // One session per connection; its end is this task's end.
            let _ = loop_.run().await;
            registry.forget(&peer_location, &handle.own_pi, handle.dialed);
        });
    }
}

/// **One argument of a bridged call**: a value, or a capability the node can name on chain
/// (C224 item 4).
///
/// A peer holds one of our capabilities as a descriptor, and the descriptor it sends back for one of
/// *ours* is `<desc:export N>` — a position in **this session's** export table, which is why only the
/// session's view can resolve it. The object at N may be a `ChainCapability`, which knows its registry
/// location; or it may be one of the session's own fixtures (the bootstrap, the greeter, a sink), which
/// has no chain name at all.
///
/// **Every case that cannot be named gets its own reason**, because the one it used to get —
/// `par_value`'s "a Symbol — Rholang has no symbol for it to land in" — described the *label* rather
/// than the situation, and a peer reading it would look for a Symbol in the wrong place.
fn argument(
    value: &Value,
    session: Option<&dyn ExportView>,
    pp: &PrettyPrinter,
    counter: &mut usize,
) -> Result<Arg, String> {
    if let Some((label, position)) = rchain_ocapn::captp::descriptor_of(value) {
        return match label.as_str() {
            // The peer's own object, addressed by the position *it* exported at. This node cannot
            // name it on chain: a peer's object has no RChain URI, and proxying it would mean the
            // chain holding a reference across a session it cannot see.
            EXPORT_IMPORT_OBJECT_LABEL | EXPORT_IMPORT_PROMISE_LABEL => Err(format!(
                "argument <{label} {position}> is an object of the peer's (its export {position}); a \
                 chain contract can be handed only objects this node can name in the chain's \
                 registry, and a peer's object has no such name"
            )),
            EXPORT_LABEL => {
                let (location, pattern) = named_capability(value, session)?;
                Ok(Arg::Named { location, pattern })
            }
            other => Err(format!("argument <{other} {position}> is not a descriptor")),
        };
    }
    if !contains_descriptor(value) {
        return par_value::value_to_par(value)
            .map(Arg::Value)
            .map_err(|e| e.to_string());
    }
    // **A capability *inside* a value** — an amount's brand. It cannot be passed bare, so the term
    // binds it where it stands: the value is rendered with each capability replaced by a fresh name,
    // and `wrap_binders` looks each one up and matches its pattern around the call (AUDIT C226).
    let mut binders: Vec<(usize, String, Option<String>)> = Vec::new();
    let rendered = value_to_term(value, session, pp, counter, &mut binders)?;
    Ok(Arg::Nested { rendered, binders })
}

/// The registry location and binding pattern of the capability a `<desc:export N>` names in this
/// session's export table, or a refusal naming why it cannot be given to a chain contract.
fn named_capability(
    value: &Value,
    session: Option<&dyn ExportView>,
) -> Result<(String, Option<String>), String> {
    let (_, position) = rchain_ocapn::captp::descriptor_of(value)
        .ok_or_else(|| "not a capability descriptor".to_string())?;
    let session = session.ok_or_else(|| {
        "a capability argument arrived outside a session, so the export it names cannot be resolved"
            .to_string()
    })?;
    let object = session.exported(&position).ok_or_else(|| {
        format!("no export at position {position} in this session to pass as an argument")
    })?;
    match object.named() {
        Some(named) => Ok((named.location, named.pattern)),
        None => Err(format!(
            "the export at position {position} is a session-local object of this node's, not a \
             chain object; it has no registry name to give a contract"
        )),
    }
}

/// Render a peer's value into a rholang term, binding every capability inside it.
///
/// A leaf is printed by the same rule as any other value — `check_renderable` first (AUDIT C220),
/// because Rholang's literal grammar has no escapes and a `"` would end the literal early in a deploy
/// the node's own key signs. A container is rebuilt around its rendered parts, so a tuple stays a
/// tuple: an amount the peer sends must reach the contract's `@(brand, value)` pattern as one.
fn value_to_term(
    value: &Value,
    session: Option<&dyn ExportView>,
    pp: &PrettyPrinter,
    counter: &mut usize,
    binders: &mut Vec<(usize, String, Option<String>)>,
) -> Result<String, String> {
    if rchain_ocapn::captp::descriptor_of(value).is_some() {
        let (location, pattern) = named_capability(value, session)?;
        let index = *counter;
        *counter += 1;
        binders.push((index, location, pattern));
        return Ok(format!("arg{index}"));
    }
    let parts =
        |xs: &[Value], counter: &mut usize, binders: &mut _| -> Result<Vec<String>, String> {
            xs.iter()
                .map(|x| value_to_term(x, session, pp, counter, binders))
                .collect()
        };
    match value {
        Value::List(xs) => Ok(format!("[{}]", parts(xs, counter, binders)?.join(", "))),
        Value::Struct(entries) => {
            let mut fields = Vec::with_capacity(entries.len());
            for (key, v) in entries {
                fields.push(format!(
                    "{}: {}",
                    pp.build_string(&RhoString::apply(key.clone())),
                    value_to_term(v, session, pp, counter, binders)?
                ));
            }
            Ok(format!("{{{}}}", fields.join(", ")))
        }
        Value::Record(xs) => match xs.as_slice() {
            [Value::Symbol(label), Value::Symbol(tag), Value::List(fields)]
                if label == par_value::TAGGED_LABEL && tag == par_value::TUPLE_TAG =>
            {
                // A one-element tuple has no literal — `(x)` parses as `x` — so it is refused rather
                // than written as something the parser would read as the element.
                if fields.len() < 2 {
                    return Err(format!(
                        "a tuple of {} element has no rholang literal to be written as",
                        fields.len()
                    ));
                }
                Ok(format!("({})", parts(fields, counter, binders)?.join(", ")))
            }
            _ => Err(
                "a labelled record that is not the tagged tuple has no rholang term".to_string(),
            ),
        },
        leaf => {
            let par = par_value::value_to_par(leaf).map_err(|e| e.to_string())?;
            rchain_rholang::pretty_printer::check_renderable(&par)
                .map_err(|e| format!("the argument cannot be rendered into a term: {e}"))?;
            Ok(pp.build_string(&par))
        }
    }
}

/// Whether a descriptor is nested anywhere inside this value.
fn contains_descriptor(value: &Value) -> bool {
    match value {
        Value::Record(fields) | Value::List(fields) => {
            rchain_ocapn::captp::descriptor_of(value).is_some()
                || fields.iter().any(contains_descriptor)
        }
        Value::Struct(fields) => fields.values().any(contains_descriptor),
        _ => false,
    }
}

/// **This node's OCapN designator** (C224 item 2): the name it advertises in every session, and half
/// of the `(designator, transport)` pair that *is* a peer's identity (`owner::peer_key`).
///
/// It was the constant `"rnode"` for every node, which made two nodes one peer — a sturdyref to one
/// resolved at the other, and the crossed-hello registry conflated their sessions. Derived from the
/// deployer key when there is one (stable across restarts, unlike the ephemeral session key), and from
/// the node's own identifier when there is not, because a listener without a key still advertises a
/// location to serve its fixtures from.
///
/// Eight bytes of a blake2b256 in hex: long enough that two nodes do not collide, short enough to read
/// in a log line or a peer locator.
pub fn node_designator(
    deployer_key: Option<&rchain_crypto::private_key::PrivateKey>,
    node_id: &rchain_comm::peer_node::NodeIdentifier,
) -> String {
    let material: Vec<u8> = match deployer_key {
        Some(key) => match rchain_crypto::signatures::signatures_alg::SignaturesAlg::to_public(
            &rchain_crypto::signatures::secp256k1::Secp256k1,
            key,
        ) {
            Ok(public) => public.bytes().to_vec(),
            // **Not the key's own bytes.** A designator is advertised to every peer, so hashing the
            // *secret* here would publish a function of it. A key that cannot be made public is a key
            // this node cannot name itself by, and the node's identifier is the honest fallback.
            Err(_) => node_id.key().to_vec(),
        },
        None => node_id.key().to_vec(),
    };
    let digest = rchain_crypto::hash::blake2b256::hash(&material);
    format!("rnode-{}", rchain_shared::base16::encode(&digest[..8]))
}

/// Take one unit of the node's bridged-deploy budget, or refuse with a reason.
///
/// Extracted from `ChainCapability::deliver` so it is testable without a `BlockApi`: the check runs
/// *before* the capability touches the chain, which means the whole observable is "the limiter was
/// consulted and the refusal names the bound" — and a bound that cannot be tested is one that gets
/// removed. The same reason `admin_bind_host` was lifted out of its spawn (AUDIT C112).
fn check_deploy_budget(limiter: &rchain_shared::rate_limiter::RateLimiter) -> Result<(), String> {
    if limiter.allow() {
        return Ok(());
    }
    Err(format!(
        "the chain bridge will not submit more than {BRIDGED_DEPLOYMENTS_PER_SEC} deploys per second"
    ))
}

/// A chain-backed capability: a delivery to it becomes a caller-signed deploy, and the CapTP
/// promise resolves from the value that deploy put on its reply channel.
///
/// **The method rides the message**, as the first Syrup *Symbol* of the delivery — Endo's own calling
/// convention. This used to be a fixed `(target, method)` pair, justified in this comment by the
/// claim that "`getBrand`, `makeEmptyPurse` and the rest are separate capabilities, not tags on a
/// message" — which `docs/src/node/ertp.md`'s object model refutes: an issuer that answers exactly
/// one method is not an issuer a peer can hold. A *published* capability may still fix its method
/// (`revVault.getBalance`), because that shape is pinned by tests and by the conformance suite.
pub struct ChainCapability {
    block_api: Arc<dyn BlockApi>,
    /// The key that signs. Today it is the node's own dev deployer key, so a bridged deploy spends
    /// the node's REV and the far contract sees the *node* as the caller. Binding a CapTP session
    /// to a caller's secp256k1 identity is the identity work `docs/src/node/ocapn.md` lists as
    /// future; until then this is the node acting as itself.
    key: PrivateKey,
    shard_id: String,
    /// The registry URI the object is reachable at: a contract's own URI, or the URI a *bridge
    /// deploy* registered its reply under (see [`invoke_member_term`]).
    target_uri: String,
    /// How to bind the object **inside** the registered value: `None` for a symbolic urn, where the
    /// registered value *is* the object, and otherwise the pattern the reply's own shape produced —
    /// an ERTP kit's `(brand, mint, issuer)` is the issuer under `(_, _, member)`.
    pattern: Option<String>,
    /// A fixed method, for the capabilities this node *publishes* (`revVault.getBalance`). `None`
    /// means the method is the first `Symbol` of each delivery — which is what an ERTP object, an
    /// object with several arms, requires.
    method: Option<String>,
    /// **The node's whole OCapN surface shares one**, so the bound is on the node's spend rather than
    /// on one capability — see [`BRIDGED_DEPLOYMENTS_PER_SEC`] (HAZOP row A3).
    limiter: Arc<rchain_shared::rate_limiter::RateLimiter>,
    reply_timeout: Duration,
    reply_interval: Duration,
}

impl ChainCapability {
    /// The `revVault.getBalance` capability: one REV address in, one balance out.
    pub fn rev_vault_balance(
        block_api: Arc<dyn BlockApi>,
        key: PrivateKey,
        shard_id: String,
        limiter: Arc<rchain_shared::rate_limiter::RateLimiter>,
    ) -> ChainCapability {
        ChainCapability {
            block_api,
            key,
            shard_id,
            target_uri: "rho:rchain:revVault".to_string(),
            pattern: None,
            method: Some("getBalance".to_string()),
            limiter,
            reply_timeout: CHAIN_REPLY_TIMEOUT,
            reply_interval: CHAIN_REPLY_INTERVAL,
        }
    }

    /// Wait for the deploy's reply channel to carry a value.
    async fn await_reply(&self, deploy_id: &[u8]) -> Result<rchain_models::ast::Par, String> {
        let name = rchain_casper::shard_invoke::reply_channel(deploy_id);
        let deadline = Instant::now() + self.reply_timeout;
        loop {
            let (data, _length) = self
                .block_api
                .get_listening_name_data_response(CHAIN_REPLY_DEPTH, &name)
                .await?;
            if !data.is_empty() {
                return match reply_outcome(&data) {
                    ShardOutcome::Value(par) => Ok(par),
                    // A registry miss produces nothing on the channel; `reply_outcome` says so.
                    ShardOutcome::Error(reason) => Err(reason),
                };
            }
            if Instant::now() >= deadline {
                // Say *why* nothing arrived. `NotProcessed` means the deploy never reached a block;
                // `ProcessedWithError` means it ran and was refused — and a refused deploy is the
                // ordinary way a lookup or a guard fails, which the caller otherwise cannot tell
                // apart from a slow chain.
                let deploy_id_hex = base16::encode(deploy_id);
                let status = self.block_api.deploy_status(&deploy_id.to_vec()).await;
                return Err(format!(
                    "timed out waiting for deploy {deploy_id_hex} to reply ({status:?})"
                ));
            }
            tokio::time::sleep(self.reply_interval).await;
        }
    }
}

#[async_trait]
impl Export for ChainCapability {
    async fn deliver(&self, args: &[Value]) -> Result<Act, String> {
        self.call(None, args).await
    }

    /// The bridge's real entry point: `handle_deliver` calls this, and the view is what lets a
    /// descriptor the peer sent become a capability the chain can be handed (C224 item 4).
    async fn deliver_in(&self, session: &dyn ExportView, args: &[Value]) -> Result<Act, String> {
        self.call(Some(session), args).await
    }

    /// Where this capability lives on chain — the answer a capability *argument* needs, when the peer
    /// hands one of the node's own objects back to another.
    fn named(&self) -> Option<Named> {
        Some(Named {
            location: self.target_uri.clone(),
            pattern: self.pattern.clone(),
        })
    }
}

impl ChainCapability {
    async fn call(&self, session: Option<&dyn ExportView>, args: &[Value]) -> Result<Act, String> {
        // **The spend bound comes first, before any work.** Every delivery below becomes a signed
        // deploy paid for out of the node's own REV, so a peer that loops is stopped here rather than
        // by its own patience (HAZOP row A3). The peer sees a `break` naming the limit.
        check_deploy_budget(&self.limiter)?;
        // The arguments crossed CapTP as Syrup; Rholang is what the chain runs. A value with no
        // `Par` shape (a Symbol, a float) is refused here rather than guessed at.
        // **The method is the first `Symbol` of the delivery**, which is Endo's calling convention
        // and the only shape that can serve an ERTP object: an issuer has `getBrand`,
        // `makeEmptyPurse` and `getAmountOf`, so "one capability, one method" would be one capability
        // per arm — and a peer cannot hold *an issuer*. A published capability may still fix its
        // method (`revVault.getBalance`), because that shape is pinned by tests and by the suite.
        let (method, args) = match &self.method {
            Some(fixed) => (fixed.clone(), args),
            None => match args.split_first() {
                Some((Value::Symbol(m), rest)) => (m.clone(), rest),
                _ => {
                    return Err(
                        "a chain capability expects the method as the first Symbol".to_string()
                    )
                }
            },
        };
        let pp = PrettyPrinter::new();
        // A capability *inside* an argument takes a binder index of its own; the counter starts past
        // the argument positions so it cannot collide with `Named`, which uses the argument's index.
        let mut counter = args.len() + 1;
        let pars = args
            .iter()
            .map(|arg| argument(arg, session, &pp, &mut counter))
            .collect::<Result<Vec<_>, _>>()?;

        // A capability whose method rides the message is one that may *return* capabilities, and a
        // returned capability has no rholang source literal — so the deploy registers its reply in
        // the same evaluation that produced it (`invoke_member_term`), and answers the deploy's reply
        // channel with `(uri, value)`: the fresh registry URI, and the value whose capabilities the
        // node then addresses *through* that URI by path.
        //
        // The deploy is signed by the node's own key in **both** shapes. A fresh key per object would
        // give the object a URI of its own under `insertSigned`, but it has no REV, so its deploy
        // cannot pay for itself (`preCharge: insufficient funds`) — measured, first run of this test.
        // `insertArbitrary` mints the URI without a key, so the funded key can sign.
        let registering = self.method.is_none();
        // A value the peer delivered is checked before it is printed into the term (AUDIT C220):
        // Rholang's literal grammar has no escapes, so an argument containing a `"` would end the
        // literal early and be read as a process in a deploy signed by the node's own key. The check
        // refuses it here, and the peer gets a `break` naming the argument rather than a deploy it
        // chose the body of.
        let term = if registering {
            invoke_member_term(
                self.target_uri.as_str(),
                self.pattern.as_deref(),
                &method,
                &pars,
            )
        } else {
            invoke_term(self.target_uri.as_str(), &method, &pars)
        }?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        // A deploy expires once `latest_block_number - valid_after_block_number > DEPLOY_LIFESPAN`,
        // so it is anchored to the current height rather than to `-1` (the faucet's note).
        let vabn = self.block_api.status().await.latest_block_number;
        // **The signed deploy must be dropped before the first `await`.** `Signed` borrows a
        // `&dyn SignaturesAlg`, which is not `Sync`, so holding one across an await makes this
        // future non-`Send` — and `Export::deliver`'s future must be `Send`. Everything needed
        // afterwards is owned by the end of this block.
        let (data, deployer, sig) = {
            let signed = signed_invoke(
                &term,
                &self.key,
                timestamp,
                CHAIN_PHLO_LIMIT,
                CHAIN_PHLO_PRICE,
                vabn,
                &self.shard_id,
            )?;
            (signed.data, signed.pk.bytes().to_vec(), signed.sig.clone())
        };
        let deploy = SignedDeployData {
            data,
            deployer,
            sig: sig.clone(),
            sig_algorithm: "secp256k1".to_string(),
        };
        // `ApiErr<String>` is a plain `String` error (the socket-level callers join a list).
        self.block_api.deploy(&deploy).await?;

        let reply = self.await_reply(&sig).await?;
        if !registering {
            return Ok(Act::value(
                par_value::par_to_value(&reply).map_err(|e| e.to_string())?,
            ));
        }
        // `(uri, value)` — the URI this reply was registered under, and the value itself.
        let (uri, value) = split_registered_reply(&reply)?;
        // The value may *hold* capabilities. Each becomes an export of its own, addressed by the URI
        // the deploy just registered plus the pattern that binds it inside that value — so a peer that
        // asked for a kit gets a descriptor per member, and a peer that asked for a purse gets one.
        let mut leaves = Vec::new();
        capability_patterns(&value, &mut leaves);
        if leaves.is_empty() {
            return Ok(Act::value(
                par_value::par_to_value(&value).map_err(|e| e.to_string())?,
            ));
        }
        let objects: Vec<Arc<dyn Export>> = leaves
            .into_iter()
            .map(|pattern| {
                Arc::new(ChainCapability {
                    block_api: self.block_api.clone(),
                    key: self.key.clone(),
                    shard_id: self.shard_id.clone(),
                    target_uri: uri.clone(),
                    pattern: Some(pattern),
                    method: None,
                    // The child shares the node's limiter: a returned capability is the same node's
                    // spend, not a new budget.
                    limiter: self.limiter.clone(),
                    reply_timeout: self.reply_timeout,
                    reply_interval: self.reply_interval,
                }) as Arc<dyn Export>
            })
            .collect();
        // **One object answers as one descriptor, not as a one-element list.** The peer asked for a
        // purse or an issuer, and `E(obj).makeEmptyPurse()` should hand it a purse — a list of one
        // would make every caller destructure a collection to reach the only thing in it.
        //
        // Written as a split rather than a length check plus an `expect`, because `expect` in
        // production code is a hard violation of the type system's partiality gate — and because the
        // empty case is real enough to deserve a name: it cannot happen here (a leaf is what put the
        // pattern in the list), and if it ever did, a promise that names the reason and breaks is
        // better than a panicking node.
        let mut objects = objects.into_iter();
        let first = objects.next();
        let rest: Vec<Arc<dyn Export>> = objects.collect();
        match first {
            Some(only) if rest.is_empty() => Ok(Act::object(only)),
            Some(only) => Ok(Act::objects(
                std::iter::once(only).chain(rest).collect::<Vec<_>>(),
            )),
            None => Err("the reply registered no capability to hand the peer".to_string()),
        }
    }
}

/// Split `(uri, value)`, the reply a registering deploy puts on its reply channel.
///
/// The reply is refused rather than guessed at when it is not that pair: a build of
/// `invoke_member_term` that answered something else would otherwise be read as "no capabilities in
/// here", and the peer would get a value where it should have got an object.
fn split_registered_reply(
    reply: &rchain_models::ast::Par,
) -> Result<(String, rchain_models::ast::Par), String> {
    let Some(rchain_models::ast::Expr::ETuple(tuple)) = reply.exprs.first() else {
        return Err(format!(
            "a registering deploy answers `(uri, value)`, this one answered {reply:?}"
        ));
    };
    let [first, second] = tuple.ps.as_slice() else {
        return Err(format!(
            "a registering deploy answers a pair, this one answered {} element(s)",
            tuple.ps.len()
        ));
    };
    // A `rho:id:…` URI comes back as a **uri literal** (`GUri`) — `rho:registry:ops`'s `buildUri`
    // returns one — while a symbolic target like `rho:rchain:ertp` would be a string. Both name the
    // same registry key: `rho:registry:lookup` accepts either and reduces it to its text.
    match first.exprs.first() {
        Some(rchain_models::ast::Expr::GUri(uri) | rchain_models::ast::Expr::GString(uri)) => {
            Ok((uri.clone(), second.clone()))
        }
        other => Err(format!(
            "the registered URI is neither a uri nor a string: {other:?}"
        )),
    }
}

/// Every capability inside a reply, with the **pattern** that binds it in a later deploy: a name or a
/// bundle *is* a capability, and a tuple is searched element by element. Everything else is data.
///
/// **The pattern is written from the value's shape, not from a position path.** The obvious encoding
/// is a path — `[2]` for the third element — and [`invoke_member_term`] would then spell it
/// `(_, _, member)`. That only works if the pattern knows how many elements the tuple has, and
/// Rholang has no rest pattern for tuples: `(_, member)` does not match a 3-element tuple, it
/// matches *nothing*, and the deploy then answers nothing at all (measured — an ERTP kit, whose reply
/// is `(brand, mint, issuer)`, sat there until the bridge's timeout). So the arity at every level the
/// pattern crosses is taken from the value the reply carried, and the sibling positions — whatever
/// their number — are wildcarded.
fn capability_patterns(par: &rchain_models::ast::Par, out: &mut Vec<String>) {
    capability_patterns_at(par, "member", out)
}

/// The recursive half: `at` is the pattern naming the leaf, and every tuple on the way down replaces
/// it with a pattern that keeps only the branch leading to it.
fn capability_patterns_at(par: &rchain_models::ast::Par, at: &str, out: &mut Vec<String>) {
    if !par.unforgeables.is_empty() || !par.bundles.is_empty() {
        out.push(at.to_string());
        return;
    }
    for expr in &par.exprs {
        if let rchain_models::ast::Expr::ETuple(tuple) = expr {
            for (index, element) in tuple.ps.iter().enumerate() {
                let fields: Vec<String> = tuple
                    .ps
                    .iter()
                    .enumerate()
                    .map(|(i, _)| {
                        if i == index {
                            at.to_string()
                        } else {
                            "_".to_string()
                        }
                    })
                    .collect();
                capability_patterns_at(element, &format!("({})", fields.join(", ")), out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The spend bound:** the node will not submit more than [`BRIDGED_DEPLOYMENTS_PER_SEC`]
    /// bridged deploys per second, and the refusal names the number.
    ///
    /// This is the observable for HAZOP row A3's DoS half. The row's *other* half — that every
    /// registering call also writes a permanent registry entry — is not boundable here and is
    /// registered as C221.
    #[test]
    fn the_bridged_deploy_budget_admits_its_number_and_then_refuses() {
        let limiter = rchain_shared::rate_limiter::RateLimiter::new(BRIDGED_DEPLOYMENTS_PER_SEC);
        for i in 0..BRIDGED_DEPLOYMENTS_PER_SEC {
            assert!(
                check_deploy_budget(&limiter).is_ok(),
                "deploy {i} is inside the budget"
            );
        }
        let refused = check_deploy_budget(&limiter).expect_err("the budget is spent");
        assert!(
            refused.contains(&BRIDGED_DEPLOYMENTS_PER_SEC.to_string()),
            "the refusal names the limit so a peer can read it: {refused}"
        );
        assert!(
            refused.contains("deploys per second"),
            "and what it is limiting: {refused}"
        );
    }

    /// **Two nodes are not one peer** (C224 item 2). Peer identity *is* `(designator, transport)`, so
    /// a shared constant designator made every node the same peer: a sturdyref to one resolved at the
    /// other. The same node keeps one name across restarts (it is derived from key material, not from
    /// the ephemeral session key), and two keys give two names.
    #[test]
    fn a_nodes_designator_comes_from_its_own_key_and_is_stable() {
        let node_id = rchain_comm::peer_node::NodeIdentifier::new(vec![9u8; 32]);
        let key_a = rchain_crypto::private_key::PrivateKey::new(vec![1u8; 32]);
        let key_b = rchain_crypto::private_key::PrivateKey::new(vec![2u8; 32]);

        let a = node_designator(Some(&key_a), &node_id);
        let b = node_designator(Some(&key_b), &node_id);
        assert_ne!(a, b, "two nodes must not share a designator");
        assert_eq!(
            a,
            node_designator(Some(&key_a), &node_id),
            "and it is stable"
        );
        assert_ne!(a, "rnode", "the constant is what this replaces");
        assert!(a.starts_with("rnode-"), "still recognisable in a log: {a}");

        // Without a key the node's own identifier names it — never the secret's bytes, which a peer
        // would be able to see.
        let keyless = node_designator(None, &node_id);
        assert_ne!(keyless, a);
        assert!(!keyless.contains(&rchain_shared::base16::encode(&[1u8; 32])[..4]));
    }

    /// A logger that remembers what it was told. `StderrLog` is not assertable, which is why a test
    /// logger exists at all — the same reason the operations lens asked for one.
    #[derive(Default)]
    struct RecordingLog(std::sync::Mutex<Vec<String>>);

    impl rchain_shared::log::Log for RecordingLog {
        fn is_trace_enabled(&self, _source: rchain_shared::log::LogSource) -> bool {
            true
        }
        fn trace(&self, _s: rchain_shared::log::LogSource, m: &str) {
            self.0
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(m.to_string());
        }
        fn debug(&self, _s: rchain_shared::log::LogSource, m: &str) {
            self.0
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(m.to_string());
        }
        fn info(&self, _s: rchain_shared::log::LogSource, m: &str) {
            self.0
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(m.to_string());
        }
        fn warn(&self, _s: rchain_shared::log::LogSource, m: &str) {
            self.0
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(m.to_string());
        }
        fn error(&self, _s: rchain_shared::log::LogSource, m: &str) {
            self.0
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(m.to_string());
        }
    }

    /// **The operator is told the listener is serving** (HAZOP row E6): this surface spends the
    /// node's REV and writes consensus state, and until this line existed "is it up?" had no answer
    /// short of reading chain state.
    ///
    /// The port is `:0`, which is the case that makes the log line worth having at all: the operator
    /// asked for any free port, so the *chosen* one is the only useful thing to print.
    #[tokio::test]
    async fn the_listener_reports_that_it_is_serving_and_on_which_port() {
        let log = Arc::new(RecordingLog::default());
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        let serving = tokio::spawn(serve_ocapn(
            OcapnListeners {
                tcp: Some("127.0.0.1:0".to_string()),
                unix: None,
            },
            Vec::new(),
            "rnode-test".to_string(),
            false,
            Arc::new(std::sync::OnceLock::new()),
            log.clone(),
            stop_rx,
        ));

        // Wait for the line rather than for a sleep: the bind is what produces it.
        let mut said = Vec::new();
        for _ in 0..200 {
            said = log.0.lock().unwrap_or_else(|p| p.into_inner()).clone();
            if !said.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        stop_tx.send(true).expect("ask the listener to stop");
        let _ = serving.await;

        let line = said.join(" | ");
        assert!(
            line.contains("OCapN listener serving tcp-testing-only on 127.0.0.1:"),
            "the operator must learn the listener is up, and on which port: {line}"
        );
        assert!(
            !line.contains("127.0.0.1:0"),
            "the *chosen* port, not the configured one: {line}"
        );
    }

    /// **The `unix` transport binds its own socket and says so on its own line** (issue #249). A node
    /// serving both transports would otherwise leave an operator unable to tell which one came up, and
    /// the path is the only thing that identifies a unix listener.
    #[tokio::test]
    async fn the_unix_listener_reports_the_socket_it_bound() {
        let log = Arc::new(RecordingLog::default());
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        let path = std::env::temp_dir().join(format!("rnode-ocapn-{}.sock", std::process::id()));
        let serving = tokio::spawn(serve_ocapn(
            OcapnListeners {
                tcp: None,
                unix: Some(path.display().to_string()),
            },
            Vec::new(),
            "rnode-test".to_string(),
            false,
            Arc::new(std::sync::OnceLock::new()),
            log.clone(),
            stop_rx,
        ));

        let mut said = Vec::new();
        for _ in 0..200 {
            said = log.0.lock().unwrap_or_else(|p| p.into_inner()).clone();
            if !said.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        stop_tx.send(true).expect("ask the listener to stop");
        let _ = serving.await;
        let _ = std::fs::remove_file(&path);

        let line = said.join(" | ");
        assert!(
            line.contains("OCapN listener serving unix on "),
            "the operator must learn the unix listener is up: {line}"
        );
        assert!(line.contains("rnode-ocapn-"), "and on which socket: {line}");
    }

    /// **A node that listens on no transport is dial-only, and its task still runs** — the property
    /// that keeps `serve_ocapn`'s drain slot unconditional and gives the surfaces that dial out a task
    /// to live in. It must not return at once; it returns when the operator's word arrives.
    #[tokio::test]
    async fn a_dial_only_node_keeps_its_task_until_the_stop_word() {
        let log = Arc::new(RecordingLog::default());
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        let serving = tokio::spawn(serve_ocapn(
            OcapnListeners::default(),
            Vec::new(),
            "rnode-test".to_string(),
            false,
            Arc::new(std::sync::OnceLock::new()),
            log.clone(),
            stop_rx,
        ));
        // Give it every chance to return early; it must not.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(
            !serving.is_finished(),
            "a dial-only node's listener task must stay alive, not return at once"
        );
        assert!(
            log.0.lock().unwrap_or_else(|p| p.into_inner()).is_empty(),
            "and with nothing to serve it binds nothing and logs no listener line"
        );
        stop_tx.send(true).expect("ask the listener to stop");
        serving
            .await
            .expect("the task ends cleanly")
            .expect("and without error");
    }
}
