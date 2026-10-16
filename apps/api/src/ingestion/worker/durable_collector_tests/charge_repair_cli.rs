use super::*;
use chrono::{DateTime, Duration, Utc};
use panda_api::ingestion::charge_detector::{
    ActiveChargeSessionSnapshot, ChargeDetectorState, ChargeEvent,
};
use panda_api::models::telemetry::{ChargerState, TelemetryEvent};
use sqlx::{postgres::PgConnectOptions, Executor, PgPool};
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB"]
async fn reconstruction_skips_active_and_pending_vehicles_but_repairs_unpinned_history() {
    let base = std::env::var("TRIP_DURABILITY_TEST_DATABASE_URL").unwrap();
    let options: PgConnectOptions = base.parse().unwrap();
    let admin = PgPool::connect_with(options.clone().database("postgres"))
        .await
        .unwrap();
    let name = format!("panda_charge_cli_{}", Uuid::new_v4().simple());
    admin
        .execute(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
        .await
        .unwrap();
    let pool = PgPool::connect_with(options.database(&name)).await.unwrap();
    panda_api::db::migrations::MIGRATOR
        .run(&pool)
        .await
        .unwrap();
    let user = Uuid::new_v4();
    let protected = Uuid::new_v4();
    let other = Uuid::new_v4();
    let active_id = Uuid::new_v4();
    let other_id = Uuid::new_v4();
    let at: DateTime<Utc> =
        sqlx::query_scalar("SELECT date_trunc('seconds',now())-interval '20 minutes'")
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("INSERT INTO panda.users(id,email,password_hash) VALUES($1,'charge-cli@example.invalid','unused')").bind(user).execute(&pool).await.unwrap();
    for (vehicle, id, key) in [
        (protected, active_id, "synthetic-protected"),
        (other, other_id, "synthetic-other"),
    ] {
        sqlx::query(
            "INSERT INTO panda.vehicles(id,user_id,rivian_vehicle_id,model) VALUES($1,$2,$3,'R1T')",
        )
        .bind(vehicle)
        .bind(user)
        .bind(key)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO panda.charge_sessions(id,vehicle_id,started_at,soc_start,source) VALUES($1,$2,$3,30,'telemetry')").bind(id).bind(vehicle).bind(at).execute(&pool).await.unwrap();
        for (seconds, soc) in [(0, 30.0), (300, 35.0), (600, 40.0)] {
            sqlx::query("INSERT INTO timeseries.telemetry(ts,vehicle_id,charge_session_id,charger_state,power_kw,battery_level,battery_capacity_wh) VALUES($1,$2,$3,'charging',12,$4,100000)")
                .bind(at+Duration::seconds(seconds)).bind(vehicle).bind((vehicle == protected).then_some(id)).bind(soc).execute(&pool).await.unwrap();
        }
    }
    let mut detector = ChargeDetectorState::from_snapshot(
        protected,
        ActiveChargeSessionSnapshot {
            session_id: active_id,
            started_at: at,
            location_lat: None,
            location_lng: None,
            soc_start: Some(30.0),
            last_soc: Some(40.0),
            charge_limit: None,
            battery_capacity_wh: Some(100000.0),
            last_charge_ts: at + Duration::seconds(600),
            last_power_ts: Some(at + Duration::seconds(600)),
            energy_used_wh: 2000.0,
            peak_charge_kw: 12.0,
        },
    );
    let checkpoint =
        serde_json::json!({"version":1,"sequence":3,"detector":detector,"last_observation":null});
    sqlx::query("INSERT INTO panda.active_charge_checkpoints(vehicle_id,active_session_id,snapshot) VALUES($1,$2,$3)").bind(protected).bind(active_id).bind(checkpoint).execute(&pool).await.unwrap();
    assert_eq!(
        repair_charge_sessions_from_telemetry(&pool).await.unwrap(),
        1,
        "an unrelated unpinned vehicle is still repaired"
    );
    let row: (Option<DateTime<Utc>>, Option<f64>) =
        sqlx::query_as("SELECT ended_at,energy_added_wh FROM panda.charge_sessions WHERE id=$1")
            .bind(active_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(row, (None, None));
    let mut stop = TelemetryEvent::empty(protected, at + Duration::seconds(601));
    stop.charger_state = Some(ChargerState::Done);
    let ChargeEvent::SessionEnded(completion) = detector.process(&stop) else {
        panic!("completion")
    };
    sqlx::query("INSERT INTO panda.pending_charge_completions(session_id,vehicle_id,session) VALUES($1,$2,$3)").bind(active_id).bind(protected).bind(serde_json::json!({"version":1,"session":completion})).execute(&pool).await.unwrap();
    sqlx::query("UPDATE panda.active_charge_checkpoints SET active_session_id=NULL,snapshot=$2 WHERE vehicle_id=$1").bind(protected).bind(serde_json::json!({"version":1,"sequence":4,"detector":detector,"last_observation":null})).execute(&pool).await.unwrap();
    repair_charge_sessions_from_telemetry(&pool).await.unwrap();
    let row: (Option<DateTime<Utc>>, Option<f64>) =
        sqlx::query_as("SELECT ended_at,energy_added_wh FROM panda.charge_sessions WHERE id=$1")
            .bind(active_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        row,
        (None, None),
        "pending completion keeps original canonical data untouched"
    );
    sqlx::query("DELETE FROM panda.pending_charge_completions WHERE session_id=$1")
        .bind(active_id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        repair_charge_sessions_from_telemetry(&pool).await.unwrap(),
        0,
        "the legacy CLI count reports newly stamped session IDs, not all updated rows"
    );
    let row: (DateTime<Utc>, f64) =
        sqlx::query_as("SELECT ended_at,energy_added_wh FROM panda.charge_sessions WHERE id=$1")
            .bind(active_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(row, (at + Duration::seconds(600), 10000.0));
    pool.close().await;
    admin
        .execute(sqlx::AssertSqlSafe(format!(
            "ALTER DATABASE {name} ALLOW_CONNECTIONS false"
        )))
        .await
        .unwrap();
    admin
        .execute(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE {name} WITH(FORCE)"
        )))
        .await
        .unwrap();
    admin.close().await;
}
