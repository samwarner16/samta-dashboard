use std::collections::HashMap;
use std::env;
use std::time::Duration;

use anyhow::{Context, Result};
use application::harness::{events_for_execution, AgentExecutor, ExecutionContext};
use application::worker::Worker;
use chrono::{DateTime, Utc};
use events::{EventEnvelope, EventPayload};
use infra::persistence::{
    EventStore, PostgresEventStore, PostgresProjectionStore, ProjectionStore,
};
use tokio::time::sleep;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<()> {
    let database_url = env::var("DATABASE_URL").context("DATABASE_URL must be set")?;
    let worker_agent_id = env::var("WORKER_AGENT_ID")
        .ok()
        .and_then(|id| Uuid::parse_str(&id).ok())
        .unwrap_or_else(Uuid::new_v4);
    let poll_ms = env::var("WORKER_POLL_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(1_500);
    let max_items_per_loop = env::var("WORKER_ITEMS_PER_LOOP")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|items| *items > 0)
        .unwrap_or(1);
    let item_affinity = read_bool_env("WORKER_ITEM_AFFINITY", false);

    let mut event_store = connect_with_retry(&database_url).await?;
    let mut projection_store = PostgresProjectionStore::new(event_store.pool().clone());
    let worker = Worker::new(worker_agent_id);

    // The harness (executor) is chosen here. Supported modes (via HARNESS_CONTROLLER env):
    // - simulated (default)
    // - api_endpoint  (HARNESS_API_ENDPOINT=... )  → harness controlled by external API
    // - local_llm / gpu (HARNESS_LLM_BASE_URL=..., HARNESS_LLM_MODEL=...) → LLM on local GPU card(s)
    //
    // This allows the exact same worker binary to run either as an API-driven client
    // or as the brain for a full local LLM (Qwen etc.) spun up on high-VRAM graphics hardware.
    let executor: Box<dyn AgentExecutor> = application::harness::create_executor_from_env(worker_agent_id);

    info!(
        "Worker started (agent={}) polling every {poll_ms}ms with max {max_items_per_loop} items per loop",
        worker.agent_id
    );
    info!(agent_id = %worker.agent_id, item_affinity = item_affinity, "worker affinity enabled (harness=simulated)");

    let mut loop_failures = 0_u32;
    let append_max_attempts = read_u32_env("WORKER_APPEND_ATTEMPTS", 4);
    let append_backoff_ms = read_u64_env("WORKER_APPEND_BACKOFF_MS", 500);

    loop {
        let mut run_states = match event_store.load_all().await {
            Ok(events) => {
                loop_failures = 0;
                run_states_from_events(&events)
            }
            Err(err) => {
                loop_failures += 1;
                if loop_failures == 1 || loop_failures % 5 == 0 {
                    warn!(
                        attempt = loop_failures,
                        error = %err,
                        "Retrying after event-store load failure"
                    );
                }
                sleep(Duration::from_millis(std::cmp::max(poll_ms, 500))).await;
                match connect_with_retry(&database_url).await {
                    Ok(new_store) => {
                        info!(run_id = "-", "Reconnected to Postgres after load failure");
                        event_store = new_store;
                        projection_store = PostgresProjectionStore::new(event_store.pool().clone());
                    }
                    Err(reconnect_err) => {
                        warn!(error = %reconnect_err, "Reconnection attempt failed");
                    }
                }
                continue;
            }
        };

        let mut did_work = false;

        for (run_id, state) in run_states.iter_mut() {
            if state.workspace_id.is_nil() || state.status != "running" {
                continue;
            }

            if state.items.is_empty() {
                continue;
            }

            let mut run_events = Vec::new();
            let mut processed_count = 0usize;
            let mut next_revision = state.latest_revision;

            while processed_count < max_items_per_loop {
                let item_id = state
                    .items
                    .iter()
                    .find(|(_, item)| {
                        if item.status != "assigned" {
                            return false;
                        }

                        if !item_affinity {
                            return true;
                        }

                        item.assigned_agent_id == Some(worker.agent_id)
                    })
                    .map(|(item_id, _)| *item_id);

                let item_id = match item_id {
                    Some(item_id) => item_id,
                    None => break,
                };

                let item_agent_id = state
                    .items
                    .get(&item_id)
                    .and_then(|item| item.assigned_agent_id)
                    .unwrap_or_else(|| worker.agent_id);

                let description = format!("item-{}", item_id); // In real flows we would carry the WorkItem description.

                if let Some(item_state) = state.items.get_mut(&item_id) {
                    if item_state.status != "assigned" {
                        continue;
                    }
                    item_state.status = "running".to_string();
                }

                // Delegate the actual "thinking + doing" to the harness.
                let ctx = ExecutionContext {
                    run_id: *run_id,
                    item_id,
                    agent_id: item_agent_id,
                    description,
                };
                let exec_result = match executor.execute(ctx).await {
                    Ok(r) => r,
                    Err(e) => {
                        warn!(error = %e, "executor failed for item");
                        // fall back to a failed item event
                        next_revision += 1;
                        run_events.push(worker.item_failed_event(
                            *run_id,
                            item_id,
                            next_revision,
                            e.to_string(),
                        ));
                        if let Some(item_state) = state.items.get_mut(&item_id) {
                            item_state.status = "failed".to_string();
                        }
                        state.status = "blocked".to_string();
                        processed_count += 1;
                        continue;
                    }
                };

                next_revision += 1;
                let exec_events = events_for_execution(
                    &worker,
                    *run_id,
                    item_id,
                    next_revision,
                    item_agent_id,
                    &exec_result,
                );
                // events_for_execution already advanced logical revisions; adjust next_revision from last produced
                if let Some(last) = exec_events.last() {
                    next_revision = last.revision;
                }
                run_events.extend(exec_events);

                if exec_result.success {
                    if let Some(item_state) = state.items.get_mut(&item_id) {
                        item_state.status = "completed".to_string();
                    }
                    state.total_cost += exec_result.cost;
                    state.effort_points += exec_result.effort;
                } else {
                    if let Some(item_state) = state.items.get_mut(&item_id) {
                        item_state.status = "failed".to_string();
                    }
                    state.status = "blocked".to_string();
                }

                state.latest_revision = next_revision;
                state.updated_at = Utc::now();
                state.status = if state.status == "blocked" {
                    "blocked".to_string()
                } else {
                    "running".to_string()
                };

                processed_count += 1;
            }

            if all_items_completed(state) {
                if state.status != "completed" {
                    next_revision += 1;
                    run_events.push(worker.complete_event(
                        *run_id,
                        next_revision,
                        state.completed_item_count(),
                    ));
                    state.status = "completed".to_string();
                    state.latest_revision = next_revision;
                    state.updated_at = Utc::now();
                }
            }

            if run_events.is_empty() {
                continue;
            }

            if let Err(error) = append_events_with_retry(
                &database_url,
                &mut event_store,
                &mut projection_store,
                *run_id,
                &run_events,
                append_max_attempts,
                append_backoff_ms,
            )
            .await
            {
                error!(run_id = %run_id, %error, "failed to append worker events");
                continue;
            }

            if let Err(error) =
                update_run_projections(&state, &projection_store, *run_id, &run_events).await
            {
                error!(run_id = %run_id, %error, "failed to upsert run projections");
            }

            did_work = true;
            info!(
                run_id = %run_id,
                status = %state.status,
                effort_points = state.effort_points,
                total_cost = state.total_cost,
                processed_items = processed_count,
                "worker advanced run"
            );
        }

        if !did_work {
            debug!("No eligible running work found in this worker tick");
        }

        sleep(Duration::from_millis(poll_ms)).await;
    }
}

