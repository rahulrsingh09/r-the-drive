use std::{sync::Arc, time::Duration};

use axum::{middleware, routing::get, Router};
use http::{
    header::{
        CACHE_CONTROL, CONTENT_SECURITY_POLICY, REFERRER_POLICY, STRICT_TRANSPORT_SECURITY, VARY,
        X_CONTENT_TYPE_OPTIONS, X_FRAME_OPTIONS,
    },
    HeaderValue, StatusCode,
};
use tower_governor::governor::GovernorConfigBuilder;
use tower_governor::GovernorLayer;
use tower_http::{
    compression::CompressionLayer,
    cors::{AllowHeaders, AllowMethods, AllowOrigin, CorsLayer},
    limit::RequestBodyLimitLayer,
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    set_header::SetResponseHeaderLayer,
    trace::{DefaultOnFailure, TraceLayer},
};

use crate::middleware::{
    auth::AppState,
    rate_limit::{self, RateLimitClass},
};

pub mod api_keys;
pub mod app_info;
pub mod auth;
pub mod backfill;
pub mod backups;
pub mod battery;
pub mod charging;
mod charging_sql;
pub mod charts;
pub mod cost_profiles;
pub mod dashboards;
pub mod efficiency;
pub mod efficiency_math;
pub mod external_connections;
pub mod grafana;
pub mod health;
pub mod idle_drain;
pub mod live;
pub mod locations;
pub mod metrics;
pub mod overview;
pub mod parked_energy;
pub mod places;
pub mod range_normalization;
pub mod rivian_stewardship;
pub mod schedules;
pub mod settings;
pub mod state_timeline;
pub mod themes;
pub mod trip_tag_filter;
pub mod trip_tags;
pub mod trips;
pub mod users;
pub mod users_support;
pub mod vehicles;

#[cfg(test)]
mod request_log_tests;

fn log_route(req: &axum::extract::Request) -> String {
    req.extensions()
        .get::<axum::extract::MatchedPath>()
        .map(|path| path.as_str())
        .unwrap_or("<unmatched>")
        .to_owned()
}

fn request_span(req: &axum::extract::Request) -> tracing::Span {
    tracing::debug_span!("http_request", method = %req.method(), route = %log_route(req))
}

async fn log_server_errors(
    axum::extract::State(decoding_key): axum::extract::State<jsonwebtoken::DecodingKey>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let method = req.method().clone();
    let path = log_route(&req);
    let route = req.uri().path();
    let is_api = ["/v1/", "/v2/"]
        .iter()
        .any(|prefix| route.starts_with(prefix));
    let limiter_class = classify_rate_limit_class(&method, route).as_header_value();
    let key_type = rate_limit::infer_key_type(req.headers(), &decoding_key);
    let authenticated = key_type != "ip_fallback";
    let mut response = next.run(req).await;
    let throttled = response.status() == StatusCode::TOO_MANY_REQUESTS;
    let headers = response.headers_mut();
    if is_api {
        headers.insert(
            "x-panda-ratelimit-class",
            HeaderValue::from_static(limiter_class),
        );
        if let Some(after) = headers.get("x-ratelimit-after").cloned() {
            for alias in ["retry-after", "x-panda-ratelimit-reset"] {
                headers.entry(alias).or_insert_with(|| after.clone());
            }
        }
    }
    if throttled {
        headers
            .entry("x-panda-ratelimit-source")
            .or_insert(HeaderValue::from_static("api"));
        let [limit, remaining, reset] = [
            "x-ratelimit-limit",
            "x-ratelimit-remaining",
            "x-ratelimit-after",
        ]
        .map(|key| {
            headers
                .get(key)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_owned()
        });
        tracing::warn!(%method, %path, status = %response.status(), limiter_source = "api",
            limiter_class, key_type, authenticated, limit, remaining, reset, "request rate limited");
    }
    if response.status().is_server_error() {
        tracing::error!(%method, %path, status = %response.status(), "request returned server error");
    }
    response
}

