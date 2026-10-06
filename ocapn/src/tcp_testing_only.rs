//! The `tcp-testing-only` netlayer — the conformance suite's transport, and only that.
//!
//! The OCapN test suite's transport of the same name is described there as "HIGHLY INSECURE, DO NOT
//! USE IN PRODUCTION": it "streams pure Syrup-encoded data directly, without encryption or metadata
//! beyond regular CapTP messages", which makes it "simple to implement, and should be enough to get
//! you through the tests". This is a faithful implementation of exactly that, and it is the transport
//! the OCapN conformance suite and this crate's own session tests run over (`ocapn/tests/`). A
//! deployment uses a different netlayer through the same [`Netlayer`] trait; nothing above this
//! module changes.
//!
//! Framing is one netstring per message, shared with the `unix` transport in [`crate::framed`] — the
//! two differ only in the socket underneath, and a framing written twice is a framing that can drift.
//! Two bounds make an adversarial peer's stream harmless: the codec's nesting depth (`crate::syrup`)
//! and the message-size cap the framing applies, so a peer that opens a value and never closes it
//! cannot grow our memory.

use std::io;
use std::net::SocketAddr;

use async_trait::async_trait;
use tokio::net::{TcpListener, TcpStream};

use crate::framed::Framed;
use crate::locator::PeerLocator;
use crate::netlayer::{NetConn, Netlayer};

/// The dial bound lives with the framing both transports share; re-exported here because this is
/// where the TCP transport's reader has always found it.
pub use crate::framed::CONNECT_TIMEOUT;

/// A `tcp-testing-only` endpoint. The same value dials (as a client) and accepts (as a server);
/// dialling needs no listener, so a purely outgoing peer still binds one on an ephemeral port.
pub struct TcpTestingOnly {
    listener: TcpListener,
}

impl TcpTestingOnly {
    /// Bind a listening socket. `addr` is `host:port`; `127.0.0.1:0` asks the OS for a free port
    /// (see [`TcpTestingOnly::local_addr`]).
    pub async fn bind(addr: &str) -> io::Result<TcpTestingOnly> {
        Ok(TcpTestingOnly {
            listener: TcpListener::bind(addr).await?,
        })
    }

    /// The bound address, including the port chosen for `:0`.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }
}

#[async_trait]
impl Netlayer for TcpTestingOnly {
    async fn new_outgoing_connection(&self, locator: &PeerLocator) -> io::Result<Box<dyn NetConn>> {
        let host = locator.hints.get("host").ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "tcp-testing-only needs a `host` hint",
            )
        })?;
        let port: u16 = locator
            .hints
            .get("port")
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "tcp-testing-only needs a `port` hint",
                )
            })?
            .parse()
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "tcp-testing-only `port` is not a number",
                )
            })?;
        let stream =
            tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect((host.as_str(), port)))
                .await
                .map_err(|_| {
                    io::Error::new(
                        io::ErrorKind::TimedOut,
                        format!("connecting to {host}:{port} took longer than {CONNECT_TIMEOUT:?}"),
                    )
                })??;
        Ok(Box::new(TcpConn {
            framed: Framed::new(stream),
        }))
    }

    async fn accept_incoming_connection(&self) -> io::Result<Box<dyn NetConn>> {
        let (stream, _peer) = self.listener.accept().await?;
        Ok(Box::new(TcpConn {
            framed: Framed::new(stream),
        }))
    }
}

/// A dialled TCP connection: the netstring framing both transports share, over a socket.
struct TcpConn {
    framed: Framed<TcpStream>,
}

#[async_trait]
impl NetConn for TcpConn {
    async fn send(&mut self, message: &[u8]) -> io::Result<()> {
        self.framed.send_message(message).await
    }

    fn peer_address(&self) -> Option<std::net::SocketAddr> {
        // A TCP socket always knows; this is what lets the dial policy tell a remote peer from a
        // local one (Law 62). A transport whose socket does not know leaves this at the default.
        self.framed.get_ref().peer_addr().ok()
    }

    async fn recv(&mut self) -> io::Result<Option<Vec<u8>>> {
        self.framed.next_message().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    // The framing moved to `crate::framed`; a test that writes raw bytes to the socket still needs
    // the extension traits in scope.
    use tokio::io::AsyncWriteExt;

    use crate::syrup::Value;

    fn loopback_locator(addr: SocketAddr) -> PeerLocator {
        PeerLocator {
            designator: "peer".into(),
            transport: "tcp-testing-only".into(),
            hints: BTreeMap::from([
                ("host".to_string(), "127.0.0.1".to_string()),
                ("port".to_string(), addr.port().to_string()),
            ]),
        }
    }

    /// A dialling endpoint needs no listener of its own, but the trait bears on one; bind a
    /// throwaway on an ephemeral port.
    async fn dialer() -> TcpTestingOnly {
        TcpTestingOnly::bind("127.0.0.1:0").await.unwrap()
    }

    #[tokio::test]
    async fn messages_are_framed_across_a_coalesced_stream() {
        let server = TcpTestingOnly::bind("127.0.0.1:0").await.unwrap();
        let addr = server.local_addr().unwrap();

        let sender = tokio::spawn(async move {
            let mut conn = dialer()
                .await
                .new_outgoing_connection(&loopback_locator(addr))
                .await
                .unwrap();
            // Two messages, written separately; TCP may deliver them in one read, which is the
            // framing this test exists to exercise.
            conn.send(&Value::Symbol("one".into()).to_bytes())
                .await
                .unwrap();
            conn.send(
                &Value::Record(vec![Value::Symbol("two".into()), Value::Int(7.into())]).to_bytes(),
            )
            .await
            .unwrap();
        });

        let mut inbound = server.accept_incoming_connection().await.unwrap();
        let m1 = inbound.recv().await.unwrap().unwrap();
        assert_eq!(Value::from_bytes(&m1).unwrap(), Value::Symbol("one".into()));
        let m2 = inbound.recv().await.unwrap().unwrap();
        assert_eq!(
            Value::from_bytes(&m2).unwrap(),
            Value::Record(vec![Value::Symbol("two".into()), Value::Int(7.into())])
        );
        sender.await.unwrap();
    }

    #[tokio::test]
    async fn a_clean_close_is_end_of_stream() {
        let server = TcpTestingOnly::bind("127.0.0.1:0").await.unwrap();
        let addr = server.local_addr().unwrap();

        tokio::spawn(async move {
            let conn = dialer()
                .await
                .new_outgoing_connection(&loopback_locator(addr))
                .await
                .unwrap();
            drop(conn); // close at a message boundary
        });

        let mut inbound = server.accept_incoming_connection().await.unwrap();
        assert!(inbound.recv().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_close_inside_a_message_is_an_error_not_a_silent_drop() {
        let server = TcpTestingOnly::bind("127.0.0.1:0").await.unwrap();
        let addr = server.local_addr().unwrap();

        tokio::spawn(async move {
            let mut stream = TcpStream::connect(addr).await.unwrap();
            // Declares a 10-byte message and sends 4 of them, then closes.
            stream.write_all(b"10:<1'a").await.unwrap();
            drop(stream);
        });

        let mut inbound = server.accept_incoming_connection().await.unwrap();
        let err = inbound.recv().await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }
}
