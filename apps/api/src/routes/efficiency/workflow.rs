use super::*;
use crate::routes::trip_tag_filter::TripTagFilter;

#[derive(Clone, Copy)]
pub(super) enum View {
    Summary,
    Mode,
    Temperature,
    Trend,
    RangeTemperature,
    Tags,
}

pub(super) struct Plan {
    vehicle: Uuid,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    tags: TripTagFilter,
    view: View,
}

impl Plan {
    pub(super) fn new(vehicle: Uuid, params: &Params, tags: TripTagFilter, view: View) -> Self {
        let days = match view {
            View::Summary | View::Trend => 90,
            View::Mode => 180,
            _ => 365,
        };
        let (from, to) = resolve_time_bounds(
            params.from,
            params.to,
            params.lifetime.unwrap_or(false),
            days,
        );
        Self {
            vehicle,
            from,
            to,
            tags,
            view,
        }
    }

    fn sql(&self) -> String {
        let cohort = format!(
            "WITH cohort AS (SELECT t.* FROM panda.trips t
            WHERE t.vehicle_id=$1 AND t.started_at >= $2 AND t.started_at <= $3
            AND t.distance_miles > 0 {})",
            sql_predicate("t", 4, 5, 6)
        );
        let (columns, eligibility, suffix) = match self.view {
            View::Summary => (
                "COALESCE(SUM(distance_miles) FILTER (WHERE efficiency_wh_per_mile IS NOT NULL),0)::float8 AS total_distance_miles,
                COALESCE(SUM(distance_miles * efficiency_wh_per_mile) FILTER (WHERE efficiency_wh_per_mile IS NOT NULL),0)::float8 AS weighted_efficiency_wh_mi,
                COALESCE(SUM(distance_miles),0)::float8 AS total_miles,
                COALESCE(SUM(distance_miles) FILTER (WHERE efficiency_wh_per_mile IS NOT NULL),0)::float8 AS efficiency_miles,
                PERCENTILE_CONT(0.1) WITHIN GROUP (ORDER BY efficiency_wh_per_mile) FILTER (WHERE efficiency_wh_per_mile IS NOT NULL) AS p10,
                PERCENTILE_CONT(0.9) WITHIN GROUP (ORDER BY efficiency_wh_per_mile) FILTER (WHERE efficiency_wh_per_mile IS NOT NULL) AS p90",
                "TRUE", ""),
            View::Mode => (
                "drive_mode, COUNT(*) AS trip_count, COALESCE(SUM(distance_miles),0)::float8 AS total_miles,
                (SUM(distance_miles * efficiency_wh_per_mile) / NULLIF(SUM(distance_miles),0))::float8 AS avg_wh_per_mi",
                "drive_mode IS NOT NULL AND efficiency_wh_per_mile IS NOT NULL", "GROUP BY drive_mode ORDER BY avg_wh_per_mi"),
            View::Temperature => (
                "(floor((outside_temp_c * 9.0/5.0 + 32) / 10.0) * 10 - 32) * 5.0/9.0 AS temp_c_low,
                ((floor((outside_temp_c * 9.0/5.0 + 32) / 10.0) + 1) * 10 - 32) * 5.0/9.0 AS temp_c_high,
                (SUM(distance_miles * efficiency_wh_per_mile) / NULLIF(SUM(distance_miles),0)) AS avg_efficiency_wh_mi,
                COUNT(*) AS trip_count, SUM(distance_miles) AS total_miles,
                CASE WHEN SUM(duration_seconds) > 0 THEN SUM(distance_miles) / (SUM(duration_seconds) / 3600.0) END AS avg_speed_mph",
                "outside_temp_c IS NOT NULL AND efficiency_wh_per_mile IS NOT NULL", "GROUP BY 1,2 ORDER BY 1"),
            View::Trend => (
                "started_at AS ts, efficiency_wh_per_mile AS trip_efficiency_wh_mi, distance_miles",
                "efficiency_wh_per_mile IS NOT NULL", "ORDER BY started_at"),
            View::RangeTemperature => (
                "id, distance_miles, efficiency_wh_per_mile, outside_temp_c AS avg_temp_c",
                "efficiency_wh_per_mile IS NOT NULL AND distance_miles > 1.0 AND outside_temp_c IS NOT NULL",
                "ORDER BY started_at DESC LIMIT 500"),
            View::Tags => return format!("{cohort}, grouped AS (
                SELECT tt.id AS tag_id, tt.name AS tag_name, c.id AS trip_id, c.distance_miles, c.efficiency_wh_per_mile
                FROM cohort c JOIN panda.trip_tag_assignments a ON a.trip_id=c.id JOIN panda.trip_tags tt ON tt.id=a.tag_id
                UNION ALL
                SELECT NULL::uuid, 'Untagged'::text, c.id, c.distance_miles, c.efficiency_wh_per_mile FROM cohort c
                WHERE NOT EXISTS (SELECT 1 FROM panda.trip_tag_assignments a WHERE a.trip_id=c.id)
                ) SELECT tag_id,tag_name,COUNT(*)::bigint AS trip_count,
                COALESCE(SUM(distance_miles),0)::float8 AS total_miles,
                COALESCE(SUM(distance_miles) FILTER (WHERE efficiency_wh_per_mile IS NOT NULL),0)::float8 AS efficiency_miles,
                COALESCE(SUM(distance_miles * efficiency_wh_per_mile) FILTER (WHERE efficiency_wh_per_mile IS NOT NULL),0)::float8 AS weighted_efficiency_wh_mi
                FROM grouped GROUP BY tag_id,tag_name ORDER BY tag_id IS NULL, lower(tag_name),tag_id"),
        };
        format!("{cohort} SELECT {columns} FROM cohort WHERE {eligibility} {suffix}")
    }

    pub(super) async fn rows<T>(&self, pool: &sqlx::PgPool) -> Result<Vec<T>, AppError>
    where
        T: for<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow> + Send + Unpin,
    {
        let sql = self.sql();
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(self.vehicle)
            .bind(self.from)
            .bind(self.to)
            .bind(&self.tags.tag_ids)
            .bind(self.tags.match_all)
            .bind(self.tags.untagged)
            .fetch_all(pool)
            .await?)
    }

    pub(super) async fn summary(&self, pool: &sqlx::PgPool) -> Result<serde_json::Value, AppError> {
        let sql = self.sql();
        let row: SummaryRow = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(self.vehicle)
            .bind(self.from)
            .bind(self.to)
            .bind(&self.tags.tag_ids)
            .bind(self.tags.match_all)
            .bind(self.tags.untagged)
            .fetch_one(pool)
            .await?;
        Ok(serde_json::json!({
            "avg_wh_per_mi": weighted_average_from_totals(row.total_distance_miles,row.weighted_efficiency_wh_mi),
            "total_miles": row.total_miles, "efficiency_miles": row.efficiency_miles,
            "coverage_percent": if row.total_miles > 0.0 {row.efficiency_miles/row.total_miles*100.0} else {0.0},
            "p10_wh_per_mi": row.p10.unwrap_or(0.0), "p90_wh_per_mi": row.p90.unwrap_or(0.0),
        }))
    }
}

#[derive(Default)]
pub(super) struct RollingEfficiency {
    window: VecDeque<TrendSample>,
    distance: f64,
    weighted: f64,
}

impl RollingEfficiency {
    pub(super) fn observe(&mut self, sample: TrendSample) -> TrendPoint {
        let ts = sample.ts;
        let efficiency = sample.trip_efficiency_wh_mi;
        self.distance += sample.distance_miles;
        self.weighted += sample.distance_miles * efficiency;
        self.window.push_back(sample);
        let cutoff = ts - chrono::Duration::hours(24);
        while self.window.front().is_some_and(|sample| sample.ts < cutoff) {
            let expired = self.window.pop_front().unwrap();
            self.distance -= expired.distance_miles;
            self.weighted -= expired.distance_miles * expired.trip_efficiency_wh_mi;
        }
        TrendPoint {
            ts,
            trip_efficiency_wh_mi: Some(efficiency),
            rolling_24h_wh_mi: (self.distance > 0.0).then_some(self.weighted / self.distance),
        }
    }
}
