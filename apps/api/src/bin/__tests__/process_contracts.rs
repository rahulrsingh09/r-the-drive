#[path = "support.rs"]
mod support;
use serde_json::json;
use support::*;

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB and built maintenance binaries"]
async fn process_contracts() {
    let f = Fixture::new().await;
    f.seed().await;
    let mut evidence = vec![];
    for (bin, args) in [
        ("compact_charge_payloads", vec!["--help"]),
        ("compact_charge_payloads", vec!["--batch-size", "0"]),
        ("compact_charge_payloads", vec!["--batch-size", "50001"]),
        ("compact_charge_payloads", vec!["--batch-size", "x"]),
        ("compact_charge_payloads", vec!["--vehicle"]),
        ("compact_charge_payloads", vec!["--vehicle", "invalid"]),
        ("compact_charge_payloads", vec!["--unknown"]),
        ("rebuild_vehicle_history", vec!["--help"]),
        ("rebuild_vehicle_history", vec!["--vehicle"]),
        ("rebuild_vehicle_history", vec!["--vehicle", "invalid"]),
        ("rebuild_vehicle_history", vec!["--unknown"]),
        ("repair_range_outliers", vec!["--help"]),
        ("repair_range_outliers", vec!["--vehicle-id"]),
        ("repair_range_outliers", vec!["--from"]),
        ("repair_range_outliers", vec!["--to", "invalid"]),
        ("repair_range_outliers", vec!["--unknown"]),
    ] {
        evidence.push(json!({"binary":bin,"args":args,"process":f.process(bin,&args).await}));
    }
    for bin in [
        "repair_range_outliers",
        "recompute_trip_energy",
        "repair_charge_sessions_from_telemetry",
        "compact_charge_payloads",
    ] {
        evidence.push(json!({"case":"empty","binary":bin,"process":f.process(bin,&[]).await,"state":f.state().await}));
    }
    evidence.push(json!({"case":"history denial","process":f.process("rebuild_vehicle_history",&["--vehicle","00000000-0000-0000-0000-00000000000a"]).await,"state":f.state().await}));
    f.sql("INSERT INTO timeseries.telemetry(ts,vehicle_id,battery_level,battery_capacity_wh,distance_to_empty_mi) SELECT '2025-01-01Z'::timestamptz+make_interval(secs=>n), '00000000-0000-0000-0000-00000000000a', CASE WHEN n%9=0 THEN NULL WHEN n%9=1 THEN 0 ELSE 50 END, CASE WHEN n%9=2 THEN 10000 ELSE 100000 END, CASE WHEN n%9=3 THEN NULL WHEN n%9=4 THEN 100 WHEN n%9=5 THEN 250 WHEN n%9=6 THEN 500 WHEN n%9=7 THEN -1 ELSE 0 END FROM generate_series(0,510) n").await;
    f.sql("INSERT INTO timeseries.telemetry(ts,vehicle_id,battery_level,battery_capacity_wh,distance_to_empty_mi) VALUES('2025-01-01Z','00000000-0000-0000-0000-00000000000b',62,91600,208),('2025-01-02Z','00000000-0000-0000-0000-00000000000c',50,100000,250)").await;
    for args in [
        vec!["--dry-run"],
        vec![
            "--from",
            "2025-01-01T00:00:05Z",
            "--to",
            "2025-01-01T00:00:06Z",
            "--vehicle-id",
            "00000000-0000-0000-0000-00000000000a",
        ],
        vec![
            "--from",
            "2025-01-02T00:00:00Z",
            "--to",
            "2025-01-01T00:00:00Z",
        ],
        vec![],
        vec![],
    ] {
        evidence.push(json!({"case":"range","args":args,"process":f.process("repair_range_outliers",&args).await,"state":f.state().await}));
    }
    f.sql("INSERT INTO panda.trips(id,vehicle_id,started_at,ended_at,distance_miles,soc_start,soc_end,range_start_mi,range_end_mi,energy_wh,efficiency_wh_per_mile,created_at) SELECT ('00000000-0000-0000-0000-'||lpad(to_hex(1000+n),12,'0'))::uuid, CASE WHEN n%3=0 THEN '00000000-0000-0000-0000-00000000000b'::uuid ELSE '00000000-0000-0000-0000-00000000000a'::uuid END, '2025-02-01Z'::timestamptz+make_interval(mins=>n*20), '2025-02-01Z'::timestamptz+make_interval(mins=>n*20+10), CASE WHEN n%7=0 THEN NULL WHEN n%7=1 THEN 0 ELSE 10 END, CASE WHEN n%5=0 THEN NULL ELSE 80 END, CASE WHEN n%5=1 THEN 90 ELSE 70 END, 200, CASE WHEN n%5=2 THEN 200 ELSE 190 END, 123, CASE WHEN n%5=3 THEN NULL ELSE 400 END, '2025-01-01Z' FROM generate_series(0,510) n").await;
    for run in 0..2 {
        evidence.push(json!({"case":"energy","run":run,"process":f.process("recompute_trip_energy",&[]).await,"state":f.state().await}));
    }
    let actual = json!(evidence);
    f.cleanup().await;
    freeze("process", actual, include_bytes!("process.json.gz"));
}

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB and built maintenance binaries"]
async fn charge_and_compaction_contracts() {
    let f = Fixture::new().await;
    f.seed().await;
    let mut evidence = vec![];
    f.sql("INSERT INTO timeseries.telemetry(ts,vehicle_id,charger_state,charger_status,time_to_end_of_charge_min,battery_level,battery_capacity_wh,power_kw,latitude,longitude) SELECT '2025-03-01Z'::timestamptz+make_interval(secs=>seconds), ('00000000-0000-0000-0000-'||lpad(to_hex(vehicle),12,'0'))::uuid, CASE WHEN seconds%2=0 THEN 'charging' END, CASE WHEN seconds%2=1 THEN 'chrgr_sts_connected_charging' END, 5, soc,100000, power, lat,lon FROM (VALUES (10,0,30,12,0,0),(10,299,31,-12,1,2),(10,300,35,20,3,4),(10,7500,40,40,5,6),(10,14701,45,60,7,8),(10,15001,50,70,9,10),(11,0,50,5,11,12),(11,300,55,8,13,14),(12,0,30,12,15,16),(12,300,40,12,17,18)) x(vehicle,seconds,soc,power,lat,lon)").await;
    f.sql("INSERT INTO panda.charge_sessions(id,vehicle_id,started_at,soc_start,source,created_at) VALUES('00000000-0000-0000-0000-000000000064','00000000-0000-0000-0000-00000000000a','2025-03-01Z',30,'telemetry','2025-01-01Z'); INSERT INTO panda.active_charge_checkpoints(vehicle_id,active_session_id,snapshot,updated_at) VALUES('00000000-0000-0000-0000-00000000000a','00000000-0000-0000-0000-000000000064','{}','2025-01-01Z')").await;
    for run in 0..2 {
        evidence.push(json!({"case":"charge active pin","run":run,"process":f.process("repair_charge_sessions_from_telemetry",&[]).await,"state":f.state().await}));
    }
    f.sql("INSERT INTO panda.pending_charge_completions(session_id,vehicle_id,session,created_at) VALUES('00000000-0000-0000-0000-000000000064','00000000-0000-0000-0000-00000000000a','{}','2025-01-01Z'); UPDATE panda.active_charge_checkpoints SET active_session_id=NULL").await;
    evidence.push(json!({"case":"charge pending pin","process":f.process("repair_charge_sessions_from_telemetry",&[]).await,"state":f.state().await}));
    f.sql("DELETE FROM panda.pending_charge_completions").await;
    for run in 0..2 {
        evidence.push(json!({"case":"charge released pin","run":run,"process":f.process("repair_charge_sessions_from_telemetry",&[]).await,"state":f.state().await}));
    }
    f.sql("INSERT INTO panda.charge_sessions(id,vehicle_id,started_at,ended_at,source,soc_end,kwh_added,network_vendor,created_at) VALUES('00000000-0000-0000-0000-000000000065','00000000-0000-0000-0000-00000000000b','2025-04-01Z','2025-04-01T00:20:30Z','rivian_api',50,20,'TESLA','2025-01-01Z')").await;
    evidence.push(json!({"case":"API metadata","process":f.process("repair_charge_sessions_from_telemetry",&[]).await,"state":f.state().await}));
    // A failed later vehicle rolls back its entire reconstruction, while previous vehicles remain committed.
    f.sql("INSERT INTO timeseries.telemetry(ts,vehicle_id,charger_state,battery_level,battery_capacity_wh) SELECT '2025-05-01Z'::timestamptz+make_interval(secs=>seconds), ('00000000-0000-0000-0000-'||lpad(to_hex(vehicle),12,'0'))::uuid,'charging',soc,100000 FROM (VALUES(10,0,30),(10,300,40),(11,0,30),(11,300,40)) x(vehicle,seconds,soc); CREATE FUNCTION panda.maintenance_charge_fault() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.vehicle_id='00000000-0000-0000-0000-00000000000b' AND NEW.started_at='2025-05-01Z' THEN RAISE EXCEPTION 'synthetic late charge fault'; END IF; RETURN NEW; END $$; CREATE TRIGGER maintenance_charge_fault BEFORE INSERT ON panda.charge_sessions FOR EACH ROW EXECUTE FUNCTION panda.maintenance_charge_fault()").await;
    evidence.push(json!({"case":"charge late fault","process":f.process("repair_charge_sessions_from_telemetry",&[]).await,"state":f.state().await}));
    f.sql("DROP TRIGGER maintenance_charge_fault ON panda.charge_sessions")
        .await;
    evidence.push(json!({"case":"charge retry","process":f.process("repair_charge_sessions_from_telemetry",&[]).await,"state":f.state().await}));
    f.sql("INSERT INTO panda.rivian_charge_payloads(id,vehicle_id,charge_session_id,operation,rivian_transaction_id,captured_at,payload,payload_fingerprint) SELECT ('00000000-0000-0000-0000-'||lpad(to_hex(200+n),12,'0'))::uuid, CASE WHEN n=5 THEN '00000000-0000-0000-0000-00000000000b'::uuid ELSE '00000000-0000-0000-0000-00000000000a'::uuid END, CASE WHEN n IN (2,3) THEN '00000000-0000-0000-0000-000000000064'::uuid END, 'getLiveSession', CASE WHEN n=4 THEN 'distinct' END, '2025-06-01Z'::timestamptz+make_interval(secs=>n), jsonb_build_object('kwh',12,'updatedAt',n), CASE WHEN n=6 THEN NULL ELSE panda.charge_payload_fingerprint('{\"kwh\":12}') END FROM generate_series(0,6) n").await;
    evidence.push(json!({"case":"pending identity report","process":f.process("compact_charge_payloads",&[]).await,"state":f.state().await}));
    evidence.push(json!({"case":"pending identity denial","process":f.process("compact_charge_payloads",&["--apply"]).await,"state":f.state().await}));
    f.sql("UPDATE panda.rivian_charge_payloads SET payload_fingerprint=panda.charge_payload_fingerprint(payload) WHERE payload_fingerprint IS NULL; INSERT INTO panda.rivian_charge_payload_identities(identity_key,vehicle_id,operation,payload_fingerprint,canonical_payload_id,created_at) SELECT panda.charge_payload_identity_key(vehicle_id,operation,rivian_transaction_id,rivian_vehicle_id,payload_fingerprint),vehicle_id,operation,payload_fingerprint,id,'2025-01-01Z' FROM panda.rivian_charge_payloads WHERE id='00000000-0000-0000-0000-0000000000c8'; INSERT INTO panda.charge_session_external_aliases(charge_session_id,external_id,alias_kind,latest_payload_id,latest_payload_captured_at,first_seen_at,last_seen_at,created_at,updated_at) VALUES('00000000-0000-0000-0000-000000000064','synthetic','transaction','00000000-0000-0000-0000-0000000000c8','2025-06-01Z','2025-01-01Z','2025-01-01Z','2025-01-01Z','2025-01-01Z')").await;
    let mut lock = f.pool.begin().await.unwrap();
    sqlx::query(
        "SELECT pg_advisory_xact_lock(hashtextextended('panda-charge-payload-compaction',0))",
    )
    .execute(&mut *lock)
    .await
    .unwrap();
    evidence.push(json!({"case":"compaction lock denial","process":f.process("compact_charge_payloads",&["--apply"]).await,"state":f.state().await}));
    lock.rollback().await.unwrap();
    f.sql("CREATE FUNCTION panda.maintenance_payload_fault() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'synthetic payload delete fault'; END $$; CREATE TRIGGER maintenance_payload_fault BEFORE DELETE ON panda.rivian_charge_payloads FOR EACH ROW EXECUTE FUNCTION panda.maintenance_payload_fault()").await;
    evidence.push(json!({"case":"compaction rollback","process":f.process("compact_charge_payloads",&["--apply","--batch-size","1"]).await,"state":f.state().await}));
    f.sql("DROP TRIGGER maintenance_payload_fault ON panda.rivian_charge_payloads")
        .await;
    for run in 0..5 {
        evidence.push(json!({"case":"compaction bounded retry","run":run,"process":f.process("compact_charge_payloads",&["--apply","--batch-size","1","--vehicle","00000000-0000-0000-0000-00000000000a"]).await,"state":f.state().await}));
    }
    let actual = json!(evidence);
    f.cleanup().await;
    freeze(
        "charge-compaction",
        actual,
        include_bytes!("charge-compaction.json.gz"),
    );
}
