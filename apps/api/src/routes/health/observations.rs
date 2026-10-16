use super::*;

pub(super) struct Observations {
    pub(super) latest: Option<LatestHealthTelemetry>,
    pub(super) tires: Option<TirePressures>,
    pub(super) closures: Option<Closures>,
}

#[derive(sqlx::FromRow)]
struct SelectedRows {
    latest_ts: Option<DateTime<Utc>>,
    latest_twelve_volt_health: Option<String>,
    latest_hv_thermal_event: Option<String>,
    latest_ota_current_version: Option<String>,
    latest_ota_available_version: Option<String>,
    latest_ota_status: Option<String>,
    latest_ota_current_status: Option<String>,
    latest_is_online: Option<bool>,
    tires_ts: Option<DateTime<Utc>>,
    tires_tire_fl_psi: Option<f64>,
    tires_tire_fr_psi: Option<f64>,
    tires_tire_rl_psi: Option<f64>,
    tires_tire_rr_psi: Option<f64>,
    tires_tire_fl_status: Option<String>,
    tires_tire_fr_status: Option<String>,
    tires_tire_rl_status: Option<String>,
    tires_tire_rr_status: Option<String>,
    closures_ts: Option<DateTime<Utc>>,
    closures_closure_frunk_closed: Option<bool>,
    closures_closure_liftgate_closed: Option<bool>,
    closures_closure_tailgate_closed: Option<bool>,
    closures_door_front_left_closed: Option<bool>,
    closures_door_front_right_closed: Option<bool>,
    closures_door_rear_left_closed: Option<bool>,
    closures_door_rear_right_closed: Option<bool>,
}

pub(super) async fn fetch(pool: &sqlx::PgPool, vehicle: Uuid) -> Result<Observations, AppError> {
    let row = sqlx::query_as::<_, SelectedRows>(
        r#"SELECT
          latest.ts AS latest_ts,
          latest.twelve_volt_health AS latest_twelve_volt_health,
          latest.hv_thermal_event AS latest_hv_thermal_event,
          latest.ota_current_version AS latest_ota_current_version,
          latest.ota_available_version AS latest_ota_available_version,
          latest.ota_status AS latest_ota_status,
          latest.ota_current_status AS latest_ota_current_status,
          latest.is_online AS latest_is_online,
          tires.ts AS tires_ts,
          tires.tire_fl_psi AS tires_tire_fl_psi,
          tires.tire_fr_psi AS tires_tire_fr_psi,
          tires.tire_rl_psi AS tires_tire_rl_psi,
          tires.tire_rr_psi AS tires_tire_rr_psi,
          tires.tire_fl_status AS tires_tire_fl_status,
          tires.tire_fr_status AS tires_tire_fr_status,
          tires.tire_rl_status AS tires_tire_rl_status,
          tires.tire_rr_status AS tires_tire_rr_status,
          closures.ts AS closures_ts,
          closures.closure_frunk_closed AS closures_closure_frunk_closed,
          closures.closure_liftgate_closed AS closures_closure_liftgate_closed,
          closures.closure_tailgate_closed AS closures_closure_tailgate_closed,
          closures.door_front_left_closed AS closures_door_front_left_closed,
          closures.door_front_right_closed AS closures_door_front_right_closed,
          closures.door_rear_left_closed AS closures_door_rear_left_closed,
          closures.door_rear_right_closed AS closures_door_rear_right_closed
        FROM (VALUES (1)) anchor(n)
        LEFT JOIN LATERAL (
            SELECT ts, twelve_volt_health, hv_thermal_event, ota_current_version, ota_available_version, ota_status, ota_current_status, is_online FROM timeseries.telemetry
            WHERE vehicle_id = $1 AND (twelve_volt_health IS NOT NULL OR hv_thermal_event IS NOT NULL OR ota_current_version IS NOT NULL OR ota_available_version IS NOT NULL OR ota_status IS NOT NULL OR ota_current_status IS NOT NULL OR is_online IS NOT NULL)
            ORDER BY ts DESC LIMIT 1
        ) latest ON true
        LEFT JOIN LATERAL (
            SELECT ts, tire_fl_psi, tire_fr_psi, tire_rl_psi, tire_rr_psi, tire_fl_status, tire_fr_status, tire_rl_status, tire_rr_status FROM timeseries.telemetry
            WHERE vehicle_id = $1 AND (tire_fl_psi IS NOT NULL OR tire_fr_psi IS NOT NULL OR tire_rl_psi IS NOT NULL OR tire_rr_psi IS NOT NULL)
            ORDER BY ts DESC LIMIT 1
        ) tires ON true
        LEFT JOIN LATERAL (
            SELECT ts, closure_frunk_closed, closure_liftgate_closed, closure_tailgate_closed, door_front_left_closed, door_front_right_closed, door_rear_left_closed, door_rear_right_closed FROM timeseries.telemetry
            WHERE vehicle_id = $1 AND (closure_frunk_closed IS NOT NULL OR closure_liftgate_closed IS NOT NULL OR closure_tailgate_closed IS NOT NULL OR door_front_left_closed IS NOT NULL OR door_front_right_closed IS NOT NULL OR door_rear_left_closed IS NOT NULL OR door_rear_right_closed IS NOT NULL)
            ORDER BY ts DESC LIMIT 1
        ) closures ON true
        "#,
    ).bind(vehicle).fetch_one(pool).await?;
    Ok(Observations {
        latest: row.latest_ts.map(|ts| LatestHealthTelemetry {
            ts,
            twelve_volt_health: row.latest_twelve_volt_health,
            hv_thermal_event: row.latest_hv_thermal_event,
            ota_current_version: row.latest_ota_current_version,
            ota_available_version: row.latest_ota_available_version,
            ota_status: row.latest_ota_status,
            ota_current_status: row.latest_ota_current_status,
            is_online: row.latest_is_online,
        }),
        tires: row.tires_ts.map(|ts| TirePressures {
            ts,
            tire_fl_psi: row.tires_tire_fl_psi,
            tire_fr_psi: row.tires_tire_fr_psi,
            tire_rl_psi: row.tires_tire_rl_psi,
            tire_rr_psi: row.tires_tire_rr_psi,
            tire_fl_status: row.tires_tire_fl_status,
            tire_fr_status: row.tires_tire_fr_status,
            tire_rl_status: row.tires_tire_rl_status,
            tire_rr_status: row.tires_tire_rr_status,
        }),
        closures: row.closures_ts.map(|ts| Closures {
            ts,
            closure_frunk_closed: row.closures_closure_frunk_closed,
            closure_liftgate_closed: row.closures_closure_liftgate_closed,
            closure_tailgate_closed: row.closures_closure_tailgate_closed,
            door_front_left_closed: row.closures_door_front_left_closed,
            door_front_right_closed: row.closures_door_front_right_closed,
            door_rear_left_closed: row.closures_door_rear_left_closed,
            door_rear_right_closed: row.closures_door_rear_right_closed,
        }),
    })
}
