//! Periodic and event-triggered polling of the Rivian consumer GraphQL API.
//!
//! This module is intentionally separate from rivian_auth.rs (which owns the
//! login flow) and ws_client.rs (which owns the real-time subscription).
//!
//! ## Stewardship
//! Every outbound HTTP call increments `outbound_graphql_requests` in
//! `panda.rivian_stewardship_counters`.  The caller is responsible for
//! invoking [`increment_poll_counter`] after each successful request.

use crate::ingestion::{
    parser,
    ws_client::{WsInboundEvent, WsInboundKind},
};
use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::ingestion::rivian_auth::rivian_refresh_csrf;
use crate::ingestion::session_store::{decrypt_tokens, encrypt_tokens, RivianTokenBundle};
use crate::services::charge_backfill::{self, ChargeBackfillError};
use crate::services::ingestion_capture::{self, Kind as CaptureKind};

/// Record one GraphQL acquisition in a running ingestion capture.
fn capture_poll<T>(
    vehicle_id: Uuid,
    operation: &str,
    result: &Result<T>,
    detail: serde_json::Value,
) {
    #[derive(Serialize)]
    #[serde(tag = "outcome")]
    enum Captured<'a> {
        #[serde(rename = "ok")]
        Complete {
            operation: &'a str,
            detail: serde_json::Value,
        },
        #[serde(rename = "auth_required")]
        Authentication { operation: &'a str, error: String },
        #[serde(rename = "error")]
        Failed { operation: &'a str, error: String },
    }
    let record = match result {
        Ok(_) => Captured::Complete { operation, detail },
        Err(error) if is_auth_error(error) => Captured::Authentication {
            operation,
            error: error.to_string(),
        },
        Err(error) => Captured::Failed {
            operation,
            error: error.to_string(),
        },
    };
    ingestion_capture::record(vehicle_id, CaptureKind::Poll, serde_json::json!(record));
}
use crate::services::charge_sessions::{
    self, ChargeSessionPayloadRef, ChargeSessionSummaryPayload, UnmatchedInsertPolicy,
};

mod execution;
mod payload_recording;
mod requests;
mod transport;
use requests::ReadOperation;

static BASELINE_FORWARD_DROP_LOG_COUNT: AtomicU64 = AtomicU64::new(0);

pub use transport::AuthError;

/// Stable transport adapter; production requests always use the private policy transport.
pub async fn gql_request<T: for<'de> Deserialize<'de>>(
    _client: &reqwest::Client,
    url: &str,
    tokens: &RivianTokenBundle,
    operation: &str,
    query: &str,
    variables: serde_json::Value,
) -> Result<T> {
    #[cfg(test)]
    if let Some(result) = contract_tests::scripted_request(url, operation, query, &variables) {
        crate::services::outbound_policy::require_telemetry_query(query)?;
        return Ok(serde_json::from_value(result?)?);
    }
    transport::gql_request(_client, url, tokens, operation, query, variables).await
}

// ── URL constants ────────────────────────────────────────────────────────────

const GATEWAY_URL: &str = crate::services::outbound_policy::GATEWAY_URL;
const CHRG_URL: &str = crate::services::outbound_policy::CHARGING_URL;

fn json_number_as_f64(value: Option<&serde_json::Value>) -> Option<f64> {
    match value {
        Some(serde_json::Value::Number(n)) => n.as_f64(),
        Some(serde_json::Value::String(s)) => s.parse().ok(),
        _ => None,
    }
}

// ── Token-refresh helpers ────────────────────────────────────────────────────

/// Returns `true` when the error is the typed [`AuthError`] marker — i.e. the
/// Rivian API explicitly reported an authentication failure (HTTP 401 or
/// GraphQL `extensions.code = "UNAUTHENTICATED"`).  Substring matching of
/// `Error::to_string()` was previously used here and produced false positives
/// against our own error-wrapper strings.
fn is_auth_error(e: &anyhow::Error) -> bool {
    e.downcast_ref::<AuthError>().is_some()
}

/// Refresh the CSRF/app-session tokens and persist the updated bundle.
/// The access token is preserved unchanged — only the short-lived CSRF pair rotates.
async fn try_refresh_csrf(
    vehicle_id: Uuid,
    rejected_tokens: &RivianTokenBundle,
    client: &reqwest::Client,
    pool: &PgPool,
    age_key: &str,
) -> Result<RivianTokenBundle> {
    tracing::info!(%vehicle_id, "refreshing Rivian CSRF session");
    let identity = age_key
        .parse::<age::x25519::Identity>()
        .map_err(|error| anyhow!("bad age key: {error}"))?;
    let mut tx = pool.begin().await?;
    let ciphertext: Vec<u8> = sqlx::query_scalar(
        "SELECT encrypted_tokens FROM panda.vehicle_credentials WHERE vehicle_id=$1 FOR UPDATE",
    )
    .bind(vehicle_id)
    .fetch_one(&mut *tx)
    .await?;
    let current = decrypt_tokens(&ciphertext, &identity)?;
    let reused = session_token_snapshot_changed(rejected_tokens, &current);
    let selected = if reused {
        current
    } else {
        let refreshed = rivian_refresh_csrf(client, &current)
            .await
            .map_err(|error| anyhow!("CSRF refresh failed: {error}"))?;
        let encrypted = encrypt_tokens(&refreshed, &identity)?;
        sqlx::query("UPDATE panda.vehicle_credentials SET encrypted_tokens=$2, last_refreshed_at=now() WHERE vehicle_id=$1")
            .bind(vehicle_id).bind(encrypted).execute(&mut *tx).await?;
        refreshed
    };
    tx.commit().await?;
    if reused {
        tracing::debug!(%vehicle_id, "reusing concurrently refreshed Rivian session");
    } else {
        tracing::info!(%vehicle_id, "Rivian CSRF session refreshed");
    }
    Ok(selected)
}

fn session_token_snapshot_changed(left: &RivianTokenBundle, right: &RivianTokenBundle) -> bool {
    !left.has_same_session_snapshot(right)
}

pub(crate) async fn load_vehicle_tokens(
    vehicle_id: Uuid,
    pool: &PgPool,
    age_key: &str,
) -> Result<(String, RivianTokenBundle)> {
    let identity = age_key
        .parse::<age::x25519::Identity>()
        .map_err(|error| anyhow!("bad age key: {error}"))?;
    let stored = sqlx::query_as::<_, (String, Vec<u8>)>(
        "SELECT v.rivian_vehicle_id, c.encrypted_tokens FROM panda.vehicles v JOIN panda.vehicle_credentials c ON c.vehicle_id = v.id WHERE v.id = $1")
        .bind(vehicle_id).fetch_optional(pool).await?;
    stored
        .map(|(provider_id, encrypted)| {
            let tokens = decrypt_tokens(&encrypted, &identity)
                .map_err(|error| anyhow!("failed to decrypt Rivian credentials: {error}"))?;
            tokens
                .validate()
                .map_err(|error| anyhow!("Rivian credential bundle is invalid: {error}"))?;
            Ok((provider_id, tokens))
        })
        .unwrap_or_else(|| Err(anyhow!("vehicle {vehicle_id} has no Rivian credentials")))
}

async fn with_vehicle_auth_retry<T, F>(
    vehicle_id: Uuid,
    pool: &PgPool,
    client: &reqwest::Client,
    age_key: &str,
    operation_name: &'static str,
    mut operation: F,
) -> Result<T>
where
    for<'a> F: FnMut(
        &'a str,
        &'a RivianTokenBundle,
        &'a PgPool,
        &'a reqwest::Client,
    ) -> BoxFuture<'a, Result<T>>,
{
    let (rivian_vehicle_id, current_tokens) =
        load_vehicle_tokens(vehicle_id, pool, age_key).await?;

    enum RequestState {
        Current(RivianTokenBundle),
        Refresh(RivianTokenBundle),
        Retry(RivianTokenBundle),
    }
    let mut state = RequestState::Current(current_tokens);
    loop {
        state = match state {
            RequestState::Current(tokens) => {
                match operation(&rivian_vehicle_id, &tokens, pool, client).await {
                    Ok(value) => {
                        reset_auth_failure_counter(pool, vehicle_id).await;
                        return Ok(value);
                    }
                    Err(error) if is_auth_error(&error) => RequestState::Refresh(tokens),
                    Err(error) => return Err(error),
                }
            }
            RequestState::Refresh(rejected) => {
                tracing::info!(%vehicle_id, operation=operation_name, "Rivian auth failed; rotating CSRF and retrying");
                match try_refresh_csrf(vehicle_id, &rejected, client, pool, age_key).await {
                    Ok(tokens) => RequestState::Retry(tokens),
                    Err(error) => {
                        record_auth_failure(pool, vehicle_id, &error.to_string()).await;
                        return Err(error);
                    }
                }
            }
            RequestState::Retry(tokens) => {
                let result = operation(&rivian_vehicle_id, &tokens, pool, client).await;
                match &result {
                    Ok(_) => reset_auth_failure_counter(pool, vehicle_id).await,
                    Err(error) if is_auth_error(error) => {
                        record_auth_failure(pool, vehicle_id, &error.to_string()).await;
                    }
                    Err(_) => {}
                }
                return result;
            }
        };
    }
}

/// Threshold for flipping auth_state to `needs_reauth`.  A single transient
/// 401 is recoverable via the CSRF rotation that already ran; we only mark
/// the credentials as dead once we've seen repeated post-rotation failures.
const NEEDS_REAUTH_FAILURE_THRESHOLD: i32 = 3;

/// Increment the consecutive-failure counter.  Flip `auth_state` to
/// `needs_reauth` only after the counter crosses the threshold.
async fn record_auth_failure(pool: &PgPool, vehicle_id: Uuid, reason: &str) {
    let result: Result<i32, sqlx::Error> = sqlx::query_scalar(
        "INSERT INTO panda.vehicle_runtime_state (vehicle_id, consecutive_auth_failures, last_auth_failure_at, updated_at) \
         VALUES ($1, 1, now(), now()) ON CONFLICT (vehicle_id) DO UPDATE SET \
         consecutive_auth_failures = vehicle_runtime_state.consecutive_auth_failures + 1, last_auth_failure_at = now(), updated_at = now() \
         RETURNING consecutive_auth_failures")
        .bind(vehicle_id).fetch_one(pool).await;
    let count = result.unwrap_or(1);
    tracing::debug!(%vehicle_id, consecutive_auth_failures=count, "rivian auth failure counted");
    if count >= NEEDS_REAUTH_FAILURE_THRESHOLD {
        mark_needs_reauth(pool, vehicle_id, reason).await;
    }
}

