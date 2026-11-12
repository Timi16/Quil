//! The whole Quill lifecycle on a local NEAR node, in about a minute.
//!
//! These are the contract builds in `deploy/wasm` (the same bytes as on
//! testnet), on a real NEAR sandbox node with real receipts, callbacks and gas.
//! The node's clock is fast-forwarded through a day of hourly fixings. The one
//! stand-in is OutLayer, which does not exist on a local node:
//! `mock_outlayer.wasm` answers `request_execution` in the shape the real one
//! does, and everything after that answer is the real fixing-oracle.
//!
//! Run with `make sandbox`. Any wrong figure exits non-zero.

use std::path::PathBuf;

use anyhow::{bail, ensure, Context, Result};
use near_workspaces::network::{Sandbox, ValidatorKey};
use near_workspaces::result::ExecutionFinalResult;
use near_workspaces::types::NearToken;
use near_workspaces::{Account, Contract, Worker};
use serde_json::{json, Value};

const UNIT: u128 = 1_000_000;
const SLOT_NANOS: u64 = 3_600_000_000_000;
/// Hourly fixings the settlement window averages.
const WINDOW: u64 = 24;
/// Alternating fixings whose mean is exactly 3.06.
const SCRIPT: [u64; 2] = [3_000_000, 3_120_000];

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("\nFAILED: {e:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let started = std::time::Instant::now();
    let wasm = |name: &str| -> Result<Vec<u8>> {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../deploy/wasm")
            .join(name);
        std::fs::read(&path)
            .with_context(|| format!("{} is missing: run `make wasm` first", path.display()))
    };

    step(
        1,
        "Start a local NEAR node and deploy the real contract builds",
    );
    // One epoch longer than the whole run: fast-forwarding costs ~5 s per epoch crossed,
    // so with the default short epochs a simulated day would take many minutes.
    let node = near_sandbox::Sandbox::start_sandbox_with_config(near_sandbox::SandboxConfig {
        additional_genesis: Some(json!({ "epoch_length": 10_000_000u64 })),
        ..Default::default()
    })
    .await
    .context("could not start the NEAR sandbox node")?;
    let worker = near_workspaces::sandbox()
        .rpc_addr(&node.rpc_addr)
        .validator_key(ValidatorKey::HomeDir(node.home_dir.path().to_path_buf()))
        .await?;
    let root = worker.root_account()?;
    let usdc = deploy(&root, "usdc", wasm("mock_usdc.wasm")?).await?;
    let outlayer = deploy(&root, "outlayer", wasm("mock_outlayer.wasm")?).await?;
    let oracle = deploy(&root, "oracle", wasm("fixing_oracle.wasm")?).await?;
    let market = deploy(&root, "market", wasm("claim_market.wasm")?).await?;
    let alice = user(&root, "alice").await?;
    let bob = user(&root, "bob").await?;

    check(usdc.call("new").transact().await?)?;
    check(oracle
        .call("new")
        .args_json(json!({
            "outlayer": outlayer.id(),
            "code": { "wasm_url": "https://github.com/Timi16/Quil/releases/download/oracle-v0.1.0/quill-oracle.wasm",
                      "wasm_hash": "c163a26f9867a3afe597de2e8abc21c157f81a9e95737a50509ad262f0d54ab6",
                      "build_target": "wasm32-wasip2" },
            "indexes": [{ "id": "H100", "min_sources": 3 }],
        }))
        .transact()
        .await?)?;
    check(
        market
            .call("new")
            .args_json(json!({ "usdc": usdc.id(), "oracle": oracle.id() }))
            .transact()
            .await?,
    )?;
    // claim-market must hold a USDC account to receive collateral, as on testnet.
    check(
        root.call(usdc.id(), "storage_deposit")
            .args_json(json!({ "account_id": market.id(), "registration_only": true }))
            .deposit(NearToken::from_micronear(1_250))
            .transact()
            .await?,
    )?;
    check(
        outlayer
            .call("set_script")
            .args_json(json!({ "values": SCRIPT.map(|v| v.to_string()) }))
            .transact()
            .await?,
    )?;
    say(format!(
        "claim-market, fixing-oracle and test USDC deployed on node at block {}",
        worker.view_block().await?.height()
    ));

    step(
        2,
        "Open an H100 market, band 1.50 to 5.00, expiring in 25 hours",
    );
    let start = slot(&worker).await?;
    let expiry = start + WINDOW + 1;
    let id: u32 = ok(alice
        .call(market.id(), "create_market")
        .args_json(
            json!({ "index": "H100", "floor": "1500000", "cap": "5000000", "expiry_slot": expiry }),
        )
        .deposit(NearToken::from_millinear(10))
        .transact()
        .await?)?
    .json()?;
    say(format!("market {id} expires at hourly slot {expiry}"));

    step(3, "Alice locks 350.00 USDC and mints 100 LONG + 100 SHORT");
    check(
        alice
            .call(usdc.id(), "faucet")
            .args_json(json!({ "amount": (350 * UNIT).to_string() }))
            .transact()
            .await?,
    )?;
    check(alice
        .call(usdc.id(), "ft_transfer_call")
        .args_json(json!({ "receiver_id": market.id(), "amount": (350 * UNIT).to_string(), "msg": json!({ "mint": id }).to_string() }))
        .deposit(NearToken::from_yoctonear(1))
        .max_gas()
        .transact()
        .await?)?;
    expect_tokens(&market, &alice, id, 100, 100).await?;
    say("alice holds 100 LONG and 100 SHORT; 350.00 USDC locked");

    step(4, "Alice sends her 100 SHORT to Bob (NEP-245 mt_transfer)");
    check(
        bob.call(usdc.id(), "storage_deposit")
            .args_json(json!({ "account_id": bob.id(), "registration_only": true }))
            .deposit(NearToken::from_micronear(1_250))
            .transact()
            .await?,
    )?;
    check(alice
        .call(market.id(), "mt_transfer")
        .args_json(json!({ "receiver_id": bob.id(), "token_id": format!("{id}:S"), "amount": (100 * UNIT).to_string(), "approval": null, "memo": null }))
        .deposit(NearToken::from_yoctonear(1))
        .transact()
        .await?)?;
    expect_tokens(&market, &alice, id, 100, 0).await?;
    expect_tokens(&market, &bob, id, 0, 100).await?;
    say("alice: 100 LONG; bob: 100 SHORT");

    step(5, "Settling before expiry is refused");
    let early = bob
        .call(market.id(), "settle")
        .args_json(json!({ "market_id": id }))
        .max_gas()
        .transact()
        .await?;
    refused(early, "Market has not expired")?;
    say("refused: Market has not expired");

    step(
        6,
        "Fast-forward a day: the oracle takes 24 hourly fixings through OutLayer's callback path",
    );
    for hour in 1..=WINDOW {
        advance_to(&worker, start + hour).await?;
        check(
            bob.call(oracle.id(), "request_fixing")
                .deposit(NearToken::from_millinear(20))
                .max_gas()
                .transact()
                .await?,
        )?;
        let fixing: Value = oracle
            .view("latest")
            .args_json(json!({ "index": "H100" }))
            .await?
            .json()?;
        ensure!(
            fixing["slot"] == json!(start + hour),
            "hour {hour}: no fixing stored for slot {}",
            start + hour
        );
        if hour == 1 || hour == WINDOW {
            say(format!(
                "slot {}: H100 {} from {} sources",
                fixing["slot"],
                micro(&fixing["value"])?,
                fixing["sources"]
            ));
        }
    }
    let again = bob
        .call(oracle.id(), "request_fixing")
        .deposit(NearToken::from_millinear(20))
        .max_gas()
        .transact()
        .await?;
    refused(again, "This hour already has a fixing")?;
    say("24 fixings stored; a second request in the same hour is refused");

    step(7, "Expiry passes; anyone settles on the 24-hour average");
    advance_to(&worker, expiry).await?;
    check(
        bob.call(market.id(), "settle")
            .args_json(json!({ "market_id": id }))
            .max_gas()
            .transact()
            .await?,
    )?;
    let m: Value = market
        .view("get_market")
        .args_json(json!({ "market_id": id }))
        .await?
        .json()?;
    ensure!(
        m["settled_at"] == json!("3060000"),
        "settled at {}, expected 3060000",
        m["settled_at"]
    );
    say("settled at 3.06 (mean of 24 fixings alternating 3.00 and 3.12)");
    let twice = bob
        .call(market.id(), "settle")
        .args_json(json!({ "market_id": id }))
        .max_gas()
        .transact()
        .await?;
    refused(twice, "already settled")?;
    say("a second settle is refused; the outcome is written once");

    step(8, "Both sides claim: LONG 156.00, SHORT 194.00");
    check(
        alice
            .call(market.id(), "claim")
            .args_json(json!({ "market_id": id }))
            .max_gas()
            .transact()
            .await?,
    )?;
    check(
        bob.call(market.id(), "claim")
            .args_json(json!({ "market_id": id }))
            .max_gas()
            .transact()
            .await?,
    )?;
    let (a, b) = (usdc_of(&usdc, &alice).await?, usdc_of(&usdc, &bob).await?);
    ensure!(a == 156 * UNIT, "alice received {}", show(a));
    ensure!(b == 194 * UNIT, "bob received {}", show(b));
    say(format!(
        "alice {} + bob {} = {}",
        show(a),
        show(b),
        show(a + b)
    ));

    step(9, "Nothing is owed and nothing can be claimed twice");
    let m: Value = market
        .view("get_market")
        .args_json(json!({ "market_id": id }))
        .await?
        .json()?;
    let locked = m["locked"].as_str().context("locked")?.parse::<u128>()?;
    let held = usdc_of(&usdc, &market).await?;
    ensure!(
        locked == 0 && held == 0,
        "market still holds {} with {} locked",
        show(held),
        show(locked)
    );
    let double = alice
        .call(market.id(), "claim")
        .args_json(json!({ "market_id": id }))
        .max_gas()
        .transact()
        .await?;
    refused(double, "Nothing to claim")?;
    say("claim-market holds 0.00 USDC; a second claim is refused");

    println!(
        "\nAll checks passed on a NEAR sandbox node in {}s. LONG 156.00 + SHORT 194.00 = 350.00 locked.",
        started.elapsed().as_secs()
    );
    Ok(())
}

