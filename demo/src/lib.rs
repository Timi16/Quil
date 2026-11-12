//! A small world for running Quill end to end without a chain.
//!
//! [`World`] holds the two real contracts, a mock USDC ledger and a clock, and
//! plays the part of the NEAR runtime between them: it delivers
//! `ft_on_transfer`, performs (or fails) the USDC transfers the market asks
//! for, hands OutLayer's answer to the oracle's callback and carries the
//! oracle's settlement value back to the market.
//!
//! It is a simulation, not a sandbox: gas, storage fees and real asynchronous
//! receipts are not modelled. What it does prove is the accounting, on the
//! same contract code that is compiled to WASM.

use std::collections::BTreeMap;
use std::panic::{catch_unwind, AssertUnwindSafe};

use claim_market::{ClaimMarket, MarketView, Side, MARKET_STORAGE_DEPOSIT};
use fixing_oracle::{CodeRef, FixingOracle, IndexConfig};
use near_sdk::json_types::U128;
use near_sdk::serde_json;
use near_sdk::test_utils::VMContextBuilder;
use near_sdk::{
    test_vm_config, testing_env, AccountId, NearToken, PromiseError, PromiseOrValue, PromiseResult,
    RuntimeFeesConfig,
};
use quill_core::SLOT_NANOS;

pub use claim_market::SettlementView;

/// Parse an account id, panicking on a typo in test code.
pub fn acct(name: &str) -> AccountId {
    name.parse().expect("valid account id")
}

/// One index value as the enclave program would report it.
#[derive(Debug, Clone, Copy)]
pub struct Reading<'a> {
    pub index: &'a str,
    /// Micro-USD.
    pub value: u64,
    pub sources: u32,
}

pub struct World {
    pub market: ClaimMarket,
    pub oracle: FixingOracle,
    /// Mock USDC balances, in micro-USDC.
    usdc: BTreeMap<AccountId, u128>,
    /// The current hourly slot.
    pub slot: u64,
    /// While true, USDC transfers out of the market fail (to exercise rollback).
    pub usdc_down: bool,
}

impl World {
    pub fn market_id() -> AccountId {
        acct("claims.quill.testnet")
    }
    pub fn oracle_id() -> AccountId {
        acct("fixing.quill.testnet")
    }
    pub fn usdc_id() -> AccountId {
        acct("usdc.fakes.testnet")
    }
    pub fn keeper() -> AccountId {
        acct("keeper.testnet")
    }

