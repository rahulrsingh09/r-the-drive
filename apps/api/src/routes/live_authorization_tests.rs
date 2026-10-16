use super::*;
use crate::authorization_test_support::Fixture;
use tokio_tungstenite::{
    tungstenite::{client::IntoClientRequest, Message as Frame},
    MaybeTlsStream, WebSocketStream,
};

type ClientSocket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

async fn server(f: &Fixture) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = f.router();
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (address, task)
}

async fn connect(
    address: std::net::SocketAddr,
    vehicle: Uuid,
    token: &str,
) -> Result<ClientSocket, tokio_tungstenite::tungstenite::Error> {
    let mut request = format!("ws://{address}/v1/vehicles/live?vehicle_id={vehicle}")
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        "sec-websocket-protocol",
        format!("bearer, bearer.{token}").parse().unwrap(),
    );
    tokio_tungstenite::connect_async(request)
        .await
        .map(|(socket, _)| socket)
}

async fn next_text(socket: &mut ClientSocket) -> String {
    tokio::time::timeout(std::time::Duration::from_secs(8), async {
        loop {
            match socket.next().await.unwrap().unwrap() {
                Frame::Text(text) => return text.to_string(),
                Frame::Ping(_) | Frame::Pong(_) => {}
                frame => panic!("expected text, got {frame:?}"),
            }
        }
    })
    .await
    .expect("timely socket message")
}

async fn assert_closed_without_telemetry(socket: &mut ClientSocket, expected_code: u16) {
    tokio::time::timeout(std::time::Duration::from_secs(8), async {
        loop {
            match socket
                .next()
                .await
                .expect("close frame")
                .expect("valid frame")
            {
                Frame::Close(Some(frame)) => {
                    assert_eq!(u16::from(frame.code), expected_code);
                    break;
                }
                Frame::Text(text) => assert_eq!(text.as_str(), LIVE_KEEPALIVE_MESSAGE),
                Frame::Ping(_) | Frame::Pong(_) => {}
                frame => panic!("unexpected frame after revocation: {frame:?}"),
            }
        }
    })
    .await
    .expect("revoked socket closes within the authorization interval");
}

async fn publish(f: &Fixture, vehicle: Uuid, payload: &str) {
    let mut conn = f
        .state
        .redis
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    let _: i64 =
        redis::AsyncCommands::publish(&mut conn, format!("vehicle:{vehicle}:status"), payload)
            .await
            .unwrap();
}

