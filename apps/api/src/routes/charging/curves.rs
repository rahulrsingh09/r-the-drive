use super::*;
mod curve_queries;
use curve_queries as sql;

#[derive(Clone, Copy)]
enum CurveSource {
    Linked,
    Minute,
}
impl CurveSource {
    fn label(self) -> &'static str {
        match self {
            Self::Linked => "telemetry",
            Self::Minute => "telemetry_1min",
        }
    }
}

struct CurveWindow {
    vehicle_id: Uuid,
    session_id: Uuid,
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
    is_dc: bool,
}
impl CurveWindow {
    fn new(
        vehicle_id: Uuid,
        session_id: Uuid,
        started_at: DateTime<Utc>,
        ended_at: DateTime<Utc>,
        charger_type: Option<&str>,
    ) -> Self {
        Self {
            vehicle_id,
            session_id,
            started_at,
            ended_at,
            is_dc: matches!(
                charger_type.map(str::to_ascii_lowercase).as_deref(),
                Some("dc" | "dcfc")
            ),
        }
    }
    async fn linked_counts(&self, pool: &sqlx::PgPool) -> Result<(i64, i64), AppError> {
        Ok(sqlx::query_as::<_, (i64, i64)>(sql::LINKED_COUNT)
            .bind(self.vehicle_id)
            .bind(self.session_id)
            .bind(self.started_at)
            .bind(self.ended_at)
            .fetch_one(pool)
            .await?)
    }
    async fn count(&self, pool: &sqlx::PgPool, source: CurveSource) -> Result<i64, AppError> {
        match source {
            CurveSource::Linked => Ok(self.linked_counts(pool).await?.0),
            CurveSource::Minute => Ok(sqlx::query_scalar::<_, i64>(sql::MINUTE_COUNT)
                .bind(self.vehicle_id)
                .bind(self.started_at)
                .bind(self.ended_at)
                .fetch_one(pool)
                .await?),
        }
    }
    async fn detail_rows(
        &self,
        pool: &sqlx::PgPool,
        source: CurveSource,
    ) -> Result<Vec<CurveRow>, AppError> {
        let query = sql::bucket_query(source, false);
        let mut request =
            sqlx::query_as::<_, CurveRow>(sqlx::AssertSqlSafe(query)).bind(self.vehicle_id);
        if matches!(source, CurveSource::Linked) {
            request = request.bind(self.session_id);
        }
        Ok(request
            .bind(self.started_at)
            .bind(self.ended_at)
            .bind(self.is_dc)
            .fetch_all(pool)
            .await?)
    }
    async fn analysis_rows(
        &self,
        pool: &sqlx::PgPool,
        source: CurveSource,
    ) -> Result<Vec<CurveAnalysisValueRow>, AppError> {
        let query = match source {
            CurveSource::Linked => sql::ANALYSIS_LINKED.to_owned(),
            CurveSource::Minute => sql::bucket_query(source, true),
        };
        let mut request = sqlx::query_as::<_, CurveAnalysisValueRow>(sqlx::AssertSqlSafe(query))
            .bind(self.vehicle_id);
        if matches!(source, CurveSource::Linked) {
            request = request.bind(self.session_id);
        }
        Ok(request
            .bind(self.started_at)
            .bind(self.ended_at)
            .bind(self.is_dc)
            .fetch_all(pool)
            .await?)
    }
}

struct DetailEvidence {
    rows: Vec<CurveRow>,
    source: &'static str,
}
impl DetailEvidence {
    fn project(self) -> Vec<serde_json::Value> {
        self.rows.into_iter().map(|row|serde_json::json!({
            "minutes_elapsed":row.minutes_elapsed,"charge_rate_kw":row.charge_rate_kw,"soc":row.soc,
            "sample_source":row.sample_source.as_deref().unwrap_or(self.source),"power_method":row.power_method,
        })).collect()
    }
}

