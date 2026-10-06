//! The CapTP connection: a session over a netlayer, with its tables and its loop.
//!
//! This is the connection the implementation guide describes — the tables and the loop — in the
//! shapes the reference implementation actually uses (`utils/captp.py`, `utils/captp_types.py`):
//!
//! * **The export table** — position 0 is the bootstrap object, and every object we hand a peer
//!   gets the next position. A peer addresses those with `<desc:export N>`.
//! * **The answer table** — when a peer delivers with an `answer-position`, it is asking us to make
//!   that position usable as a *recipient* for later pipelined deliveries; we remember what the
//!   delivery resolved to so a later `<desc:answer N>` reaches it.
//! * **The reply path** — a result is sent as `[<fulfill> <value>]` (or `[<break> <reason>]`) to
//!   the peer's `resolve-me-desc`, addressed as `<desc:export N>`.
//!
//! Objects do not send directly. A delivery returns an [`Act`] — outgoing messages and a reply —
//! which the loop performs. That keeps the connection single-owner (no lock around the socket, and
//! no deadlock when an object wants to talk back) at the cost of an object not being able to await
//! a reply to something it sends, which nothing here needs.
//!
//! **That last clause was false the moment an object had to open a *second* session**, which is what
//! a sturdyref enlivener is: it dials the peer a sturdyref names and fetches an object from it, so a
//! delivery on one session has to send on another and wait for the answer. [`Session::split`] is the
//! answer — the session keeps its single owner (a task running [`SessionLoop`]), and objects that
//! need to send into it keep the [`SessionHandle`]. See `owner.rs` for the loop, the handle, and the
//! crossed-hello rule that says which of two simultaneous sessions between the same peers dies.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use num_bigint::{BigUint, Sign};

use crate::captp::{
    Deliver, Desc, OpGcAnswers, OpGcExports, OpListen, DELIVER_LABEL, IMPORT_OBJECT_LABEL,
    IMPORT_PROMISE_LABEL, LISTEN_LABEL,
};
use crate::locator::PeerLocator;
use crate::netlayer::NetConn;
use crate::session::{my_location_payload, Abort, SessionError, StartSession, ABORT_LABEL};
use crate::session_id::{public_identifier_of_key, Octets32};
use crate::syrup::Value;

/// What a message did to a session's loop.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Flow {
    Continue,
    /// The peer aborted; the loop is done.
    Stop,
}

/// What an object replies with.
pub enum Reply {
    /// Nothing is owed to the peer.
    Nothing,
    /// A plain value.
    Value(Value),
    /// A new object, which the session exports and describes to the peer.
    Object(Arc<dyn Export>),
    /// Several new objects; the reply is a list of their descriptors, in order.
    Objects(Vec<Arc<dyn Export>>),
    /// **An answer this object is not ready to give** (Law 61, AUDIT C223).
    ///
    /// Some objects cannot produce their answer without waiting on something outside the session — a
    /// handoff claim whose gift has not been deposited yet is the one this port has. Waiting *inside*
    /// `handle_deliver` stalls the whole session: the loop cannot read the next message, so an
    /// unrelated delivery on the same session goes unanswered for as long as the wait lasts. So the
    /// object hands the loop a future instead and the session goes on serving; the loop writes this
    /// delivery's answer when the future lands.
    ///
    /// The observable is the *stall*, not the timeout: with a claim waiting, an unrelated delivery on
    /// the same session is answered promptly.
    Deferred(DeferredAct),
}

/// What writing an answer did.
pub(crate) enum AnswerOutcome {
    /// The answer was written, and it names this export position when it was an object (so the answer
    /// position can be pointed at it).
    Answered(Option<u64>),
    /// The export table was full, so the delivery was broken instead — and nothing may be recorded for
    /// its answer position, because there is no answer.
    Broke,
}

/// A future that produces the act an answer owes, boxed so `Reply` stays a plain enum and `dyn` so an
/// object can build it without naming its own type.
pub type DeferredAct =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<Act, String>> + Send>>;

/// What a promise does when `op:listen` arrives.
pub enum ListenOutcome {
    /// The promise is already settled; tell the listener now, with these delivery args.
    Settled(Vec<Value>),
    /// Registered; the promise will deliver when its resolver settles it.
    Registered,
    /// **Refused, with a reason that is true.** A promise at its listener cap used to have only
    /// `None` to say with, which `handle_listen` reports as "not a promise" — a false reason for an
    /// honest refusal (AUDIT C223). The reason travels as the `op:abort` this listener's session gets.
    Refused(String),
}

/// A message an object asks the session to send on its behalf.
pub struct Outgoing {
    pub to: Desc,
    pub args: Vec<Value>,
    /// Allocate an `answer-position` for this delivery — a promise the peer may pipeline onto — and
    /// report it collected at once with `op:gc-answers`, because nothing in this crate keeps an
    /// answer as a recipient.
    pub answer: bool,
    /// Export this object and offer it as the delivery's `resolve-me-desc`, so the peer can resolve
    /// the delivery into something of ours.
    pub hand_out: Option<Arc<dyn Export>>,
}

impl Outgoing {
    /// A plain message to a descriptor the object already holds.
    pub fn to(to: Desc, args: Vec<Value>) -> Outgoing {
        Outgoing {
            to,
            args,
            answer: false,
            hand_out: None,
        }
    }
}

/// The result of a delivery: messages to send first, then what to reply.
///
/// `out` exists for the objects that must *address a peer's object* — the greeter sends `Hello` to
/// the object it was handed — without giving every object the socket.
pub struct Act {
    pub out: Vec<Outgoing>,
    pub reply: Reply,
}

impl Act {
    /// No outgoing messages, no reply.
    pub fn nothing() -> Act {
        Act {
            out: Vec::new(),
            reply: Reply::Nothing,
        }
    }
    /// No outgoing messages, a value reply.
    pub fn value(v: Value) -> Act {
        Act {
            out: Vec::new(),
            reply: Reply::Value(v),
        }
    }
    /// No outgoing messages, a new object to export.
    pub fn object(o: Arc<dyn Export>) -> Act {
        Act {
            out: Vec::new(),
            reply: Reply::Object(o),
        }
    }
    /// No outgoing messages; reply with descriptors for several new objects.
    pub fn objects(os: Vec<Arc<dyn Export>>) -> Act {
        Act {
            out: Vec::new(),
            reply: Reply::Objects(os),
        }
    }
}

