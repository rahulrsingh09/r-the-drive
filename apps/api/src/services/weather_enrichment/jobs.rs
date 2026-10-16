use super::*;

pub(super) struct WeatherJob {
    pub trip_id: Uuid,
    pub settings: ConnectionSettingsRow,
}

impl WeatherJob {
    pub async fn claim(pool: &PgPool) -> Result<Option<Self>> {
        weather::recover_interrupted_jobs(pool).await?;
        let settings = match connections::require_enabled(pool, connections::OPEN_METEO).await {
            Ok(settings) => settings,
            Err(crate::errors::AppError::ExternalConnectionDisabled(_)) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let requests: i32 = sqlx::query_scalar(
            "SELECT CASE WHEN usage_date = (now() AT TIME ZONE 'UTC')::date THEN request_count ELSE 0 END
             FROM panda.external_connection_activity WHERE connection_id=$1",
        ).bind(connections::OPEN_METEO).fetch_optional(pool).await?.unwrap_or(0);
        if requests >= DAILY_REQUEST_BUDGET {
            return Ok(None);
        }
        let trip_id = sqlx::query_scalar(
            "WITH candidate AS (
                 SELECT trip_id FROM panda.weather_enrichment_jobs
                 WHERE status IN ('pending','failed') AND attempts < 5 AND next_attempt_at <= now()
                 ORDER BY next_attempt_at, created_at
                 FOR UPDATE SKIP LOCKED LIMIT 1
             ) UPDATE panda.weather_enrichment_jobs j
               SET status='running', attempts=j.attempts+1, updated_at=now()
               FROM candidate c WHERE j.trip_id=c.trip_id RETURNING j.trip_id",
        )
        .fetch_optional(pool)
        .await?;
        Ok(trip_id.map(|trip_id| Self { trip_id, settings }))
    }

    pub async fn finish(self, pool: &PgPool, outcome: Result<EnrichTripOutcome>) -> Result<()> {
        let decision = JobDecision::from(outcome);
        let transition = match &decision {
            JobDecision::Complete { .. } => Transition::terminal("succeeded"),
            JobDecision::Unavailable => Transition::terminal("unavailable"),
            JobDecision::Paused => Transition::deferred("pending", RetryAt::Now, None),
            JobDecision::Busy => Transition::deferred("pending", RetryAt::After(60), None),
            JobDecision::Budget => Transition::deferred(
                "failed",
                RetryAt::Tomorrow,
                Some("Daily weather request budget reached"),
            ),
            JobDecision::Failure(error) => {
                connections::record_failure(pool, connections::OPEN_METEO, &error.to_string())
                    .await;
                let attempts: i32 = sqlx::query_scalar(
                    "SELECT attempts FROM panda.weather_enrichment_jobs WHERE trip_id=$1",
                )
                .bind(self.trip_id)
                .fetch_one(pool)
                .await?;
                let retry_after = error
                    .downcast_ref::<WeatherFetchError>()
                    .and_then(|error| error.retry_after_seconds)
                    .unwrap_or(0);
                let transition = Transition {
                    status: "failed",
                    refund_attempt: false,
                    completed: false,
                    retry: RetryAt::After(retry_seconds(attempts, retry_after)),
                    error: Some("Weather provider request failed"),
                };
                transition.store(pool, self.trip_id).await?;
                tracing::warn!(trip_id=%self.trip_id, attempts, "weather_enrichment.trip_failed");
                return Ok(());
            }
        };
        transition.store(pool, self.trip_id).await?;
        if matches!(
            decision,
            JobDecision::Complete {
                provider_called: true
            }
        ) {
            connections::record_success(pool, connections::OPEN_METEO).await;
        }
        Ok(())
    }
}

enum JobDecision {
    Complete { provider_called: bool },
    Unavailable,
    Paused,
    Busy,
    Budget,
    Failure(anyhow::Error),
}

impl From<Result<EnrichTripOutcome>> for JobDecision {
    fn from(outcome: Result<EnrichTripOutcome>) -> Self {
        match outcome {
            Ok(EnrichTripOutcome::Complete { provider_called }) => {
                Self::Complete { provider_called }
            }
            Ok(EnrichTripOutcome::Unavailable) => Self::Unavailable,
            Err(error) if error.is::<WeatherPausedError>() => Self::Paused,
            Err(error) if error.is::<WeatherBusyError>() => Self::Busy,
            Err(error) if error.is::<WeatherBudgetError>() => Self::Budget,
            Err(error) => Self::Failure(error),
        }
    }
}

#[derive(Debug, PartialEq)]
enum RetryAt {
    Keep,
    Now,
    After(i64),
    Tomorrow,
}

struct Transition {
    status: &'static str,
    refund_attempt: bool,
    completed: bool,
    retry: RetryAt,
    error: Option<&'static str>,
}

impl Transition {
    fn terminal(status: &'static str) -> Self {
        Self {
            status,
            refund_attempt: false,
            completed: true,
            retry: RetryAt::Keep,
            error: None,
        }
    }

    fn deferred(status: &'static str, retry: RetryAt, error: Option<&'static str>) -> Self {
        Self {
            status,
            refund_attempt: true,
            completed: false,
            retry,
            error,
        }
    }

    async fn store(&self, pool: &PgPool, trip_id: Uuid) -> Result<()> {
        let (schedule, delay) = match self.retry {
            RetryAt::Keep => ("keep", 0),
            RetryAt::Now => ("now", 0),
            RetryAt::After(seconds) => ("after", seconds),
            RetryAt::Tomorrow => ("tomorrow", 0),
        };
        sqlx::query(
            "UPDATE panda.weather_enrichment_jobs SET
                 status=$2,
                 attempts=CASE WHEN $3 THEN GREATEST(attempts-1,0) ELSE attempts END,
                 completed_at=CASE WHEN $4 THEN now() ELSE completed_at END,
                 next_attempt_at=CASE $5
                     WHEN 'now' THEN now()
                     WHEN 'after' THEN now()+make_interval(secs=>$6)
                     WHEN 'tomorrow' THEN date_trunc('day',now())+interval '1 day'
                     ELSE next_attempt_at END,
                 last_error=$7, updated_at=now()
             WHERE trip_id=$1",
        )
        .bind(trip_id)
        .bind(self.status)
        .bind(self.refund_attempt)
        .bind(self.completed)
        .bind(schedule)
        .bind(delay as f64)
        .bind(self.error)
        .execute(pool)
        .await?;
        Ok(())
    }
}

pub(super) fn retry_seconds(attempts: i32, retry_after: i64) -> i64 {
    let exponential = 60_i64
        .saturating_mul(2_i64.pow(attempts.min(8) as u32))
        .min(21_600);
    exponential.max(retry_after).min(86_400)
}
