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
INPUT_ROOT="$ROOT_DIR/scripts/output/storagers"
OUTPUT_FILE="$ROOT_DIR/scripts/expdata/exp2-fig4.txt"

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

format_storage_bytes() {
  local bytes="$1"
  awk -v bytes="$bytes" 'BEGIN { printf "%sB(%.3fKB)", bytes, bytes / 1024 }'
}

declare -A grouped_entries
declare -A seen_adsmodes

mapfile -d '' report_files < <(find "$INPUT_ROOT" -type f -name '*.txt' -print0 | sort -z)

if [[ ${#report_files[@]} -eq 0 ]]; then
  echo "No storager report files found under $INPUT_ROOT" >&2
  exit 1
fi

for file in "${report_files[@]}"; do
  rel_path="${file#"$INPUT_ROOT"/}"
  adsmode="${rel_path%%/*}"
  storager_id="$(extract_field storager_id "$file")"
  record_count="$(extract_field record_count "$file")"
  storage_bytes="$(extract_field storage_bytes "$file")"

  entry="$(printf '%s,%s,%s' \
    "$storager_id" \
    "$record_count" \
    "$(format_storage_bytes "$storage_bytes")")"

  grouped_entries["$adsmode"]+="${entry}"$'\n'
  seen_adsmodes["$adsmode"]=1
done

mapfile -t sorted_adsmodes < <(printf '%s\n' "${!seen_adsmodes[@]}" | sort)

for adsmode in "${sorted_adsmodes[@]}"; do
  grouped_lines="${grouped_entries[$adsmode]}"
  line="$(
    printf '%s' "$grouped_lines" \
      | sed '/^$/d' \
      | sort -t, -k1,1V \
      | paste -sd';' -
  )"

  printf '%s,%s,%s:%s\n' \
    "$DATASET" \
    "$adsmode" \
    "$HASHMODE" \
    "$line" >> "$OUTPUT_FILE"
done

echo "Appended ${#sorted_adsmodes[@]} lines to $OUTPUT_FILE"