/// Where a chain-backed object lives, for an object that has to name it on chain rather than pass it
/// as data (C224 item 4): a registry location and the pattern that binds the object inside the value
/// registered there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Named {
    pub location: String,
    /// `None` when the registered value *is* the object.
    pub pattern: Option<String>,
}

/// What an object may ask the session about — currently one thing: **which object a descriptor the
/// peer sent refers to**. A descriptor is a position in *this session's* export table, so only the
/// session can resolve it, and only the object receiving the delivery knows whether it needs to.
pub trait ExportView: Send + Sync {
    /// The object exported at `position`, if the peer addressed one that exists.
    fn exported(&self, position: &BigUint) -> Option<Arc<dyn Export>>;
}

impl ExportView for crate::capacity::Bounded<u64, Arc<dyn Export>> {
    fn exported(&self, position: &BigUint) -> Option<Arc<dyn Export>> {
        let position = u64::try_from(position.clone()).ok()?;
        self.get(&position).cloned()
    }
}

/// A local object a peer may deliver to.
#[async_trait]
pub trait Export: Send + Sync {
    /// Handle one delivery. `Err(reason)` becomes a `break` for a peer that asked for a reply.
    async fn deliver(&self, args: &[Value]) -> Result<Act, String>;

    /// Where this object lives on chain, when it does — the answer that lets a *capability argument*
    /// be named rather than refused. A provided method, so the objects that are session-local (the
    /// fixtures, the proxy, the gift store) say nothing and stay unchanged.
    fn named(&self) -> Option<Named> {
        None
    }

    /// Handle one delivery **with a view of the session's exports**.
    ///
    /// Provided, and defaulting to [`Export::deliver`], so this changes no existing object: the ones
    /// that take a capability as an argument override it. The view is how a `desc:export N` the peer
    /// sent — a position in *our* table — becomes the object it names.
    async fn deliver_in(&self, _session: &dyn ExportView, args: &[Value]) -> Result<Act, String> {
        self.deliver(args).await
    }

    /// Register a listener for `op:listen` — a descriptor to deliver `[<fulfill …>]` or
    /// `[<break …>]` to once this promise settles. `None` means this object is not a promise, and
    /// `op:listen` on it is refused rather than silently ignored.
    fn listen(&self, _listener: Desc) -> Option<ListenOutcome> {
        None
    }
}

/// This peer's session identity: an Ed25519 key and where it accepts connections.
pub struct Identity {
    secret: [u8; 32],
    public: Vec<u8>,
    location: PeerLocator,
}

impl Identity {
    /// Derive the keypair from a 32-byte seed. The seed is the session key; OCapN generates a fresh
    /// one per session and never reuses it.
    pub fn from_seed(secret: [u8; 32], location: PeerLocator) -> Result<Identity, ConnectionError> {
        let public = rchain_crypto::signatures::ed25519::Ed25519::to_public_bytes(&secret)
            .map_err(|e| ConnectionError::Key(e.to_string()))?;
        Ok(Identity {
            secret,
            public,
            location,
        })
    }

    pub fn public_key(&self) -> &[u8] {
        &self.public
    }

    /// A fresh session key for `location`. OCapN generates a key per session and never reuses one,
    /// so a listener calls this for every accepted connection.
    pub fn fresh(location: PeerLocator) -> Result<Identity, ConnectionError> {
        use rand::Rng;
        let mut seed = [0u8; 32];
        rand::rng().fill_bytes(&mut seed);
        Identity::from_seed(seed, location)
    }

    pub fn location(&self) -> &PeerLocator {
        &self.location
    }

    /// Our side's public identifier — two SHA-256 rounds over the Syrup-encoded public-key record.
    pub fn public_identifier(&self) -> Octets32 {
        public_identifier_of_key(&public_key_record(&self.public))
    }

    /// The session secret. A third-party handoff's *receive* is signed with it.
    pub fn secret(&self) -> [u8; 32] {
        self.secret
    }

    /// Our `op:start-session`, signed over `<my-location <location>>`.
    pub fn start_session(&self) -> Result<StartSession, ConnectionError> {
        let payload = my_location_payload(&self.location).to_bytes();
        let sig = rchain_crypto::signatures::ed25519::Ed25519::sign_bytes(&payload, &self.secret)
            .map_err(|e| ConnectionError::Key(e.to_string()))?;
        Ok(StartSession {
            captp_version: crate::session::CAPTP_VERSION.to_string(),
            session_pubkey: self.public.clone(),
            acceptable_location: self.location.clone(),
            acceptable_location_sig: sig,
            // We sign over exactly what we send, so the record we keep is the one we encoded (C224
            // item 1). A peer that signs the same way is verified against *its* bytes, not a
            // re-encoding of them.
            locator_record: self.location.to_syrup(),
        })
    }
}

/// The public-key record a `StartSession` carries, as a `Value` (so the public identifier can be
/// taken over exactly what goes on the wire).
pub fn public_key_record(raw: &[u8]) -> Value {
    crate::session::public_key_syrup(raw)
}

/// An established CapTP session.
pub struct Session {
    conn: Box<dyn NetConn>,
    /// The peer's `op:start-session`, **once it has arrived**.
    ///
    /// `None` is a real state and not an error: we have sent ours and the peer has not answered. The
    /// conformance suite's crossed-hello and handoff fixtures are exactly that — they read our
    /// start-session on the leg we dial and go on without sending one — so a session has to be
    /// usable (deliveries go out, an `op:abort` can be sent) before the peer describes itself. Every
    /// field that would describe the peer is `None` until the handshake completes, rather than filled
    /// in with something that is not the peer.
    peer: Option<StartSession>,
    /// The shared session id, derived from both public identifiers — known once the peer has spoken.
    pub id: Option<Octets32>,
    /// **Bounded** (AUDIT C223): a peer grows this table by fetching and by being handed objects, so
    /// it is a [`crate::capacity::Bounded`] whose only way in checks the cap.
    exports: crate::capacity::Bounded<u64, Arc<dyn Export>>,
    /// Answer positions the peer asked us to keep usable, and the export each resolved to. Keyed by
    /// the *peer's* number, and bounded for that reason.
    answers: crate::capacity::Bounded<u64, Option<u64>>,
    next_export: u64,
    /// The next answer position to allocate for an outgoing delivery.
    next_answer: u64,
    /// Our own public-key bytes, kept because the session id needs both keys and the peer's may
    /// arrive long after ours was sent.
    own_key: Vec<u8>,
    /// The session secret. Kept because a third-party handoff's *receive* is signed with it — the
    /// give names this key, and the exporter verifies the receive against it, so a peer that could
    /// not sign with its own session key could not be handed anything.
    own_secret: [u8; 32],
    /// Our public identifier on this session, and the peer's once known. The crossed-hello rule needs
    /// *ours* on both legs and the peer's on the leg it dialed, and a handoff's `gifter_side` names
    /// ours — never the peer's on the leg we dialed.
    own_pi: Octets32,
    peer_pi: Option<Octets32>,
    /// The peer's session public-key bytes, as sent — a handoff's `receiver_key` names these.
    peer_key: Option<Vec<u8>>,
    /// True when we dialed this session, false when we accepted it.
    dialed: bool,
    /// Our `op:start-session`, written but not yet sent: a deferred accept reads the peer's first and
    /// holds ours back until the session is booked in the registry (see [`Session::announce`]).
    pending_start: Option<Value>,
    /// Answers that landed after the delivery that owed them (Law 61). A spawned waiter sends the
    /// resolved act here and the **loop** writes it, so nothing outside the loop touches the
    /// connection.
    deferred: tokio::sync::mpsc::Sender<(Deliver, Result<Act, String>)>,
    /// The receiving half of the above, taken once by [`crate::owner::split`] and owned by the loop
    /// from then on. `None` after the take; the session never reads it itself.
    deferred_rx: Option<tokio::sync::mpsc::Receiver<(Deliver, Result<Act, String>)>>,
}

