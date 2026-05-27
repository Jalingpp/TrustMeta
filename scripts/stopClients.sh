#!/usr/bin/env bash

set -euo pipefail

usage() {
  echo "Usage: $(basename "$0") [count]"
  echo "  count  Optional number of clients to stop; defaults to all"
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
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
CLIENT_STATE_DIR="$SCRIPT_DIR/data/clients"
PID_DIR="$SCRIPT_DIR/data/pids"
LOG_DIR="$SCRIPT_DIR/logs"

mkdir -p "$PID_DIR"
mkdir -p "$CLIENT_STATE_DIR"
mkdir -p "$LOG_DIR"

entries=()
for state_dir in "$CLIENT_STATE_DIR"/client*; do
  [[ -d "$state_dir" ]] || continue
  client_name="$(basename "$state_dir")"
  entries+=("$client_name")
done

if (( STOP_COUNT == 0 )); then
  STOP_COUNT=${#entries[@]}
fi

if (( STOP_COUNT > ${#entries[@]} )); then
  echo "Error: requested $STOP_COUNT clients, but only ${#entries[@]} are available" >&2
  exit 1
fi

stop_one() {
  local client_name="$1"
  local state_dir="$CLIENT_STATE_DIR/$client_name"
  local pid_file="$PID_DIR/${client_name}.pid"
  local log_file="$LOG_DIR/${client_name}.log"

  if [[ -f "$pid_file" ]]; then
    local pid
    pid="$(tr -d '[:space:]' < "$pid_file")"
    if [[ -n "$pid" && -d "/proc/$pid" ]]; then
      echo "Stopping $client_name (pid=$pid)"
      kill "$pid" 2>/dev/null || true
      sleep 1
      if kill -0 "$pid" 2>/dev/null; then
        kill -9 "$pid" 2>/dev/null || true
      fi
    else
      echo "Not running: $client_name (stale or empty pid)"
    fi
    rm -f "$pid_file"
  else
    echo "Not running: $client_name (missing pid file)"
  fi

  rm -rf "$state_dir"
  rm -f "$log_file"
}

for ((i = 0; i < STOP_COUNT; i++)); do
  if (( i < ${#entries[@]} )); then
    stop_one "${entries[$i]}"
  fi
done

find "$CLIENT_STATE_DIR" -mindepth 1 -maxdepth 1 -exec rm -rf {} +
find "$LOG_DIR" -maxdepth 1 -type f -name 'client-*.log' -delete
