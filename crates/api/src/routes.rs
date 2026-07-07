use crate::dto::{
    CostEffortHistoryPoint, CreateRunRequest, CreateRunResponse, CreateWorkspaceRequest,
    CreateWorkspaceResponse, OverviewMetricsResponse, ProjectionStatusResponse,
    RebuildProjectionsResponse, RunActionResponse, RunStatusResponse, RunSummaryResponse,
    TimelineEventResponse, WorkItemStatusResponse, WorkspaceResponse,
};
use crate::ws::ws_handler;
use application::orchestrator::Orchestrator;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Result as AxumResult,
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use infra::{
    persistence::{PostgresEventStore, PostgresProjectionStore},
    replay::rebuild_projections,
    ws_gateway::WsGateway,
};
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    pub orchestrator: Arc<Orchestrator>,
    pub ws_gateway: Arc<WsGateway>,
    pub db_pool: PgPool,
}

pub fn create_router(
    orchestrator: Arc<Orchestrator>,
    ws_gateway: Arc<WsGateway>,
    db_pool: PgPool,
) -> Router {
    let state = AppState {
        orchestrator,
        ws_gateway,
        db_pool,
    };

    Router::new()
        .route("/health", get(|| async { "OK" }))
        .route(
            "/api/workspaces",
            get(list_workspaces).post(create_workspace),
        )
        .route("/api/runs", get(list_runs).post(create_run))
        .route("/api/runs/:run_id", get(run_status))
        .route("/api/runs/:run_id/pause", post(pause_run))
        .route("/api/runs/:run_id/resume", post(resume_run))
        .route("/api/runs/:run_id/cancel", post(cancel_run))
        .route("/api/runs/:run_id/retry", post(retry_run))
        .route("/ws", get(ws_handler))
        .route("/api/projections/status", get(projection_status))
        .route(
            "/api/projections/rebuild",
            post(rebuild_projections_handler),
        )
        .route("/api/metrics/overview", get(overview_metrics))
        .with_state(state)
}

async fn create_workspace(
    State(state): State<AppState>,
    Json(payload): Json<CreateWorkspaceRequest>,
) -> AxumResult<Json<CreateWorkspaceResponse>, (StatusCode, String)> {
    let id = state
        .orchestrator
        .create_workspace(payload.name)
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    Ok(Json(CreateWorkspaceResponse { id }))
}

async fn list_workspaces(
    State(state): State<AppState>,
) -> AxumResult<Json<Vec<WorkspaceResponse>>, (StatusCode, String)> {
    let workspaces: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT DISTINCT ON (resource_id) resource_id, payload->>'name' AS name \
         FROM event_log \
         WHERE event_type = $1 \
         ORDER BY resource_id, occurred_at DESC",
    )
    .bind("workspace_created")
    .fetch_all(&state.db_pool)
    .await
    .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
    .into_iter()
    .map(|(id, name)| (id, name))
    .collect();

    let workspaces = workspaces
        .into_iter()
        .map(|(id, name)| WorkspaceResponse { id, name })
        .collect();

    Ok(Json(workspaces))
}

async fn projection_status(
    State(state): State<AppState>,
) -> AxumResult<Json<ProjectionStatusResponse>, (StatusCode, String)> {
    let event_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM event_log")
        .fetch_one(&state.db_pool)
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;

    let run_projection_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM agent_runs_projection")
            .fetch_one(&state.db_pool)
            .await
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;

    Ok(Json(ProjectionStatusResponse {
        event_count,
        run_projection_count,
    }))
}

async fn create_run(
    State(state): State<AppState>,
    Json(payload): Json<CreateRunRequest>,
) -> AxumResult<Json<CreateRunResponse>, (StatusCode, String)> {
    let workspace_id = payload.workspace_id;
    let target_item_count = payload.target_item_count.filter(|count| *count > 0);
    let agent_count = payload.agent_count.filter(|count| *count > 0);

    let id = state
        .orchestrator
        .start_run(
            workspace_id,
            payload.objective,
            target_item_count,
            agent_count,
        )
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;

    upsert_run_projection(&state.db_pool, id, workspace_id, "running", Utc::now()).await?;

    Ok(Json(CreateRunResponse { id }))
}

