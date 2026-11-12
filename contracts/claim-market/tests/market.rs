//! Behaviour tests for `claim-market`, through its public interface only.

use claim_market::{ClaimMarket, SettlementView, Side, MARKET_STORAGE_DEPOSIT};
use near_sdk::json_types::U128;
use near_sdk::test_utils::VMContextBuilder;
use near_sdk::testing_env;
use near_sdk::{AccountId, NearToken, PromiseError, PromiseOrValue};
use quill_core::SLOT_NANOS;
use quill_core::UNIT;

const NOW: u64 = 1_000;
const EXPIRY: u64 = 1_100;

fn acct(name: &str) -> AccountId {
    name.parse().unwrap()
}
fn contract_id() -> AccountId {
    acct("market.testnet")
}
fn usdc() -> AccountId {
    acct("usdc.testnet")
}
fn alice() -> AccountId {
    acct("alice.testnet")
}
fn bob() -> AccountId {
    acct("bob.testnet")
}

/// Act as `predecessor` at hourly `slot`, attaching `deposit`.
fn at(predecessor: &AccountId, slot: u64, deposit: NearToken) {
    testing_env!(VMContextBuilder::new()
        .current_account_id(contract_id())
        .predecessor_account_id(predecessor.clone())
        .block_timestamp(slot * SLOT_NANOS)
        .attached_deposit(deposit)
        .build());
}

fn one_yocto() -> NearToken {
    NearToken::from_yoctonear(1)
}

/// A contract with one H100 market: band 1.50 to 5.00, expiring at `EXPIRY`.
fn setup() -> ClaimMarket {
    at(&alice(), NOW, MARKET_STORAGE_DEPOSIT);
    let mut c = ClaimMarket::new(usdc(), acct("oracle.testnet"));
    let id = c.create_market("H100".into(), U128(1_500_000), U128(5_000_000), EXPIRY);
    assert_eq!(id, 0);
    c
}

/// USDC arrives from `who` and mints into market 0. Returns the refund.
fn mint(c: &mut ClaimMarket, who: &AccountId, amount: u128) -> u128 {
    at(&usdc(), NOW, NearToken::from_yoctonear(0));
    match c.ft_on_transfer(who.clone(), U128(amount), r#"{"mint":0}"#.into()) {
        PromiseOrValue::Value(refund) => refund.0,
        PromiseOrValue::Promise(_) => panic!("mint should not return a promise"),
    }
}

fn settle_at(c: &mut ClaimMarket, answer: SettlementView) -> bool {
    at(&contract_id(), EXPIRY, NearToken::from_yoctonear(0));
    c.on_settlement(0, Ok(answer))
}

fn bal(c: &ClaimMarket, side: Side, who: &AccountId) -> u128 {
    c.balance_of(0, side, who.clone()).0
}

// ----- creating markets -----

#[test]
#[should_panic(expected = "cap must be greater than floor")]
fn rejects_an_empty_band() {
    let mut c = setup();
    at(&alice(), NOW, MARKET_STORAGE_DEPOSIT);
    c.create_market("H100".into(), U128(5), U128(5), EXPIRY);
}

#[test]
#[should_panic(expected = "Expiry must be in the future")]
fn rejects_an_expiry_in_the_past() {
    let mut c = setup();
    at(&alice(), NOW, MARKET_STORAGE_DEPOSIT);
    c.create_market("H100".into(), U128(1), U128(2), NOW);
}

#[test]
#[should_panic(expected = "Attach at least 0.01 NEAR")]
fn market_creation_needs_a_storage_deposit() {
    let mut c = setup();
    at(&alice(), NOW, NearToken::from_yoctonear(1));
    c.create_market("H100".into(), U128(1), U128(2), EXPIRY);
}

// ----- minting -----

