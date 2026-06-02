#!/usr/bin/env bash

set -euo pipefail

if [[ $# -ne 0 ]]; then
  echo "Usage: $(basename "$0")" >&2
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INPUT_DIR="$SCRIPT_DIR/expdata"

if [[ ! -d "$INPUT_DIR" ]]; then
  echo "Error: missing directory: $INPUT_DIR" >&2
  exit 1
fi

mapfile -d '' -t report_files < <(find "$INPUT_DIR" -maxdepth 1 -type f -name '*.txt' -print0 | sort -z)

if [[ ${#report_files[@]} -eq 0 ]]; then
  echo "No txt files found under $INPUT_DIR" >&2
  exit 0
fi

deduped_count=0
for file in "${report_files[@]}"; do
  tmp_file="$(mktemp "${file}.XXXXXX")"
  awk '!seen[$0]++' "$file" > "$tmp_file"
  mv "$tmp_file" "$file"
  deduped_count=$((deduped_count + 1))
done

echo "Deduplicated $deduped_count file(s) under $INPUT_DIR"