// ----- helpers -----

async fn deploy(root: &Account, name: &str, code: Vec<u8>) -> Result<Contract> {
    let account = root
        .create_subaccount(name)
        .initial_balance(NearToken::from_near(10))
        .transact()
        .await?
        .into_result()?;
    Ok(account.deploy(&code).await?.into_result()?)
}

async fn user(root: &Account, name: &str) -> Result<Account> {
    Ok(root
        .create_subaccount(name)
        .initial_balance(NearToken::from_near(10))
        .transact()
        .await?
        .into_result()?)
}

/// A successful transaction, or the contract's own failure message.
fn ok(result: ExecutionFinalResult) -> Result<ExecutionFinalResult> {
    if result.is_success() {
        return Ok(result);
    }
    bail!("transaction failed: {}", failure(&result))
}

/// A successful transaction whose result is not needed.
fn check(result: ExecutionFinalResult) -> Result<()> {
    ok(result).map(drop)
}

/// A transaction that must fail with `expected` in its message.
fn refused(result: ExecutionFinalResult, expected: &str) -> Result<()> {
    ensure!(
        result.is_failure(),
        "expected a refusal containing `{expected}`, but it succeeded"
    );
    let why = failure(&result);
    ensure!(why.contains(expected), "expected `{expected}`, got: {why}");
    Ok(())
}

