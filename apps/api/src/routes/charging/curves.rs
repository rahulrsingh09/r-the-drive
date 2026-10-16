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
    async fn count(&self, pool: &sqlx::PgPool, source: CurveSource) -> Result<i64, AppError> {
        let query = match source {
            CurveSource::Linked => sql::LINKED_COUNT,
            CurveSource::Minute => sql::MINUTE_COUNT,
        };
        let mut request = sqlx::query_scalar::<_, i64>(query).bind(self.vehicle_id);
        if matches!(source, CurveSource::Linked) {
            request = request.bind(self.session_id);
        }
        Ok(request
            .bind(self.started_at)
            .bind(self.ended_at)
            .fetch_one(pool)
            .await?)
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
    let source = if window.count(pool, CurveSource::Linked).await? >= 2 {
        CurveSource::Linked
    } else {
        CurveSource::Minute
    };
    let mut evidence = DetailEvidence {
        rows: window.detail_rows(pool, source).await?,
        source: source.label(),
    };
    if evidence.rows.len() < 2 {
        let stored = sqlx::query_as::<_, CurveRow>(sql::DETAIL_FALLBACK)
            .bind(vehicle_id)
            .bind(started_at)
            .bind(session_id)
            .bind(window.is_dc)
            .fetch_all(pool)
            .await?;
        if !stored.is_empty() {
            evidence = DetailEvidence {
                rows: stored,
                source: "rivian_charge_curve_points",
            };
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
    for source in [CurveSource::Linked, CurveSource::Minute] {
        if window.count(pool, source).await? < 2 {
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
    let stored = sqlx::query_as::<_, RawCurvePointRow>(sql::ANALYSIS_FALLBACK)
        .bind(vehicle_id)
        .bind(session_id)
        .fetch_all(pool)
        .await?;
    let Some((start, end)) = soc_start.zip(soc_end) else {
        return Ok(Vec::new());
    };
    Ok(build_fallback_curve_samples(
        session_id,
        started_at,
        ended_at,
        charger_type.as_deref(),
        start,
        end,
        &stored,
        if window.is_dc { 300.0 } else { 22.0 },
    ))
}
