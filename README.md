# Autonomous Agent Dashboard (Rust)

An interactive dashboard to spin up autonomous agents, assign tasks, monitor live progress, costs, effort, and status.

## Architecture Diagram

```text
[ UI / Browser / Tauri ] <--(REST/WS)--> [ API Gateway (Axum) ]
                                                  |
                                                  v
                                       [ Application Services ]
                          (Orchestrator, Planner, Dispatcher, Worker, Cost)
                                                  |
                                                  v
                                       [ Event Bus + Outbox ]
                                                  |
                                                  v
                                  [ Append-Only Event Log (Postgres) ]
                                                  |
                                                  v
                                  [ Read-side Projections (SQLite/Postgres + Redis) ]
```

## Startup Steps

1. `cp .env.example .env`
2. Start the full stack with `BOOT_CLEAN_SLATE=1 ./scripts/boot-all.sh` (default is clean slate: removes stale processes/containers and recreates the Postgres volume)
3. The API auto-runs migrations on boot. Use the `/api/projections/rebuild` endpoint or the `rebuild-projections` binary if you need a manual projection replay.
4. Open dashboard UI at `http://127.0.0.1:4173/?api=http://127.0.0.1:8080` (or use `DASHBOARD_OPEN=1`).

Primary commands (preferred):
- `BOOT_CLEAN_SLATE=1 ./scripts/boot-all.sh`
- `./work/smoke-test.sh`
- `./scripts/shutdown-all.sh`

## Commands

**Primary (script-driven):**

- `BOOT_CLEAN_SLATE=1 ./scripts/boot-all.sh` — full local stack (Postgres, Redis, API, worker, dashboard)
- `./work/smoke-test.sh` — end-to-end smoke (workspace + run + projection + completion assertions)
- `./scripts/shutdown-all.sh` — stop everything + docker compose down
- `./scripts/start-dashboard.sh` — serve the static frontend on 4173 (polls the API)
- `./scripts/run-agent-harness.sh start N` — launch N simulated workers (see harness section)
- `./scripts/local-services.sh` — launchd/systemd or direct wrappers

**Other useful:**

- `./scripts/boot-all.sh` (without BOOT_CLEAN_SLATE) for incremental restarts
- `cargo run -p application --bin rebuild-projections` — manual projection rebuild (rarely needed)
- `cargo run -p api` / `cargo run -p application --bin worker` for manual component runs

Legacy `cargo make` tasks (Makefile.toml) are present but no longer the recommended path. Scripts are authoritative.

Optional boot overrides:
- `DATABASE_URL` (default `postgres://user:pass@127.0.0.1:5432/agents_db`)
- `BOOT_CLEAN_SLATE` (`1` to wipe postgres data volume and restart containers, `0` to preserve)
- `DASHBOARD_PORT` (default `4173`)
- `DASHBOARD_API_BASE` (default `http://127.0.0.1:8080`)
- `BOOT_DOCKER_WAIT_SECONDS` (default `45`, wait time for Docker daemon readiness before aborting boot)
- `BOOT_AUTO_START_DOCKER` (`1` default; attempts to open Docker Desktop automatically on macOS when daemon is unavailable, `0` to disable)
- `DASHBOARD_OPEN=1` (auto-open browser)
- `BOOT_USE_SERVICE_WRAPPERS=1` and `BOOT_SERVICE_MODE=direct|launchd|systemd`
- `WORKER_EFFORT_PER_ITEM` / `WORKER_COST_PER_ITEM` (default item cost simulation)
- `SMOKE_TARGET_ITEMS` / `SMOKE_AGENT_COUNT` (smoke run parameters)

The `rebuild-projections` binary runs SQLx migrations and replays all events into the read projections.

See:
- `HANDOFF.md` — full current state, RunPod one-pod testing guide, and resume instructions.
- `PLAN.md` — detailed parallel implementation plan.
- `docs/runpod-onepod-guide.md` — step-by-step for setting up one RunPod to test the worker/harness script (local_llm mode on GPU).
- `docs/` for more (GPU pod deployment notes).

Long-term vision: control plane for agentic armies on RunPod / Vast.ai pods with high-VRAM cards for batch Qwen 2.5 32B.

### Multi-worker harness

Launch a local pool of workers that share the same event store:

```bash
./scripts/run-agent-harness.sh start 4
```

Harness controls:

- `./scripts/run-agent-harness.sh status`
- `./scripts/run-agent-harness.sh logs`
- `./scripts/run-agent-harness.sh stop`

Harness knobs:

- `WORKER_AGENT_IDS` (comma-separated IDs, length controls count)
- `WORKER_AGENT_ID` (single-worker mode override)
- `WORKER_AGENT_ID_PREFIX` (deterministic seed for generated IDs)
- `WORKER_POOL_SIZE` (default when count is omitted)
- `WORKER_ITEM_AFFINITY=1` (only process work items assigned to worker agent)
- `WORKER_ITEMS_PER_LOOP`, `WORKER_POLL_MS`, effort/cost and DB retry knobs

### Projection status endpoint

- `GET /api/projections/status` returns:
  - `event_count`
  - `run_projection_count`

Example:

```bash
curl http://127.0.0.1:8080/api/projections/status
```

### Workspace and run polling endpoints

- `GET /api/workspaces` returns all workspaces discovered in event history.
- `GET /api/runs` returns all run summaries from projections.
- `GET /api/runs/{run_id}` returns run status + stream metadata and per-item timeline:
  - `stream_event_count`
  - `stream_revision`
  - `latest_event_type`
  - `latest_event_at`
  - `work_items` (status per item)
- `GET /api/metrics/overview` returns dashboard summary counts:
  - `total_events`
  - `event_stream_count`
  - `total_workspaces`
  - `total_runs`
  - `running_runs`
  - `completed_runs`
  - `blocked_runs`
  - `total_work_items`
  - `last_event_at`

Examples:

```bash
curl http://127.0.0.1:8080/api/workspaces
curl http://127.0.0.1:8080/api/runs
curl http://127.0.0.1:8080/api/runs/<run_id>
curl http://127.0.0.1:8080/api/metrics/overview
```

## Smoke test

Preferred:

```bash
BOOT_CLEAN_SLATE=1 ./scripts/boot-all.sh
./work/smoke-test.sh
```

Manual:

1. `docker compose up -d postgres redis`
2. In one terminal: `cargo run -p api`
3. In another: `./work/smoke-test.sh`

### Run control endpoints

- `POST /api/runs/{run_id}/pause`
- `POST /api/runs/{run_id}/resume`
- `POST /api/runs/{run_id}/cancel`
- `POST /api/runs/{run_id}/retry`