impl Session {
    /// Accept a connection: the peer speaks first, then we reply — which is the order the suite's
    /// `setup_session` expects ("the peer replies with its own start-session").
    ///
    /// This answers immediately. A peer that grabs the connection the moment it reads our
    /// start-session can act before the caller has booked the session anywhere, so anything that
    /// needs the session **registered before the peer can use it** — a handoff receiver looking up
    /// the session a sturdyref names — must use [`Session::accept_deferred`] plus
    /// [`Session::announce`] instead (see `owner::accept_and_book`).
    pub async fn accept(
        conn: Box<dyn NetConn>,
        identity: &Identity,
        bootstrap: Arc<dyn Export>,
    ) -> Result<Session, ConnectionError> {
        let mut session = Session::accept_deferred(conn, identity, bootstrap).await?;
        session.announce().await?;
        Ok(session)
    }

    /// Accept a connection, reading the peer's `op:start-session` **without answering it yet**.
    ///
    /// The peer is now waiting on us, so the caller must follow with [`Session::announce`] — in the
    /// order the protocol wants, which is *after* the session has been booked in the
    /// [`crate::owner::SessionRegistry`]. Booking after answering is a race the suite caught: the
    /// peer acts on our start-session at once, so a receiver that is handed a sturdyref to this very
    /// peer would not find this session, would dial a second one, and the two would look like crossed
    /// hellos — which the register then resolves by killing the connection the peer is using.
    pub async fn accept_deferred(
        conn: Box<dyn NetConn>,
        identity: &Identity,
        bootstrap: Arc<dyn Export>,
    ) -> Result<Session, ConnectionError> {
        Self::accept_deferred_within(conn, identity, bootstrap, HANDSHAKE_TIMEOUT).await
    }

    /// [`Session::accept_deferred`] with the bound spelled out.
    ///
    /// The seam exists so the bound is **testable without waiting 30 s** — the same reason
    /// `admin_bind_host` was extracted from its spawn (AUDIT C112): a choice inside an `async` block
    /// can otherwise only be observed by starting a server and connecting to it, and a security bound
    /// nobody tests is a bound that gets removed. Production callers use the constant.
    pub async fn accept_deferred_within(
        conn: Box<dyn NetConn>,
        identity: &Identity,
        bootstrap: Arc<dyn Export>,
        handshake_timeout: std::time::Duration,
    ) -> Result<Session, ConnectionError> {
        let mut conn = conn;
        let peer = match read_start_session(&mut conn, handshake_timeout).await {
            Ok(Ok(ss)) => ss,
            Ok(Err(reason)) => {
                send_abort(&mut conn, &reason).await;
                return Err(ConnectionError::Handshake(reason));
            }
            // **A handshake we gave up on is still a handshake we say goodbye to.** The bound
            // (`HANDSHAKE_TIMEOUT`) exists so a silent connector does not pin the task; telling it why
            // costs one write on a socket that is still open, and it is the difference between a
            // refusal and a mystery. A transport failure is the one case with nothing to say.
            Err(e @ ConnectionError::Handshake(_)) => {
                send_abort(&mut conn, &e.to_string()).await;
                return Err(e);
            }
            Err(e) => return Err(e),
        };
        if !peer.location_signature_is_valid() {
            let reason = "invalid location signature".to_string();
            send_abort(&mut conn, &reason).await;
            return Err(ConnectionError::Handshake(reason));
        }
        let ours = identity.start_session()?;
        let mut session = Session::new(conn, identity, Some(peer), bootstrap, false);
        session.pending_start = Some(ours.to_syrup()?);
        Ok(session)
    }

    /// **Where the peer is**, when the transport knows (Law 62, AUDIT C225). A dial this peer asks for
    /// is judged against it: a remote peer may not make this node reach its own loopback.
    pub fn peer_address(&self) -> Option<std::net::SocketAddr> {
        self.conn.peer_address()
    }

    /// Write the held-back `op:start-session`, completing a [`Session::accept_deferred`] handshake.
    ///
    /// Idempotent: once sent there is nothing left to announce.
    pub async fn announce(&mut self) -> Result<(), ConnectionError> {
        if let Some(start) = self.pending_start.take() {
            send(&mut self.conn, &start).await?;
        }
        Ok(())
    }

    /// Dial a peer: we speak first, then read their reply.
    ///
    /// **We speak first, and that is a message we have to actually write**: the reference suite's
    /// `setup_session` sends its own `op:start-session` and then *reads*, so a dialer that only read
    /// would leave the peer waiting for a message that never comes (measured: the receiver's
    /// `setup_session` timed out on the leg we dialed for exactly this reason).
    pub async fn dial(
        conn: Box<dyn NetConn>,
        identity: &Identity,
        bootstrap: Arc<dyn Export>,
    ) -> Result<Session, ConnectionError> {
        let mut session = Session::new(conn, identity, None, bootstrap, true);
        let ours = identity.start_session()?;
        send(&mut session.conn, &ours.to_syrup()?).await?;
        session.complete_handshake().await?;
        Ok(session)
    }

