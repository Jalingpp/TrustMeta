#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/lib.sh"

state_file=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --state-file)
      state_file="${2:-}"
      shift 2
      ;;
    *)
      echo "Error: unknown argument: $1" >&2
      exit 1
      ;;
  esac
done

if [[ -z "$state_file" || ! -f "$state_file" ]]; then
  echo "Error: --state-file must point to a deployed chain state file" >&2
  exit 1
fi
if ! command -v geth >/dev/null 2>&1 || ! command -v jq >/dev/null 2>&1; then
  echo "Error: geth and jq are required for chain submission" >&2
  exit 1
fi

payload="$(< /dev/stdin)"
change_count="$(jq -r '.changes | length' <<<"$payload")"
if [[ "$change_count" == "0" ]]; then
  printf '{"tx_hash":""}\n'
  exit 0
fi

rpc_url="$(awk -F= '$1 == "eth_rpc_url" { print substr($0, index($0, "=") + 1); exit }' "$state_file")"
contract_address="$(awk -F= '$1 == "eth_contract_address" { print substr($0, index($0, "=") + 1); exit }' "$state_file")"
from_address="$(awk -F= '$1 == "eth_from_address" { print substr($0, index($0, "=") + 1); exit }' "$state_file")"

if [[ -z "$rpc_url" || -z "$contract_address" || -z "$from_address" ]]; then
  echo "Error: incomplete Ethereum chain state in $state_file" >&2
  exit 1
fi

prefixes_json="$(jq -c '[.changes[].prefix]' <<<"$payload")"
summaries_json="$(jq -c '[.changes[].summary_hex]' <<<"$payload")"

javascript="$(
  jq -nr \
    --arg contract "$contract_address" \
    --arg from "$from_address" \
    --argjson prefixes "$prefixes_json" \
    --argjson summaries "$summaries_json" \
    '
      "var prefixes = " + ($prefixes | tojson) + ";" +
      "var summaries = " + ($summaries | tojson) + ";" +
      "function word(value) { return value.replace(/^0x/, \"\").padStart(64, \"0\"); }" +
      "function arrayData(values) { return word(\"0x\" + values.length.toString(16)) + values.map(function(value) { return word(value); }).join(\"\"); }" +
      "var prefixDigests = prefixes.map(function(value) { return web3.sha3(value); });" +
      "var summaryDigests = summaries.map(function(value) { return web3.sha3(value); });" +
      "var prefixData = arrayData(prefixDigests);" +
      "var summaryData = arrayData(summaryDigests);" +
      "var summaryOffset = 64 + prefixData.length / 2;" +
      "var data = web3.sha3(\"commitBatch(bytes32[],bytes32[])\").slice(0, 10) + word(\"0x40\") + word(\"0x\" + summaryOffset.toString(16)) + prefixData + summaryData;" +
      "var txHash = eth.sendTransaction({from: " + ($from | tojson) + ", to: " + ($contract | tojson) + ", data: data, gas: \"0x989680\"});" +
      "console.log(JSON.stringify({tx_hash: txHash}));"
    '
)"

submission_output="$(geth attach --exec "$javascript" "$rpc_url" 2>&1)"
transaction_json="$(grep -Eo '\{"tx_hash":"0x[0-9a-fA-F]+"\}' <<<"$submission_output" | tail -n 1 || true)"
if [[ -z "$transaction_json" ]]; then
  echo "Error: failed to submit commitBatch transaction: $submission_output" >&2
  exit 1
fi

transaction_hash="$(jq -r '.tx_hash' <<<"$transaction_json")"
receipt=""
for _ in $(seq 1 300); do
  receipt_raw="$(geth attach --exec "JSON.stringify(eth.getTransactionReceipt('$transaction_hash'))" "$rpc_url" 2>/dev/null | tail -n 1)"
  receipt="$(
    jq -c 'if type == "string" then fromjson else . end' <<<"$receipt_raw" 2>/dev/null || true
  )"
  if [[ "$receipt" != "null" && -n "$receipt" ]]; then
    break
  fi
  sleep 0.1
done

if [[ -z "$receipt" || "$receipt" == "null" ]]; then
  echo "Error: timed out waiting for transaction receipt $transaction_hash" >&2
  exit 1
fi
if [[ "$(jq -r '.status // empty' <<<"$receipt")" != "0x1" ]]; then
  echo "Error: commitBatch transaction reverted: $receipt" >&2
  exit 1
fi

printf '%s\n' "$transaction_json"
