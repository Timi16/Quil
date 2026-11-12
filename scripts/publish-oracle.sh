#!/usr/bin/env bash
# Publishes the code and the oracle program. Run this yourself: it makes things public.
#   1. pushes main to origin (github.com/Timi16/Quil)
#   2. creates the GitHub release oracle-v0.1.0 with quill-oracle.wasm attached,
#      which gives OutLayer the public https URL it downloads the program from.
set -euo pipefail
TAG=oracle-v0.1.0
WASM=deploy/oracle/quill-oracle.wasm

if [ ! -f "$WASM" ]; then
  echo "Build it first: make oracle-wasm && mkdir -p deploy/oracle && cp target/wasm32-wasip2/release/quill-oracle.wasm deploy/oracle/" >&2
  exit 1
fi
HASH=$(shasum -a 256 "$WASM" | cut -c1-64)

git push origin main
gh release create "$TAG" "$WASM" "$WASM.sha256" \
  --title "Oracle program $TAG" \
  --notes "The program Quill's fixing-oracle runs inside OutLayer's TEE. SHA-256: $HASH. Rebuild it with \`make oracle-wasm\` and compare."

REPO=$(gh repo view --json nameWithOwner --jq .nameWithOwner)
echo
echo "Published. Program URL:"
echo "https://github.com/$REPO/releases/download/$TAG/quill-oracle.wasm"
echo "Tell Claude it is published."
