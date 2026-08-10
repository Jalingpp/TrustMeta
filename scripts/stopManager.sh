#!/usr/bin/env bash

set -euo pipefail

usage() {
  echo "Usage: $(basename "$0")"
  echo "  Stops the Manager process using scripts/data/pids/manager.pid"
}

if [[ $# -gt 0 ]]; then
  usage
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PID_FILE="$SCRIPT_DIR/data/pids/manager.pid"
STATE_FILE="$SCRIPT_DIR/data/manager.state"
RUNNING_MARKER="$SCRIPT_DIR/data/manager.running"
LOG_FILE="$SCRIPT_DIR/logs/manager.log"
CHAIN_ENABLED=""
CHAIN_DATA_DIR="${MANAGER_ETH_DATA_DIR:-$SCRIPT_DIR/data/ethereum}"
CHAIN_STATE_FILE="${MANAGER_ETH_STATE_FILE:-$SCRIPT_DIR/data/ethereum.state}"
CHAIN_OUTBOX_FILE="${MANAGER_ETH_OUTBOX_FILE:-$SCRIPT_DIR/data/ethereum.outbox.jsonl}"

if [[ -f "$STATE_FILE" ]]; then
  while IFS='=' read -r key value; do
    case "$key" in
      chain_enabled) CHAIN_ENABLED="$value" ;;
      eth_data_dir) CHAIN_DATA_DIR="$value" ;;
      eth_state_file) CHAIN_STATE_FILE="$value" ;;
      eth_outbox_file) CHAIN_OUTBOX_FILE="$value" ;;
    esac
  done < "$STATE_FILE"
fi

if [[ -z "$CHAIN_ENABLED" ]]; then
  CHAIN_ENABLED="${MANAGER_CHAIN_ENABLED:-1}"
fi
case "${CHAIN_ENABLED,,}" in
  0|false|off|no)
    CHAIN_ENABLED=0
    ;;
  *)
    CHAIN_ENABLED=1
    ;;
esac

if [[ ! -f "$PID_FILE" ]]; then
  echo "Error: pid file not found: $PID_FILE" >&2
  exit 1
fi

pid="$(tr -d '[:space:]' < "$PID_FILE")"
if [[ -z "$pid" ]]; then
  echo "Error: empty pid file: $PID_FILE" >&2
  rm -f "$PID_FILE"
  exit 1
fi

if kill -0 "$pid" 2>/dev/null; then
  echo "Stopping manager (pid=$pid)"
  kill "$pid" 2>/dev/null || true
  sleep 1
  if kill -0 "$pid" 2>/dev/null; then
    kill -9 "$pid" 2>/dev/null || true
  fi
else
  echo "Manager is not running (stale pid $pid)"
fi

rm -f "$PID_FILE"
rm -f "$STATE_FILE"
rm -f "$RUNNING_MARKER"
rm -f "$LOG_FILE"
rm -f "$SCRIPT_DIR/logs"/upload-prefix-imports-*.txt

if [[ "$CHAIN_ENABLED" -eq 1 ]]; then
  MANAGER_ETH_DATA_DIR="$CHAIN_DATA_DIR" \
    MANAGER_ETH_STATE_FILE="$CHAIN_STATE_FILE" \
    "$SCRIPT_DIR/blockchain/stop_geth.sh" || true
  rm -rf "$CHAIN_DATA_DIR"
  rm -f "$CHAIN_STATE_FILE"
  rm -f "$CHAIN_OUTBOX_FILE"
fi
