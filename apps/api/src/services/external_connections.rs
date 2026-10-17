mod activity;

use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::Serialize;
use sqlx::{FromRow, PgPool};

use crate::errors::AppError;

pub const RIVIAN_ACCOUNT: &str = "rivian_account";
pub const OPEN_METEO: &str = "open_meteo";
pub const NOMINATIM: &str = "nominatim";
pub const BASEMAP: &str = "basemap";
pub const ICONIFY: &str = "iconify";
pub const S3_BACKUP: &str = "s3_backup";

pub const OPTIONAL_CONNECTIONS: &[&str] = &[OPEN_METEO, NOMINATIM, BASEMAP, ICONIFY];

/// Advisory policy based on observed Rivian ecosystem behavior, not an
/// authoritative provider contract.
pub const RIVIAN_RENEWAL_INTERVAL: Duration = Duration::days(180);
pub const RIVIAN_RENEWAL_WARNING: Duration = Duration::days(7);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RivianRenewalState {
    Healthy,
    RenewalSoon,
    RenewalDue,
    ReauthRequired,
}

#[derive(Debug, Clone, Serialize)]
pub struct RivianConnectionStatus {
    pub credential_issued_at: Option<DateTime<Utc>>,
    pub expected_renewal_at: Option<DateTime<Utc>>,
    pub renewal_state: Option<RivianRenewalState>,
    pub observed_health: Option<String>,
    pub observed_error: Option<String>,
}

pub fn rivian_renewal_state(
    issued_at: DateTime<Utc>,
    now: DateTime<Utc>,
    auth_failed: bool,
) -> RivianRenewalState {
    if auth_failed {
        return RivianRenewalState::ReauthRequired;
    }
    let renewal_at = issued_at + RIVIAN_RENEWAL_INTERVAL;
    [
        (renewal_at, RivianRenewalState::RenewalDue),
        (
            renewal_at - RIVIAN_RENEWAL_WARNING,
            RivianRenewalState::RenewalSoon,
        ),
    ]
    .into_iter()
    .find_map(|(threshold, state)| (now >= threshold).then_some(state))
    .unwrap_or(RivianRenewalState::Healthy)
}

pub fn rivian_renewal_state_for_auth(
    issued_at: DateTime<Utc>,
    now: DateTime<Utc>,
    auth_state: Option<&str>,
) -> RivianRenewalState {
    rivian_renewal_state(issued_at, now, auth_state == Some("needs_reauth"))
}

pub async fn rivian_status(pool: &PgPool) -> Result<RivianConnectionStatus, AppError> {
    let (issued_at, auth_failed): (Option<DateTime<Utc>>, bool) = sqlx::query_as(
        "SELECT MIN(token_created_at),
            EXISTS(SELECT 1 FROM panda.vehicle_runtime_state WHERE auth_state='needs_reauth')
         FROM panda.vehicle_credentials",
    )
    .fetch_one(pool)
    .await?;
    let runtime: Option<(Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT worker_health, worker_health_msg FROM panda.vehicle_runtime_state
         ORDER BY CASE worker_health WHEN 'error' THEN 5 WHEN 'degraded' THEN 4 WHEN 'stale' THEN 3
                    WHEN 'starting' THEN 2 WHEN 'connected' THEN 1 ELSE 0 END DESC, updated_at DESC LIMIT 1"
    ).fetch_optional(pool).await?;
    let (observed_health, observed_error) = runtime.unwrap_or_default();
    Ok(RivianConnectionStatus {
        credential_issued_at: issued_at,
        expected_renewal_at: issued_at.map(|at| at + RIVIAN_RENEWAL_INTERVAL),
        renewal_state: issued_at.map(|at| rivian_renewal_state(at, Utc::now(), auth_failed)),
        observed_health,
        observed_error,
    })
}