    /// Deploy both contracts at hourly slot `start_slot`.
    pub fn new(start_slot: u64) -> Self {
        let mut world = Self {
            // Placeholders so `enter` can run before the real constructors.
            market: {
                enter(
                    &Self::market_id(),
                    &Self::market_id(),
                    start_slot,
                    NearToken::from_yoctonear(0),
                    vec![],
                );
                ClaimMarket::new(Self::usdc_id(), Self::oracle_id())
            },
            oracle: {
                enter(
                    &Self::oracle_id(),
                    &Self::oracle_id(),
                    start_slot,
                    NearToken::from_yoctonear(0),
                    vec![],
                );
                FixingOracle::new(
                    acct("outlayer.testnet"),
                    CodeRef {
                        wasm_url: "https://example.com/quill-oracle.wasm".into(),
                        wasm_hash: "0".repeat(64),
                        build_target: "wasm32-wasip2".into(),
                    },
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
            },
            usdc: BTreeMap::new(),
            slot: start_slot,
            usdc_down: false,
        };
        world.usdc.insert(Self::market_id(), 0);
        world
    }

    // ----- clock and ledger -----

    pub fn advance(&mut self, slots: u64) {
        self.slot += slots;
    }

    /// Give `who` mock USDC (stands in for collateral arriving through NEAR Intents).
    pub fn fund(&mut self, who: &AccountId, amount: u128) {
        *self.usdc.entry(who.clone()).or_default() += amount;
    }

    pub fn usdc_of(&self, who: &AccountId) -> u128 {
        self.usdc.get(who).copied().unwrap_or(0)
    }

    /// Move mock USDC directly between accounts (an off-contract trade payment).
    pub fn pay(&mut self, from: &AccountId, to: &AccountId, amount: u128) {
        let have = self.usdc_of(from);
        assert!(have >= amount, "{from} has {have}, cannot pay {amount}");
        self.usdc.insert(from.clone(), have - amount);
        self.fund(to, amount);
    }

    /// Every micro-USDC in existence. Must never change except through `fund`.
    pub fn total_usdc(&self) -> u128 {
        self.usdc.values().sum()
    }

    // ----- market actions -----

    pub fn create_market(
        &mut self,
        who: &AccountId,
        index: &str,
        floor: u128,
        cap: u128,
        expiry_slot: u64,
    ) -> u32 {
        enter(
            &Self::market_id(),
            who,
            self.slot,
            MARKET_STORAGE_DEPOSIT,
            vec![],
        );
        self.market
            .create_market(index.into(), U128(floor), U128(cap), expiry_slot)
    }

    /// `ft_transfer_call` of `amount` USDC with `{"mint": market}`. Returns units minted per side.
    ///
    /// Panics with the contract's message if the mint is refused; no USDC moves in that case,
    /// exactly as the token contract would refund it.
    pub fn mint(&mut self, who: &AccountId, market: u32, amount: u128) -> u128 {
        assert!(self.usdc_of(who) >= amount, "{who} cannot afford {amount}");
        let before = self.balance(market, Side::Long, who);
        enter(
            &Self::market_id(),
            &Self::usdc_id(),
            self.slot,
            NearToken::from_yoctonear(0),
            vec![],
        );
        let msg = format!("{{\"mint\":{market}}}");
        let refund = match self.market.ft_on_transfer(who.clone(), U128(amount), msg) {
            PromiseOrValue::Value(refund) => refund.0,
            PromiseOrValue::Promise(_) => unreachable!("mint never returns a promise"),
        };
        self.pay(who, &Self::market_id(), amount - refund);
        self.balance(market, Side::Long, who) - before
    }

    pub fn transfer(
        &mut self,
        from: &AccountId,
        to: &AccountId,
        market: u32,
        side: Side,
        units: u128,
    ) {
        enter(
            &Self::market_id(),
            from,
            self.slot,
            NearToken::from_yoctonear(1),
            vec![],
        );
        self.market.transfer(market, side, to.clone(), U128(units));
    }

    /// Burn `units` of LONG and SHORT for collateral. Returns the USDC received (0 if the transfer failed).
    pub fn redeem_pair(&mut self, who: &AccountId, market: u32, units: u128) -> u128 {
        let before = self.snapshot(market, who);
        enter(
            &Self::market_id(),
            who,
            self.slot,
            NearToken::from_yoctonear(0),
            vec![],
        );
        let _ = self.market.redeem_pair(market, U128(units));
        self.finish_payout(who, market, before)
    }

    /// Claim a settled position. Returns the USDC received (0 if worthless or the transfer failed).
    pub fn claim(&mut self, who: &AccountId, market: u32) -> u128 {
        let before = self.snapshot(market, who);
        enter(
            &Self::market_id(),
            who,
            self.slot,
            NearToken::from_yoctonear(0),
            vec![],
        );
        let _ = self.market.claim(market);
        self.finish_payout(who, market, before)
    }

    /// Ask the oracle and record its answer, as `settle` plus its callback would.
    pub fn settle(&mut self, market: u32) -> SettlementView {
        let view = self.market_view(market);
        enter(
            &Self::market_id(),
            &Self::keeper(),
            self.slot,
            NearToken::from_yoctonear(0),
            vec![],
        );
        let _ = self.market.settle(market);

        enter(
            &Self::oracle_id(),
            &Self::market_id(),
            self.slot,
            NearToken::from_yoctonear(0),
            vec![],
        );
        let answer = self.oracle.settlement_value(view.index, view.expiry_slot);
        // The two contracts define the same JSON shape; cross the boundary as JSON, like the chain does.
        let answer: SettlementView =
            serde_json::from_value(serde_json::to_value(answer).expect("serializable"))
                .expect("same JSON shape");

        enter(
            &Self::market_id(),
            &Self::market_id(),
            self.slot,
            NearToken::from_yoctonear(0),
            vec![],
        );
        self.market.on_settlement(market, Ok(answer.clone()));
        answer
    }

    // ----- oracle actions -----

    /// A keeper requests this hour's fixing and OutLayer answers with `readings`.
    /// Returns how many index values the oracle stored.
    pub fn fixing(&mut self, readings: &[Reading]) -> u32 {
        let values: Vec<String> = readings
            .iter()
            .map(|r| format!("\"{}\":{}", r.index, r.value))
            .collect();
        let counts: Vec<String> = readings
            .iter()
            .map(|r| format!("\"{}\":{}", r.index, r.sources))
            .collect();
        let output = format!(
            "{{\"slot\":{},\"v\":{{{}}},\"n\":{{{}}},\"inputs\":\"sha256:demo-{}\"}}",
            self.slot,
            values.join(","),
            counts.join(","),
            self.slot
        );
        self.fixing_raw(Ok(output.into_bytes()))
    }

    /// As [`World::fixing`], with OutLayer's raw answer (or failure).
    pub fn fixing_raw(&mut self, answer: Result<Vec<u8>, ()>) -> u32 {
        enter(
            &Self::oracle_id(),
            &Self::keeper(),
            self.slot,
            NearToken::from_millinear(100),
            vec![],
        );
        let _ = self.oracle.request_fixing();

        let result = match answer {
            Ok(bytes) => PromiseResult::Successful(bytes),
            Err(()) => PromiseResult::Failed,
        };
        enter(
            &Self::oracle_id(),
            &Self::oracle_id(),
            self.slot,
            NearToken::from_yoctonear(0),
            vec![result],
        );
        self.oracle.on_fixing(self.slot)
    }

    // ----- reads -----

    pub fn balance(&self, market: u32, side: Side, who: &AccountId) -> u128 {
        self.market.balance_of(market, side, who.clone()).0
    }

    pub fn market_view(&self, market: u32) -> MarketView {
        self.market.get_market(market).expect("market exists")
    }

    // ----- internals -----

    fn snapshot(&self, market: u32, who: &AccountId) -> (u128, u128, u128) {
        (
            self.market_view(market).locked.0,
            self.balance(market, Side::Long, who),
            self.balance(market, Side::Short, who),
        )
    }

    /// Play the USDC contract and the `on_payout` callback for whatever the market just released.
    fn finish_payout(&mut self, who: &AccountId, market: u32, before: (u128, u128, u128)) -> u128 {
        let (locked, long, short) = before;
        let (locked_now, long_now, short_now) = self.snapshot(market, who);
        let amount = locked - locked_now;
        if amount == 0 {
            return 0; // nothing was sent, so there is no callback
        }
        let sent = if self.usdc_down {
            Err(PromiseError::Failed)
        } else {
            self.pay(&Self::market_id(), who, amount);
            Ok(())
        };
        let delivered = sent.is_ok();
        enter(
            &Self::market_id(),
            &Self::market_id(),
            self.slot,
            NearToken::from_yoctonear(0),
            vec![],
        );
        self.market.on_payout(
            who.clone(),
            market,
            U128(long - long_now),
            U128(short - short_now),
            U128(amount),
            sent,
        );
        if delivered {
            amount
        } else {
            0
        }
    }
}

/// Run `action`, expecting the contract to refuse it. Returns the refusal message.
pub fn refused<T>(action: impl FnOnce() -> T) -> String {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {})); // keep expected refusals out of the output
    let result = catch_unwind(AssertUnwindSafe(action));
    std::panic::set_hook(hook);
    match result {
        Ok(_) => panic!("expected the contract to refuse, but it accepted"),
        Err(payload) => {
            let raw = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "refused".into());
            contract_message(&raw)
        }
    }
}

/// Set the mocked NEAR environment for the next contract call.
fn enter(
    current: &AccountId,
    predecessor: &AccountId,
    slot: u64,
    deposit: NearToken,
    results: Vec<PromiseResult>,
) {
    testing_env!(
        VMContextBuilder::new()
            .current_account_id(current.clone())
            .predecessor_account_id(predecessor.clone())
            .block_timestamp(slot * SLOT_NANOS)
            .attached_deposit(deposit)
            .build(),
        test_vm_config(),
        RuntimeFeesConfig::test(),
        Default::default(),
        results
    );
}

/// The mocked runtime wraps some contract panics as `GuestPanic { panic_msg: "..." }`; unwrap that.
fn contract_message(raw: &str) -> String {
    const MARK: &str = "panic_msg: \"";
    match raw.find(MARK) {
        Some(start) => {
            let rest = &raw[start + MARK.len()..];
            rest.split('"').next().unwrap_or(rest).to_string()
        }
        None => raw.to_string(),
    }
}
