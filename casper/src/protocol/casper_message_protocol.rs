//! Per-message packet serde (port of `CasperMessageProtocol.scala`).
//!
//! The packet → `CasperMessageProto` dispatch (`toCasperMessageProto`) lives in `rchain_models`
//! (the casper wire protos are crate-private there).

use rchain_models::casper::protocol::casper_message::{
    BlockHashMessage, BlockMessage, BlockRequest, FinalizedFringe, FinalizedFringeRequest,
    ForkChoiceTipRequest, HasBlock, HasBlockRequest, StoreItemsMessage, StoreItemsMessageRequest,
};
use rchain_models::casper::protocol::packet_type_tag::{
    FromPacket, PacketParseResult, PacketTypeTag, ToPacket,
};

macro_rules! impl_serde {
    ($serde:ident, $model:ty, $tag:expr) => {
        /// Per-message `ToPacket`/`FromPacket` instance.
        pub struct $serde;
        impl ToPacket<$model> for $serde {
            fn tag(&self) -> PacketTypeTag {
                $tag
            }
            fn content(&self, model: &$model) -> Vec<u8> {
                model.to_bytes()
            }
        }
        impl FromPacket<$model> for $serde {
            fn parse(&self, content: &[u8]) -> PacketParseResult<$model> {
                <$model>::from_bytes(content)
            }
        }
    };
}

impl_serde!(BlockMessageSerde, BlockMessage, PacketTypeTag::BlockMessage);
impl_serde!(
    BlockHashMessageSerde,
    BlockHashMessage,
    PacketTypeTag::BlockHashMessage
);
impl_serde!(BlockRequestSerde, BlockRequest, PacketTypeTag::BlockRequest);
impl_serde!(HasBlockSerde, HasBlock, PacketTypeTag::HasBlock);
impl_serde!(
    HasBlockRequestSerde,
    HasBlockRequest,
    PacketTypeTag::HasBlockRequest
);
impl_serde!(
    ForkChoiceTipRequestSerde,
    ForkChoiceTipRequest,
    PacketTypeTag::ForkChoiceTipRequest
);
impl_serde!(
    FinalizedFringeSerde,
    FinalizedFringe,
    PacketTypeTag::FinalizedFringe
);
impl_serde!(
    FinalizedFringeRequestSerde,
    FinalizedFringeRequest,
    PacketTypeTag::FinalizedFringeRequest
);
impl_serde!(
    StoreItemsMessageRequestSerde,
    StoreItemsMessageRequest,
    PacketTypeTag::StoreItemsMessageRequest
);
impl_serde!(
    StoreItemsMessageSerde,
    StoreItemsMessage,
    PacketTypeTag::StoreItemsMessage
);

#[cfg(test)]
mod tests {
    use super::*;

    /// This test used to build the request with `hash: vec![1, 2, 3]` and assert it round-trips,
    /// which pinned the defect as intended behaviour: the handler's first statement called
    /// `BlockHash::from_slice`, so any peer could kill the shard's node-launch task with those three
    /// bytes. The hash is now the 32-byte refinement, so the short fixture is unrepresentable. The
    /// *refusal* is pinned where the codec lives, in the models crate
    /// (`block_request_with_a_short_hash_is_refused`, `block_request_from_bytes_refuses_a_short_hash`).
    #[test]
    fn block_request_serde_round_trips() {
        let serde = BlockRequestSerde;
        let req = BlockRequest {
            hash: rchain_models::block_hash::BlockHash::new([1u8; 32]),
        };
        let bytes = serde.content(&req);
        assert_eq!(serde.parse(&bytes).unwrap(), req);
    }

    #[test]
    fn block_hash_message_packet_round_trips() {
        let serde = BlockHashMessageSerde;
        let msg = BlockHashMessage {
            block_hash: rchain_models::block_hash::BlockHash::new([7u8; 32]),
            block_creator: vec![9, 9],
        };
        let packet = serde.mk_packet(&msg);
        assert_eq!(packet.type_id, "BlockHashMessage");
        assert_eq!(serde.parse_from(&packet).unwrap(), msg);
    }
}
