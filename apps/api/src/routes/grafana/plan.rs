use super::*;
use crate::services::admitted_read::AdmittedRead;
use crate::services::resource_limits::ResourcePermit;
use std::sync::LazyLock;

// Plans are compiled exclusively from the metric allowlist, never request text.
static QUERIES: LazyLock<Vec<String>> = LazyLock::new(|| {
    ALLOWED_METRICS
        .iter()
        .map(|column| {
            format!(
                "SELECT (extract(epoch FROM ts) * 1000.0)::float8 AS ts_ms, {column}::float8 AS value \
         FROM timeseries.telemetry WHERE vehicle_id = $1 AND ts BETWEEN $2 AND $3 \
         AND {column} IS NOT NULL ORDER BY ts LIMIT $4"
            )
        })
        .collect()
});

pub(super) struct Selection {
    metric: usize,
    vehicle: Uuid,
}

pub(super) struct ReadPlan {
    selections: Vec<Selection>,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    limit: i64,
}

impl ReadPlan {
    pub(super) async fn admit(
        state: &AppState,
        auth: &AuthUser,
        fallback: Option<Uuid>,
        body: GrafanaQueryBody,
    ) -> Result<Self, AppError> {
        if body.targets.len() > 32 {
            return Err(AppError::Validation(
                "at most 32 Grafana targets may be requested".into(),
            ));
        }
        if body.range.from > body.range.to {
            return Err(AppError::Validation(
                "range.from must not exceed range.to".into(),
            ));
        }
        let mut selections = Vec::with_capacity(body.targets.len());
        for target in body.targets {
            let metric = ALLOWED_METRICS
                .iter()
                .position(|&name| name == target.target)
                .ok_or_else(|| AppError::Validation("unknown Grafana metric".into()))?;
            let vehicle = target
                .vehicle_id
                .or(fallback)
                .ok_or_else(|| AppError::Validation("vehicleId is required".into()))?;
            require_vehicle_read_access(&state.pool, auth, vehicle).await?;
            selections.push(Selection { metric, vehicle });
        }
        Ok(Self {
            selections,
            from: body.range.from,
            to: body.range.to,
            limit: body.max_data_points.unwrap_or(1000).clamp(1, 10_000),
        })
    }

    pub(super) async fn execute(
        self,
        state: &AppState,
        permit: ResourcePermit,
    ) -> Result<Json<Vec<TimeSeriesResult>>, AppError> {
        let mut tx = AdmittedRead::begin(
            &state.pool,
            permit,
            state.config.security.metrics_timeout_seconds,
        )
        .await?;
        let mut results = Vec::with_capacity(self.selections.len());
        // Repeat selections retain their output positions inside the admitted repeatable-read snapshot.
        for selection in self.selections {
            let rows = sqlx::query_as::<_, (f64, f64)>(sqlx::AssertSqlSafe(
                QUERIES[selection.metric].as_str(),
            ))
            .bind(selection.vehicle)
            .bind(self.from)
            .bind(self.to)
            .bind(self.limit)
            .fetch_all(&mut *tx)
            .await?;
            results.push(TimeSeriesResult {
                target: ALLOWED_METRICS[selection.metric].to_owned(),
                datapoints: rows
                    .into_iter()
                    .map(|(time, value)| [value, time])
                    .collect(),
            });
        }
        tx.commit().await?;
        Ok(Json(results))
    }
}