    /// Dial a peer and send our `op:start-session` **without waiting for theirs**.
    ///
    /// The reference suite's crossed-hello and handoff fixtures read our start-session on the leg we
    /// dial and never send one (their `setup_session` — the only thing that sends a start-session —
    /// is not called on that leg), so waiting would deadlock the caller. What such a session can
    /// still do is everything that matters on that leg: send deliveries, and send an `op:abort` when
    /// the crossed-hello rule makes it the loser. Its `id`, `peer_identifiers` and `peer_key` are
    /// `None` until the peer answers, and a caller that needs them (a handoff naming the session id)
    /// is looking at the *other* leg — the one the peer dialed, where the handshake is complete.
    pub async fn dial_deferred(
        conn: Box<dyn NetConn>,
        identity: &Identity,
        bootstrap: Arc<dyn Export>,
    ) -> Result<Session, ConnectionError> {
        let mut session = Session::new(conn, identity, None, bootstrap, true);
        let ours = identity.start_session()?;
        send(&mut session.conn, &ours.to_syrup()?).await?;
        Ok(session)
    }

    /// Read and validate the peer's `op:start-session`, completing the handshake.
    pub async fn complete_handshake(&mut self) -> Result<(), ConnectionError> {
        if self.peer.is_some() {
            return Ok(());
        }
        let peer = match read_start_session(&mut self.conn, HANDSHAKE_TIMEOUT).await? {
            Ok(ss) => ss,
            Err(reason) => {
                send_abort(&mut self.conn, &reason).await;
                return Err(ConnectionError::Handshake(reason));
            }
        };
        self.adopt_peer(peer)
    }

    /// Take the peer's start-session as the session's own: identifiers, id, and key.
    fn adopt_peer(&mut self, peer: StartSession) -> Result<(), ConnectionError> {
        if !peer.location_signature_is_valid() {
            return Err(ConnectionError::Handshake(
                "invalid location signature".to_string(),
            ));
        }
        let (id, peer_pi) = {
            // The id needs both public keys; ours is in `own_pi`'s preimage, which `new` kept.
            let own = public_key_record(&self.own_key);
            let theirs = public_key_record(&peer.session_pubkey);
            (
                crate::session_id::session_id(&own.to_bytes(), &theirs.to_bytes()),
                public_identifier_of_key(&theirs),
            )
        };
        self.id = Some(id);
        self.peer_pi = Some(peer_pi);
        self.peer_key = Some(peer.session_pubkey.clone());
        self.peer = Some(peer);
        Ok(())
    }

    fn new(
        conn: Box<dyn NetConn>,
        identity: &Identity,
        peer: Option<StartSession>,
        bootstrap: Arc<dyn Export>,
        dialed: bool,
    ) -> Session {
        let mut exports = crate::capacity::Bounded::new(crate::capacity::MAX_EXPORTS);
        // Position 0 is the bootstrap, by definition — and it is never refused, because the cap is
        // far above one entry.
        let _ = exports.try_insert(0, bootstrap);
        // **Bounded**, like every queue on a path a peer can drive: a peer that keeps claiming gifts
        // that never arrive must not be able to grow this without limit. A full queue is not an error
        // for the waiter — see `Reply::Deferred`: the answer simply never lands, and the claim waits
        // out its own deadline, which is the behaviour it had before.
        let (deferred_tx, deferred_rx) =
            tokio::sync::mpsc::channel(crate::capacity::MAX_DEFERRED_ANSWERS);
        let mut session = Session {
            conn,
            peer: None,
            id: None,
            exports,
            answers: crate::capacity::Bounded::new(crate::capacity::MAX_ANSWERS),
            next_export: 1,
            next_answer: 0,
            own_pi: public_identifier_of_key(&public_key_record(identity.public_key())),
            own_key: identity.public_key().to_vec(),
            own_secret: identity.secret(),
            peer_pi: None,
            peer_key: None,
            dialed,
            pending_start: None,
            deferred: deferred_tx,
            deferred_rx: Some(deferred_rx),
        };
        if let Some(peer) = peer {
            // A construction-time peer is one whose start-session was already read (`accept`), so a
            // failure here cannot be retried — but its signature was validated before it got this far.
            let _ = session.adopt_peer(peer);
        }
        session
    }

    /// The peer's start-session, once it has answered.
    pub fn peer(&self) -> Option<&StartSession> {
        self.peer.as_ref()
    }

    /// The loop: read a message, act on it. Returns when the peer aborts or closes cleanly.
    pub async fn run(&mut self) -> Result<(), ConnectionError> {
        loop {
            let Some(bytes) = self.conn.recv().await? else {
                return Ok(());
            };
            let message =
                Value::from_bytes(&bytes).map_err(|e| ConnectionError::Protocol(e.to_string()))?;
            if self.handle_message(message).await? == Flow::Stop {
                return Ok(());
            }
        }
    }

    /// Act on one message. `Flow::Stop` means the peer aborted and the loop should end.
    pub(crate) async fn handle_message(&mut self, message: Value) -> Result<Flow, ConnectionError> {
        let Some(label) = record_label(&message) else {
            return Err(ConnectionError::Protocol("message is not a record".into()));
        };
        match label {
            DELIVER_LABEL => self.handle_deliver(&message).await?,
            LISTEN_LABEL => self.handle_listen(&message).await?,
            ABORT_LABEL => return Ok(Flow::Stop),
            // A peer that answers our start-session late is completing the handshake, not violating
            // the protocol: a deferred dial is legal (see `dial_deferred`). A *second* one is a
            // violation, and so is one on a session the peer never... — both are refused by
            // `adopt_peer` only for the signature, so the state check is here.
            crate::session::START_SESSION_LABEL => {
                if self.peer.is_some() {
                    let reason = "a second op:start-session on one connection".to_string();
                    send_abort(&mut self.conn, &reason).await;
                    return Err(ConnectionError::Protocol(reason));
                }
                let start = StartSession::from_syrup(&message)
                    .map_err(|e| ConnectionError::Protocol(e.to_string()))?;
                self.adopt_peer(start)?;
            }
            // **An inbound release is honoured, not ignored** (AUDIT C223). The comment that used to
            // stand here — "GC only ever releases resources; ignoring it is safe, not partial" — was
            // right that ignoring is *safe*, and wrong that it is *not partial*: the peer's explicit
            // release is the only thing that could shrink these tables, so dropping it meant the caps
            // were the whole story and a peer asking for its position back was told nothing.
            //
            // The positions are the peer's view of *our* tables: `op:gc-exports` names our exports,
            // `op:gc-answers` our answer positions. Position 0 is the bootstrap, by definition, and is
            // never released — a peer that tried would be releasing the object it is talking to.
            crate::captp::GC_EXPORTS_LABEL => {
                let gc = crate::captp::OpGcExports::from_syrup(&message)
                    .map_err(ConnectionError::Protocol)?;
                for position in &gc.positions {
                    release_position(self, position, Release::Export);
                }
            }
            crate::captp::GC_ANSWERS_LABEL => {
                let gc = crate::captp::OpGcAnswers::from_syrup(&message)
                    .map_err(ConnectionError::Protocol)?;
                for position in &gc.positions {
                    release_position(self, position, Release::Answer);
                }
            }
            other => {
                // Anything else we do not implement; say so rather than silently stall.
                let reason = format!("unsupported operation {other:?}");
                send_abort(&mut self.conn, &reason).await;
                return Err(ConnectionError::Protocol(reason));
            }
        }
        Ok(Flow::Continue)
    }