pub fn build_router(state: AppState) -> Router {
    let limits = IdentityLimits::new(&state);
    let mut namespaces = [Router::new(), Router::new()];
    for scope in RouteScope::ALL {
        let routes = registrations()
            .into_iter()
            .filter(|(registered, _)| *registered == scope)
            .fold(Router::new(), |router, (_, factory)| {
                router.merge(factory())
            });
        let routes = scope.apply(routes, &state, &limits);
        let index = usize::from(scope == RouteScope::Themes);
        namespaces[index] = std::mem::take(&mut namespaces[index]).merge(routes);
    }
    let [v1, v2] = namespaces;
    let app = Router::new()
        .route("/health", get(health))
        .nest("/v1", v1)
        .nest("/v2", v2)
        .layer(middleware::from_fn_with_state(
            state.jwt_keys.decoding.clone(),
            log_server_errors,
        ))
        .layer(cors_policy(&state))
        .layer(CompressionLayer::new())
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(request_span)
                .on_failure(DefaultOnFailure::new()),
        );
    security_headers()
        .into_iter()
        .fold(app, |router, (name, value)| {
            router.layer(SetResponseHeaderLayer::overriding(
                name,
                HeaderValue::from_static(value),
            ))
        })
        .with_state(state)
}

async fn health(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> impl axum::response::IntoResponse {
    match crate::services::redis_health::ping(&state.redis).await {
        Ok(()) => (StatusCode::OK, "ok"),
        Err(error) => {
            tracing::error!(operation = "health", error = %error, "secure_session_store.unavailable");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "secure session storage unavailable",
            )
        }
    }
}

fn classify_rate_limit_class(method: &http::Method, path: &str) -> RateLimitClass {
    const PREFIXES: &[(RateLimitClass, &[&str])] = &[
        (
            RateLimitClass::AuthPublic,
            &[
                "/v1/auth/login",
                "/v1/auth/register",
                "/v1/auth/setup",
                "/v1/auth/account-invitations/",
                "/v1/auth/bootstrap",
                "/v1/auth/refresh",
                "/v1/auth/oidc/start",
                "/v1/auth/oidc/callback",
            ],
        ),
        (
            RateLimitClass::AuthMetadata,
            &[
                "/v1/auth/me",
                "/v1/auth/preferences",
                "/v1/auth/identities",
                "/v2/auth/preferences",
                "/v2/themes",
                "/v1/dashboards/by-slug/",
            ],
        ),
    ];
    for (class, prefixes) in PREFIXES {
        if prefixes.iter().any(|prefix| path.starts_with(prefix)) {
            return *class;
        }
    }
    match path {
        "/v1/settings/timezone" => RateLimitClass::AuthMetadata,
        "/v1/vehicles/live" => RateLimitClass::HeavyRead,
        _ if path.contains("/live-session") => RateLimitClass::HeavyRead,
        _ if READ_METHODS.contains(method) => RateLimitClass::AuthRead,
        _ => RateLimitClass::AuthWrite,
    }
}

fn apply_rate_limit_settings<K, M>(
    builder: &mut GovernorConfigBuilder<K, M>,
    per_minute: u32,
    burst: u32,
    methods: Option<Vec<http::Method>>,
) where
    K: tower_governor::key_extractor::KeyExtractor,
    M: governor::middleware::RateLimitingMiddleware<governor::clock::QuantaInstant>,
{
    builder
        .burst_size(burst)
        .period(Duration::from_secs_f64(60.0 / f64::from(per_minute)));
    if let Some(methods) = methods {
        builder.methods(methods);
    }
}

#[cfg(test)]
mod full_replacement_characterization {
    use super::*;
    use axum::{body::Body, extract::ConnectInfo};
    use tower::ServiceExt;

    fn state() -> AppState {
        let keys = crate::keys::generate_keys().unwrap();
        AppState {
            pool: sqlx::postgres::PgPoolOptions::new()
                .connect_lazy("postgres://synthetic:synthetic@127.0.0.1:1/synthetic")
                .unwrap(),
            redis: redis::Client::open("redis://127.0.0.1:1/").unwrap(),
            jwt_keys: Arc::new(
                crate::middleware::auth::JwtKeys::new(&keys.jwt_private_pem, &keys.jwt_public_pem)
                    .unwrap(),
            ),
            age_key: keys.age_key,
            config: serde_json::from_value(serde_json::json!({
                "database_url":"postgres://synthetic:synthetic@127.0.0.1:1/synthetic",
                "redis_url":"redis://127.0.0.1:1/", "panda_env":"test",
                "allowed_origins":["http://localhost:4321"]
            }))
            .unwrap(),
            nominatim_cache: Default::default(),
            supervisor: crate::ingestion::supervisor::SupervisorHandle::noop(),
            resources: Default::default(),
        }
    }

