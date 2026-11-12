//! The test token: faucet limits, standard transfers and metadata.

use mock_usdc::{MockUsdc, FAUCET_LIMIT};
use near_contract_standards::fungible_token::metadata::FungibleTokenMetadataProvider;
use near_contract_standards::fungible_token::FungibleTokenCore;
use near_contract_standards::storage_management::StorageManagement;
use near_sdk::json_types::U128;
use near_sdk::test_utils::VMContextBuilder;
use near_sdk::{testing_env, AccountId, NearToken};

fn acct(name: &str) -> AccountId {
    name.parse().unwrap()
}

fn as_caller(name: &str, deposit: NearToken) {
    testing_env!(VMContextBuilder::new()
        .current_account_id(acct("usdc.testnet"))
        .predecessor_account_id(acct(name))
        .attached_deposit(deposit)
        .build());
}

fn token() -> MockUsdc {
    as_caller("deployer.testnet", NearToken::from_yoctonear(0));
    MockUsdc::new()
}

#[test]
fn faucet_mints_to_the_caller_and_registers_them() {
    let mut t = token();
    as_caller("alice.testnet", NearToken::from_yoctonear(0));
    assert_eq!(t.faucet(U128(350_000_000)), U128(350_000_000));
    assert_eq!(t.faucet(U128(1)), U128(350_000_001));
    assert_eq!(t.ft_total_supply(), U128(350_000_001));
    assert!(t.storage_balance_of(acct("alice.testnet")).is_some());
}

#[test]
#[should_panic(expected = "Ask for between")]
fn faucet_rejects_zero() {
    let mut t = token();
    as_caller("alice.testnet", NearToken::from_yoctonear(0));
    t.faucet(U128(0));
}

#[test]
#[should_panic(expected = "Ask for between")]
fn faucet_rejects_more_than_the_limit() {
    let mut t = token();
    as_caller("alice.testnet", NearToken::from_yoctonear(0));
    t.faucet(U128(FAUCET_LIMIT + 1));
}

#[test]
fn standard_transfer_between_registered_accounts() {
    let mut t = token();
    as_caller("bob.testnet", NearToken::from_yoctonear(0));
    t.faucet(U128(1));
    as_caller("alice.testnet", NearToken::from_yoctonear(0));
    t.faucet(U128(100));
    as_caller("alice.testnet", NearToken::from_yoctonear(1));
    t.ft_transfer(acct("bob.testnet"), U128(40), None);
    assert_eq!(t.ft_balance_of(acct("alice.testnet")), U128(60));
    assert_eq!(t.ft_balance_of(acct("bob.testnet")), U128(41));
}

#[test]
#[should_panic(expected = "is not registered")]
fn cannot_send_to_an_unregistered_account() {
    let mut t = token();
    as_caller("alice.testnet", NearToken::from_yoctonear(0));
    t.faucet(U128(100));
    as_caller("alice.testnet", NearToken::from_yoctonear(1));
    t.ft_transfer(acct("nobody.testnet"), U128(1), None);
}

#[test]
fn says_plainly_that_it_is_a_test_token_with_six_decimals() {
    let t = token();
    let metadata = t.ft_metadata();
    assert_eq!((metadata.decimals, metadata.symbol.as_str()), (6, "tUSDC"));
    assert!(metadata.name.contains("no value"));
}
