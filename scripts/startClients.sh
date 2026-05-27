#!/usr/bin/env bash

set -euo pipefail

usage() {
  echo "Usage: $(basename "$0") [ads_mode] [set_proof_mode]"
  echo "  ads_mode          ADS mode: mpt|mest|acctrie|acctree (default: acctrie)"
  echo "  set_proof_mode    Set proof mode: polynomial|accumulator (default: polynomial)"
}

if [[ $# -gt 2 ]]; then
  usage
  exit 1
fi

ADS_MODE="${1:-acctrie}"
SET_PROOF_MODE="${2:-polynomial}"
ADS_MODE_SET=$([[ $# -ge 1 ]] && echo 1 || echo 0)
SET_PROOF_MODE_SET=$([[ $# -ge 2 ]] && echo 1 || echo 0)

case "$ADS_MODE" in
  mpt|mest|acctrie|acctree) ;;
  *)
    echo "Error: invalid ads_mode: $ADS_MODE" >&2
    usage
    exit 1
    ;;
esac

case "$SET_PROOF_MODE" in
  polynomial|accumulator) ;;
  *)
    echo "Error: invalid set_proof_mode: $SET_PROOF_MODE" >&2
    usage
    exit 1
    ;;
esac

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
source "$SCRIPT_DIR/lib/rebuild_if_needed.sh"
source "$SCRIPT_DIR/lib/manager_addr.sh"
LOG_DIR="$SCRIPT_DIR/logs"
CLIENT_STATE_DIR="$SCRIPT_DIR/data/clients"
MANAGER_ADDR_FILE="$SCRIPT_DIR/data/manageraddrs"
MANAGER_STATE_FILE="$SCRIPT_DIR/data/manager.state"
SESSION_STATE_FILE="$CLIENT_STATE_DIR/session.state"
CLIENT_BIN="$ROOT_DIR/target/release/client"
CLIENT_ID=1
export CLIENT_RPC_TIMEOUT_SECS="${CLIENT_RPC_TIMEOUT_SECS:-600}"
export ACCUMULATOR_PUBLIC_PARAMS_FILE="$SCRIPT_DIR/data/accumulator_public_params.bin"

if [[ "$ADS_MODE" == "acctree" || "$ADS_MODE" == "mpt" ]]; then
  export CLIENT_HEAVY_RPC_TIMEOUT_SECS="${CLIENT_HEAVY_RPC_TIMEOUT_SECS:-3600}"
  export CLIENT_HEAVY_CONNECT_TIMEOUT_SECS="${CLIENT_HEAVY_CONNECT_TIMEOUT_SECS:-30}"
  export CLIENT_HEAVY_TCP_KEEPALIVE_SECS="${CLIENT_HEAVY_TCP_KEEPALIVE_SECS:-300}"
else
  export CLIENT_RPC_TIMEOUT_SECS="${CLIENT_RPC_TIMEOUT_SECS:-600}"
  export CLIENT_CONNECT_TIMEOUT_SECS="${CLIENT_CONNECT_TIMEOUT_SECS:-10}"
  export CLIENT_TCP_KEEPALIVE_SECS="${CLIENT_TCP_KEEPALIVE_SECS:-30}"
fi

mkdir -p "$LOG_DIR"
mkdir -p "$CLIENT_STATE_DIR"

ensure_release_binary client "$CLIENT_BIN"

timestamp() {
  date -u +%Y%m%dT%H%M%SZ
}

RUN_COUNTER=0

load_session_state() {
  if [[ ! -f "$SESSION_STATE_FILE" ]]; then
    return
  fi

  while IFS='=' read -r key value; do
    case "$key" in
      ads_mode)
        if [[ "$ADS_MODE_SET" -eq 0 ]]; then
          ADS_MODE="$value"
        fi
        ;;
      set_proof_mode)
        if [[ "$SET_PROOF_MODE_SET" -eq 0 ]]; then
          SET_PROOF_MODE="$value"
        fi
        ;;
    esac
  done < "$SESSION_STATE_FILE"
}

load_manager_state() {
  if [[ ! -f "$MANAGER_STATE_FILE" ]]; then
    return
  fi

  while IFS='=' read -r key value; do
    case "$key" in
      ads_mode)
        MANAGER_STATE_ADS_MODE="$value"
        ;;
      set_proof_mode)
        MANAGER_STATE_SET_PROOF_MODE="$value"
        ;;
      manager_bind_addr)
        MANAGER_STATE_BIND_ADDR="$value"
        ;;
    esac
  done < "$MANAGER_STATE_FILE"
}

