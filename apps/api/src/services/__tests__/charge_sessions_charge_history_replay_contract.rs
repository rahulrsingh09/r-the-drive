use super::charge_history_contract::{cleanup, database, evidence, id, vehicle};
use super::*;
use serde_json::json;

#[tokio::test]
#[ignore = "requires a disposable TimescaleDB DATABASE_URL"]
async fn charge_history_replay_insert_conflict_invalid_and_matching_precedence() {
    let (pool, admin, name) = database().await;
    let v = vehicle(&pool, 40).await;
    let other = vehicle(&pool, 41).await;
    sqlx::query("ALTER TABLE panda.charge_sessions ALTER COLUMN id SET DEFAULT '00000000-0000-0000-0000-000000000fa1'::uuid").execute(&pool).await.unwrap();
    for (n, payload) in [
        (
            4050,
            json!({"transactionId":"api-only","startInstant":"2026-08-04T10:00:00Z","endInstant":"2026-08-04T10:01:59.800Z","totalEnergyKwh":1.25,"rangeAddedKm":8.1,"vendor":"Other","paidTotal":2.5}),
        ),
        (
            4051,
            json!({"transactionId":"api-only","startInstant":"2026-08-04T10:00:00Z","endInstant":"2026-08-04T10:01:59.800Z","totalEnergyKwh":1.25,"rangeAddedKm":8.1,"vendor":"Other","paidTotal":2.5}),
        ),
        (
            4052,
            json!({"transactionId":"invalid","totalEnergyKwh":"wrong"}),
        ),
        (4053, json!({"transactionId":"no-time"})),
    ] {
        sqlx::query("INSERT INTO panda.rivian_charge_payloads(id,vehicle_id,operation,payload,captured_at) VALUES($1,$2,'getCompletedSessionSummaries',$3,'2026-08-04T12:00Z')")
            .bind(id(n)).bind(v).bind(payload).execute(&pool).await.unwrap();
    }
    assert_eq!(
        replay_charge_payload_summaries(&pool, Some(v), UnmatchedInsertPolicy::Never)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        replay_charge_payload_summaries(&pool, Some(v), UnmatchedInsertPolicy::Always)
            .await
            .unwrap(),
        2
    );
    let row = charge_history_contract::session(&pool, id(4001)).await;
    assert_eq!(row["duration_minutes"], json!(1));
    assert_eq!(row["source"], json!("rivian_api"));
    assert_eq!(row["is_rivian_network"], json!(false));
    assert_eq!(row["energy_added_wh"], json!(null));
    assert_eq!(row["kwh_added"], json!(1.25));
    assert_eq!(
        replay_charge_payload_summaries(&pool, Some(v), UnmatchedInsertPolicy::Always)
            .await
            .unwrap(),
        2
    );
    let conflict: ChargeSessionSummaryPayload = serde_json::from_value(
        json!({"transactionId":"api-only","startInstant":"2026-08-04T10:00:00Z"}),
    )
    .unwrap();
    assert_eq!(
        reconcile_completed_session_summary(
            &pool,
            other,
            &conflict,
            UnmatchedInsertPolicy::Always,
            None
        )
        .await
        .unwrap(),
        None
    );
    evidence(&pool, "replay", json!({"linked":2,"conflict":null})).await;

    sqlx::query("INSERT INTO panda.charge_sessions(id,vehicle_id,started_at,ended_at,source) VALUES($1,$3,'2026-08-05T10:00Z','2026-08-05T10:10Z','telemetry'),($2,$3,'2026-08-05T10:20Z','2026-08-05T10:30Z','telemetry')")
        .bind(id(4002)).bind(id(4003)).bind(v).execute(&pool).await.unwrap();
    let ambiguous: ChargeSessionSummaryPayload =
        serde_json::from_value(json!({"startInstant":"2026-08-05T10:15:00Z"})).unwrap();
    assert_eq!(
        reconcile_completed_session_summary(
            &pool,
            v,
            &ambiguous,
            UnmatchedInsertPolicy::Never,
            None
        )
        .await
        .unwrap(),
        None
    );
    sqlx::query("INSERT INTO panda.charge_session_external_aliases(charge_session_id,external_id,alias_kind,transaction_id_grouping_key) VALUES($1,'exact','txn','g'),($2,'other','txn','g')")
        .bind(id(4002)).bind(id(4003)).execute(&pool).await.unwrap();
    let by_alias: ChargeSessionSummaryPayload = serde_json::from_value(
        json!({"transactionId":"exact","startInstant":"2026-08-05T10:25:00Z"}),
    )
    .unwrap();
    assert_eq!(
        reconcile_completed_session_summary(
            &pool,
            v,
            &by_alias,
            UnmatchedInsertPolicy::Never,
            None
        )
        .await
        .unwrap(),
        Some(id(4002))
    );
    let by_group: ChargeSessionSummaryPayload = serde_json::from_value(
        json!({"startInstant":"2026-08-05T10:25:00Z","meta":{"transactionIdGroupingKey":"g"}}),
    )
    .unwrap();
    assert_eq!(
        reconcile_completed_session_summary(
            &pool,
            v,
            &by_group,
            UnmatchedInsertPolicy::Never,
            None
        )
        .await
        .unwrap(),
        Some(id(4002))
    );
    evidence(
        &pool,
        "matching",
        json!({"alias":id(4002),"group":id(4002),"ambiguous":null}),
    )
    .await;
    sqlx::query("INSERT INTO panda.rivian_charge_payloads(id,vehicle_id,operation,payload,captured_at) VALUES($1,$2,'getCompletedSessionSummaries','{\"startInstant\":\"bad-time\"}','2026-08-04T12:00Z')")
        .bind(id(4054)).bind(v).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO timeseries.telemetry(ts,vehicle_id) VALUES('2026-08-04T12:00Z',$1)")
        .bind(v)
        .execute(&pool)
        .await
        .unwrap();
    assert!(diagnose_charge_sessions(&pool, Some(v)).await.is_err());
    cleanup(pool, admin, name).await;
}