/// Restore the non-secret provider defaults after a fresh install or a
/// sanitized restore. Custom endpoints and encrypted credentials always win:
/// this only fills rows that are entirely absent.
pub async fn ensure_defaults(pool: &PgPool) -> Result<(), AppError> {
    const DEFAULT_ROWS: &str = r#"[{"id":"rivian_account","enabled":true,"mode":"remote","weather_precision":null,"forecast_url":null,"archive_url":null,"base_url":null,"light_url_template":null,"dark_url_template":null,"attribution":null,"attribution_url":null,"custom_autocomplete":false,"allow_private_network":false,"basemap_provider":"auto"},{"id":"open_meteo","enabled":false,"mode":"disabled","weather_precision":"approximate","forecast_url":"https://api.open-meteo.com/v1/forecast","archive_url":"https://archive-api.open-meteo.com/v1/archive","base_url":null,"light_url_template":null,"dark_url_template":null,"attribution":"Weather data by Open-Meteo","attribution_url":"https://open-meteo.com/","custom_autocomplete":false,"allow_private_network":false,"basemap_provider":"auto"},{"id":"nominatim","enabled":false,"mode":"disabled","weather_precision":null,"forecast_url":null,"archive_url":null,"base_url":"https://nominatim.openstreetmap.org","light_url_template":null,"dark_url_template":null,"attribution":"OpenStreetMap contributors","attribution_url":"https://www.openstreetmap.org/copyright","custom_autocomplete":false,"allow_private_network":false,"basemap_provider":"auto"},{"id":"basemap","enabled":false,"mode":"disabled","weather_precision":null,"forecast_url":null,"archive_url":null,"base_url":null,"light_url_template":"https://basemaps.cartocdn.com/light_all/{z}/{x}/{y}.png","dark_url_template":"https://basemaps.cartocdn.com/dark_all/{z}/{x}/{y}.png","attribution":"OpenStreetMap contributors and CARTO","attribution_url":"https://carto.com/attributions","custom_autocomplete":false,"allow_private_network":false,"basemap_provider":"auto"},{"id":"iconify","enabled":false,"mode":"disabled","weather_precision":null,"forecast_url":null,"archive_url":null,"base_url":"https://api.iconify.design","light_url_template":null,"dark_url_template":null,"attribution":"Iconify","attribution_url":"https://iconify.design/","custom_autocomplete":false,"allow_private_network":false,"basemap_provider":"auto"},{"id":"s3_backup","enabled":false,"mode":"disabled","weather_precision":null,"forecast_url":null,"archive_url":null,"base_url":null,"light_url_template":null,"dark_url_template":null,"attribution":null,"attribution_url":null,"custom_autocomplete":false,"allow_private_network":false,"basemap_provider":"auto"}]"#;
    let defaults: serde_json::Value =
        serde_json::from_str(DEFAULT_ROWS).map_err(|error| AppError::Internal(error.into()))?;
    sqlx::query(
        "INSERT INTO panda.external_connection_settings(id,enabled,mode,weather_precision,forecast_url,archive_url,base_url,light_url_template,dark_url_template,attribution,attribution_url,custom_autocomplete,allow_private_network,basemap_provider)
         SELECT id,enabled,mode,weather_precision,forecast_url,archive_url,base_url,light_url_template,dark_url_template,attribution,attribution_url,custom_autocomplete,allow_private_network,basemap_provider FROM jsonb_populate_recordset(NULL::panda.external_connection_settings,$1::jsonb)
         ON CONFLICT(id) DO NOTHING"
    ).bind(defaults).execute(pool).await?;
    Ok(())
}

#[derive(Debug, Clone, FromRow)]
pub struct ConnectionSettingsRow {
    pub id: String,
    pub enabled: bool,
    pub mode: String,
    pub basemap_provider: String,
    pub weather_precision: Option<String>,
    pub forecast_url: Option<String>,
    pub archive_url: Option<String>,
    pub base_url: Option<String>,
    pub light_url_template: Option<String>,
    pub dark_url_template: Option<String>,
    pub attribution: Option<String>,
    pub attribution_url: Option<String>,
    pub request_identifier: Option<String>,
    pub custom_autocomplete: bool,
    /// Deprecated compatibility flag. New policy is represented by the CIDR
    /// allowlist and policy state below.
    pub allow_private_network: bool,
    pub private_network_allowlist: Vec<String>,
    pub private_network_policy_state: String,
    pub api_key_encrypted: Option<Vec<u8>>,
    pub bearer_token_encrypted: Option<Vec<u8>>,
    pub updated_at: DateTime<Utc>,
}

pub use crate::private_deployment::outbound::{connection_allowed, require_connection_allowed};

impl ConnectionSettingsRow {
    fn apply_outbound_policy(mut self) -> Self {
        let allowed = connection_allowed(&self.id)
            && match self.id.as_str() {
                OPEN_METEO => crate::private_deployment::weather::settings_allowed(&self),
                _ => true,
            };
        if !allowed {
            self.enabled = false;
            self.mode.clear();
            self.mode.push_str("disabled");
        }
        self
    }

