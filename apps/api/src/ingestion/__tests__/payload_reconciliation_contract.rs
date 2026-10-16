use super::*;
use crate::services::charge_payload_identity::reconcile_retained_payload_references;
use crate::services::charge_sessions::payload_reconciliation_fixture::*;
use serde_json::{json, Value};

fn output(result: Result<ChargePayloadRecordOutcome>) -> Value {
    match result {
        Ok(ChargePayloadRecordOutcome::Recorded {
            payload_ref,
            repaired,
        }) => json!({
            "payload_id":payload_ref.payload_id,"captured_at":payload_ref.captured_at,
            "session":payload_ref.charge_session_id,"unchanged":payload_ref.unchanged,"repaired":repaired
        }),
        Ok(ChargePayloadRecordOutcome::EvidenceExpired) => json!({"expired":true}),
        Err(error) => json!({"error":error.to_string()}),
    }
}

async fn record(pool: &PgPool, v: Uuid, tx: Option<&str>, payload: Value) -> Value {
    output(
        record_charge_payload_with_ref(pool, v, "history", tx, Some("synthetic-rv"), payload).await,
    )
}

async fn historical(
    pool: &PgPool,
    n: u128,
    v: Uuid,
    session: Option<Uuid>,
    transaction: &str,
    at: &str,
    payload: Value,
) {
    sqlx::query("INSERT INTO panda.rivian_charge_payloads(id,vehicle_id,charge_session_id,operation,rivian_transaction_id,rivian_vehicle_id,captured_at,payload) VALUES($1,$2,$3,'history',$4,'synthetic-rv',$5,$6)")
        .bind(id(n)).bind(v).bind(session).bind(transaction).bind(instant(at)).bind(payload).execute(pool).await.unwrap();
}

#[tokio::test]
#[ignore = "requires unique restricted synthetic PAYLOAD_DATABASE_URL"]
async fn payload_reconciliation_identity_survivor_and_retention() {
    let db = Fixture::new().await;
    let v = vehicle(&db.pool, 1).await;
    let foreign = vehicle(&db.pool, 2).await;
    let local = session(
        &db.pool,
        100,
        v,
        Some("telemetry"),
        "2026-01-01T10:00:00Z",
        None,
    )
    .await;
    let other = session(
        &db.pool,
        101,
        foreign,
        Some("telemetry"),
        "2026-01-01T10:00:00Z",
        None,
    )
    .await;
    let payload = json!({"energy":12.50,"status":null,"nested":{"b":2,"a":1},"samples":[]});
    for (n, sid, at) in [
        (200, None, "2026-01-01T10:00:00Z"),
        (201, Some(other), "2026-01-01T09:00:00Z"),
        (202, Some(local), "2026-01-01T11:00:00Z"),
        (203, Some(local), "2026-01-01T11:00:00Z"),
    ] {
        historical(&db.pool, n, v, sid, "shared", at, payload.clone()).await;
    }
    let first = record(&db.pool, v, Some("shared"), payload.clone()).await;
    assert_eq!(first["payload_id"], json!(id(202)));
    assert_eq!(first["session"], json!(local));
    let second = record(
        &db.pool,
        v,
        Some("shared"),
        json!({"samples":[],"nested":{"a":1,"b":2},"status":null,"energy":12.5}),
    )
    .await;
    assert_eq!(first, second);
    let mut states = vec![snapshot(&db.pool).await];
    sqlx::query("UPDATE panda.rivian_charge_payloads SET payload_fingerprint=NULL WHERE id=$1")
        .bind(id(202))
        .execute(&db.pool)
        .await
        .unwrap();
    let fingerprint_repair = record(&db.pool, v, Some("shared"), payload.clone()).await;
    assert_eq!(fingerprint_repair, first);
    states.push(snapshot(&db.pool).await);
    sqlx::query("INSERT INTO panda.charge_session_external_aliases(charge_session_id,external_id,alias_kind,latest_payload_id,latest_payload_captured_at) VALUES($1,'shared','cdrs',$2,'2026-01-01T11:00:00Z')")
        .bind(local).bind(id(202)).execute(&db.pool).await.unwrap();
    sqlx::query("DELETE FROM panda.rivian_charge_payloads WHERE id=$1")
        .bind(id(202))
        .execute(&db.pool)
        .await
        .unwrap();
    let repaired = record(&db.pool, v, Some("shared"), payload.clone()).await;
    assert_eq!(repaired["payload_id"], json!(id(203)));
    assert_eq!(repaired["repaired"], json!(true));
    states.push(snapshot(&db.pool).await);
    sqlx::query("UPDATE panda.rivian_charge_payloads SET payload='{\"wrong\":true}' WHERE id=$1")
        .bind(id(203))
        .execute(&db.pool)
        .await
        .unwrap();
    let replaced = record(&db.pool, v, Some("shared"), payload.clone()).await;
    assert_eq!(replaced["payload_id"], json!(id(201)));
    assert!(replaced["session"].is_null());
    states.push(snapshot(&db.pool).await);
    sqlx::query("DELETE FROM panda.rivian_charge_payloads WHERE id=ANY($1)")
        .bind(vec![id(200), id(201)])
        .execute(&db.pool)
        .await
        .unwrap();
    let expired = record(&db.pool, v, Some("shared"), payload.clone()).await;
    assert_eq!(expired, json!({"expired":true}));
    let expired_retry = record(&db.pool, v, Some("shared"), payload.clone()).await;
    assert_eq!(expired_retry, expired);
    states.push(snapshot(&db.pool).await);
    let retained = reconcile_retained_payload_references(&db.pool, 1)
        .await
        .unwrap();
    assert_eq!(retained, (0, 0));
    evidence(
        "payload-survivor",
        json!({"first":first,"second":second,"fingerprint_repair":fingerprint_repair,"repaired":repaired,"replaced":replaced,"expired":expired,"retention":retained,"states":states}),
        &[],
    );
    db.cleanup().await;
}

