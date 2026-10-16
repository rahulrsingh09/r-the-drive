use super::contract_support::*;
use serde_json::json;

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB and built maintenance binaries"]
async fn interruption_contracts() {
    let f = Fixture::new().await;
    f.seed().await;
    f.sql("INSERT INTO panda.trips(id,vehicle_id,started_at,ended_at,distance_miles,soc_start,soc_end,energy_wh,created_at) SELECT ('00000000-0000-0000-0000-'||lpad(to_hex(3000+n),12,'0'))::uuid,'00000000-0000-0000-0000-00000000000a','2025-09-01Z'::timestamptz+make_interval(mins=>n*20),'2025-09-01Z'::timestamptz+make_interval(mins=>n*20+10),10,80,70,123,'2025-01-01Z' FROM generate_series(0,2) n; CREATE FUNCTION panda.maintenance_energy_wait() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.id='00000000-0000-0000-0000-000000000bb9' THEN PERFORM pg_advisory_xact_lock(8675309); END IF; RETURN NEW; END $$; CREATE TRIGGER maintenance_energy_wait BEFORE UPDATE ON panda.trips FOR EACH ROW EXECUTE FUNCTION panda.maintenance_energy_wait()").await;
    let mut lock = f.pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(8675309)")
        .execute(&mut *lock)
        .await
        .unwrap();
    let run = f.process("recompute_trip_energy", &[]);
    let interrupt = async {
        let pid = tokio::time::timeout(std::time::Duration::from_secs(20), async {
            loop {
                let pid: Option<i32> = sqlx::query_scalar("SELECT activity.pid FROM pg_stat_activity activity JOIN pg_locks locks ON locks.pid=activity.pid WHERE activity.datname=current_database() AND locks.locktype='advisory' AND NOT locks.granted AND locks.objid=8675309 LIMIT 1").fetch_optional(&f.pool).await.unwrap();
                if let Some(pid) = pid { break pid; }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        }).await.unwrap();
        let state = f.state().await;
        let cancelled: bool = sqlx::query_scalar("SELECT pg_cancel_backend($1)")
            .bind(pid)
            .fetch_one(&f.pool)
            .await
            .unwrap();
        assert!(cancelled);
        state
    };
    let (process, blocked) = tokio::join!(run, interrupt);
    let cancelled = f.state().await;
    assert_eq!(
        blocked, cancelled,
        "cancelled statement must not commit its row"
    );
    lock.rollback().await.unwrap();
    f.sql("DROP TRIGGER maintenance_energy_wait ON panda.trips")
        .await;
    let retry = f.process("recompute_trip_energy", &[]).await;
    let completed = f.state().await;
    f.sql("UPDATE panda.trips SET energy_wh=123; CREATE FUNCTION panda.maintenance_energy_failure() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.id='00000000-0000-0000-0000-000000000bb9' THEN RAISE EXCEPTION 'synthetic late energy fault'; END IF; RETURN NEW; END $$; CREATE TRIGGER maintenance_energy_failure BEFORE UPDATE ON panda.trips FOR EACH ROW EXECUTE FUNCTION panda.maintenance_energy_failure()").await;
    let failure = f.process("recompute_trip_energy", &[]).await;
    let failed = f.state().await;
    f.sql("DROP TRIGGER maintenance_energy_failure ON panda.trips")
        .await;
    let final_retry = f.process("recompute_trip_energy", &[]).await;
    let final_state = f.state().await;
    let actual = json!({"process":process,"blocked":blocked,"cancelled":cancelled,"retry":retry,"completed":completed,"failure":failure,"failed":failed,"final_retry":final_retry,"final_state":final_state});
    f.cleanup().await;
    freeze(
        "interruption",
        actual,
        include_bytes!("interruption.json.gz"),
    );
}
