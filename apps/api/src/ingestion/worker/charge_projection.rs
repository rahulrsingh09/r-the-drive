use super::*;

pub(super) async fn load_active_charge_snapshot_on(
    conn: &mut PgConnection,
    vehicle_id: Uuid,
) -> anyhow::Result<Option<ActiveChargeSessionSnapshot>> {
    let rows = sqlx::query_as::<_, LegacyChargeSample>(
        r#"SELECT t.charge_session_id AS session_id,t.ts,t.latitude,t.longitude,
                  t.battery_level,t.battery_capacity_wh,t.battery_limit,t.power_kw
           FROM timeseries.telemetry t
           LEFT JOIN panda.charge_sessions cs ON cs.id=t.charge_session_id
           WHERE t.vehicle_id=$1 AND t.charge_session_id IS NOT NULL
             AND (cs.id IS NULL OR cs.ended_at IS NULL)
             AND t.ts >= now()-($2::int * interval '1 hour')
           ORDER BY t.ts"#,
    )
    .bind(vehicle_id)
    .bind(CHARGE_DETECTOR_REHYDRATE_LOOKBACK_HOURS)
    .fetch_all(&mut *conn)
    .await?;
    let cutoff: DateTime<Utc> = sqlx::query_scalar("SELECT now()-($1::int * interval '1 minute')")
        .bind(CHARGE_DETECTOR_REHYDRATE_STALENESS_MINUTES)
        .fetch_one(conn)
        .await?;
    Ok(reconstruct_legacy_charge(rows, cutoff))
}

#[derive(sqlx::FromRow)]
struct LegacyChargeSample {
    session_id: Uuid,
    ts: DateTime<Utc>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    battery_level: Option<f64>,
    battery_capacity_wh: Option<f64>,
    battery_limit: Option<f64>,
    power_kw: Option<f64>,
}

fn reconstruct_legacy_charge(
    rows: Vec<LegacyChargeSample>,
    cutoff: DateTime<Utc>,
) -> Option<ActiveChargeSessionSnapshot> {
    let mut candidates = std::collections::BTreeMap::<Uuid, LegacyCandidate>::new();
    for row in rows {
        candidates
            .entry(row.session_id)
            .or_insert_with(|| LegacyCandidate::new(&row))
            .observe(row);
    }
    candidates
        .into_values()
        .filter(|candidate| candidate.snapshot.last_charge_ts >= cutoff)
        .max_by_key(|candidate| candidate.snapshot.last_charge_ts)
        .map(LegacyCandidate::finish)
}

struct LegacyCandidate {
    snapshot: ActiveChargeSessionSnapshot,
    power: Vec<(DateTime<Utc>, Option<f64>)>,
}
impl LegacyCandidate {
    fn new(row: &LegacyChargeSample) -> Self {
        Self {
            snapshot: ActiveChargeSessionSnapshot {
                session_id: row.session_id,
                started_at: row.ts,
                last_charge_ts: row.ts,
                location_lat: None,
                location_lng: None,
                soc_start: None,
                last_soc: None,
                charge_limit: None,
                battery_capacity_wh: None,
                last_power_ts: None,
                energy_used_wh: 0.0,
                peak_charge_kw: 0.0,
            },
            power: Vec::new(),
        }
    }
    fn observe(&mut self, row: LegacyChargeSample) {
        let target = &mut self.snapshot;
        target.last_charge_ts = row.ts;
        if target.location_lat.is_none() {
            if let (Some(lat), Some(lng)) = (row.latitude, row.longitude) {
                if lat != 0.0 || lng != 0.0 {
                    target.location_lat = Some(lat);
                    target.location_lng = Some(lng);
                }
            }
        }
        target.soc_start = target.soc_start.or(row.battery_level);
        target.last_soc = row.battery_level.or(target.last_soc);
        target.battery_capacity_wh = row.battery_capacity_wh.or(target.battery_capacity_wh);
        if let Some(limit) = row.battery_limit {
            // PostgreSQL MAX ranks NaN above every finite value.
            target.charge_limit = Some(match target.charge_limit {
                Some(previous) if previous.is_nan() => previous,
                Some(previous) if !limit.is_nan() => previous.max(limit),
                _ => limit,
            });
        }
        self.power.push((row.ts, row.power_kw));
    }
    fn finish(mut self) -> ActiveChargeSessionSnapshot {
        let power = durable_charge_restart::ReconstructedChargePower::from_samples(
            self.snapshot.started_at,
            self.power,
        );
        self.snapshot.last_power_ts = Some(power.last_power_ts);
        self.snapshot.energy_used_wh = power.energy_used_wh;
        self.snapshot.peak_charge_kw = power.peak_charge_kw;
        self.snapshot
    }
}

