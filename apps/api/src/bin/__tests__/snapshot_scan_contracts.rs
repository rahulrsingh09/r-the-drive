use super::contract_support::{id, Fixture};
use super::SnapshotScan;
use anyhow::{ensure, Context, Result};
use chrono::{DateTime, Duration, Utc};
use sqlx::{postgres::PgRow, FromRow, PgPool};
use std::time::Duration as Wait;
use uuid::Uuid;

const SCALARS: &str = "SELECT id FROM panda.charge_sessions ORDER BY id";
const LOCATIONS: &str = "SELECT id, vehicle_id, location_lat, location_lng FROM panda.charge_sessions WHERE vehicle_id=$1 ORDER BY id";
const TELEMETRY: &str = "SELECT ts, vehicle_id, speed_mph, battery_level FROM timeseries.telemetry WHERE vehicle_id=$1 ORDER BY ts";

#[derive(Debug, PartialEq, FromRow)]
struct Location {
    id: Uuid,
    vehicle_id: Uuid,
    location_lat: Option<f64>,
    location_lng: Option<f64>,
}

#[derive(Debug, PartialEq, FromRow)]
struct Telemetry {
    ts: DateTime<Utc>,
    vehicle_id: Uuid,
    speed_mph: Option<f64>,
    battery_level: Option<f64>,
}

async fn one_connection(fixture: &Fixture) -> PgPool {
    panda_api::db::pool::application_pool_options()
        .max_connections(1)
        .min_connections(1)
        .connect(&fixture.url)
        .await
        .unwrap()
}

async fn pid(pool: &PgPool) -> Result<i32> {
    Ok(sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(pool)
        .await?)
}

