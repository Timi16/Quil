//! Collateral and payout arithmetic for a capped claim.
//!
//! One LONG plus one SHORT is always worth exactly the band width. Rounding is
//! chosen so the contract can never owe more than it holds: minting rounds the
//! collateral **up**, every payout rounds **down**.

use crate::UNIT;

/// Why an amount could not be computed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathError {
    /// `cap` must be strictly greater than `floor`.
    EmptyBand,
    /// The amount is too large to represent.
    Overflow,
}

/// Which side of a market a token is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// Pays more the higher the index settles.
    Long,
    /// Pays more the lower the index settles.
    Short,
}

/// How a market ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Settled on an index value (micro-USD). The value is clamped to the band when paying.
    Settled(u128),
    /// No usable price ever arrived: each side gets half the band.
    Void,
}

/// A market's price band, in micro-USD per unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Band {
    floor: u128,
    cap: u128,
}

impl Band {
    /// A band from `floor` to `cap`. `cap` must be above `floor`.
    pub fn new(floor: u128, cap: u128) -> Result<Self, MathError> {
        if cap <= floor {
            return Err(MathError::EmptyBand);
        }
        Ok(Self { floor, cap })
    }

    pub fn floor(&self) -> u128 {
        self.floor
    }

    pub fn cap(&self) -> u128 {
        self.cap
    }

    /// `cap - floor`: the most one LONG or one SHORT can ever pay, per whole unit.
    pub fn width(&self) -> u128 {
        self.cap - self.floor
    }

    /// `index` limited to the band.
    pub fn clamp(&self, index: u128) -> u128 {
        index.clamp(self.floor, self.cap)
    }

    /// USDC that must be locked to mint `units` of LONG and of SHORT. Rounds up.
    pub fn collateral_for(&self, units: u128) -> Result<u128, MathError> {
        let product = units.checked_mul(self.width()).ok_or(MathError::Overflow)?;
        Ok(product.div_ceil(UNIT))
    }

    /// The most units a `deposit` can mint, and what they cost.
    ///
    /// The cost never exceeds the deposit; the difference is the caller's refund.
    pub fn units_for_deposit(&self, deposit: u128) -> Result<(u128, u128), MathError> {
        let units = deposit.checked_mul(UNIT).ok_or(MathError::Overflow)? / self.width();
        let cost = self.collateral_for(units)?;
        debug_assert!(cost <= deposit);
        Ok((units, cost))
    }

    /// USDC returned for burning `units` of LONG together with `units` of SHORT. Rounds down.
    pub fn pair_value(&self, units: u128) -> Result<u128, MathError> {
        Ok(units.checked_mul(self.width()).ok_or(MathError::Overflow)? / UNIT)
    }

    /// USDC paid to `units` of `side` given how the market ended. Rounds down.
    pub fn payout(&self, side: Side, units: u128, outcome: Outcome) -> Result<u128, MathError> {
        let per_unit = match outcome {
            Outcome::Settled(index) => {
                let clamped = self.clamp(index);
                match side {
                    Side::Long => clamped - self.floor,
                    Side::Short => self.cap - clamped,
                }
            }
            // Symmetric by construction; the odd micro-USD, if any, stays in the contract.
            Outcome::Void => self.width() / 2,
        };
        Ok(units.checked_mul(per_unit).ok_or(MathError::Overflow)? / UNIT)
    }
}
