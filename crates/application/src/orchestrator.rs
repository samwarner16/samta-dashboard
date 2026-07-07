use std::collections::HashMap;

use anyhow::{anyhow, Result};
use chrono::Utc;
use domain::entities::AgentInstance;
use events::{EventEnvelope, EventPayload};
use infra::persistence::EventStore;
use infra::ws_gateway::WsGateway;
use uuid::Uuid;

use crate::{dispatcher::Dispatcher, planner::Planner};

pub struct Orchestrator {
    event_store: std::sync::Arc<dyn EventStore>,
    ws_gateway: std::sync::Arc<WsGateway>,
    planner: Planner,
    dispatcher: Dispatcher,
}

impl Orchestrator {
    pub fn new(event_store: std::sync::Arc<dyn EventStore>, ws_gateway: std::sync::Arc<WsGateway>) -> Self {
        Self {
            event_store,
            ws_gateway,
            planner: Planner::new(),
            dispatcher: Dispatcher::new(),
        }
    }

    pub async fn create_workspace(&self, name: String) -> Result<Uuid> {
        let workspace_id = Uuid::new_v4();
        let event = self
            .new_event(
                workspace_id,
                self.next_revision(workspace_id).await?,
                EventPayload::WorkspaceCreated { name },
            )
            .await?;

        self.event_store
            .append(workspace_id, vec![event.clone()])
            .await?;
        self.ws_gateway.broadcast(event);

        Ok(workspace_id)
    }

    pub async fn create_agent_profile(
        &self,
        _workspace_id: Uuid,
        name: String,
        role: String,
    ) -> Result<Uuid> {
        let profile_id = Uuid::new_v4();
        let payload = EventPayload::AgentProfileCreated {
            name,
            role,
            cost_cap: 0.0,
        };

        let event = self
            .new_event(profile_id, self.next_revision(profile_id).await?, payload)
            .await?;
        self.event_store
            .append(profile_id, vec![event.clone()])
            .await?;
        self.ws_gateway.broadcast(event);

        Ok(profile_id)
    }

    pub async fn start_run(
        &self,
        workspace_id: Uuid,
        objective: String,
        target_item_count: Option<u16>,
        agent_count: Option<u16>,
    ) -> Result<Uuid> {
        let run_id = Uuid::new_v4();

        let mut revision = self.next_revision(run_id).await?;
        let mut events = vec![
            self.new_event(
                run_id,
                revision,
                EventPayload::RunStarted {
                    workspace_id,
                    objective: objective.clone(),
                },
            )
            .await?,
        ];

        revision += 1;
        let target_items = self.planner.resolve_target_item_count(target_item_count);
        let requested_agents = self.planner.resolve_agent_count(agent_count);
        let items = self
            .planner
            .decompose_objective_with_target(run_id, objective, target_items)
            .await?;
        let assigned_items = self
            .dispatcher
            .assign_items(items, self.default_agents(requested_agents))
            .await?;

        for item in assigned_items {
            events.push(
                self.new_event(
                    run_id,
                    revision,
                    EventPayload::WorkItemAssigned {
                        run_id,
                        item_id: item.id,
                        agent_id: item.assigned_agent_id.unwrap_or_else(Uuid::new_v4),
                    },
                )
                .await?,
            );
            revision += 1;
        }

        self.event_store.append(run_id, events.clone()).await?;
        for event in events {
            self.ws_gateway.broadcast(event);
        }

        Ok(run_id)
    }

    pub async fn pause_run(&self, run_id: Uuid) -> Result<bool> {
        self.change_status(run_id, "paused", Some("manual pause".to_string())).await
    }

    pub async fn resume_run(&self, run_id: Uuid) -> Result<bool> {
        self.change_status(run_id, "running", Some("manual resume".to_string())).await
    }

    pub async fn cancel_run(&self, run_id: Uuid) -> Result<bool> {
        self.change_status(run_id, "cancelled", Some("manual cancel".to_string()))
            .await
    }

    pub async fn retry_run(&self, run_id: Uuid) -> Result<bool> {
        let events = self.event_store.load(run_id).await?;
        if events.is_empty() {
            return Err(anyhow!("run {run_id} not found"));
        }

        let run_state = RunReplayState::from_events(&events);
        if run_state.status == "running" {
            return Ok(false);
        }

        if run_state.status == "completed" {
            return Ok(false);
        }

        let mut revision = self.next_revision(run_id).await?;
        let mut new_events = vec![
            self.new_event(
                run_id,
                revision,
                EventPayload::RunStatusChanged {
                    run_id,
                    status: "running".to_string(),
                    reason: Some("manual retry".to_string()),
                },
            )
            .await?,
        ];
        revision += 1;

        let mut reassigned = 0;
        for (item_id, state) in &run_state.work_items {
            if state.status == "completed" {
                continue;
            }

            new_events.push(
                self.new_event(
                    run_id,
                    revision,
                    EventPayload::WorkItemAssigned {
                        run_id,
                        item_id: *item_id,
                        agent_id: state.assigned_agent_id.unwrap_or_else(Uuid::new_v4),
                    },
                )
                .await?,
            );
            revision += 1;
            reassigned += 1;
        }

        if reassigned == 0 {
            return Ok(false);
        }

        self.event_store.append(run_id, new_events.clone()).await?;
        for event in new_events {
            self.ws_gateway.broadcast(event);
        }

        Ok(true)
    }

