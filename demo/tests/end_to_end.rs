//! Whole-system scenarios: both contracts and a mock USDC ledger in one process.

use claim_market::Side;
use near_sdk::json_types::U128;
use quill_core::settlement::GRACE_SLOTS;
use quill_core::UNIT;
use quill_demo::{acct, refused, Reading, SettlementView, World};

const START: u64 = 490_000;
const EXPIRY: u64 = START + 24;

fn h100(value: u64) -> [Reading<'static>; 1] {
    [Reading {
        index: "H100",
        value,
        sources: 4,
    }]
}

/// A world with one H100 market (band 1.50 to 5.00) and Alice holding 100 pairs.
fn world_with_position() -> (World, u32) {
    let alice = acct("alice.testnet");
    let mut w = World::new(START);
    w.fund(&alice, 350 * UNIT);
    let market = w.create_market(&alice, "H100", 1_500_000, 5_000_000, EXPIRY);
    assert_eq!(w.mint(&alice, market, 350 * UNIT), 100 * UNIT);
    (w, market)
}

fn fix_every_hour(w: &mut World, hours: u64, value: u64) {
    for _ in 0..hours {
        assert_eq!(w.fixing(&h100(value)), 1);
        w.advance(1);
    }
}

#[test]
fn mint_fix_settle_claim_conserves_every_micro_usdc() {
    let (alice, bob) = (acct("alice.testnet"), acct("bob.testnet"));
    let (mut w, market) = world_with_position();
    let total = w.total_usdc();
    w.transfer(&alice, &bob, market, Side::Short, 100 * UNIT);

    fix_every_hour(&mut w, 24, 3_060_000);
    assert_eq!(w.settle(market), SettlementView::Final(U128(3_060_000)));

    assert_eq!(w.claim(&alice, market), 156 * UNIT);
    assert_eq!(w.claim(&bob, market), 194 * UNIT);
    assert_eq!(w.usdc_of(&World::market_id()), 0);
    assert_eq!(w.total_usdc(), total);
}

#[test]
fn a_refused_mint_moves_no_usdc() {
    let alice = acct("alice.testnet");
    let (mut w, market) = world_with_position();
    w.fund(&alice, 10 * UNIT);
    w.advance(24); // the market has expired
    let message = refused(|| w.mint(&alice, market, 10 * UNIT));
    assert!(message.contains("Market is closed to minting"), "{message}");
    assert_eq!(w.usdc_of(&alice), 10 * UNIT);
    assert_eq!(w.usdc_of(&World::market_id()), 350 * UNIT);
}

#[test]
fn redeeming_a_pair_early_returns_the_exact_collateral() {
    let alice = acct("alice.testnet");
    let (mut w, market) = world_with_position();
    assert_eq!(w.redeem_pair(&alice, market, 40 * UNIT), 140 * UNIT);
    assert_eq!(w.usdc_of(&alice), 140 * UNIT);
    assert_eq!(w.market_view(market).locked.0, 210 * UNIT);
}

#[test]
fn a_usdc_outage_never_loses_a_position() {
    let alice = acct("alice.testnet");
    let (mut w, market) = world_with_position();
    w.usdc_down = true;
    assert_eq!(w.redeem_pair(&alice, market, 40 * UNIT), 0);
    assert_eq!(w.balance(market, Side::Long, &alice), 100 * UNIT);
    assert_eq!(w.balance(market, Side::Short, &alice), 100 * UNIT);
    assert_eq!(w.market_view(market).locked.0, 350 * UNIT);

    w.usdc_down = false;
    assert_eq!(w.redeem_pair(&alice, market, 100 * UNIT), 350 * UNIT);
    assert_eq!(w.usdc_of(&World::market_id()), 0);
}

#[test]
fn an_oracle_that_goes_quiet_extends_the_window_then_settles() {
    let alice = acct("alice.testnet");
    let (mut w, market) = world_with_position();
    // Only 16 of the 24 hours get a fixing before expiry.
    fix_every_hour(&mut w, 16, 3_000_000);
    w.advance(8);
    assert_eq!(w.slot, EXPIRY);
    assert_eq!(w.settle(market), SettlementView::Waiting);
    assert!(refused(|| w.claim(&alice, market)).contains("Market is not settled yet"));

    // The oracle comes back; two more hours reach the quorum of 18.
    fix_every_hour(&mut w, 2, 3_900_000);
    assert_eq!(w.settle(market), SettlementView::Final(U128(3_100_000))); // (16 * 3.0 + 2 * 3.9) / 18
}

#[test]
fn an_oracle_that_never_returns_ends_in_a_fair_void() {
    let (alice, bob) = (acct("alice.testnet"), acct("bob.testnet"));
    let (mut w, market) = world_with_position();
    w.transfer(&alice, &bob, market, Side::Short, 100 * UNIT);

    w.advance(24 + GRACE_SLOTS - 1);
    assert_eq!(w.settle(market), SettlementView::Waiting);
    w.advance(1);
    assert_eq!(w.settle(market), SettlementView::Void);

    // Each side gets half the band: funds are never stuck.
    assert_eq!(w.claim(&alice, market), 175 * UNIT);
    assert_eq!(w.claim(&bob, market), 175 * UNIT);
    assert_eq!(w.usdc_of(&World::market_id()), 0);
}

#[test]
fn a_spike_in_the_last_hour_does_not_move_settlement() {
    let (mut w, market) = world_with_position();
    fix_every_hour(&mut w, 23, 3_000_000);
    // Someone floods the marketplaces in the final hour: the reading is flagged and left out.
    assert_eq!(w.fixing(&h100(30_000_000)), 1);
    w.advance(1);
    assert_eq!(w.settle(market), SettlementView::Final(U128(3_000_000)));
}

#[test]
fn settlement_cannot_be_rewritten_by_later_fixings() {
    let (mut w, market) = world_with_position();
    fix_every_hour(&mut w, 24, 3_060_000);
    assert_eq!(w.settle(market), SettlementView::Final(U128(3_060_000)));
    fix_every_hour(&mut w, 5, 4_500_000);
    assert!(refused(|| w.settle(market)).contains("Market is already settled"));
    assert_eq!(w.market_view(market).settled_at, Some(U128(3_060_000)));
}

#[test]
fn a_market_on_an_index_the_oracle_never_publishes_voids() {
    let alice = acct("alice.testnet");
    let mut w = World::new(START);
    w.fund(&alice, 100 * UNIT);
    let market = w.create_market(&alice, "MADE_UP", 1_000_000, 2_000_000, EXPIRY);
    w.mint(&alice, market, 100 * UNIT);
    w.advance(24 + GRACE_SLOTS);
    assert_eq!(w.settle(market), SettlementView::Void);
    // She holds both sides, so she gets everything back.
    assert_eq!(w.claim(&alice, market), 100 * UNIT);
}
