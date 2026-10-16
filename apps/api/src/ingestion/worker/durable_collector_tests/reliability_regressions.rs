use super::*;

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB"]
async fn residual_fix_counter_failure_retains_retry_and_flush_day() {
    let db = CollectorFixture::new().await;
    let mut batch = CounterBatch::new(db.vehicle_id);
    batch.increment("ws_messages_received");
    db.pool.execute("ALTER TABLE panda.rivian_stewardship_counters ADD CONSTRAINT reject_residual_count CHECK(false) NOT VALID").await.unwrap();
    batch.flush(&db.pool).await;
    assert_eq!(batch.counts.get("ws_messages_received"), Some(&1));
    db.pool
        .execute(
            "ALTER TABLE panda.rivian_stewardship_counters DROP CONSTRAINT reject_residual_count",
        )
        .await
        .unwrap();
    batch.flush(&db.pool).await;
    batch.flush(&db.pool).await;
    assert!(batch.counts.is_empty());
    let count: i64 = sqlx::query_scalar(
        "SELECT sum(ws_messages_received)::bigint FROM panda.rivian_stewardship_counters",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(count, 1);
    db.finish().await;
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB and Redis"]
async fn residual_fix_software_insert_failure_rolls_back_and_retries_same_version() {
    let db = CollectorFixture::new().await;
    let mut harness = PipelineHarness::new(&db, true).await;
    let mut event = db.event(0);
    event.ota_current_version = Some("v1".into());
    harness.telemetry(event.clone(), "baseline").await;
    db.pool.execute("ALTER TABLE panda.software_versions ADD CONSTRAINT reject_residual_version CHECK(version<>'v2')").await.unwrap();
    event.ts += Duration::seconds(60);
    event.ota_current_version = Some("v2".into());
    harness.telemetry(event.clone(), "baseline").await;
    let closed: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT observed_until FROM panda.software_versions WHERE version='v1'")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(closed, None);
    db.pool
        .execute("ALTER TABLE panda.software_versions DROP CONSTRAINT reject_residual_version")
        .await
        .unwrap();
    harness.telemetry(event.clone(), "baseline").await;
    harness.telemetry(event.clone(), "baseline").await;
    drop(harness);
    let mut restarted = PipelineHarness::new(&db, true).await;
    restarted.telemetry(event, "baseline").await;
    let rows:Vec<(String,DateTime<Utc>,Option<DateTime<Utc>>)>=sqlx::query_as("SELECT version,installed_at,observed_until FROM panda.software_versions ORDER BY installed_at").fetch_all(&db.pool).await.unwrap();
    assert_eq!(
        rows,
        vec![
            ("v1".into(), db.at, Some(db.at + Duration::seconds(60))),
            ("v2".into(), db.at + Duration::seconds(60), None)
        ]
    );
    drop(restarted);
    db.finish().await;
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB"]
async fn residual_fix_power_cannot_overwrite_real_voltage() {
    let db = CollectorFixture::new().await;
    let id = Uuid::new_v4();
    ensure_active_charge_session(&db.pool, db.vehicle_id, id, &db.event(0))
        .await
        .unwrap();
    sqlx::query("UPDATE panda.charge_sessions SET peak_voltage=450 WHERE id=$1")
        .bind(id)
        .execute(&db.pool)
        .await
        .unwrap();
    let session = crate::ingestion::charge_detector::CompletedChargeSession {
        session_id: id,
        vehicle_id: db.vehicle_id,
        started_at: db.at,
        ended_at: db.at + Duration::seconds(120),
        location_lat: None,
        location_lng: None,
        soc_start: Some(40.0),
        soc_end: Some(42.0),
        charge_limit: None,
        battery_capacity_wh: None,
        energy_used_wh: None,
        energy_added_wh: None,
        peak_charge_kw: Some(150.0),
        avg_charge_rate_kw: None,
        charger_type: None,
    };
    let mut conn = db.pool.acquire().await.unwrap();
    persist_charge_session_record(&mut conn, &session, None, None, &MatchedLocation::none())
        .await
        .unwrap();
    let row: (Option<f64>, Option<f64>) = sqlx::query_as(
        "SELECT peak_voltage,max_charge_rate_kw FROM panda.charge_sessions WHERE id=$1",
    )
    .bind(id)
    .fetch_one(&mut *conn)
    .await
    .unwrap();
    assert_eq!(row, (Some(450.0), Some(150.0)));
    drop(conn);
    db.finish().await;
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB"]
async fn residual_fix_retention_releases_lock_and_continues_after_one_table_failure() {
    let db = CollectorFixture::new().await;
    seed_raw(&db).await;
    db.pool.execute("CREATE FUNCTION panda.reject_raw_delete() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'synthetic retention failure'; END $$; CREATE TRIGGER reject_raw_delete BEFORE DELETE ON panda.rivian_ws_raw_events FOR EACH ROW EXECUTE FUNCTION panda.reject_raw_delete()").await.unwrap();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .min_connections(2)
        .max_connections(2)
        .connect_with((*db.pool.connect_options()).clone())
        .await
        .unwrap();
    let first = pool.acquire().await.unwrap();
    let second = pool.acquire().await.unwrap();
    drop(first);
    drop(second);
    cleanup_raw_events(&pool, 0).await;
    let counts:(i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM panda.rivian_ws_raw_events),(SELECT count(*) FROM panda.rivian_parallax_events)").fetch_one(&db.pool).await.unwrap();
    assert_eq!(counts, (2, 1));
    let mut observer = db.pool.acquire().await.unwrap();
    let available: bool =
        sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(0x726d5241::bigint)")
            .fetch_one(&mut *observer)
            .await
            .unwrap();
    assert!(available, "retention lock must not outlive its transaction");
    drop(observer);
    pool.close().await;
    db.finish().await;
}

pub(super) async fn seed_raw(db: &CollectorFixture) {
    for days in [0, 2] {
        sqlx::query("INSERT INTO panda.rivian_ws_raw_events(vehicle_id,received_at,event_type,payload_text) VALUES($1,now()-($2*interval '1 day'),'telemetry','{}')").bind(db.vehicle_id).bind(days).execute(&db.pool).await.unwrap();
        sqlx::query("INSERT INTO panda.rivian_parallax_events(vehicle_id,received_at,rvm,payload_b64) VALUES($1,now()-($2*interval '1 day'),'fixture','e30=')").bind(db.vehicle_id).bind(days).execute(&db.pool).await.unwrap();
    }
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB"]
async fn residual_fix_retention_overflow_cannot_delete_fresh_rows() {
    let db = CollectorFixture::new().await;
    seed_raw(&db).await;
    cleanup_raw_events(&db.pool, i64::MAX).await;
    let counts:(i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM panda.rivian_ws_raw_events),(SELECT count(*) FROM panda.rivian_parallax_events)").fetch_one(&db.pool).await.unwrap();
    assert_eq!(counts, (2, 2));
    db.finish().await;
}
