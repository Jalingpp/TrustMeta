#!/usr/bin/env bash

set -euo pipefail

usage() {
  echo "Usage: $(basename "$0") [storager_count] [ads_mode] [set_proof_mode] [split_threshold]"
  echo "  [storager_count]  Optional number of storagers to use from snaddrs"
  echo "  [ads_mode]        Optional ADS mode: mpt|mest|acctrie|acctree (default: acctrie)"
  echo "  [set_proof_mode]   Optional set proof mode: polynomial|accumulator (default: polynomial)"
  echo "  [split_threshold] Optional EPRing split threshold (default: 150)"
}

if [[ $# -gt 4 ]]; then
  usage
  exit 1
fi

STORAGER_COUNT=""
ADS_MODE="acctrie"
SET_PROOF_MODE="polynomial"
SPLIT_THRESHOLD="150"

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
  else
    ADS_MODE="$1"
    if [[ $# -ge 2 ]]; then
      SET_PROOF_MODE="$2"
    fi
    if [[ $# -ge 3 ]]; then
      SPLIT_THRESHOLD="$3"
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
MANAGER_ADDR_FILE="$SCRIPT_DIR/data/manageraddrs"
STATE_FILE="$SCRIPT_DIR/data/manager.state"
LOG_DIR="$SCRIPT_DIR/logs"
PID_DIR="$SCRIPT_DIR/data/pids"
export ACCUMULATOR_PUBLIC_PARAMS_FILE="$SCRIPT_DIR/data/accumulator_public_params.bin"
if [[ "$ADS_MODE" == "acctree" || "$ADS_MODE" == "mpt" ]]; then
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

if [[ ! -f "$MANAGER_ADDR_FILE" ]]; then
  echo "Error: manager address file not found: $MANAGER_ADDR_FILE" >&2
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

  if [[ "$line" != *,* ]]; then
    echo "Error: invalid line in snaddrs: $line" >&2
    exit 1
  fi

  name="${line%%,*}"
  addr="${line#*,}"
  if [[ -z "$name" ]]; then
    echo "Error: invalid line in snaddrs: $line" >&2
    exit 1
  fi
  if [[ ! "$addr" =~ ^[^[:space:]]+:[0-9]+$ ]]; then
    echo "Error: invalid address in snaddrs: $line" >&2
    exit 1
  fi

  storager_addrs+=("http://${addr}")
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
if MANAGER_BIND_ADDR="$(read_manager_addr_file "$MANAGER_ADDR_FILE")"; then
  :
else
  MANAGER_BIND_ADDR="127.0.0.1:50051"
fi
MANAGER_BIN="$ROOT_DIR/target/release/manager"
LOG_FILE="$LOG_DIR/manager.log"
PID_FILE="$PID_DIR/manager.pid"
export EPRING_SPLIT_THRESHOLD="$SPLIT_THRESHOLD"

ensure_release_binary manager "$MANAGER_BIN"
stop_existing_manager

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
nohup "$MANAGER_BIN" --bind-addr "$MANAGER_BIND_ADDR" --ads-mode "$ADS_MODE" --set-proof-mode "$SET_PROOF_MODE" --split-threshold "$SPLIT_THRESHOLD" --storagers "$STORAGER_LIST" >"$LOG_FILE" 2>&1 &

printf '%s\n' "$!" > "$PID_FILE"
{
  echo "manager_pid=$!"
  echo "manager_bind_addr=$MANAGER_BIND_ADDR"
  echo "ads_mode=$ADS_MODE"
  echo "set_proof_mode=$SET_PROOF_MODE"
  echo "split_threshold=$SPLIT_THRESHOLD"
  echo "storager_count=${#storager_addrs[@]}"
  printf 'storager_addrs='
  (IFS=,; printf '%s' "${storager_addrs[*]}")
  echo
  echo "started_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
} > "$STATE_FILE"
echo "  pid=$! log=$LOG_FILE pidfile=$PID_FILE"
