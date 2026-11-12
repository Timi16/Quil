//! `make demo`: Quill's whole story in one run, with every claim checked.
//!
//! Exits non-zero if any number is off by a single micro-USDC.

use claim_market::Side;
use quill_core::UNIT;
use quill_demo::{acct, refused, Reading, SettlementView, World};

const START: u64 = 490_000;
const FLOOR: u128 = 1_500_000;
const CAP: u128 = 5_000_000;

/// Micro-dollars as dollars and cents, rounded to the nearest cent.
fn usd(micro: u128) -> String {
    let cents = (micro + 5_000) / 10_000;
    format!("{}.{:02}", cents / 100, cents % 100)
}

fn step(n: u32, title: &str) {
    println!("\n{n}. {title}");
}

fn check(label: &str, got: u128, want: u128) {
    assert_eq!(got, want, "{label}: got {got}, want {want}");
    println!("   ok  {label}: {}", usd(got));
}

fn main() {
    let alice = acct("alice.testnet"); // rents GPUs, fears a price rise: wants LONG
    let bob = acct("bob.testnet"); // rents GPUs out, fears a fall: wants SHORT
    let mut w = World::new(START);
    let expiry = START + 30;

    println!("Quill demo: both contracts and a mock USDC ledger, in one process.");
    if std::env::args().any(|arg| arg == "--live") {
        live_fixing();
    }
    println!("\nSteps 1 to 11 use scripted prices so the run is identical every time.");

    step(
        1,
        "Collateral arrives (in production: from any chain through NEAR Intents)",
    );
    w.fund(&alice, 350 * UNIT);
    w.fund(&bob, 194 * UNIT);
    let minted_usdc = w.total_usdc();
    check("alice holds USDC", w.usdc_of(&alice), 350 * UNIT);

    step(2, "Anyone creates the market: H100, band 1.50 to 5.00");
    let market = w.create_market(&bob, "H100", FLOOR, CAP, expiry);
    println!("   ok  market {market} created, expires at hourly slot {expiry}");

    step(3, "Alice mints 100 pairs by sending 350 USDC");
    let units = w.mint(&alice, market, 350 * UNIT);
    check("LONG minted", units, 100 * UNIT);
    check(
        "collateral locked",
        w.market_view(market).locked.0,
        350 * UNIT,
    );
    check("alice USDC left", w.usdc_of(&alice), 0);

    step(
        4,
        "Alice keeps LONG and sells her 100 SHORT to Bob for 194 USDC",
    );
    w.transfer(&alice, &bob, market, Side::Short, 100 * UNIT);
    w.pay(&bob, &alice, 194 * UNIT);
    check(
        "alice LONG",
        w.balance(market, Side::Long, &alice),
        100 * UNIT,
    );
    check(
        "bob SHORT",
        w.balance(market, Side::Short, &bob),
        100 * UNIT,
    );

    step(5, "Things that must be refused");
    let eve = acct("eve.testnet");
    println!(
        "   ok  mint from a fake token: {}",
        refused(|| {
            // Eve calls ft_on_transfer herself instead of going through USDC.
            let mut fake = World::new(START);
            let m = fake.create_market(&eve, "H100", FLOOR, CAP, expiry);
            fake.fund(&eve, UNIT);
            fake.market.ft_on_transfer(
                eve.clone(),
                near_sdk::json_types::U128(350 * UNIT),
                format!("{{\"mint\":{m}}}"),
            )
        })
    );
    println!(
        "   ok  sending tokens you do not hold: {}",
        refused(|| w.transfer(&eve, &bob, market, Side::Long, UNIT))
    );
    println!(
        "   ok  settling before expiry: {}",
        refused(|| w.settle(market))
    );
    println!(
        "   ok  claiming before settlement: {}",
        refused(|| w.claim(&alice, market))
    );

    step(6, "The oracle publishes hourly fixings from the enclave");
    w.advance(6); // the final 24 hours before expiry start here
    for hour in 0..24u64 {
        let value = 3_002_500 + 5_000 * hour; // drifts up; the 24-hour mean is exactly 3.06
        let stored = w.fixing(&[Reading {
            index: "H100",
            value,
            sources: 4,
        }]);
        assert_eq!(stored, 1);

        if hour == 10 {
            // A second answer for the same hour cannot overwrite the first.
            let msg = refused(|| {
                w.fixing(&[Reading {
                    index: "H100",
                    value: 9_999_999,
                    sources: 4,
                }])
            });
            println!("   ok  second fixing in one hour: {msg}");
        }
        w.advance(1);
    }
    println!(
        "   ok  24 fixings stored, latest {}",
        usd(w.oracle.latest("H100".into()).unwrap().value.0)
    );

    step(7, "Bad oracle answers are ignored, not trusted");
    let mut probe = World::new(START);
    assert_eq!(
        probe.fixing(&[Reading {
            index: "H100",
            value: 3_000_000,
            sources: 2
        }]),
        0
    );
    println!("   ok  only 2 sources: rejected");
    assert_eq!(
        probe.fixing_raw(Ok(b"<html>502 Bad Gateway</html>".to_vec())),
        0
    );
    println!("   ok  garbage output: rejected, hour can be retried");
    assert_eq!(probe.fixing_raw(Err(())), 0);
    println!("   ok  failed execution: rejected, hour can be retried");
    assert_eq!(
        probe.fixing(&[Reading {
            index: "H100",
            value: 3_000_000,
            sources: 4
        }]),
        1
    );
    println!("   ok  a good answer for the same hour is then accepted");

    step(8, "Expiry: anyone settles the market on the 24-hour mean");
    assert_eq!(w.slot, expiry);
    let answer = w.settle(market);
    assert_eq!(
        answer,
        SettlementView::Final(near_sdk::json_types::U128(3_060_000))
    );
    check(
        "settlement index",
        w.market_view(market).settled_at.unwrap().0,
        3_060_000,
    );

    step(
        9,
        "The USDC contract has an outage: a failed payout is rolled back",
    );
    w.usdc_down = true;
    check("alice receives", w.claim(&alice, market), 0);
    check(
        "alice still holds LONG",
        w.balance(market, Side::Long, &alice),
        100 * UNIT,
    );
    check(
        "collateral still locked",
        w.market_view(market).locked.0,
        350 * UNIT,
    );
    w.usdc_down = false;

    step(10, "Both sides claim");
    check("alice (LONG) receives", w.claim(&alice, market), 156 * UNIT);
    check("bob (SHORT) receives", w.claim(&bob, market), 194 * UNIT);
    println!(
        "   ok  claiming twice: {}",
        refused(|| w.claim(&alice, market))
    );

    step(11, "The books balance");
    let m = w.market_view(market);
    check("collateral left in the market", m.locked.0, 0);
    check("LONG supply left", m.long_supply.0, 0);
    check(
        "USDC held by the contract",
        w.usdc_of(&World::market_id()),
        0,
    );
    check("alice ends with", w.usdc_of(&alice), 350 * UNIT); // 194 from the sale + 156 claimed
    check("bob ends with", w.usdc_of(&bob), 194 * UNIT); // paid 194, claimed 194
    check("USDC in existence", w.total_usdc(), minted_usdc);

    println!("\nAll checks passed. LONG 156.00 + SHORT 194.00 = 350.00 locked. Nothing was created or lost.");
}

/// Fetch real prices right now and hand the oracle program's exact output to the real contract callback.
fn live_fixing() {
    println!(
        "\n0. LIVE: prices fetched this minute, through the oracle program, into the contract"
    );
    let mut w = World::new(START);
    let input = quill_oracle::Input {
        slot: w.slot,
        indexes: vec!["H100".into(), "B200".into(), "INF_OUT".into()],
        previous: Default::default(),
    };
    let result = quill_oracle::run(&input, &quill_oracle::fetch::Http);
    for line in &result.evidence {
        println!("   src {line}");
    }
    for problem in &result.problems {
        println!("   !!  {problem}");
    }
    let bytes = near_sdk::serde_json::to_vec(&result.output).expect("serializable");
    println!("   out {}", String::from_utf8_lossy(&bytes));
    let stored = w.fixing_raw(Ok(bytes));
    println!("   ok  the contract validated and stored {stored} index value(s)");
    for index in ["H100", "B200", "INF_OUT"] {
        if let Some(fixing) = w.oracle.latest(index.into()) {
            println!(
                "   ok  {index}: ${} from {} source(s)",
                usd(fixing.value.0),
                fixing.sources
            );
        }
    }
}
