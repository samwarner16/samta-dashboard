#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_DIR}"

if [ -f .env ]; then
  set -o allexport
  # shellcheck disable=SC1091
  source .env
  set +o allexport
fi

export DATABASE_URL="${DATABASE_URL:-postgres://user:pass@127.0.0.1:5432/agents_db}"
export WORKER_POLL_MS="${WORKER_POLL_MS:-1500}"
export WORKER_ITEMS_PER_LOOP="${WORKER_ITEMS_PER_LOOP:-1}"
export WORKER_DB_CONNECT_ATTEMPTS="${WORKER_DB_CONNECT_ATTEMPTS:-24}"
export WORKER_DB_CONNECT_BACKOFF_MS="${WORKER_DB_CONNECT_BACKOFF_MS:-500}"
export WORKER_DB_SCHEMA_ATTEMPTS="${WORKER_DB_SCHEMA_ATTEMPTS:-24}"
export WORKER_DB_SCHEMA_BACKOFF_MS="${WORKER_DB_SCHEMA_BACKOFF_MS:-500}"

if ! command -v cargo >/dev/null 2>&1; then
  if [ -f "${HOME}/.cargo/env" ]; then
    # shellcheck disable=SC1091
    source "${HOME}/.cargo/env"
  fi
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo not found in PATH. Ensure Rust is installed and in PATH for services." >&2
  exit 1
fi

exec cargo run -p application --bin worker
