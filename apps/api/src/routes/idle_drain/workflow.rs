use super::*;

#[derive(sqlx::FromRow)]
struct Activity {
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
}

#[derive(Default)]
struct Gaps {
    starts: Vec<DateTime<Utc>>,
    ends: Vec<DateTime<Utc>>,
}

impl Gaps {
    fn between(activities: Vec<Activity>) -> Self {
        let mut gaps = Self::default();
        let mut covered_until: Option<DateTime<Utc>> = None;
        for activity in activities {
            if let Some(end) = covered_until {
                if activity.started_at > end {
                    gaps.starts.push(end);
                    gaps.ends.push(activity.started_at);
                    covered_until = Some(activity.ended_at);
                    continue;
                }
            }
            covered_until =
                Some(covered_until.map_or(activity.ended_at, |end| end.max(activity.ended_at)));
        }
        gaps
    }
}

pub(super) struct ParkedHistory {
    pub(super) vehicle: Uuid,
    pub(super) from: DateTime<Utc>,
    pub(super) to: DateTime<Utc>,
    pub(super) minimum_hours: f64,
    pub(super) limit: usize,
    pub(super) include_excluded: bool,
}

impl ParkedHistory {
    pub(super) async fn execute(self, pool: &sqlx::PgPool) -> Result<Vec<PhantomPeriod>, AppError> {
        let mut snapshot = pool.begin().await?;
        // Gap selection and its overlays must observe the same history snapshot.
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *snapshot)
            .await?;
        let activities = sqlx::query_as::<_, Activity>(
            "SELECT started_at, ended_at FROM (
                SELECT started_at, ended_at FROM panda.trips WHERE vehicle_id=$1 AND ended_at IS NOT NULL
                    AND started_at < $3 + ($4 * INTERVAL '1 hour') AND ended_at > $2 - ($4 * INTERVAL '1 hour')
                UNION ALL
                SELECT started_at, ended_at FROM panda.charge_sessions WHERE vehicle_id=$1 AND ended_at IS NOT NULL
                    AND started_at < $3 + ($4 * INTERVAL '1 hour') AND ended_at > $2 - ($4 * INTERVAL '1 hour')
            ) activity ORDER BY started_at, ended_at")
            .bind(self.vehicle).bind(self.from).bind(self.to).bind(ACTIVE_PADDING_HOURS)
            .fetch_all(&mut *snapshot).await?;
        let gaps = Gaps::between(activities);
        let evidence = sqlx::query_as::<_, PhantomCandidateRow>(super::evidence::GAP_EVIDENCE)
            .bind(self.vehicle)
            .bind(self.from)
            .bind(self.to)
            .bind(self.minimum_hours)
            .bind(gaps.starts)
            .bind(gaps.ends)
            .fetch_all(&mut *snapshot)
            .await?;
        snapshot.commit().await?;
        Ok(evidence
            .into_iter()
            .map(derive_phantom_period)
            .filter(|period| {
                self.include_excluded || period.validation_status == ValidationStatus::Validated
            })
            .take(self.limit)
            .collect())
    }
}
