use chrono::Utc;
use events::{EventEnvelope, EventPayload};
use uuid::Uuid;

pub struct Worker {
    pub agent_id: Uuid,
    // TODO: Inject tool execution clients
}

impl Worker {
    pub fn new(agent_id: Uuid) -> Self {
        Self { agent_id }
    }

    pub fn execute_item(
        &self,
        run_id: Uuid,
        item_id: Uuid,
        revision: i32,
        agent_id: Uuid,
        effort: i32,
        cost: f64,
    ) -> Vec<EventEnvelope> {
        vec![
            self.item_started_event(run_id, item_id, revision, agent_id),
            self.item_completed_event(run_id, item_id, revision + 1, effort, cost, agent_id),
        ]
    }

    pub fn item_started_event(
        &self,
        run_id: Uuid,
        item_id: Uuid,
        revision: i32,
        agent_id: Uuid,
    ) -> EventEnvelope {
        EventEnvelope {
            id: Uuid::new_v4(),
            occurred_at: Utc::now(),
            actor_id: agent_id,
            resource_id: run_id,
            correlation_id: None,
            causation_id: None,
            revision,
            metadata: serde_json::Value::Null,
            payload: EventPayload::WorkItemStarted {
                run_id,
                item_id,
                agent_id: self.agent_id,
            },
        }
    }

    pub fn item_completed_event(
        &self,
        run_id: Uuid,
        item_id: Uuid,
        revision: i32,
        effort: i32,
        cost: f64,
        agent_id: Uuid,
    ) -> EventEnvelope {
        EventEnvelope {
            id: Uuid::new_v4(),
            occurred_at: Utc::now(),
            actor_id: agent_id,
            resource_id: run_id,
            correlation_id: None,
            causation_id: None,
            revision,
            metadata: serde_json::Value::Null,
            payload: EventPayload::WorkItemCompleted {
                run_id,
                item_id,
                effort,
                cost,
            },
        }
    }

    pub fn progress_chunk_event(
        &self,
        run_id: Uuid,
        item_id: Uuid,
        revision: i32,
        effort: i32,
        cost: f64,
        agent_id: Uuid,
    ) -> EventEnvelope {
        EventEnvelope {
            id: Uuid::new_v4(),
            occurred_at: Utc::now(),
            actor_id: agent_id,
            resource_id: run_id,
            correlation_id: None,
            causation_id: None,
            revision,
            metadata: serde_json::Value::Null,
            payload: EventPayload::ProgressChunkEmitted {
                run_id,
                item_id,
                effort,
                cost,
            },
        }
    }

    pub fn complete_event(&self, run_id: Uuid, revision: i32, total_completed_items: i32) -> EventEnvelope {
        EventEnvelope {
            id: Uuid::new_v4(),
            occurred_at: Utc::now(),
            actor_id: self.agent_id,
            resource_id: run_id,
            correlation_id: None,
            causation_id: None,
            revision,
            metadata: serde_json::Value::Null,
            payload: EventPayload::RunCompleted {
                run_id,
                summary: format!("worker completed run after {} items", total_completed_items),
            },
        }
    }

    pub fn item_failed_event(&self, run_id: Uuid, item_id: Uuid, revision: i32, reason: String) -> EventEnvelope {
        EventEnvelope {
            id: Uuid::new_v4(),
            occurred_at: Utc::now(),
            actor_id: self.agent_id,
            resource_id: run_id,
            correlation_id: None,
            causation_id: None,
            revision,
            metadata: serde_json::Value::Null,
            payload: EventPayload::WorkItemFailed {
                run_id,
                item_id,
                reason,
            },
        }
    }
}
