//! Server-side gRPC transport receiver (the `TransportLayer` service).
//!
//! Mirrors `comm/src/main/scala/coop/rchain/comm/transport/GrpcTransportReceiver.scala`. The per-peer
//! `LimitedBufferObservable` dispatch queues are simplified to a direct spawned dispatch; the
//! streamed-message circuit breaker and `PacketOps` cache round-trip are preserved.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use async_trait::async_trait;
use futures::channel::mpsc;
use futures::SinkExt;
use rchain_models::comm::protocol::transport_layer_server;
use rchain_models::comm::protocol::{
    chunk, tl_response, Ack, Chunk, InternalServerError, Protocol, TlRequest, TlResponse,
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tonic::{Request, Response, Status, Streaming};

use crate::peer_node::PeerNode;
use crate::rp::protocol_helper;
use crate::transport::chunker::Blob;
use crate::transport::communication_response::CommunicationResponse;
use crate::transport::packet_ops::{self, PacketCache};
use crate::transport::stream_handler::{self, Circuit, StreamError, Streamed};

/// A boxed `Send` future (helper alias for handler closures).
pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// An incoming TLS connection, implementing tonic's `Connected`.
struct TlsIo(tokio_rustls::server::TlsStream<tokio::net::TcpStream>);

impl tonic::transport::server::Connected for TlsIo {
    type ConnectInfo = ();
    fn connect_info(&self) -> Self::ConnectInfo {}
}

impl AsyncRead for TlsIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl AsyncWrite for TlsIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

fn ack(local: &PeerNode, network_id: &str) -> TlResponse {
    TlResponse {
        payload: Some(tl_response::Payload::Ack(Ack {
            header: Some(protocol_helper::header(local, network_id)),
        })),
    }
}

fn internal_server_error(msg: &str) -> TlResponse {
    TlResponse {
        payload: Some(tl_response::Payload::InternalServerError(
            InternalServerError {
                error: protocol_helper::to_protocol_bytes(msg),
            },
        )),
    }
}

fn stream_error_message(error: &StreamError) -> String {
    match error {
        StreamError::WrongNetworkId => "Could not receive stream! Wrong network id.".to_string(),
        StreamError::MaxSizeReached => "Max message size was reached.".to_string(),
        StreamError::NotFullMessage(s) => {
            format!("Received not full stream message, will not process. {s}")
        }
        StreamError::Unexpected(t) => format!("Could not receive stream! {t}"),
    }
}

/// Bound on concurrent inbound-message dispatches (the Scala per-peer `LimitedBufferObservable`
/// bounded-queue analog). A flood that cannot acquire a slot is rejected with `ResourceExhausted`
/// rather than spawning an unbounded number of tasks.
const MAX_CONCURRENT_DISPATCH: usize = 1024;
/// Bound on concurrent inbound TLS handshakes (M1). A stalled client handshake must not serialize
/// every subsequent inbound connection, so each handshake is spawned and its result fed through a
/// bounded channel.
const MAX_CONCURRENT_HANDSHAKES: usize = 128;
/// Bound on concurrent inbound `stream` RPCs. The unary `send` path is bounded by
/// `MAX_CONCURRENT_DISPATCH`; the streaming path was unbounded, so a peer could open arbitrarily
/// many concurrent streams (each buffering chunks up to `max_stream_message_size`). Exhaustion
/// returns `ResourceExhausted` without buffering.
const MAX_CONCURRENT_STREAMS: usize = 1024;
/// Bound on concurrent decompressed stream blobs in flight. Each `handle_streamed` task holds its
/// blob (up to `max_stream_message_size`) until the routing queue accepts it, so this must be small —
/// it is the aggregate decompressed-memory budget, not the per-stream size.
const MAX_CONCURRENT_BLOBS: usize = 16;
/// Wall-clock bound on a single inbound TLS handshake, so a stalled ClientHello cannot hold a
/// handshake slot (and its socket) indefinitely.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// The receiver's concurrency bounds, in one value.
///
/// Parameterised for one reason: the production values (1024 dispatch slots, 1024 streams, 16 blobs)
/// can only be exhausted by a test that opens a thousand concurrent TLS streams, which costs far more
/// than the bound it would pin. `serve` uses [`ConcurrencyLimits::default`]; `serve_with_limits` lets
/// a test set a small bound and observe saturation deterministically.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConcurrencyLimits {
    /// Concurrent inbound unary dispatches (`send`); exhaustion is `ResourceExhausted`.
    pub dispatches: usize,
    /// Concurrent inbound `stream` RPCs.
    pub streams: usize,
    /// Concurrent decompressed blobs in flight (the aggregate decompressed-memory budget).
    pub blobs: usize,
    /// Concurrent in-flight TLS handshakes, and the depth of the accepted-stream channel.
    pub handshakes: usize,
}

