use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use panda_api::routes::range_normalization::plausible_max_mi_per_kwh;
use sqlx::postgres::PgPoolOptions;
use tracing::info;
use uuid::Uuid;

/// Plausible efficiency ceilings bound as `$4` (every other model) and `$5`
/// (R2). They come from the live normalizer so repairs never disagree with it.
fn efficiency_ceilings() -> (f64, f64) {
    (
        plausible_max_mi_per_kwh(None),
        plausible_max_mi_per_kwh(Some("R2")),
    )
}

#[derive(Debug, Default)]
struct RepairScope {
    vehicle_id: Option<Uuid>,
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    dry_run: bool,
}

fn parse_scope() -> Result<RepairScope> {
    let mut scope = RepairScope::default();
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--vehicle-id" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow!("--vehicle-id requires a UUID value"))?;
                scope.vehicle_id = Some(value.parse::<Uuid>()?);
            }
            "--from" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow!("--from requires an RFC3339 timestamp"))?;
                scope.from = Some(DateTime::parse_from_rfc3339(&value)?.with_timezone(&Utc));
            }
            "--to" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow!("--to requires an RFC3339 timestamp"))?;
                scope.to = Some(DateTime::parse_from_rfc3339(&value)?.with_timezone(&Utc));
            }
            "--dry-run" => {
                scope.dry_run = true;
            }
            "--help" | "-h" => {
                println!(
                    "Usage: cargo run --bin repair_range_outliers [--dry-run] [--vehicle-id <uuid>] [--from <rfc3339>] [--to <rfc3339>]"
                );
                std::process::exit(0);
            }
            other => return Err(anyhow!("Unknown argument: {other}")),
        }
    }

    Ok(scope)
}

#[derive(Debug, sqlx::FromRow)]
struct RepairSummary {
    converted_rows: i64,
    nulled_rows: i64,
    untouched_rows: i64,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let scope = parse_scope()?;
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await?;

    let summary = summarize(&pool, &scope).await?;
    info!(
        converted_rows = summary.converted_rows,
        nulled_rows = summary.nulled_rows,
        untouched_rows = summary.untouched_rows,
        dry_run = scope.dry_run,
        "range outlier analysis"
    );

    if scope.dry_run {
        return Ok(());
    }

    let updated = apply_repairs(&pool, &scope).await?;
    info!(rows_updated = updated, "range outlier repair complete");

    Ok(())
}

const RANGE_ASSESSMENT: &str = r#"
    WITH eligible AS (
        SELECT t.ts, t.vehicle_id, t.distance_to_empty_mi,
               t.distance_to_empty_mi / t.battery_level * 100.0 AS raw_full_range,
               (t.distance_to_empty_mi / 1.609344) / t.battery_level * 100.0 AS converted_full_range,
               (CASE WHEN t.battery_capacity_wh > 1000.0
                    THEN t.battery_capacity_wh / 1000.0 ELSE t.battery_capacity_wh END)
                   * CASE WHEN upper(trim(v.model)) = 'R2' THEN $5::float8 ELSE $4::float8 END
                   AS plausible_full_range
        FROM timeseries.telemetry t
        JOIN panda.vehicles v ON v.id = t.vehicle_id
        WHERE t.distance_to_empty_mi IS NOT NULL
          AND t.battery_level IS NOT NULL AND t.battery_level > 0
          AND t.battery_capacity_wh IS NOT NULL AND t.battery_capacity_wh > 10000
          AND ($1::uuid IS NULL OR t.vehicle_id = $1)
          AND ($2::timestamptz IS NULL OR t.ts >= $2)
          AND ($3::timestamptz IS NULL OR t.ts <= $3)
    ), assessed AS (
        SELECT ts, vehicle_id, distance_to_empty_mi,
               CASE WHEN raw_full_range <= plausible_full_range THEN 0
                    WHEN converted_full_range <= plausible_full_range THEN 1
                    ELSE 2 END AS action
        FROM eligible
    )
"#;

async fn summarize(pool: &sqlx::PgPool, scope: &RepairScope) -> Result<RepairSummary> {
    let (default_ceiling, r2_ceiling) = efficiency_ceilings();
    let statement = sqlx::AssertSqlSafe(format!(
        "{RANGE_ASSESSMENT} SELECT COUNT(*) FILTER (WHERE action = 1)::int8 AS converted_rows,
         COUNT(*) FILTER (WHERE action = 2)::int8 AS nulled_rows,
         COUNT(*) FILTER (WHERE action = 0)::int8 AS untouched_rows FROM assessed"
    ));
    Ok(sqlx::query_as::<_, RepairSummary>(statement)
        .bind(scope.vehicle_id)
        .bind(scope.from)
        .bind(scope.to)
        .bind(default_ceiling)
        .bind(r2_ceiling)
        .fetch_one(pool)
        .await?)
}

async fn apply_repairs(pool: &sqlx::PgPool, scope: &RepairScope) -> Result<u64> {
    let (default_ceiling, r2_ceiling) = efficiency_ceilings();
    let statement = sqlx::AssertSqlSafe(format!(
        "{RANGE_ASSESSMENT} UPDATE timeseries.telemetry t
         SET distance_to_empty_mi = CASE WHEN assessed.action = 1
             THEN NULLIF(assessed.distance_to_empty_mi / 1.609344, 0.0) END
         FROM assessed
         WHERE t.ts = assessed.ts AND t.vehicle_id = assessed.vehicle_id
           AND assessed.action <> 0"
    ));
    Ok(sqlx::query(statement)
        .bind(scope.vehicle_id)
        .bind(scope.from)
        .bind(scope.to)
        .bind(default_ceiling)
        .bind(r2_ceiling)
        .execute(pool)
        .await?
        .rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;
    use panda_api::routes::range_normalization::{
        PLAUSIBLE_MAX_MI_PER_KWH, R2_PLAUSIBLE_MAX_MI_PER_KWH,
    };

    #[test]
    fn ceilings_match_the_live_normalizer() {
        assert_eq!(
            efficiency_ceilings(),
            (PLAUSIBLE_MAX_MI_PER_KWH, R2_PLAUSIBLE_MAX_MI_PER_KWH)
        );
    }

    #[test]
    fn legitimate_r2_range_stays_under_the_r2_ceiling_only() {
        // 208 mi at 62 % of a 91.6 kWh pack is about 3.66 mi/kWh.
        let full_range = 208.0 / 62.0 * 100.0;
        let (default_ceiling, r2_ceiling) = efficiency_ceilings();
        assert!(full_range > 91.6 * default_ceiling);
        assert!(full_range <= 91.6 * r2_ceiling);
    }
}
