# The contract between the web app and the rest of Quill

The web app is static files in `app/` (HTML, CSS, plain ES-module JavaScript, no build step), served by the Rust API at `/`. Everything below is what the app may rely on. All amounts are exact decimal strings with six places (`"3.060000"`); never parse them to floats for money, only for drawing charts.

## Read API (same origin)

| Route | Returns |
|---|---|
| `GET /v1/config` | `{ "network": "testnet", "rpc": "https://rpc.testnet.near.org", "claim_market": "market.…", "fixing_oracle": "oracle.…", "usdc": "usdc.…", "usdc_symbol": "tUSDC", "feed_payee": "…", "feed_price": "0.010000", "explorer": "https://testnet.nearblocks.io", "slot": 498472 }`. Any contract field may be `null` when not configured. A transaction links to `{explorer}/txns/{hash}`, an account to `{explorer}/address/{account}`. |
| `GET /health` | `{ "status": "ok", "slot": 498471, "chain": {...} | null }` |
| `GET /v1/index/live` | `{ slot, values: [{ index, value, sources }], evidence: ["H100\|akash\|2560000\|56", …], problems: [string], inputs }` computed now from the public sources (not on-chain) |
| `GET /v1/markets` | `[Market]`, newest first |
| `GET /v1/markets/{id}` | `Market` |
| `GET /v1/fixings/{index}/latest` | `Fixing` (404 if none yet) |
| `GET /v1/fixings/{index}/history?hours=48` | `[Fixing]` newest first, accepted and flagged, at most 168 hours |
| `GET /v1/portfolio/{account}` | `[Position]` |
| `GET /v1/collateral/tokens` | `[{ asset_id, symbol, blockchain, decimals, price }]` tokens NEAR Intents can route from |
| `POST /v1/collateral/quote` | body `{ origin_asset, amount_out, recipient, refund_to, dry }` → `{ dry, deposit_address \| null, amount_in, amount_in_formatted, amount_out, amount_out_formatted, deadline, time_estimate_secs }` (`amount_out` is micro-USDC as a string) |
| `GET /v1/collateral/status/{deposit_address}` | `{ status, tx_hashes: [string] }` where status is `PENDING_DEPOSIT`, `PROCESSING`, `SUCCESS`, `INCOMPLETE_DEPOSIT`, `REFUNDED` or `FAILED` |
| `GET /v1/feed/{index}` | Paid read. Without payment: HTTP 402 and `{ "error", "price": "0.010000", "price_micro": "10000", "asset": "usdc.…", "pay_to": "…", "network", "how" }`. With headers `X-Payment: <tx hash>` and `X-Payer: <the account that signed it>` for a tUSDC `ft_transfer` of at least the price to `pay_to`: 200 and `{ "fixing": Fixing, "oracle": "oracle.…", "code_hash": "c163a26f…", "wasm_url": "https://…/quill-oracle.wasm", "paid": { "tx", "payer", "amount": "0.010000" } }`. A refused payment is 402 again with `error` saying why ("already used", "pays less than the price", "payment not found…"). A tx hash buys one read. 503 if the feed is off. |

Errors are `{ "error": "message" }` with 400, 402, 404, 502 or 503.

```text
Market   { id, index, floor, cap, width, expiry_slot, expires_at, status: open|expired|settled|void,
           collateral_locked, long_supply, short_supply, index_value|null, long_value|null, short_value|null }
Fixing   { index, slot, at, value, sources, inputs, accepted }
Position { market, index, status, long, short, worth_now|null, max_payout, claimable }
```

`expires_at` and `at` are Unix seconds. A slot is one hour: `slot * 3600` is its start.

## Wallet actions (NEAR testnet, signed in the user's wallet)

Use NEAR Wallet Selector with MyNearWallet (and Meteor if easy), loaded as ES modules from a CDN with exact versions. Every call below is a FunctionCall; gas 300 Tgas unless noted.

| Action | Contract | Method and args | Deposit |
|---|---|---|---|
| Get test USDC | usdc | `faucet { amount: "500000000" }` (500 tUSDC) | 0 |
| Register on the token (once per account, before receiving payouts) | usdc | `storage_deposit { account_id: <user>, registration_only: true }` | 0.00125 NEAR |
| Mint `n` pairs | usdc | `ft_transfer_call { receiver_id: <claim_market>, amount: <collateral micro-USDC>, msg: "{\"mint\":<market id>}" }` where collateral = n × width | 1 yoctoNEAR |
| Send LONG or SHORT | claim_market | `mt_transfer { receiver_id, token_id: "<market>:L" or "<market>:S", amount: <micro units>, approval: null, memo: null }` | 1 yoctoNEAR |
| Redeem `n` pairs before settlement | claim_market | `redeem_pair { market_id, units: <micro units> }` | 0 |
| Settle an expired market | claim_market | `settle { market_id }` | 0 |
| Claim a settled market | claim_market | `claim { market_id }` | 0 |
| Pay for one feed read | usdc | `ft_transfer { receiver_id: <feed_payee>, amount: "10000" }` then call `/v1/feed/{index}` with `X-Payment: <tx hash>` and `X-Payer: <signer account>` | 1 yoctoNEAR |

Units: 1 claim token = 1,000,000 base units = one GPU-hour (or one million inference tokens). Collateral for `n` whole units = `n × width` USDC.

There is no order book on testnet. Buying LONG or SHORT from someone else means they `mt_transfer` it to you; the Trade screen mints pairs and lets the user send the side they do not want.

## Verified live (12 Nov)

Run `make api-testnet` (port 8080; `PORT=8091 make api-testnet` for another). Against the real testnet contracts these all answer today: `/v1/config`, `/v1/fixings/H100/history?hours=3`, `/v1/feed/H100` (402, then 200 with a real payment from `agent.quill-1112-f33adb.testnet`), `/v1/collateral/tokens` (230 tokens), a dry `/v1/collateral/quote` (5 USDC from Base costs about 5.055 USDC, ~35 s), and `/v1/portfolio/agent.quill-1112-f33adb.testnet` (2 LONG in market 0, received by `mt_transfer`).

1Click is mainnet only: a quote's `recipient` must be a mainnet NEAR account (`*.near` or implicit). The app should say this plainly on the collateral screen and default to dry quotes.
