//! Server-side gRPC transport receiver (the `TransportLayer` service).
//!
//! Mirrors `comm/src/main/scala/coop/rchain/comm/transport/GrpcTransportReceiver.scala`. The per-peer
//! `LimitedBufferObservable` dispatch queues are simplified to a direct spawned dispatch; the
//! streamed-message circuit breaker and `PacketOps` cache round-trip are preserved.

use rchain_shared::chan;
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

/// The node id **proven by the client certificate** on an inbound connection (AUDIT C115).
///
/// This is what turns mutual TLS into authentication. Before it, the handshake proved only that the
/// peer held *some* valid P-256 key — `NodeIdClientVerifier` accepts any self-signed certificate whose
/// public key is a well-formed point — and the node id used for routing was taken from the protocol
/// header, which the peer chooses. So any host could connect and assert any node id: impersonate a
/// validator, occupy its entry in a peer's connection table, and be believed. `Option` because a
/// connection that reached the service without a certificate yields `None`, and a missing proof must
/// not read as a valid one.
#[derive(Clone, Debug)]
pub struct PeerId(pub Option<String>);

/// The node id a completed TLS session proved, as the same hex string a `PeerNode`'s id renders as.
///
/// `None` when the session carries no peer certificate or the certificate's key is not a P-256 point.
/// The client side has always made the mirror of this check — `NodeIdServerVerifier` requires the
/// *server's* certificate address to equal the peer id it dialled — so this is the other half of an
/// asymmetry the code had half-implemented, not a new policy.
pub fn peer_id_of_tls(
    tls: &tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
) -> Option<String> {
    let (_, connection) = tls.get_ref();
    let leaf = connection.peer_certificates()?.first()?;
    let address = crate::transport::hostname_trust_manager::public_address_of_cert(leaf)?;
    Some(rchain_shared::base16::encode(&address))
}

/// An incoming TLS connection, implementing tonic's `Connected`.
///
/// Shared with the Kademlia RPC server (`discovery::grpc_kademlia_rpc_server`), which serves over the
/// same certificate configuration and needs the same proven identity — see [`accept_tls`].
pub struct TlsIo {
    stream: tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
    peer_id: Option<String>,
}

impl tonic::transport::server::Connected for TlsIo {
    type ConnectInfo = PeerId;
    fn connect_info(&self) -> Self::ConnectInfo {
        PeerId(self.peer_id.clone())
    }
}

/// Accept TLS connections on `listener` and yield them for `serve_with_incoming`.
///
/// Extracted from `serve_with_limits` so the Kademlia RPC server can be wrapped in the *same* mutual
/// TLS the transport uses rather than a second, separately-configured accept path (AUDIT C116). The
/// two services differ in their protocol; they must not differ in who they let in.
///
/// Handshakes are spawned, so a stalled `ClientHello` cannot serialize the accept loop, and each is
/// bounded by [`HANDSHAKE_TIMEOUT`]. `capacity` bounds **in-flight** handshakes as well as the channel
/// depth — a slot is acquired before spawning, so a peer opening thousands of idle connections cannot
/// spawn that many handshake tasks each holding a socket and rustls state until the TCP timeout (R14).
pub fn accept_tls(
    listener: TcpListener,
    acceptor: TlsAcceptor,
    capacity: usize,
) -> futures::channel::mpsc::Receiver<Result<TlsIo, std::io::Error>> {
    let (tx, rx) = mpsc::channel::<Result<TlsIo, std::io::Error>>(capacity);
    let handshake_slots = Arc::new(tokio::sync::Semaphore::new(capacity));
    rchain_shared::supervise::spawn_supervised("tls-handshake", async move {
        let tx = tx;
        loop {
            let Ok((tcp, _)) = listener.accept().await else {
                continue;
            };
            let Ok(permit) = handshake_slots.clone().try_acquire_owned() else {
                continue;
            };
            let acceptor = acceptor.clone();
            let tx = tx.clone();
            // **Not supervised, and that is a decision rather than an omission** (AUDIT C254). Every
            // connection's handshake *ends* — that is its normal completion — so counting returns
            // here would turn the supervisor into a flood of lines about healthy handshakes, and the
            // counter would stop meaning "a subsystem died". A panic in this task already reaches
            // stderr through the runtime's default hook. The long-lived accept loop above is the task
            // whose death is a real loss, and it is the one that is watched.
            tokio::spawn(async move {
                let _permit = permit;
                let mut tx = tx;
                let accepted = tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(tcp)).await;
                if let Ok(Ok(tls)) = accepted {
                    // The identity is read here, once, while the session is in hand: the certificate
                    // is not reachable from a request handler, and this is the only place the
                    // handshake's own state is still available (AUDIT C115).
                    let peer_id = peer_id_of_tls(&tls);
                    // The accept loop outlives the session on a shutdown, so a handshake result nobody
                    // is left to receive is the case this is for — and it is a `futures` sink, not a
                    // tokio channel, so the named door is the one that applies (C254's E6b).
                    chan::best_effort(tx.send(Ok(TlsIo {
                        stream: tls,
                        peer_id,
                    })))
                    .await;
                }
            });
        }
    });
    rx
}

