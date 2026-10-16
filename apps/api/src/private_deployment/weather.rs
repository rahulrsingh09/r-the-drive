//! The only optional backend egress: bounded, anonymous, coarse weather estimates.

use anyhow::{bail, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;

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
    settings.mode == "remote"
        && settings.weather_precision.as_deref() == Some("approximate")
        && settings.forecast_url.as_deref() == Some(FORECAST_URL)
        && settings.archive_url.as_deref() == Some(ARCHIVE_URL)
        && !settings.allow_private_network
        && settings.private_network_allowlist.is_empty()
        && settings.private_network_policy_state == "restricted"
        && settings.api_key_encrypted.is_none()
        && settings.bearer_token_encrypted.is_none()
}

pub fn update_allowed(
    mode: &str,
    precision: Option<&str>,
    forecast: Option<&str>,
    archive: Option<&str>,
    has_credentials: bool,
    private_network: bool,
) -> bool {
    matches!(mode, "remote" | "hosted" | "disabled")
        && precision.is_none_or(|value| value == "approximate")
        && forecast.is_none_or(|value| value == FORECAST_URL)
        && archive.is_none_or(|value| value == ARCHIVE_URL)
        && !has_credentials
        && !private_network
}

fn request_url(
    lat: f64,
    lng: f64,
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
) -> Result<url::Url> {
    if !lat.is_finite()
        || !lng.is_finite()
        || !(-90.0..=90.0).contains(&lat)
        || !(-180.0..=180.0).contains(&lng)
        || ended_at < started_at
        || ended_at - started_at > chrono::Duration::days(1)
        || ended_at > Utc::now()
    {
        bail!("Invalid weather sample bounds");
    }
    let endpoint = if Utc::now() - ended_at < chrono::Duration::days(5) {
        FORECAST_URL
    } else {
        ARCHIVE_URL
    };
    let mut url = url::Url::parse(endpoint)?;
    url.query_pairs_mut()
        .append_pair("latitude", &format!("{:.2}", (lat * 100.0).round() / 100.0))
        .append_pair(
            "longitude",
            &format!("{:.2}", (lng * 100.0).round() / 100.0),
        )
        .append_pair("hourly", "temperature_2m")
        .append_pair("timezone", "UTC")
        .append_pair("temperature_unit", "celsius")
        .append_pair("start_date", &started_at.date_naive().to_string())
        .append_pair("end_date", &ended_at.date_naive().to_string());
    Ok(url)
}

#[derive(Debug, PartialEq)]
enum Admission {
    Ready,
    Wait,
    Budget,
}

async fn reserve(pool: &PgPool) -> Result<Admission> {
    let admitted = sqlx::query_scalar::<_, bool>(
        r#"INSERT INTO panda.external_connection_activity
             (connection_id, last_attempt_at, usage_date, request_count)
           VALUES ('open_meteo', clock_timestamp(), (clock_timestamp() AT TIME ZONE 'UTC')::date, 1)
           ON CONFLICT (connection_id) DO UPDATE SET
             last_attempt_at = clock_timestamp(),
             usage_date = (clock_timestamp() AT TIME ZONE 'UTC')::date,
             request_count = CASE
               WHEN external_connection_activity.usage_date = (clock_timestamp() AT TIME ZONE 'UTC')::date
               THEN external_connection_activity.request_count + 1 ELSE 1 END
           WHERE (external_connection_activity.usage_date <> (clock_timestamp() AT TIME ZONE 'UTC')::date
                  OR external_connection_activity.request_count < $1)
             AND (external_connection_activity.last_attempt_at IS NULL
                  OR external_connection_activity.last_attempt_at <= clock_timestamp() - interval '5 seconds')
           RETURNING TRUE"#,
    )
    .bind(DAILY_REQUEST_BUDGET)
    .fetch_optional(pool)
    .await?;
    if admitted == Some(true) {
        return Ok(Admission::Ready);
    }
    let exhausted: bool = sqlx::query_scalar(
        "SELECT usage_date = (clock_timestamp() AT TIME ZONE 'UTC')::date AND request_count >= $1
         FROM panda.external_connection_activity WHERE connection_id = 'open_meteo'",
    )
    .bind(DAILY_REQUEST_BUDGET)
    .fetch_one(pool)
    .await?;
    Ok(if exhausted {
        Admission::Budget
    } else {
        Admission::Wait
    })
}

pub async fn fetch(
    pool: &PgPool,
    lat: f64,
    lng: f64,
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
) -> Result<Value> {
    let url = request_url(lat, lng, started_at, ended_at)?;
    let mut waits = 0;
    let client = loop {
        connections::require_enabled(pool, connections::OPEN_METEO)
            .await
            .map_err(|error| match error {
                crate::errors::AppError::ExternalConnectionDisabled(_) => {
                    anyhow::Error::from(WeatherPausedError)
                }
                other => other.into(),
            })?;
        // Resolve before admission so slow DNS cannot bunch admitted requests.
        let client = outbound::outbound_client_for_url(&url, &[]).await?;
        match reserve(pool).await? {
            Admission::Ready => break client,
            Admission::Budget => return Err(WeatherBudgetError.into()),
            Admission::Wait => {
                waits += 1;
                if waits >= 6 {
                    return Err(WeatherBusyError.into());
                }
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        }
    };
    // A fresh pinned client carries neither Rivian headers nor operator proxy settings.
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("Weather connection failed"))?;
    if !response.status().is_success() {
        return Err(WeatherFetchError {
            status: response.status(),
            retry_after_seconds: response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<i64>().ok()),
        }
        .into());
    }
    let body = outbound::read_json(response, RESPONSE_LIMIT, "Weather provider").await?;
    validate_payload(&body, started_at, ended_at)?;
    Ok(body)
}

pub async fn recover_interrupted_jobs(pool: &PgPool) -> Result<()> {
    sqlx::query(
        "UPDATE panda.weather_enrichment_jobs SET status='pending',
         attempts=GREATEST(attempts-1,0), next_attempt_at=now(), last_error=NULL, updated_at=now()
         WHERE status='running' AND updated_at < now()-interval '15 minutes'",
    )
    .execute(pool)
    .await?;
    Ok(())
}

fn validate_payload(body: &Value, start: DateTime<Utc>, end: DateTime<Utc>) -> Result<()> {
    let times = body.pointer("/hourly/time").and_then(Value::as_array);
    let temperatures = body
        .pointer("/hourly/temperature_2m")
        .and_then(Value::as_array);
    let (Some(times), Some(temperatures)) = (times, temperatures) else {
        bail!("Invalid weather response");
    };
    if times.is_empty() || times.len() > 48 || times.len() != temperatures.len() {
        bail!("Invalid weather response size");
    }
    let mut previous = None;
    for (time, temperature) in times.iter().zip(temperatures) {
        let timestamp = time
            .as_str()
            .and_then(|value| chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M").ok())
            .ok_or_else(|| anyhow::anyhow!("Invalid weather timestamp"))?;
        if timestamp.date() < start.date_naive()
            || timestamp.date() > end.date_naive()
            || previous.is_some_and(|previous| timestamp <= previous)
            || (!temperature.is_null()
                && !temperature
                    .as_f64()
                    .is_some_and(|value| value.is_finite() && (-100.0..=70.0).contains(&value)))
        {
            bail!("Invalid weather sample");
        }
        previous = Some(timestamp);
    }
    Ok(())
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
