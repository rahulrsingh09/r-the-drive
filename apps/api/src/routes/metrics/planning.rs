use super::*;
use sqlx::Row;

#[derive(Clone, Copy)]
enum Relation {
    Trips,
    Charges,
    Telemetry(&'static str),
}

#[derive(Clone, Copy)]
enum Measure {
    Distance,
    OdometerDistance,
    Count,
    ChargedEnergy,
    Cost,
    SessionEnergy,
    Efficiency,
    GrossEfficiency,
    Temperature,
    Duration,
    Sensor(&'static str),
}

#[derive(Clone, Copy)]
struct Evidence {
    relation: Relation,
    measure: Measure,
}

impl Evidence {
    fn select(metric: &MetricDef) -> Result<Self, AppError> {
        let (relation, measure) = match metric.source {
            MetricSource::Telemetry(column) => {
                if !ALLOWED_TELEMETRY_COLUMNS.contains(&column) {
                    return Err(AppError::Validation(format!(
                        "unknown telemetry column: {column}"
                    )));
                }
                (
                    Relation::Telemetry(metric.default_aggregation),
                    Measure::Sensor(column),
                )
            }
            MetricSource::Summary => match metric.id {
                "total_miles" => (Relation::Trips, Measure::OdometerDistance),
                "trip_miles" => (Relation::Trips, Measure::Distance),
                "total_trips" => (Relation::Trips, Measure::Count),
                "energy_charged" => (Relation::Charges, Measure::ChargedEnergy),
                "charging_sessions" => (Relation::Charges, Measure::Count),
                "total_cost" => (Relation::Charges, Measure::Cost),
                "avg_session_energy" => (Relation::Charges, Measure::SessionEnergy),
                "avg_efficiency" => (Relation::Trips, Measure::Efficiency),
                "avg_gross_efficiency" => (Relation::Trips, Measure::GrossEfficiency),
                "avg_outside_temp_c" => (Relation::Trips, Measure::Temperature),
                "avg_trip_duration" => (Relation::Trips, Measure::Duration),
                _ => return Err(AppError::Validation("unsupported series metric".into())),
            },
        };
        Ok(Self { relation, measure })
    }
}

#[derive(Clone, Copy)]
pub(super) enum Projection {
    Scalar,
    Weighted,
}

impl Projection {
    pub(super) fn point(self, row: &sqlx::postgres::PgRow) -> Result<MetricSeriesPoint, AppError> {
        Ok(MetricSeriesPoint {
            ts: row.try_get("ts")?,
            value: match self {
                Self::Scalar => row.try_get("value")?,
                Self::Weighted => {
                    let evidence = <WeightedEfficiencyRow as sqlx::FromRow<
                        '_,
                        sqlx::postgres::PgRow,
                    >>::from_row(row)?;
                    weighted_average_from_totals(
                        evidence.total_distance_miles,
                        evidence.weighted_efficiency_wh_mi,
                    )
                }
            },
        })
    }
}

#[derive(Clone, Copy)]
enum Span<'a> {
    Latest,
    Series(&'a str),
}

pub(super) struct QueryPlan {
    pub(super) sql: String,
    pub(super) projection: Projection,
    pub(super) filtered: bool,
    latest_sensor: bool,
}

impl QueryPlan {
    pub(super) fn latest(metric: &MetricDef, filter: &TripTagFilter) -> Result<Self, AppError> {
        Self::compile(Evidence::select(metric)?, Span::Latest, filter)
    }

    pub(super) fn series(
        metric: &MetricDef,
        bucket: &str,
        filter: &TripTagFilter,
    ) -> Result<Self, AppError> {
        Self::compile(Evidence::select(metric)?, Span::Series(bucket), filter)
    }

