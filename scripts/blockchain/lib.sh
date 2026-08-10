#!/usr/bin/env bash

if [[ -z "${BASH_VERSION:-}" ]]; then
  echo "Error: blockchain lib must be sourced from bash" >&2
  return 1 2>/dev/null || exit 1
fi

BLOCKCHAIN_SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TRUSTMETA_ROOT="$(cd "$BLOCKCHAIN_SCRIPT_DIR/../.." && pwd)"

MANAGER_CHAIN_ENABLED="${MANAGER_CHAIN_ENABLED:-1}"
MANAGER_ETH_DATA_DIR="${MANAGER_ETH_DATA_DIR:-$TRUSTMETA_ROOT/scripts/data/ethereum}"
MANAGER_ETH_RPC_HOST="${MANAGER_ETH_RPC_HOST:-127.0.0.1}"
MANAGER_ETH_RPC_PORT="${MANAGER_ETH_RPC_PORT:-8545}"
MANAGER_ETH_RPC_URL="${MANAGER_ETH_RPC_URL:-http://$MANAGER_ETH_RPC_HOST:$MANAGER_ETH_RPC_PORT}"
MANAGER_ETH_STATE_FILE="${MANAGER_ETH_STATE_FILE:-$TRUSTMETA_ROOT/scripts/data/ethereum.state}"
MANAGER_ETH_PID_FILE="${MANAGER_ETH_PID_FILE:-$TRUSTMETA_ROOT/scripts/data/pids/geth.pid}"
MANAGER_ETH_LOG_FILE="${MANAGER_ETH_LOG_FILE:-$TRUSTMETA_ROOT/scripts/logs/geth.log}"
MANAGER_ETH_BUILD_DIR="${MANAGER_ETH_BUILD_DIR:-$TRUSTMETA_ROOT/contracts/build}"
MANAGER_ETH_CONTRACT_SOURCE="${MANAGER_ETH_CONTRACT_SOURCE:-$TRUSTMETA_ROOT/contracts/EPRootProofRegistry.sol}"

chain_json_rpc() {
  local method="$1"
  local params="${2:-[]}"

  curl --fail --silent --show-error \
    --connect-timeout 2 \
    --max-time 5 \
    -H 'Content-Type: application/json' \
    --data "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$method\",\"params\":$params}" \
    "$MANAGER_ETH_RPC_URL" 2>/dev/null
}

chain_result() {
  local method="$1"
  local params="${2:-[]}"
  local response

  if ! response="$(chain_json_rpc "$method" "$params")"; then
    return 1
  fi
  if [[ -z "$response" ]]; then
    return 1
  fi
  if [[ "$(jq -r '.error.message // empty' <<<"$response")" != "" ]]; then
    echo "Ethereum RPC $method failed: $(jq -r '.error.message' <<<"$response")" >&2
    return 1
  fi
  jq -r '.result' <<<"$response"
}

chain_is_running() {
  if [[ ! -f "$MANAGER_ETH_PID_FILE" ]]; then
    return 1
  fi

  local pid
  pid="$(tr -d '[:space:]' < "$MANAGER_ETH_PID_FILE")"
  [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null
}

stop_chain() {
  if [[ -f "$MANAGER_ETH_PID_FILE" ]]; then
    local pid
    pid="$(tr -d '[:space:]' < "$MANAGER_ETH_PID_FILE")"
    if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
      echo "Stopping Geth developer chain (pid=$pid)"
      kill "$pid" 2>/dev/null || true
      for _ in $(seq 1 20); do
        if ! kill -0 "$pid" 2>/dev/null; then
          break
        fi
        sleep 0.1
      done
      if kill -0 "$pid" 2>/dev/null; then
        kill -9 "$pid" 2>/dev/null || true
      fi
    fi
  fi
  rm -f "$MANAGER_ETH_PID_FILE"
}

wait_for_chain_rpc() {
  for _ in $(seq 1 100); do
    if chain_result eth_chainId >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.1
  done

  echo "Timed out waiting for Ethereum RPC at $MANAGER_ETH_RPC_URL" >&2
  return 1
}

write_chain_state() {
  local chain_id="$1"
  local contract_address="$2"
  local from_address="$3"

  mkdir -p "$(dirname "$MANAGER_ETH_STATE_FILE")"
  {
    echo "eth_rpc_url=$MANAGER_ETH_RPC_URL"
    echo "eth_chain_id=$chain_id"
    echo "eth_contract_address=$contract_address"
    echo "eth_from_address=$from_address"
    echo "eth_data_dir=$MANAGER_ETH_DATA_DIR"
    echo "eth_abi_file=$MANAGER_ETH_BUILD_DIR/EPRootProofRegistry.abi"
  } >"$MANAGER_ETH_STATE_FILE"
}
