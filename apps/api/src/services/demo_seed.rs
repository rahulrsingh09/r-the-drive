use chrono::{DateTime, Duration, Timelike, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::errors::AppError;

mod history_plan;
mod telemetry;

const FIXTURE_JSON: &str = include_str!("../../fixtures/demo/history-v1.json");
const DEMO_OSM_ID_BASE: i64 = -9_100_000;
const DEMO_ACTIVE_DAY_OFFSETS: [i64; 12] = [0, 1, 2, 4, 5, 6, 7, 8, 10, 11, 12, 13];

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DemoFixtureSpec {
    pub schema_version: u32,
    pub source_model: String,
    pub window_days: i64,
    pub telemetry_rows: i64,
    pub active_days: i32,
    pub trip_count: i32,
    pub charge_count: i32,
    pub weather_sample_count: i32,
    pub coverage: DemoFixtureCoverage,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DemoFixtureCoverage {
    pub location: f64,
    pub battery: f64,
    pub tires: f64,
    pub doors: f64,
    pub outside_temperature: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DemoSeedCounts {
    pub telemetry: i64,
    pub trips: i64,
    pub charges: i64,
    pub weather_samples: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DemoSeedSummary {
    pub seeded_at: DateTime<Utc>,
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    pub counts: DemoSeedCounts,
}

#[derive(Clone, Copy)]
struct DemoAddress {
    label: &'static str,
    road: &'static str,
    city: &'static str,
    latitude: f64,
    longitude: f64,
}

const DEMO_ADDRESSES: [DemoAddress; 4] = [
    DemoAddress {
        label: "Demo National Mall",
        road: "National Mall",
        city: "Washington",
        latitude: 38.8895,
        longitude: -77.0353,
    },
    DemoAddress {
        label: "Demo Arlington",
        road: "Memorial Avenue",
        city: "Arlington",
        latitude: 38.8816,
        longitude: -77.0910,
    },
    DemoAddress {
        label: "Demo Dulles",
        road: "Saarinen Circle",
        city: "Dulles",
        latitude: 38.9531,
        longitude: -77.4565,
    },
    DemoAddress {
        label: "Demo Great Falls",
        road: "Old Dominion Drive",
        city: "McLean",
        latitude: 38.9987,
        longitude: -77.2544,
    },
];

#[derive(Clone, Copy)]
struct ModelProfile {
    capacity_wh: f64,
    max_range_mi: f64,
    has_liftgate: bool,
    has_truck_closures: bool,
}

fn model_profile(model: &str) -> Result<ModelProfile, AppError> {
    const PROFILES: &[(&[&str], ModelProfile)] = &[
        (
            &["R1S"],
            ModelProfile {
                capacity_wh: 135_000.0,
                max_range_mi: 260.0,
                has_liftgate: true,
                has_truck_closures: false,
            },
        ),
        (
            &["R1T"],
            ModelProfile {
                capacity_wh: 135_000.0,
                max_range_mi: 248.0,
                has_liftgate: false,
                has_truck_closures: true,
            },
        ),
        (
            &["R2", "R2S", "R2-S"],
            ModelProfile {
                capacity_wh: 82_000.0,
                max_range_mi: 300.0,
                has_liftgate: true,
                has_truck_closures: false,
            },
        ),
    ];
    PROFILES
        .iter()
        .find(|(names, _)| names.contains(&model))
        .map(|(_, profile)| *profile)
        .ok_or_else(|| AppError::Validation("model must be one of R1T, R1S, R2".into()))
}

pub fn load_fixture() -> Result<DemoFixtureSpec, AppError> {
    let fixture: DemoFixtureSpec = serde_json::from_str(FIXTURE_JSON)
        .map_err(|error| AppError::Validation(format!("demo fixture is invalid: {error}")))?;
    validate_fixture(&fixture)?;
    Ok(fixture)
}

pub fn validate_fixture(fixture: &DemoFixtureSpec) -> Result<(), AppError> {
    if fixture.schema_version != 1 {
        return Err(AppError::Validation(format!(
            "unsupported demo fixture schema version {}",
            fixture.schema_version
        )));
    }
    if fixture.source_model != "R1S" || fixture.window_days != 14 {
        return Err(AppError::Validation(
            "demo fixture must be a 14-day sanitized R1S profile".into(),
        ));
    }
    if !(1_000..=20_000).contains(&fixture.telemetry_rows)
        || !(7..=14).contains(&fixture.active_days)
        || !(10..=100).contains(&fixture.trip_count)
        || !(2..=20).contains(&fixture.charge_count)
        || fixture.weather_sample_count < fixture.trip_count
    {
        return Err(AppError::Validation(
            "demo fixture density is outside the approved bounds".into(),
        ));
    }
    for (name, value) in [
        ("location", fixture.coverage.location),
        ("battery", fixture.coverage.battery),
        ("tires", fixture.coverage.tires),
        ("doors", fixture.coverage.doors),
        ("outside_temperature", fixture.coverage.outside_temperature),
    ] {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(AppError::Validation(format!(
                "demo fixture coverage '{name}' must be between zero and one"
            )));
        }
    }
    Ok(())
}

/// Serialize create-if-missing requests and recheck authority under the user lock.
pub(crate) async fn lock_demo_creator(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
) -> Result<(), AppError> {
    let creator = sqlx::query_as::<_, (bool, String)>(
        "SELECT is_disabled, role FROM panda.users WHERE id=$1 FOR UPDATE",
    )
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await?;
    match creator {
        Some((false, role)) if matches!(role.as_str(), "admin" | "super_user") => Ok(()),
        Some(_) => Err(AppError::Forbidden),
        None => Err(AppError::Unauthorized),
    }
}

/// Populate only an empty demo. This path never requires history deletion.
pub async fn seed_new_demo_vehicle(
    tx: &mut Transaction<'_, Postgres>,
    vehicle_id: Uuid,
    model: &str,
    anchor: DateTime<Utc>,
) -> Result<DemoSeedSummary, AppError> {
    lock_demo_vehicle(tx, vehicle_id).await?;
    let populated: bool = sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1 FROM timeseries.telemetry WHERE vehicle_id=$1
            UNION ALL SELECT 1 FROM panda.trips WHERE vehicle_id=$1
            UNION ALL SELECT 1 FROM panda.charge_sessions WHERE vehicle_id=$1
            UNION ALL SELECT 1 FROM panda.vehicle_state_periods WHERE vehicle_id=$1
            UNION ALL SELECT 1 FROM panda.software_versions WHERE vehicle_id=$1
            UNION ALL SELECT 1 FROM panda.battery_capacity_snapshots WHERE vehicle_id=$1
            UNION ALL SELECT 1 FROM panda.rivian_charge_curve_points WHERE vehicle_id=$1
            UNION ALL SELECT 1 FROM panda.vehicle_images WHERE vehicle_id=$1
            UNION ALL SELECT 1 FROM panda.vehicle_latest_status WHERE vehicle_id=$1
            UNION ALL SELECT 1 FROM panda.vehicle_runtime_state WHERE vehicle_id=$1
        )",
    )
    .bind(vehicle_id)
    .fetch_one(&mut **tx)
    .await?;
    if populated {
        return Err(AppError::Conflict(
            "Fresh demo seeding requires an empty vehicle; existing data was preserved.".into(),
        ));
    }
    populate_demo_history(tx, vehicle_id, model, anchor).await
}

/// Replace illustrative history. The HTTP route forbids this in production.
pub async fn seed_demo_vehicle(
    tx: &mut Transaction<'_, Postgres>,
    vehicle_id: Uuid,
    model: &str,
    anchor: DateTime<Utc>,
) -> Result<DemoSeedSummary, AppError> {
    // Reject invalid input before resetting anything in the caller's transaction.
    load_fixture()?;
    model_profile(model)?;
    lock_demo_vehicle(tx, vehicle_id).await?;
    clear_seeded_history(tx, vehicle_id).await?;
    populate_demo_history(tx, vehicle_id, model, anchor).await
}

async fn lock_demo_vehicle(
    tx: &mut Transaction<'_, Postgres>,
    vehicle_id: Uuid,
) -> Result<(), AppError> {
    let key: String =
        sqlx::query_scalar("SELECT rivian_vehicle_id FROM panda.vehicles WHERE id=$1 FOR UPDATE")
            .bind(vehicle_id)
            .fetch_optional(&mut **tx)
            .await?
            .ok_or(AppError::NotFound)?;
    if !key.starts_with("demo-") {
        return Err(AppError::Validation(
            "only demo vehicles can receive demo history".into(),
        ));
    }
    Ok(())
}

async fn populate_demo_history(
    tx: &mut Transaction<'_, Postgres>,
    vehicle_id: Uuid,
    model: &str,
    anchor: DateTime<Utc>,
) -> Result<DemoSeedSummary, AppError> {
    let fixture = load_fixture()?;
    let profile = model_profile(model)?;
    let anchor = anchor.with_nanosecond(0).unwrap_or(anchor);
    let window_start = anchor - Duration::days(fixture.window_days);

    let address_ids = ensure_demo_addresses(tx).await?;
    seed_trips(
        tx,
        vehicle_id,
        anchor,
        fixture.trip_count,
        fixture.weather_sample_count,
        &address_ids,
    )
    .await?;
    seed_charges(tx, vehicle_id, anchor, fixture.charge_count, &address_ids).await?;
    seed_telemetry(tx, vehicle_id, anchor, &fixture, profile).await?;
    seed_state_periods(tx, vehicle_id).await?;
    seed_software_history(tx, vehicle_id, anchor).await?;
    seed_latest_status(tx, vehicle_id, model, anchor, profile).await?;

    let counts = read_seed_counts(tx, vehicle_id).await?;
    if counts.telemetry != fixture.telemetry_rows
        || counts.trips != i64::from(fixture.trip_count)
        || counts.charges != i64::from(fixture.charge_count)
        || counts.weather_samples != i64::from(fixture.weather_sample_count)
    {
        return Err(AppError::Validation(format!(
            "demo seed count mismatch: telemetry={}, trips={}, charges={}, weather={}",
            counts.telemetry, counts.trips, counts.charges, counts.weather_samples
        )));
    }

    Ok(DemoSeedSummary {
        seeded_at: anchor,
        window_start,
        window_end: anchor,
        counts,
    })
}

async fn clear_seeded_history(
    tx: &mut Transaction<'_, Postgres>,
    vehicle_id: Uuid,
) -> Result<(), AppError> {
    for statement in [
        "DELETE FROM timeseries.telemetry WHERE vehicle_id = $1",
        "DELETE FROM panda.vehicle_state_periods WHERE vehicle_id = $1",
        "DELETE FROM panda.rivian_charge_curve_points WHERE vehicle_id = $1",
        "DELETE FROM panda.trips WHERE vehicle_id = $1",
        "DELETE FROM panda.charge_sessions WHERE vehicle_id = $1",
        "DELETE FROM panda.software_versions WHERE vehicle_id = $1",
        "DELETE FROM panda.vehicle_images WHERE vehicle_id = $1",
    ] {
        sqlx::query(statement)
            .bind(vehicle_id)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

async fn ensure_demo_addresses(tx: &mut Transaction<'_, Postgres>) -> Result<Vec<Uuid>, AppError> {
    let mut ids = Vec::with_capacity(DEMO_ADDRESSES.len());
    for (index, address) in DEMO_ADDRESSES.iter().enumerate() {
        let osm_id = DEMO_OSM_ID_BASE - index as i64;
        let id = sqlx::query_scalar::<_, Uuid>(
            r#"INSERT INTO panda.addresses
                 (display_name, osm_id, latitude, longitude, road, city, state, country, raw)
               VALUES ($1, $2, $3, $4, $5, $6, 'DC', 'US',
                       jsonb_build_object('source', 'panda_demo_fixture', 'fixture_version', 1))
               ON CONFLICT (osm_id) DO UPDATE
               SET display_name = EXCLUDED.display_name,
                   latitude = EXCLUDED.latitude,
                   longitude = EXCLUDED.longitude,
                   road = EXCLUDED.road,
                   city = EXCLUDED.city,
                   state = EXCLUDED.state,
                   country = EXCLUDED.country,
                   raw = EXCLUDED.raw
               RETURNING id"#,
        )
        .bind(address.label)
        .bind(osm_id)
        .bind(address.latitude)
        .bind(address.longitude)
        .bind(address.road)
        .bind(address.city)
        .fetch_one(&mut **tx)
        .await?;
        ids.push(id);
    }
    Ok(ids)
}

async fn seed_trips(
    tx: &mut Transaction<'_, Postgres>,
    vehicle_id: Uuid,
    anchor: DateTime<Utc>,
    trip_count: i32,
    weather_sample_count: i32,
    address_ids: &[Uuid],
) -> Result<(), AppError> {
    for plan in history_plan::trips(anchor, trip_count, weather_sample_count) {
        let history_plan::TripSlot {
            index,
            route,
            destination,
            started_at,
            ended_at,
            duration_minutes,
            weather_samples: samples_for_trip,
        } = plan;
        let start = DEMO_ADDRESSES[route];
        let end = DEMO_ADDRESSES[destination];
        let mid_lat = (start.latitude + end.latitude) / 2.0 + 0.006;
        let mid_lng = (start.longitude + end.longitude) / 2.0 - 0.004;
        let distance_miles = 4.5 + f64::from(index % 7) * 3.1;
        let efficiency = 330.0 + f64::from(index % 6) * 28.0;
        let soc_start = 82.0 - f64::from(index % 8) * 5.0;
        let soc_end = (soc_start - (distance_miles / 3.2)).max(18.0);
        let outside_temp = 8.0 + f64::from(index % 9) * 2.2;
        let drive_mode = ["all_purpose", "conserve", "sport", "snow"][index as usize % 4];
        let route_preview = serde_json::json!([
            [start.longitude, start.latitude],
            [mid_lng, mid_lat],
            [end.longitude, end.latitude]
        ]);

        let trip_id = sqlx::query_scalar::<_, Uuid>(
            r#"INSERT INTO panda.trips
                 (vehicle_id, started_at, ended_at, start_lat, start_lng, end_lat, end_lng,
                  distance_miles, duration_seconds, soc_start, soc_end, efficiency_wh_per_mile,
                  max_speed_mph, drive_mode, outside_temp_c, avg_speed_mph, energy_wh, regen_wh,
                  elevation_gain_m, elevation_loss_m, inside_temp_avg_c, start_odometer_mi,
                  end_odometer_mi, start_position_ts, end_position_ts, start_address_id,
                  end_address_id, power_max_kw, power_min_kw, range_start_mi, range_end_mi,
                  energy_strategy, route_preview, route_preview_version, outside_temp_source)
               VALUES
                 ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,
                  $19,$20,$21,$22,$23,$2,$3,$24,$25,$26,$27,$28,$29,'telemetry_delta',$30,1,'open_meteo')
               RETURNING id"#,
        )
        .bind(vehicle_id)
        .bind(started_at)
        .bind(ended_at)
        .bind(start.latitude)
        .bind(start.longitude)
        .bind(end.latitude)
        .bind(end.longitude)
        .bind(distance_miles)
        .bind(duration_minutes * 60)
        .bind(soc_start)
        .bind(soc_end)
        .bind(efficiency)
        .bind(45.0 + f64::from(index % 5) * 4.0)
        .bind(drive_mode)
        .bind(outside_temp)
        .bind(distance_miles / (f64::from(duration_minutes) / 60.0))
        .bind(distance_miles * efficiency)
        .bind(distance_miles * efficiency * 0.12)
        .bind(18.0 + f64::from(index % 6) * 12.0)
        .bind(12.0 + f64::from(index % 4) * 9.0)
        .bind(20.5 + f64::from(index % 5) * 0.6)
        .bind(15_000.0 + f64::from(index) * 11.0)
        .bind(15_000.0 + f64::from(index) * 11.0 + distance_miles)
        .bind(address_ids[route])
        .bind(address_ids[destination])
        .bind(70.0 + f64::from(index % 5) * 8.0)
        .bind(-28.0 - f64::from(index % 4) * 5.0)
        .bind(245.0 - f64::from(index % 8) * 11.0)
        .bind(235.0 - f64::from(index % 8) * 11.0)
        .bind(route_preview)
        .fetch_one(&mut **tx)
        .await?;

        let mut samples = Vec::with_capacity(samples_for_trip as usize);
        for sample_index in 0..samples_for_trip {
            let fraction = if samples_for_trip == 1 {
                0.5
            } else {
                f64::from(sample_index) / f64::from(samples_for_trip - 1)
            };
            let elapsed_seconds = (f64::from(duration_minutes * 60) * fraction).round() as i32;
            let sampled_at = started_at + Duration::seconds(i64::from(elapsed_seconds));
            let latitude = start.latitude + (end.latitude - start.latitude) * fraction;
            let longitude = start.longitude + (end.longitude - start.longitude) * fraction;
            samples.push((
                sampled_at,
                elapsed_seconds,
                latitude,
                longitude,
                outside_temp + fraction * 1.8,
            ));
        }
        sqlx::query(
            "INSERT INTO panda.trip_weather_samples (trip_id,sampled_at,elapsed_seconds,provider_latitude,provider_longitude,temperature_c,source)
             SELECT $1,sampled_at,elapsed,lat,lng,temp,'open_meteo'
             FROM unnest($2::timestamptz[],$3::int[],$4::float8[],$5::float8[],$6::float8[]) AS s(sampled_at,elapsed,lat,lng,temp)"
        ).bind(trip_id)
            .bind(samples.iter().map(|s|s.0).collect::<Vec<_>>())
            .bind(samples.iter().map(|s|s.1).collect::<Vec<_>>())
            .bind(samples.iter().map(|s|s.2).collect::<Vec<_>>())
            .bind(samples.iter().map(|s|s.3).collect::<Vec<_>>())
            .bind(samples.iter().map(|s|s.4).collect::<Vec<_>>())
            .execute(&mut **tx).await?;

        sqlx::query(
            r#"INSERT INTO panda.weather_enrichment_jobs
                 (trip_id, status, attempts, next_attempt_at, completed_at, updated_at)
               VALUES ($1, 'succeeded', 1, $2, $2, $2)"#,
        )
        .bind(trip_id)
        .bind(anchor)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

async fn seed_charges(
    tx: &mut Transaction<'_, Postgres>,
    vehicle_id: Uuid,
    anchor: DateTime<Utc>,
    charge_count: i32,
    address_ids: &[Uuid],
) -> Result<(), AppError> {
    for plan in history_plan::charges(anchor, charge_count) {
        let history_plan::ChargeSlot {
            index,
            address: slot,
            started_at,
            ended_at,
            duration,
            kind,
        } = plan;
        let soc_start = 31.0 + f64::from(index) * 7.0;
        let soc_end = if kind == Some("dc") {
            82.0
        } else {
            (soc_start + 24.0).min(90.0)
        };
        let kwh_added = 18.0 + f64::from(index) * 7.5;
        let max_rate = if kind == Some("dc") { 186.0 } else { 11.2 };
        let charge_id = sqlx::query_scalar::<_, Uuid>(
            r#"INSERT INTO panda.charge_sessions
                 (vehicle_id, started_at, ended_at, location_lat, location_lng, source_location_lat, source_location_lng, is_home,
                  charger_type, kwh_added, soc_start, soc_end, charge_limit, max_charge_rate_kw,
                  duration_minutes, cost_usd, outside_temp_c, energy_added_wh, energy_used_wh,
                  avg_charge_rate_kw, address_id, network_vendor, is_free_session,
                  is_rivian_network, source, currency_code, data_confidence)
               VALUES ($1,$2,$3,$4,$5,$4,$5,$6,$7,$8,$9,$10,85,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,
                       FALSE,'telemetry','USD','telemetry') RETURNING id"#,
        )
        .bind(vehicle_id)
        .bind(started_at)
        .bind(ended_at)
        .bind(DEMO_ADDRESSES[slot].latitude)
        .bind(DEMO_ADDRESSES[slot].longitude)
        .bind(slot < 2)
        .bind(kind)
        .bind(kwh_added)
        .bind(soc_start)
        .bind(soc_end)
        .bind(max_rate)
        .bind(duration)
        .bind(kwh_added * if slot < 2 { 0.16 } else { 0.38 })
        .bind(11.0 + f64::from(index) * 3.0)
        .bind(kwh_added * 1_000.0)
        .bind(kwh_added * 1_045.0)
        .bind(kwh_added / (f64::from(duration) / 60.0))
        .bind(address_ids[slot])
        .bind(if kind == Some("dc") {
            Some("Demo Adventure Network")
        } else {
            None
        })
        .bind(kind == Some("dc"))
        .fetch_one(&mut **tx)
        .await?;

        if kind == Some("dc") {
            let curve: Vec<_> = (0..=24)
                .map(|point| {
                    let fraction = f64::from(point) / 24.0;
                    (
                        started_at
                            + Duration::seconds((fraction * f64::from(duration * 60)) as i64),
                        186.0 - fraction.powf(1.7) * 118.0,
                    )
                })
                .collect();
            sqlx::query(
                "INSERT INTO panda.rivian_charge_curve_points (vehicle_id,charge_session_id,ts,power_kw,captured_at)
                 SELECT $1,$2,ts,power,$5 FROM unnest($3::timestamptz[],$4::float8[]) AS curve(ts,power)"
            ).bind(vehicle_id).bind(charge_id)
                .bind(curve.iter().map(|p|p.0).collect::<Vec<_>>())
                .bind(curve.iter().map(|p|p.1).collect::<Vec<_>>())
                .bind(anchor).execute(&mut **tx).await?;
        }
    }
    Ok(())
}

async fn seed_telemetry(
    tx: &mut Transaction<'_, Postgres>,
    vehicle_id: Uuid,
    anchor: DateTime<Utc>,
    fixture: &DemoFixtureSpec,
    profile: ModelProfile,
) -> Result<(), AppError> {
    telemetry::seed(tx, vehicle_id, anchor, fixture, profile).await
}

async fn seed_state_periods(
    tx: &mut Transaction<'_, Postgres>,
    vehicle_id: Uuid,
) -> Result<(), AppError> {
    let anchors=sqlx::query_as::<_,history_plan::Period>(
        "SELECT 'drive'::text AS state,started_at,ended_at FROM panda.trips WHERE vehicle_id=$1
         UNION ALL SELECT 'charging'::text,started_at,ended_at FROM panda.charge_sessions WHERE vehicle_id=$1
         ORDER BY started_at"
    ).bind(vehicle_id).fetch_all(&mut **tx).await?;
    let periods = history_plan::periods(anchors);
    sqlx::query(
        "INSERT INTO panda.vehicle_state_periods (vehicle_id,state,started_at,ended_at)
         SELECT $1,state,started_at,ended_at FROM unnest($2::text[],$3::timestamptz[],$4::timestamptz[]) AS p(state,started_at,ended_at)"
    ).bind(vehicle_id)
        .bind(periods.iter().map(|p|p.state.as_str()).collect::<Vec<_>>())
        .bind(periods.iter().map(|p|p.started_at).collect::<Vec<_>>())
        .bind(periods.iter().map(|p|p.ended_at).collect::<Vec<_>>())
        .execute(&mut **tx).await?;
    Ok(())
}

async fn seed_software_history(
    tx: &mut Transaction<'_, Postgres>,
    vehicle_id: Uuid,
    anchor: DateTime<Utc>,
) -> Result<(), AppError> {
    sqlx::query(
        r#"INSERT INTO panda.software_versions
             (vehicle_id, version, installed_at, observed_until)
           VALUES
             ($1, '2026.10.0', $2 - interval '120 days', $2 - interval '45 days'),
             ($1, '2026.14.0', $2 - interval '45 days', $2 - interval '14 days'),
             ($1, '2026.18.0', $2 - interval '14 days', NULL)"#,
    )
    .bind(vehicle_id)
    .bind(anchor)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn seed_latest_status(
    tx: &mut Transaction<'_, Postgres>,
    vehicle_id: Uuid,
    model: &str,
    anchor: DateTime<Utc>,
    profile: ModelProfile,
) -> Result<(), AppError> {
    sqlx::query(
        r#"INSERT INTO panda.vehicle_latest_status
             (vehicle_id, ts, battery_level, battery_capacity_wh, distance_to_empty_mi,
              battery_limit, power_state, charger_state, charger_state_ts, charger_status,
              time_to_end_of_charge_min, drive_mode, gear_status, altitude_m, speed_mph,
              cabin_temp_c, driver_temp_c, heading_deg, odometer_miles,
              tire_fl_psi, tire_fr_psi, tire_rl_psi, tire_rr_psi,
              tire_fl_status, tire_fr_status, tire_rl_status, tire_rr_status,
              tire_fl_valid, tire_fr_valid, tire_rl_valid, tire_rr_valid,
              door_front_left_locked, door_front_right_locked, door_rear_left_locked,
              door_rear_right_locked, door_front_left_closed, door_front_right_closed,
              door_rear_left_closed, door_rear_right_closed, closure_frunk_locked,
              closure_frunk_closed, closure_liftgate_locked, closure_liftgate_closed,
              closure_tailgate_locked, closure_tailgate_closed, ota_current_version,
              ota_status, ota_current_status, hv_thermal_event, twelve_volt_health,
              charge_port_open, charger_derate_active, cabin_precon_status,
              cabin_precon_type, pet_mode_active, pet_mode_temp_ok, defrost_active,
              steering_wheel_heat, seat_fl_heat, seat_fr_heat, seat_rl_heat, seat_rr_heat,
              seat_fl_vent, seat_fr_vent, tonneau_locked, tonneau_closed,
              side_bin_left_locked, side_bin_right_locked, side_bin_left_closed,
              side_bin_right_closed, window_fl_closed, window_fr_closed, window_rl_closed,
              window_rr_closed, gear_guard_locked, gear_guard_video_status,
              wiper_fluid_low, brake_fluid_low, alarm_active, service_mode, updated_at)
           VALUES
             ($1,$2,68,$3,$4,85,'ready','Connected',$2,'chrgr_sts_connected_no_chrg',NULL,
              'all_purpose','park',18,0,22.1,21.0,118,15062,
              48.1,48.0,49.8,49.7,'normal','normal','normal','normal',
              TRUE,TRUE,TRUE,TRUE,TRUE,TRUE,TRUE,TRUE,TRUE,TRUE,TRUE,TRUE,
              TRUE,TRUE,
              CASE WHEN $5::bool THEN TRUE END, CASE WHEN $5::bool THEN TRUE END,
              CASE WHEN $6::bool THEN TRUE END, CASE WHEN $6::bool THEN TRUE END,
              '2026.18.0','idle','up_to_date','none','normal',
              TRUE,FALSE,'off','none',FALSE,TRUE,FALSE,0,0,0,0,0,0,0,
              CASE WHEN $6::bool THEN TRUE END, CASE WHEN $6::bool THEN TRUE END,
              CASE WHEN $6::bool THEN TRUE END, CASE WHEN $6::bool THEN TRUE END,
              CASE WHEN $6::bool THEN TRUE END, CASE WHEN $6::bool THEN TRUE END,
              TRUE,TRUE,TRUE,TRUE,TRUE,'idle',FALSE,FALSE,FALSE,FALSE,$2)
           ON CONFLICT (vehicle_id) DO UPDATE SET
             ts=EXCLUDED.ts, battery_level=EXCLUDED.battery_level,
             battery_capacity_wh=EXCLUDED.battery_capacity_wh,
             distance_to_empty_mi=EXCLUDED.distance_to_empty_mi,
             battery_limit=EXCLUDED.battery_limit, power_state=EXCLUDED.power_state,
             charger_state=EXCLUDED.charger_state, charger_state_ts=EXCLUDED.charger_state_ts,
             charger_status=EXCLUDED.charger_status, time_to_end_of_charge_min=EXCLUDED.time_to_end_of_charge_min,
             drive_mode=EXCLUDED.drive_mode, gear_status=EXCLUDED.gear_status,
             altitude_m=EXCLUDED.altitude_m, speed_mph=EXCLUDED.speed_mph,
             cabin_temp_c=EXCLUDED.cabin_temp_c, driver_temp_c=EXCLUDED.driver_temp_c,
             heading_deg=EXCLUDED.heading_deg, odometer_miles=EXCLUDED.odometer_miles,
             tire_fl_psi=EXCLUDED.tire_fl_psi, tire_fr_psi=EXCLUDED.tire_fr_psi,
             tire_rl_psi=EXCLUDED.tire_rl_psi, tire_rr_psi=EXCLUDED.tire_rr_psi,
             tire_fl_status=EXCLUDED.tire_fl_status, tire_fr_status=EXCLUDED.tire_fr_status,
             tire_rl_status=EXCLUDED.tire_rl_status, tire_rr_status=EXCLUDED.tire_rr_status,
             tire_fl_valid=EXCLUDED.tire_fl_valid, tire_fr_valid=EXCLUDED.tire_fr_valid,
             tire_rl_valid=EXCLUDED.tire_rl_valid, tire_rr_valid=EXCLUDED.tire_rr_valid,
             door_front_left_locked=EXCLUDED.door_front_left_locked,
             door_front_right_locked=EXCLUDED.door_front_right_locked,
             door_rear_left_locked=EXCLUDED.door_rear_left_locked,
             door_rear_right_locked=EXCLUDED.door_rear_right_locked,
             door_front_left_closed=EXCLUDED.door_front_left_closed,
             door_front_right_closed=EXCLUDED.door_front_right_closed,
             door_rear_left_closed=EXCLUDED.door_rear_left_closed,
             door_rear_right_closed=EXCLUDED.door_rear_right_closed,
             closure_frunk_locked=EXCLUDED.closure_frunk_locked,
             closure_frunk_closed=EXCLUDED.closure_frunk_closed,
             closure_liftgate_locked=EXCLUDED.closure_liftgate_locked,
             closure_liftgate_closed=EXCLUDED.closure_liftgate_closed,
             closure_tailgate_locked=EXCLUDED.closure_tailgate_locked,
             closure_tailgate_closed=EXCLUDED.closure_tailgate_closed,
             ota_current_version=EXCLUDED.ota_current_version, ota_status=EXCLUDED.ota_status,
             ota_current_status=EXCLUDED.ota_current_status,
             hv_thermal_event=EXCLUDED.hv_thermal_event, twelve_volt_health=EXCLUDED.twelve_volt_health,
             charge_port_open=EXCLUDED.charge_port_open,
             charger_derate_active=EXCLUDED.charger_derate_active,
             cabin_precon_status=EXCLUDED.cabin_precon_status,
             cabin_precon_type=EXCLUDED.cabin_precon_type,
             pet_mode_active=EXCLUDED.pet_mode_active, pet_mode_temp_ok=EXCLUDED.pet_mode_temp_ok,
             defrost_active=EXCLUDED.defrost_active, steering_wheel_heat=EXCLUDED.steering_wheel_heat,
             seat_fl_heat=EXCLUDED.seat_fl_heat, seat_fr_heat=EXCLUDED.seat_fr_heat,
             seat_rl_heat=EXCLUDED.seat_rl_heat, seat_rr_heat=EXCLUDED.seat_rr_heat,
             seat_fl_vent=EXCLUDED.seat_fl_vent, seat_fr_vent=EXCLUDED.seat_fr_vent,
             tonneau_locked=EXCLUDED.tonneau_locked, tonneau_closed=EXCLUDED.tonneau_closed,
             side_bin_left_locked=EXCLUDED.side_bin_left_locked,
             side_bin_right_locked=EXCLUDED.side_bin_right_locked,
             side_bin_left_closed=EXCLUDED.side_bin_left_closed,
             side_bin_right_closed=EXCLUDED.side_bin_right_closed,
             window_fl_closed=EXCLUDED.window_fl_closed, window_fr_closed=EXCLUDED.window_fr_closed,
             window_rl_closed=EXCLUDED.window_rl_closed, window_rr_closed=EXCLUDED.window_rr_closed,
             gear_guard_locked=EXCLUDED.gear_guard_locked,
             gear_guard_video_status=EXCLUDED.gear_guard_video_status,
             wiper_fluid_low=EXCLUDED.wiper_fluid_low, brake_fluid_low=EXCLUDED.brake_fluid_low,
             alarm_active=EXCLUDED.alarm_active, service_mode=EXCLUDED.service_mode,
             updated_at=EXCLUDED.updated_at"#,
    )
    .bind(vehicle_id)
    .bind(anchor)
    .bind(profile.capacity_wh)
    .bind(profile.max_range_mi * 0.68)
    .bind(profile.has_liftgate)
    .bind(profile.has_truck_closures)
    .execute(&mut **tx)
    .await?;

    sqlx::query(
        r#"INSERT INTO panda.vehicle_runtime_state
             (vehicle_id, is_online, last_event_at, worker_health, worker_health_msg, updated_at)
           VALUES ($1, FALSE, $2, 'passive', $3, $2)
           ON CONFLICT (vehicle_id) DO UPDATE SET
             is_online=FALSE, last_event_at=EXCLUDED.last_event_at,
             worker_health=EXCLUDED.worker_health,
             worker_health_msg=EXCLUDED.worker_health_msg, updated_at=EXCLUDED.updated_at"#,
    )
    .bind(vehicle_id)
    .bind(anchor)
    .bind(format!("Demo {model} data through {}", anchor.date_naive()))
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn read_seed_counts(
    tx: &mut Transaction<'_, Postgres>,
    vehicle_id: Uuid,
) -> Result<DemoSeedCounts, AppError> {
    let telemetry = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM timeseries.telemetry WHERE vehicle_id=$1",
    )
    .bind(vehicle_id)
    .fetch_one(&mut **tx)
    .await?;
    let trips =
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM panda.trips WHERE vehicle_id=$1")
            .bind(vehicle_id)
            .fetch_one(&mut **tx)
            .await?;
    let charges = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM panda.charge_sessions WHERE vehicle_id=$1",
    )
    .bind(vehicle_id)
    .fetch_one(&mut **tx)
    .await?;
    let weather_samples = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM panda.trip_weather_samples tw
         JOIN panda.trips t ON t.id=tw.trip_id WHERE t.vehicle_id=$1",
    )
    .bind(vehicle_id)
    .fetch_one(&mut **tx)
    .await?;
    Ok(DemoSeedCounts {
        telemetry,
        trips,
        charges,
        weather_samples,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_in_fixture_is_valid_and_contains_only_aggregate_shape() {
        let fixture = load_fixture().expect("fixture should validate");
        assert_eq!(fixture.telemetry_rows, 5_664);
        assert_eq!(fixture.trip_count, 31);
        assert!(!FIXTURE_JSON.contains("latitude"));
        assert!(!FIXTURE_JSON.contains("longitude"));
        assert!(!FIXTURE_JSON.contains("vin"));
        assert!(!FIXTURE_JSON.contains("started_at"));
    }

    #[test]
    fn model_profiles_keep_truck_and_suv_closures_distinct() {
        let r1t = model_profile("R1T").unwrap();
        let r1s = model_profile("R1S").unwrap();
        let r2 = model_profile("R2").unwrap();
        assert!(r1t.has_truck_closures);
        assert!(!r1t.has_liftgate);
        assert!(r1s.has_liftgate);
        assert!(!r1s.has_truck_closures);
        assert!(r2.has_liftgate);
        assert!(!r2.has_truck_closures);
        assert_eq!(model_profile("R2S").unwrap().has_liftgate, r2.has_liftgate);
    }

    #[tokio::test]
    #[ignore = "requires DATABASE_URL"]
    async fn seeds_every_model_with_stable_relational_counts() {
        let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
        let pool = sqlx::PgPool::connect(&database_url).await.unwrap();
        let test_email = format!("demo-seed-{}@example.com", Uuid::new_v4());
        let user_id = sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO panda.users (email, password_hash, role)
             VALUES ($1, 'demo-seed-test-only', 'user') RETURNING id",
        )
        .bind(&test_email)
        .fetch_one(&pool)
        .await
        .expect("create demo seed test user");

        for model in ["R1T", "R1S", "R2"] {
            let mut tx = pool.begin().await.unwrap();
            let vehicle_id = sqlx::query_scalar::<_, Uuid>(
                "INSERT INTO panda.vehicles
                   (user_id, rivian_vehicle_id, model, battery_capacity_wh, name)
                 VALUES ($1,$2,$3,135000,$4) RETURNING id",
            )
            .bind(user_id)
            .bind(format!("demo-{model}-seed-test"))
            .bind(model)
            .bind(format!("Demo {model}"))
            .fetch_one(&mut *tx)
            .await
            .unwrap();

            let first_anchor = Utc::now();
            let summary = seed_demo_vehicle(&mut tx, vehicle_id, model, first_anchor)
                .await
                .unwrap();
            assert_eq!(summary.counts.telemetry, 5_664);
            assert_eq!(summary.counts.trips, 31);
            assert_eq!(summary.counts.charges, 4);
            assert_eq!(summary.counts.weather_samples, 80);

            let unlinked_trips = sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM panda.trips trip
                 WHERE trip.vehicle_id=$1 AND NOT EXISTS (
                   SELECT 1 FROM timeseries.telemetry telemetry WHERE telemetry.trip_id=trip.id
                 )",
            )
            .bind(vehicle_id)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
            assert_eq!(unlinked_trips, 0, "every demo trip needs detail samples");

            let curve_points = sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM panda.rivian_charge_curve_points WHERE vehicle_id=$1",
            )
            .bind(vehicle_id)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
            assert_eq!(curve_points, 25);

            let pending_weather_jobs = sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM panda.weather_enrichment_jobs job
                 JOIN panda.trips trip ON trip.id=job.trip_id
                 WHERE trip.vehicle_id=$1 AND job.status <> 'succeeded'",
            )
            .bind(vehicle_id)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
            assert_eq!(pending_weather_jobs, 0);

            let out_of_bounds_locations = sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM timeseries.telemetry
                 WHERE vehicle_id=$1 AND latitude IS NOT NULL AND longitude IS NOT NULL
                   AND NOT (latitude BETWEEN 38.70 AND 39.10 AND longitude BETWEEN -77.60 AND -76.80)",
            )
            .bind(vehicle_id)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
            assert_eq!(out_of_bounds_locations, 0);

            let states = sqlx::query_scalar::<_, String>(
                "SELECT DISTINCT state FROM panda.vehicle_state_periods
                 WHERE vehicle_id=$1 ORDER BY state",
            )
            .bind(vehicle_id)
            .fetch_all(&mut *tx)
            .await
            .unwrap();
            for expected in ["charging", "drive", "ready", "sleep"] {
                assert!(
                    states.iter().any(|state| state == expected),
                    "missing {expected} period"
                );
            }

            let phantom_drain_periods = sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM panda.vehicle_state_periods period
                 WHERE period.vehicle_id=$1 AND period.state='sleep'
                   AND (SELECT count(*) FROM timeseries.telemetry telemetry
                        WHERE telemetry.vehicle_id=period.vehicle_id
                          AND telemetry.ts BETWEEN period.started_at AND period.ended_at
                          AND telemetry.battery_level IS NOT NULL) >= 2
                   AND (SELECT max(telemetry.battery_level)-min(telemetry.battery_level)
                        FROM timeseries.telemetry telemetry
                        WHERE telemetry.vehicle_id=period.vehicle_id
                          AND telemetry.ts BETWEEN period.started_at AND period.ended_at) > 0",
            )
            .bind(vehicle_id)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
            assert!(
                phantom_drain_periods > 0,
                "demo needs a measurable sleep drain example"
            );

            let closures = sqlx::query_as::<
                _,
                (
                    Option<bool>,
                    Option<bool>,
                    Option<bool>,
                    Option<bool>,
                    Option<bool>,
                    Option<bool>,
                ),
            >(
                "SELECT closure_liftgate_closed, closure_tailgate_closed,
                        tonneau_closed, side_bin_left_closed, side_bin_right_closed,
                        closure_frunk_closed
                 FROM panda.vehicle_latest_status WHERE vehicle_id=$1",
            )
            .bind(vehicle_id)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
            match model {
                "R1S" => {
                    assert_eq!(closures.0, Some(true));
                    assert_eq!(closures.1, None);
                    assert_eq!(closures.2, None);
                }
                "R1T" => {
                    assert_eq!(closures.0, None);
                    assert_eq!(closures.1, Some(true));
                    assert_eq!(closures.2, Some(true));
                    assert_eq!(closures.3, Some(true));
                    assert_eq!(closures.4, Some(true));
                }
                "R2" => {
                    assert_eq!(
                        (closures.0, closures.1, closures.2, closures.3, closures.4),
                        (Some(true), None, None, None, None)
                    );
                    assert_eq!(closures.5, Some(true));
                }
                _ => unreachable!(),
            }

            let connection = sqlx::query_as::<
                _,
                (Option<String>, Option<String>, Option<i32>, Option<bool>),
            >(
                "SELECT charger_state, charger_status, time_to_end_of_charge_min, charge_port_open
                 FROM panda.vehicle_latest_status WHERE vehicle_id=$1",
            )
            .bind(vehicle_id)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
            assert_eq!(connection.0.as_deref(), Some("Connected"));
            assert_eq!(connection.1.as_deref(), Some("chrgr_sts_connected_no_chrg"));
            assert_eq!(connection.2, None);
            assert_eq!(connection.3, Some(true));

            sqlx::query("UPDATE panda.vehicles SET name='Keep this demo name' WHERE id=$1")
                .bind(vehicle_id)
                .execute(&mut *tx)
                .await
                .unwrap();
            let refreshed = seed_demo_vehicle(
                &mut tx,
                vehicle_id,
                model,
                first_anchor + Duration::hours(1),
            )
            .await
            .unwrap();
            assert_eq!(refreshed.counts.telemetry, 5_664);
            assert_eq!(refreshed.counts.trips, 31);
            assert_eq!(refreshed.counts.charges, 4);
            assert_eq!(refreshed.counts.weather_samples, 80);
            let preserved_name =
                sqlx::query_scalar::<_, String>("SELECT name FROM panda.vehicles WHERE id=$1")
                    .bind(vehicle_id)
                    .fetch_one(&mut *tx)
                    .await
                    .unwrap();
            assert_eq!(preserved_name, "Keep this demo name");
            tx.rollback().await.unwrap();
        }

        sqlx::query("DELETE FROM panda.users WHERE id = $1")
            .bind(user_id)
            .execute(&pool)
            .await
            .expect("delete demo seed test user");
    }
}

#[cfg(test)]
#[path = "__tests__/demo_generation.rs"]
mod characterization;