    pub(super) fn bind<'a>(
        &'a self,
        vehicle: Uuid,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        filter: &TripTagFilter,
    ) -> sqlx::query::Query<'a, sqlx::Postgres, sqlx::postgres::PgArguments> {
        let mut query = sqlx::query(sqlx::AssertSqlSafe(self.sql.as_str())).bind(vehicle);
        if !self.latest_sensor {
            query = query.bind(from).bind(to);
        }
        if self.filtered {
            query = query
                .bind(filter.tag_ids.clone())
                .bind(filter.match_all)
                .bind(filter.untagged);
        }
        query
    }

    fn compile(
        evidence: Evidence,
        span: Span<'_>,
        filter: &TripTagFilter,
    ) -> Result<Self, AppError> {
        let filtered = matches!(evidence.relation, Relation::Trips) && filter.is_active();
        let latest = matches!(span, Span::Latest);
        let raw = matches!(span, Span::Series("raw"));
        let telemetry = matches!(evidence.relation, Relation::Telemetry(_));
        let latest_sensor = telemetry && latest;
        let grouped = !latest && (!raw || filtered);
        let projection =
            if matches!(evidence.measure, Measure::Efficiency) && (latest || !raw || filtered) {
                Projection::Weighted
            } else {
                Projection::Scalar
            };
        if matches!(evidence.measure, Measure::OdometerDistance) && grouped && !filtered {
            return Ok(Self {
                sql: odometer_history().into(),
                projection,
                filtered,
                latest_sensor,
            });
        }
        let table = match evidence.relation {
            Relation::Trips if filtered => "filtered_trips",
            Relation::Trips => "panda.trips",
            Relation::Charges => "panda.charge_sessions",
            Relation::Telemetry(_) => "timeseries.telemetry",
        };
        let time_column = if telemetry { "ts" } else { "started_at" };
        let timestamp = match span {
            Span::Latest if latest_sensor => "ts".into(),
            Span::Latest => "$3".into(),
            Span::Series("raw") => time_column.into(),
            Span::Series(bucket) => bucket_expression(bucket, time_column),
        };
        let (expression, condition) = expression(evidence, latest, raw, filtered)?;
        let select = match projection {
            Projection::Scalar => format!("{expression} AS value"),
            Projection::Weighted => expression,
        };
        let mut predicates = Vec::new();
        if !filtered {
            predicates.push("vehicle_id = $1".to_owned());
            if !latest_sensor {
                predicates.push(format!("{time_column} >= $2 AND {time_column} <= $3"));
            }
        }
        if let Some(condition) = condition {
            predicates.push(condition);
        }
        let predicate = if predicates.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", predicates.join(" AND "))
        };
        let order = if latest_sensor {
            " ORDER BY ts DESC LIMIT 1"
        } else if grouped {
            " GROUP BY 1 ORDER BY 1"
        } else if !latest && telemetry {
            " ORDER BY ts"
        } else if !latest {
            " ORDER BY started_at, id"
        } else {
            ""
        };
        let scope = if filtered {
            format!("WITH filtered_trips AS (SELECT t.* FROM panda.trips t WHERE t.vehicle_id=$1 AND t.started_at >= $2 AND t.started_at <= $3{}) ", sql_predicate("t",4,5,6))
        } else {
            String::new()
        };
        Ok(Self {
            sql: format!(
                "{scope}SELECT {timestamp} AS ts, {select} FROM {table}{predicate}{order}"
            ),
            projection,
            filtered,
            latest_sensor,
        })
    }
}

fn bucket_expression(bucket: &str, column: &str) -> String {
    match bucket {
        "minute" => format!("date_trunc('minute', {column})"),
        "5min" => format!("time_bucket(INTERVAL '5 minutes', {column})"),
        "15min" => format!("time_bucket(INTERVAL '15 minutes', {column})"),
        "hour" => format!("date_trunc('hour', {column})"),
        _ => format!("date_trunc('day', {column})"),
    }
}

