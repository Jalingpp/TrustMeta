#!/usr/bin/env bash

set -euo pipefail

usage() {
  echo "Usage: $(basename "$0") [storager_count] [ads_mode] [set_proof_mode] [split_threshold] [route_mode]"
  echo "  [storager_count]  Optional number of storagers to use from snaddrs"
  echo "  [ads_mode]        Optional ADS mode: mpt|mest|acctrie|acctree (default: acctrie)"
  echo "  [set_proof_mode]   Optional set proof mode: polynomial|accumulator (default: polynomial)"
  echo "  [split_threshold] Optional EPRing split threshold (default: 150)"
  echo "  [route_mode]      Optional routing backend: epring|chring (default: epring)"
}

if [[ $# -gt 5 ]]; then
  usage
  exit 1
fi

STORAGER_COUNT=""
ADS_MODE="acctrie"
SET_PROOF_MODE="polynomial"
SPLIT_THRESHOLD="150"
ROUTE_MODE="epring"

if [[ $# -ge 1 ]]; then
  if [[ "$1" =~ ^[0-9]+$ ]]; then
    STORAGER_COUNT="$1"
    if [[ $# -ge 2 ]]; then
      ADS_MODE="$2"
    fi
    if [[ $# -ge 3 ]]; then
      SET_PROOF_MODE="$3"
    fi
    if [[ $# -ge 4 ]]; then
      SPLIT_THRESHOLD="$4"
    fi
    if [[ $# -ge 5 ]]; then
      ROUTE_MODE="$5"
    fi
  else
    ADS_MODE="$1"
    if [[ $# -ge 2 ]]; then
      SET_PROOF_MODE="$2"
    fi
    if [[ $# -ge 3 ]]; then
      SPLIT_THRESHOLD="$3"
    fi
    if [[ $# -ge 4 ]]; then
      ROUTE_MODE="$4"
    fi
  fi
fi

case "$ADS_MODE" in
  mpt|mest|acctrie|acctree) ;;
  *)
    echo "Error: invalid ads_mode: $ADS_MODE" >&2
    usage
    exit 1
    ;;
esac

case "$SET_PROOF_MODE" in
  polynomial|accumulator) ;;
  *)
    echo "Error: invalid set_proof_mode: $SET_PROOF_MODE" >&2
    usage
    exit 1
    ;;
esac

case "$ROUTE_MODE" in
  epring|chring) ;;
  *)
    echo "Error: invalid route_mode: $ROUTE_MODE" >&2
    usage
    exit 1
    ;;
esac

if ! [[ "$SPLIT_THRESHOLD" =~ ^[0-9]+$ ]] || [[ "$SPLIT_THRESHOLD" -le 0 ]]; then
  echo "Error: invalid split_threshold: $SPLIT_THRESHOLD" >&2
  usage
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
source "$SCRIPT_DIR/lib/rebuild_if_needed.sh"
source "$SCRIPT_DIR/lib/manager_addr.sh"
ADDR_FILE="$SCRIPT_DIR/data/snaddrs"
MANAGER_BIND_ADDR_FILE="${MANAGER_BIND_ADDR_FILE:-$SCRIPT_DIR/data/manageraddrs}"
MANAGER_PUBLIC_ADDR_FILE="${MANAGER_PUBLIC_ADDR_FILE:-$SCRIPT_DIR/data/managerpublicaddrs}"
MANAGER_PUBLIC_ADDR="${MANAGER_PUBLIC_ADDR:-}"
STATE_FILE="$SCRIPT_DIR/data/manager.state"
RUNNING_MARKER="$SCRIPT_DIR/data/manager.running"
LOG_DIR="$SCRIPT_DIR/logs"
PID_DIR="$SCRIPT_DIR/data/pids"
export ACCUMULATOR_PUBLIC_PARAMS_FILE="$SCRIPT_DIR/data/accumulator_public_params.bin"
if [[ "$ADS_MODE" == "acctree" || "$ADS_MODE" == "mpt" || "$ADS_MODE" == "acctrie" ]]; then
  export MANAGER_HEAVY_STORAGER_RPC_TIMEOUT_SECS="${MANAGER_HEAVY_STORAGER_RPC_TIMEOUT_SECS:-3600}"
  export MANAGER_HEAVY_STORAGER_CONNECT_TIMEOUT_SECS="${MANAGER_HEAVY_STORAGER_CONNECT_TIMEOUT_SECS:-30}"
  export MANAGER_HEAVY_STORAGER_TCP_KEEPALIVE_SECS="${MANAGER_HEAVY_STORAGER_TCP_KEEPALIVE_SECS:-300}"
  export MANAGER_HEAVY_STORAGER_HTTP2_KEEPALIVE_INTERVAL_SECS="${MANAGER_HEAVY_STORAGER_HTTP2_KEEPALIVE_INTERVAL_SECS:-120}"
  export MANAGER_HEAVY_STORAGER_KEEPALIVE_TIMEOUT_SECS="${MANAGER_HEAVY_STORAGER_KEEPALIVE_TIMEOUT_SECS:-3600}"
else
  export MANAGER_STORAGER_RPC_TIMEOUT_SECS="${MANAGER_STORAGER_RPC_TIMEOUT_SECS:-600}"
  export MANAGER_STORAGER_CONNECT_TIMEOUT_SECS="${MANAGER_STORAGER_CONNECT_TIMEOUT_SECS:-30}"
  export MANAGER_STORAGER_TCP_KEEPALIVE_SECS="${MANAGER_STORAGER_TCP_KEEPALIVE_SECS:-120}"
  export MANAGER_STORAGER_HTTP2_KEEPALIVE_INTERVAL_SECS="${MANAGER_STORAGER_HTTP2_KEEPALIVE_INTERVAL_SECS:-60}"
  export MANAGER_STORAGER_KEEPALIVE_TIMEOUT_SECS="${MANAGER_STORAGER_KEEPALIVE_TIMEOUT_SECS:-120}"
fi

if [[ ! -f "$ADDR_FILE" ]]; then
  echo "Error: address file not found: $ADDR_FILE" >&2
  exit 1
fi

if [[ ! -f "$MANAGER_BIND_ADDR_FILE" ]]; then
  echo "Error: manager bind address file not found: $MANAGER_BIND_ADDR_FILE" >&2
  exit 1
fi

mkdir -p "$LOG_DIR"
mkdir -p "$PID_DIR"

stop_existing_manager() {
  if [[ ! -f "$PID_FILE" ]]; then
    :
  else
    local pid
    pid="$(tr -d '[:space:]' < "$PID_FILE")"
    if [[ -z "$pid" ]]; then
      rm -f "$PID_FILE"
    elif kill -0 "$pid" 2>/dev/null; then
      echo "Stopping existing manager (pid=$pid)"
      kill "$pid" 2>/dev/null || true
      sleep 1
      if kill -0 "$pid" 2>/dev/null; then
        kill -9 "$pid" 2>/dev/null || true
      fi
    fi

    rm -f "$PID_FILE" "$STATE_FILE" "$LOG_FILE"
  fi

  if ! command -v lsof >/dev/null 2>&1; then
    return
  fi

  local port listening_pids port_pid cmdline
  port="${MANAGER_BIND_ADDR##*:}"
  [[ -z "$port" ]] && return

  listening_pids="$(lsof -tiTCP:"$port" -sTCP:LISTEN 2>/dev/null || true)"
  for port_pid in $listening_pids; do
    if [[ ! -r "/proc/$port_pid/cmdline" ]]; then
      continue
    fi

    cmdline="$(tr '\0' ' ' < "/proc/$port_pid/cmdline")"
    if [[ "$cmdline" == *"/target/release/manager"* || "$cmdline" == *" target/release/manager "* || "$cmdline" == *" target/release/manager" ]]; then
      echo "Stopping existing manager on port $port (pid=$port_pid)"
      kill "$port_pid" 2>/dev/null || true
      sleep 1
      if kill -0 "$port_pid" 2>/dev/null; then
        kill -9 "$port_pid" 2>/dev/null || true
      fi
    fi
  done
}

storager_addrs=()
storager_count=0
while IFS= read -r line; do
  [[ -z "$line" ]] && continue
  [[ "$line" =~ ^[[:space:]]*# ]] && continue

  IFS=',' read -r name bind_addr public_addr extra <<<"$line"
  if [[ -n "${extra:-}" ]]; then
    echo "Error: invalid line in snaddrs (expected 2 or 3 fields): $line" >&2
    exit 1
  fi

  if [[ -z "${public_addr:-}" ]]; then
    public_addr="$bind_addr"
  fi

  if [[ -z "$name" ]]; then
    echo "Error: invalid line in snaddrs: $line" >&2
    exit 1
  fi
  if [[ ! "$bind_addr" =~ ^[^[:space:]]+:[0-9]+$ || ! "$public_addr" =~ ^[^[:space:]]+:[0-9]+$ ]]; then
    echo "Error: invalid address in snaddrs: $line" >&2
    exit 1
  fi

  storager_addrs+=("http://${public_addr}")
  storager_count=$((storager_count + 1))
  if [[ -n "$STORAGER_COUNT" && "$storager_count" -ge "$STORAGER_COUNT" ]]; then
    break
  fi
done < "$ADDR_FILE"

if (( ${#storager_addrs[@]} == 0 )); then
  echo "Error: no storager addresses found in $ADDR_FILE" >&2
  exit 1
fi

if [[ -n "$STORAGER_COUNT" && "$STORAGER_COUNT" -gt ${#storager_addrs[@]} ]]; then
  echo "Error: requested $STORAGER_COUNT storagers, but only ${#storager_addrs[@]} addresses are available" >&2
  exit 1
fi

STORAGER_LIST="$(IFS=,; echo "${storager_addrs[*]}")"
export MANAGER_STORAGER_COUNT="${STORAGER_COUNT:-${#storager_addrs[@]}}"
if MANAGER_BIND_ADDR="$(read_manager_addr_file "$MANAGER_BIND_ADDR_FILE")"; then
  :
else
  MANAGER_BIND_ADDR="127.0.0.1:50051"
fi

resolve_manager_public_addr() {
  local bind_addr="$1"
  local bind_port="${bind_addr##*:}"
  local bind_host="${bind_addr%:*}"

  if [[ -n "$MANAGER_PUBLIC_ADDR" ]]; then
    printf '%s\n' "$MANAGER_PUBLIC_ADDR"
    return 0
  fi

  if MANAGER_PUBLIC_ADDR_FROM_FILE="$(read_manager_addr_file "$MANAGER_PUBLIC_ADDR_FILE" 2>/dev/null)"; then
    if [[ -n "$MANAGER_PUBLIC_ADDR_FROM_FILE" ]]; then
      printf '%s\n' "$MANAGER_PUBLIC_ADDR_FROM_FILE"
      return 0
    fi
  fi

  if [[ "$bind_host" == "0.0.0.0" || "$bind_host" == "::" ]]; then
    printf '127.0.0.1:%s\n' "$bind_port"
    return 0
  fi

  printf '%s\n' "$bind_addr"
}

MANAGER_PUBLIC_ADDR="$(resolve_manager_public_addr "$MANAGER_BIND_ADDR")"
MANAGER_BIN="$ROOT_DIR/target/release/manager"
LOG_FILE="$LOG_DIR/manager.log"
PID_FILE="$PID_DIR/manager.pid"
export EPRING_SPLIT_THRESHOLD="$SPLIT_THRESHOLD"

ensure_release_binary manager "$MANAGER_BIN"
stop_existing_manager

CHAIN_ENABLED="${MANAGER_CHAIN_ENABLED:-1}"
case "${CHAIN_ENABLED,,}" in
  0|false|off|no)
    CHAIN_ENABLED=0
    ;;
  *)
    CHAIN_ENABLED=1
    ;;
esac

CHAIN_STARTED=0
cleanup_chain_on_start_failure() {
  local status="$?"
  if [[ "$status" -ne 0 && "$CHAIN_STARTED" -eq 1 ]]; then
    echo "Manager startup failed; cleaning up the private Ethereum chain" >&2
    "$ROOT_DIR/scripts/blockchain/stop_geth.sh" || true
    rm -rf "${MANAGER_ETH_DATA_DIR:-$SCRIPT_DIR/data/ethereum}"
    rm -f "${MANAGER_ETH_STATE_FILE:-$SCRIPT_DIR/data/ethereum.state}"
    rm -f "${MANAGER_ETH_OUTBOX_FILE:-$SCRIPT_DIR/data/ethereum.outbox.jsonl}"
  fi
  return "$status"
}
trap cleanup_chain_on_start_failure EXIT

if [[ "$CHAIN_ENABLED" -eq 1 ]]; then
  export MANAGER_CHAIN_ENABLED=1
  export MANAGER_ETH_DATA_DIR="${MANAGER_ETH_DATA_DIR:-$SCRIPT_DIR/data/ethereum}"
  export MANAGER_ETH_PID_FILE="${MANAGER_ETH_PID_FILE:-$SCRIPT_DIR/data/pids/geth.pid}"
  export MANAGER_ETH_LOG_FILE="${MANAGER_ETH_LOG_FILE:-$SCRIPT_DIR/logs/geth.log}"
  export MANAGER_ETH_BUILD_DIR="${MANAGER_ETH_BUILD_DIR:-$ROOT_DIR/contracts/build}"
  export MANAGER_ETH_STATE_FILE="${MANAGER_ETH_STATE_FILE:-$SCRIPT_DIR/data/ethereum.state}"
  export MANAGER_ETH_OUTBOX_FILE="${MANAGER_ETH_OUTBOX_FILE:-$SCRIPT_DIR/data/ethereum.outbox.jsonl}"
  export MANAGER_ETH_SUBMITTER="${MANAGER_ETH_SUBMITTER:-$ROOT_DIR/scripts/blockchain/chain_submit.sh}"
  export MANAGER_ETH_RESET_SCRIPT="${MANAGER_ETH_RESET_SCRIPT:-$ROOT_DIR/scripts/blockchain/reset_chain.sh}"
  if [[ -f "$RUNNING_MARKER" ]]; then
    echo "Previous manager run marker found; creating a fresh private Ethereum chain"
  fi
  CHAIN_STARTED=1
  "$ROOT_DIR/scripts/blockchain/reset_chain.sh"

  if [[ ! -f "$MANAGER_ETH_STATE_FILE" ]]; then
    echo "Error: chain state file not found after Geth startup: $MANAGER_ETH_STATE_FILE" >&2
    exit 1
  fi
  while IFS='=' read -r key value; do
    case "$key" in
      eth_rpc_url) export MANAGER_ETH_RPC_URL="$value" ;;
      eth_chain_id) export MANAGER_ETH_CHAIN_ID="$value" ;;
      eth_contract_address) export MANAGER_ETH_CONTRACT_ADDRESS="$value" ;;
      eth_from_address) export MANAGER_ETH_FROM_ADDRESS="$value" ;;
    esac
  done < "$MANAGER_ETH_STATE_FILE"
  rm -f "$MANAGER_ETH_OUTBOX_FILE"
else
  export MANAGER_CHAIN_ENABLED=0
fi

if command -v lsof >/dev/null 2>&1; then
  port="${MANAGER_BIND_ADDR##*:}"
  if [[ -n "$port" ]]; then
    existing_pids="$(lsof -tiTCP:"$port" -sTCP:LISTEN 2>/dev/null || true)"
    if [[ -n "$existing_pids" ]]; then
      echo "Error: port $port is still in use by: $existing_pids" >&2
      echo "       Stop the process above before starting a new manager." >&2
      exit 1
    fi
  fi
fi

echo "Starting manager"
nohup "$MANAGER_BIN" --bind-addr "$MANAGER_BIND_ADDR" --ads-mode "$ADS_MODE" --set-proof-mode "$SET_PROOF_MODE" --split-threshold "$SPLIT_THRESHOLD" --route-mode "$ROUTE_MODE" --storagers "$STORAGER_LIST" >"$LOG_FILE" 2>&1 &

printf '%s\n' "$!" > "$PID_FILE"
printf '%s\n' "$!" > "$RUNNING_MARKER"
{
  echo "manager_pid=$!"
  echo "manager_bind_addr=$MANAGER_BIND_ADDR"
  echo "manager_public_addr=$MANAGER_PUBLIC_ADDR"
  echo "ads_mode=$ADS_MODE"
  echo "set_proof_mode=$SET_PROOF_MODE"
  echo "split_threshold=$SPLIT_THRESHOLD"
  echo "route_mode=$ROUTE_MODE"
  echo "chain_enabled=$CHAIN_ENABLED"
  if [[ "$CHAIN_ENABLED" -eq 1 ]]; then
    echo "eth_data_dir=$MANAGER_ETH_DATA_DIR"
    echo "eth_state_file=$MANAGER_ETH_STATE_FILE"
    echo "eth_outbox_file=$MANAGER_ETH_OUTBOX_FILE"
  fi
  echo "storager_count=${#storager_addrs[@]}"
  printf 'storager_addrs='
  (IFS=,; printf '%s' "${storager_addrs[*]}")
  echo
  echo "started_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
} > "$STATE_FILE"
trap - EXIT
echo "  pid=$! log=$LOG_FILE pidfile=$PID_FILE"
