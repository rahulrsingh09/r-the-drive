use sqlx::{postgres::PgPoolOptions, PgPool};

/// Shared runtime connection settings; callers retain their own pool limits.
/// Migration and restore connections keep their separate initialization.
pub fn application_pool_options() -> PgPoolOptions {
    PgPoolOptions::new().after_connect(|connection, _| {
        Box::pin(async move {
            // Resolve trusted geospatial extensions before temporary types.
            sqlx::query("SET search_path = pg_catalog, panda, public, pg_temp")
                .execute(connection)
                .await?;
            Ok(())
        })
    })
}

pub async fn create_pool(database_url: &str) -> anyhow::Result<PgPool> {
    let pool = application_pool_options()
        .max_connections(20)
        .connect(database_url)
        .await?;
    Ok(pool)
}

#[cfg(test)]
#[path = "pool_tests.rs"]
mod tests;