save_session_state() {
  mkdir -p "$CLIENT_STATE_DIR"
  {
    echo "ads_mode=$ADS_MODE"
    echo "set_proof_mode=$SET_PROOF_MODE"
    echo "saved_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  } > "$SESSION_STATE_FILE"
}

MANAGER_STATE_ADS_MODE=""
MANAGER_STATE_SET_PROOF_MODE=""
MANAGER_STATE_BIND_ADDR=""

parse_command() {
  python3 - "$1" <<'PY'
import shlex
import sys

try:
    tokens = shlex.split(sys.argv[1])
except ValueError as exc:
    print(f"ERROR:{exc}")
    sys.exit(0)

for token in tokens:
    print(token)
PY
}

run_client() {
  local kind="$1"
  local input_dir="$2"
  shift
  shift
  RUN_COUNTER=$((RUN_COUNTER + 1))
  local log_file="$LOG_DIR/client-${kind}-$(timestamp)-${RUN_COUNTER}.log"
  {
    echo "[$(timestamp)] command: $kind $*"
    echo "[$(timestamp)] manager: $MANAGER_ADDR"
    echo "[$(timestamp)] input_dir: $input_dir"
    echo "[$(timestamp)] ads_mode: $ADS_MODE"
    echo "[$(timestamp)] set_proof_mode: $SET_PROOF_MODE"
  } > "$log_file"

  set +e
  coproc PROGRESS_TAIL { tail -n0 -F "$log_file" 2>/dev/null; }
  local tail_fd="${PROGRESS_TAIL[0]}"
  local tail_pid="$PROGRESS_TAIL_PID"

  {
    echo "[$(timestamp)] client started"
    "$CLIENT_BIN" --manager-addr "$MANAGER_ADDR" --ads-mode "$ADS_MODE" --set-proof-mode "$SET_PROOF_MODE" --client-id "$CLIENT_ID" --input-dir "$input_dir" "$@"
  } >> "$log_file" 2>&1 &
  local client_pid=$!

  while kill -0 "$client_pid" 2>/dev/null; do
    if IFS= read -r -t 1 line <&"$tail_fd"; then
      case "$line" in
        *" progress: "*) printf '%s\n' "$line" ;;
      esac
    fi
  done

  while IFS= read -r -t 0.1 line <&"$tail_fd"; do
    case "$line" in
      *" progress: "*) printf '%s\n' "$line" ;;
    esac
  done

  wait "$client_pid"
  local status=$?

  kill "$tail_pid" >/dev/null 2>&1 || true
  wait "$tail_pid" >/dev/null 2>&1 || true
  set -e
  return "$status"
}

print_help() {
  cat <<'EOF'
Commands:
  upload <records_location> [count]
  query <workload_location> [count]
  update <updates_location> [count]
  reset
  clear
  offline
  help
  exit | quit

Update file format:
  fid,old_keyword,new_keyword
  or fid|old_keyword1,old_keyword2|new_keyword1,new_keyword2
EOF
}

is_positive_integer() {
  [[ "${1:-}" =~ ^[0-9]+$ ]] && [[ "$1" -gt 0 ]]
}

make_input_subset() {
  local source_file="$1"
  local limit="$2"
  local subset_file
  subset_file="$(mktemp "$LOG_DIR/input-subset-XXXXXX.txt")"

  if awk -v limit="$limit" '
    function trim(s) {
      sub(/^[[:space:]]+/, "", s)
      sub(/[[:space:]]+$/, "", s)
      return s
    }

    BEGIN {
      count = 0
    }

    {
      line = $0
      sub(/^\xef\xbb\xbf/, "", line)
      line = trim(line)
      if (line == "" || line ~ /^#/) {
        next
      }

      print line
      count++
      if (count >= limit) {
        exit 0
      }
    }

    END {
      if (count < limit) {
        exit 2
      }
    }
  ' "$source_file" > "$subset_file"; then
    printf '%s\n' "$subset_file"
    return 0
  fi

  local status=$?
  rm -f "$subset_file"
  if [[ "$status" -eq 2 ]]; then
    echo "Error: input file only has fewer than $limit valid lines: $source_file" >&2
  else
    echo "Error: failed to prepare input subset from $source_file" >&2
  fi
  return 1
}

echo "Interactive client shell"
load_session_state
load_manager_state

if [[ -n "$MANAGER_STATE_ADS_MODE" && "$ADS_MODE_SET" -eq 0 ]]; then
  ADS_MODE="$MANAGER_STATE_ADS_MODE"
fi

if [[ -n "$MANAGER_STATE_SET_PROOF_MODE" && "$SET_PROOF_MODE_SET" -eq 0 ]]; then
  SET_PROOF_MODE="$MANAGER_STATE_SET_PROOF_MODE"
fi

if MANAGER_ADDR_FROM_FILE="$(read_manager_addr_file "$MANAGER_ADDR_FILE")"; then
  MANAGER_ADDR="$(manager_http_addr "$MANAGER_ADDR_FROM_FILE")"
else
  MANAGER_ADDR="http://127.0.0.1:50051"
fi

if [[ -n "$MANAGER_STATE_ADS_MODE" && "$MANAGER_STATE_ADS_MODE" != "$ADS_MODE" ]]; then
  echo "Error: client ads_mode ($ADS_MODE) does not match manager ads_mode ($MANAGER_STATE_ADS_MODE)" >&2
  echo "       Restart manager/storagers with the same ads_mode, or start clients with matching args." >&2
  exit 1
