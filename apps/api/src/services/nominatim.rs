use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde_json::Value;
use sqlx::PgPool;

use crate::{errors::AppError, services::external_connections};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NominatimLane {
    InteractiveSearch,
    BackgroundReverseGeocode,
}

#[derive(Clone, Copy, Debug)]
pub struct NominatimSlot {
    pub lane: NominatimLane,
    pub queued_ms: u64,
    pub gate_wait_ms: u64,
    pub effective_interval_ms: u64,
}

#[derive(Debug)]
struct NominatimState {
    next_allowed: Instant,
    adaptive_extra_ms: u64,
}

#[derive(Debug)]
struct NominatimScheduler {
    state: tokio::sync::Mutex<NominatimState>,
    interactive_waiters: AtomicUsize,
    base_interval_ms: u64,
    max_adaptive_extra_ms: u64,
}

struct InteractiveWaiter<'a>(&'a AtomicUsize);

impl Drop for InteractiveWaiter<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl NominatimScheduler {
    fn new(base_interval_ms: u64, max_adaptive_extra_ms: u64) -> Self {
        Self {
            state: tokio::sync::Mutex::new(NominatimState {
                next_allowed: Instant::now(),
                adaptive_extra_ms: 0,
            }),
            interactive_waiters: AtomicUsize::new(0),
            base_interval_ms,
            max_adaptive_extra_ms,
        }
    }

    async fn acquire_slot(&self, lane: NominatimLane) -> NominatimSlot {
        let _waiter = (lane == NominatimLane::InteractiveSearch).then(|| {
            self.interactive_waiters.fetch_add(1, Ordering::SeqCst);
            InteractiveWaiter(&self.interactive_waiters)
        });
        let mut progress = scheduling::QueueProgress::new(lane);
        loop {
            let decision = if lane == NominatimLane::BackgroundReverseGeocode
                && self.interactive_waiters.load(Ordering::SeqCst) > 0
            {
                scheduling::Admission::YieldInteractive
            } else {
                let mut state = self.state.lock().await;
                scheduling::Admission::reserve(&mut state, Instant::now(), self.base_interval_ms)
            };
            match decision {
                scheduling::Admission::YieldInteractive => {
                    tokio::time::sleep(Duration::from_millis(50)).await
                }
                scheduling::Admission::Wait(duration) => {
                    progress.wait_for(duration);
                    tokio::time::sleep(duration).await;
                }
                scheduling::Admission::Granted(interval) => {
                    return progress.finish(interval);
                }
            }
        }
    }

    async fn report_outcome(&self, status: Option<u16>, retry_after: Option<Duration>) {
        let adjustment = scheduling::RateAdjustment::from_response(status, retry_after);
        if matches!(adjustment, scheduling::RateAdjustment::Unchanged) {
            return;
        }
        let mut state = self.state.lock().await;
        state.adaptive_extra_ms = adjustment.apply(
            state.adaptive_extra_ms,
            self.base_interval_ms,
            self.max_adaptive_extra_ms,
        );
    }

    async fn adaptive_extra_ms(&self) -> u64 {
        let state = self.state.lock().await;
        state.adaptive_extra_ms
    }
}

fn scheduler() -> &'static NominatimScheduler {
    static SCHEDULER: OnceLock<NominatimScheduler> = OnceLock::new();
    SCHEDULER.get_or_init(|| NominatimScheduler::new(1100, 15_000))
}

pub async fn acquire_slot(lane: NominatimLane) -> NominatimSlot {
    scheduler().acquire_slot(lane).await
}

pub async fn report_outcome(status: Option<u16>, retry_after: Option<Duration>) {
    scheduler().report_outcome(status, retry_after).await
}

pub async fn adaptive_extra_ms() -> u64 {
    scheduler().adaptive_extra_ms().await
}

pub fn retry_after_from_headers(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|header| header.to_str().ok())
        .and_then(|text| text.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
}

pub async fn search(
    pool: &PgPool,
    _client: &reqwest::Client,
    query: &str,
    limit: u8,
) -> Result<Vec<Value>, AppError> {
    let parameters = [
        ("format", "jsonv2".to_owned()),
        ("addressdetails", "1".to_owned()),
        ("limit", limit.clamp(1, 10).to_string()),
        ("q", query.to_owned()),
    ];
    request_address(pool, AddressRequest::Search, &parameters)
        .await?
        .ok_or_else(|| AppError::DependencyUnavailable("Address provider returned an error".into()))
}

