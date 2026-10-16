use super::contract_support::*;
use super::*;
use serde_json::json;

#[tokio::test]
#[ignore = "requires explicit isolated synthetic TimescaleDB and built maintenance binaries"]
async fn rebuild_contracts() {
    let f = Fixture::new().await;
    f.seed().await;
    let client = Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(1))
        .build()
        .unwrap();
    let mut evidence = vec![];
    evidence.push(json!({"case":"scope","all":load_vehicle_scope(&f.pool,None).await.unwrap(),"foreign":load_vehicle_scope(&f.pool,Some(id(999))).await.unwrap()}));
    evidence.push(json!({"case":"protected state delete","error":backfill_state_periods_for_vehicle(&f.pool,id(10)).await.unwrap_err().to_string(),"state":f.state().await}));
    // Vehicle 12 has no membership owner, so location enrichment cannot make an outbound request.
    f.sql("INSERT INTO timeseries.telemetry(ts,vehicle_id,power_state,speed_mph,odometer_miles,battery_level,battery_capacity_wh,distance_to_empty_mi,latitude,longitude) SELECT '2025-07-01Z'::timestamptz+make_interval(secs=>seconds),'00000000-0000-0000-0000-00000000000c',power,speed,odo,soc,100000,200,lat,lon FROM (VALUES(0,'go',20,100,80,47.0,-122.0),(60,'go',30,101,79,47.01,-122.01),(120,'go',20,102,78,47.02,-122.02),(180,'ready',0,102,78,47.02,-122.02),(481,'ready',0,102,78,47.02,-122.02)) x(seconds,power,speed,odo,soc,lat,lon)").await;
    for run in 0..2 {
        let replayed = replay_trips_for_vehicle(&f.pool, id(12), &client)
            .await
            .unwrap();
        let stamped = backfill_trip_ids_for_vehicle(&f.pool, id(12))
            .await
            .unwrap();
        evidence.push(json!({"case":"replay","run":run,"replayed":replayed,"stamped":stamped,"state":f.state().await}));
    }
    f.sql("INSERT INTO timeseries.telemetry(ts,vehicle_id,charger_state,battery_level,battery_capacity_wh,latitude,longitude,power_kw) SELECT '2025-08-01Z'::timestamptz+make_interval(secs=>seconds),'00000000-0000-0000-0000-00000000000c','charging',soc,100000,lat,lon,power FROM (VALUES(0,20,0,0,10),(60,21,47,NULL,12),(300,30,48,-122,20),(7500,40,49,-123,50),(14701,40,50,-124,60),(15001,50,51,-125,70)) x(seconds,soc,lat,lon,power)").await;
    for run in 0..2 {
        let repaired = repair_charge_sessions_for_vehicle(&f.pool, id(12))
            .await
            .unwrap();
        evidence.push(
            json!({"case":"charge replay","run":run,"repaired":repaired,"state":f.state().await}),
        );
    }
    evidence.push(json!({"case":"CLI denial after reconstruction","process":f.process("rebuild_vehicle_history",&["--vehicle","00000000-0000-0000-0000-00000000000c"]).await,"state":f.state().await}));
    let actual = json!(evidence);
    f.cleanup().await;
    freeze("rebuild", actual, include_bytes!("rebuild.json.gz"));
}
