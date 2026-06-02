#!/usr/bin/env bash

set -euo pipefail

if [[ $# -ne 0 ]]; then
  echo "Usage: $(basename "$0")" >&2
  exit 1
fi

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

extract_route_mode() {
  local file="$1"
  extract_field route_mode "$file"
}

format_storage_bytes() {
  local bytes="$1"
  awk -v bytes="$bytes" 'BEGIN { printf "%sB(%.3fKB)", bytes, bytes / 1024 }'
}

declare -A grouped_entries
declare -A seen_groups

mapfile -d '' report_files < <(find "$INPUT_ROOT" -type f -name '*.txt' -print0 | sort -z)

if [[ ${#report_files[@]} -eq 0 ]]; then
  echo "No storager report files found under $INPUT_ROOT" >&2
  exit 1
fi

for file in "${report_files[@]}"; do
  rel_path="${file#"$INPUT_ROOT"/}"
  adsmode="${rel_path%%/*}"
  dataset="$(extract_field dataset "$file")"
  storager_id="$(extract_field storager_id "$file")"
  if record_count_after_update="$(extract_field record_count_after_update "$file" 2>/dev/null)"; then
    record_count="$record_count_after_update"
  else
    record_count="$(extract_field record_count "$file")"
  fi
  storage_bytes="$(extract_field storage_bytes "$file")"
  persistence_mode="$(extract_field persistence_mode "$file")"
  route_mode="$(extract_route_mode "$file")"
  group_key="$dataset|$adsmode|$persistence_mode|$route_mode"

  entry="$(printf '%s,%s,%s' \
    "$storager_id" \
    "$record_count" \
    "$(format_storage_bytes "$storage_bytes")")"

  grouped_entries["$group_key"]+="${entry}"$'\n'
  seen_groups["$group_key"]=1
done

mapfile -t sorted_groups < <(printf '%s\n' "${!seen_groups[@]}" | sort)

for group_key in "${sorted_groups[@]}"; do
  IFS='|' read -r dataset adsmode persistence_mode route_mode <<< "$group_key"
  grouped_lines="${grouped_entries[$group_key]}"
  line="$(
    printf '%s' "$grouped_lines" \
      | sed '/^$/d' \
      | sort -t, -k1,1V \
      | paste -sd';' -
  )"

  printf '%s,%s,%s,%s:%s\n' \
    "$dataset" \
    "$adsmode" \
    "$persistence_mode" \
    "$route_mode" \
    "$line" >> "$OUTPUT_FILE"
done

echo "Appended ${#sorted_groups[@]} lines to $OUTPUT_FILE"
