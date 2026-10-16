//! Deployment policy for telemetry-only Rivian access and optional egress.
//! Saved settings cannot relax this policy. Provider tokens are NOT read-only.

use crate::{
    errors::AppError,
    services::external_connections::{OPEN_METEO, RIVIAN_ACCOUNT, S3_BACKUP},
};
use std::{sync::OnceLock, time::Duration};

pub const GATEWAY_URL: &str = "https://rivian.com/api/gql/gateway/graphql";
pub const CHARGING_URL: &str = "https://rivian.com/api/gql/chrg/user/graphql";
pub const WEBSOCKET_URL: &str = "wss://api.rivian.com/gql-consumer-subscriptions/graphql";

#[derive(Debug, thiserror::Error)]
pub enum PolicyError {
    #[error("Rivian destination rejected by telemetry-only policy")]
    Destination,
    #[error("Vehicle changes are disabled by telemetry-only policy")]
    VehicleWrite,
    #[error("Optional external traffic is disabled by telemetry-only policy")]
    OptionalTraffic,
    #[error("Outbound redirects are disabled")]
    Redirect,
    #[error("Upstream response exceeds the size limit")]
    ResponseTooLarge,
    #[error("Invalid upstream JSON")]
    Json(#[from] serde_json::Error),
    #[error("Upstream HTTP request failed")]
    Http(#[from] reqwest::Error),
}

pub fn connection_allowed(id: &str) -> bool {
    matches!(id, RIVIAN_ACCOUNT | S3_BACKUP | OPEN_METEO) || optional_traffic_allowed()
}

pub fn require_connection_allowed(id: &str) -> Result<(), AppError> {
    if connection_allowed(id) {
        Ok(())
    } else {
        Err(AppError::ExternalConnectionDisabled(id.to_string()))
    }
}

pub fn optional_traffic_allowed() -> bool {
    false
}

pub fn require_optional_traffic() -> Result<(), PolicyError> {
    if optional_traffic_allowed() {
        Ok(())
    } else {
        Err(PolicyError::OptionalTraffic)
    }
}

pub async fn optional_client_for_url(
    url: &url::Url,
    allowlist: &[ipnet::IpNet],
) -> Result<reqwest::Client, crate::errors::AppError> {
    require_optional_traffic().map_err(|_| {
        crate::errors::AppError::ExternalConnectionDisabled("optional providers".into())
    })?;
    crate::services::outbound::outbound_client_for_url(url, allowlist).await
}

/// No operation-name exception: a mutation disguised as a telemetry operation
/// is still denied. Auth mutations use the private, fixed auth flow.
pub fn require_telemetry_query(query: &str) -> Result<(), PolicyError> {
    let words: Vec<_> = query
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .filter(|word| !word.is_empty())
        .collect();
    if words.first() != Some(&"query")
        || words
            .iter()
            .any(|word| matches!(*word, "mutation" | "subscription"))
    {
        return Err(PolicyError::VehicleWrite);
    }
    Ok(())
}

pub fn validate_http_destination(url: &str) -> Result<(), PolicyError> {
    // Exact strings also exclude userinfo, alternate ports, fragments, query
    // parameters, encoded paths and lookalike hosts before adding credentials.
    if matches!(url, GATEWAY_URL | CHARGING_URL) {
        Ok(())
    } else {
        Err(PolicyError::Destination)
    }
}

pub fn validate_ws_destination(url: &str) -> Result<(), PolicyError> {
    if url == WEBSOCKET_URL {
        Ok(())
    } else {
        Err(PolicyError::Destination)
    }
}

pub fn gateway_url() -> Result<String, PolicyError> {
    #[cfg(test)]
    if let Ok(url) = MOCK_GATEWAY.try_with(Clone::clone) {
        return Ok(url);
    }
    validated_gateway_url(
        std::env::var("RIVIAN_GRAPHQL_GATEWAY_URL").unwrap_or_else(|_| GATEWAY_URL.into()),
    )
}

fn validated_gateway_url(url: String) -> Result<String, PolicyError> {
    if url != GATEWAY_URL {
        return Err(PolicyError::Destination);
    }
    Ok(url)
}

/// Keep client ownership here: caller-supplied redirect/proxy policies must
/// never affect requests containing Rivian passwords or session headers.
pub fn rivian_post(url: &str) -> Result<reqwest::RequestBuilder, PolicyError> {
    #[cfg(test)]
    let is_mock = MOCK_GATEWAY.try_with(|mock| mock == url).unwrap_or(false);
    #[cfg(not(test))]
    let is_mock = false;
    if !is_mock {
        validate_http_destination(url)?;
    }
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    let client = CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("fixed Rivian HTTP client")
    });
    Ok(client.post(url))
}

