//! The keeper's decisions, tick by tick.

use quill_keeper::{
    plan, Action, MarketInfo, Memory, Observation, FIXING_RETRY_SECS, SETTLE_RETRY_SECS, SLOT_SECS,
};

const SLOT: u64 = 498_455;
const NOW: u64 = SLOT * SLOT_SECS + 120; // two minutes into the hour

fn seen(now: u64, hour_is_fixed: bool, markets: &[MarketInfo]) -> Observation {
    Observation {
        now,
        hour_is_fixed,
        markets: markets.to_vec(),
    }
}

fn market(id: u32, expiry_slot: u64, finished: bool) -> MarketInfo {
    MarketInfo {
        id,
        expiry_slot,
        finished,
    }
}

#[test]
fn requests_the_hour_once() {
    let (actions, memory) = plan(&seen(NOW, false, &[]), &Memory::default());
    assert_eq!(actions, [Action::RequestFixing { slot: SLOT }]);

    // A minute later the result has not landed yet: do not pay twice.
    let (actions, _) = plan(&seen(NOW + 60, false, &[]), &memory);
    assert!(actions.is_empty());
}

#[test]
fn retries_a_failed_hour_after_a_pause() {
    let (_, memory) = plan(&seen(NOW, false, &[]), &Memory::default());
    let later = NOW + FIXING_RETRY_SECS;
    assert_eq!(
        plan(&seen(later, false, &[]), &memory).0,
        [Action::RequestFixing { slot: SLOT }]
    );
}

#[test]
fn does_nothing_once_the_hour_is_fixed() {
    assert!(plan(&seen(NOW, true, &[]), &Memory::default()).0.is_empty());
}

#[test]
fn a_new_hour_is_requested_straight_away() {
    let (_, memory) = plan(&seen(NOW, false, &[]), &Memory::default());
    let next_hour = (SLOT + 1) * SLOT_SECS + 5;
    assert_eq!(
        plan(&seen(next_hour, false, &[]), &memory).0,
        [Action::RequestFixing { slot: SLOT + 1 }]
    );
}

#[test]
fn settles_only_expired_unfinished_markets() {
    let markets = [
        market(0, SLOT + 10, false), // still open
        market(1, SLOT, false),      // expires this hour: due
        market(2, SLOT - 30, true),  // already settled
        market(3, SLOT - 2, false),  // expired earlier, never settled: due
    ];
    let (actions, _) = plan(&seen(NOW, true, &markets), &Memory::default());
    assert_eq!(
        actions,
        [Action::Settle { market: 1 }, Action::Settle { market: 3 }]
    );
}

#[test]
fn a_market_that_was_not_ready_is_retried_every_ten_minutes() {
    let markets = [market(4, SLOT, false)];
    let (_, memory) = plan(&seen(NOW, true, &markets), &Memory::default());
    assert!(
        plan(&seen(NOW + SETTLE_RETRY_SECS - 1, true, &markets), &memory)
            .0
            .is_empty()
    );
    assert_eq!(
        plan(&seen(NOW + SETTLE_RETRY_SECS, true, &markets), &memory).0,
        [Action::Settle { market: 4 }]
    );
}

#[test]
fn finished_markets_are_forgotten() {
    let (_, memory) = plan(
        &seen(NOW, true, &[market(5, SLOT, false)]),
        &Memory::default(),
    );
    assert!(memory.settle_attempts.contains_key(&5));
    let (_, memory) = plan(&seen(NOW + 60, true, &[market(5, SLOT, true)]), &memory);
    assert!(memory.settle_attempts.is_empty());
}

#[test]
fn fixing_comes_before_settlement_in_the_same_tick() {
    let (actions, _) = plan(
        &seen(NOW, false, &[market(6, SLOT - 1, false)]),
        &Memory::default(),
    );
    assert_eq!(
        actions,
        [
            Action::RequestFixing { slot: SLOT },
            Action::Settle { market: 6 }
        ]
    );
}

#[test]
fn failures_are_explained_in_one_line() {
    use quill_keeper::explain;
    let panic = r#"Failure(ExecutionResult { ... ExecutionError("Smart contract panicked: Market has not expired") ... })"#;
    assert_eq!(explain(panic), "refused: Market has not expired");
    assert_eq!(
        explain("... AccountDoesNotExist { account_id: ... } ..."),
        "the contract account does not exist yet"
    );
    assert_eq!(
        explain("... CodeDoesNotExist { account_id: ... } ..."),
        "the account has no contract deployed"
    );
    assert_eq!(explain(&"x".repeat(500)).len(), 160);
}
