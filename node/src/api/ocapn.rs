//! The node's OCapN listener (issue #249).
//!
//! OCapN is the object-capability network an Agoric vat speaks: a peer dials, the two sides
//! handshake over a netlayer, and the peer works against objects the node exports. This module is
//! the node's end of that — a listener that serves a fresh session per connection — and the
//! **bridge**: a chain-backed capability whose deliveries become signed deploys.
//!
//! **A node may listen on any of four transports, several, or (dial-only) none.**
//! `tcp-testing-only` is the OCapN project's own, which its README flags as "HIGHLY INSECURE — DO NOT
//! USE IN PRODUCTION": plain TCP, no encryption, no authentication, so it is off unless
//! `api-server.ocapn-listen` names an address and a node reachable from anywhere it does not control
//! should leave it unset. `unix` (`api-server.ocapn-listen-unix`) takes a socket path and
//! authenticates by the socket's file mode, but is reachable only from this host. **`noise`
//! (`api-server.ocapn-listen-noise`) is the one a remote peer should use**: the handshake authenticates
//! both ends and encrypts everything above it, with no certificate authority and no daemon, and it is
//! checked against Agoric's own implementation (`spec/audit/evidence/ocapn-noise/`).
//! `websocket` (`api-server.ocapn-listen-websocket`) is the transport `@endo/ocapn` speaks, so it is
//! the one a *published* peer can be pointed at — and it is **weaker**: as the reference writes it,
//! `ws://` carries no TLS and its handshake has only the server prove itself.
//!
//! **The last two need `api-server.ocapn-identity-key`**, because both name this node by an Ed25519
//! key it must hold: `noise` puts it in the SYN's cleartext prefix and `websocket` signs a challenge
//! with it. That key is also what makes the node's designator a name a peer can *check* rather than a
//! hash of the deployer key.
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