pub(super) async fn detail(
    pool: &sqlx::PgPool,
    vehicle_id: Uuid,
    session_id: Uuid,
    started_at: DateTime<Utc>,
    ended_at: Option<DateTime<Utc>>,
    charger_type: Option<&str>,
) -> Result<Vec<serde_json::Value>, AppError> {
    let window = CurveWindow::new(
        vehicle_id,
        session_id,
        started_at,
        ended_at.unwrap_or_else(Utc::now),
        charger_type,
    );
    let (linked_sample_count, linked_power_count) = window.linked_counts(pool).await?;

    // Telemetry power, where the vehicle sends it, has the finest resolution.
    // Without it (the R2), the recorded charging graph is measured power, and
    // a state-of-charge estimate is the last resort.
    let recorded = if linked_power_count < 2 {
        load_recorded_curve_points(pool, vehicle_id, session_id, started_at, window.ended_at)
            .await?
    } else {
        Vec::new()
    };
    if recorded.len() >= 2 {
        return Ok(recorded_curve_json(&recorded, started_at));
    }

    let source = if linked_sample_count >= 2 {
        CurveSource::Linked
    } else {
        CurveSource::Minute
    };
    let evidence = DetailEvidence {
        rows: window.detail_rows(pool, source).await?,
        source: source.label(),
    };
    if evidence.rows.len() < 2 {
        let fallback = if linked_power_count < 2 {
            recorded
        } else {
            load_recorded_curve_points(pool, vehicle_id, session_id, started_at, window.ended_at)
                .await?
        };
        if !fallback.is_empty() {
            return Ok(recorded_curve_json(&fallback, started_at));
        }
    }
    Ok(evidence.project())
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn analysis(
    pool: &sqlx::PgPool,
    vehicle_id: Uuid,
    session_id: Uuid,
    started_at: DateTime<Utc>,
    ended_at: Option<DateTime<Utc>>,
    charger_type: Option<String>,
    soc_start: Option<f64>,
    soc_end: Option<f64>,
) -> Result<Vec<CurveSampleRow>, AppError> {
    let Some(ended_at) = ended_at else {
        return Ok(Vec::new());
    };
    let window = CurveWindow::new(
        vehicle_id,
        session_id,
        started_at,
        ended_at,
        charger_type.as_deref(),
    );
    let (linked_sample_count, linked_power_count) = window.linked_counts(pool).await?;
    let recorded = if linked_power_count < 2 {
        load_recorded_curve_points(pool, vehicle_id, session_id, started_at, ended_at).await?
    } else {
        Vec::new()
    };
    let soc_bounds = soc_start.zip(soc_end);
    if recorded.len() >= 2
        && (soc_bounds.is_some() || recorded.iter().all(|point| point.soc.is_some()))
    {
        return Ok(build_fallback_curve_samples(
            session_id,
            started_at,
            ended_at,
            charger_type.as_deref(),
            soc_bounds,
            &recorded,
            crate::services::charging::PLAUSIBLE_MAX_CHARGE_KW,
        ));
    }

    for (source, count) in [
        (CurveSource::Linked, linked_sample_count),
        (
            CurveSource::Minute,
            window.count(pool, CurveSource::Minute).await?,
        ),
    ] {
        if count < 2 {
            continue;
        }
        let rows = window.analysis_rows(pool, source).await?;
        if rows.len() < 2 {
            continue;
        }
        return Ok(rows
            .into_iter()
            .map(|row| CurveSampleRow {
                session_id,
                minutes_elapsed: row.minutes_elapsed,
                charge_rate_kw: row.charge_rate_kw.unwrap_or(0.0),
                soc: row.soc,
                sample_source: source.label().to_owned(),
                charger_type: charger_type.clone(),
                power_method: if row.power_method.as_deref() == Some("recorded") {
                    "recorded"
                } else {
                    "soc_delta"
                },
            })
            .collect());
    }
    let fallback = if linked_power_count < 2 {
        recorded
    } else {
        load_recorded_curve_points(pool, vehicle_id, session_id, started_at, ended_at).await?
    };
    if fallback.is_empty()
        || (soc_bounds.is_none() && fallback.iter().any(|point| point.soc.is_none()))
    {
        return Ok(Vec::new());
    }
    Ok(build_fallback_curve_samples(
        session_id,
        started_at,
        ended_at,
        charger_type.as_deref(),
        soc_bounds,
        &fallback,
        crate::services::charging::PLAUSIBLE_MAX_CHARGE_KW,
    ))
}
