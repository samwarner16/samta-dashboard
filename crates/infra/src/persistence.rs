use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use domain::entities::{AgentRunProjection, WorkItemProjection};
use events::EventEnvelope;
use sqlx::{postgres::{PgPoolOptions, PgRow}, PgPool, Row, types::Json};
use uuid::Uuid;

#[async_trait]
pub trait EventStore: Send + Sync {
    async fn append(&self, aggregate_id: Uuid, events: Vec<EventEnvelope>) -> Result<()>;
    async fn load(&self, aggregate_id: Uuid) -> Result<Vec<EventEnvelope>>;
    async fn load_all(&self) -> Result<Vec<EventEnvelope>>;
}

#[async_trait]
pub trait ProjectionStore: Send + Sync {
    async fn get_run(&self, run_id: Uuid) -> Result<Option<AgentRunProjection>>;
    async fn get_work_items(&self, run_id: Uuid) -> Result<Vec<WorkItemProjection>>;
    async fn upsert_run_projection(
        &self,
        run_id: Uuid,
        workspace_id: Uuid,
        status: &str,
        total_cost: f64,
        effort_points: i32,
        updated_at: DateTime<Utc>,
    ) -> Result<()>;
    async fn upsert_work_item_projection(
        &self,
        item_id: Uuid,
        run_id: Uuid,
        status: &str,
        assigned_agent_id: Option<Uuid>,
    ) -> Result<()>;
    async fn clear(&self) -> Result<()>;
}

#[derive(Debug, Clone)]
pub struct PostgresEventStore {
    pool: PgPool,
}

impl PostgresEventStore {
    pub async fn connect(database_url: &str) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await?;
        Ok(Self { pool })
    }

    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

#[async_trait]
impl EventStore for PostgresEventStore {
    async fn append(&self, aggregate_id: Uuid, events: Vec<EventEnvelope>) -> Result<()> {
        if events.is_empty() {
            return Ok(());
        }

        let mut tx = self.pool.begin().await?;

        let latest: i32 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(revision), 0) FROM event_log WHERE resource_id = $1",
        )
            .bind(aggregate_id)
            .fetch_one(&mut *tx)
            .await?;

        let mut expected = latest;
        for event in events {
            expected += 1;
            if event.revision != expected {
                return Err(anyhow!(
                    "invalid event revision for aggregate {}: expected {} but got {}",
                    aggregate_id,
                    expected,
                    event.revision
                ));
            }

            let event_type = event.event_type().as_str().to_string();
            let payload_json = serde_json::to_value(&event.payload)?;

            sqlx::query(
                "INSERT INTO event_log (id, occurred_at, actor_id, resource_id, correlation_id, causation_id, revision, metadata, event_type, payload) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
            )
            .bind(event.id)
            .bind(event.occurred_at)
            .bind(event.actor_id)
            .bind(aggregate_id)
            .bind(event.correlation_id)
            .bind(event.causation_id)
            .bind(event.revision)
            .bind(Json(event.metadata))
            .bind(event_type)
            .bind(Json(payload_json))
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    async fn load(&self, aggregate_id: Uuid) -> Result<Vec<EventEnvelope>> {
        let rows: Vec<PgRow> = sqlx::query(
            "SELECT id, occurred_at, actor_id, resource_id, correlation_id, causation_id, revision, metadata, payload \
             FROM event_log WHERE resource_id = $1 ORDER BY revision ASC",
        )
        .bind(aggregate_id)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(event_from_row).collect()
    }

    async fn load_all(&self) -> Result<Vec<EventEnvelope>> {
        let rows: Vec<PgRow> = sqlx::query(
            "SELECT id, occurred_at, actor_id, resource_id, correlation_id, causation_id, revision, metadata, payload \
             FROM event_log ORDER BY occurred_at ASC, revision ASC",
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(event_from_row).collect()
    }
}

#[derive(Debug, Clone)]
pub struct PostgresProjectionStore {
    pool: PgPool,
}

impl PostgresProjectionStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ProjectionStore for PostgresProjectionStore {
    async fn get_run(&self, run_id: Uuid) -> Result<Option<AgentRunProjection>> {
        let row = sqlx::query("SELECT run_id, workspace_id, status, total_cost, effort_points, updated_at \
                               FROM agent_runs_projection WHERE run_id = $1")
            .bind(run_id)
            .fetch_optional(&self.pool)
            .await?;

        if let Some(row) = row {
            let projection = AgentRunProjection {
                run_id: row.try_get("run_id")?,
                workspace_id: row.try_get("workspace_id")?,
                status: row.try_get("status")?,
                total_cost: row.try_get("total_cost")?,
                effort_points: row.try_get("effort_points")?,
                updated_at: row.try_get("updated_at")?,
            };
            Ok(Some(projection))
        } else {
            Ok(None)
        }
    }

    async fn get_work_items(&self, run_id: Uuid) -> Result<Vec<WorkItemProjection>> {
        let rows: Vec<PgRow> = sqlx::query(
            "SELECT item_id, run_id, status, assigned_agent_id FROM work_items_projection WHERE run_id = $1",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                Ok(WorkItemProjection {
                    item_id: row.try_get("item_id")?,
                    run_id: row.try_get("run_id")?,
                    status: row.try_get("status")?,
                    assigned_agent_id: row.try_get("assigned_agent_id")?,
                })
            })
            .collect()
    }

    async fn upsert_run_projection(
        &self,
        run_id: Uuid,
        workspace_id: Uuid,
        status: &str,
        total_cost: f64,
        effort_points: i32,
        updated_at: DateTime<Utc>,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO agent_runs_projection (run_id, workspace_id, status, total_cost, effort_points, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (run_id)
             DO UPDATE SET workspace_id = EXCLUDED.workspace_id, status = EXCLUDED.status,
                           total_cost = EXCLUDED.total_cost, effort_points = EXCLUDED.effort_points,
                           updated_at = EXCLUDED.updated_at",
        )
        .bind(run_id)
        .bind(workspace_id)
        .bind(status)
        .bind(total_cost)
        .bind(effort_points)
        .bind(updated_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn upsert_work_item_projection(
        &self,
        item_id: Uuid,
        run_id: Uuid,
        status: &str,
        assigned_agent_id: Option<Uuid>,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO work_items_projection (item_id, run_id, status, assigned_agent_id)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (item_id)
             DO UPDATE SET run_id = EXCLUDED.run_id,
                           status = EXCLUDED.status,
                           assigned_agent_id = EXCLUDED.assigned_agent_id",
        )
        .bind(item_id)
        .bind(run_id)
        .bind(status)
        .bind(assigned_agent_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn clear(&self) -> Result<()> {
        sqlx::query("DELETE FROM work_items_projection")
            .execute(&self.pool)
            .await?;

        sqlx::query("DELETE FROM agent_runs_projection")
            .execute(&self.pool)
            .await?;

        Ok(())
    }
}

fn event_from_row(row: PgRow) -> Result<EventEnvelope> {
    let payload: serde_json::Value = row.try_get("payload")?;
    let payload = serde_json::from_value(payload)?;

    Ok(EventEnvelope {
        id: row.try_get("id")?,
        occurred_at: row.try_get("occurred_at")?,
        actor_id: row.try_get("actor_id")?,
        resource_id: row.try_get("resource_id")?,
        correlation_id: row.try_get("correlation_id")?,
        causation_id: row.try_get("causation_id")?,
        revision: row.try_get("revision")?,
        metadata: row.try_get::<serde_json::Value, _>("metadata")?,
        payload,
    })
}
