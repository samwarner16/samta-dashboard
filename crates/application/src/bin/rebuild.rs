use std::env;

use anyhow::Result;
use infra::{migrations::run_migrations, persistence::{PostgresEventStore, PostgresProjectionStore}, replay::rebuild_projections};

#[tokio::main]
async fn main() -> Result<()> {
    let database_url = env::var("DATABASE_URL")?;

    let event_store = PostgresEventStore::connect(&database_url).await?;
    run_migrations(event_store.pool()).await?;

    let projection_pool = event_store.pool().clone();
    let projection_store = PostgresProjectionStore::new(projection_pool);

    let rebuilt_runs = rebuild_projections(&event_store, &projection_store).await?;
    println!("Rebuilt projections for {rebuilt_runs} runs");

    Ok(())
}
