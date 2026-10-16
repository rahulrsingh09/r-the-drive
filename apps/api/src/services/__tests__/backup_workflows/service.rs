use super::*;
use serde_json::Value;

use crate::services::backups::workflow_test_support as support;

#[test]
fn backup_workflow_schedule_and_selection_contract() {
    let mut rows = Vec::new();
    for zone in ["UTC", "America/Chicago", "Asia/Kolkata"] {
        let timezone: Tz = zone.parse().unwrap();
        for time in ["00:00:00", "02:30:00", "23:59:00"] {
            let run_at: NaiveTime = time.parse().unwrap();
            for now in [
                "2024-02-29T02:30:00Z",
                "2026-01-01T00:00:00Z",
                "2026-03-08T08:30:00Z",
                "2026-11-01T07:30:00Z",
                "2026-12-31T23:59:00Z",
            ] {
                let now: DateTime<Utc> = now.parse().unwrap();
                for (frequency, day) in [
                    ("daily", 0),
                    ("weekly", 0),
                    ("weekly", 6),
                    ("monthly", 1),
                    ("monthly", 31),
                ] {
                    let settings = BackupSettings {
                        enabled: true,
                        frequency: BackupFrequency::try_from(frequency).unwrap(),
                        run_at,
                        timezone,
                        day_of_week: Some(day),
                        day_of_month: Some(day),
                        retention_count: 8,
                        prefix: "fixture".into(),
                        local_enabled: true,
                        s3_enabled: false,
                        s3: None,
                    };
                    rows.push(json!({"zone":zone,"time":time,"now":now,"frequency":frequency,"day":day,"due":compute_due_run_at(&settings,now).unwrap()}));
                }
            }
        }
    }
    for prefix in ["", "panda", "nested/prefix/", "../escape", "x//y", "x\\y"] {
        let cfg:Config=serde_json::from_value(json!({"database_url":"postgres://synthetic","redis_url":"redis://synthetic","backup_artifact_dir":"/fixture"})).unwrap();
        rows.push(json!({"prefix":prefix,"artifact":build_artifact_path(&cfg,prefix,"2026-01-01T00:00:00Z".parse().unwrap(),Uuid::nil()).map(|p|p.display().to_string()).map_err(|e|e.to_string()),"normalize":normalize_prefix(prefix).map_err(|e|e.to_string())}));
    }
    for trigger in [
        BackupRunTrigger::Manual,
        BackupRunTrigger::Scheduled,
        BackupRunTrigger::PreRestore,
    ] {
        rows.push(json!({"trigger":trigger.as_str()}));
    }
    support::record("schedule", &json!(rows), include_str!("schedule.json"));
}

fn config(db: &support::DatabaseFixture, root: &Path) -> Config {
    let mut cfg:Config = serde_json::from_value(json!({"database_url":db.url,"redis_url":"redis://127.0.0.1:1","panda_env":"production","backup_artifact_dir":root.join("backups"),"vehicle_image_cache_dir":root.join("images"),"recovery":{"min_free_bytes":0}})).unwrap();
    cfg.recovery.min_free_bytes = 0;
    cfg
}
fn normalize(value: &mut Value, root: &Path) {
    match value {
        Value::Object(fields) => {
            for (key, v) in fields {
                if ["created_at", "updated_at", "started_at", "completed_at"]
                    .contains(&key.as_str())
                    && v.is_string()
                {
                    *v = json!("<clock>")
                } else {
                    normalize(v, root)
                }
            }
        }
        Value::Array(values) => {
            for v in values {
                normalize(v, root)
            }
        }
        Value::String(s) => {
            *s = s.replace(root.to_str().unwrap(), "<fixture-root>");
        }
        _ => {}
    }
}
async fn state(pool: &PgPool) -> Value {
    let runs: Vec<Value> =
        sqlx::query_scalar("SELECT to_jsonb(r) FROM panda.backup_runs r ORDER BY id")
            .fetch_all(pool)
            .await
            .unwrap();
    let artifacts: Vec<Value> =
        sqlx::query_scalar("SELECT to_jsonb(a) FROM panda.backup_artifacts a ORDER BY id")
            .fetch_all(pool)
            .await
            .unwrap();
    json!({"runs":runs,"artifacts":artifacts})
}

