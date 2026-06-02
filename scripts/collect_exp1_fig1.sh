#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
INPUT_ROOT="$ROOT_DIR/scripts/output/clients"
OUTPUT_FILE="$ROOT_DIR/scripts/expdata/exp1-fig1.txt"

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

mapfile -d '' upload_files < <(find "$INPUT_ROOT" -type f -name '*-upload-*.txt' -print0 | sort -z)

if [[ ${#upload_files[@]} -eq 0 ]]; then
  echo "No upload report files found under $INPUT_ROOT" >&2
  exit 1
fi

for file in "${upload_files[@]}"; do
  dataset="$(extract_field dataset "$file")"
  ads_mode="$(extract_field ads_mode "$file" | tr '[:upper:]' '[:lower:]')"
  record_number="$(extract_field records "$file")"
  upload_throughput_per_sec="$(extract_field upload_throughput_per_sec "$file")"
  if average_upload_latency_ms="$(extract_field average_upload_latency_ms "$file" 2>/dev/null)"; then
    average_insert_latency_ms="$average_upload_latency_ms"
  else
    average_insert_latency_ms="$(extract_field average_insert_latency_ms "$file")"
  fi

  printf '%s,%s,%s:%s,%sms\n' \
    "$dataset" \
    "$ads_mode" \
    "$record_number" \
    "$upload_throughput_per_sec" \
    "$average_insert_latency_ms" >> "$OUTPUT_FILE"
done

echo "Appended $((${#upload_files[@]})) lines to $OUTPUT_FILE"
