use super::payload_reconciliation_fixture::*;
use super::*;
use serde_json::json;

#[tokio::test]
#[ignore = "requires unique restricted synthetic PAYLOAD_DATABASE_URL"]
async fn summary_reconciliation_complete_precedence() {
    let db = Fixture::new().await;
    let vehicle_id = vehicle(&db.pool, 1).await;
    let mut observations = Vec::new();
    let summary: ChargeSessionSummaryPayload = serde_json::from_value(json!({
        "startInstant":"2026-01-01T10:00:00.250Z","endInstant":"2026-01-01T11:01:59.750Z",
        "chargerType":"DCFC","totalEnergyKwh":12.25,"rangeAddedKm":45.125,"vendor":"Rivian",
        "paidTotal":0.0,"isHomeCharger":true,"currencyCode":"USD","city":"Synthetic",
        "vehicleId":"fixture-rv","vehicleName":"Fixture","isPublic":false,
        "meta":{"transactionIdGroupingKey":"b,a","dataSources":["CDRS:fixture"]}
    }))
    .unwrap();
    for (i, source) in [
        None,
        Some("telemetry"),
        Some("rivian_api"),
        Some("telemetry+rivian_api"),
        Some("custom"),
    ]
    .into_iter()
    .enumerate()
    {
        let sid = session(
            &db.pool,
            100 + i as u128,
            vehicle_id,
            source,
            &format!("2026-01-01T10:{:02}:00Z", 10 + i),
            None,
        )
        .await;
        update_session_from_summary(&db.pool, sid, source, &summary, Some("dc"), Some(true))
            .await
            .unwrap();
        observations.push(snapshot(&db.pool).await);
        let before = snapshot(&db.pool).await;
        update_session_from_summary(
            &db.pool,
            sid,
            Some(summarize_source(source, true)),
            &summary,
            Some("dc"),
            Some(true),
        )
        .await
        .unwrap();
        assert_eq!(snapshot(&db.pool).await, before);
    }
    let sid = id(102);
    sqlx::query("UPDATE panda.charge_sessions SET location_override_mode='none',is_home=false,kwh_added=0,network_vendor='',range_added_km=0,is_free_session=false,is_rivian_network=false,rivian_paid_total=9,duration_minutes=0,charger_type='ac',rivian_charger_type='',currency_code='',rivian_city='',rivian_vehicle_id='',rivian_vehicle_name='',is_public=true,rivian_meta='{}' WHERE id=$1")
        .bind(sid).execute(&db.pool).await.unwrap();
    let wider: ChargeSessionSummaryPayload = serde_json::from_value(json!({
        "startInstant":"2026-01-01T09:00:00Z","endInstant":"2026-01-01T12:00:00Z","totalEnergyKwh":99,"vendor":"Tesla","isHomeCharger":true,"paidTotal":20
    })).unwrap();
    update_session_from_summary(
        &db.pool,
        sid,
        Some("rivian_api"),
        &wider,
        Some("dc"),
        Some(true),
    )
    .await
    .unwrap();
    observations.push(snapshot(&db.pool).await);
    let empty: ChargeSessionSummaryPayload = serde_json::from_value(json!({})).unwrap();
    update_session_from_summary(&db.pool, sid, None, &empty, None, None)
        .await
        .unwrap();
    observations.push(snapshot(&db.pool).await);
    for (i, end) in [
        "2026-01-01T10:00:59.499999Z",
        "2026-01-01T10:00:59.500000Z",
        "2026-01-01T09:59:00.500000Z",
        "2026-01-01T09:58:59.500000Z",
    ]
    .iter()
    .enumerate()
    {
        let sid = session(
            &db.pool,
            200 + i as u128,
            vehicle_id,
            Some("rivian_api"),
            &format!("2026-01-01T10:{:02}:00Z", 20 + i),
            None,
        )
        .await;
        let summary: ChargeSessionSummaryPayload=serde_json::from_value(json!({"startInstant":"2026-01-01T10:00:00Z","endInstant":end,"vendor":"Tesla","paidTotal":-1})).unwrap();
        update_session_from_summary(&db.pool, sid, Some("rivian_api"), &summary, None, None)
            .await
            .unwrap();
    }
    observations.push(snapshot(&db.pool).await);
    let before = snapshot(&db.pool).await;
    update_session_from_summary(&db.pool, id(99999), None, &summary, None, None)
        .await
        .unwrap();
    assert_eq!(before, snapshot(&db.pool).await);
    let error = sqlx::query("DELETE FROM timeseries.telemetry")
        .execute(&db.pool)
        .await
        .unwrap_err();
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some("42501")
    );
    evidence(
        "summary-precedence",
        json!({"states":observations,"protected_error":error.to_string()}),
        &[],
    );
    db.cleanup().await;
}