pub(super) async fn ensure_active_charge_session(
    pool: &PgPool,
    vehicle_id: Uuid,
    session_id: Uuid,
    event: &TelemetryEvent,
) -> anyhow::Result<()> {
    let mut conn = pool.acquire().await?;
    ensure_active_charge_session_on(&mut conn, vehicle_id, session_id, event).await
}

pub(super) async fn ensure_active_charge_session_on(
    conn: &mut PgConnection,
    vehicle_id: Uuid,
    session_id: Uuid,
    event: &TelemetryEvent,
) -> anyhow::Result<()> {
    let location = match (event.latitude, event.longitude) {
        (Some(latitude), Some(longitude))
            if latitude.is_finite()
                && longitude.is_finite()
                && (-90.0..=90.0).contains(&latitude)
                && (-180.0..=180.0).contains(&longitude)
                && !(latitude == 0.0 && longitude == 0.0) =>
        {
            Some((latitude, longitude))
        }
        _ => None,
    };
    let statement = charge_upsert(
        r#"INSERT INTO panda.charge_sessions
           (id, vehicle_id, started_at,
            location_lat, location_lng, source_location_lat, source_location_lng,
            soc_start, charge_limit, source, data_confidence)
           SELECT $1, $2,
                  COALESCE(MIN(t.ts), $3),
                  $6, $7, $6, $7,
                  COALESCE(
                      (ARRAY_AGG(t.battery_level ORDER BY t.ts)
                       FILTER (WHERE t.battery_level IS NOT NULL))[1],
                      $4
                  ),
                  COALESCE(MAX(t.battery_limit), $5),
                  'telemetry',
                  'telemetry'
           FROM timeseries.telemetry t
           WHERE t.vehicle_id = $2
             AND t.charge_session_id = $1"#,
        ACTIVE_FIELDS,
    );
    let saved: Option<Uuid> = sqlx::query_scalar(sqlx::AssertSqlSafe(statement.as_str()))
        .bind(session_id)
        .bind(vehicle_id)
        .bind(event.ts)
        .bind(event.battery_level)
        .bind(event.battery_limit)
        .bind(location.map(|(latitude, _)| latitude))
        .bind(location.map(|(_, longitude)| longitude))
        .fetch_optional(conn)
        .await?;
    anyhow::ensure!(
        saved == Some(session_id),
        "active charge session ownership mismatch"
    );
    Ok(())
}

pub(super) async fn apply_charging_session_update(
    pool: &PgPool,
    redis_conn: &mut redis::aio::MultiplexedConnection,
    vehicle_id: Uuid,
    event: &parser::ChargingSessionEvent,
    received_at: DateTime<Utc>,
    active_session_id: Option<Uuid>,
) {
    let live_key = rivian_poll::live_session_redis_key(vehicle_id);
    match LiveProjection::plan(event, received_at, active_session_id) {
        LiveProjection::Refresh(live) => {
            if let Ok(json) = serde_json::to_string(&live) {
                if let Err(error) = redis_conn
                    .set_ex::<_, _, ()>(&live_key, json, rivian_poll::LIVE_SESSION_TTL_SECONDS)
                    .await
                {
                    tracing::warn!(vehicle_id=%vehicle_id, err=%error, "live session Redis write failed");
                }
            }
            if let Err(error) =
                rivian_poll::persist_live_session_data(pool, active_session_id, &live).await
            {
                tracing::warn!(vehicle_id=%vehicle_id, err=%error, "live session database update failed");
            }
        }
        LiveProjection::Clear => {
            if let Err(error) = redis_conn.del::<_, ()>(&live_key).await {
                tracing::debug!(vehicle_id=%vehicle_id, err=%error, "live session Redis cleanup failed");
            }
        }
        LiveProjection::Retain => {}
    }

    if let Err(error) = rivian_poll::persist_charging_session_chart(
        pool,
        vehicle_id,
        active_session_id,
        &event.chart_data,
    )
    .await
    {
        tracing::warn!(vehicle_id=%vehicle_id, err=%error, "live charge curve update failed");
    }
}

