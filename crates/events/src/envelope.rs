use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub id: Uuid,
    pub occurred_at: DateTime<Utc>,
    pub actor_id: Uuid,
    pub resource_id: Uuid,
    pub correlation_id: Option<Uuid>,
    pub causation_id: Option<Uuid>,
    pub revision: i32,
    pub metadata: serde_json::Value,
    pub payload: EventPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    WorkspaceCreated,
    AgentProfileCreated,
    RunStarted,
    RunStatusChanged,
    WorkItemAssigned,
    WorkItemStarted,
    WorkItemCompleted,
    WorkItemFailed,
    ProgressChunkEmitted,
    BlockerEncountered,
    RunCompleted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum EventPayload {
    WorkspaceCreated {
        name: String,
    },
    AgentProfileCreated {
        name: String,
        role: String,
        cost_cap: f64,
    },
    RunStarted {
        workspace_id: Uuid,
        objective: String,
    },
    RunStatusChanged {
        run_id: Uuid,
        status: String,
        reason: Option<String>,
    },
    WorkItemAssigned {
        run_id: Uuid,
        item_id: Uuid,
        agent_id: Uuid,
    },
    WorkItemStarted {
        run_id: Uuid,
        item_id: Uuid,
        agent_id: Uuid,
    },
    WorkItemCompleted {
        run_id: Uuid,
        item_id: Uuid,
        effort: i32,
        cost: f64,
    },
    WorkItemFailed {
        run_id: Uuid,
        item_id: Uuid,
        reason: String,
    },
    ProgressChunkEmitted {
        run_id: Uuid,
        item_id: Uuid,
        effort: i32,
        cost: f64,
    },
    BlockerEncountered {
        run_id: Uuid,
        item_id: Uuid,
        reason: String,
    },
    RunCompleted {
        run_id: Uuid,
        summary: String,
    },
}

impl EventType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WorkspaceCreated => "workspace_created",
            Self::AgentProfileCreated => "agent_profile_created",
            Self::RunStarted => "run_started",
            Self::RunStatusChanged => "run_status_changed",
            Self::WorkItemAssigned => "work_item_assigned",
            Self::WorkItemStarted => "work_item_started",
            Self::WorkItemCompleted => "work_item_completed",
            Self::WorkItemFailed => "work_item_failed",
            Self::ProgressChunkEmitted => "progress_chunk_emitted",
            Self::BlockerEncountered => "blocker_encountered",
            Self::RunCompleted => "run_completed",
        }
    }
}

impl EventEnvelope {
    pub fn event_type(&self) -> EventType {
        match &self.payload {
            EventPayload::WorkspaceCreated { .. } => EventType::WorkspaceCreated,
            EventPayload::AgentProfileCreated { .. } => EventType::AgentProfileCreated,
            EventPayload::RunStarted { .. } => EventType::RunStarted,
            EventPayload::RunStatusChanged { .. } => EventType::RunStatusChanged,
            EventPayload::WorkItemAssigned { .. } => EventType::WorkItemAssigned,
            EventPayload::WorkItemStarted { .. } => EventType::WorkItemStarted,
            EventPayload::WorkItemCompleted { .. } => EventType::WorkItemCompleted,
            EventPayload::WorkItemFailed { .. } => EventType::WorkItemFailed,
            EventPayload::ProgressChunkEmitted { .. } => EventType::ProgressChunkEmitted,
            EventPayload::BlockerEncountered { .. } => EventType::BlockerEncountered,
            EventPayload::RunCompleted { .. } => EventType::RunCompleted,
        }
    }
}
