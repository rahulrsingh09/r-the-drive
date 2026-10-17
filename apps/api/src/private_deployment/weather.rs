//! The only optional backend egress: bounded, anonymous, coarse weather estimates.

use anyhow::{bail, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;

#[cfg(test)]
#[path = "full_replacement_key_custody_weather_tests.rs"]
mod full_replacement_key_custody_tests;

use crate::services::{external_connections as connections, outbound};

pub const FORECAST_URL: &str = "https://api.open-meteo.com/v1/forecast";
pub const ARCHIVE_URL: &str = "https://archive-api.open-meteo.com/v1/archive";
pub const DAILY_REQUEST_BUDGET: i32 = 200;
const RESPONSE_LIMIT: usize = 64 * 1024;

#[derive(Debug, thiserror::Error)]
#[error("Daily weather request budget reached")]
pub struct WeatherBudgetError;

#[derive(Debug, thiserror::Error)]
#[error("Weather connection disabled")]
pub struct WeatherPausedError;

#[derive(Debug, thiserror::Error)]
#[error("Weather requests are busy")]
pub struct WeatherBusyError;

#[derive(Debug, thiserror::Error)]
#[error("Weather provider returned HTTP {status}")]
pub struct WeatherFetchError {
    pub status: reqwest::StatusCode,
    pub retry_after_seconds: Option<i64>,
}

pub fn settings_allowed(settings: &connections::ConnectionSettingsRow) -> bool {
    let credentials =
        settings.api_key_encrypted.is_some() || settings.bearer_token_encrypted.is_some();
    settings.mode == "remote"
        && settings.weather_precision.is_some()
        && settings.forecast_url.is_some()
        && settings.archive_url.is_some()
        && settings.private_network_allowlist.is_empty()
        && settings.private_network_policy_state == "restricted"
        && update_allowed(
            &settings.mode,
            settings.weather_precision.as_deref(),
            settings.forecast_url.as_deref(),
            settings.archive_url.as_deref(),
            credentials,
            settings.allow_private_network,
        )
}

pub fn update_allowed(
    mode: &str,
    precision: Option<&str>,
    forecast: Option<&str>,
    archive: Option<&str>,
    has_credentials: bool,
    private_network: bool,
) -> bool {
    if has_credentials || private_network || !["remote", "hosted", "disabled"].contains(&mode) {
        return false;
    }
    [
        (precision, "approximate"),
        (forecast, FORECAST_URL),
        (archive, ARCHIVE_URL),
    ]
    .into_iter()
    .all(|(configured, required)| configured.is_none_or(|value| value == required))
}

fn request_url(
    lat: f64,
    lng: f64,
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
) -> Result<url::Url> {
    let now = Utc::now();
    let duration = ended_at.signed_duration_since(started_at);
    let coordinates_valid = [(lat, 90.0), (lng, 180.0)]
        .into_iter()
        .all(|(value, bound)| value.is_finite() && (-bound..=bound).contains(&value));
    if !coordinates_valid
        || duration < chrono::Duration::zero()
        || duration > chrono::Duration::days(1)
        || ended_at > now
    {
        bail!("Invalid weather sample bounds");
    }
    let endpoint = match now.signed_duration_since(ended_at) < chrono::Duration::days(5) {
        true => FORECAST_URL,
        false => ARCHIVE_URL,
    };
    let mut url = url::Url::parse(endpoint)?;
    let pairs = [
        ("latitude", format!("{:.2}", (lat * 100.0).round() / 100.0)),
        ("longitude", format!("{:.2}", (lng * 100.0).round() / 100.0)),
        ("hourly", "temperature_2m".to_owned()),
        ("timezone", "UTC".to_owned()),
        ("temperature_unit", "celsius".to_owned()),
        ("start_date", started_at.date_naive().to_string()),
        ("end_date", ended_at.date_naive().to_string()),
    ];
    url.query_pairs_mut().extend_pairs(pairs);
    Ok(url)
}

#[derive(Debug, PartialEq)]
enum Admission {
    Ready,
    Wait,
    Budget,
}

async fn reserve(pool: &PgPool) -> Result<Admission> {
    let count = sqlx::query_scalar::<_, i32>(
        "WITH instant AS MATERIALIZED (SELECT clock_timestamp() AS ts)
         INSERT INTO panda.external_connection_activity
           (connection_id, last_attempt_at, usage_date, request_count)
         SELECT 'open_meteo', ts, (ts AT TIME ZONE 'UTC')::date, 1 FROM instant
         ON CONFLICT (connection_id) DO UPDATE SET
           last_attempt_at = EXCLUDED.last_attempt_at,
           usage_date = EXCLUDED.usage_date,
           request_count = CASE WHEN external_connection_activity.usage_date = EXCLUDED.usage_date
             THEN external_connection_activity.request_count + 1 ELSE 1 END
         WHERE (external_connection_activity.usage_date <> EXCLUDED.usage_date
                OR external_connection_activity.request_count < $1)
           AND (external_connection_activity.last_attempt_at IS NULL
                OR external_connection_activity.last_attempt_at <= EXCLUDED.last_attempt_at - interval '5 seconds')
         RETURNING request_count"
    ).bind(DAILY_REQUEST_BUDGET).fetch_optional(pool).await?;
    match count {
        Some(_) => Ok(Admission::Ready),
        None => {
            let exhausted: bool = sqlx::query_scalar(
                "SELECT usage_date = (clock_timestamp() AT TIME ZONE 'UTC')::date AND request_count >= $1
                 FROM panda.external_connection_activity WHERE connection_id = 'open_meteo'"
            ).bind(DAILY_REQUEST_BUDGET).fetch_one(pool).await?;
            Ok(match exhausted {
                true => Admission::Budget,
                false => Admission::Wait,
            })
        }
    }
}

pub async fn fetch(
    pool: &PgPool,
    lat: f64,
    lng: f64,
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
) -> Result<Value> {
    let url = request_url(lat, lng, started_at, ended_at)?;
    for attempt in 0..6 {
        connections::require_enabled(pool, connections::OPEN_METEO)
            .await
            .map_err(|error| match error {
                crate::errors::AppError::ExternalConnectionDisabled(_) => {
                    anyhow::Error::from(WeatherPausedError)
                }
                other => other.into(),
            })?;
        // Resolve before admission so DNS latency cannot bunch admitted requests.
        let client = outbound::outbound_client_for_url(&url, &[]).await?;
        match reserve(pool).await? {
            Admission::Budget => return Err(WeatherBudgetError.into()),
            Admission::Wait => {
                if attempt == 5 {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                continue;
            }
            Admission::Ready => {}
        }
        let response = client
            .get(url)
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("Weather connection failed"))?;
        match response.status().is_success() {
            false => {
                let retry_after_seconds = response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse::<i64>().ok());
                return Err(WeatherFetchError {
                    status: response.status(),
                    retry_after_seconds,
                }
                .into());
            }
            true => {
                let body =
                    outbound::read_json(response, RESPONSE_LIMIT, "Weather provider").await?;
                validate_payload(&body, started_at, ended_at)?;
                return Ok(body);
            }
        }
    }
    Err(WeatherBusyError.into())
}

