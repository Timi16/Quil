//! `claim-market`: the only contract in Quill that holds money.
//!
//! USDC enters through `ft_on_transfer` (mint) and leaves through exactly two
//! paths: `redeem_pair` and `claim`. There is no owner, no admin method and no
//! way to change a market after it is created.
//!
//! LONG and SHORT balances are kept in an internal ledger and moved with
//! `transfer`. (Wrapping them as NEP-141 tokens is a follow-up; the collateral
//! logic does not change.)
//!
//! Known limit: the contract pays for its own balance storage. A production
//! deployment should add NEP-145 storage deposits.

use near_sdk::json_types::U128;
use near_sdk::serde_json::{self, json};
use near_sdk::store::{LookupMap, Vector};
use near_sdk::{
    assert_one_yocto, env, ext_contract, near, require, AccountId, Gas, NearToken, PanicOnDefault,
    Promise, PromiseError, PromiseOrValue,
};
use quill_core::payout::{Band, Outcome, Side as CoreSide};
use quill_core::{slot_of, UNIT};

/// Smallest mint: 0.01 of a unit. Stops dust positions that cost more to store than they hold.
pub const MIN_MINT_UNITS: u128 = UNIT / 100;
/// Deposit required to create a market, to cover its storage.
pub const MARKET_STORAGE_DEPOSIT: NearToken = NearToken::from_millinear(10);
/// Longest index identifier accepted.
pub const MAX_INDEX_LEN: usize = 32;

const GAS_FT_TRANSFER: Gas = Gas::from_tgas(15);
const GAS_PAYOUT_CALLBACK: Gas = Gas::from_tgas(10);
const GAS_ORACLE_VIEW: Gas = Gas::from_tgas(60);
const GAS_SETTLE_CALLBACK: Gas = Gas::from_tgas(10);

/// Which token of a market.
#[near(serializers = [borsh, json])]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Long,
    Short,
}

impl Side {
    fn key(self) -> &'static str {
        match self {
            Side::Long => "L",
            Side::Short => "S",
        }
    }
}

impl From<Side> for CoreSide {
    fn from(side: Side) -> Self {
        match side {
            Side::Long => CoreSide::Long,
            Side::Short => CoreSide::Short,
        }
    }
}

/// How a market ended, as stored.
#[near(serializers = [borsh])]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoredOutcome {
    Settled(u128),
    Void,
}

impl From<StoredOutcome> for Outcome {
    fn from(outcome: StoredOutcome) -> Self {
        match outcome {
            StoredOutcome::Settled(index) => Outcome::Settled(index),
            StoredOutcome::Void => Outcome::Void,
        }
    }
}

/// One market. Everything above `long_supply` is fixed at creation.
#[near(serializers = [borsh])]
#[derive(Clone, Debug)]
pub struct Market {
    index: String,
    floor: u128,
    cap: u128,
    expiry_slot: u64,
    long_supply: u128,
    short_supply: u128,
    /// USDC held for this market. Always covers every possible payout.
    locked: u128,
    outcome: Option<StoredOutcome>,
}

impl Market {
    fn band(&self) -> Band {
        Band::new(self.floor, self.cap).unwrap_or_else(|_| env::panic_str("Corrupt market band"))
    }
}

/// A market as returned to callers.
#[near(serializers = [json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarketView {
    pub id: u32,
    pub index: String,
    pub floor: U128,
    pub cap: U128,
    pub expiry_slot: u64,
    pub long_supply: U128,
    pub short_supply: U128,
    pub locked: U128,
    /// The settlement index in micro-USD, once settled on a price.
    pub settled_at: Option<U128>,
    /// True if the market ended with no usable price: each side gets half the band.
    pub void: bool,
}

/// The oracle's answer to "what does this market settle on?". Mirrors `fixing-oracle`.
#[near(serializers = [json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettlementView {
    Waiting,
    Final(U128),
    Fallback(U128),
    Void,
}

/// `msg` of the `ft_transfer_call` that mints: `{"mint": <market id>}`.
#[near(serializers = [json])]
struct MintMsg {
    mint: u32,
}

#[ext_contract(ext_usdc)]
pub trait FungibleToken {
    fn ft_transfer(&mut self, receiver_id: AccountId, amount: U128, memo: Option<String>);
}