    /// Hand the session to a task that will own it, keeping a handle to send through.
    ///
    /// The session stays single-owner: the loop owns the socket and both tables, and a handle can
    /// only *ask* it to send. That is what lets an object on one session send on another without a
    /// lock around either socket (see `owner.rs`).
    pub fn split(
        self,
    ) -> (
        crate::owner::SessionHandle,
        crate::owner::SessionLoop,
        crate::owner::SessionContext,
    ) {
        crate::owner::split(self)
    }

    /// Send one delivery on this session and offer `resolve_me` as the place its fulfilment (or
    /// break) may land. Used by [`crate::owner::SessionLoop`], and by tests that drive one session
    /// by hand.
    pub(crate) async fn hand_off(
        &mut self,
        to: Desc,
        args: Vec<Value>,
        resolve_me: Option<Arc<dyn Export>>,
    ) -> Result<(), ConnectionError> {
        self.send_outgoing(Outgoing {
            to,
            args,
            answer: false,
            hand_out: resolve_me,
        })
        .await
    }

    /// Send an `op:abort` and stop. The crossed-hello rule's losing connection does this.
    pub(crate) async fn abort_with(&mut self, reason: &str) {
        send_abort(&mut self.conn, reason).await;
    }

    /// Our public identifier on this session, and the peer's if it has answered. The crossed-hello
    /// rule compares *ours* on both legs, so ours is always here.
    pub fn public_identifiers(&self) -> (&Octets32, Option<&Octets32>) {
        (&self.own_pi, self.peer_pi.as_ref())
    }

    /// The peer's session public-key bytes, as it sent them — `None` until it answers.
    pub fn peer_key(&self) -> Option<&[u8]> {
        self.peer_key.as_deref()
    }

    /// The peer's key in the form the wire uses for a public key — what a handoff's `receiver_key`
    /// names.
    pub fn peer_key_record(&self) -> Option<Value> {
        self.peer_key.as_deref().map(public_key_record)
    }

    /// The session secret, for the one thing that must sign with it: a handoff receive. The give
    /// names this session's public key, and the exporter checks the receive against it.
    pub fn secret(&self) -> [u8; 32] {
        self.own_secret
    }

    /// Whether we dialed this session (as opposed to accepting it). The crossed-hello rule compares
    /// the two *dialing* sides of a crossing, so it has to know which session is ours.
    pub fn is_dialed(&self) -> bool {
        self.dialed
    }

    /// Send a raw CapTP message. The loop uses the typed paths; this is for drivers and tests that
    /// need to assert on the exact wire shape.
    pub async fn send_message(&mut self, message: &Value) -> Result<(), ConnectionError> {
        send(&mut self.conn, message).await
    }

    /// Read one message without acting on it.
    pub async fn recv_message(&mut self) -> Result<Option<Value>, ConnectionError> {
        match self.conn.recv().await? {
            Some(bytes) => Ok(Some(
                Value::from_bytes(&bytes).map_err(|e| ConnectionError::Protocol(e.to_string()))?,
            )),
            None => Ok(None),
        }
    }

    async fn handle_deliver(&mut self, message: &Value) -> Result<(), ConnectionError> {
        let deliver =
            Deliver::from_syrup(message).map_err(|e| ConnectionError::Protocol(e.to_string()))?;

        // The delivery's import descriptors are released as soon as it is handled, and the peer is
        // told now — before anything is sent back, so the accounting never depends on the reply.
        self.gc_released(&deliver.args).await?;

        let target = match resolve_to(&self.exports, &self.answers, &deliver.to)? {
            Resolution::Object(o) => o,
            Resolution::Broken(reason) => {
                // A delivery pipelined onto an answer that broke must itself break.
                self.fulfil(
                    deliver.resolve_me_desc.as_ref(),
                    vec![Value::Symbol("break".to_string()), Value::String(reason)],
                )
                .await?;
                return Ok(());
            }
            Resolution::Missing => {
                let reason = "no such export or answer".to_string();
                send_abort(&mut self.conn, &reason).await;
                return Err(ConnectionError::Protocol(reason));
            }
        };

        // **With the session's exports in view** (C224 item 4): an object that can name a capability
        // on chain needs to know which object a `desc:export N` the peer sent refers to, and only the
        // session holds that table. Every other object ignores the view.
        let act = match target.deliver_in(&self.exports, &deliver.args).await {
            Ok(act) => act,
            Err(reason) => {
                // The answer this delivery would have filled is now broken, so a later pipelined
                // delivery onto it breaks too.
                if let Some(n) = &deliver.answer_pos {
                    let _ = self.answers.try_insert(position(n)?, None);
                }
                // A break is `[<break> <reason>]`, sent to the peer's resolver if it left one.
                self.fulfil(
                    deliver.resolve_me_desc.as_ref(),
                    vec![Value::Symbol("break".to_string()), Value::String(reason)],
                )
                .await?;
                return Ok(());
            }
        };

        // **An answer the object could not produce yet keeps the loop free** (Law 61, AUDIT C223).
        // The out-messages go now — they are what the object wanted to say whatever the answer turns
        // out to be — and the answer itself is written by the loop when the waiter lands. The wait
        // used to happen *inside* `deliver_in` above, which is what stalled the session: `handle_deliver`
        // is awaited on the loop task, so nothing else on this session was read while a claim waited
        // out its ten seconds.
        if let Reply::Deferred(fut) = act.reply {
            for outgoing in act.out {
                self.send_outgoing(outgoing).await?;
            }
            // Checked now — an abort must be the first thing the peer sees — and recorded with **no**
            // object yet: a pipelined delivery to this position breaks rather than resolving, which is
            // the honest answer while the answer itself is unknown. Re-recording it when the waiter
            // lands would be the silent re-point AUDIT C223 refuses.
            self.check_answer(&deliver).await?;
            self.record_answer(&deliver, None);
            let tx = self.deferred_sender();
            let deliver = deliver.clone();
            tokio::spawn(async move {
                let act = fut.await;
                // A full queue loses the answer rather than the session (see
                // `MAX_DEFERRED_ANSWERS`): the claim times out exactly as it did before the deferral.
                let _ = tx.send((deliver, act)).await;
            });
            return Ok(());
        }

        // What we owe the peer, and what a later pipelined delivery to this answer must reach.
        self.check_answer(&deliver).await?;
        match self.answer(&deliver, act).await? {
            AnswerOutcome::Answered(target) => self.record_answer(&deliver, target),
            AnswerOutcome::Broke => {}
        }
        Ok(())
    }

