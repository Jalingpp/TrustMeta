#!/usr/bin/env bash

set -euo pipefail

if [[ $# -ne 0 ]]; then
  echo "Usage: $(basename "$0")" >&2
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
INPUT_ROOT="$ROOT_DIR/scripts/output/manager"
OUTPUT_FILE="$ROOT_DIR/scripts/expdata/exp4-fig1.txt"

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

extract_io_field() {
  local field_name="$1"
  local file="$2"
  awk -F= -v field_name="$field_name" '
    $1 == "split_migration_io" {
      n = split($2, parts, ",")
      for (i = 1; i <= n; i++) {
        split(parts[i], kv, ":")
        if (kv[1] == field_name) {
          print kv[2]
          found = 1
          exit
        }
      }
    }
    END {
      if (!found) exit 1
    }
  ' "$file"
}

mapfile -d '' report_files < <(find "$INPUT_ROOT" -type f -name '*.txt' -print0 | sort -z)

if [[ ${#report_files[@]} -eq 0 ]]; then
  echo "No manager report files found under $INPUT_ROOT, skipping exp4-fig1 collection." >&2
  exit 0
fi

written=0
for file in "${report_files[@]}"; do
  if ! dataset="$(extract_field dataset "$file" 2>/dev/null)"; then
    continue
  fi
  if ! adsmode="$(extract_field route_mode "$file" 2>/dev/null)"; then
    continue
  fi
  if ! persistence_mode="$(extract_field persistence_mode "$file" 2>/dev/null)"; then
    continue
  fi
  if ! record_number="$(extract_field upload_record_count "$file" 2>/dev/null)"; then
    continue
  fi
  if ! split_migration_total_duration_ms="$(extract_field split_migration_total_duration_ms "$file" 2>/dev/null)"; then
    continue
  fi
  if ! split_migration_count="$(extract_field split_migration_count "$file" 2>/dev/null)"; then
    continue
  fi
  if ! payload_mb="$(extract_io_field payload_mb "$file" 2>/dev/null)"; then
    continue
  fi
  if ! io_total_mb="$(extract_io_field io_total_mb "$file" 2>/dev/null)"; then
    continue
  fi
  if ! io_amp_ratio="$(extract_io_field io_amp_ratio "$file" 2>/dev/null)"; then
    continue
  fi

  printf '%s,%s,%s,%s:%sms,%s,%smb,%smb,%s\n' \
    "$dataset" \
    "$adsmode" \
    "$persistence_mode" \
    "$record_number" \
    "$split_migration_total_duration_ms" \
    "$split_migration_count" \
    "$payload_mb" \
    "$io_total_mb" \
    "$io_amp_ratio" >> "$OUTPUT_FILE"

  written=$((written + 1))
done

if [[ "$written" -eq 0 ]]; then
  echo "No manager report files with required fields found under $INPUT_ROOT, skipping exp4-fig1 collection." >&2
  exit 0
fi

echo "Appended $written lines to $OUTPUT_FILE"