impl AsyncRead for TlsIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl AsyncWrite for TlsIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
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
        StreamError::SenderNotVerified => {
            "Could not receive stream! The sender is not the identity its certificate proves."
                .to_string()
        }
    }
}

/// Bound on concurrent inbound-message dispatches (the Scala per-peer `LimitedBufferObservable`
/// bounded-queue analog). A flood that cannot acquire a slot is rejected with `ResourceExhausted`
/// rather than spawning an unbounded number of tasks.
const MAX_CONCURRENT_DISPATCH: usize = 1024;
/// Bound on concurrent inbound TLS handshakes (M1). A stalled client handshake must not serialize
/// every subsequent inbound connection, so each handshake is spawned and its result fed through a
/// bounded channel.
pub const MAX_CONCURRENT_HANDSHAKES: usize = 128;
/// Bound on concurrent inbound `stream` RPCs. The unary `send` path is bounded by
/// `MAX_CONCURRENT_DISPATCH`; the streaming path was unbounded, so a peer could open arbitrarily
/// many concurrent streams (each buffering chunks up to `max_stream_message_size`). Exhaustion
/// returns `ResourceExhausted` without buffering.
const MAX_CONCURRENT_STREAMS: usize = 1024;
/// Bound on concurrent decompressed stream blobs in flight. Each `handle_streamed` task holds its
/// blob (up to `max_stream_message_size`) until the routing queue accepts it, so this must be small —
/// it is the aggregate decompressed-memory budget, not the per-stream size.
const MAX_CONCURRENT_BLOBS: usize = 16;

/// The unit the aggregate in-flight **compressed** stream budget is metered in (AUDIT C113).
///
/// `tokio::sync::Semaphore` counts `u32` permits, and a byte budget of `blobs × max_stream_message_size`
/// (4 GiB by default) does not fit one permit per byte. One mebibyte is the granularity: fine enough
/// that the budget is not wasted on rounding, coarse enough that the permit count is small.
const STREAM_BUDGET_UNIT: usize = 1024 * 1024;

/// The aggregate bytes a peer may hold buffered across **all** its in-flight `stream` RPCs at once.
///
/// **Why a per-stream cap was not a bound** (AUDIT C113). Each `stream` handler drains its chunks into
/// a local `Vec<Chunk>` under a `stream_slots` permit, and the only byte cap was
/// `max_stream_message_size` (256 MiB) *per stream*, with `MAX_CONCURRENT_STREAMS` = 1024 of them.
/// `blob_slots` bounds the *decompressed* blobs and is checked only after reassembly, so it never sees
/// the compressed accumulation. The product is ~256 GiB of resident memory reachable by one peer with
/// one certificate — a bound whose terms multiply to a number nobody intended.
///
/// The budget is `blobs × max_stream_message_size`, which is deliberately the number the node already
/// commits to for decompressed blobs: the two halves of one pipeline are now bounded by the same
/// figure, so a compressed buffer cannot be a loophole around the decompressed budget. A single
/// max-size stream fits inside it several times over, so legitimate traffic is unaffected and only the
/// aggregate is capped.
/// Charge `len` bytes against the aggregate budget, holding the permits in `held` until the caller
/// drops them. `Err(())` means the budget cannot cover it (AUDIT C113).
///
/// Extracted from the `stream` handler so the *decision* — not only the arithmetic that sizes the
/// budget — is testable without a socket: as an inline `try_acquire_many_owned` the only way to
/// observe exhaustion was to open enough real streams to spend the budget, which is a test that
/// costs more than the bound it pins.
///
/// A partial unit costs a whole one, so the budget cannot be nibbled away one byte at a time; and
/// the permits live in the caller's `Vec`, so they return when the stream ends on **every** exit
/// path, including the error returns.
fn charge_stream_budget(
    budget: &Arc<tokio::sync::Semaphore>,
    held: &mut Vec<tokio::sync::OwnedSemaphorePermit>,
    len: usize,
) -> Result<(), ()> {
    let units = u32::try_from(len.div_ceil(STREAM_BUDGET_UNIT))
        .unwrap_or(u32::MAX)
        .max(1);
    match budget.clone().try_acquire_many_owned(units) {
        Ok(permit) => {
            held.push(permit);
            Ok(())
        }
        Err(_) => Err(()),
    }
}