    async fn handle_listen(&mut self, message: &Value) -> Result<(), ConnectionError> {
        let listen =
            OpListen::from_syrup(message).map_err(|e| ConnectionError::Protocol(e.to_string()))?;
        let target = match resolve_to(&self.exports, &self.answers, &listen.to)? {
            Resolution::Object(o) => o,
            Resolution::Broken(_) | Resolution::Missing => {
                let reason = "op:listen names no object".to_string();
                send_abort(&mut self.conn, &reason).await;
                return Err(ConnectionError::Protocol(reason));
            }
        };
        let listener = match &listen.resolve_me_desc {
            Desc::ImportObject(n) | Desc::ImportPromise(n) => Desc::Export(n.clone()),
            _ => {
                let reason = "op:listen needs an import descriptor to notify".to_string();
                send_abort(&mut self.conn, &reason).await;
                return Err(ConnectionError::Protocol(reason));
            }
        };
        match target.listen(listener.clone()) {
            // Already settled: the notification is owed now.
            Some(ListenOutcome::Settled(args)) => self.send_deliver(listener, args).await?,
            // Registered; the promise will speak up when its resolver settles it.
            Some(ListenOutcome::Registered) => {}
            Some(ListenOutcome::Refused(reason)) => {
                send_abort(&mut self.conn, &reason).await;
                return Err(ConnectionError::Protocol(reason));
            }
            None => {
                let reason = "op:listen on something that is not a promise".to_string();
                send_abort(&mut self.conn, &reason).await;
                return Err(ConnectionError::Protocol(reason));
            }
        }
        Ok(())
    }

    /// Tell the peer we have released every import descriptor a delivery carried.
    ///
    /// Nothing in this crate retains a reference it is handed: an object's only way to keep one is
    /// to store the descriptor itself, and the command it may carry in an [`Act`] is a descriptor
    /// it chose, not one it was given. So a delivery's imports are released as soon as it is
    /// handled, and the release is reported at once. An object that *did* retain would need a
    /// signal here that it had, and would then be over-collecting.
    async fn gc_released(&mut self, args: &[Value]) -> Result<(), ConnectionError> {
        let mut counts: BTreeMap<u64, u64> = BTreeMap::new();
        for arg in args {
            count_imports(arg, &mut counts);
        }
        if counts.is_empty() {
            return Ok(());
        }
        let message = OpGcExports {
            positions: counts.keys().copied().map(BigUint::from).collect(),
            wire_deltas: counts.values().copied().map(BigUint::from).collect(),
        }
        .to_syrup();
        send(&mut self.conn, &message).await
    }

    /// Send `[<fulfill> …]` — or nothing, when the peer asked for no reply.
    async fn fulfil(
        &mut self,
        resolve_me_desc: Option<&Desc>,
        args: Vec<Value>,
    ) -> Result<(), ConnectionError> {
        if let Some(Desc::ImportObject(p) | Desc::ImportPromise(p)) = resolve_me_desc {
            self.send_deliver(Desc::Export(p.clone()), args).await?;
        }
        Ok(())
    }

    /// **Write the answer a delivery is owed** — what its `resolve-me-desc` receives, and what a later
    /// pipelined delivery to its answer position reaches. Returns the export position the answer named
    /// if it was an object, so the caller can point the answer position at it.
    ///
    /// Split out of [`Session::handle_deliver`] so a **deferred** answer (Law 61, AUDIT C223) is
    /// written by the same code as an immediate one rather than by a second implementation that could
    /// drift. The answer position is *not* booked here: the immediate path books it with the position
    /// this returns, and the deferred path booked it when it deferred (with no object yet).
    pub(crate) async fn answer(
        &mut self,
        deliver: &Deliver,
        act: Act,
    ) -> Result<AnswerOutcome, ConnectionError> {
        for outgoing in act.out {
            self.send_outgoing(outgoing).await?;
        }
        let (reply, answer_target) = match act.reply {
            Reply::Nothing => (None, None),
            Reply::Value(v) => (Some(v), None),
            // **A full export table breaks this delivery rather than dropping the object.** The
            // alternative — exporting nothing and answering as though it had — would hand the peer a
            // descriptor for a position that does not exist, which is the C18 class: it fails
            // silently instead of loudly. The answer is a `break` naming the bound (AUDIT C223).
            Reply::Object(o) => match self.insert_export(o) {
                Ok(pos) => (Some(Desc::ImportObject(pos.into()).to_syrup()), Some(pos)),
                Err(reason) => {
                    break_delivery(&mut self.conn, deliver, reason).await?;
                    return Ok(AnswerOutcome::Broke);
                }
            },
            Reply::Objects(os) => {
                let mut parts = Vec::with_capacity(os.len());
                for o in os {
                    match self.insert_export(o) {
                        Ok(pos) => parts.push(Desc::ImportObject(pos.into()).to_syrup()),
                        Err(reason) => {
                            break_delivery(&mut self.conn, deliver, reason).await?;
                            return Ok(AnswerOutcome::Broke);
                        }
                    }
                }
                (Some(Value::List(parts)), None)
            }
            // An answer cannot defer an answer: the deferral exists so the *loop* is free, and a
            // deferred answer that deferred again would leave nothing to write.
            Reply::Deferred(_) => {
                return Err(ConnectionError::Protocol(
                    "a deferred answer cannot itself be deferred".to_string(),
                ))
            }
        };
        if let Some(value) = reply {
            self.fulfil(
                deliver.resolve_me_desc.as_ref(),
                vec![Value::Symbol("fulfill".to_string()), value],
            )
            .await?;
        }
        Ok(AnswerOutcome::Answered(answer_target))
    }

