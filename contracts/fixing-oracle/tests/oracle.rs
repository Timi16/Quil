//! Behaviour tests for `fixing-oracle`, through its public interface only.
//! OutLayer's answer is injected as the promise result the real callback reads.

use fixing_oracle::{CodeRef, FixingOracle, IndexConfig, SettlementView};
use near_sdk::json_types::U128;
use near_sdk::serde_json::{self, Value};
use near_sdk::test_utils::VMContextBuilder;
use near_sdk::{
    test_vm_config, testing_env, AccountId, NearToken, PromiseResult, RuntimeFeesConfig, VMContext,
};
use quill_core::settlement::GRACE_SLOTS;
use quill_core::SLOT_NANOS;

const NOW: u64 = 5_000;

fn acct(name: &str) -> AccountId {
    name.parse().unwrap()
}

fn ctx(predecessor: &str, slot: u64) -> VMContext {
    VMContextBuilder::new()
        .current_account_id(acct("oracle.testnet"))
        .predecessor_account_id(acct(predecessor))
        .block_timestamp(slot * SLOT_NANOS)
        .attached_deposit(NearToken::from_millinear(100))
        .build()
}

fn at(predecessor: &str, slot: u64) {
    testing_env!(ctx(predecessor, slot));
}

/// Run the real `on_fixing` callback with `result` as OutLayer's promise result.
fn deliver(o: &mut FixingOracle, slot: u64, result: Result<Vec<u8>, ()>) -> u32 {
    let promise = match result {
        Ok(bytes) => PromiseResult::Successful(bytes),
        Err(()) => PromiseResult::Failed,
    };
    testing_env!(
        ctx("oracle.testnet", slot),
        test_vm_config(),
        RuntimeFeesConfig::test(),
        Default::default(),
        vec![promise]
    );
    o.on_fixing(slot)
}

fn code() -> CodeRef {
    CodeRef {
        wasm_url: "https://example.com/quill-oracle.wasm".into(),
        wasm_hash: "a".repeat(64),
        build_target: "wasm32-wasip2".into(),
    }
}

fn setup() -> FixingOracle {
    at("keeper.testnet", NOW);
    FixingOracle::new(
        acct("outlayer.testnet"),
        code(),
        vec![
            IndexConfig {
                id: "H100".into(),
                min_sources: 3,
            },
            IndexConfig {
                id: "INF_OUT".into(),
                min_sources: 1,
            },
        ],
    )
}

fn output(slot: u64, h100: u64, sources: u32) -> Vec<u8> {
    format!(
        r#"{{"slot":{slot},"v":{{"H100":{h100}}},"n":{{"H100":{sources}}},"inputs":"sha256:abc"}}"#
    )
    .into_bytes()
}

/// Store an H100 fixing for `slot`.
fn fix(o: &mut FixingOracle, slot: u64, value: u64) -> u32 {
    deliver(o, slot, Ok(output(slot, value, 4)))
}

// ----- deployment -----

#[test]
#[should_panic(expected = "wasm_hash must be a 64-character hex SHA-256")]
fn rejects_a_bad_code_hash() {
    at("keeper.testnet", NOW);
    let mut bad = code();
    bad.wasm_hash = "xyz".into();
    FixingOracle::new(
        acct("outlayer.testnet"),
        bad,
        vec![IndexConfig {
            id: "H100".into(),
            min_sources: 3,
        }],
    );
}

#[test]
#[should_panic(expected = "Duplicate index id")]
fn rejects_duplicate_indexes() {
    at("keeper.testnet", NOW);
    let h = IndexConfig {
        id: "H100".into(),
        min_sources: 3,
    };
    FixingOracle::new(acct("outlayer.testnet"), code(), vec![h.clone(), h]);
}

#[test]
fn code_ref_is_what_was_deployed() {
    let o = setup();
    assert_eq!(o.code_ref(), code());
}

// ----- requesting -----

#[test]
#[should_panic(expected = "A request for this hour is already in flight")]
fn one_request_per_hour_in_flight() {
    let mut o = setup();
    let _ = o.request_fixing();
    let _ = o.request_fixing();
}

#[test]
#[should_panic(expected = "This hour already has a fixing")]
fn cannot_request_an_hour_that_is_fixed() {
    let mut o = setup();
    fix(&mut o, NOW, 3_060_000);
    let _ = o.request_fixing();
}