#[tokio::test]
#[ignore = "requires unique restricted synthetic PAYLOAD_DATABASE_URL"]
async fn payload_reconciliation_corruption_rollback_and_retry() {
    let db = Fixture::new().await;
    let v = vehicle(&db.pool, 1).await;
    let payload = json!({"energy":0,"empty":"","null":null});
    historical(
        &db.pool,
        200,
        v,
        None,
        "fault",
        "2026-01-01T10:00:00Z",
        payload.clone(),
    )
    .await;
    let first = record(&db.pool, v, Some("fault"), payload.clone()).await;
    sqlx::query("UPDATE panda.rivian_charge_payload_identities SET operation='wrong'")
        .execute(&db.pool)
        .await
        .unwrap();
    let corrupt_before = snapshot(&db.pool).await;
    let metadata_error = record(&db.pool, v, Some("fault"), payload.clone()).await;
    assert_eq!(
        metadata_error,
        json!({"error":"charge payload identity metadata is inconsistent"})
    );
    assert_eq!(corrupt_before, snapshot(&db.pool).await);
    sqlx::query("UPDATE panda.rivian_charge_payload_identities SET operation='history'")
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE panda.rivian_charge_payloads SET rivian_transaction_id='wrong' WHERE id=$1",
    )
    .bind(id(200))
    .execute(&db.pool)
    .await
    .unwrap();
    let canonical_error = record(&db.pool, v, Some("fault"), payload.clone()).await;
    assert_eq!(
        canonical_error,
        json!({"error":"charge payload canonical evidence is inconsistent"})
    );
    sqlx::query(
        "UPDATE panda.rivian_charge_payloads SET rivian_transaction_id='fault' WHERE id=$1",
    )
    .bind(id(200))
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::raw_sql("CREATE FUNCTION panda.fixture_identity_failure() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'synthetic identity fault' USING ERRCODE='P0001'; END $$; CREATE TRIGGER fixture_identity_failure BEFORE INSERT ON panda.rivian_charge_payload_identities FOR EACH ROW EXECUTE FUNCTION panda.fixture_identity_failure();")
        .execute(&db.pool).await.unwrap();
    let before = snapshot(&db.pool).await;
    let late_error = record(&db.pool, v, None, json!({"fresh":true})).await;
    assert_eq!(
        late_error,
        json!({"error":"error returned from database: synthetic identity fault at line 3909"})
    );
    assert_eq!(before, snapshot(&db.pool).await);
    sqlx::query("DROP TRIGGER fixture_identity_failure ON panda.rivian_charge_payload_identities")
        .execute(&db.pool)
        .await
        .unwrap();
    let retry = record(&db.pool, v, None, json!({"fresh":true})).await;
    let fresh = Uuid::parse_str(retry["payload_id"].as_str().unwrap()).unwrap();
    let replay = record(&db.pool, v, None, json!({"fresh":true})).await;
    assert_eq!(replay["payload_id"], retry["payload_id"]);
    assert_eq!(retry["unchanged"], json!(false));
    assert_eq!(replay["unchanged"], json!(true));
    evidence(
        "payload-faults",
        json!({"first":first,"metadata_error":metadata_error,"canonical_error":canonical_error,"late_error":late_error,"retry":retry,"replay":replay,"state":snapshot(&db.pool).await}),
        &[(fresh, "<fresh-payload>")],
    );
    db.cleanup().await;
}

