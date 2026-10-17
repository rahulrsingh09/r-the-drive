//! Provider-neutral OIDC protocol helpers.
//! Secrets and provider tokens stay server-side; Redis holds only short-lived
//! authorization transactions.
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use openidconnect::{
    core::{
        CoreAuthDisplay, CoreAuthPrompt, CoreErrorResponseType, CoreGenderClaim, CoreJsonWebKey,
        CoreJweContentEncryptionAlgorithm, CoreJwsSigningAlgorithm, CoreProviderMetadata,
        CoreRevocableToken, CoreRevocationErrorResponse, CoreTokenIntrospectionResponse,
        CoreTokenType,
    },
    AdditionalClaims, AsyncHttpClient, AuthType, AuthorizationCode, Client, ClientId, ClientSecret,
    EmptyExtraTokenFields, HttpRequest, HttpResponse, IdTokenFields, IssuerUrl, Nonce,
    OAuth2TokenResponse, PkceCodeVerifier, RedirectUrl, StandardErrorResponse,
    StandardTokenResponse, SubjectIdentifier,
};
use rand::{distributions::Alphanumeric, Rng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::{future::Future, pin::Pin};
use url::Url;

use crate::{
    errors::AppError,
    services::authentication_settings::{self, EffectiveAuthenticationSettings},
};

/// OIDC providers commonly put authorization data such as `groups` or a
/// tenant marker in non-core ID-token claims.  Flattening preserves those
/// values while the surrounding `openidconnect` types still verify the token
/// before this map is ever exposed to policy evaluation.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct ProviderClaims {
    #[serde(flatten)]
    values: BTreeMap<String, serde_json::Value>,
}
impl AdditionalClaims for ProviderClaims {}

type OidcTokenFields = IdTokenFields<
    ProviderClaims,
    EmptyExtraTokenFields,
    CoreGenderClaim,
    CoreJweContentEncryptionAlgorithm,
    CoreJwsSigningAlgorithm,
>;
type OidcTokenResponse = StandardTokenResponse<OidcTokenFields, CoreTokenType>;

#[derive(Debug, thiserror::Error)]
enum HttpsOnlyClientError {
    #[error("OIDC provider attempted a non-HTTPS request")]
    UnsafeEndpoint,
    #[error(transparent)]
    Request(#[from] AppError),
}

struct HttpsOnlyClient(crate::config::SecurityConfig);

type OidcClient<
    HasAuthUrl = openidconnect::EndpointNotSet,
    HasDeviceAuthUrl = openidconnect::EndpointNotSet,
    HasIntrospectionUrl = openidconnect::EndpointNotSet,
    HasRevocationUrl = openidconnect::EndpointNotSet,
    HasTokenUrl = openidconnect::EndpointNotSet,
    HasUserInfoUrl = openidconnect::EndpointNotSet,
> = Client<
    ProviderClaims,
    CoreAuthDisplay,
    CoreGenderClaim,
    CoreJweContentEncryptionAlgorithm,
    CoreJsonWebKey,
    CoreAuthPrompt,
    StandardErrorResponse<CoreErrorResponseType>,
    OidcTokenResponse,
    CoreTokenIntrospectionResponse,
    CoreRevocableToken,
    CoreRevocationErrorResponse,
    HasAuthUrl,
    HasDeviceAuthUrl,
    HasIntrospectionUrl,
    HasRevocationUrl,
    HasTokenUrl,
    HasUserInfoUrl,
>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    pub state: String,
    /// SHA-256 of a separate browser-only cookie; it is intentionally not the
    /// OAuth state value so a leaked authorization URL cannot satisfy binding.
    pub browser_binding: String,
    pub nonce: String,
    pub verifier: String,
    pub return_to: String,
    pub user_id: Option<uuid::Uuid>,
    pub link: bool,
    /// Argon2 hash staged for an initial-password operation. The hash never
    /// reaches the browser, and callback completion still requires the exact
    /// OIDC identity that is already linked to `user_id`.
    #[serde(default)]
    pub pending_password_hash: Option<String>,
    #[serde(default)]
    pub invitation_id: Option<uuid::Uuid>,
}

