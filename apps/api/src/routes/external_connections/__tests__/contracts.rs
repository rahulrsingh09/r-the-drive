use super::*;
use serde_json::{json, Value};
pub(super) mod fixture;
use fixture::{freeze, id, Fixture};

const SETTINGS: &str = "/v1/settings/external-connections";

#[test]
fn external_workflow_secret_adapter_contract() {
    let generated = crate::keys::generate_keys().unwrap();
    assert!(encrypt_secret("invalid", None).unwrap().is_none());
    assert!(encrypt_secret("invalid", Some(" ")).unwrap().is_none());
    assert!(encrypt_secret("invalid", Some("synthetic")).is_err());
    let first = encrypt_secret(&generated.age_key, Some(" synthetic "))
        .unwrap()
        .unwrap();
    let second = encrypt_secret(&generated.age_key, Some("synthetic"))
        .unwrap()
        .unwrap();
    assert_ne!(first, second, "age encryption retains randomization");
    assert!(
        decrypt_secret(&generated.age_key, Some(&first))
            .unwrap()
            .as_deref()
            == Some("synthetic")
    );
    assert!(decrypt_secret("invalid", Some(&first)).is_err());
    assert!(decrypt_secret(&generated.age_key, Some(&[0, 1, 2])).is_err());
}

async fn observe(
    f: &Fixture,
    rows: &mut Vec<Value>,
    label: &str,
    user: Option<u128>,
    method: &str,
    path: &str,
    body: Value,
) {
    let response = f
        .request(user.map(id), method, path, &body.to_string())
        .await;
    rows.push(json!({"case":label, "response":response, "rows":f.rows().await}));
}

async fn seeded_fixture() -> Fixture {
    let f = Fixture::new().await;
    for (user, role) in [
        (1, "admin"),
        (2, "super_user"),
        (3, "user"),
        (4, "user"),
        (5, "user"),
        (6, "user"),
    ] {
        f.insert_user(id(user), role).await;
    }
    for (vehicle, owner) in [(10, 3), (11, 6)] {
        sqlx::query("INSERT INTO panda.vehicles (id,user_id,rivian_vehicle_id,model,created_at,updated_at) VALUES ($1,$2,$3,'R1T','2026-01-01Z','2026-01-01Z')")
            .bind(id(vehicle)).bind(id(owner)).bind(format!("synthetic-external-{vehicle}"))
            .execute(&f.state.pool).await.unwrap();
    }
    for (key, vehicle, user, role) in [
        (20, 10, 3, "owner"),
        (21, 10, 4, "manager"),
        (22, 10, 5, "viewer"),
        (23, 11, 6, "owner"),
    ] {
        sqlx::query("INSERT INTO panda.vehicle_memberships (id,vehicle_id,user_id,role,created_at,updated_at) VALUES ($1,$2,$3,$4,'2026-01-01Z','2026-01-01Z')")
            .bind(id(key)).bind(id(vehicle)).bind(id(user)).bind(role)
            .execute(&f.state.pool).await.unwrap();
    }
    connections::ensure_defaults(&f.state.pool).await.unwrap();
    f.execute(
        "UPDATE panda.external_connection_settings SET updated_at='2026-01-01Z';
        UPDATE panda.backup_settings SET updated_at='2026-01-01Z';",
    )
    .await;
    f
}