#[derive(Debug, Default)]
struct RunState {
    workspace_id: Uuid,
    status: String,
    total_cost: f64,
    effort_points: i32,
    latest_revision: i32,
    updated_at: DateTime<Utc>,
    items: HashMap<Uuid, ItemState>,
}

#[derive(Debug, Default)]
struct ItemState {
    status: String,
    assigned_agent_id: Option<Uuid>,
}

impl RunState {
    fn completed_item_count(&self) -> i32 {
        self.items
            .values()
            .filter(|item| item.status == "completed")
            .count() as i32
    }
}

fn all_items_completed(state: &RunState) -> bool {
    !state.items.is_empty() && state.items.values().all(|item| item.status == "completed")
}

fn item_status_from_payload(
    event_payload: &EventPayload,
    fallback_agent_id: Option<Uuid>,
) -> Option<(Uuid, &'static str, Option<Uuid>)> {
    match event_payload {
        EventPayload::WorkItemAssigned {
            item_id, agent_id, ..
        } => Some((*item_id, "assigned", Some(*agent_id))),
        EventPayload::WorkItemStarted {
            item_id, agent_id, ..
        } => Some((*item_id, "running", Some(*agent_id))),
        EventPayload::WorkItemCompleted { item_id, .. } => {
            Some((*item_id, "completed", fallback_agent_id))
        }
        EventPayload::WorkItemFailed { item_id, .. } => {
            Some((*item_id, "failed", fallback_agent_id))
        }
        EventPayload::BlockerEncountered { item_id, .. } => {
            Some((*item_id, "blocked", fallback_agent_id))
        }
        _ => None,
    }
}

