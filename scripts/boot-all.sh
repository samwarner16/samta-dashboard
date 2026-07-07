#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

DEFAULT_API_URL="http://127.0.0.1:8080"
DEFAULT_DASHBOARD_PORT="4173"
DEFAULT_DASHBOARD_API_BASE="${DASHBOARD_API_BASE:-$DEFAULT_API_URL}"
DASHBOARD_API_BASE="${DASHBOARD_API_BASE:-$DEFAULT_DASHBOARD_API_BASE}"
DASHBOARD_PORT="${DASHBOARD_PORT:-$DEFAULT_DASHBOARD_PORT}"
BOOT_CLEAN_SLATE="${BOOT_CLEAN_SLATE:-1}"
BOOT_USE_SERVICE_WRAPPERS="${BOOT_USE_SERVICE_WRAPPERS:-0}"
BOOT_WAIT_SECONDS="${BOOT_WAIT_SECONDS:-180}"
BOOT_DB_WAIT_SECONDS="${BOOT_DB_WAIT_SECONDS:-60}"
BOOT_SERVICE_MODE="${BOOT_SERVICE_MODE:-launchd}"
BOOT_API_SERVICE_PID="${ROOT_DIR}/.dashboard-api-service.pid"
BOOT_WORKER_SERVICE_PID="${ROOT_DIR}/.dashboard-worker-service.pid"

info() {
  printf '\033[1;34m[boot]\033[0m %s\n' "$1"
}

warn() {
  printf '\033[1;33m[boot]\033[0m %s\n' "$1"
}

run_log() {
  printf '\033[1;32m[boot]\033[0m %s\n' "$1"
}

require_cmd() {
  local name="$1"
  if ! command -v "$name" >/dev/null 2>&1; then
    warn "Missing required command: $name"
    return 1
  fi
}

is_process_running() {
  local pid_file="$1"
  if [ ! -f "$pid_file" ]; then
    return 1
  fi

  local pid
  pid="$(cat "${pid_file}")"
  [ -n "$pid" ] && ps -p "${pid}" >/dev/null 2>&1
}

ensure_docker_available() {
  if ! docker info >/dev/null 2>&1; then
    warn "Docker daemon is not reachable. Start Docker Desktop and retry."
    return 1
  fi
}

