use super::*;

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB"]
async fn residual_retention_cancellation_rolls_back_and_releases_lock() {
    let db = CollectorFixture::new().await;
    super::reliability_regressions::seed_raw(&db).await;
    let mut blocker = db.pool.acquire().await.unwrap();
    sqlx::query("SELECT pg_advisory_lock(81234)")
        .execute(&mut *blocker)
        .await
        .unwrap();
    db.pool.execute("CREATE FUNCTION panda.pause_raw_delete() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_advisory_xact_lock(81234); RETURN OLD; END $$; CREATE TRIGGER pause_raw_delete BEFORE DELETE ON panda.rivian_ws_raw_events FOR EACH ROW EXECUTE FUNCTION panda.pause_raw_delete()").await.unwrap();
    let pool = db.pool.clone();
    let task = tokio::spawn(async move {
        cleanup_raw_events(&pool, 1).await;
    });
    let mut waiting = false;
    for _ in 0..100 {
        waiting=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_locks WHERE locktype='advisory' AND objid=81234 AND NOT granted AND database=(SELECT oid FROM pg_database WHERE datname=current_database()))").fetch_one(&db.pool).await.unwrap();
        if waiting {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(waiting, "cleanup must reach the controlled delete boundary");
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    sqlx::query("SELECT pg_advisory_unlock(81234)")
        .execute(&mut *blocker)
        .await
        .unwrap();
    drop(blocker);
    let mut unlocked = false;
    for _ in 0..100 {
        unlocked = sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(0x726d5241::bigint)")
            .fetch_one(&db.pool)
            .await
            .unwrap();
        if unlocked {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(unlocked);
    let counts:(i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM panda.rivian_ws_raw_events),(SELECT count(*) FROM panda.rivian_parallax_events)").fetch_one(&db.pool).await.unwrap();
    assert_eq!(counts, (2, 2));
    db.finish().await;
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB"]
async fn residual_counter_batches_use_database_flush_day_across_midnight() {
    let db = CollectorFixture::new().await;
    let mut days = Vec::new();
    for timezone in ["Etc/GMT+12", "Etc/GMT-14"] {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .after_connect(move |conn, _| {
                Box::pin(async move {
                    sqlx::query("SELECT set_config('TimeZone',$1,false)")
                        .bind(timezone)
                        .execute(conn)
                        .await?;
                    Ok(())
                })
            })
            .connect_with((*db.pool.connect_options()).clone())
            .await
            .unwrap();
        let day: chrono::NaiveDate = sqlx::query_scalar("SELECT CURRENT_DATE")
            .fetch_one(&pool)
            .await
            .unwrap();
        days.push(day);
        let mut batch = CounterBatch::new(db.vehicle_id);
        batch.increment("ws_messages_received");
        batch.flush(&pool).await;
        pool.close().await;
    }
    assert_ne!(days[0], days[1]);
    let rows: Vec<(chrono::NaiveDate, i64)> = sqlx::query_as(
        "SELECT day,ws_messages_received FROM panda.rivian_stewardship_counters ORDER BY day",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(rows, vec![(days[0], 1), (days[1], 1)]);
    db.finish().await;
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB"]
async fn residual_software_close_and_read_failures_keep_retry_state_and_late_policy() {
    let db = CollectorFixture::new().await;
    let mut history = software_history::SoftwareHistory::default();
    history.observe(&db.pool, db.vehicle_id, "v1", db.at).await;
    db.pool.execute("ALTER TABLE panda.software_versions ADD CONSTRAINT reject_version_close CHECK(observed_until IS NULL)").await.unwrap();
    history
        .observe(&db.pool, db.vehicle_id, "v2", db.at + Duration::seconds(60))
        .await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM panda.software_versions")
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        1
    );
    db.pool.execute("ALTER TABLE panda.software_versions DROP CONSTRAINT reject_version_close; ALTER TABLE panda.software_versions RENAME TO residual_unavailable_versions").await.unwrap();
    history
        .observe(&db.pool, db.vehicle_id, "v2", db.at + Duration::seconds(60))
        .await;
    db.pool
        .execute("ALTER TABLE panda.residual_unavailable_versions RENAME TO software_versions")
        .await
        .unwrap();
    history
        .observe(&db.pool, db.vehicle_id, "v2", db.at + Duration::seconds(60))
        .await;
    history
        .observe(&db.pool, db.vehicle_id, "v3", db.at + Duration::seconds(30))
        .await;
    let rows: Vec<(String, DateTime<Utc>, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT version,installed_at,observed_until FROM panda.software_versions ORDER BY id",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        rows,
        vec![
            ("v1".into(), db.at, Some(db.at + Duration::seconds(60))),
            (
                "v2".into(),
                db.at + Duration::seconds(60),
                Some(db.at + Duration::seconds(30))
            ),
            ("v3".into(), db.at + Duration::seconds(30), None)
        ]
    );
    db.finish().await;
}

fn completion(db: &CollectorFixture) -> crate::ingestion::charge_detector::CompletedChargeSession {
    crate::ingestion::charge_detector::CompletedChargeSession {
        session_id: Uuid::new_v4(),
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
    }
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB"]
async fn residual_optional_annotation_failure_preserves_completed_record_and_retries() {
    let db = CollectorFixture::new().await;
    sqlx::query("INSERT INTO panda.vehicle_memberships(vehicle_id,user_id,role) SELECT id,user_id,'owner' FROM panda.vehicles WHERE id=$1")
        .bind(db.vehicle_id).execute(&db.pool).await.unwrap();
    assert!(get_vehicle_owner_id(&db.pool, db.vehicle_id)
        .await
        .unwrap()
        .is_some());
    let session = completion(&db);
    ensure_active_charge_session(&db.pool, db.vehicle_id, session.session_id, &db.event(0))
        .await
        .unwrap();
    db.pool.execute("ALTER TABLE panda.charge_session_user_annotations ADD CONSTRAINT reject_residual_annotation CHECK(false) NOT VALID").await.unwrap();
    let error = persist_charge_session(&db.pool, &session)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("reject_residual_annotation"));
    let end: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT ended_at FROM panda.charge_sessions WHERE id=$1")
            .bind(session.session_id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(end, Some(session.ended_at));
    db.pool.execute("ALTER TABLE panda.charge_session_user_annotations DROP CONSTRAINT reject_residual_annotation").await.unwrap();
    persist_charge_session(&db.pool, &session).await.unwrap();
    persist_charge_session(&db.pool, &session).await.unwrap();
    let annotations: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM panda.charge_session_user_annotations WHERE charge_session_id=$1",
    )
    .bind(session.session_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(annotations, 1);
    db.finish().await;
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB"]
async fn residual_charge_coordinate_precedence_and_partial_pairs() {
    let db = CollectorFixture::new().await;
    let mut session = completion(&db);
    let mut event = db.event(0);
    event.latitude = Some(30.0);
    event.longitude = Some(-97.0);
    write_telemetry(&db.pool, &event, None, None).await.unwrap();
    ensure_active_charge_session(&db.pool, db.vehicle_id, session.session_id, &db.event(0))
        .await
        .unwrap();
    assert_eq!(
        charge_session_coordinates(&db.pool, &session).await,
        (Some(30.0), Some(-97.0))
    );
    sqlx::query("UPDATE panda.charge_sessions SET location_lat=31,location_lng=-98 WHERE id=$1")
        .bind(session.session_id)
        .execute(&db.pool)
        .await
        .unwrap();
    session.location_lat = Some(32.0);
    assert_eq!(
        charge_session_coordinates(&db.pool, &session).await,
        (Some(31.0), Some(-98.0))
    );
    session.location_lng = Some(-99.0);
    assert_eq!(
        charge_session_coordinates(&db.pool, &session).await,
        (Some(32.0), Some(-99.0))
    );
    db.finish().await;
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB"]
async fn residual_restricted_role_history_denial_and_independent_raw_retention() {
    let base = std::env::var("TRIP_DURABILITY_TEST_DATABASE_URL").unwrap();
    let admin = PgPool::connect(&base).await.unwrap();
    let name = format!("panda_worker_residual_role_{}", Uuid::new_v4().simple());
    admin.execute(sqlx::AssertSqlSafe(format!("CREATE ROLE {name} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOREPLICATION NOBYPASSRLS PASSWORD 'synthetic-worker-role-only'"))).await.unwrap();
    admin
        .execute(sqlx::AssertSqlSafe(format!(
            "CREATE DATABASE {name} OWNER {name}"
        )))
        .await
        .unwrap();
    let mut admin_url = url::Url::parse(&base).unwrap();
    admin_url.set_path(&format!("/{name}"));
    let bootstrap = PgPool::connect(admin_url.as_str()).await.unwrap();
    bootstrap.execute(sqlx::AssertSqlSafe(format!("CREATE SCHEMA panda AUTHORIZATION {name}; CREATE SCHEMA timeseries AUTHORIZATION {name}; REVOKE CREATE ON SCHEMA public FROM PUBLIC; REVOKE ALL ON SCHEMA panda,timeseries FROM PUBLIC; CREATE EXTENSION IF NOT EXISTS pgcrypto WITH SCHEMA public; CREATE EXTENSION IF NOT EXISTS timescaledb WITH SCHEMA public; CREATE EXTENSION IF NOT EXISTS cube WITH SCHEMA panda; CREATE EXTENSION IF NOT EXISTS earthdistance WITH SCHEMA panda"))).await.unwrap();
    let mut app_url = admin_url.clone();
    app_url.set_username(&name).unwrap();
    app_url
        .set_password(Some("synthetic-worker-role-only"))
        .unwrap();
    let migration = PgPool::connect(app_url.as_str()).await.unwrap();
    crate::db::migrations::run_current_migrations(&migration)
        .await
        .unwrap();
    migration.close().await;
    let pool = crate::db::pool::create_pool(app_url.as_str())
        .await
        .unwrap();
    let config: Config = serde_json::from_value(
        serde_json::json!({"database_url":"unused","redis_url":"unused","panda_env":"production"}),
    )
    .unwrap();
    crate::private_deployment::history::protect_history(&pool, &config)
        .await
        .unwrap();
    let flags: (bool, bool, bool) = sqlx::query_as(
        "SELECT rolsuper,rolcreatedb,rolbypassrls FROM pg_roles WHERE rolname=current_user",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(flags, (false, false, false));
    let user = Uuid::new_v4();
    let vehicle = Uuid::new_v4();
    sqlx::query("INSERT INTO panda.users(id,email,password_hash) VALUES($1,'restricted@example.invalid','unused')").bind(user).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO panda.vehicles(id,user_id,rivian_vehicle_id,model) VALUES($1,$2,'synthetic-restricted-charge','R1T')").bind(vehicle).bind(user).execute(&pool).await.unwrap();
    let at = sqlx::query_scalar("SELECT date_trunc('milliseconds',now())-interval '10 minutes'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let db = CollectorFixture {
        pool,
        admin,
        name: name.clone(),
        vehicle_id: vehicle,
        at,
    };

    super::reliability_regressions::seed_raw(&db).await;
    db.pool
        .execute("REVOKE DELETE ON panda.rivian_ws_raw_events FROM CURRENT_USER")
        .await
        .unwrap();
    let mut history = software_history::SoftwareHistory::default();
    history
        .observe(&db.pool, db.vehicle_id, "restricted-v1", db.at)
        .await;
    history
        .observe(
            &db.pool,
            db.vehicle_id,
            "restricted-v2",
            db.at + Duration::seconds(60),
        )
        .await;
    let mut counters = CounterBatch::new(db.vehicle_id);
    counters.increment("raw_events_persisted");
    counters.flush(&db.pool).await;
    cleanup_raw_events(&db.pool, 1).await;
    let counts:(i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM panda.rivian_ws_raw_events),(SELECT count(*) FROM panda.rivian_parallax_events)").fetch_one(&db.pool).await.unwrap();
    assert_eq!(counts, (2, 1));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM panda.software_versions")
            .fetch_one(&db.pool)
            .await
            .unwrap(),
        2
    );
    for statement in [
        "DELETE FROM panda.rivian_ws_raw_events",
        "DELETE FROM panda.software_versions",
        "TRUNCATE panda.software_versions",
        "DELETE FROM timeseries.telemetry",
        "DELETE FROM panda.trips",
    ] {
        let error = db.pool.execute(statement).await.unwrap_err();
        assert_eq!(
            error.as_database_error().unwrap().code().as_deref(),
            Some("42501")
        );
    }
    bootstrap.close().await;
    db.finish().await;
    let cleanup = PgPool::connect(&base).await.unwrap();
    cleanup
        .execute(sqlx::AssertSqlSafe(format!("DROP ROLE {name}")))
        .await
        .unwrap();
    cleanup.close().await;
}
