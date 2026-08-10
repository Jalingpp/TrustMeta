#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/lib.sh"

if ! command -v geth >/dev/null 2>&1; then
  echo "Error: geth is required when MANAGER_CHAIN_ENABLED=1" >&2
  exit 1
fi

if chain_is_running && [[ -f "$MANAGER_ETH_STATE_FILE" ]]; then
  echo "Geth developer chain is already running"
  exit 0
fi

stop_chain
rm -f "$MANAGER_ETH_STATE_FILE"
mkdir -p "$MANAGER_ETH_DATA_DIR" "$(dirname "$MANAGER_ETH_PID_FILE")" "$(dirname "$MANAGER_ETH_LOG_FILE")"

echo "Starting Geth developer chain at $MANAGER_ETH_RPC_URL"
nohup geth \
  --datadir "$MANAGER_ETH_DATA_DIR" \
  --dev \
  --dev.period 0 \
  --http \
  --http.addr "$MANAGER_ETH_RPC_HOST" \
  --http.port "$MANAGER_ETH_RPC_PORT" \
  --http.api eth,net,web3 \
  --http.vhosts "localhost,127.0.0.1" \
  --ipcdisable \
  --nodiscover \
  >"$MANAGER_ETH_LOG_FILE" 2>&1 &

printf '%s\n' "$!" >"$MANAGER_ETH_PID_FILE"
wait_for_chain_rpc
"$SCRIPT_DIR/deploy_contract.sh"