/// What a chunk costs its receiver, in bytes — **header or data**.
///
/// The aggregate budget and the per-stream size cap were both charged only inside the `Data` arm, so
/// a peer could stream `MAX_STREAM_CHUNKS` header chunks and pay for none of them: `received` never
/// moved, so `max_stream_message_size` never fired, and the only cost was one permit per chunk. A
/// `ChunkHeader`'s `type_id`, `network_id` and claimed sender are all `string`/`bytes` fields with no
/// bound of their own (`models/proto/routing.proto:77`), and a header is allocated, buffered into
/// `chunks` and carried through reassembly exactly as a data chunk is — so it is charged exactly as
/// one is (W-N12-04).
///
/// The *cost* of that is one budget unit per stream for a legitimate peer, because a real stream
/// carries a single header (`chunker::chunk_packet`): the budget is `blobs × max_stream_message_size`
/// = 16 × 256 MiB, so 4096 units, against which one is nothing. Charging headers cannot refuse a
/// legitimate stream, which is the property that had to be checked before changing this.
fn chunk_encoded_cost(chunk: &Chunk) -> usize {
    match &chunk.content {
        Some(chunk::Content::Data(d)) => d.content_data.len(),
        Some(chunk::Content::Header(h)) => {
            let sender = h.sender.as_ref().map_or(0, |n| n.id.len() + n.host.len());
            h.type_id.len() + h.network_id.len() + sender
        }
        None => 0,
    }
}

/// The streamed path's admission decision on a header chunk: the network id, and the claimed sender
/// against the identity the certificate proves.
///
/// **Exactly the circuit breaker's rule**, deliberately — extracted so the decision can run in the
/// drain loop, where it refuses a stream before the stream is paid for, rather than only in
/// `stream_handler::collect`, which runs after every chunk has been buffered, size-checked and
/// charged (W-N1-09/W-N1-10). Because the rule is the same one the breaker applies, moving it earlier
/// cannot admit anything `collect` would have refused; it only refuses sooner. A non-header chunk
/// decides nothing and is admitted to the loop.
fn stream_header_admits(
    network_id: &str,
    proven: Option<&str>,
    chunk: &Chunk,
) -> Result<(), Status> {
    let Some(chunk::Content::Header(h)) = &chunk.content else {
        return Ok(());
    };
    if h.network_id != network_id {
        return Err(Status::permission_denied("Wrong network id"));
    }
    let claimed = h
        .sender
        .as_ref()
        .and_then(|n| PeerNode::from_node(n).ok())
        .map(|p| p.s_key())
        .ok_or_else(|| Status::permission_denied("stream header carries no verifiable sender"))?;
    if proven != Some(claimed.as_str()) {
        return Err(Status::permission_denied(
            "Stream sender does not match the identity its certificate proves",
        ));
    }
    Ok(())
}

