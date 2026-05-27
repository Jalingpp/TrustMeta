#!/usr/bin/env bash

if [[ -z "${BASH_VERSION:-}" ]]; then
  echo "Error: rebuild_if_needed.sh must be sourced from bash" >&2
  return 1 2>/dev/null || exit 1
fi

REBUILD_HELPER_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REBUILD_HELPER_ROOT="$(cd "$REBUILD_HELPER_DIR/../.." && pwd)"

workspace_sources_changed() {
  local binary="$1"

  [[ ! -e "$binary" ]] && return 0

  for manifest in "$REBUILD_HELPER_ROOT/Cargo.toml" "$REBUILD_HELPER_ROOT/Cargo.lock"; do
    if [[ -f "$manifest" && "$manifest" -nt "$binary" ]]; then
      return 0
    fi
  done

  if [[ -d "$REBUILD_HELPER_ROOT/crates" ]]; then
    local newer
    newer="$(find "$REBUILD_HELPER_ROOT/crates" -type f -newer "$binary" -print -quit 2>/dev/null || true)"
    [[ -n "$newer" ]] && return 0
  fi

  return 1
}

ensure_release_binary() {
  local package="$1"
  local binary="$2"
  local cargo_bin="${CARGO_BIN:-}"

  if [[ -x "$binary" ]]; then
    if ! workspace_sources_changed "$binary"; then
      return 0
    fi
  fi

  if [[ -z "$cargo_bin" ]]; then
    cargo_bin="$(command -v cargo || true)"
  fi

  if [[ -z "$cargo_bin" ]]; then
    if [[ -x "$binary" ]]; then
      echo "Error: $package binary is stale, but cargo is not available for rebuild" >&2
    else
      echo "Error: $package binary is missing and cargo is not available" >&2
    fi
    return 1
  fi

  echo "Rebuilding $package"
  (cd "$REBUILD_HELPER_ROOT" && "$cargo_bin" build --release -p "$package")

  if [[ ! -x "$binary" ]]; then
    echo "Error: expected release binary not found after build: $binary" >&2
    return 1
  fi
}
