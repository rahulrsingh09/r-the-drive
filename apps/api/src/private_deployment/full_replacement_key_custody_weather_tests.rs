use super::*;

#[test]
fn full_replacement_key_custody_weather_update_policy_matrix() {
    for mode in ["remote", "hosted", "disabled"] {
        assert!(update_allowed(mode, None, None, None, false, false));
        assert!(update_allowed(
            mode,
            Some("approximate"),
            Some(FORECAST_URL),
            Some(ARCHIVE_URL),
            false,
            false
        ));
        for origin in [
            "http://127.0.0.1/weather",
            "https://10.0.0.1/weather",
            "http://169.254.169.254",
            "https://api.open-meteo.com@evil.invalid/v1/forecast",
            "https://api.open-meteo.com/v1/forecast?token=secret",
        ] {
            assert!(!update_allowed(
                mode,
                None,
                Some(origin),
                None,
                false,
                false
            ));
            assert!(!update_allowed(
                mode,
                None,
                None,
                Some(origin),
                false,
                false
            ));
        }
        assert!(!update_allowed(
            mode,
            Some("exact"),
            None,
            None,
            false,
            false
        ));
        assert!(!update_allowed(mode, None, None, None, true, false));
        assert!(!update_allowed(mode, None, None, None, false, true));
    }
    assert!(!update_allowed("local", None, None, None, false, false));
}

#[tokio::test]
#[ignore = "requires disposable TimescaleDB DATABASE_URL and REDIS_URL"]
async fn full_replacement_key_custody_weather_recovery_and_settings_guards() {
    let fixture = crate::authorization_test_support::Fixture::new().await;
    let pool = &fixture.state.pool;
    connections::ensure_defaults(pool).await.unwrap();
    let mut settings = connections::load(pool, connections::OPEN_METEO)
        .await
        .unwrap();
    settings.mode = "remote".into();
    assert!(settings_allowed(&settings));
    for case in 0..7 {
        let mut forbidden = settings.clone();
        match case {
            0 => forbidden.allow_private_network = true,
            1 => forbidden.private_network_allowlist = vec!["10.0.0.0/8".into()],
            2 => forbidden.private_network_policy_state = "migration_required".into(),
            3 => forbidden.api_key_encrypted = Some(vec![1]),
            4 => forbidden.bearer_token_encrypted = Some(vec![1]),
            5 => forbidden.forecast_url = Some("http://127.0.0.1".into()),
            _ => forbidden.archive_url = Some("http://169.254.169.254".into()),
        }
        assert!(!settings_allowed(&forbidden), "{case}");
    }
    let stale = uuid::Uuid::new_v4();
    let fresh = uuid::Uuid::new_v4();
    let pending = uuid::Uuid::new_v4();
    for (id, status, age, attempts) in [
        (stale, "running", 20, 2),
        (fresh, "running", 1, 0),
        (pending, "pending", 20, 3),
    ] {
        sqlx::query("INSERT INTO panda.weather_enrichment_jobs(trip_id,status,attempts,updated_at,last_error) VALUES($1,$2,$3,now()-make_interval(mins=>$4),'synthetic-failure')")
            .bind(id).bind(status).bind(attempts).bind(age).execute(pool).await.unwrap();
    }
    for _ in 0..2 {
        recover_interrupted_jobs(pool).await.unwrap();
        for (id, expected_status, expected_attempts, expected_error) in [
            (stale, "pending", 1, None),
            (fresh, "running", 0, Some("synthetic-failure")),
            (pending, "pending", 3, Some("synthetic-failure")),
        ] {
            let (status, attempts, error): (String, i32, Option<String>) = sqlx::query_as("SELECT status,attempts,last_error FROM panda.weather_enrichment_jobs WHERE trip_id=$1")
                .bind(id).fetch_one(pool).await.unwrap();
            assert_eq!(
                (status.as_str(), attempts, error.as_deref()),
                (expected_status, expected_attempts, expected_error)
            );
        }
    }
    fixture.cleanup().await;
}