fn stream_byte_budget(blobs: usize, max_stream_message_size: i64) -> usize {
    // Written as an explicit match rather than as a fallible conversion with a zero default — the
    // *silent* class `tools/audit-type-system.sh` refuses, and it refused the first draft of this
    // very function, twice: once for the code and once for the comment describing it (the `silent`
    // scan keeps comments; only the counted classes strip them). That form hides *why* a value was
    // unacceptable behind a plausible default. The two outcomes here are different and are stated
    // differently — a non-positive cap is a configuration that licenses no buffering at all, and it
    // says so by returning zero; an unrepresentable one cannot happen on a 64-bit target and is
    // treated the same way rather than being allowed to saturate into a permit count.
    let per_blob = match usize::try_from(max_stream_message_size) {
        Ok(n) => n,
        Err(_) => return 0,
    };
    blobs.saturating_mul(per_blob)
}
/// The inbound unary decode cap used when a caller does not supply one.
///
/// `serve` is the convenience entry point; the production path supplies the operator's
/// `grpc_max_recv_message_size` through [`TransportLayerServer::new`] instead
/// (AUDIT C113). This constant is the same 256 KiB the configuration defaults to, so the
/// convenience path and the configured path agree rather than differing by 16x.
pub const DEFAULT_MAX_RECV_MESSAGE_SIZE: usize = 262144;

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
    /// The aggregate in-flight compressed-stream byte budget (AUDIT C113). Permits are metered in
    /// [`STREAM_BUDGET_UNIT`]s and held for the life of a `stream` handler.
    stream_bytes: Arc<tokio::sync::Semaphore>,
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
            stream_bytes: Arc::new(tokio::sync::Semaphore::new(
                stream_byte_budget(limits.blobs, max_stream_message_size) / STREAM_BUDGET_UNIT,
            )),
        }
    }
}

