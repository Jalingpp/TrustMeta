#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
INPUT_ROOT="$ROOT_DIR/scripts/output/clients"
OUTPUT_FILE="$ROOT_DIR/scripts/expdata/exp1-fig3.txt"

mkdir -p "$(dirname "$OUTPUT_FILE")"

extract_field() {
  local key="$1"
  local file="$2"
  awk -F= -v key="$key" '
    $1 == key {
      sub(/^[[:space:]]+/, "", $2)
      sub(/[[:space:]]+$/, "", $2)
      print $2
      found = 1
      exit
    }
    END {
      if (!found) exit 1
    }
  ' "$file"
}

extract_optional_field() {
  local key="$1"
  local file="$2"
  awk -F= -v key="$key" '
    $1 == key {
      sub(/^[[:space:]]+/, "", $2)
      sub(/[[:space:]]+$/, "", $2)
      print $2
      found = 1
      exit
    }
    END {
      if (!found) exit 2
    }
  ' "$file"
}

extract_number() {
  local value="$1"
  printf '%s\n' "$value" | awk '{print $1}'
}

format_ms() {
  local value="$1"
  awk -v v="$value" 'BEGIN { printf "%.3fms", v }'
}

mapfile -d '' report_files < <(find "$INPUT_ROOT" -type f -name '*-update-*.txt' -print0 | sort -z)

if [[ ${#report_files[@]} -eq 0 ]]; then
  echo "No update report files found under $INPUT_ROOT" >&2
  exit 1
fi

written=0
for file in "${report_files[@]}"; do
  if ! dataset="$(extract_field dataset "$file" 2>/dev/null)"; then
    continue
  fi

  if ! adsmode="$(extract_field ads_mode "$file" 2>/dev/null | tr '[:upper:]' '[:lower:]')"; then
    continue
  fi

  if ! average_update_latency_ms="$(extract_field average_update_latency_ms "$file" 2>/dev/null)"; then
    continue
  fi

  if update_throughput_raw="$(extract_optional_field update_throughput_per_sec "$file" 2>/dev/null)"; then
    update_throughput_per_sec="$(extract_number "$update_throughput_raw")"
  elif update_throughput_raw="$(extract_optional_field throughput_updates_per_sec "$file" 2>/dev/null)"; then
    update_throughput_per_sec="$(extract_number "$update_throughput_raw")"
  else
    echo "Skipping $file: missing update throughput field" >&2
    continue
  fi

  if concurrency_raw="$(extract_optional_field concurrency "$file" 2>/dev/null)"; then
    concurrency="$(extract_number "$concurrency_raw")"
  else
    concurrency="1"
  fi

  printf '%s,%s,%s:%s,%s\n' \
    "$dataset" \
    "$adsmode" \
    "$concurrency" \
    "$update_throughput_per_sec" \
    "$(format_ms "$average_update_latency_ms")" >> "$OUTPUT_FILE"
  written=$((written + 1))
done

if [[ "$written" -eq 0 ]]; then
  echo "No update report files with required fields found under $INPUT_ROOT" >&2
  exit 1
fi

echo "Appended $written lines to $OUTPUT_FILE"