#[tokio::test]
#[ignore = "requires unique restricted synthetic PAYLOAD_DATABASE_URL"]
async fn summary_reconciliation_selection_and_late_failure() {
    let db = Fixture::new().await;
    let v = vehicle(&db.pool, 1).await;
    let foreign = vehicle(&db.pool, 2).await;
    session(
        &db.pool,
        100,
        v,
        Some("rivian_api"),
        "2026-01-01T10:00:00Z",
        Some("2026-01-01T10:20:00Z"),
    )
    .await;
    session(
        &db.pool,
        101,
        v,
        Some("telemetry"),
        "2026-01-01T10:10:00Z",
        Some("2026-01-01T10:30:00Z"),
    )
    .await;
    session(
        &db.pool,
        99,
        foreign,
        Some("telemetry+rivian_api"),
        "2026-01-01T10:00:00Z",
        Some("2026-01-01T10:30:00Z"),
    )
    .await;
    for n in [99, 100, 101] {
        sqlx::query("INSERT INTO panda.charge_session_external_aliases(charge_session_id,external_id,alias_kind,transaction_id_grouping_key) VALUES($1,'collision','cdrs','group')")
            .bind(id(n)).execute(&db.pool).await.unwrap();
    }
    let aliases = vec![ChargeSessionExternalAlias {
        external_id: "collision".into(),
        alias_kind: ExternalAliasKind::Cdrs,
        transaction_id_grouping_key: Some("group".into()),
    }];
    let alias = find_session_by_external_alias(&db.pool, v, &aliases)
        .await
        .unwrap()
        .unwrap();
    let group = find_session_by_grouping_key(
        &db.pool,
        v,
        Some("group"),
        Some(instant("2026-01-01T10:10:00Z")),
    )
    .await
    .unwrap()
    .unwrap();
    let overlap = find_overlapping_session(
        &db.pool,
        v,
        Some(instant("2026-01-01T10:00:00Z")),
        Some(instant("2026-01-01T10:30:00Z")),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        (alias.id, group.id, overlap.id),
        (id(100), id(100), id(101))
    );
    let mut selections = vec![json!({"alias":alias.id,"group":group.id,"overlap":overlap.id})];
    for start in [
        "2026-01-01T08:59:59.999999Z",
        "2026-01-01T09:00:00Z",
        "2026-01-01T09:55:00Z",
        "2026-01-01T10:10:00Z",
        "2026-01-01T11:10:00Z",
        "2026-01-01T11:10:00.000001Z",
    ] {
        selections.push(json!({"start":start,"nearby":find_time_window_session(&db.pool,v,Some(instant(start))).await.unwrap().map(|r|r.id)}));
    }
    let summary: ChargeSessionSummaryPayload=serde_json::from_value(json!({"transactionId":"collision","startInstant":"2026-01-01T10:00:00Z","totalEnergyKwh":12.5})).unwrap();
    sqlx::raw_sql("CREATE FUNCTION panda.fixture_alias_failure() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'synthetic alias fault' USING ERRCODE='P0001'; END $$; CREATE TRIGGER fixture_alias_failure BEFORE INSERT OR UPDATE ON panda.charge_session_external_aliases FOR EACH ROW EXECUTE FUNCTION panda.fixture_alias_failure();")
        .execute(&db.pool).await.unwrap();
    let error = reconcile_completed_session_summary(
        &db.pool,
        v,
        &summary,
        UnmatchedInsertPolicy::Never,
        None,
    )
    .await
    .unwrap_err();
    let after_failure = snapshot(&db.pool).await;
    assert_eq!(after_failure["sessions"][1]["kwh_added"], json!(12.5));
    sqlx::query("DROP TRIGGER fixture_alias_failure ON panda.charge_session_external_aliases")
        .execute(&db.pool)
        .await
        .unwrap();
    let retry = reconcile_completed_session_summary(
        &db.pool,
        v,
        &summary,
        UnmatchedInsertPolicy::Never,
        None,
    )
    .await
    .unwrap();
    assert_eq!(retry, Some(id(100)));
    evidence(
        "summary-selection",
        json!({"selections":selections,"error":error.to_string(),"after_failure":after_failure,"retry":retry,"final":snapshot(&db.pool).await}),
        &[],
    );
    db.cleanup().await;
}

#[test]
fn summary_reconciliation_malformed_wire_contract() {
    for payload in [
        json!([]),
        json!({"totalEnergyKwh":"12"}),
        json!({"startInstant":""}),
        json!({"meta":{"dataSources":[null]}}),
    ] {
        assert!(serde_json::from_value::<ChargeSessionSummaryPayload>(payload).is_err());
    }
    let empty: ChargeSessionSummaryPayload = serde_json::from_value(json!({})).unwrap();
    let explicit: ChargeSessionSummaryPayload =
        serde_json::from_value(json!({"totalEnergyKwh":null,"meta":null})).unwrap();
    assert_eq!(
        serde_json::to_value(empty).unwrap(),
        serde_json::to_value(explicit).unwrap()
    );
}

