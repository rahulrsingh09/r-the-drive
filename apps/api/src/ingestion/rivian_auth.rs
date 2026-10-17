//! Rivian authentication flow.

#[cfg(test)]
#[path = "full_replacement_rivian_auth_tests.rs"]
mod full_replacement_key_custody_tests;

use crate::ingestion::session_store::RivianTokenBundle;
use crate::services::outbound_policy::{self, gateway_url};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const APOLLO_CLIENT_NAME: &str = "com.rivian.ios.consumer-apollo-ios";
const USER_AGENT: &str = "RivianApp/707 CFNetwork/1237 Darwin/20.4.0";

#[derive(Debug, thiserror::Error)]
pub enum RivianAuthError {
    #[error("Invalid credentials")]
    InvalidCredentials,
    #[error("Invalid OTP")]
    InvalidOtp,
    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("Unexpected response: {0}")]
    UnexpectedResponse(String),
}

impl From<outbound_policy::PolicyError> for RivianAuthError {
    fn from(error: outbound_policy::PolicyError) -> Self {
        Self::UnexpectedResponse(error.to_string())
    }
}

#[derive(Debug, Clone)]
pub struct RivianOtpChallenge {
    pub email: String,
    pub otp_token: String,
    pub csrf_token: String,
    pub app_session_token: String,
}

