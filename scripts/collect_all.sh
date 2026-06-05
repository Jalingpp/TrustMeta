#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

collect_scripts=(
  # "$SCRIPT_DIR/collect_exp1_fig1.sh"
  # "$SCRIPT_DIR/collect_exp1_fig2.sh"
  # "$SCRIPT_DIR/collect_exp1_fig3.sh"
  "$SCRIPT_DIR/collect_exp2_fig1.sh"
  "$SCRIPT_DIR/collect_exp2_fig2.sh"
  "$SCRIPT_DIR/collect_exp2_fig3.sh"
  # "$SCRIPT_DIR/collect_exp3_fig1.sh"
  # "$SCRIPT_DIR/collect_exp4_fig1.sh"
)

for script in "${collect_scripts[@]}"; do
  if [[ ! -f "$script" ]]; then
    echo "Error: missing collect script: $script" >&2
    exit 1
  fi
done

echo "Running ${#collect_scripts[@]} collect scripts..."

for script in "${collect_scripts[@]}"; do
  echo "==> $(basename "$script")"
  bash "$script"
done

echo "All collect scripts completed successfully."
