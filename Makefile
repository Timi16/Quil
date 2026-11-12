# Quill: common tasks. Needs Rust (rustup) on PATH.

# rustup installs cargo here; make sure every target finds it.
export PATH := $(HOME)/.cargo/bin:$(PATH)
.PHONY: test lint fmt check demo demo-live demo-verbose wasm oracle-wasm oracle-live api keeper keeper-once api-testnet testnet-account publish-oracle deploy-oracle

test:        ## run every test in the workspace
	cargo test --workspace

lint:        ## clippy with warnings as errors
	cargo clippy --workspace --all-targets -- -D warnings

fmt:         ## format everything
	cargo fmt --all

check: fmt lint test   ## what CI runs

demo:        ## the whole story, end to end, in-process and offline (contract event logs hidden)
	cargo run --quiet -p quill-demo 2>/dev/null || (echo 'demo failed: run make demo-verbose' && exit 1)

demo-verbose: ## the same, with every contract event log
	cargo run --quiet -p quill-demo

wasm:        ## build every contract for NEAR (with --cfg near), shrink with wasm-opt, write deploy/wasm/SHA256SUMS
	./scripts/build-contracts.sh

oracle-live: ## run the oracle program against the real price sources (needs internet)
	echo '{"slot":0,"indexes":["H100","B200","INF_OUT"]}' | cargo run --quiet -p quill-oracle

demo-live:   ## the demo, opening with a fixing made from live prices (needs internet)
	cargo run --quiet -p quill-demo -- --live 2>/dev/null || (echo 'demo failed: run make demo-verbose' && exit 1)

oracle-wasm: ## build the oracle program for OutLayer's enclave and print the hash to pin
	cargo build --target wasm32-wasip2 --release -p quill-oracle
	@shasum -a 256 target/wasm32-wasip2/release/quill-oracle.wasm

api:         ## run the read API on http://localhost:8080 (set CLAIM_MARKET_ID and FIXING_ORACLE_ID to read the chain)
	cargo run --quiet -p quill-api

testnet-account: ## create and fund a NEAR testnet account for deploying (run this yourself, once)
	./scripts/testnet-account.sh

TESTNET_ACCOUNT = $(shell sed -n 's/^QUILL_ACCOUNT=//p' .env.testnet 2>/dev/null)
CHAIN_ENV = CLAIM_MARKET_ID=market.$(TESTNET_ACCOUNT) FIXING_ORACLE_ID=oracle.$(TESTNET_ACCOUNT)

keeper:      ## run the keeper against testnet, signing as the account in .env.testnet
	KEEPER_ACCOUNT=$(TESTNET_ACCOUNT) $(CHAIN_ENV) cargo run --quiet -p quill-keeper

keeper-once: ## one keeper tick, then exit
	ONCE=1 KEEPER_ACCOUNT=$(TESTNET_ACCOUNT) $(CHAIN_ENV) cargo run --quiet -p quill-keeper

api-testnet: ## the API + web app on http://localhost:8080 against the testnet contracts, paid feed on
	$(CHAIN_ENV) USDC_ID=usdc.$(TESTNET_ACCOUNT) FEED_PAYEE=$(TESTNET_ACCOUNT) cargo run --quiet -p quill-api

publish-oracle: ## PUBLIC: push main and release quill-oracle.wasm on GitHub (run this yourself)
	./scripts/publish-oracle.sh

deploy-oracle: ## deploy fixing-oracle pinned to the published program: make deploy-oracle URL=<program url>
	./scripts/deploy-oracle.sh $(URL)
