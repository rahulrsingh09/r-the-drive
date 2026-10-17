use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

struct SensorInventory {
    sensor_id: &'static str,
    fields: &'static [&'static str],
}

#[derive(Serialize)]
struct AuditModelRecord {
    model: String,
    vehicle_count: i64,
    sample_count: i64,
    first_seen: Option<DateTime<Utc>>,
    last_seen: Option<DateTime<Utc>>,
    disposition: &'static str,
}

#[derive(Serialize)]
struct SensorAuditRecord {
    sensor_id: String,
    fields: Vec<String>,
    global_disposition: &'static str,
    models: Vec<AuditModelRecord>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let database_url = std::env::var("DATABASE_URL").context("DATABASE_URL must be set")?;
    let vehicle_filter = parse_vehicle_filter()?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .connect(&database_url)
        .await?;
    let outcome = async {
        let mut report = Vec::new();
        for sensor in inventory() {
            report.push(audit_sensor(&pool, sensor, vehicle_filter).await?);
        }

        println!("{}", serde_json::to_string_pretty(&report)?);
        Ok(())
    }
    .await;
    pool.close().await;
    outcome
}

fn parse_vehicle_filter() -> Result<Option<Uuid>> {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    let Some(position) = arguments
        .iter()
        .position(|argument| argument == "--vehicle")
    else {
        return Ok(None);
    };
    arguments
        .get(position + 1)
        .context("expected UUID after --vehicle")?
        .parse()
        .context("invalid vehicle UUID")
        .map(Some)
}

async fn audit_sensor(
    pool: &sqlx::PgPool,
    sensor: &SensorInventory,
    vehicle_filter: Option<Uuid>,
) -> Result<SensorAuditRecord> {
    let predicate = sensor
        .fields
        .iter()
        .map(|field| format!("t.{field} IS NOT NULL"))
        .collect::<Vec<_>>()
        .join(" OR ");
    let sql = format!(
        r#"
        SELECT v.model, COUNT(*)::bigint AS vehicle_count, SUM(samples.count)::bigint AS sample_count,
               MIN(samples.first_seen) AS first_seen, MAX(samples.last_seen) AS last_seen
        FROM panda.vehicles v
        CROSS JOIN LATERAL (
            SELECT COUNT(*)::bigint AS count, MIN(t.ts) AS first_seen, MAX(t.ts) AS last_seen
            FROM timeseries.telemetry t WHERE t.vehicle_id = v.id AND ({predicate})
        ) samples
        WHERE ($1::uuid IS NULL OR v.id = $1)
        GROUP BY v.model ORDER BY v.model
    "#
    );
    let rows = sqlx::query_as::<
        _,
        (
            String,
            i64,
            i64,
            Option<DateTime<Utc>>,
            Option<DateTime<Utc>>,
        ),
    >(sqlx::AssertSqlSafe(sql))
    .bind(vehicle_filter)
    .fetch_all(pool)
    .await?;
    let mut global_disposition = "remove-from-shared-defaults";
    let models = rows
        .into_iter()
        .map(
            |(model, vehicle_count, sample_count, first_seen, last_seen)| {
                let disposition = if sample_count > 0 {
                    global_disposition = "keep-historical-fallback";
                    "keep-historical-fallback"
                } else {
                    "remove-from-shared-defaults"
                };
                AuditModelRecord {
                    model,
                    vehicle_count,
                    sample_count,
                    first_seen,
                    last_seen,
                    disposition,
                }
            },
        )
        .collect();
    Ok(SensorAuditRecord {
        sensor_id: sensor.sensor_id.into(),
        fields: sensor.fields.iter().map(|field| (*field).into()).collect(),
        global_disposition,
        models,
    })
}

fn inventory() -> &'static [SensorInventory] {
    &[
        SensorInventory {
            sensor_id: "battery_health_status",
            fields: &["twelve_volt_health"],
        },
        SensorInventory {
            sensor_id: "hv_thermal",
            fields: &["hv_thermal_event"],
        },
        SensorInventory {
            sensor_id: "ota_current_version",
            fields: &["ota_current_version"],
        },
        SensorInventory {
            sensor_id: "ota_available_version",
            fields: &["ota_available_version"],
        },
        SensorInventory {
            sensor_id: "brake_fluid_warning",
            fields: &["brake_fluid_low"],
        },
        SensorInventory {
            sensor_id: "wiper_fluid_warning",
            fields: &["wiper_fluid_low"],
        },
        SensorInventory {
            sensor_id: "alarm_status",
            fields: &["alarm_active"],
        },
        SensorInventory {
            sensor_id: "service_mode",
            fields: &["service_mode"],
        },
        SensorInventory {
            sensor_id: "gear_guard_locked",
            fields: &["gear_guard_locked", "gear_guard_video_status"],
        },
        SensorInventory {
            sensor_id: "charge_port_open",
            fields: &["charge_port_open"],
        },
        SensorInventory {
            sensor_id: "charger_derate_active",
            fields: &["charger_derate_active"],
        },
        SensorInventory {
            sensor_id: "defrost_active",
            fields: &["defrost_active"],
        },
        SensorInventory {
            sensor_id: "cabin_precon",
            fields: &["cabin_precon_status", "cabin_precon_type"],
        },
        SensorInventory {
            sensor_id: "pet_mode",
            fields: &["pet_mode_active", "pet_mode_temp_ok"],
        },
        SensorInventory {
            sensor_id: "seat_fl_heat",
            fields: &["seat_fl_heat"],
        },
        SensorInventory {
            sensor_id: "seat_fr_heat",
            fields: &["seat_fr_heat"],
        },
        SensorInventory {
            sensor_id: "seat_rl_heat",
            fields: &["seat_rl_heat"],
        },
        SensorInventory {
            sensor_id: "seat_rr_heat",
            fields: &["seat_rr_heat"],
        },
        SensorInventory {
            sensor_id: "seat_fl_vent",
            fields: &["seat_fl_vent"],
        },
        SensorInventory {
            sensor_id: "seat_fr_vent",
            fields: &["seat_fr_vent"],
        },
        SensorInventory {
            sensor_id: "steering_wheel_heat",
            fields: &["steering_wheel_heat"],
        },
        SensorInventory {
            sensor_id: "tonneau_status",
            fields: &["tonneau_closed", "tonneau_locked"],
        },
        SensorInventory {
            sensor_id: "window_status",
            fields: &[
                "window_fl_closed",
                "window_fr_closed",
                "window_rl_closed",
                "window_rr_closed",
            ],
        },
        SensorInventory {
            sensor_id: "tire_pressure",
            fields: &[
                "tire_fl_psi",
                "tire_fr_psi",
                "tire_rl_psi",
                "tire_rr_psi",
                "tire_fl_status",
                "tire_fr_status",
                "tire_rl_status",
                "tire_rr_status",
                "tire_fl_valid",
                "tire_fr_valid",
                "tire_rl_valid",
                "tire_rr_valid",
            ],
        },
        SensorInventory {
            sensor_id: "closures",
            fields: &[
                "closure_frunk_closed",
                "closure_liftgate_closed",
                "closure_tailgate_closed",
                "door_front_left_closed",
                "door_front_right_closed",
                "door_rear_left_closed",
                "door_rear_right_closed",
            ],
        },
    ]
}
