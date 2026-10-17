use super::*;
use sqlx::Executor;
use std::time::Instant;

async fn configure_poll(f: &Fixture, mode: i32, ready_at: i64) {
    sqlx::query("UPDATE panda.followup_poll_config SET mode=$1,ready_at=$2")
        .bind(mode)
        .bind(ready_at)
        .execute(&f.state.pool)
        .await
        .unwrap();
    f.state
        .pool
        .execute(
            "SELECT setval('panda.followup_reads',1,false); DELETE FROM panda.followup_read_times",
        )
        .await
        .unwrap();
}

async fn read_count(f: &Fixture) -> i64 {
    sqlx::query_scalar(
        "SELECT CASE WHEN is_called THEN last_value ELSE 0 END FROM panda.followup_reads",
    )
    .fetch_one(&f.state.pool)
    .await
    .unwrap()
}

async fn poll_result(f: &Fixture, expected: bool, count: i64) -> Duration {
    let started = Instant::now();
    let result = tokio::time::timeout(
        Duration::from_secs(12),
        wait_for_vehicle_data(&f.state.pool, id(101)),
    )
    .await
    .expect("bounded readiness workflow");
    let elapsed = started.elapsed();
    assert_eq!(result, expected);
    assert_eq!(read_count(f).await, count);
    let minimum = Duration::from_millis(500 * (count - i64::from(expected)) as u64);
    assert!(elapsed >= minimum, "{elapsed:?} < {minimum:?}");
    assert!(elapsed < minimum + Duration::from_secs(3), "{elapsed:?}");
    elapsed
}