#[tokio::test]
#[ignore = "requires unique restricted synthetic PAYLOAD_DATABASE_URL"]
async fn summary_reconciliation_preserves_durable_pins() {
    let db = Fixture::new().await;
    let v = vehicle(&db.pool, 1).await;
    session(
        &db.pool,
        100,
        v,
        Some("telemetry"),
        "2026-01-01T10:00:00Z",
        Some("2026-01-01T11:00:00Z"),
    )
    .await;
    session(
        &db.pool,
        101,
        v,
        Some("rivian_api"),
        "2026-01-01T10:10:00Z",
        Some("2026-01-01T10:50:00Z"),
    )
    .await;
    sqlx::query("INSERT INTO panda.active_charge_checkpoints(vehicle_id,active_session_id,snapshot) VALUES($1,$2,'{\"fixture\":true}')")
        .bind(v).bind(id(100)).execute(&db.pool).await.unwrap();
    let pinned = canonicalize_charge_sessions(&db.pool, Some(v))
        .await
        .unwrap();
    assert_eq!(pinned.duplicate_rows_deleted, 0);
    let pinned_state = snapshot(&db.pool).await;
    sqlx::query(
        "UPDATE panda.active_charge_checkpoints SET active_session_id=NULL WHERE vehicle_id=$1",
    )
    .bind(v)
    .execute(&db.pool)
    .await
    .unwrap();
    let merged = canonicalize_charge_sessions(&db.pool, Some(v))
        .await
        .unwrap();
    assert_eq!(merged.duplicate_rows_deleted, 1);
    let repeated = canonicalize_charge_sessions(&db.pool, Some(v))
        .await
        .unwrap();
    assert_eq!(repeated.duplicate_rows_deleted, 0);
    evidence(
        "summary-pins",
        json!({"pinned":pinned,"pinned_state":pinned_state,"merged":merged,"repeated":repeated,"final":snapshot(&db.pool).await}),
        &[],
    );
    db.cleanup().await;
}

#[tokio::test]
#[ignore = "requires unique restricted synthetic PAYLOAD_DATABASE_URL"]
async fn summary_reconciliation_nearby_ambiguity_boundary() {
    let db = Fixture::new().await;
    let v = vehicle(&db.pool, 1).await;
    session(
        &db.pool,
        100,
        v,
        Some("telemetry"),
        "2026-01-01T10:00:00Z",
        None,
    )
    .await;
    session(
        &db.pool,
        101,
        v,
        Some("telemetry"),
        "2026-01-01T10:30:00Z",
        None,
    )
    .await;
    let before = snapshot(&db.pool).await;
    for (start, expected) in [
        ("2026-01-01T10:07:30Z", Some(id(100))),
        ("2026-01-01T10:07:30.000001Z", None),
        ("2026-01-01T10:15:00Z", None),
        ("2026-01-01T10:22:29.999999Z", None),
        ("2026-01-01T10:22:30Z", Some(id(101))),
    ] {
        let actual = find_time_window_session(&db.pool, v, Some(instant(start)))
            .await
            .unwrap()
            .map(|row| row.id);
        assert_eq!(actual, expected, "{start}");
    }
    assert_eq!(snapshot(&db.pool).await, before);
    db.cleanup().await;
}

#[tokio::test]
#[ignore = "requires unique restricted synthetic PAYLOAD_DATABASE_URL"]
async fn summary_reconciliation_concurrent_fields_are_not_lost() {
    let db = Fixture::new().await;
    let v = vehicle(&db.pool, 1).await;
    let sid = session(
        &db.pool,
        100,
        v,
        Some("telemetry"),
        "2026-01-01T10:00:00Z",
        None,
    )
    .await;
    sqlx::query("UPDATE panda.charge_sessions SET rivian_meta='{}' WHERE id=$1")
        .bind(sid)
        .execute(&db.pool)
        .await
        .unwrap();
    let mut expected = snapshot(&db.pool).await;
    expected["sessions"][0]["kwh_added"] = json!(12.5);
    expected["sessions"][0]["range_added_km"] = json!(44);
    expected["sessions"][0]["source"] = json!("telemetry+rivian_api");
    expected["sessions"][0]["data_confidence"] = json!("telemetry_enriched");
    let energy = serde_json::from_value(json!({"totalEnergyKwh":12.5})).unwrap();
    let range = serde_json::from_value(json!({"rangeAddedKm":44.0})).unwrap();
    let (left, right) = tokio::join!(
        update_session_from_summary(&db.pool, sid, Some("telemetry"), &energy, None, None),
        update_session_from_summary(&db.pool, sid, Some("telemetry"), &range, None, None),
    );
    left.unwrap();
    right.unwrap();
    assert_eq!(snapshot(&db.pool).await, expected);
    db.cleanup().await;
}
