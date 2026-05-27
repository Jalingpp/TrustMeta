#!/usr/bin/env bash

if [[ -z "${BASH_VERSION:-}" ]]; then
  echo "Error: manager_addr.sh must be sourced from bash" >&2
  return 1 2>/dev/null || exit 1
fi

read_manager_addr_file() {
  local file="$1"
  local line

  if [[ ! -f "$file" ]]; then
    return 1
  fi

  while IFS= read -r line || [[ -n "$line" ]]; do
    [[ -z "$line" ]] && continue
    [[ "$line" =~ ^[[:space:]]*# ]] && continue

    line="${line#"${line%%[![:space:]]*}"}"
    line="${line%"${line##*[![:space:]]}"}"
    line="${line#http://}"
    line="${line#https://}"
    printf '%s\n' "$line"
    return 0
  done < "$file"

  return 1
}

manager_http_addr() {
  local addr="$1"

  if [[ "$addr" == http://* || "$addr" == https://* ]]; then
    printf '%s\n' "$addr"
  else
    printf 'http://%s\n' "$addr"
  fi
}
