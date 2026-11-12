//! LONG and SHORT as NEP-245 multi-tokens.

use claim_market::mt::{parse_token_id, token_id, MAX_BATCH, MIN_NEW_HOLDING, MT_SPEC};
use claim_market::{ClaimMarket, Side, MARKET_STORAGE_DEPOSIT};
use near_sdk::json_types::U128;
use near_sdk::test_utils::{get_logs, VMContextBuilder};
use near_sdk::{
    test_vm_config, testing_env, AccountId, NearToken, PromiseOrValue, PromiseResult,
    RuntimeFeesConfig,
};
use quill_core::{SLOT_NANOS, UNIT};

const NOW: u64 = 1_000;

fn acct(name: &str) -> AccountId {
    name.parse().unwrap()
}
fn alice() -> AccountId {
    acct("alice.testnet")
}
fn bob() -> AccountId {
    acct("bob.testnet")
}
fn pool() -> AccountId {
    acct("pool.testnet")
}

fn at(predecessor: &AccountId, deposit: NearToken, results: Vec<PromiseResult>) {
    testing_env!(
        VMContextBuilder::new()
            .current_account_id(acct("market.testnet"))
            .predecessor_account_id(predecessor.clone())
            .block_timestamp(NOW * SLOT_NANOS)
            .attached_deposit(deposit)
            .build(),
        test_vm_config(),
        RuntimeFeesConfig::test(),
        Default::default(),
        results
    );
}

fn yocto() -> NearToken {
    NearToken::from_yoctonear(1)
}