async fn reset_auth_failure_counter(pool: &PgPool, vehicle_id: Uuid) {
    let _ = AcquisitionWrite::new("panda.vehicle_runtime_state")
        .field(
            "consecutive_auth_failures",
            Stored::Int(Some(0)),
            MergeWrite::Replace,
        )
        .field(
            "last_auth_failure_at",
            Stored::Time(None),
            MergeWrite::Replace,
        )
        .field("updated_at", Stored::Sql("now()"), MergeWrite::Replace)
        .update(
            pool,
            &[("vehicle_id", vehicle_id)],
            "consecutive_auth_failures <> 0",
        )
        .await;
}

/// Write an auth-required state into `vehicle_runtime_state` while keeping
/// collector health separate from credential status.
async fn mark_needs_reauth(pool: &PgPool, vehicle_id: Uuid, reason: &str) {
    let _ = AcquisitionWrite::new("panda.vehicle_runtime_state")
        .field("vehicle_id", Stored::Id(vehicle_id), MergeWrite::Keep)
        .field("is_online", Stored::Flag(Some(false)), MergeWrite::Replace)
        .field(
            "worker_health",
            Stored::Text(Some("error")),
            MergeWrite::Replace,
        )
        .field(
            "worker_health_msg",
            Stored::Text(Some(reason)),
            MergeWrite::Replace,
        )
        .field(
            "auth_state",
            Stored::Text(Some("needs_reauth")),
            MergeWrite::Replace,
        )
        .field(
            "auth_reason_code",
            Stored::Text(Some("rivian_auth_expired")),
            MergeWrite::Replace,
        )
        .field("updated_at", Stored::Sql("now()"), MergeWrite::Replace)
        .upsert(pool, "(vehicle_id)", "")
        .await;
}

// ── Stewardship counter ──────────────────────────────────────────────────────

pub async fn increment_poll_counter(pool: &PgPool, vehicle_id: Uuid) {
    let _ = AcquisitionWrite::new("panda.rivian_stewardship_counters")
        .field("vehicle_id", Stored::Id(vehicle_id), MergeWrite::Keep)
        .field("day", Stored::Date(Utc::now().date_naive()), MergeWrite::Keep)
        .field("outbound_graphql_requests", Stored::Count(1), MergeWrite::Expression(
            "panda.rivian_stewardship_counters.outbound_graphql_requests + EXCLUDED.outbound_graphql_requests"))
        .upsert(pool, "(vehicle_id, day)", "").await;
}