pub async fn reverse(
    pool: &PgPool,
    _client: &reqwest::Client,
    lat: f64,
    lon: f64,
) -> Result<Option<Value>, AppError> {
    let parameters = [
        ("format", "jsonv2".to_owned()),
        ("addressdetails", "1".to_owned()),
        ("lat", lat.to_string()),
        ("lon", lon.to_string()),
    ];
    request_address(pool, AddressRequest::Reverse, &parameters).await
}

fn endpoint(
    settings: &external_connections::ConnectionSettingsRow,
    path: &str,
) -> Result<url::Url, AppError> {
    let base = settings
        .base_url
        .as_deref()
        .ok_or_else(|| AppError::Validation("Nominatim endpoint missing".into()))?;
    let normalized = format!("{}/", base.trim_end_matches('/'));
    url::Url::parse(&normalized)
        .and_then(|url| url.join(path))
        .map_err(|_| AppError::Validation("Nominatim endpoint is invalid".into()))
}

fn user_agent(settings: &external_connections::ConnectionSettingsRow) -> String {
    settings
        .request_identifier
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "R (self-hosted Rivian telemetry)".into())
}

#[derive(Clone, Copy)]
enum AddressRequest {
    Search,
    Reverse,
}

impl AddressRequest {
    fn policy(self) -> (&'static str, NominatimLane, &'static str) {
        match self {
            Self::Search => (
                "search",
                NominatimLane::InteractiveSearch,
                "Address search request failed",
            ),
            Self::Reverse => (
                "reverse",
                NominatimLane::BackgroundReverseGeocode,
                "Reverse geocoding request failed",
            ),
        }
    }
}

async fn request_address<T: serde::de::DeserializeOwned>(
    pool: &PgPool,
    request: AddressRequest,
    parameters: &[(&str, String)],
) -> Result<Option<T>, AppError> {
    use crate::services::outbound;
    let (path, lane, send_failure) = request.policy();
    let settings =
        external_connections::require_enabled(pool, external_connections::NOMINATIM).await?;
    let endpoint = endpoint(&settings, path)?;
    let slot = acquire_slot(lane).await;
    external_connections::record_attempt(pool, external_connections::NOMINATIM).await;
    let allowlist = outbound::configured_private_network_allowlist(&settings)?;
    let client = outbound::outbound_client_for_url(&endpoint, &allowlist).await?;
    let sent = client
        .get(endpoint)
        .header(reqwest::header::USER_AGENT, user_agent(&settings))
        .query(parameters)
        .send()
        .await;
    let response = match sent {
        Ok(response) => response,
        Err(_) => {
            external_connections::record_failure(
                pool,
                external_connections::NOMINATIM,
                send_failure,
            )
            .await;
            return Err(AppError::DependencyUnavailable(send_failure.into()));
        }
    };
    let status = response.status();
    report_outcome(
        Some(status.as_u16()),
        retry_after_from_headers(response.headers()),
    )
    .await;
    if status.is_success() {
        let decoded = outbound::read_json(
            response,
            outbound::operator_security()?.geocoder_max_response_bytes,
            "Address provider",
        )
        .await
        .map_err(|_| {
            AppError::DependencyUnavailable("Address provider returned invalid data".into())
        })?;
        external_connections::record_success(pool, external_connections::NOMINATIM).await;
        return Ok(Some(decoded));
    }
    external_connections::record_failure(
        pool,
        external_connections::NOMINATIM,
        &format!("HTTP {status}"),
    )
    .await;
    match request {
        AddressRequest::Search => {
            tracing::warn!(status = %status, lane = ?slot.lane, queued_ms = slot.queued_ms, gate_wait_ms = slot.gate_wait_ms, "nominatim.search_http_error");
            Err(AppError::DependencyUnavailable(
                "Address provider returned an error".into(),
            ))
        }
        AddressRequest::Reverse => {
            tracing::warn!(status = %status, lane = ?slot.lane, queued_ms = slot.queued_ms, gate_wait_ms = slot.gate_wait_ms, "nominatim.reverse_http_error");
            Ok(None)
        }
    }
}

#[cfg(test)]
#[path = "__tests__/places_profiles/scheduler.rs"]
mod places_profiles_contracts;

#[path = "nominatim/scheduling.rs"]
mod scheduling;