async fn upsert_run_projection(
    db_pool: &PgPool,
    run_id: Uuid,
    workspace_id: Uuid,
    status: &str,
    updated_at: chrono::DateTime<Utc>,
) -> AxumResult<(), (StatusCode, String)> {
    sqlx::query(
        "INSERT INTO agent_runs_projection (run_id, workspace_id, status, total_cost, effort_points, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6) \
         ON CONFLICT (run_id) \
         DO UPDATE SET workspace_id = EXCLUDED.workspace_id, \
                       status = EXCLUDED.status, \
                       total_cost = EXCLUDED.total_cost, \
                       effort_points = EXCLUDED.effort_points, \
                       updated_at = EXCLUDED.updated_at",
    )
    .bind(run_id)
    .bind(workspace_id)
    .bind(status)
    .bind(0.0f64)
    .bind(0i32)
    .bind(updated_at)
    .execute(db_pool)
    .await
    .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;

    Ok(())
}

async fn sync_projection_status(
    db_pool: &PgPool,
    run_id: Uuid,
    workspace_id: Option<Uuid>,
    status: &str,
) -> AxumResult<(), (StatusCode, String)> {
    match workspace_id {
        Some(workspace_id) => {
            sqlx::query(
                "INSERT INTO agent_runs_projection (run_id, workspace_id, status, total_cost, effort_points, updated_at) \
                 VALUES ($1, $2, $3, 0.0, 0, now()) \
                 ON CONFLICT (run_id) DO UPDATE SET status = EXCLUDED.status, updated_at = EXCLUDED.updated_at",
            )
            .bind(run_id)
            .bind(workspace_id)
            .bind(status)
            .execute(db_pool)
            .await
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
        }
        None => {
            let updated = sqlx::query("UPDATE agent_runs_projection SET status = $1, updated_at = now() WHERE run_id = $2")
                .bind(status)
                .bind(run_id)
                .execute(db_pool)
                .await
                .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;

            if updated.rows_affected() == 0 {
                return Err((
                    StatusCode::NOT_FOUND,
                    "run projection not found".to_string(),
                ));
            }
        }
    }

    Ok(())
}

async fn list_runs(
    State(state): State<AppState>,
) -> AxumResult<Json<Vec<RunSummaryResponse>>, (StatusCode, String)> {
    let runs: Vec<(Uuid, Uuid, String, f64, i32, String)> = sqlx::query_as(
        "SELECT run_id, workspace_id, status, total_cost, effort_points, \
         to_char(updated_at, 'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"') AS updated_at \
         FROM agent_runs_projection \
         ORDER BY updated_at DESC",
    )
    .fetch_all(&state.db_pool)
    .await
    .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;

    let runs = runs
        .into_iter()
        .map(
            |(run_id, workspace_id, status, total_cost, effort_points, updated_at)| {
                RunSummaryResponse {
                    run_id,
                    workspace_id,
                    status,
                    total_cost,
                    effort_points,
                    updated_at,
                }
            },
        )
        .collect();

    Ok(Json(runs))
}

