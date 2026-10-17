//! Backend live-status WebSocket: JWT via Sec-WebSocket-Protocol, fan-out from Redis.

use axum::{
    extract::{
        ws::{CloseFrame, Message, WebSocket},
        Path, Query, State, WebSocketUpgrade,
    },
    response::IntoResponse,
    routing::get,
    Router,
};
use chrono::{DateTime, Utc};
use futures::{SinkExt, StreamExt};
use jsonwebtoken::{decode, Algorithm, Validation};
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    db::vehicles::require_vehicle_read_access,
    errors::AppError,
    middleware::auth::{require_vehicle_access, AppState, AuthUser, Claims},
};

mod projection;

#[cfg(test)]
#[path = "live_authorization_tests.rs"]
mod authorization_tests;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/vehicles/live", get(live_handler))
        .route("/vehicles/{id}/live-session", get(live_session_handler))
}

#[derive(Deserialize)]
struct LiveParams {
    vehicle_id: Option<Uuid>,
}

const LIVE_KEEPALIVE_MESSAGE: &str = r#"{"type":"keepalive"}"#;
const LIVE_AUTH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);
const LIVE_AUTH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

#[derive(Deserialize)]
struct LiveClientControlMessage {
    #[serde(rename = "type")]
    message_type: Option<String>,
}

fn is_live_probe(message: &str) -> bool {
    serde_json::from_str::<LiveClientControlMessage>(message)
        .ok()
        .and_then(|control| control.message_type)
        .as_deref()
        == Some("probe")
}

/// Extract and validate a JWT from the `Sec-WebSocket-Protocol: bearer.<token>` header.
/// Returns the decoded claims on success.
pub(crate) fn extract_jwt_from_headers(
    headers: &axum::http::HeaderMap,
    jwt_keys: &crate::middleware::auth::JwtKeys,
) -> Result<Claims, AppError> {
    let proto_header = headers
        .get("sec-websocket-protocol")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let token = proto_header
        .split(',')
        .map(str::trim)
        .find_map(|p| p.strip_prefix("bearer."))
        .ok_or(AppError::Unauthorized)?;

    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(&["panda.app"]);
    validation.leeway = 0;

    decode::<Claims>(token, &jwt_keys.decoding, &validation)
        .map_err(|_| AppError::Unauthorized)
        .map(|d| d.claims)
}

