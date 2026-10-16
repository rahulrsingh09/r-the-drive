use sqlx::{postgres::PgPoolOptions, PgPool};

pub async fn create_pool(database_url: &str) -> anyhow::Result<PgPool> {
    let pool = PgPoolOptions::new()
        .max_connections(20)
        .after_connect(|connection, _| {
            Box::pin(async move {
                // The application-owned schema contains cube/earthdistance.
                // Keep built-ins first and temporary types last; public retains
                // the existing TimescaleDB helpers. Migration pools stay separate.
                sqlx::query("SET search_path = pg_catalog, panda, public, pg_temp")
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect(database_url)
        .await?;
    Ok(pool)
}

#[cfg(test)]
#[path = "pool_tests.rs"]
mod tests;
