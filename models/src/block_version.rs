//! Block versions, and the schedule that says which one a block at a given height must carry.
//!
//! Mirrors `models/src/main/scala/coop/rchain/models/BlockVersion.scala` for the constants; the
//! schedule is the port's own.
//!
//! **Why a schedule, and not just a list.** A node replays a chain from its genesis, so a rule change
//! that a binary simply starts applying is applied to every *old* block too, and the node then
//! disagrees with the history it is replaying. Until this existed the only clean way to change what a
//! block means was a new genesis — which is why the rholang bit operations (#279) shipped as a hard
//! fork, and why `docs/src/spec/testnet-acceptance.md` §5 listed "hard-fork / activation gate" as
//! absent. The schedule makes a rule change a *scheduled* one instead: a release adds a row
//! `(version, from_height)`, blocks below the height keep the old version and the old rules, blocks
//! at or above it must carry the new version, and code that implements the new rule asks
//! [`is_active`] for the block it is executing. Every node still has to run the new binary before
//! the height — no protocol avoids that for a validity change — but the chain and its state carry on.
//!
//! **What a row may not do.** A row's height has to be above every height a running chain has already
//! finalised under the old rule, or the new binary refuses history it accepted. The table is compiled
//! in and fixed at release time (the plan's H1, `docs/src/contributor/post-quantum-plan.md` §14); a
//! governance-set table (H2) is the follow-up, and it reads the same shape.

/// One row of the activation schedule: a block at `from_height` or above carries `version`, until the
/// next row's height.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Activation {
    pub version: i32,
    pub from_height: i64,
}

/// The activation schedule, in strictly increasing `version` and `from_height`, starting at height 0.
///
/// One row today: version 1 from genesis, which is exactly the rule every chain has run, so adding the
/// schedule changes no block. The next fork adds its row here.
pub const SCHEDULE: &[Activation] = &[Activation {
    version: 1,
    from_height: 0,
}];

/// The newest block version this software implements (the last row of [`SCHEDULE`]).
pub const CURRENT: i32 = 1;

/// All block versions this software implements, oldest first.
pub const SUPPORTED: [i32; 1] = [CURRENT];

/// The version a block at `height` must carry.
///
/// A height below the first row's — which the shipped table, starting at 0, never has — reads as the
/// first row: the schedule says nothing earlier, and the oldest rule is the one a chain started with.
pub fn version_at(height: i64) -> i32 {
    SCHEDULE
        .iter()
        .take_while(|row| row.from_height <= height)
        .last()
        .unwrap_or(&SCHEDULE[0])
        .version
}

/// Whether the rules introduced by `version` apply to a block at `height`.
///
/// This is the question new consensus code asks, rather than comparing a block's own `version` field:
/// the field is checked against [`version_at`] on the block path, so the two agree on every accepted
/// block, but the height is what a replay of an old block has before it has a block to read.
pub fn is_active(version: i32, height: i64) -> bool {
    version_at(height) >= version
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// The table's own shape: it starts at genesis, both columns strictly increase, the last row is
    /// [`CURRENT`], and every version in it is one [`SUPPORTED`] names. A release that adds a row out
    /// of order, or forgets to move `CURRENT`, fails here rather than on a live net.
    #[test]
    fn the_schedule_is_well_formed() {
        assert_eq!(SCHEDULE[0].from_height, 0, "the schedule starts at genesis");
        for pair in SCHEDULE.windows(2) {
            assert!(pair[0].version < pair[1].version, "versions increase");
            assert!(
                pair[0].from_height < pair[1].from_height,
                "heights increase"
            );
        }
        assert_eq!(SCHEDULE.last().unwrap().version, CURRENT);
        let scheduled: Vec<i32> = SCHEDULE.iter().map(|row| row.version).collect();
        assert_eq!(scheduled, SUPPORTED.to_vec());
    }

    /// Today's single row reproduces the rule every chain has run: version 1 at every height.
    #[test]
    fn the_shipped_schedule_changes_no_block() {
        for height in [0, 1, 1_000, i64::MAX] {
            assert_eq!(version_at(height), 1);
        }
    }

    proptest! {
        /// The version never goes backwards as the chain grows, so a rule once active stays active.
        #[test]
        fn the_version_is_monotone_in_height(a in any::<i64>(), b in any::<i64>()) {
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            prop_assert!(version_at(lo) <= version_at(hi));
        }

        /// `is_active` is exactly "the scheduled version has reached this one".
        #[test]
        fn is_active_reads_the_schedule(v in 0i32..4, h in any::<i64>()) {
            prop_assert_eq!(is_active(v, h), version_at(h) >= v);
        }
    }
}