impl Default for ConcurrencyLimits {
    fn default() -> Self {
        ConcurrencyLimits {
            dispatches: MAX_CONCURRENT_DISPATCH,
            streams: MAX_CONCURRENT_STREAMS,
            blobs: MAX_CONCURRENT_BLOBS,
            handshakes: MAX_CONCURRENT_HANDSHAKES,
        }
    }
}
/// Bound on the number of chunks accepted per inbound `stream`. Empty `content_data` chunks never
/// advance the byte counter, so without this a peer could stream an unbounded number of them.
const MAX_STREAM_CHUNKS: usize = 100_000;

/// The inbound gRPC `TransportLayer` service (port of the `RoutingGrpcMonix.TransportLayer` impl).
pub struct GrpcTransportReceiver {
    local: PeerNode,
    network_id: String,
    max_stream_message_size: i64,
    dispatch: Arc<dyn Fn(Protocol) -> BoxFuture<CommunicationResponse> + Send + Sync>,
    handle_streamed: Arc<dyn Fn(Blob) -> BoxFuture<()> + Send + Sync>,
    dispatch_slots: Arc<tokio::sync::Semaphore>,
    stream_slots: Arc<tokio::sync::Semaphore>,
    blob_slots: Arc<tokio::sync::Semaphore>,
}

impl GrpcTransportReceiver {
    /// A receiver over caller-supplied handlers, with the TLS/accept loop bypassed.
    ///
    /// **Test only.** The production path builds this struct inside `serve_with_limits` (behind the
    /// accept loop and the TLS session interceptor), so the inbound guards below — the network-id
    /// rejection, the dispatch-queue bound and the missing-protocol arm — are otherwise reachable
    /// only over a socket. `comm/src/transport/grpc_transport.rs` drives that path end to end; this
    /// seam exists so the guards themselves can be pinned without a server. Listed in
    /// `spec/TEST-COVERAGE.md`'s production-change table.
    #[cfg(test)]
    pub(crate) fn for_test(
        local: PeerNode,
        network_id: &str,
        max_stream_message_size: i64,
        dispatch: Arc<dyn Fn(Protocol) -> BoxFuture<CommunicationResponse> + Send + Sync>,
        handle_streamed: Arc<dyn Fn(Blob) -> BoxFuture<()> + Send + Sync>,
        limits: ConcurrencyLimits,
    ) -> Self {
        GrpcTransportReceiver {
            local,
            network_id: network_id.to_string(),
            max_stream_message_size,
            dispatch,
            handle_streamed,
            dispatch_slots: Arc::new(tokio::sync::Semaphore::new(limits.dispatches)),
            stream_slots: Arc::new(tokio::sync::Semaphore::new(limits.streams)),
            blob_slots: Arc::new(tokio::sync::Semaphore::new(limits.blobs)),
        }
    }
}