#[test]
fn a_failed_execution_frees_the_hour_for_a_retry() {
    let mut o = setup();
    let _ = o.request_fixing();
    assert_eq!(deliver(&mut o, NOW, Err(())), 0);
    let _ = o.request_fixing(); // does not panic
}

#[test]
fn the_next_hour_can_be_requested_while_an_old_one_is_stuck() {
    let mut o = setup();
    let _ = o.request_fixing();
    at("keeper.testnet", NOW + 1);
    let _ = o.request_fixing();
}

// ----- storing results -----

#[test]
fn stores_a_good_fixing() {
    let mut o = setup();
    assert_eq!(fix(&mut o, NOW, 3_060_000), 1);
    let f = o.latest("H100".into()).unwrap();
    assert_eq!(
        (f.slot, f.value.0, f.sources, f.accepted),
        (NOW, 3_060_000, 4, true)
    );
    assert_eq!(f.inputs, "sha256:abc");
    assert_eq!(o.latest("INF_OUT".into()), None);
}

#[test]
fn accepts_output_double_encoded_as_a_json_string() {
    let mut o = setup();
    let inner = String::from_utf8(output(NOW, 3_060_000, 4)).unwrap();
    let wrapped = serde_json::to_vec(&Value::String(inner)).unwrap();
    assert_eq!(deliver(&mut o, NOW, Ok(wrapped)), 1);
}

#[test]
fn garbage_output_is_ignored_without_panicking() {
    let mut o = setup();
    for bad in [
        &b"not json"[..],
        b"null",
        b"{}",
        b"[1,2,3]",
        b"{\"slot\":\"x\"}",
        b"",
    ] {
        assert_eq!(deliver(&mut o, NOW, Ok(bad.to_vec())), 0);
    }
    assert_eq!(o.latest("H100".into()), None);
}

#[test]
fn output_for_another_hour_is_rejected() {
    let mut o = setup();
    assert_eq!(deliver(&mut o, NOW, Ok(output(NOW - 1, 3_060_000, 4))), 0);
    assert_eq!(o.fixing_at("H100".into(), NOW), None);
    assert_eq!(o.fixing_at("H100".into(), NOW - 1), None);
}

#[test]
fn too_few_sources_or_a_zero_value_is_rejected() {
    let mut o = setup();
    assert_eq!(deliver(&mut o, NOW, Ok(output(NOW, 3_060_000, 2))), 0);
    assert_eq!(deliver(&mut o, NOW, Ok(output(NOW, 0, 4))), 0);
    assert_eq!(o.fixing_at("H100".into(), NOW), None);
}

