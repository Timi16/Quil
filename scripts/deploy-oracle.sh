#!/usr/bin/env bash
# Deploys fixing-oracle to oracle.<account> pinned to a published program.
#   1. downloads the program from the URL and checks its SHA-256 matches the local build
#   2. asks OutLayer to run it once directly, to prove OutLayer can fetch and execute it
#   3. creates oracle.<account> and deploys fixing-oracle pinned to that URL and hash
#
# Usage: scripts/deploy-oracle.sh <program url>
set -euo pipefail
export PATH="$HOME/.cargo/bin:$PATH"
URL="${1:?usage: scripts/deploy-oracle.sh <program url>}"
. ./.env.testnet
A="$QUILL_ACCOUNT"; O="oracle.$A"
LOCAL=$(shasum -a 256 deploy/oracle/quill-oracle.wasm | cut -c1-64)

echo "== 1. download and verify"
TMP=$(mktemp -d)
curl -fsSL "$URL" -o "$TMP/p.wasm"
REMOTE=$(shasum -a 256 "$TMP/p.wasm" | cut -c1-64)
[ "$REMOTE" = "$LOCAL" ] || { echo "hash mismatch: published $REMOTE, local $LOCAL" >&2; exit 1; }
echo "   ok  $REMOTE"

SLOT=$(( $(date +%s) / 3600 ))
echo "== 2. OutLayer runs it directly (slot $SLOT)"
ARGS=$(python3 -c "import json,sys; print(json.dumps({'source':{'WasmUrl':{'url':sys.argv[1],'hash':sys.argv[2],'build_target':'wasm32-wasip2'}},'resource_limits':{'max_instructions':50000000000,'max_memory_mb':256,'max_execution_seconds':120},'input_data':json.dumps({'slot':int(sys.argv[3]),'indexes':['H100','B200','INF_OUT'],'previous':{}}),'response_format':'Json'}))" "$URL" "$LOCAL" "$SLOT")
near contract call-function as-transaction outlayer.testnet request_execution json-args "$ARGS" \
  prepaid-gas '300 Tgas' attached-deposit '0.05 NEAR' sign-as "$A" network-config testnet \
  sign-with-legacy-keychain send 2>&1 | grep -E "Transaction ID|execution_completed|Execution failed|Refunded|return value|\{\"slot\"" | sed 's/^[│ ]*//' | cut -c1-900

echo "== 3. deploy fixing-oracle to $O"
if ! near account view-account-summary "$O" network-config testnet now >/dev/null 2>&1; then
  near account create-account fund-myself "$O" '2.3 NEAR' autogenerate-new-keypair save-to-legacy-keychain \
    sign-as "$A" network-config testnet sign-with-legacy-keychain send 2>&1 | grep -E "has been successfully|rror"
fi
INIT=$(python3 -c "import json,sys; print(json.dumps({'outlayer':'outlayer.testnet','code':{'wasm_url':sys.argv[1],'wasm_hash':sys.argv[2],'build_target':'wasm32-wasip2'},'indexes':[{'id':'H100','min_sources':3},{'id':'B200','min_sources':3},{'id':'INF_OUT','min_sources':1}]}))" "$URL" "$LOCAL")
near contract deploy "$O" use-file deploy/wasm/fixing_oracle.wasm with-init-call new json-args "$INIT" \
  prepaid-gas '100 Tgas' attached-deposit '0 NEAR' network-config testnet sign-with-legacy-keychain send 2>&1 \
  | grep -E "Transaction ID|succeeded|rror|panicked"
echo "Done. Next: make keeper-once"
