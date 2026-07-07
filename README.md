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
2. Start the full stack with `./scripts/boot-all.sh` (default behavior is **clean slate**: stale containers/processes are removed and Postgres data volume is recreated)
3. `DATABASE_URL=postgres://user:pass@127.0.0.1:5432/agents_db cargo run -p application --bin rebuild-projections` (or run migrations manually)
4. Open dashboard UI at `http://127.0.0.1:4173` after boot

## Commands

- `./work/smoke-test.sh`: End-to-end smoke validation (create workspace/run, rebuild projections, print counts)
- `cargo make rebuild-projections`: Rebuild projection tables from event replay
- `cargo make smoke`: Legacy alias for end-to-end validation
- `cargo make serve-dashboard`: Serve frontend poller on `http://127.0.0.1:4173`
- `./scripts/start-dashboard.sh`: Start dashboard at `http://127.0.0.1:4173`
- `./scripts/boot-all.sh`: Start postgres/redis, kill stale app processes, start API, worker, and dashboard
- `./scripts/shutdown-all.sh`: Stop API/worker/dashboard processes and `docker compose down`
- `./scripts/systemd/*.service.template`: Optional Linux service templates
- `./scripts/local-services.sh`: Install/start/stop local launchd/systemd or direct pid-based service wrappers

Optional boot overrides:
- `DATABASE_URL` (default `postgres://user:pass@127.0.0.1:5432/agents_db`)
- `BOOT_CLEAN_SLATE` (`1` to wipe postgres data volume and restart containers, `0` to preserve)
- `DASHBOARD_PORT` (default `4173`)
- `DASHBOARD_API_BASE` (default `http://127.0.0.1:8080`)
- `DASHBOARD_OPEN=1` (auto-open browser)
- `BOOT_USE_SERVICE_WRAPPERS=1` and `BOOT_SERVICE_MODE=direct|launchd|systemd`
- `WORKER_EFFORT_PER_ITEM` / `WORKER_COST_PER_ITEM` (default item cost simulation)
- `SMOKE_TARGET_ITEMS` / `SMOKE_AGENT_COUNT` (smoke run parameters)

The `rebuild-projections` binary runs SQLx migrations and replays all events into the read projections.

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

1. Ensure services are running: `docker compose up -d postgres redis`
2. Run API in one terminal: `DATABASE_URL=postgres://user:pass@127.0.0.1:5432/agents_db cargo run -p api`
3. In another terminal: `./work/smoke-test.sh`

### Run control endpoints

- `POST /api/runs/{run_id}/pause`
- `POST /api/runs/{run_id}/resume`
- `POST /api/runs/{run_id}/cancel`
- `POST /api/runs/{run_id}/retry`
