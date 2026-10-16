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
        let query = sqlx::query(statement);
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
        Ok(sqlx::query_as("FETCH FORWARD 256 FROM maintenance_rows")
            .fetch_all(&mut *self.transaction)
            .await?)
    }

    pub(super) async fn finish(self) -> anyhow::Result<()> {
        self.transaction.commit().await?;
        Ok(())
    }
}
