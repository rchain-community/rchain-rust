//! Communication errors.
//!
//! Mirrors `comm/src/main/scala/coop/rchain/comm/errors.scala`.

use crate::peer_node::PeerNode;

/// A communication error (port of the sealed `CommError`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommError {
    UnknownCommError(String),
    DatagramSizeError(i32),
    HeaderNotAvailable,
    ProtocolException(String),
    UnknownProtocolError(String),
    PublicKeyNotAvailable(PeerNode),
    ParseError(String),
    EncryptionHandshakeIncorrectlySigned,
    BootstrapNotProvided,
    PeerNodeNotFound(PeerNode),
    PeerUnavailable(PeerNode),
    WrongNetwork(PeerNode, String),
    MessageTooLarge(PeerNode),
    CouldNotConnectToBootstrap,
    InternalCommunicationError(String),
    TimeOut,
    UpstreamNotAvailable,
    UnexpectedMessage(String),
    SenderNotAvailable,
    PongNotReceivedForPing(PeerNode),
    UnableToStorePacket(String),
    UnableToRestorePacket(String),
}

/// The `CommErr` result alias (port of `CommError.CommErr`).
pub type CommErr<A> = Result<A, CommError>;

impl CommError {
    /// The human-readable message (port of `CommError.errorMessage`).
    pub fn message(&self) -> String {
        match self {
            CommError::PeerUnavailable(_) => "Peer is currently unavailable".to_string(),
            CommError::MessageTooLarge(p) => {
                format!("Message rejected by peer {p} because it was too large")
            }
            CommError::PongNotReceivedForPing(_) => {
                "Peer is behind a firewall and can't be accessed from outside".to_string()
            }
            CommError::CouldNotConnectToBootstrap => {
                "Node could not connect to bootstrap node".to_string()
            }
            CommError::TimeOut => "Timeout".to_string(),
            CommError::InternalCommunicationError(msg) => {
                format!("Internal communication error. {msg}")
            }
            CommError::UnknownProtocolError(msg) => format!("Unknown protocol error. {msg}"),
            CommError::UnableToStorePacket(p) => {
                format!("Could not serialize packet {p}.")
            }
            CommError::UnableToRestorePacket(p) => {
                format!("Could not deserialize packet {p}.")
            }
            CommError::ProtocolException(msg) => format!("Protocol error. {msg}"),
            other => format!("{other:?}"),
        }
    }
}

impl std::fmt::Display for CommError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for CommError {}

#[cfg(test)]
mod tests {
    use super::*;
    use rchain_shared::refined::Port;

    use crate::peer_node::NodeIdentifier;

    fn peer() -> PeerNode {
        PeerNode::from(
            NodeIdentifier::new(vec![7u8; 32]),
            "host".to_string(),
            Port::new(40400),
            Port::new(40404),
        )
    }

    /// The variants that carry a **human message** are the ones an operator reads when a node
    /// cannot peer: each renders the Scala `errorMessage` text, and the three that interpolate their
    /// payload do so (a fence-post that lost its message would read "Could not serialize packet .").
    /// The rest fall through to the derived `Debug`, which is the Scala's own fallback arm.
    #[test]
    fn the_named_messages_are_the_scala_texts_and_the_rest_fall_through_to_debug() {
        assert_eq!(
            CommError::PeerUnavailable(peer()).message(),
            "Peer is currently unavailable"
        );
        assert_eq!(
            CommError::MessageTooLarge(peer()).message(),
            format!(
                "Message rejected by peer {} because it was too large",
                peer()
            )
        );
        assert_eq!(
            CommError::PongNotReceivedForPing(peer()).message(),
            "Peer is behind a firewall and can't be accessed from outside"
        );
        assert_eq!(
            CommError::CouldNotConnectToBootstrap.message(),
            "Node could not connect to bootstrap node"
        );
        assert_eq!(CommError::TimeOut.message(), "Timeout");
        assert_eq!(
            CommError::InternalCommunicationError("disk full".to_string()).message(),
            "Internal communication error. disk full"
        );
        assert_eq!(
            CommError::UnknownProtocolError("no such message".to_string()).message(),
            "Unknown protocol error. no such message"
        );
        assert_eq!(
            CommError::UnableToStorePacket("Blob".to_string()).message(),
            "Could not serialize packet Blob."
        );
        assert_eq!(
            CommError::UnableToRestorePacket("Chunk".to_string()).message(),
            "Could not deserialize packet Chunk."
        );
        assert_eq!(
            CommError::ProtocolException("bad handshake".to_string()).message(),
            "Protocol error. bad handshake"
        );

        // The fallback arm: a variant with no Scala message renders its `Debug` form, so a caller
        // still gets something identifying rather than an empty string.
        assert_eq!(
            CommError::HeaderNotAvailable.message(),
            format!("{:?}", CommError::HeaderNotAvailable)
        );
        assert!(!CommError::BootstrapNotProvided.message().is_empty());
        assert!(CommError::UpstreamNotAvailable
            .message()
            .contains("UpstreamNotAvailable"));
        assert!(CommError::SenderNotAvailable
            .message()
            .contains("SenderNotAvailable"));
    }

    /// `Display` is `message()`, so an error printed with `{}` and one printed with `{:?}` differ
    /// exactly where the Scala's do — and the type is a real `Error` value for `?`/`Box<dyn Error>`.
    #[test]
    fn display_is_the_message_and_it_is_an_error_value() {
        let error = CommError::ProtocolException("bad handshake".to_string());
        assert_eq!(error.to_string(), error.message());
        assert_ne!(format!("{error:?}"), error.to_string());

        let boxed: Box<dyn std::error::Error> = Box::new(CommError::TimeOut);
        assert_eq!(boxed.to_string(), "Timeout");

        assert_eq!(CommError::TimeOut, CommError::TimeOut.clone());
        assert_ne!(CommError::TimeOut, CommError::UpstreamNotAvailable);
    }
}
