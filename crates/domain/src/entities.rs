use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Organization {
    pub id: Uuid,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProfile {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    pub role: String,
    pub toolset: Vec<String>,
    pub cost_cap: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInstance {
    pub id: Uuid,
    pub profile_id: Uuid,
    pub status: String, // e.g., "idle", "busy"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRun {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub objective: String,
    pub status: String, // "pending", "running", "completed", "failed"
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkItem {
    pub id: Uuid,
    pub run_id: Uuid,
    pub description: String,
    pub assigned_agent_id: Option<Uuid>,
    pub status: String, // "unassigned", "assigned", "blocked", "done"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffortPoint {
    pub item_id: Uuid,
    pub amount: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostMetric {
    pub run_id: Uuid,
    pub amount: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRunProjection {
    pub run_id: Uuid,
    pub workspace_id: Uuid,
    pub status: String,
    pub total_cost: f64,
    pub effort_points: i32,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkItemProjection {
    pub item_id: Uuid,
    pub run_id: Uuid,
    pub status: String,
    pub assigned_agent_id: Option<Uuid>,
}
