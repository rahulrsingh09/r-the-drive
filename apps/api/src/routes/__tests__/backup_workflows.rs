#[tokio::test]
#[ignore = "requires unique disposable restricted-role database and loopback S3 fixture"]
async fn full_replacement_remote_catalog_download_and_cleanup() {
    use crate::services::s3_backups::workflow_test_transport::{head, reply, Fake};
    use sha2::{Digest, Sha256};

    let db = DatabaseFixture::new().await;
    let root = support::temporary_directory();
    let mut state = app(&db, root.path()).await;
    sqlx::query("INSERT INTO panda.users(id,email,password_hash,role) VALUES($1,'remote-fixture@example.invalid','inert','admin')")
        .bind(id(1)).execute(&db.pool).await.unwrap();
    let package = support::package();
    let checksum = hex::encode(Sha256::digest(&package));
    let listing = format!(
        "<ListBucketResult><IsTruncated>false</IsTruncated><Contents><Key>fixture/package.rma.tar.gz</Key><Size>{}</Size></Contents></ListBucketResult>",
        package.len()
    );
    let fake = Fake::new("query-route-remote", vec![
        reply("GET", 200, listing.into_bytes()),
        head("2026-01-02T03:04:05Z", &checksum),
        reply("GET", 200, package.clone()),
        reply("GET", 200, package.clone()),
        reply("GET", 200, package.clone()),
    ]).await;
    state.config.panda_env = Some("test".into());
    state.config.security.s3_allow_development_garage = true;
    state.config.s3_endpoint = Some(fake.settings.endpoint.clone());
    state.config.s3_access_key = Some(fake.settings.access_key.clone());
    state.config.s3_secret_key = Some(fake.settings.secret_key.clone());
    fs::create_dir_all(&state.config.backup_artifact_dir).await.unwrap();
    sqlx::query(
        "INSERT INTO panda.backup_settings(id,enabled,frequency,run_at,day_of_week,local_enabled,s3_enabled,bucket,prefix)
         VALUES(TRUE,FALSE,'weekly','03:00',0,TRUE,TRUE,'bucket','fixture')
         ON CONFLICT(id) DO UPDATE SET s3_enabled=TRUE,bucket='bucket',prefix='fixture'")
        .execute(&db.pool).await.unwrap();
    reconcile_s3_catalog(&state).await.unwrap();
    let mut artifacts = load_artifacts(&state).await.unwrap();
    assert_eq!(artifacts.len(), 1);
    let mut artifact = artifacts.pop().unwrap();
    assert_eq!(artifact.storage_path, "s3://bucket/fixture/package.rma.tar.gz");
    assert_eq!(artifact.checksum_sha256, checksum);
    assert_eq!(artifact.manifest["restore_availability"], "available");
    let staged = materialize_artifact(&state, &artifact).await.unwrap();
    assert_eq!(fs::read(&staged.path).await.unwrap(), package);
    let staged_path = staged.path.clone();
    drop(staged);
    assert!(!std::path::Path::new(&staged_path).exists());

    let application = router().layer(Extension(state.jwt_keys.decoding.clone())).with_state(state.clone());
    let response = application.oneshot(Request::builder()
        .uri(format!("/admin/backups/artifacts/{}/download", artifact.id))
        .header("authorization", format!("Bearer {}", token(&state, 1)))
        .body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(to_bytes(response.into_body(), 200000).await.unwrap().as_ref(), package);

    artifact.checksum_sha256 = "incorrect-catalog-checksum".into();
    assert!(matches!(materialize_artifact(&state, &artifact).await, Err(AppError::Validation(_))));
    let staging = std::path::Path::new(&state.config.backup_artifact_dir).join(".remote-staging");
    assert!(std::fs::read_dir(staging).unwrap().next().is_none());
    assert_eq!(fake.finish().as_array().unwrap().len(), 5);
    db.cleanup().await;
}

use super::*;
use axum::{body::to_bytes, http::Request, Extension};
use serde_json::json;
use std::sync::Arc;
use tower::ServiceExt;

use crate::services::backups::workflow_test_support as support;
use support::{id, DatabaseFixture};

#[tokio::test]
#[ignore = "requires unique disposable restricted-role database and loopback restore supervisor"]
async fn full_replacement_restore_handoff_checkpoints_and_rejection() {
    use crate::services::s3_backups::workflow_test_transport::{reply, Fake};
    let db = DatabaseFixture::new().await;
    let root = support::temporary_directory();
    let mut state = app(&db, root.path()).await;
    fs::create_dir_all(&state.config.backup_artifact_dir).await.unwrap();
    fs::write(restore_jobs::agent_key_path(&state.config), "synthetic-agent-key\n").await.unwrap();
    sqlx::query(
        "INSERT INTO panda.backup_artifacts(id,storage_type,file_name,storage_path,size_bytes,checksum_sha256,manifest)
         VALUES($1,'s3','fixture.rma.tar.gz','s3://fixture/fixture.rma.tar.gz',0,'fixture','{}')")
        .bind(id(700)).execute(&db.pool).await.unwrap();
    for (index, status) in [200, 503].into_iter().enumerate() {
        let job_id = id(710 + index as u128);
        let request_id = id(720 + index as u128);
        sqlx::query(
            "INSERT INTO panda.backup_restore_requests(id,artifact_id,status,confirmation_phrase)
             VALUES($1,$2,'pending','RESTORE')")
            .bind(request_id).bind(id(700)).execute(&db.pool).await.unwrap();
        let job: restore_jobs::RestoreJob = serde_json::from_value(json!({
            "id":job_id, "artifact_id":id(700), "artifact_path":"/synthetic/unopened.rma.tar.gz",
            "restore_request_id":request_id, "phase":"queued", "progress_percent":5,
            "message":"Restore queued", "capability_sha256":"synthetic-capability-digest",
            "created_at":"2026-01-01T00:00:00Z", "updated_at":"2026-01-01T00:00:00Z"
        })).unwrap();
        restore_jobs::write(&state.config, &job).await.unwrap();
        let fake = Fake::new("query-route-supervisor", vec![reply("POST", status, vec![])]).await;
        state.config.restore_agent_url = fake.settings.endpoint.clone();
        let result = prepare_and_handoff_restore(state.clone(), job_id).await;
        if status == 200 {
            result.unwrap();
        } else {
            assert!(matches!(result, Err(AppError::DependencyUnavailable(ref message))
                if message == "Restore supervisor rejected the job with status 503 Service Unavailable."));
        }
        let stored = restore_jobs::read(&state.config, job_id).await.unwrap();
        assert!(matches!(stored.phase, restore_jobs::RestorePhase::PreparingCandidate));
        assert_eq!(stored.progress_percent, 15);
        let snapshot = stored.catalog_snapshot.unwrap();
        assert!(snapshot.restore_requests.iter().any(|request| request.id == request_id && request.status == "running"));
        let request_status: String = sqlx::query_scalar("SELECT status FROM panda.backup_restore_requests WHERE id=$1")
            .bind(request_id).fetch_one(&db.pool).await.unwrap();
        assert_eq!(request_status, "running");
        let calls = fake.finish();
        assert_eq!(calls[0]["path"], format!("/internal/jobs/{job_id}/execute"));
        assert_eq!(calls[0]["bytes"], 0);
    }
    db.cleanup().await;
}

async fn app(db: &DatabaseFixture, root: &std::path::Path) -> AppState {
    let keys = crate::keys::bootstrap_development_keys(&db.pool)
        .await
        .unwrap();
    let mut state = AppState {
        pool: db.pool.clone(), redis: redis::Client::open("redis://127.0.0.1:1/").unwrap(),
        jwt_keys: Arc::new(crate::middleware::auth::JwtKeys::new(&keys.jwt_private_pem, &keys.jwt_public_pem).unwrap()),
        age_key: keys.age_key.clone(),
        config: serde_json::from_value(json!({
            "database_url":db.url, "redis_url":"redis://127.0.0.1:1/", "panda_env":"production", "age_encryption_key":keys.age_key,
            "backup_artifact_dir":root.join("backups"), "vehicle_image_cache_dir":root.join("images"),
            "restore_agent_key_file":root.join("agent-key"), "restore_agent_url":"http://127.0.0.1:1",
            "recovery":{"min_free_bytes":0,"max_upload_bytes":200000,"upload_deadline_seconds":1}
        })).unwrap(),
        nominatim_cache: Default::default(), supervisor: crate::ingestion::supervisor::SupervisorHandle::noop(), resources: Default::default(),
    };
    state.config.recovery.min_free_bytes = 0;
    state.config.recovery.max_upload_bytes = 200000;
    state.config.recovery.upload_deadline_seconds = 1;
    state
}

fn token(state: &AppState, user: u128) -> String {
    use crate::middleware::auth::Claims;
    jsonwebtoken::encode(
        &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256),
        &Claims {
            sub: id(user),
            iss: "panda.app".into(),
            iat: Utc::now().timestamp(),
            exp: Utc::now().timestamp() + 600,
            default_vehicle_id: None,
            sid: None,
        },
        &state.jwt_keys.encoding,
    )
    .unwrap()
}

async fn request(
    state: &AppState,
    user: Option<u128>,
    method: &str,
    path: &str,
    body: Body,
    headers: &[(&str, &str)],
) -> Value {
    let router = router()
        .merge(upload_router())
        .layer(Extension(state.jwt_keys.decoding.clone()))
        .with_state(state.clone());
    let mut req = Request::builder().method(method).uri(path);
    if let Some(user) = user {
        req = req.header("authorization", format!("Bearer {}", token(state, user)));
    }
    for (key, value) in headers {
        req = req.header(*key, *value);
    }
    let response = router.oneshot(req.body(body).unwrap()).await.unwrap();
    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), json!(v.to_str().unwrap())))
        .collect::<serde_json::Map<_, _>>();
    let bytes = to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({"bytes":hex::encode(&bytes)}))
    };
    json!({"status":status,"headers":headers,"body":body})
}