/// The contract's own panic text where there is one, else the raw failure.
fn failure(result: &ExecutionFinalResult) -> String {
    result
        .failures()
        .into_iter()
        .map(|f| {
            let raw = format!("{:?}", f.clone().into_result().err());
            raw.split_once("Smart contract panicked: ")
                .and_then(|(_, rest)| rest.split_once("\")").or_else(|| rest.split_once('"')))
                .map(|(message, _)| message.to_string())
                .unwrap_or(raw)
        })
        .collect::<Vec<_>>()
        .join("; ")
}

async fn slot(worker: &Worker<Sandbox>) -> Result<u64> {
    Ok(worker.view_block().await?.timestamp() / SLOT_NANOS)
}

/// Produce blocks until the node's clock reaches the start of `target` slot.
/// The node's time per block is measured once, then each hour is one jump.
async fn advance_to(worker: &Worker<Sandbox>, target: u64) -> Result<()> {
    static PER_BLOCK: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    for _ in 0..16 {
        let now = worker.view_block().await?;
        if now.timestamp() / SLOT_NANOS >= target {
            return Ok(());
        }
        let per_block = match PER_BLOCK.get() {
            Some(ns) => *ns,
            None => {
                worker.fast_forward(100).await?;
                let after = worker.view_block().await?;
                let ns =
                    (after.timestamp() - now.timestamp()) / (after.height() - now.height()).max(1);
                *PER_BLOCK.get_or_init(|| ns.max(1))
            }
        };
        let now = worker.view_block().await?.timestamp();
        let remaining = (target * SLOT_NANOS).saturating_sub(now);
        if std::env::var_os("QUILL_SANDBOX_DEBUG").is_some() {
            eprintln!("   .. {per_block} ns per block, {remaining} ns to slot {target}");
        }
        if remaining > 0 {
            worker.fast_forward(remaining / per_block + 1).await?;
        }
    }
    bail!("the node's clock did not reach slot {target}")
}

