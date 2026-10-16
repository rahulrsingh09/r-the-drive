//! Reconstruct missing charge_sessions from telemetry evidence.
//!
//! This repairs sessions that were not closed by the live detector, usually
//! because the API restarted while a charge was in progress or Rivian sent
//! sparse partial patches without charger_state on every row.
//!
//!   cargo run --bin repair_charge_sessions_from_telemetry

#[path = "maintenance_replay/charge_windows.rs"]
mod charge_windows;
#[path = "maintenance_replay/snapshot_scan.rs"]
mod snapshot_scan;
use snapshot_scan::SnapshotScan;

use anyhow::Result;
use panda_api::db::pool::application_pool_options;
use panda_api::{
    db::vehicles::get_vehicle_owner_id, services::charge_sessions::canonicalize_charge_sessions,
    services::cost::recompute_charge_session_cost, services::geofences::match_geofence,
};
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool = application_pool_options()
        .max_connections(5)
        .connect(&database_url)
        .await?;

    let repaired = repair_charge_sessions_from_telemetry(&pool).await?;
    let canonicalized = canonicalize_charge_sessions(&pool, None).await?;
    let repaired_api_metadata = repair_api_summary_metadata(&pool).await?;
    let estimated_soc = estimate_api_session_soc_from_energy(&pool).await?;
    let retagged = repair_charge_session_locations(&pool).await?;
    let recosted = repair_charge_session_costs(&pool).await?;
    info!(
        repaired,
        charge_clusters_merged = canonicalized.clusters_merged,
        charge_duplicates_deleted = canonicalized.duplicate_rows_deleted,
        charge_payload_reconciliations = canonicalized.payload_reconciliations,
        repaired_api_metadata,
        estimated_soc,
        retagged,
        recosted,
        "charge session telemetry repair complete"
    );
    Ok(())
}

async fn repair_charge_sessions_from_telemetry(pool: &sqlx::PgPool) -> Result<u64> {
    let vehicles: Vec<uuid::Uuid> = sqlx::query_scalar(
        "SELECT DISTINCT vehicle_id FROM timeseries.telemetry ORDER BY vehicle_id",
    )
    .fetch_all(pool)
    .await?;
    let mut repaired = 0;
    for vehicle_id in vehicles {
        let mut tx = pool.begin().await?;
        panda_api::services::charge_sessions::lock_charge_history(&mut tx, vehicle_id).await?;
        let pinned: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM panda.active_charge_checkpoints WHERE vehicle_id=$1 AND active_session_id IS NOT NULL) \
             OR EXISTS(SELECT 1 FROM panda.pending_charge_completions WHERE vehicle_id=$1)",
        ).bind(vehicle_id).fetch_one(&mut *tx).await?;
        if pinned {
            continue;
        }
        repaired += repair_vehicle_charge_sessions(&mut tx, vehicle_id).await?;
        tx.commit().await?;
    }
    Ok(repaired)
}

