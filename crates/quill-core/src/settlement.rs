//! Which price a market settles on, and when.
//!
//! A market never settles on one reading. It settles on the mean of the hourly
//! fixings in the 24 hours before expiry, with a defined rule for every way
//! that window can come up short. No rule needs an operator's decision.

/// Hourly slots in the settlement window before expiry.
pub const WINDOW_SLOTS: u64 = 24;
/// Fixings needed in the window for a normal settlement.
pub const QUORUM: usize = 18;
/// How many slots past expiry the window may extend to reach [`QUORUM`].
pub const GRACE_SLOTS: u64 = 72;
/// How far before expiry the last-resort fallback looks.
pub const FALLBACK_SLOTS: u64 = 168;

/// The state of a market's settlement value at a given moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settlement {
    /// Not decidable yet: expiry has not passed, or the window is still filling.
    Waiting,
    /// Mean of at least [`QUORUM`] fixings from the window (possibly extended).
    Final(u128),
    /// Quorum was never reached: mean of whatever fixings exist near expiry.
    Fallback(u128),
    /// No fixing exists anywhere near expiry.
    Void,
}

/// The oracle's judgement of one incoming value against recent history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Use it.
    Accept,
    /// Keep it on record but leave it out of settlement and history.
    Flag,
}

/// Which way a run of flagged values has been pointing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Streak {
    #[default]
    None,
    /// `n` consecutive flagged values above the reference.
    Above(u8),
    /// `n` consecutive flagged values below the reference.
    Below(u8),
}

/// Flagged values in a row, all on the same side, after which the next one is accepted.
/// Stops one honest jump from freezing the oracle forever.
pub const STREAK_TO_ACCEPT: u8 = 2;

/// Decide whether `candidate` is plausible given the `reference` (the median of
/// recent accepted fixings) and the current run of flagged values.
///
/// A value within 50% of the reference is accepted. One outside is flagged,
/// unless the previous [`STREAK_TO_ACCEPT`] values were flagged on the same
/// side: then the market has really moved and the value is accepted.
pub fn judge(candidate: u128, reference: Option<u128>, streak: Streak) -> (Verdict, Streak) {
    let Some(reference) = reference else {
        return (Verdict::Accept, Streak::None);
    };
    let low = reference / 2;
    let high = reference.saturating_add(reference / 2);
    if (low..=high).contains(&candidate) {
        return (Verdict::Accept, Streak::None);
    }
    let above = candidate > high;
    let run = match (streak, above) {
        (Streak::Above(n), true) | (Streak::Below(n), false) => n,
        _ => 0,
    };
    if run >= STREAK_TO_ACCEPT {
        (Verdict::Accept, Streak::None)
    } else if above {
        (Verdict::Flag, Streak::Above(run + 1))
    } else {
        (Verdict::Flag, Streak::Below(run + 1))
    }
}

/// The settlement value for a market expiring at `expiry_slot`, as known at `now_slot`.
///
/// `fixing_at(slot)` returns the accepted fixing for an hourly slot, if any.
/// A slot only counts once it is over (`slot < now_slot`), so the answer for a
/// given set of fixings never changes once it stops being `Waiting`.
pub fn settlement_value(
    expiry_slot: u64,
    now_slot: u64,
    fixing_at: impl Fn(u64) -> Option<u128>,
) -> Settlement {
    if now_slot < expiry_slot {
        return Settlement::Waiting;
    }
    let window_start = expiry_slot.saturating_sub(WINDOW_SLOTS);
    let grace_end = expiry_slot.saturating_add(GRACE_SLOTS);

    // The normal window, then one extra hour at a time until quorum.
    let mut sum = 0u128;
    let mut count = 0usize;
    for slot in window_start..grace_end.min(now_slot) {
        if let Some(value) = fixing_at(slot) {
            sum = sum.saturating_add(value);
            count += 1;
        }
        // Quorum is only checked at the end of the base window and after each extra hour,
        // so a full base window always uses all of its fixings.
        if slot + 1 >= expiry_slot && count >= QUORUM {
            return Settlement::Final(sum / count as u128);
        }
    }
    if now_slot < grace_end {
        return Settlement::Waiting;
    }

    // Quorum never arrived. Use everything from a week before expiry to the end of grace.
    let fallback_start = expiry_slot.saturating_sub(FALLBACK_SLOTS);
    let (mut sum, mut count) = (sum, count);
    for slot in fallback_start..window_start {
        if let Some(value) = fixing_at(slot) {
            sum = sum.saturating_add(value);
            count += 1;
        }
    }
    if count == 0 {
        Settlement::Void
    } else {
        Settlement::Fallback(sum / count as u128)
    }
}