#[ext_contract(ext_oracle)]
pub trait FixingOracle {
    fn settlement_value(&self, index: String, expiry_slot: u64) -> SettlementView;
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct ClaimMarket {
    /// The collateral token. Set once.
    usdc: AccountId,
    /// The oracle every market settles on. Set once.
    oracle: AccountId,
    markets: Vector<Market>,
    /// `"<market>:<L|S>:<account>"` to token base units.
    balances: LookupMap<String, u128>,
}

#[near]
impl ClaimMarket {
    #[init]
    pub fn new(usdc: AccountId, oracle: AccountId) -> Self {
        Self {
            usdc,
            oracle,
            markets: Vector::new(b"m".to_vec()),
            balances: LookupMap::new(b"b".to_vec()),
        }
    }

    // ----- markets -----

    /// Create a market. Anyone may; its terms can never change afterwards.
    #[payable]
    pub fn create_market(
        &mut self,
        index: String,
        floor: U128,
        cap: U128,
        expiry_slot: u64,
    ) -> u32 {
        require!(
            env::attached_deposit() >= MARKET_STORAGE_DEPOSIT,
            "Attach at least 0.01 NEAR to cover the market's storage"
        );
        require!(
            !index.is_empty() && index.len() <= MAX_INDEX_LEN,
            "Index id must be 1 to 32 characters"
        );
        require!(
            Band::new(floor.0, cap.0).is_ok(),
            "cap must be greater than floor"
        );
        require!(
            expiry_slot > self.now_slot(),
            "Expiry must be in the future"
        );

        let id = self.markets.len();
        self.markets.push(Market {
            index: index.clone(),
            floor: floor.0,
            cap: cap.0,
            expiry_slot,
            long_supply: 0,
            short_supply: 0,
            locked: 0,
            outcome: None,
        });
        emit(
            "market_created",
            json!({ "market": id, "index": index, "floor": floor, "cap": cap, "expiry_slot": expiry_slot }),
        );
        id
    }

    // ----- money in -----

    /// NEP-141 receiver: USDC sent with `{"mint": id}` mints LONG and SHORT to the sender.
    ///
    /// Returns the unused amount, which the token contract refunds. Any panic
    /// here refunds the whole transfer.
    pub fn ft_on_transfer(
        &mut self,
        sender_id: AccountId,
        amount: U128,
        msg: String,
    ) -> PromiseOrValue<U128> {
        require!(
            env::predecessor_account_id() == self.usdc,
            "Only the collateral token can mint"
        );
        let MintMsg { mint: market_id } = serde_json::from_str(&msg)
            .unwrap_or_else(|_| env::panic_str("msg must be {\"mint\": <market id>}"));

        let mut market = self.market_or_panic(market_id);
        require!(
            market.outcome.is_none() && self.now_slot() < market.expiry_slot,
            "Market is closed to minting"
        );
        let (units, cost) = market
            .band()
            .units_for_deposit(amount.0)
            .unwrap_or_else(|_| env::panic_str("Amount too large"));
        require!(units >= MIN_MINT_UNITS, "Deposit is below the minimum mint");

        self.credit(market_id, Side::Long, &sender_id, units);
        self.credit(market_id, Side::Short, &sender_id, units);
        market.long_supply += units;
        market.short_supply += units;
        market.locked += cost;
        self.save_market(market_id, market);

        emit(
            "minted",
            json!({ "market": market_id, "account": sender_id, "units": U128(units), "locked": U128(cost) }),
        );
        PromiseOrValue::Value(U128(amount.0 - cost))
    }

    // ----- moving claims -----

    /// Send LONG or SHORT to another account.
    #[payable]
    pub fn transfer(&mut self, market_id: u32, side: Side, receiver_id: AccountId, units: U128) {
        assert_one_yocto();
        let sender = env::predecessor_account_id();
        require!(sender != receiver_id, "Sender and receiver must differ");
        require!(units.0 > 0, "Nothing to transfer");
        self.market_or_panic(market_id);

        self.debit(market_id, side, &sender, units.0);
        self.credit(market_id, side, &receiver_id, units.0);
        emit(
            "transferred",
            json!({ "market": market_id, "side": side, "from": sender, "to": receiver_id, "units": units }),
        );
    }

    // ----- money out -----