async fn repair_vehicle_charge_sessions(
    conn: &mut sqlx::PgConnection,
    vehicle_id: uuid::Uuid,
) -> Result<u64> {
    let repaired: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(
        charge_windows::reconstruction_query(r#"
        matched_existing AS (
            SELECT DISTINCT ON (c.id)
                c.*,
                cs.id AS existing_id
            FROM candidates c
            JOIN panda.charge_sessions cs
              ON cs.vehicle_id = c.vehicle_id
             AND cs.started_at <= c.ended_at
             AND COALESCE(cs.ended_at, cs.started_at) >= c.started_at
             AND COALESCE(cs.source, 'telemetry') <> 'rivian_api'
            ORDER BY c.id,
                     (cs.kwh_added IS NOT NULL) DESC,
                     cs.started_at
        ),
        updated AS (
            UPDATE panda.charge_sessions cs
            SET
                started_at = CASE
                    WHEN cs.source = 'rivian_api' THEN cs.started_at
                    ELSE matched_existing.started_at
                END,
                ended_at = CASE
                    WHEN cs.source = 'rivian_api' THEN COALESCE(cs.ended_at, matched_existing.ended_at)
                    ELSE matched_existing.ended_at
                END,
                source_location_lat = COALESCE(cs.source_location_lat, matched_existing.location_lat),
                source_location_lng = COALESCE(cs.source_location_lng, matched_existing.location_lng),
                location_lat = CASE WHEN cs.location_override_mode = 'automatic'
                    THEN COALESCE(cs.location_lat, matched_existing.location_lat) ELSE cs.location_lat END,
                location_lng = CASE WHEN cs.location_override_mode = 'automatic'
                    THEN COALESCE(cs.location_lng, matched_existing.location_lng) ELSE cs.location_lng END,
                soc_start = CASE
                    WHEN cs.source = 'rivian_api' THEN COALESCE(cs.soc_start, matched_existing.soc_start)
                    ELSE matched_existing.soc_start
                END,
                soc_end = CASE
                    WHEN cs.source = 'rivian_api' THEN COALESCE(cs.soc_end, matched_existing.soc_end)
                    ELSE matched_existing.soc_end
                END,
                charge_limit = COALESCE(cs.charge_limit, matched_existing.charge_limit),
                duration_minutes = CASE
                    WHEN cs.source = 'rivian_api' THEN COALESCE(
                        cs.duration_minutes,
                        EXTRACT(EPOCH FROM (COALESCE(cs.ended_at, matched_existing.ended_at) - cs.started_at))::int / 60,
                        matched_existing.duration_minutes
                    )
                    ELSE matched_existing.duration_minutes
                END,
                kwh_added = CASE
                    WHEN cs.source = 'rivian_api' AND cs.kwh_added IS NOT NULL THEN cs.kwh_added
                    ELSE matched_existing.energy_added_wh / 1000.0
                END,
                energy_added_wh = CASE
                    WHEN cs.source = 'rivian_api' AND COALESCE(cs.kwh_added, cs.energy_added_wh / 1000.0) IS NOT NULL
                    THEN COALESCE(cs.energy_added_wh, cs.kwh_added * 1000.0)
                    ELSE matched_existing.energy_added_wh
                END,
                max_charge_rate_kw = COALESCE(cs.max_charge_rate_kw, matched_existing.max_charge_rate_kw),
                avg_charge_rate_kw = CASE
                    WHEN cs.source = 'rivian_api' AND cs.avg_charge_rate_kw IS NOT NULL THEN cs.avg_charge_rate_kw
                    WHEN matched_existing.duration_minutes > 0
                    THEN (
                        CASE
                            WHEN cs.source = 'rivian_api' AND cs.kwh_added IS NOT NULL THEN cs.kwh_added
                            ELSE matched_existing.energy_added_wh / 1000.0
                        END
                    ) / (matched_existing.duration_minutes::float8 / 60.0)
                END,
                cost_method = COALESCE(cs.cost_method, 'telemetry_repair'),
                source = COALESCE(cs.source, 'telemetry'),
                data_confidence = 'telemetry',
                charger_type = COALESCE(
                    cs.charger_type,
                    CASE
                        WHEN matched_existing.max_charge_rate_kw < 12 THEN 'ac'
                        WHEN matched_existing.max_charge_rate_kw < 50 THEN 'ac_l2'
                        WHEN matched_existing.max_charge_rate_kw IS NOT NULL THEN 'dc'
                        WHEN matched_existing.duration_minutes > 0
                             AND matched_existing.energy_added_wh / 1000.0 / (matched_existing.duration_minutes::float8 / 60.0) < 12 THEN 'ac'
                        WHEN matched_existing.duration_minutes > 0
                             AND matched_existing.energy_added_wh / 1000.0 / (matched_existing.duration_minutes::float8 / 60.0) < 50 THEN 'ac_l2'
                        WHEN matched_existing.duration_minutes > 0 THEN 'dc'
                    END
                )
            FROM matched_existing
            WHERE cs.id = matched_existing.existing_id
              AND (
                  matched_existing.started_at <> cs.started_at
                  OR matched_existing.ended_at <> COALESCE(cs.ended_at, cs.started_at)
                  OR (COALESCE(cs.source, '') <> 'rivian_api' AND cs.soc_start IS DISTINCT FROM matched_existing.soc_start)
                  OR (COALESCE(cs.source, '') <> 'rivian_api' AND cs.soc_end IS DISTINCT FROM matched_existing.soc_end)
                  OR (COALESCE(cs.source, '') <> 'rivian_api' AND cs.duration_minutes IS DISTINCT FROM matched_existing.duration_minutes)
                  OR cs.soc_start IS NULL
                  OR cs.soc_end IS NULL
                  OR cs.kwh_added IS NULL
                  OR cs.energy_added_wh IS NULL
                  OR cs.duration_minutes IS NULL
            )
            RETURNING cs.id, cs.vehicle_id, cs.started_at, cs.ended_at
        ),
        inserted AS (
            INSERT INTO panda.charge_sessions (
                id, vehicle_id, started_at, ended_at,
                location_lat, location_lng, source_location_lat, source_location_lng,
                soc_start, soc_end, charge_limit, duration_minutes,
                kwh_added, max_charge_rate_kw, cost_usd,
                energy_added_wh, energy_used_wh,
                avg_charge_rate_kw, peak_voltage,
                cost_method, charger_type, source, data_confidence
            )
            SELECT
                id, vehicle_id, started_at, ended_at,
                location_lat, location_lng, location_lat, location_lng,
                soc_start, soc_end, charge_limit, duration_minutes,
                energy_added_wh / 1000.0, max_charge_rate_kw, NULL,
                energy_added_wh, NULL,
                CASE
                    WHEN duration_minutes > 0 THEN energy_added_wh / 1000.0 / (duration_minutes::float8 / 60.0)
                END,
                NULL,
                'telemetry_repair',
                CASE
                    WHEN max_charge_rate_kw < 12 THEN 'ac'
                    WHEN max_charge_rate_kw < 50 THEN 'ac_l2'
                    WHEN max_charge_rate_kw IS NOT NULL THEN 'dc'
                    WHEN duration_minutes > 0 AND energy_added_wh / 1000.0 / (duration_minutes::float8 / 60.0) < 12 THEN 'ac'
                    WHEN duration_minutes > 0 AND energy_added_wh / 1000.0 / (duration_minutes::float8 / 60.0) < 50 THEN 'ac_l2'
                    WHEN duration_minutes > 0 THEN 'dc'
                END,
                'telemetry',
                'telemetry'
            FROM candidates
            WHERE NOT EXISTS (
                  SELECT 1
                  FROM matched_existing
                  WHERE matched_existing.id = candidates.id
              )
              AND NOT EXISTS (
                  SELECT 1
                  FROM panda.charge_sessions cs
                  WHERE cs.vehicle_id = candidates.vehicle_id
                    AND cs.started_at <= candidates.ended_at
                    AND COALESCE(cs.ended_at, cs.started_at) >= candidates.started_at
                    AND COALESCE(cs.source, 'telemetry') <> 'rivian_api'
              )
            RETURNING id, vehicle_id, started_at, ended_at
        ),
        repaired AS (
            SELECT * FROM inserted
            UNION ALL
            SELECT * FROM updated
        ),
        merged_duplicates AS (
            DELETE FROM panda.charge_sessions duplicate
            USING repaired
            WHERE duplicate.vehicle_id = repaired.vehicle_id
              AND duplicate.id <> repaired.id
              AND duplicate.started_at <= repaired.ended_at
              AND COALESCE(duplicate.ended_at, duplicate.started_at) >= repaired.started_at
              AND COALESCE(duplicate.source, '') <> 'rivian_api'
            RETURNING duplicate.id
        ),
        stamped AS (
            UPDATE timeseries.telemetry t
            SET charge_session_id = repaired.id
            FROM repaired
            WHERE t.vehicle_id = repaired.vehicle_id
              AND t.ts >= repaired.started_at
              AND t.ts <= repaired.ended_at
              AND (
                  t.charge_session_id IS NULL
                  OR NOT EXISTS (
                      SELECT 1
                      FROM panda.charge_sessions existing
                      WHERE existing.id = t.charge_session_id
                  )
              )
            RETURNING repaired.id
        )
        SELECT COUNT(DISTINCT id)::int8 AS repaired
        FROM stamped
        "#, false),
    ))
    .bind(vehicle_id)
    .fetch_one(conn)
    .await?;

    Ok(repaired as u64)
}

