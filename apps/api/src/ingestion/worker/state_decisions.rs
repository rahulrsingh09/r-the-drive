//! Pure source, routing and protocol-health decisions for inbound envelopes.
//!
//! Keeping these mappings separate from database writers makes protocol-state
//! changes directly testable without constructing a worker or database pool.

use crate::ingestion::ws_client::WsInboundEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SampleSource {
    Socket,
    Baseline,
    Parallax,
}

impl SampleSource {
    pub(super) fn of(inbound: &WsInboundEvent) -> Self {
        match inbound.message_type.as_deref() {
            Some(kind) if kind.starts_with("parallax:") => Self::Parallax,
            Some("baseline") => Self::Baseline,
            _ => Self::Socket,
        }
    }

    pub(super) fn is_parallax(self) -> bool {
        self == Self::Parallax
    }
    pub(super) fn accounts_for_socket(self) -> bool {
        self == Self::Socket
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EnvelopeRoute {
    Projection,
    Heartbeat,
    Telemetry,
}

impl EnvelopeRoute {
    pub(super) fn of(inbound: &WsInboundEvent) -> Self {
        match (&inbound.telemetry, inbound.kind) {
            (None, _) => Self::Projection,
            (Some(_), crate::ingestion::ws_client::WsInboundKind::Heartbeat) => Self::Heartbeat,
            (Some(_), _) => Self::Telemetry,
        }
    }
}

pub(super) fn is_synthetic_control(message_type: Option<&str>) -> bool {
    matches!(
        message_type,
        Some(
            "connection_open"
                | "connection_ack"
                | "connection_init"
                | "subscribe"
                | "reconnect"
                | "ws_handshake_rejected"
                | "ws_schema_rejected"
                | "ws_schema_degraded"
                | "ws_no_active_subscriptions"
        )
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RuntimeHealthUpdate {
    pub(super) online: bool,
    pub(super) worker_health: &'static str,
    pub(super) worker_health_msg: String,
    pub(super) auth_state: &'static str,
    pub(super) auth_reason_code: Option<&'static str>,
}

pub(super) fn runtime_health_update_for_ws_control(
    inbound: &WsInboundEvent,
) -> Option<RuntimeHealthUpdate> {
    let kind = inbound.message_type.as_deref()?;
    let (online, health, fallback, reason, use_detail) = match kind {
        "connection_open" => (
            true,
            "starting",
            "Rivian telemetry socket opened; waiting for authentication",
            None,
            false,
        ),
        "connection_ack" => (true, "connected", "", None, false),
        "reconnect" => (
            false,
            "degraded",
            "Rivian telemetry connection interrupted; reconnecting",
            None,
            false,
        ),
        "ws_handshake_rejected" => (
            false,
            "error",
            "Rivian WS handshake rejected",
            Some("rivian_ws_handshake_rejected"),
            true,
        ),
        "ws_schema_rejected" => (
            false,
            "degraded",
            "Rivian WS VehicleState schema rejected",
            Some("rivian_ws_schema_rejected"),
            true,
        ),
        "ws_schema_degraded" => (
            false,
            "degraded",
            "Rivian WS subscription degraded to recover from schema drift",
            Some("rivian_ws_schema_rejected"),
            false,
        ),
        "ws_no_active_subscriptions" => (
            false,
            "degraded",
            "Rivian WS reported no active subscriptions; reconnecting",
            Some("rivian_ws_no_active_subscriptions"),
            true,
        ),
        _ => return None,
    };
    let denied = kind == "ws_handshake_rejected" && ws_handshake_auth_rejected(&inbound.raw);
    let message = use_detail
        .then(|| read_ws_detail_message(&inbound.raw))
        .flatten()
        .unwrap_or_else(|| fallback.to_owned());
    Some(RuntimeHealthUpdate {
        online,
        worker_health: health,
        worker_health_msg: message,
        auth_state: if denied { "needs_reauth" } else { "authorized" },
        auth_reason_code: if denied {
            Some("rivian_ws_auth_rejected")
        } else {
            reason
        },
    })
}

fn read_ws_detail_message(raw: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(raw).ok()?;
    value
        .get("reason")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

fn ws_handshake_auth_rejected(raw: &str) -> bool {
    let value = serde_json::from_str::<serde_json::Value>(raw).ok();
    matches!(
        value
            .as_ref()
            .and_then(|value| value.get("http_status"))
            .and_then(serde_json::Value::as_u64),
        Some(401 | 403)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingestion::ws_client::WsInboundKind;
    use chrono::Utc;

    fn control(message_type: &str, raw: serde_json::Value) -> WsInboundEvent {
        WsInboundEvent {
            kind: WsInboundKind::Control,
            received_at: Utc::now(),
            raw: raw.to_string(),
            message_type: Some(message_type.into()),
            telemetry: None,
            charging_session: None,
            battery_cell_type: None,
        }
    }

    #[test]
    fn envelope_routing_keeps_source_and_payload_policy_independent() {
        for (message, source) in [
            ("next", SampleSource::Socket),
            ("baseline", SampleSource::Baseline),
            ("parallax:power", SampleSource::Parallax),
            ("Parallax:power", SampleSource::Socket),
        ] {
            for kind in [
                WsInboundKind::Control,
                WsInboundKind::Heartbeat,
                WsInboundKind::Telemetry,
                WsInboundKind::ChargingSession,
            ] {
                let mut inbound = control(message, serde_json::Value::Null);
                inbound.kind = kind;
                assert_eq!(SampleSource::of(&inbound), source);
                assert_eq!(EnvelopeRoute::of(&inbound), EnvelopeRoute::Projection);
                inbound.telemetry = Some(crate::models::telemetry::TelemetryEvent::empty(
                    uuid::Uuid::nil(),
                    Utc::now(),
                ));
                assert_eq!(
                    EnvelopeRoute::of(&inbound),
                    if kind == WsInboundKind::Heartbeat {
                        EnvelopeRoute::Heartbeat
                    } else {
                        EnvelopeRoute::Telemetry
                    }
                );
            }
        }
    }

    #[test]
    fn synthetic_control_allowlist_is_explicit() {
        assert!(is_synthetic_control(Some("connection_open")));
        assert!(is_synthetic_control(Some("ws_schema_degraded")));
        assert!(!is_synthetic_control(Some("vehicle_state")));
        assert!(!is_synthetic_control(None));
    }

    #[test]
    fn invalid_detail_payload_uses_stable_fallback() {
        let inbound = control("ws_handshake_rejected", serde_json::Value::Null);
        let update = runtime_health_update_for_ws_control(&inbound).expect("health update");

        assert_eq!(update.worker_health, "error");
        assert_eq!(update.worker_health_msg, "Rivian WS handshake rejected");
        assert_eq!(update.auth_state, "authorized");
    }

    #[test]
    fn rejected_credentials_require_reauthentication() {
        for status in [401, 403] {
            let inbound = control(
                "ws_handshake_rejected",
                serde_json::json!({"http_status": status}),
            );
            let update = runtime_health_update_for_ws_control(&inbound).expect("health update");

            assert_eq!(update.auth_state, "needs_reauth");
            assert_eq!(update.auth_reason_code, Some("rivian_ws_auth_rejected"));
        }
    }

    #[test]
    fn reconnect_is_non_healthy_until_connection_reopens() {
        let reconnect = control("reconnect", serde_json::json!({"backoff_seconds": 5}));
        let reconnect_update =
            runtime_health_update_for_ws_control(&reconnect).expect("reconnect health update");
        assert!(!reconnect_update.online);
        assert_eq!(reconnect_update.worker_health, "degraded");

        let opened = control("connection_open", serde_json::Value::Null);
        let opened_update =
            runtime_health_update_for_ws_control(&opened).expect("connection health update");
        assert!(opened_update.online);
        assert_eq!(opened_update.worker_health, "starting");

        let acknowledged = control("connection_ack", serde_json::Value::Null);
        let acknowledged_update =
            runtime_health_update_for_ws_control(&acknowledged).expect("ack health update");
        assert!(acknowledged_update.online);
        assert_eq!(acknowledged_update.worker_health, "connected");
    }
}
