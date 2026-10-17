use axum::{extract::State, routing::get, Json, Router};
use serde_json::json;
use sqlx::Row;

use crate::{
    errors::AppError,
    middleware::auth::{AppState, AuthUser},
};

pub fn router() -> Router<AppState> {
    Router::new().route("/admin/rivian/stewardship", get(stewardship))
}

async fn stewardship(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<serde_json::Value>, AppError> {
    require_jwt_admin(&state, &auth).await?;
    let projection = TOTAL_COUNTERS
        .iter()
        .map(|field| format!("COALESCE(SUM({field}),0)::BIGINT AS {field}"))
        .collect::<Vec<_>>()
        .join(",");
    let totals = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {projection} FROM panda.rivian_stewardship_counters WHERE day >= CURRENT_DATE - 1"
    )))
    .fetch_one(&state.pool)
    .await?;
    let active_collectors = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)::BIGINT
        FROM panda.vehicle_runtime_state
        WHERE worker_health = 'connected'
          AND COALESCE(last_seen_at, last_event_at, updated_at) > now() - INTERVAL '10 minutes'
        "#,
    )
    .fetch_one(&state.pool)
    .await?;
    let raw_events_retained = sqlx::query_scalar::<_, i64>("SELECT COUNT(*)::BIGINT FROM panda.rivian_ws_raw_events WHERE received_at >= now() - ($1::int * INTERVAL '1 day')")
        .bind(state.config.rivian_raw_event_retention_days.max(1) as i32).fetch_one(&state.pool).await?;
    let rows = sqlx::query(
        r#"
        WITH vehicle_totals AS (
          SELECT vehicle_id,
                 SUM(ws_messages_received)::BIGINT AS ws_messages_received,
                 SUM(ws_heartbeats_received)::BIGINT AS ws_heartbeats_received,
                 SUM(ws_payload_messages_received)::BIGINT AS ws_payload_messages_received,
                 SUM(ws_reconnects)::BIGINT AS ws_reconnects,
                 SUM(telemetry_writes_persisted)::BIGINT AS telemetry_writes_persisted,
                 SUM(telemetry_writes_suppressed)::BIGINT AS telemetry_writes_suppressed,
                 SUM(collector_lock_skips)::BIGINT AS collector_lock_skips
          FROM panda.rivian_stewardship_counters
          WHERE day >= CURRENT_DATE - 1
          GROUP BY vehicle_id
        )
        SELECT
          v.id::TEXT AS vehicle_id,
          COALESCE(v.name, v.model) AS display_name,
          r.worker_health,
          r.last_seen_at,
          r.last_payload_at,
          r.last_persisted_at,
          r.last_heartbeat_at,
          COALESCE(c.ws_messages_received, 0)::BIGINT AS ws_messages_received,
          COALESCE(c.ws_heartbeats_received, 0)::BIGINT AS ws_heartbeats_received,
          COALESCE(c.ws_payload_messages_received, 0)::BIGINT AS ws_payload_messages_received,
          COALESCE(c.ws_reconnects, 0)::BIGINT AS ws_reconnects,
          COALESCE(c.telemetry_writes_persisted, 0)::BIGINT AS telemetry_writes_persisted,
          COALESCE(c.telemetry_writes_suppressed, 0)::BIGINT AS telemetry_writes_suppressed,
          COALESCE(c.collector_lock_skips, 0)::BIGINT AS collector_lock_skips
        FROM panda.vehicles v
        LEFT JOIN panda.vehicle_runtime_state r ON r.vehicle_id = v.id
        LEFT JOIN vehicle_totals c ON c.vehicle_id = v.id
        ORDER BY display_name
        "#,
    )
    .fetch_all(&state.pool)
    .await?;
    let vehicles: Vec<_> = rows
        .into_iter()
        .map(|row| {
            let mut fields = counter_values(&row, VEHICLE_COUNTERS);
            for key in ["vehicle_id", "display_name", "worker_health"] {
                fields.insert(
                    key.into(),
                    json!(row.try_get::<Option<String>, _>(key).ok().flatten()),
                );
            }
            for key in VEHICLE_CLOCKS {
                fields.insert(
                    (*key).into(),
                    json!(row
                        .try_get::<Option<chrono::DateTime<chrono::Utc>>, _>(*key)
                        .ok()
                        .flatten()),
                );
            }
            serde_json::Value::Object(fields)
        })
        .collect();
    Ok(Json(json!({
        "generated_at":chrono::Utc::now(), "retention_days":state.config.rivian_raw_event_retention_days,
        "raw_event_persistence_enabled":state.config.rivian_persist_raw_events,
        "duplicate_suppression_enabled":state.config.rivian_suppress_duplicate_telemetry,
        "active_collectors":active_collectors, "raw_events_retained":raw_events_retained,
        "totals_24h":counter_values(&totals,TOTAL_COUNTERS), "vehicles":vehicles,
    })))
}