pub async fn read_json<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
) -> Result<T, PolicyError> {
    let bytes = read_limited(response, 8 * 1024 * 1024).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

pub async fn read_limited(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, PolicyError> {
    if response.status().is_redirection() {
        return Err(PolicyError::Redirect);
    }
    response.error_for_status_ref()?;
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(PolicyError::ResponseTooLarge);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if chunk.len() > limit.saturating_sub(bytes.len()) {
            return Err(PolicyError::ResponseTooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

// Compiled only into unit tests; task scoping prevents concurrent tests from
// changing one another's destinations. No environment switch enables mocks.
#[cfg(test)]
tokio::task_local! {
    static MOCK_GATEWAY: String;
}

#[cfg(test)]
pub async fn with_mock_gateway<F: std::future::Future>(url: String, future: F) -> F::Output {
    let parsed = url::Url::parse(&url).expect("mock URL");
    assert_eq!(parsed.scheme(), "http");
    assert_eq!(parsed.username(), "");
    assert!(parsed.password().is_none());
    assert!(parsed
        .host_str()
        .and_then(|host| host.parse::<std::net::IpAddr>().ok())
        .is_some_and(|ip| ip.is_loopback()));
    MOCK_GATEWAY.scope(url, future).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::StatusCode, response::Response, routing::post, Router};
    use tokio::net::TcpListener;

    #[test]
    fn exact_rivian_destinations_exclude_url_confusion() {
        for url in [GATEWAY_URL, CHARGING_URL] {
            assert!(validate_http_destination(url).is_ok());
        }
        assert!(validate_ws_destination(WEBSOCKET_URL).is_ok());
        assert!(validated_gateway_url(GATEWAY_URL.into()).is_ok());
        assert!(validated_gateway_url(CHARGING_URL.into()).is_err());
        assert!(validated_gateway_url("http://127.0.0.1:8080/graphql".into()).is_err());
        for url in [
            "http://rivian.com/api/gql/gateway/graphql",
            "https://rivian.com.evil.test/api/gql/gateway/graphql",
            "https://rivian.com@evil.test/api/gql/gateway/graphql",
            "https://user:password@rivian.com/api/gql/gateway/graphql",
            "https://rivian.com:444/api/gql/gateway/graphql",
            "https://rivian.com/api/gql/gateway/graphql?redirect=evil",
            "https://rivian.com/api/gql/gateway/graphql#fragment",
            "https://rivian.com/api/gql/gateway/%67raphql",
            "https://127.0.0.1/api/gql/gateway/graphql",
            "https://[::ffff:127.0.0.1]/graphql",
            "https://169.254.169.254/latest/meta-data",
            "https://rivian.com\\@evil.test/graphql",
        ] {
            assert!(validate_http_destination(url).is_err(), "{url}");
            assert!(rivian_post(url).is_err(), "{url}");
            assert!(validated_gateway_url(url.into()).is_err(), "{url}");
        }
        for url in [
            "ws://api.rivian.com/gql-consumer-subscriptions/graphql",
            "wss://api.rivian.com@evil.test/gql-consumer-subscriptions/graphql",
            "wss://user:password@api.rivian.com/gql-consumer-subscriptions/graphql",
            "wss://api.rivian.com.evil.test/gql-consumer-subscriptions/graphql",
            "wss://api.rivian.com/gql-consumer-subscriptions/graphql?token=x",
        ] {
            assert!(validate_ws_destination(url).is_err());
        }
    }

    #[test]
    fn vehicle_mutations_cannot_hide_behind_query_operation_names() {
        assert!(
            require_telemetry_query("query getUserInfo { currentUser { vehicles { id } } }")
                .is_ok()
        );
        for query in [
            "mutation getUserInfo { updateVehicleChargingSettings { __typename } }",
            "query safe { currentUser { id } } mutation getUserInfo { deleteDepartureSchedule }",
            "# query\nmutation getUserInfo { createDepartureSchedule }",
            "mutation Login { updateDepartureSchedule }",
            "subscription getUserInfo { vehicleState }",
            "{ updateVehicleChargingSettings }",
        ] {
            assert!(matches!(
                require_telemetry_query(query),
                Err(PolicyError::VehicleWrite)
            ));
        }
    }

    #[tokio::test]
    async fn credential_requests_never_follow_redirects() {
        let sink = TcpListener::bind("127.0.0.1:0").await.unwrap();
        for code in [301, 302, 303, 307, 308] {
            let destination = format!("http://{}/stolen", sink.local_addr().unwrap());
            let app = Router::new().route(
                "/graphql",
                post(move || async move {
                    Response::builder()
                        .status(code)
                        .header("location", destination)
                        .body(Body::empty())
                        .unwrap()
                }),
            );
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/graphql", listener.local_addr().unwrap());
            let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            with_mock_gateway(url.clone(), async {
                let response = rivian_post(&url)
                    .unwrap()
                    .header("A-Sess", "synthetic-app")
                    .header("U-Sess", "synthetic-user")
                    .header("Csrf-Token", "synthetic-csrf")
                    .bearer_auth("synthetic-access")
                    .json(&serde_json::json!({"password": "synthetic-password"}))
                    .send()
                    .await
                    .unwrap();
                assert_eq!(response.status().as_u16(), code);
                assert!(matches!(
                    read_limited(response, 1024).await,
                    Err(PolicyError::Redirect)
                ));
            })
            .await;
            server.abort();
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(100), sink.accept())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn response_limits_cover_content_length_and_chunked_bodies() {
        for chunked in [false, true] {
            let app = Router::new().route(
                "/graphql",
                post(move || async move {
                    if chunked {
                        let stream =
                            futures::stream::iter([Ok::<_, std::io::Error>("12345678"), Ok("9")]);
                        Response::new(Body::from_stream(stream))
                    } else {
                        Response::new(Body::from("123456789"))
                    }
                }),
            );
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/graphql", listener.local_addr().unwrap());
            let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            with_mock_gateway(url.clone(), async {
                let response = rivian_post(&url).unwrap().send().await.unwrap();
                assert!(matches!(
                    read_limited(response, 8).await,
                    Err(PolicyError::ResponseTooLarge)
                ));
                let response = rivian_post(&url).unwrap().send().await.unwrap();
                assert_eq!(read_limited(response, 9).await.unwrap(), b"123456789");
            })
            .await;
            server.abort();
        }
    }

    #[tokio::test]
    async fn websocket_handshake_rejects_redirect_without_contacting_target() {
        let sink = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let destination = format!("ws://{}/stolen", sink.local_addr().unwrap());
        let app = Router::new().route(
            "/graphql",
            axum::routing::get(move || async move {
                Response::builder()
                    .status(StatusCode::TEMPORARY_REDIRECT)
                    .header("location", destination)
                    .body(Body::empty())
                    .unwrap()
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}/graphql", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let mut request = url.into_client_request().unwrap();
        request
            .headers_mut()
            .insert("A-Sess", "synthetic-app".parse().unwrap());
        let error = tokio_tungstenite::connect_async(request).await.unwrap_err();
        assert!(
            matches!(error, tokio_tungstenite::tungstenite::Error::Http(response)
            if response.status() == StatusCode::TEMPORARY_REDIRECT)
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(100), sink.accept())
                .await
                .is_err()
        );
        server.abort();
    }
}
