//! The stand-in answers in the shape the fixing-oracle reads.

use mock_outlayer::MockOutlayer;
use near_sdk::json_types::U64;
use near_sdk::serde_json::json;

#[test]
fn answers_for_the_requested_hour_from_the_script_in_order() {
    let mut m = MockOutlayer::default();
    m.set_script(vec![U64(3_000_000), U64(3_120_000)]);
    let input =
        |slot: u64| json!({ "slot": slot, "indexes": ["H100"], "previous": {} }).to_string();
    assert_eq!(m.request_execution(input(7))["v"]["H100"], 3_000_000);
    assert_eq!(m.request_execution(input(8))["v"]["H100"], 3_120_000);
    let third = m.request_execution(input(9));
    assert_eq!(third["v"]["H100"], 3_000_000, "the script repeats");
    assert_eq!(third["slot"], 9);
    assert_eq!(third["n"]["H100"], 4);
    assert_eq!(m.calls(), U64(3));
}

#[test]
#[should_panic(expected = "no script set")]
fn without_a_script_it_refuses() {
    MockOutlayer::default().request_execution(json!({ "slot": 1 }).to_string());
}

#[test]
#[should_panic(expected = "input_data is not JSON")]
fn garbage_input_is_refused() {
    let mut m = MockOutlayer::default();
    m.set_script(vec![U64(1)]);
    m.request_execution("nope".into());
}
