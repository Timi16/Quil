//! Tests for the helpers at the root of `quill_core`.

use quill_core::{median, slot_of, SLOT_NANOS};

#[test]
fn median_of_nothing_is_none() {
    assert_eq!(median(&mut []), None);
}

#[test]
fn median_odd_and_even() {
    assert_eq!(median(&mut [5]), Some(5));
    assert_eq!(median(&mut [3, 1, 2]), Some(2));
    // The worked example in the docs: four provider medians give 3.055.
    assert_eq!(
        median(&mut [2_840_000, 2_990_000, 3_120_000, 3_210_000]),
        Some(3_055_000)
    );
}

#[test]
fn median_does_not_overflow() {
    assert_eq!(median(&mut [u128::MAX, u128::MAX]), Some(u128::MAX));
    assert_eq!(median(&mut [u128::MAX, u128::MAX - 1]), Some(u128::MAX - 1));
}

#[test]
fn slots_are_hours() {
    assert_eq!(slot_of(0), 0);
    assert_eq!(slot_of(SLOT_NANOS - 1), 0);
    assert_eq!(slot_of(SLOT_NANOS), 1);
}