fn run_id_for_event(event: &EventEnvelope) -> Option<Uuid> {
    match &event.payload {
        EventPayload::RunStarted { .. } => Some(event.resource_id),
        EventPayload::RunStatusChanged { run_id, .. } => Some(*run_id),
        EventPayload::WorkItemAssigned { run_id, .. } => Some(*run_id),
        EventPayload::WorkItemStarted { run_id, .. } => Some(*run_id),
        EventPayload::WorkItemCompleted { run_id, .. } => Some(*run_id),
        EventPayload::WorkItemFailed { run_id, .. } => Some(*run_id),
        EventPayload::ProgressChunkEmitted { run_id, .. } => Some(*run_id),
        EventPayload::BlockerEncountered { run_id, .. } => Some(*run_id),
        EventPayload::RunCompleted { run_id, .. } => Some(*run_id),
        _ => None,
    }
}

fn run_states_from_events(events: &[EventEnvelope]) -> HashMap<Uuid, RunState> {
    let mut run_states = HashMap::new();

    for event in events {
        let run_id = match run_id_for_event(event) {
            Some(id) => id,
            None => continue,
        };

        let state = run_states.entry(run_id).or_insert_with(|| RunState {
            workspace_id: Uuid::nil(),
            status: "pending".to_string(),
            total_cost: 0.0,
            effort_points: 0,
            latest_revision: 0,
            updated_at: event.occurred_at,
            items: HashMap::new(),
        });

        state.latest_revision = state.latest_revision.max(event.revision);
        state.updated_at = event.occurred_at;

        match &event.payload {
            EventPayload::RunStarted { workspace_id, .. } => {
                state.workspace_id = *workspace_id;
                state.status = "running".to_string();
            }
            EventPayload::WorkItemAssigned {
                item_id, agent_id, ..
            } => {
                let item = state.items.entry(*item_id).or_insert_with(|| ItemState {
                    status: "assigned".to_string(),
                    assigned_agent_id: Some(*agent_id),
                });
                item.status = "assigned".to_string();
                item.assigned_agent_id = Some(*agent_id);
            }
            EventPayload::WorkItemStarted {
                item_id, agent_id, ..
            } => {
                let item = state.items.entry(*item_id).or_insert_with(|| ItemState {
                    status: "running".to_string(),
                    assigned_agent_id: Some(*agent_id),
                });
                item.status = "running".to_string();
                item.assigned_agent_id = Some(*agent_id);
            }
            EventPayload::WorkItemCompleted {
                item_id,
                effort,
                cost,
                ..
            } => {
                let item = state
                    .items
                    .entry(*item_id)
                    .or_insert_with(ItemState::default);
                item.status = "completed".to_string();
                state.total_cost += *cost;
                state.effort_points += *effort;
            }
            EventPayload::WorkItemFailed { item_id, .. } => {
                let item = state
                    .items
                    .entry(*item_id)
                    .or_insert_with(ItemState::default);
                item.status = "failed".to_string();
                state.status = "blocked".to_string();
            }
            EventPayload::ProgressChunkEmitted { effort, cost, .. } => {
                state.total_cost += *cost;
                state.effort_points += *effort;
            }
            EventPayload::RunStatusChanged { status, .. } => {
                state.status = status.clone();
            }
            EventPayload::RunCompleted { .. } => {
                state.status = "completed".to_string();
            }
            EventPayload::BlockerEncountered { .. } => {
                state.status = "blocked".to_string();
            }
            _ => {}
        }
    }

    run_states
}

