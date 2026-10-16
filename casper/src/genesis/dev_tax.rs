//! The developer allocation — a genesis vault derived from the total the wallets file allocates.
//!
//! **A deliberate, registered deviation, not a port.** The Scala ceremony installs exactly the vaults
//! its wallets file names. This module adds one vault that no wallets file contains, so it is recorded
//! as a hard-fork deviation in `spec/audit/passes.md` and carries Law 78 in `spec/Rchain/Genesis.lean`:
//! the rule below is stated and proved there, and this file is the Rust the law is tied to.
//!
//! **Why a derived vault rather than a data line.** REV has no mint — supply *is* the genesis wallets
//! file (`rholang/src/system_processes.rs`, "REV is minted at genesis only") — and the vault balances
//! are installed as native state outside the genesis block's deploys, so they are content-addressed
//! into the genesis post-state hash (AUDIT C46). A number fixed in a file the operator supplies would
//! therefore be a number a joiner can disagree about silently; deriving it from the same file every
//! node already reads makes creator and joiner compute the same hash by construction.
//!
//! The rule is `max(20 000 000 REV, 2% of the allocated total)`. The arms cross at exactly one billion
//! REV (`DEV_TAX_CROSSOVER`), so below that allocation the floor governs and above it the percentage
//! does.

use rchain_rholang::util::rev_address::RevAddress;
use rchain_shared::refined::NonNegI64;

use super::contracts::Vault;

/// Drops per REV. The tree has no other code constant for this (it is a docs/tests convention —
/// `node/src/api/faucet.rs` prices 0.3 REV at 30 000 000 drops, the same 1e8); introducing it here
/// names the unit the allocation is stated in rather than leaving it implicit in a literal.
pub const DROPS_PER_REV: i128 = 100_000_000;

/// The floor of the allocation, in REV.
pub const DEV_TAX_FLOOR_REV: i128 = 20_000_000;

/// The floor, in drops.
pub const DEV_TAX_FLOOR: i128 = DEV_TAX_FLOOR_REV * DROPS_PER_REV;

/// The rate, in basis points (200 = 2%), on the existing `executor_share` convention.
pub const DEV_TAX_BPS: i128 = 200;

/// The basis-point denominator, on the existing `executor_share` convention.
pub const BPS_DENOMINATOR: i128 = 10_000;

/// The allocated total at which the percentage arm first reaches the floor — **one billion REV**
/// (1e17 drops). Below it the floor governs; at or above it, the percentage.
pub const DEV_TAX_CROSSOVER: i128 = DEV_TAX_FLOOR * BPS_DENOMINATOR / DEV_TAX_BPS;

/// The allocation owed on an allocated total of `total_drops`: `max(20M REV, 2% of the total)`.
///
/// Integer division truncates, so the rounding dust stays with the network rather than the
/// allocation. Overflow is refused rather than clamped, per this crate's rule for a genesis value
/// that decides an amount (`contracts::ProofOfStake` does the same to a negative bond bound).
pub fn dev_tax_amount(total_drops: i128) -> Result<NonNegI64, String> {
    if total_drops < 0 {
        return Err(format!(
            "the allocated total cannot be negative, got {total_drops} drops"
        ));
    }
    let percentage = total_drops
        .checked_mul(DEV_TAX_BPS)
        .ok_or_else(|| format!("the allocated total {total_drops} drops overflows the 2% arm"))?
        / BPS_DENOMINATOR;
    let amount = DEV_TAX_FLOOR.max(percentage);
    let amount = i64::try_from(amount).map_err(|_| {
        format!("the developer allocation of {amount} drops does not fit a vault balance")
    })?;
    NonNegI64::try_from(amount).map_err(|e| e.to_string())
}

/// Parse a configured `dev-tax-address`, for the startup check and for the derivation — one statement
/// of what a valid address is, so the two cannot drift.
pub fn parse_address(address: &str) -> Result<RevAddress, String> {
    RevAddress::parse(address).map_err(|e| {
        format!("casper.genesis-block-data.dev-tax-address `{address}` is not a REV address: {e}")
    })
}

