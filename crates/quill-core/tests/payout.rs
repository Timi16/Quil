//! Tests for `quill_core::payout`.

use quill_core::payout::{Band, MathError, Outcome, Side};
use quill_core::UNIT;

const H100: (u128, u128) = (1_500_000, 5_000_000);

fn band() -> Band {
    Band::new(H100.0, H100.1).unwrap()
}

/// Small deterministic generator so the property tests need no dependency.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 11
    }
    fn below(&mut self, n: u64) -> u128 {
        (self.next() % n) as u128
    }
}

#[test]
fn rejects_empty_or_inverted_bands() {
    assert_eq!(Band::new(5, 5), Err(MathError::EmptyBand));
    assert_eq!(Band::new(6, 5), Err(MathError::EmptyBand));
}

#[test]
fn worked_example_from_the_docs() {
    // 100 GPU-hours, band 1.50 to 5.00, index settles at 3.06.
    let b = band();
    let units = 100 * UNIT;
    assert_eq!(b.collateral_for(units), Ok(350 * UNIT));
    assert_eq!(b.units_for_deposit(350 * UNIT), Ok((units, 350 * UNIT)));
    let at = Outcome::Settled(3_060_000);
    assert_eq!(b.payout(Side::Long, units, at), Ok(156 * UNIT));
    assert_eq!(b.payout(Side::Short, units, at), Ok(194 * UNIT));
}

#[test]
fn index_outside_the_band_is_clamped() {
    let b = band();
    let units = 10 * UNIT;
    for below in [0, 1, 1_499_999, 1_500_000] {
        assert_eq!(b.payout(Side::Long, units, Outcome::Settled(below)), Ok(0));
        assert_eq!(
            b.payout(Side::Short, units, Outcome::Settled(below)),
            Ok(35 * UNIT)
        );
    }
    for above in [5_000_000, 5_000_001, u128::MAX] {
        assert_eq!(
            b.payout(Side::Long, units, Outcome::Settled(above)),
            Ok(35 * UNIT)
        );
        assert_eq!(b.payout(Side::Short, units, Outcome::Settled(above)), Ok(0));
    }
}

#[test]
fn void_pays_each_side_half() {
    let b = band();
    let units = 4 * UNIT;
    assert_eq!(b.payout(Side::Long, units, Outcome::Void), Ok(7 * UNIT));
    assert_eq!(b.payout(Side::Short, units, Outcome::Void), Ok(7 * UNIT));
}

#[test]
fn minting_rounds_up_and_refunds_the_rest() {
    // A band whose width does not divide evenly: 1 base unit costs a fraction of a micro-USDC.
    let b = Band::new(1, 4).unwrap(); // width 3 micro-USD
    assert_eq!(b.collateral_for(1), Ok(1)); // ceil(3 / 1e6) = 1
    assert_eq!(b.collateral_for(UNIT), Ok(3));
    assert_eq!(b.collateral_for(UNIT + 1), Ok(4)); // ceil(3.000003) = 4
                                                   // 10 micro-USDC buys floor(10e6 / 3) units and costs exactly 10.
    let (units, cost) = b.units_for_deposit(10).unwrap();
    assert_eq!(units, 3_333_333);
    assert_eq!(cost, 10);
}

#[test]
fn zero_is_handled() {
    let b = band();
    assert_eq!(b.units_for_deposit(0), Ok((0, 0)));
    assert_eq!(b.collateral_for(0), Ok(0));
    assert_eq!(b.pair_value(0), Ok(0));
    assert_eq!(b.payout(Side::Long, 0, Outcome::Settled(3_000_000)), Ok(0));
}

#[test]
fn overflow_is_an_error_not_a_panic() {
    let b = band();
    assert_eq!(b.collateral_for(u128::MAX), Err(MathError::Overflow));
    assert_eq!(b.units_for_deposit(u128::MAX), Err(MathError::Overflow));
    assert_eq!(b.pair_value(u128::MAX), Err(MathError::Overflow));
    assert_eq!(
        b.payout(Side::Short, u128::MAX, Outcome::Void),
        Err(MathError::Overflow)
    );
}

/// The solvency property, over many random markets:
/// whatever is minted in however many pieces, and however it is later split
/// between holders, redeemed or settled, total payouts never exceed the
/// collateral locked.
#[test]
fn payouts_never_exceed_locked_collateral() {
    let mut rng = Lcg(0x5EED);
    for _ in 0..20_000 {
        let floor = rng.below(20_000_000);
        let b = Band::new(floor, floor + 1 + rng.below(50_000_000)).unwrap();

        // Several deposits of awkward sizes.
        let mut locked = 0u128;
        let mut supply = 0u128;
        for _ in 0..1 + rng.below(6) {
            let deposit = rng.below(5_000_000_000);
            let (units, cost) = b.units_for_deposit(deposit).unwrap();
            assert!(cost <= deposit);
            locked += cost;
            supply += units;
        }

        // Some pairs are redeemed early.
        let redeemed = if supply == 0 {
            0
        } else {
            rng.below(supply as u64 + 1)
        };
        let mut paid = b.pair_value(redeemed).unwrap();
        let outstanding = supply - redeemed;

        // The rest is held by several accounts, LONG and SHORT split differently.
        let outcome = match rng.below(10) {
            0 => Outcome::Void,
            _ => Outcome::Settled(rng.below(90_000_000)),
        };
        for side in [Side::Long, Side::Short] {
            let mut left = outstanding;
            while left > 0 {
                let chunk = 1 + rng.below(left as u64);
                paid += b.payout(side, chunk, outcome).unwrap();
                left -= chunk;
            }
        }
        assert!(
            paid <= locked,
            "paid {paid} > locked {locked} for {b:?} {outcome:?}"
        );
    }
}

/// LONG plus SHORT is worth the band width at every settlement value, to within rounding dust.
#[test]
fn long_plus_short_equals_width() {
    let mut rng = Lcg(42);
    for _ in 0..20_000 {
        let floor = rng.below(10_000_000);
        let b = Band::new(floor, floor + 1 + rng.below(10_000_000)).unwrap();
        let units = rng.below(1_000_000_000_000);
        let outcome = Outcome::Settled(rng.below(25_000_000));
        let long = b.payout(Side::Long, units, outcome).unwrap();
        let short = b.payout(Side::Short, units, outcome).unwrap();
        let whole = b.pair_value(units).unwrap();
        assert!(long + short <= whole);
        assert!(
            whole - (long + short) <= 1,
            "more than one micro-USDC of dust"
        );
    }
}