/// Market 0 (band 1.50 to 5.00) with Alice holding 100 LONG and 100 SHORT.
fn setup() -> ClaimMarket {
    at(&alice(), MARKET_STORAGE_DEPOSIT, vec![]);
    let mut c = ClaimMarket::new(acct("usdc.testnet"), acct("oracle.testnet"));
    c.create_market("H100".into(), U128(1_500_000), U128(5_000_000), NOW + 100);
    at(&acct("usdc.testnet"), NearToken::from_yoctonear(0), vec![]);
    let _ = c.ft_on_transfer(alice(), U128(350 * UNIT), r#"{"mint":0}"#.into());
    c
}

fn balance(c: &ClaimMarket, who: &AccountId, id: &str) -> u128 {
    c.mt_balance_of(who.clone(), id.into()).0
}

#[test]
fn token_ids_have_one_spelling() {
    assert_eq!(parse_token_id("0:L"), Some((0, Side::Long)));
    assert_eq!(parse_token_id("12:S"), Some((12, Side::Short)));
    for bad in [
        "03:L",
        "0:X",
        "0",
        ":L",
        "x:L",
        "0:l",
        "-1:L",
        "0:L:",
        "4294967296:L",
        "",
    ] {
        assert_eq!(parse_token_id(bad), None, "{bad:?}");
    }
    assert_eq!(token_id(7, Side::Short), "7:S");
}

#[test]
fn minting_emits_nep245_and_shows_in_the_views() {
    let c = setup();
    assert!(get_logs()
        .iter()
        .any(|l| l.contains(r#""standard":"nep245""#) && l.contains(r#""event":"mt_mint""#)));
    assert_eq!(balance(&c, &alice(), "0:L"), 100 * UNIT);
    assert_eq!(
        c.mt_batch_balance_of(
            alice(),
            vec!["0:L".into(), "0:S".into(), "9:L".into(), "junk".into()]
        ),
        vec![U128(100 * UNIT), U128(100 * UNIT), U128(0), U128(0)]
    );
    assert_eq!(c.mt_supply("0:S".into()), Some(U128(100 * UNIT)));
    assert_eq!(
        c.mt_batch_supply(vec!["0:L".into(), "1:L".into()]),
        vec![Some(U128(100 * UNIT)), None]
    );
    assert_eq!(
        c.mt_token(vec!["0:L".into(), "5:S".into()])
            .iter()
            .map(|t| t.is_some())
            .collect::<Vec<_>>(),
        [true, false]
    );
    assert_eq!(c.mt_metadata_contract().spec, MT_SPEC);
}

#[test]
fn mt_transfer_moves_one_token() {
    let mut c = setup();
    at(&alice(), yocto(), vec![]);
    c.mt_transfer(
        bob(),
        "0:S".into(),
        U128(40 * UNIT),
        None,
        Some("sold".into()),
    );
    assert_eq!(
        (balance(&c, &alice(), "0:S"), balance(&c, &bob(), "0:S")),
        (60 * UNIT, 40 * UNIT)
    );
    assert_eq!(balance(&c, &bob(), "0:L"), 0);
    let log = get_logs()
        .into_iter()
        .find(|l| l.contains("mt_transfer"))
        .unwrap();
    assert!(log.contains(r#""memo":"sold""#) && log.contains(r#""new_owner_id":"bob.testnet""#));
    // Supply does not change on a transfer.
    assert_eq!(c.mt_supply("0:S".into()), Some(U128(100 * UNIT)));
}

#[test]
fn batch_transfer_moves_both_sides_at_once() {
    let mut c = setup();
    at(&alice(), yocto(), vec![]);
    c.mt_batch_transfer(
        bob(),
        vec!["0:L".into(), "0:S".into()],
        vec![U128(UNIT), U128(2 * UNIT)],
        None,
        None,
    );
    assert_eq!(
        (balance(&c, &bob(), "0:L"), balance(&c, &bob(), "0:S")),
        (UNIT, 2 * UNIT)
    );
}

#[test]
#[should_panic(expected = "Approvals are not supported")]
fn approvals_are_refused() {
    let mut c = setup();
    at(&alice(), yocto(), vec![]);
    c.mt_transfer(bob(), "0:L".into(), U128(UNIT), Some((pool(), 1)), None);
}

#[test]
#[should_panic(expected = "Requires attached deposit of exactly 1 yoctoNEAR")]
fn transfers_need_one_yocto() {
    let mut c = setup();
    at(&alice(), NearToken::from_yoctonear(0), vec![]);
    c.mt_transfer(bob(), "0:L".into(), U128(UNIT), None, None);
}

#[test]
#[should_panic(expected = "Unknown token id")]
fn unknown_token_ids_are_refused() {
    let mut c = setup();
    at(&alice(), yocto(), vec![]);
    c.mt_transfer(bob(), "0:LONG".into(), U128(UNIT), None, None);
}

#[test]
#[should_panic(expected = "No such market")]
fn tokens_of_a_missing_market_are_refused() {
    let mut c = setup();
    at(&alice(), yocto(), vec![]);
    c.mt_transfer(bob(), "3:L".into(), U128(UNIT), None, None);
}

#[test]
#[should_panic(expected = "same length")]
fn mismatched_batches_are_refused() {
    let mut c = setup();
    at(&alice(), yocto(), vec![]);
    c.mt_batch_transfer(
        bob(),
        vec!["0:L".into(), "0:S".into()],
        vec![U128(UNIT)],
        None,
        None,
    );
}

#[test]
#[should_panic(expected = "At most 16 tokens per call")]
fn oversized_batches_are_refused() {
    let mut c = setup();
    at(&alice(), yocto(), vec![]);
    let ids = vec!["0:L".to_string(); MAX_BATCH + 1];
    c.mt_batch_transfer(bob(), ids, vec![U128(UNIT); MAX_BATCH + 1], None, None);
}

#[test]
#[should_panic(expected = "A transfer to a new holder must be at least 0.01 unit")]
fn dust_cannot_open_a_new_holding() {
    let mut c = setup();
    at(&alice(), yocto(), vec![]);
    c.mt_transfer(bob(), "0:L".into(), U128(MIN_NEW_HOLDING - 1), None, None);
}

#[test]
fn dust_can_top_up_an_existing_holding() {
    let mut c = setup();
    at(&alice(), yocto(), vec![]);
    c.mt_transfer(bob(), "0:L".into(), U128(MIN_NEW_HOLDING), None, None);
    c.mt_transfer(bob(), "0:L".into(), U128(1), None, None);
    assert_eq!(balance(&c, &bob(), "0:L"), MIN_NEW_HOLDING + 1);
}

// ----- transfer_call and resolve -----

/// Alice sends 40 SHORT to the pool with a call; the pool answers `answer`.
fn send_to_pool_and_resolve(answer: PromiseResult, pool_spends: u128) -> (ClaimMarket, Vec<U128>) {
    let mut c = setup();
    at(&alice(), yocto(), vec![]);
    match c.mt_transfer_call(
        pool(),
        "0:S".into(),
        U128(40 * UNIT),
        None,
        None,
        "deposit".into(),
    ) {
        PromiseOrValue::Promise(_) => {}
        PromiseOrValue::Value(_) => panic!("transfer_call must call the receiver"),
    }
    assert_eq!(
        balance(&c, &pool(), "0:S"),
        40 * UNIT,
        "tokens arrive before the receiver is called"
    );
    if pool_spends > 0 {
        // While its callback runs, the pool moves some of what it received on.
        at(&pool(), yocto(), vec![]);
        c.mt_transfer(bob(), "0:S".into(), U128(pool_spends), None, None);
    }
    at(
        &acct("market.testnet"),
        NearToken::from_yoctonear(0),
        vec![answer],
    );
    let kept = c.mt_resolve_transfer(
        vec![alice()],
        pool(),
        vec!["0:S".into()],
        vec![U128(40 * UNIT)],
        None,
    );
    (c, kept)
}

fn answer(unused: &[u128]) -> PromiseResult {
    let list: Vec<U128> = unused.iter().map(|u| U128(*u)).collect();
    PromiseResult::Successful(near_sdk::serde_json::to_vec(&list).unwrap())
}

#[test]
fn receiver_keeps_everything() {
    let (c, kept) = send_to_pool_and_resolve(answer(&[0]), 0);
    assert_eq!(kept, vec![U128(40 * UNIT)]);
    assert_eq!(
        (balance(&c, &alice(), "0:S"), balance(&c, &pool(), "0:S")),
        (60 * UNIT, 40 * UNIT)
    );
}

#[test]
fn receiver_returns_part() {
    let (c, kept) = send_to_pool_and_resolve(answer(&[15 * UNIT]), 0);
    assert_eq!(kept, vec![U128(25 * UNIT)]);
    assert_eq!(
        (balance(&c, &alice(), "0:S"), balance(&c, &pool(), "0:S")),
        (75 * UNIT, 25 * UNIT)
    );
}

#[test]
fn a_failed_receiver_refunds_everything() {
    let (c, kept) = send_to_pool_and_resolve(PromiseResult::Failed, 0);
    assert_eq!(kept, vec![U128(0)]);
    assert_eq!(balance(&c, &alice(), "0:S"), 100 * UNIT);
}

#[test]
fn a_nonsense_answer_counts_as_a_full_refund() {
    // The mocked chain keeps storage across these runs, so check the receiver, which must end empty.
    for nonsense in [
        b"\"oops\"".to_vec(),
        b"[]".to_vec(),
        b"[\"1\",\"2\"]".to_vec(),
        b"not json".to_vec(),
    ] {
        let (c, kept) = send_to_pool_and_resolve(PromiseResult::Successful(nonsense), 0);
        assert_eq!((kept, balance(&c, &pool(), "0:S")), (vec![U128(0)], 0));
    }
}

#[test]
fn a_receiver_cannot_claim_back_more_than_it_was_sent() {
    let (c, kept) = send_to_pool_and_resolve(answer(&[999 * UNIT]), 0);
    assert_eq!(kept, vec![U128(0)]);
    assert_eq!(balance(&c, &alice(), "0:S"), 100 * UNIT);
    assert_eq!(
        c.mt_supply("0:S".into()),
        Some(U128(100 * UNIT)),
        "no tokens are created by a refund"
    );
}

#[test]
fn a_refund_is_limited_to_what_the_receiver_still_holds() {
    // The pool says "all unused" but already passed 30 of the 40 to Bob.
    let (c, kept) = send_to_pool_and_resolve(answer(&[40 * UNIT]), 30 * UNIT);
    assert_eq!(kept, vec![U128(30 * UNIT)]);
    assert_eq!(balance(&c, &alice(), "0:S"), 70 * UNIT);
    assert_eq!(
        (balance(&c, &pool(), "0:S"), balance(&c, &bob(), "0:S")),
        (0, 30 * UNIT)
    );
}

#[test]
fn redeeming_and_claiming_emit_mt_burn() {
    let mut c = setup();
    at(&alice(), NearToken::from_yoctonear(0), vec![]);
    let _ = c.redeem_pair(0, U128(10 * UNIT));
    let log = get_logs()
        .into_iter()
        .find(|l| l.contains("mt_burn"))
        .expect("mt_burn");
    assert!(log.contains(r#""token_ids":["0:L","0:S"]"#), "{log}");
}
