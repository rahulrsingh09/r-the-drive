use super::*;
use std::{
    collections::VecDeque,
    pin::Pin,
    task::{Context, Poll},
};

#[derive(Default)]
struct ScriptedSocket {
    incoming: VecDeque<Result<Message, WsError>>,
    outgoing: Vec<Message>,
    pending: bool,
}

impl futures::Stream for ScriptedSocket {
    type Item = Result<Message, WsError>;
    fn poll_next(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.incoming.pop_front() {
            Some(frame) => Poll::Ready(Some(frame)),
            None if self.pending => Poll::Pending,
            None => Poll::Ready(None),
        }
    }
}
impl futures::Sink<Message> for ScriptedSocket {
    type Error = WsError;
    fn poll_ready(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), WsError>> {
        Poll::Ready(Ok(()))
    }
    fn start_send(mut self: Pin<&mut Self>, frame: Message) -> Result<(), WsError> {
        self.outgoing.push(frame);
        Ok(())
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), WsError>> {
        Poll::Ready(Ok(()))
    }
    fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), WsError>> {
        Poll::Ready(Ok(()))
    }
}
fn tokens() -> RivianTokenBundle {
    RivianTokenBundle {
        access_token: "synthetic".into(),
        refresh_token: "synthetic".into(),
        app_session_token: "app".into(),
        user_session_token: "user".into(),
        csrf_token: "csrf".into(),
        created_at: Utc::now(),
    }
}
fn text(value: Value) -> Result<Message, WsError> {
    Ok(Message::Text(value.to_string().into()))
}
async fn drive(socket: &mut ScriptedSocket) -> (anyhow::Result<WsLoopEnd>, Vec<WsInboundEvent>) {
    let (tx, mut rx) = mpsc::channel(64);
    let (_shutdown, mut shutdown) = tokio::sync::broadcast::channel(1);
    let query = VehicleStateSubscriptionHealth::default().build_query();
    let result = drive_socket(
        socket,
        &Uuid::nil(),
        "synthetic-vehicle",
        &tokens(),
        &tx,
        &mut shutdown,
        WsSubscription {
            query: &query,
            disabled_field_count: 4,
        },
    )
    .await;
    drop(tx);
    let mut events = Vec::new();
    while let Some(event) = rx.recv().await {
        events.push(event);
    }
    (result, events)
}

#[tokio::test]
async fn provider_contract_socket_handshake_requests_and_root_routing() {
    let mut socket = ScriptedSocket {
        incoming: VecDeque::from([
            Ok(Message::Ping(vec![1, 2].into())),
            text(json!({"type":"connection_ack"})),
            text(json!({"type":"next","id":"1","payload":{"data":{"chargingSession":null}}})),
            text(
                json!({"type":"next","id":"2","payload":{"data":{"vehicleState":{"batteryLevel":{"value":50,"timeStamp":"2026-10-16T00:00:00Z"}}}}}),
            ),
            text(json!({"type":"error","id":"2","payload":[{"message":"charging unavailable"}]})),
            text(
                json!({"type":"next","payload":{"data":{"vehicleState":{"cloudConnection":{"isOnline":true}}}}}),
            ),
            Ok(Message::Text("not json".into())),
        ]),
        ..Default::default()
    };
    let (result, events) = drive(&mut socket).await;
    assert_eq!(result.unwrap_err().to_string(), "WS closed");
    assert_eq!(socket.outgoing.len(), 4);
    let init: Value = serde_json::from_str(socket.outgoing[0].to_text().unwrap()).unwrap();
    assert_eq!(init["type"], "connection_init");
    assert_eq!(init["payload"]["u-sess"], "user");
    assert_eq!(init["payload"]["client-version"], "1.13.0-1494");
    assert!(matches!(&socket.outgoing[1], Message::Pong(data) if data.as_ref() == [1,2]));
    for (index, id, operation, query) in [
        (
            2,
            "1",
            "vehicleState",
            VehicleStateSubscriptionHealth::default().build_query(),
        ),
        (
            3,
            "2",
            "chargingSession",
            CHARGING_SESSION_SUBSCRIPTION.into(),
        ),
    ] {
        let sub: Value = serde_json::from_str(socket.outgoing[index].to_text().unwrap()).unwrap();
        assert_eq!(
            sub,
            json!({"id":id,"type":"subscribe","payload":{"operationName":operation,"query":query,"variables":{"vehicleID":"synthetic-vehicle"}}})
        );
    }
    assert_eq!(
        events.iter().map(|e| e.kind).collect::<Vec<_>>(),
        vec![
            WsInboundKind::Control,
            WsInboundKind::Control,
            WsInboundKind::Control,
            WsInboundKind::Control,
            WsInboundKind::Control,
            WsInboundKind::ChargingSession,
            WsInboundKind::Telemetry,
            WsInboundKind::Control,
            WsInboundKind::Heartbeat,
            WsInboundKind::Control
        ]
    );
    assert_eq!(
        events[6].telemetry.as_ref().unwrap().vehicle_id,
        Uuid::nil()
    );
}

#[tokio::test]
async fn provider_contract_close_and_rejection_outcomes() {
    use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
    for (code, reason, renew) in [
        (4420, "Connection TTL expired", true),
        (4420, "other", false),
        (4410, "no subscriptions", false),
        (1000, "normal", false),
    ] {
        let mut socket = ScriptedSocket {
            incoming: VecDeque::from([
                text(json!({"type":"connection_ack"})),
                Ok(Message::Close(Some(CloseFrame {
                    code: CloseCode::from(code),
                    reason: reason.into(),
                }))),
            ]),
            ..Default::default()
        };
        let (result, events) = drive(&mut socket).await;
        if renew {
            assert_eq!(result.unwrap(), WsLoopEnd::ConnectionTtlExpired);
        } else {
            assert!(result.is_err());
        }
        assert_eq!(
            events
                .iter()
                .any(|e| e.message_type.as_deref() == Some("ws_no_active_subscriptions")),
            code == 4410
        );
    }
    for kind in ["error", "complete"] {
        let mut socket = ScriptedSocket {
            incoming: VecDeque::from([
                text(json!({"type":"connection_ack"})),
                text(
                    json!({"type":kind,"id":"1","payload":[{"message":"Cannot query field \"gnssSpeed\" on type \"VehicleState\"."}]}),
                ),
            ]),
            ..Default::default()
        };
        let (result, events) = drive(&mut socket).await;
        assert_eq!(
            result
                .unwrap_err()
                .downcast_ref::<RivianSubscriptionRejection>()
                .unwrap()
                .invalid_fields,
            vec!["gnssSpeed"]
        );
        assert_eq!(
            events.last().unwrap().message_type.as_deref(),
            Some("ws_schema_rejected")
        );
    }
}

#[test]
fn provider_contract_field_health_and_vehicle_isolation() {
    let mut left = VehicleStateSubscriptionHealth::default();
    let right = VehicleStateSubscriptionHealth::default();
    let fields = vec!["gnssSpeed".into(), "unknown".into()];
    assert!(!left.record_rejection(&Uuid::nil(), &fields));
    assert!(left.record_rejection(&Uuid::nil(), &fields));
    assert!(!left.build_query().contains("gnssSpeed"));
    assert!(right.build_query().contains("gnssSpeed"));
    assert!(!left.disabled_fields.contains("unknown"));
    assert_eq!(next_backoff_secs(u64::MAX, 900), 900);
    for base in [1, 10, 900] {
        let delay = jittered_backoff_secs(base);
        assert!((base..=base + base / 4).contains(&delay));
    }
}
