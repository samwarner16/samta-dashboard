#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

HARNESS_PID_FILE="${ROOT_DIR}/.agent-harness.pids"
RUN_WORKER_SCRIPT="${ROOT_DIR}/scripts/service/run-worker.sh"
ACTION="${1:-start}"
REQUESTED_COUNT="${2:-}"
DEFAULT_COUNT="${WORKER_POOL_SIZE:-2}"
WORKER_IDS=()

log() {
  printf '[harness] %s\n' "$1"
}

warn() {
  printf '[harness] %s\n' "$1" >&2
}

require_cmd() {
  local name="$1"
  if ! command -v "${name}" >/dev/null 2>&1; then
    echo "Missing required command: ${name}" >&2
    exit 1
  fi
}

require_cmd bash

if [ ! -f "${RUN_WORKER_SCRIPT}" ]; then
  echo "Missing worker runner script: ${RUN_WORKER_SCRIPT}" >&2
  exit 1
fi

is_valid_uuid() {
  local value="$1"
  [[ "${value}" =~ ^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[1-5][0-9a-fA-F]{3}-[89abAB][0-9a-fA-F]{3}-[0-9a-Fa-f]{12}$ ]]
}

normalize_uuid() {
  local value="$1"
  printf '%s' "$(echo "${value}" | tr '[:upper:]' '[:lower:]')"
}

agent_id_from_seed() {
  local seed="$1"
  local idx="$2"

  if command -v python3 >/dev/null 2>&1; then
    python3 - "$seed" "$idx" <<'PY'
import uuid
import sys

seed = sys.argv[1]
idx = sys.argv[2]
print(uuid.uuid5(uuid.NAMESPACE_DNS, f"{seed}:{idx}"))
PY
    return
  fi

  if command -v uuidgen >/dev/null 2>&1; then
    uuidgen
    return
  fi

  date +%s%N | sha256sum | awk '{print $1}' | cut -c1-32 | sed 's/\(........\)\(....\)\(....\)\(....\)\(............\)/\1-\2-\3-\4-\5/'
}

resolve_worker_ids() {
  local count="$1"
  local explicit_env="${WORKER_AGENT_IDS:-}"
  WORKER_IDS=()

  if [ -n "${explicit_env}" ]; then
    local -a parsed_ids
    IFS=',' read -r -a parsed_ids <<< "${explicit_env}"
    for candidate in "${parsed_ids[@]}"; do
      candidate="$(echo "${candidate}" | tr -d '[:space:]')"
      if [ -z "${candidate}" ]; then
        continue
      fi
      if is_valid_uuid "${candidate}"; then
        WORKER_IDS+=("$(normalize_uuid "${candidate}")")
      else
        warn "Ignoring invalid WORKER_AGENT_IDS entry: ${candidate}"
      fi
    done
  fi

  if [ "${#WORKER_IDS[@]}" -gt 0 ]; then
    if [ "${count}" -lt "${#WORKER_IDS[@]}" ]; then
      WORKER_IDS=("${WORKER_IDS[@]:0:${count}}")
    elif [ "${count}" -gt "${#WORKER_IDS[@]}" ]; then
      warn "Requested ${count} workers but only ${#WORKER_IDS[@]} ids in WORKER_AGENT_IDS; using fallback ids for extras."
    fi
  elif [ "${count}" -eq 1 ] && [ -n "${WORKER_AGENT_ID:-}" ] && is_valid_uuid "${WORKER_AGENT_ID}"; then
    WORKER_IDS=("${WORKER_AGENT_ID}")
  fi

  if [ "${#WORKER_IDS[@]}" -lt "${count}" ]; then
    local prefix="${WORKER_AGENT_ID_PREFIX:-harness-worker}"
    local next_index=1
    while [ "${#WORKER_IDS[@]}" -lt "${count}" ]; do
      WORKER_IDS+=("$(normalize_uuid "$(agent_id_from_seed "${prefix}" "${next_index}")")")
      next_index=$((next_index + 1))
    done
  fi

  if [ "${#WORKER_IDS[@]}" -gt "${count}" ]; then
    WORKER_IDS=("${WORKER_IDS[@]:0:${count}}")
  fi
}

start_worker() {
  local idx="$1"
  local worker_id="$2"
  local log_file="$3"
  local item_affinity="${WORKER_ITEM_AFFINITY:-0}"

  WORKER_AGENT_ID="${worker_id}" \
  WORKER_ITEM_AFFINITY="${item_affinity}" \
  WORKER_POLL_MS="${WORKER_POLL_MS:-1500}" \
  WORKER_ITEMS_PER_LOOP="${WORKER_ITEMS_PER_LOOP:-1}" \
  WORKER_EFFORT_PER_ITEM="${WORKER_EFFORT_PER_ITEM:-5}" \
  WORKER_COST_PER_ITEM="${WORKER_COST_PER_ITEM:-0.25}" \
  WORKER_DB_CONNECT_ATTEMPTS="${WORKER_DB_CONNECT_ATTEMPTS:-24}" \
  WORKER_DB_CONNECT_BACKOFF_MS="${WORKER_DB_CONNECT_BACKOFF_MS:-500}" \
  WORKER_DB_SCHEMA_ATTEMPTS="${WORKER_DB_SCHEMA_ATTEMPTS:-24}" \
  WORKER_DB_SCHEMA_BACKOFF_MS="${WORKER_DB_SCHEMA_BACKOFF_MS:-500}" \
  WORKER_APPEND_ATTEMPTS="${WORKER_APPEND_ATTEMPTS:-4}" \
  WORKER_APPEND_BACKOFF_MS="${WORKER_APPEND_BACKOFF_MS:-500}" \
  nohup "${RUN_WORKER_SCRIPT}" > "${log_file}" 2>&1 &

  printf '%s|%s|%s\n' "$!" "${worker_id}" "${log_file}" >> "${HARNESS_PID_FILE}"
}

