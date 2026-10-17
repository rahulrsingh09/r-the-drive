//! Low-level GraphQL transport for the Rivian consumer APIs.
//!
//! This module owns request construction and response-envelope handling only.
//! Token refresh and vehicle-specific retry policy remain in the parent module.

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use uuid::Uuid;

use crate::ingestion::session_store::RivianTokenBundle;
use crate::services::outbound_policy;

const APOLLO_CLIENT_NAME: &str = "com.rivian.ios.consumer-apollo-ios";
const USER_AGENT: &str = "RivianApp/707 CFNetwork/1237 Darwin/20.4.0";

#[derive(Debug, Deserialize)]
struct GqlEnvelope<T> {
    data: Option<T>,
    errors: Option<Vec<GqlError>>,
}

#[derive(Debug, Deserialize)]
struct GqlError {
    #[allow(dead_code)]
    message: String,
    #[serde(default)]
    extensions: Option<GqlErrorExtensions>,
}

#[derive(Debug, Deserialize)]
struct GqlErrorExtensions {
    code: Option<String>,
}

fn fmt_errors(errors: &[GqlError]) -> String {
    // Upstream error fields may echo credentials or other request inputs.
    format!("{} upstream errors (details withheld)", errors.len())
}

/// Typed marker error for an explicit authentication failure from Rivian.
#[derive(Debug, thiserror::Error)]
#[error("Rivian API: authentication required")]
pub struct AuthError;

fn errors_indicate_auth(errors: &[GqlError]) -> bool {
    errors.iter().any(|error| {
        error
            .extensions
            .as_ref()
            .and_then(|value| value.code.as_deref())
            == Some("UNAUTHENTICATED")
    })
}

