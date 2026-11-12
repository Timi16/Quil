//! Quill's money maths, with no dependencies and no I/O.
//!
//! Everything a contract or the oracle needs to decide an amount lives here so
//! it can be tested exhaustively off-chain and reused unchanged on-chain.
//!
//! Conventions, used everywhere:
//! * USDC amounts are in micro-USDC (6 decimals).
//! * Prices are in micro-USD per unit (one GPU-hour, or one million tokens).
//! * Claim tokens (LONG and SHORT) have 6 decimals: [`UNIT`] base units are
//!   a claim on one whole unit of the underlying.

pub mod index;
pub mod payout;
pub mod settlement;

/// Base units in one whole USDC, one whole USD of price, and one whole claim token.
pub const UNIT: u128 = 1_000_000;

/// Nanoseconds in one hourly fixing slot.
pub const SLOT_NANOS: u64 = 3_600_000_000_000;

/// The hourly slot a nanosecond timestamp falls in.
pub const fn slot_of(timestamp_nanos: u64) -> u64 {
    timestamp_nanos / SLOT_NANOS
}

/// Median of a set of values. For an even count it is the mean of the two
/// middle values, rounded down. Returns `None` for an empty set.
///
/// Sorts `values` in place.
pub fn median(values: &mut [u128]) -> Option<u128> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let mid = values.len() / 2;
    Some(if values.len() % 2 == 1 {
        values[mid]
    } else {
        // a/2 + b/2 + carry avoids overflow for any pair of u128.
        let (a, b) = (values[mid - 1], values[mid]);
        a / 2 + b / 2 + (a % 2 + b % 2) / 2
    })
}
