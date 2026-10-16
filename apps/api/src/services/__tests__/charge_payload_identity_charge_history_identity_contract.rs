use super::*;
use crate::services::charge_sessions::charge_history_contract::{
    cleanup, database, evidence, id, vehicle,
};
use serde_json::json;

#[tokio::test]
#[ignore = "requires a disposable TimescaleDB DATABASE_URL"]
async fn charge_history_identity_batches_retention_and_atomic_failure() {
    let (pool, admin, name) = database().await;
    let v = vehicle(&pool, 10).await;
    let other = vehicle(&pool, 11).await;
    let payloads = [
        (
            id(1001),
            v,
            "op",
            Some("txn"),
            json!({"energy":1,"nested":{"updatedAt":"one"},"array":[1,null]}),
        ),
        (
            id(1002),
            v,
            "op",
            Some("txn"),
            json!({"nested":{"updatedAt":"two"},"array":[1,null],"energy":1.0}),
        ),
        (
            id(1003),
            v,
            "op",
            None,
            json!({"energy":1,"array":[1,null],"nested":{}}),
        ),
        (
            id(1004),
            other,
            "op",
            Some("txn"),
            json!({"energy":1,"array":[1,null],"nested":{}}),
        ),
        (
            id(1005),
            v,
            "other",
            Some("txn"),
            json!({"energy":1,"array":[1,null],"nested":{}}),
        ),
        (
            id(1006),
            v,
            "op",
            Some("txn"),
            json!({"energy":null,"array":[null,1],"nested":{}}),
        ),
        (
            id(1007),
            v,
            "op",
            Some("txn"),
            json!({"energy":1,"nested":{"updatedAt":"three"},"array":[1,null]}),
        ),
    ];
    for (i, (p, v, op, txn, payload)) in payloads.iter().enumerate() {
        sqlx::query("INSERT INTO panda.rivian_charge_payloads(id,vehicle_id,operation,rivian_transaction_id,captured_at,payload) VALUES($1,$2,$3,$4,'2026-08-01T10:00Z'::timestamptz+$5*interval '1 second',$6)")
            .bind(p).bind(v).bind(op).bind(txn).bind(i as f64).bind(payload).execute(&pool).await.unwrap();
    }
    // The durable status and the payload writes must roll back together.
    sqlx::query("ALTER TABLE panda.charge_payload_identity_backfill_status ADD CONSTRAINT fixture_reject_progress CHECK (rows_scanned=0)").execute(&pool).await.unwrap();
    assert!(process_batch(&pool, 2).await.is_err());
    assert_eq!(pending_count(&pool).await.unwrap(), 7);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM panda.rivian_charge_payload_identities")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    sqlx::query("ALTER TABLE panda.charge_payload_identity_backfill_status DROP CONSTRAINT fixture_reject_progress").execute(&pool).await.unwrap();
    let mut lock = pool.acquire().await.unwrap();
    sqlx::query("SELECT pg_advisory_lock(hashtextextended($1,0))")
        .bind(ADVISORY_LOCK_KEY)
        .execute(&mut *lock)
        .await
        .unwrap();
    assert_eq!(
        run_until_complete(&pool, BackfillConfig::default())
            .await
            .unwrap(),
        WorkerResult::Busy
    );
    sqlx::query("SELECT pg_advisory_unlock(hashtextextended($1,0))")
        .bind(ADVISORY_LOCK_KEY)
        .execute(&mut *lock)
        .await
        .unwrap();
    drop(lock);
    mark_started(&pool).await.unwrap();
    let first = process_batch(&pool, 1).await.unwrap();
    let rest = process_batch(&pool, 10).await.unwrap();
    assert_eq!(first, (1, 1));
    // PostgreSQL jsonb numeric text retains 1 versus 1.0 for fingerprints.
    assert_eq!(rest, (6, 5));
    assert_eq!(process_batch(&pool, 10).await.unwrap(), (0, 0));
    assert_eq!(
        run_until_complete(
            &pool,
            BackfillConfig {
                batch_size: 2,
                pause: Duration::ZERO
            }
        )
        .await
        .unwrap(),
        WorkerResult::Complete
    );
    sqlx::query("INSERT INTO panda.charge_sessions(id,vehicle_id,started_at) VALUES($1,$2,'2026-08-01T10:00Z')")
        .bind(id(1050)).bind(v).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO panda.charge_session_external_aliases(charge_session_id,external_id,alias_kind,latest_payload_id,latest_payload_captured_at) VALUES($1,'expired','txn',$2,'2026-08-01T10:00Z')")
        .bind(id(1050)).bind(id(1001)).execute(&pool).await.unwrap();
    evidence(&pool, "identity-filled", json!({"first":first,"rest":rest})).await;
    sqlx::query("DELETE FROM panda.rivian_charge_payloads WHERE id=$1")
        .bind(id(1001))
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        reconcile_retained_payload_references(&pool, 1)
            .await
            .unwrap(),
        (1, 1)
    );
    assert_eq!(
        reconcile_retained_payload_references(&pool, 1)
            .await
            .unwrap(),
        (0, 0)
    );
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM panda.rivian_charge_payload_identities WHERE canonical_payload_id IS NULL").fetch_one(&pool).await.unwrap(),1);
    evidence(
        &pool,
        "identity-retained",
        json!({"pending":pending_count(&pool).await.unwrap()}),
    )
    .await;
    cleanup(pool, admin, name).await;
}