async fn repair_charge_session_locations(pool: &sqlx::PgPool) -> Result<u64> {
    let mut scan = SnapshotScan::open(
        pool,
        r#"SELECT id, vehicle_id, location_lat, location_lng
           FROM panda.charge_sessions
           WHERE geofence_id IS NULL
             AND location_lat IS NOT NULL
             AND location_lng IS NOT NULL"#,
        None,
    )
    .await?;

    let mut repaired = 0u64;

    loop {
        let sessions = scan.next::<ChargeLocationRow>().await?;
        if sessions.is_empty() {
            break;
        }
        for session in sessions {
            let owner_id = get_vehicle_owner_id(pool, session.vehicle_id).await?;
            let matched = match (owner_id, session.location_lat, session.location_lng) {
                (Some(user_id), Some(lat), Some(lon)) => {
                    match_geofence(pool, user_id, lat, lon).await?
                }
                _ => None,
            };

            let Some(matched) = matched else {
                continue;
            };

            let result = sqlx::query!(
                r#"UPDATE panda.charge_sessions
               SET geofence_id = COALESCE(geofence_id, $2),
                   address_id  = COALESCE(address_id,  $3),
                   is_home     = COALESCE(is_home,     $4)
               WHERE id = $1"#,
                session.id,
                matched.id,
                matched.address_id,
                matched.is_home,
            )
            .execute(pool)
            .await?;

            repaired += result.rows_affected();
        }
    }
    scan.finish().await?;
    Ok(repaired)
}