/// The developer vault to append to a parsed wallets file, or `None` when no address is configured.
///
/// **Refuses a wallets file that already names the address.** The caller appends this vault to the
/// list and `set_vault_balance` *overwrites* a repeated key, so a duplicate would silently replace
/// that holder's balance with the allocation instead of adding to it — a defect the type cannot
/// catch, so it is caught here.
pub fn dev_vault(vaults: &[Vault], address: &str) -> Result<Option<Vault>, String> {
    if address.is_empty() {
        return Ok(None);
    }
    let rev_address = parse_address(address)?;
    if vaults.iter().any(|v| v.rev_address == rev_address) {
        return Err(format!(
            "the wallets file already names the developer address {address}: the allocation is \
             appended to the file's vaults, and a duplicate entry would replace that holder's \
             balance rather than add to it"
        ));
    }
    let total = vaults.iter().try_fold(0i128, |acc, v| {
        acc.checked_add(i128::from(i64::from(v.initial_balance)))
            .ok_or_else(|| "the allocated total overflows".to_string())
    })?;
    let initial_balance = dev_tax_amount(total)?;
    Ok(Some(Vault {
        rev_address,
        initial_balance,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// REV as drops, for readable expectations.
    fn rev(n: i128) -> i128 {
        n * DROPS_PER_REV
    }

    fn amount(total_rev: i128) -> i64 {
        i64::from(dev_tax_amount(rev(total_rev)).expect("a valid total"))
    }

    /// **The law's Rust witness.** The floor governs below one billion REV and the percentage at or
    /// above it, and the two arms meet exactly at the crossover.
    #[test]
    fn the_floor_governs_below_a_billion_rev_and_two_percent_above() {
        // Just below the crossover: 2% is under the floor, so the floor governs.
        let just_below = DEV_TAX_CROSSOVER - 1;
        assert!(
            just_below * DEV_TAX_BPS / BPS_DENOMINATOR < DEV_TAX_FLOOR,
            "the crossover must be the *first* total at which 2% reaches the floor"
        );
        assert_eq!(
            i64::from(dev_tax_amount(just_below).unwrap()),
            DEV_TAX_FLOOR as i64
        );

        // At the crossover both arms are equal: one billion REV allocates 20M REV either way.
        assert_eq!(DEV_TAX_CROSSOVER, rev(1_000_000_000));
        assert_eq!(
            DEV_TAX_CROSSOVER * DEV_TAX_BPS / BPS_DENOMINATOR,
            DEV_TAX_FLOOR
        );
        assert_eq!(amount(1_000_000_000), DEV_TAX_FLOOR as i64);

        // Above it the percentage governs and exceeds the floor.
        assert_eq!(amount(2_000_000_000), rev(40_000_000) as i64);
    }

    /// The floor is never undershot, at any total.
    #[test]
    fn the_allocation_is_never_below_the_floor() {
        for total in [0i128, 1, rev(1), rev(999_999), rev(1_000_000_000)] {
            assert!(i64::from(dev_tax_amount(total).unwrap()) >= DEV_TAX_FLOOR as i64);
        }
    }

    /// Monotone: a larger allocation never owes less.
    #[test]
    fn the_allocation_is_monotone_in_the_total() {
        let mut last = 0i64;
        for total in [
            0i128,
            rev(1),
            rev(500_000_000),
            rev(1_000_000_000),
            rev(5_000_000_000),
        ] {
            let now = i64::from(dev_tax_amount(total).unwrap());
            assert!(now >= last, "the allocation fell going up to {total} drops");
            last = now;
        }
    }

    /// The allocation on the allocation this network actually has: 908 962 714 REV, dated 13 May 2026
    /// (`docs/src/node/history-chain.md`). 2% of it is 18 179 254 REV, under the floor, so the
    /// allocation is 20 000 000 REV and the genesis total becomes 928 962 714 REV.
    #[test]
    fn the_published_allocation_takes_the_floor() {
        let published = 908_962_714i128;
        assert_eq!(amount(published), DEV_TAX_FLOOR as i64);
        // 2% of the published total is 18 179 254.28 REV — under the floor, so it is not taken.
        assert_eq!(
            rev(published) * DEV_TAX_BPS / BPS_DENOMINATOR,
            1_817_925_428_000_000
        );
        assert_eq!(
            published + amount(published) as i128 / DROPS_PER_REV,
            928_962_714
        );
    }

    /// The allocation is a whole number of drops and truncates toward the network.
    #[test]
    fn the_percentage_arm_truncates_the_dust() {
        // Above the crossover, and /50 leaves a remainder: the exact quotient is
        // 20 000 000 000 000 000.66, kept as the integer.
        let total = 1_000_000_000_000_000_033i128;
        assert!(total > DEV_TAX_CROSSOVER);
        assert_eq!(
            i64::from(dev_tax_amount(total).unwrap()),
            20_000_000_000_000_000
        );
        let dust = (total * DEV_TAX_BPS) % BPS_DENOMINATOR;
        assert!(
            dust > 0 && dust < BPS_DENOMINATOR,
            "expected the arm to leave dust, got {dust}"
        );
    }

    /// An implausible total is refused, not clamped.
    #[test]
    fn an_implausible_total_is_refused() {
        assert!(dev_tax_amount(-1).is_err());
        assert!(dev_tax_amount(i128::MAX).is_err());
    }

    /// No address configured means no vault — the devnet and testnet path, whose genesis is unchanged.
    #[test]
    fn no_address_allocates_nothing() {
        assert_eq!(dev_vault(&[], "").unwrap(), None);
    }

    /// The vault is appended, and a wallets file that already names the address is refused rather
    /// than folded into it.
    #[test]
    fn the_vault_is_appended_and_a_duplicate_holder_is_refused() {
        let holder = RevAddress::from_public_key(&rchain_crypto::public_key::PublicKey::new({
            let mut key = vec![0u8; 65];
            key[0] = 0x04;
            key
        }))
        .unwrap();
        let dev = RevAddress::from_public_key(&rchain_crypto::public_key::PublicKey::new({
            let mut key = vec![0u8; 65];
            key[0] = 0x04;
            key[1] = 0x01;
            key
        }))
        .unwrap();
        let vaults = vec![Vault {
            rev_address: holder,
            initial_balance: NonNegI64::try_from(rev(908_962_714) as i64).unwrap(),
        }];

        let appended = dev_vault(&vaults, &dev.to_base58()).unwrap().unwrap();
        assert_eq!(
            appended.initial_balance,
            NonNegI64::try_from(rev(20_000_000) as i64).unwrap()
        );
        assert_eq!(appended.rev_address, dev);

        assert!(dev_vault(
            &[
                vaults[0].clone(),
                Vault {
                    rev_address: dev.clone(),
                    initial_balance: NonNegI64::try_from(1).unwrap(),
                }
            ],
            &dev.to_base58()
        )
        .is_err());
    }
}