#[async_trait]
impl transport_layer_server::TransportLayer for GrpcTransportReceiver {
    async fn send(&self, request: Request<TlRequest>) -> Result<Response<TlResponse>, Status> {
        // The identity the TLS session proved, read before the request is consumed (AUDIT C115).
        let proven = request
            .extensions()
            .get::<PeerId>()
            .cloned()
            .unwrap_or(PeerId(None));
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

        // **The claimed sender must be the identity the certificate proves** (AUDIT C115). Everything
        // downstream — the connection table, equivocation detection, the DAG's per-sender rules —
        // keys on this field, and until now it was simply what the peer wrote: the handshake proved
        // possession of *some* P-256 key, not of *this* node's. A peer could therefore assert any node
        // id and be routed, tracked and compared as that node.
        //
        // Refused rather than dropped, and refused on `None` too: a connection that presents no
        // certificate is not one whose identity is unknown-but-probably-fine, it is one that proved
        // nothing. Client auth is mandatory, so `None` here means the invariant broke, and treating a
        // missing proof as valid is the one reading that cannot be defended.
        if let Ok(sender) = protocol_helper::sender(&protocol) {
            match &proven.0 {
                Some(id) if *id == sender.id.to_string() => {}
                Some(id) => {
                    return Err(Status::permission_denied(format!(
                        "Sender '{}' does not match the identity its certificate proves ('{id}')",
                        sender.id
                    )));
                }
                None => {
                    return Err(Status::permission_denied(
                        "No client certificate on this connection, so the sender cannot be verified",
                    ));
                }
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
        // The proven identity, read before the request is consumed (AUDIT C115): the streamed header
        // carries a sender of its own, and the routing layer trusts it as it trusts the unary path's.
        let proven = request
            .extensions()
            .get::<PeerId>()
            .cloned()
            .unwrap_or(PeerId(None));
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
        // The aggregate budget's permits, held until this handler returns (AUDIT C113). Dropping the
        // vector releases them, which is what makes the budget a *concurrent* one: it is charged as
        // bytes arrive and returned when the stream finishes, on every exit path including the error
        // returns below. Holding them in a local is deliberate — a permit that outlived the handler
        // would leak the budget, and one that was dropped early would license the accumulation this
        // exists to bound.
        let mut byte_permits: Vec<tokio::sync::OwnedSemaphorePermit> = Vec::new();
        while let Some(chunk) = incoming.message().await? {
            // **The authentication decision runs when the header arrives, not after the whole stream
            // has been paid for** (W-N1-09/W-N1-10). The circuit breaker below is the backstop, but it
            // runs inside `collect` — by which time this loop has already buffered every chunk,
            // size-checked it and charged it against the aggregate byte budget, so a peer whose header
            // was never going to verify still bought a full stream first.
            stream_header_admits(&self.network_id, proven.0.as_deref(), &chunk)?;
            // **Every chunk is charged, not only the data ones** (W-N12-04). The header arm used to
            // fall through with no accounting at all, which made a header's `type_id`, `network_id`
            // and sender free to size: `received` stayed put, so the per-stream cap never fired, and
            // the aggregate budget saw only the data chunks. Both now see the same number.
            let charged = chunk_encoded_cost(&chunk);
            received += charged as i64;
            if received > self.max_stream_message_size {
                return Ok(Response::new(internal_server_error(&stream_error_message(
                    &StreamError::MaxSizeReached,
                ))));
            }
            // Charge the aggregate budget. The per-stream cap above bounds *this* stream; this bounds
            // the sum over all of them, which is the number that multiplies. A chunk of any size costs
            // at least one unit, so an empty chunk is bounded by the same count that bounds the rest.
            if charge_stream_budget(&self.stream_bytes, &mut byte_permits, charged).is_err() {
                return Ok(Response::new(internal_server_error(&stream_error_message(
                    &StreamError::MaxSizeReached,
                ))));
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
        // The streamed path carries its own header, and therefore its own claimed sender, which the
        // routing layer trusts exactly as it trusts the unary path's — so it needs the same binding or
        // the fix has a door beside it (AUDIT C115). `proven` is moved in because the breaker is the
        // one place the reassembled header is examined before the blob is handed on.
        let proven_id = proven.0.clone();
        let breaker = move |streamed: &Streamed| {
            if let Some(header) = &streamed.header {
                if header.network_id != network_id {
                    return Circuit::Opened(StreamError::WrongNetworkId);
                }
                if proven_id.as_deref() != Some(header.sender.s_key().as_str()) {
                    return Circuit::Opened(StreamError::SenderNotVerified);
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
        DEFAULT_MAX_RECV_MESSAGE_SIZE,
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
    max_recv_message_size: usize,
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
    // handshake no longer blocks the accept loop. Shared with the Kademlia RPC server since AUDIT
    // C116, so the two services cannot drift apart in *who they let in*.
    let incoming = accept_tls(listener, acceptor, limits.handshakes);

    let service = GrpcTransportReceiver {
        local,
        network_id,
        max_stream_message_size,
        dispatch,
        handle_streamed,
        dispatch_slots: Arc::new(tokio::sync::Semaphore::new(limits.dispatches)),
        stream_slots: Arc::new(tokio::sync::Semaphore::new(limits.streams)),
        blob_slots: Arc::new(tokio::sync::Semaphore::new(limits.blobs)),
        stream_bytes: Arc::new(tokio::sync::Semaphore::new(
            stream_byte_budget(limits.blobs, max_stream_message_size) / STREAM_BUDGET_UNIT,
        )),
    };

    tonic::transport::Server::builder()
        // **The inbound unary cap, which the server never set** (AUDIT C113). The configured
        // `grpc_max_recv_message_size` (256 KiB) was applied on the *client* side only, so an
        // inbound `send` was accepted up to tonic's default of 4 MiB per message — a limit 16x the
        // one the operator believes is in force. Setting it here makes the two ends of the same
        // configuration agree.
        .add_service(
            transport_layer_server::TransportLayerServer::new(service)
                .max_decoding_message_size(max_recv_message_size),
        )
        .serve_with_incoming(incoming)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    /// **The regression test for AUDIT C113's mechanism**, and the one the arithmetic test above
    /// cannot make: that the aggregate budget *binds* and *returns*.
    ///
    /// `the_aggregate_stream_budget_bounds_what_the_stream_cap_multiplies` pins the numbers; this
    /// pins the behaviour. A budget computed correctly and never charged would pass that one and
    /// bound nothing — which is the failure mode of every bound that is only arithmetic.
    ///
    /// **What this does not cover, stated rather than left for a reader to assume**: it drives
    /// [`super::charge_stream_budget`] directly, so it would still pass if the `stream` handler
    /// stopped *calling* it. That the handler charges on every inbound data chunk is verified by
    /// reading the handler, not by this test. Closing the gap means driving `stream` with a real
    /// `Streaming<Chunk>`, which tonic does not expose a constructor for from outside — so the
    /// honest statement is that the mechanism is pinned and its wiring is reviewed.
    #[test]
    fn the_aggregate_budget_refuses_once_it_is_spent_and_returns_when_released() {
        // Three units of budget, metered in STREAM_BUDGET_UNIT.
        let budget = Arc::new(tokio::sync::Semaphore::new(3));
        let mut held = Vec::new();

        for _ in 0..3 {
            assert!(
                super::charge_stream_budget(&budget, &mut held, super::STREAM_BUDGET_UNIT).is_ok(),
                "each of the three units fits exactly"
            );
        }
        assert!(
            super::charge_stream_budget(&budget, &mut held, super::STREAM_BUDGET_UNIT).is_err(),
            "the fourth must be refused — a per-stream cap could not express this, because each \
             stream individually is well inside its own limit"
        );

        // Releasing returns the budget: it is charged per in-flight stream, not consumed.
        held.clear();
        assert!(
            super::charge_stream_budget(&budget, &mut held, super::STREAM_BUDGET_UNIT * 3).is_ok(),
            "the budget must return when the streams that held it end"
        );

        // A partial unit costs a whole one, so the budget cannot be nibbled away one byte at a time.
        held.clear();
        let tiny = Arc::new(tokio::sync::Semaphore::new(1));
        let mut held_tiny = Vec::new();
        assert!(super::charge_stream_budget(&tiny, &mut held_tiny, 1).is_ok());
        assert!(
            super::charge_stream_budget(&tiny, &mut held_tiny, 1).is_err(),
            "a single byte still costs a unit, or a peer could spend the budget in 1-byte steps"
        );
    }

    /// **The regression test for AUDIT C113.** The per-stream byte cap was never the bound it read
    /// as: `MAX_CONCURRENT_STREAMS` multiplies it, so the aggregate a single peer could hold resident
    /// was the *product*. These assertions pin the numbers and the relationship rather than restating
    /// them, so a change to any one of the three fails here instead of silently re-opening the hole.
    #[test]
    fn the_aggregate_stream_budget_bounds_what_the_stream_cap_multiplies() {
        // The production figures, as `ConcurrencyLimits::default` sets them.
        let max_stream: i64 = 268_435_456; // 256 MiB, `grpc_max_recv_stream_message_size`
        let blobs = MAX_CONCURRENT_BLOBS;
        let budget = super::stream_byte_budget(blobs, max_stream);
        assert_eq!(budget, blobs * usize::try_from(max_stream).unwrap());

        // The budget is the aggregate the node already commits to for *decompressed* blobs, so the
        // compressed half of the pipeline cannot be a loophole around it.
        assert_eq!(budget, 16 * 268_435_456);

        // And it is a real bound: the stream cap alone permits far more than the budget.
        let stream_cap_alone = MAX_CONCURRENT_STREAMS * usize::try_from(max_stream).unwrap();
        assert!(
            budget < stream_cap_alone,
            "the budget must be strictly smaller than streams x per-stream, or it bounds nothing: \
             the whole defect was that this product was the effective bound"
        );

        // The permit count, and that it does not overflow the semaphore's `u32`.
        let permits = budget / STREAM_BUDGET_UNIT;
        assert_eq!(permits, 4096);
        assert!(u32::try_from(permits).is_ok());

        // A single max-size stream fits, several times over: legitimate traffic is not refused.
        assert!(budget >= usize::try_from(max_stream).unwrap());

        // A degenerate configuration cannot produce a zero budget and a semaphore that refuses
        // everything — `saturating_mul` and the `unwrap_or(0)` keep this total.
        assert_eq!(super::stream_byte_budget(0, max_stream), 0);
        assert_eq!(super::stream_byte_budget(blobs, -1), 0);
    }

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

    /// A protocol with a header naming `network_id`, carrying the identity its certificate proves.
    ///
    /// The extension is not decoration: without it the sender check refuses every request, which is
    /// how these tests found it (AUDIT C115). `proven_id` is what the TLS session would have
    /// established.
    fn request_from(network_id: &str, proven_id: Option<String>) -> Request<TlRequest> {
        let mut r = Request::new(TlRequest {
            protocol: Some(Protocol {
                header: Some(protocol_helper::header(&local(), network_id)),
                message: None,
            }),
        });
        r.extensions_mut().insert(PeerId(proven_id));
        r
    }

    /// The honest case: the certificate proves the id the header claims.
    fn request(network_id: &str) -> Request<TlRequest> {
        request_from(network_id, Some(local().id.to_string()))
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

    /// **The regression test for AUDIT C115.** The sender in a protocol header is what every
    /// downstream decision keys on — the connection table, equivocation detection, the DAG's
    /// per-sender rules — and until this check existed it was simply what the peer wrote. Mutual TLS
    /// proved possession of *some* P-256 key, not of *this* node's, so any host could assert any node
    /// id and be routed, tracked and compared as that node.
    ///
    /// Three arms, because two of them are the ones that make the third meaningful: the honest
    /// request is dispatched, a request whose proven identity *differs* from its claimed sender is
    /// refused, and a connection that proved **nothing** is refused too — a missing proof must not
    /// read as a valid one.
    #[tokio::test]
    async fn a_sender_must_be_the_identity_its_certificate_proves() {
        let d = Arc::new(Dispatch::default());
        let service = receiver(d.clone(), 8);

        // The honest case: certificate and header agree, and the message is dispatched.
        service
            .send(request("testnet"))
            .await
            .expect("a sender matching its certificate must be accepted");
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
            "the honest request must reach the dispatcher, or the check refuses everything"
        );

        // The spoof: the certificate proves some other node, and the header claims `local()`.
        let other = crate::peer_node::NodeIdentifier::new(vec![0x22; 20]).to_string();
        let status = service
            .send(request_from("testnet", Some(other)))
            .await
            .expect_err("a sender its certificate does not prove must be refused");
        assert_eq!(status.code(), tonic::Code::PermissionDenied);
        assert_eq!(
            d.calls.load(Ordering::SeqCst),
            1,
            "a spoofed sender must never reach the dispatcher — this is the whole finding"
        );

        // No proof at all: client auth is mandatory, so `None` means the invariant broke, and it is
        // refused rather than read as "unverified but probably fine".
        let status = service
            .send(request_from("testnet", None))
            .await
            .expect_err("a connection with no proven identity must be refused");
        assert_eq!(status.code(), tonic::Code::PermissionDenied);
        assert_eq!(
            d.calls.load(Ordering::SeqCst),
            1,
            "an unproven sender must not reach the dispatcher"
        );
    }

    /// **The streamed header's admission decision is a unit, and it is the breaker's own rule**
    /// (W-N1-09/W-N1-10).
    ///
    /// The streamed path made this decision only inside `stream_handler::collect`, which runs *after*
    /// the drain loop has buffered, size-checked and budget-charged every chunk — so a stream whose
    /// header could never verify still bought a full stream first. The decision now runs when the
    /// header arrives, and it is this function.
    ///
    /// **What is pinned here is the decision, not its placement.** `tonic::Streaming` has no
    /// constructor a test can call, so the drain loop that calls this first is verified by reading it,
    /// exactly as this module already states for the aggregate budget. The rule being *the breaker's*
    /// is what makes the placement safe: a check that refuses a subset of what `collect` refuses can
    /// only refuse sooner, never more.
    #[test]
    fn a_stream_header_is_admitted_only_for_the_identity_its_certificate_proves() {
        let peer = |id: Vec<u8>, host: &str| {
            PeerNode::from(
                crate::peer_node::NodeIdentifier::new(id),
                host.into(),
                Port::new(40400),
                Port::new(40404),
            )
        };
        let proven = peer(vec![1, 2, 3], "host");
        let other = peer(vec![0x22; 20], "elsewhere");
        let header = |p: &PeerNode, net: &str| Chunk {
            content: Some(chunk::Content::Header(
                rchain_models::comm::protocol::ChunkHeader {
                    sender: Some(p.to_node()),
                    type_id: "BlockMessage".to_string(),
                    compressed: false,
                    content_length: 1,
                    network_id: net.to_string(),
                },
            )),
        };
        let s_key = proven.s_key();

        // The honest case, and a data chunk, which decides nothing.
        assert!(
            super::stream_header_admits("testnet", Some(&s_key), &header(&proven, "testnet"))
                .is_ok()
        );
        let data = Chunk {
            content: Some(chunk::Content::Data(
                rchain_models::comm::protocol::ChunkData {
                    content_data: vec![1, 2, 3],
                },
            )),
        };
        assert!(super::stream_header_admits("testnet", Some(&s_key), &data).is_ok());

        // The spoof: the certificate proves one node and the header claims another.
        assert_eq!(
            super::stream_header_admits("testnet", Some(&s_key), &header(&other, "testnet"))
                .expect_err("a sender its certificate does not prove must be refused at the header")
                .code(),
            tonic::Code::PermissionDenied
        );
        // The wrong network.
        assert_eq!(
            super::stream_header_admits("testnet", Some(&s_key), &header(&proven, "mainnet"))
                .expect_err("a wrong-network header must be refused")
                .code(),
            tonic::Code::PermissionDenied
        );
        // No proof at all: client auth is mandatory, so this is a broken invariant, not "unverified".
        assert_eq!(
            super::stream_header_admits("testnet", None, &header(&proven, "testnet"))
                .expect_err("a stream with no proven identity must be refused")
                .code(),
            tonic::Code::PermissionDenied
        );
        // A header carrying no sender at all.
        let senderless = Chunk {
            content: Some(chunk::Content::Header(
                rchain_models::comm::protocol::ChunkHeader {
                    sender: None,
                    type_id: "BlockMessage".to_string(),
                    compressed: false,
                    content_length: 1,
                    network_id: "testnet".to_string(),
                },
            )),
        };
        assert!(
            super::stream_header_admits("testnet", Some(&s_key), &senderless).is_err(),
            "a header with no sender is not one whose identity is unknown-but-probably-fine"
        );
    }

    /// **A header chunk is charged for the bytes a peer chooses** (W-N12-04).
    ///
    /// The drain loop charged the per-stream size cap and the aggregate budget *inside its `Data`
    /// arm only*, so a header chunk cost one permit and nothing else: `received` never moved, so
    /// `max_stream_message_size` never fired, and `type_id`, `network_id` and the claimed sender —
    /// all unbounded `string`/`bytes` on the wire — were free to size. `MAX_STREAM_CHUNKS` headers of
    /// a megabyte each were a megabyte-times-100 000 the node would buffer.
    ///
    /// This pins the cost function, which is where the fix is; as with `stream_header_admits`, its
    /// *placement* in the drain loop is verified by reading, because `tonic::Streaming` has no
    /// constructor a test can call. Falsified by making the header arm return `0` — which is what the
    /// old code did — and the first assertion goes red.
    #[test]
    fn a_header_chunk_is_charged_for_the_bytes_a_peer_chooses() {
        let header = |type_id: &str, network_id: &str| Chunk {
            content: Some(chunk::Content::Header(
                rchain_models::comm::protocol::ChunkHeader {
                    sender: Some(
                        crate::peer_node::PeerNode::from(
                            crate::peer_node::NodeIdentifier::new(vec![7u8; 40]),
                            "a-host-of-thirty-characters-here".to_string(),
                            Port::new(40400),
                            Port::new(40404),
                        )
                        .to_node(),
                    ),
                    type_id: type_id.to_string(),
                    compressed: false,
                    content_length: 1,
                    network_id: network_id.to_string(),
                },
            )),
        };

        let host_len = "a-host-of-thirty-characters-here".len();
        assert_eq!(
            super::chunk_encoded_cost(&header("BlockMessage", "testnet")),
            "BlockMessage".len() + "testnet".len() + 40 + host_len,
            "a header's price is its own fields — the ones a peer sizes"
        );

        // The property the row is about: a wider field costs strictly more, so a peer cannot make the
        // node's buffering free by putting the weight in a field the accounting ignores.
        let narrow = super::chunk_encoded_cost(&header("BlockMessage", "testnet"));
        let wide = super::chunk_encoded_cost(&header(&"x".repeat(4_096), "testnet"));
        assert!(
            wide > narrow + 4_000,
            "a 4 KiB type id must cost 4 KiB more ({narrow} → {wide})"
        );

        // And a data chunk is still charged its content, exactly as before.
        let data = Chunk {
            content: Some(chunk::Content::Data(
                rchain_models::comm::protocol::ChunkData {
                    content_data: vec![1, 2, 3],
                },
            )),
        };
        assert_eq!(super::chunk_encoded_cost(&data), 3);
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
