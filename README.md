# Quill

A market for hedging the price of AI compute, on NEAR.

You lock USDC and get two tokens: LONG pays more if the price of a GPU-hour settles high, SHORT pays more if it settles low. Together they are always worth exactly what was locked, so there is no margin and nothing to liquidate. The settlement price is a 24-hour average of hourly fixings computed from public marketplaces by a program whose hash is pinned in the oracle contract.

**Status: hackathon build, 12 Nov 2026.** All three contracts are live on NEAR testnet. The oracle program runs inside OutLayer's TEE and has put real, live prices on-chain (first fixing: H100 $2.64 from 4 sources). `claim-market` has minted, redeemed and refunded with real transactions, and LONG and SHORT are standard NEP-245 tokens that have moved between accounts on testnet. A paid price feed has sold a real read for 0.01 tUSDC, and NEAR Intents (1Click) quotes live cross-chain collateral. No market has settled on testnet yet: market 0 expires on 14 Nov 2026 at 01:00 UTC. The section "What is and isn't done" is exact about that.

## Try it

You need [Rust](https://rustup.rs) (stable). Nothing else: no keys, no accounts, no node.

1. **Run the whole story.**

   ```
   make demo
   ```

   You see 11 numbered steps, each printing `ok` with a figure: Alice locks 350.00 USDC and mints 100 LONG + 100 SHORT on an H100 market with a band of 1.50 to 5.00; she sells the SHORT to Bob; the oracle posts 24 hourly fixings; the market settles at 3.06; Alice claims 156.00 and Bob claims 194.00. It ends with `All checks passed` and the books balanced to the micro-USDC. Along the way it shows a fake token, an early settlement, a duplicate fixing, a garbage oracle answer and a failed USDC transfer all being refused or rolled back. Any wrong number exits non-zero.

2. **See real prices go through it.** Needs internet.

   ```
   make demo-live
   ```

   Step 0 fetches prices this minute from Vast.ai, Akash, RunPod, Shadeform and NEAR AI Cloud, prints one `src` line per source, then hands the oracle program's exact output to the oracle contract, which validates and stores it. Expect a line like `ok  H100: $2.64 from 4 source(s)`. The figure changes from run to run because the data is live.

3. **Run the oracle program alone.** Needs internet.

   ```
   make oracle-live
   ```

   You see the evidence lines on stderr and one line of JSON on stdout: the values in micro-USD, how many sources stand behind each, and a SHA-256 of the evidence.

4. **Run the tests and the linter.**

   ```
   make check
   ```

   You see 197 tests pass and clippy report nothing, with warnings treated as errors.

5. **Build the contracts for NEAR.**

   ```
   rustup target add wasm32-unknown-unknown
   make wasm
   ```

   You get `claim_market.wasm`, `fixing_oracle.wasm` and `mock_usdc.wasm`, shrunk, in `deploy/wasm/` with a `SHA256SUMS` file. The hashes match the code deployed on testnet (see "Check the deployed code" below).

6. **Build the oracle program for the enclave.**

   ```
   rustup target add wasm32-wasip2
   make oracle-wasm
   ```

   You get `quill-oracle.wasm` and its SHA-256, the hash `fixing-oracle` pins at deployment.

7. **Run the read API.** Needs internet for the live index.

   ```
   make api
   curl localhost:8080/v1/index/live
   ```

   You get JSON with the current H100, B200 and inference index values, the evidence line for each source that counted, and a `problems` list naming any source that was down or too thin this minute. `curl localhost:8080/v1/markets` answers `503` with a clear message until the contracts are deployed and `CLAIM_MARKET_ID` and `FIXING_ORACLE_ID` are set.

8. **Use the app and the API against testnet.** Needs internet.

   ```
   make api-testnet
   ```

   Open http://localhost:8080. The app reads the real testnet contracts: markets, the hourly fixings with their sources, portfolios. Connect a NEAR testnet wallet to take test USDC from the faucet, mint pairs, send LONG or SHORT, redeem, settle and claim.

   The same server answers the API (all routes are in [services/api/src/lib.rs](services/api/src/lib.rs)):

   ```
   curl localhost:8080/v1/fixings/H100/history?hours=6
   curl localhost:8080/v1/portfolio/agent.quill-1112-f33adb.testnet
   curl -i localhost:8080/v1/feed/H100
   ```

   The last one answers `402 Payment Required` with the price (0.01 tUSDC), who to pay and how. Pay with one `ft_transfer`, then send the transaction hash in `X-Payment` and your account in `X-Payer`: you get the fixing, the oracle program's hash and a receipt. The same hash a second time is refused.

9. **Price collateral from another chain.** Needs internet. 1Click runs on mainnet only, so this is a dry quote: nothing is sent.

   ```
   curl -X POST localhost:8080/v1/collateral/quote -H 'content-type: application/json' -d '{
     "origin_asset": "nep141:base-0x833589fcd6edb6e08f4c7c32d4f71b54bda02913.omft.near",
     "amount_out": "5000000", "recipient": "<your-account>.near",
     "refund_to": "<your address on Base>", "dry": true }'
   ```

   You see how much USDC on Base delivers exactly 5.00 USDC to your NEAR account, and roughly how long it takes (on 12 Nov: 5.05527 USDC, about 35 seconds). With `"dry": false` 1Click also returns a deposit address; the USDC lands in your own NEAR account, never in Quill's, and you mint from there.

## On testnet

| Contract | Account | State |
|---|---|---|
| `claim-market` | `market.quill-1112-f33adb.testnet` | Live. LONG and SHORT are NEP-245 tokens `0:L` and `0:S`. Market 0: H100, band 1.50 to 5.00, expires 14 Nov 2026 01:00 UTC |
| test USDC (`mock-usdc`) | `usdc.quill-1112-f33adb.testnet` | Live. 6 decimals, open faucet, no value |
| `fixing-oracle` | `oracle.quill-1112-f33adb.testnet` | Live, with a week of history readable through `fixings_between`. Pinned to [`quill-oracle.wasm`](https://github.com/Timi16/Quil/releases/tag/oracle-v0.1.0), SHA-256 `c163a26f…0d54ab6`. A keeper requests a fixing every hour |

Real transactions you can look up on the [testnet explorer](https://testnet.nearblocks.io) (`/txns/<hash>`):

| What | Transaction |
|---|---|
| Mint: 350 tUSDC in, 100 LONG + 100 SHORT out | `9fBssvtcuMS5KKXXmYtWKcA7cnjteFqu9zAA6mhi2Lg8` |
| Redeem 20 pairs: 70 tUSDC paid back | `WBQM9PiW1iEcDuHJfqDXWBMYE4rRaA12u4SV8C6ktca` |
| Mint with a bad message: refunded in full | `7MfdzazffsSraBkb3CT4LwW4qMQt1x6vBFpMkDQwZ99M` |
| OutLayer fetching our program, checking its hash and running it in its TEE | `GuewQhWjpHDTQPojuhHRLHCiiwSFCNjFtQvfuFBxdizY` |
| Oracle request reaching OutLayer and its callback (early spike) | `82p4vXQyg4jK6PRnsL6SLtCpDF7q82TAoLP9mKj1SrPA` |
| NEP-245 `mt_transfer`: 2 LONG to another account | `5xherJEYw9gyEVH9Gm4kWiuENGmBFxNb8DNjiMUx7425` |
| An agent pays 0.01 tUSDC for one feed read (the API verified it and served the fixing once) | `7UQK6j9MwNG3r4Bfz48LfuU7gYYNzYPcSBdEJafpkrMa` |

First fixing on-chain (hourly slot 498471, 12 Nov 2026 15:00 UTC), computed in OutLayer's TEE from live data and validated by the contract: H100 **2.643750** (4 sources), B200 **6.985000** (3), inference **1.200000** (NEAR AI). OutLayer charged 0.0014 NEAR for the run.

Check the deployed code: `make wasm` rebuilds both contracts to the hashes in [deploy/wasm/SHA256SUMS](deploy/wasm/SHA256SUMS), which are the hashes of the code on the two accounts (read them with the RPC `view_code` method). Check the oracle program yourself: `make oracle-wasm` rebuilds it to the same SHA-256 the contract pins (`near contract call-function as-read-only oracle.quill-1112-f33adb.testnet code_ref json-args '{}' network-config testnet now`).

Read it through the API:

```
make api-testnet
curl localhost:8080/v1/markets
curl localhost:8080/v1/portfolio/quill-1112-f33adb.testnet
```

## How it works

| Piece | Where | What it does |
|---|---|---|
| `claim-market` | [contracts/claim-market](contracts/claim-market/src/lib.rs) | The only contract that holds USDC. Mints LONG and SHORT against full collateral, redeems pairs, settles once, pays claims. |
| `fixing-oracle` | [contracts/fixing-oracle](contracts/fixing-oracle/src/lib.rs) | Lets anyone request the hourly fixing from OutLayer's TEE, checks the answer, stores it, and serves the settlement value. |
| `quill-oracle` | [oracle](oracle/src/lib.rs) | The program meant to run in the enclave. Fetches the sources, takes a median per provider, then the median across providers. |
| `quill-core` | [crates/quill-core](crates/quill-core/src/lib.rs) | All the arithmetic, with no dependencies: payouts and rounding, the settlement window, the plausibility check, the index. |
| `quill-demo` | [demo](demo/src/lib.rs) | Both contracts and a mock USDC ledger in one process. |
| `quill-keeper` | [services/keeper](services/keeper/src/lib.rs) | Asks for each hour's fixing and settles expired markets. It has no privileges: both calls are public, so anyone can run one. |
| `mock-usdc` | [contracts/mock-usdc](contracts/mock-usdc/src/lib.rs) | Test only. A standard NEP-141 token with 6 decimals and a faucet, standing in for USDC on testnet. |
| `quill-api` | [services/api](services/api/src/lib.rs) | HTTP API and web server. Reads markets, fixings and portfolios from the contracts; sells the latest fixing per read for tUSDC ([feed.rs](services/api/src/feed.rs)); relays NEAR Intents quotes for collateral ([intents.rs](services/api/src/intents.rs)). Holds no keys and signs nothing. |
| web app | [app](app/) | Static HTML, CSS and JavaScript, no build step. Every transaction is signed in the user's own wallet. |

The full design, including the threat model, is in [ARCHITECTURE.md](ARCHITECTURE.md).

Rules the code keeps:

- **USDC leaves `claim-market` through two functions only**, `redeem_pair` and `claim`. There is no owner and no admin method in either contract.
- **Minting rounds the collateral up; every payout rounds down.** Randomised tests mint, trade, redeem and claim in random order and assert the contract never owes more than it holds.
- **A market's outcome is written once.** Later fixings cannot change it.
- **A market never settles on one reading.** It uses the mean of the 24 hourly fixings before expiry. If fewer than 18 exist the window extends for up to 72 hours, then falls back to whatever exists in the prior week, and if nothing exists each side gets half the band. No rule needs anyone's decision.
- **A sudden jump is flagged, not trusted.** A fixing more than 50% from the recent median is stored but left out of settlement, unless the next two hours agree with it.
- **A bad oracle answer is ignored, never fatal.** Garbage, the wrong hour, too few sources or an oversized result is logged and the hour can be requested again.

## Where the prices come from

Checked on 11 Nov 2026; all five answer without an API key.

| Source | What it gives | How it counts |
|---|---|---|
| Vast.ai | Individual on-demand offers | Median of per-GPU prices; needs 5 offers |
| Shadeform | Instance types across its clouds | Median of per-GPU prices; needs 5 |
| RunPod | Two list prices per GPU (secure, community) | Their median |
| Akash | One median per GPU model | Used as given |
| NEAR AI Cloud | Per-token price of each model | Output price of one pinned model, per 1M tokens |

A GPU index needs three of the four providers. Each provider is one vote however many listings it has, so flooding one marketplace with fake offers moves the index at most to the next provider's price; there is a test for exactly that.

## What is and isn't done

Done and run from this repository: all contracts with their tests, the oracle program against live sources, the offline end-to-end demo, the API with its paid feed and collateral quotes, the keeper and the web app. On testnet: all three contracts are deployed; the oracle program runs in OutLayer and real fixings are on-chain; `claim-market` has been exercised with real transactions; the API reads it all.

Not done:

- **No market has settled on testnet yet.** Market 0 expires on 14 Nov 2026 at 01:00 UTC and needs 18 hourly fixings in its final 24 hours; the keeper is collecting them. Settlement is otherwise proven in the offline demo and tests.
- **The keeper runs on one laptop.** If it stops, anyone can call `request_fixing` and `settle`, but nobody else is doing so yet.
- **No attestation is checked on-chain.** OutLayer's callback carries the program's output, not a TDX quote; the root of trust is the OutLayer contract.
- **`claim-market` and `fixing-oracle` on testnet still have their deploy keys.** The design deletes them so the contracts can never be changed; that is one command per contract and has not been run yet.
- **The demo is a simulation, not a NEAR sandbox.** Gas and real asynchronous receipts are covered only by the testnet runs above.
- **No market to trade LONG or SHORT in.** They are NEP-245 tokens, the standard NEAR Intents uses, but no pool or order book lists them. On testnet, "buying" a side means the holder sends it to you.
- **Cross-chain collateral is quoted, not yet deposited.** 1Click has no testnet. The API gets real mainnet quotes and deposit addresses, but no real cross-chain deposit has been made, and Quill's contracts are on testnet, so the USDC that arrives on mainnet cannot mint there yet.
- **The paid feed is HTTP 402 verified against the chain, not the x402 facilitator protocol.** The API checks the payment transaction over RPC. Anyone who sees a payment on-chain before its payer uses it could claim that one read; at 0.01 tUSDC a read, that is the accepted cost of having no signed payload.
- Not audited.

## Honest limits of the idea

- The index is a median of public list prices. List prices are sticky, so a hedge carries basis risk against what a buyer really pays.
- Trust in the price rests on OutLayer and Intel TDX, and on the marketplaces' own data. The contract does not verify a TDX quote itself.
- RunPod and Akash publish pre-aggregated figures, so two of the four votes rest on a few numbers each.
- NEAR AI inference prices change on a schedule rather than continuously, so that market protects a budget more than it tracks a moving price.

## Prior art

Compute-price markets exist. At Monad's Metropolis hackathon, [PIRY](https://github.com/Cassxbt/piry), [Agate](https://github.com/s0nderlabs/agate) and [Rackrate](https://github.com/Len3hq/rackrate) each built a fully-collateralized capped claim on an H100 index, and [Corpus](https://github.com/JTraversa/corpus-monad) built a signed, paid compute-price feed. Quill uses the same claim structure. What it sets out to add is an oracle that runs in a TEE with its code pinned on-chain, a second market on NEAR AI Cloud's own inference prices, and collateral from any chain through NEAR Intents. All three are running: the oracle program in OutLayer on testnet, the NEAR AI inference index, and live 1Click quotes for collateral (mainnet, dry so far).

## Licence

MIT. See [LICENSE](LICENSE).