async fn live_handler(
    State(state): State<AppState>,
    Query(p): Query<LiveParams>,
    headers: axum::http::HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<impl IntoResponse, AppError> {
    let vid = p
        .vehicle_id
        .ok_or(AppError::Validation("vehicle_id required".into()))?;

    let claims = extract_jwt_from_headers(&headers, &state.jwt_keys)?;
    authorize_live(&state.pool, &claims, vid)
        .await
        .map_err(|_| AppError::Unauthorized)?;
    let permit = state
        .resources
        .live(claims.sub, vid, &state.config.security)?;
    Ok(ws
        .max_message_size(4096)
        .max_frame_size(4096)
        .protocols(["bearer"])
        .on_upgrade(move |socket| handle_socket(socket, vid, state, claims, permit)))
}

/// GET /v1/vehicles/{id}/live-session
/// Returns the latest live charging session data from Redis (written by run_poll_loop).
/// Returns 204 No Content when no live session is active.
async fn live_session_handler(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(vehicle_id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    require_vehicle_access(&auth, vehicle_id)?;
    require_vehicle_read_access(&state.pool, &auth, vehicle_id).await?;
    let mut connection = state.redis.get_multiplexed_async_connection().await?;
    let legacy: Option<String> = redis::AsyncCommands::get(
        &mut connection,
        format!("vehicle:{vehicle_id}:live_session"),
    )
    .await?;
    let active = ActiveLiveSession::read(&state.pool, vehicle_id).await?;
    let merged = merge_live_session(legacy, active, Utc::now());
    Ok(live_session_response(merged))
}

#[derive(sqlx::FromRow)]
struct ActiveLiveSession {
    parallax_live_power_kw: Option<f64>,
    parallax_total_charged_kwh: Option<f64>,
    parallax_pack_energy_kwh: Option<f64>,
    parallax_thermal_energy_kwh: Option<f64>,
    parallax_time_remaining_minutes: Option<i32>,
    parallax_power_observed_at: Option<DateTime<Utc>>,
    parallax_total_energy_observed_at: Option<DateTime<Utc>>,
    parallax_pack_energy_observed_at: Option<DateTime<Utc>>,
    parallax_thermal_energy_observed_at: Option<DateTime<Utc>>,
    parallax_time_observed_at: Option<DateTime<Utc>>,
}

fn merge_live_session(
    raw: Option<String>,
    active: Option<ActiveLiveSession>,
    now: DateTime<Utc>,
) -> Option<String> {
    let active = active?;
    let mut snapshot = projection::LiveSnapshot::from_legacy(raw.as_deref());
    for observation in projection::observations(active) {
        if let Some(selected) = observation.select(now) {
            snapshot.overlay(selected);
        }
    }
    snapshot.finish()
}

fn live_session_response(raw: Option<String>) -> axum::response::Response {
    let Some(raw) = raw else {
        return axum::http::StatusCode::NO_CONTENT.into_response();
    };
    let value = serde_json::from_str::<serde_json::Value>(&raw).unwrap_or_default();
    axum::Json(value).into_response()
}

async fn authorize_live(
    pool: &sqlx::PgPool,
    claims: &Claims,
    vehicle_id: Uuid,
) -> Result<(), (u16, &'static str)> {
    let check = async {
        if let Err(error) = crate::services::sessions::require_enabled_session(pool, claims).await {
            return Err(match error {
                AppError::Forbidden | AppError::Unauthorized => (4401, "Session ended"),
                _ => (1011, "Authorization unavailable"),
            });
        }
        let membership = sqlx::query_scalar::<_, i32>(
            "SELECT 1 FROM panda.vehicle_memberships m JOIN panda.vehicles v ON v.id=m.vehicle_id WHERE m.vehicle_id=$1 AND m.user_id=$2 LIMIT 1")
            .bind(vehicle_id).bind(claims.sub).fetch_optional(pool).await;
        match membership {
            Err(_) => Err((1011, "Authorization unavailable")),
            Ok(None) => Err((4403, "Vehicle access removed")),
            Ok(Some(_)) if claims.exp <= Utc::now().timestamp() => Err((4401, "Session expired")),
            Ok(Some(_)) => Ok(()),
        }
    };
    match tokio::time::timeout(LIVE_AUTH_TIMEOUT, check).await {
        Ok(result) => result,
        Err(_) => Err((1011, "Authorization unavailable")),
    }
}

enum SocketAction {
    Stop,
    Ignore,
    Close(u16, &'static str),
    Send {
        authorize: bool,
        messages: Vec<Message>,
    },
}

async fn handle_socket(
    socket: WebSocket,
    vehicle_id: Uuid,
    state: AppState,
    claims: Claims,
    _permit: crate::services::resource_limits::ResourcePermit,
) {
    let (mut sink, mut incoming) = socket.split();
    let lifetime = chrono::DateTime::from_timestamp(claims.exp, 0)
        .and_then(|expiry| (expiry - Utc::now()).to_std().ok())
        .unwrap_or_default();
    let expiry = tokio::time::Instant::now() + lifetime;
    let setup_deadline =
        || expiry.min(tokio::time::Instant::now() + std::time::Duration::from_secs(5));
    let mut subscription =
        match tokio::time::timeout_at(setup_deadline(), state.redis.get_async_pubsub()).await {
            Ok(Ok(subscription)) => subscription,
            _ => {
                tracing::error!("redis pubsub connect failed or timed out");
                return;
            }
        };
    let channel = format!("vehicle:{vehicle_id}:status");
    match tokio::time::timeout_at(setup_deadline(), subscription.subscribe(channel)).await {
        Ok(Ok(())) => {}
        _ => {
            tracing::error!("redis subscribe failed");
            return;
        }
    }
    let mut telemetry = subscription.into_on_message();
    let mut authorization_clock = tokio::time::interval(LIVE_AUTH_INTERVAL);
    authorization_clock.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        let action = tokio::select! {
            biased;
            _ = tokio::time::sleep_until(expiry) => SocketAction::Close(4401, "Session expired"),
            message = telemetry.next() => match message {
                None => SocketAction::Stop,
                Some(message) => match message.get_payload::<String>() {
                    Ok(text) => SocketAction::Send { authorize: true, messages: vec![Message::Text(text.into())] },
                    Err(_) => SocketAction::Ignore,
                }
            },
            _ = authorization_clock.tick() => SocketAction::Send { authorize: true,
                messages: vec![Message::Text(LIVE_KEEPALIVE_MESSAGE.into()), Message::Ping(Vec::new().into())] },
            message = incoming.next() => match message {
                Some(Ok(Message::Text(text))) if is_live_probe(&text) => SocketAction::Send {
                    authorize: false, messages: vec![Message::Text(LIVE_KEEPALIVE_MESSAGE.into())] },
                None | Some(Ok(Message::Close(_))) => SocketAction::Stop,
                _ => SocketAction::Ignore,
            },
        };
        match action {
            SocketAction::Stop => return,
            SocketAction::Ignore => {}
            SocketAction::Close(code, reason) => {
                close_live(&mut sink, code, reason).await;
                return;
            }
            SocketAction::Send {
                authorize,
                messages,
            } => {
                if authorize {
                    if let Err((code, reason)) =
                        authorize_live(&state.pool, &claims, vehicle_id).await
                    {
                        close_live(&mut sink, code, reason).await;
                        return;
                    }
                }
                for message in messages {
                    if !send_live(&mut sink, message, expiry).await {
                        return;
                    }
                }
            }
        }
    }
}

async fn send_live(
    sink: &mut futures::stream::SplitSink<WebSocket, Message>,
    message: Message,
    expiry: tokio::time::Instant,
) -> bool {
    let deadline = expiry.min(tokio::time::Instant::now() + std::time::Duration::from_secs(5));
    tokio::select! {
        biased;
        result = sink.send(message) => result.is_ok(),
        _ = tokio::time::sleep_until(deadline) => false,
    }
}

async fn close_live(
    sink: &mut futures::stream::SplitSink<WebSocket, Message>,
    code: u16,
    reason: &'static str,
) {
    let frame = Message::Close(Some(CloseFrame {
        code,
        reason: reason.into(),
    }));
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
    send_live(sink, frame, deadline).await;
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "__tests__/live_projection_contract.rs"]
mod live_projection_contract;

#[cfg(test)]
#[path = "__tests__/live_projection_http.rs"]
mod live_projection_http;

impl ActiveLiveSession {
    async fn read(pool: &sqlx::PgPool, vehicle_id: Uuid) -> Result<Option<Self>, AppError> {
        sqlx::query_as(
            r#"SELECT parallax_live_power_kw,parallax_total_charged_kwh,
                  parallax_pack_energy_kwh,parallax_thermal_energy_kwh,
                  parallax_time_remaining_minutes,parallax_power_observed_at,
                  parallax_total_energy_observed_at,parallax_pack_energy_observed_at,
                  parallax_thermal_energy_observed_at,parallax_time_observed_at
           FROM panda.charge_sessions
           WHERE vehicle_id=$1 AND ended_at IS NULL ORDER BY started_at DESC LIMIT 1"#,
        )
        .bind(vehicle_id)
        .fetch_optional(pool)
        .await
        .map_err(AppError::from)
    }
}
