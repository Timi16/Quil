#!/usr/bin/env bash
# Build every contract for NEAR the way cargo-near would, then shrink it.
#
# `--cfg near` matters: without it near-sdk's panics never reach the NEAR host,
# so every refusal shows up as a bare `unreachable` trap with no message.
set -euo pipefail
export PATH="$HOME/.cargo/bin:$PATH"
OUT=deploy/wasm
mkdir -p "$OUT"

RUSTFLAGS="--cfg near" cargo build --target wasm32-unknown-unknown --release \
  -p claim-market -p fixing-oracle -p mock-usdc -p mock-outlayer

for contract in claim_market fixing_oracle mock_usdc mock_outlayer; do
  wasm-opt -Oz --strip-debug --strip-producers \
    --enable-bulk-memory --enable-bulk-memory-opt --enable-sign-ext \
    --enable-mutable-globals --enable-nontrapping-float-to-int \
    "target/wasm32-unknown-unknown/release/$contract.wasm" -o "$OUT/$contract.wasm"
done

cd "$OUT" && shasum -a 256 claim_market.wasm fixing_oracle.wasm mock_usdc.wasm mock_outlayer.wasm | tee SHA256SUMS
ls -l claim_market.wasm fixing_oracle.wasm mock_usdc.wasm mock_outlayer.wasm | awk '{print $5, $9}'