pub async fn recover_interrupted_jobs(pool: &PgPool) -> Result<()> {
    sqlx::query(
        "WITH abandoned AS MATERIALIZED (
           SELECT trip_id FROM panda.weather_enrichment_jobs
           WHERE status='running' AND updated_at < now()-interval '15 minutes' FOR UPDATE
         )
         UPDATE panda.weather_enrichment_jobs job SET status='pending',
           attempts=GREATEST(job.attempts-1,0), next_attempt_at=now(), last_error=NULL, updated_at=now()
         FROM abandoned WHERE job.trip_id=abandoned.trip_id"
    ).execute(pool).await?;
    Ok(())
}

fn validate_payload(body: &Value, start: DateTime<Utc>, end: DateTime<Utc>) -> Result<()> {
    let arrays = ["/hourly/time", "/hourly/temperature_2m"]
        .map(|pointer| body.pointer(pointer).and_then(Value::as_array));
    let [Some(times), Some(temperatures)] = arrays else {
        bail!("Invalid weather response");
    };
    if !(1..=48).contains(&times.len()) || times.len() != temperatures.len() {
        bail!("Invalid weather response size");
    }
    times
        .iter()
        .zip(temperatures)
        .try_fold(None, |previous, (time, temperature)| {
            let timestamp = time
                .as_str()
                .and_then(|value| {
                    chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M").ok()
                })
                .ok_or_else(|| anyhow::anyhow!("Invalid weather timestamp"))?;
            let valid_temperature = temperature.is_null()
                || temperature
                    .as_f64()
                    .is_some_and(|value| value.is_finite() && (-100.0..=70.0).contains(&value));
            let date_in_window =
                (start.date_naive()..=end.date_naive()).contains(&timestamp.date());
            if !date_in_window
                || !valid_temperature
                || previous.is_some_and(|previous| timestamp <= previous)
            {
                bail!("Invalid weather sample");
            }
            Ok(Some(timestamp))
        })
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn request_is_fixed_coarse_anonymous_and_bounded() {
        let end = Utc::now() - chrono::Duration::hours(1);
        let url = request_url(30.26721, -97.74311, end, end).unwrap();
        assert_eq!(url.host_str(), Some("api.open-meteo.com"));
        assert_eq!(url.path(), "/v1/forecast");
        assert_eq!(url.username(), "");
        let query = url
            .query_pairs()
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(query.len(), 7);
        assert_eq!(query["latitude"], "30.27");
        assert_eq!(query["longitude"], "-97.74");
        assert_eq!(query["hourly"], "temperature_2m");
        let old = end - chrono::Duration::days(10);
        assert_eq!(
            request_url(0., 0., old, old).unwrap().host_str(),
            Some("archive-api.open-meteo.com")
        );
        for (lat, lng) in [(f64::NAN, 0.), (0., f64::INFINITY), (91., 0.), (0., -181.)] {
            assert!(request_url(lat, lng, end, end).is_err());
        }
        assert!(request_url(0., 0., end - chrono::Duration::hours(25), end).is_err());
        assert!(request_url(0., 0., end, end - chrono::Duration::seconds(1)).is_err());
        assert!(request_url(0., 0., end, end + chrono::Duration::days(1)).is_err());
    }

    #[test]
    fn response_rejects_invalid_or_unrelated_estimates() {
        let start = "2026-01-01T12:00:00Z".parse().unwrap();
        let body = json!({"hourly":{"time":["2026-01-01T12:00"],"temperature_2m":[12.5]}});
        assert!(validate_payload(&body, start, start).is_ok());
        for temperatures in [json!([900]), json!(["secret"]), json!([])] {
            let mut bad = body.clone();
            bad["hourly"]["temperature_2m"] = temperatures;
            assert!(validate_payload(&bad, start, start).is_err());
        }
        let mut bad = body.clone();
        bad["hourly"]["time"] = json!(["2025-01-01T12:00"]);
        assert!(validate_payload(&bad, start, start).is_err());
        assert!(validate_payload(&json!([body]), start, start).is_err());
    }

    #[tokio::test]
    #[ignore = "requires disposable TimescaleDB DATABASE_URL and REDIS_URL"]
    async fn authorization_weather_budget_survives_concurrency_and_day_rollover() {
        let fixture = crate::authorization_test_support::Fixture::new().await;
        let pool = &fixture.state.pool;
        connections::ensure_defaults(pool).await.unwrap();
        let now = Utc::now();
        let disabled = fetch(pool, 39.0, -98.0, now, now).await.unwrap_err();
        assert!(disabled.downcast_ref::<WeatherPausedError>().is_some());
        let untouched: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM panda.external_connection_activity WHERE connection_id='open_meteo'",
        ).fetch_one(pool).await.unwrap();
        assert_eq!(untouched, 0);
        let mut jobs = tokio::task::JoinSet::new();
        for _ in 0..16 {
            let pool = pool.clone();
            jobs.spawn(async move { reserve(&pool).await.unwrap() });
        }
        let mut admitted = 0;
        while let Some(result) = jobs.join_next().await {
            admitted += usize::from(result.unwrap() == Admission::Ready);
        }
        assert_eq!(admitted, 1);
        sqlx::query("UPDATE panda.external_connection_activity SET request_count=$1, last_attempt_at=now()-interval '10 seconds' WHERE connection_id='open_meteo'")
            .bind(DAILY_REQUEST_BUDGET-1).execute(pool).await.unwrap();
        assert_eq!(reserve(pool).await.unwrap(), Admission::Ready);
        assert_eq!(reserve(&pool.clone()).await.unwrap(), Admission::Budget);
        sqlx::query("UPDATE panda.external_connection_activity SET usage_date=CURRENT_DATE-1, last_attempt_at=now()-interval '10 seconds' WHERE connection_id='open_meteo'")
            .execute(pool).await.unwrap();
        assert_eq!(reserve(pool).await.unwrap(), Admission::Ready);
        let count: i32 = sqlx::query_scalar("SELECT request_count FROM panda.external_connection_activity WHERE connection_id='open_meteo'").fetch_one(pool).await.unwrap();
        assert_eq!(count, 1);
        fixture.cleanup().await;
    }
}