async fn append_events_with_retry(
    database_url: &str,
    event_store: &mut PostgresEventStore,
    projection_store: &mut PostgresProjectionStore,
    run_id: Uuid,
    base_events: &[EventEnvelope],
    max_attempts: u32,
    mut backoff_ms: u64,
) -> Result<()> {
    let mut attempt = 0_u32;

    loop {
        attempt += 1;
        let latest_revision = match event_store.load(run_id).await {
            Ok(events) => events.last().map_or(0, |event| event.revision),
            Err(error) => {
                warn!(
                    run_id = %run_id,
                    attempt,
                    max_attempts,
                    delay_ms = backoff_ms,
                    error = %error,
                    "could not read current run revision before append"
                );

                if let Ok(new_store) = connect_with_retry(database_url).await {
                    *event_store = new_store;
                    *projection_store = PostgresProjectionStore::new(event_store.pool().clone());
                } else {
                    warn!(run_id = %run_id, "retrying append with existing event store after load failure");
                }

                if attempt >= max_attempts {
                    return Err(error);
                }

                sleep(Duration::from_millis(std::cmp::min(backoff_ms, 5_000))).await;
                backoff_ms = std::cmp::min(backoff_ms.saturating_mul(2), 5_000);
                continue;
            }
        };

        let events = with_rebased_revisions(base_events, latest_revision);

        match event_store.append(run_id, events.clone()).await {
            Ok(()) => return Ok(()),
            Err(error) => {
                warn!(
                    run_id = %run_id,
                    attempt,
                    max_attempts,
                    delay_ms = backoff_ms,
                    error = %error,
                    "append attempt failed"
                );

                if attempt >= max_attempts {
                    return Err(error);
                }

                if let Ok(new_store) = connect_with_retry(database_url).await {
                    *event_store = new_store;
                    *projection_store = PostgresProjectionStore::new(event_store.pool().clone());
                } else {
                    warn!(run_id = %run_id, "retrying append with existing store after connection failure");
                }

                sleep(Duration::from_millis(std::cmp::min(backoff_ms, 5_000))).await;
                backoff_ms = std::cmp::min(backoff_ms.saturating_mul(2), 5_000);
            }
        }
    }
}

fn with_rebased_revisions(events: &[EventEnvelope], latest_revision: i32) -> Vec<EventEnvelope> {
    events
        .iter()
        .enumerate()
        .map(|(index, event)| {
            let mut rebased = event.clone();
            rebased.revision = latest_revision + index as i32 + 1;
            rebased
        })
        .collect()
}