/// Identity information is constructed only after `openidconnect` has verified
/// the ID-token signature, issuer, audience/azp, expiry, and nonce.
#[derive(Debug, Clone)]
pub struct VerifiedIdentity {
    pub issuer: String,
    pub subject: String,
    pub email: Option<String>,
    pub email_verified: bool,
    pub claims: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProviderMetadata {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    #[serde(default)]
    pub jwks_uri: Option<String>,
    #[serde(default)]
    pub token_endpoint_auth_methods_supported: Vec<String>,
    #[serde(default)]
    pub code_challenge_methods_supported: Vec<String>,
}

pub fn random_token() -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(48)
        .map(char::from)
        .collect()
}
pub fn browser_binding(cookie_value: &str) -> String {
    hex::encode(Sha256::digest(cookie_value.as_bytes()))
}
pub fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}
impl<'c> AsyncHttpClient<'c> for HttpsOnlyClient {
    type Error = HttpsOnlyClientError;
    type Future =
        Pin<Box<dyn Future<Output = Result<HttpResponse, Self::Error>> + Send + Sync + 'c>>;
    fn call(&'c self, request: HttpRequest) -> Self::Future {
        Box::pin(self.execute(request))
    }
}

impl HttpsOnlyClient {
    async fn execute(&self, request: HttpRequest) -> Result<HttpResponse, HttpsOnlyClientError> {
        let (head, body) = request.into_parts();
        let destination = head.uri.to_string();
        validate_https_provider_endpoint("request", &destination)
            .map_err(|_| HttpsOnlyClientError::UnsafeEndpoint)?;
        let url = Url::parse(&destination).map_err(|_| HttpsOnlyClientError::UnsafeEndpoint)?;
        let allowed =
            crate::services::outbound::operator_allowlist(&self.0.oidc_private_network_allowlist)?;
        let transport = crate::services::outbound::outbound_client_for_url(&url, &allowed).await?;
        let response = transport
            .request(head.method, url)
            .body(body)
            .headers(head.headers)
            .send()
            .await
            .map_err(|_| AppError::DependencyUnavailable("OIDC request failed".into()))?;
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = crate::services::outbound::read_response_limited(
            response,
            self.0.oidc_max_response_bytes,
            "OIDC provider",
        )
        .await?;
        let mut http = HttpResponse::new(bytes);
        *http.status_mut() = status;
        *http.headers_mut() = headers;
        Ok(http)
    }
}

fn https_only_client() -> Result<HttpsOnlyClient, AppError> {
    crate::services::outbound::operator_security().map(HttpsOnlyClient)
}

pub fn validate_return_to(value: Option<&str>) -> Result<String, AppError> {
    let target = value.unwrap_or("/");
    let forbidden = [
        !target.starts_with('/'),
        target.starts_with("//"),
        target.contains('\\'),
        target.to_ascii_lowercase().contains("%5c"),
        target.starts_with("/login?"),
        target == "/login",
    ];
    if forbidden.into_iter().any(|test| test) {
        return Err(AppError::Validation(
            "return_to must be a relative application path".into(),
        ));
    }
    let address = Url::parse(&["http://panda.invalid", target].concat())
        .map_err(|_| AppError::Validation("invalid return_to".into()))?;
    match address.host_str() {
        Some("panda.invalid") => Ok(address[url::Position::BeforePath..].to_owned()),
        _ => Err(AppError::Validation("return_to must be internal".into())),
    }
}

fn configured_issuer(settings: &EffectiveAuthenticationSettings) -> Result<IssuerUrl, AppError> {
    let Some(value) = settings.issuer_url.as_ref() else {
        return Err(AppError::Validation("OIDC issuer is not configured".into()));
    };
    match IssuerUrl::new(value.clone()) {
        Ok(issuer) if issuer.url().scheme() == "https" => Ok(issuer),
        Ok(_) => Err(AppError::Validation("OIDC issuer must use HTTPS".into())),
        Err(_) => Err(AppError::Validation("OIDC issuer is invalid".into())),
    }
}