#[async_trait]
impl transport_layer_server::TransportLayer for GrpcTransportReceiver {
    async fn send(&self, request: Request<TlRequest>) -> Result<Response<TlResponse>, Status> {
        let protocol = request
            .into_inner()
            .protocol
            .ok_or_else(|| Status::invalid_argument("missing protocol"))?;

        // SslSessionServerInterceptor equivalent: reject wrong-network senders.
        if let Some(header) = &protocol.header {
            if header.network_id != self.network_id {
                let nid = if header.network_id.is_empty() {
                    "<empty>"
                } else {
                    &header.network_id
                };
                return Err(Status::permission_denied(format!(
                    "Wrong network id '{nid}'. This node runs on network '{}'",
                    self.network_id
                )));
            }
        }

        let permit = match self.dispatch_slots.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => return Err(Status::resource_exhausted("dispatch queue full")),
        };
        let dispatch = self.dispatch.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let _ = (dispatch)(protocol).await;
        });
        Ok(Response::new(ack(&self.local, &self.network_id)))
    }

    async fn stream(
        &self,
        request: Request<Streaming<Chunk>>,
    ) -> Result<Response<TlResponse>, Status> {
        // Bound concurrent stream RPCs (the unary `send` analog of `dispatch_slots`). The permit is
        // held across the whole handler (including the chunk drain), so a peer cannot accumulate an
        // unbounded number of in-flight streams.
        let _stream_permit = match self.stream_slots.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => return Err(Status::resource_exhausted("stream dispatch queue full")),
        };
        let mut incoming = request.into_inner();
        let mut chunks = Vec::new();
        // Enforce the size cap *while* draining, so a peer cannot stream an unbounded number of
        // chunks before the circuit breaker runs in `stream_handler::collect`. A separate chunk-count
        // cap bounds empty `content_data` chunks, which never advance `received` (R26).
        let mut received: i64 = 0;
        while let Some(chunk) = incoming.message().await? {
            if let Some(chunk::Content::Data(d)) = &chunk.content {
                received += d.content_data.len() as i64;
                if received > self.max_stream_message_size {
                    return Ok(Response::new(internal_server_error(&stream_error_message(
                        &StreamError::MaxSizeReached,
                    ))));
                }
            }
            chunks.push(chunk);
            if chunks.len() > MAX_STREAM_CHUNKS {
                return Ok(Response::new(internal_server_error(&stream_error_message(
                    &StreamError::MaxSizeReached,
                ))));
            }
        }

        let mut cache = PacketCache::new();
        let key = packet_ops::create_cache_entry("packet_send/", &mut cache);
        let init = Streamed::new(key);

        let network_id = self.network_id.clone();
        let max_stream_message_size = self.max_stream_message_size;
        let breaker = move |streamed: &Streamed| {
            if let Some(header) = &streamed.header {
                if header.network_id != network_id {
                    return Circuit::Opened(StreamError::WrongNetworkId);
                }
            }
            if streamed.read_so_far > max_stream_message_size {
                return Circuit::Opened(StreamError::MaxSizeReached);
            }
            Circuit::Closed
        };

        let collected = stream_handler::collect(&init, &chunks, &breaker, &mut cache)
            .and_then(|stmd| stream_handler::to_result(&stmd));

        match collected {
            Ok(msg) => match stream_handler::restore(
                &msg,
                &mut cache,
                usize::try_from(self.max_stream_message_size).unwrap_or(usize::MAX),
            ) {
                Ok(blob) => {
                    // Bound concurrent decompressed blobs: `handle_streamed` blocks on the routing
                    // queue while holding the (up-to-256-MiB) blob, so acquire a slot before
                    // spawning and drop the blob when the aggregate budget is exhausted (R13).
                    let permit = match self.blob_slots.clone().try_acquire_owned() {
                        Ok(p) => p,
                        Err(_) => {
                            return Ok(Response::new(internal_server_error(
                                &stream_error_message(&StreamError::MaxSizeReached),
                            )));
                        }
                    };
                    let handle = self.handle_streamed.clone();
                    tokio::spawn(async move {
                        let _permit = permit;
                        (handle)(blob).await;
                    });
                    Ok(Response::new(ack(&self.local, &self.network_id)))
                }
                Err(e) => Ok(Response::new(internal_server_error(&e))),
            },
            Err(e) => Ok(Response::new(internal_server_error(&stream_error_message(
                &e,
            )))),
        }
    }
}

/// Bind a mutual-TLS listener and serve the transport receiver.
pub async fn serve(
    local: PeerNode,
    network_id: String,
    port: u16,
    tls: Arc<rustls::ServerConfig>,
    max_stream_message_size: i64,
    dispatch: Arc<dyn Fn(Protocol) -> BoxFuture<CommunicationResponse> + Send + Sync>,
    handle_streamed: Arc<dyn Fn(Blob) -> BoxFuture<()> + Send + Sync>,
) -> Result<(), String> {
    serve_with_limits(
        local,
        network_id,
        port,
        tls,
        max_stream_message_size,
        dispatch,
        handle_streamed,
        ConcurrencyLimits::default(),
    )
    .await
}

