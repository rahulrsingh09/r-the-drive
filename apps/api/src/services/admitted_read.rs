//! A heavy-read permit follows the database work, including cancellation.
use crate::{errors::AppError, services::resource_limits::ResourcePermit};
use sqlx::{Connection, PgConnection, PgPool, Postgres, Transaction};
use std::{
    ops::{Deref, DerefMut},
    time::Duration,
};

pub(crate) struct AdmittedRead {
    pool: PgPool,
    lease: Option<ReadLease>,
}

struct ReadLease {
    transaction: Transaction<'static, Postgres>,
    permit: ResourcePermit,
    backend_pid: Option<i32>,
}

impl AdmittedRead {
    pub(crate) async fn begin(
        pool: &PgPool,
        permit: ResourcePermit,
        timeout_seconds: u64,
    ) -> Result<Self, AppError> {
        let transaction = pool.begin().await?;
        let mut read = Self {
            pool: pool.clone(),
            lease: Some(ReadLease {
                transaction,
                permit,
                backend_pid: None,
            }),
        };
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *read)
            .await?;
        let (_, pid): (String, i32) =
            sqlx::query_as("SELECT set_config('statement_timeout', $1, true), pg_backend_pid()")
                .bind(format!("{}ms", timeout_seconds.saturating_mul(1000)))
                .fetch_one(&mut *read)
                .await?;
        read.lease
            .as_mut()
            .expect("active read transaction")
            .backend_pid = Some(pid);
        Ok(read)
    }

    pub(crate) async fn commit(mut self) -> Result<(), AppError> {
        let lease = self.lease.take().expect("active read transaction");
        let completion = tokio::spawn(lease.finish(None));
        match completion.await {
            Ok(result) => result.map_err(AppError::from),
            Err(error) => Err(AppError::Internal(anyhow::anyhow!(error))),
        }
    }
}

impl Deref for AdmittedRead {
    type Target = PgConnection;
    fn deref(&self) -> &Self::Target {
        &self
            .lease
            .as_ref()
            .expect("active read transaction")
            .transaction
    }
}

impl DerefMut for AdmittedRead {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self
            .lease
            .as_mut()
            .expect("active read transaction")
            .transaction
    }
}

impl Drop for AdmittedRead {
    fn drop(&mut self) {
        if let Some(lease) = self.lease.take() {
            let options = self.pool.connect_options();
            tokio::spawn(async move {
                if let Err(error) = lease.finish(Some(options)).await {
                    tracing::warn!(?error, "heavy read rollback failed");
                }
            });
        }
    }
}

impl ReadLease {
    async fn finish(
        self,
        cancel: Option<std::sync::Arc<sqlx::postgres::PgConnectOptions>>,
    ) -> sqlx::Result<()> {
        let Self {
            transaction,
            permit,
            backend_pid,
        } = self;
        let result = if let Some(options) = cancel {
            if let Some(pid) = backend_pid {
                let cancelled = tokio::time::timeout(Duration::from_secs(5), async {
                    let mut control = PgConnection::connect_with(&options).await?;
                    sqlx::query_scalar::<_, bool>("SELECT pg_cancel_backend($1)")
                        .bind(pid)
                        .fetch_one(&mut control)
                        .await?;
                    control.close().await
                })
                .await;
                if !matches!(cancelled, Ok(Ok(()))) {
                    tracing::warn!(
                        backend_pid = pid,
                        "heavy read cancellation failed; waiting for database deadline"
                    );
                }
            }
            transaction.rollback().await
        } else {
            transaction.commit().await
        };
        drop(permit);
        result
    }
}
