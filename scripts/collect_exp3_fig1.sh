#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
INPUT_ROOT="$ROOT_DIR/scripts/output/clients"
OUTPUT_FILE="$ROOT_DIR/scripts/expdata/exp3-fig1.txt"

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

extract_number() {
  local value="$1"
  printf '%s\n' "$value" | awk '{print $1}'
}

format_ms() {
  local value="$1"
  awk -v v="$value" 'BEGIN { printf "%.3fms", v }'
}

format_bytes() {
  local value="$1"
  awk -v v="$value" 'BEGIN { printf "%.3fB", v }'
}

mapfile -d '' report_files < <(find "$INPUT_ROOT" -type f -name '*.txt' -print0 | sort -z)

if [[ ${#report_files[@]} -eq 0 ]]; then
  echo "No client report files found under $INPUT_ROOT, skipping exp3-fig1 collection." >&2
  exit 0
fi

written=0
for file in "${report_files[@]}"; do
  if ! dataset="$(extract_field dataset "$file" 2>/dev/null)"; then
    continue
  fi
  if ! average_query_keyword_count_raw="$(extract_field average_query_keyword_count "$file" 2>/dev/null)"; then
    continue
  fi
  if ! average_manager_proof_aggregation_latency_ms="$(extract_field average_manager_proof_aggregation_latency_ms "$file" 2>/dev/null)"; then
    continue
  fi
  if ! average_manager_set_operation_proof_generation_latency_ms="$(extract_field average_manager_set_operation_proof_generation_latency_ms "$file" 2>/dev/null)"; then
    continue
  fi
  if ! average_proof_size_bytes_raw="$(extract_field average_proof_size_bytes "$file" 2>/dev/null)"; then
    continue
  fi
  if ! average_proof_verification_latency_ms="$(extract_field average_proof_verification_latency_ms "$file" 2>/dev/null)"; then
    continue
  fi

  adsmode="$(extract_field ads_mode "$file" | tr '[:upper:]' '[:lower:]')"
  average_query_keyword_count="$(extract_number "$average_query_keyword_count_raw")"
  manager_latency_sum="$(awk -v a="$average_manager_proof_aggregation_latency_ms" -v b="$average_manager_set_operation_proof_generation_latency_ms" 'BEGIN { printf "%.3f", a + b }')"
  average_proof_size_bytes="$(extract_number "$average_proof_size_bytes_raw")"

  printf '%s,%s,%s:%s,%s,%s\n' \
    "$dataset" \
    "$adsmode" \
    "$average_query_keyword_count" \
    "$(format_ms "$manager_latency_sum")" \
    "$(format_bytes "$average_proof_size_bytes")" \
    "$(format_ms "$average_proof_verification_latency_ms")" >> "$OUTPUT_FILE"
  written=$((written + 1))
done

if [[ "$written" -eq 0 ]]; then
  echo "No client report files with required fields found under $INPUT_ROOT, skipping exp3-fig1 collection." >&2
  exit 0
fi

echo "Appended $written lines to $OUTPUT_FILE"