pub async fn discover(
    settings: &EffectiveAuthenticationSettings,
) -> Result<ProviderMetadata, AppError> {
    let issuer = configured_issuer(settings)?;
    let endpoint = Url::parse(&format!(
        "{}/.well-known/openid-configuration",
        issuer.as_str().trim_end_matches('/')
    ))
    .map_err(|_| AppError::Validation("OIDC issuer is invalid".into()))?;
    let security = crate::services::outbound::operator_security()?;
    let allowed =
        crate::services::outbound::operator_allowlist(&security.oidc_private_network_allowlist)?;
    let transport = crate::services::outbound::outbound_client_for_url(&endpoint, &allowed).await?;
    let response = transport
        .get(endpoint)
        .send()
        .await
        .map_err(|_| AppError::Validation("OIDC provider is unavailable".into()))?;
    let response = response
        .error_for_status()
        .map_err(|_| AppError::Validation("OIDC provider discovery failed".into()))?;
    let metadata: ProviderMetadata = crate::services::outbound::read_json(
        response,
        security.oidc_max_response_bytes,
        "OIDC discovery",
    )
    .await?;
    if metadata.issuer.trim_end_matches('/') != issuer.as_str().trim_end_matches('/') {
        return Err(AppError::Validation("OIDC issuer mismatch".into()));
    }
    for (name, endpoint) in [
        (
            "authorization",
            Some(metadata.authorization_endpoint.as_str()),
        ),
        ("token", Some(metadata.token_endpoint.as_str())),
        ("JWKS", metadata.jwks_uri.as_deref()),
    ] {
        if let Some(endpoint) = endpoint {
            validate_https_provider_endpoint(name, endpoint)?;
        }
    }
    if !settings.oidc_enabled {
        return Err(AppError::Validation("OIDC is disabled".into()));
    }
    match metadata.code_challenge_methods_supported.as_slice() {
        [] => Ok(metadata),
        methods if methods.iter().any(|method| method == "S256") => Ok(metadata),
        _ => Err(AppError::Validation(
            "OIDC provider does not support PKCE S256".into(),
        )),
    }
}

fn validate_https_provider_endpoint(name: &str, value: &str) -> Result<(), AppError> {
    let address = Url::parse(value)
        .map_err(|_| AppError::Validation(format!("OIDC {name} endpoint is invalid")))?;
    let safe = address.scheme() == "https"
        && address.host_str().is_some()
        && address.username().is_empty()
        && address.password().is_none()
        && address.fragment().is_none();
    if safe {
        Ok(())
    } else {
        Err(AppError::Validation(format!(
            "OIDC {name} endpoint must be absolute HTTPS without credentials or fragment"
        )))
    }
}

fn callback_url(settings: &EffectiveAuthenticationSettings) -> Result<RedirectUrl, AppError> {
    authentication_settings::oidc_callback_url(settings.public_base_url.as_deref()).and_then(
        |callback| {
            RedirectUrl::new(callback)
                .map_err(|_| AppError::Validation("OIDC callback URL is invalid".into()))
        },
    )
}

fn validate_client_configuration(
    settings: &EffectiveAuthenticationSettings,
) -> Result<(), AppError> {
    for (value, message) in [
        (
            settings.client_id.as_deref(),
            "OIDC client ID is not configured",
        ),
        (
            settings.client_secret.as_deref(),
            "OIDC client secret is not configured",
        ),
    ] {
        if value.is_none_or(|s| s.trim().is_empty()) {
            return Err(AppError::Validation(message.into()));
        }
    }
    authentication_settings::normalize_oidc_scopes(&settings.scopes)?;
    if ["auto", "client_secret_basic", "client_secret_post"]
        .contains(&settings.token_auth_method.as_str())
    {
        Ok(())
    } else {
        Err(AppError::Validation(
            "OIDC token authentication method is invalid".into(),
        ))
    }
}

