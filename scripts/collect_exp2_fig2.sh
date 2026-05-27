#!/usr/bin/env bash

set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "Usage: $(basename "$0") <hashmode>" >&2
  exit 1
fi

HASHMODE="$1"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
INPUT_ROOT="$ROOT_DIR/scripts/output/clients"
OUTPUT_FILE="$ROOT_DIR/scripts/expdata/exp2-fig2.txt"

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

normalize_number() {
  local value="$1"
  printf '%s\n' "$value" | awk '{print $1}'
}

mapfile -d '' report_files < <(find "$INPUT_ROOT" -type f -name '*-query-*.txt' -print0 | sort -z)

if [[ ${#report_files[@]} -eq 0 ]]; then
  echo "No query report files found under $INPUT_ROOT" >&2
  exit 1
fi

for file in "${report_files[@]}"; do
  dataset="$(extract_field dataset "$file")"
  adsmode="$(basename "$(dirname "$file")")"
  average_query_keyword_count="$(normalize_number "$(extract_field average_query_keyword_count "$file")")"
  average_query_latency_ms="$(extract_field average_query_latency_ms "$file")"

  printf '%s,%s,%s,%s:%sms\n' \
    "$dataset" \
    "$adsmode" \
    "$HASHMODE" \
    "$average_query_keyword_count" \
    "$average_query_latency_ms" >> "$OUTPUT_FILE"
done

echo "Appended ${#report_files[@]} lines to $OUTPUT_FILE"
