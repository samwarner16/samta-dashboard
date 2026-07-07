#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

log() {
  printf '\033[1;33m[boot]\033[0m %s\n' "$1"
}

BOOT_USE_SERVICE_WRAPPERS="${BOOT_USE_SERVICE_WRAPPERS:-0}"
DASHBOARD_PORT="${DASHBOARD_PORT:-4173}"

kill_pid_file() {
  local pid_file="$1"
  if [ ! -f "${pid_file}" ]; then
    return 0
  fi

  local pid
  pid="$(cat "${pid_file}")"
  if [ -n "${pid}" ] && ps -p "${pid}" >/dev/null 2>&1; then
    kill "${pid}" >/dev/null 2>&1 || true
    wait "${pid}" 2>/dev/null || true
  fi
  rm -f "${pid_file}"
}

kill_port_listener() {
  local port="$1"
  local pids=""
  if command -v lsof >/dev/null 2>&1; then
    pids="$(lsof -iTCP:"${port}" -sTCP:LISTEN -n -P 2>/dev/null | tail -n +2 | awk '{print $2}' | sort -u || true)"
  elif command -v fuser >/dev/null 2>&1; then
    pids="$(fuser "${port}"/tcp 2>/dev/null || true)"
  fi

  if [ -n "${pids}" ]; then
    for pid in ${pids}; do
      if [ -n "${pid}" ] && ps -p "${pid}" >/dev/null 2>&1; then
        kill -9 "${pid}" >/dev/null 2>&1 || true
      fi
    done
  fi
}

cleanup_pid() {
  local pid_file="$1"
  local pid

  if [ -f "${pid_file}" ]; then
    pid="$(cat "${pid_file}")"
    if [ -n "${pid}" ] && ps -p "${pid}" >/dev/null 2>&1; then
      log "Stopping ${pid_file} (pid ${pid})"
      kill "${pid}" || true
      wait "${pid}" 2>/dev/null || true
    fi
    rm -f "${pid_file}"
  fi
}

cleanup_pid "${ROOT_DIR}/.dashboard-boot-api.pid"
cleanup_pid "${ROOT_DIR}/.dashboard-boot-worker.pid"
cleanup_pid "${ROOT_DIR}/.dashboard-boot-ui.pid"
cleanup_pid "${ROOT_DIR}/.dashboard-api-service.pid"
cleanup_pid "${ROOT_DIR}/.dashboard-worker-service.pid"
kill_port_listener 8080
kill_port_listener "${DASHBOARD_PORT}"

if [ "${BOOT_USE_SERVICE_WRAPPERS}" = "1" ]; then
  "${ROOT_DIR}/scripts/local-services.sh" stop >/dev/null 2>&1 || true
fi

if command -v docker-compose >/dev/null 2>&1; then
  docker compose down
else
  docker compose down
fi

if command -v pkill >/dev/null 2>&1; then
  pkill -f "target/debug/api" >/dev/null 2>&1 || true
  pkill -f "target/debug/worker" >/dev/null 2>&1 || true
  pkill -f "target/debug/rebuild-projections" >/dev/null 2>&1 || true
  pkill -f "python3 -m http.server 4173" >/dev/null 2>&1 || true
fi

log "All services stopped."