async fn secure_metadata(
    settings: &EffectiveAuthenticationSettings,
) -> Result<CoreProviderMetadata, AppError> {
    if !settings.oidc_enabled {
        return Err(AppError::NotFound);
    }
    let issuer = configured_issuer(settings)?;
    let transport = https_only_client()?;
    let metadata = CoreProviderMetadata::discover_async(issuer, &transport)
        .await
        .map_err(|_| {
            AppError::Validation("OIDC provider discovery or JWKS retrieval failed".into())
        })?;
    validate_https_provider_endpoint("authorization", metadata.authorization_endpoint().as_str())?;
    let Some(token) = metadata.token_endpoint() else {
        return Err(AppError::Validation(
            "OIDC provider has no token endpoint".into(),
        ));
    };
    for (name, endpoint) in [
        ("token", Some(token.as_str())),
        ("JWKS", Some(metadata.jwks_uri().as_str())),
        ("UserInfo", metadata.userinfo_endpoint().map(|u| u.as_str())),
    ] {
        if let Some(endpoint) = endpoint {
            validate_https_provider_endpoint(name, endpoint)?;
        }
    }
    Ok(metadata)
}

pub fn provider_configuration_ready(settings: &EffectiveAuthenticationSettings) -> bool {
    let mut candidate = settings.clone();
    candidate.issuer_url = candidate.issuer_url.map(|s| s.trim().to_owned());
    configured_issuer(&candidate)
        .and_then(|_| callback_url(settings))
        .and_then(|_| validate_client_configuration(settings))
        .is_ok()
}

pub async fn exchange_and_verify(
    settings: &EffectiveAuthenticationSettings,
    transaction: &Transaction,
    code: &str,
) -> Result<VerifiedIdentity, AppError> {
    let redirect = callback_url(settings)?;
    let metadata = secure_metadata(settings).await?;
    let issuer = metadata.issuer().as_str().trim_end_matches('/').to_owned();
    validate_client_configuration(settings)?;
    let client = OidcClient::from_provider_metadata(
        metadata,
        ClientId::new(settings.client_id.clone().ok_or(AppError::Unauthorized)?),
        Some(ClientSecret::new(
            settings
                .client_secret
                .clone()
                .ok_or(AppError::Unauthorized)?,
        )),
    )
    .set_redirect_uri(redirect);
    let client = match settings.token_auth_method.as_str() {
        "client_secret_basic" => client.set_auth_type(AuthType::BasicAuth),
        "client_secret_post" => client.set_auth_type(AuthType::RequestBody),
        _ => client,
    };
    let transport = https_only_client()?;
    let exchange = client
        .exchange_code(AuthorizationCode::new(code.to_owned()))
        .map_err(|_| AppError::Unauthorized)?;
    let response = exchange
        .set_pkce_verifier(PkceCodeVerifier::new(transaction.verifier.clone()))
        .request_async(&transport)
        .await
        .map_err(|_| AppError::Unauthorized)?;
    let token = response
        .extra_fields()
        .id_token()
        .ok_or(AppError::Unauthorized)?;
    let claims = token
        .claims(
            &client.id_token_verifier(),
            &Nonce::new(transaction.nonce.clone()),
        )
        .map_err(|_| AppError::Unauthorized)?;
    let subject = claims.subject().as_str().to_owned();
    let mut email = claims
        .email()
        .map(|e| e.as_str().trim().to_ascii_lowercase())
        .filter(|e| !e.is_empty());
    let mut verified = claims.email_verified() == Some(true);
    let mut admitted_claims = claims.additional_claims().values.clone();
    let missing_policy = settings
        .required_claim_name
        .as_ref()
        .is_some_and(|name| !admitted_claims.contains_key(name));
    let needs_userinfo =
        (!verified || email.is_none() || missing_policy) && client.user_info_url().is_some();
    if needs_userinfo {
        // UserInfo is accepted only with the ID token's verified subject binding.
        let request = client.user_info(
            response.access_token().clone(),
            Some(SubjectIdentifier::new(subject.clone())),
        );
        if let Ok(request) = request {
            if let Ok(info) = request
                .request_async::<ProviderClaims, _, CoreGenderClaim>(&transport)
                .await
            {
                if (!verified || email.is_none()) && info.email_verified() == Some(true) {
                    email = info
                        .email()
                        .map(|e| e.as_str().trim().to_ascii_lowercase())
                        .filter(|e| !e.is_empty());
                    verified = email.is_some();
                }
                merge_missing_claims(&mut admitted_claims, &info.additional_claims().values);
            }
        }
    }
    Ok(VerifiedIdentity {
        issuer,
        subject,
        email,
        email_verified: verified,
        claims: serde_json::to_value(admitted_claims).map_err(|_| AppError::Unauthorized)?,
    })
}

