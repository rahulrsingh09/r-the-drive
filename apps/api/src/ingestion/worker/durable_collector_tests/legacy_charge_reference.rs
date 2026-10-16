//! Frozen SQL reference from characterization commit bd89569; synthetic comparisons only.
use super::*;
#[derive(Debug, sqlx::FromRow)]
struct ActiveChargeDetectorRow {
    session_id: Uuid,
    started_at: DateTime<Utc>,
    last_charge_ts: DateTime<Utc>,
    location_lat: Option<f64>,
    location_lng: Option<f64>,
    soc_start: Option<f64>,
    last_soc: Option<f64>,
    charge_limit: Option<f64>,
    battery_capacity_wh: Option<f64>,
    power_samples_ts: Vec<DateTime<Utc>>,
    power_samples_kw: Vec<Option<f64>>,
}

async fn baseline_legacy_snapshot(
    conn: &mut PgConnection,
    vehicle_id: Uuid,
) -> anyhow::Result<Option<ActiveChargeSessionSnapshot>> {
    let row = sqlx::query_as::<_, ActiveChargeDetectorRow>(
        r#"
        WITH active_samples AS (
            SELECT
                t.charge_session_id,
                t.ts,
                t.latitude,
                t.longitude,
                t.battery_level,
                t.battery_capacity_wh,
                t.battery_limit,
                t.power_kw
            FROM timeseries.telemetry t
            LEFT JOIN panda.charge_sessions cs
              ON cs.id = t.charge_session_id
            WHERE t.vehicle_id = $1
              AND t.charge_session_id IS NOT NULL
              -- Both materialized and not-yet-materialized sessions are
              -- eligible.  The old cs.id IS NULL predicate discarded the
              -- exact row ensure_active_charge_session had just created.
              AND (cs.id IS NULL OR cs.ended_at IS NULL)
              AND t.ts >= now() - ($2::int * interval '1 hour')
        ),
        candidate_sessions AS (
            SELECT
                charge_session_id AS session_id,
                MIN(ts) AS started_at,
                MAX(ts) AS last_charge_ts,
                (ARRAY_AGG(latitude ORDER BY ts) FILTER (
                    WHERE latitude IS NOT NULL
                      AND longitude IS NOT NULL
                      AND NOT (latitude = 0 AND longitude = 0)
                ))[1] AS location_lat,
                (ARRAY_AGG(longitude ORDER BY ts) FILTER (
                    WHERE latitude IS NOT NULL
                      AND longitude IS NOT NULL
                      AND NOT (latitude = 0 AND longitude = 0)
                ))[1] AS location_lng,
                (ARRAY_AGG(battery_level ORDER BY ts) FILTER (WHERE battery_level IS NOT NULL))[1] AS soc_start,
                (ARRAY_AGG(battery_level ORDER BY ts DESC) FILTER (WHERE battery_level IS NOT NULL))[1] AS last_soc,
                MAX(battery_limit) AS charge_limit,
                (ARRAY_AGG(battery_capacity_wh ORDER BY ts DESC) FILTER (WHERE battery_capacity_wh IS NOT NULL))[1] AS battery_capacity_wh,
                ARRAY_AGG(ts ORDER BY ts) AS power_samples_ts,
                ARRAY_AGG(power_kw ORDER BY ts) AS power_samples_kw
            FROM active_samples
            GROUP BY charge_session_id
        )
        SELECT
            session_id,
            started_at,
            last_charge_ts,
            location_lat,
            location_lng,
            soc_start,
            last_soc,
            charge_limit,
            battery_capacity_wh,
            power_samples_ts,
            power_samples_kw
        FROM candidate_sessions
        WHERE last_charge_ts >= now() - ($3::int * interval '1 minute')
        ORDER BY last_charge_ts DESC
        LIMIT 1
        "#,
    )
    .bind(vehicle_id)
    .bind(CHARGE_DETECTOR_REHYDRATE_LOOKBACK_HOURS)
    .bind(CHARGE_DETECTOR_REHYDRATE_STALENESS_MINUTES)
    .fetch_optional(conn)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };

    let power = durable_charge_restart::ReconstructedChargePower::from_samples(
        row.started_at,
        row.power_samples_ts.into_iter().zip(row.power_samples_kw),
    );
    Ok(Some(ActiveChargeSessionSnapshot {
        session_id: row.session_id,
        started_at: row.started_at,
        location_lat: row.location_lat,
        location_lng: row.location_lng,
        soc_start: row.soc_start,
        last_soc: row.last_soc,
        charge_limit: row.charge_limit,
        battery_capacity_wh: row.battery_capacity_wh,
        last_charge_ts: row.last_charge_ts,
        last_power_ts: Some(power.last_power_ts),
        energy_used_wh: power.energy_used_wh,
        peak_charge_kw: power.peak_charge_kw,
    }))
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB"]
async fn residual_legacy_reconstruction_matches_sql_for_sparse_late_and_nonfinite_metadata() {
    let db = CollectorFixture::new().await;
    let older = Uuid::from_u128(501);
    let active = Uuid::from_u128(502);
    for (seconds, id, level, limit, power) in [
        (60, active, None, Some(80.0), Some(-10.0)),
        (-120, active, Some(f64::NAN), Some(f64::NAN), None),
        (0, older, Some(12.0), Some(100.0), Some(-20.0)),
        (120, active, Some(50.0), Some(85.0), Some(f64::INFINITY)),
    ] {
        let mut event = db.event(seconds);
        event.battery_level = level;
        event.battery_limit = limit;
        event.power_kw = power;
        if seconds == -120 {
            event.latitude = Some(f64::INFINITY);
            event.longitude = Some(-97.0);
            event.battery_capacity_wh = Some(f64::NEG_INFINITY);
            assert!(
                write_telemetry(&db.pool, &event, None, Some(id))
                    .await
                    .is_err(),
                "geospatial storage rejects infinite coordinates"
            );
            event.latitude = Some(30.0);
        }
        write_telemetry(&db.pool, &event, None, Some(id))
            .await
            .unwrap();
    }
    let mut conn = db.pool.acquire().await.unwrap();
    let expected = baseline_legacy_snapshot(&mut conn, db.vehicle_id)
        .await
        .unwrap()
        .unwrap();
    let actual = load_active_charge_snapshot_on(&mut conn, db.vehicle_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(actual.session_id, expected.session_id);
    assert_eq!(actual.started_at, expected.started_at);
    assert_eq!(actual.last_charge_ts, expected.last_charge_ts);
    assert_eq!(actual.last_power_ts, expected.last_power_ts);
    for (actual, expected) in [
        (actual.location_lat, expected.location_lat),
        (actual.location_lng, expected.location_lng),
        (actual.soc_start, expected.soc_start),
        (actual.last_soc, expected.last_soc),
        (actual.charge_limit, expected.charge_limit),
        (actual.battery_capacity_wh, expected.battery_capacity_wh),
        (Some(actual.energy_used_wh), Some(expected.energy_used_wh)),
        (Some(actual.peak_charge_kw), Some(expected.peak_charge_kw)),
    ] {
        assert_eq!(actual.map(f64::to_bits), expected.map(f64::to_bits));
    }
    drop(conn);
    db.finish().await;
}