async fn run_status(
    State(state): State<AppState>,
    Path(run_id): Path<Uuid>,
) -> AxumResult<Json<RunStatusResponse>, (StatusCode, String)> {
    let projection_row = sqlx::query_as::<_, (Uuid, Uuid, String, f64, i32, String)>(
        "SELECT run_id, workspace_id, status, total_cost, effort_points, \
         to_char(updated_at, 'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"') AS updated_at \
         FROM agent_runs_projection WHERE run_id = $1",
    )
    .bind(run_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;

    let (run_id, workspace_id, status, total_cost, effort_points, updated_at) =
        projection_row.ok_or((StatusCode::NOT_FOUND, "run not found".to_string()))?;

    let stream_event_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM event_log WHERE resource_id = $1")
            .bind(run_id)
            .fetch_one(&state.db_pool)
            .await
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;

    let latest_event = sqlx::query_as::<_, (i32, String, String)>(
        "SELECT revision, event_type, to_char(occurred_at, 'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"') AS occurred_at \
         FROM event_log \
         WHERE resource_id = $1 \
         ORDER BY revision DESC \
         LIMIT 1",
    )
    .bind(run_id)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;

    let (stream_revision, latest_event_type, latest_event_at) = latest_event
        .map(|(revision, event_type, occurred_at)| (revision, Some(event_type), Some(occurred_at)))
        .unwrap_or((0, None, None));

    let work_items: Vec<WorkItemStatusResponse> =
        sqlx::query_as::<_, (Uuid, Uuid, String, Option<Uuid>)>(
            "SELECT item_id, run_id, status, assigned_agent_id FROM work_items_projection WHERE run_id = $1",
        )
        .bind(run_id)
        .fetch_all(&state.db_pool)
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
        .into_iter()
        .map(|(item_id, run_id, status, assigned_agent_id)| WorkItemStatusResponse {
            item_id,
            run_id,
            status,
            assigned_agent_id,
        })
        .collect();

    let timeline_rows = sqlx::query_as::<_, (i32, String, String, Value)>(
        "
        SELECT revision,
               event_type,
               to_char(occurred_at, 'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"') AS occurred_at,
               payload::jsonb AS details
        FROM event_log
        WHERE resource_id = $1
        ORDER BY revision ASC
    ",
    )
    .bind(run_id)
    .fetch_all(&state.db_pool)
    .await
    .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;

    let timeline = timeline_rows
        .iter()
        .map(
            |(revision, event_type, occurred_at, details)| TimelineEventResponse {
                revision: *revision,
                event_type: event_type.clone(),
                occurred_at: occurred_at.clone(),
                details: details.clone(),
            },
        )
        .collect::<Vec<_>>();

    let cost_effort_history = build_cost_effort_history(&timeline_rows);

    Ok(Json(RunStatusResponse {
        run_id,
        workspace_id,
        status,
        total_cost,
        effort_points,
        stream_event_count,
        stream_revision,
        latest_event_type,
        latest_event_at,
        updated_at,
        work_items,
        timeline,
        cost_effort_history,
    }))
}

fn build_cost_effort_history(rows: &[(i32, String, String, Value)]) -> Vec<CostEffortHistoryPoint> {
    let mut total_cost = 0.0_f64;
    let mut total_effort = 0_i32;

    rows.iter()
        .filter_map(|(revision, event_type, occurred_at, details)| {
            let (delta_effort, delta_cost) = extract_effort_cost(event_type, details);
            if delta_effort == 0 && delta_cost == 0.0 {
                return None;
            }

            total_effort += delta_effort;
            total_cost += delta_cost;
            Some(CostEffortHistoryPoint {
                revision: *revision,
                occurred_at: occurred_at.clone(),
                total_cost,
                total_effort,
            })
        })
        .collect()
}

fn extract_effort_cost(event_type: &str, details: &Value) -> (i32, f64) {
    match event_type {
        "progress_chunk_emitted" | "work_item_completed" => {
            let effort = details
                .get("effort")
                .and_then(Value::as_i64)
                .map_or(0, |value| value as i32);
            let cost = details.get("cost").and_then(Value::as_f64).unwrap_or(0.0);
            (effort, cost)
        }
        "work_item_started"
        | "work_item_assigned"
        | "work_item_failed"
        | "blocker_encountered"
        | "run_started"
        | "run_completed"
        | "run_status_changed"
        | "workspace_created"
        | "agent_profile_created" => (0, 0.0),
        _ => {
            if let Some(payload_event_type) = details.get("type").and_then(Value::as_str) {
                return extract_effort_cost(payload_event_type, details);
            }
            (0, 0.0)
        }
    }
}

async fn overview_metrics(
    State(state): State<AppState>,
) -> AxumResult<Json<OverviewMetricsResponse>, (StatusCode, String)> {
    let total_events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM event_log")
        .fetch_one(&state.db_pool)
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;

    let event_stream_count: i64 =
        sqlx::query_scalar("SELECT COUNT(DISTINCT resource_id) FROM event_log")
            .fetch_one(&state.db_pool)
            .await
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;

    let total_workspaces: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM (SELECT 1 FROM event_log WHERE event_type = 'workspace_created' GROUP BY resource_id) AS w",
    )
    .fetch_one(&state.db_pool)
    .await
    .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;

    let total_runs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_runs_projection")
        .fetch_one(&state.db_pool)
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;

    let running_runs: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM agent_runs_projection WHERE status = 'running'")
            .fetch_one(&state.db_pool)
            .await
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;

    let completed_runs: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM agent_runs_projection WHERE status = 'completed'")
            .fetch_one(&state.db_pool)
            .await
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;

    let blocked_runs: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM agent_runs_projection WHERE status = 'blocked'")
            .fetch_one(&state.db_pool)
            .await
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;

    let total_work_items: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM work_items_projection")
        .fetch_one(&state.db_pool)
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;

    let last_event_at = sqlx::query_scalar::<_, Option<String>>(
        "SELECT to_char(MAX(occurred_at), 'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"') FROM event_log",
    )
    .fetch_one(&state.db_pool)
    .await
    .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;

    Ok(Json(OverviewMetricsResponse {
        total_events,
        event_stream_count,
        total_workspaces,
        total_runs,
        running_runs,
        completed_runs,
        blocked_runs,
        total_work_items,
        last_event_at,
    }))
}