fn merge_missing_claims(
    target: &mut BTreeMap<String, serde_json::Value>,
    fallback: &BTreeMap<String, serde_json::Value>,
) {
    let mut merged = fallback.clone();
    merged.append(target);
    *target = merged;
}

pub async fn test_provider(settings: &EffectiveAuthenticationSettings) -> Result<(), AppError> {
    callback_url(settings)?;
    secure_metadata(settings)
        .await
        .and_then(|_| validate_client_configuration(settings))
}

pub fn claim_matches(
    identity: &VerifiedIdentity,
    name: Option<&str>,
    expected: Option<&str>,
) -> bool {
    if name.is_none() && expected.is_none() {
        return true;
    }
    let Some((name, expected)) = name
        .zip(expected)
        .filter(|(n, v)| !n.trim().is_empty() && !v.trim().is_empty())
    else {
        return false;
    };
    let Some(claim) = identity.claims.get(name) else {
        return false;
    };
    if let Some(values) = claim.as_array() {
        return values
            .iter()
            .filter_map(serde_json::Value::as_str)
            .any(|value| value == expected);
    }
    claim.as_str() == Some(expected)
}

pub fn domain_allowed(email: &str, domains: &[String]) -> bool {
    match (domains.is_empty(), email.rsplit_once('@')) {
        (true, _) => true,
        (false, Some((_, domain))) => domains
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(domain)),
        _ => false,
    }
}