#[test]
fn mint_credits_both_sides_and_locks_the_collateral() {
    let mut c = setup();
    assert_eq!(mint(&mut c, &alice(), 350 * UNIT), 0);
    assert_eq!(bal(&c, Side::Long, &alice()), 100 * UNIT);
    assert_eq!(bal(&c, Side::Short, &alice()), 100 * UNIT);
    let m = c.get_market(0).unwrap();
    assert_eq!(
        (m.long_supply.0, m.short_supply.0, m.locked.0),
        (100 * UNIT, 100 * UNIT, 350 * UNIT)
    );
}

#[test]
fn mint_refunds_what_it_cannot_use() {
    let mut c = setup();
    // 350.000001 USDC: the last micro-USDC is not enough for another base unit, so it is refunded.
    let refund = mint(&mut c, &alice(), 350 * UNIT + 1);
    assert_eq!(refund, 1);
    let m = c.get_market(0).unwrap();
    assert_eq!(
        m.locked.0 + refund,
        350 * UNIT + 1,
        "every micro-USDC is either locked or refunded"
    );
    assert_eq!(
        bal(&c, Side::Long, &alice()),
        bal(&c, Side::Short, &alice())
    );
}

#[test]
#[should_panic(expected = "Only the collateral token can mint")]
fn only_the_collateral_token_can_mint() {
    let mut c = setup();
    at(&alice(), NOW, NearToken::from_yoctonear(0));
    let _ = c.ft_on_transfer(alice(), U128(350 * UNIT), r#"{"mint":0}"#.into());
}

#[test]
#[should_panic(expected = "msg must be")]
fn mint_needs_a_valid_message() {
    let mut c = setup();
    at(&usdc(), NOW, NearToken::from_yoctonear(0));
    let _ = c.ft_on_transfer(alice(), U128(350 * UNIT), "hello".into());
}

#[test]
#[should_panic(expected = "No such market")]
fn mint_into_a_missing_market_fails() {
    let mut c = setup();
    at(&usdc(), NOW, NearToken::from_yoctonear(0));
    let _ = c.ft_on_transfer(alice(), U128(350 * UNIT), r#"{"mint":7}"#.into());
}

#[test]
#[should_panic(expected = "Deposit is below the minimum mint")]
fn dust_mints_are_rejected() {
    let mut c = setup();
    mint(&mut c, &alice(), 1);
}

#[test]
#[should_panic(expected = "Market is closed to minting")]
fn cannot_mint_at_or_after_expiry() {
    let mut c = setup();
    at(&usdc(), EXPIRY, NearToken::from_yoctonear(0));
    let _ = c.ft_on_transfer(alice(), U128(350 * UNIT), r#"{"mint":0}"#.into());
}

// ----- transfers -----

#[test]
fn transfer_moves_one_side_only() {
    let mut c = setup();
    mint(&mut c, &alice(), 350 * UNIT);
    at(&alice(), NOW, one_yocto());
    c.transfer(0, Side::Short, bob(), U128(40 * UNIT));
    assert_eq!(bal(&c, Side::Short, &alice()), 60 * UNIT);
    assert_eq!(bal(&c, Side::Short, &bob()), 40 * UNIT);
    assert_eq!(bal(&c, Side::Long, &alice()), 100 * UNIT);
    assert_eq!(bal(&c, Side::Long, &bob()), 0);
}

#[test]
#[should_panic(expected = "Insufficient balance")]
fn cannot_transfer_more_than_you_hold() {
    let mut c = setup();
    mint(&mut c, &alice(), 350 * UNIT);
    at(&alice(), NOW, one_yocto());
    c.transfer(0, Side::Long, bob(), U128(100 * UNIT + 1));
}

#[test]
#[should_panic(expected = "Requires attached deposit of exactly 1 yoctoNEAR")]
fn transfer_needs_one_yocto() {
    let mut c = setup();
    mint(&mut c, &alice(), 350 * UNIT);
    at(&alice(), NOW, NearToken::from_yoctonear(0));
    c.transfer(0, Side::Long, bob(), U128(1));
}

// ----- redeeming pairs -----

#[test]
fn redeem_pair_burns_both_sides_and_releases_collateral() {
    let mut c = setup();
    mint(&mut c, &alice(), 350 * UNIT);
    at(&alice(), NOW, NearToken::from_yoctonear(0));
    let _ = c.redeem_pair(0, U128(40 * UNIT));
    assert_eq!(bal(&c, Side::Long, &alice()), 60 * UNIT);
    assert_eq!(bal(&c, Side::Short, &alice()), 60 * UNIT);
    let m = c.get_market(0).unwrap();
    assert_eq!((m.long_supply.0, m.locked.0), (60 * UNIT, 210 * UNIT));
}

#[test]
#[should_panic(expected = "Insufficient balance")]
fn redeem_needs_both_sides() {
    let mut c = setup();
    mint(&mut c, &alice(), 350 * UNIT);
    at(&alice(), NOW, one_yocto());
    c.transfer(0, Side::Short, bob(), U128(100 * UNIT));
    at(&alice(), NOW, NearToken::from_yoctonear(0));
    let _ = c.redeem_pair(0, U128(UNIT));
}

#[test]
fn a_failed_usdc_transfer_restores_everything() {
    let mut c = setup();
    mint(&mut c, &alice(), 350 * UNIT);
    at(&alice(), NOW, NearToken::from_yoctonear(0));
    let _ = c.redeem_pair(0, U128(40 * UNIT));

    at(&contract_id(), NOW, NearToken::from_yoctonear(0));
    let ok = c.on_payout(
        alice(),
        0,
        U128(40 * UNIT),
        U128(40 * UNIT),
        U128(140 * UNIT),
        Err(PromiseError::Failed),
    );
    assert!(!ok);
    assert_eq!(bal(&c, Side::Long, &alice()), 100 * UNIT);
    assert_eq!(bal(&c, Side::Short, &alice()), 100 * UNIT);
    let m = c.get_market(0).unwrap();
    assert_eq!(
        (m.long_supply.0, m.short_supply.0, m.locked.0),
        (100 * UNIT, 100 * UNIT, 350 * UNIT)
    );
}

#[test]
fn a_successful_payout_changes_nothing_more() {
    let mut c = setup();
    mint(&mut c, &alice(), 350 * UNIT);
    at(&alice(), NOW, NearToken::from_yoctonear(0));
    let _ = c.redeem_pair(0, U128(40 * UNIT));
    at(&contract_id(), NOW, NearToken::from_yoctonear(0));
    assert!(c.on_payout(
        alice(),
        0,
        U128(40 * UNIT),
        U128(40 * UNIT),
        U128(140 * UNIT),
        Ok(())
    ));
    assert_eq!(c.get_market(0).unwrap().locked.0, 210 * UNIT);
}

// ----- settling -----

#[test]
#[should_panic(expected = "Market has not expired")]
fn cannot_settle_before_expiry() {
    let mut c = setup();
    at(&bob(), EXPIRY - 1, NearToken::from_yoctonear(0));
    let _ = c.settle(0);
}

#[test]
fn settlement_is_recorded_once_and_never_changes() {
    let mut c = setup();
    assert!(!settle_at(&mut c, SettlementView::Waiting));
    assert_eq!(c.get_market(0).unwrap().settled_at, None);

    assert!(settle_at(&mut c, SettlementView::Final(U128(3_060_000))));
    assert_eq!(c.get_market(0).unwrap().settled_at, Some(U128(3_060_000)));

    // A later, different answer is ignored.
    assert!(settle_at(&mut c, SettlementView::Final(U128(9_990_000))));
    assert!(settle_at(&mut c, SettlementView::Void));
    let m = c.get_market(0).unwrap();
    assert_eq!((m.settled_at, m.void), (Some(U128(3_060_000)), false));
}

#[test]
fn a_failed_oracle_call_leaves_the_market_open() {
    let mut c = setup();
    at(&contract_id(), EXPIRY, NearToken::from_yoctonear(0));
    assert!(!c.on_settlement(0, Err(PromiseError::Failed)));
    assert_eq!(c.get_market(0).unwrap().settled_at, None);
}

#[test]
#[should_panic(expected = "Market is settled: use claim")]
fn cannot_redeem_pairs_after_settlement() {
    let mut c = setup();
    mint(&mut c, &alice(), 350 * UNIT);
    settle_at(&mut c, SettlementView::Final(U128(3_060_000)));
    at(&alice(), EXPIRY, NearToken::from_yoctonear(0));
    let _ = c.redeem_pair(0, U128(UNIT));
}

// ----- claiming -----

#[test]
#[should_panic(expected = "Market is not settled yet")]
fn cannot_claim_before_settlement() {
    let mut c = setup();
    mint(&mut c, &alice(), 350 * UNIT);
    at(&alice(), EXPIRY, NearToken::from_yoctonear(0));
    let _ = c.claim(0);
}

#[test]
fn the_worked_example_pays_156_and_194() {
    let mut c = setup();
    mint(&mut c, &alice(), 350 * UNIT);
    // Alice keeps LONG and sells all her SHORT to Bob.
    at(&alice(), NOW, one_yocto());
    c.transfer(0, Side::Short, bob(), U128(100 * UNIT));
    settle_at(&mut c, SettlementView::Final(U128(3_060_000)));

    at(&alice(), EXPIRY, NearToken::from_yoctonear(0));
    let _ = c.claim(0);
    assert_eq!(
        c.get_market(0).unwrap().locked.0,
        194 * UNIT,
        "alice was paid 156"
    );

    at(&bob(), EXPIRY, NearToken::from_yoctonear(0));
    let _ = c.claim(0);
    let m = c.get_market(0).unwrap();
    assert_eq!(
        (m.long_supply.0, m.short_supply.0, m.locked.0),
        (0, 0, 0),
        "bob was paid 194; nothing is left"
    );
}

#[test]
fn a_worthless_side_is_burned_without_a_transfer() {
    let mut c = setup();
    mint(&mut c, &alice(), 350 * UNIT);
    at(&alice(), NOW, one_yocto());
    c.transfer(0, Side::Short, bob(), U128(100 * UNIT));
    // Index settles above the cap: SHORT is worth nothing.
    settle_at(&mut c, SettlementView::Fallback(U128(9_000_000)));

    at(&bob(), EXPIRY, NearToken::from_yoctonear(0));
    match c.claim(0) {
        PromiseOrValue::Value(paid) => assert_eq!(paid.0, 0),
        PromiseOrValue::Promise(_) => panic!("nothing should be sent"),
    }
    assert_eq!(bal(&c, Side::Short, &bob()), 0);
    assert_eq!(
        c.get_market(0).unwrap().locked.0,
        350 * UNIT,
        "all collateral is still there for LONG"
    );
}

#[test]
fn a_void_market_pays_each_side_half() {
    let mut c = setup();
    mint(&mut c, &alice(), 350 * UNIT);
    at(&alice(), NOW, one_yocto());
    c.transfer(0, Side::Short, bob(), U128(100 * UNIT));
    settle_at(&mut c, SettlementView::Void);
    assert!(c.get_market(0).unwrap().void);

    at(&alice(), EXPIRY, NearToken::from_yoctonear(0));
    let _ = c.claim(0);
    assert_eq!(c.get_market(0).unwrap().locked.0, 175 * UNIT);
    at(&bob(), EXPIRY, NearToken::from_yoctonear(0));
    let _ = c.claim(0);
    assert_eq!(c.get_market(0).unwrap().locked.0, 0);
}

#[test]
#[should_panic(expected = "Nothing to claim")]
fn cannot_claim_twice() {
    let mut c = setup();
    mint(&mut c, &alice(), 350 * UNIT);
    settle_at(&mut c, SettlementView::Final(U128(3_060_000)));
    at(&alice(), EXPIRY, NearToken::from_yoctonear(0));
    let _ = c.claim(0);
    let _ = c.claim(0);
}

#[test]
fn a_failed_claim_transfer_can_be_claimed_again() {
    let mut c = setup();
    mint(&mut c, &alice(), 350 * UNIT);
    settle_at(&mut c, SettlementView::Final(U128(3_060_000)));
    at(&alice(), EXPIRY, NearToken::from_yoctonear(0));
    let _ = c.claim(0);

    at(&contract_id(), EXPIRY, NearToken::from_yoctonear(0));
    c.on_payout(
        alice(),
        0,
        U128(100 * UNIT),
        U128(100 * UNIT),
        U128(350 * UNIT),
        Err(PromiseError::Failed),
    );
    assert_eq!(bal(&c, Side::Long, &alice()), 100 * UNIT);
    assert_eq!(c.get_market(0).unwrap().locked.0, 350 * UNIT);

    at(&alice(), EXPIRY, NearToken::from_yoctonear(0));
    let _ = c.claim(0);
    assert_eq!(c.get_market(0).unwrap().locked.0, 0);
}

// ----- the whole point -----

/// Many accounts mint, trade, redeem and claim in a random order. At every step
/// the collateral locked covers the worst case, and at the end nothing is owed.
#[test]
fn locked_collateral_always_covers_every_claim() {
    let mut seed = 0xC0FFEEu64;
    let mut next = |n: u64| {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((seed >> 33) % n) as u128
    };
    let people = [alice(), bob(), acct("carol.testnet"), acct("dave.testnet")];

    for round in 0..40 {
        let mut c = setup();
        for _ in 0..30 {
            let who = &people[next(4) as usize];
            match next(3) {
                0 => {
                    let _ = mint(&mut c, who, 1_000_000 + next(900_000_000));
                }
                1 => {
                    let side = if next(2) == 0 {
                        Side::Long
                    } else {
                        Side::Short
                    };
                    let have = bal(&c, side, who);
                    let to = &people[next(4) as usize];
                    if have > 0 && to != who {
                        at(who, NOW, one_yocto());
                        c.transfer(0, side, to.clone(), U128(1 + next(have as u64)));
                    }
                }
                _ => {
                    let pairs = bal(&c, Side::Long, who).min(bal(&c, Side::Short, who));
                    if pairs > UNIT {
                        at(who, NOW, NearToken::from_yoctonear(0));
                        let _ = c.redeem_pair(0, U128(UNIT + next((pairs - UNIT) as u64)));
                    }
                }
            }
            let m = c.get_market(0).unwrap();
            assert_eq!(
                m.long_supply, m.short_supply,
                "supplies stay equal while the market is open"
            );
            let worst_case = (m.long_supply.0 * (m.cap.0 - m.floor.0)).div_ceil(UNIT);
            assert!(
                m.locked.0 >= worst_case,
                "round {round}: locked {} < owed {worst_case}",
                m.locked.0
            );
        }

        let answer = match round % 4 {
            0 => SettlementView::Void,
            1 => SettlementView::Final(U128(0)),
            2 => SettlementView::Final(U128(99_000_000)),
            _ => SettlementView::Final(U128(1_500_000 + next(3_500_000))),
        };
        settle_at(&mut c, answer);
        for who in &people {
            if bal(&c, Side::Long, who) + bal(&c, Side::Short, who) > 0 {
                at(who, EXPIRY, NearToken::from_yoctonear(0));
                let _ = c.claim(0); // would panic on underflow if the contract owed more than it locked
            }
        }
        let m = c.get_market(0).unwrap();
        assert_eq!((m.long_supply.0, m.short_supply.0), (0, 0));
        // What is left is rounding dust only: under one micro-USDC per mint, redeem and claim.
        assert!(
            m.locked.0 < 200,
            "round {round}: {} micro-USDC left behind",
            m.locked.0
        );
    }
}