    async fn change_status(&self, run_id: Uuid, status: &str, reason: Option<String>) -> Result<bool> {
        let events = self.event_store.load(run_id).await?;
        if events.is_empty() {
            return Err(anyhow!("run {run_id} not found"));
        }

        let current = RunReplayState::from_events(&events).status;
        if current == status {
            return Ok(false);
        }

        let event = self
            .new_event(
                run_id,
                self.next_revision(run_id).await?,
                EventPayload::RunStatusChanged {
                    run_id,
                    status: status.to_string(),
                    reason,
                },
            )
            .await?;

        self.event_store.append(run_id, vec![event.clone()]).await?;
        self.ws_gateway.broadcast(event);

        Ok(true)
    }

    fn default_agents(&self, requested_count: usize) -> Vec<AgentInstance> {
        let requested_count = requested_count.max(1);
        (0..requested_count)
            .map(|_| AgentInstance {
                id: Uuid::new_v4(),
                profile_id: Uuid::new_v4(),
                status: "idle".to_string(),
            })
            .collect()
    }

    async fn new_event(
        &self,
        resource_id: Uuid,
        revision: i32,
        payload: EventPayload,
    ) -> Result<EventEnvelope> {
        Ok(EventEnvelope {
            id: Uuid::new_v4(),
            occurred_at: Utc::now(),
            actor_id: Uuid::new_v4(),
            resource_id,
            correlation_id: None,
            causation_id: None,
            revision,
            metadata: serde_json::Value::Null,
            payload,
        })
    }

    async fn next_revision(&self, resource_id: Uuid) -> Result<i32> {
        let events = self.event_store.load(resource_id).await?;
        match events.last() {
            Some(last) => Ok(last.revision + 1),
            None => Ok(1),
        }
    }
}

#[derive(Default)]
struct WorkItemReplayState {
    status: String,
    assigned_agent_id: Option<Uuid>,
}

struct RunReplayState {
    status: String,
    work_items: HashMap<Uuid, WorkItemReplayState>,
}

impl RunReplayState {
    fn from_events(events: &[EventEnvelope]) -> Self {
        let mut state = Self {
            status: "pending".to_string(),
            work_items: HashMap::new(),
        };

        for event in events {
            match &event.payload {
                EventPayload::RunStarted { .. } => {
                    state.status = "running".to_string();
                }
                EventPayload::RunStatusChanged { status, .. } => {
                    state.status = status.clone();
                }
                EventPayload::WorkItemAssigned {
                    item_id,
                    agent_id,
                    ..
                } => {
                    let entry = state.work_items.entry(*item_id).or_insert(WorkItemReplayState {
                        status: "assigned".to_string(),
                        assigned_agent_id: Some(*agent_id),
                    });
                    entry.status = "assigned".to_string();
                    entry.assigned_agent_id = Some(*agent_id);
                }
                EventPayload::WorkItemStarted { item_id, agent_id, .. } => {
                    let entry = state.work_items.entry(*item_id).or_insert(WorkItemReplayState {
                        status: "running".to_string(),
                        assigned_agent_id: Some(*agent_id),
                    });
                    entry.status = "running".to_string();
                    entry.assigned_agent_id = Some(*agent_id);
                }
                EventPayload::WorkItemCompleted { item_id, .. } => {
                    let entry = state.work_items.entry(*item_id).or_insert(WorkItemReplayState {
                        status: "completed".to_string(),
                        assigned_agent_id: None,
                    });
                    entry.status = "completed".to_string();
                }
                EventPayload::WorkItemFailed { item_id, .. } => {
                    let entry = state.work_items.entry(*item_id).or_insert(WorkItemReplayState {
                        status: "failed".to_string(),
                        assigned_agent_id: None,
                    });
                    entry.status = "failed".to_string();
                }
                EventPayload::BlockerEncountered { item_id, .. } => {
                    let entry = state.work_items.entry(*item_id).or_insert(WorkItemReplayState {
                        status: "blocked".to_string(),
                        assigned_agent_id: None,
                    });
                    entry.status = "blocked".to_string();
                    state.status = "blocked".to_string();
                }
                EventPayload::RunCompleted { .. } => {
                    state.status = "completed".to_string();
                }
                _ => {}
            }
        }

        state
    }
}