use rchain_shared::lock::Unpoison;
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
use rchain_ocapn::capacity::MAX_DIALED_SESSIONS;
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
use rchain_ocapn::noise::{NoiseIdentity, NoiseNetlayer};
use rchain_ocapn::par_value;
use rchain_ocapn::syrup::Value;
use rchain_ocapn::tcp_testing_only::TcpTestingOnly;
use rchain_ocapn::unix::UnixNetlayer;
use rchain_ocapn::websocket::WebsocketNetlayer;
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
    capability_limiter: Arc<rchain_shared::rate_limiter::KeyedRateLimiter>,
) -> ChainCapability {
    ChainCapability {
        block_api,
        key,
        shard_id,
        target_uri: "rho:rchain:ertp".to_string(),
        pattern: None,
        method: None,
        limiter,
        capability_limiter,
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
/// to the capability, which is the same work as binding a session to a deployer key. **That work is
/// declined with Law 63a** (AUDIT C221): a session-key binding changes only what the node *knows*, not
/// who pays, so it would leave the node funding a stranger's deploys — the liability C221 names — and
/// the closure is the relay, which is a cross-implementation change. Law 63a's note carries the
/// decision; this limiter, plus `MAX_SESSIONS`, is the bound that holds until then.
pub const BRIDGED_DEPLOYMENTS_PER_SEC: u64 = 4;

/// **One capability's share of the node's bridged-deploy budget** (W-N8-02).
///
/// The node-wide bound above is the right bound for the *spend* — every bridged delivery is a signed
/// deploy paid out of the node's own REV, so replacing it with a per-capability limiter would be a
/// bound of *N* times the number, one per exported capability, and the wargame row's own
/// `resource-amplification` classification is what that would worsen.
///
/// What a node-wide bound alone gets wrong is fairness, which is a different fault: the one
/// capability a peer is looping on can spend the whole allowance, and every other capability this
/// node exports is refused for as long as it does. A share *under* the total answers that without
/// moving the total — the same shape the deploy pool's own bound took, for the same reason.
/// **A quarter, derived from the total rather than written beside it.** The point is that one
/// capability cannot take the whole allowance: at a quarter, four capabilities are served at once and
/// a fifth waits its turn — the node's spend is unchanged and no single object can starve the rest of
/// its surface. Stated as its own number it could be edited to something that is *not* under the
/// total, which would leave the first capability to arrive holding the entire budget, which is the
/// state this exists to leave; derived, that cannot be done by editing one of the two.
///
/// A total below four therefore makes the share zero, and `KeyedRateLimiter` treats zero as *closed*
/// rather than unlimited: the bridge stops loudly instead of quietly serving one caller per
/// capability. That is the fail-safe direction, and it is why the zero case is a refusal rather than a
/// special case here.
pub const BRIDGED_DEPLOYMENTS_PER_CAPABILITY_PER_SEC: u64 = BRIDGED_DEPLOYMENTS_PER_SEC / 4;

/// Distinct capabilities the share's map will track at once. Same rule as every other keyed bound
/// here: the map is itself a resource, and at the cap a new key is refused rather than admitted.
pub const MAX_TRACKED_CAPABILITIES: usize = 1_024;

/// Phlo for a bridged deploy — the same budget the faucet and the 2PC coordinator use.
const CHAIN_PHLO_LIMIT: i64 = 1_000_000;
const CHAIN_PHLO_PRICE: i64 = 1;
/// How long a delivery waits for its deploy's reply, and how often it looks.
const CHAIN_REPLY_TIMEOUT: Duration = Duration::from_secs(30);
const CHAIN_REPLY_INTERVAL: Duration = Duration::from_millis(250);
/// How far back to look for the reply datum, in blocks. Matches the socket-level callers.
const CHAIN_REPLY_DEPTH: i32 = 50;

/// **How many session-admission and session-end lines the node writes per second** (HAZOP row C236).
///
/// The two lines are the node's only record of *who* it is serving — without them an operator sees
/// the listener come up and nothing else, and cannot tell that an unauthenticated `websocket` peer was
/// admitted, over which transport, or under which name. They are worth the default level; the *rate*
/// is the peer's to choose, so they go through a limiter rather than being either silent or a flood.
/// Above it the line is written at `debug`, so a suppressed audit is still recoverable with the level
/// turned up.
const SESSION_AUDIT_PER_SEC: u64 = 20;

/// How many OCapN sessions the node serves at once.
///
/// Every session is a task, a socket, and its own export/answer tables, and the tables' size is the
/// peer's to choose, so the ceiling is what makes the per-session bounds add up to a node bound. 64 is
/// ~10× the conformance suite's heaviest case (it opens about six, serially) and far below any file
/// descriptor ceiling; the parameter that matters for an operator is that it is *finite*.
const MAX_SESSIONS: usize = 64;

/// How long the accept loop waits before trying again after a connection it could not establish.
///
/// Small, because an honest peer establishes in milliseconds and the retry exists only to keep a
/// listener that has genuinely broken from spinning a core; large enough that a peer which can only
/// fail holds the loop for a fraction of the time rather than all of it.
const ACCEPT_BACKOFF: std::time::Duration = std::time::Duration::from_millis(100);

/// Where the node listens for OCapN peers (issue #249).
///
/// **A fixed struct rather than a list, because the accept loop's `select!` is fixed-arity**: each
/// configured transport gets an arm, and one that is not configured gets an arm over a
/// never-completing future rather than a conditionally-built one. A node with **no** transport
/// configured is not an error — it is a **dial-only** node, and the task still runs so the surfaces
/// that dial out have somewhere to live.
///
/// **`noise` is the one a remote peer can reach.** `tcp-testing-only` is plaintext and
/// unauthenticated and `unix` is local by construction, so this is the transport that makes the node
/// a peer on a network rather than a service on the host that runs it. Its handshake is verified
/// against Agoric's own implementation (`spec/audit/evidence/ocapn-noise/`).
///
/// **`onion` is absent.** It is the only concrete transport in the OCapN draft, and it is not built:
/// it needs a `tor` daemon on the node and the draft it is pinned by says it is "likely to undergo
/// significant change". A key naming it would configure a transport this node cannot construct.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OcapnListeners {
    /// `host:port` for the `tcp-testing-only` transport.
    pub tcp: Option<String>,
    /// A socket path for the `unix` transport, whose authentication is the socket's file mode.
    pub unix: Option<String>,
    /// `host:port` for the `noise` transport, which authenticates both ends and encrypts the channel.
    pub noise: Option<String>,
    /// `host:port` for the `websocket` transport — the one `@endo/ocapn` speaks, and so the one a
    /// published peer can be pointed at. **Weaker than `noise`**: as the reference writes it, `ws://`
    /// carries no TLS and authenticates only the server. Reach for `noise` unless the peer speaks
    /// nothing else.
    pub websocket: Option<String>,
}

impl OcapnListeners {
    /// Whether the node listens on any transport. A node that listens on none is dial-only.
    pub fn any(&self) -> bool {
        self.tcp.is_some()
            || self.unix.is_some()
            || self.noise.is_some()
            || self.websocket.is_some()
    }

    /// Whether any configured transport needs the node's own key material — which is the two that
    /// authenticate with it, as opposed to the two that do not.
    pub fn needs_identity(&self) -> bool {
        self.noise.is_some() || self.websocket.is_some()
    }
}

/// **The node's OCapN identity, or why it has none** (HAZOP row C238) — one place, so the rules can be
/// tested without assembling a node.
///
/// Three cases, and each is a decision rather than an inference:
///
/// * a listener that authenticates with the key **needs** it — `noise` names the node by it and
///   `websocket` signs a challenge with it — so a missing `api-server.ocapn-identity-key` is refused
///   with the key to set, not a node that comes up nameless;
/// * **a key file with no such listener is read and validated, and still not used.** It is not an
///   error — it is what a node that will be dialled rather than dialling looks like — but *silently
///   ignoring* it was: an operator who set the key because the page says to got a node that never
///   opened the file, so a file of the wrong length, a loose mode, an all-zero key or a path in a
///   directory that does not exist were all indistinguishable from a correct one. Reading it here is
///   what makes that configuration report its own mistakes; the node's designator still does not
///   move, because nothing consumes the key. (An absent file is created — that is how a node gets a
///   stable name at all — and is the one case that is not a mistake.);
/// * neither set is a node with no identity, and the designator falls back to the derivation that
///   predates Noise (C224 item 2).
pub fn ocapn_identity_for(
    listeners: &OcapnListeners,
    key_path: Option<&str>,
) -> Result<Option<NoiseIdentity>, String> {
    match (listeners.needs_identity(), key_path) {
        (true, Some(path)) => Ok(Some(load_or_create_noise_identity(path)?)),
        (true, None) => Err(
            "api-server.ocapn-listen-noise and api-server.ocapn-listen-websocket need \
             api-server.ocapn-identity-key: both name this node by an Ed25519 key it must hold, \
             and a node with no key file cannot hold one across restarts"
                .to_string(),
        ),
        (false, Some(path)) => {
            load_or_create_noise_identity(path)?;
            Ok(None)
        }
        (false, None) => Ok(None),
    }
}

/// The node's **Noise identity**, loaded from the configured key file: an Ed25519 seed and an X25519
/// static, both stable across restarts.
///
/// **Why the node needs one and did not have one.** CapTP's own session identity is Ed25519, but it is
/// *ephemeral* — fresh per session — so it cannot name the node to anyone. The Noise handshake needs
/// the opposite: a key a peer can hold in advance and check, and the SYN it sends carries the peer's
/// Ed25519 verifying key in cleartext precisely so the responder can refuse a handshake meant for
/// someone else. `node_designator` is a *hash* of the deployer key and cannot sign, so a node that
/// wants to be reachable over Noise needs its own key material.
///
/// **The file is 64 bytes**: the Ed25519 seed, then the X25519 static, mode `0600`. Generated on
/// first use and persisted, because an identity that changes on restart is one no peer can name.
pub fn load_or_create_noise_identity(
    path: &str,
) -> Result<rchain_ocapn::noise::NoiseIdentity, String> {
    let path = std::path::Path::new(path);
    let found = |n: usize| {
        format!(
            "{} is not a Noise identity: expected 64 bytes (an Ed25519 seed then an X25519 static), \
             found {n}",
            path.display()
        )
    };
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // **Generated, then written before it is used**, so a node that fails right after still
            // has the identity it advertised rather than a new one on the next start.
            let identity = rchain_ocapn::noise::NoiseIdentity::generate()?;
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            // **Created with the mode, not chmodded after, and created exclusively.** Writing first
            // and restricting second left the node's identity — the key that names it on both
            // transports — readable at the process umask for as long as the two calls took, and a
            // plain write let two concurrent starts both generate and both overwrite, so a node could
            // run on a key the file no longer held (HAZOP rows C12, D3, E6, E8).
            write_new_private(path, &identity.to_persisted_bytes())?;
            return Ok(identity);
        }
        Err(e) => return Err(format!("reading {}: {e}", path.display())),
    };
    // **The mode is checked on the read side too.** It was enforced only when this node created the
    // file, so a world-readable identity — a copy, a restore from a backup, a file another tool made
    // — was read and trusted silently, and any other uid could then impersonate the node (HAZOP rows
    // E5, D3, F7). Refused rather than repaired: an operator who sees this has a key they should
    // re-issue, and quietly tightening the mode would hide that it was exposed.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path)
            .map_err(|e| e.to_string())?
            .permissions()
            .mode();
        if mode & 0o077 != 0 {
            return Err(format!(
                "{} is mode {:03o}: a node identity must not be readable or writable by another uid \
                 — re-issue it rather than widening who can impersonate this node",
                path.display(),
                mode & 0o777
            ));
        }
    }
    if bytes.len() != 64 {
        return Err(found(bytes.len()));
    }
    let seed: [u8; 32] = bytes[..32].try_into().map_err(|_| found(bytes.len()))?;
    let stat: [u8; 32] = bytes[32..].try_into().map_err(|_| found(bytes.len()))?;
    // **An all-zero key is refused.** `NoiseIdentity::new` re-derives the verifying key and the
    // X25519 public from whatever it is given, and for a zeroed file both are publicly computable —
    // so a truncated or blanked identity yielded a node whose name and session keys anyone could
    // derive, while every length check passed (HAZOP row D5). Only the *length* used to be validated.
    if seed == [0u8; 32] || stat == [0u8; 32] {
        return Err(format!(
            "{} is an all-zero key: this node's name and session keys would be publicly computable",
            path.display()
        ));
    }
    rchain_ocapn::noise::NoiseIdentity::new(seed, stat)
}

/// Create the identity file at `path` with mode `0600`, failing rather than overwriting if it already
/// exists — the exclusive half of [`load_or_create_noise_identity`].
#[cfg(unix)]
fn write_new_private(path: &std::path::Path, bytes: &[u8; 64]) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("creating {}: {e}", path.display()))?;
    file.write_all(bytes)
        .map_err(|e| format!("writing {}: {e}", path.display()))
}

/// The same, where there is no mode to set — the file is created exclusively and the platform's own
/// permissions decide who can read it.
#[cfg(not(unix))]
fn write_new_private(path: &std::path::Path, bytes: &[u8; 64]) -> Result<(), String> {
    std::fs::write(path, bytes).map_err(|e| format!("writing {}: {e}", path.display()))
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
    /// **The ceiling a dialed session draws from** (HAZOP row C241), shared with every fixture that
    /// dials. The accept path has its own per-transport shares; this is the budget for the sessions
    /// the node *starts*, and without it a dialer could hold `MAX_PEERS` sessions against a ceiling
    /// of `MAX_SESSIONS`.
    dials: Arc<tokio::sync::Semaphore>,
}

