//! The dialing dispatcher: a locator names its transport, and this routes the dial to it.
//!
//! A node that speaks more than one transport needs exactly one thing from a wrapper — when something
//! asks to dial `ocapn://peer.unix?path=…`, the Unix socket should be the one that answers. The
//! locator carries that name ([`crate::locator`]'s `transport` field, which is also half of the
//! protocol's peer identity), so the routing is a lookup and an unknown name is a refusal that says
//! what the node *does* speak rather than a silent fallback to whichever layer was registered first.
//!
//! **Outbound only, and that is not a shortcut.** `Netlayer::accept_incoming_connection` is handed no
//! locator — that is the trait's shape, taken from the protocol's own two functions — so a layer that
//! multiplexed accepts could not tell its caller **which** transport accepted. The caller needs to
//! know: an accepted session advertises a location, and the honest one is the transport it arrived on.
//! So a node accepts on each configured listener in its own right, and this dispatches dials, where the
//! locator is in hand. [`MultiNetlayer::accept_incoming_connection`] therefore refuses, with that
//! reason, rather than pretending.

use std::collections::BTreeMap;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;

use crate::locator::PeerLocator;
use crate::netlayer::{NetConn, Netlayer};

/// The transports a node dials, keyed by the name a locator writes after the dot.
#[derive(Default)]
pub struct MultiNetlayer {
    layers: BTreeMap<String, Arc<dyn Netlayer>>,
}

impl MultiNetlayer {
    pub fn new() -> MultiNetlayer {
        MultiNetlayer::default()
    }

    /// Add a transport. The name is the locator's `transport`: `"unix"` answers
    /// `ocapn://peer.unix?path=…`, `"tcp-testing-only"` answers `ocapn://peer.tcp-testing-only?host=…`.
    pub fn with(mut self, transport: impl Into<String>, layer: Arc<dyn Netlayer>) -> MultiNetlayer {
        self.layers.insert(transport.into(), layer);
        self
    }

    /// The transports configured, in name order. For a reader of the refusal below.
    pub fn transports(&self) -> Vec<&str> {
        self.layers.keys().map(String::as_str).collect()
    }

    /// The layer for a transport, or a refusal naming the ones there are.
    fn layer(&self, transport: &str) -> io::Result<&Arc<dyn Netlayer>> {
        self.layers.get(transport).ok_or_else(|| {
            let speaks = if self.layers.is_empty() {
                "nothing".to_string()
            } else {
                self.layers.keys().cloned().collect::<Vec<_>>().join(", ")
            };
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("no netlayer for transport `{transport}`; this node speaks {speaks}"),
            )
        })
    }
}

#[async_trait]
impl Netlayer for MultiNetlayer {
    async fn new_outgoing_connection(&self, locator: &PeerLocator) -> io::Result<Box<dyn NetConn>> {
        self.layer(&locator.transport)?
            .new_outgoing_connection(locator)
            .await
    }

    /// The origin-carrying dial dispatches the same way, and **to the layer's own override** — the
    /// dial policy is applied per transport (a `unix` layer's origin is `None` where a TCP layer's is
    /// a socket address), so routing to the layer and letting it decide keeps that decision in one
    /// place rather than re-deriving it here.
    async fn new_outgoing_connection_from(
        &self,
        locator: &PeerLocator,
        origin: Option<SocketAddr>,
    ) -> io::Result<Box<dyn NetConn>> {
        self.layer(&locator.transport)?
            .new_outgoing_connection_from(locator, origin)
            .await
    }

    async fn accept_incoming_connection(&self) -> io::Result<Box<dyn NetConn>> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "the dispatcher dials only: `accept_incoming_connection` is handed no locator, so it could \
             not tell its caller which transport accepted — which is what an accepted session needs to \
             advertise. Accept on each configured listener instead.",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A layer that refuses every dial with its own name, so a test can tell *which* layer was
    /// reached without standing up a socket.
    struct Stub(&'static str);

    #[async_trait]
    impl Netlayer for Stub {
        async fn new_outgoing_connection(&self, _: &PeerLocator) -> io::Result<Box<dyn NetConn>> {
            Err(io::Error::other(self.0))
        }

        async fn accept_incoming_connection(&self) -> io::Result<Box<dyn NetConn>> {
            Err(io::Error::other(self.0))
        }
    }

    /// `expect_err` wants the success side to be `Debug`, and a `Box<dyn NetConn>` is not; take the
    /// error out by hand.
    fn refused(result: io::Result<Box<dyn NetConn>>) -> io::Error {
        match result {
            Err(e) => e,
            Ok(_) => panic!("every stub refuses, so a dial cannot succeed"),
        }
    }

    fn locator(transport: &str) -> PeerLocator {
        PeerLocator {
            designator: "peer".into(),
            transport: transport.into(),
            hints: Default::default(),
        }
    }

    fn dispatcher() -> MultiNetlayer {
        MultiNetlayer::new()
            .with("one", Arc::new(Stub("one")))
            .with("two", Arc::new(Stub("two")))
    }

    /// **The dispatch is the point**: a locator naming `two` reaches `two`, not whichever layer
    /// happened to be registered first.
    #[tokio::test]
    async fn a_locator_reaches_the_transport_it_names() {
        let multi = dispatcher();
        for name in ["one", "two"] {
            let err = refused(multi.new_outgoing_connection(&locator(name)).await);
            assert_eq!(
                err.to_string(),
                name,
                "the wrong layer answered for `{name}`"
            );
        }
        assert_eq!(multi.transports(), vec!["one", "two"]);
    }

    /// The origin-carrying dial routes identically — it is what the dialing fixtures call, so a
    /// dispatcher that only handled the plain one would work everywhere except in production.
    #[tokio::test]
    async fn the_origin_carrying_dial_dispatches_the_same_way() {
        let multi = dispatcher();
        let err = refused(
            multi
                .new_outgoing_connection_from(&locator("two"), Some("127.0.0.1:1".parse().unwrap()))
                .await,
        );
        assert_eq!(err.to_string(), "two");
    }

    /// An unknown transport names what the node speaks, rather than failing with a blank reason or
    /// falling through to a default.
    #[tokio::test]
    async fn an_unknown_transport_names_what_the_node_speaks() {
        let err = refused(
            dispatcher()
                .new_outgoing_connection(&locator("noise"))
                .await,
        );
        let reason = err.to_string();
        assert!(reason.contains("noise"), "{reason}");
        assert!(reason.contains("one, two"), "{reason}");

        // And a node with nothing configured says so rather than printing an empty list.
        let empty = MultiNetlayer::new();
        let reason = refused(empty.new_outgoing_connection(&locator("unix")).await).to_string();
        assert!(reason.contains("speaks nothing"), "{reason}");
    }

    /// Accepting through the dispatcher is refused with the reason, not answered from an arbitrary
    /// layer — the caller would advertise the wrong transport.
    #[tokio::test]
    async fn the_dispatcher_refuses_to_accept() {
        let err = match dispatcher().accept_incoming_connection().await {
            Err(e) => e,
            Ok(_) => panic!("the dispatcher dials only, and must not accept"),
        };
        assert_eq!(err.kind(), io::ErrorKind::Unsupported);
        assert!(
            err.to_string().contains("which transport accepted"),
            "{err}"
        );
    }
}
