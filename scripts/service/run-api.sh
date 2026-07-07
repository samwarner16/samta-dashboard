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
export API_HOST="${API_HOST:-0.0.0.0}"
export API_PORT="${API_PORT:-8080}"
export API_DB_CONNECT_ATTEMPTS="${API_DB_CONNECT_ATTEMPTS:-32}"
export API_DB_CONNECT_BACKOFF_MS="${API_DB_CONNECT_BACKOFF_MS:-500}"
export API_DB_SCHEMA_ATTEMPTS="${API_DB_SCHEMA_ATTEMPTS:-24}"
export API_DB_SCHEMA_BACKOFF_MS="${API_DB_SCHEMA_BACKOFF_MS:-500}"

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

exec cargo run -p api