async fn estimate_api_session_soc_from_energy(pool: &sqlx::PgPool) -> Result<u64> {
    let repaired = sqlx::query!(
        r#"
        UPDATE panda.charge_sessions cs
        SET soc_start = GREATEST(
                0.0,
                cs.soc_end - (cs.kwh_added / (v.battery_capacity_wh / 1000.0) * 100.0)
            )
        FROM panda.vehicles v
        WHERE cs.vehicle_id = v.id
          AND cs.source = 'rivian_api'
          AND cs.kwh_added IS NOT NULL
          AND cs.soc_end IS NOT NULL
          AND v.battery_capacity_wh IS NOT NULL
          AND v.battery_capacity_wh > 0
          AND (
              cs.soc_start IS NULL
              OR ((cs.soc_end - cs.soc_start) / 100.0 * (v.battery_capacity_wh / 1000.0)) < cs.kwh_added * 0.6
          )
        "#
    )
    .execute(pool)
    .await?
    .rows_affected();

    Ok(repaired)
}

async fn repair_api_summary_metadata(pool: &sqlx::PgPool) -> Result<u64> {
    let repaired = sqlx::query!(
        r#"
        UPDATE panda.charge_sessions
        SET duration_minutes = COALESCE(
                duration_minutes,
                CASE
                    WHEN ended_at IS NOT NULL
                    THEN EXTRACT(EPOCH FROM (ended_at - started_at))::int / 60
                END
            ),
            charger_type = COALESCE(
                charger_type,
                CASE
                    WHEN lower(COALESCE(network_vendor, '')) = ANY(ARRAY['tesla','rivian','electrify america','evgo']) THEN 'dc'
                    WHEN is_home THEN 'ac'
                END
            )
        WHERE source = 'rivian_api'
          AND (
              duration_minutes IS NULL
              OR charger_type IS NULL
          )
        "#
    )
    .execute(pool)
    .await?
    .rows_affected();

    Ok(repaired)
}

async fn repair_charge_session_costs(pool: &sqlx::PgPool) -> Result<u64> {
    let mut scan = SnapshotScan::open(pool, "SELECT id FROM panda.charge_sessions", None).await?;

    let mut repaired = 0u64;

    loop {
        let session_ids = scan.next::<(uuid::Uuid,)>().await?;
        if session_ids.is_empty() {
            break;
        }
        for (session_id,) in session_ids {
            if recompute_charge_session_cost(pool, session_id)
                .await?
                .is_some()
            {
                repaired += 1;
            }
        }
    }
    scan.finish().await?;
    Ok(repaired)
}

#[cfg(test)]
#[path = "../ingestion/worker/durable_collector_tests/charge_repair_cli.rs"]
mod durable_charge_tests;

#[derive(sqlx::FromRow)]
struct ChargeLocationRow {
    id: uuid::Uuid,
    vehicle_id: uuid::Uuid,
    location_lat: Option<f64>,
    location_lng: Option<f64>,
}
