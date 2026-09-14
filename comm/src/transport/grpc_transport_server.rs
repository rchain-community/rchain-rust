//! Server-side transport layer wrapper.
//!
//! Mirrors `comm/src/main/scala/coop/rchain/comm/transport/GrpcTransportServer.scala`.

use std::sync::Arc;

use rchain_models::comm::protocol::Protocol;

use crate::peer_node::PeerNode;
use crate::transport::chunker::Blob;
use crate::transport::communication_response::CommunicationResponse;
use crate::transport::grpc_transport_receiver::{self, BoxFuture, ConcurrencyLimits};
use crate::transport::hostname_trust_manager;

/// The server-side transport (port of `TransportLayerServer` / `GrpcTransportServer`).
pub struct TransportLayerServer {
    local: PeerNode,
    network_id: String,
    port: u16,
    tls: Arc<rustls::ServerConfig>,
    max_stream_message_size: i64,
}

impl TransportLayerServer {
    pub fn new(
        local: PeerNode,
        network_id: String,
        port: u16,
        cert_pem: &str,
        key_pem: &str,
        max_stream_message_size: i64,
    ) -> Result<Self, String> {
        let tls = hostname_trust_manager::server_config(cert_pem, key_pem)?;
        Ok(TransportLayerServer {
            local,
            network_id,
            port,
            tls,
            max_stream_message_size,
        })
    }

    /// Serve the transport, dispatching inbound protocol messages to `dispatch` and reassembled
    /// streamed blobs to `handle_streamed`.
    pub async fn serve<D, S>(&self, dispatch: D, handle_streamed: S) -> Result<(), String>
    where
        D: Fn(Protocol) -> BoxFuture<CommunicationResponse> + Send + Sync + 'static,
        S: Fn(Blob) -> BoxFuture<()> + Send + Sync + 'static,
    {
        self.serve_with_limits(dispatch, handle_streamed, ConcurrencyLimits::default())
            .await
    }

    /// Serve the transport with explicit concurrency bounds (see [`ConcurrencyLimits`]), so a test
    /// can exhaust a bound at a scale it can afford.
    pub async fn serve_with_limits<D, S>(
        &self,
        dispatch: D,
        handle_streamed: S,
        limits: ConcurrencyLimits,
    ) -> Result<(), String>
    where
        D: Fn(Protocol) -> BoxFuture<CommunicationResponse> + Send + Sync + 'static,
        S: Fn(Blob) -> BoxFuture<()> + Send + Sync + 'static,
    {
        grpc_transport_receiver::serve_with_limits(
            self.local.clone(),
            self.network_id.clone(),
            self.port,
            self.tls.clone(),
            self.max_stream_message_size,
            Arc::new(dispatch),
            Arc::new(handle_streamed),
            limits,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rchain_shared::refined::Port;

    use crate::peer_node::NodeIdentifier;
    use crate::transport::generate_certificate_if_absent::generate_certificate;

    fn local() -> PeerNode {
        PeerNode::from(
            NodeIdentifier::new(vec![1u8; 32]),
            "127.0.0.1".to_string(),
            Port::new(40400),
            Port::new(40400),
        )
    }

    /// A server built from a real certificate/key pair is constructed, and the error path is the
    /// one that matters at startup: a node whose cert file is corrupt must fail with an error it can
    /// report, not panic while parsing a PEM. `serve` itself is covered end to end over a socket by
    /// `send_round_trips_over_socket` in `grpc_transport.rs`, which is the only way to exercise the
    /// receiver — this pins what that test cannot: what happens when the *configuration* is wrong.
    #[test]
    fn the_server_is_built_from_a_valid_pair_and_refuses_a_bad_one() {
        let (cert, key) = generate_certificate().expect("a fresh certificate");
        TransportLayerServer::new(
            local(),
            "testnet".to_string(),
            40400,
            &cert,
            &key,
            16 * 1024 * 1024,
        )
        .expect("a valid certificate and key");

        let err = TransportLayerServer::new(
            local(),
            "testnet".to_string(),
            40400,
            "not a certificate",
            &key,
            16 * 1024 * 1024,
        )
        .map(|_| ())
        .expect_err("a garbage certificate");
        assert!(!err.is_empty(), "the error says something: {err}");

        let err = TransportLayerServer::new(
            local(),
            "testnet".to_string(),
            40400,
            &cert,
            "not a key",
            16 * 1024 * 1024,
        )
        .map(|_| ())
        .expect_err("a garbage key");
        assert!(!err.is_empty(), "the error says something: {err}");

        // An empty pair is refused too, rather than yielding a server that cannot accept anything.
        assert!(TransportLayerServer::new(
            local(),
            "testnet".to_string(),
            40400,
            "",
            "",
            16 * 1024 * 1024
        )
        .is_err());
    }

    /// A certificate that parses but is not a key pair for it — the realistic corrupt-file case —
    /// is refused as well: the server config is only built when the key matches the certificate.
    #[test]
    fn a_key_from_another_certificate_is_refused() {
        let (cert, _) = generate_certificate().expect("cert");
        let (_, other_key) = generate_certificate().expect("other");
        assert!(
            TransportLayerServer::new(
                local(),
                "testnet".to_string(),
                40400,
                &cert,
                &other_key,
                16 * 1024 * 1024
            )
            .is_err(),
            "a mismatched key must not produce a working server"
        );
    }
}