    /// **Refuse a re-used answer position before anything is written for this delivery.**
    ///
    /// An answer position is the sender's own promise slot: the peer hands out `desc:answer N` to
    /// third parties, so silently re-pointing N at a different delivery breaks a reference the peer
    /// (or someone it told) may still hold. Before this check, `try_insert` replaced the mapping and
    /// nothing said so.
    ///
    /// Checked *before* the answer is written rather than after: an abort must be the first thing the
    /// peer sees for the offending delivery, not something that arrives behind a `fulfill` it will
    /// act on.
    pub(crate) async fn check_answer(&mut self, deliver: &Deliver) -> Result<(), ConnectionError> {
        let Some(n) = &deliver.answer_pos else {
            return Ok(());
        };
        let pos = position(n)?;
        if self.answers.contains_key(&pos) {
            let reason = format!("answer position {pos} was already used on this session");
            send_abort(&mut self.conn, &reason).await;
            return Err(ConnectionError::Protocol(reason));
        }
        Ok(())
    }

    /// Record what a delivery's answer position resolved to, so a later pipelined delivery reaches it.
    ///
    /// A **full** table is not an error: the peer chose the position, and one it cannot have is one a
    /// later delivery cannot pipeline onto — which the `Resolution::Missing` path reports as
    /// "no such export or answer".
    pub(crate) fn record_answer(&mut self, deliver: &Deliver, target: Option<u64>) {
        let Some(n) = &deliver.answer_pos else {
            return;
        };
        let Ok(pos) = position(n) else {
            return;
        };
        let _ = self.answers.try_insert(pos, target);
    }

    /// Tell the peer a **deferred** answer broke. The same `[<break> <reason>]` an immediate refusal
    /// sends, written by the loop when the waiter fails rather than by the object that started it.
    pub(crate) async fn break_answer(
        &mut self,
        deliver: &Deliver,
        reason: String,
    ) -> Result<(), ConnectionError> {
        self.fulfil(
            deliver.resolve_me_desc.as_ref(),
            vec![Value::Symbol("break".to_string()), Value::String(reason)],
        )
        .await
    }

    /// Take the receiving half of the deferred-answer channel. [`crate::owner::split`] calls this
    /// once; the session never reads it, because only the loop writes to the connection.
    pub(crate) fn take_deferred(
        &mut self,
    ) -> Option<tokio::sync::mpsc::Receiver<(Deliver, Result<Act, String>)>> {
        self.deferred_rx.take()
    }

    /// Hand a resolved deferred answer to the loop.
    pub(crate) fn deferred_sender(
        &self,
    ) -> tokio::sync::mpsc::Sender<(Deliver, Result<Act, String>)> {
        self.deferred.clone()
    }

    /// Give an object a position in the export table, or refuse because the table is full.
    ///
    /// `Err` rather than a silent wrap: a table at its cap means this session has handed the peer as
    /// many objects as it may hold, and the honest answer is a `break` naming the bound (AUDIT C223).
    fn insert_export(&mut self, object: Arc<dyn Export>) -> Result<u64, String> {
        let pos = self.next_export;
        self.exports
            .try_insert(pos, object)
            .map_err(|full| format!("this session's export table is full ({full})"))?;
        self.next_export += 1;
        Ok(pos)
    }

    /// Send one of an object's outgoing messages, exporting anything it hands over and allocating
    /// an answer if it asked for one.
    ///
    /// An answer is reported collected immediately (`op:gc-answers`): this crate never keeps one as
    /// a recipient for the peer to pipeline onto, so there is nothing to wait for. An implementation
    /// that did keep answers would collect them when the last reference went, not here.
    async fn send_outgoing(&mut self, outgoing: Outgoing) -> Result<(), ConnectionError> {
        let resolve_me_desc = match outgoing.hand_out {
            Some(object) => Some(Desc::ImportObject(
                self.insert_export(object)
                    .map_err(ConnectionError::Protocol)?
                    .into(),
            )),
            None => None,
        };
        let answer_pos = if outgoing.answer {
            let pos = self.next_answer;
            self.next_answer += 1;
            Some(pos)
        } else {
            None
        };
        let deliver = Deliver {
            to: outgoing.to,
            args: outgoing.args,
            answer_pos: answer_pos.map(BigUint::from),
            resolve_me_desc,
        };
        send(&mut self.conn, &deliver.to_syrup()).await?;
        if let Some(pos) = answer_pos {
            let gc = OpGcAnswers {
                positions: vec![BigUint::from(pos)],
            }
            .to_syrup();
            send(&mut self.conn, &gc).await?;
        }
        Ok(())
    }

    async fn send_deliver(&mut self, to: Desc, args: Vec<Value>) -> Result<(), ConnectionError> {
        let deliver = Deliver {
            to,
            args,
            answer_pos: None,
            resolve_me_desc: None,
        };
        send(&mut self.conn, &deliver.to_syrup()).await
    }
}

/// Count every import descriptor anywhere in a delivered value, keyed by the peer's export
/// position — the wire-delta accounting `op:gc-exports` reports.
fn count_imports(value: &Value, counts: &mut BTreeMap<u64, u64>) {
    match value {
        Value::Record(fields) => {
            if let Some(Value::Symbol(label)) = fields.first() {
                if label == IMPORT_OBJECT_LABEL || label == IMPORT_PROMISE_LABEL {
                    if let Some(Value::Int(n)) = fields.get(1) {
                        if n.sign() != Sign::Minus {
                            if let Ok(pos) = u64::try_from(n.magnitude().clone()) {
                                *counts.entry(pos).or_default() += 1;
                            }
                        }
                    }
                }
            }
            for field in fields {
                count_imports(field, counts);
            }
        }
        Value::List(xs) => {
            for x in xs {
                count_imports(x, counts);
            }
        }
        Value::Struct(m) => {
            for v in m.values() {
                count_imports(v, counts);
            }
        }
        _ => {}
    }
}

/// What a `to` descriptor names.
enum Resolution {
    Object(Arc<dyn Export>),
    /// An answer whose delivery broke; a pipelined delivery onto it must break too.
    Broken(String),
    /// No such export or answer.
    Missing,
}

/// Which table an inbound release names.
#[derive(Clone, Copy)]
enum Release {
    Export,
    Answer,
}