stop_harness() {
  if [ ! -f "${HARNESS_PID_FILE}" ]; then
    echo "No agent harness pids found."
    return 0
  fi

  while IFS='|' read -r pid agent_id log_file; do
    [ -z "${pid}" ] && continue
    if kill -0 "${pid}" >/dev/null 2>&1; then
      kill "${pid}" || true
      wait "${pid}" 2>/dev/null || true
      log "Stopped worker pid=${pid} agent=${agent_id}"
    else
      log "Worker pid=${pid} agent=${agent_id} already exited"
    fi
  done < "${HARNESS_PID_FILE}"

  rm -f "${HARNESS_PID_FILE}"
}

status_harness() {
  if [ ! -f "${HARNESS_PID_FILE}" ]; then
    echo "No harness worker manifest found."
    return 0
  fi

  local manifest="${HARNESS_PID_FILE}"
  local has_running=false

  echo "Harness workers:"
  while IFS='|' read -r pid agent_id log_file; do
    [ -z "${pid}" ] && continue
    if ps -p "${pid}" >/dev/null 2>&1; then
      echo "  ${pid} agent=${agent_id} ${log_file} (running)"
      has_running=true
    else
      echo "  ${pid} agent=${agent_id} ${log_file} (stopped)"
    fi
  done < "${manifest}"

  if [ "${has_running}" = "false" ]; then
    echo "  no active harness workers"
  fi
}

show_usage() {
  cat <<'USAGE'
Usage: ./scripts/run-agent-harness.sh [start|up|run|stop|status|logs] [worker_count]

Environment variables:
  WORKER_POOL_SIZE       Default worker count when positional count is omitted (default: 2)
  WORKER_AGENT_IDS       Comma-separated explicit IDs (overrides count/seed for prefix)
  WORKER_AGENT_ID        Explicit ID for single-worker start mode
  WORKER_AGENT_ID_PREFIX Deterministic worker-id seed when WORKER_AGENT_IDS not provided
  WORKER_ITEMS_PER_LOOP  Items processed per worker loop
  WORKER_POLL_MS         Poll interval for each worker
  WORKER_ITEM_AFFINITY   Set to 1 so workers only process items assigned to their id
  WORKER_EFFORT_PER_ITEM Simulated effort per completed item
  WORKER_COST_PER_ITEM   Simulated cost per completed item
  WORKER_DB_CONNECT_ATTEMPTS / BACKOFF_MS
  WORKER_DB_SCHEMA_ATTEMPTS / BACKOFF_MS
  WORKER_APPEND_ATTEMPTS / BACKOFF_MS
USAGE
}

ensure_count() {
  local value="$1"
  if [ -z "${value}" ]; then
    value="${DEFAULT_COUNT}"
  elif [ "${value}" = "0" ]; then
    value="${DEFAULT_COUNT}"
  fi

  if [[ ! "${value}" =~ ^[0-9]+$ ]] || [ "${value}" -lt 1 ]; then
    echo "Worker count must be a positive integer." >&2
    exit 1
  fi

  echo "${value}"
}

case "${ACTION}" in
  start|up|run)
    COUNT="$(ensure_count "${REQUESTED_COUNT}")"
    resolve_worker_ids "${COUNT}"

    if [ -f "${HARNESS_PID_FILE}" ]; then
      stop_harness
    fi
    : > "${HARNESS_PID_FILE}"

    for idx in $(seq 0 $((COUNT - 1))); do
      worker_id="${WORKER_IDS[$idx]}"
      log_file="${ROOT_DIR}/.agent-harness-worker-$(printf '%02d' "$((idx + 1))").log"
      log "Starting worker #$((idx + 1))/${COUNT} with agent_id=${worker_id} (log=${log_file})"
      start_worker "$((idx + 1))" "${worker_id}" "${log_file}"
    done

    echo "Started ${COUNT} harness worker(s)."
    echo "Manifest: ${HARNESS_PID_FILE}"
    ;;
  stop)
    stop_harness
    echo "Stopped agent harness workers."
    ;;
  status)
    status_harness
    ;;
  logs)
    if [ -f "${HARNESS_PID_FILE}" ]; then
      while IFS='|' read -r pid agent_id log_file; do
        [ -z "${pid}" ] && continue
        echo "=== ${log_file} [agent=${agent_id} pid=${pid}] ==="
        tail -n 80 "${log_file}" 2>/dev/null || true
      done < "${HARNESS_PID_FILE}"
    else
      echo "No harness metadata file found."
    fi
    ;;
  help|-h|--help)
    show_usage
    ;;
  *)
    show_usage
    exit 1
    ;;
esac