async fn json_request(
    state: &AppState,
    user: Option<u128>,
    method: &str,
    path: &str,
    body: Value,
) -> Value {
    request(
        state,
        user,
        method,
        path,
        Body::from(body.to_string()),
        &[("content-type", "application/json")],
    )
    .await
}

async fn snapshot(state: &AppState) -> Value {
    let mut result = serde_json::Map::new();
    for (name, order) in [
        ("backup_settings", "id"),
        ("backup_runs", "created_at,id"),
        ("backup_artifacts", "created_at,id"),
        ("backup_restore_requests", "requested_at,id"),
    ] {
        let rows: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT to_jsonb(t) FROM panda.{name} t ORDER BY {order}"
        )))
        .fetch_all(&state.pool)
        .await
        .unwrap();
        result.insert(name.into(), json!(rows));
    }
    let timezone: Vec<Value> = sqlx::query_scalar(
        "SELECT to_jsonb(t) FROM panda.system_config t WHERE key='app_timezone'",
    )
    .fetch_all(&state.pool)
    .await
    .unwrap();
    result.insert("app_timezone".into(), json!(timezone));
    json!(result)
}

struct Normalizer {
    ids: std::collections::BTreeMap<String, String>,
    root: String,
}
impl Normalizer {
    fn new(root: &std::path::Path) -> Self {
        Self {
            ids: Default::default(),
            root: root.to_string_lossy().into_owned(),
        }
    }
    fn apply(&mut self, value: &mut Value) {
        match value {
            Value::Object(fields) => {
                for (key, v) in fields {
                    if [
                        "created_at",
                        "updated_at",
                        "started_at",
                        "completed_at",
                        "requested_at",
                    ]
                    .contains(&key.as_str())
                        && v.is_string()
                    {
                        if !v.as_str().unwrap().starts_with("2026-01-") {
                            *v = json!("<clock>");
                        }
                    } else if key == "secret_key_encrypted" && !v.is_null() {
                        *v = json!("<random age ciphertext>");
                    } else {
                        self.apply(v);
                    }
                }
            }
            Value::Array(values) => {
                for v in values {
                    self.apply(v)
                }
            }
            Value::String(s) => {
                if let Ok(u) = Uuid::parse_str(s) {
                    if u.as_u128() > 10000 {
                        let n = self.ids.len();
                        *s = self
                            .ids
                            .entry(s.clone())
                            .or_insert_with(|| format!("<uuid-{n}>"))
                            .clone();
                    }
                } else if s.contains(&self.root) {
                    *s = s.replace(&self.root, "<fixture-root>");
                    if let Some(pos) = s.find("/imports/") {
                        *s = format!("{}/imports/<upload-clock-and-id>.rma.tar.gz", &s[..pos]);
                    }
                } else if s.starts_with("attachment; filename=\"import-") {
                    *s = "attachment; filename=\"<upload-clock-and-id>.rma.tar.gz\"".into();
                } else if s.starts_with("import-") && s.ends_with(".rma.tar.gz") {
                    *s = "<upload-clock-and-id>.rma.tar.gz".into();
                }
            }
            _ => {}
        }
    }
}
fn settings() -> Value {
    json!({"enabled":false,"frequency":"weekly","run_at":"03:00","timezone":"UTC","day_of_week":0,"day_of_month":1,"retention_count":8,"local_enabled":true,"s3_enabled":false,"target_type":"s3","endpoint":"","region":null,"bucket":"","prefix":"panda","access_key":null,"secret_key":null,"clear_secret_key":false})
}

