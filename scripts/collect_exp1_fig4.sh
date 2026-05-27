#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
STORAGER_ROOT="$ROOT_DIR/scripts/output/storagers"
CLIENT_ROOT="$ROOT_DIR/scripts/output/clients"
OUTPUT_FILE="$ROOT_DIR/scripts/expdata/exp1-fig4.txt"

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

resolve_dataset() {
  local adsmode="$1"
  local client_dir="$CLIENT_ROOT/$adsmode"
  local latest_file

  if [[ ! -d "$client_dir" ]]; then
    echo "unknown"
    return 0
  fi

  latest_file="$(
    find "$client_dir" -type f -name '*-upload-*.txt' -printf '%T@ %p\n' 2>/dev/null \
      | sort -nr \
      | awk 'NR==1 {print substr($0, index($0, $2))}'
  )"

  if [[ -z "${latest_file:-}" ]]; then
    echo "unknown"
    return 0
  fi

  extract_field dataset "$latest_file" 2>/dev/null || echo "unknown"
}

mapfile -t adsmode_dirs < <(find "$STORAGER_ROOT" -mindepth 1 -maxdepth 1 -type d | sort)

if [[ ${#adsmode_dirs[@]} -eq 0 ]]; then
  echo "No storager output directories found under $STORAGER_ROOT" >&2
  exit 1
fi

for adsmode_dir in "${adsmode_dirs[@]}"; do
  adsmode="$(basename "$adsmode_dir")"
  dataset="$(resolve_dataset "$adsmode")"
  total_record_count=0
  total_proof_size=0
  file_count=0

  while IFS= read -r -d '' file; do
    record_count="$(extract_field record_count "$file")"
    proof_size="$(extract_field average_query_proof_size_bytes "$file")"

    total_record_count="$(awk -v a="$total_record_count" -v b="$record_count" 'BEGIN { printf "%.10f", a + b }')"
    total_proof_size="$(awk -v a="$total_proof_size" -v b="$proof_size" 'BEGIN { printf "%.10f", a + b }')"
    file_count=$((file_count + 1))
  done < <(find "$adsmode_dir" -type f -name '*.txt' -print0 | sort -z)

  if [[ "$file_count" -eq 0 ]]; then
    continue
  fi

  avg_record_count="$(awk -v total="$total_record_count" -v n="$file_count" 'BEGIN { printf "%.3f", total / n }')"
  avg_proof_size="$(awk -v total="$total_proof_size" -v n="$file_count" 'BEGIN { printf "%.3f", total / n }')"

  printf '%s,%s,%skv_pairs:%sbytes\n' \
    "$dataset" \
    "$adsmode" \
    "$avg_record_count" \
    "$avg_proof_size" >> "$OUTPUT_FILE"
done

echo "Appended $((${#adsmode_dirs[@]})) lines to $OUTPUT_FILE"
