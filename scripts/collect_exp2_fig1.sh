#!/usr/bin/env bash

set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "Usage: $(basename "$0") <dataset> <hashmode>" >&2
  exit 1
fi

DATASET="$1"
HASHMODE="$2"

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

mapfile -d '' report_files < <(find "$INPUT_ROOT" -type f -name '*.txt' -print0 | sort -z)

if [[ ${#report_files[@]} -eq 0 ]]; then
  echo "No manager report files found under $INPUT_ROOT" >&2
  exit 1
fi

for file in "${report_files[@]}"; do
  adsmode="$(basename "$(dirname "$file")")"
  storager_count="$(extract_field storager_count "$file")"
  average_storagers_per_boolean_query="$(extract_field average_storagers_per_boolean_query "$file")"

  printf '%s,%s,%s,%s:%s\n' \
    "$DATASET" \
    "$adsmode" \
    "$HASHMODE" \
    "$storager_count" \
    "$average_storagers_per_boolean_query" >> "$OUTPUT_FILE"
done

echo "Appended ${#report_files[@]} lines to $OUTPUT_FILE"
