use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct CreateRunRequest {
    pub workspace_id: Uuid,
    pub objective: String,
    #[serde(default)]
    pub target_item_count: Option<u16>,
    #[serde(default)]
    pub agent_count: Option<u16>,
}

#[derive(Debug, Deserialize)]
pub struct CreateWorkspaceRequest {
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct CreateWorkspaceResponse {
    pub id: Uuid,
}

#[derive(Debug, Serialize)]
pub struct CreateRunResponse {
    pub id: Uuid,
}

#[derive(Debug, Serialize)]
pub struct ProjectionStatusResponse {
    pub event_count: i64,
    pub run_projection_count: i64,
}

#[derive(Debug, Serialize)]
pub struct RebuildProjectionsResponse {
    pub rebuilt_runs: usize,
}

#[derive(Debug, Serialize)]
pub struct WorkspaceResponse {
    pub id: Uuid,
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct RunSummaryResponse {
    pub run_id: Uuid,
    pub workspace_id: Uuid,
    pub status: String,
    pub total_cost: f64,
    pub effort_points: i32,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct RunActionResponse {
    pub run_id: Uuid,
    pub status: String,
    pub applied: bool,
}

#[derive(Debug, Serialize)]
pub struct WorkItemStatusResponse {
    pub item_id: Uuid,
    pub run_id: Uuid,
    pub status: String,
    pub assigned_agent_id: Option<Uuid>,
}

#[derive(Debug, Serialize)]
pub struct TimelineEventResponse {
    pub revision: i32,
    pub event_type: String,
    pub occurred_at: String,
    pub details: Value,
}

#[derive(Debug, Serialize)]
pub struct CostEffortHistoryPoint {
    pub revision: i32,
    pub occurred_at: String,
    pub total_cost: f64,
    pub total_effort: i32,
}

#[derive(Debug, Serialize)]
pub struct RunStatusResponse {
    pub run_id: Uuid,
    pub workspace_id: Uuid,
    pub status: String,
    pub total_cost: f64,
    pub effort_points: i32,
    pub stream_event_count: i64,
    pub stream_revision: i32,
    pub latest_event_type: Option<String>,
    pub latest_event_at: Option<String>,
    pub updated_at: String,
    pub work_items: Vec<WorkItemStatusResponse>,
    pub timeline: Vec<TimelineEventResponse>,
    pub cost_effort_history: Vec<CostEffortHistoryPoint>,
}

#[derive(Debug, Serialize)]
pub struct OverviewMetricsResponse {
    pub total_events: i64,
    pub event_stream_count: i64,
    pub total_workspaces: i64,
    pub total_runs: i64,
    pub running_runs: i64,
    pub completed_runs: i64,
    pub blocked_runs: i64,
    pub total_work_items: i64,
    pub last_event_at: Option<String>,
}
