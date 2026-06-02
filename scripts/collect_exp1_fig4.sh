#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
STORAGER_ROOT="$ROOT_DIR/scripts/output/storagers"
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

mapfile -d '' -t report_files < <(find "$STORAGER_ROOT" -type f -name '*.txt' -print0 2>/dev/null | sort -z)

if [[ ${#report_files[@]} -eq 0 ]]; then
  echo "No storager report files found under $STORAGER_ROOT" >&2
  exit 1
fi

declare -A grouped_record_count_total
declare -A grouped_proof_size_total
declare -A grouped_file_count
declare -A seen_groups

for file in "${report_files[@]}"; do
  rel_path="${file#"$STORAGER_ROOT"/}"
  adsmode="${rel_path%%/*}"

  if ! dataset="$(extract_field dataset "$file" 2>/dev/null)"; then
    continue
  fi

  if ! persistence_mode="$(extract_field persistence_mode "$file" 2>/dev/null)"; then
    continue
  fi

  if uploads_number="$(extract_field total_uploads "$file" 2>/dev/null)"; then
    :
  elif uploads_number="$(extract_field upload_record_count "$file" 2>/dev/null)"; then
    :
  else
    continue
  fi

  if [[ ! "$uploads_number" =~ ^[0-9]+$ ]]; then
    continue
  fi

  if record_count="$(extract_field record_count_after_update "$file" 2>/dev/null)"; then
    :
  else
    record_count="$(extract_field record_count "$file")"
  fi
  proof_size="$(extract_field average_query_proof_size_bytes "$file")"

  group_key="$dataset|$adsmode|$persistence_mode|$uploads_number"
  grouped_record_count_total["$group_key"]="$(awk -v a="${grouped_record_count_total[$group_key]:-0}" -v b="$record_count" 'BEGIN { printf "%.10f", a + b }')"
  grouped_proof_size_total["$group_key"]="$(awk -v a="${grouped_proof_size_total[$group_key]:-0}" -v b="$proof_size" 'BEGIN { printf "%.10f", a + b }')"
  grouped_file_count["$group_key"]=$(( ${grouped_file_count["$group_key"]:-0} + 1 ))
  seen_groups["$group_key"]=1
done

mapfile -t sorted_groups < <(printf '%s\n' "${!seen_groups[@]}" | sort)

for group_key in "${sorted_groups[@]}"; do
  IFS='|' read -r dataset adsmode persistence_mode uploads_number <<< "$group_key"
  file_count="${grouped_file_count[$group_key]}"
  avg_record_count="$(awk -v total="${grouped_record_count_total[$group_key]}" -v n="$file_count" 'BEGIN { printf "%.3f", total / n }')"
  avg_proof_size="$(awk -v total="${grouped_proof_size_total[$group_key]}" -v n="$file_count" 'BEGIN { printf "%.3f", total / n }')"

  printf '%s,%s,%s,%skv_pairs,%sbytes\n' \
    "$dataset" \
    "$adsmode" \
    "$persistence_mode" \
    "$uploads_number:$avg_record_count" \
    "$avg_proof_size" >> "$OUTPUT_FILE"
done

echo "Appended ${#sorted_groups[@]} lines to $OUTPUT_FILE"
