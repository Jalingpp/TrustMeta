#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/lib.sh"

stop_chain
rm -rf "$MANAGER_ETH_DATA_DIR" "$MANAGER_ETH_BUILD_DIR"
rm -f "$MANAGER_ETH_STATE_FILE" "${MANAGER_ETH_OUTBOX_FILE:-$TRUSTMETA_ROOT/scripts/data/ethereum.outbox.jsonl}"
"$SCRIPT_DIR/start_geth.sh"
