use sqlx::{postgres::PgRow, FromRow, PgPool, Postgres, Transaction};
use uuid::Uuid;

pub(super) struct SnapshotScan {
    transaction: Transaction<'static, Postgres>,
}

impl SnapshotScan {
    pub(super) async fn open(
        pool: &PgPool,
        select: &'static str,
        vehicle_id: Option<Uuid>,
    ) -> anyhow::Result<Self> {
        let mut transaction = pool.begin().await?;
        // Full-history maintenance consumes the entire cursor, including on small pages.
        sqlx::query("SET LOCAL cursor_tuple_fraction = 1.0")
            .execute(&mut *transaction)
            .await?;
        let statement = sqlx::AssertSqlSafe(format!(
            "DECLARE maintenance_rows NO SCROLL CURSOR FOR {select}"
        ));
        let query = sqlx::query(statement).persistent(false);
        let query = match vehicle_id {
            Some(id) => query.bind(id),
            None => query,
        };
        query.execute(&mut *transaction).await?;
        Ok(Self { transaction })
    }

    pub(super) async fn next<T>(&mut self) -> anyhow::Result<Vec<T>>
    where
        T: for<'row> FromRow<'row, PgRow> + Send + Unpin,
    {
        // Cursor row shapes differ between maintenance stages on the same connection.
        Ok(sqlx::query_as("FETCH FORWARD 256 FROM maintenance_rows")
            .persistent(false)
            .fetch_all(&mut *self.transaction)
            .await?)
    }

    pub(super) async fn finish(self) -> anyhow::Result<()> {
        self.transaction.commit().await?;
        Ok(())
    }
}

#[cfg(test)]
#[allow(dead_code)]
pub(super) mod full_replacement_db {
    use super::*;
    use sqlx::Executor;

    pub struct Database {
        pub pool: PgPool,
        admin: PgPool,
        name: String,
    }

    impl Database {
        pub async fn new(schema: &'static str) -> Self {
            let mut url = url::Url::parse(
                &std::env::var("MAINTENANCE_DATABASE_URL")
                    .expect("explicit synthetic fixture required"),
            )
            .unwrap();
            assert_eq!(url.host_str(), Some("db"));
            assert_eq!(url.username(), "maintenance_bootstrap");
            url.set_path("/postgres");
            let admin = PgPool::connect(url.as_str()).await.unwrap();
            let name = format!("maintenance_algorithm_{}", Uuid::new_v4().simple());
            admin
                .execute(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
                .await
                .unwrap();
            url.set_path(&format!("/{name}"));
            let pool = PgPool::connect(url.as_str()).await.unwrap();
            sqlx::raw_sql(schema).execute(&pool).await.unwrap();
            Self { pool, admin, name }
        }

        pub async fn cleanup(self) {
            self.pool.close().await;
            self.admin
                .execute(sqlx::AssertSqlSafe(format!(
                    "DROP DATABASE {} WITH(FORCE)",
                    self.name
                )))
                .await
                .unwrap();
            self.admin.close().await;
        }
    }
}