#[test]
fn unknown_indexes_in_the_output_are_ignored() {
    let mut o = setup();
    let bytes =
        format!(r#"{{"slot":{NOW},"v":{{"DOGE":1,"H100":3060000}},"n":{{"DOGE":9,"H100":4}}}}"#)
            .into_bytes();
    assert_eq!(deliver(&mut o, NOW, Ok(bytes)), 1);
    assert_eq!(o.fixing_at("DOGE".into(), NOW), None);
}

#[test]
fn a_stored_fixing_is_never_overwritten() {
    let mut o = setup();
    fix(&mut o, NOW, 3_060_000);
    assert_eq!(fix(&mut o, NOW, 9_999_999), 0);
    assert_eq!(o.fixing_at("H100".into(), NOW).unwrap().value.0, 3_060_000);
}

#[test]
fn single_source_index_needs_only_one_source() {
    let mut o = setup();
    let bytes =
        format!(r#"{{"slot":{NOW},"v":{{"INF_OUT":600000}},"n":{{"INF_OUT":1}}}}"#).into_bytes();
    assert_eq!(deliver(&mut o, NOW, Ok(bytes)), 1);
    assert_eq!(o.latest("INF_OUT".into()).unwrap().value.0, 600_000);
}

// ----- plausibility -----

#[test]
fn a_sudden_jump_is_flagged_then_accepted_if_it_persists() {
    let mut o = setup();
    for i in 0..6 {
        fix(&mut o, NOW + i, 3_000_000);
    }
    // More than 50% above the recent median: stored, but flagged.
    fix(&mut o, NOW + 6, 6_000_000);
    assert!(!o.fixing_at("H100".into(), NOW + 6).unwrap().accepted);
    assert_eq!(
        o.latest("H100".into()).unwrap().slot,
        NOW + 5,
        "latest skips flagged values"
    );

    fix(&mut o, NOW + 7, 6_100_000);
    assert!(!o.fixing_at("H100".into(), NOW + 7).unwrap().accepted);
    // Third hour in a row: the price really moved.
    fix(&mut o, NOW + 8, 6_050_000);
    assert!(o.fixing_at("H100".into(), NOW + 8).unwrap().accepted);
    assert_eq!(o.latest("H100".into()).unwrap().value.0, 6_050_000);
}

#[test]
fn one_bad_reading_does_not_poison_the_history() {
    let mut o = setup();
    for i in 0..6 {
        fix(&mut o, NOW + i, 3_000_000);
    }
    fix(&mut o, NOW + 6, 30_000_000); // flagged
    fix(&mut o, NOW + 7, 3_050_000); // back to normal
    assert!(o.fixing_at("H100".into(), NOW + 7).unwrap().accepted);
    // The run was reset, so another spike starts from one.
    fix(&mut o, NOW + 8, 30_000_000);
    fix(&mut o, NOW + 9, 30_000_000);
    assert!(!o.fixing_at("H100".into(), NOW + 9).unwrap().accepted);
}

// ----- settlement -----

#[test]
fn settlement_is_the_mean_of_the_last_24_hours() {
    let mut o = setup();
    let expiry = NOW + 24;
    for i in 0..24u64 {
        fix(&mut o, NOW + i, 3_000_000 + i * 10_000);
    }
    at("anyone.testnet", expiry - 1);
    assert_eq!(
        o.settlement_value("H100".into(), expiry),
        SettlementView::Waiting
    );
    at("anyone.testnet", expiry);
    assert_eq!(
        o.settlement_value("H100".into(), expiry),
        SettlementView::Final(U128(3_115_000))
    );
}

#[test]
fn flagged_fixings_do_not_count_towards_settlement() {
    let mut o = setup();
    let expiry = NOW + 24;
    for i in 0..23u64 {
        fix(&mut o, NOW + i, 3_000_000);
    }
    fix(&mut o, NOW + 23, 90_000_000); // flagged spike in the last hour
    at("anyone.testnet", expiry);
    assert_eq!(
        o.settlement_value("H100".into(), expiry),
        SettlementView::Final(U128(3_000_000))
    );
}

#[test]
fn an_index_with_no_fixings_ends_void_after_the_grace_period() {
    let o = setup();
    let expiry = NOW + 24;
    at("anyone.testnet", expiry + GRACE_SLOTS - 1);
    assert_eq!(
        o.settlement_value("H100".into(), expiry),
        SettlementView::Waiting
    );
    at("anyone.testnet", expiry + GRACE_SLOTS);
    assert_eq!(
        o.settlement_value("H100".into(), expiry),
        SettlementView::Void
    );
    // An index this oracle never publishes behaves the same way.
    assert_eq!(
        o.settlement_value("NOPE".into(), expiry),
        SettlementView::Void
    );
}

#[test]
fn a_thin_window_falls_back_after_the_grace_period() {
    let mut o = setup();
    let expiry = NOW + 24;
    for i in 0..5u64 {
        fix(&mut o, NOW + i, 3_000_000);
    }
    at("anyone.testnet", expiry + GRACE_SLOTS);
    assert_eq!(
        o.settlement_value("H100".into(), expiry),
        SettlementView::Fallback(U128(3_000_000))
    );
}

#[test]
fn history_is_newest_first_and_bounded() {
    let mut o = setup();
    for i in 0..5u64 {
        fix(&mut o, NOW + i, 3_000_000 + i * 1_000);
    }
    at("anyone.testnet", NOW + 10);
    let all = o.fixings_between("H100".into(), NOW + 10, 48);
    assert_eq!(
        all.iter().map(|f| f.slot).collect::<Vec<_>>(),
        [NOW + 4, NOW + 3, NOW + 2, NOW + 1, NOW]
    );
    // The window counts hours back from `to_slot`, inclusive.
    assert_eq!(o.fixings_between("H100".into(), NOW + 4, 2).len(), 2);
    assert!(o.fixings_between("H100".into(), NOW + 4, 0).is_empty());
    assert!(o.fixings_between("NOPE".into(), NOW + 4, 48).is_empty());
    // A huge span is capped at one week, and slot 0 does not underflow.
    assert_eq!(o.fixings_between("H100".into(), NOW + 4, u64::MAX).len(), 5);
    assert!(o.fixings_between("H100".into(), 3, 48).is_empty());
}