async fn expect_tokens(
    market: &Contract,
    who: &Account,
    id: u32,
    long: u128,
    short: u128,
) -> Result<()> {
    let got: Vec<String> = market
        .view("mt_batch_balance_of")
        .args_json(
            json!({ "account_id": who.id(), "token_ids": [format!("{id}:L"), format!("{id}:S")] }),
        )
        .await?
        .json()?;
    let want = [(long * UNIT).to_string(), (short * UNIT).to_string()];
    ensure!(
        got == want,
        "{} holds {:?}, expected {:?}",
        who.id(),
        got,
        want
    );
    Ok(())
}

async fn usdc_of(usdc: &Contract, who: &impl HasId) -> Result<u128> {
    let raw: String = usdc
        .view("ft_balance_of")
        .args_json(json!({ "account_id": who.account_id() }))
        .await?
        .json()?;
    Ok(raw.parse()?)
}

trait HasId {
    fn account_id(&self) -> String;
}
impl HasId for Account {
    fn account_id(&self) -> String {
        self.id().to_string()
    }
}
impl HasId for Contract {
    fn account_id(&self) -> String {
        self.id().to_string()
    }
}

fn micro(v: &Value) -> Result<String> {
    Ok(show(v.as_str().context("not a string amount")?.parse()?))
}

/// Exact: "156.00", "2.64375".
fn show(micro: u128) -> String {
    let mut text = format!("{}.{:06}", micro / UNIT, micro % UNIT);
    while text.ends_with('0') && text.len() - text.find('.').unwrap_or(0) > 3 {
        text.pop();
    }
    text
}

fn step(n: u32, what: &str) {
    println!("\n{n}. {what}");
}

fn say(line: impl AsRef<str>) {
    println!("   ok  {}", line.as_ref());
}