async fn rebuild_projections_handler(
    State(state): State<AppState>,
) -> AxumResult<Json<RebuildProjectionsResponse>, (StatusCode, String)> {
    let event_store = PostgresEventStore::new(state.db_pool.clone());
    let projection_store = PostgresProjectionStore::new(state.db_pool.clone());
    let rebuilt_runs = rebuild_projections(&event_store, &projection_store)
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;

    Ok(Json(RebuildProjectionsResponse { rebuilt_runs }))
}

async fn pause_run(
    State(state): State<AppState>,
    Path(run_id): Path<Uuid>,
) -> AxumResult<Json<RunActionResponse>, (StatusCode, String)> {
    let changed = state
        .orchestrator
        .pause_run(run_id)
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;

    if changed {
        let status = "paused".to_string();
        sync_projection_status(&state.db_pool, run_id, None, &status).await?;

        Ok(Json(RunActionResponse {
            run_id,
            status,
            applied: true,
        }))
    } else {
        let (run_status, _) = fetch_run_status_only(&state.db_pool, run_id).await?;
        Ok(Json(RunActionResponse {
            run_id,
            status: run_status,
            applied: false,
        }))
    }
}

async fn resume_run(
    State(state): State<AppState>,
    Path(run_id): Path<Uuid>,
) -> AxumResult<Json<RunActionResponse>, (StatusCode, String)> {
    let changed = state
        .orchestrator
        .resume_run(run_id)
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;

    if changed {
        let status = "running".to_string();
        sync_projection_status(&state.db_pool, run_id, None, &status).await?;
        Ok(Json(RunActionResponse {
            run_id,
            status,
            applied: true,
        }))
    } else {
        let (run_status, _) = fetch_run_status_only(&state.db_pool, run_id).await?;
        Ok(Json(RunActionResponse {
            run_id,
            status: run_status,
            applied: false,
        }))
    }
}

async fn cancel_run(
    State(state): State<AppState>,
    Path(run_id): Path<Uuid>,
) -> AxumResult<Json<RunActionResponse>, (StatusCode, String)> {
    let changed = state
        .orchestrator
        .cancel_run(run_id)
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;

    if changed {
        let status = "cancelled".to_string();
        sync_projection_status(&state.db_pool, run_id, None, &status).await?;
        Ok(Json(RunActionResponse {
            run_id,
            status,
            applied: true,
        }))
    } else {
        let (run_status, _) = fetch_run_status_only(&state.db_pool, run_id).await?;
        Ok(Json(RunActionResponse {
            run_id,
            status: run_status,
            applied: false,
        }))
    }
}

async fn retry_run(
    State(state): State<AppState>,
    Path(run_id): Path<Uuid>,
) -> AxumResult<Json<RunActionResponse>, (StatusCode, String)> {
    let changed = state
        .orchestrator
        .retry_run(run_id)
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;

    if changed {
        let status = "running".to_string();
        let workspace_id = fetch_run_workspace(&state.db_pool, run_id).await?;
        sync_projection_status(&state.db_pool, run_id, workspace_id, &status).await?;
        Ok(Json(RunActionResponse {
            run_id,
            status,
            applied: true,
        }))
    } else {
        let (run_status, _) = fetch_run_status_only(&state.db_pool, run_id).await?;
        Ok(Json(RunActionResponse {
            run_id,
            status: run_status,
            applied: false,
        }))
    }
}

async fn fetch_run_workspace(
    db_pool: &PgPool,
    run_id: Uuid,
) -> Result<Option<Uuid>, (StatusCode, String)> {
    let row = sqlx::query_scalar::<_, Option<Uuid>>(
        "
        SELECT (payload->>'workspace_id')::uuid
        FROM event_log
        WHERE resource_id = $1
          AND event_type = 'run_started'
          AND payload ? 'workspace_id'
        LIMIT 1
    ",
    )
    .bind(run_id)
    .fetch_optional(db_pool)
    .await
    .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;

    Ok(row.flatten())
}

async fn fetch_run_status_only(
    db_pool: &PgPool,
    run_id: Uuid,
) -> Result<(String, chrono::DateTime<Utc>), (StatusCode, String)> {
    let row = sqlx::query_as::<_, (String, chrono::DateTime<Utc>)>(
        "SELECT status, updated_at FROM agent_runs_projection WHERE run_id = $1",
    )
    .bind(run_id)
    .fetch_optional(db_pool)
    .await
    .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;

    row.ok_or((StatusCode::NOT_FOUND, "run not found".to_string()))
}
