use super::*;
use serde_json::{json, Value};

pub async fn database() -> (PgPool, PgPool, String) {
    let fixture = super::payload_reconciliation_fixture::Fixture::new().await;
    (fixture.pool, fixture.admin, fixture.name)
}

pub async fn cleanup(pool: PgPool, admin: PgPool, name: String) {
    pool.close().await;
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DROP DATABASE \"{name}\" WITH (FORCE)"
    )))
    .execute(&admin)
    .await
    .unwrap();
    sqlx::query(sqlx::AssertSqlSafe(format!("DROP ROLE {name}_role")))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
}

pub fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

pub async fn vehicle(pool: &PgPool, n: u128) -> Uuid {
    let user = id(n + 10000);
    let vehicle = id(n);
    sqlx::query("INSERT INTO panda.users(id,email,password_hash) VALUES($1,$2,'synthetic')")
        .bind(user)
        .bind(format!("history-{n}@example.test"))
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO panda.vehicles(id,user_id,rivian_vehicle_id,model,name) VALUES($1,$2,$3,'R1T','Synthetic')")
        .bind(vehicle).bind(user).bind(format!("history-{n}")).execute(pool).await.unwrap();
    vehicle
}

pub async fn session(pool: &PgPool, session: Uuid) -> Value {
    sqlx::query_scalar("SELECT to_jsonb(s) FROM panda.charge_sessions s WHERE id=$1")
        .bind(session)
        .fetch_one(pool)
        .await
        .unwrap()
}

pub fn patch(mut value: Value, changes: Value) -> Value {
    value
        .as_object_mut()
        .unwrap()
        .extend(changes.as_object().unwrap().clone());
    value
}