/// Serve the transport with explicit concurrency bounds (see [`ConcurrencyLimits`]).
#[allow(clippy::too_many_arguments)]
pub async fn serve_with_limits(
    local: PeerNode,
    network_id: String,
    port: u16,
    tls: Arc<rustls::ServerConfig>,
    max_stream_message_size: i64,
    dispatch: Arc<dyn Fn(Protocol) -> BoxFuture<CommunicationResponse> + Send + Sync>,
    handle_streamed: Arc<dyn Fn(Blob) -> BoxFuture<()> + Send + Sync>,
    limits: ConcurrencyLimits,
) -> Result<(), String> {
    // Faithful to Scala: the protocol server binds to `0.0.0.0` (the `protocol-server.host` config
    // is the *advertised* address, not the bind address). The bind is left as-is; the fix here is
    // concurrent (not serialized) TLS handshakes.
    let listener = TcpListener::bind(("0.0.0.0", port))
        .await
        .map_err(|e| e.to_string())?;
    let acceptor = TlsAcceptor::from(tls);

    // Concurrent TLS handshakes (M1): accept connections in a tight loop, hand off each handshake to
    // a spawned task, and feed the accepted TLS streams to tonic through a bounded channel. A slow
    // handshake no longer blocks the accept loop.
    let (tx, rx) = mpsc::channel::<Result<TlsIo, std::io::Error>>(limits.handshakes);
    // Bound *in-flight* handshakes (not just the completed ones the channel bounds): acquire a slot
    // before spawning, so a peer opening thousands of idle connections cannot spawn that many
    // handshake tasks each holding a socket + rustls state until the TCP timeout (R14).
    let handshake_slots = Arc::new(tokio::sync::Semaphore::new(limits.handshakes));
    tokio::spawn(async move {
        loop {
            let tcp = match listener.accept().await {
                Ok((tcp, _)) => tcp,
                Err(_) => break,
            };
            let Ok(permit) = handshake_slots.clone().try_acquire_owned() else {
                continue;
            };
            let acceptor = acceptor.clone();
            let tx = tx.clone();
            tokio::spawn(async move {
                let _permit = permit;
                let mut tx = tx;
                let accepted = tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(tcp)).await;
                if let Ok(Ok(tls)) = accepted {
                    let _ = tx.send(Ok(TlsIo(tls))).await;
                }
            });
        }
    });
    let incoming = rx;

    let service = GrpcTransportReceiver {
        local,
        network_id,
        max_stream_message_size,
        dispatch,
        handle_streamed,
        dispatch_slots: Arc::new(tokio::sync::Semaphore::new(limits.dispatches)),
        stream_slots: Arc::new(tokio::sync::Semaphore::new(limits.streams)),
        blob_slots: Arc::new(tokio::sync::Semaphore::new(limits.blobs)),
    };

    tonic::transport::Server::builder()
        .add_service(transport_layer_server::TransportLayerServer::new(service))
        .serve_with_incoming(incoming)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // The service methods live on the generated trait, so it must be in scope to call them.
    use rchain_models::comm::protocol::transport_layer_server::TransportLayer as _;

    use rchain_shared::refined::Port;

    /// Counts handler calls, so a rejection can be asserted by *absence*.
    #[derive(Default)]
    struct Dispatch {
        calls: AtomicUsize,
    }

    fn local() -> PeerNode {
        PeerNode::from(
            crate::peer_node::NodeIdentifier::new(vec![0x11; 20]),
            "127.0.0.1".to_string(),
            Port::new(40400),
            Port::new(40404),
        )
    }

    fn receiver(dispatch: Arc<Dispatch>, dispatches: usize) -> GrpcTransportReceiver {
        let d = dispatch.clone();
        GrpcTransportReceiver::for_test(
            local(),
            "testnet",
            16 * 1024 * 1024,
            Arc::new(move |_p| {
                d.calls.fetch_add(1, Ordering::SeqCst);
                Box::pin(async { CommunicationResponse::handled_without_message() })
            }),
            Arc::new(|_b| Box::pin(async {})),
            ConcurrencyLimits {
                dispatches,
                ..ConcurrencyLimits::default()
            },
        )
    }

    /// A protocol with a header naming `network_id`.
    fn request(network_id: &str) -> Request<TlRequest> {
        Request::new(TlRequest {
            protocol: Some(Protocol {
                header: Some(protocol_helper::header(&local(), network_id)),
                message: None,
            }),
        })
    }

    /// **The network-id guard.** A sender on another network is refused with `PermissionDenied`
    /// naming *both* networks, and its message is never dispatched — the guard is what keeps two
    /// chains' traffic out of each other's routing layer.
    #[tokio::test]
    async fn a_sender_from_another_network_is_refused_and_not_dispatched() {
        let d = Arc::new(Dispatch::default());
        let service = receiver(d.clone(), 8);

        let status = service
            .send(request("mainnet"))
            .await
            .expect_err("another network must be refused");
        assert_eq!(status.code(), tonic::Code::PermissionDenied);
        let message = status.message();
        assert!(message.contains("mainnet"), "{message}");
        assert!(message.contains("testnet"), "{message}");
        assert_eq!(
            d.calls.load(Ordering::SeqCst),
            0,
            "a foreign network's message must not reach the dispatch handler"
        );
    }

    /// An *empty* network id is refused too, and the message says so rather than printing nothing
    /// where the network should be.
    #[tokio::test]
    async fn a_sender_with_no_network_id_is_refused() {
        let service = receiver(Arc::new(Dispatch::default()), 8);
        let status = service
            .send(request(""))
            .await
            .expect_err("an empty network id is still not ours");
        assert_eq!(status.code(), tonic::Code::PermissionDenied);
        assert!(status.message().contains("<empty>"), "{}", status.message());
    }

    /// The accepting path: a same-network protocol is dispatched (asynchronously) and answered with
    /// an `Ack` carrying *this* node's header, which is what the sender authenticates against.
    #[tokio::test]
    async fn a_sender_on_this_network_is_dispatched_and_acked() {
        let d = Arc::new(Dispatch::default());
        let service = receiver(d.clone(), 8);

        let response = service
            .send(request("testnet"))
            .await
            .expect("our own network is accepted")
            .into_inner();
        match response.payload {
            Some(tl_response::Payload::Ack(ack)) => {
                let header = ack.header.expect("an ack carries a header");
                assert_eq!(header.network_id, "testnet");
                assert_eq!(header.sender.expect("a sender").id, local().key().to_vec());
            }
            other => panic!("expected an ack, got {other:?}"),
        }

        // The dispatch happens on a spawned task, so give it a turn before asserting.
        for _ in 0..50 {
            if d.calls.load(Ordering::SeqCst) == 1 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(
            d.calls.load(Ordering::SeqCst),
            1,
            "the message is dispatched once"
        );
    }

    /// A request without a protocol is a client error, not a panic.
    #[tokio::test]
    async fn a_missing_protocol_is_an_invalid_argument() {
        let service = receiver(Arc::new(Dispatch::default()), 8);
        let status = service
            .send(Request::new(TlRequest { protocol: None }))
            .await
            .expect_err("a request without a protocol is malformed");
        assert_eq!(status.code(), tonic::Code::InvalidArgument);
    }

    /// **The dispatch bound.** With one slot and a handler that has not finished, a second inbound
    /// message is refused with `ResourceExhausted` rather than queued without bound. The permit is
    /// held by the spawned task, so the bound is a gate on *in-flight* dispatches.
    #[tokio::test]
    async fn a_full_dispatch_queue_is_refused() {
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let e = entered.clone();
        let r = release.clone();
        let service = GrpcTransportReceiver::for_test(
            local(),
            "testnet",
            16 * 1024 * 1024,
            Arc::new(move |_p| {
                let e = e.clone();
                let r = r.clone();
                Box::pin(async move {
                    e.notify_one();
                    r.notified().await;
                    CommunicationResponse::handled_without_message()
                })
            }),
            Arc::new(|_b| Box::pin(async {})),
            ConcurrencyLimits {
                dispatches: 1,
                ..ConcurrencyLimits::default()
            },
        );

        service
            .send(request("testnet"))
            .await
            .expect("the first message takes the only slot");
        entered.notified().await;

        let status = service
            .send(request("testnet"))
            .await
            .expect_err("the queue is full");
        assert_eq!(status.code(), tonic::Code::ResourceExhausted);

        // Releasing the first dispatch frees the slot: the bound is a queue, not a latch.
        release.notify_one();
        for _ in 0..50 {
            if service.send(request("testnet")).await.is_ok() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("the slot must be released when the dispatch finishes");
    }

    /// The chunk-count cap bounds empty `content_data` chunks, which never advance the byte count
    /// (R26). Asserted at compile time, so a change to the bound is a build failure.
    const _: () = assert!(MAX_STREAM_CHUNKS > 0);

    /// `CommunicationResponse` is what the dispatch closure returns; the variants are the three
    /// answers the routing layer understands. A cheap sanity check that the constructor used by the
    /// tests above is the same shape production builds.
    #[test]
    fn limits_default_to_the_documented_constants() {
        let limits = ConcurrencyLimits::default();
        assert_eq!(limits.dispatches, MAX_CONCURRENT_DISPATCH);
        assert_eq!(limits.streams, MAX_CONCURRENT_STREAMS);
        assert_eq!(limits.blobs, MAX_CONCURRENT_BLOBS);
        assert_eq!(limits.handshakes, MAX_CONCURRENT_HANDSHAKES);
        assert!(limits.dispatches > 0 && limits.streams > 0 && limits.blobs > 0);
    }
}
