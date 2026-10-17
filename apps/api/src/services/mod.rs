pub(crate) mod admitted_read;
pub mod app_settings;
pub mod artifact_files;
pub mod authentication_settings;
pub mod backup_catalog;
pub mod backups;
pub mod basemap_cache;
pub mod charge_backfill;
pub mod charge_payload_identity;
pub mod charge_session_repair;
pub mod charge_sessions;
pub mod charging;
pub mod chart_registry;
pub mod chart_validation;
pub mod cost;
pub mod demo_seed;
pub mod external_connections;
pub mod geofences;
pub mod ingestion_capture;
pub mod nominatim;
pub mod oidc;
pub mod outbound;
pub mod outbound_policy;
pub mod recovery_extract;
pub(crate) mod recovery_workspace;
pub mod redis_health;
pub mod resource_limits;
pub mod restore_compatibility;
mod restore_dump_inspection;
pub mod restore_history;
pub mod restore_jobs;
pub mod restore_policy;
pub mod s3_backups;
pub mod s3_transport;
pub mod security_audit;
pub mod sessions;
pub mod trip_enrichment;
pub mod trip_routes;
pub mod weather_enrichment;
#[cfg(test)]
mod domain_replacement_contracts {
    use super::*;
    use crate::{config::SecurityConfig, errors::AppError};
    use std::sync::Arc;
    use uuid::Uuid;

    #[test]
    fn charging_preserves_empty_reverse_intervals_and_thresholds() {
        assert_eq!(charging::compute_energy_used_wh(&[]), 0.0);
        assert_eq!(charging::compute_energy_used_wh(&[(1.0, 100.0)]), 0.0);
        assert_eq!(
            charging::compute_energy_used_wh(&[(3600.0, -2.0), (0.0, 999.0)]),
            -2000.0
        );
        for (power, kind) in [
            (11.999, "ac"),
            (12.0, "ac_l2"),
            (49.999, "ac_l2"),
            (50.0, "dc"),
            (f64::NAN, "dc"),
        ] {
            assert_eq!(charging::classify_charger_type(power), kind);
        }
    }

    #[test]
    fn admission_zero_limits_and_independent_live_heavy_buckets() {
        let limits = Arc::new(resource_limits::ResourceLimits::default());
        let user = Uuid::nil();
        let config = SecurityConfig {
            metrics_max_per_user: 1,
            metrics_max_global: 1,
            ws_max_per_user: 1,
            ws_max_per_vehicle: 1,
            ws_max_global: 1,
            ..Default::default()
        };
        let heavy = limits.heavy(user, &config).unwrap();
        let live = limits.live(user, Uuid::nil(), &config).unwrap();
        assert!(matches!(
            limits.heavy(Uuid::new_v4(), &config),
            Err(AppError::ResourceLimited("heavy_read"))
        ));
        assert!(matches!(
            limits.live(Uuid::new_v4(), Uuid::new_v4(), &config),
            Err(AppError::ResourceLimited("live_connections"))
        ));
        drop((heavy, live));
        assert!(limits.heavy(user, &config).is_ok());
        let config = SecurityConfig {
            metrics_max_global: 0,
            ws_max_per_vehicle: 0,
            ..config
        };
        assert!(limits.heavy(user, &config).is_err());
        assert!(limits.live(user, Uuid::nil(), &config).is_err());
    }

