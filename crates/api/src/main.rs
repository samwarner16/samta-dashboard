mod dto;
mod routes;
mod ws;

use std::env;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use application::orchestrator::Orchestrator;
use infra::persistence::EventStore;
use infra::{migrations::run_migrations, persistence::PostgresEventStore, ws_gateway::WsGateway};
use tokio::time::sleep;
use tower_http::cors::CorsLayer;
use tracing::{info, warn};
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let database_url = env::var("DATABASE_URL").context("DATABASE_URL must be set")?;
    let event_store = connect_with_retry(&database_url)
        .await
        .context("connect to database")?;

    run_migrations(event_store.pool())
        .await
        .context("run SQLx migrations")?;

    let event_store_pool = event_store.pool().clone();

    let ws_gateway = Arc::new(WsGateway::new(100));
    let orchestrator = Arc::new(Orchestrator::new(Arc::new(event_store), ws_gateway.clone()));

    let app = routes::create_router(orchestrator, ws_gateway, event_store_pool);
    let app = app.layer(CorsLayer::permissive());

    let host = env::var("API_HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port = env::var("API_PORT").unwrap_or_else(|_| "8080".to_string());
    let addr: SocketAddr = format!("{host}:{port}")
        .parse()
        .context("invalid API host/port")?;

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind API listener at {addr}"))?;
    tracing::info!("API listening on {addr}");
    axum::serve(listener, app)
        .await
        .context("api serve failed")?;

    Ok(())
}

async fn connect_with_retry(database_url: &str) -> Result<PostgresEventStore> {
    let mut attempts = 0_u32;
    let max_attempts = env::var("API_DB_CONNECT_ATTEMPTS")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(12);
    let mut backoff_ms = env::var("API_DB_CONNECT_BACKOFF_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(250);
    let max_schema_attempts = env::var("API_DB_SCHEMA_ATTEMPTS")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(10);
    let schema_backoff_ms = env::var("API_DB_SCHEMA_BACKOFF_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(250);

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
                        "Postgres connected but event schema is not ready"
                    );
                    sleep(Duration::from_millis(backoff_ms)).await;
                    backoff_ms = std::cmp::min(backoff_ms.saturating_mul(2), 5_000);
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
                sleep(Duration::from_millis(backoff_ms)).await;
                backoff_ms = std::cmp::min(backoff_ms.saturating_mul(2), 5_000);
            }
        }
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
