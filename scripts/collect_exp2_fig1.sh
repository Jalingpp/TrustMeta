#!/usr/bin/env bash

set -euo pipefail

if [[ $# -ne 0 ]]; then
  echo "Usage: $(basename "$0")" >&2
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
INPUT_ROOT="$ROOT_DIR/scripts/output/manager"
OUTPUT_FILE="$ROOT_DIR/scripts/expdata/exp2-fig1.txt"

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

extract_route_mode() {
  local file="$1"
  extract_field route_mode "$file"
}

mapfile -d '' report_files < <(find "$INPUT_ROOT" -type f -name '*.txt' -print0 | sort -z)

if [[ ${#report_files[@]} -eq 0 ]]; then
  echo "No manager report files found under $INPUT_ROOT, skipping exp2-fig1 collection." >&2
  exit 0
fi

written=0
for file in "${report_files[@]}"; do
  if ! dataset="$(extract_field dataset "$file" 2>/dev/null)"; then
    continue
  fi
  rel_path="${file#"$INPUT_ROOT"/}"
  adsmode="${rel_path%%/*}"
  if ! route_mode="$(extract_route_mode "$file" 2>/dev/null)"; then
    continue
  fi
  if ! storager_count="$(extract_field storager_count "$file" 2>/dev/null)"; then
    continue
  fi
  if ! average_storagers_per_boolean_query="$(extract_field average_storagers_per_boolean_query "$file" 2>/dev/null)"; then
    continue
  fi

  printf '%s,%s,%s,%s:%s\n' \
    "$dataset" \
    "$adsmode" \
    "$route_mode" \
    "$storager_count" \
    "$average_storagers_per_boolean_query" >> "$OUTPUT_FILE"
  written=$((written + 1))
done

if [[ "$written" -eq 0 ]]; then
  echo "No manager report files with required fields found under $INPUT_ROOT, skipping exp2-fig1 collection." >&2
  exit 0
fi

echo "Appended $written lines to $OUTPUT_FILE"
