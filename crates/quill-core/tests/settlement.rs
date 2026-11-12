//! Tests for `quill_core::settlement`.

use quill_core::settlement::*;

use std::collections::BTreeMap;

const EXPIRY: u64 = 10_000;

fn lookup(fixings: &BTreeMap<u64, u128>) -> impl Fn(u64) -> Option<u128> + '_ {
    |slot| fixings.get(&slot).copied()
}

fn filled(slots: impl IntoIterator<Item = u64>, value: u128) -> BTreeMap<u64, u128> {
    slots.into_iter().map(|s| (s, value)).collect()
}

#[test]
fn waits_until_expiry() {
    let f = filled(EXPIRY - 24..EXPIRY, 3);
    assert_eq!(
        settlement_value(EXPIRY, EXPIRY - 1, lookup(&f)),
        Settlement::Waiting
    );
    assert_eq!(
        settlement_value(EXPIRY, EXPIRY, lookup(&f)),
        Settlement::Final(3)
    );
}

#[test]
fn full_window_is_the_mean_of_all_24() {
    let mut f = BTreeMap::new();
    for (i, slot) in (EXPIRY - 24..EXPIRY).enumerate() {
        f.insert(slot, 2_000_000 + i as u128 * 10_000);
    }
    // Mean of 2.00, 2.01, ... 2.23 is 2.115.
    assert_eq!(
        settlement_value(EXPIRY, EXPIRY, lookup(&f)),
        Settlement::Final(2_115_000)
    );
    // Asking later gives the same answer.
    assert_eq!(
        settlement_value(EXPIRY, EXPIRY + 500, lookup(&f)),
        Settlement::Final(2_115_000)
    );
}

#[test]
fn exactly_quorum_settles_and_one_short_waits() {
    let f = filled(EXPIRY - 18..EXPIRY, 5);
    assert_eq!(
        settlement_value(EXPIRY, EXPIRY, lookup(&f)),
        Settlement::Final(5)
    );
    let f = filled(EXPIRY - 17..EXPIRY, 5);
    assert_eq!(
        settlement_value(EXPIRY, EXPIRY, lookup(&f)),
        Settlement::Waiting
    );
}

#[test]
fn fixings_outside_the_window_are_ignored() {
    let mut f = filled(EXPIRY - 24..EXPIRY, 4);
    f.insert(EXPIRY - 25, 1_000_000_000); // just before the window
    f.insert(EXPIRY, 1_000_000_000); // after expiry, not needed
    assert_eq!(
        settlement_value(EXPIRY, EXPIRY + 10, lookup(&f)),
        Settlement::Final(4)
    );
}

#[test]
fn window_extends_one_hour_at_a_time() {
    // 16 in the base window, then fixings at expiry+0 and expiry+1.
    let mut f = filled(EXPIRY - 16..EXPIRY, 10);
    f.insert(EXPIRY, 28);
    f.insert(EXPIRY + 1, 28);
    // Not yet: the slot at expiry+1 is not over.
    assert_eq!(
        settlement_value(EXPIRY, EXPIRY + 1, lookup(&f)),
        Settlement::Waiting
    );
    // 18 fixings once expiry+1 has finished: (16*10 + 2*28) / 18 = 12.
    assert_eq!(
        settlement_value(EXPIRY, EXPIRY + 2, lookup(&f)),
        Settlement::Final(12)
    );
    // Later fixings do not change a value that is already final.
    f.insert(EXPIRY + 2, 1_000);
    assert_eq!(
        settlement_value(EXPIRY, EXPIRY + 50, lookup(&f)),
        Settlement::Final(12)
    );
}

#[test]
fn falls_back_after_grace_with_whatever_exists() {
    let mut f = filled(EXPIRY - 5..EXPIRY, 6); // only 5 in the window
    f.insert(EXPIRY - 100, 12); // older, inside the 7-day lookback
    f.insert(EXPIRY - 500, 999); // too old to count
    assert_eq!(
        settlement_value(EXPIRY, EXPIRY + GRACE_SLOTS - 1, lookup(&f)),
        Settlement::Waiting
    );
    // (5*6 + 12) / 6 = 7
    assert_eq!(
        settlement_value(EXPIRY, EXPIRY + GRACE_SLOTS, lookup(&f)),
        Settlement::Fallback(7)
    );
}

#[test]
fn void_when_nothing_was_ever_fixed() {
    let f = BTreeMap::new();
    assert_eq!(
        settlement_value(EXPIRY, EXPIRY + GRACE_SLOTS - 1, lookup(&f)),
        Settlement::Waiting
    );
    assert_eq!(
        settlement_value(EXPIRY, EXPIRY + GRACE_SLOTS, lookup(&f)),
        Settlement::Void
    );
}

#[test]
fn early_expiry_slots_do_not_underflow() {
    let f = filled(0..3, 9);
    assert_eq!(
        settlement_value(3, 3 + GRACE_SLOTS, lookup(&f)),
        Settlement::Fallback(9)
    );
}

#[test]
fn judge_accepts_first_value_and_values_in_range() {
    assert_eq!(
        judge(7, None, Streak::None),
        (Verdict::Accept, Streak::None)
    );
    for v in [50, 100, 150] {
        assert_eq!(
            judge(v, Some(100), Streak::Above(2)),
            (Verdict::Accept, Streak::None)
        );
    }
}

#[test]
fn judge_flags_a_jump_then_accepts_a_persistent_one() {
    let (v, s) = judge(151, Some(100), Streak::None);
    assert_eq!((v, s), (Verdict::Flag, Streak::Above(1)));
    let (v, s) = judge(400, Some(100), s);
    assert_eq!((v, s), (Verdict::Flag, Streak::Above(2)));
    // Third in a row on the same side: the market really moved.
    assert_eq!(judge(380, Some(100), s), (Verdict::Accept, Streak::None));
}

#[test]
fn judge_resets_the_streak_when_direction_flips() {
    let (_, s) = judge(300, Some(100), Streak::None);
    let (_, s) = judge(300, Some(100), s);
    assert_eq!(s, Streak::Above(2));
    // A crash the other way starts a new run instead of being accepted.
    assert_eq!(judge(10, Some(100), s), (Verdict::Flag, Streak::Below(1)));
}

#[test]
fn judge_handles_extreme_references() {
    assert_eq!(
        judge(u128::MAX, Some(u128::MAX), Streak::None).0,
        Verdict::Accept
    );
    assert_eq!(judge(1, Some(0), Streak::None).0, Verdict::Flag);
    assert_eq!(judge(0, Some(0), Streak::None).0, Verdict::Accept);
}