pub async fn evidence(pool: &PgPool, name: &str, extra: Value) {
    let mut value: Value = sqlx::query_scalar(r#"SELECT jsonb_build_object(
        'sessions',COALESCE((SELECT jsonb_agg(to_jsonb(s) ORDER BY s.id) FROM panda.charge_sessions s),'[]'::jsonb),
        'aliases',COALESCE((SELECT jsonb_agg(to_jsonb(a) ORDER BY a.charge_session_id,a.external_id) FROM panda.charge_session_external_aliases a),'[]'::jsonb),
        'annotations',COALESCE((SELECT jsonb_agg(to_jsonb(a) ORDER BY a.charge_session_id,a.user_id) FROM panda.charge_session_user_annotations a),'[]'::jsonb),
        'payloads',COALESCE((SELECT jsonb_agg(to_jsonb(p) ORDER BY p.id) FROM panda.rivian_charge_payloads p),'[]'::jsonb),
        'identities',COALESCE((SELECT jsonb_agg(to_jsonb(i) ORDER BY i.identity_key) FROM panda.rivian_charge_payload_identities i),'[]'::jsonb),
        'identity_status',(SELECT to_jsonb(s) FROM panda.charge_payload_identity_backfill_status s WHERE job_key='charge_payload_identity'),
        'telemetry',COALESCE((SELECT jsonb_agg(to_jsonb(t) ORDER BY t.vehicle_id,t.ts) FROM timeseries.telemetry t),'[]'::jsonb),
        'curves',COALESCE((SELECT jsonb_agg(to_jsonb(p) ORDER BY p.vehicle_id,p.ts) FROM panda.rivian_charge_curve_points p),'[]'::jsonb),
        'parallax_curves',COALESCE((SELECT jsonb_agg(to_jsonb(p) ORDER BY p.vehicle_id,p.source_at,p.segment_index) FROM timeseries.parallax_charge_curve_points p),'[]'::jsonb),
        'repair_cursor',COALESCE((SELECT jsonb_agg(to_jsonb(c) ORDER BY c.vehicle_id) FROM panda.charge_session_repair_cursor c),'[]'::jsonb),
        'journals',COALESCE((SELECT jsonb_agg(to_jsonb(j) ORDER BY j.id) FROM panda.charge_session_repair_journal j),'[]'::jsonb))"#)
        .fetch_one(pool).await.unwrap();
    value["observations"] = extra;
    fn normalize(v: &mut Value) {
        match v {
            Value::Object(o) => {
                for (k, v) in o {
                    if matches!(
                        k.as_str(),
                        "received_at"
                            | "created_at"
                            | "updated_at"
                            | "first_seen_at"
                            | "last_seen_at"
                            | "heartbeat_at"
                            | "completed_at"
                            | "reverted_at"
                            | "reconciliation_checked_at"
                    ) && !v.is_null()
                    {
                        *v = json!("<clock>");
                    } else {
                        normalize(v);
                    }
                }
            }
            Value::Array(a) => {
                for v in a {
                    normalize(v);
                }
            }
            _ => {}
        }
    }
    // The status job start is a processing clock; session starts remain exact.
    if !value["identity_status"]["started_at"].is_null() {
        value["identity_status"]["started_at"] = json!("<clock>");
    }
    normalize(&mut value);
    if let Some(dir) = std::env::var("CHARGE_HISTORY_ORACLE_DIR")
        .ok()
        .filter(|v| !v.is_empty())
    {
        let expected: Value =
            serde_json::from_slice(&std::fs::read(format!("{dir}/{name}.json")).unwrap()).unwrap();
        assert_eq!(value, expected, "complete persisted output: {name}");
    }
    if let Some(dir) = std::env::var("CHARGE_HISTORY_EVIDENCE_DIR")
        .ok()
        .filter(|v| !v.is_empty())
    {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            format!("{dir}/{name}.json"),
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn charge_history_normalization_and_invalid_payload_contract() {
    for (raw, expected) in [
        (None, None),
        (Some(" , , "), None),
        (Some(" z,a,z, A "), Some("A,a,z")),
    ] {
        assert_eq!(normalize_grouping_key(raw).as_deref(), expected);
    }
    for (raw, expected) in [
        (" AC ", Some("ac")),
        ("ran15_dispenser", Some("dc")),
        ("unknown", None),
        ("", None),
    ] {
        assert_eq!(normalize_api_charger_type(Some(raw)), expected);
    }
    assert_eq!(
        infer_api_charger_type(Some("Tesla"), Some(true)),
        Some("ac")
    );
    assert_eq!(infer_is_rivian_network(Some("ChargePoint")), Some(true));
    assert_eq!(infer_is_rivian_network(Some("Other")), Some(false));
    assert_eq!(infer_is_rivian_network(Some(" ")), None);
    for raw in [
        json!([]),
        json!({"totalEnergyKwh":"wrong"}),
        json!({"startInstant":"bad"}),
        json!({"meta":{"dataSources":[3]}}),
    ] {
        assert!(serde_json::from_value::<ChargeSessionSummaryPayload>(raw).is_err());
    }
    let summary: ChargeSessionSummaryPayload = serde_json::from_value(json!({"transactionId":" x ","meta":{"transactionIdGroupingKey":" b,a,b ","dataSources":["CDRS:x","TXN:a","VTXN:x","bad","TXN: ","lower:y"]}})).unwrap();
    let aliases = parse_summary_aliases(&summary);
    assert_eq!(
        aliases
            .iter()
            .map(|a| (
                a.external_id.as_str(),
                a.alias_kind.as_str(),
                a.transaction_id_grouping_key.as_deref()
            ))
            .collect::<Vec<_>>(),
        vec![
            ("x", "cdrs", Some("a,b")),
            ("a", "txn", Some("a,b")),
            ("x", "vtxn", Some("a,b")),
            ("y", "unknown", Some("a,b"))
        ]
    );
    let now: DateTime<Utc> = "2026-10-16T10:00:00Z".parse().unwrap();
    assert!(!should_insert_unmatched_session(
        UnmatchedInsertPolicy::Always,
        None
    ));
    assert!(!should_insert_unmatched_session(
        UnmatchedInsertPolicy::Never,
        Some(now)
    ));
    assert!(should_insert_unmatched_session(
        UnmatchedInsertPolicy::RecentOnly {
            now,
            lookback_days: 1
        },
        Some(now - chrono::Duration::days(1))
    ));
    assert!(!should_insert_unmatched_session(
        UnmatchedInsertPolicy::RecentOnly {
            now,
            lookback_days: 1
        },
        Some(now - chrono::Duration::days(1) - chrono::Duration::seconds(1))
    ));
}

#[tokio::test]
#[ignore = "requires a disposable TimescaleDB DATABASE_URL"]
async fn charge_history_complete_summary_and_cluster_contract() {
    let (pool, admin, name) = database().await;
    let v = vehicle(&pool, 1).await;
    let other = vehicle(&pool, 2).await;
    let s = id(101);
    sqlx::query("INSERT INTO panda.charge_sessions(id,vehicle_id,started_at,ended_at,source,soc_start,soc_end,kwh_added,location_override_mode,is_home) VALUES($1,$2,'2026-08-01T10:00Z','2026-08-01T11:00Z','telemetry',20,60,25,'none',false)")
        .bind(s).bind(v).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO panda.charge_sessions(id,vehicle_id,started_at,ended_at,source,rivian_session_id) VALUES($1,$2,'2026-08-01T10:00Z','2026-08-01T11:00Z','telemetry','other-vehicle')")
        .bind(id(201)).bind(other).execute(&pool).await.unwrap();
    let untouched = session(&pool, id(201)).await;
    let before = session(&pool, s).await;
    let summary: ChargeSessionSummaryPayload = serde_json::from_value(json!({"transactionId":"provider-txn","startInstant":"2026-08-01T09:58:00Z","endInstant":"2026-08-01T11:04:00Z","chargerType":"DCFC","currencyCode":"USD","totalEnergyKwh":40.5,"rangeAddedKm":120.5,"city":"Test City","vehicleId":"provider-vehicle","vehicleName":"R1T","vendor":"Rivian","paidTotal":0.0,"isPublic":true,"isHomeCharger":true,"isRoamingNetwork":false,"meta":{"transactionIdGroupingKey":"z,a,z","dataSources":["VTXN:provider-txn","CDRS:provider-cdr"]}})).unwrap();
    assert_eq!(
        reconcile_completed_session_summary(&pool, v, &summary, UnmatchedInsertPolicy::Never, None)
            .await
            .unwrap(),
        Some(s)
    );
    let expected = patch(
        before,
        json!({"api_started_at":"2026-08-01T09:58:00+00:00","api_ended_at":"2026-08-01T11:04:00+00:00","network_vendor":"Rivian","range_added_km":120.5,"is_free_session":true,"is_rivian_network":true,"rivian_paid_total":0,"charger_type":"dc","source":"telemetry+rivian_api","data_confidence":"telemetry_enriched","rivian_charger_type":"DCFC","currency_code":"USD","rivian_city":"Test City","rivian_vehicle_id":"provider-vehicle","rivian_vehicle_name":"R1T","is_public":true,"rivian_meta":serde_json::to_value(&summary).unwrap(),"rivian_session_id":"provider-cdr"}),
    );
    assert_eq!(session(&pool, s).await, expected);
    for _ in 0..2 {
        assert_eq!(
            reconcile_completed_session_summary(
                &pool,
                v,
                &summary,
                UnmatchedInsertPolicy::Always,
                None
            )
            .await
            .unwrap(),
            Some(s)
        );
    }
    assert_eq!(session(&pool, s).await, expected);
    assert_eq!(session(&pool, id(201)).await, untouched);
    let no_time: ChargeSessionSummaryPayload =
        serde_json::from_value(json!({"transactionId":"provider-txn","totalEnergyKwh":900}))
            .unwrap();
    assert_eq!(
        reconcile_completed_session_summary(&pool, v, &no_time, UnmatchedInsertPolicy::Never, None)
            .await
            .unwrap(),
        Some(s)
    );
    assert_eq!(session(&pool, s).await, expected);
    let no_id: ChargeSessionSummaryPayload = serde_json::from_value(json!({})).unwrap();
    assert_eq!(
        reconcile_completed_session_summary(&pool, v, &no_id, UnmatchedInsertPolicy::Always, None)
            .await
            .unwrap(),
        None
    );
    evidence(&pool, "summary", json!({"matched":s})).await;

    // A separated API row joins through a shared alias; restart evidence joins two later rows.
    sqlx::query("INSERT INTO panda.charge_sessions(id,vehicle_id,started_at,ended_at,source,kwh_added,energy_added_wh) VALUES($1,$2,'2026-08-01T08:00Z','2026-08-01T08:30Z','rivian_api',50,50000),($3,$2,'2026-08-02T10:00Z',NULL,'telemetry',NULL,NULL),($4,$2,'2026-08-02T10:10Z','2026-08-02T11:00Z','telemetry',10,10000)")
        .bind(id(102)).bind(v).bind(id(103)).bind(id(104)).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO panda.charge_session_external_aliases(charge_session_id,external_id,alias_kind,transaction_id_grouping_key) VALUES($1,'provider-cdr','cdrs','a,z')")
        .bind(id(102)).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO timeseries.telemetry(ts,vehicle_id,charge_session_id,power_kw) VALUES('2026-08-02T10:09Z',$1,$2,10),('2026-08-02T10:10Z',$1,$3,11)")
        .bind(v).bind(id(103)).bind(id(104)).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO panda.charge_session_user_annotations(charge_session_id,user_id,cost_usd,currency_code) VALUES($1,$3,2,'USD'),($2,$3,9,'CAD'),($2,'00000000-0000-0000-0000-000000002712',7,'EUR')")
        .bind(s).bind(id(102)).bind(id(10001)).execute(&pool).await.unwrap();
    let stats = canonicalize_charge_sessions(&pool, Some(v)).await.unwrap();
    assert_eq!(stats.clusters_merged, 2);
    assert_eq!(stats.duplicate_rows_deleted, 2);
    assert_eq!(session(&pool, s).await["kwh_added"], json!(50));
    assert_eq!(session(&pool, s).await["energy_added_wh"], json!(50000));
    assert_eq!(
        session(&pool, s).await["started_at"],
        json!("2026-08-01T10:00:00+00:00")
    );
    assert_eq!(
        session(&pool, id(103)).await["ended_at"],
        json!("2026-08-02T11:00:00+00:00")
    );
    assert_eq!(session(&pool, id(201)).await, untouched);
    evidence(&pool, "canonical", serde_json::to_value(stats).unwrap()).await;
    let repeated = canonicalize_charge_sessions(&pool, Some(v)).await.unwrap();
    assert_eq!(repeated.clusters_merged, 0);
    evidence(
        &pool,
        "canonical-repeat",
        serde_json::to_value(repeated).unwrap(),
    )
    .await;
    cleanup(pool, admin, name).await;
}
