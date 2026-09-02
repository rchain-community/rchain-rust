//! Experimental zero-action ledger (issue #8).
//!
//! This is a pure conflict-detection oracle, not a replacement for RSpace semantics. A
//! balanced pair of sibling effects (`Inject` +1 and `Consume` -1 on the same concrete
//! channel) may form a local COMM proposal. An unbalanced entry is *not* an error: it means
//! the effect must be stored in RSpace or must wait for an existing RSpace counterpart.
//!
//! The phase plan is in `docs/src/qucalc/zfa-concurrent-reducer.md`.

use std::collections::BTreeMap;

/// Error returned when recording a delta would overflow the `i64` balance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZfaLedgerError {
    BalanceOverflow,
}

/// Per-channel balance of in-flight sibling effects.
///
/// The channel key is generic (`SortedProc`, `Blake2b256Hash`, or any `Ord` key) so this
/// module does not depend on the rholang/model crates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZfaLedger<C> {
    balances: BTreeMap<C, i64>,
}

impl<C> Default for ZfaLedger<C> {
    fn default() -> Self {
        ZfaLedger {
            balances: BTreeMap::new(),
        }
    }
}

impl<C: Ord + Clone> ZfaLedger<C> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one +1 (Inject/produce) or -1 (Consume/receive demand) on a channel.
    pub fn record(&mut self, channel: C, delta: i64) -> Result<(), ZfaLedgerError> {
        if delta == 0 {
            return Ok(());
        }
        let key = channel;
        let zero = {
            let balance = self.balances.entry(key.clone()).or_insert(0);
            let new_balance = balance
                .checked_add(delta)
                .ok_or(ZfaLedgerError::BalanceOverflow)?;
            *balance = new_balance;
            new_balance == 0
        };
        if zero {
            self.balances.remove(&key);
        }
        Ok(())
    }

    /// All recorded balances are zero.
    pub fn is_zero_action(&self) -> bool {
        self.balances.values().all(|&v| v == 0)
    }

    /// True when this ledger and `other` touch no channel in common.
    pub fn is_disjoint(&self, other: &Self) -> bool {
        self.balances
            .keys()
            .all(|channel| !other.balances.contains_key(channel))
    }

    /// Union another ledger into this one, preserving no-zero entries.
    pub fn merge(&mut self, other: Self) -> Result<(), ZfaLedgerError> {
        for (channel, delta) in other.balances {
            self.record(channel, delta)?;
        }
        Ok(())
    }

    /// The current balance of `channel`.
    pub fn balance(&self, channel: &C) -> i64 {
        self.balances.get(channel).copied().unwrap_or(0)
    }

    /// The channels currently touched by this ledger.
    pub fn channels(&self) -> impl Iterator<Item = &C> {
        self.balances.keys()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn record_many(ledger: &mut ZfaLedger<u8>, ops: &[(u8, i64)]) -> Result<(), ZfaLedgerError> {
        for &(channel, delta) in ops {
            ledger.record(channel, delta)?;
        }
        Ok(())
    }

    #[test]
    fn inject_then_consume_is_zero() {
        let mut ledger = ZfaLedger::new();
        ledger.record(1, 1).unwrap();
        assert!(!ledger.is_zero_action());
        ledger.record(1, -1).unwrap();
        assert!(ledger.is_zero_action());
        assert_eq!(ledger.balance(&1), 0);
    }

    #[test]
    fn two_injects_are_not_zero() {
        let mut ledger = ZfaLedger::new();
        ledger.record(1, 1).unwrap();
        ledger.record(1, 1).unwrap();
        assert!(!ledger.is_zero_action());
        assert_eq!(ledger.balance(&1), 2);
    }

    #[test]
    fn different_channels_do_not_cancel() {
        let mut ledger = ZfaLedger::new();
        ledger.record(1, 1).unwrap();
        ledger.record(2, -1).unwrap();
        assert!(!ledger.is_zero_action());
        assert!(!ledger.is_disjoint(&ledger));
    }

    #[test]
    fn disjoint_ledgers_are_disjoint() {
        let mut a = ZfaLedger::new();
        let mut b = ZfaLedger::new();
        a.record(1, 1).unwrap();
        b.record(2, 1).unwrap();
        assert!(a.is_disjoint(&b));
        assert!(b.is_disjoint(&a));
    }

    #[test]
    fn overlapping_ledgers_are_not_disjoint() {
        let mut a = ZfaLedger::new();
        let mut b = ZfaLedger::new();
        a.record(1, 1).unwrap();
        b.record(1, -1).unwrap();
        assert!(!a.is_disjoint(&b));
    }

    #[test]
    fn merge_is_commutative_for_disjoint_ledgers() {
        let mut a = ZfaLedger::new();
        let mut b = ZfaLedger::new();
        a.record(1, 1).unwrap();
        b.record(2, -1).unwrap();

        let mut left = a.clone();
        left.merge(b.clone()).unwrap();
        let mut right = b;
        right.merge(a).unwrap();
        assert_eq!(left, right);
        assert_eq!(left.balance(&1), 1);
        assert_eq!(left.balance(&2), -1);
    }

    #[test]
    fn overflow_is_reported_not_silent() {
        let mut ledger = ZfaLedger::new();
        ledger.record(1, i64::MAX).unwrap();
        assert_eq!(ledger.record(1, 1), Err(ZfaLedgerError::BalanceOverflow));
    }

    proptest! {
        /// The ledger is order-insensitive for any bounded sequence of deltas.
        #[test]
        fn record_order_does_not_matter(
            ops in prop::collection::vec((any::<u8>(), -2i64..=2i64), 0..12),
        ) {
            let mut forward = ZfaLedger::new();
            let mut backward = ZfaLedger::new();
            let mut reverse = ops.clone();
            reverse.reverse();

            record_many(&mut forward, &ops).unwrap();
            record_many(&mut backward, &reverse).unwrap();

            prop_assert_eq!(&forward, &backward);
            let total_is_zero = ops
                .iter()
                .fold(BTreeMap::<u8, i64>::new(), |mut acc, &(c, d)| {
                    *acc.entry(c).or_insert(0) += d;
                    acc
                })
                .values()
                .all(|&v| v == 0);
            prop_assert_eq!(forward.is_zero_action(), total_is_zero);
        }
    }
}
