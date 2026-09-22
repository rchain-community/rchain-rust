//! Block validation status (port of `BlockStatus.scala`).

/// The outcome of validating a block (port of `BlockStatus`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BlockStatus {
    Valid,
    InvalidBlockNumber,
    InvalidRepeatDeploy,
    InvalidSequenceNumber,
    InvalidDeployShardId,
    JustificationRegression,
    NeglectedInvalidBlock,
    InvalidStateHash,
    InvalidBondsCache,
    InvalidRejectedDeploy,
    ContainsExpiredDeploy,
    ContainsFutureDeploy,
    ContainsLowCostDeploy,
}

impl BlockStatus {
    pub fn is_valid(&self) -> bool {
        matches!(self, BlockStatus::Valid)
    }
}

impl std::fmt::Display for BlockStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            BlockStatus::Valid => "valid",
            BlockStatus::InvalidBlockNumber => "invalid block number",
            BlockStatus::InvalidRepeatDeploy => "a deploy was repeated across blocks",
            BlockStatus::InvalidSequenceNumber => "invalid sender sequence number",
            BlockStatus::InvalidDeployShardId => "deploy shard id does not match the block's shard",
            BlockStatus::JustificationRegression => "a justification regressed from its parent",
            BlockStatus::NeglectedInvalidBlock => "an invalid block was used as a justification",
            BlockStatus::InvalidStateHash => {
                "the block's declared post-state hash does not match the state recomputed by \
                 replaying its deploys — a node state-accounting inconsistency, not an error in your \
                 deploy or API call"
            }
            BlockStatus::InvalidBondsCache => "invalid bonds cache",
            BlockStatus::InvalidRejectedDeploy => "the block's rejected-deploy set does not match its parents",
            BlockStatus::ContainsExpiredDeploy => "a deploy has expired",
            BlockStatus::ContainsFutureDeploy => "a deploy has a future validity window",
            BlockStatus::ContainsLowCostDeploy => "a deploy's phlo price is below the minimum",
        };
        write!(f, "{s}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every status, so a variant added without a message (or with a copy-pasted one) fails here.
    const ALL: [BlockStatus; 13] = [
        BlockStatus::Valid,
        BlockStatus::InvalidBlockNumber,
        BlockStatus::InvalidRepeatDeploy,
        BlockStatus::InvalidSequenceNumber,
        BlockStatus::InvalidDeployShardId,
        BlockStatus::JustificationRegression,
        BlockStatus::NeglectedInvalidBlock,
        BlockStatus::InvalidStateHash,
        BlockStatus::InvalidBondsCache,
        BlockStatus::InvalidRejectedDeploy,
        BlockStatus::ContainsExpiredDeploy,
        BlockStatus::ContainsFutureDeploy,
        BlockStatus::ContainsLowCostDeploy,
    ];

    /// `Valid` is the **only** status that is valid: `is_valid` is the one predicate the block
    /// processor branches on, and a non-`Valid` status that answered `true` would admit an invalid
    /// block into the DAG.
    #[test]
    fn only_the_valid_status_is_valid() {
        assert!(BlockStatus::Valid.is_valid());
        for status in ALL {
            if status != BlockStatus::Valid {
                assert!(!status.is_valid(), "{status:?} must not be valid");
            }
        }
    }

    /// Every status renders a distinct, non-empty sentence — these reach an operator through the
    /// API when a block is rejected, and two statuses sharing a message would make the rejection
    /// undiagnosable. The `InvalidStateHash` message is the longest and explains that the mismatch
    /// is an interpreter/state-accounting fault rather than a malformed deploy, so it is asserted
    /// in full.
    #[test]
    fn every_status_renders_a_distinct_message() {
        let mut messages: Vec<String> = Vec::new();
        for status in ALL {
            let message = status.to_string();
            assert!(!message.is_empty(), "{status:?} has no message");
            assert!(
                !messages.contains(&message),
                "two statuses share a message: {message}"
            );
            messages.push(message);
        }
        assert_eq!(messages.len(), ALL.len());

        assert_eq!(BlockStatus::Valid.to_string(), "valid");
        assert_eq!(
            BlockStatus::InvalidStateHash.to_string(),
            "the block's declared post-state hash does not match the state recomputed by replaying \
             its deploys — a node state-accounting inconsistency, not an error in your deploy or API \
             call"
        );
        assert_eq!(
            BlockStatus::InvalidDeployShardId.to_string(),
            "deploy shard id does not match the block's shard"
        );
        assert_eq!(
            BlockStatus::ContainsLowCostDeploy.to_string(),
            "a deploy's phlo price is below the minimum"
        );
    }

    /// The status is a small `Copy` value that hashes: the block processor keeps it in maps and
    /// compares it, so `Eq`/`Hash` must agree (`Valid` equal to itself, distinct from the rest).
    #[test]
    fn the_status_compares_and_hashes_by_value() {
        use std::collections::HashSet;

        let copied = BlockStatus::Valid;
        assert_eq!(copied, BlockStatus::Valid);

        let set: HashSet<BlockStatus> = ALL.into_iter().collect();
        assert_eq!(set.len(), ALL.len(), "all thirteen are distinct");

        // A rejected block's status is never equal to `Valid`, which is what the caller tests.
        assert_ne!(BlockStatus::InvalidBondsCache, BlockStatus::Valid);
    }
}
