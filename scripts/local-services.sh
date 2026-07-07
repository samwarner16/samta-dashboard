#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ACTION="${1:-status}"
SCOPE="${2:-both}"

PLATFORM="$(uname -s)"
API_SERVICE="com.go-ahead-and-call.api"
WORKER_SERVICE="com.go-ahead-and-call.worker"
FORCE_NO_LAUNCHD="${LOCAL_SERVICES_NO_LAUNCHD:-0}"

if [ "${SCOPE}" = "api" ]; then
  SERVICES=("api")
elif [ "${SCOPE}" = "worker" ]; then
  SERVICES=("worker")
else
  SERVICES=("api" "worker")
fi

service_label() {
  local svc="$1"
  if [ "$svc" = "api" ]; then
    echo "$API_SERVICE"
  else
    echo "$WORKER_SERVICE"
  fi
}

direct_pid_file() {
  local svc="$1"
  if [ "$svc" = "api" ]; then
    echo "${ROOT_DIR}/.dashboard-api-service.pid"
  else
    echo "${ROOT_DIR}/.dashboard-worker-service.pid"
  fi
}

direct_mode_file() {
  local svc="$1"
  if [ "$svc" = "api" ]; then
    echo "${ROOT_DIR}/.dashboard-api-service.mode"
  else
    echo "${ROOT_DIR}/.dashboard-worker-service.mode"
  fi
}

direct_log_file() {
  local svc="$1"
  if [ "$svc" = "api" ]; then
    echo "${ROOT_DIR}/.dashboard-api-service.log"
  else
    echo "${ROOT_DIR}/.dashboard-worker-service.log"
  fi
}

usage() {
  cat <<'EOF'
Usage: scripts/local-services.sh <action> [api|worker|both]

Actions:
  install   Generate and install launchd/systemd unit files
  start     Start API/worker services
  stop      Stop API/worker services
  restart   Restart API/worker services
  status    Print service status
  logs      Tail service logs: requires third arg api|worker|all (default all)

Examples:
  scripts/local-services.sh install
  scripts/local-services.sh start both
  scripts/local-services.sh logs api
EOF
  exit 1
}

if [ "${ACTION}" = "help" ] || [ "${ACTION}" = "-h" ] || [ "${ACTION}" = "--help" ]; then
  usage
fi

run_in_systemd() {
  local verb="$1"
  for svc in "${SERVICES[@]}"; do
    local unit="go-ahead-and-call-${svc}.service"
    systemctl --user "$verb" "$unit" >/dev/null
  done
}

start_service_direct() {
  local svc="$1"
  local script="${ROOT_DIR}/scripts/service/run-${svc}.sh"
  local pid_file
  local mode_file
  local log_file

  pid_file="$(direct_pid_file "$svc")"
  mode_file="$(direct_mode_file "$svc")"
  log_file="$(direct_log_file "$svc")"

  if [ ! -x "$script" ]; then
    echo "Service script missing or not executable: $script" >&2
    return 1
  fi

  printf '%s\n' "direct" > "${mode_file}"
  nohup "${script}" >"${log_file}" 2>&1 &
  echo "$!" > "${pid_file}"
}

stop_service_direct() {
  local svc="$1"
  local pid_file
  local mode_file
  local pid

  pid_file="$(direct_pid_file "$svc")"
  mode_file="$(direct_mode_file "$svc")"

  if [ -f "${pid_file}" ]; then
    pid="$(cat "${pid_file}")"
    if [ -n "${pid}" ] && ps -p "${pid}" >/dev/null 2>&1; then
      kill "${pid}" >/dev/null 2>&1 || true
      wait "${pid}" 2>/dev/null || true
    fi
    rm -f "${pid_file}"
  fi
  rm -f "${mode_file}"
}

status_service_direct() {
  local svc="$1"
  local label
  local pid_file
  local pid

  label="$(service_label "$svc")"
  pid_file="$(direct_pid_file "$svc")"

  if [ -f "${pid_file}" ]; then
    pid="$(cat "${pid_file}")"
    if [ -n "${pid}" ] && ps -p "${pid}" >/dev/null 2>&1; then
      echo "${label}: running"
    else
      echo "${label}: stopped"
    fi
  else
    echo "${label}: stopped"
  fi
}

start_direct() {
  for svc in "${SERVICES[@]}"; do
    start_service_direct "$svc"
  done
}

stop_direct() {
  for svc in "${SERVICES[@]}"; do
    stop_service_direct "$svc"
  done
}

status_direct() {
  for svc in "${SERVICES[@]}"; do
    status_service_direct "$svc"
  done
}

status_systemd() {
  for svc in "${SERVICES[@]}"; do
    local unit="go-ahead-and-call-${svc}.service"
    printf '%s: ' "$unit"
    if systemctl --user is-active --quiet "$unit"; then
      echo "active"
    elif systemctl --user is-enabled --quiet "$unit" 2>/dev/null; then
      echo "inactive"
    else
      echo "missing/disabled"
    fi
  done
}