/// Honour one position from an inbound `op:gc-*` (AUDIT C223).
///
/// **Position 0 is never released.** It is the bootstrap on every session, by definition, and the
/// only object whose absence would leave the peer holding a reference to the thing it is talking to.
/// A peer that names it is asking for something the protocol cannot give, and the release is dropped
/// rather than obeyed — the one case where refusing to act is the safe reading.
///
/// A position that is not there is not an error: a peer may release something twice (a re-delivery it
/// already accounted for), and a `BTreeMap::remove` of an absent key is exactly that no-op.
fn release_position(session: &mut Session, position: &num_bigint::BigUint, which: Release) {
    let Ok(pos) = u64::try_from(position.clone()) else {
        return;
    };
    if pos == crate::captp::BOOTSTRAP_POSITION {
        return;
    }
    match which {
        Release::Export => {
            session.exports.remove(&pos);
        }
        Release::Answer => {
            session.answers.remove(&pos);
        }
    }
}

/// Break one delivery — `[<break> <reason>]` to the peer's resolver, when it left one.
///
/// Extracted for the tables-at-capacity refusals (AUDIT C223): a delivery we cannot answer *properly*
/// is answered *honestly*, and the reason names the bound it hit. The same shape `handle_deliver` uses
/// when an object refuses.
async fn break_delivery(
    conn: &mut Box<dyn NetConn>,
    deliver: &Deliver,
    reason: String,
) -> Result<(), ConnectionError> {
    if let Some(Desc::ImportObject(p) | Desc::ImportPromise(p)) = &deliver.resolve_me_desc {
        let break_message = Deliver {
            to: Desc::Export(p.clone()),
            args: vec![Value::Symbol("break".to_string()), Value::String(reason)],
            answer_pos: None,
            resolve_me_desc: None,
        };
        send(conn, &break_message.to_syrup()).await?;
    }
    Ok(())
}

fn resolve_to(
    exports: &crate::capacity::Bounded<u64, Arc<dyn Export>>,
    answers: &crate::capacity::Bounded<u64, Option<u64>>,
    to: &Desc,
) -> Result<Resolution, ConnectionError> {
    match to {
        Desc::Export(n) => Ok(match exports.get(&position(n)?) {
            Some(o) => Resolution::Object(o.clone()),
            None => Resolution::Missing,
        }),
        Desc::Answer(n) => Ok(match answers.get(&position(n)?) {
            Some(Some(pos)) => match exports.get(pos) {
                Some(o) => Resolution::Object(o.clone()),
                None => Resolution::Missing,
            },
            Some(None) => Resolution::Broken("the delivery it pipelined onto broke".to_string()),
            None => Resolution::Missing,
        }),
        _ => Ok(Resolution::Missing),
    }
}

/// The shared session id, from our identifier and the peer's record on the wire.
fn position(n: &num_bigint::BigUint) -> Result<u64, ConnectionError> {
    u64::try_from(n.clone()).map_err(|_| ConnectionError::Protocol("position too large".into()))
}

fn record_label(v: &Value) -> Option<&str> {
    match v {
        Value::Record(fields) => match fields.first() {
            Some(Value::Symbol(s)) => Some(s.as_str()),
            _ => None,
        },
        _ => None,
    }
}

/// Read the peer's `op:start-session`, **bounded** — the one await in this crate that a peer can
/// hold open for nothing.
///
/// **Why this await and not the steady-state read.** A peer that connects and sends nothing pins a
/// task for ever (`Session::accept_deferred` → here, and the node spawns one task per connection);
/// that is a slow-loris, and it is this await. A steady-state read is deliberately *not* bounded: an
/// idle session is legitimate, and the conformance suite has three tests that need one leg to stay
/// silent (including `test_valid_handoff_wait_deposit_gift`, which withdraws and then makes the peer
/// wait for its deposit). Handshakes are different — no correct peer waits to speak.
///
/// **The bound must be shorter than the peer's own.** The reference client reads its handshake with a
/// 60 s timeout (`utils/captp.py`), so anything at or above that leaves *us* hanging while it gives
/// up. 30 s is under it with margin and far above any real handshake.
pub const HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

async fn read_start_session(
    conn: &mut Box<dyn NetConn>,
    handshake_timeout: std::time::Duration,
) -> Result<Result<StartSession, String>, ConnectionError> {
    let Some(bytes) = tokio::time::timeout(handshake_timeout, conn.recv())
        .await
        .map_err(|_| {
            ConnectionError::Handshake("the peer did not speak first in time".to_string())
        })??
    else {
        return Err(ConnectionError::Protocol(
            "connection closed during handshake".into(),
        ));
    };
    let message =
        Value::from_bytes(&bytes).map_err(|e| ConnectionError::Protocol(e.to_string()))?;
    match StartSession::from_syrup(&message) {
        Ok(ss) => Ok(Ok(ss)),
        Err(SessionError::UnsupportedVersion(v)) => {
            Ok(Err(format!("unsupported captp-version {v:?}")))
        }
        Err(e) => Err(ConnectionError::Protocol(e.to_string())),
    }
}

async fn send(conn: &mut Box<dyn NetConn>, value: &Value) -> Result<(), ConnectionError> {
    conn.send(&value.to_bytes()).await.map_err(Into::into)
}

async fn send_abort(conn: &mut Box<dyn NetConn>, reason: &str) {
    let abort = Abort {
        reason: reason.to_string(),
    };
    // Best effort: the peer may already be gone.
    let _ = conn.send(&abort.to_syrup().to_bytes()).await;
}

/// A connection-level failure.
#[derive(Debug)]
pub enum ConnectionError {
    /// The socket failed.
    Transport(std::io::Error),
    /// A handshake was refused; the reason is what the peer was told.
    Handshake(String),
    /// A message was not what CapTP says it should be.
    Protocol(String),
    /// The session key could not be derived or used.
    Key(String),
    /// The session this referred to has ended — its loop dropped, or the peer closed.
    Closed,
}

impl fmt::Display for ConnectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConnectionError::Transport(e) => write!(f, "ocapn transport: {e}"),
            ConnectionError::Handshake(r) => write!(f, "ocapn handshake refused: {r}"),
            ConnectionError::Protocol(r) => write!(f, "ocapn protocol error: {r}"),
            ConnectionError::Key(r) => write!(f, "ocapn session key: {r}"),
            ConnectionError::Closed => write!(f, "ocapn session ended"),
        }
    }
}

impl std::error::Error for ConnectionError {}

impl From<std::io::Error> for ConnectionError {
    fn from(e: std::io::Error) -> Self {
        ConnectionError::Transport(e)
    }
}

impl From<SessionError> for ConnectionError {
    fn from(e: SessionError) -> Self {
        ConnectionError::Protocol(e.to_string())
    }
}