    #[tokio::test]
    async fn middleware_body_budgets_and_privacy_headers() {
        let app = build_router(state());
        for (path, bytes, expected) in [
            ("/v1/auth/login", 65537, 413),
            ("/v1/charts", 65537, 413),
            ("/v1/admin/backups/imports", 65537, 401),
            ("/v2/themes", 98305, 413),
            ("/v2/themes", 65537, 401),
        ] {
            let request = http::Request::builder()
                .method("POST")
                .uri(path)
                .header("content-type", "application/json")
                .header("content-length", bytes)
                .extension(ConnectInfo(
                    "127.0.0.1:12345".parse::<std::net::SocketAddr>().unwrap(),
                ))
                .body(Body::from(vec![b' '; bytes]))
                .unwrap();
            let response = app.clone().oneshot(request).await.unwrap();
            assert_eq!(
                response.status().as_u16(),
                expected,
                "{path} with {bytes} bytes"
            );
            assert_eq!(response.headers()[X_FRAME_OPTIONS], "DENY");
            assert_eq!(response.headers()[X_CONTENT_TYPE_OPTIONS], "nosniff");
            if path == "/v2/themes" && expected != 413 {
                assert_eq!(response.headers()[CACHE_CONTROL], "private, no-store");
                assert_eq!(response.headers()[VARY], "Authorization, Cookie");
            }
        }
    }

    #[test]
    fn observable_limiter_classification_order() {
        for (path, expected) in [
            ("/v1/auth/login-extra", "auth_public"),
            ("/v1/auth/me/live-session", "auth_metadata"),
            ("/v2/themes", "auth_metadata"),
            ("/v1/settings/timezone", "auth_metadata"),
            ("/v1/settings/timezone/other", "auth_read"),
            ("/v1/vehicles/live", "heavy_read"),
            ("/unmatched/live-session/path", "heavy_read"),
            ("/v1/charts", "auth_read"),
        ] {
            assert_eq!(
                classify_rate_limit_class(&http::Method::GET, path).as_header_value(),
                expected
            );
        }
        assert_eq!(
            classify_rate_limit_class(&http::Method::PUT, "/v1/charts").as_header_value(),
            "auth_write"
        );
    }
}

const READ_METHODS: [http::Method; 3] =
    [http::Method::GET, http::Method::HEAD, http::Method::OPTIONS];
