//! Request → response ADT for the protocol dispatch layer.
//!
//! Mirrors `comm/src/main/scala/coop/rchain/comm/transport/CommunicationResponse.scala`.

use rchain_models::comm::protocol::Protocol;

use crate::errors::CommError;

/// The result of handling an inbound protocol message (port of `CommunicationResponse`).
#[derive(Clone, Debug, PartialEq)]
pub enum CommunicationResponse {
    HandledWithMessage(Protocol),
    HandledWithoutMessage,
    NotHandled(CommError),
}

impl CommunicationResponse {
    pub fn handled_with_message(protocol: Protocol) -> Self {
        CommunicationResponse::HandledWithMessage(protocol)
    }

    pub fn handled_without_message() -> Self {
        CommunicationResponse::HandledWithoutMessage
    }

    pub fn not_handled(error: CommError) -> Self {
        CommunicationResponse::NotHandled(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn protocol() -> Protocol {
        Protocol {
            header: None,
            message: None,
        }
    }

    /// The three constructors build the three variants, and the error one carries the error it was
    /// given — the dispatch layer's caller matches on the variant to decide whether to reply, so a
    /// constructor that collapsed two cases would swallow a failure.
    #[test]
    fn the_three_constructors_build_three_distinct_variants() {
        let with = CommunicationResponse::handled_with_message(protocol());
        let without = CommunicationResponse::handled_without_message();
        let failed = CommunicationResponse::not_handled(CommError::TimeOut);

        assert_eq!(with, CommunicationResponse::HandledWithMessage(protocol()));
        assert_eq!(without, CommunicationResponse::HandledWithoutMessage);
        assert_eq!(failed, CommunicationResponse::NotHandled(CommError::TimeOut));

        assert_ne!(with, without);
        assert_ne!(without, failed);
        assert_ne!(with, failed);
        match failed {
            CommunicationResponse::NotHandled(error) => {
                assert_eq!(error, CommError::TimeOut);
                assert_eq!(error.message(), "Timeout");
            }
            other => panic!("expected NotHandled, got {other:?}"),
        }
    }
}
