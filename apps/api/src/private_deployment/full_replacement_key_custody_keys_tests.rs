use super::*;

#[test]
#[cfg(unix)]
fn full_replacement_key_custody_bundle_rejects_symlink_and_partial_reads() {
    let temp = tempfile::tempdir().unwrap();
    let keys = generate_keys().unwrap();
    let target = temp.path().join("bundle");
    let alias = temp.path().join("alias");
    std::os::unix::fs::symlink(&target, &alias).unwrap();
    assert!(write_key_bundle(&alias, &keys).is_err());
    assert!(!target.exists());
    write_key_bundle(&target, &keys).unwrap();
    std::fs::remove_file(target.join("age_key.txt")).unwrap();
    assert!(read_key_bundle(&target).is_err());
    assert!(write_key_bundle(&target, &keys).is_err());
    assert!(!target.join("age_key.txt").exists());
}

#[tokio::test]
#[ignore = "requires disposable KEY_CUSTODY_TEST_DATABASE_URL"]
async fn full_replacement_key_custody_cancelled_bootstrap_releases_transaction() {
    let admin_url = std::env::var("KEY_CUSTODY_TEST_DATABASE_URL").unwrap();
    let admin = PgPool::connect(&admin_url).await.unwrap();
    let database = format!("custody_cancel_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {database}")))
        .execute(&admin)
        .await
        .unwrap();
    let mut url = url::Url::parse(&admin_url).unwrap();
    url.set_path(&database);
    let pool = PgPool::connect(url.as_str()).await.unwrap();
    sqlx::raw_sql("CREATE SCHEMA panda;
        CREATE TABLE panda.system_config (key text PRIMARY KEY, value text NOT NULL);
        CREATE TABLE panda.vehicle_credentials (encrypted_tokens bytea NOT NULL);
        CREATE TABLE panda.backup_settings (secret_key_encrypted bytea);
        CREATE TABLE panda.external_connection_settings (api_key_encrypted bytea, bearer_token_encrypted bytea);
        CREATE TABLE panda.authentication_settings (client_secret_encrypted bytea);")
        .execute(&pool).await.unwrap();
    let mut hold = locked_transaction(&pool).await.unwrap();
    let copy = pool.clone();
    let waiter = tokio::spawn(async move { bootstrap_development_keys(&copy).await });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(!waiter.is_finished());
    waiter.abort();
    assert!(waiter.await.is_err());
    assert_eq!(
        read_database_keys(&mut hold).await.unwrap(),
        [None, None, None]
    );
    hold.rollback().await.unwrap();
    let keys = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        bootstrap_development_keys(&pool),
    )
    .await
    .unwrap()
    .unwrap();
    let reread = export_database_keys(&pool).await.unwrap();
    assert_eq!(keys.age_key, reread.age_key);
    pool.close().await;
    sqlx::query(sqlx::AssertSqlSafe(format!("DROP DATABASE {database}")))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
}