    pub fn is_active(&self) -> bool {
        if !self.enabled || self.mode == "disabled" {
            return false;
        }
        match self.id.as_str() {
            OPEN_METEO => {
                connection_allowed(&self.id)
                    && crate::private_deployment::weather::settings_allowed(self)
            }
            _ => connection_allowed(&self.id),
        }
    }
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct ConnectionActivityRow {
    pub last_attempt_at: Option<DateTime<Utc>>,
    pub last_success_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub usage_date: NaiveDate,
    pub request_count: i32,
    pub last_test_at: Option<DateTime<Utc>>,
    pub last_test_ok: Option<bool>,
    pub last_test_error: Option<String>,
}

pub async fn load(pool: &PgPool, id: &str) -> Result<ConnectionSettingsRow, AppError> {
    sqlx::query_as::<_, ConnectionSettingsRow>(
        r#"SELECT id, enabled, mode, basemap_provider, weather_precision, forecast_url, archive_url,
                  base_url, light_url_template, dark_url_template, attribution,
                  attribution_url, request_identifier, custom_autocomplete,
                  allow_private_network, private_network_allowlist::text[] AS private_network_allowlist,
                  private_network_policy_state, api_key_encrypted, bearer_token_encrypted,
                  updated_at
           FROM panda.external_connection_settings
           WHERE id = $1"#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or(AppError::NotFound)
    .map(ConnectionSettingsRow::apply_outbound_policy)
}

pub async fn list(
    pool: &PgPool,
) -> Result<Vec<(ConnectionSettingsRow, ConnectionActivityRow)>, AppError> {
    #[derive(FromRow)]
    struct ConnectionWithActivity {
        #[sqlx(flatten)]
        settings: ConnectionSettingsRow,
        #[sqlx(flatten)]
        activity: ConnectionActivityRow,
    }
    let rows = sqlx::query_as::<_, ConnectionWithActivity>(
        r#"SELECT s.id, s.enabled, s.mode, s.basemap_provider, s.weather_precision, s.forecast_url,
                  s.archive_url, s.base_url, s.light_url_template, s.dark_url_template,
                  s.attribution, s.attribution_url, s.request_identifier,
                  s.custom_autocomplete, s.allow_private_network,
                  s.private_network_allowlist::text[] AS private_network_allowlist,
                  s.private_network_policy_state, s.api_key_encrypted,
                  s.bearer_token_encrypted, s.updated_at,
                  a.last_attempt_at, a.last_success_at, a.last_error,
                  a.last_test_at, a.last_test_ok, a.last_test_error,
                  COALESCE(a.usage_date, CURRENT_DATE) AS usage_date,
                  COALESCE(a.request_count, 0) AS request_count
           FROM panda.external_connection_settings s
           LEFT JOIN panda.external_connection_activity a ON a.connection_id = s.id
           ORDER BY CASE s.id
             WHEN 'rivian_account' THEN 1 WHEN 'open_meteo' THEN 2
             WHEN 'nominatim' THEN 3 WHEN 'basemap' THEN 4
             WHEN 'iconify' THEN 5 ELSE 6 END"#,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| (row.settings.apply_outbound_policy(), row.activity))
        .collect())
}

pub async fn require_enabled(pool: &PgPool, id: &str) -> Result<ConnectionSettingsRow, AppError> {
    require_connection_allowed(id)?;
    let settings = load(pool, id).await?;
    if settings.is_active() {
        Ok(settings)
    } else {
        Err(AppError::ExternalConnectionDisabled(id.to_string()))
    }
}

pub async fn record_attempt(pool: &PgPool, id: &str) {
    activity::Event::Attempt.persist(pool, id).await;
}

pub async fn record_success(pool: &PgPool, id: &str) {
    activity::Event::Success.persist(pool, id).await;
}

pub async fn record_failure(pool: &PgPool, id: &str, message: &str) {
    activity::Event::Failure(message).persist(pool, id).await;
}

pub async fn record_test(pool: &PgPool, id: &str, ok: bool, message: Option<&str>) {
    activity::Event::Test { ok, message }
        .persist(pool, id)
        .await;
}

fn sanitize_error(message: &str) -> String {
    message
        .lines()
        .next()
        .unwrap_or("External request failed")
        .chars()
        .take_while(|character| *character != '?')
        .take(240)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        rivian_renewal_state, rivian_renewal_state_for_auth, sanitize_error, RivianRenewalState,
    };
    use chrono::{Duration, TimeZone, Utc};

    #[test]
    fn strips_query_strings_and_bounds_error_text() {
        let result = sanitize_error(&format!(
            "request https://example.test/path?lat=1&lng=2 {}",
            "x".repeat(300)
        ));
        assert!(!result.contains("lat="));
        assert!(result.len() <= 240);
    }

    #[test]
    fn derives_advisory_renewal_boundaries_and_auth_override() {
        let issued = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        assert!(matches!(
            rivian_renewal_state(issued, issued + Duration::days(172), false),
            RivianRenewalState::Healthy
        ));
        assert!(matches!(
            rivian_renewal_state(issued, issued + Duration::days(173), false),
            RivianRenewalState::RenewalSoon
        ));
        assert!(matches!(
            rivian_renewal_state(issued, issued + Duration::days(180), false),
            RivianRenewalState::RenewalDue
        ));
        assert!(matches!(
            rivian_renewal_state(issued, issued + Duration::days(1), true),
            RivianRenewalState::ReauthRequired
        ));
        assert!(matches!(
            rivian_renewal_state_for_auth(issued, issued + Duration::days(1), Some("needs_reauth")),
            RivianRenewalState::ReauthRequired
        ));
    }
}
