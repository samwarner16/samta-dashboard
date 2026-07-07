use anyhow::Result;
use sqlx::PgPool;

pub async fn run_migrations(pool: &PgPool) -> Result<()> {
    // Workspace keeps migrations at the repository root, so travel up from
    // crates/infra to the workspace root.
    sqlx::migrate!("../../migrations").run(pool).await?;
    Ok(())
}