fi

if [[ -n "$MANAGER_STATE_SET_PROOF_MODE" && "$MANAGER_STATE_SET_PROOF_MODE" != "$SET_PROOF_MODE" ]]; then
  echo "Error: client set_proof_mode ($SET_PROOF_MODE) does not match manager set_proof_mode ($MANAGER_STATE_SET_PROOF_MODE)" >&2
  echo "       Restart manager/storagers with the same set_proof_mode, or start clients with matching args." >&2
  exit 1
fi

save_session_state
echo "  manager=$MANAGER_ADDR"
echo "  ads_mode=$ADS_MODE"
echo "  set_proof_mode=$SET_PROOF_MODE"
print_help

while true; do
  printf 'client> '
  if ! IFS= read -r line; then
    echo
    break
  fi

  line="$(printf '%s' "$line" | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')"
  [[ -z "$line" ]] && continue

  mapfile -t args < <(parse_command "$line")
  if (( ${#args[@]} == 0 )); then
    continue
  fi
  if [[ "${args[0]}" == ERROR:* ]]; then
    echo "语法错误"
    continue
  fi

  cmd="${args[0]}"
  case "$cmd" in
    help)
      print_help
      ;;
    offline)
      save_session_state
      echo "offline"
      break
      ;;
    exit|quit)
      save_session_state
      break
      ;;
    upload)
      if [[ ${#args[@]} -ne 2 && ${#args[@]} -ne 3 ]]; then
        echo "用法: upload <records_location> [count]"
        continue
      fi
      upload_file="${args[1]}"
      upload_input_dir="$(dirname "$upload_file")"
      if [[ ${#args[@]} -eq 3 ]]; then
        if ! is_positive_integer "${args[2]}"; then
          echo "用法: upload <records_location> [count]"
          echo "count 必须是正整数"
          continue
        fi

        if limited_upload_file="$(make_input_subset "$upload_file" "${args[2]}")"; then
          if run_client upload "$upload_input_dir" --mode upload --report-count "${args[2]}" --records-file "$limited_upload_file"; then
            echo "upload ok"
          else
            echo "upload failed; see scripts/logs/"
          fi
          rm -f "$limited_upload_file"
        else
          continue
        fi
      else
        if run_client upload "$upload_input_dir" --mode upload --records-file "$upload_file"; then
          echo "upload ok"
        else
          echo "upload failed; see scripts/logs/"
        fi
      fi
      ;;
    query)
      if [[ ${#args[@]} -ne 2 && ${#args[@]} -ne 3 ]]; then
        echo "用法: query <workload_location> [count]"
        continue
      fi
      query_file="${args[1]}"
      query_input_dir="$(dirname "$query_file")"
      if [[ ${#args[@]} -eq 3 ]]; then
        if ! is_positive_integer "${args[2]}"; then
          echo "用法: query <workload_location> [count]"
          echo "count 必须是正整数"
          continue
        fi

        if limited_query_file="$(make_input_subset "$query_file" "${args[2]}")"; then
          if run_client query "$query_input_dir" --mode query --report-count "${args[2]}" --query-file "$limited_query_file"; then
            echo "query ok"
          else
            echo "query failed; see scripts/logs/"
          fi
          rm -f "$limited_query_file"
        else
          continue
        fi
      else
        if run_client query "$query_input_dir" --mode query --query-file "$query_file"; then
          echo "query ok"
        else
          echo "query failed; see scripts/logs/"
        fi
      fi
      ;;
    update)
      if [[ ${#args[@]} -ne 2 && ${#args[@]} -ne 3 ]]; then
        echo "用法: update <updates_location> [count]"
        continue
      fi
      update_file="${args[1]}"
      update_input_dir="$(dirname "$update_file")"
      if [[ ${#args[@]} -eq 3 ]]; then
        if ! is_positive_integer "${args[2]}"; then
          echo "用法: update <updates_location> [count]"
          echo "count 必须是正整数"
          continue
        fi

        if limited_update_file="$(make_input_subset "$update_file" "${args[2]}")"; then
          if run_client update "$update_input_dir" --mode update --report-count "${args[2]}" --update-file "$limited_update_file"; then
            echo "update ok"
          else
            echo "update failed; see scripts/logs/"
          fi
          rm -f "$limited_update_file"
        else
          continue
        fi
      else
        if run_client update "$update_input_dir" --mode update --update-file "$update_file"; then
          echo "update ok"
        else
          echo "update failed; see scripts/logs/"
        fi
      fi
      ;;
    reset|clear)
      if run_client reset "$CLIENT_STATE_DIR" --mode reset; then
        echo "reset ok"
      else
        echo "reset failed; see scripts/logs/"
      fi
      ;;
    *)
      echo "未知命令: $cmd"
      ;;
  esac
done