#[tokio::test]
#[ignore = "requires a new disposable restricted TimescaleDB fixture"]
async fn backup_workflow_retention_failure_restart_contract() {
    use sqlx::Executor;
    let db = support::DatabaseFixture::new().await;
    let root = support::temporary_directory();
    let cfg = config(&db, root.path());
    fs::create_dir_all(&cfg.backup_artifact_dir).await.unwrap();
    let mut rows = Vec::new();
    for (n, storage) in [
        (1, "local"),
        (2, "local"),
        (3, "safety"),
        (4, "uploaded"),
        (5, "s3"),
    ] {
        let path = Path::new(&cfg.backup_artifact_dir).join(format!("{n}.rma.tar.gz"));
        fs::write(&path, format!("fixture-{n}")).await.unwrap();
        sqlx::query("INSERT INTO panda.backup_runs(id,trigger,status,phase,progress_percent,artifact_key,created_at,updated_at) VALUES($1,'manual','succeeded','completed',100,$2,$3,$3)")
            .bind(support::id(n)).bind(path.to_str()).bind("2026-01-01T00:00:00Z".parse::<DateTime<Utc>>().unwrap()+Duration::hours(n as i64)).execute(&db.pool).await.unwrap();
        sqlx::query("INSERT INTO panda.backup_artifacts(id,run_id,storage_type,file_name,storage_path,size_bytes,checksum_sha256,manifest,created_at) VALUES($1,$2,$3,$4,$5,9,'fixture','{}',$6)")
            .bind(support::id(n+100)).bind(support::id(n)).bind(storage).bind(format!("{n}.rma.tar.gz")).bind(path.to_str()).bind("2026-01-01T00:00:00Z".parse::<DateTime<Utc>>().unwrap()+Duration::hours(n as i64)).execute(&db.pool).await.unwrap();
    }
    rows.push(json!({"case":"seed","state":state(&db.pool).await}));
    prune_retained_artifacts(&db.pool, &cfg, 0).await.unwrap();
    rows.push(json!({"case":"retention minimum one","state":state(&db.pool).await,"files":[Path::new(&cfg.backup_artifact_dir).join("1.rma.tar.gz").exists(),Path::new(&cfg.backup_artifact_dir).join("2.rma.tar.gz").exists(),Path::new(&cfg.backup_artifact_dir).join("3.rma.tar.gz").exists()]}));
    let once = state(&db.pool).await;
    prune_retained_artifacts(&db.pool, &cfg, 1).await.unwrap();
    assert_eq!(once, state(&db.pool).await);
    sqlx::query("INSERT INTO panda.backup_runs(id,trigger,status,phase,progress_percent) VALUES($1,'manual','running','dumping',10)").bind(support::id(6)).execute(&db.pool).await.unwrap();
    let changed = reconcile_running_runs(&db.pool).await.unwrap();
    assert_eq!(changed, 1);
    assert_eq!(reconcile_running_runs(&db.pool).await.unwrap(), 0);
    rows.push(json!({"case":"restart reconciles once","state":state(&db.pool).await}));
    let lock = BackupLock::acquire(&db.pool).await.unwrap();
    let denied = BackupLock::acquire(&db.pool)
        .await
        .err()
        .unwrap()
        .to_string();
    assert_eq!(denied, "Conflict: A backup job is already running.");
    rows.push(json!({"case":"concurrent admission","error":denied}));
    drop(lock);
    let mut acquired = None;
    for _ in 0..100 {
        if let Ok(lock) = BackupLock::acquire(&db.pool).await {
            acquired = Some(lock);
            break;
        }
        tokio::time::sleep(StdDuration::from_millis(10)).await;
    }
    acquired
        .expect("canceled owner releases session lock")
        .release()
        .await;
    db.pool.execute("INSERT INTO panda.backup_settings(id,frequency,run_at,timezone,prefix) VALUES(TRUE,'daily','03:00','UTC','../escape')").await.unwrap();
    sqlx::query("INSERT INTO panda.backup_runs(id,trigger,status,phase,progress_percent) VALUES($1,'manual','running','queued',0)").bind(support::id(7)).execute(&db.pool).await.unwrap();
    let error = run_backup_inner_for_run(&db.pool, &cfg, support::id(7), BackupRunTrigger::Manual)
        .await
        .unwrap_err();
    rows.push(json!({"case":"invalid settings fails durable run","error":error.to_string(),"state":state(&db.pool).await}));
    let mut raw = json!(rows);
    support::evidence(
        "service-original.json",
        &serde_json::to_vec_pretty(&raw).unwrap(),
    );
    normalize(&mut raw, root.path());
    support::record("service", &raw, include_str!("service.json"));
    db.cleanup().await;
}

#[path = "execution.rs"]
mod execution_contracts;
