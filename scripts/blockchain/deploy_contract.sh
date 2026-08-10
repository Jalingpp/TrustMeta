#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/lib.sh"

if ! command -v solc >/dev/null 2>&1; then
  echo "Error: solc is required to deploy EPRootProofRegistry" >&2
  exit 1
fi

if ! command -v curl >/dev/null 2>&1 || ! command -v jq >/dev/null 2>&1; then
  echo "Error: curl and jq are required to deploy EPRootProofRegistry" >&2
  exit 1
fi

wait_for_chain_rpc

mkdir -p "$MANAGER_ETH_BUILD_DIR"
solc --optimize --bin --abi --overwrite \
  --output-dir "$MANAGER_ETH_BUILD_DIR" \
  "$MANAGER_ETH_CONTRACT_SOURCE" >/dev/null

bytecode_file="$MANAGER_ETH_BUILD_DIR/EPRootProofRegistry.bin"
if [[ ! -s "$bytecode_file" ]]; then
  echo "Error: compiled contract bytecode not found at $bytecode_file" >&2
  exit 1
fi

chain_id="$(chain_result eth_chainId)"
accounts_response="$(chain_json_rpc eth_accounts)"
from_address="$(jq -r '.result[0] // empty' <<<"$accounts_response")"
if [[ -z "$from_address" || "$from_address" == "null" ]]; then
  echo "Error: Geth developer account is unavailable via eth_accounts" >&2
  exit 1
fi

bytecode="$(tr -d '[:space:]' < "$bytecode_file")"
address_without_prefix="${from_address#0x}"
constructor_arg="$(printf '%064s' "$address_without_prefix" | tr ' ' '0')"
deployment_data="0x${bytecode}${constructor_arg}"
request_params="$(jq -cn --arg from "$from_address" --arg data "$deployment_data" \
  '[{from: $from, data: $data, gas: "0x7a1200"}]')"
transaction_hash="$(chain_result eth_sendTransaction "$request_params")"

if [[ -z "$transaction_hash" || "$transaction_hash" == "null" ]]; then
  echo "Error: contract deployment did not return a transaction hash" >&2
  exit 1
fi

receipt=""
for _ in $(seq 1 100); do
  receipt="$(chain_result eth_getTransactionReceipt "[\"$transaction_hash\"]")"
  if [[ "$receipt" != "null" ]]; then
    break
  fi
  sleep 0.1
done

if [[ -z "$receipt" || "$receipt" == "null" ]]; then
  echo "Error: timed out waiting for deployment transaction $transaction_hash" >&2
  exit 1
fi

status="$(jq -r '.status // empty' <<<"$receipt")"
contract_address="$(jq -r '.contractAddress // empty' <<<"$receipt")"
if [[ "$status" != "0x1" || -z "$contract_address" || "$contract_address" == "null" ]]; then
  echo "Error: contract deployment failed: $receipt" >&2
  exit 1
fi

write_chain_state "$chain_id" "$contract_address" "$from_address"
echo "Deployed EPRootProofRegistry at $contract_address on chain $chain_id"
