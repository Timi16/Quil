//! LONG and SHORT as NEP-245 multi-tokens, the standard NEAR Intents also uses.
//!
//! Token ids are `"<market>:L"` (LONG) and `"<market>:S"` (SHORT). Balances
//! live in the same ledger the market uses for collateral, so a transfer here
//! can never change what the contract owes in total.
//!
//! Approvals are not supported: only the holder can move their tokens.

use near_sdk::json_types::U128;
use near_sdk::serde_json::json;
use near_sdk::{
    assert_one_yocto, env, ext_contract, near, require, AccountId, Gas, PromiseOrValue,
};

use crate::{ClaimMarket, ClaimMarketExt, Side, MIN_MINT_UNITS};

pub const MT_SPEC: &str = "mt-1.0.0";
/// Most token ids one batch call may move.
pub const MAX_BATCH: usize = 16;
/// A transfer that gives the receiver their first holding of a token must be at
/// least this large. The contract pays the storage for every holding, so this
/// keeps a stream of dust transfers from costing it more than it costs the sender.
pub const MIN_NEW_HOLDING: u128 = MIN_MINT_UNITS;

const GAS_RESOLVE: Gas = Gas::from_tgas(15);
/// Largest receiver answer read back: one decimal string per token.
const MAX_RESOLVE_RESULT: usize = 64 * MAX_BATCH;

/// An approval as NEP-245 passes it. Quill accepts only `None`.
pub type Approval = Option<(AccountId, u64)>;
/// The approvals NEP-245 hands back to `mt_resolve_transfer`: always `None` here.
pub type ResolvedApprovals = Option<Vec<Option<Vec<(AccountId, u64, U128)>>>>;

#[near(serializers = [json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MtContractMetadata {
    pub spec: String,
    pub name: String,
}

#[near(serializers = [json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub token_id: String,
    /// Fungible within a token id, so there is no single owner.
    pub owner_id: Option<AccountId>,
}

#[ext_contract(ext_mt_receiver)]
pub trait MultiTokenReceiver {
    fn mt_on_transfer(
        &mut self,
        sender_id: AccountId,
        previous_owner_ids: Vec<AccountId>,
        token_ids: Vec<String>,
        amounts: Vec<U128>,
        msg: String,
    ) -> PromiseOrValue<Vec<U128>>;
}

/// `"3:L"` to `(3, Long)`. Anything else is not a Quill token.
pub fn parse_token_id(token_id: &str) -> Option<(u32, Side)> {
    let (market, side) = token_id.split_once(':')?;
    if market.len() > 1 && market.starts_with('0') {
        return None; // one spelling per token: "3:L", never "03:L"
    }
    let market: u32 = market.parse().ok()?;
    let side = match side {
        "L" => Side::Long,
        "S" => Side::Short,
        _ => return None,
    };
    Some((market, side))
}

pub fn token_id(market_id: u32, side: Side) -> String {
    format!("{market_id}:{}", side.key())
}

#[near]
impl ClaimMarket {
    #[payable]
    pub fn mt_transfer(
        &mut self,
        receiver_id: AccountId,
        token_id: String,
        amount: U128,
        approval: Approval,
        memo: Option<String>,
    ) {
        assert_one_yocto();
        require!(approval.is_none(), "Approvals are not supported");
        let sender = env::predecessor_account_id();
        self.move_tokens(
            &sender,
            &receiver_id,
            &[token_id],
            &[amount],
            memo.as_deref(),
        );
    }

    #[payable]
    pub fn mt_batch_transfer(
        &mut self,
        receiver_id: AccountId,
        token_ids: Vec<String>,
        amounts: Vec<U128>,
        approvals: Option<Vec<Approval>>,
        memo: Option<String>,
    ) {
        assert_one_yocto();
        require_no_approvals(approvals.as_deref());
        let sender = env::predecessor_account_id();
        self.move_tokens(&sender, &receiver_id, &token_ids, &amounts, memo.as_deref());
    }

    #[payable]
    pub fn mt_transfer_call(
        &mut self,
        receiver_id: AccountId,
        token_id: String,
        amount: U128,
        approval: Approval,
        memo: Option<String>,
        msg: String,
    ) -> PromiseOrValue<Vec<U128>> {
        assert_one_yocto();
        require!(approval.is_none(), "Approvals are not supported");
        self.transfer_and_call(receiver_id, vec![token_id], vec![amount], memo, msg)
    }

    #[payable]
    pub fn mt_batch_transfer_call(
        &mut self,
        receiver_id: AccountId,
        token_ids: Vec<String>,
        amounts: Vec<U128>,
        approvals: Option<Vec<Approval>>,
        memo: Option<String>,
        msg: String,
    ) -> PromiseOrValue<Vec<U128>> {
        assert_one_yocto();
        require_no_approvals(approvals.as_deref());
        self.transfer_and_call(receiver_id, token_ids, amounts, memo, msg)
    }