fn expression(
    evidence: Evidence,
    latest: bool,
    raw: bool,
    filtered: bool,
) -> Result<(String, Option<String>), AppError> {
    let aggregate = latest || !raw || filtered;
    let result = match evidence.measure {
        Measure::Distance | Measure::OdometerDistance => (
            if latest { "COALESCE(SUM(distance_miles), 0)::float8" } else if aggregate { "SUM(distance_miles)::float8" } else { "distance_miles::float8" }.into(),
            (latest || filtered).then(|| "distance_miles > 0".into()),
        ),
        Measure::Count => ((if aggregate { "COUNT(*)::float8" } else { "1.0::float8" }).into(), None),
        Measure::ChargedEnergy | Measure::SessionEnergy => {
            let value = "COALESCE(kwh_added, energy_added_wh / 1000.0)";
            let sql = if !aggregate { format!("{value}::float8") }
                else if matches!(evidence.measure,Measure::SessionEnergy) { format!("AVG({value})::float8") }
                else if latest { format!("COALESCE(SUM({value}), 0)::float8") }
                else { format!("SUM({value})::float8") };
            (sql, None)
        }
        Measure::Cost => ((if latest { "COALESCE(SUM(cost_usd), 0)::float8" } else if aggregate { "SUM(cost_usd)::float8" } else { "cost_usd::float8" }).into(),None),
        Measure::Efficiency => ((if aggregate {
            "SUM(distance_miles)::float8 AS total_distance_miles, SUM(distance_miles * efficiency_wh_per_mile)::float8 AS weighted_efficiency_wh_mi"
        } else { "efficiency_wh_per_mile::float8" }).into(),Some("efficiency_wh_per_mile IS NOT NULL AND distance_miles > 0".into())),
        Measure::GrossEfficiency => ((if latest { "CASE WHEN SUM(distance_miles) > 0 THEN SUM(energy_wh + COALESCE(regen_wh, 0)) / SUM(distance_miles) ELSE NULL END::float8" }
            else if aggregate { "(SUM(energy_wh + COALESCE(regen_wh, 0)) / NULLIF(SUM(distance_miles), 0))::float8" }
            else { "(energy_wh + COALESCE(regen_wh, 0)) / NULLIF(distance_miles, 0)::float8" }).into(),Some("energy_wh IS NOT NULL AND distance_miles > 0".into())),
        Measure::Temperature => ((if aggregate { "CASE WHEN SUM(duration_seconds) > 0 THEN SUM(outside_temp_c * duration_seconds) / SUM(duration_seconds) ELSE NULL END::float8" } else { "outside_temp_c::float8" }).into(),Some(if aggregate { "outside_temp_c IS NOT NULL AND duration_seconds > 0" } else { "outside_temp_c IS NOT NULL" }.into())),
        Measure::Duration => ((if aggregate { "AVG(duration_seconds / 60.0)::float8" } else { "(duration_seconds / 60.0)::float8" }).into(),Some("duration_seconds IS NOT NULL".into())),
        Measure::Sensor(column) => {
            let sql = if latest || raw { format!("{column}::float8") }
                else {
                    let Relation::Telemetry(aggregation) = evidence.relation else { unreachable!() };
                    let aggregation = match aggregation {
                        "avg" | "mean" => "AVG", "max" => "MAX", "sum" => "SUM",
                        other => return Err(AppError::Validation(format!("unknown aggregation: {other}"))),
                    };
                    format!("{aggregation}({column})::float8")
                };
            (sql,Some(format!("{column} IS NOT NULL")))
        }
    };
    Ok(result)
}

fn odometer_history() -> &'static str {
    "SELECT day AS ts, miles_driven::float8 AS value
     FROM timeseries.odometer_daily
     WHERE vehicle_id = $1 AND day >= $2 AND day <= $3
     UNION ALL
     SELECT date_trunc('day', started_at) AS ts, SUM(distance_miles)::float8 AS value
     FROM panda.trips
     WHERE vehicle_id = $1 AND started_at >= $2 AND started_at <= $3
     AND NOT EXISTS (
       SELECT 1 FROM timeseries.odometer_daily
       WHERE vehicle_id = $1 AND day >= $2 AND day <= $3
     )
     GROUP BY 1 ORDER BY 1"
}