#[tokio::test]
#[ignore = "requires unique restricted synthetic PAYLOAD_DATABASE_URL"]
async fn payload_reconciliation_concurrent_and_cancelled_transactions() {
    let db = Fixture::new().await;
    let v = vehicle(&db.pool, 1).await;
    let foreign = vehicle(&db.pool, 2).await;
    let mut outputs = vec![record(&db.pool, v, Some("race"), json!({"race":true})).await];
    outputs.extend(
        futures::future::join_all(
            (0..7).map(|_| record(&db.pool, v, Some("race"), json!({"race":true}))),
        )
        .await,
    );
    let payload_id = Uuid::parse_str(outputs[0]["payload_id"].as_str().unwrap()).unwrap();
    assert!(outputs.iter().all(|r| r["payload_id"] == json!(payload_id)));
    assert_eq!(
        outputs
            .iter()
            .filter(|r| r["unchanged"] == json!(false))
            .count(),
        1
    );
    let foreign_output = record(&db.pool, foreign, Some("race"), json!({"race":true})).await;
    let foreign_id = Uuid::parse_str(foreign_output["payload_id"].as_str().unwrap()).unwrap();
    assert_ne!(payload_id, foreign_id);
    let mut lock = db.pool.begin().await.unwrap();
    sqlx::query("SELECT identity_key FROM panda.rivian_charge_payload_identities WHERE vehicle_id=$1 FOR UPDATE")
        .bind(v).execute(&mut *lock).await.unwrap();
    let before = snapshot(&db.pool).await;
    let cancelled = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        record(&db.pool, v, Some("race"), json!({"race":true})),
    )
    .await;
    assert!(cancelled.is_err());
    lock.rollback().await.unwrap();
    let retry = record(&db.pool, v, Some("race"), json!({"race":true})).await;
    assert_eq!(before, snapshot(&db.pool).await);
    let reopened = crate::db::pool::application_pool_options()
        .connect_with((*db.pool.connect_options()).clone())
        .await
        .unwrap();
    assert_eq!(
        record(&reopened, v, Some("race"), json!({"race":true})).await,
        retry
    );
    assert_eq!(snapshot(&reopened).await, before);
    reopened.close().await;
    evidence(
        "payload-concurrency",
        json!({"outputs":outputs,"foreign":foreign_output,"cancelled":true,"retry":retry,"state":snapshot(&db.pool).await}),
        &[
            (payload_id, "<race-payload>"),
            (foreign_id, "<foreign-payload>"),
        ],
    );
    db.cleanup().await;
}

#[tokio::test]
#[ignore = "requires unique restricted synthetic PAYLOAD_DATABASE_URL"]
async fn payload_reconciliation_bounded_retention_and_alias_rollback() {
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
    for n in 200..203 {
        let transaction = format!("retention-{n}");
        historical(
            &db.pool,
            n,
            v,
            None,
            &transaction,
            "2026-01-01T10:00:00Z",
            json!({"n":n}),
        )
        .await;
        record(&db.pool, v, Some(&transaction), json!({"n":n})).await;
        sqlx::query("INSERT INTO panda.charge_session_external_aliases(charge_session_id,external_id,alias_kind,latest_payload_id,latest_payload_captured_at) VALUES($1,$2,'txn',$3,'2026-01-01T10:00:00Z')")
            .bind(sid).bind(&transaction).bind(id(n)).execute(&db.pool).await.unwrap();
    }
    historical(
        &db.pool,
        300,
        v,
        None,
        "retention-200",
        "2026-01-01T11:00:00Z",
        json!({"n":200}),
    )
    .await;
    sqlx::query("DELETE FROM panda.rivian_charge_payloads WHERE id=$1")
        .bind(id(200))
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::raw_sql("CREATE FUNCTION panda.fixture_alias_failure() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'synthetic alias fault' USING ERRCODE='P0001'; END $$; CREATE TRIGGER fixture_alias_failure BEFORE UPDATE ON panda.charge_session_external_aliases FOR EACH ROW EXECUTE FUNCTION panda.fixture_alias_failure();")
        .execute(&db.pool).await.unwrap();
    let before = snapshot(&db.pool).await;
    let error = record(&db.pool, v, Some("retention-200"), json!({"n":200})).await;
    assert_eq!(
        error,
        json!({"error":"error returned from database: synthetic alias fault at line 3909"})
    );
    assert_eq!(snapshot(&db.pool).await, before);
    let retention_error = reconcile_retained_payload_references(&db.pool, 1)
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(snapshot(&db.pool).await, before);
    sqlx::query("DROP TRIGGER fixture_alias_failure ON panda.charge_session_external_aliases")
        .execute(&db.pool)
        .await
        .unwrap();
    let retry = record(&db.pool, v, Some("retention-200"), json!({"n":200})).await;
    assert_eq!(retry["payload_id"], json!(id(300)));
    sqlx::query("DELETE FROM panda.rivian_charge_payloads WHERE id=ANY($1)")
        .bind(vec![id(201), id(202)])
        .execute(&db.pool)
        .await
        .unwrap();
    let zero = reconcile_retained_payload_references(&db.pool, 0)
        .await
        .unwrap();
    let first = reconcile_retained_payload_references(&db.pool, 1)
        .await
        .unwrap();
    let second = reconcile_retained_payload_references(&db.pool, 1)
        .await
        .unwrap();
    let done = reconcile_retained_payload_references(&db.pool, 1)
        .await
        .unwrap();
    assert_eq!(
        (zero, first, second, done),
        ((0, 0), (1, 1), (1, 1), (0, 0))
    );
    evidence(
        "payload-retention",
        json!({"error":error,"retention_error":retention_error,"retry":retry,"counts":[zero,first,second,done],"state":snapshot(&db.pool).await}),
        &[],
    );
    db.cleanup().await;
}