#[tokio::test]
#[ignore = "requires a newly created disposable restricted TimescaleDB fixture"]
async fn backup_workflow_http_and_complete_state_contract() {
    let db = DatabaseFixture::new().await;
    let root = support::temporary_directory();
    let state = app(&db, root.path()).await;
    for (n, role) in [
        (1, "admin"),
        (2, "user"),
        (3, "user"),
        (4, "user"),
        (5, "super_user"),
    ] {
        sqlx::query("INSERT INTO panda.users(id,email,password_hash,role,created_at,updated_at) VALUES($1,$2,'inert',$3,'2026-01-01Z','2026-01-01Z')").bind(id(n)).bind(format!("backup-{n}@example.invalid")).bind(role).execute(&db.pool).await.unwrap();
    }
    sqlx::query("INSERT INTO panda.vehicles(id,user_id,rivian_vehicle_id,model) VALUES($1,$2,'synthetic-backup','synthetic')").bind(id(10)).bind(id(2)).execute(&db.pool).await.unwrap();
    for (n, role) in [(2, "owner"), (3, "manager"), (4, "viewer")] {
        sqlx::query(
            "INSERT INTO panda.vehicle_memberships(vehicle_id,user_id,role) VALUES($1,$2,$3)",
        )
        .bind(id(10))
        .bind(id(n))
        .bind(role)
        .execute(&db.pool)
        .await
        .unwrap();
    }
    let mut records = Vec::new();
    let privileges:Value=sqlx::query_scalar("SELECT jsonb_build_object('superuser',rolsuper,'createdb',rolcreatedb,'createrole',rolcreaterole,'bypassrls',rolbypassrls,'delete_history',has_table_privilege(current_user,'timeseries.telemetry','DELETE'),'truncate_charges',has_table_privilege(current_user,'panda.charge_sessions','TRUNCATE')) FROM pg_roles WHERE rolname=current_user").fetch_one(&db.pool).await.unwrap();
    assert_eq!(
        privileges,
        json!({"superuser":false,"createdb":false,"createrole":false,"bypassrls":false,"delete_history":false,"truncate_charges":false})
    );
    records.push(json!({"case":"restricted history role","output":privileges}));
    let initial = snapshot(&state).await;
    for user in [None, Some(2), Some(3), Some(4), Some(99)] {
        for (method, path, body) in [
            ("GET", "/admin/backups", Value::Null),
            ("PUT", "/admin/backups/settings", settings()),
            ("POST", "/admin/backups/run", json!({})),
            (
                "POST",
                "/admin/backups/restore-requests",
                json!({"artifact_id":id(999),"confirmation_phrase":"RESTORE"}),
            ),
            (
                "POST",
                "/admin/backups/restores/preflight",
                json!({"artifact_id":id(999)}),
            ),
        ] {
            let output = json_request(&state, user, method, path, body).await;
            assert!(matches!(output["status"].as_u64(), Some(401 | 403)));
            records.push(json!({"case":"authorization","user":user,"path":path,"output":output}));
        }
    }
    assert_eq!(initial, snapshot(&state).await);
    for user in [1, 5] {
        records.push(json!({"case":"empty overview","user":user,"output":request(&state,Some(user),"GET","/admin/backups",Body::empty(),&[]).await}));
    }
    for (name, change) in [
        (
            "run time first",
            json!({"run_at":"25:00","timezone":"nope","day_of_week":9}),
        ),
        (
            "timezone second",
            json!({"timezone":"nope","day_of_week":9}),
        ),
        ("weekly missing", json!({"day_of_week":null})),
        (
            "monthly zero",
            json!({"frequency":"monthly","day_of_month":0}),
        ),
        ("no destination", json!({"local_enabled":false})),
        (
            "bucket before endpoint",
            json!({"s3_enabled":true,"endpoint":"nonsense"}),
        ),
        (
            "bad endpoint",
            json!({"s3_enabled":true,"bucket":"fixture","endpoint":"nonsense"}),
        ),
        (
            "endpoint scheme",
            json!({"s3_enabled":true,"bucket":"fixture","endpoint":"file:///tmp"}),
        ),
        (
            "ssrf denial",
            json!({"s3_enabled":true,"bucket":"fixture","endpoint":"http://127.0.0.1:1"}),
        ),
        ("prefix traversal", json!({"prefix":"../escape"})),
        ("retention constraint", json!({"retention_count":0})),
    ] {
        let mut b = settings();
        b.as_object_mut()
            .unwrap()
            .extend(change.as_object().unwrap().clone());
        let output = json_request(&state, Some(1), "PUT", "/admin/backups/settings", b).await;
        assert!(output["status"].as_u64().unwrap() >= 400, "{name}");
        records.push(json!({"case":name,"output":output,"state":snapshot(&state).await}));
    }
    for (name, body) in [
        ("malformed", Body::from("{")),
        ("oversized", Body::from(" ".repeat(2 * 1024 * 1024 + 1))),
    ] {
        records.push(json!({"case":name,"output":request(&state,Some(1),"PUT","/admin/backups/settings",body,&[("content-type","application/json")]).await}));
    }
    for (name, change) in [
        (
            "normalization",
            json!({"frequency":"daily","day_of_week":6,"day_of_month":31,"run_at":" 23:59 ","timezone":" Asia/Kolkata ","region":" ","bucket":" bucket ","prefix":"demo/prefix/","access_key":" fixture-access ","secret_key":" fixture-secret "}),
        ),
        ("preserve secret", json!({"secret_key":null})),
        (
            "replace secret",
            json!({"secret_key":"replacement-fixture"}),
        ),
        (
            "clear overrides replace",
            json!({"secret_key":"discarded-fixture","clear_secret_key":true}),
        ),
    ] {
        let mut b = settings();
        b.as_object_mut()
            .unwrap()
            .extend(change.as_object().unwrap().clone());
        let output = json_request(&state, Some(1), "PUT", "/admin/backups/settings", b).await;
        assert_eq!(output["status"], 200, "{name}: {output}");
        records.push(json!({"case":name,"output":output,"state":snapshot(&state).await}));
    }
    for (name, bytes, headers) in [
        (
            "declared size precedes extension",
            vec![],
            vec![
                ("content-length", "200001"),
                ("x-panda-file-name", "bad.txt"),
            ],
        ),
        (
            "extension rejection",
            vec![],
            vec![("x-panda-file-name", "bad.txt")],
        ),
        ("empty archive", vec![], vec![]),
        ("invalid archive", b"not gzip".to_vec(), vec![]),
        (
            "truncated archive",
            {
                let mut p = support::package();
                p.truncate(p.len() - 8);
                p
            },
            vec![],
        ),
        ("stream limit", vec![0u8; 200001], vec![]),
    ] {
        let output = request(
            &state,
            Some(1),
            "POST",
            "/admin/backups/imports",
            Body::from(bytes),
            &headers,
        )
        .await;
        assert!(output["status"].as_u64().unwrap() >= 400);
        records.push(json!({"case":name,"output":output,"state":snapshot(&state).await}));
    }
    let interrupted = Body::from_stream(futures::stream::iter(vec![
        Ok::<_, std::io::Error>(axum::body::Bytes::from_static(b"partial")),
        Err(std::io::Error::other("synthetic interruption")),
    ]));
    records.push(json!({"case":"interrupted upload","output":request(&state,Some(1),"POST","/admin/backups/imports",interrupted,&[]).await,"state":snapshot(&state).await}));
    let stalled = Body::from_stream(futures::stream::pending::<
        Result<axum::body::Bytes, std::io::Error>,
    >());
    records.push(json!({"case":"upload deadline","output":request(&state,Some(1),"POST","/admin/backups/imports",stalled,&[]).await,"state":snapshot(&state).await}));
    let bytes = support::package();
    let uploaded = request(
        &state,
        Some(1),
        "POST",
        "/admin/backups/imports",
        Body::from(bytes.clone()),
        &[("x-panda-file-name", "../../fixture.rma.tar.gz")],
    )
    .await;
    assert_eq!(uploaded["status"], 201, "{uploaded}");
    let artifact: Uuid = uploaded["body"]["artifact"]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let run: Uuid = uploaded["body"]["artifact"]["run_id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    records
        .push(json!({"case":"successful upload","output":uploaded,"state":snapshot(&state).await}));
    let download = request(
        &state,
        Some(1),
        "GET",
        &format!("/admin/backups/artifacts/{artifact}/download"),
        Body::empty(),
        &[],
    )
    .await;
    assert_eq!(download["body"]["bytes"], hex::encode(bytes));
    records.push(json!({"case":"download complete bytes","output":download}));
    for phrase in ["wrong", " RESTORE ", "RESTORE"] {
        records.push(json!({"case":"restore request","phrase":phrase,"output":json_request(&state,Some(1),"POST","/admin/backups/restore-requests",json!({"artifact_id":artifact,"confirmation_phrase":phrase,"notes":" fixture notes "})).await,"state":snapshot(&state).await}));
    }
    records.push(json!({"case":"active import delete blocked","output":request(&state,Some(1),"DELETE",&format!("/admin/backups/imports/{artifact}"),Body::empty(),&[]).await,"state":snapshot(&state).await}));
    records.push(json!({"case":"restore supervisor unavailable before phrase","output":json_request(&state,Some(1),"POST","/admin/backups/restores",json!({"artifact_id":artifact,"confirmation_phrase":"wrong","plan_id":"bad","package_checksum_sha256":"bad"})).await}));
    records.push(json!({"case":"preflight missing","output":json_request(&state,Some(1),"POST","/admin/backups/restores/preflight",json!({"artifact_id":id(999)})).await}));
    records.push(json!({"case":"preflight malformed dump","output":json_request(&state,Some(1),"POST","/admin/backups/restores/preflight",json!({"artifact_id":artifact})).await}));
    for query in [
        "page=0&per_page=0",
        "page=2&per_page=3",
        "page=4294967295&per_page=4294967295",
    ] {
        records.push(json!({"case":"pagination","query":query,"output":request(&state,Some(1),"GET",&format!("/admin/backups?{query}"),Body::empty(),&[]).await}));
    }
    sqlx::query("UPDATE panda.backup_restore_requests SET status='failed' WHERE artifact_id=$1")
        .bind(artifact)
        .execute(&db.pool)
        .await
        .unwrap();
    records.push(json!({"case":"delete imported package","output":request(&state,Some(1),"DELETE",&format!("/admin/backups/imports/{artifact}"),Body::empty(),&[]).await,"state":snapshot(&state).await}));
    assert!(!sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM panda.backup_runs WHERE id=$1)"
    )
    .bind(run)
    .fetch_one(&db.pool)
    .await
    .unwrap());
    let raw = json!(records);
    support::evidence(
        "http-original.json",
        &serde_json::to_vec_pretty(&raw).unwrap(),
    );
    let mut normalized = raw;
    Normalizer::new(root.path()).apply(&mut normalized);
    support::record(
        "http",
        &normalized,
        include_str!("backup_workflows/http.json"),
    );
    db.cleanup().await;
}

#[test]
fn backup_workflow_next_schedule_contract() {
    let mut records = Vec::new();
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
                let now = now
                    .parse::<DateTime<Utc>>()
                    .unwrap()
                    .with_timezone(&timezone);
                records.push(json!({"zone":zone,"time":time,"now":now,"daily":next_daily_run(now,run_at),"sunday":next_weekly_run(now,run_at,0),"saturday":next_weekly_run(now,run_at,6),"first":next_monthly_run(now,run_at,1),"last":next_monthly_run(now,run_at,31)}));
            }
        }
    }
    support::record(
        "next-schedule",
        &json!(records),
        include_str!("backup_workflows/next-schedule.json"),
    );
}
