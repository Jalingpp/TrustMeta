#!/usr/bin/env bash

set -euo pipefail

usage() {
  echo "Usage: $(basename "$0") [ip] <count> [ads_mode] [mpt_persist_interval] [acctrie_persistence_mode]"
  echo "  [ip]  Optional IPv4 address; only start storagers whose address matches this ip"
  echo "  <count>  Number of storager processes to start"
  echo "  [ads_mode]  Optional ADS mode: mpt|mest|acctrie|acctree"
  echo "  [mpt_persist_interval]  Optional MPT full-persist interval (default: 32)"
  echo "  [acctrie_persistence_mode]  Optional AccTrie persistence mode: page|kvdb (default: page)"
}

if [[ $# -lt 1 || $# -gt 5 ]]; then
  usage
  exit 1
fi

IP_FILTER=""
if [[ $# -ge 2 && "$1" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  IP_FILTER="$1"
  shift
fi

if [[ $# -lt 1 || $# -gt 4 ]]; then
  usage
  exit 1
fi

if ! [[ "$1" =~ ^[0-9]+$ ]] || [[ "$1" -le 0 ]]; then
  echo "Error: count must be a positive integer" >&2
  usage
  exit 1
fi

COUNT="$1"
ADS_MODE="${2:-}"
MPT_PERSIST_INTERVAL="${3:-32}"
ACCTRIE_PERSISTENCE_MODE="${4:-page}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
source "$SCRIPT_DIR/lib/rebuild_if_needed.sh"
ADDR_FILE="$SCRIPT_DIR/data/snaddrs"
LOG_DIR="$SCRIPT_DIR/logs"
PID_DIR="$SCRIPT_DIR/data/pids"
STORAGER_DATA_ROOT="$SCRIPT_DIR/data/storager"
export ACCUMULATOR_PUBLIC_PARAMS_FILE="$SCRIPT_DIR/data/accumulator_public_params.bin"
if [[ "$ADS_MODE" == "acctree" || "$ADS_MODE" == "mpt" ]]; then
  export STORAGER_HEAVY_SERVER_TCP_KEEPALIVE_SECS="${STORAGER_HEAVY_SERVER_TCP_KEEPALIVE_SECS:-300}"
  export STORAGER_HEAVY_SERVER_HTTP2_KEEPALIVE_INTERVAL_SECS="${STORAGER_HEAVY_SERVER_HTTP2_KEEPALIVE_INTERVAL_SECS:-120}"
  export STORAGER_HEAVY_SERVER_HTTP2_KEEPALIVE_TIMEOUT_SECS="${STORAGER_HEAVY_SERVER_HTTP2_KEEPALIVE_TIMEOUT_SECS:-3600}"
else
  export STORAGER_SERVER_TCP_KEEPALIVE_SECS="${STORAGER_SERVER_TCP_KEEPALIVE_SECS:-120}"
  export STORAGER_SERVER_HTTP2_KEEPALIVE_INTERVAL_SECS="${STORAGER_SERVER_HTTP2_KEEPALIVE_INTERVAL_SECS:-60}"
  export STORAGER_SERVER_HTTP2_KEEPALIVE_TIMEOUT_SECS="${STORAGER_SERVER_HTTP2_KEEPALIVE_TIMEOUT_SECS:-120}"
fi

if [[ ! -f "$ADDR_FILE" ]]; then
  echo "Error: address file not found: $ADDR_FILE" >&2
  exit 1
fi

mkdir -p "$LOG_DIR"
mkdir -p "$PID_DIR"
mkdir -p "$STORAGER_DATA_ROOT"

addrs=()
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
  if [[ -n "$IP_FILTER" && "${addr%:*}" != "$IP_FILTER" ]]; then
    continue
  fi

  addrs+=("$name|$addr")
done < "$ADDR_FILE"

if [[ -n "$IP_FILTER" && ${#addrs[@]} -eq 0 ]]; then
  echo "Error: no storager addresses found in $ADDR_FILE for ip $IP_FILTER" >&2
  exit 1
fi

if (( COUNT > ${#addrs[@]} )); then
  if [[ -n "$IP_FILTER" ]]; then
    echo "Error: requested $COUNT storagers for ip $IP_FILTER, but only ${#addrs[@]} addresses are available" >&2
  else
    echo "Error: requested $COUNT storagers, but only ${#addrs[@]} addresses are available" >&2
  fi
  exit 1
fi

if [[ -n "$ADS_MODE" ]]; then
  case "$ADS_MODE" in
    mpt|mest|acctrie|acctree) ;;
    *)
      echo "Error: invalid ads_mode: $ADS_MODE" >&2
      usage
      exit 1
      ;;
  esac
fi

if [[ "$ADS_MODE" == "acctrie" || "$ADS_MODE" == "accumulator" ]]; then
  case "$ACCTRIE_PERSISTENCE_MODE" in
    page|kvdb) ;;
    *)
      echo "Error: invalid acctrie_persistence_mode: $ACCTRIE_PERSISTENCE_MODE" >&2
      usage
      exit 1
      ;;
  esac
fi

if ! [[ "$MPT_PERSIST_INTERVAL" =~ ^[0-9]+$ ]] || [[ "$MPT_PERSIST_INTERVAL" -le 0 ]]; then
  echo "Error: invalid mpt_persist_interval: $MPT_PERSIST_INTERVAL" >&2
  usage
  exit 1
fi

STORAGER_BIN="$ROOT_DIR/target/release/storager"

ensure_release_binary storager "$STORAGER_BIN"

stop_existing_storager() {
  local name="$1"
  local addr="$2"
  local port="${addr##*:}"
  local pid_file="$PID_DIR/${name}.pid"

  if [[ -f "$pid_file" ]]; then
    local pid
    pid="$(tr -d '[:space:]' < "$pid_file")"
    if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
      echo "Stopping existing $name (pid=$pid)"
      kill "$pid" 2>/dev/null || true
      sleep 1
      if kill -0 "$pid" 2>/dev/null; then
        kill -9 "$pid" 2>/dev/null || true
      fi
    fi
    rm -f "$pid_file"
  fi

  if ! command -v lsof >/dev/null 2>&1; then
    return
  fi

  local listening_pids port_pid cmdline
  listening_pids="$(lsof -tiTCP:"$port" -sTCP:LISTEN 2>/dev/null || true)"
  for port_pid in $listening_pids; do
    if [[ ! -r "/proc/$port_pid/cmdline" ]]; then
      continue
    fi

    cmdline="$(tr '\0' ' ' < "/proc/$port_pid/cmdline")"
    if [[ "$cmdline" == *"/target/release/storager"* || "$cmdline" == *" target/release/storager "* || "$cmdline" == *" target/release/storager" ]]; then
      echo "Stopping existing $name on $addr (pid=$port_pid)"
      kill "$port_pid" 2>/dev/null || true
      sleep 1
      if kill -0 "$port_pid" 2>/dev/null; then
        kill -9 "$port_pid" 2>/dev/null || true
      fi
    fi
  done
}

for ((i = 0; i < COUNT; i++)); do
  entry="${addrs[$i]}"
  name="${entry%%|*}"
  addr="${entry#*|}"
  port="${addr##*:}"
  log_file="$LOG_DIR/${name}.log"
  pid_file="$PID_DIR/${name}.pid"

  stop_existing_storager "$name" "$addr"

  if command -v lsof >/dev/null 2>&1; then
    existing_pids="$(lsof -tiTCP:"$port" -sTCP:LISTEN 2>/dev/null || true)"
    if [[ -n "$existing_pids" ]]; then
      echo "Error: port $port is still in use by: $existing_pids" >&2
      echo "       Stop the process above before starting $name." >&2
      exit 1
    fi
  fi

  echo "Starting $name on $addr"
  export STORAGER_MPT_PERSIST_INTERVAL="$MPT_PERSIST_INTERVAL"
  if [[ -n "$ADS_MODE" ]]; then
    if [[ "$ADS_MODE" == "acctrie" || "$ADS_MODE" == "accumulator" ]]; then
      nohup "$STORAGER_BIN" --bind-addr "$addr" --port "$port" --ads-mode "$ADS_MODE" --acctrie-persistence "$ACCTRIE_PERSISTENCE_MODE" --storager-id "$name" >"$log_file" 2>&1 &
    else
      nohup "$STORAGER_BIN" --bind-addr "$addr" --port "$port" --ads-mode "$ADS_MODE" --storager-id "$name" >"$log_file" 2>&1 &
    fi
  else
    nohup "$STORAGER_BIN" --bind-addr "$addr" --port "$port" --storager-id "$name" >"$log_file" 2>&1 &
  fi
  launcher_pid=$!
  pid_to_store="$launcher_pid"

  printf '%s\n' "$pid_to_store" > "$pid_file"
  echo "  pid=$pid_to_store log=$log_file pidfile=$pid_file"
done
