use std::env;

use anyhow::Result;
use infra::{migrations::run_migrations, persistence::{PostgresEventStore, PostgresProjectionStore}, replay::rebuild_projections};
use anyhow::Context;

#[tokio::main]
async fn main() -> Result<()> {
    let database_url = env::var("DATABASE_URL")?;
    let event_store = PostgresEventStore::connect(&database_url)
        .await
        .context("rebuild: connect to Postgres")?;
    run_migrations(event_store.pool())
        .await
        .context("rebuild: run migrations")?;

    let projection_pool = event_store.pool().clone();
    let projection_store = PostgresProjectionStore::new(projection_pool);

    let rebuilt_runs = rebuild_projections(&event_store, &projection_store)
        .await
        .context("rebuild: replay projections")?;
    println!("Rebuilt projections for {rebuilt_runs} runs");

    Ok(())
}