/// Where a completed session happened. Charging frames rarely carry GPS, so
/// when the detector never saw a fix, use the row's stored location and then
/// the vehicle's nearest telemetry fix, so the saved place and address can
/// still be matched.
pub(super) async fn charge_session_coordinates(
    pool: &PgPool,
    session: &crate::ingestion::charge_detector::CompletedChargeSession,
) -> (Option<f64>, Option<f64>) {
    if let (Some(lat), Some(lng)) = (session.location_lat, session.location_lng) {
        return (Some(lat), Some(lng));
    }

    let stored = sqlx::query_as::<_, (Option<f64>, Option<f64>)>(
        "SELECT location_lat, location_lng FROM panda.charge_sessions WHERE id = $1",
    )
    .bind(session.session_id)
    .fetch_optional(pool)
    .await;
    match stored {
        Ok(Some((Some(lat), Some(lng)))) => return (Some(lat), Some(lng)),
        Ok(_) => {}
        Err(error) => {
            tracing::warn!(session_id=%session.session_id, error=%error, "worker.charge_session_stored_location_failed");
        }
    }

    match crate::services::trip_enrichment::nearest_telemetry_fix(
        pool,
        session.vehicle_id,
        session.started_at,
    )
    .await
    {
        Ok(Some((lat, lng))) => (Some(lat), Some(lng)),
        Ok(None) => (None, None),
        Err(error) => {
            tracing::warn!(session_id=%session.session_id, error=%error, "worker.charge_session_telemetry_fix_failed");
            (None, None)
        }
    }
}

pub(super) async fn persist_charge_session(
    pool: &PgPool,
    session: &crate::ingestion::charge_detector::CompletedChargeSession,
) -> anyhow::Result<()> {
    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_default();
    let owner_id = get_vehicle_owner_id(pool, session.vehicle_id).await?;
    let (location_lat, location_lng) = charge_session_coordinates(pool, session).await;
    let location_match = match (owner_id, location_lat, location_lng) {
        (Some(user_id), Some(lat), Some(lon)) => {
            resolve_trip_location(pool, &http_client, user_id, lat, lon).await?
        }
        _ => MatchedLocation::none(),
    };

    let mut conn = pool.begin().await?;
    crate::services::charge_sessions::lock_charge_history(&mut conn, session.vehicle_id).await?;
    let present: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM panda.charge_sessions WHERE id=$1 AND vehicle_id=$2)",
    )
    .bind(session.session_id)
    .bind(session.vehicle_id)
    .fetch_one(&mut *conn)
    .await?;
    anyhow::ensure!(
        present,
        "charge session was canonicalized before optional enrichment"
    );
    persist_charge_session_record(
        &mut conn,
        session,
        location_lat,
        location_lng,
        &location_match,
    )
    .await?;
    conn.commit().await?;

    if let Some(user_id) = owner_id {
        sqlx::query(
            r#"INSERT INTO panda.charge_session_user_annotations
               (charge_session_id, user_id, geofence_id, address_id, is_home, computed_at)
               VALUES ($1, $2, $3, $4, $5, now())
               ON CONFLICT (charge_session_id, user_id) DO UPDATE
               SET geofence_id = EXCLUDED.geofence_id,
                   address_id = EXCLUDED.address_id,
                   is_home = EXCLUDED.is_home,
                   computed_at = now(),
                   updated_at = now()"#,
        )
        .bind(session.session_id)
        .bind(user_id)
        .bind(location_match.geofence_id)
        .bind(location_match.address_id)
        .bind(location_match.is_home)
        .execute(pool)
        .await?;
    }

    let _ = recompute_charge_session_cost(pool, session.session_id).await?;
    Ok(())
}

pub(super) async fn persist_charge_session_record(
    conn: &mut PgConnection,
    session: &crate::ingestion::charge_detector::CompletedChargeSession,
    location_lat: Option<f64>,
    location_lng: Option<f64>,
    location_match: &MatchedLocation,
) -> anyhow::Result<()> {
    let duration_minutes = ((session.ended_at - session.started_at).num_seconds() / 60) as i32;
    let energy_added_kwh = session.energy_added_wh.map(|wh| wh / 1000.0);
    let statement = charge_upsert(
        r#"INSERT INTO panda.charge_sessions
           (id, vehicle_id, started_at, ended_at,
            location_lat, location_lng, source_location_lat, source_location_lng,
            is_home,
            soc_start, soc_end, charge_limit, duration_minutes,
            kwh_added, max_charge_rate_kw, cost_usd,
            energy_added_wh, energy_used_wh,
            avg_charge_rate_kw, peak_voltage,
            geofence_id, address_id, cost_profile_id, cost_method,
            charger_type, source, data_confidence)
              VALUES ($1,$2,$3,$4,$5,$6,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22,$23,'telemetry','telemetry')"#,
        COMPLETED_FIELDS,
    );
    let saved: Option<Uuid> = sqlx::query_scalar(sqlx::AssertSqlSafe(statement.as_str()))
        .bind(session.session_id)
        .bind(session.vehicle_id)
        .bind(session.started_at)
        .bind(session.ended_at)
        .bind(location_lat)
        .bind(location_lng)
        .bind(location_match.is_home)
        .bind(session.soc_start)
        .bind(session.soc_end)
        .bind(session.charge_limit)
        .bind(duration_minutes)
        .bind(energy_added_kwh)
        .bind(session.peak_charge_kw)
        .bind(Option::<f64>::None)
        .bind(session.energy_added_wh)
        .bind(session.energy_used_wh)
        .bind(session.avg_charge_rate_kw)
        .bind(session.peak_charge_kw)
        .bind(location_match.geofence_id)
        .bind(location_match.address_id)
        .bind(Option::<Uuid>::None)
        .bind(Option::<&str>::None)
        .bind(session.charger_type.as_deref())
        .fetch_optional(conn)
        .await?;
    anyhow::ensure!(
        saved == Some(session.session_id),
        "completed charge session ownership mismatch"
    );

    Ok(())
}

