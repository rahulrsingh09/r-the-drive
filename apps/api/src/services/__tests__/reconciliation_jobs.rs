use super::*;
use crate::services::weather_enrichment::job_fixture::{evidence, id, snapshot, vehicle, Fixture};
use serde_json::json;
fn stats(s: BackfillStats) -> serde_json::Value {
    json!({"scanned":s.scanned,"filled":s.filled,"failed":s.failed,"skipped":s.skipped})
}
#[tokio::test]
#[ignore = "requires unique restricted JOBS_DATABASE_URL; synthetic history only"]
async fn jobs_charge_coordinate_selection_window_budget_and_idempotence() {
    let f = Fixture::new().await;
    let p = &f.state.pool;
    let v = vehicle(p, 60).await;
    let other = vehicle(p, 61).await;
    for n in 0..502 {
        sqlx::query("INSERT INTO panda.charge_sessions(id,vehicle_id,started_at,rivian_session_id,location_lat,source_location_lat) VALUES($1,$2,'2026-01-01Z'::timestamptz+make_interval(secs=>$3),$4,11,22)")
            .bind(id(6000+n)).bind(v).bind(n as f64*3600.).bind(format!("synthetic-{n}")).execute(p).await.unwrap();
        sqlx::query("INSERT INTO timeseries.telemetry(vehicle_id,ts,latitude,longitude) VALUES($1,'2026-01-01Z'::timestamptz+make_interval(secs=>$2),30,-97)")
            .bind(v).bind(n as f64*3600.+if n==501 {601.} else{600.}).execute(p).await.unwrap();
    }
    sqlx::query("INSERT INTO panda.charge_sessions(id,vehicle_id,started_at,location_override_mode) VALUES($1,$3,'2026-03-01Z','none'),($2,$4,'2026-03-01Z','automatic')")
        .bind(id(6998)).bind(id(6999)).bind(v).bind(other).execute(p).await.unwrap();
    let a = backfill_charge_session_coordinates(p, Some(v))
        .await
        .unwrap();
    assert_eq!(
        stats(a),
        json!({"scanned":500,"filled":499,"skipped":1,"failed":0})
    );
    let mut rows = vec![snapshot(p, stats(a)).await];
    let b = backfill_charge_session_coordinates(p, Some(v))
        .await
        .unwrap();
    assert_eq!(
        stats(b),
        json!({"scanned":3,"filled":2,"skipped":1,"failed":0})
    );
    rows.push(snapshot(p, stats(b)).await);
    let c = backfill_charge_session_coordinates(p, Some(v))
        .await
        .unwrap();
    assert_eq!(
        stats(c),
        json!({"scanned":1,"filled":0,"skipped":1,"failed":0})
    );
    rows.push(snapshot(p, stats(c)).await);
    evidence("charge-coordinates", &json!(rows));
    f.cleanup().await;
}
#[derive(Clone)]
struct Events(std::sync::Arc<std::sync::Mutex<Vec<String>>>);
impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Events {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        struct Message(Option<String>);
        impl tracing::field::Visit for Message {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                if field.name() == "message" {
                    self.0 = Some(format!("{value:?}"));
                }
            }
        }
        let mut message = Message(None);
        event.record(&mut message);
        if let Some(message) = message.0 {
            if message.starts_with("trip_enrichment.reconciler_") {
                self.0.lock().unwrap().push(message);
            }
        }
    }
}
#[tokio::test]
#[ignore = "requires unique restricted JOBS_DATABASE_URL; provider settings remain disabled"]
async fn jobs_reconciliation_continues_after_each_failed_location_stage() {
    use tracing_subscriber::prelude::*;
    let f = Fixture::new().await;
    let p = &f.state.pool;
    let v = vehicle(p, 70).await;
    crate::services::external_connections::ensure_defaults(p)
        .await
        .unwrap();
    sqlx::query("INSERT INTO panda.trips(id,vehicle_id,started_at,ended_at) VALUES($1,$2,'2026-01-01Z','2026-01-01T00:05Z')").bind(id(7000)).bind(v).execute(p).await.unwrap();
    sqlx::query(
        "INSERT INTO panda.charge_sessions(id,vehicle_id,started_at) VALUES($1,$2,'2026-01-01Z')",
    )
    .bind(id(7001))
    .bind(v)
    .execute(p)
    .await
    .unwrap();
    sqlx::query("INSERT INTO timeseries.telemetry(vehicle_id,ts,latitude,longitude) VALUES($1,'2026-01-01Z',30,-97)").bind(v).execute(p).await.unwrap();
    sqlx::query("ALTER TABLE panda.trips ADD CONSTRAINT synthetic_reject_coordinate CHECK(start_lat IS NULL)").execute(p).await.unwrap();
    let events = Events(Default::default());
    let subscriber = tracing_subscriber::registry().with(events.clone());
    let guard = tracing::subscriber::set_default(subscriber);
    reconcile_locations(p, &Client::new()).await;
    drop(guard);
    let recorded = events.0.lock().unwrap().clone();
    assert_eq!(
        recorded,
        vec!["trip_enrichment.reconciler_trip_coordinates_failed"]
    );
    let coordinate: Option<f64> =
        sqlx::query_scalar("SELECT location_lat FROM panda.charge_sessions WHERE id=$1")
            .bind(id(7001))
            .fetch_one(p)
            .await
            .unwrap();
    assert_eq!(coordinate, Some(30.));
    let mut rows = vec![snapshot(p, json!({"ordered_errors":recorded})).await];
    sqlx::query("ALTER TABLE panda.trips DROP CONSTRAINT synthetic_reject_coordinate")
        .execute(p)
        .await
        .unwrap();
    reconcile_locations(p, &Client::new()).await;
    rows.push(snapshot(p, json!({"restarted":true})).await);
    reconcile_locations(p, &Client::new()).await;
    rows.push(snapshot(p, json!({"replayed":true})).await);
    evidence("reconciliation", &json!(rows));
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires unique restricted JOBS_DATABASE_URL; all transports disabled"]
async fn jobs_reconciliation_worker_attempts_all_stages_and_cancels() {
    use tracing_subscriber::prelude::*;
    let f = Fixture::new().await;
    let p = &f.state.pool;
    let events = Events(Default::default());
    let guard =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(events.clone()));
    sqlx::query("REVOKE SELECT ON panda.trips,panda.charge_sessions FROM CURRENT_USER")
        .execute(p)
        .await
        .unwrap();
    let task = start_reconciliation_worker(p.clone());
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if events.0.lock().unwrap().len() >= 5 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    drop(guard);
    let messages = events.0.lock().unwrap().clone();
    assert_eq!(
        messages,
        vec![
            "trip_enrichment.reconciler_trip_coordinates_failed",
            "trip_enrichment.reconciler_trip_geofences_failed",
            "trip_enrichment.reconciler_charge_coordinates_failed",
            "trip_enrichment.reconciler_charge_locations_failed",
            "trip_enrichment.reconciler_failed"
        ]
    );
    sqlx::query("GRANT SELECT ON panda.trips,panda.charge_sessions TO CURRENT_USER")
        .execute(p)
        .await
        .unwrap();
    evidence(
        "reconciliation-errors",
        &snapshot(p, json!({"ordered_errors":messages})).await,
    );
    f.cleanup().await;
}
