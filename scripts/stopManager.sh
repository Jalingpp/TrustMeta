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
LOG_FILE="$SCRIPT_DIR/logs/manager.log"

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
rm -f "$LOG_FILE"