enum LiveProjection {
    Refresh(Box<rivian_poll::LiveSessionData>),
    Clear,
    Retain,
}
impl LiveProjection {
    fn plan(
        event: &parser::ChargingSessionEvent,
        received_at: DateTime<Utc>,
        active: Option<Uuid>,
    ) -> Self {
        match (
            rivian_poll::normalize_charging_session(event, received_at),
            active,
        ) {
            (Some(live), Some(_)) => Self::Refresh(Box::new(live)),
            (Some(_), None) => Self::Clear,
            (None, _) if should_clear_live_session(event) => Self::Clear,
            (None, _) => Self::Retain,
        }
    }
}

#[derive(Clone, Copy)]
enum Merge {
    Incoming,
    IncomingNonNull,
    ExistingNonNull,
    Earliest,
}
const ACTIVE_FIELDS: &[(&str, Merge)] = &[
    ("started_at", Merge::ExistingNonNull),
    ("location_lat", Merge::ExistingNonNull),
    ("location_lng", Merge::ExistingNonNull),
    ("source_location_lat", Merge::ExistingNonNull),
    ("source_location_lng", Merge::ExistingNonNull),
    ("soc_start", Merge::ExistingNonNull),
    ("charge_limit", Merge::ExistingNonNull),
];
const COMPLETED_FIELDS: &[(&str, Merge)] = &[
    ("started_at", Merge::Earliest),
    ("ended_at", Merge::Incoming),
    ("location_lat", Merge::IncomingNonNull),
    ("location_lng", Merge::IncomingNonNull),
    ("source_location_lat", Merge::IncomingNonNull),
    ("source_location_lng", Merge::IncomingNonNull),
    ("is_home", Merge::IncomingNonNull),
    ("soc_start", Merge::ExistingNonNull),
    ("soc_end", Merge::Incoming),
    ("charge_limit", Merge::IncomingNonNull),
    ("duration_minutes", Merge::Incoming),
    ("kwh_added", Merge::IncomingNonNull),
    ("max_charge_rate_kw", Merge::IncomingNonNull),
    ("energy_added_wh", Merge::IncomingNonNull),
    ("energy_used_wh", Merge::IncomingNonNull),
    ("avg_charge_rate_kw", Merge::IncomingNonNull),
    ("peak_voltage", Merge::IncomingNonNull),
    ("geofence_id", Merge::IncomingNonNull),
    ("address_id", Merge::IncomingNonNull),
    ("charger_type", Merge::IncomingNonNull),
    ("source", Merge::ExistingNonNull),
    ("data_confidence", Merge::ExistingNonNull),
];
fn charge_upsert(insert: &str, policies: &[(&str, Merge)]) -> String {
    let assignments = policies
        .iter()
        .map(|(column, policy)| {
            let saved = format!("charge_sessions.{column}");
            let incoming = format!("EXCLUDED.{column}");
            let expression = match policy {
                Merge::Incoming => incoming,
                Merge::IncomingNonNull => format!("COALESCE({incoming},{saved})"),
                Merge::ExistingNonNull => format!("COALESCE({saved},{incoming})"),
                Merge::Earliest => format!("LEAST({saved},{incoming})"),
            };
            format!("{column}={expression}")
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("{insert} ON CONFLICT(id) DO UPDATE SET {assignments} WHERE charge_sessions.vehicle_id=EXCLUDED.vehicle_id RETURNING id")
}
