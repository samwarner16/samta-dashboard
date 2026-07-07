#!/usr/bin/env bash
set -euo pipefail

PORT="${DASHBOARD_PORT:-4173}"
API_BASE="${DASHBOARD_API_BASE:-http://127.0.0.1:8080}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
FRONTEND_DIR="${PROJECT_DIR}/crates/dashboard/frontend"

cd "${FRONTEND_DIR}"

if [ "${DASHBOARD_OPEN:-0}" != "0" ]; then
  if command -v open >/dev/null 2>&1; then
    open "http://127.0.0.1:${PORT}/?api=${API_BASE}"
  elif command -v xdg-open >/dev/null 2>&1; then
    xdg-open "http://127.0.0.1:${PORT}/?api=${API_BASE}"
  fi
fi

echo "Serving dashboard at http://127.0.0.1:${PORT}/?api=${API_BASE}"
python3 -m http.server "${PORT}" --directory .