pub fn authorization_url(
    metadata: &ProviderMetadata,
    settings: &EffectiveAuthenticationSettings,
    tx: &Transaction,
    callback: &str,
) -> Result<Url, AppError> {
    validate_client_configuration(settings)?;
    let scopes = authentication_settings::normalize_oidc_scopes(&settings.scopes)?;
    let challenge = pkce_challenge(&tx.verifier);
    let mut authorization = Url::parse(&metadata.authorization_endpoint)
        .map_err(|_| AppError::Validation("OIDC authorization endpoint is invalid".into()))?;
    let id = settings
        .client_id
        .as_deref()
        .ok_or_else(|| AppError::Validation("OIDC client ID is not configured".into()))?;
    let parameters = [
        ("response_type", "code"),
        ("client_id", id),
        ("redirect_uri", callback),
        ("scope", scopes.as_str()),
        ("state", tx.state.as_str()),
        ("nonce", tx.nonce.as_str()),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
    ];
    authorization.query_pairs_mut().extend_pairs(parameters);
    Ok(authorization)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pkce_is_url_safe_and_deterministic() {
        assert_eq!(pkce_challenge("abc"), pkce_challenge("abc"));
        assert!(!pkce_challenge("abc").contains('='));
    }
    #[test]
    fn rejects_external_return_targets() {
        assert!(validate_return_to(Some("https://evil.invalid")).is_err());
        assert!(validate_return_to(Some("//evil.invalid")).is_err());
        assert!(validate_return_to(Some("/\\attacker.example")).is_err());
        assert!(validate_return_to(Some("/%5Cattacker.example")).is_err());
        assert!(validate_return_to(Some("/%5cattacker.example")).is_err());
    }

    #[test]
    fn preserves_safe_internal_return_query_and_fragment() {
        assert_eq!(
            validate_return_to(Some("/page?next=%2Fdashboard#details")).unwrap(),
            "/page?next=%2Fdashboard#details"
        );
    }

    #[test]
    fn browser_binding_is_not_the_oauth_state_value() {
        assert_ne!(browser_binding("state-value"), "state-value");
        assert_ne!(browser_binding("one"), browser_binding("two"));
    }

    #[test]
    fn accepts_path_segment_issuer_discovery_url() {
        let issuer = "https://id.example.test/realms/panda";
        let discovery = format!(
            "{}/.well-known/openid-configuration",
            issuer.trim_end_matches('/')
        );
        assert_eq!(
            Url::parse(&discovery).unwrap().path(),
            "/realms/panda/.well-known/openid-configuration"
        );
    }

    fn verified_identity_with_claim(name: &str, value: serde_json::Value) -> VerifiedIdentity {
        VerifiedIdentity {
            issuer: "https://issuer.example.test".into(),
            subject: "subject".into(),
            email: None,
            email_verified: false,
            claims: serde_json::json!({ name: value }),
        }
    }

    #[test]
    fn required_claim_matches_verified_scalar_claim() {
        let identity = verified_identity_with_claim("tenant", serde_json::json!("rivian"));
        assert!(claim_matches(&identity, Some("tenant"), Some("rivian")));
        assert!(!claim_matches(&identity, Some("tenant"), Some("other")));
    }

    #[test]
    fn required_claim_matches_verified_array_claim() {
        let identity =
            verified_identity_with_claim("groups", serde_json::json!(["users", "fleet"]));
        assert!(claim_matches(&identity, Some("groups"), Some("fleet")));
        assert!(!claim_matches(&identity, Some("groups"), Some("admins")));
    }

    #[test]
    fn userinfo_only_fills_claims_missing_from_the_verified_id_token() {
        let mut id_token = BTreeMap::from([
            ("groups".into(), serde_json::json!(["fleet"])),
            ("tenant".into(), serde_json::json!("primary")),
        ]);
        let userinfo = BTreeMap::from([
            ("tenant".into(), serde_json::json!("different")),
            ("region".into(), serde_json::json!("us-central")),
        ]);
        merge_missing_claims(&mut id_token, &userinfo);
        assert_eq!(id_token["tenant"], serde_json::json!("primary"));
        assert_eq!(id_token["region"], serde_json::json!("us-central"));
    }

    #[test]
    fn readiness_rejects_empty_fields_insecure_issuers_and_missing_openid_scope() {
        let mut settings = EffectiveAuthenticationSettings {
            oidc_enabled: true,
            password_login_enabled: true,
            issuer_url: Some("https://issuer.example".into()),
            public_base_url: Some("https://panda.example".into()),
            client_id: Some("client".into()),
            client_secret: Some("secret".into()),
            button_label: "Sign in with SSO".into(),
            scopes: "openid email profile".into(),
            token_auth_method: "auto".into(),
            auto_signup: false,
            auto_link_verified_email: false,
            oidc_auto_login: false,
            allowed_email_domains: vec![],
            required_claim_name: None,
            required_claim_value: None,
        };
        assert!(provider_configuration_ready(&settings));
        settings.scopes = "email profile".into();
        assert!(!provider_configuration_ready(&settings));
        settings.scopes = "openid email".into();
        settings.issuer_url = Some("http://issuer.example".into());
        assert!(!provider_configuration_ready(&settings));
        settings.issuer_url = Some("https://issuer.example".into());
        settings.client_id = Some(" ".into());
        assert!(!provider_configuration_ready(&settings));
    }

    #[test]
    fn provider_endpoints_require_safe_https_urls() {
        assert!(validate_https_provider_endpoint(
            "authorization",
            "https://identity.example/authorize?tenant=fleet"
        )
        .is_ok());
        for invalid in [
            "http://identity.example/authorize",
            "https://user:password@identity.example/authorize",
            "https://identity.example/authorize#fragment",
            "not a URL",
        ] {
            assert!(
                validate_https_provider_endpoint("authorization", invalid).is_err(),
                "accepted {invalid}"
            );
        }
    }
}
