use std::collections::HashMap;

use anyhow::Result;
use chrono::{DateTime, Utc};
use events::EventPayload;
use uuid::Uuid;

use crate::persistence::{EventStore, ProjectionStore};

#[derive(Debug)]
struct RunProjectionState {
    run_id: Uuid,
    workspace_id: Uuid,
    status: String,
    total_cost: f64,
    effort_points: i32,
    updated_at: DateTime<Utc>,
}

#[derive(Debug)]
struct ItemProjectionState {
    item_id: Uuid,
    run_id: Uuid,
    status: String,
    assigned_agent_id: Option<Uuid>,
}

pub async fn rebuild_projections<E, P>(event_store: &E, projection_store: &P) -> Result<usize>
where
    E: EventStore,
    P: ProjectionStore,
{
    let events = event_store.load_all().await?;

    projection_store.clear().await?;

    let mut run_states: HashMap<Uuid, RunProjectionState> = HashMap::new();
    let mut item_states: HashMap<Uuid, ItemProjectionState> = HashMap::new();

    for event in events {
        match event.payload {
            EventPayload::RunStarted { workspace_id, .. } => {
                let run_id = event.resource_id;
                run_states.insert(
                    run_id,
                    RunProjectionState {
                        run_id,
                        workspace_id,
                        status: "running".to_string(),
                        total_cost: 0.0,
                        effort_points: 0,
                        updated_at: event.occurred_at,
                    },
                );
            }
            EventPayload::RunStatusChanged { run_id, status, .. } => {
                let run_state = run_states
                    .entry(run_id)
                    .or_insert_with(|| RunProjectionState {
                        run_id,
                        workspace_id: Uuid::nil(),
                        status: status.clone(),
                        total_cost: 0.0,
                        effort_points: 0,
                        updated_at: event.occurred_at,
                    });

                run_state.status = status;
                run_state.updated_at = event.occurred_at;
            }
            EventPayload::ProgressChunkEmitted {
                run_id,
                effort,
                cost,
                ..
            } => {
                let state = run_states
                    .entry(run_id)
                    .or_insert_with(|| RunProjectionState {
                        run_id,
                        workspace_id: Uuid::nil(),
                        status: "running".to_string(),
                        total_cost: 0.0,
                        effort_points: 0,
                        updated_at: event.occurred_at,
                    });

                state.total_cost += cost;
                state.effort_points += effort;
                state.status = "running".to_string();
                state.updated_at = event.occurred_at;
            }
            EventPayload::WorkItemAssigned {
                run_id,
                item_id,
                agent_id,
            } => {
                let item = item_states
                    .entry(item_id)
                    .or_insert_with(|| ItemProjectionState {
                        item_id,
                        run_id,
                        status: "assigned".to_string(),
                        assigned_agent_id: Some(agent_id),
                    });
                item.status = "assigned".to_string();
                item.assigned_agent_id = Some(agent_id);

                run_states
                    .entry(run_id)
                    .or_insert_with(|| RunProjectionState {
                        run_id,
                        workspace_id: Uuid::nil(),
                        status: "running".to_string(),
                        total_cost: 0.0,
                        effort_points: 0,
                        updated_at: event.occurred_at,
                    });
            }
            EventPayload::WorkItemStarted {
                run_id, item_id, ..
            } => {
                let item = item_states
                    .entry(item_id)
                    .or_insert_with(|| ItemProjectionState {
                        item_id,
                        run_id,
                        status: "running".to_string(),
                        assigned_agent_id: None,
                    });
                item.status = "running".to_string();

                run_states
                    .entry(run_id)
                    .or_insert_with(|| RunProjectionState {
                        run_id,
                        workspace_id: Uuid::nil(),
                        status: "running".to_string(),
                        total_cost: 0.0,
                        effort_points: 0,
                        updated_at: event.occurred_at,
                    });
            }
            EventPayload::WorkItemCompleted {
                run_id,
                item_id,
                effort,
                cost,
            } => {
                let item = item_states
                    .entry(item_id)
                    .or_insert_with(|| ItemProjectionState {
                        item_id,
                        run_id,
                        status: "completed".to_string(),
                        assigned_agent_id: None,
                    });
                item.status = "completed".to_string();

                let state = run_states
                    .entry(run_id)
                    .or_insert_with(|| RunProjectionState {
                        run_id,
                        workspace_id: Uuid::nil(),
                        status: "running".to_string(),
                        total_cost: 0.0,
                        effort_points: 0,
                        updated_at: event.occurred_at,
                    });
                state.status = "running".to_string();
                state.updated_at = event.occurred_at;
                state.total_cost += cost;
                state.effort_points += effort;
            }
            EventPayload::WorkItemFailed {
                run_id, item_id, ..
            } => {
                let item = item_states
                    .entry(item_id)
                    .or_insert_with(|| ItemProjectionState {
                        item_id,
                        run_id,
                        status: "failed".to_string(),
                        assigned_agent_id: None,
                    });
                item.status = "failed".to_string();

                let state = run_states
                    .entry(run_id)
                    .or_insert_with(|| RunProjectionState {
                        run_id,
                        workspace_id: Uuid::nil(),
                        status: "blocked".to_string(),
                        total_cost: 0.0,
                        effort_points: 0,
                        updated_at: event.occurred_at,
                    });
                state.status = "blocked".to_string();
                state.updated_at = event.occurred_at;
            }
            EventPayload::BlockerEncountered {
                run_id, item_id, ..
            } => {
                let item = item_states
                    .entry(item_id)
                    .or_insert_with(|| ItemProjectionState {
                        item_id,
                        run_id,
                        status: "blocked".to_string(),
                        assigned_agent_id: None,
                    });
                item.status = "blocked".to_string();

                let state = run_states
                    .entry(run_id)
                    .or_insert_with(|| RunProjectionState {
                        run_id,
                        workspace_id: Uuid::nil(),
                        status: "blocked".to_string(),
                        total_cost: 0.0,
                        effort_points: 0,
                        updated_at: event.occurred_at,
                    });
                state.status = "blocked".to_string();
                state.updated_at = event.occurred_at;
            }
            EventPayload::RunCompleted { run_id, .. } => {
                let state = run_states
                    .entry(run_id)
                    .or_insert_with(|| RunProjectionState {
                        run_id,
                        workspace_id: Uuid::nil(),
                        status: "completed".to_string(),
                        total_cost: 0.0,
                        effort_points: 0,
                        updated_at: event.occurred_at,
                    });
                state.status = "completed".to_string();
                state.updated_at = event.occurred_at;
            }
            _ => {}
        }
    }

    for state in run_states.values() {
        projection_store
            .upsert_run_projection(
                state.run_id,
                state.workspace_id,
                &state.status,
                state.total_cost,
                state.effort_points,
                state.updated_at,
            )
            .await?;
    }

    for item in item_states.values() {
        projection_store
            .upsert_work_item_projection(
                item.item_id,
                item.run_id,
                &item.status,
                item.assigned_agent_id,
            )
            .await?;
    }

    Ok(run_states.len())
}
