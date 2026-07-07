#!/usr/bin/env bash
set -euo pipefail

set -o allexport
if [ -f ".env" ]; then
  # shellcheck disable=SC1091
  source .env
fi
set +o allexport

: "${DATABASE_URL:=postgres://user:pass@127.0.0.1:5432/agents_db}"
: "${API_HOST:=127.0.0.1}"
: "${API_PORT:=8080}"
: "${SMOKE_API_WAIT_SECONDS:=180}"
: "${SMOKE_TIMEOUT_SECONDS:=180}"
: "${SMOKE_TARGET_EFFORT:=}"
: "${SMOKE_TARGET_ITEMS:=4}"
: "${SMOKE_AGENT_COUNT:=1}"

if [ "${API_HOST}" = "0.0.0.0" ]; then
  API_HOST="127.0.0.1"
fi

API_URL="http://${API_HOST}:${API_PORT}"
TARGET_EFFORT="${SMOKE_TARGET_EFFORT}"

log() {
  printf '[smoke] %s\n' "$1"
}

fail() {
  log "FAIL: $1"
  exit 1
}

ok_or_fail() {
  if ! "$@"; then
    fail "command failed: $*"
  fi
}

validate_json() {
  local payload="$1"
  local label="$2"

  if ! jq -e . >/dev/null 2>&1 <<<"${payload}"; then
    log "Invalid JSON for ${label}: ${payload}"
    fail "${label} was not valid JSON"
  fi
}

require_cmd() {
  local name="$1"
  if ! command -v "${name}" >/dev/null 2>&1; then
    fail "Missing required command: ${name}"
  fi
}

post_json() {
  local method="$1"
  local path="$2"
  local payload="${3:-}"

  local args=("-sS" "-H" "Content-Type: application/json")
  if [ -n "${payload}" ]; then
    args+=("-d" "${payload}")
  fi

  if [ "${method}" = "POST" ]; then
    curl "${args[@]}" -X POST "${API_URL}${path}"
  else
    curl "${args[@]}" -X GET "${API_URL}${path}"
  fi
}

extract_field() {
  local json_payload="$1"
  local field="$2"

  jq -r ".${field} // empty" <<<"${json_payload}" | sed 's/^null$//' || true
}

extract_created_id() {
  local json_payload="$1"
  local id

  id="$(jq -r '.id // empty' <<<"${json_payload}" | sed 's/^null$//' || true)"
  if [ -z "${id}" ] || [ "${id}" = "null" ]; then
    id="$(echo "${json_payload}" | tr -d '\"\\r\\n' | sed 's/^[[:space:]]*//; s/[[:space:]]*$//')"
  fi
  echo "${id}"
}

require_cmd curl
require_cmd docker
require_cmd jq

log "Checking postgres + redis"
if [ "${SMOKE_SKIP_DOCKER_UP:-0}" != "1" ]; then
  ok_or_fail docker compose up -d postgres redis
else
  log "Skipping docker compose up (SMOKE_SKIP_DOCKER_UP=1)"
fi

log "Waiting for API readiness"
deadline=$((SECONDS + SMOKE_API_WAIT_SECONDS))
until curl -sf "${API_URL}/health" >/dev/null 2>&1; do
  if [ "${SECONDS}" -ge "${deadline}" ]; then
    fail "API did not become available at ${API_URL}/health"
  fi
  sleep 1
done

log "Creating workspace"
workspace_payload="$(ok_or_fail post_json POST /api/workspaces "$(jq -n --arg name smoke-workspace '{"name": $name}' )")"
validate_json "${workspace_payload}" "workspace response"
workspace_id="$(extract_created_id "${workspace_payload}")"
if [ -z "${workspace_id}" ]; then
  fail "Workspace creation returned empty id"
fi
log "Workspace: ${workspace_id}"

log "Creating run"
run_payload="$(ok_or_fail post_json POST /api/runs "$(jq -n \
  --arg ws_id "${workspace_id}" \
  --arg obj "smoke-test objective" \
  --argjson target_items "${SMOKE_TARGET_ITEMS}" \
  --argjson agent_count "${SMOKE_AGENT_COUNT}" \
  '{ workspace_id: $ws_id, objective: $obj, target_item_count: $target_items, agent_count: $agent_count }')")"
validate_json "${run_payload}" "run create response"
run_id="$(extract_created_id "${run_payload}")"
if [ -z "${run_id}" ]; then
  fail "Run creation returned empty id"
fi
log "Run: ${run_id}"

log "Rebuilding projections"
ok_or_fail env DATABASE_URL="${DATABASE_URL}" cargo run -p application --bin rebuild-projections

echo "Event and projection totals:"
ok_or_fail docker compose exec -T postgres psql -U "${POSTGRES_USER:-user}" -d "${POSTGRES_DB:-agents_db}" -c "select count(*) as event_count from event_log;"
ok_or_fail docker compose exec -T postgres psql -U "${POSTGRES_USER:-user}" -d "${POSTGRES_DB:-agents_db}" -c "select count(*) as projection_runs from agent_runs_projection;"

echo "API status snapshot:"
ok_or_fail post_json GET /api/projections/status

echo "Workspace list:"
ok_or_fail post_json GET /api/workspaces

echo "Run list:"
ok_or_fail post_json GET /api/runs

echo "Run detail:"
run_payload_initial="$(ok_or_fail post_json GET "/api/runs/${run_id}")"
validate_json "${run_payload_initial}" "run detail response"
printf '%s\n' "${run_payload_initial}"

if [ -z "${TARGET_EFFORT}" ]; then
  initial_work_items=$(jq -r '.work_items | length' <<<"${run_payload_initial}")
  if [ -z "${initial_work_items}" ] || [ "${initial_work_items}" = "null" ]; then
    initial_work_items=0
  fi
  TARGET_EFFORT=$((initial_work_items * 5))
  if [ ${TARGET_EFFORT} -lt 1 ]; then
    TARGET_EFFORT=0
  fi
fi

echo "Overview metrics:"
ok_or_fail post_json GET /api/metrics/overview

deadline=$((SECONDS + SMOKE_TIMEOUT_SECONDS))
attempt=0
while true; do
  attempt=$((attempt + 1))
  run_payload="$(post_json GET "/api/runs/${run_id}")"
  validate_json "${run_payload}" "run poll response"

  if [ -z "${run_payload}" ]; then
    fail "No run payload returned on poll #${attempt}"
  fi

  run_status="$(extract_field "${run_payload}" status)"
  effort_points="$(extract_field "${run_payload}" effort_points)"
  latest_event="$(extract_field "${run_payload}" latest_event_type)"

  effort_points=${effort_points:-0}
  log "Poll ${attempt}: status=${run_status} effort=${effort_points} latest_event=${latest_event}"

  if [ "${run_status}" = "completed" ]; then
    if [ "${effort_points}" -ge "${TARGET_EFFORT}" ]; then
      log "Run completed and reached target effort (${effort_points}/${TARGET_EFFORT})."
      break
    fi
    fail "Run completed but reached insufficient effort (${effort_points}/${TARGET_EFFORT})."
  fi

  if [ "${run_status}" = "blocked" ] || [ "${run_status}" = "cancelled" ]; then
    fail "Run ended in terminal state: ${run_status}"
  fi

  if [ "${effort_points}" -gt 0 ]; then
    log "Progress observed."
  fi

  if [ "${SECONDS}" -ge "${deadline}" ]; then
    fail "Smoke test timed out waiting for completion after ${SMOKE_TIMEOUT_SECONDS}s"
  fi

  sleep 1
done

log "Smoke test complete."
exit 0