const WRITE_METHODS: [http::Method; 4] = [
    http::Method::POST,
    http::Method::PUT,
    http::Method::PATCH,
    http::Method::DELETE,
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum RouteScope {
    Public,
    Common,
    Metadata,
    Heavy,
    Upload,
    Themes,
}

type RouteFactory = fn() -> Router<AppState>;
type IdentityConfig = Arc<
    tower_governor::governor::GovernorConfig<
        rate_limit::AuthIdentityKeyExtractor,
        governor::middleware::StateInformationMiddleware,
    >,
>;

struct IdentityLimits {
    read: IdentityConfig,
    write: IdentityConfig,
    metadata: IdentityConfig,
    heavy: IdentityConfig,
}

impl IdentityLimits {
    fn new(state: &AppState) -> Self {
        let config = &state.config.rate_limit;
        let make = |per_minute, burst, methods| {
            let mut builder = GovernorConfigBuilder::default().key_extractor(
                rate_limit::AuthIdentityKeyExtractor::new(state.jwt_keys.decoding.clone()),
            );
            apply_rate_limit_settings(&mut builder, per_minute, burst, Some(methods));
            Arc::new(
                builder
                    .use_headers()
                    .finish()
                    .expect("validated rate limit configuration"),
            )
        };
        Self {
            read: make(
                config.auth_read_per_minute,
                config.auth_read_burst,
                READ_METHODS.to_vec(),
            ),
            write: make(
                config.auth_write_per_minute,
                config.auth_write_burst,
                WRITE_METHODS.to_vec(),
            ),
            metadata: make(
                config.auth_metadata_per_minute,
                config.auth_metadata_burst,
                READ_METHODS
                    .into_iter()
                    .chain([http::Method::PUT])
                    .collect(),
            ),
            heavy: make(
                config.heavy_read_per_minute,
                config.heavy_read_burst,
                READ_METHODS.to_vec(),
            ),
        }
    }
}

impl RouteScope {
    const ALL: [Self; 6] = [
        Self::Public,
        Self::Common,
        Self::Metadata,
        Self::Heavy,
        Self::Upload,
        Self::Themes,
    ];

    fn apply(
        self,
        mut router: Router<AppState>,
        state: &AppState,
        limits: &IdentityLimits,
    ) -> Router<AppState> {
        if self == Self::Themes {
            router = router
                .layer(SetResponseHeaderLayer::overriding(
                    CACHE_CONTROL,
                    HeaderValue::from_static("private, no-store"),
                ))
                .layer(SetResponseHeaderLayer::overriding(
                    VARY,
                    HeaderValue::from_static("Authorization, Cookie"),
                ));
        }
        match self {
            Self::Public => {
                let mut builder = GovernorConfigBuilder::default()
                    .key_extractor(rate_limit::TrustedProxyIpKeyExtractor);
                apply_rate_limit_settings(
                    &mut builder,
                    state.config.rate_limit.auth_public_per_minute,
                    state.config.rate_limit.auth_public_burst,
                    None,
                );
                router = router.layer(GovernorLayer::new(Arc::new(
                    builder
                        .use_headers()
                        .finish()
                        .expect("validated public rate limit configuration"),
                )));
            }
            Self::Metadata => router = router.layer(GovernorLayer::new(limits.metadata.clone())),
            Self::Heavy => {
                // The live read limiter precedes its body limit on the inbound path.
                router = router
                    .layer(RequestBodyLimitLayer::new(64 * 1024))
                    .layer(GovernorLayer::new(limits.heavy.clone()));
            }
            Self::Common | Self::Upload | Self::Themes => {
                router = router
                    .layer(GovernorLayer::new(limits.read.clone()))
                    .layer(GovernorLayer::new(limits.write.clone()));
            }
        }
        let body_limit = match self {
            Self::Upload | Self::Heavy => None,
            Self::Themes => Some(96 * 1024),
            _ => Some(64 * 1024),
        };
        if let Some(bytes) = body_limit {
            router = router.layer(RequestBodyLimitLayer::new(bytes));
        }
        if self != Self::Public {
            router = router.layer(axum::Extension(state.jwt_keys.decoding.clone()));
        }
        router
    }
}

fn registrations() -> Vec<(RouteScope, RouteFactory)> {
    use RouteScope::*;
    vec![
        (Public, auth::router),
        (Metadata, auth::metadata_router),
        (Metadata, dashboards::metadata_router),
        (Heavy, live::router),
        (Upload, backups::upload_router),
        (Themes, themes::router),
        (Common, app_info::router),
        (Common, api_keys::router),
        (Common, backups::router),
        (Common, backfill::router),
        (Common, vehicles::router),
        (Common, battery::router),
        (Common, trips::router),
        (Common, trip_tags::router),
        (Common, charging::router),
        (Common, charts::router),
        (Common, efficiency::router),
        (Common, external_connections::router),
        (Common, metrics::router),
        (Common, dashboards::router),
        (Common, cost_profiles::router),
        (Common, places::router),
        (Common, rivian_stewardship::router),
        (Common, schedules::router),
        (Common, overview::router),
        (Common, parked_energy::router),
        (Common, state_timeline::router),
        (Common, health::router),
        (Common, idle_drain::router),
        (Common, locations::router),
        (Common, grafana::router),
        (Common, auth::protected_router),
        (Common, settings::router),
        (Common, users::router),
    ]
}

fn cors_policy(state: &AppState) -> CorsLayer {
    let origins = state
        .config
        .allowed_origins
        .iter()
        .flat_map(|origin| origin.parse::<HeaderValue>())
        .collect::<Vec<_>>();
    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods(AllowMethods::list([
            http::Method::GET,
            http::Method::POST,
            http::Method::PUT,
            http::Method::PATCH,
            http::Method::DELETE,
        ]))
        .allow_headers(AllowHeaders::list([
            http::header::AUTHORIZATION,
            http::header::CONTENT_TYPE,
            http::header::ACCEPT,
            http::header::IF_MATCH,
            http::HeaderName::from_static("x-theme-preferences-if-match"),
        ]))
        .expose_headers([http::header::ETAG])
        .allow_credentials(true)
}

fn security_headers() -> [(http::HeaderName, &'static str); 5] {
    [
        (STRICT_TRANSPORT_SECURITY, "max-age=31536000; includeSubDomains"),
        (X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (X_FRAME_OPTIONS, "DENY"),
        (REFERRER_POLICY, "no-referrer"),
        (CONTENT_SECURITY_POLICY, "default-src 'self'; img-src 'self' data: blob:; style-src 'self' 'unsafe-inline'; script-src 'self'; connect-src 'self'; font-src 'self' data:; worker-src 'self' blob:; object-src 'none'; base-uri 'self'; form-action 'self'; frame-ancestors 'none'"),
    ]
}

fn route_table<const N: usize>(
    entries: [(&'static str, axum::routing::MethodRouter<AppState>); N],
) -> Router<AppState> {
    let mut contracts =
        std::collections::BTreeMap::<&str, axum::routing::MethodRouter<AppState>>::new();
    for (path, methods) in entries {
        assert!(path.starts_with('/'), "invalid route contract: {path}");
        let accumulated = contracts.remove(path).unwrap_or_default();
        contracts.insert(path, accumulated.merge(methods));
    }
    contracts
        .into_iter()
        .fold(Router::new(), |router, (path, methods)| {
            router.route(path, methods)
        })
}

fn time_bounds(
    from: Option<chrono::DateTime<chrono::Utc>>,
    to: Option<chrono::DateTime<chrono::Utc>>,
    lifetime: bool,
    default_days: i64,
) -> (chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>) {
    let end = to.unwrap_or_else(chrono::Utc::now);
    let start = match (lifetime, from) {
        (true, _) => chrono::DateTime::UNIX_EPOCH,
        (false, Some(start)) => start,
        (false, None) => chrono::Utc::now() - chrono::Duration::days(default_days),
    };
    (start, end)
}

struct VehicleRequest<'a> {
    auth: &'a crate::middleware::auth::AuthUser,
    id: uuid::Uuid,
}
impl<'a> VehicleRequest<'a> {
    fn select(
        auth: &'a crate::middleware::auth::AuthUser,
        id: Option<uuid::Uuid>,
    ) -> Result<Self, crate::errors::AppError> {
        let id =
            id.ok_or_else(|| crate::errors::AppError::Validation("vehicle_id required".into()))?;
        crate::middleware::auth::require_vehicle_access(auth, id)?;
        Ok(Self { auth, id })
    }
    async fn read(self, state: &AppState) -> Result<Self, crate::errors::AppError> {
        crate::db::vehicles::require_vehicle_read_access(&state.pool, self.auth, self.id).await?;
        Ok(self)
    }
}

async fn installation_admin(
    pool: &sqlx::PgPool,
    user_id: uuid::Uuid,
) -> Result<bool, crate::errors::AppError> {
    let allowed: Option<bool> = sqlx::query_scalar(
        "SELECT role=ANY(ARRAY['admin','super_user']) FROM panda.users WHERE id=$1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    Ok(allowed.unwrap_or(false))
}

enum RecordOperation {
    Read,
    Write { administrator: bool },
}
fn authorize_record(
    owner: Option<uuid::Uuid>,
    locked: bool,
    user: uuid::Uuid,
    operation: RecordOperation,
) -> Result<(), crate::errors::AppError> {
    let allowed = match operation {
        RecordOperation::Read => owner.is_none_or(|owner| owner == user),
        RecordOperation::Write {
            administrator: true,
        } => true,
        RecordOperation::Write {
            administrator: false,
        } => !locked && owner == Some(user),
    };
    allowed
        .then_some(())
        .ok_or(crate::errors::AppError::Forbidden)
}