#[tokio::test]
#[ignore = "requires disposable TimescaleDB DATABASE_URL and REDIS_URL"]
async fn authorization_live_revocation_blocks_queued_telemetry() {
    let f = Fixture::new().await;
    let (address, task) = server(&f).await;
    for revoke in ["membership", "user", "disabled", "vehicle", "session"] {
        let user = f.user("user").await;
        let vehicle = f.vehicle(user, &format!("demo-live-{revoke}")).await;
        f.sessions(user, vehicle).await;
        let sid: Uuid =
            sqlx::query_scalar("SELECT id FROM panda.session_families WHERE user_id=$1")
                .bind(user)
                .fetch_one(&f.state.pool)
                .await
                .unwrap();
        let token = crate::middleware::auth::issue_session_access_token(
            user,
            Some(vehicle),
            Some(sid),
            &f.state.jwt_keys,
        )
        .unwrap();
        let mut socket = connect(address, vehicle, &token).await.unwrap();
        assert_eq!(next_text(&mut socket).await, LIVE_KEEPALIVE_MESSAGE);
        publish(&f, vehicle, r#"{"battery_level":42}"#).await;
        assert_eq!(next_text(&mut socket).await, r#"{"battery_level":42}"#);
        let (query, id) = match revoke {
            "membership" => (
                "DELETE FROM panda.vehicle_memberships WHERE user_id = $1",
                user,
            ),
            "user" => ("DELETE FROM panda.users WHERE id = $1", user),
            "disabled" => (
                "UPDATE panda.users SET is_disabled = TRUE WHERE id = $1",
                user,
            ),
            "session" => (
                "UPDATE panda.session_families SET revoked_at=now() WHERE id=$1",
                sid,
            ),
            _ => ("DELETE FROM panda.vehicles WHERE id = $1", vehicle),
        };
        sqlx::query(query)
            .bind(id)
            .execute(&f.state.pool)
            .await
            .unwrap();
        publish(
            &f,
            vehicle,
            r#"{"private_telemetry":"must never be delivered"}"#,
        )
        .await;
        assert_closed_without_telemetry(
            &mut socket,
            if matches!(revoke, "membership" | "vehicle") {
                4403
            } else {
                4401
            },
        )
        .await;
    }
    task.abort();
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires disposable TimescaleDB DATABASE_URL and REDIS_URL"]
async fn authorization_idle_live_socket_closes_after_membership_revocation() {
    let f = Fixture::new().await;
    let user = f.user("user").await;
    let vehicle = f.vehicle(user, "demo-idle").await;
    let (address, task) = server(&f).await;
    let mut socket = connect(address, vehicle, &f.token(user)).await.unwrap();
    assert_eq!(next_text(&mut socket).await, LIVE_KEEPALIVE_MESSAGE);
    sqlx::query("DELETE FROM panda.vehicle_memberships WHERE user_id = $1")
        .bind(user)
        .execute(&f.state.pool)
        .await
        .unwrap();
    assert_closed_without_telemetry(&mut socket, 4403).await;
    task.abort();
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires disposable TimescaleDB DATABASE_URL and REDIS_URL"]
async fn authorization_live_socket_expires_with_its_access_token() {
    let f = Fixture::new().await;
    let user = f.user("user").await;
    let vehicle = f.vehicle(user, "demo-expiry").await;
    let claims = Claims {
        sub: user,
        iss: "panda.app".into(),
        iat: Utc::now().timestamp(),
        exp: Utc::now().timestamp() + 3,
        default_vehicle_id: Some(vehicle),
        sid: None,
    };
    let token = jsonwebtoken::encode(
        &jsonwebtoken::Header::new(Algorithm::RS256),
        &claims,
        &f.state.jwt_keys.encoding,
    )
    .unwrap();
    let (address, task) = server(&f).await;
    let mut socket = connect(address, vehicle, &token).await.unwrap();
    assert_eq!(next_text(&mut socket).await, LIVE_KEEPALIVE_MESSAGE);
    assert_closed_without_telemetry(&mut socket, 4401).await;
    task.abort();
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires disposable TimescaleDB DATABASE_URL and REDIS_URL"]
async fn authorization_live_socket_fails_closed_when_database_is_unavailable() {
    let f = Fixture::new().await;
    let user = f.user("user").await;
    let vehicle = f.vehicle(user, "demo-outage").await;
    let (address, task) = server(&f).await;
    let mut socket = connect(address, vehicle, &f.token(user)).await.unwrap();
    assert_eq!(next_text(&mut socket).await, LIVE_KEEPALIVE_MESSAGE);
    f.state.pool.close().await;
    publish(
        &f,
        vehicle,
        r#"{"private_telemetry":"must never be delivered"}"#,
    )
    .await;
    assert_closed_without_telemetry(&mut socket, 1011).await;
    task.abort();
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires disposable TimescaleDB DATABASE_URL and REDIS_URL"]
async fn authorization_live_handshake_rejects_missing_disabled_and_nonmember_users() {
    let f = Fixture::new().await;
    let owner = f.user("user").await;
    let vehicle = f.vehicle(owner, "demo-handshake").await;
    let outsider = f.user("user").await;
    let disabled = f.user("user").await;
    f.member(disabled, vehicle, "viewer").await;
    sqlx::query("UPDATE panda.users SET is_disabled = TRUE WHERE id = $1")
        .bind(disabled)
        .execute(&f.state.pool)
        .await
        .unwrap();
    let deleted = f.user("user").await;
    f.member(deleted, vehicle, "viewer").await;
    sqlx::query("DELETE FROM panda.users WHERE id = $1")
        .bind(deleted)
        .execute(&f.state.pool)
        .await
        .unwrap();
    let (address, task) = server(&f).await;
    let before = f.snapshot().await;
    for user in [outsider, disabled, deleted] {
        match connect(address, vehicle, &f.token(user)).await {
            Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
                assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
            }
            _ => panic!("unauthorized handshake must be rejected"),
        }
    }
    assert_eq!(f.snapshot().await, before);
    task.abort();
    f.cleanup().await;
}
