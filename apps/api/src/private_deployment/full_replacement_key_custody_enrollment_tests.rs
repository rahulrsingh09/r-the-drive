use super::*;

#[tokio::test]
#[ignore = "requires disposable TimescaleDB DATABASE_URL and REDIS_URL"]
async fn full_replacement_key_custody_enrollment_roles_and_cancelled_locks() {
    let fixture = crate::authorization_test_support::Fixture::new().await;
    let owner = fixture.user("user").await;
    let manager = fixture.user("user").await;
    let viewer = fixture.user("user").await;
    let stranger = fixture.user("user").await;
    let vehicle = fixture.vehicle(owner, "synthetic-custody-enrollment").await;
    fixture.member(manager, vehicle, "manager").await;
    fixture.member(viewer, vehicle, "viewer").await;
    let pool = &fixture.state.pool;
    let before = fixture.snapshot().await;
    for (user, allowed) in [
        (owner, true),
        (manager, true),
        (viewer, false),
        (stranger, false),
    ] {
        let mut tx = pool.begin().await.unwrap();
        let result = lock_enrollment(&mut tx, user, "synthetic-custody-enrollment").await;
        if allowed {
            assert_eq!(result.unwrap(), Some(vehicle));
        } else {
            assert!(matches!(result, Err(AppError::Forbidden)));
        }
        tx.rollback().await.unwrap();
    }
    let mut tx = pool.begin().await.unwrap();
    assert!(matches!(
        lock_active_enrollment_user(&mut tx, Uuid::new_v4()).await,
        Err(AppError::Unauthorized)
    ));
    tx.rollback().await.unwrap();
    assert_eq!(before, fixture.snapshot().await);
    sqlx::query("UPDATE panda.users SET is_disabled=true WHERE id=$1")
        .bind(stranger)
        .execute(pool)
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    assert!(matches!(
        lock_enrollment(&mut tx, stranger, "new-synthetic-vehicle").await,
        Err(AppError::Forbidden)
    ));
    tx.rollback().await.unwrap();

    let mut holder = pool.begin().await.unwrap();
    assert_eq!(
        lock_enrollment(&mut holder, owner, "new-synthetic-vehicle")
            .await
            .unwrap(),
        None
    );
    let waiting_pool = pool.clone();
    let waiter = tokio::spawn(async move {
        let mut tx = waiting_pool.begin().await.unwrap();
        lock_enrollment(&mut tx, manager, "new-synthetic-vehicle").await
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(
        !waiter.is_finished(),
        "same remote ID must serialize even across users"
    );
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    holder.rollback().await.unwrap();
    let mut retry = pool.begin().await.unwrap();
    assert_eq!(
        tokio::time::timeout(
            std::time::Duration::from_secs(3),
            lock_enrollment(&mut retry, manager, "new-synthetic-vehicle")
        )
        .await
        .unwrap()
        .unwrap(),
        None
    );
    retry.rollback().await.unwrap();
    fixture.cleanup().await;
}