    /// Returns the amounts the receiver kept. Whatever it reports unused, or
    /// everything if it failed or answered nonsense, goes back to the sender,
    /// limited to what the receiver still holds.
    #[private]
    pub fn mt_resolve_transfer(
        &mut self,
        previous_owner_ids: Vec<AccountId>,
        receiver_id: AccountId,
        token_ids: Vec<String>,
        amounts: Vec<U128>,
        #[allow(unused_variables)] approvals: ResolvedApprovals,
    ) -> Vec<U128> {
        let sent: Vec<u128> = amounts.iter().map(|a| a.0).collect();
        let unused: Vec<u128> = env::promise_result_checked(0, MAX_RESOLVE_RESULT)
            .ok()
            .and_then(|bytes| near_sdk::serde_json::from_slice::<Vec<U128>>(&bytes).ok())
            .filter(|answer| answer.len() == sent.len())
            .map(|answer| answer.iter().zip(&sent).map(|(u, s)| u.0.min(*s)).collect())
            .unwrap_or_else(|| sent.clone());

        let mut kept = Vec::with_capacity(sent.len());
        for (i, id) in token_ids.iter().enumerate() {
            let (market, side) = parse_token_id(id).expect("validated when sent");
            let owner = previous_owner_ids.get(i).unwrap_or(&previous_owner_ids[0]);
            let refund = unused[i].min(self.balance(market, side, &receiver_id));
            if refund > 0 {
                self.debit(market, side, &receiver_id, refund);
                self.credit(market, side, owner, refund);
                nep245(
                    "mt_transfer",
                    json!({ "old_owner_id": receiver_id, "new_owner_id": owner, "token_ids": [id],
                            "amounts": [U128(refund)], "memo": "refund" }),
                );
            }
            kept.push(U128(sent[i] - refund));
        }
        kept
    }

    // ----- views -----

    pub fn mt_balance_of(&self, account_id: AccountId, token_id: String) -> U128 {
        U128(self.token_balance(&account_id, &token_id))
    }

    pub fn mt_batch_balance_of(&self, account_id: AccountId, token_ids: Vec<String>) -> Vec<U128> {
        token_ids
            .iter()
            .map(|id| U128(self.token_balance(&account_id, id)))
            .collect()
    }

    pub fn mt_supply(&self, token_id: String) -> Option<U128> {
        self.token_supply(&token_id).map(U128)
    }

    pub fn mt_batch_supply(&self, token_ids: Vec<String>) -> Vec<Option<U128>> {
        token_ids
            .iter()
            .map(|id| self.token_supply(id).map(U128))
            .collect()
    }

    pub fn mt_token(&self, token_ids: Vec<String>) -> Vec<Option<Token>> {
        token_ids
            .into_iter()
            .map(|id| {
                self.token_supply(&id).map(|_| Token {
                    token_id: id,
                    owner_id: None,
                })
            })
            .collect()
    }

    pub fn mt_metadata_contract(&self) -> MtContractMetadata {
        MtContractMetadata {
            spec: MT_SPEC.to_string(),
            name: "Quill compute-price claims (LONG = <market>:L, SHORT = <market>:S)".to_string(),
        }
    }
}

// ----- internals -----

impl ClaimMarket {
    fn token_balance(&self, account: &AccountId, token_id: &str) -> u128 {
        match parse_token_id(token_id) {
            Some((market, side)) if self.markets.get(market).is_some() => {
                self.balance(market, side, account)
            }
            _ => 0,
        }
    }

    fn token_supply(&self, token_id: &str) -> Option<u128> {
        let (market, side) = parse_token_id(token_id)?;
        let m = self.markets.get(market)?;
        Some(match side {
            Side::Long => m.long_supply,
            Side::Short => m.short_supply,
        })
    }

    /// Validate and move a batch. Panics (and so reverts everything) on any bad entry.
    pub(crate) fn move_tokens(
        &mut self,
        sender: &AccountId,
        receiver: &AccountId,
        token_ids: &[String],
        amounts: &[U128],
        memo: Option<&str>,
    ) {
        require!(sender != receiver, "Sender and receiver must differ");
        require!(!token_ids.is_empty(), "Nothing to transfer");
        require!(
            token_ids.len() == amounts.len(),
            "token_ids and amounts must have the same length"
        );
        require!(token_ids.len() <= MAX_BATCH, "At most 16 tokens per call");

        for (id, amount) in token_ids.iter().zip(amounts) {
            let (market, side) = parse_token_id(id).unwrap_or_else(|| {
                env::panic_str("Unknown token id: use \"<market>:L\" or \"<market>:S\"")
            });
            require!(self.markets.get(market).is_some(), "No such market");
            require!(amount.0 > 0, "Amounts must be positive");
            if self.balance(market, side, receiver) == 0 {
                require!(
                    amount.0 >= MIN_NEW_HOLDING,
                    "A transfer to a new holder must be at least 0.01 unit"
                );
            }
            self.debit(market, side, sender, amount.0);
            self.credit(market, side, receiver, amount.0);
        }
        let mut data = json!({ "old_owner_id": sender, "new_owner_id": receiver, "token_ids": token_ids, "amounts": amounts });
        if let Some(memo) = memo {
            data["memo"] = json!(memo);
        }
        nep245("mt_transfer", data);
    }

    fn transfer_and_call(
        &mut self,
        receiver_id: AccountId,
        token_ids: Vec<String>,
        amounts: Vec<U128>,
        memo: Option<String>,
        msg: String,
    ) -> PromiseOrValue<Vec<U128>> {
        let sender = env::predecessor_account_id();
        self.move_tokens(&sender, &receiver_id, &token_ids, &amounts, memo.as_deref());
        let owners = vec![sender.clone(); token_ids.len()];
        ext_mt_receiver::ext(receiver_id.clone())
            .mt_on_transfer(
                sender,
                owners.clone(),
                token_ids.clone(),
                amounts.clone(),
                msg,
            )
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(GAS_RESOLVE)
                    .mt_resolve_transfer(owners, receiver_id, token_ids, amounts, None),
            )
            .into()
    }
}

fn require_no_approvals(approvals: Option<&[Approval]>) {
    require!(
        approvals.is_none_or(|list| list.iter().all(Option::is_none)),
        "Approvals are not supported"
    );
}

/// A NEP-245 event (`standard: "nep245"`).
pub(crate) fn nep245(event: &str, data: near_sdk::serde_json::Value) {
    env::log_str(&format!(
        "EVENT_JSON:{}",
        json!({ "standard": "nep245", "version": "1.0.0", "event": event, "data": [data] })
    ));
}