/// Send one GraphQL request and deserialize its `data` object.
pub async fn gql_request<T: for<'de> Deserialize<'de>>(
    _client: &reqwest::Client,
    url: &str,
    tokens: &RivianTokenBundle,
    operation: &str,
    query: &str,
    variables: serde_json::Value,
) -> Result<T> {
    outbound_policy::require_telemetry_query(query)?;
    let body = serde_json::json!({"operationName":operation,"query":query,"variables":variables});
    let mut request = outbound_policy::rivian_post(url)?;
    for (name, value) in [
        ("User-Agent", USER_AGENT),
        ("Accept", "application/json"),
        ("Content-Type", "application/json"),
        ("Apollographql-Client-Name", APOLLO_CLIENT_NAME),
        ("A-Sess", tokens.app_session_token.as_str()),
        ("U-Sess", tokens.user_session_token.as_str()),
    ] {
        request = request.header(name, value);
    }
    request = request
        .header("dc-cid", format!("m-ios-{}", Uuid::new_v4()))
        .json(&body);
    if let Some(value) = Some(tokens.csrf_token.as_str()).filter(|value| !value.is_empty()) {
        request = request.header("Csrf-Token", value);
    }
    if !tokens.access_token.is_empty() {
        request = request.bearer_auth(&tokens.access_token);
    }
    let response = request.send().await.context("HTTP request failed")?;
    match response.status() {
        reqwest::StatusCode::UNAUTHORIZED => Err(anyhow!(AuthError)),
        status if !status.is_success() => Err(anyhow!("Rivian API: HTTP {status}")),
        _ => {
            let envelope: GqlEnvelope<T> = outbound_policy::read_json(response)
                .await
                .context("failed to parse Rivian API response")?;
            match envelope {
                GqlEnvelope {
                    errors: Some(errors),
                    ..
                } if !errors.is_empty() => {
                    if errors_indicate_auth(&errors) {
                        Err(anyhow!(AuthError))
                    } else {
                        Err(anyhow!("Rivian GQL errors: {}", fmt_errors(&errors)))
                    }
                }
                GqlEnvelope {
                    data: Some(data), ..
                } => Ok(data),
                _ => Err(anyhow!("Rivian API: empty data for {operation}")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_tokens() -> RivianTokenBundle {
        RivianTokenBundle {
            access_token: "synthetic-access".into(),
            refresh_token: "synthetic-refresh".into(),
            app_session_token: "synthetic-app".into(),
            user_session_token: "synthetic-user".into(),
            csrf_token: "synthetic-csrf".into(),
            created_at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn legitimate_telemetry_uses_policy_transport_and_session_headers() {
        use axum::{http::HeaderMap, routing::post, Json, Router};
        let app = Router::new().route(
            "/graphql",
            post(
                |headers: HeaderMap, Json(body): Json<serde_json::Value>| async move {
                    assert_eq!(headers["a-sess"], "synthetic-app");
                    assert_eq!(headers["u-sess"], "synthetic-user");
                    assert_eq!(headers["authorization"], "Bearer synthetic-access");
                    assert_eq!(headers["csrf-token"], "synthetic-csrf");
                    assert_eq!(body["operationName"], "getUserInfo");
                    Json(serde_json::json!({"data": {"vehicles": [{"id": "synthetic-vehicle"}]}}))
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/graphql", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        outbound_policy::with_mock_gateway(url.clone(), async {
            let result: serde_json::Value = gql_request(
                &reqwest::Client::new(),
                &url,
                &synthetic_tokens(),
                "getUserInfo",
                "query getUserInfo { currentUser { vehicles { id } } }",
                serde_json::Value::Null,
            )
            .await
            .unwrap();
            assert_eq!(result["vehicles"][0]["id"], "synthetic-vehicle");
        })
        .await;
        server.abort();
    }

    #[tokio::test]
    async fn all_vehicle_write_entrypoints_are_denied_before_network_or_database() {
        use crate::ingestion::rivian_poll;
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
            .unwrap();
        let client = reqwest::Client::new();
        let tokens = synthetic_tokens();
        let vehicle_id = Uuid::new_v4();
        let charging = serde_json::from_value(serde_json::json!({"enabled": true})).unwrap();
        let departure = rivian_poll::DepartureScheduleInput {
            name: None,
            enabled: true,
            occurrence: None,
            comfort_settings: Some(serde_json::json!({"defrost": true})),
        };
        let errors = [
            rivian_poll::mutate_charging_schedule(
                "synthetic",
                vehicle_id,
                &charging,
                &pool,
                &client,
                &tokens,
            )
            .await
            .unwrap_err(),
            rivian_poll::create_departure_schedule(
                "synthetic",
                vehicle_id,
                &departure,
                &pool,
                &client,
                &tokens,
            )
            .await
            .unwrap_err(),
            rivian_poll::update_departure_schedule(
                "synthetic",
                vehicle_id,
                "schedule",
                &departure,
                &pool,
                &client,
                &tokens,
            )
            .await
            .unwrap_err(),
            rivian_poll::delete_departure_schedule(
                "synthetic",
                vehicle_id,
                "schedule",
                &pool,
                &client,
                &tokens,
            )
            .await
            .unwrap_err(),
        ];
        for error in errors {
            assert!(matches!(
                error.downcast_ref::<outbound_policy::PolicyError>(),
                Some(outbound_policy::PolicyError::VehicleWrite)
            ));
        }
        let error = gql_request::<serde_json::Value>(
            &client,
            outbound_policy::GATEWAY_URL,
            &tokens,
            "getUserInfo",
            "mutation getUserInfo { updateVehicleChargingSettings { __typename } }",
            serde_json::Value::Null,
        )
        .await
        .unwrap_err();
        assert!(error
            .downcast_ref::<outbound_policy::PolicyError>()
            .is_some());
    }

    #[test]
    fn detects_only_explicit_unauthenticated_graphql_codes() {
        let auth = GqlError {
            message: "session expired".into(),
            extensions: Some(GqlErrorExtensions {
                code: Some("UNAUTHENTICATED".into()),
            }),
        };
        let ordinary = GqlError {
            message: "authentication text in an ordinary error".into(),
            extensions: Some(GqlErrorExtensions {
                code: Some("BAD_USER_INPUT".into()),
            }),
        };

        assert!(errors_indicate_auth(&[auth]));
        assert!(!errors_indicate_auth(&[ordinary]));
    }

    #[test]
    fn formats_multiple_graphql_errors_in_source_order() {
        let errors = vec![
            GqlError {
                message: "first".into(),
                extensions: None,
            },
            GqlError {
                message: "second".into(),
                extensions: None,
            },
        ];

        assert_eq!(fmt_errors(&errors), "2 upstream errors (details withheld)");
    }
}