// ── Vehicle enrichment (getUserInfo static fields) ───────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UserInfoEnrichmentData {
    current_user: Option<CurrentUserEnrichment>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CurrentUserEnrichment {
    vehicles: Option<Vec<VehicleEnrichmentItem>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VehicleEnrichmentItem {
    id: String,
    vehicle: Option<VehicleEnrichmentDetails>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VehicleEnrichmentDetails {
    mobile_configuration: Option<MobileConfiguration>,
    vehicle_state: Option<VehicleStateEnrichment>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MobileConfiguration {
    trim_option: Option<MobileConfigOption>,
    #[allow(dead_code)]
    exterior_color_option: Option<MobileConfigOption>,
    interior_color_option: Option<MobileConfigOption>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct MobileConfigOption {
    option_id: Option<String>,
    option_name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VehicleStateEnrichment {
    #[serde(rename = "supportedFeatures")]
    supported_features: Option<Vec<SupportedFeature>>,
}

/// API response shape — `status` field is received from Rivian but not used locally.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct SupportedFeature {
    name: Option<String>,
    status: Option<String>,
}

/// Fetch trim/interior/wheel/charger-port static fields and upsert into `vehicles`.
pub async fn fetch_vehicle_enrichment(
    rivian_vehicle_id: &str,
    vehicle_id: Uuid,
    pool: &PgPool,
    client: &reqwest::Client,
    tokens: &RivianTokenBundle,
) -> Result<()> {
    let data: UserInfoEnrichmentData = ReadOperation::Enrichment
        .execute(client, tokens, rivian_vehicle_id)
        .await?;
    let selected = data
        .current_user
        .into_iter()
        .flat_map(|user| user.vehicles.into_iter().flatten())
        .find(|vehicle| vehicle.id == rivian_vehicle_id)
        .ok_or_else(|| anyhow!("vehicle {rivian_vehicle_id} not found in getUserInfo"))?;
    let (mobile, features) = match selected.vehicle {
        Some(details) => (
            details.mobile_configuration,
            details
                .vehicle_state
                .and_then(|state| state.supported_features),
        ),
        None => (None, None),
    };
    let (interior, trim) = match mobile {
        Some(config) => (
            config
                .interior_color_option
                .and_then(|value| value.option_name),
            config.trim_option.and_then(|value| value.option_name),
        ),
        None => (None, None),
    };
    let features = features.map(|values| {
        serde_json::Value::Array(
            values
                .into_iter()
                .filter_map(|feature| feature.name.map(serde_json::Value::String))
                .collect(),
        )
    });
    AcquisitionWrite::new("panda.vehicles")
        .field(
            "interior_color",
            Stored::Text(interior.as_deref()),
            MergeWrite::Present,
        )
        .field("trim", Stored::Text(trim.as_deref()), MergeWrite::Present)
        .field(
            "supported_features",
            Stored::Json(features.as_ref()),
            MergeWrite::Present,
        )
        .field("updated_at", Stored::Sql("now()"), MergeWrite::Replace)
        .update(pool, &[("id", vehicle_id)], "")
        .await?;
    increment_poll_counter(pool, vehicle_id).await;
    tracing::debug!(%vehicle_id, "vehicle enrichment upserted");
    Ok(())
}

// ── Wallboxes ─────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WallboxesData {
    get_registered_wallboxes: Option<Vec<WallboxItem>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WallboxItem {
    wallbox_id: Option<String>,
    name: Option<String>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    max_power: Option<serde_json::Value>,
    model: Option<String>,
    serial_number: Option<String>,
    #[serde(rename = "softwareVersion")]
    firmware_version: Option<String>,
    linked: Option<bool>,
}

/// Fetch registered home wallboxes and upsert into `panda.wallboxes`.
pub async fn fetch_wallboxes(
    user_id: Uuid,
    vehicle_id: Uuid,
    pool: &PgPool,
    client: &reqwest::Client,
    tokens: &RivianTokenBundle,
) -> Result<()> {
    let data: WallboxesData = ReadOperation::Wallboxes.execute(client, tokens, "").await?;
    let boxes = data.get_registered_wallboxes.unwrap_or_default();
    let count = boxes.len();
    for (id, wallbox) in boxes
        .iter()
        .filter_map(|wallbox| wallbox.wallbox_id.as_deref().map(|id| (id, wallbox)))
    {
        AcquisitionWrite::new("panda.wallboxes")
            .field("user_id", Stored::Id(user_id), MergeWrite::Keep)
            .field(
                "rivian_wallbox_id",
                Stored::Text(Some(id)),
                MergeWrite::Keep,
            )
            .field(
                "name",
                Stored::Text(wallbox.name.as_deref()),
                MergeWrite::Replace,
            )
            .field(
                "latitude",
                Stored::Float(wallbox.latitude),
                MergeWrite::Replace,
            )
            .field(
                "longitude",
                Stored::Float(wallbox.longitude),
                MergeWrite::Replace,
            )
            .field(
                "max_power_kw",
                Stored::Float(json_number_as_f64(wallbox.max_power.as_ref())),
                MergeWrite::Replace,
            )
            .field(
                "model",
                Stored::Text(wallbox.model.as_deref()),
                MergeWrite::Replace,
            )
            .field(
                "serial_number",
                Stored::Text(wallbox.serial_number.as_deref()),
                MergeWrite::Replace,
            )
            .field(
                "firmware_version",
                Stored::Text(wallbox.firmware_version.as_deref()),
                MergeWrite::Replace,
            )
            .field("linked", Stored::Flag(wallbox.linked), MergeWrite::Replace)
            .field("updated_at", Stored::Sql("now()"), MergeWrite::Replace)
            .upsert(pool, "(user_id, rivian_wallbox_id)", "")
            .await?;
    }
    increment_poll_counter(pool, vehicle_id).await;
    tracing::debug!(%vehicle_id, %count, "wallboxes upserted");
    Ok(())
}

pub async fn fetch_vehicle_enrichment_for_vehicle(
    vehicle_id: Uuid,
    pool: &PgPool,
    client: &reqwest::Client,
    age_key: &str,
) -> Result<()> {
    VehicleRead::Enrichment
        .acquire(vehicle_id, pool, client, age_key)
        .await
        .map(|_| ())
}

pub async fn fetch_wallboxes_for_vehicle(
    user_id: Uuid,
    vehicle_id: Uuid,
    pool: &PgPool,
    client: &reqwest::Client,
    age_key: &str,
) -> Result<()> {
    VehicleRead::Wallboxes(user_id)
        .acquire(vehicle_id, pool, client, age_key)
        .await
        .map(|_| ())
}

// ── Charge session history ────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CompletedSessionsData {
    get_completed_session_summaries: Option<Vec<ChargeSessionSummaryPayload>>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct CompletedSessionItem {
    /// Rivian's own session identifier
    transaction_id: Option<String>,
    start_instant: Option<DateTime<Utc>>,
    end_instant: Option<DateTime<Utc>>,
    charger_type: Option<String>,
    currency_code: Option<String>,
    /// kWh added during the session
    total_energy_kwh: Option<f64>,
    /// km of range added
    range_added_km: Option<f64>,
    city: Option<String>,
    vehicle_id: Option<String>,
    vehicle_name: Option<String>,
    /// e.g. "Rivian", "Electrify America"
    vendor: Option<String>,
    /// Total billed amount in USD (Rivian network only)
    paid_total: Option<f64>,
    is_public: Option<bool>,
    /// SoC at session start (0–100)
    is_home_charger: Option<bool>,
    /// SoC at session end (0–100)
    is_roaming_network: Option<bool>,
    meta: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct LiveSessionHistoryData {
    get_live_session_history: Option<LiveSessionHistory>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct LiveSessionHistory {
    chart_data: Option<Vec<LiveCurvePoint>>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct LiveCurvePoint {
    kw: Option<f64>,
    time: Option<DateTime<Utc>>,
}

const RECENT_INCREMENTAL_INSERT_LOOKBACK_DAYS: i64 = 30;
const LIVE_CURVE_OVERLAP_MINUTES: i64 = 5;

async fn record_charge_history_sync_started(pool: &PgPool, vehicle_id: Uuid) {
    let mut row = AcquisitionWrite::new("panda.vehicle_runtime_state").field(
        "vehicle_id",
        Stored::Id(vehicle_id),
        MergeWrite::Keep,
    );
    for column in ["last_charge_history_sync_at", "updated_at"] {
        row = row.field(column, Stored::Sql("now()"), MergeWrite::Replace);
    }
    let _ = row.upsert(pool, "(vehicle_id)", "").await;
}

async fn record_charge_history_sync_succeeded(pool: &PgPool, vehicle_id: Uuid) {
    let mut row = AcquisitionWrite::new("panda.vehicle_runtime_state").field(
        "vehicle_id",
        Stored::Id(vehicle_id),
        MergeWrite::Keep,
    );
    for column in [
        "last_charge_history_sync_at",
        "last_charge_history_success_at",
        "updated_at",
    ] {
        row = row.field(column, Stored::Sql("now()"), MergeWrite::Replace);
    }
    let _ = row.upsert(pool, "(vehicle_id)", "").await;
}

/// Fetch completed charge sessions from Rivian's charging endpoint.
///
/// ## Modes
/// - **`full_backfill = false`** (post-session enrichment): enriches existing
///   local rows by matching on `rivian_session_id` then by start-time window.
///   Recent completed sessions without a local match are inserted so missed
///   telemetry sessions can still land in the canonical charging history.
/// - **`full_backfill = true`** (first-start backfill): additionally INSERTs
///   sessions that have no local match at all, tagged with `source = 'rivian_api'`.
///   The provider returns one complete summary response; this adapter does not paginate.
pub async fn fetch_charge_history(
    rivian_vehicle_id: &str,
    vehicle_id: Uuid,
    pool: &PgPool,
    client: &reqwest::Client,
    tokens: &RivianTokenBundle,
) -> Result<usize> {
    fetch_charge_history_inner_v2(rivian_vehicle_id, vehicle_id, false, pool, client, tokens).await
}

pub async fn fetch_charge_history_for_vehicle(
    vehicle_id: Uuid,
    pool: &PgPool,
    client: &reqwest::Client,
    age_key: &str,
) -> Result<usize> {
    VehicleRead::History
        .acquire(vehicle_id, pool, client, age_key)
        .await
}

/// Same as [`fetch_charge_history`] but inserts new sessions from the API that
/// have no local counterpart (for the first-start full backfill flow).
pub async fn fetch_charge_history_full(
    rivian_vehicle_id: &str,
    vehicle_id: Uuid,
    pool: &PgPool,
    client: &reqwest::Client,
    tokens: &RivianTokenBundle,
) -> Result<usize> {
    fetch_charge_history_inner_v2(rivian_vehicle_id, vehicle_id, true, pool, client, tokens).await
}

async fn fetch_charge_history_inner_v2(
    _rivian_vehicle_id: &str,
    vehicle_id: Uuid,
    full_backfill: bool,
    pool: &PgPool,
    client: &reqwest::Client,
    tokens: &RivianTokenBundle,
) -> Result<usize> {
    let mut lease = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("rivian-charge-history:{vehicle_id}"))
        .execute(&mut *lease)
        .await?;
    let acquired =
        fetch_charge_history_locked(vehicle_id, full_backfill, pool, client, tokens).await;
    let released = lease.rollback().await.map_err(|error| {
        tracing::error!(%vehicle_id, %error, "charge history lock release failed");
        anyhow!("failed to release charge-history transaction lock: {error}")
    });
    match acquired {
        Err(error) => Err(error),
        Ok(count) => released.map(|()| count),
    }
}

async fn fetch_charge_history_locked(
    vehicle_id: Uuid,
    full_backfill: bool,
    pool: &PgPool,
    client: &reqwest::Client,
    tokens: &RivianTokenBundle,
) -> Result<usize> {
    let data: CompletedSessionsData = ReadOperation::CompletedHistory
        .execute(client, tokens, "")
        .await?;
    increment_poll_counter(pool, vehicle_id).await;
    let policy = if full_backfill {
        UnmatchedInsertPolicy::Always
    } else {
        UnmatchedInsertPolicy::RecentOnly {
            now: Utc::now(),
            lookback_days: RECENT_INCREMENTAL_INSERT_LOOKBACK_DAYS,
        }
    };
    let summaries = data.get_completed_session_summaries.unwrap_or_default();
    let total_processed = summaries.len();
    let mut audit = [0usize; 5];
    let mut error_sample = None;
    let mut total_linked = 0usize;
    for summary in summaries {
        let evidence = record_charge_payload_with_ref(
            pool,
            vehicle_id,
            "getCompletedSessionSummaries",
            summary.transaction_id.as_deref(),
            summary.vehicle_id.as_deref(),
            serde_json::to_value(&summary).unwrap_or_else(|_| serde_json::json!({})),
        )
        .await;
        let payload = match evidence {
            Ok(ChargePayloadRecordOutcome::Recorded {
                payload_ref,
                repaired,
            }) => {
                audit[usize::from(payload_ref.unchanged)] += 1;
                audit[2] += usize::from(repaired);
                Some(payload_ref)
            }
            Ok(ChargePayloadRecordOutcome::EvidenceExpired) => {
                audit[3] += 1;
                None
            }
            Err(error) => {
                audit[4] += 1;
                error_sample.get_or_insert_with(|| safe_charge_audit_error(&error));
                None
            }
        };
        if unchanged_linked_session(payload).is_some() {
            continue;
        }
        let Some(session_id) = charge_sessions::reconcile_completed_session_summary(
            pool, vehicle_id, &summary, policy, payload,
        )
        .await?
        else {
            continue;
        };
        if let Some(payload) = payload {
            let mut query = sqlx::QueryBuilder::<sqlx::Postgres>::new(
                "UPDATE panda.rivian_charge_payloads SET charge_session_id = ",
            );
            query
                .push_bind(session_id)
                .push(" WHERE id = ")
                .push_bind(payload.payload_id)
                .push(" AND charge_session_id IS DISTINCT FROM ")
                .push_bind(session_id);
            query.build().execute(pool).await?;
        }
        total_linked += 1;
        crate::services::cost::recompute_charge_session_cost(pool, session_id).await?;
    }
    let [payloads_inserted, payloads_reused, payloads_repaired, payloads_evidence_expired, payload_audit_failures] =
        audit;
    if payloads_repaired != 0 || payload_audit_failures != 0 {
        tracing::warn!(%vehicle_id, payloads_repaired, payload_audit_failures, error_sample=error_sample.as_deref().unwrap_or("none"), "charge history payload audit required repair");
    }
    if payloads_evidence_expired != 0 {
        tracing::info!(%vehicle_id, payloads_evidence_expired, "charge history retained payload evidence expired");
    }
    tracing::debug!(%vehicle_id, total_processed, total_linked, payloads_inserted, payloads_reused, full_backfill, "charge history synced");
    Ok(total_processed)
}

fn unchanged_linked_session(payload_ref: Option<ChargeSessionPayloadRef>) -> Option<Uuid> {
    payload_ref.and_then(|payload| {
        if payload.unchanged {
            payload.charge_session_id
        } else {
            None
        }
    })
}

fn safe_charge_audit_error(error: &anyhow::Error) -> String {
    let text = error.to_string();
    let sensitive = [
        "payload",
        "fingerprint",
        "identity",
        "transaction",
        "vehicle",
    ]
    .iter()
    .any(|term| {
        text.as_bytes()
            .windows(term.len())
            .any(|window| window.eq_ignore_ascii_case(term.as_bytes()))
    });
    match sensitive {
        true => "sensitive_audit_error".to_owned(),
        false => text.chars().take(256).collect(),
    }
}

async fn record_charge_payload_with_ref(
    pool: &PgPool,
    vehicle_id: Uuid,
    operation: &str,
    rivian_transaction_id: Option<&str>,
    rivian_vehicle_id: Option<&str>,
    payload: serde_json::Value,
) -> Result<ChargePayloadRecordOutcome> {
    payload_recording::record(
        pool,
        vehicle_id,
        operation,
        rivian_transaction_id,
        rivian_vehicle_id,
        payload,
    )
    .await
}

#[derive(Debug)]
enum ChargePayloadRecordOutcome {
    Recorded {
        payload_ref: ChargeSessionPayloadRef,
        repaired: bool,
    },
    /// The durable semantic identity remains, but retention has expired every
    /// matching JSON payload. The caller must continue normal session
    /// reconciliation without manufacturing replacement evidence.
    EvidenceExpired,
}

async fn find_exact_payload_survivor(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    vehicle_id: Uuid,
    operation: &str,
    rivian_transaction_id: Option<&str>,
    rivian_vehicle_id: Option<&str>,
    fingerprint: &[u8],
) -> Result<Option<(Uuid, DateTime<Utc>, Option<Uuid>)>> {
    type SurvivorRow = (Uuid, DateTime<Utc>, Option<Uuid>, Option<Uuid>);
    let candidate: Option<SurvivorRow> = sqlx::query_as(
        r#"SELECT payload.id, payload.captured_at, payload.charge_session_id, cs.vehicle_id
           FROM panda.rivian_charge_payloads payload
           LEFT JOIN panda.charge_sessions cs ON cs.id = payload.charge_session_id
           WHERE payload.vehicle_id = $1 AND payload.operation = $2
             AND payload.rivian_transaction_id IS NOT DISTINCT FROM $3::text
             AND payload.rivian_vehicle_id IS NOT DISTINCT FROM $4::text
             AND panda.charge_payload_fingerprint(payload.payload) = $5
           ORDER BY (payload.charge_session_id IS NOT NULL AND cs.vehicle_id = $1) DESC,
                    payload.captured_at, payload.id
           LIMIT 1 FOR UPDATE OF payload"#,
    )
    .bind(vehicle_id)
    .bind(operation)
    .bind(rivian_transaction_id)
    .bind(rivian_vehicle_id)
    .bind(fingerprint)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(candidate.map(|(id, captured_at, session, owner)| {
        (
            id,
            captured_at,
            session.filter(|_| owner == Some(vehicle_id)),
        )
    }))
}

async fn repair_payload_aliases(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    previous_payload_id: Uuid,
    replacement_payload_id: Uuid,
    replacement_captured_at: DateTime<Utc>,
) -> Result<()> {
    AcquisitionWrite::new("panda.charge_session_external_aliases")
        .field(
            "latest_payload_id",
            Stored::Id(replacement_payload_id),
            MergeWrite::Replace,
        )
        .field(
            "latest_payload_captured_at",
            Stored::Time(Some(replacement_captured_at)),
            MergeWrite::Replace,
        )
        .update(&mut **tx, &[("latest_payload_id", previous_payload_id)], "")
        .await?;
    Ok(())
}

async fn clear_expired_payload_aliases(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    previous_payload_id: Option<Uuid>,
) -> Result<()> {
    if let Some(previous) = previous_payload_id {
        AcquisitionWrite::new("panda.charge_session_external_aliases")
            .field(
                "latest_payload_id",
                Stored::Sql("NULL"),
                MergeWrite::Replace,
            )
            .field(
                "latest_payload_captured_at",
                Stored::Time(None),
                MergeWrite::Replace,
            )
            .update(&mut **tx, &[("latest_payload_id", previous)], "")
            .await?;
    }
    Ok(())
}

#[cfg(test)]
const LIVE_SESSION_HISTORY_QUERY: &str = requests::LIVE_HISTORY_TEST_QUERY;

pub async fn fetch_live_session_history(
    rivian_vehicle_id: &str,
    vehicle_id: Uuid,
    active_session_id: Option<Uuid>,
    pool: &PgPool,
    client: &reqwest::Client,
    tokens: &RivianTokenBundle,
) -> Result<usize> {
    let data: LiveSessionHistoryData = ReadOperation::LiveHistory
        .execute(client, tokens, rivian_vehicle_id)
        .await?;
    increment_poll_counter(pool, vehicle_id).await;
    let points = data
        .get_live_session_history
        .and_then(|history| history.chart_data)
        .unwrap_or_default();
    let latest: Option<DateTime<Utc>> = sqlx::query_scalar(
        "SELECT MAX(ts) FROM panda.rivian_charge_curve_points WHERE vehicle_id = $1",
    )
    .bind(vehicle_id)
    .fetch_one(pool)
    .await?;
    let cutoff = latest.map(|at| at - chrono::Duration::minutes(LIVE_CURVE_OVERLAP_MINUTES));
    let records_received = points.len();
    let mut ordered = std::collections::BTreeMap::<DateTime<Utc>, Vec<Option<f64>>>::new();
    for point in points {
        if let Some(at) = point
            .time
            .filter(|at| cutoff.is_none_or(|cutoff| *at >= cutoff))
        {
            ordered.entry(at).or_default().push(point.kw);
        }
    }
    let (timestamps, power): (Vec<_>, Vec<_>) = ordered
        .into_iter()
        .flat_map(|(at, readings)| readings.into_iter().map(move |power| (at, power)))
        .unzip();
    let changed = if timestamps.is_empty() {
        0
    } else {
        persist_curve_batch(pool, vehicle_id, active_session_id, &timestamps, &power).await?
    };
    tracing::debug!(%vehicle_id, records_received, records_selected=timestamps.len(), curve_points_changed=changed,
        "live charge curve synchronized");
    Ok(changed)
}

pub async fn fetch_live_session_history_for_vehicle(
    vehicle_id: Uuid,
    active_session_id: Option<Uuid>,
    pool: &PgPool,
    client: &reqwest::Client,
    age_key: &str,
) -> Result<usize> {
    VehicleRead::LiveHistory(active_session_id)
        .acquire(vehicle_id, pool, client, age_key)
        .await
}

// ── Live session data (WebSocket chargingSession subscription) ──────────────

/// Serialized to Redis and served by GET /v1/vehicles/:id/live-session.
/// Field names must match the frontend LiveSession TypeScript interface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveSessionData {
    pub soc_pct: Option<f64>,
    pub power_kw: Option<f64>,
    pub energy_kwh: Option<f64>,
    pub range_added_km: Option<f64>,
    pub time_remaining_min: Option<f64>,
    pub charger_type: Option<String>,
    pub ts: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub charge_rate_kph: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_elapsed_seconds: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_free_session: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vehicle_charger_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
}

pub(crate) const LIVE_SESSION_TTL_SECONDS: u64 = 120;

pub(crate) fn live_session_redis_key(vehicle_id: Uuid) -> String {
    format!("vehicle:{vehicle_id}:live_session")
}

/// Convert the documented `chargingSession` payload into the existing live
/// Redis contract. `timeRemaining` and `timeElapsed` are seconds in Rivian's
/// charging payload; the public live-session field remains minutes.
pub fn normalize_charging_session(
    event: &crate::ingestion::parser::ChargingSessionEvent,
    received_at: DateTime<Utc>,
) -> Option<LiveSessionData> {
    let live = event.live_data.as_ref()?;
    let normalized = LiveSessionData {
        soc_pct: event
            .chart_data
            .iter()
            .filter_map(|point| point.soc)
            .next_back(),
        power_kw: live.power_kw,
        energy_kwh: live.total_charged_energy,
        range_added_km: live.range_added_this_session,
        time_remaining_min: live.time_remaining_seconds.map(|seconds| seconds / 60.0),
        charger_type: live.vehicle_charger_state.clone(),
        ts: received_at,
        charge_rate_kph: live.kilometers_charged_per_hour,
        time_elapsed_seconds: live.time_elapsed_seconds,
        price: live.price,
        currency: live.currency.clone(),
        is_free_session: live.is_free_session,
        vehicle_charger_state: live.vehicle_charger_state.clone(),
        started_at: live.start_time,
    };
    let has_metric = [
        normalized.power_kw,
        normalized.energy_kwh,
        normalized.range_added_km,
        normalized.time_remaining_min,
        normalized.charge_rate_kph,
        normalized.time_elapsed_seconds,
        normalized.price,
    ]
    .iter()
    .any(Option::is_some);
    let has_metadata = normalized.currency.is_some()
        || normalized.is_free_session.is_some()
        || normalized.vehicle_charger_state.is_some()
        || normalized.started_at.is_some();
    let has_chart = event.chart_data.iter().any(|point| {
        point.soc.is_some()
            || point.power_kw.is_some()
            || point.start_time.is_some()
            || point.end_time.is_some()
            || point.time_estimation_validity_status.is_some()
            || point.vehicle_charger_state.is_some()
    });
    (has_metric || has_metadata || has_chart).then_some(normalized)
}

/// Persist the fields that belong to the active canonical charge session.
/// The WebSocket worker owns when this is called; this helper owns the SQL
/// shape so HTTP history and live-stream enrichment share one seam.
pub async fn persist_live_session_data(
    pool: &PgPool,
    active_session_id: Option<Uuid>,
    live: &LiveSessionData,
) -> Result<()> {
    if let Some(session_id) = active_session_id {
        let mut row = AcquisitionWrite::new("panda.charge_sessions")
            .field("live_current_currency", Stored::Text(live.currency.as_deref()), MergeWrite::Present)
            .field("live_time_elapsed_seconds", Stored::Int(live.time_elapsed_seconds.map(|seconds| seconds.round() as i32)), MergeWrite::Present)
            .field("live_session_started_at", Stored::Time(live.started_at), MergeWrite::Present)
            .field("live_time_remaining_minutes", Stored::Int(live.time_remaining_min.map(|minutes| minutes.round() as i32)), MergeWrite::Present)
            .field("is_free_session", Stored::Flag(live.is_free_session), MergeWrite::Present)
            .field("duration_minutes", Stored::Int(live.time_elapsed_seconds.map(|seconds| (seconds / 60.0).round() as i32)), MergeWrite::Present)
            .field("source", Stored::Sql("CASE WHEN source = 'rivian_api' THEN 'telemetry+rivian_api' ELSE COALESCE(source, 'telemetry') END"), MergeWrite::Replace)
            .field("data_confidence", Stored::Sql("CASE WHEN source = 'rivian_api' THEN 'telemetry_enriched' ELSE COALESCE(data_confidence, 'telemetry') END"), MergeWrite::Replace);
        for (column, value) in [
            ("live_current_price", live.price),
            ("live_total_charged_kwh", live.energy_kwh),
            ("live_range_added_km", live.range_added_km),
            ("live_power_kw", live.power_kw),
            ("live_charge_rate_kph", live.charge_rate_kph),
            ("kwh_added", live.energy_kwh),
            ("range_added_km", live.range_added_km),
        ] {
            row = row.field(column, Stored::Float(value), MergeWrite::Present);
        }
        if live.power_kw.is_some() {
            row = row.field(
                "max_charge_rate_kw",
                Stored::Float(live.power_kw),
                MergeWrite::Greatest,
            );
        }
        row.update(pool, &[("id", session_id)], "ended_at IS NULL")
            .await?;
    }
    Ok(())
}

/// Persist chart power points emitted by `chargingSession`. The existing
/// curve table deliberately stores only observed power; absent or malformed
/// timestamps are ignored instead of inventing samples.
pub async fn persist_charging_session_chart(
    pool: &PgPool,
    vehicle_id: Uuid,
    active_session_id: Option<Uuid>,
    points: &[crate::ingestion::parser::ChargingSessionChartPoint],
) -> Result<usize> {
    let (timestamps, power): (Vec<_>, Vec<_>) = points
        .iter()
        .filter_map(|point| point.start_time.map(|at| (at, point.power_kw)))
        .unzip();
    match timestamps.is_empty() {
        true => Ok(0),
        false => {
            persist_curve_batch(pool, vehicle_id, active_session_id, &timestamps, &power).await
        }
    }
}

async fn persist_curve_batch(
    pool: &PgPool,
    vehicle_id: Uuid,
    active_session_id: Option<Uuid>,
    timestamps: &[DateTime<Utc>],
    power: &[Option<f64>],
) -> Result<usize> {
    if timestamps.is_empty() {
        return Ok(0);
    }
    let actions = sqlx::query_scalar::<_, bool>(
        r#"INSERT INTO panda.rivian_charge_curve_points
               (vehicle_id, charge_session_id, ts, power_kw)
           SELECT $1, $2, incoming.ts, incoming.power_kw
           FROM unnest($3::timestamptz[], $4::double precision[])
               AS incoming(ts, power_kw)
           ON CONFLICT (vehicle_id, ts)
           DO UPDATE SET
               charge_session_id = COALESCE(
                   rivian_charge_curve_points.charge_session_id,
                   EXCLUDED.charge_session_id
               ),
               power_kw = COALESCE(
                   EXCLUDED.power_kw,
                   rivian_charge_curve_points.power_kw
               )
           WHERE rivian_charge_curve_points.charge_session_id IS DISTINCT FROM
                     COALESCE(
                         rivian_charge_curve_points.charge_session_id,
                         EXCLUDED.charge_session_id
                     )
              OR rivian_charge_curve_points.power_kw IS DISTINCT FROM
                     COALESCE(
                         EXCLUDED.power_kw,
                         rivian_charge_curve_points.power_kw
                     )
           RETURNING xmax = 0"#,
    )
    .bind(vehicle_id)
    .bind(active_session_id)
    .bind(timestamps)
    .bind(power)
    .fetch_all(pool)
    .await?;
    Ok(actions.len())
}

// ── Charging schedule ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChargingScheduleData {
    get_vehicle: Option<VehicleChargingSchedules>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VehicleChargingSchedules {
    charging_schedules: Option<Vec<ChargePolicyItem>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChargePolicyItem {
    enabled: Option<bool>,
    start_time: Option<i32>,
    duration: Option<i32>,
    amperage: Option<f64>,
    location: Option<ChargingScheduleLocation>,
    week_days: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChargingScheduleLocation {
    latitude: Option<f64>,
    longitude: Option<f64>,
}

/// Fetch the vehicle's charging schedule and upsert into `charging_schedules`.
pub async fn fetch_charging_schedule(
    rivian_vehicle_id: &str,
    vehicle_id: Uuid,
    pool: &PgPool,
    client: &reqwest::Client,
    tokens: &RivianTokenBundle,
) -> Result<()> {
    let data: ChargingScheduleData = ReadOperation::ChargingSchedule
        .execute(client, tokens, rivian_vehicle_id)
        .await?;
    let selected = data
        .get_vehicle
        .into_iter()
        .flat_map(|vehicle| vehicle.charging_schedules.into_iter().flatten())
        .last();
    if let Some(schedule) = selected {
        let (latitude, longitude) = schedule
            .location
            .as_ref()
            .map(|value| (value.latitude, value.longitude))
            .unwrap_or_default();
        AcquisitionWrite::new("panda.charging_schedules")
            .field("vehicle_id", Stored::Id(vehicle_id), MergeWrite::Keep)
            .field(
                "enabled",
                Stored::Flag(Some(schedule.enabled.unwrap_or(false))),
                MergeWrite::Replace,
            )
            .field(
                "start_time_minutes",
                Stored::Int(schedule.start_time),
                MergeWrite::Replace,
            )
            .field(
                "duration_minutes",
                Stored::Int(schedule.duration),
                MergeWrite::Replace,
            )
            .field(
                "amperage",
                Stored::Float(schedule.amperage),
                MergeWrite::Replace,
            )
            .field("location_lat", Stored::Float(latitude), MergeWrite::Replace)
            .field(
                "location_lng",
                Stored::Float(longitude),
                MergeWrite::Replace,
            )
            .field(
                "week_days",
                Stored::Texts(schedule.week_days.as_deref()),
                MergeWrite::Replace,
            )
            .field("rivian_updated_at", Stored::Time(None), MergeWrite::Replace)
            .field("updated_at", Stored::Sql("now()"), MergeWrite::Replace)
            .upsert(pool, "(vehicle_id)", "")
            .await?;
    }
    increment_poll_counter(pool, vehicle_id).await;
    tracing::debug!(%vehicle_id, "charging schedule upserted");
    Ok(())
}

pub async fn fetch_charging_schedule_for_vehicle(
    vehicle_id: Uuid,
    pool: &PgPool,
    client: &reqwest::Client,
    age_key: &str,
) -> Result<()> {
    VehicleRead::Schedule
        .acquire(vehicle_id, pool, client, age_key)
        .await
        .map(|_| ())
}

// ── Departure schedules ───────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DepartureSchedulesData {
    get_departure_schedules: Option<Vec<DepartureScheduleItem>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DepartureScheduleItem {
    id: Option<String>,
    name: Option<String>,
    enabled: Option<bool>,
    occurrence: Option<serde_json::Value>,
    preconditioning_settings: Option<serde_json::Value>,
}

/// Fetch departure/preconditioning schedules and upsert into `departure_schedules`.
pub async fn fetch_departure_schedules(
    rivian_vehicle_id: &str,
    vehicle_id: Uuid,
    pool: &PgPool,
    client: &reqwest::Client,
    tokens: &RivianTokenBundle,
) -> Result<()> {
    let data: DepartureSchedulesData = ReadOperation::DepartureSchedules
        .execute(client, tokens, rivian_vehicle_id)
        .await?;
    let schedules = data.get_departure_schedules.unwrap_or_default();
    for (id, schedule) in schedules
        .iter()
        .filter_map(|schedule| schedule.id.as_deref().map(|id| (id, schedule)))
    {
        AcquisitionWrite::new("panda.departure_schedules")
            .field("vehicle_id", Stored::Id(vehicle_id), MergeWrite::Keep)
            .field(
                "rivian_schedule_id",
                Stored::Text(Some(id)),
                MergeWrite::Keep,
            )
            .field(
                "name",
                Stored::Text(schedule.name.as_deref()),
                MergeWrite::Replace,
            )
            .field(
                "enabled",
                Stored::Flag(Some(schedule.enabled.unwrap_or(false))),
                MergeWrite::Replace,
            )
            .field(
                "occurrence",
                Stored::Json(schedule.occurrence.as_ref()),
                MergeWrite::Replace,
            )
            .field(
                "comfort_settings",
                Stored::Json(schedule.preconditioning_settings.as_ref()),
                MergeWrite::Replace,
            )
            .field("updated_at", Stored::Sql("now()"), MergeWrite::Replace)
            .upsert(pool, "(vehicle_id, rivian_schedule_id)", "")
            .await?;
    }
    increment_poll_counter(pool, vehicle_id).await;
    tracing::debug!(%vehicle_id, "departure schedules upserted");
    Ok(())
}

// ── OTA details ───────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OtaDetailsData {
    #[serde(rename = "getOTAUpdateDetails", alias = "getOtaUpdateDetails")]
    get_ota_update_details: Option<OtaDetailsPayload>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OtaDetailsPayload {
    release_notes_url: Option<String>,
}

/// Fetch OTA update details and store the release notes URL in `vehicles`.
pub async fn fetch_ota_details(
    rivian_vehicle_id: &str,
    vehicle_id: Uuid,
    pool: &PgPool,
    client: &reqwest::Client,
    tokens: &RivianTokenBundle,
) -> Result<()> {
    let data: OtaDetailsData = ReadOperation::OtaDetails
        .execute(client, tokens, rivian_vehicle_id)
        .await?;
    let url = data
        .get_ota_update_details
        .and_then(|details| details.release_notes_url);
    if let Some(value) = url.as_deref() {
        AcquisitionWrite::new("panda.vehicles")
            .field(
                "ota_release_notes_url",
                Stored::Text(Some(value)),
                MergeWrite::Replace,
            )
            .field("updated_at", Stored::Sql("now()"), MergeWrite::Replace)
            .update(pool, &[("id", vehicle_id)], "")
            .await?;
    }
    increment_poll_counter(pool, vehicle_id).await;
    tracing::debug!(%vehicle_id, ?url, "OTA details upserted");
    Ok(())
}

// ── Charging schedule mutation ────────────────────────────────────────────────

/// Input for creating or updating the charging schedule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChargingScheduleInput {
    pub enabled: bool,
    pub start_time_minutes: Option<i32>,
    pub duration_minutes: Option<i32>,
    pub amperage: Option<f64>,
    pub location_lat: Option<f64>,
    pub location_lng: Option<f64>,
    pub week_days: Option<Vec<String>>,
}

/// Stable adapter: vehicle changes are denied by the read-only provider policy.
pub async fn mutate_charging_schedule(
    rivian_vehicle_id: &str,
    vehicle_id: Uuid,
    input: &ChargingScheduleInput,
    pool: &PgPool,
    client: &reqwest::Client,
    tokens: &RivianTokenBundle,
) -> Result<()> {
    let _ = (rivian_vehicle_id, vehicle_id, input, pool, client, tokens);
    Err(crate::services::outbound_policy::PolicyError::VehicleWrite.into())
}

pub async fn mutate_charging_schedule_for_vehicle(
    vehicle_id: Uuid,
    input: &ChargingScheduleInput,
    pool: &PgPool,
    client: &reqwest::Client,
    age_key: &str,
) -> Result<()> {
    let (provider_id, tokens) = load_vehicle_tokens(vehicle_id, pool, age_key).await?;
    mutate_charging_schedule(&provider_id, vehicle_id, input, pool, client, &tokens).await
}

// ── Departure schedule mutations ──────────────────────────────────────────────

/// Input for creating or updating a departure schedule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepartureScheduleInput {
    pub name: Option<String>,
    pub enabled: bool,
    pub occurrence: Option<serde_json::Value>,
    pub comfort_settings: Option<serde_json::Value>,
}

/// Stable adapter: creating departure schedules is denied by provider policy.
pub async fn create_departure_schedule(
    rivian_vehicle_id: &str,
    vehicle_id: Uuid,
    input: &DepartureScheduleInput,
    pool: &PgPool,
    client: &reqwest::Client,
    tokens: &RivianTokenBundle,
) -> Result<String> {
    let _ = (rivian_vehicle_id, vehicle_id, input, pool, client, tokens);
    Err(crate::services::outbound_policy::PolicyError::VehicleWrite.into())
}

pub async fn create_departure_schedule_for_vehicle(
    vehicle_id: Uuid,
    input: &DepartureScheduleInput,
    pool: &PgPool,
    client: &reqwest::Client,
    age_key: &str,
) -> Result<String> {
    let (provider_id, tokens) = load_vehicle_tokens(vehicle_id, pool, age_key).await?;
    create_departure_schedule(&provider_id, vehicle_id, input, pool, client, &tokens).await
}

/// Stable adapter: updating departure schedules is denied by provider policy.
pub async fn update_departure_schedule(
    rivian_vehicle_id: &str,
    vehicle_id: Uuid,
    rivian_schedule_id: &str,
    input: &DepartureScheduleInput,
    pool: &PgPool,
    client: &reqwest::Client,
    tokens: &RivianTokenBundle,
) -> Result<()> {
    let _ = (
        rivian_vehicle_id,
        vehicle_id,
        rivian_schedule_id,
        input,
        pool,
        client,
        tokens,
    );
    Err(crate::services::outbound_policy::PolicyError::VehicleWrite.into())
}

pub async fn update_departure_schedule_for_vehicle(
    vehicle_id: Uuid,
    rivian_schedule_id: &str,
    input: &DepartureScheduleInput,
    pool: &PgPool,
    client: &reqwest::Client,
    age_key: &str,
) -> Result<()> {
    let (provider_id, tokens) = load_vehicle_tokens(vehicle_id, pool, age_key).await?;
    update_departure_schedule(
        &provider_id,
        vehicle_id,
        rivian_schedule_id,
        input,
        pool,
        client,
        &tokens,
    )
    .await
}

/// Stable adapter: deleting departure schedules is denied by provider policy.
pub async fn delete_departure_schedule(
    rivian_vehicle_id: &str,
    vehicle_id: Uuid,
    rivian_schedule_id: &str,
    pool: &PgPool,
    client: &reqwest::Client,
    tokens: &RivianTokenBundle,
) -> Result<()> {
    let _ = (
        rivian_vehicle_id,
        vehicle_id,
        rivian_schedule_id,
        pool,
        client,
        tokens,
    );
    Err(crate::services::outbound_policy::PolicyError::VehicleWrite.into())
}

pub async fn delete_departure_schedule_for_vehicle(
    vehicle_id: Uuid,
    rivian_schedule_id: &str,
    pool: &PgPool,
    client: &reqwest::Client,
    age_key: &str,
) -> Result<()> {
    let (provider_id, tokens) = load_vehicle_tokens(vehicle_id, pool, age_key).await?;
    delete_departure_schedule(
        &provider_id,
        vehicle_id,
        rivian_schedule_id,
        pool,
        client,
        &tokens,
    )
    .await
}

// ── Poll task entry points ────────────────────────────────────────────────────

// ── Startup / periodic polls ──────────────────────────────────────────────────

/// Run all one-shot startup polls for a vehicle.  Errors are logged but not
/// fatal — a failure in one poll must not prevent others from running.
///
/// If `history_backfilled_at IS NULL` (first time this vehicle's worker has
/// ever run), a full charge-history backfill is performed that also inserts
/// sessions sourced entirely from the Rivian API.  On subsequent starts only
/// the summary response is reconciled with the recent-insert policy.
pub async fn run_startup_polls(
    vehicle_id: Uuid,
    user_id: Uuid,
    pool: PgPool,
    client: reqwest::Client,
    age_key: String,
    baseline_tx: mpsc::Sender<WsInboundEvent>,
) {
    execution::VehiclePoll {
        vehicle_id,
        pool: &pool,
        client: &client,
        age_key: &age_key,
    }
    .startup(user_id, &baseline_tx)
    .await;
}

/// Periodic and event-driven poll loop.
///
/// - Watches `power_state_rx` so it can adapt cadence with [`poll_interval`].
/// - Charging changes wake cadence evaluation; history runs after 5 minutes
///   in Ready/Charging power states or 30 minutes in other states.
pub(crate) struct PollLoopSignals {
    pub power_state_rx: tokio::sync::watch::Receiver<Option<crate::models::telemetry::PowerState>>,
    pub charging_rx: tokio::sync::watch::Receiver<bool>,
    pub shutdown: tokio::sync::broadcast::Receiver<()>,
}

pub(crate) async fn run_poll_loop(
    vehicle_id: Uuid,
    pool: PgPool,
    client: reqwest::Client,
    age_key: String,
    signals: PollLoopSignals,
    _redis: redis::Client,
) {
    execution::VehiclePoll {
        vehicle_id,
        pool: &pool,
        client: &client,
        age_key: &age_key,
    }
    .periodic(signals)
    .await;
}

#[cfg(test)]
mod tests {
    use super::{
        live_session_redis_key, record_charge_payload_with_ref, unchanged_linked_session,
        ChargePayloadRecordOutcome, LIVE_SESSION_HISTORY_QUERY, LIVE_SESSION_TTL_SECONDS,
    };
    use crate::ingestion::parser::parse_charging_session_message;
    use crate::ingestion::session_store::{decrypt_tokens, encrypt_tokens, RivianTokenBundle};
    use crate::services::charge_payload_identity::reconcile_retained_payload_references;
    use crate::services::charge_sessions::{
        infer_is_rivian_network, normalize_api_charger_type, ChargeSessionPayloadRef,
    };
    use age::secrecy::ExposeSecret;
    use axum::{extract::State, routing::post, Json, Router};
    use chrono::Utc;
    use serde_json::json;
    use sqlx::PgPool;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    fn recorded_payload(outcome: ChargePayloadRecordOutcome) -> (ChargeSessionPayloadRef, bool) {
        match outcome {
            ChargePayloadRecordOutcome::Recorded {
                payload_ref,
                repaired,
            } => (payload_ref, repaired),
            ChargePayloadRecordOutcome::EvidenceExpired => {
                panic!("expected retained charge-payload evidence")
            }
        }
    }

    pub(super) struct DisposableDatabase {
        fixture: crate::services::charge_sessions::payload_reconciliation_fixture::Fixture,
        pub(super) pool: PgPool,
    }

    impl DisposableDatabase {
        pub(super) async fn new() -> Self {
            let fixture =
                crate::services::charge_sessions::payload_reconciliation_fixture::Fixture::new()
                    .await;
            Self {
                pool: fixture.pool.clone(),
                fixture,
            }
        }

        pub(super) async fn cleanup(self) {
            self.fixture.cleanup().await;
        }
    }

    pub(super) async fn seed_vehicle(pool: &PgPool) -> uuid::Uuid {
        let user_id: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO panda.users (email, password_hash) VALUES ($1, 'test-only') RETURNING id",
        )
        .bind(format!("charge-test-{}@example.test", uuid::Uuid::new_v4()))
        .fetch_one(pool)
        .await
        .expect("seed test user");
        sqlx::query_scalar(
            "INSERT INTO panda.vehicles (user_id, rivian_vehicle_id, model, name) VALUES ($1, $2, 'test', 'Charge test') RETURNING id",
        )
        .bind(user_id)
        .bind(format!("charge-test-{}", uuid::Uuid::new_v4()))
        .fetch_one(pool)
        .await
        .expect("seed test vehicle")
    }

    async fn payload_count(pool: &PgPool, vehicle_id: uuid::Uuid) -> i64 {
        sqlx::query_scalar(
            "SELECT count(*) FROM panda.rivian_charge_payloads WHERE vehicle_id = $1",
        )
        .bind(vehicle_id)
        .fetch_one(pool)
        .await
        .expect("count payloads")
    }

    async fn seed_charge_session(pool: &PgPool, vehicle_id: uuid::Uuid) -> uuid::Uuid {
        sqlx::query_scalar(
            "INSERT INTO panda.charge_sessions (vehicle_id, started_at)
             VALUES ($1, now()) RETURNING id",
        )
        .bind(vehicle_id)
        .fetch_one(pool)
        .await
        .expect("seed charge session")
    }

    #[tokio::test]
    #[ignore = "requires a disposable TimescaleDB DATABASE_URL with CREATEDB permission"]
    async fn concurrent_auth_rejections_refresh_once_without_stale_overwrite() {
        let db = DisposableDatabase::new().await;
        let vehicle_id = seed_vehicle(&db.pool).await;
        let identity = age::x25519::Identity::generate();
        let age_key = identity.to_string().expose_secret().to_owned();
        let rejected = RivianTokenBundle {
            access_token: "access".into(),
            refresh_token: "refresh".into(),
            app_session_token: "rejected-app".into(),
            user_session_token: "user".into(),
            csrf_token: "rejected-csrf".into(),
            created_at: Utc::now(),
        };
        let encrypted = encrypt_tokens(&rejected, &identity).unwrap();
        sqlx::query(
            "INSERT INTO panda.vehicle_credentials(vehicle_id,encrypted_tokens,token_created_at) VALUES($1,$2,$3)",
        )
        .bind(vehicle_id)
        .bind(encrypted)
        .bind(rejected.created_at)
        .execute(&db.pool)
        .await
        .unwrap();

        let refresh_count = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route(
                "/graphql",
                post(
                    |State(count): State<Arc<AtomicUsize>>, Json(_body): Json<serde_json::Value>| async move {
                        count.fetch_add(1, Ordering::SeqCst);
                        Json(json!({"data":{"createCsrfToken":{"csrfToken":"fresh-csrf","appSessionToken":"fresh-app"}}}))
                    },
                ),
            )
            .with_state(refresh_count.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        crate::services::outbound_policy::with_mock_gateway(
            format!("http://{address}/graphql"),
            async {
                let client = reqwest::Client::new();
                let (left, right) = tokio::join!(
                    super::try_refresh_csrf(vehicle_id, &rejected, &client, &db.pool, &age_key),
                    super::try_refresh_csrf(vehicle_id, &rejected, &client, &db.pool, &age_key),
                );
                let left = left.unwrap();
                let right = right.unwrap();
                assert_eq!(refresh_count.load(Ordering::SeqCst), 1);
                assert_eq!(left.csrf_token, "fresh-csrf");
                assert_eq!(right.csrf_token, "fresh-csrf");

                let persisted = sqlx::query_scalar::<_, Vec<u8>>(
                    "SELECT encrypted_tokens FROM panda.vehicle_credentials WHERE vehicle_id=$1",
                )
                .bind(vehicle_id)
                .fetch_one(&db.pool)
                .await
                .unwrap();
                let persisted = decrypt_tokens(&persisted, &identity).unwrap();
                assert_eq!(persisted.csrf_token, "fresh-csrf");
                assert_eq!(persisted.app_session_token, "fresh-app");
            },
        )
        .await;
        server.abort();
        db.cleanup().await;
    }

    #[tokio::test]
    #[ignore = "requires a disposable TimescaleDB DATABASE_URL with CREATEDB permission"]
    async fn record_charge_payload_reuses_a_valid_canonical() {
        let db = DisposableDatabase::new().await;
        let vehicle_id = seed_vehicle(&db.pool).await;
        let payload = serde_json::json!({"state":"complete","energy":12.5});
        let first = recorded_payload(
            record_charge_payload_with_ref(
                &db.pool,
                vehicle_id,
                "history",
                Some("tx-1"),
                Some("rv-1"),
                payload.clone(),
            )
            .await
            .unwrap(),
        );
        let second = recorded_payload(
            record_charge_payload_with_ref(
                &db.pool,
                vehicle_id,
                "history",
                Some("tx-1"),
                Some("rv-1"),
                payload,
            )
            .await
            .unwrap(),
        );
        assert!(!first.0.unchanged);
        assert!(!first.1);
        assert!(second.0.unchanged);
        assert_eq!(first.0.payload_id, second.0.payload_id);
        assert_eq!(payload_count(&db.pool, vehicle_id).await, 1);
        db.cleanup().await;
    }

    #[tokio::test]
    #[ignore = "requires a disposable TimescaleDB DATABASE_URL with CREATEDB permission"]
    async fn record_charge_payload_repairs_an_orphan_survivor_without_duplicate() {
        let db = DisposableDatabase::new().await;
        let vehicle_id = seed_vehicle(&db.pool).await;
        let payload = serde_json::json!({"state":"orphan"});
        let orphaned = recorded_payload(
            record_charge_payload_with_ref(
                &db.pool,
                vehicle_id,
                "history",
                Some("tx-orphan"),
                Some("rv-1"),
                payload.clone(),
            )
            .await
            .unwrap(),
        )
        .0;
        let survivor = uuid::Uuid::new_v4();
        sqlx::query("INSERT INTO panda.rivian_charge_payloads (id, vehicle_id, operation, rivian_transaction_id, rivian_vehicle_id, payload) VALUES ($1,$2,'history','tx-orphan','rv-1',$3)")
            .bind(survivor).bind(vehicle_id).bind(&payload).execute(&db.pool).await.unwrap();
        let alias_session_id = seed_charge_session(&db.pool, vehicle_id).await;
        sqlx::query(
            "INSERT INTO panda.charge_session_external_aliases
             (charge_session_id, external_id, alias_kind, latest_payload_id, latest_payload_captured_at)
             VALUES ($1, 'orphan-alias', 'test', $2, now())",
        )
        .bind(alias_session_id)
        .bind(orphaned.payload_id)
        .execute(&db.pool)
        .await
        .unwrap();
        sqlx::query("DELETE FROM panda.rivian_charge_payloads WHERE id = $1")
            .bind(orphaned.payload_id)
            .execute(&db.pool)
            .await
            .unwrap();
        let result = recorded_payload(
            record_charge_payload_with_ref(
                &db.pool,
                vehicle_id,
                "history",
                Some("tx-orphan"),
                Some("rv-1"),
                payload,
            )
            .await
            .unwrap(),
        );
        assert!(result.1);
        assert_eq!(result.0.payload_id, survivor);
        assert_eq!(payload_count(&db.pool, vehicle_id).await, 1);
        let canonical: uuid::Uuid = sqlx::query_scalar(
            "SELECT canonical_payload_id FROM panda.rivian_charge_payload_identities",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(canonical, survivor);
        let alias_payload_id: Option<uuid::Uuid> = sqlx::query_scalar(
            "SELECT latest_payload_id FROM panda.charge_session_external_aliases
             WHERE charge_session_id = $1 AND external_id = 'orphan-alias'",
        )
        .bind(alias_session_id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(alias_payload_id, Some(survivor));
        db.cleanup().await;
    }

    #[tokio::test]
    #[ignore = "requires a disposable TimescaleDB DATABASE_URL with CREATEDB permission"]
    async fn record_charge_payload_tombstones_expired_evidence_without_replacement() {
        let db = DisposableDatabase::new().await;
        let vehicle_id = seed_vehicle(&db.pool).await;
        let payload = serde_json::json!({"state":"replaced"});
        let first = recorded_payload(
            record_charge_payload_with_ref(
                &db.pool,
                vehicle_id,
                "history",
                Some("tx-replaced"),
                Some("rv-1"),
                payload.clone(),
            )
            .await
            .unwrap(),
        )
        .0;
        sqlx::query("DELETE FROM panda.rivian_charge_payloads WHERE id = $1")
            .bind(first.payload_id)
            .execute(&db.pool)
            .await
            .unwrap();
        let expired = record_charge_payload_with_ref(
            &db.pool,
            vehicle_id,
            "history",
            Some("tx-replaced"),
            Some("rv-1"),
            payload,
        )
        .await
        .unwrap();
        assert!(matches!(
            expired,
            ChargePayloadRecordOutcome::EvidenceExpired
        ));
        assert_eq!(payload_count(&db.pool, vehicle_id).await, 0);
        let canonical: Option<uuid::Uuid> = sqlx::query_scalar(
            "SELECT canonical_payload_id FROM panda.rivian_charge_payload_identities",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(canonical, None);
        db.cleanup().await;
    }

    #[tokio::test]
    #[ignore = "requires a disposable TimescaleDB DATABASE_URL with CREATEDB permission"]
    async fn record_charge_payload_concurrent_calls_converge_on_one_row() {
        let db = DisposableDatabase::new().await;
        let vehicle_id = seed_vehicle(&db.pool).await;
        let payload = serde_json::json!({"state":"concurrent"});
        let calls = (0..8).map(|_| {
            record_charge_payload_with_ref(
                &db.pool,
                vehicle_id,
                "history",
                Some("tx-concurrent"),
                Some("rv-1"),
                payload.clone(),
            )
        });
        let results = futures::future::join_all(calls).await;
        let ids: std::collections::HashSet<_> = results
            .into_iter()
            .map(|result| recorded_payload(result.unwrap()).0.payload_id)
            .collect();
        assert_eq!(ids.len(), 1);
        assert_eq!(payload_count(&db.pool, vehicle_id).await, 1);
        db.cleanup().await;
    }

    #[tokio::test]
    #[ignore = "requires a disposable TimescaleDB DATABASE_URL with CREATEDB permission"]
    async fn concurrent_expired_payload_replay_never_recreates_json_evidence() {
        let db = DisposableDatabase::new().await;
        let vehicle_id = seed_vehicle(&db.pool).await;
        let payload = serde_json::json!({"state":"expired-concurrent"});
        let original = recorded_payload(
            record_charge_payload_with_ref(
                &db.pool,
                vehicle_id,
                "history",
                Some("tx-expired-concurrent"),
                Some("rv-1"),
                payload.clone(),
            )
            .await
            .unwrap(),
        )
        .0;
        sqlx::query("DELETE FROM panda.rivian_charge_payloads WHERE id = $1")
            .bind(original.payload_id)
            .execute(&db.pool)
            .await
            .unwrap();

        let results = futures::future::join_all((0..8).map(|_| {
            record_charge_payload_with_ref(
                &db.pool,
                vehicle_id,
                "history",
                Some("tx-expired-concurrent"),
                Some("rv-1"),
                payload.clone(),
            )
        }))
        .await;
        assert!(results
            .into_iter()
            .all(|result| matches!(result.unwrap(), ChargePayloadRecordOutcome::EvidenceExpired)));
        assert_eq!(payload_count(&db.pool, vehicle_id).await, 0);
        db.cleanup().await;
    }

    #[tokio::test]
    #[ignore = "requires a disposable TimescaleDB DATABASE_URL with CREATEDB permission"]
    async fn retention_reconciler_clears_orphan_identity_and_alias_cache() {
        let db = DisposableDatabase::new().await;
        let vehicle_id = seed_vehicle(&db.pool).await;
        let payload = serde_json::json!({"state":"reconcile-orphan"});
        let recorded = recorded_payload(
            record_charge_payload_with_ref(
                &db.pool,
                vehicle_id,
                "history",
                Some("tx-reconcile"),
                Some("rv-1"),
                payload,
            )
            .await
            .unwrap(),
        )
        .0;
        let session_id = seed_charge_session(&db.pool, vehicle_id).await;
        sqlx::query(
            "INSERT INTO panda.charge_session_external_aliases
             (charge_session_id, external_id, alias_kind, latest_payload_id, latest_payload_captured_at)
             VALUES ($1, 'retention-orphan', 'test', $2, now())",
        )
        .bind(session_id)
        .bind(recorded.payload_id)
        .execute(&db.pool)
        .await
        .unwrap();
        sqlx::query("DELETE FROM panda.rivian_charge_payloads WHERE id = $1")
            .bind(recorded.payload_id)
            .execute(&db.pool)
            .await
            .unwrap();

        assert_eq!(
            reconcile_retained_payload_references(&db.pool, 10)
                .await
                .unwrap(),
            (1, 1)
        );
        let canonical: Option<uuid::Uuid> = sqlx::query_scalar(
            "SELECT canonical_payload_id FROM panda.rivian_charge_payload_identities",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap();
        let alias: (Option<uuid::Uuid>, Option<chrono::DateTime<Utc>>) = sqlx::query_as(
            "SELECT latest_payload_id, latest_payload_captured_at
             FROM panda.charge_session_external_aliases
             WHERE charge_session_id = $1 AND external_id = 'retention-orphan'",
        )
        .bind(session_id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(canonical, None);
        assert_eq!(alias, (None, None));
        db.cleanup().await;
    }

    #[tokio::test]
    #[ignore = "requires a disposable TimescaleDB DATABASE_URL with CREATEDB permission"]
    async fn retention_reconciler_clears_stale_alias_when_identity_keeps_a_survivor() {
        let db = DisposableDatabase::new().await;
        let vehicle_id = seed_vehicle(&db.pool).await;
        let keeper = recorded_payload(
            record_charge_payload_with_ref(
                &db.pool,
                vehicle_id,
                "history",
                Some("tx-keeper"),
                Some("rv-1"),
                serde_json::json!({"state":"keeper"}),
            )
            .await
            .unwrap(),
        )
        .0;
        let victim = uuid::Uuid::new_v4();
        sqlx::query(
            "INSERT INTO panda.rivian_charge_payloads
             (id, vehicle_id, operation, payload)
             VALUES ($1, $2, 'compaction-victim', '{\"state\":\"victim\"}'::jsonb)",
        )
        .bind(victim)
        .bind(vehicle_id)
        .execute(&db.pool)
        .await
        .unwrap();
        let session_id = seed_charge_session(&db.pool, vehicle_id).await;
        sqlx::query(
            "INSERT INTO panda.charge_session_external_aliases
             (charge_session_id, external_id, alias_kind, latest_payload_id, latest_payload_captured_at)
             VALUES ($1, 'compactor-race', 'test', $2, now())",
        )
        .bind(session_id)
        .bind(victim)
        .execute(&db.pool)
        .await
        .unwrap();
        sqlx::query("DELETE FROM panda.rivian_charge_payloads WHERE id = $1")
            .bind(victim)
            .execute(&db.pool)
            .await
            .unwrap();

        assert_eq!(
            reconcile_retained_payload_references(&db.pool, 10)
                .await
                .unwrap(),
            (0, 1)
        );
        let canonical: Option<uuid::Uuid> = sqlx::query_scalar(
            "SELECT canonical_payload_id FROM panda.rivian_charge_payload_identities",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap();
        let alias: (Option<uuid::Uuid>, Option<chrono::DateTime<Utc>>) = sqlx::query_as(
            "SELECT latest_payload_id, latest_payload_captured_at
             FROM panda.charge_session_external_aliases
             WHERE charge_session_id = $1 AND external_id = 'compactor-race'",
        )
        .bind(session_id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(canonical, Some(keeper.payload_id));
        assert_eq!(alias, (None, None));
        db.cleanup().await;
    }

    #[tokio::test]
    #[ignore = "requires a disposable TimescaleDB DATABASE_URL with CREATEDB permission"]
    async fn retention_reference_migration_preserves_the_ninety_day_policy() {
        let db = DisposableDatabase::new().await;
        let nullable: String = sqlx::query_scalar(
            "SELECT is_nullable
             FROM information_schema.columns
             WHERE table_schema = 'panda'
               AND table_name = 'rivian_charge_payload_identities'
               AND column_name = 'canonical_payload_id'",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap();
        let retention_is_unchanged: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                 SELECT 1
                 FROM timescaledb_information.jobs
                 WHERE hypertable_schema = 'panda'
                   AND hypertable_name = 'rivian_charge_payloads'
                   AND proc_name = 'policy_retention'
                   AND (config ->> 'drop_after')::interval = interval '90 days'
             )",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(nullable, "YES");
        assert!(retention_is_unchanged);
        db.cleanup().await;
    }

    #[test]
    fn live_session_redis_contract_uses_expected_key_and_ttl() {
        let vehicle_id = uuid::Uuid::parse_str("76d88de5-f6fa-41cb-b560-126defb7885b").unwrap();
        assert_eq!(
            live_session_redis_key(vehicle_id),
            "vehicle:76d88de5-f6fa-41cb-b560-126defb7885b:live_session"
        );
        assert_eq!(LIVE_SESSION_TTL_SECONDS, 120);
    }

    #[test]
    fn charging_session_normalization_keeps_observed_values_and_converts_seconds() {
        let event = parse_charging_session_message(
            r#"{"type":"next","payload":{"data":{"chargingSession":{"chartData":[{"soc":66.0,"powerKW":9.5,"startTime":"2026-08-18T12:00:00Z"}],"liveData":{"powerKW":9.5,"totalChargedEnergy":12.5,"rangeAddedThisSession":44.0,"timeRemaining":120,"vehicleChargerState":"charging_active","startTime":"2026-08-18T12:00:00Z"}}}}}"#,
        )
        .unwrap()
        .unwrap();

        let live = super::normalize_charging_session(&event, Utc::now()).unwrap();
        assert_eq!(live.soc_pct, Some(66.0));
        assert_eq!(live.power_kw, Some(9.5));
        assert_eq!(live.energy_kwh, Some(12.5));
        assert_eq!(live.range_added_km, Some(44.0));
        assert_eq!(live.time_remaining_min, Some(2.0));
        assert_eq!(
            live.vehicle_charger_state.as_deref(),
            Some("charging_active")
        );
    }

    #[test]
    fn empty_charging_session_frame_does_not_publish_null_snapshot() {
        let event = parse_charging_session_message(
            r#"{"type":"next","payload":{"data":{"chargingSession":{"liveData":{"powerKW":null,"kilometersChargedPerHour":null,"rangeAddedThisSession":null,"totalChargedEnergy":null,"timeElapsed":null,"timeRemaining":null,"price":null,"currency":null,"isFreeSession":null,"vehicleChargerState":null,"startTime":null},"chartData":[]}}}}"#,
        )
        .unwrap()
        .unwrap();

        assert!(super::normalize_charging_session(&event, Utc::now()).is_none());
    }

    #[test]
    fn live_history_query_uses_the_current_id_variable_contract() {
        assert!(LIVE_SESSION_HISTORY_QUERY.contains("$vehicleId: ID!"));
        assert!(!LIVE_SESSION_HISTORY_QUERY.contains("$vehicleId: String!"));
    }

    #[test]
    fn only_unchanged_payloads_with_a_live_session_link_skip_reconciliation() {
        let session_id = uuid::Uuid::new_v4();
        let base = ChargeSessionPayloadRef {
            payload_id: uuid::Uuid::new_v4(),
            captured_at: Utc::now(),
            charge_session_id: Some(session_id),
            unchanged: true,
        };

        assert_eq!(unchanged_linked_session(Some(base)), Some(session_id));
        assert_eq!(
            unchanged_linked_session(Some(ChargeSessionPayloadRef {
                unchanged: false,
                ..base
            })),
            None
        );
        assert_eq!(
            unchanged_linked_session(Some(ChargeSessionPayloadRef {
                charge_session_id: None,
                ..base
            })),
            None
        );
    }

    #[test]
    fn normalizes_documented_charger_types() {
        assert_eq!(normalize_api_charger_type(Some("wallbox")), Some("ac"));
        assert_eq!(normalize_api_charger_type(Some("Level2")), Some("ac"));
        assert_eq!(normalize_api_charger_type(Some("dcfc")), Some("dc"));
        assert_eq!(normalize_api_charger_type(Some("mystery")), None);
    }

    // ── Regression: home-session inference bug ──────────────────────────────
    // The previous version inferred `is_rivian_network = Some(true)` whenever
    // `isRoamingNetwork=false` AND `isHomeCharger` was null/false — which
    // mis-flagged home AC sessions as paid Rivian-network rows and let the
    // cost service substitute `rivian_paid_total` for the real profile cost.
    // The fix: infer ONLY from an explicit known vendor string.

    #[test]
    fn home_session_with_null_vendor_is_not_rivian_network() {
        // The exact shape Rivian returns for a typical home charger: no vendor
        // tag, no roaming/home-charger flags set.
        assert_eq!(infer_is_rivian_network(None), None);
    }

    #[test]
    fn known_paid_networks_are_identified() {
        assert_eq!(infer_is_rivian_network(Some("Rivian")), Some(true));
        assert_eq!(
            infer_is_rivian_network(Some("Electrify America")),
            Some(true)
        );
        assert_eq!(infer_is_rivian_network(Some("EVgo")), Some(true));
        assert_eq!(infer_is_rivian_network(Some("ChargePoint")), Some(true));
        assert_eq!(infer_is_rivian_network(Some("Tesla")), Some(true));
    }

    #[test]
    fn unknown_vendor_is_not_a_paid_network() {
        // Defensive: an unrecognized vendor name should not be assumed paid.
        assert_eq!(
            infer_is_rivian_network(Some("Some Random EVSE")),
            Some(false)
        );
    }

    #[test]
    fn vendor_matching_is_case_insensitive() {
        assert_eq!(infer_is_rivian_network(Some("RIVIAN")), Some(true));
        assert_eq!(infer_is_rivian_network(Some("rivian")), Some(true));
    }
}

#[cfg(test)]
#[path = "rivian_poll/contract_tests.rs"]
mod contract_tests;

#[cfg(test)]
#[path = "__tests__/payload_reconciliation_contract.rs"]
mod payload_reconciliation_contract;

use crate::ingestion::{
    AcquisitionMerge as MergeWrite, AcquisitionValue as Stored, AcquisitionWrite,
};

#[derive(Clone, Copy)]
enum VehicleRead {
    Enrichment,
    Wallboxes(Uuid),
    History,
    LiveHistory(Option<Uuid>),
    Schedule,
}

impl VehicleRead {
    async fn acquire(
        self,
        vehicle_id: Uuid,
        pool: &PgPool,
        client: &reqwest::Client,
        age_key: &str,
    ) -> Result<usize> {
        let operation = match self {
            Self::Enrichment => "fetch_vehicle_enrichment",
            Self::Wallboxes(_) => "fetch_wallboxes",
            Self::History => "fetch_charge_history",
            Self::LiveHistory(_) => "fetch_live_session_history",
            Self::Schedule => "fetch_charging_schedule",
        };
        if matches!(self, Self::History) {
            record_charge_history_sync_started(pool, vehicle_id).await;
        }
        let result = with_vehicle_auth_retry(
            vehicle_id,
            pool,
            client,
            age_key,
            operation,
            move |provider_id, tokens, pool, client| {
                Box::pin(async move {
                    match self {
                        Self::Enrichment => {
                            fetch_vehicle_enrichment(provider_id, vehicle_id, pool, client, tokens)
                                .await
                                .map(|()| 0)
                        }
                        Self::Wallboxes(owner) => {
                            fetch_wallboxes(owner, vehicle_id, pool, client, tokens)
                                .await
                                .map(|()| 0)
                        }
                        Self::History => {
                            fetch_charge_history(provider_id, vehicle_id, pool, client, tokens)
                                .await
                        }
                        Self::LiveHistory(active) => {
                            fetch_live_session_history(
                                provider_id,
                                vehicle_id,
                                active,
                                pool,
                                client,
                                tokens,
                            )
                            .await
                        }
                        Self::Schedule => {
                            fetch_charging_schedule(provider_id, vehicle_id, pool, client, tokens)
                                .await
                                .map(|()| 0)
                        }
                    }
                })
            },
        )
        .await;
        if matches!(self, Self::History) && result.is_ok() {
            record_charge_history_sync_succeeded(pool, vehicle_id).await;
        }
        result
    }
}

#[cfg(test)]
mod full_replacement_tests {
    use super::*;
    #[tokio::test]
    #[ignore = "requires unique restricted synthetic PAYLOAD_DATABASE_URL"]
    async fn live_projection_preserves_peak_floor_nulls_rounding_and_closed_guard() {
        let db = tests::DisposableDatabase::new().await;
        let vehicle = tests::seed_vehicle(&db.pool).await;
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO panda.charge_sessions(id,vehicle_id,started_at,source,live_current_currency) VALUES($1,$2,now(),'rivian_api','USD')")
            .bind(id).bind(vehicle).execute(&db.pool).await.unwrap();
        let mut live = LiveSessionData {
            soc_pct: None,
            power_kw: Some(-2.0),
            energy_kwh: Some(1.25),
            range_added_km: None,
            time_remaining_min: Some(1.5),
            charger_type: None,
            ts: Utc::now(),
            charge_rate_kph: None,
            time_elapsed_seconds: Some(90.5),
            price: None,
            currency: None,
            is_free_session: None,
            vehicle_charger_state: None,
            started_at: None,
        };
        persist_live_session_data(&db.pool, Some(id), &live)
            .await
            .unwrap();
        type LiveState = (
            Option<f64>,
            Option<String>,
            Option<i32>,
            Option<i32>,
            Option<i32>,
            Option<String>,
            Option<String>,
        );
        let result: LiveState = sqlx::query_as(
            "SELECT max_charge_rate_kw,live_current_currency,live_time_elapsed_seconds,duration_minutes,live_time_remaining_minutes,source,data_confidence FROM panda.charge_sessions WHERE id=$1")
            .bind(id).fetch_one(&db.pool).await.unwrap();
        assert_eq!(
            result,
            (
                Some(0.0),
                Some("USD".into()),
                Some(91),
                Some(2),
                Some(2),
                Some("telemetry+rivian_api".into()),
                Some("telemetry_enriched".into())
            )
        );
        live.power_kw = Some(f64::NAN);
        persist_live_session_data(&db.pool, Some(id), &live)
            .await
            .unwrap();
        let peak: f64 =
            sqlx::query_scalar("SELECT max_charge_rate_kw FROM panda.charge_sessions WHERE id=$1")
                .bind(id)
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert!(peak.is_nan());
        sqlx::query("UPDATE panda.charge_sessions SET ended_at=now() WHERE id=$1")
            .bind(id)
            .execute(&db.pool)
            .await
            .unwrap();
        live.energy_kwh = Some(999.0);
        persist_live_session_data(&db.pool, Some(id), &live)
            .await
            .unwrap();
        let energy: f64 =
            sqlx::query_scalar("SELECT kwh_added FROM panda.charge_sessions WHERE id=$1")
                .bind(id)
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert_eq!(energy, 1.25);
        db.cleanup().await;
    }
}
