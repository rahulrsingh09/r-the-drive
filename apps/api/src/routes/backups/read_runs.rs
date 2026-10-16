use super::*;
use sqlx::{Postgres, QueryBuilder};

pub(super) enum Selection {
    Page { page: u32, per_page: u32 },
    Id(Uuid),
    LatestSuccess,
}
impl Selection {
    fn query(&self) -> QueryBuilder<Postgres> {
        let mut query = QueryBuilder::new("SELECT id, trigger, status, phase, progress_percent, artifact_key, started_at, completed_at, error_message, created_at, updated_at FROM panda.backup_runs");
        match self {
            Self::Id(id) => {
                query.push(" WHERE id = ").push_bind(*id);
            }
            Self::LatestSuccess => {
                query.push(" WHERE status = 'succeeded' ORDER BY completed_at DESC NULLS LAST, created_at DESC LIMIT 1");
            }
            Self::Page { page, per_page } => {
                query
                    .push(" ORDER BY created_at DESC LIMIT ")
                    .push_bind(i64::from(*per_page))
                    .push(" OFFSET ")
                    .push_bind(i64::from(page.saturating_sub(1)) * i64::from(*per_page));
            }
        }
        query
    }
    pub(super) async fn many(self, state: &AppState) -> Result<Vec<BackupRunResponse>, AppError> {
        self.query()
            .build_query_as::<BackupRunRow>()
            .fetch_all(&state.pool)
            .await?
            .into_iter()
            .map(map_run_row)
            .collect()
    }
    pub(super) async fn optional(
        self,
        state: &AppState,
    ) -> Result<Option<BackupRunResponse>, AppError> {
        self.query()
            .build_query_as::<BackupRunRow>()
            .fetch_optional(&state.pool)
            .await?
            .map(map_run_row)
            .transpose()
    }
}