impl OcapnDialer {
    /// Build a dialer over an already-bound layer. [`serve_ocapn`] publishes one through an
    /// [`OcapnDialSlot`]; this is for a caller that has its own.
    pub fn new(
        netlayer: Arc<dyn Netlayer>,
        registry: Arc<rchain_ocapn::owner::SessionRegistry>,
        location: PeerLocator,
        dials: Arc<tokio::sync::Semaphore>,
    ) -> OcapnDialer {
        OcapnDialer {
            netlayer,
            registry,
            location,
            dials,
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
        let enlivener = rchain_ocapn::enliven::Enlivener::with_dial_budget(
            self.netlayer.clone(),
            self.location.clone(),
            self.registry.clone(),
            rchain_ocapn::owner::session_slot(),
            Some(self.dials.clone()),
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

    /// Accept one connection, **retrying a failure rather than letting it end the listener**.
    ///
    /// **The two establishing transports fail their accept for reasons a peer chooses.** `noise` runs
    /// a handshake inside accept and `websocket` an upgrade and an in-band challenge, so a peer that
    /// connects and then abandons one fails the accept — and a listener that ended there would let a
    /// port scan take the node's whole OCapN surface down with a connection it did not even have to
    /// finish. `tcp-testing-only` and `unix` fail only for real listener trouble, which is retried
    /// here too and said out loud at `warn`: the delay is what keeps a listener that has genuinely
    /// broken from spinning a core while it complains, and a burst of warnings is a signal an operator
    /// can act on where a silent death is not.
    ///
    /// Never returns for a transport the node does not listen on — `accept` is `pending()` there.
    async fn accept_recovering(
        &self,
        log: &Arc<dyn rchain_shared::log::Log>,
        source: rchain_shared::log::LogSource,
    ) -> (Box<dyn NetConn>, PeerLocator) {
        loop {
            match self.accept().await {
                Ok(accepted) => return accepted,
                Err(e) => {
                    log.warn(source, &format!("an OCapN connection was refused: {e}"));
                    tokio::time::sleep(ACCEPT_BACKOFF).await;
                }
            }
        }
    }
}

/// **The host a peer is told to dial for a listener bound at `bound`** (HAZOP row C237).
///
/// `local_addr()` is what the socket is bound to, and it is the right answer for a specific address —
/// but a bind to `0.0.0.0` or `::` means *every* address on this host, and **that is not an address
/// another host can dial**: a remote peer that follows it reaches itself, so every sturdyref and
/// handoff to this node is unusable off-host. Same-host dialling happens to work, which is why no
/// test — and no run against a peer — had measured it.
///
/// So an unspecified bind **needs** the operator to say what to advertise. It is refused here rather
/// than advertised, and the error names the key to set: a node that starts and hands out unusable
/// locations is worse than one that does not start.
fn advertised_host(
    bound: std::net::SocketAddr,
    configured: Option<&str>,
    transport: &str,
) -> Result<String, String> {
    match configured {
        Some(host) => Ok(host.to_string()),
        None if bound.ip().is_unspecified() => Err(format!(
            "the {transport} OCapN listener is bound to {bound}, which is every address on this \
             host and not one a peer can dial back — a remote peer that follows it reaches itself, \
             so every location this node hands out would be unusable off-host. Set \
             api-server.ocapn-advertised-host to the name or address peers should dial"
        )),
        None => Ok(bound.ip().to_string()),
    }
}

/// Bind the `tcp-testing-only` listener and build the location a peer reaches it at.
async fn listen_tcp(
    address: &str,
    policy: DialPolicy,
    designator: &str,
    chain: usize,
    advertised: Option<&str>,
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
    // conflated their sessions.
    //
    // **Where it comes from depends on configuration, and this comment used to name only one case.**
    // `node_runtime` derives it: the **Ed25519 verifying key** when a `noise` or `websocket` listener
    // gave the node an identity — because that is the name the handshake actually checks — and
    // `node_designator`'s deployer-key hash otherwise. Both are stable across restarts and distinct
    // per node; they are not the same *shape*, so a node the operator later gives an identity changes
    // name (HAZOP row D8).
    let location = PeerLocator {
        designator: designator.to_string(),
        transport: "tcp-testing-only".to_string(),
        hints: BTreeMap::from([
            (
                "host".to_string(),
                advertised_host(local, advertised, "tcp-testing-only")?,
            ),
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

/// Bind the `noise` listener and build the location a peer reaches it at.
///
/// **The `host` hint is the *advertised* one, not the bound address** (HAZOP row C237): see
/// [`advertised_host`]. A node bound to `0.0.0.0` without `api-server.ocapn-advertised-host` is
/// refused here rather than handing out a location no peer can dial.
///
/// **The location carries this node's Ed25519 verifying key**, in the `verify` hint, because that is
/// what a dialler must put in the SYN's cleartext prefix — the handshake is where the name is checked,
/// so the name has to travel with the address. This is the one advertised location that says *who*
/// the node is and not only where it is.
async fn listen_noise(
    address: &str,
    policy: DialPolicy,
    designator: &str,
    chain: usize,
    identity: NoiseIdentity,
    advertised: Option<&str>,
    log: &Arc<dyn rchain_shared::log::Log>,
    source: rchain_shared::log::LogSource,
) -> Result<(Arc<dyn Netlayer>, PeerLocator), String> {
    let verifying = identity.verifying_key();
    let bound = NoiseNetlayer::bind(address, identity)
        .await
        .map_err(|e| e.to_string())?;
    let local = bound.local_addr().map_err(|e| e.to_string())?;
    log.info(
        source,
        &format!("OCapN listener serving noise on {local} ({chain} chain-backed capability/ies)"),
    );
    let location = PeerLocator {
        designator: designator.to_string(),
        transport: "noise".to_string(),
        hints: BTreeMap::from([
            (
                "host".to_string(),
                advertised_host(local, advertised, "noise")?,
            ),
            ("port".to_string(), local.port().to_string()),
            (
                "verify".to_string(),
                rchain_shared::base16::encode(&verifying),
            ),
        ]),
    };
    Ok((Arc::new(PolicyNetlayer::new(bound, policy)), location))
}

/// Bind the `websocket` listener and build the location a peer reaches it at.
///
/// The location carries the `url` hint the reference reads — it appends no path and no query, so the
/// whole address is the hint — and this node's Ed25519 verifying key, which the peer checks the
/// challenge response against.
async fn listen_websocket(
    address: &str,
    policy: DialPolicy,
    chain: usize,
    identity: NoiseIdentity,
    advertised: Option<&str>,
    log: &Arc<dyn rchain_shared::log::Log>,
    source: rchain_shared::log::LogSource,
) -> Result<(Arc<dyn Netlayer>, PeerLocator), String> {
    let bound = WebsocketNetlayer::bind(address, identity)
        .await
        .map_err(|e| e.to_string())?;
    let local = bound.local_addr().map_err(|e| e.to_string())?;
    log.info(
        source,
        &format!(
            "OCapN listener serving websocket on {local} ({chain} chain-backed capability/ies)"
        ),
    );
    // **The advertised host, not the bound one** (HAZOP row C237): `advertised_host` refuses an
    // unspecified bind the operator has not named, and otherwise returns the bound address.
    let host = advertised_host(local, advertised, "websocket")?;
    // **No designator argument**: this transport names itself, and the convention is Endo's rather
    // than the operator's (AUDIT C245) — see `WebsocketNetlayer::location`.
    let location = bound.location(Some(&host)).map_err(|e| e.to_string())?;
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
    // The node's Noise identity, required exactly when `listeners.noise` is set.
    noise_identity: Option<NoiseIdentity>,
    // The host peers are told to dial. Required exactly when a listener is bound to an address no
    // peer can reach (`0.0.0.0`/`::`) — see `advertised_host`.
    advertised_host: Option<String>,
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
                advertised_host.as_deref(),
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
    // **A `noise` address without an identity is a configuration the node cannot honour**, and it says
    // so rather than binding a listener whose name would be thirty-two zero bytes.
    let noise = match (listeners.noise.as_deref(), noise_identity.as_ref()) {
        (Some(address), Some(identity)) => Some(
            listen_noise(
                address,
                policy.clone(),
                &designator,
                chain.len(),
                identity.clone(),
                advertised_host.as_deref(),
                &log,
                source,
            )
            .await?,
        ),
        (Some(_), None) => {
            return Err(
                "api-server.ocapn-listen-noise is set but the node has no identity; set \
                 api-server.ocapn-identity-key so the node has a name a peer can check"
                    .to_string(),
            )
        }
        (None, _) => None,
    };
    // The same shape, and the same requirement: the challenge response is signed with the node's key,
    // so a websocket listener without one could not answer it.
    let websocket = match (listeners.websocket.as_deref(), noise_identity.as_ref()) {
        (Some(address), Some(identity)) => Some(
            listen_websocket(
                address,
                policy.clone(),
                chain.len(),
                identity.clone(),
                advertised_host.as_deref(),
                &log,
                source,
            )
            .await?,
        ),
        (Some(_), None) => {
            return Err(
                "api-server.ocapn-listen-websocket is set but the node has no identity; set \
                 api-server.ocapn-identity-key so the node has a key to answer the challenge with"
                    .to_string(),
            )
        }
        (None, _) => None,
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
    if let Some((layer, _)) = &noise {
        dialer = dialer.with("noise", layer.clone());
    }
    if let Some((layer, _)) = &websocket {
        dialer = dialer.with("websocket", layer.clone());
    }
    let dialer: Arc<dyn Netlayer> = Arc::new(dialer);
    // The location this node advertises in a dial it starts itself. **`noise` first, then the order
    // that predates it (tcp, then unix)**: a peer that is handed this location has to be able to dial
    // it back, and `noise` is both reachable and authenticated while `tcp-testing-only` is reachable
    // and not and `unix` is authenticated and not. Putting `noise` first does not change what the two
    // transports that existed before advertise between themselves.
    let outward = noise
        .as_ref()
        .or(tcp.as_ref())
        .or(unix.as_ref())
        .or(websocket.as_ref())
        .map(|(_, location)| location.clone());
    let tcp = Listener::new(tcp);
    let unix = Listener::new(unix);
    let noise = Listener::new(noise);
    let websocket = Listener::new(websocket);
    // One registry and one gift store for the node's whole OCapN surface: the crossed-hello rule
    // compares the sessions *this node* has with a peer, and a handoff is deposited on one session
    // and withdrawn on another.
    let registry = Arc::new(rchain_ocapn::owner::SessionRegistry::default());
    let handoffs = Arc::new(rchain_ocapn::handoff::Handoffs::default());
    // **Publish the dialer** for the admin route that starts a dial of the node's own. A node with no
    // transport does not publish one: there is no layer to dial with, and the route says so rather
    // than dialing into "this node speaks nothing".
    // **One dial budget for the node's own sessions** (HAZOP row C241): the admin route and every
    // fixture that dials draw from it, so the sessions the node *starts* are bounded like the ones it
    // accepts. A dialed session holds its permit for its whole life.
    let dials = Arc::new(tokio::sync::Semaphore::new(
        rchain_ocapn::capacity::MAX_DIALED_SESSIONS,
    ));
    if let Some(location) = outward {
        let _ = dial_slot.set(OcapnDialer {
            netlayer: dialer.clone(),
            registry: registry.clone(),
            location,
            dials: dials.clone(),
        });
    }
    // **The session ceiling: every transport gets an equal share of it, and the permit is taken
    // before the task exists** (HAZOP rows B1 and C229). One node-wide semaphore let a peer hold 3 000
    // idle connections (+41 MB, RSS never returned), so the count is bounded; but a *transport-blind*
    // bound also let the transport that authenticates **nobody** (`tcp-testing-only`, `websocket`)
    // take every permit and starve `noise`, the one that authenticates both ends. Each transport now
    // holds its own share, and the shares sum to the ceiling **less the dial reserve** rather than
    // nesting under it — so a node listening on one transport still has all of the ceiling that is
    // not held back for its own dials. Taken before the spawn, so a refused connection costs nothing
    // but the socket, which `accept_transport` drops.
    //
    // **What the share does not do, said here rather than implied:** a peer that establishes a session
    // and then says nothing still holds its permit for as long as it likes. An idle session is
    // legitimate — `HANDSHAKE_TIMEOUT`'s note records why the steady-state read is deliberately *not*
    // bounded, and the conformance suite needs a silent leg — so the share bounds the *blast radius*:
    // the whole surface is never one transport's to take, and never a peer's to take with one
    // identity.
    let configured = [&tcp, &unix, &noise, &websocket]
        .iter()
        .filter(|listener| listener.inner.is_some())
        .count();
    // **And the dials come out of the same ceiling** (HAZOP row C241). A dialed session holds its
    // permit for its whole life just as an accepted one does, so it has to be *counted* against
    // `MAX_SESSIONS` rather than against a second budget beside it — otherwise a peer that makes the
    // node dial holds sessions past the ceiling the operator was told, and the two numbers have to be
    // added up by hand to know the node's real surface. `MAX_DIALED_SESSIONS` is reserved for the
    // sessions the node *starts* (the admin route, and every fixture that dials); the accept paths
    // split what is left, and the two together are exactly the ceiling.
    let accept_total = MAX_SESSIONS.saturating_sub(MAX_DIALED_SESSIONS).max(1);
    let share = (accept_total / configured.max(1)).max(1);
    let factory = SessionFactory {
        chain: Arc::new(chain),
        dialer,
        registry,
        handoffs,
        dials: dials.clone(),
        log,
        // **The same word that ends the listeners ends their sessions** (HAZOP row C231). Each
        // session task is spawned and detached, so returning from this function on the stop word left
        // every live session running: the sockets closed but the sessions the operator was shutting
        // down carried on until the process died, and nothing said so.
        stop: stop.clone(),
        audit: Arc::new(rchain_shared::rate_limiter::RateLimiter::new(
            SESSION_AUDIT_PER_SEC,
        )),
    };
    // **One accept task per transport** (HAZOP row C239) — see `accept_transport` for the cancellation
    // a single `select!` caused, and why a `biased` select is not the fix. A transport the node does
    // not listen on still gets a task: it waits for ever on a `pending()` accept, which keeps the
    // shape uniform and is also what keeps a **dial-only** node's task alive.
    let mut accepts = tokio::task::JoinSet::new();
    for (transport, listener) in [
        ("tcp-testing-only", tcp),
        ("unix", unix),
        ("noise", noise),
        ("websocket", websocket),
    ] {
        accepts.spawn(accept_transport(
            transport,
            listener,
            factory.clone(),
            Arc::new(tokio::sync::Semaphore::new(share)),
            share,
            accept_total,
            source,
        ));
    }
    // **The stop word ends the accept tasks, not just this function.** Aborting them drops each
    // listener's accept future with its socket; returning while they ran on would leave the node's
    // ports bound with nothing draining the sessions that arrive.
    let _ = stop_requested(stop.clone()).await;
    accepts.abort_all();
    Ok(())
}

/// The node's shared surfaces, cloned into each accept task and then into each session task.
#[derive(Clone)]
struct SessionFactory {
    /// The chain-backed capabilities to publish on each session's bootstrap. Shared across sessions
    /// because they hold no per-session state.
    chain: Arc<Vec<(Vec<u8>, Arc<dyn Export>)>>,
    dialer: Arc<dyn Netlayer>,
    registry: Arc<rchain_ocapn::owner::SessionRegistry>,
    handoffs: Arc<rchain_ocapn::handoff::Handoffs>,
    /// The node's dial ceiling, handed to every session's fixtures so a fixture that dials draws from
    /// the same budget as the admin route (HAZOP row C241).
    dials: Arc<tokio::sync::Semaphore>,
    log: Arc<dyn rchain_shared::log::Log>,
    /// The operator's stop word, so a session ends with the node rather than outliving it (HAZOP row
    /// C231).
    stop: watch::Receiver<bool>,
    /// **The audit trail's rate bound** (HAZOP row C236). Admission and a session's end are written at
    /// `info` — the default level — because an operator's only other view is "the listener was up";
    /// the *rate* is the peer's to choose, so those lines go through this limiter rather than being
    /// either silent or a flood.
    audit: Arc<rchain_shared::rate_limiter::RateLimiter>,
}

/// **One accept task per transport** (HAZOP row C239).
///
/// The accept loop used to be one `tokio::select!` over an arm per transport, and `select!` is not
/// `biased`: when any arm completes, Tokio **drops** the other branches' futures. So a websocket peer
/// that was mid-establishment — its accept future `Pending` inside the TLS/WebSocket upgrade or the
/// in-band challenge — had its already-accepted socket dropped, resetting the connection the moment a
/// connection arrived on `tcp`, `unix` or `noise`. The peer did nothing wrong and nothing was logged,
/// because a dropped future is not an `Err`. The red team measured it. A `biased` select is **not**
/// the fix: it would let one silent arm starve the others.
///
/// A task per transport removes the cancellation outright — nothing outside a transport's own task can
/// end its pending accept. The ceiling is per transport too (`share`, HAZOP row C229), so no transport
/// — in particular not one that authenticates nobody — can take the node's whole session surface.
async fn accept_transport(
    transport: &'static str,
    listener: Listener,
    factory: SessionFactory,
    sessions: Arc<tokio::sync::Semaphore>,
    share: usize,
    // The ceiling less the dial reserve — the number every transport's share is a fraction of, and
    // what a refusal names so an operator can tell a full transport from a full node.
    accept_total: usize,
    source: rchain_shared::log::LogSource,
) {
    loop {
        let (connection, location) = listener.accept_recovering(&factory.log, source).await;
        let Ok(permit) = sessions.clone().try_acquire_owned() else {
            // At this transport's share: close the socket rather than queue it. A peer that keeps
            // connecting gets refusals, not a growing backlog, and the node's *other* transports keep
            // serving. Said out loud, at `warn`, and naming the transport and the number: a node whose
            // sessions are all held is either under attack or has a stuck peer, and the operator cannot
            // tell either from silence — nor which transport is full.
            factory.log.warn(
                source,
                &format!(
                    "at {transport}'s session share ({share} of {accept_total} accepts, of a ceiling \
                     of {MAX_SESSIONS}); refusing a new connection"
                ),
            );
            drop(connection);
            continue;
        };
        let factory = factory.clone();
        tokio::spawn(async move {
            factory.serve(connection, location, permit, source).await;
        });
    }
}

impl SessionFactory {
    /// Serve one accepted connection to its end.
    ///
    /// The permit is held for the session's life and released when this task ends: the task's own
    /// lifetime *is* the session's, so it needs no name beyond this binding.
    async fn serve(
        self,
        connection: Box<dyn NetConn>,
        location: PeerLocator,
        permit: tokio::sync::OwnedSemaphorePermit,
        source: rchain_shared::log::LogSource,
    ) {
        let _permit = permit;
        let SessionFactory {
            chain,
            dialer,
            registry,
            handoffs,
            dials,
            log,
            stop,
            audit,
        } = self;
        // A fresh session key per session, as OCapN requires.
        let Ok(identity) = Identity::fresh(location.clone()) else {
            return;
        };
        // The per-session objects that dial find their session through this slot, which is filled in
        // once the session exists — `accept` needs the bootstrap before that.
        let slot = rchain_ocapn::owner::session_slot();
        // The fixtures are rebuilt per session (they hold per-session promise state); the chain-backed
        // capability is shared, because it does not.
        let mut bootstrap =
            fixtures::conformance_bootstrap_with(handoffs, slot.clone(), registry.clone());
        for (swiss, capability) in chain.iter() {
            bootstrap.publish(swiss.clone(), capability.clone());
        }
        fixtures::publish_dialing_fixtures_with_budget(
            &mut bootstrap,
            dialer,
            location,
            registry.clone(),
            slot.clone(),
            Some(dials),
        );
        // **Book, then answer** — see `owner::accept_and_book`: a session has to be in the registry
        // before the peer can act on it, or a delivery that arrives on the first round trip after our
        // start-session (a handoff, a fetch) reaches an object that cannot find the session it belongs
        // to. The fourth element is the locator the session was **booked** under — the peer's own
        // advertised location whenever the peer is who it says it is, and the key the handshake proved
        // otherwise (HAZOP row C243) — and it is what `forget` must be given, or an entry the peer
        // cannot be found by would never be removed.
        let (handle, loop_, context, booked) = match rchain_ocapn::owner::accept_and_book(
            connection,
            &identity,
            Arc::new(bootstrap),
            &registry,
        )
        .await
        {
            Ok(parts) => parts,
            Err(reason) => {
                // A refused handshake, and a session that lost its crossing, have both already been
                // answered with `op:abort` where one was owed. Logged at **debug**, not warn: a
                // crossing is a legitimate outcome of the protocol, and a peer-driven stream of them
                // would make a warning meaningless (the log-flood the operations lens warned about).
                log.debug(source, &format!("session not served: {reason}"));
                return;
            }
        };
        *slot.lock().unpoison() = Some(context);
        // **Admission is auditable** (HAZOP row C236). The name is the peer's own assertion where the
        // transport authenticates nobody, and the key the handshake *proved* where it does — both,
        // because "who said they were calling" and "who the transport says it is" are different
        // facts and an operator needs to see which is which.
        let who = match booked.hints.get(rchain_ocapn::owner::VERIFY_HINT) {
            Some(proved) => format!(
                "{} (proved {proved}) over {}",
                booked.designator, booked.transport
            ),
            None => format!("{} over {}", booked.designator, booked.transport),
        };
        if audit.allow() {
            log.info(source, &format!("session admitted: {who}"));
        } else {
            log.debug(
                source,
                &format!("session admitted (audit suppressed): {who}"),
            );
        }
        // One session per connection; its end is this task's end.
        // **The loop's outcome was discarded**, so a session that ended with an error was
        // indistinguishable from one that ended cleanly — the node said nothing either way, and it
        // took an instrumented run to see that a session had ended at all. Said at `debug`, because a
        // peer closing its session is ordinary and a `warn` would be meaningless; but said, which it
        // was not. (Found by this study's RCA.)
        //
        // **And the operator's stop ends the session too** (HAZOP row C231). The session tasks are
        // spawned and detached, so the listener returning on the stop word left every live session
        // running: the listeners' sockets closed and the sessions carried on until the process died,
        // which is a drain that drains the wrong half. Cancelling `run` here drops the session's
        // socket, which is what the peer sees either way.
        tokio::select! {
            outcome = loop_.run() => {
                if let Err(e) = outcome {
                    log.debug(source, &format!("session loop ended: {e}"));
                }
            }
            _ = stop_requested(stop) => {
                if audit.allow() {
                    log.info(source, &format!("session ended: {who} (the node is stopping)"));
                }
            }
        }
        // The end of what was admitted, at the same level and through the same limiter (HAZOP row
        // C236): an admission nobody can pair with an end is a leak an operator cannot see.
        if audit.allow() {
            log.info(source, &format!("session ended: {who}"));
        }
        registry.forget(&booked, &handle.own_pi, handle.dialed);
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

/// Take one unit of the node's bridged-deploy budget **and one of this capability's share of it**, or
/// refuse with a reason naming which bound was hit.
///
/// Extracted from `ChainCapability::call` so it is testable without a `BlockApi`: the check runs
/// *before* the capability touches the chain, which means the whole observable is "the limiter was
/// consulted and the refusal names the bound" — and a bound that cannot be tested is one that gets
/// removed. The same reason `admin_bind_host` was lifted out of its spawn (AUDIT C112).
///
/// The share is consulted first, so the common refusal — one object looping — does not spend a token
/// from the budget every other capability shares.
fn check_deploy_budget(
    node: &rchain_shared::rate_limiter::RateLimiter,
    per_capability: &rchain_shared::rate_limiter::KeyedRateLimiter,
    capability: &[u8],
) -> Result<(), String> {
    if !per_capability.allow(capability) {
        return Err(format!(
            "one capability may take at most {BRIDGED_DEPLOYMENTS_PER_CAPABILITY_PER_SEC} of the \
             chain bridge's {BRIDGED_DEPLOYMENTS_PER_SEC} deploys per second"
        ));
    }
    if node.allow() {
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
    /// the node's REV and the far contract sees the *node* as the caller. **Binding a CapTP session
    /// to a caller's secp256k1 identity is declined with Law 63a** (AUDIT C221): it changes what the
    /// node *knows*, not who pays, so it does not dissolve the liability that the node funds a
    /// stranger's deploys — the relay does, and the relay is a cross-implementation change. Until it
    /// lands this is the node acting as itself, bounded by [`BRIDGED_DEPLOYMENTS_PER_SEC`].
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
    /// **The node's whole OCapN surface shares this one**: the bound is on the node's spend, because
    /// every delivery is paid out of the node's own REV — see [`BRIDGED_DEPLOYMENTS_PER_SEC`].
    limiter: Arc<rchain_shared::rate_limiter::RateLimiter>,
    /// **And this capability's share of it** (see [`BRIDGED_DEPLOYMENTS_PER_CAPABILITY_PER_SEC`]).
    /// Keyed on the capability's own designator, so the one object a peer loops on cannot spend the
    /// allowance every other exported capability shares.
    capability_limiter: Arc<rchain_shared::rate_limiter::KeyedRateLimiter>,
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
        capability_limiter: Arc<rchain_shared::rate_limiter::KeyedRateLimiter>,
    ) -> ChainCapability {
        ChainCapability {
            block_api,
            key,
            shard_id,
            target_uri: "rho:rchain:revVault".to_string(),
            pattern: None,
            method: Some("getBalance".to_string()),
            limiter,
            capability_limiter,
            reply_timeout: CHAIN_REPLY_TIMEOUT,
            reply_interval: CHAIN_REPLY_INTERVAL,
        }
    }

    /// What this capability *is* in the node's own registry: the URI it is reachable at and the
    /// pattern that binds the object inside the registered value. Two capabilities with the same pair
    /// are the same object to a peer, which is what makes it the right key for a per-object share.
    fn designator(&self) -> Vec<u8> {
        let mut key = self.target_uri.clone().into_bytes();
        key.push(b'|');
        if let Some(pattern) = &self.pattern {
            key.extend_from_slice(pattern.as_bytes());
        }
        key
    }

    /// Wait for the deploy's reply channel to carry a value.
    async fn await_reply(&self, deploy_id: &[u8]) -> Result<rchain_models::ast::Par, String> {
        let name = rchain_casper::shard_invoke::reply_channel(deploy_id);
        let deadline = Instant::now() + self.reply_timeout;
        loop {
            // **Without the finality floor.** The value being waited for is in a block this node has
            // just produced, and on a chain whose finality lags — or on every single-validator
            // fixture, where it never advances past genesis — that block is above the floor by
            // construction. The floored read is for a *caller* asking what a chain says; this is the
            // node asking what its own deploy said, and waiting for finality here would mean the
            // bridge could not answer at all (W-N3-04).
            let (data, _length) = self
                .block_api
                .get_unfinalised_name_data_response(CHAIN_REPLY_DEPTH, &name)
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
        check_deploy_budget(&self.limiter, &self.capability_limiter, &self.designator())?;
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
                    // **The child shares the node's limiter but not the parent's share.** A returned
                    // capability is the same node's spend, so it draws on the same budget — but its
                    // own designator is the share's key, so a peer looping on one returned object
                    // cannot spend the allowance its siblings share.
                    limiter: self.limiter.clone(),
                    capability_limiter: self.capability_limiter.clone(),
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
        let share = rchain_shared::rate_limiter::KeyedRateLimiter::new(
            BRIDGED_DEPLOYMENTS_PER_CAPABILITY_PER_SEC,
            MAX_TRACKED_CAPABILITIES,
        );
        // **A distinct capability each time**, so what refuses the fifth delivery is the *total* and
        // not the per-capability share — the share has its own test below, and with the share at one
        // a single looping capability would never reach the total.
        for i in 0..BRIDGED_DEPLOYMENTS_PER_SEC {
            let key = format!("rho:rchain:cap{i}|");
            assert!(
                check_deploy_budget(&limiter, &share, key.as_bytes()).is_ok(),
                "deploy {i} is inside the budget"
            );
        }
        let refused = check_deploy_budget(&limiter, &share, b"rho:rchain:cap5|")
            .expect_err("the budget is spent");
        assert!(
            refused.contains(&BRIDGED_DEPLOYMENTS_PER_SEC.to_string()),
            "the refusal names the limit so a peer can read it: {refused}"
        );
        assert!(
            refused.contains("deploys per second"),
            "and what it is limiting: {refused}"
        );
    }

    /// **One capability cannot spend the allowance every other one shares** (W-N8-02).
    ///
    /// The node-wide bound is the right bound for the *spend* — a limiter per capability instead of it
    /// would be a bound of N times the number, which is the class the row itself is filed under. What
    /// it gets wrong on its own is fairness: the object a peer loops on takes the whole allowance and
    /// every other exported capability is refused while it does. The share is *under* the total, so
    /// this asserts both halves at once — the looping capability is refused while the total is
    /// demonstrably unspent, and a different capability is served.
    ///
    /// Falsified by passing the same key for both capabilities, or by dropping the share check: the
    /// second capability is then refused as well.
    #[test]
    fn one_capability_cannot_spend_anothers_share_of_the_bridge_budget() {
        let limiter = rchain_shared::rate_limiter::RateLimiter::new(BRIDGED_DEPLOYMENTS_PER_SEC);
        let share = rchain_shared::rate_limiter::KeyedRateLimiter::new(
            BRIDGED_DEPLOYMENTS_PER_CAPABILITY_PER_SEC,
            MAX_TRACKED_CAPABILITIES,
        );
        let looping = b"rho:rchain:ertp|";
        let other = b"rho:rchain:revVault|";

        for i in 0..BRIDGED_DEPLOYMENTS_PER_CAPABILITY_PER_SEC {
            assert!(
                check_deploy_budget(&limiter, &share, looping).is_ok(),
                "delivery {i} is inside this capability's share"
            );
        }
        let refused = check_deploy_budget(&limiter, &share, looping)
            .expect_err("the looping capability is over its share");
        assert!(
            refused.contains("one capability"),
            "and the refusal says it is the share rather than the budget: {refused}"
        );
        assert!(
            check_deploy_budget(&limiter, &share, other).is_ok(),
            "another capability's share is its own; the looping one must not have refused it"
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
            self.0.lock().unpoison().push(m.to_string());
        }
        fn debug(&self, _s: rchain_shared::log::LogSource, m: &str) {
            self.0.lock().unpoison().push(m.to_string());
        }
        fn info(&self, _s: rchain_shared::log::LogSource, m: &str) {
            self.0.lock().unpoison().push(m.to_string());
        }
        fn warn(&self, _s: rchain_shared::log::LogSource, m: &str) {
            self.0.lock().unpoison().push(m.to_string());
        }
        fn error(&self, _s: rchain_shared::log::LogSource, m: &str) {
            self.0.lock().unpoison().push(m.to_string());
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
                noise: None,
                websocket: None,
            },
            Vec::new(),
            "rnode-test".to_string(),
            false,
            Arc::new(std::sync::OnceLock::new()),
            None,
            None,
            log.clone(),
            stop_rx,
        ));

        // Wait for the line rather than for a sleep: the bind is what produces it.
        let mut said = Vec::new();
        for _ in 0..200 {
            said = log.0.lock().unpoison().clone();
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
                noise: None,
                websocket: None,
            },
            Vec::new(),
            "rnode-test".to_string(),
            false,
            Arc::new(std::sync::OnceLock::new()),
            None,
            None,
            log.clone(),
            stop_rx,
        ));

        let mut said = Vec::new();
        for _ in 0..200 {
            said = log.0.lock().unpoison().clone();
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
            None,
            None,
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
            log.0.lock().unpoison().is_empty(),
            "and with nothing to serve it binds nothing and logs no listener line"
        );
        stop_tx.send(true).expect("ask the listener to stop");
        serving
            .await
            .expect("the task ends cleanly")
            .expect("and without error");
    }

    /// **Admission and a session's end are visible at the default level** (HAZOP row C236).
    ///
    /// The node logged that its listener was up and nothing else: not which peer was admitted, over
    /// which transport, under which name — so an operator could not tell that an unauthenticated
    /// `tcp-testing-only` (or `websocket`) peer had been admitted at all. Both lines are at `info`
    /// now, naming the peer, and the end is the admission's pair: an admission nobody can match with
    /// an end is a leak nobody can see. This drives a real CapTP dial, because a line that only
    /// appears for a connection no peer could have made is not an audit trail.
    #[tokio::test]
    async fn an_operators_log_names_the_peer_that_was_admitted_and_when_it_left() {
        use rchain_ocapn::bootstrap::Bootstrap;
        use rchain_ocapn::conn::{Identity, Session};
        use rchain_ocapn::locator::PeerLocator;
        use rchain_ocapn::netlayer::Netlayer;
        use rchain_ocapn::tcp_testing_only::TcpTestingOnly;

        let log = Arc::new(RecordingLog::default());
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        let serving = tokio::spawn(serve_ocapn(
            OcapnListeners {
                tcp: Some("127.0.0.1:0".to_string()),
                ..OcapnListeners::default()
            },
            Vec::new(),
            "rnode-test".to_string(),
            false,
            Arc::new(std::sync::OnceLock::new()),
            None,
            None,
            log.clone(),
            stop_rx,
        ));

        let port: u16 = loop {
            let said = log.0.lock().unpoison().clone();
            if let Some(port) = said
                .iter()
                .find_map(|line| line.split("tcp-testing-only on 127.0.0.1:").nth(1))
                .and_then(|rest| rest.split([' ', '(']).next())
                .and_then(|port| port.parse().ok())
            {
                break port;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        };

        // A peer that names itself, over the transport that authenticates nobody: the name is the
        // peer's own assertion, which is exactly what an operator has to be able to see.
        let dialer = TcpTestingOnly::bind("127.0.0.1:0")
            .await
            .expect("bind the dialing side");
        let node = PeerLocator {
            designator: "rnode-test".to_string(),
            transport: "tcp-testing-only".to_string(),
            hints: BTreeMap::from([
                ("host".to_string(), "127.0.0.1".to_string()),
                ("port".to_string(), port.to_string()),
            ]),
        };
        let ours = PeerLocator {
            designator: "caller".to_string(),
            transport: "tcp-testing-only".to_string(),
            hints: BTreeMap::new(),
        };
        let connection = dialer
            .new_outgoing_connection(&node)
            .await
            .expect("dial the node");
        let identity = Identity::fresh(ours).expect("a session key");
        let client = Session::dial(connection, &identity, Arc::new(Bootstrap::default()))
            .await
            .expect("the node completes the handshake");
        assert!(
            log.0
                .lock()
                .unpoison()
                .iter()
                .any(|line| line.contains("session admitted: caller over tcp-testing-only")),
            "the operator must see who was admitted, over which transport: {:?}",
            log.0.lock().unpoison()
        );

        // The peer goes away, and the end is recorded with the same name.
        drop(client);
        let mut ended = false;
        for _ in 0..100 {
            let said = log.0.lock().unpoison().clone();
            if said
                .iter()
                .any(|line| line.contains("session ended: caller over tcp-testing-only"))
            {
                ended = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(
            ended,
            "and when the peer left: {:?}",
            log.0.lock().unpoison()
        );

        stop_tx.send(true).expect("ask the listener to stop");
        let _ = serving.await;
    }

    /// **A configured identity key is read even when nothing consumes it** (HAZOP row C238).
    ///
    /// The three cases of [`ocapn_identity_for`], each asserted rather than inferred: a listener that
    /// authenticates with the key cannot do without one; a key with no such listener is validated —
    /// so a path that does not exist is a startup error instead of a silently ignored setting — and
    /// still does not become the node's identity; and neither set is a node with no identity at all.
    #[tokio::test]
    async fn an_identity_key_with_no_listener_is_still_read_and_validated() {
        let dir = std::env::temp_dir().join(format!("ocapn-identity-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        let good = dir.join("ocapn.key");
        let truncated = dir.join("truncated.key");

        let listener = |noise: bool| OcapnListeners {
            tcp: None,
            unix: None,
            noise: noise.then(|| "127.0.0.1:0".to_string()),
            websocket: None,
        };

        // A listener that authenticates with the key needs one, and the refusal names the key to set.
        let refused = ocapn_identity_for(&listener(true), None)
            .err()
            .expect("a Noise listener without a key is a configuration the node cannot honour");
        assert!(
            refused.contains("api-server.ocapn-identity-key"),
            "the refusal names the key that fixes it: {refused}"
        );

        // With a path, the node gets an identity and the file is written for next time.
        assert!(
            ocapn_identity_for(&listener(true), Some(good.to_str().expect("utf-8")))
                .expect("a fresh identity is generated")
                .is_some(),
            "a Noise listener with a key is a node with a name"
        );
        assert!(
            good.exists(),
            "and the key is kept, so the name survives a restart"
        );

        // **No listener, and a key that is present but wrong: refused.** This is the row's subject —
        // the setting used to be ignored, so a malformed key was indistinguishable from a correct
        // one. (An absent path is not an error on either path: the file is *created* on first use,
        // which is how a node gets a stable name at all.)
        std::fs::write(&truncated, [7u8; 10]).expect("a key file of the wrong length");
        let err = ocapn_identity_for(&listener(false), Some(truncated.to_str().expect("utf-8")))
            .err()
            .expect("a configured key that cannot be read is a mistake, not a no-op");
        assert!(
            err.contains("64"),
            "and the reason says what a key file has to be: {err}"
        );

        // An *absent* file is not a mistake on either path — it is created, directory and all, which
        // is how a node gets a stable name at all. Asserted so the two cases stay distinguished.
        let fresh = dir.join("elsewhere").join("ocapn.key");
        assert!(
            ocapn_identity_for(&listener(false), Some(fresh.to_str().expect("utf-8")))
                .expect("an absent key is generated, not refused")
                .is_none(),
            "and it is still not adopted, because nothing consumes it"
        );

        // No listener and a key that *does* exist: validated, and still not this node's identity —
        // the designator does not move (the decision the row's `owes` allows).
        assert!(
            ocapn_identity_for(&listener(false), Some(good.to_str().expect("utf-8")))
                .expect("a valid key is not an error")
                .is_none(),
            "a key nothing consumes is validated and not adopted"
        );

        // Neither: a node with no identity, whose designator falls back to the pre-Noise derivation.
        assert!(ocapn_identity_for(&listener(false), None)
            .expect("no key is not an error")
            .is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **A listener bound to every address advertises a host a peer can actually dial** (HAZOP row
    /// C237).
    ///
    /// `local_addr()` is right for a specific bind and useless for `0.0.0.0`/`::`: a remote peer that
    /// follows `0.0.0.0` reaches *itself*, so every sturdyref and handoff to this node is unusable
    /// off-host. Same-host dialling happens to work, which is why nothing had measured it. The rule is
    /// the operator names the host, and a node with no name to advertise **does not start** rather
    /// than handing out locations nobody can use.
    #[tokio::test]
    async fn a_listener_bound_to_every_address_advertises_the_host_the_operator_named() {
        let log: Arc<dyn rchain_shared::log::Log> = Arc::new(RecordingLog::default());
        let source = rchain_shared::log::LogSource::new("coop.rchain.node.api.ocapn");
        let policy = DialPolicy {
            deny_local: false,
            allow: Vec::new(),
        };

        // Unnamed: refused, and the message says which key to set.
        let refused = match listen_tcp(
            "0.0.0.0:0",
            policy.clone(),
            "rnode-test",
            0,
            None,
            &log,
            source,
        )
        .await
        {
            Err(reason) => reason,
            Ok(_) => panic!("a bind to every address with nothing to advertise must be refused"),
        };
        assert!(
            refused.contains("api-server.ocapn-advertised-host"),
            "the refusal names the key that fixes it: {refused}"
        );

        // Named: advertised, and the port is the one actually chosen.
        let (_, location) = listen_tcp(
            "0.0.0.0:0",
            policy.clone(),
            "rnode-test",
            0,
            Some("node.example"),
            &log,
            source,
        )
        .await
        .expect("a bind to every address with a host to advertise is bound");
        assert_eq!(
            location.hints.get("host").map(String::as_str),
            Some("node.example"),
            "the `host` hint is the operator's, not the bound address"
        );
        assert_ne!(
            location.hints.get("port").map(String::as_str),
            Some("0"),
            "and the port is the chosen one, not the configured `:0`"
        );

        // A specific bind needs no help: it is its own answer.
        let (_, specific) = listen_tcp("127.0.0.1:0", policy, "rnode-test", 0, None, &log, source)
            .await
            .expect("a specific bind is bound");
        assert_eq!(
            specific.hints.get("host").map(String::as_str),
            Some("127.0.0.1")
        );
    }

    /// **An honest peer's establishment is not cancelled by another transport's traffic** (HAZOP row
    /// C239).
    ///
    /// The accept loop was one `tokio::select!` over an arm per transport, and a completing arm drops
    /// the other branches' futures: a websocket peer that was mid-upgrade had its already-accepted
    /// socket **dropped** the moment a connection landed on `tcp`, `unix` or `noise` — and nothing was
    /// logged, because a dropped future is not an `Err`. A `biased` select is not the fix; per-transport
    /// accept tasks are, and this is the observable.
    ///
    /// The test parks an upgrade — a socket connected to the websocket port that has sent nothing, so
    /// the node's accept is pending inside its read of the HTTP request — makes a connection on `tcp`,
    /// and then *finishes* the upgrade on the parked socket. Under the old loop that socket was already
    /// gone by then.
    #[tokio::test]
    async fn a_peers_establishment_is_not_cancelled_by_another_transports_traffic() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let log = Arc::new(RecordingLog::default());
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        let serving = tokio::spawn(serve_ocapn(
            OcapnListeners {
                tcp: Some("127.0.0.1:0".to_string()),
                unix: None,
                noise: None,
                websocket: Some("127.0.0.1:0".to_string()),
            },
            Vec::new(),
            "rnode-test".to_string(),
            false,
            Arc::new(std::sync::OnceLock::new()),
            Some(rchain_ocapn::noise::NoiseIdentity::generate().expect("a fresh identity")),
            None,
            log.clone(),
            stop_rx,
        ));

        // Both ports are `:0`, so the listener lines are the only place the chosen ones appear.
        let port_after = |said: &[String], marker: &str| -> Option<u16> {
            said.iter()
                .find_map(|line| line.split(marker).nth(1))
                .and_then(|rest| rest.split([' ', '(']).next())
                .and_then(|port| port.parse().ok())
        };
        let (tcp_port, ws_port) = loop {
            let said = log.0.lock().unpoison().clone();
            let tcp = port_after(&said, "tcp-testing-only on 127.0.0.1:");
            let ws = port_after(&said, "websocket on 127.0.0.1:");
            if let (Some(tcp), Some(ws)) = (tcp, ws) {
                break (tcp, ws);
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        };

        // 1. Park an upgrade: connect and say nothing, so the node is waiting for the HTTP request.
        let mut parked = tokio::net::TcpStream::connect(("127.0.0.1", ws_port))
            .await
            .expect("connect to the websocket listener");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // 2. Make traffic land on another transport — this is what used to drop the future above.
        let on_tcp = tokio::net::TcpStream::connect(("127.0.0.1", tcp_port))
            .await
            .expect("connect to the tcp listener");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // 3. Finish the upgrade on the parked socket. If the accept had been cancelled, the socket
        //    would be reset: the write fails, or the read comes back empty.
        let request = format!(
            "GET / HTTP/1.1\r\nHost: 127.0.0.1:{ws_port}\r\nUpgrade: websocket\r\nConnection: \
             Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: \
             13\r\n\r\n"
        );
        parked
            .write_all(request.as_bytes())
            .await
            .expect("the parked socket is still open");
        let mut answer = [0u8; 128];
        let n = tokio::time::timeout(std::time::Duration::from_secs(5), parked.read(&mut answer))
            .await
            .expect("the parked upgrade is answered, not left hanging")
            .expect("the parked socket still reads");
        let response = String::from_utf8_lossy(&answer[..n]);
        assert!(
            response.starts_with("HTTP/1.1 101"),
            "the transport another connection's traffic landed on still completes its upgrade: \
             {response:?}"
        );

        drop(on_tcp);
        stop_tx.send(true).expect("ask the listener to stop");
        let _ = serving.await;
    }

    /// **One transport cannot take the node's whole session surface** (HAZOP row C229).
    ///
    /// The ceiling used to be one transport-blind semaphore, so the transport that authenticates
    /// *nobody* — `tcp-testing-only`, or `websocket`, whose handshake has only the server prove itself
    /// — could take every permit and starve `noise`, the one that authenticates both ends. Each
    /// transport now holds its own share, and the shares **sum** to the ceiling less the dial reserve
    /// rather than nesting under it, so a single-transport node still has everything its own dials do
    /// not hold back — the two sums are exactly `MAX_SESSIONS`, which is what **C241** is about: a
    /// session the node *starts* is counted against the ceiling the operator was told, not beside it.
    ///
    /// The permit is taken at accept, **before any handshake**, so raw sockets are enough to fill a
    /// share — which is also why this is the cheapest possible proof that a share exists: the number
    /// the node refuses at is the share, and a *different* transport is still served at a moment when
    /// this one is full.
    #[tokio::test]
    async fn one_transport_cannot_take_the_nodes_whole_session_surface() {
        let log = Arc::new(RecordingLog::default());
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        let socket =
            std::env::temp_dir().join(format!("rnode-ocapn-share-{}.sock", std::process::id()));
        let serving = tokio::spawn(serve_ocapn(
            OcapnListeners {
                tcp: Some("127.0.0.1:0".to_string()),
                unix: Some(socket.display().to_string()),
                noise: None,
                websocket: None,
            },
            Vec::new(),
            "rnode-test".to_string(),
            false,
            Arc::new(std::sync::OnceLock::new()),
            None,
            None,
            log.clone(),
            stop_rx,
        ));

        let port_of = |marker: &str| -> Option<u16> {
            log.0
                .lock()
                .unpoison()
                .iter()
                .find_map(|line| line.split(marker).nth(1))
                .and_then(|rest| rest.split([' ', '(']).next())
                .and_then(|port| port.parse().ok())
        };
        let tcp_port = loop {
            if let Some(port) = port_of("tcp-testing-only on 127.0.0.1:") {
                break port;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        };

        // Two transports are configured, so a share is half of the ceiling **less the dial reserve**
        // — and asserting the number is the whole point, because it is the only thing that fails if a
        // dialed session starts being counted outside `MAX_SESSIONS` again.
        let accept_total = MAX_SESSIONS - MAX_DIALED_SESSIONS;
        let share = accept_total / 2;
        assert!(share > 1, "the share has to be a number worth asserting");

        // Fill `tcp`'s share. Each socket is held open and says nothing — the permit is taken at
        // accept, so it costs a file descriptor and nothing else.
        let mut held = Vec::new();
        for _ in 0..share {
            held.push(
                tokio::net::TcpStream::connect(("127.0.0.1", tcp_port))
                    .await
                    .expect("connect"),
            );
        }
        // The next connections are refused, and the warn names the *share*, not the whole ceiling.
        // A few extra sockets are sent because the accept task may still be working through the
        // first batch; the assertion is that the refusal lands, not on which attempt.
        let mut refused = false;
        for _ in 0..share {
            let _extra = tokio::net::TcpStream::connect(("127.0.0.1", tcp_port))
                .await
                .expect("connect");
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            let said = log.0.lock().unpoison().clone();
            if said.iter().any(|line| {
                line.contains("tcp-testing-only's session share")
                    && line.contains(&format!("({share} of {accept_total} accepts,"))
            }) {
                refused = true;
                break;
            }
        }
        assert!(
            refused,
            "a full transport must refuse at its own share and say which: {:?}",
            log.0.lock().unpoison()
        );

        // **And the other transport is untouched.** `tcp-testing-only` is holding its whole share at
        // this moment; a connection on `unix` is still accepted, which is the property that was
        // missing when the semaphore was node-wide.
        let before = log.0.lock().unpoison().len();
        let unix = tokio::net::UnixStream::connect(&socket).await;
        assert!(
            unix.is_ok(),
            "unix must still accept while tcp is at its share: {unix:?}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let after = log.0.lock().unpoison().clone();
        assert_eq!(
            before,
            after.len(),
            "and nothing on unix was refused: {after:?}"
        );

        drop(held);
        stop_tx.send(true).expect("ask the listener to stop");
        let _ = serving.await;
    }
}