pub enum LoginResult {
    Authenticated(RivianTokenBundle),
    OtpRequired(RivianOtpChallenge),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RivianVehicleSummary {
    pub id: String,
    pub name: Option<String>,
    pub vin: Option<String>,
    pub model: Option<String>,
    pub model_year: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RivianVehicleImage {
    pub order_id: Option<String>,
    pub vehicle_id: Option<String>,
    pub url: String,
    pub extension: Option<String>,
    pub resolution: Option<String>,
    pub size: Option<String>,
    pub design: Option<String>,
    pub placement: Option<String>,
    pub overlays: serde_json::Value,
    pub source: String,
    pub vehicle_version: String,
}

#[derive(Debug, Deserialize)]
struct LoginResponse {
    data: Option<LoginData>,
    errors: Option<Vec<GqlError>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoginData {
    login: Option<LoginPayload>,
    #[serde(rename = "loginWithOTP")]
    login_with_otp: Option<LoginPayload>,
    #[serde(rename = "createCsrfToken")]
    create_csrf_token: Option<CreateCsrfTokenPayload>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoginPayload {
    #[serde(rename = "__typename")]
    typename: Option<String>,
    access_token: Option<String>,
    refresh_token: Option<String>,
    user_session_token: Option<String>,
    otp_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GqlError {
    message: String,
    extensions: Option<GqlErrorExtensions>,
}

#[derive(Debug, Deserialize)]
struct GqlErrorExtensions {
    code: Option<String>,
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateCsrfTokenPayload {
    csrf_token: String,
    app_session_token: String,
}

#[derive(Debug, Deserialize)]
struct UserInfoResponse {
    data: Option<UserInfoData>,
    errors: Option<Vec<GqlError>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UserInfoData {
    current_user: Option<CurrentUser>,
}

#[derive(Debug, Deserialize)]
struct CurrentUser {
    vehicles: Option<Vec<UserVehicle>>,
}

#[derive(Debug, Deserialize)]
struct UserVehicle {
    id: String,
    vin: Option<String>,
    name: Option<String>,
    vehicle: Option<UserVehicleDetails>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UserVehicleDetails {
    model_year: Option<i32>,
    model: Option<String>,
}

struct CsrfSession {
    csrf_token: String,
    app_session_token: String,
}

const LOGIN_QUERY: &str = r#"
      mutation Login($email: String!, $password: String!) {
        login(email: $email, password: $password) {
          __typename
          ... on MobileLoginResponse {
            accessToken
            refreshToken
            userSessionToken
          }
          ... on MobileMFALoginResponse {
            otpToken
          }
        }
      }
    "#;

const OTP_QUERY: &str = r#"
      mutation LoginWithOTP($email: String!, $otpCode: String!, $otpToken: String!) {
        loginWithOTP(email: $email, otpCode: $otpCode, otpToken: $otpToken) {
          __typename
          ... on MobileLoginResponse {
            accessToken
            refreshToken
            userSessionToken
          }
        }
      }
    "#;

const USER_QUERY: &str = r#"
      query getUserInfo {
        currentUser {
          vehicles {
            id
            vin
            name
            vehicle {
              modelYear
              model
            }
          }
        }
      }
    "#;

const CSRF_QUERY: &str = r#"
      mutation CreateCSRFToken {
        createCsrfToken {
          __typename
          csrfToken
          appSessionToken
        }
      }
    "#;

impl LoginPayload {
    fn into_tokens(
        self,
        session: CsrfSession,
        context: &str,
    ) -> Result<RivianTokenBundle, RivianAuthError> {
        let [access, refresh, user] = [
            ("accessToken", self.access_token),
            ("refreshToken", self.refresh_token),
            ("userSessionToken", self.user_session_token),
        ]
        .map(|(field, value)| {
            value.filter(|value| !value.is_empty()).ok_or_else(|| {
                RivianAuthError::UnexpectedResponse(format!(
                    "missing {field} in {context} response"
                ))
            })
        });
        Ok(RivianTokenBundle {
            access_token: access?,
            refresh_token: refresh?,
            app_session_token: session.app_session_token,
            user_session_token: user?,
            csrf_token: session.csrf_token,
            created_at: chrono::Utc::now(),
        })
    }
}

enum SessionHeaders<'a> {
    Anonymous,
    Csrf(&'a CsrfSession),
    User(&'a RivianTokenBundle),
}

async fn gateway_request<T: serde::de::DeserializeOwned>(
    body: serde_json::Value,
    session: SessionHeaders<'_>,
) -> Result<T, RivianAuthError> {
    let mut headers = reqwest::header::HeaderMap::new();
    for (name, value) in [
        ("User-Agent", USER_AGENT),
        ("Accept", "application/json"),
        ("Content-Type", "application/json"),
        ("Apollographql-Client-Name", APOLLO_CLIENT_NAME),
    ] {
        headers.insert(
            reqwest::header::HeaderName::from_bytes(name.as_bytes()).expect("fixed header name"),
            reqwest::header::HeaderValue::from_static(value),
        );
    }
    let mut request = outbound_policy::rivian_post(&gateway_url()?)?
        .headers(headers)
        .header("dc-cid", format!("m-ios-{}", Uuid::new_v4()));
    match session {
        SessionHeaders::Anonymous => {}
        SessionHeaders::Csrf(session) => {
            request = request
                .header("A-Sess", &session.app_session_token)
                .header("Csrf-Token", &session.csrf_token);
        }
        SessionHeaders::User(tokens) => {
            request = request
                .header("A-Sess", &tokens.app_session_token)
                .header("U-Sess", &tokens.user_session_token);
            if !tokens.csrf_token.is_empty() {
                request = request.header("Csrf-Token", &tokens.csrf_token);
            }
            if !tokens.access_token.is_empty() {
                request = request.bearer_auth(&tokens.access_token);
            }
        }
    }
    let response = request.json(&body).send().await?;
    match response.status() {
        reqwest::StatusCode::UNAUTHORIZED => Err(RivianAuthError::InvalidCredentials),
        _ => outbound_policy::read_json(response)
            .await
            .map_err(Into::into),
    }
}

/// Refresh only the short-lived pair; access, refresh, user session and creation time survive.
pub async fn rivian_refresh_csrf(
    client: &reqwest::Client,
    tokens: &RivianTokenBundle,
) -> Result<RivianTokenBundle, RivianAuthError> {
    let CsrfSession {
        csrf_token,
        app_session_token,
    } = create_csrf_session(client).await?;
    let mut refreshed = tokens.clone();
    refreshed.csrf_token = csrf_token;
    refreshed.app_session_token = app_session_token;
    Ok(refreshed)
}

pub async fn rivian_login(
    client: &reqwest::Client,
    email: &str,
    password: &str,
) -> Result<LoginResult, RivianAuthError> {
    let session = create_csrf_session(client).await?;
    let response = post_graphql(
        client,
        serde_json::json!({
            "operationName":"Login", "query":LOGIN_QUERY,
            "variables":{"email":email,"password":password}
        }),
        Some(&session),
    )
    .await?;
    if let Some(errors) = response.errors.filter(|errors| !errors.is_empty()) {
        return Err(map_login_errors(errors));
    }
    let payload = response
        .data
        .and_then(|data| data.login)
        .ok_or_else(|| RivianAuthError::UnexpectedResponse("empty login payload".into()))?;
    if payload.typename.as_deref() != Some("MobileMFALoginResponse") {
        return payload
            .into_tokens(session, "login")
            .map(LoginResult::Authenticated);
    }
    let otp_token = payload
        .otp_token
        .filter(|token| !token.is_empty())
        .ok_or_else(|| {
            RivianAuthError::UnexpectedResponse("missing otpToken in MFA response".into())
        })?;
    Ok(LoginResult::OtpRequired(RivianOtpChallenge {
        email: email.to_owned(),
        otp_token,
        csrf_token: session.csrf_token,
        app_session_token: session.app_session_token,
    }))
}

pub async fn rivian_login_otp(
    client: &reqwest::Client,
    challenge: &RivianOtpChallenge,
    otp_code: &str,
) -> Result<RivianTokenBundle, RivianAuthError> {
    let session = CsrfSession {
        csrf_token: challenge.csrf_token.clone(),
        app_session_token: challenge.app_session_token.clone(),
    };
    let response = post_graphql(
        client,
        serde_json::json!({
            "operationName":"LoginWithOTP", "query":OTP_QUERY,
            "variables":{"email":challenge.email,"otpCode":otp_code,"otpToken":challenge.otp_token}
        }),
        Some(&session),
    )
    .await?;
    match response.errors.filter(|errors| !errors.is_empty()) {
        Some(errors) => Err(map_otp_errors(errors)),
        None => response
            .data
            .and_then(|data| data.login_with_otp)
            .ok_or_else(|| RivianAuthError::UnexpectedResponse("empty OTP payload".into()))?
            .into_tokens(session, "OTP"),
    }
}

pub async fn rivian_user_vehicles(
    _client: &reqwest::Client,
    tokens: &RivianTokenBundle,
) -> Result<Vec<RivianVehicleSummary>, RivianAuthError> {
    let response: UserInfoResponse = gateway_request(
        serde_json::json!({
            "operationName":"getUserInfo", "query":USER_QUERY, "variables":serde_json::Value::Null
        }),
        SessionHeaders::User(tokens),
    )
    .await?;
    if let Some(errors) = response.errors.filter(|errors| !errors.is_empty()) {
        return Err(RivianAuthError::UnexpectedResponse(format_gql_error(
            &errors,
        )));
    }
    let entries = response
        .data
        .and_then(|data| data.current_user)
        .and_then(|user| user.vehicles)
        .unwrap_or_default();
    Ok(entries
        .into_iter()
        .map(|entry| {
            let (model, model_year) = entry
                .vehicle
                .map(|details| (details.model, details.model_year))
                .unwrap_or_default();
            RivianVehicleSummary {
                id: entry.id,
                name: entry.name,
                vin: entry.vin,
                model,
                model_year,
            }
        })
        .collect())
}

pub async fn rivian_vehicle_images(
    _client: &reqwest::Client,
    _tokens: &RivianTokenBundle,
) -> Result<Vec<RivianVehicleImage>, RivianAuthError> {
    // Private deployments never admit image traffic, including authentication retries.
    Err(outbound_policy::PolicyError::OptionalTraffic.into())
}

async fn create_csrf_session(client: &reqwest::Client) -> Result<CsrfSession, RivianAuthError> {
    match post_graphql(client, serde_json::json!({
        "operationName":"CreateCSRFToken", "query":CSRF_QUERY, "variables":serde_json::Value::Null
    }), None).await? {
        LoginResponse { errors:Some(errors), .. } =>
            Err(RivianAuthError::UnexpectedResponse(format_gql_error(&errors))),
        LoginResponse { data:Some(LoginData { create_csrf_token:Some(payload), .. }), .. } =>
            Ok(CsrfSession { csrf_token:payload.csrf_token, app_session_token:payload.app_session_token }),
        _ => Err(RivianAuthError::UnexpectedResponse("empty CSRF payload".into())),
    }
}

async fn post_graphql(
    _client: &reqwest::Client,
    body: serde_json::Value,
    session: Option<&CsrfSession>,
) -> Result<LoginResponse, RivianAuthError> {
    let headers = session.map_or(SessionHeaders::Anonymous, SessionHeaders::Csrf);
    gateway_request(body, headers).await
}

fn map_login_errors(errors: Vec<GqlError>) -> RivianAuthError {
    const PASSWORD_ERRORS: [&str; 5] = [
        "invalid credentials",
        "invalid email or password",
        "incorrect email or password",
        "invalid password",
        "incorrect password",
    ];
    let invalid = errors.iter().any(|error| {
        error
            .extensions
            .as_ref()
            .and_then(|extension| extension.reason.as_deref())
            == Some("BAD_CURRENT_PASSWORD")
            || PASSWORD_ERRORS
                .iter()
                .any(|pattern| error.message.to_lowercase().contains(pattern))
    });
    if invalid {
        RivianAuthError::InvalidCredentials
    } else {
        RivianAuthError::UnexpectedResponse(format_gql_error(&errors))
    }
}

fn map_otp_errors(errors: Vec<GqlError>) -> RivianAuthError {
    let invalid = errors
        .iter()
        .filter_map(|error| error.extensions.as_ref())
        .any(|extension| {
            matches!(
                (extension.code.as_deref(), extension.reason.as_deref()),
                (Some("BAD_USER_INPUT"), Some("INVALID_OTP"))
                    | (Some("UNAUTHENTICATED"), Some("OTP_TOKEN_EXPIRED"))
            )
        });
    if invalid {
        RivianAuthError::InvalidOtp
    } else {
        RivianAuthError::UnexpectedResponse(format_gql_error(&errors))
    }
}

fn format_gql_error(errors: &[GqlError]) -> String {
    // Upstream error fields may echo credentials or other request inputs.
    format!("Rivian GraphQL request failed ({} errors)", errors.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upstream_errors_do_not_echo_request_secrets() {
        let errors: Vec<GqlError> = serde_json::from_value(serde_json::json!([{
            "message": "synthetic-secret-password",
            "extensions": {
                "code": "synthetic-secret-token",
                "reason": "synthetic-secret-email"
            }
        }]))
        .unwrap();
        assert_eq!(
            format_gql_error(&errors),
            "Rivian GraphQL request failed (1 errors)"
        );
        assert!(!map_login_errors(errors)
            .to_string()
            .contains("synthetic-secret"));
    }
    use axum::{
        extract::State,
        http::{HeaderMap, StatusCode},
        routing::post,
        Json, Router,
    };
    use serde_json::{json, Value};
    use std::sync::{Arc, Mutex};

    #[test]
    fn classifies_only_explicit_login_credential_errors_as_invalid_credentials() {
        let error = map_login_errors(vec![GqlError {
            message: "Invalid email or password".into(),
            extensions: None,
        }]);

        assert!(matches!(error, RivianAuthError::InvalidCredentials));
    }

    #[test]
    fn does_not_misclassify_csrf_errors_as_bad_passwords() {
        let error = map_login_errors(vec![GqlError {
            message: "Invalid CSRF token".into(),
            extensions: Some(GqlErrorExtensions {
                code: Some("UNAUTHENTICATED".into()),
                reason: None,
            }),
        }]);

        assert!(matches!(error, RivianAuthError::UnexpectedResponse(_)));
    }

    #[derive(Debug, Clone)]
    struct RecordedRequest {
        operation_name: String,
        headers: HeaderMap,
        body: Value,
    }

    #[tokio::test]
    async fn auth_transport_ignores_caller_redirect_policy() {
        let sink = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let destination = format!("http://{}/stolen", sink.local_addr().unwrap());
        let app =
            Router::new().route(
                "/graphql",
                post(move || async move {
                    (StatusCode::TEMPORARY_REDIRECT, [("location", destination)])
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/graphql", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        outbound_policy::with_mock_gateway(url, async {
            let client = reqwest::Client::new();
            let session = CsrfSession {
                csrf_token: "synthetic-csrf".into(),
                app_session_token: "synthetic-app".into(),
            };
            let result = post_graphql(&client, json!({
                "operationName": "Login",
                "variables": {"email": "synthetic@example.test", "password": "synthetic-password"},
            }), Some(&session)).await;
            assert!(matches!(
                result,
                Err(RivianAuthError::UnexpectedResponse(_))
            ));
        })
        .await;
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), sink.accept())
                .await
                .is_err()
        );
        server.abort();
    }

    #[tokio::test]
    async fn login_and_otp_follow_current_rivian_graphql_shape() {
        let recorded = Arc::new(Mutex::new(Vec::<RecordedRequest>::new()));
        let app = Router::new()
            .route("/api/gql/gateway/graphql", post(mock_rivian_gateway))
            .with_state(recorded.clone());

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let _server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        outbound_policy::with_mock_gateway(
            format!("http://{addr}/api/gql/gateway/graphql"),
            assert_login_and_otp_requests(recorded),
        )
        .await;
    }

    async fn assert_login_and_otp_requests(recorded: Arc<Mutex<Vec<RecordedRequest>>>) {
        let client = reqwest::Client::new();
        let challenge = match rivian_login(&client, "driver@example.com", "secret")
            .await
            .unwrap()
        {
            LoginResult::OtpRequired(challenge) => challenge,
            LoginResult::Authenticated(_) => panic!("expected MFA challenge"),
        };

        assert_eq!(challenge.email, "driver@example.com");
        assert_eq!(challenge.otp_token, "otp-token");

        let tokens = rivian_login_otp(&client, &challenge, "123456")
            .await
            .unwrap();
        assert_eq!(tokens.access_token, "access-token");
        assert_eq!(tokens.refresh_token, "refresh-token");
        assert_eq!(tokens.user_session_token, "user-session-token");
        assert_eq!(tokens.app_session_token, "app-session-token");
        assert_eq!(tokens.csrf_token, "csrf-token");

        let vehicles = rivian_user_vehicles(&client, &tokens).await.unwrap();
        assert_eq!(
            vehicles,
            vec![RivianVehicleSummary {
                id: "vehicle-123".into(),
                name: Some("Compass Yellow".into()),
                vin: Some("7FCTGAAL0NN000001".into()),
                model: Some("R1T".into()),
                model_year: Some(2022),
            }]
        );

        let original_created_at = tokens.created_at;
        let refreshed = rivian_refresh_csrf(&client, &tokens).await.unwrap();
        assert_eq!(refreshed.created_at, original_created_at);
        assert_eq!(refreshed.access_token, tokens.access_token);
        assert_eq!(refreshed.refresh_token, tokens.refresh_token);
        assert_eq!(refreshed.user_session_token, tokens.user_session_token);

        let requests = recorded.lock().unwrap();
        assert_eq!(requests.len(), 5);
        assert_eq!(requests[0].operation_name, "CreateCSRFToken");
        assert!(requests[0].headers.get("csrf-token").is_none());
        assert!(requests[0].headers.get("a-sess").is_none());

        assert_eq!(requests[1].operation_name, "Login");
        assert_eq!(requests[1].headers["csrf-token"], "csrf-token");
        assert_eq!(requests[1].headers["a-sess"], "app-session-token");
        assert_eq!(
            requests[1].headers["apollographql-client-name"],
            APOLLO_CLIENT_NAME
        );
        assert_eq!(requests[1].body["variables"]["email"], "driver@example.com");
        assert_eq!(requests[1].body["variables"]["password"], "secret");

        assert_eq!(requests[2].operation_name, "LoginWithOTP");
        assert_eq!(requests[2].headers["csrf-token"], "csrf-token");
        assert_eq!(requests[2].headers["a-sess"], "app-session-token");
        assert_eq!(requests[2].body["variables"]["email"], "driver@example.com");
        assert_eq!(requests[2].body["variables"]["otpCode"], "123456");
        assert_eq!(requests[2].body["variables"]["otpToken"], "otp-token");

        assert_eq!(requests[3].operation_name, "getUserInfo");
        assert_eq!(requests[3].headers["csrf-token"], "csrf-token");
        assert_eq!(requests[3].headers["a-sess"], "app-session-token");
        assert_eq!(requests[3].headers["u-sess"], "user-session-token");
        assert_eq!(requests[3].headers["authorization"], "Bearer access-token");
        assert_eq!(requests[4].operation_name, "CreateCSRFToken");
    }

    async fn mock_rivian_gateway(
        State(recorded): State<Arc<Mutex<Vec<RecordedRequest>>>>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> (StatusCode, Json<Value>) {
        let operation_name = body["operationName"].as_str().unwrap_or("").to_string();
        recorded.lock().unwrap().push(RecordedRequest {
            operation_name: operation_name.clone(),
            headers,
            body,
        });

        let response = match operation_name.as_str() {
            "CreateCSRFToken" => json!({
                "data": {
                    "createCsrfToken": {
                        "__typename": "CreateCSRFTokenResponse",
                        "csrfToken": "csrf-token",
                        "appSessionToken": "app-session-token"
                    }
                }
            }),
            "Login" => json!({
                "data": {
                    "login": {
                        "__typename": "MobileMFALoginResponse",
                        "otpToken": "otp-token"
                    }
                }
            }),
            "LoginWithOTP" => json!({
                "data": {
                    "loginWithOTP": {
                        "__typename": "MobileLoginResponse",
                        "accessToken": "access-token",
                        "refreshToken": "refresh-token",
                        "userSessionToken": "user-session-token"
                    }
                }
            }),
            "getUserInfo" => json!({
                "data": {
                    "currentUser": {
                        "vehicles": [{
                            "id": "vehicle-123",
                            "vin": "7FCTGAAL0NN000001",
                            "name": "Compass Yellow",
                            "vehicle": {
                                "modelYear": 2022,
                                "model": "R1T"
                            }
                        }]
                    }
                }
            }),
            _ => json!({
                "errors": [{
                    "message": "unexpected operation",
                    "extensions": { "code": "BAD_REQUEST" }
                }]
            }),
        };

        (StatusCode::OK, Json(response))
    }
}