    /// Burn equal amounts of LONG and SHORT for their full collateral. Open markets only.
    pub fn redeem_pair(&mut self, market_id: u32, units: U128) -> Promise {
        let account = env::predecessor_account_id();
        let mut market = self.market_or_panic(market_id);
        require!(market.outcome.is_none(), "Market is settled: use claim");
        let amount = market
            .band()
            .pair_value(units.0)
            .unwrap_or_else(|_| env::panic_str("Amount too large"));
        require!(amount > 0, "Too few units to redeem");

        self.debit(market_id, Side::Long, &account, units.0);
        self.debit(market_id, Side::Short, &account, units.0);
        market.long_supply -= units.0;
        market.short_supply -= units.0;
        market.locked -= amount;
        self.save_market(market_id, market);

        emit(
            "redeemed",
            json!({ "market": market_id, "account": account, "units": units, "paid": U128(amount) }),
        );
        self.pay(account, market_id, units.0, units.0, amount)
    }

    /// Ask the oracle for the settlement value. Anyone may call, any number of times.
    pub fn settle(&mut self, market_id: u32) -> Promise {
        let market = self.market_or_panic(market_id);
        require!(market.outcome.is_none(), "Market is already settled");
        require!(
            self.now_slot() >= market.expiry_slot,
            "Market has not expired"
        );

        ext_oracle::ext(self.oracle.clone())
            .with_static_gas(GAS_ORACLE_VIEW)
            .settlement_value(market.index, market.expiry_slot)
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(GAS_SETTLE_CALLBACK)
                    .on_settlement(market_id),
            )
    }

    /// Burn the caller's LONG and SHORT in a settled market and pay what they are worth.
    pub fn claim(&mut self, market_id: u32) -> PromiseOrValue<U128> {
        let account = env::predecessor_account_id();
        let mut market = self.market_or_panic(market_id);
        let outcome: Outcome = market
            .outcome
            .unwrap_or_else(|| env::panic_str("Market is not settled yet"))
            .into();

        let long = self.balance(market_id, Side::Long, &account);
        let short = self.balance(market_id, Side::Short, &account);
        require!(long > 0 || short > 0, "Nothing to claim");

        let band = market.band();
        let pay = |side: CoreSide, units: u128| {
            band.payout(side, units, outcome)
                .unwrap_or_else(|_| env::panic_str("Amount too large"))
        };
        let amount = pay(CoreSide::Long, long) + pay(CoreSide::Short, short);

        self.debit(market_id, Side::Long, &account, long);
        self.debit(market_id, Side::Short, &account, short);
        market.long_supply -= long;
        market.short_supply -= short;
        market.locked -= amount;
        self.save_market(market_id, market);

        emit(
            "claimed",
            json!({ "market": market_id, "account": account, "long": U128(long), "short": U128(short), "paid": U128(amount) }),
        );
        if amount == 0 {
            // Worthless tokens are burned; there is nothing to send.
            return PromiseOrValue::Value(U128(0));
        }
        PromiseOrValue::Promise(self.pay(account, market_id, long, short, amount))
    }

    // ----- callbacks -----

    /// Records the oracle's answer. A market's outcome is written once and never changes.
    #[private]
    pub fn on_settlement(
        &mut self,
        market_id: u32,
        #[callback_result] answer: Result<SettlementView, PromiseError>,
    ) -> bool {
        let mut market = self.market_or_panic(market_id);
        if market.outcome.is_some() {
            return true;
        }
        let outcome = match answer {
            Ok(SettlementView::Final(index)) | Ok(SettlementView::Fallback(index)) => {
                StoredOutcome::Settled(index.0)
            }
            Ok(SettlementView::Void) => StoredOutcome::Void,
            Ok(SettlementView::Waiting) => {
                env::log_str("Settlement value is not ready yet");
                return false;
            }
            Err(_) => {
                env::log_str("Oracle call failed; settle can be retried");
                return false;
            }
        };
        market.outcome = Some(outcome);
        self.save_market(market_id, market);
        match outcome {
            StoredOutcome::Settled(index) => emit(
                "settled",
                json!({ "market": market_id, "index_value": U128(index) }),
            ),
            StoredOutcome::Void => emit("settled", json!({ "market": market_id, "void": true })),
        }
        true
    }