async fn update_run_projections(
    state: &RunState,
    projection_store: &PostgresProjectionStore,
    run_id: Uuid,
    run_events: &[EventEnvelope],
) -> Result<()> {
    for event in run_events {
        let assigned_agent = state
            .items
            .get(&match event.payload {
                EventPayload::WorkItemCompleted { item_id, .. }
                | EventPayload::WorkItemFailed { item_id, .. }
                | EventPayload::WorkItemStarted { item_id, .. }
                | EventPayload::WorkItemAssigned { item_id, .. }
                | EventPayload::ProgressChunkEmitted { item_id, .. } => item_id,
                _ => continue,
            })
            .and_then(|item| item.assigned_agent_id);

        if let Some((item_id, item_status, assigned_agent_id)) =
            item_status_from_payload(&event.payload, assigned_agent)
        {
            projection_store
                .upsert_work_item_projection(item_id, run_id, item_status, assigned_agent_id)
                .await?;
        }
    }

    if state.workspace_id.is_nil() {
        warn!(run_id = %run_id, "missing workspace id in reconstructed run state");
        return Ok(());
    }

    projection_store
        .upsert_run_projection(
            run_id,
            state.workspace_id,
            &state.status,
            state.total_cost,
            state.effort_points,
            state.updated_at,
        )
        .await?;

    Ok(())
}

async fn connect_with_retry(database_url: &str) -> Result<PostgresEventStore> {
    let mut attempts = 0_u32;
    let max_attempts = read_u32_env("WORKER_DB_CONNECT_ATTEMPTS", 12);
    let mut backoff_ms = read_u64_env("WORKER_DB_CONNECT_BACKOFF_MS", 250);
    let max_schema_attempts = read_u32_env("WORKER_DB_SCHEMA_ATTEMPTS", 10);
    let schema_backoff_ms = read_u64_env("WORKER_DB_SCHEMA_BACKOFF_MS", 250);

    loop {
        attempts += 1;
        match PostgresEventStore::connect(database_url).await {
            Ok(store) => {
                if let Err(err) =
                    wait_for_store_ready(&store, max_schema_attempts, schema_backoff_ms).await
                {
                    if attempts >= max_attempts {
                        return Err(err).context("Postgres ready check failed before timeout");
                    }

                    warn!(
                        attempt = attempts,
                        max = max_attempts,
                        delay_ms = backoff_ms,
                        error = %err,
                        "Postgres connected but event store schema is not ready"
                    );
                } else {
                    info!(
                        attempt = attempts,
                        "Connected to Postgres and validated event schema"
                    );
                    return Ok(store);
                }
            }
            Err(err) => {
                if attempts >= max_attempts {
                    return Err(err).context("failed to connect to Postgres before timeout");
                }

                warn!(
                    attempt = attempts,
                    max = max_attempts,
                    delay_ms = backoff_ms,
                    error = %err,
                    "Waiting for Postgres"
                );
            }
        }

        sleep(Duration::from_millis(backoff_ms)).await;
        backoff_ms = std::cmp::min(backoff_ms.saturating_mul(2), 5_000);
    }
}

async fn wait_for_store_ready(
    event_store: &PostgresEventStore,
    max_attempts: u32,
    initial_backoff_ms: u64,
) -> Result<()> {
    let mut attempts = 0_u32;
    let mut backoff_ms = initial_backoff_ms;

    loop {
        attempts += 1;

        match event_store.load(Uuid::nil()).await {
            Ok(_) => return Ok(()),
            Err(err) => {
                if attempts >= max_attempts {
                    return Err(err).context("event store schema not available");
                }

                warn!(
                    attempt = attempts,
                    max = max_attempts,
                    delay_ms = backoff_ms,
                    error = %err,
                    "Waiting for event store schema"
                );
                sleep(Duration::from_millis(backoff_ms)).await;
                backoff_ms = std::cmp::min(backoff_ms.saturating_mul(2), 5_000);
            }
        }
    }
}

fn read_u32_env(key: &str, default: u32) -> u32 {
    env::var(key)
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(default)
}

fn read_bool_env(key: &str, default: bool) -> bool {
    env::var(key)
        .ok()
        .map(|value| matches!(value.to_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(default)
}

fn read_u64_env(key: &str, default: u64) -> u64 {
    env::var(key)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(default)
}
