#!/usr/bin/env bash

set -euo pipefail

usage() {
  echo "Usage: $(basename "$0") [count]"
  echo "  count  Optional number of storager processes to stop; defaults to all"
}

if [[ $# -gt 1 ]]; then
  usage
  exit 1
fi

if [[ $# -eq 1 ]]; then
  if ! [[ "$1" =~ ^[0-9]+$ ]] || [[ "$1" -le 0 ]]; then
    echo "Error: count must be a positive integer" >&2
    usage
    exit 1
  fi
  STOP_COUNT="$1"
else
  STOP_COUNT=0
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ADDR_FILE="$SCRIPT_DIR/data/snaddrs"
PID_DIR="$SCRIPT_DIR/data/pids"
STORAGER_DATA_ROOT="$SCRIPT_DIR/data/storager"
LOG_DIR="$SCRIPT_DIR/logs"

if [[ ! -f "$ADDR_FILE" ]]; then
  echo "Error: address file not found: $ADDR_FILE" >&2
  exit 1
fi

if [[ ! -d "$PID_DIR" ]]; then
  echo "Error: pid directory not found: $PID_DIR" >&2
  exit 1
fi

if [[ ! -d "$STORAGER_DATA_ROOT" ]]; then
  mkdir -p "$STORAGER_DATA_ROOT"
fi

mkdir -p "$LOG_DIR"

entries=()
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

  if [[ -z "$name" || ! "$bind_addr" =~ ^[^[:space:]]+:[0-9]+$ || ! "$public_addr" =~ ^[^[:space:]]+:[0-9]+$ ]]; then
    echo "Error: invalid line in snaddrs: $line" >&2
    exit 1
  fi

  entries+=("$name|$bind_addr|$public_addr")
done < "$ADDR_FILE"

if (( STOP_COUNT == 0 )); then
  STOP_COUNT=${#entries[@]}
fi

if (( STOP_COUNT > ${#entries[@]} )); then
  echo "Error: requested $STOP_COUNT storagers, but only ${#entries[@]} addresses are available" >&2
  exit 1
fi

stop_one() {
  local name="$1"
  local bind_addr="$2"
  local public_addr="$3"
  local pid_file="$PID_DIR/${name}.pid"

  if [[ ! -f "$pid_file" ]]; then
    echo "Not running: $name (bind=$bind_addr, public=$public_addr) (missing pid file)"
    return
  fi

  local pid
  pid="$(tr -d '[:space:]' < "$pid_file")"
  if [[ -z "$pid" ]]; then
    echo "Not running: $name (bind=$bind_addr, public=$public_addr) (empty pid file)"
    rm -f "$pid_file"
    return
  fi

  if kill -0 "$pid" 2>/dev/null; then
    echo "Stopping $name (bind=$bind_addr, public=$public_addr) (pid=$pid)"
    kill "$pid" 2>/dev/null || true
    sleep 1
    if kill -0 "$pid" 2>/dev/null; then
      kill -9 "$pid" 2>/dev/null || true
    fi
  else
    echo "Not running: $name (bind=$bind_addr, public=$public_addr) (stale pid $pid)"
  fi

  rm -f "$pid_file"
}

for ((i = 0; i < STOP_COUNT; i++)); do
  entry="${entries[$i]}"
  name="${entry%%|*}"
  rest="${entry#*|}"
  bind_addr="${rest%%|*}"
  public_addr="${rest#*|}"
  stop_one "$name" "$bind_addr" "$public_addr"
done

find "$STORAGER_DATA_ROOT" -mindepth 1 -maxdepth 1 -exec rm -rf {} +
find "$LOG_DIR" -maxdepth 1 -type f -name 'sn*.log' -delete