    #[tokio::test]
    #[ignore = "requires dedicated JOBS_DATABASE_URL"]
    async fn admitted_read_cancellation_deadline_and_commit_keep_the_permit() {
        let fixture = weather_enrichment::job_fixture::Fixture::new().await;
        let pool = fixture.state.pool.clone();
        let limits = Arc::new(resource_limits::ResourceLimits::default());
        let config = SecurityConfig {
            metrics_max_per_user: 1,
            metrics_max_global: 1,
            ..Default::default()
        };
        let user = Uuid::nil();
        let permit = limits.heavy(user, &config).unwrap();
        let mut read = admitted_read::AdmittedRead::begin(&pool, permit, 30)
            .await
            .unwrap();
        let settings: (String, String, String) = sqlx::query_as(
            "SELECT current_setting('transaction_isolation'), current_setting('transaction_read_only'), current_setting('statement_timeout')"
        ).fetch_one(&mut *read).await.unwrap();
        assert_eq!(
            settings,
            ("repeatable read".into(), "on".into(), "30s".into())
        );
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *read)
            .await
            .unwrap();
        let query =
            tokio::spawn(
                async move { sqlx::query("SELECT pg_sleep(30)").execute(&mut *read).await },
            );
        for attempt in 0..100 {
            let sleeping: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1 AND wait_event='PgSleep')")
                .bind(pid).fetch_one(&pool).await.unwrap();
            if sleeping {
                break;
            }
            assert!(attempt < 99, "read did not reach the server");
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(limits.heavy(user, &config).is_err());
        query.abort();
        assert!(query.await.unwrap_err().is_cancelled());
        let permit = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if let Ok(permit) = limits.heavy(user, &config) {
                    break permit;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("cancellation must drain and release admission");
        let sleeping: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1 AND wait_event='PgSleep')",
        )
        .bind(pid)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(!sleeping, "permit released before the server query ended");
        let mut read = admitted_read::AdmittedRead::begin(&pool, permit, 1)
            .await
            .unwrap();
        let error = sqlx::query("SELECT pg_sleep(3)")
            .execute(&mut *read)
            .await
            .unwrap_err();
        assert_eq!(
            error.as_database_error().unwrap().code().as_deref(),
            Some("57014")
        );
        drop(read);
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if let Ok(permit) = limits.heavy(user, &config) {
                    admitted_read::AdmittedRead::begin(&pool, permit, 1)
                        .await
                        .unwrap()
                        .commit()
                        .await
                        .unwrap();
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(limits.heavy(user, &config).is_ok());
        drop(pool);
        fixture.cleanup().await;
    }

    #[tokio::test]
    #[ignore = "requires dedicated JOBS_DATABASE_URL"]
    async fn timezone_updates_roll_back_when_backup_settings_reject_the_write() {
        use sqlx::Executor;
        let fixture = weather_enrichment::job_fixture::Fixture::new().await;
        let pool = &fixture.state.pool;
        app_settings::set_app_timezone(pool, chrono_tz::America::Los_Angeles)
            .await
            .unwrap();
        assert_eq!(
            app_settings::load_app_timezone(pool).await.unwrap(),
            chrono_tz::America::Los_Angeles
        );
        sqlx::query("INSERT INTO panda.backup_settings(id) VALUES(true) ON CONFLICT DO NOTHING")
            .execute(pool)
            .await
            .unwrap();
        pool.execute("CREATE FUNCTION panda.reject_domain_timezone() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'synthetic timezone rejection'; END $$;
            CREATE TRIGGER reject_domain_timezone BEFORE UPDATE ON panda.backup_settings FOR EACH ROW EXECUTE FUNCTION panda.reject_domain_timezone()")
            .await.unwrap();
        let error = app_settings::set_app_timezone(pool, chrono_tz::UTC)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("synthetic timezone rejection"));
        assert_eq!(
            app_settings::load_app_timezone_name(pool).await.unwrap(),
            "America/Los_Angeles"
        );
        fixture.cleanup().await;
    }

    #[tokio::test]
    #[ignore = "requires dedicated no-egress Docker HTTP fixture"]
    async fn pinned_http_limits_redirects_json_and_address_denials() {
        use axum::{routing::get, Router};
        use std::future::IntoFuture;
        let host = std::env::var("DOMAIN_HTTP_HOST").unwrap();
        let cidr = std::env::var("DOMAIN_HTTP_CIDR").unwrap().parse().unwrap();
        let listener = tokio::net::TcpListener::bind("0.0.0.0:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(
            axum::serve(
                listener,
                Router::new()
                    .route(
                        "/json",
                        get(|| async { axum::Json(serde_json::json!({"fixture":true})) }),
                    )
                    .route("/bad", get(|| async { "not-json" }))
                    .route("/large", get(|| async { "0123456789" }))
                    .route(
                        "/redirect",
                        get(|| async { axum::response::Redirect::temporary("/json") }),
                    ),
            )
            .into_future(),
        );
        let base = url::Url::parse(&format!("http://{host}:{port}/json")).unwrap();
        assert!(outbound::outbound_client_for_url(&base, &[]).await.is_err());
        let client = outbound::outbound_client_for_url(&base, &[cidr])
            .await
            .unwrap();
        let response = client.get(base.clone()).send().await.unwrap();
        let body: serde_json::Value = outbound::read_json(response, 100, "fixture").await.unwrap();
        assert_eq!(body, serde_json::json!({"fixture":true}));
        let response = client
            .get(base.join("large").unwrap())
            .send()
            .await
            .unwrap();
        assert!(
            matches!(outbound::read_response_limited(response, 9, "fixture").await, Err(AppError::DependencyUnavailable(message)) if message=="fixture response exceeded the limit")
        );
        let response = client.get(base.join("bad").unwrap()).send().await.unwrap();
        assert!(
            matches!(outbound::read_json::<serde_json::Value>(response, 100, "fixture").await, Err(AppError::DependencyUnavailable(message)) if message=="fixture returned invalid JSON")
        );
        let response = client
            .get(base.join("redirect").unwrap())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::TEMPORARY_REDIRECT);
        for raw in [
            "http://127.0.0.1/",
            "http://169.254.169.254/",
            "https://metadata.google.internal/",
            "https://user:password@example.invalid/",
            "ftp://example.invalid/",
        ] {
            assert!(
                outbound::outbound_client_for_url(&url::Url::parse(raw).unwrap(), &[cidr])
                    .await
                    .is_err()
            );
        }
        server.abort();
        assert!(server.await.unwrap_err().is_cancelled());
    }

    #[tokio::test]
    #[ignore = "requires dedicated JOBS_DATABASE_URL"]
    async fn timezone_source_precedence_preserves_blank_and_invalid_values() {
        let fixture = weather_enrichment::job_fixture::Fixture::new().await;
        let pool = &fixture.state.pool;
        sqlx::query("INSERT INTO panda.backup_settings(id,timezone) VALUES(true,'Europe/Paris') ON CONFLICT(id) DO UPDATE SET timezone=EXCLUDED.timezone")
            .execute(pool).await.unwrap();
        for (value, expected) in [
            ("  ", Some("Europe/Paris")),
            ("UTC", Some("UTC")),
            ("not-a-zone", None),
        ] {
            sqlx::query("INSERT INTO panda.system_config(key,value) VALUES($1,$2) ON CONFLICT(key) DO UPDATE SET value=EXCLUDED.value")
                .bind(app_settings::APP_TIMEZONE_KEY).bind(value).execute(pool).await.unwrap();
            match expected {
                Some(name) => assert_eq!(
                    app_settings::load_app_timezone_name(pool).await.unwrap(),
                    name
                ),
                None => assert!(matches!(
                    app_settings::load_app_timezone_name(pool).await,
                    Err(AppError::Validation(message)) if message == "timezone must be a valid IANA timezone"
                )),
            }
        }
        fixture.cleanup().await;
    }
}