async fn read_all<T>(pool: &PgPool, select: &'static str, vehicle: Option<Uuid>) -> Result<Vec<T>>
where
    T: for<'row> FromRow<'row, PgRow> + Send + Unpin,
{
    let mut scan = SnapshotScan::open(pool, select, vehicle).await?;
    let mut rows = Vec::new();
    let mut pages = Vec::new();
    loop {
        let page = scan.next::<T>().await?;
        pages.push(page.len());
        if page.is_empty() {
            break;
        }
        rows.extend(page);
    }
    scan.finish().await?;
    ensure!(
        pages == [256, 256, 1, 0],
        "cursor page bounds changed: {pages:?}"
    );
    Ok(rows)
}

async fn seed_pages(fixture: &Fixture) {
    fixture.seed().await;
    fixture.sql(
        "INSERT INTO panda.charge_sessions(id,vehicle_id,started_at,ended_at,location_lat,location_lng) \
         SELECT ('00000000-0000-0000-0000-'||lpad(to_hex(30000+n),12,'0'))::uuid, \
         '00000000-0000-0000-0000-00000000000a','2025-09-01Z'::timestamptz+make_interval(mins=>n), \
         '2025-09-01Z'::timestamptz+make_interval(mins=>n+1),47+n::float8/10000, \
         CASE WHEN n%2=0 THEN -122-n::float8/10000 END FROM generate_series(0,512) n; \
         INSERT INTO timeseries.telemetry(ts,vehicle_id,speed_mph,battery_level) \
         SELECT '2025-09-01Z'::timestamptz+make_interval(secs=>n), \
         '00000000-0000-0000-0000-00000000000a',n::float8/2,80-n::float8/100 \
         FROM generate_series(0,512) n",
    ).await;
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB"]
async fn cursor_shapes_reuse_one_restricted_connection() {
    let fixture = Fixture::new().await;
    seed_pages(&fixture).await;
    let before = fixture.state().await;
    let pool = one_connection(&fixture).await;
    let result: Result<()> = async {
        let backend = pid(&pool).await?;
        let privileges: (bool, bool, bool) = sqlx::query_as(
            "SELECT rolsuper,has_table_privilege(current_user,'timeseries.telemetry','DELETE'), \
             has_table_privilege(current_user,'timeseries.telemetry','TRUNCATE') \
             FROM pg_roles WHERE rolname=current_user",
        )
        .fetch_one(&pool)
        .await?;
        ensure!(
            privileges == (false, false, false),
            "fixture history privileges changed"
        );
        let expected_ids: Vec<_> = (0..513).map(|n| (id(30000 + n),)).collect();
        let expected_locations: Vec<_> = (0..513)
            .map(|n| Location {
                id: id(30000 + n),
                vehicle_id: id(10),
                location_lat: Some(47.0 + n as f64 / 10000.0),
                location_lng: (n % 2 == 0).then_some(-122.0 - n as f64 / 10000.0),
            })
            .collect();
        let epoch: DateTime<Utc> = "2025-09-01T00:00:00Z".parse()?;
        let expected_telemetry: Vec<_> = (0..513)
            .map(|n| Telemetry {
                ts: epoch + Duration::seconds(n),
                vehicle_id: id(10),
                speed_mph: Some(n as f64 / 2.0),
                battery_level: Some(80.0 - n as f64 / 100.0),
            })
            .collect();
        for round in 0..3 {
            ensure!(
                read_all::<(Uuid,)>(&pool, SCALARS, None).await? == expected_ids,
                "full scalar output differs in round {round}"
            );
            ensure!(
                pid(&pool).await? == backend,
                "scalar scan replaced the connection"
            );
            ensure!(
                read_all::<Location>(&pool, LOCATIONS, Some(id(10)))
                    .await
                    .with_context(|| format!("location scan round {round}"))?
                    == expected_locations,
                "full location output differs in round {round}"
            );
            ensure!(
                pid(&pool).await? == backend,
                "location scan replaced the connection"
            );
            ensure!(
                read_all::<Telemetry>(&pool, TELEMETRY, Some(id(10))).await? == expected_telemetry,
                "full telemetry output differs in round {round}"
            );
            ensure!(
                pid(&pool).await? == backend,
                "telemetry scan replaced the connection"
            );
        }
        ensure!(
            fixture.state().await == before,
            "read-only scans changed history"
        );
        println!("9 scans, 4617 complete rows, 36 bounded pages, one restricted backend {backend}");
        Ok(())
    }
    .await;
    pool.close().await;
    fixture.cleanup().await;
    result.unwrap();
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB"]
async fn cursor_drop_rolls_back_and_restarts_on_same_connection() {
    let fixture = Fixture::new().await;
    seed_pages(&fixture).await;
    let before = fixture.state().await;
    let pool = one_connection(&fixture).await;
    let result: Result<()> = async {
        let backend = pid(&pool).await?;
        let mut scan = SnapshotScan::open(&pool, SCALARS, None).await?;
        ensure!(
            scan.next::<(Uuid,)>().await? == (0..256).map(|n| (id(30000 + n),)).collect::<Vec<_>>(),
            "interrupted first page differs"
        );
        drop(scan);
        ensure!(pid(&pool).await? == backend, "rollback replaced connection");
        let cursors: i64 =
            sqlx::query_scalar("SELECT count(*) FROM pg_cursors WHERE name='maintenance_rows'")
                .fetch_one(&pool)
                .await?;
        ensure!(cursors == 0, "dropped cursor survived transaction rollback");
        ensure!(
            read_all::<(Uuid,)>(&pool, SCALARS, None).await?
                == (0..513).map(|n| (id(30000 + n),)).collect::<Vec<_>>(),
            "retry did not restart from first row"
        );
        ensure!(pid(&pool).await? == backend, "retry replaced connection");
        ensure!(
            fixture.state().await == before,
            "drop/retry changed history"
        );
        Ok(())
    }
    .await;
    pool.close().await;
    fixture.cleanup().await;
    result.unwrap();
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB"]
async fn cursor_cancel_rolls_back_without_losing_connection() {
    let fixture = Fixture::new().await;
    fixture.seed().await;
    let before = fixture.state().await;
    let pool = one_connection(&fixture).await;
    let result: Result<()> = async {
        let backend = pid(&pool).await?;
        let mut scan = SnapshotScan::open(&pool, "SELECT 7::int4 AS value FROM pg_sleep(30)", None).await?;
        let cancel = async {
            tokio::time::timeout(Wait::from_secs(10), async {
                loop {
                    let sleeping: bool = sqlx::query_scalar(
                        "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1 AND wait_event='PgSleep')",
                    ).bind(backend).fetch_one(&fixture.pool).await?;
                    if sleeping { break; }
                    tokio::time::sleep(Wait::from_millis(10)).await;
                }
                let cancelled: bool = sqlx::query_scalar("SELECT pg_cancel_backend($1)")
                    .bind(backend).fetch_one(&fixture.pool).await?;
                ensure!(cancelled, "restricted role could not cancel its own statement");
                Ok::<_, anyhow::Error>(())
            }).await??;
            Ok::<_, anyhow::Error>(())
        };
        let (fetched, cancelled) = tokio::join!(scan.next::<(i32,)>(), cancel);
        cancelled?;
        let error = fetched.expect_err("cancelled FETCH must fail");
        ensure!(error.downcast_ref::<sqlx::Error>().and_then(|e| e.as_database_error())
            .and_then(|e| e.code()).as_deref() == Some("57014"), "unexpected cancellation error: {error}");
        drop(scan);
        ensure!(pid(&pool).await? == backend, "cancellation discarded the physical connection");
        let mut retry = SnapshotScan::open(&pool, "SELECT 9::int4 AS value", None).await?;
        ensure!(retry.next::<(i32,)>().await? == [(9,)], "cursor restart after cancellation failed");
        ensure!(retry.next::<(i32,)>().await?.is_empty(), "cursor termination changed");
        retry.finish().await?;
        ensure!(pid(&pool).await? == backend, "retry replaced cancelled connection");
        ensure!(fixture.state().await == before, "cancellation changed history");
        Ok(())
    }.await;
    pool.close().await;
    fixture.cleanup().await;
    result.unwrap();
}
