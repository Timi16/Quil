#!/usr/bin/env bash
# Creates a fresh NEAR testnet account, funded by the faucet, and remembers its name
# in .env.testnet (git-ignored). The key is saved by the NEAR CLI under ~/.near-credentials.
#
# Usage: scripts/testnet-account.sh [name.testnet]
set -euo pipefail
export PATH="$HOME/.cargo/bin:$PATH"

if ! command -v near >/dev/null; then
  echo "The NEAR CLI is not installed. Run: cargo install near-cli-rs --locked" >&2
  exit 1
fi

if [ -f .env.testnet ] && grep -q '^QUILL_ACCOUNT=' .env.testnet; then
  echo "Already set up: $(grep '^QUILL_ACCOUNT=' .env.testnet | cut -d= -f2). Delete .env.testnet to make another."
  exit 0
fi

NAME="${1:-quill-$(date +%m%d)-$(openssl rand -hex 3).testnet}"
echo "Creating $NAME on NEAR testnet (free test NEAR from the faucet)..."
near account create-account sponsor-by-faucet-service "$NAME" \
  autogenerate-new-keypair save-to-legacy-keychain \
  network-config testnet create

echo "QUILL_ACCOUNT=$NAME" > .env.testnet
echo
echo "Done. Account: $NAME"
echo "Tell Claude it is ready."
