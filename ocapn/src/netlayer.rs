//! The netlayer abstraction: a bidirectional FIFO between two peers.
//!
//! `draft-specifications/Netlayers.md` and the implementation guide present two functions —
//! `new_outgoing_connection(ocapn_locator)` and `accept_incoming_connection()` — over a channel
//! that is "a bidirectional FIFO". CapTP stays "agnostic to latency, liveness, and privacy
//! characteristics of each underlying protocol": those are the netlayer's business, and the
//! protocol above sees only a queue of messages.
//!
//! A message on the channel is one CapTP operation, carried as one Syrup value. The netlayers
//! implemented here are [`crate::tcp_testing_only`] — the conformance suite's transport — and
//! [`crate::unix`], a domain socket authenticated by the socket's file mode, with [`crate::multi`]
//! dispatching a dial by the locator's transport name.
//!
//! **A Noise transport is deliberately not built.** Noise is the one OCapN names for a real
//! deployment, and a third layer is cheap (unit 2's dispatcher exists to make it so) — but OCapN is
//! still pre-specification and **neither reference implementation in reach speaks Noise**: the
//! conformance suite at `31f0b80` offers `testing_only_tcp` and `onion`, and the Endo version vendored
//! for the spike (`1.1.1`) offers `tcp-test-only` and `websocket`. A Noise layer here would therefore
//! talk only to itself — it could not be interop-tested, and its parameters (the pattern, the
//! prologue, and how the static key relates to the Ed25519 session identity) would be guesses rather
//! than a specification. **The gate that unblocks it is a reference that speaks it.** Until then the
//! transport to add is the one the peers you care about actually speak, and a production netlayer
//! (Tor, libp2p, IBC) implements the same two functions with nothing above the seam moving.

use std::io;

use async_trait::async_trait;

use crate::locator::PeerLocator;

/// One open channel to a peer.
#[async_trait]
pub trait NetConn: Send {
    /// Write one CapTP message — a single Syrup value's bytes.
    async fn send(&mut self, message: &[u8]) -> io::Result<()>;

    /// **The peer's own address**, when the transport knows it (Law 62, AUDIT C225).
    ///
    /// The dial policy needs it to tell a *remote* peer from a local one: a peer on another host must
    /// not be able to make this node reach the node's own loopback services. A transport that cannot
    /// say is unchanged — the default is `None`, which the policy reads as "cannot be judged".
    fn peer_address(&self) -> Option<std::net::SocketAddr> {
        None
    }

    /// Read one CapTP message. `Ok(None)` is a clean end of stream *at a message boundary*; a
    /// stream that ends in the middle of a message is an error, never a silently dropped message.
    async fn recv(&mut self) -> io::Result<Option<Vec<u8>>>;
}

/// How a peer is dialled, and how it accepts dials — the two functions the netlayer standard
/// fixes.
#[async_trait]
pub trait Netlayer: Send + Sync {
    /// `new_outgoing_connection(ocapn_locator)` — dial the peer the locator names.
    async fn new_outgoing_connection(&self, locator: &PeerLocator) -> io::Result<Box<dyn NetConn>>;

    /// The same dial, told **which peer asked for it** (Law 62): `origin` is the address of the
    /// session the request arrived on, when the transport knows it. A dial a *remote* peer asked for
    /// is refused when the target is one of this node's own local addresses, so a peer cannot use this
    /// node to reach what the peer itself cannot.
    ///
    /// **Provided, and defaulting to the origin-free dial.** The netlayer standard fixes two
    /// functions, and a transport that cannot report an origin must stay a netlayer — so this is a
    /// refinement [`crate::dial_policy::PolicyNetlayer`] opts into, not a third function every
    /// implementation has to write.
    async fn new_outgoing_connection_from(
        &self,
        locator: &PeerLocator,
        _origin: Option<std::net::SocketAddr>,
    ) -> io::Result<Box<dyn NetConn>> {
        self.new_outgoing_connection(locator).await
    }

    /// `accept_incoming_connection()` — await the next peer that dials us.
    async fn accept_incoming_connection(&self) -> io::Result<Box<dyn NetConn>>;
}