async fn require_jwt_admin(state: &AppState, auth: &AuthUser) -> Result<(), AppError> {
    if auth.api_access_level.is_some() {
        return Err(AppError::Forbidden);
    }

    let role: Option<String> = sqlx::query_scalar("SELECT role FROM panda.users WHERE id = $1")
        .bind(auth.user_id)
        .fetch_optional(&state.pool)
        .await?
        .flatten();

    if matches!(role.as_deref(), Some("admin") | Some("super_user")) {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}

const TOTAL_COUNTERS: &[&str] = &[
    "ws_messages_received",
    "ws_heartbeats_received",
    "ws_payload_messages_received",
    "ws_control_messages_received",
    "ws_connections_opened",
    "ws_reconnects",
    "outbound_messages_sent",
    "outbound_graphql_requests",
    "telemetry_writes_persisted",
    "telemetry_writes_suppressed",
    "telemetry_suppressed_duplicate",
    "telemetry_suppressed_empty",
    "telemetry_suppressed_threshold",
    "collector_lock_skips",
    "raw_events_persisted",
];

const VEHICLE_COUNTERS: &[&str] = &[
    "ws_messages_received",
    "ws_heartbeats_received",
    "ws_payload_messages_received",
    "ws_reconnects",
    "telemetry_writes_persisted",
    "telemetry_writes_suppressed",
    "collector_lock_skips",
];

const VEHICLE_CLOCKS: &[&str] = &[
    "last_seen_at",
    "last_payload_at",
    "last_persisted_at",
    "last_heartbeat_at",
];

fn counter_values(
    row: &sqlx::postgres::PgRow,
    names: &[&str],
) -> serde_json::Map<String, serde_json::Value> {
    names
        .iter()
        .map(|name| {
            (
                (*name).to_owned(),
                json!(row.try_get::<i64, _>(*name).unwrap_or(0)),
            )
        })
        .collect()
}

#[cfg(test)]
mod full_replacement_tests {
    use super::*;
    use crate::authorization_test_support::Fixture;

    #[tokio::test]
    #[ignore = "requires disposable TimescaleDB DATABASE_URL and REDIS_URL"]
    async fn stewardship_counter_projection_and_jwt_restrictions() {
        let f = Fixture::new().await;
        let admin = f.user("admin").await;
        let regular = f.user("user").await;
        let vehicle = f.vehicle(regular, "demo-counter-projection").await;
        sqlx::query("INSERT INTO panda.rivian_stewardship_counters(vehicle_id,day,ws_messages_received,telemetry_writes_persisted) VALUES($1,CURRENT_DATE,3,5),($1,CURRENT_DATE-1,7,11),($1,CURRENT_DATE-2,1000,1000)")
            .bind(vehicle).execute(&f.state.pool).await.unwrap();
        let auth = AuthUser {
            user_id: admin,
            default_vehicle_id: None,
            api_access_level: None,
            api_vehicle_id: None,
        };
        let before = chrono::Utc::now();
        let value = stewardship(State(f.state.clone()), auth).await.unwrap().0;
        let generated: chrono::DateTime<chrono::Utc> =
            serde_json::from_value(value["generated_at"].clone()).unwrap();
        assert!(generated >= before && generated <= chrono::Utc::now());
        assert_eq!(value["totals_24h"]["ws_messages_received"], 10);
        assert_eq!(value["totals_24h"]["telemetry_writes_persisted"], 16);
        assert_eq!(value["totals_24h"].as_object().unwrap().len(), 15);
        assert_eq!(value["vehicles"].as_array().unwrap().len(), 1);
        assert_eq!(value["vehicles"][0]["vehicle_id"], vehicle.to_string());
        assert_eq!(value["vehicles"][0]["ws_messages_received"], 10);
        assert_eq!(value["vehicles"][0]["telemetry_writes_persisted"], 16);
        assert_eq!(value["vehicles"][0].as_object().unwrap().len(), 14);
        for (user, access) in [(regular, None), (admin, Some("read".to_owned()))] {
            let auth = AuthUser {
                user_id: user,
                default_vehicle_id: None,
                api_access_level: access,
                api_vehicle_id: None,
            };
            assert!(matches!(
                stewardship(State(f.state.clone()), auth).await,
                Err(AppError::Forbidden)
            ));
        }
        f.cleanup().await;
    }
}