async fn cancel_poll(f: &Fixture, during_read: bool) {
    configure_poll(f, if during_read { 5 } else { 1 }, 99).await;
    let mut future = Box::pin(wait_for_vehicle_data(&f.state.pool, id(101)));
    tokio::select! {
        result = &mut future => panic!("poll returned before cancellation: {result}"),
        _ = async {
            tokio::time::timeout(Duration::from_secs(2), async {
                while read_count(f).await == 0 {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            }).await.unwrap();
            tokio::time::sleep(Duration::from_millis(40)).await;
        } => {}
    }
    drop(future);
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(
        read_count(f).await,
        1,
        "cancellation must not detach polling"
    );
}

#[tokio::test]
#[ignore = "requires unique disposable restricted-role DB and Redis"]
async fn followup_poll_database_timing_errors_and_cancellation() {
    let f = Fixture::new().await;
    f.seed().await;
    f.state.pool.execute(
        r#"ALTER TABLE panda.vehicle_runtime_state RENAME TO followup_original_runtime;
           CREATE SEQUENCE panda.followup_reads;
           CREATE TABLE panda.followup_read_times(at timestamptz NOT NULL);
           CREATE TABLE panda.followup_poll_config(mode int NOT NULL,ready_at bigint NOT NULL);
           INSERT INTO panda.followup_poll_config VALUES(0,99);
           CREATE FUNCTION panda.followup_runtime_read()
           RETURNS TABLE(vehicle_id uuid,worker_health text,auth_state text)
           LANGUAGE plpgsql VOLATILE AS $$
           DECLARE n bigint; setting record;
           BEGIN
             n := nextval('panda.followup_reads');
             SELECT * INTO setting FROM panda.followup_poll_config;
             INSERT INTO panda.followup_read_times VALUES(clock_timestamp());
             IF setting.mode=2 AND n<setting.ready_at THEN
               RAISE EXCEPTION 'synthetic readiness read failure';
             END IF;
             IF setting.mode=0 THEN RETURN; END IF;
             IF setting.mode=5 THEN PERFORM pg_sleep(1); END IF;
             vehicle_id := '00000000-0000-0000-0000-000000000065';
             worker_health := CASE WHEN setting.mode=4 THEN NULL WHEN n>=setting.ready_at THEN 'connected' ELSE 'starting' END;
             auth_state := CASE WHEN setting.mode=4 THEN NULL WHEN n>=setting.ready_at THEN 'authorized' ELSE 'unauthorized' END;
             RETURN NEXT;
           END $$;
           CREATE VIEW panda.vehicle_runtime_state AS SELECT * FROM panda.followup_runtime_read()"#,
    ).await.unwrap();

    configure_poll(&f, 3, 1).await;
    assert!(poll_result(&f, true, 1).await < Duration::from_millis(450));
    configure_poll(&f, 3, 3).await;
    poll_result(&f, true, 3).await;
    configure_poll(&f, 2, 3).await;
    poll_result(&f, true, 3).await;
    for mode in [0, 1, 2, 4] {
        configure_poll(&f, mode, 99).await;
        poll_result(&f, false, 10).await;
        if mode != 2 {
            let last_gap: f64 = sqlx::query_scalar(
                "SELECT extract(epoch FROM clock_timestamp()-max(at))::float8 FROM panda.followup_read_times",
            ).fetch_one(&f.state.pool).await.unwrap();
            assert!(last_gap >= 0.49, "last failed read still needs its sleep");
            let minimum_gap: f64 = sqlx::query_scalar(
                "SELECT min(extract(epoch FROM at-previous))::float8 FROM (SELECT at,lag(at) OVER(ORDER BY at) previous FROM panda.followup_read_times) r",
            ).fetch_one(&f.state.pool).await.unwrap();
            assert!(minimum_gap >= 0.49, "reads must not overlap or skip sleeps");
        }
    }
    cancel_poll(&f, false).await;
    cancel_poll(&f, true).await;
    f.cleanup().await;
}

async fn cache_row(f: &Fixture) -> Value {
    sqlx::query_scalar(
        "SELECT to_jsonb(s) FROM panda.vehicle_artwork_cache_state s WHERE vehicle_id=$1",
    )
    .bind(id(101))
    .fetch_one(&f.state.pool)
    .await
    .unwrap()
}

async fn refresh_and_check(
    f: &Fixture,
    error: Option<&str>,
    status: &str,
    total: i32,
    ready: i32,
) -> Value {
    refresh_vehicle_artwork_cache_state(&f.state.pool, &f.state.config, id(101), error).await;
    let row = cache_row(f).await;
    assert_eq!(row["status"], status);
    assert_eq!(row["asset_count"], total);
    assert_eq!(row["ready_asset_count"], ready);
    assert_eq!(
        row["last_error"],
        error
            .map(|_| json!("Rivian artwork repair failed; retry is scheduled."))
            .unwrap_or(Value::Null)
    );
    row
}

#[tokio::test]
#[ignore = "requires unique disposable restricted-role DB and Redis"]
async fn followup_artwork_projection_filesystem_and_database_faults() {
    let f = Fixture::new().await;
    f.seed().await;
    let empty = refresh_and_check(&f, None, "pending", 0, 0).await;
    assert!(empty["last_repair_success_at"].is_null());
    refresh_and_check(&f, Some(""), "failed", 0, 0).await;

    let root = std::path::Path::new(&f.state.config.vehicle_image_cache_dir);
    std::fs::create_dir_all(root).unwrap();
    std::fs::write(root.join("main.webp"), b"main").unwrap();
    std::fs::write(root.join("overlay.webp"), b"overlay").unwrap();
    let main = hex::encode(Sha256::digest(b"main"));
    let overlay = hex::encode(Sha256::digest(b"overlay"));
    let metadata = json!({
        "mirror_status":"ready","mirror_relpath":"main.webp","sha256":main,
        "overlay_mirrors":[{"mirror_status":"failed","mirror_relpath":"overlay.webp","sha256":overlay}]
    });
    for (placement, url, data) in [
        ("side", "https://synthetic.invalid/main", metadata),
        (
            "front",
            "/local/ignored.webp",
            json!({"overlay_mirrors":[{}]}),
        ),
        ("rear", "HTTPS://synthetic.invalid/ignored", json!({})),
    ] {
        sqlx::query("INSERT INTO panda.vehicle_images(vehicle_id,placement,size,url,metadata,overlays) VALUES($1,$2,'large',$3,$4,'[]')")
            .bind(id(101)).bind(placement).bind(url).bind(data)
            .execute(&f.state.pool).await.unwrap();
    }
    let ready = refresh_and_check(&f, Some("private diagnostic"), "ready", 2, 2).await;
    assert!(!ready["last_repair_success_at"].is_null());
    f.state.pool.execute(
        "UPDATE panda.vehicle_artwork_cache_state SET attempts=7,last_repair_attempt_at='2026-01-01Z',last_repair_success_at='2026-01-01Z'",
    ).await.unwrap();
    let previous = cache_row(&f).await;
    std::fs::write(root.join("main.webp"), b"corrupt").unwrap();
    let pending = refresh_and_check(&f, None, "pending", 2, 1).await;
    for key in [
        "attempts",
        "last_repair_attempt_at",
        "last_repair_success_at",
    ] {
        assert_eq!(pending[key], previous[key]);
    }
    std::fs::remove_file(root.join("overlay.webp")).unwrap();
    refresh_and_check(&f, Some("unredacted detail"), "failed", 2, 0).await;
    std::fs::create_dir(root.join("overlay.webp")).unwrap();
    refresh_and_check(&f, None, "pending", 2, 0).await;
    std::fs::write(root.join("main.webp"), b"main").unwrap();
    sqlx::query("UPDATE panda.vehicle_images SET metadata=jsonb_set(metadata,'{mirror_status}','\"pending\"') WHERE placement='side'")
        .execute(&f.state.pool).await.unwrap();
    refresh_and_check(&f, None, "pending", 2, 0).await;

    f.state
        .pool
        .execute("ALTER TABLE panda.vehicle_images RENAME TO followup_images_fault")
        .await
        .unwrap();
    let failed_read = refresh_and_check(&f, Some("read fault"), "failed", 0, 0).await;
    assert_eq!(
        failed_read["last_repair_success_at"],
        previous["last_repair_success_at"]
    );
    refresh_and_check(&f, None, "pending", 0, 0).await;
    f.state
        .pool
        .execute("ALTER TABLE panda.followup_images_fault RENAME TO vehicle_images")
        .await
        .unwrap();

    let before_failed_write = cache_row(&f).await;
    f.state
        .pool
        .execute("ALTER TABLE panda.vehicle_artwork_cache_state RENAME TO followup_cache_fault")
        .await
        .unwrap();
    refresh_vehicle_artwork_cache_state(
        &f.state.pool,
        &f.state.config,
        id(101),
        Some("write fault"),
    )
    .await;
    f.state
        .pool
        .execute("ALTER TABLE panda.followup_cache_fault RENAME TO vehicle_artwork_cache_state")
        .await
        .unwrap();
    assert_eq!(cache_row(&f).await, before_failed_write);
    f.cleanup().await;
}