kill_pid_file() {
  local pid_file="$1"
  if [ ! -f "${pid_file}" ]; then
    return 0
  fi

  local pid
  pid="$(cat "${pid_file}")"
  if [ -n "${pid}" ] && ps -p "${pid}" >/dev/null 2>&1; then
    kill "${pid}" >/dev/null 2>&1 || true
    sleep 0.2
    if ps -p "${pid}" >/dev/null 2>&1; then
      kill -9 "${pid}" >/dev/null 2>&1 || true
      wait "${pid}" 2>/dev/null || true
    fi
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

wait_for_postgres() {
  local timeout_seconds="$1"
  local user="${POSTGRES_USER:-user}"
  local db="${POSTGRES_DB:-agents_db}"
  local deadline=$((SECONDS + timeout_seconds))

  while ! docker compose exec -T postgres psql -U "${user}" -d "${db}" -c "select 1" >/dev/null 2>&1; do
    if [ "$SECONDS" -ge "$deadline" ]; then
      warn "Postgres did not become ready at ${user}@${db} within ${timeout_seconds}s"
      return 1
    fi
    sleep 1
  done
}

boot_api_log() {
  if [ "${BOOT_USE_SERVICE_WRAPPERS}" = "1" ]; then
    echo "${ROOT_DIR}/.dashboard-api-service.log"
  else
    echo "${ROOT_DIR}/.dashboard-boot-api.log"
  fi
}

boot_worker_log() {
  if [ "${BOOT_USE_SERVICE_WRAPPERS}" = "1" ]; then
    echo "${ROOT_DIR}/.dashboard-worker-service.log"
  else
    echo "${ROOT_DIR}/.dashboard-boot-worker.log"
  fi
}

wait_for_http() {
  local url="$1"
  local timeout_seconds="$2"
  local pid_file="${3:-}"
  local deadline=$((SECONDS + timeout_seconds))

  while ! curl -fsS "$url" >/dev/null 2>&1; do
    if [ -n "${pid_file}" ] && [ ! -f "${pid_file}" ]; then
      warn "Expected process file not found: ${pid_file}"
      return 2
    fi

    if [ -n "${pid_file}" ] && ! is_process_running "${pid_file}"; then
      warn "Process from ${pid_file} exited before ${url} became healthy"
      return 2
    fi

    if [ "$SECONDS" -ge "$deadline" ]; then
      warn "timed out waiting for ${url}"
      return 1
    fi
    sleep 1
  done
}

cleanup_processes() {
  info "Cleaning up old local processes..."
  if [ "${BOOT_USE_SERVICE_WRAPPERS}" = "1" ]; then
    "${ROOT_DIR}/scripts/local-services.sh" stop >/dev/null 2>&1 || true
  fi

  kill_pid_file "${ROOT_DIR}/.dashboard-boot-api.pid"
  kill_pid_file "${ROOT_DIR}/.dashboard-boot-worker.pid"
  kill_pid_file "${ROOT_DIR}/.dashboard-boot-ui.pid"
  kill_pid_file "${BOOT_API_SERVICE_PID}"
  kill_pid_file "${BOOT_WORKER_SERVICE_PID}"

  kill_port_listener 8080
  kill_port_listener "${DASHBOARD_PORT}"
  kill_port_listener 6379

  pkill -f "target/debug/api" >/dev/null 2>&1 || true
  pkill -f "target/debug/worker" >/dev/null 2>&1 || true
  pkill -f "target/debug/rebuild-projections" >/dev/null 2>&1 || true
  pkill -f "python3 -m http.server ${DASHBOARD_PORT}" >/dev/null 2>&1 || true
}

start_services() {
  ensure_docker_available
  if [ "${BOOT_CLEAN_SLATE}" = "1" ]; then
    warn "BOOT_CLEAN_SLATE enabled: tearing down existing containers/volumes for a fresh DB state."
    docker compose down -v --remove-orphans
  fi

  info "Starting docker stack (postgres + redis)..."
  docker compose up -d postgres redis
  if ! wait_for_postgres "${BOOT_DB_WAIT_SECONDS}"; then
    return 1
  fi
}

start_api() {
  if [ "${BOOT_USE_SERVICE_WRAPPERS}" = "1" ]; then
    if [ "${BOOT_SERVICE_MODE}" = "direct" ]; then
      info "Starting API via direct fallback service mode"
      start_api_direct
      return
    fi

    info "Starting API via local service wrapper (${BOOT_SERVICE_MODE})"
    "${ROOT_DIR}/scripts/local-services.sh" install api >/dev/null || true
    "${ROOT_DIR}/scripts/local-services.sh" start api
    return
  fi

  local logfile="${ROOT_DIR}/.dashboard-boot-api.log"
  info "Starting API (log: ${logfile})"
  DATABASE_URL="${DATABASE_URL:-postgres://user:pass@127.0.0.1:5432/agents_db}" \
    API_DB_CONNECT_ATTEMPTS="${API_DB_CONNECT_ATTEMPTS:-32}" \
    API_DB_CONNECT_BACKOFF_MS="${API_DB_CONNECT_BACKOFF_MS:-500}" \
    API_DB_SCHEMA_ATTEMPTS="${API_DB_SCHEMA_ATTEMPTS:-24}" \
    API_DB_SCHEMA_BACKOFF_MS="${API_DB_SCHEMA_BACKOFF_MS:-500}" \
    API_HOST="${API_HOST:-0.0.0.0}" \
    API_PORT="${API_PORT:-8080}" \
    nohup cargo run -p api >"${logfile}" 2>&1 &
  echo $! > "${ROOT_DIR}/.dashboard-boot-api.pid"
}

start_worker() {
  if [ "${BOOT_USE_SERVICE_WRAPPERS}" = "1" ]; then
    if [ "${BOOT_SERVICE_MODE}" = "direct" ]; then
      info "Starting worker via direct fallback service mode"
      start_worker_direct
      return
    fi

    info "Starting worker via local service wrapper (${BOOT_SERVICE_MODE})"
    "${ROOT_DIR}/scripts/local-services.sh" install worker >/dev/null || true
    "${ROOT_DIR}/scripts/local-services.sh" start worker
    return
  fi

  local logfile="${ROOT_DIR}/.dashboard-boot-worker.log"
  info "Starting worker (log: ${logfile})"
  DATABASE_URL="${DATABASE_URL:-postgres://user:pass@127.0.0.1:5432/agents_db}" \
    WORKER_DB_CONNECT_ATTEMPTS="${WORKER_DB_CONNECT_ATTEMPTS:-24}" \
    WORKER_DB_CONNECT_BACKOFF_MS="${WORKER_DB_CONNECT_BACKOFF_MS:-500}" \
    WORKER_DB_SCHEMA_ATTEMPTS="${WORKER_DB_SCHEMA_ATTEMPTS:-24}" \
    WORKER_DB_SCHEMA_BACKOFF_MS="${WORKER_DB_SCHEMA_BACKOFF_MS:-500}" \
    nohup cargo run -p application --bin worker >"${logfile}" 2>&1 &
  echo $! > "${ROOT_DIR}/.dashboard-boot-worker.pid"
}

start_api_direct() {
  local logfile="${ROOT_DIR}/.dashboard-api-service.log"
  local pid_file="${BOOT_API_SERVICE_PID}"

  info "Starting API via run script (direct)"
  DATABASE_URL="${DATABASE_URL:-postgres://user:pass@127.0.0.1:5432/agents_db}" \
    API_DB_CONNECT_ATTEMPTS="${API_DB_CONNECT_ATTEMPTS:-32}" \
    API_DB_CONNECT_BACKOFF_MS="${API_DB_CONNECT_BACKOFF_MS:-500}" \
    API_DB_SCHEMA_ATTEMPTS="${API_DB_SCHEMA_ATTEMPTS:-24}" \
    API_DB_SCHEMA_BACKOFF_MS="${API_DB_SCHEMA_BACKOFF_MS:-500}" \
    API_HOST="${API_HOST:-0.0.0.0}" \
    API_PORT="${API_PORT:-8080}" \
    nohup "${ROOT_DIR}/scripts/service/run-api.sh" >"${logfile}" 2>&1 &
  echo "$!" > "${pid_file}"
}

start_worker_direct() {
  local logfile="${ROOT_DIR}/.dashboard-worker-service.log"
  local pid_file="${BOOT_WORKER_SERVICE_PID}"

  info "Starting worker via run script (direct)"
  DATABASE_URL="${DATABASE_URL:-postgres://user:pass@127.0.0.1:5432/agents_db}" \
    WORKER_DB_CONNECT_ATTEMPTS="${WORKER_DB_CONNECT_ATTEMPTS:-24}" \
    WORKER_DB_CONNECT_BACKOFF_MS="${WORKER_DB_CONNECT_BACKOFF_MS:-500}" \
    WORKER_DB_SCHEMA_ATTEMPTS="${WORKER_DB_SCHEMA_ATTEMPTS:-24}" \
    WORKER_DB_SCHEMA_BACKOFF_MS="${WORKER_DB_SCHEMA_BACKOFF_MS:-500}" \
    nohup "${ROOT_DIR}/scripts/service/run-worker.sh" >"${logfile}" 2>&1 &
  echo "$!" > "${pid_file}"
}

start_dashboard() {
  local logfile="${ROOT_DIR}/.dashboard-boot-ui.log"
  info "Starting dashboard on port ${DASHBOARD_PORT} (log: ${logfile})"
  DASHBOARD_API_BASE="${DASHBOARD_API_BASE}" DASHBOARD_PORT="${DASHBOARD_PORT}" \
    nohup scripts/start-dashboard.sh >"${logfile}" 2>&1 &
  echo $! > "${ROOT_DIR}/.dashboard-boot-ui.pid"
}

wait_for_stack() {
  local api_url="http://127.0.0.1:${API_PORT:-8080}"
  local api_log
  local ui_log="${ROOT_DIR}/.dashboard-boot-ui.log"
  local api_pid_file="${ROOT_DIR}/.dashboard-boot-api.pid"

  if [ "${BOOT_USE_SERVICE_WRAPPERS}" = "1" ]; then
    api_pid_file="${BOOT_API_SERVICE_PID}"
  fi

  info "Waiting for API health (${api_url}/health)"
  if ! wait_for_http "${api_url}/health" "${BOOT_WAIT_SECONDS}" "${api_pid_file}"; then
    api_log="$(boot_api_log)"
    warn "API health check failed. tail ${api_log} for details."
    return 1
  fi

  if [ ! -f "${ui_log}" ]; then
    ui_log="${ROOT_DIR}/.dashboard-ui.log"
  fi
  info "Waiting for dashboard page (http://127.0.0.1:${DASHBOARD_PORT})"
  if ! wait_for_http "http://127.0.0.1:${DASHBOARD_PORT}" "${BOOT_WAIT_SECONDS}"; then
    warn "Dashboard startup check failed. tail ${ui_log} for details."
    return 1
  fi
}

main() {
  require_cmd docker
  require_cmd cargo
  require_cmd python3

  info "Booting full stack..."
  cleanup_processes
  start_services
  start_api
  start_worker
  start_dashboard

  if wait_for_stack; then
    status=0
  else
    status=1
  fi

  if [ "$status" -ne 0 ] && [ "${BOOT_USE_SERVICE_WRAPPERS}" = "1" ] && [ "${BOOT_SERVICE_MODE}" != "direct" ]; then
    warn "Wrapper-mode bootstrap did not start API/worker reliably. Falling back to direct service mode."
    BOOT_SERVICE_MODE="direct"
    cleanup_processes
    sleep 1
    start_api
    start_worker
    start_dashboard
    if wait_for_stack; then
      status=0
    else
      status=1
    fi
  fi

  if [ "$status" -ne 0 ]; then
    warn "Stack startup did not complete."
    exit 1
  fi

  run_log "Startup sequence issued."
  run_log "API:          http://127.0.0.1:${API_PORT:-8080}"
  run_log "Dashboard:    http://127.0.0.1:${DASHBOARD_PORT}/?api=${DASHBOARD_API_BASE}"
  run_log "Logs:"
  run_log "  API log:      $(boot_api_log)"
  run_log "  Worker log:   $(boot_worker_log)"
  run_log "  UI log:      ${ROOT_DIR}/.dashboard-boot-ui.log"
  run_log "  Use 'tail -f <logfile>' to monitor."
  warn "To stop everything: ./scripts/shutdown-all.sh"
}

main "$@"