install_systemd() {
  bash "${ROOT_DIR}/scripts/systemd/install-templates.sh"
  systemctl --user daemon-reload
  for svc in "${SERVICES[@]}"; do
    local unit="go-ahead-and-call-${svc}.service"
    systemctl --user enable "$unit"
  done
}

run_in_launchd() {
  local verb="$1"
  local uid
  uid="$(id -u)"
  local domain="gui/$uid"
  local target_dir="${HOME}/Library/LaunchAgents"

  for svc in "${SERVICES[@]}"; do
    local label
    local plist
    local expanded
    if [ "$svc" = "api" ]; then
      label="${API_SERVICE}"
      expanded="${target_dir}/${label}.plist"
    else
      label="${WORKER_SERVICE}"
      expanded="${target_dir}/${label}.plist"
    fi

    launchctl bootout "$domain/$label" 2>/dev/null || true
    if [ "$verb" = "start" ] || [ "$verb" = "restart" ]; then
      launchctl bootstrap "$domain" "$expanded"
    fi
  done
}

status_launchd() {
  local uid="$(id -u)"
  local domain="gui/$(id -u)"
  for svc in "${SERVICES[@]}"; do
    local label
    if [ "$svc" = "api" ]; then
      label="${API_SERVICE}"
    else
      label="${WORKER_SERVICE}"
    fi

    if launchctl print "${domain}/${label}" >/dev/null 2>&1; then
      echo "$label: running"
    else
      echo "$label: stopped"
    fi
  done
}

install_launchd() {
  local target_dir="${HOME}/Library/LaunchAgents"
  mkdir -p "$target_dir"

  for tpl in "${ROOT_DIR}/scripts/launchd"/*.template; do
    local basename_file
    basename_file="$(basename "$tpl")"
    local service_name
    service_name="${basename_file%.template}"
    local label
    if [[ "$service_name" == api* ]]; then
      label="${API_SERVICE}"
    elif [[ "$service_name" == worker* ]]; then
      label="${WORKER_SERVICE}"
    else
      continue
    fi

    sed \
      -e "s#<REPO_DIR>#${ROOT_DIR}#g" \
      -e "s#<HOME_DIR>#${HOME}#g" \
      "$tpl" > "${target_dir}/${label}.plist"
  done
}

log_file() {
  local svc="$1"
  case "$svc" in
    api)
      echo "${ROOT_DIR}/.dashboard-api-service.log"
      ;;
    worker)
      echo "${ROOT_DIR}/.dashboard-worker-service.log"
      ;;
    *)
      echo "${ROOT_DIR}/.dashboard-boot-api.log"
      ;;
  esac
}

start_services() {
  if [ "$PLATFORM" = "Darwin" ]; then
    if [ "$FORCE_NO_LAUNCHD" = "1" ]; then
      start_direct
    else
      run_in_launchd start
    fi
  elif [ "$PLATFORM" = "Linux" ]; then
    run_in_systemd start
  else
    echo "Unsupported platform: $PLATFORM" >&2
    exit 1
  fi
}

stop_services() {
  if [ "$PLATFORM" = "Darwin" ]; then
    if [ "$FORCE_NO_LAUNCHD" = "1" ]; then
      stop_direct
      return
    fi

    run_in_launchd stop
    stop_direct
  elif [ "$PLATFORM" = "Linux" ]; then
    run_in_systemd stop
  else
    echo "Unsupported platform: $PLATFORM" >&2
    exit 1
  fi
}

status_services() {
  if [ "$PLATFORM" = "Darwin" ]; then
    if [ "$FORCE_NO_LAUNCHD" = "1" ]; then
      status_direct
    else
      status_launchd
    fi
  elif [ "$PLATFORM" = "Linux" ]; then
    status_systemd
  else
    echo "Unsupported platform: $PLATFORM" >&2
    exit 1
  fi
}

install_services() {
  if [ "$PLATFORM" = "Darwin" ]; then
    install_launchd
  elif [ "$PLATFORM" = "Linux" ]; then
    install_systemd
  else
    echo "Unsupported platform: $PLATFORM" >&2
    exit 1
  fi
}

case "$ACTION" in
  install)
    install_services
    ;;
  start)
    start_services
    ;;
  stop)
    stop_services
    ;;
  restart)
    stop_services
    start_services
    ;;
  status)
    status_services
    ;;
  logs)
    LOG_SCOPE="${3:-all}"
    if [ "$LOG_SCOPE" = "all" ]; then
      for svc in api worker; do
        echo "=== $svc ==="
        tail -n 80 "$(log_file "$svc")"
      done
    else
      if [ "$LOG_SCOPE" != "api" ] && [ "$LOG_SCOPE" != "worker" ]; then
        echo "invalid log scope: $LOG_SCOPE" >&2
        exit 1
      fi
      tail -n 80 "$(log_file "$LOG_SCOPE")"
    fi
    ;;
  *)
    usage
    ;;
esac

echo "Done: ${ACTION} ${SCOPE}"
