use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;
pub struct DatabaseFixture {
    pub admin: PgPool,
    pub pool: PgPool,
    pub database: String,
    pub role: String,
    pub url: String,
}

impl DatabaseFixture {
    pub async fn new() -> Self {
        use sqlx::Executor;
        let mut url = url::Url::parse(
            &std::env::var("BACKUP_CONTRACT_DATABASE_URL")
                .expect("explicit disposable BACKUP_CONTRACT_DATABASE_URL required"),
        )
        .unwrap();
        assert_eq!(url.host_str(), Some("db"));
        assert_eq!(url.username(), "backup_fixture");
        assert_eq!(url.path(), "/backup_fixture");
        url.set_path("/postgres");
        let admin = PgPool::connect(url.as_str()).await.unwrap();
        let suffix = Uuid::new_v4().simple().to_string();
        let role = format!("backup_role_{suffix}");
        let database = format!("backup_contract_{suffix}");
        admin.execute(sqlx::AssertSqlSafe(format!(
            "CREATE ROLE {role} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOREPLICATION NOBYPASSRLS PASSWORD 'synthetic_backup_app'"
        ))).await.unwrap();
        admin
            .execute(sqlx::AssertSqlSafe(format!(
                "CREATE DATABASE {database} OWNER {role}"
            )))
            .await
            .unwrap();
        url.set_path(&format!("/{database}"));
        let bootstrap = PgPool::connect(url.as_str()).await.unwrap();
        bootstrap
            .execute(sqlx::AssertSqlSafe(format!(
                "CREATE SCHEMA panda AUTHORIZATION {role};
             REVOKE CREATE ON SCHEMA public FROM PUBLIC;
             REVOKE ALL ON SCHEMA panda FROM PUBLIC;
             CREATE EXTENSION IF NOT EXISTS pgcrypto WITH SCHEMA public;
             CREATE EXTENSION IF NOT EXISTS timescaledb WITH SCHEMA public;
             CREATE EXTENSION IF NOT EXISTS cube WITH SCHEMA panda;
             CREATE EXTENSION IF NOT EXISTS earthdistance WITH SCHEMA panda;"
            )))
            .await
            .unwrap();
        bootstrap.close().await;
        url.set_username(&role).unwrap();
        url.set_password(Some("synthetic_backup_app")).unwrap();
        let migration_pool = PgPool::connect(url.as_str()).await.unwrap();
        crate::db::migrations::run_current_migrations(&migration_pool)
            .await
            .unwrap();
        migration_pool.close().await;
        let pool = crate::db::pool::application_pool_options()
            .max_connections(8)
            .connect(url.as_str())
            .await
            .unwrap();
        let config = serde_json::from_value(json!({
            "database_url":url.as_str(), "redis_url":"redis://unused", "panda_env":"production"
        }))
        .unwrap();
        crate::private_deployment::history::protect_history(&pool, &config)
            .await
            .unwrap();
        Self {
            admin,
            pool,
            database,
            role,
            url: url.to_string(),
        }
    }

    pub async fn cleanup(self) {
        use sqlx::Executor;
        self.pool.close().await;
        self.admin
            .execute(sqlx::AssertSqlSafe(format!(
                "DROP DATABASE {} WITH (FORCE)",
                self.database
            )))
            .await
            .unwrap();
        self.admin
            .execute(sqlx::AssertSqlSafe(format!("DROP ROLE {}", self.role)))
            .await
            .unwrap();
        self.admin.close().await;
    }
}

pub fn id(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

pub fn temporary_directory() -> tempfile::TempDir {
    match std::env::var_os("BACKUP_CONTRACT_ARTIFACT_DIR") {
        Some(directory) => tempfile::tempdir_in(directory).unwrap(),
        None => tempfile::tempdir().unwrap(),
    }
}

pub fn evidence(name: &str, content: &[u8]) {
    if let Some(directory) = std::env::var_os("BACKUP_CONTRACT_ARTIFACT_DIR") {
        let run = std::env::var("BACKUP_CONTRACT_RUN").unwrap_or_else(|_| "actual".into());
        std::fs::write(
            std::path::Path::new(&directory).join(format!("{run}-{name}")),
            content,
        )
        .unwrap();
    }
}

pub fn record(name: &str, raw: &Value, expected: &str) {
    evidence(
        &format!("{name}-raw.json"),
        &serde_json::to_vec_pretty(raw).unwrap(),
    );
    let frozen: Value = serde_json::from_str(expected).expect("frozen expectation");
    assert_eq!(raw, &frozen, "complete {name} contract");
}

pub fn package() -> Vec<u8> {
    use sha2::{Digest, Sha256};
    let mut database = b"PGDMP".to_vec();
    let mut state = 0x9e37_79b9_u32;
    for _ in 0..(96 * 1024) {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        database.push(state as u8);
    }
    let settings = br#"{"present":false}"#;
    let manifest = serde_json::to_vec(&json!({
        "format":"panda-recovery-v1", "format_version":1,
        "components":{
            "database":{"path":"database.dump", "sha256":hex::encode(Sha256::digest(&database)),"size_bytes":database.len()},
            "backup_settings":{"path":"backup-settings.json","sha256":hex::encode(Sha256::digest(settings)),"size_bytes":settings.len()},
            "vehicle_image_cache":{"files":[]}
        }
    })).unwrap();
    let mut archive = tar::Builder::new(flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::default(),
    ));
    for (path, bytes) in [
        ("manifest.json", manifest.as_slice()),
        ("database.dump", database.as_slice()),
        ("backup-settings.json", settings.as_slice()),
    ] {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o600);
        header.set_cksum();
        archive
            .append_data(&mut header, path, std::io::Cursor::new(bytes))
            .unwrap();
    }
    archive.into_inner().unwrap().finish().unwrap()
}