    /// If the USDC transfer failed, put back exactly what `redeem_pair` or `claim` took.
    #[private]
    pub fn on_payout(
        &mut self,
        account: AccountId,
        market_id: u32,
        long_units: U128,
        short_units: U128,
        amount: U128,
        #[callback_result] sent: Result<(), PromiseError>,
    ) -> bool {
        if sent.is_ok() {
            return true;
        }
        let mut market = self.market_or_panic(market_id);
        self.credit(market_id, Side::Long, &account, long_units.0);
        self.credit(market_id, Side::Short, &account, short_units.0);
        market.long_supply += long_units.0;
        market.short_supply += short_units.0;
        market.locked += amount.0;
        self.save_market(market_id, market);
        emit(
            "payout_reverted",
            json!({ "market": market_id, "account": account, "amount": amount }),
        );
        false
    }

    // ----- views -----

    pub fn get_market(&self, market_id: u32) -> Option<MarketView> {
        self.markets.get(market_id).map(|m| MarketView {
            id: market_id,
            index: m.index.clone(),
            floor: U128(m.floor),
            cap: U128(m.cap),
            expiry_slot: m.expiry_slot,
            long_supply: U128(m.long_supply),
            short_supply: U128(m.short_supply),
            locked: U128(m.locked),
            settled_at: match m.outcome {
                Some(StoredOutcome::Settled(index)) => Some(U128(index)),
                _ => None,
            },
            void: matches!(m.outcome, Some(StoredOutcome::Void)),
        })
    }

    pub fn market_count(&self) -> u32 {
        self.markets.len()
    }

    pub fn balance_of(&self, market_id: u32, side: Side, account_id: AccountId) -> U128 {
        U128(self.balance(market_id, side, &account_id))
    }

    /// The collateral token and the oracle. Neither can be changed.
    pub fn config(&self) -> (AccountId, AccountId) {
        (self.usdc.clone(), self.oracle.clone())
    }
}

// ----- internals -----

impl ClaimMarket {
    fn now_slot(&self) -> u64 {
        slot_of(env::block_timestamp())
    }

    fn market_or_panic(&self, market_id: u32) -> Market {
        self.markets
            .get(market_id)
            .cloned()
            .unwrap_or_else(|| env::panic_str("No such market"))
    }

    fn save_market(&mut self, market_id: u32, market: Market) {
        *self
            .markets
            .get_mut(market_id)
            .unwrap_or_else(|| env::panic_str("No such market")) = market;
    }

    fn balance_key(market_id: u32, side: Side, account: &AccountId) -> String {
        format!("{market_id}:{}:{account}", side.key())
    }

    fn balance(&self, market_id: u32, side: Side, account: &AccountId) -> u128 {
        self.balances
            .get(&Self::balance_key(market_id, side, account))
            .copied()
            .unwrap_or(0)
    }

    fn credit(&mut self, market_id: u32, side: Side, account: &AccountId, units: u128) {
        if units == 0 {
            return;
        }
        let key = Self::balance_key(market_id, side, account);
        let balance = self.balances.get(&key).copied().unwrap_or(0);
        self.balances.insert(key, balance + units);
    }

    fn debit(&mut self, market_id: u32, side: Side, account: &AccountId, units: u128) {
        if units == 0 {
            return;
        }
        let key = Self::balance_key(market_id, side, account);
        let balance = self.balances.get(&key).copied().unwrap_or(0);
        require!(balance >= units, "Insufficient balance");
        if balance == units {
            self.balances.remove(&key);
        } else {
            self.balances.insert(key, balance - units);
        }
    }

    /// Send USDC, then check it arrived. State was already updated by the caller.
    fn pay(
        &self,
        account: AccountId,
        market_id: u32,
        long_units: u128,
        short_units: u128,
        amount: u128,
    ) -> Promise {
        ext_usdc::ext(self.usdc.clone())
            .with_attached_deposit(NearToken::from_yoctonear(1))
            .with_static_gas(GAS_FT_TRANSFER)
            .ft_transfer(account.clone(), U128(amount), None)
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(GAS_PAYOUT_CALLBACK)
                    .on_payout(
                        account,
                        market_id,
                        U128(long_units),
                        U128(short_units),
                        U128(amount),
                    ),
            )
    }
}

/// NEP-297 event.
fn emit(event: &str, data: serde_json::Value) {
    env::log_str(&format!(
        "EVENT_JSON:{}",
        json!({ "standard": "quill", "version": "1.0.0", "event": event, "data": [data] })
    ));
}