#[tokio::test]
#[ignore = "requires unique disposable EXTERNAL_CONTRACT_DATABASE_URL and REDIS_URL"]
async fn external_workflow_http_and_persisted_contract() {
    let mut f = seeded_fixture().await;
    let mut rows = vec![];
    let before = f.rows().await;
    connections::ensure_defaults(&f.state.pool).await.unwrap();
    assert_eq!(
        before,
        f.rows().await,
        "default insertion is exactly idempotent"
    );
    for user in [
        None,
        Some(1),
        Some(2),
        Some(3),
        Some(4),
        Some(5),
        Some(6),
        Some(99),
    ] {
        observe(
            &f,
            &mut rows,
            &format!("initial-list-{user:?}"),
            user,
            "GET",
            SETTINGS,
            json!({}),
        )
        .await;
    }
    for user in [3, 4, 5, 6] {
        for (suffix, method, body) in [
            (
                "/open_meteo",
                "PUT",
                json!({"enabled":true,"mode":"remote"}),
            ),
            ("/disable-optional", "POST", json!({})),
            ("/basemap/cache/purge", "POST", json!({})),
            (
                "/open_meteo/test",
                "POST",
                json!({"enabled":true,"mode":"remote"}),
            ),
        ] {
            observe(
                &f,
                &mut rows,
                &format!("denied-{user}-{suffix}"),
                Some(user),
                method,
                &format!("{SETTINGS}{suffix}"),
                body,
            )
            .await;
        }
    }
    assert_eq!(before, f.rows().await, "denials cannot mutate rows");
    for target in [
        "missing",
        "rivian_account",
        "s3_backup",
        "nominatim",
        "basemap",
        "iconify",
    ] {
        observe(
            &f,
            &mut rows,
            &format!("enable-{target}"),
            Some(1),
            "PUT",
            &format!("{SETTINGS}/{target}"),
            json!({"enabled":true,"mode":"remote"}),
        )
        .await;
    }
    for (label, body) in [
        (
            "unknown-mode",
            json!({"enabled":true,"mode":"wrong","weather_precision":"wrong"}),
        ),
        (
            "precision",
            json!({"enabled":true,"mode":"remote","weather_precision":"exact"}),
        ),
        (
            "secret",
            json!({"enabled":true,"mode":"remote","api_key":"synthetic-input"}),
        ),
        (
            "whitespace-secret",
            json!({"enabled":true,"mode":"remote","bearer_token":" "}),
        ),
        (
            "nonmap-preference",
            json!({"enabled":true,"mode":"remote","basemap_provider":"auto"}),
        ),
        (
            "enabled-disabled-mode",
            json!({"enabled":true,"mode":"disabled"}),
        ),
        ("weather-remote", json!({"enabled":true,"mode":"remote"})),
        ("weather-repeat", json!({"enabled":true,"mode":"remote"})),
        (
            "weather-hosted-stale",
            json!({"enabled":true,"mode":"hosted","forecast_url":"http://127.0.0.1/stale","archive_url":"https://example.invalid/stale","allow_private_network":true,"private_network_allowlist":["10.0.0.0/8"]}),
        ),
        (
            "weather-clear-presentation",
            json!({"enabled":true,"mode":"remote","base_url":"  ","request_identifier":"  ","dark_url_template":"  "}),
        ),
        (
            "weather-disable",
            json!({"enabled":false,"mode":"disabled","weather_precision":"exact"}),
        ),
        (
            "weather-disabled-test",
            json!({"enabled":false,"mode":"disabled"}),
        ),
    ] {
        let suffix = if label == "weather-disabled-test" {
            "/open_meteo/test"
        } else {
            "/open_meteo"
        };
        let method = if label == "weather-disabled-test" {
            "POST"
        } else {
            "PUT"
        };
        observe(
            &f,
            &mut rows,
            label,
            Some(1),
            method,
            &format!("{SETTINGS}{suffix}"),
            body,
        )
        .await;
    }
    observe(
        &f,
        &mut rows,
        "weather-unsaved-enabled-diagnostic",
        Some(1),
        "POST",
        &format!("{SETTINGS}/open_meteo/test"),
        json!({"enabled":true,"mode":"remote"}),
    )
    .await;
    for (label, body) in [
        (
            "map-disabled-custom",
            json!({"enabled":false,"mode":"custom","basemap_provider":"auto","light_url_template":" https://example.invalid/{z}/{x}/{y} ","dark_url_template":" dark ","attribution":" fixture ","attribution_url":" https://example.invalid/ ","request_identifier":" agent ","custom_autocomplete":true,"private_network_allowlist":["10.0.0.0/8","10.0.0.0/8"]}),
        ),
        (
            "map-preserve-null",
            json!({"enabled":false,"mode":"disabled"}),
        ),
        (
            "map-clear-cidr",
            json!({"enabled":false,"mode":"remote","private_network_allowlist":[],"basemap_provider":"carto","clear_api_key":true}),
        ),
        (
            "map-invalid-provider",
            json!({"enabled":false,"mode":"disabled","basemap_provider":"unknown"}),
        ),
        (
            "map-invalid-cidr",
            json!({"enabled":false,"mode":"disabled","private_network_allowlist":["127.0.0.0/8"]}),
        ),
        (
            "map-invalid-mode-first",
            json!({"enabled":false,"mode":"wat","private_network_allowlist":["wat"],"basemap_provider":"wat"}),
        ),
    ] {
        observe(
            &f,
            &mut rows,
            label,
            Some(1),
            "PUT",
            &format!("{SETTINGS}/basemap"),
            body,
        )
        .await;
    }
    f.execute("UPDATE panda.external_connection_settings SET api_key_encrypted=decode('010203','hex'),bearer_token_encrypted=decode('040506','hex') WHERE id='basemap'").await;
    observe(
        &f,
        &mut rows,
        "stored-key-redaction",
        Some(3),
        "GET",
        SETTINGS,
        json!({}),
    )
    .await;
    observe(
        &f,
        &mut rows,
        "clear-both-secrets",
        Some(2),
        "PUT",
        &format!("{SETTINGS}/basemap"),
        json!({"enabled":false,"mode":"disabled","clear_api_key":true,"clear_bearer_token":true}),
    )
    .await;
    observe(
        &f,
        &mut rows,
        "disable-all",
        Some(2),
        "POST",
        &format!("{SETTINGS}/disable-optional"),
        json!({}),
    )
    .await;
    observe(
        &f,
        &mut rows,
        "disable-all-repeat",
        Some(2),
        "POST",
        &format!("{SETTINGS}/disable-optional"),
        json!({}),
    )
    .await;
    for path in [
        "/v1/external/basemap/config",
        "/v1/external/basemap/raster/light/6/14/24.png",
        "/v1/external/basemap/openfreemap/styles/positron",
        "/v1/external/basemap/openfreemap/invalid",
        "/v1/external/iconify/search?query=fixture",
        "/v1/external/iconify/fixture.json",
    ] {
        observe(&f, &mut rows, path, Some(3), "GET", path, json!({})).await;
    }
    for target in [
        "rivian_account",
        "s3_backup",
        "nominatim",
        "basemap",
        "iconify",
        "missing",
    ] {
        observe(
            &f,
            &mut rows,
            &format!("diagnostic-{target}"),
            Some(1),
            "POST",
            &format!("{SETTINGS}/{target}/test"),
            json!({"enabled":true,"mode":"remote"}),
        )
        .await;
    }
    for (label, body) in [
        ("empty", "".to_owned()),
        ("malformed", "{".into()),
        ("null", "null".into()),
        ("missing", "{}".into()),
        ("wrong-type", r#"{"enabled":"true","mode":"remote"}"#.into()),
        (
            "oversized",
            json!({"enabled":true,"mode":"remote","api_key":"x".repeat(2*1024*1024)}).to_string(),
        ),
    ] {
        let response = f
            .request(Some(id(1)), "PUT", &format!("{SETTINGS}/open_meteo"), &body)
            .await;
        rows.push(json!({"case":label,"response":response,"rows":f.rows().await}));
    }
    // A response read can fail after the settings statement has committed.
    f.execute("REVOKE SELECT ON panda.backup_settings FROM CURRENT_USER")
        .await;
    let response = f
        .request(
            Some(id(1)),
            "PUT",
            &format!("{SETTINGS}/open_meteo"),
            r#"{"enabled":true,"mode":"remote"}"#,
        )
        .await;
    f.execute("GRANT SELECT ON panda.backup_settings TO CURRENT_USER")
        .await;
    rows.push(json!({"case":"late-response-failure","response":response,"rows":f.rows().await}));
    observe(
        &f,
        &mut rows,
        "late-failure-retry",
        Some(1),
        "PUT",
        &format!("{SETTINGS}/open_meteo"),
        json!({"enabled":true,"mode":"remote"}),
    )
    .await;
    f.execute("REVOKE UPDATE ON panda.external_connection_settings FROM CURRENT_USER")
        .await;
    let response = f
        .request(
            Some(id(1)),
            "PUT",
            &format!("{SETTINGS}/open_meteo"),
            r#"{"enabled":false,"mode":"disabled"}"#,
        )
        .await;
    f.execute("GRANT UPDATE ON panda.external_connection_settings TO CURRENT_USER")
        .await;
    rows.push(json!({"case":"write-failure","response":response,"rows":f.rows().await}));
    observe(
        &f,
        &mut rows,
        "write-failure-retry",
        Some(1),
        "PUT",
        &format!("{SETTINGS}/open_meteo"),
        json!({"enabled":false,"mode":"disabled"}),
    )
    .await;
    f.state.pool.close().await;
    f.state.pool = crate::db::pool::application_pool_options()
        .connect(&f.state.config.database_url)
        .await
        .unwrap();
    observe(
        &f,
        &mut rows,
        "reopened-pool",
        Some(1),
        "GET",
        SETTINGS,
        json!({}),
    )
    .await;
    let raw = json!(rows);
    f.cleanup().await;
    freeze("http", &raw, include_str!("http.json"));
}

#[tokio::test]
#[ignore = "requires unique disposable EXTERNAL_CONTRACT_DATABASE_URL and REDIS_URL"]
async fn external_workflow_activity_renewal_and_cache_contract() {
    let f = seeded_fixture().await;
    let pool = &f.state.pool;
    let mut rows = vec![];
    let protection: Value = sqlx::query_scalar("SELECT jsonb_build_object(
        'superuser',rolsuper,'createdb',rolcreatedb,'createrole',rolcreaterole,'bypassrls',rolbypassrls,
        'delete_vehicle',has_table_privilege(current_user,'panda.vehicles','DELETE'),
        'truncate_charge',has_table_privilege(current_user,'panda.charge_sessions','TRUNCATE'),
        'migration0031',EXISTS(SELECT 1 FROM public._sqlx_migrations WHERE version=1000031 AND success),
        'charge_checkpoint',to_regclass('panda.active_charge_checkpoints') IS NOT NULL,
        'charge_outbox',to_regclass('panda.pending_charge_completions') IS NOT NULL)
        FROM pg_roles WHERE rolname=current_user").fetch_one(pool).await.unwrap();
    assert_eq!(protection["migration0031"], true);
    assert_eq!(protection["charge_checkpoint"], true);
    assert_eq!(protection["charge_outbox"], true);
    rows.push(json!({"case":"protection","output":protection}));
    rows.push(
        json!({"case":"empty-renewal","output":connections::rivian_status(pool).await.unwrap()}),
    );
    for (vehicle, date, health, auth) in [
        (10, "2026-01-01T00:00:00Z", "degraded", "authenticated"),
        (11, "2026-02-01T00:00:00Z", "error", "needs_reauth"),
    ] {
        sqlx::query("INSERT INTO panda.vehicle_credentials (vehicle_id,encrypted_tokens,token_created_at) VALUES ($1,decode('000102','hex'),$2)")
            .bind(id(vehicle)).bind(date.parse::<DateTime<Utc>>().unwrap()).execute(pool).await.unwrap();
        sqlx::query("INSERT INTO panda.vehicle_runtime_state (vehicle_id,worker_health,worker_health_msg,auth_state,updated_at) VALUES ($1,$2,$3,$4,'2026-01-01Z')")
            .bind(id(vehicle)).bind(health).bind(format!("fixture {health}")).bind(auth).execute(pool).await.unwrap();
    }
    rows.push(json!({"case":"renewal-auth-override","output":connections::rivian_status(pool).await.unwrap(),"rows":f.rows().await}));
    for health in [
        "error",
        "degraded",
        "stale",
        "starting",
        "connected",
        "unknown",
    ] {
        f.execute("UPDATE panda.vehicle_runtime_state SET auth_state='authenticated'")
            .await;
        sqlx::query("UPDATE panda.vehicle_runtime_state SET worker_health=$1,worker_health_msg=$1 WHERE vehicle_id=$2").bind(health).bind(id(11)).execute(pool).await.unwrap();
        rows.push(json!({"case":format!("health-{health}"),"output":connections::rivian_status(pool).await.unwrap()}));
    }
    for operation in [
        "failure",
        "success",
        "test-failure",
        "test-success",
        "attempt",
        "attempt",
    ] {
        match operation {
            "failure" => {
                connections::record_failure(
                    pool,
                    "open_meteo",
                    "synthetic request?private=query\nsecond",
                )
                .await
            }
            "success" => connections::record_success(pool, "open_meteo").await,
            "test-failure" => {
                connections::record_test(
                    pool,
                    "open_meteo",
                    false,
                    Some("synthetic test?hidden\nsecond"),
                )
                .await
            }
            "test-success" => connections::record_test(pool, "open_meteo", true, None).await,
            _ => connections::record_attempt(pool, "open_meteo").await,
        }
        rows.push(json!({"case":operation,"rows":f.rows().await}));
    }
    f.execute("UPDATE panda.external_connection_activity SET usage_date='2026-01-01',request_count=199,last_test_checks='[{\"fixture\":true}]' WHERE connection_id='open_meteo'").await;
    connections::record_attempt(pool, "open_meteo").await;
    rows.push(json!({"case":"day-rollover","rows":f.rows().await}));
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..24 {
        let pool = pool.clone();
        tasks.spawn(async move { connections::record_attempt(&pool, "open_meteo").await });
    }
    while let Some(result) = tasks.join_next().await {
        result.unwrap();
    }
    let count: i32 = sqlx::query_scalar("SELECT request_count FROM panda.external_connection_activity WHERE connection_id='open_meteo'").fetch_one(pool).await.unwrap();
    assert_eq!(count, 25);
    rows.push(json!({"case":"concurrent-attempts","rows":f.rows().await}));
    f.execute("UPDATE panda.external_connection_activity SET request_count=2147483647 WHERE connection_id='open_meteo'").await;
    let before_overflow = f.rows().await;
    connections::record_attempt(pool, "open_meteo").await;
    assert_eq!(
        before_overflow,
        f.rows().await,
        "overflow rolls back the complete activity statement"
    );
    rows.push(json!({"case":"counter-overflow","rows":f.rows().await}));
    f.execute("UPDATE panda.external_connection_activity SET request_count=25 WHERE connection_id='open_meteo';
        REVOKE UPDATE ON panda.external_connection_activity FROM CURRENT_USER;
        GRANT UPDATE (last_attempt_at,usage_date,request_count) ON panda.external_connection_activity TO CURRENT_USER").await;
    connections::record_attempt(pool, "open_meteo").await;
    let before_success = f.rows().await;
    connections::record_success(pool, "open_meteo").await;
    assert_eq!(
        before_success,
        f.rows().await,
        "success needs only its own update columns"
    );
    rows.push(json!({"case":"column-scoped-attempt","rows":f.rows().await}));
    f.execute("REVOKE UPDATE (last_attempt_at,usage_date,request_count) ON panda.external_connection_activity FROM CURRENT_USER;
        GRANT UPDATE ON panda.external_connection_activity TO CURRENT_USER").await;
    f.execute("REVOKE UPDATE ON panda.external_connection_activity FROM CURRENT_USER")
        .await;
    let before = f.rows().await;
    connections::record_attempt(pool, "open_meteo").await;
    connections::record_failure(pool, "open_meteo", "lost update").await;
    assert_eq!(
        before,
        f.rows().await,
        "best-effort writes fail without partial state"
    );
    f.execute("GRANT UPDATE ON panda.external_connection_activity TO CURRENT_USER")
        .await;
    connections::record_attempt(pool, "open_meteo").await;
    rows.push(json!({"case":"activity-failure-retry","rows":f.rows().await}));
    for (target, operation) in [
        ("basemap", "success"),
        ("nominatim", "test"),
        ("iconify", "attempt"),
    ] {
        match operation {
            "success" => connections::record_success(pool, target).await,
            "test" => connections::record_test(pool, target, false, Some("first test")).await,
            _ => connections::record_attempt(pool, target).await,
        }
    }
    connections::record_attempt(pool, "missing").await;
    rows.push(json!({"case":"activity-first-inserts-and-missing-fk","rows":f.rows().await}));
    for message in ["", "\nsecond line", "unicode 🦀?discard", &"é".repeat(300)] {
        connections::record_failure(pool, "open_meteo", message).await;
        rows.push(json!({"case":"sanitized-errors","rows":f.rows().await}));
    }
    f.execute(
        "DELETE FROM panda.external_connection_activity WHERE connection_id='iconify';
        DELETE FROM panda.external_connection_settings WHERE id='iconify'",
    )
    .await;
    connections::ensure_defaults(pool).await.unwrap();
    rows.push(json!({"case":"missing-default-restored","rows":f.rows().await}));
    for n in [30, 31] {
        sqlx::query("INSERT INTO panda.addresses (id,display_name,latitude,longitude,created_at) VALUES ($1,'synthetic address',0,0,'2026-01-01Z')")
            .bind(id(n)).execute(pool).await.unwrap();
    }
    sqlx::query("INSERT INTO panda.geofences (id,user_id,name,latitude,longitude,address_id,created_at,updated_at) VALUES ($1,$2,'synthetic place',0,0,$3,'2026-01-01Z','2026-01-01Z')")
        .bind(id(32)).bind(id(3)).bind(id(30)).execute(pool).await.unwrap();
    let mut redis = f
        .state
        .redis
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    for (key, value) in [
        ("external:basemap:fixture", "abc"),
        ("external:basemap:fixture:content-type", "image/png"),
        ("external:basemap:fixture:max-age", "12"),
        ("external:nominatim:search:fixture", "result"),
        ("external:unrelated:fixture", "untouched"),
    ] {
        let _: () = redis.set(key, value).await.unwrap();
    }
    let mut pipe = redis::pipe();
    for n in 0..1100 {
        pipe.cmd("SET")
            .arg(format!("external:basemap:page:{n}"))
            .arg("x")
            .ignore();
    }
    let _: () = pipe.query_async(&mut redis).await.unwrap();
    rows.push(json!({"case":"cache-pages","output":cache_summary(&f.state,"basemap").await}));
    observe(
        &f,
        &mut rows,
        "purge-map-pages",
        Some(1),
        "POST",
        &format!("{SETTINGS}/basemap/cache/purge"),
        json!({}),
    )
    .await;
    observe(
        &f,
        &mut rows,
        "purge-map-idempotent",
        Some(1),
        "POST",
        &format!("{SETTINGS}/basemap/cache/purge"),
        json!({}),
    )
    .await;
    observe(
        &f,
        &mut rows,
        "purge-address-cache",
        Some(1),
        "POST",
        &format!("{SETTINGS}/nominatim/cache/purge"),
        json!({}),
    )
    .await;
    let addresses: Vec<uuid::Uuid> =
        sqlx::query_scalar("SELECT id FROM panda.addresses ORDER BY id")
            .fetch_all(pool)
            .await
            .unwrap();
    assert_eq!(addresses, vec![id(30)]);
    observe(
        &f,
        &mut rows,
        "purge-unsupported",
        Some(1),
        "POST",
        &format!("{SETTINGS}/open_meteo/cache/purge"),
        json!({}),
    )
    .await;
    let unrelated: String = redis.get("external:unrelated:fixture").await.unwrap();
    assert_eq!(unrelated, "untouched");
    let _: () = redis.del("external:unrelated:fixture").await.unwrap();
    let raw = json!(rows);
    f.cleanup().await;
    freeze("activity", &raw, include_str!("activity.json"));
}

#[tokio::test]
async fn external_workflow_preference_and_validation_contract() {
    let issued: DateTime<Utc> = "2026-01-01T00:00:00Z".parse().unwrap();
    let mut rows = vec![];
    for nanos in [-1, 0, 1] {
        for days in [0, 173, 180] {
            for auth in [None, Some("needs_reauth"), Some("error")] {
                let now =
                    issued + chrono::Duration::days(days) + chrono::Duration::nanoseconds(nanos);
                rows.push(
                    json!({"case":"renewal","days":days,"nanos":nanos,"auth":auth,
                    "output":connections::rivian_renewal_state_for_auth(issued,now,auth)}),
                );
            }
        }
    }
    for enabled in [false, true] {
        for mode in ["disabled", "custom", "remote", "hosted"] {
            for stored in [false, true] {
                for preference in [
                    None,
                    Some("auto"),
                    Some("carto"),
                    Some("openfreemap"),
                    Some("invalid"),
                ] {
                    for submitted in [None, Some(""), Some(" "), Some("synthetic")] {
                        for clear in [false, true] {
                            let settings =
                                super::tests::basemap_settings(mode, "auto", stored, enabled);
                            let body = UpdateConnectionBody {
                                enabled,
                                mode: mode.into(),
                                basemap_provider: preference.map(str::to_owned),
                                api_key: submitted.map(str::to_owned),
                                clear_api_key: Some(clear),
                                ..Default::default()
                            };
                            let projected =
                                resolve_basemap_provider_update("basemap", &body, Some(&settings))
                                    .map_err(|e| e.to_string());
                            let effective = resolve_effective_basemap_provider_update(
                                "basemap", &body, &settings,
                            )
                            .map_err(|e| e.to_string());
                            rows.push(json!({"case":"preference","enabled":enabled,"mode":mode,"stored":stored,
                                "preference":preference,"submitted":submitted.map(|s| if s.trim().is_empty() {"blank"} else {"present"}),
                                "clear":clear,"projected":projected,"effective":effective,
                                "resolved":resolve_basemap_provider(&settings)}));
                        }
                    }
                }
            }
        }
    }
    for body in [
        json!({"enabled":false,"mode":"wrong","weather_precision":"wrong","private_network_allowlist":["wrong"]}),
        json!({"enabled":false,"mode":"custom","weather_precision":"wrong"}),
        json!({"enabled":false,"mode":"disabled","private_network_allowlist":["0.0.0.0/0"]}),
        json!({"enabled":true,"mode":"custom"}),
        json!({"enabled":true,"mode":"remote"}),
        json!({"enabled":true,"mode":"remote","weather_precision":"exact"}),
    ] {
        for target in ["open_meteo", "basemap", "nominatim", "iconify", "unknown"] {
            let result = validate_update(target, &serde_json::from_value(body.clone()).unwrap())
                .await
                .map_err(|e| e.to_string());
            rows.push(json!({"case":"validation","target":target,"body":body,"output":result}));
        }
    }
    freeze("pure", &json!(rows), include_str!("pure.json"));
}
