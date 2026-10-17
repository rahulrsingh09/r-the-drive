mod cache;
mod catalog;
mod preferences;
mod settings;

use crate::private_deployment::outbound::optional_client_for_url as outbound_client_for_url;
use crate::services::outbound::{
    configured_private_network_allowlist, endpoint_is_private, is_forbidden_ip,
    is_link_local_or_metadata, is_private_ip, parse_private_network_allowlist,
    read_response_limited,
};

use axum::{
    body::Body,
    extract::{OriginalUri, Path, Query, State},
    http::{header, HeaderMap, HeaderValue, Response},
    routing::{get, post, put},
    Json, Router,
};
use base64::Engine;
use chrono::{DateTime, Utc};
use ipnet::IpNet;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{
    db::users::{get_user_role, require_admin_or_super_user, UserRole},
    errors::AppError,
    ingestion::session_store::encrypt_json,
    middleware::auth::{AppState, AuthUser},
    services::external_connections::{self as connections, ConnectionSettingsRow},
};

const BASEMAP_RASTER_ROUTE: &str = "/external/basemap/raster/{style}/{z}/{x}/{y}";
const OPENFREEMAP_PROXY_ROUTE: &str = "/external/basemap/openfreemap/{*resource}";
// Bump when cached OpenFreeMap responses change representation. The first
// Bump this whenever the cached response representation or rewritten dependent
// URL contract changes. It versions both Redis entries and browser-facing URLs.
const OPENFREEMAP_CACHE_FORMAT: &str = "v3";
// MapLibre requires style sprite URLs to be absolute before transformRequest
// runs. The browser rewrites this reserved origin to its current Panda
// origin and attaches authentication, so it is never contacted directly.
const OPENFREEMAP_PROXY_ORIGIN: &str = "https://panda.invalid";

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/settings/external-connections", get(list_connections))
        .route(
            "/settings/external-connections/disable-optional",
            post(disable_optional),
        )
        .route(
            "/settings/external-connections/{id}",
            put(update_connection),
        )
        .route(
            "/settings/external-connections/{id}/test",
            post(test_connection),
        )
        .route(
            "/settings/external-connections/{id}/cache/purge",
            post(purge_connection_cache),
        )
        .route(BASEMAP_RASTER_ROUTE, get(proxy_basemap_tile))
        .route("/external/basemap/config", get(basemap_config))
        .route(OPENFREEMAP_PROXY_ROUTE, get(proxy_openfreemap_resource))
        .route("/external/iconify/search", get(proxy_iconify_search))
        .route("/external/iconify/{resource}", get(proxy_iconify_resource))
}

#[derive(Debug, Serialize)]
struct ExternalConnectionsResponse {
    can_manage: bool,
    connections: Vec<ConnectionResponse>,
}

#[derive(Debug, Serialize)]
struct ConnectionResponse {
    id: String,
    name: &'static str,
    purpose: &'static str,
    data_shared: &'static [&'static str],
    disabled_effect: &'static str,
    execution: &'static str,
    privacy_url: Option<&'static str>,
    terms_url: Option<&'static str>,
    editable: bool,
    enabled: bool,
    mode: String,
    basemap_provider: Option<String>,
    endpoint: Option<String>,
    endpoint_is_private: bool,
    weather_precision: Option<String>,
    forecast_url: Option<String>,
    archive_url: Option<String>,
    base_url: Option<String>,
    light_url_template: Option<String>,
    dark_url_template: Option<String>,
    attribution: Option<String>,
    attribution_url: Option<String>,
    request_identifier: Option<String>,
    custom_autocomplete: bool,
    /// Deprecated compatibility field. New clients should use
    /// `private_network_allowlist` and `private_network_policy_state`.
    allow_private_network: bool,
    private_network_allowlist: Vec<String>,
    private_network_policy_state: String,
    has_api_key: bool,
    has_bearer_token: bool,
    updated_at: DateTime<Utc>,
    last_attempt_at: Option<DateTime<Utc>>,
    last_success_at: Option<DateTime<Utc>>,
    last_error: Option<String>,
    request_count_today: i32,
    last_test_at: Option<DateTime<Utc>>,
    last_test_ok: Option<bool>,
    last_test_error: Option<String>,
    credential_issued_at: Option<DateTime<Utc>>,
    expected_renewal_at: Option<DateTime<Utc>>,
    renewal_state: Option<connections::RivianRenewalState>,
    observed_health: Option<String>,
    observed_error: Option<String>,
    cache: Option<ConnectionCacheResponse>,
}

#[derive(Debug, Serialize)]
struct ConnectionCacheResponse {
    entries: u64,
    bytes: u64,
    persistent: bool,
    purgeable: bool,
    description: &'static str,
}

struct ConnectionDefinition {
    id: &'static str,
    name: &'static str,
    purpose: &'static str,
    data_shared: &'static [&'static str],
    disabled_effect: &'static str,
    execution: &'static str,
    privacy_url: Option<&'static str>,
    terms_url: Option<&'static str>,
    editable: bool,
}

const DEFINITIONS: &[ConnectionDefinition] = &[
    ConnectionDefinition { id: connections::RIVIAN_ACCOUNT, name: "Rivian account", purpose: "Read-only vehicle telemetry and charging history.", data_shared: &["Rivian account tokens", "Vehicle identifiers", "Telemetry and history queries"], disabled_effect: "Disconnecting a vehicle stops telemetry and history import. Existing local data remains.", execution: "Server", privacy_url: Some("https://rivian.com/legal/privacy"), terms_url: Some("https://rivian.com/legal/terms"), editable: false },
    ConnectionDefinition { id: connections::OPEN_METEO, name: "Open-Meteo weather", purpose: "Estimated outside temperature along completed drives.", data_shared: &["Rounded drive coordinates by default", "Drive date", "Temperature variable request"], disabled_effect: "New drives will not receive estimated outside temperatures or temperature-based efficiency data. Existing values remain.", execution: "Server", privacy_url: Some("https://open-meteo.com/en/terms"), terms_url: Some("https://open-meteo.com/en/terms"), editable: true },
    ConnectionDefinition { id: connections::NOMINATIM, name: "OpenStreetMap Nominatim", purpose: "Address search and readable trip endpoint labels.", data_shared: &["Exact coordinate for reverse geocoding", "Search text after explicit submit"], disabled_effect: "Address search and new automatic trip labels stop. Coordinates, saved places, and cached labels remain.", execution: "Server", privacy_url: Some("https://osmfoundation.org/wiki/Privacy_Policy"), terms_url: Some("https://operations.osmfoundation.org/policies/nominatim/"), editable: true },
    ConnectionDefinition { id: connections::BASEMAP, name: "Map basemap", purpose: "Street and geographic context behind exact trip routes.", data_shared: &["Requested map areas and resource paths", "Panda server IP"], disabled_effect: "Routes remain visible on a neutral background, without streets or place context.", execution: "Server proxy", privacy_url: Some("https://carto.com/privacy/"), terms_url: Some("https://carto.com/legal/"), editable: true },
    ConnectionDefinition { id: connections::ICONIFY, name: "Iconify catalog", purpose: "Search and load dashboard icons not bundled locally.", data_shared: &["Icon names", "Explicit icon search text"], disabled_effect: "Remote icon search stops; bundled icons and local fallbacks remain.", execution: "Server proxy", privacy_url: Some("https://iconify.design/privacy/"), terms_url: Some("https://iconify.design/terms/"), editable: true },
    ConnectionDefinition { id: connections::S3_BACKUP, name: "S3-compatible backups", purpose: "Optional off-site backup storage managed in Backups.", data_shared: &["Backup artifact", "Configured object-store credentials"], disabled_effect: "New off-site backups stop; local backup behavior and existing objects remain.", execution: "Server", privacy_url: None, terms_url: None, editable: false },
];

async fn list_connections(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<ExternalConnectionsResponse>, AppError> {
    let role = get_user_role(&state.pool, auth.user_id).await?;
    let can_manage = matches!(role, UserRole::Admin | UserRole::SuperUser);
    Ok(Json(build_response(&state, can_manage).await?))
}

async fn build_response(
    state: &AppState,
    can_manage: bool,
) -> Result<ExternalConnectionsResponse, AppError> {
    let rows = connections::list(&state.pool).await?;
    let backup = sqlx::query_as::<_, (bool, Option<String>)>(
        "SELECT s3_enabled, NULLIF(endpoint, '') FROM panda.backup_settings WHERE id = TRUE",
    )
    .fetch_optional(&state.pool)
    .await?
    .unwrap_or((false, None));
    let mut result = Vec::with_capacity(rows.len());
    let rivian_status = connections::rivian_status(&state.pool).await?;
    for (settings, activity) in rows {
        let Some(definition) = DEFINITIONS.iter().find(|item| item.id == settings.id) else {
            continue;
        };
        let presentation = catalog::Presentation::resolve(&settings, &backup, definition);
        let cache = cache_summary(state, &settings.id).await;
        result.push(ConnectionResponse {
            id: settings.id.clone(),
            name: definition.name,
            purpose: definition.purpose,
            data_shared: definition.data_shared,
            disabled_effect: definition.disabled_effect,
            execution: definition.execution,
            privacy_url: presentation.privacy_url,
            terms_url: presentation.terms_url,
            editable: definition.editable
                && can_manage
                && connections::connection_allowed(&settings.id),
            enabled: presentation.enabled,
            mode: presentation.mode,
            basemap_provider: (settings.id == connections::BASEMAP)
                .then_some(settings.basemap_provider.clone()),
            endpoint_is_private: settings.allow_private_network
                || presentation
                    .endpoint
                    .as_deref()
                    .map(endpoint_is_private)
                    .unwrap_or(false),
            endpoint: presentation.endpoint,
            weather_precision: settings.weather_precision,
            forecast_url: settings.forecast_url,
            archive_url: settings.archive_url,
            base_url: settings.base_url,
            light_url_template: settings.light_url_template,
            dark_url_template: settings.dark_url_template,
            attribution: presentation.attribution,
            attribution_url: presentation.attribution_url,
            request_identifier: settings.request_identifier,
            custom_autocomplete: settings.custom_autocomplete,
            allow_private_network: settings.allow_private_network,
            private_network_allowlist: settings.private_network_allowlist,
            private_network_policy_state: settings.private_network_policy_state,
            has_api_key: settings.api_key_encrypted.is_some(),
            has_bearer_token: settings.bearer_token_encrypted.is_some(),
            updated_at: settings.updated_at,
            last_attempt_at: activity.last_attempt_at,
            last_success_at: activity.last_success_at,
            last_error: activity.last_error,
            request_count_today: if activity.usage_date == Utc::now().date_naive() {
                activity.request_count
            } else {
                0
            },
            last_test_at: activity.last_test_at,
            last_test_ok: activity.last_test_ok,
            last_test_error: activity.last_test_error,
            credential_issued_at: (settings.id == connections::RIVIAN_ACCOUNT)
                .then_some(rivian_status.credential_issued_at)
                .flatten(),
            expected_renewal_at: (settings.id == connections::RIVIAN_ACCOUNT)
                .then_some(rivian_status.expected_renewal_at)
                .flatten(),
            renewal_state: (settings.id == connections::RIVIAN_ACCOUNT)
                .then_some(rivian_status.renewal_state.clone())
                .flatten(),
            observed_health: (settings.id == connections::RIVIAN_ACCOUNT)
                .then_some(rivian_status.observed_health.clone())
                .flatten(),
            observed_error: (settings.id == connections::RIVIAN_ACCOUNT)
                .then_some(rivian_status.observed_error.clone())
                .flatten(),
            cache,
        });
    }
    Ok(ExternalConnectionsResponse {
        can_manage,
        connections: result,
    })
}

async fn cache_summary(state: &AppState, id: &str) -> Option<ConnectionCacheResponse> {
    let (pattern, persistent, description) = match id {
        connections::BASEMAP => ("external:basemap:*", false, "Map tiles expire after seven days and share a bounded least-recently-used cache."),
        connections::NOMINATIM => ("external:nominatim:search:*", true, "Persistent address search results and reverse-geocoded addresses. Purging keeps addresses used by trips, charging sessions, or saved places."),
        _ => return None,
    };
    let (mut entries, mut bytes) = redis_cache_metrics(state, pattern).await;
    if persistent {
        let (rows, storage) = sqlx::query_as::<_, (i64, i64)>(
            "SELECT COUNT(*), pg_total_relation_size('panda.addresses')",
        )
        .fetch_one(&state.pool)
        .await
        .unwrap_or((0, 0));
        entries = entries.saturating_add(rows.max(0) as u64);
        bytes = bytes.saturating_add(storage.max(0) as u64);
    }
    Some(ConnectionCacheResponse {
        entries,
        bytes,
        persistent,
        purgeable: true,
        description,
    })
}

async fn redis_cache_metrics(state: &AppState, pattern: &str) -> (u64, u64) {
    let totals = cache::traverse(state, pattern, cache::Operation::Measure).await;
    (totals.entries, totals.bytes)
}

#[derive(Debug, Serialize)]
struct PurgeConnectionCacheResponse {
    purged_entries: u64,
    message: &'static str,
}

async fn purge_connection_cache(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<PurgeConnectionCacheResponse>, AppError> {
    require_admin_or_super_user(&state.pool, auth.user_id).await?;
    let (pattern, persistent) = match id.as_str() {
        connections::BASEMAP => ("external:basemap:*", false),
        connections::NOMINATIM => ("external:nominatim:search:*", true),
        _ => {
            return Err(AppError::Validation(
                "this connection does not have a purgeable persistent cache".into(),
            ))
        }
    };
    let mut purged_entries = purge_redis_cache(&state, pattern).await;
    if persistent {
        state.nominatim_cache.write().await.clear();
        let removed = sqlx::query_scalar::<_, i64>(r#"DELETE FROM panda.addresses a
                   WHERE NOT EXISTS (SELECT 1 FROM panda.geofences g WHERE g.address_id = a.id)
                     AND NOT EXISTS (SELECT 1 FROM panda.trips t WHERE t.start_address_id = a.id OR t.end_address_id = a.id)
                     AND NOT EXISTS (SELECT 1 FROM panda.trip_user_annotations tua WHERE tua.start_address_id = a.id OR tua.end_address_id = a.id)
                     AND NOT EXISTS (SELECT 1 FROM panda.charge_sessions cs WHERE cs.address_id = a.id)
                     AND NOT EXISTS (SELECT 1 FROM panda.charge_session_user_annotations csua WHERE csua.address_id = a.id)
                   RETURNING 1"#).fetch_all(&state.pool).await?.len() as u64;
        purged_entries = purged_entries.saturating_add(removed);
    }
    Ok(Json(PurgeConnectionCacheResponse {
        purged_entries,
        message: "Persistent cache purged. Existing trip labels and saved places were retained.",
    }))
}

async fn purge_redis_cache(state: &AppState, pattern: &str) -> u64 {
    cache::traverse(state, pattern, cache::Operation::Purge)
        .await
        .entries
}

#[derive(Debug, Default, Deserialize)]
struct UpdateConnectionBody {
    enabled: bool,
    mode: String,
    basemap_provider: Option<String>,
    weather_precision: Option<String>,
    forecast_url: Option<String>,
    archive_url: Option<String>,
    base_url: Option<String>,
    light_url_template: Option<String>,
    dark_url_template: Option<String>,
    attribution: Option<String>,
    attribution_url: Option<String>,
    request_identifier: Option<String>,
    custom_autocomplete: Option<bool>,
    allow_private_network: Option<bool>,
    private_network_allowlist: Option<Vec<String>>,
    api_key: Option<String>,
    clear_api_key: Option<bool>,
    bearer_token: Option<String>,
    clear_bearer_token: Option<bool>,
}

async fn update_connection(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(mut body): Json<UpdateConnectionBody>,
) -> Result<Json<ExternalConnectionsResponse>, AppError> {
    require_admin_or_super_user(&state.pool, auth.user_id).await?;
    let definition = DEFINITIONS
        .iter()
        .find(|item| item.id == id)
        .ok_or(AppError::NotFound)?;
    if !definition.editable {
        return Err(AppError::Forbidden);
    }
    if body.enabled {
        connections::require_connection_allowed(&id)?;
    }
    if id == connections::OPEN_METEO {
        if matches!(body.mode.as_str(), "remote" | "hosted") {
            body.forecast_url = Some(crate::private_deployment::weather::FORECAST_URL.into());
            body.archive_url = Some(crate::private_deployment::weather::ARCHIVE_URL.into());
            body.allow_private_network = Some(false);
            body.private_network_allowlist = Some(Vec::new());
        }
        validate_update(&id, &body).await?;
        body.weather_precision = Some("approximate".into());
        body.allow_private_network = Some(false);
        body.private_network_allowlist = Some(Vec::new());
        body.clear_api_key = Some(true);
        body.clear_bearer_token = Some(true);
    }
    let private_network_allowlist = validate_update(&id, &body).await?;
    let existing_basemap = if id == connections::BASEMAP {
        Some(connections::load(&state.pool, connections::BASEMAP).await?)
    } else {
        None
    };
    let basemap_provider = resolve_basemap_provider_update(&id, &body, existing_basemap.as_ref())?;

    let api_key_encrypted = encrypt_secret(&state.age_key, body.api_key.as_deref())?;
    let bearer_token_encrypted = encrypt_secret(&state.age_key, body.bearer_token.as_deref())?;
    let plan = settings::UpdatePlan::resolve(&id, &body);

    sqlx::query(
        r#"UPDATE panda.external_connection_settings SET
             enabled = $2, mode = $3, weather_precision = COALESCE($4, weather_precision),
             forecast_url = COALESCE($5, forecast_url), archive_url = COALESCE($6, archive_url),
             base_url = COALESCE($7, base_url), light_url_template = COALESCE($8, light_url_template),
             dark_url_template = $9, attribution = COALESCE($10, attribution),
             attribution_url = $11, request_identifier = $12,
             custom_autocomplete = COALESCE($13, custom_autocomplete),
             allow_private_network = CASE WHEN $15 THEN cardinality(ARRAY(SELECT unnest($16::text[])::cidr)) > 0 ELSE COALESCE($14, allow_private_network) END,
             private_network_allowlist = CASE WHEN $15 THEN ARRAY(SELECT unnest($16::text[])::cidr) ELSE private_network_allowlist END,
             private_network_policy_state = CASE WHEN $15 THEN $17 WHEN $14 THEN 'migration_required' ELSE private_network_policy_state END,
             basemap_provider = COALESCE($18, basemap_provider),
             api_key_encrypted = CASE WHEN $19 THEN NULL WHEN $20 IS NOT NULL THEN $20 ELSE api_key_encrypted END,
             bearer_token_encrypted = CASE WHEN $21 THEN NULL WHEN $22 IS NOT NULL THEN $22 ELSE bearer_token_encrypted END,
             updated_at = now(), updated_by = $23
           WHERE id = $1"#,
    )
    .bind(&id)
    .bind(body.enabled)
    .bind(plan.mode)
    .bind(body.weather_precision.as_deref())
    .bind(plan.forecast_url)
    .bind(plan.archive_url)
    .bind(plan.base_url)
    .bind(plan.light_url_template)
    .bind(plan.dark_url_template)
    .bind(plan.attribution)
    .bind(plan.attribution_url)
    .bind(plan.request_identifier)
    .bind(body.custom_autocomplete)
    .bind(body.allow_private_network)
    .bind(private_network_allowlist.is_some())
    .bind(private_network_allowlist.clone())
    .bind(
        if private_network_allowlist
            .as_ref()
            .is_some_and(|allowlist| !allowlist.is_empty())
        {
            "configured"
        } else {
            "restricted"
        },
    )
    .bind(basemap_provider)
    .bind(body.clear_api_key.unwrap_or(false))
    .bind(api_key_encrypted)
    .bind(body.clear_bearer_token.unwrap_or(false))
    .bind(bearer_token_encrypted)
    .bind(auth.user_id)
    .execute(&state.pool)
    .await?;

    // Endpoint/style changes must never reuse a response from the prior
    // provider. Address labels remain durable; only lookup-result cache keys
    // are invalidated here.
    match id.as_str() {
        connections::BASEMAP => {
            purge_redis_cache(&state, "external:basemap:*").await;
        }
        connections::NOMINATIM => {
            purge_redis_cache(&state, "external:nominatim:search:*").await;
            state.nominatim_cache.write().await.clear();
        }
        _ => {}
    }

    Ok(Json(build_response(&state, true).await?))
}

async fn disable_optional(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<ExternalConnectionsResponse>, AppError> {
    require_admin_or_super_user(&state.pool, auth.user_id).await?;
    sqlx::query(
        "UPDATE panda.external_connection_settings SET enabled = FALSE, mode = 'disabled', updated_at = now(), updated_by = $1 WHERE id = ANY($2)",
    )
    .bind(auth.user_id)
    .bind(connections::OPTIONAL_CONNECTIONS)
    .execute(&state.pool)
    .await?;
    Ok(Json(build_response(&state, true).await?))
}

#[derive(Debug, Serialize)]
struct TestConnectionResponse {
    ok: bool,
    tested_at: DateTime<Utc>,
    checks: Vec<TestConnectionCheck>,
    preview_data_url: Option<String>,
}

#[derive(Debug, Serialize)]
struct TestConnectionCheck {
    label: String,
    ok: bool,
    message: String,
}

#[derive(Debug, Serialize)]
struct BasemapConfigResponse {
    enabled: bool,
    provider_preference: String,
    resolved_provider: String,
    /// A non-secret revision of the persisted basemap setting. This gives the
    /// browser and MapLibre a new tile identity after any basemap save.
    revision: String,
    styles: Vec<BasemapStyleDescriptor>,
    attributions: Vec<BasemapAttributionLink>,
}

#[derive(Debug, Serialize)]
struct BasemapStyleDescriptor {
    id: &'static str,
    label: &'static str,
    kind: &'static str,
    light_url: String,
    dark_url: String,
    perspective_3d: bool,
}

#[derive(Debug, Serialize)]
struct BasemapAttributionLink {
    label: String,
    url: Option<String>,
}

async fn basemap_config(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> Result<Json<BasemapConfigResponse>, AppError> {
    let settings = connections::load(&state.pool, connections::BASEMAP).await?;
    Ok(Json(BasemapConfigResponse::from_settings(settings)))
}

fn basemap_proxy_url(style: &str, revision: &str) -> String {
    format!("/v1/external/basemap/raster/{style}/{{z}}/{{x}}/{{y}}.png?v={revision}")
}

fn openfreemap_style_proxy_url(style: &str, revision: &str) -> String {
    format!(
        "/v1/external/basemap/openfreemap/styles/{style}?v={revision}&cf={OPENFREEMAP_CACHE_FORMAT}"
    )
}

fn resolve_basemap_provider(settings: &ConnectionSettingsRow) -> &'static str {
    preferences::stored(settings)
}

fn basemap_styles(provider: &str, revision: &str) -> Vec<BasemapStyleDescriptor> {
    let specs: &[(&str, &str, &str, &str, bool)] = match provider {
        "carto" | "custom" => &[("follow-theme", "Follow appearance", "light", "dark", false)],
        "openfreemap" => &[
            (
                "follow-theme",
                "Follow appearance",
                "positron",
                "dark",
                false,
            ),
            ("positron", "Positron", "positron", "positron", false),
            ("bright", "Bright", "bright", "bright", false),
            ("liberty", "Liberty", "liberty", "liberty", false),
            ("dark", "Dark", "dark", "dark", false),
            ("fiord", "Fiord", "fiord", "fiord", false),
            ("3d", "3D", "liberty", "liberty", true),
        ],
        _ => &[],
    };
    let vector = provider == "openfreemap";
    let url = if vector {
        openfreemap_style_proxy_url
    } else {
        basemap_proxy_url
    };
    specs
        .iter()
        .map(
            |&(id, label, light, dark, perspective_3d)| BasemapStyleDescriptor {
                id,
                label,
                kind: if vector { "style" } else { "raster" },
                light_url: url(light, revision),
                dark_url: url(dark, revision),
                perspective_3d,
            },
        )
        .collect()
}

fn basemap_attributions(
    settings: &ConnectionSettingsRow,
    provider: &str,
) -> Vec<BasemapAttributionLink> {
    let links = match provider {
        "openfreemap" => vec![
            ("OpenFreeMap", Some("https://openfreemap.org/")),
            ("© OpenMapTiles", Some("https://openmaptiles.org/")),
            (
                "Data from OpenStreetMap",
                Some("https://www.openstreetmap.org/copyright"),
            ),
        ],
        "carto" => vec![
            (
                "© OpenStreetMap contributors",
                Some("https://www.openstreetmap.org/copyright"),
            ),
            ("© CARTO", Some("https://carto.com/attributions")),
        ],
        "custom" => settings
            .attribution
            .as_deref()
            .into_iter()
            .map(|label| (label, settings.attribution_url.as_deref()))
            .collect(),
        _ => Vec::new(),
    };
    links
        .into_iter()
        .map(|(label, url)| BasemapAttributionLink {
            label: label.to_owned(),
            url: url.map(str::to_owned),
        })
        .collect()
}

async fn test_connection(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<UpdateConnectionBody>,
) -> Result<Json<TestConnectionResponse>, AppError> {
    require_admin_or_super_user(&state.pool, auth.user_id).await?;
    connections::require_connection_allowed(&id)?;
    if !body.enabled || body.mode == "disabled" {
        return Err(AppError::ExternalConnectionDisabled(id));
    }
    let private_network_allowlist = validate_update(&id, &body)
        .await?
        .map(|allowlist| parse_private_network_allowlist(&allowlist))
        .transpose()?
        .unwrap_or_default();
    if id == connections::OPEN_METEO {
        let now = Utc::now();
        let result =
            crate::private_deployment::weather::fetch(&state.pool, 39.0, -98.0, now, now).await;
        let ok = result.is_ok();
        let message = if ok {
            "Connection succeeded with synthetic data and the shared weather request budget."
        } else {
            "Weather test unavailable. Save the enabled connection and check its daily request budget."
        };
        connections::record_test(&state.pool, &id, ok, (!ok).then_some(message)).await;
        return Ok(Json(TestConnectionResponse {
            ok,
            tested_at: Utc::now(),
            checks: vec![TestConnectionCheck {
                label: "Synthetic weather request".into(),
                ok,
                message: message.into(),
            }],
            preview_data_url: None,
        }));
    }
    let settings = connections::load(&state.pool, &id).await?;
    connections::record_attempt(&state.pool, &id).await;
    let Some(probe) = ConnectionProbe::compile(&id, &body, &settings)? else {
        connections::record_test(&state.pool, &id, true, None).await;
        return Ok(Json(TestConnectionResponse {
            ok: true,
            tested_at: Utc::now(),
            checks: vec![TestConnectionCheck {
                label: "Configuration".into(),
                ok: true,
                message: "This connection is managed by its owning settings surface.".into(),
            }],
            preview_data_url: None,
        }));
    };
    let request = probe
        .request(&state, &body, &settings, &private_network_allowlist)
        .await?;
    let (ok, message, stored_error, preview_data_url) = match request.send().await {
        Err(error) => (
            false,
            "Connection failed; no trip data was sent.".to_owned(),
            Some(error.to_string()),
            None,
        ),
        Ok(response) if !response.status().is_success() => {
            let message = format!("Provider returned HTTP {}", response.status());
            (false, message.clone(), Some(message), None)
        }
        Ok(response) => {
            let image = id == connections::BASEMAP
                && response
                    .headers()
                    .get(header::CONTENT_TYPE)
                    .and_then(|header| header.to_str().ok())
                    .is_some_and(|mime| mime.starts_with("image/"));
            let preview = if image {
                let mime = response
                    .headers()
                    .get(header::CONTENT_TYPE)
                    .and_then(|header| header.to_str().ok())
                    .unwrap_or("image/png")
                    .to_owned();
                let bytes =
                    read_response_limited(response, 5 * 1024 * 1024, "Basemap preview").await?;
                Some(format!(
                    "data:{mime};base64,{}",
                    base64::engine::general_purpose::STANDARD.encode(bytes)
                ))
            } else {
                None
            };
            (
                true,
                "Connection succeeded with synthetic test data.".to_owned(),
                None,
                preview,
            )
        }
    };
    connections::record_test(&state.pool, &id, ok, stored_error.as_deref()).await;
    Ok(Json(TestConnectionResponse {
        ok,
        tested_at: Utc::now(),
        checks: vec![TestConnectionCheck {
            label: "Synthetic request".into(),
            ok,
            message,
        }],
        preview_data_url,
    }))
}

async fn proxy_basemap_tile(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path((style, z, x, y)): Path<(String, u8, u32, String)>,
) -> Result<Response<Body>, AppError> {
    let settings = connections::require_enabled(&state.pool, connections::BASEMAP).await?;
    let tile = RasterRequest::compile(&settings, style, z, x, &y)?;
    if let Some(cached) = CachedBasemap::read(&state, &tile.cache_key, "image/png").await {
        return cached.respond();
    }
    let endpoint_text = expand_tile_template(tile.template, z, x, tile.y);
    connections::record_attempt(&state.pool, connections::BASEMAP).await;
    let endpoint = Url::parse(&endpoint_text)
        .map_err(|_| AppError::Validation("invalid tile endpoint".into()))?;
    let allowlist = configured_private_network_allowlist(&settings)?;
    let credentials = (
        tile.provider == "carto" && should_forward_carto_api_key(&settings.mode, &endpoint),
        should_forward_basemap_bearer_token(tile.provider),
    );
    let mut request = outbound_client_for_url(&endpoint, &allowlist)
        .await?
        .get(endpoint);
    if credentials.0 {
        if let Some(key) = decrypt_secret(&state.age_key, settings.api_key_encrypted.as_deref())? {
            request = request.query(&[carto_basemap_key_query(&key)]);
        }
    }
    if credentials.1 {
        if let Some(token) =
            decrypt_secret(&state.age_key, settings.bearer_token_encrypted.as_deref())?
        {
            request = request.bearer_auth(token);
        }
    }
    let upstream = match request.send().await {
        Err(_) => {
            connections::record_failure(
                &state.pool,
                connections::BASEMAP,
                "Basemap request failed",
            )
            .await;
            return Err(AppError::DependencyUnavailable(
                "Basemap request failed".into(),
            ));
        }
        Ok(response) if response.status().is_success() => response,
        Ok(response) => {
            connections::record_failure(
                &state.pool,
                connections::BASEMAP,
                &format!("HTTP {}", response.status()),
            )
            .await;
            return Err(AppError::DependencyUnavailable(
                "Basemap provider returned an error".into(),
            ));
        }
    };
    let cache_key = tile.cache_key;
    let tile = CachedBasemap::from_raster(upstream).await?;
    tile.store(&state, &cache_key).await;
    connections::record_success(&state.pool, connections::BASEMAP).await;
    tile.respond()
}

fn should_forward_carto_api_key(_mode: &str, endpoint: &Url) -> bool {
    endpoint.host_str().is_some_and(|host| {
        host == "carto.com" || host.ends_with(".carto.com") || host.ends_with(".cartocdn.com")
    })
}

fn should_forward_basemap_bearer_token(provider: &str) -> bool {
    provider == "custom"
}

/// CARTO Basemaps authenticate with `key`; this is deliberately distinct from
/// Open-Meteo's `apikey` query parameter and CARTO API access tokens.
fn carto_basemap_key_query(key: &str) -> (&'static str, &str) {
    ("key", key)
}

async fn proxy_openfreemap_resource(
    State(state): State<AppState>,
    _auth: AuthUser,
    headers: HeaderMap,
    Path(resource): Path<String>,
) -> Result<Response<Body>, AppError> {
    let request_id = headers
        .get("x-request-id")
        .and_then(|header| header.to_str().ok());
    let resource_class = openfreemap_resource_class(&resource);
    let started_at = std::time::Instant::now();
    if !openfreemap_resource_is_allowed(&resource) {
        log_openfreemap_failure(
            request_id,
            resource_class,
            None,
            "invalid_resource",
            started_at,
        );
        return Err(AppError::Validation("invalid OpenFreeMap resource".into()));
    }
    let settings = connections::require_enabled(&state.pool, connections::BASEMAP).await?;
    if resolve_basemap_provider(&settings) != "openfreemap" {
        return Err(AppError::ExternalConnectionDisabled(
            connections::BASEMAP.into(),
        ));
    }
    let cache_key = openfreemap_cache_key(settings.updated_at.timestamp_millis(), &resource);
    if let Some(cached) = CachedBasemap::read(&state, &cache_key, "application/octet-stream").await
    {
        return cached.respond();
    }
    match fetch_openfreemap(&state, &resource).await {
        Ok(reply) => {
            reply.store(&state, &cache_key).await;
            connections::record_success(&state.pool, connections::BASEMAP).await;
            reply.respond()
        }
        Err(failure) => {
            let (error, class, status) = failure.parts();
            if let Some(status) = status {
                connections::record_failure(
                    &state.pool,
                    connections::BASEMAP,
                    &format!("HTTP {status}"),
                )
                .await;
                log_openfreemap_failure(
                    request_id,
                    resource_class,
                    Some(status.as_u16()),
                    class,
                    started_at,
                );
            } else {
                record_openfreemap_failure(
                    &state,
                    request_id,
                    resource_class,
                    None,
                    class,
                    started_at,
                )
                .await;
            }
            Err(error)
        }
    }
}

async fn record_openfreemap_failure(
    state: &AppState,
    request_id: Option<&str>,
    resource_class: &'static str,
    upstream_status: Option<u16>,
    error_class: &'static str,
    started_at: std::time::Instant,
) {
    connections::record_failure(
        &state.pool,
        connections::BASEMAP,
        &format!("OpenFreeMap {error_class}"),
    )
    .await;
    log_openfreemap_failure(
        request_id,
        resource_class,
        upstream_status,
        error_class,
        started_at,
    );
}

fn log_openfreemap_failure(
    request_id: Option<&str>,
    resource_class: &'static str,
    upstream_status: Option<u16>,
    error_class: &'static str,
    started_at: std::time::Instant,
) {
    tracing::warn!(
        event = "openfreemap.proxy_failed",
        provider = "openfreemap",
        resource_kind = resource_class,
        style_or_resource_class = resource_class,
        upstream_status = ?upstream_status,
        error_class,
        duration_ms = started_at.elapsed().as_millis() as u64,
        cache_state = "miss",
        request_id = ?request_id,
        "OpenFreeMap provider request failed"
    );
}

fn openfreemap_resource_class(resource: &str) -> &'static str {
    match resource.split('/').next() {
        Some("styles") => "style",
        Some("planet") | Some("natural_earth") => "tile",
        Some("sprites") => "sprite",
        Some("fonts") => "font",
        _ => "unknown",
    }
}

fn openfreemap_cache_key(revision: i64, resource: &str) -> String {
    format!("external:basemap:openfreemap:{OPENFREEMAP_CACHE_FORMAT}:{revision}:{resource}")
}

async fn read_openfreemap_response_limited(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, &'static str> {
    if response
        .content_length()
        .is_some_and(|bytes| bytes > limit as u64)
    {
        return Err("response_size");
    }
    let mut remaining = limit;
    let mut output = Vec::new();
    loop {
        let chunk = response.chunk().await.map_err(|_| "response_read")?;
        match chunk {
            None => return Ok(output),
            Some(chunk) => {
                remaining = remaining.checked_sub(chunk.len()).ok_or("response_size")?;
                output.extend_from_slice(&chunk);
            }
        }
    }
}

fn openfreemap_resource_is_allowed(resource: &str) -> bool {
    let invalid_character = resource
        .chars()
        .any(|ch| ch.is_control() || matches!(ch, '\\' | '?' | '#'));
    if resource.is_empty() || resource.len() > 1024 || invalid_character || resource.contains("..")
    {
        return false;
    }
    let parts: Vec<_> = resource.split('/').collect();
    if parts.contains(&"") {
        return false;
    }
    let Some((family, rest)) = parts.split_first() else {
        return false;
    };
    match (*family, rest) {
        ("styles", [style]) => openfreemap_style_name(style),
        ("planet", []) => true,
        ("planet", [snapshot, z, x, y]) => {
            openfreemap_planet_snapshot(snapshot)
                && [z, x].into_iter().all(|value| is_ascii_decimal(value))
                && numeric_resource_with_suffix(y, ".pbf")
        }
        ("natural_earth", ["ne2sr", z, x, y]) => {
            [z, x].into_iter().all(|value| is_ascii_decimal(value))
                && numeric_resource_with_suffix(y, ".png")
        }
        ("sprites", [version, asset]) => {
            openfreemap_sprite_version(version) && openfreemap_sprite_asset(asset)
        }
        ("fonts", [font, range]) if font.len() <= 512 => {
            range.strip_suffix(".pbf").is_some_and(|stem| {
                !stem.is_empty()
                    && stem
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || byte == b'-')
            })
        }
        _ => false,
    }
}

fn openfreemap_style_name(style: &str) -> bool {
    matches!(style, "positron" | "bright" | "liberty" | "dark" | "fiord")
}

fn openfreemap_planet_snapshot(snapshot: &str) -> bool {
    snapshot.strip_suffix("_pt").is_some_and(|value| {
        !value.is_empty()
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || byte == b'_')
    })
}

fn openfreemap_sprite_version(version: &str) -> bool {
    version.strip_prefix("ofm_").is_some_and(|value| {
        !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
    })
}

fn openfreemap_sprite_asset(asset: &str) -> bool {
    matches!(asset, "ofm.json" | "ofm.png" | "ofm@2x.png")
}

fn is_ascii_decimal(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn numeric_resource_with_suffix(value: &str, suffix: &str) -> bool {
    value.strip_suffix(suffix).is_some_and(is_ascii_decimal)
}

fn is_json_content_type(content_type: &str) -> bool {
    content_type.split(';').next().is_some_and(|value| {
        let value = value.trim().to_ascii_lowercase();
        value == "application/json" || value.ends_with("+json")
    })
}

fn rewrite_openfreemap_json(bytes: &[u8]) -> Result<Vec<u8>, AppError> {
    let mut value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|_| AppError::DependencyUnavailable("OpenFreeMap returned invalid JSON".into()))?;
    rewrite_openfreemap_value(&mut value);
    serde_json::to_vec(&value).map_err(|error| AppError::Internal(error.into()))
}

fn rewrite_openfreemap_value(value: &mut serde_json::Value) {
    let mut pending = vec![value];
    while let Some(node) = pending.pop() {
        match node {
            serde_json::Value::String(text) => {
                let resource = [
                    "https://tiles.openfreemap.org/",
                    "https://__TILEJSON_DOMAIN__/",
                ]
                .into_iter()
                .find_map(|prefix| text.strip_prefix(prefix));
                if let Some(resource) = resource {
                    *text = format!("{OPENFREEMAP_PROXY_ORIGIN}/v1/external/basemap/openfreemap/{resource}?cf={OPENFREEMAP_CACHE_FORMAT}");
                }
            }
            serde_json::Value::Array(items) => pending.extend(items.iter_mut()),
            serde_json::Value::Object(properties) => pending.extend(properties.values_mut()),
            _ => {}
        }
    }
}

#[derive(Debug, Deserialize)]
struct IconifySearchParams {
    query: String,
    limit: Option<u8>,
    prefix: Option<String>,
}

async fn proxy_iconify_search(
    State(state): State<AppState>,
    _auth: AuthUser,
    Query(params): Query<IconifySearchParams>,
) -> Result<Response<Body>, AppError> {
    let settings = connections::require_enabled(&state.pool, connections::ICONIFY).await?;
    let mut endpoint = endpoint_join(settings.base_url.as_deref(), "search")?;
    let limit = params.limit.unwrap_or(40).clamp(1, 40).to_string();
    let arguments = params
        .prefix
        .as_deref()
        .filter(|prefix| !prefix.is_empty())
        .map(|prefix| ("prefix", prefix))
        .into_iter()
        .chain([("query", params.query.as_str()), ("limit", limit.as_str())]);
    endpoint.query_pairs_mut().extend_pairs(arguments);
    let allowlist = configured_private_network_allowlist(&settings)?;
    proxy_json(&state, connections::ICONIFY, endpoint, &allowlist).await
}

async fn proxy_iconify_resource(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(resource): Path<String>,
    OriginalUri(uri): OriginalUri,
) -> Result<Response<Body>, AppError> {
    let valid =
        resource.ends_with(".json") && !["/", ".."].into_iter().any(|part| resource.contains(part));
    if !valid {
        return Err(AppError::Validation("invalid icon resource".into()));
    }
    let settings = connections::require_enabled(&state.pool, connections::ICONIFY).await?;
    let endpoint = match uri.query() {
        None => endpoint_join(settings.base_url.as_deref(), &resource)?,
        Some(query) => {
            let mut endpoint = endpoint_join(settings.base_url.as_deref(), &resource)?;
            endpoint.set_query(Some(query));
            endpoint
        }
    };
    proxy_json(
        &state,
        connections::ICONIFY,
        endpoint,
        &configured_private_network_allowlist(&settings)?,
    )
    .await
}

async fn proxy_json(
    state: &AppState,
    id: &str,
    endpoint: Url,
    allowlist: &[IpNet],
) -> Result<Response<Body>, AppError> {
    connections::record_attempt(&state.pool, id).await;
    let client = outbound_client_for_url(&endpoint, allowlist).await?;
    let response = client.get(endpoint).send().await;
    let response = match response {
        Err(_) => {
            return Err(AppError::DependencyUnavailable(format!(
                "{id} request failed"
            )))
        }
        Ok(response) if response.status().is_success() => response,
        Ok(response) => {
            connections::record_failure(&state.pool, id, &format!("HTTP {}", response.status()))
                .await;
            return Err(AppError::DependencyUnavailable(format!(
                "{id} provider returned an error"
            )));
        }
    };
    let body = read_response_limited(response, 10 * 1024 * 1024, id).await?;
    connections::record_success(&state.pool, id).await;
    tile_response(body, "application/json", 3600)
}

fn tile_response(
    bytes: Vec<u8>,
    content_type: &str,
    max_age: u64,
) -> Result<Response<Body>, AppError> {
    let mime = HeaderValue::from_str(content_type)
        .unwrap_or_else(|_| HeaderValue::from_static("image/png"));
    let mut response = Response::new(Body::from(bytes));
    response.headers_mut().extend([
        (header::CONTENT_TYPE, mime),
        (
            header::CACHE_CONTROL,
            HeaderValue::from_str(&format!("private, max-age={max_age}"))
                .map_err(|error| AppError::Internal(error.into()))?,
        ),
    ]);
    Ok(response)
}

fn upstream_cache_ttl(headers: &axum::http::HeaderMap) -> u64 {
    let Some(value) = headers
        .get(header::CACHE_CONTROL)
        .and_then(|header| header.to_str().ok())
    else {
        return 86_400;
    };
    let mut selected = None;
    for directive in value.split(',').map(str::trim) {
        if directive.eq_ignore_ascii_case("no-store") {
            return 0;
        }
        if selected.is_none() {
            selected = directive
                .strip_prefix("max-age=")
                .and_then(|number| number.parse::<u64>().ok());
        }
    }
    selected.unwrap_or(86_400).min(86_400)
}

async fn validate_update(
    id: &str,
    body: &UpdateConnectionBody,
) -> Result<Option<Vec<String>>, AppError> {
    if id == connections::OPEN_METEO
        && body.enabled
        && !crate::private_deployment::weather::update_allowed(
            &body.mode,
            body.weather_precision.as_deref(),
            body.forecast_url.as_deref(),
            body.archive_url.as_deref(),
            body.api_key.as_ref().is_some_and(|value| !value.is_empty())
                || body
                    .bearer_token
                    .as_ref()
                    .is_some_and(|value| !value.is_empty()),
            body.allow_private_network == Some(true)
                || body
                    .private_network_allowlist
                    .as_ref()
                    .is_some_and(|value| !value.is_empty()),
        )
    {
        return Err(AppError::Validation(
            "This deployment allows only free Open-Meteo endpoints, approximate coordinates, and no provider credentials or private networks.".into(),
        ));
    }
    if !matches!(
        body.mode.as_str(),
        "remote" | "hosted" | "custom" | "disabled"
    ) {
        return Err(AppError::Validation(
            "mode must be remote, custom, or disabled".into(),
        ));
    }
    if body.mode == "custom"
        && !matches!(
            id,
            connections::OPEN_METEO | connections::NOMINATIM | connections::BASEMAP
        )
    {
        return Err(AppError::Validation(
            "this connection does not support a custom endpoint".into(),
        ));
    }
    if let Some(precision) = body.weather_precision.as_deref() {
        if !matches!(precision, "approximate" | "exact") {
            return Err(AppError::Validation(
                "weather_precision must be approximate or exact".into(),
            ));
        }
    }
    let parsed = body
        .private_network_allowlist
        .as_ref()
        .map(|allowlist| parse_private_network_allowlist(allowlist))
        .transpose()?;
    if body.enabled && body.mode == "custom" {
        let allowlist = parsed.as_deref().unwrap_or_default();
        let checks: Vec<(Option<&str>, bool)> = match id {
            connections::OPEN_METEO => vec![
                (body.forecast_url.as_deref(), false),
                (body.archive_url.as_deref(), false),
            ],
            connections::NOMINATIM => vec![(body.base_url.as_deref(), false)],
            connections::BASEMAP => {
                let light = body.light_url_template.as_deref().ok_or_else(|| {
                    AppError::Validation("custom basemap requires a light URL template".into())
                })?;
                std::iter::once((Some(light), true))
                    .chain(
                        body.dark_url_template
                            .as_deref()
                            .map(|url| (Some(url), true)),
                    )
                    .collect()
            }
            _ => Vec::new(),
        };
        for (endpoint, tile) in checks {
            if tile {
                validate_tile_template(endpoint.expect("tile endpoint exists"), allowlist).await?;
            } else {
                validate_endpoint(endpoint, allowlist).await?;
            }
        }
        if id == connections::BASEMAP
            && body
                .attribution
                .as_deref()
                .is_none_or(|text| text.trim().is_empty())
        {
            return Err(AppError::Validation(
                "custom basemap attribution is required".into(),
            ));
        }
    }
    Ok(parsed.map(canonical_allowlist))
}

fn resolve_basemap_provider_update(
    id: &str,
    body: &UpdateConnectionBody,
    existing: Option<&ConnectionSettingsRow>,
) -> Result<Option<String>, AppError> {
    preferences::update(id, body, existing)
}

fn resolve_effective_basemap_provider_update(
    id: &str,
    body: &UpdateConnectionBody,
    existing: &ConnectionSettingsRow,
) -> Result<String, AppError> {
    preferences::effective(id, body, existing)
}

async fn validate_tile_template(value: &str, allowlist: &[IpNet]) -> Result<(), AppError> {
    let required = ["{z}", "{x}", "{y}"];
    if let Some(token) = required.into_iter().find(|token| !value.contains(token)) {
        return Err(AppError::Validation(format!(
            "tile template must contain {token}"
        )));
    }
    let endpoint = expand_tile_template(value, 0, 0, 0);
    validate_endpoint(Some(&endpoint), allowlist).await
}

async fn validate_endpoint(value: Option<&str>, allowlist: &[IpNet]) -> Result<(), AppError> {
    let value = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AppError::Validation("endpoint is required".into()))?;
    let endpoint = Url::parse(value)
        .map_err(|_| AppError::Validation("endpoint must be a valid URL".into()))?;
    let restrictions = [
        (!["http", "https"].contains(&endpoint.scheme()), "endpoint must use HTTP or HTTPS"),
        (!endpoint.username().is_empty() || endpoint.password().is_some() || endpoint.query().is_some() || endpoint.fragment().is_some(),
            "endpoint credentials, query strings, and fragments are not allowed; use the encrypted secret field"),
    ];
    for (rejected, message) in restrictions {
        if rejected {
            return Err(AppError::Validation(message.into()));
        }
    }
    let host = endpoint
        .host_str()
        .ok_or_else(|| AppError::Validation("endpoint host is required".into()))?;
    if is_link_local_or_metadata(host) {
        return Err(AppError::Validation(
            "link-local and cloud metadata endpoints are not allowed".into(),
        ));
    }
    let port = endpoint
        .port_or_known_default()
        .ok_or_else(|| AppError::Validation("endpoint port is required".into()))?;
    let addresses = tokio::net::lookup_host((host, port))
        .await
        .map_err(|_| AppError::Validation("endpoint host could not be resolved".into()))?;
    let (private, public) = addresses.into_iter().try_fold(
        (endpoint_is_private(value), false),
        |(private, public), address| {
            let ip = address.ip();
            if is_forbidden_ip(ip) {
                return Err(AppError::Validation(
                    "link-local and cloud metadata endpoints are not allowed".into(),
                ));
            }
            if !is_private_ip(ip) {
                return Ok((private, true));
            }
            if !allowlist.iter().any(|network| network.contains(&ip)) {
                return Err(AppError::Validation(
                    "endpoint resolved to a private address outside its configured CIDR allowlist"
                        .into(),
                ));
            }
            Ok((true, public))
        },
    )?;
    for (rejected, message) in [
        (
            private && public,
            "endpoint DNS returned mixed public and private addresses",
        ),
        (
            private && allowlist.is_empty(),
            "private-network endpoints require an explicit CIDR allowlist",
        ),
        (
            endpoint.scheme() == "http" && !private,
            "HTTP is permitted only for a confirmed local/private endpoint",
        ),
    ] {
        if rejected {
            return Err(AppError::Validation(message.into()));
        }
    }
    Ok(())
}

fn canonical_allowlist(allowlist: Vec<IpNet>) -> Vec<String> {
    allowlist
        .into_iter()
        .map(|network| network.to_string())
        .collect()
}

fn active_endpoint(settings: &ConnectionSettingsRow) -> Option<String> {
    match settings.id.as_str() {
        connections::OPEN_METEO => settings.forecast_url.clone(),
        connections::BASEMAP if resolve_basemap_provider(settings) == "openfreemap" => {
            Some("https://tiles.openfreemap.org".into())
        }
        connections::BASEMAP => settings.light_url_template.clone(),
        _ => settings.base_url.clone(),
    }
}

fn endpoint_join(base: Option<&str>, path: &str) -> Result<Url, AppError> {
    let base = base.ok_or_else(|| AppError::Validation("connection endpoint missing".into()))?;
    let mut normalized = base.trim_end_matches('/').to_string();
    normalized.push('/');
    Url::parse(&normalized)
        .and_then(|url| url.join(path))
        .map_err(|_| AppError::Validation("connection endpoint is invalid".into()))
}

fn expand_tile_template(template: &str, z: u8, x: u32, y: u32) -> String {
    template
        .replace("{z}", &z.to_string())
        .replace("{x}", &x.to_string())
        .replace("{y}", &y.to_string())
}
fn normalize(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn encrypt_secret(age_key: &str, secret: Option<&str>) -> Result<Option<Vec<u8>>, AppError> {
    let Some(secret) = secret.map(str::trim).filter(|secret| !secret.is_empty()) else {
        return Ok(None);
    };
    let codec = ConnectionSecret::new(age_key)?;
    codec.encode(secret).map(Some)
}

fn decrypt_secret(age_key: &str, encrypted: Option<&[u8]>) -> Result<Option<String>, AppError> {
    match encrypted {
        None => Ok(None),
        Some(bytes) => ConnectionSecret::new(age_key)?.decode(bytes).map(Some),
    }
}

#[cfg(test)]
#[path = "external_connections/__tests__/contracts.rs"]
mod contracts;

struct ConnectionSecret(age::x25519::Identity);
impl ConnectionSecret {
    fn new(key: &str) -> Result<Self, AppError> {
        key.parse().map(Self).map_err(|_| {
            AppError::Internal(anyhow::anyhow!(
                "invalid age key for external connection secret"
            ))
        })
    }
    fn encode(&self, text: &str) -> Result<Vec<u8>, AppError> {
        encrypt_json(&text.to_owned(), &self.0).map_err(AppError::from)
    }
    fn decode(&self, bytes: &[u8]) -> Result<String, AppError> {
        crate::ingestion::session_store::decrypt_json(bytes, &self.0).map_err(AppError::from)
    }
}

struct CachedBasemap {
    bytes: Vec<u8>,
    content_type: String,
    max_age: u64,
}
impl CachedBasemap {
    async fn read(state: &AppState, key: &str, fallback: &str) -> Option<Self> {
        let mut redis = state.redis.get_multiplexed_async_connection().await.ok()?;
        let bytes = redis.get::<_, Option<Vec<u8>>>(key).await.ok()??;
        crate::services::basemap_cache::touch(&mut redis, key).await;
        let mut metadata = Vec::with_capacity(2);
        for suffix in ["content-type", "max-age"] {
            metadata.push(
                redis
                    .get::<_, Option<String>>(format!("{key}:{suffix}"))
                    .await
                    .ok()
                    .flatten(),
            );
        }
        let max_age = metadata[1]
            .as_deref()
            .and_then(|value| value.parse().ok())
            .unwrap_or(86_400);
        let content_type = metadata.remove(0).unwrap_or_else(|| fallback.to_owned());
        Some(Self {
            bytes,
            content_type,
            max_age,
        })
    }
    fn respond(self) -> Result<Response<Body>, AppError> {
        tile_response(self.bytes, &self.content_type, self.max_age)
    }
}

impl BasemapConfigResponse {
    fn from_settings(settings: ConnectionSettingsRow) -> Self {
        let provider = resolve_basemap_provider(&settings);
        let revision = settings.updated_at.timestamp_millis().to_string();
        let styles = basemap_styles(provider, &revision);
        let attributions = basemap_attributions(&settings, provider);
        Self {
            enabled: settings.is_active(),
            provider_preference: settings.basemap_provider,
            resolved_provider: provider.to_owned(),
            revision,
            styles,
            attributions,
        }
    }
}

struct RasterRequest<'a> {
    provider: &'static str,
    template: &'a str,
    y: u32,
    cache_key: String,
}
impl<'a> RasterRequest<'a> {
    fn compile(
        settings: &'a ConnectionSettingsRow,
        style: String,
        z: u8,
        x: u32,
        y: &str,
    ) -> Result<Self, AppError> {
        let provider = resolve_basemap_provider(settings);
        if !["carto", "custom"].contains(&provider) {
            return Err(AppError::Validation(
                "raster tiles are not active for this basemap provider".into(),
            ));
        }
        let y = y
            .trim_end_matches(".png")
            .parse::<u32>()
            .map_err(|_| AppError::Validation("invalid tile coordinate".into()))?;
        let bound = 1_u32.checked_shl(u32::from(z)).unwrap_or(0);
        for (rejected, message) in [
            (z > 22, "invalid tile zoom"),
            (x >= bound || y >= bound, "invalid tile coordinate"),
            (
                !["light", "dark"].contains(&style.as_str()),
                "invalid tile style",
            ),
        ] {
            if rejected {
                return Err(AppError::Validation(message.into()));
            }
        }
        let choices = if style == "dark" {
            [
                settings.dark_url_template.as_deref(),
                settings.light_url_template.as_deref(),
            ]
        } else {
            [settings.light_url_template.as_deref(), None]
        };
        let template = choices
            .into_iter()
            .flatten()
            .next()
            .ok_or_else(|| AppError::Validation("tile template missing".into()))?;
        Ok(Self {
            provider,
            template,
            y,
            cache_key: format!(
                "external:basemap:{}:{style}:{z}:{x}:{y}",
                settings.updated_at.timestamp()
            ),
        })
    }
}

impl CachedBasemap {
    async fn from_raster(response: reqwest::Response) -> Result<Self, AppError> {
        if response.content_length().unwrap_or(0) > 5 * 1024 * 1024 {
            return Err(AppError::DependencyUnavailable(
                "Basemap tile exceeded the response limit".into(),
            ));
        }
        let max_age = upstream_cache_ttl(response.headers());
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|header| header.to_str().ok())
            .unwrap_or("image/png")
            .to_owned();
        let bytes = read_response_limited(response, 5 * 1024 * 1024, "Basemap tile").await?;
        Ok(Self {
            bytes,
            content_type,
            max_age,
        })
    }
    async fn store(&self, state: &AppState, key: &str) {
        crate::services::basemap_cache::store(
            &state.redis,
            key,
            &self.bytes,
            &self.content_type,
            self.max_age,
            &state.config.security,
        )
        .await;
    }
}

enum OpenMapFailure {
    Stage(&'static str, AppError),
    Status(reqwest::StatusCode),
}
impl OpenMapFailure {
    fn parts(self) -> (AppError, &'static str, Option<reqwest::StatusCode>) {
        match self {
            Self::Stage(class, error) => (error, class, None),
            Self::Status(status) => (
                AppError::DependencyUnavailable("OpenFreeMap provider returned an error".into()),
                "upstream_status",
                Some(status),
            ),
        }
    }
}

async fn fetch_openfreemap(
    state: &AppState,
    resource: &str,
) -> Result<CachedBasemap, OpenMapFailure> {
    let endpoint =
        Url::parse(&format!("https://tiles.openfreemap.org/{resource}")).map_err(|_| {
            OpenMapFailure::Stage(
                "endpoint_parse",
                AppError::Validation("invalid OpenFreeMap resource".into()),
            )
        })?;
    connections::record_attempt(&state.pool, connections::BASEMAP).await;
    let client = outbound_client_for_url(&endpoint, &[])
        .await
        .map_err(|error| OpenMapFailure::Stage("client_setup", error))?;
    let response = client.get(endpoint).send().await.map_err(|_| {
        OpenMapFailure::Stage(
            "transport",
            AppError::DependencyUnavailable("OpenFreeMap request failed".into()),
        )
    })?;
    if !response.status().is_success() {
        return Err(OpenMapFailure::Status(response.status()));
    }
    let max_age = upstream_cache_ttl(response.headers());
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|header| header.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_owned();
    let bytes = read_openfreemap_response_limited(response, 10 * 1024 * 1024)
        .await
        .map_err(|class| {
            let message = if class == "response_size" {
                "OpenFreeMap resource response exceeded the limit"
            } else {
                "OpenFreeMap resource response failed"
            };
            OpenMapFailure::Stage(class, AppError::DependencyUnavailable(message.into()))
        })?;
    let bytes = match is_json_content_type(&content_type) {
        true => rewrite_openfreemap_json(&bytes)
            .map_err(|error| OpenMapFailure::Stage("invalid_json", error))?,
        false => bytes,
    };
    Ok(CachedBasemap {
        bytes,
        content_type,
        max_age,
    })
}

struct ConnectionProbe {
    endpoint: Url,
    provider: Option<String>,
    user_agent: Option<String>,
    query: Vec<(&'static str, &'static str)>,
}
impl ConnectionProbe {
    fn compile(
        id: &str,
        body: &UpdateConnectionBody,
        settings: &ConnectionSettingsRow,
    ) -> Result<Option<Self>, AppError> {
        let probe = match id {
            connections::NOMINATIM => {
                let base = if body.mode == "custom" {
                    body.base_url.as_deref()
                } else {
                    Some("https://nominatim.openstreetmap.org")
                };
                Self {
                    endpoint: endpoint_join(base, "search")?,
                    provider: None,
                    user_agent: Some(
                        body.request_identifier
                            .as_deref()
                            .map(str::trim)
                            .filter(|text| !text.is_empty())
                            .unwrap_or("R (self-hosted Rivian telemetry)")
                            .to_owned(),
                    ),
                    query: vec![("format", "jsonv2"), ("q", "Kansas"), ("limit", "1")],
                }
            }
            connections::ICONIFY => Self {
                endpoint: endpoint_join(Some("https://api.iconify.design"), "search")?,
                provider: None,
                user_agent: None,
                query: vec![("query", "thermometer"), ("limit", "1")],
            },
            connections::BASEMAP => {
                let provider = resolve_effective_basemap_provider_update(id, body, settings)?;
                let template = match (provider.as_str(), body.mode.as_str()) {
                    ("openfreemap", _) => Some("https://tiles.openfreemap.org/styles/positron"),
                    (_, "custom") => body.light_url_template.as_deref(),
                    _ => Some("https://basemaps.cartocdn.com/light_all/{z}/{x}/{y}.png"),
                }
                .ok_or_else(|| AppError::Validation("light tile template required".into()))?;
                let endpoint = Url::parse(&expand_tile_template(template, 6, 14, 24))
                    .map_err(|_| AppError::Validation("light tile template required".into()))?;
                Self {
                    endpoint,
                    provider: Some(provider),
                    user_agent: None,
                    query: Vec::new(),
                }
            }
            _ => return Ok(None),
        };
        Ok(Some(probe))
    }
    async fn request(
        self,
        state: &AppState,
        body: &UpdateConnectionBody,
        settings: &ConnectionSettingsRow,
        allowlist: &[IpNet],
    ) -> Result<reqwest::RequestBuilder, AppError> {
        let carto_key = self.provider.as_deref() == Some("carto")
            && should_forward_carto_api_key(&body.mode, &self.endpoint);
        let mut request = outbound_client_for_url(&self.endpoint, allowlist)
            .await?
            .get(self.endpoint);
        if let Some(user_agent) = self.user_agent {
            request = request.header(header::USER_AGENT, user_agent);
        }
        if !self.query.is_empty() {
            request = request.query(&self.query);
        }
        if let Some(provider) = self.provider {
            let key = body
                .api_key
                .as_deref()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_owned)
                .or(decrypt_secret(
                    &state.age_key,
                    settings.api_key_encrypted.as_deref(),
                )?);
            if carto_key {
                if let Some(key) = key {
                    request = request.query(&[carto_basemap_key_query(&key)]);
                }
            }
            if should_forward_basemap_bearer_token(&provider) {
                let token = body
                    .bearer_token
                    .as_deref()
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .map(str::to_owned)
                    .or(decrypt_secret(
                        &state.age_key,
                        settings.bearer_token_encrypted.as_deref(),
                    )?);
                if let Some(token) = token {
                    request = request.bearer_auth(token);
                }
            }
        }
        Ok(request)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        active_endpoint, basemap_proxy_url, basemap_styles, carto_basemap_key_query,
        endpoint_is_private, is_forbidden_ip, is_private_ip, openfreemap_cache_key,
        openfreemap_resource_is_allowed, parse_private_network_allowlist, resolve_basemap_provider,
        resolve_effective_basemap_provider_update, rewrite_openfreemap_json,
        should_forward_basemap_bearer_token, should_forward_carto_api_key, validate_tile_template,
        UpdateConnectionBody, BASEMAP_RASTER_ROUTE, OPENFREEMAP_PROXY_ROUTE,
    };
    use super::{
        connections, outbound_client_for_url, update_connection, validate_update, AppError,
    };
    use crate::services::external_connections::ConnectionSettingsRow;
    use axum::{
        body::Body,
        extract::{Path, State},
        http::{Request, StatusCode},
        routing::get,
        Json, Router,
    };
    use chrono::Utc;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    use tower::ServiceExt;
    use url::Url;

    #[tokio::test]
    async fn validates_xyz_template_and_requires_cidr_migration_for_private_targets() {
        let private_allowlist = parse_private_network_allowlist(&["127.0.0.0/8".into()])
            .expect_err("loopback is never allowlistable");
        assert!(matches!(private_allowlist, super::AppError::Validation(_)));
        assert!(
            validate_tile_template("https://127.0.0.1/{z}/{x}/{y}.png", &[])
                .await
                .is_err()
        );
        assert!(
            validate_tile_template("http://127.0.0.1/{z}/{x}/{y}.png", &[])
                .await
                .is_err()
        );
        assert!(validate_tile_template("https://127.0.0.1/{z}/{x}.png", &[])
            .await
            .is_err());
        assert!(endpoint_is_private("http://localhost:8080"));
    }

    #[test]
    fn accepts_only_private_cidr_ranges() {
        let allowlist = parse_private_network_allowlist(&[
            "10.0.0.0/8".into(),
            "fd00::/8".into(),
            "10.0.0.0/8".into(),
        ])
        .expect("private CIDRs");
        assert_eq!(allowlist.len(), 2);
        assert!(parse_private_network_allowlist(&["0.0.0.0/0".into()]).is_err());
        assert!(parse_private_network_allowlist(&["172.0.0.0/8".into()]).is_err());
    }

    #[test]
    fn blocks_metadata_and_non_public_address_classes() {
        assert!(is_forbidden_ip(IpAddr::V4(Ipv4Addr::new(
            169, 254, 169, 254
        ))));
        assert!(is_forbidden_ip(IpAddr::V4(Ipv4Addr::UNSPECIFIED)));
        assert!(is_forbidden_ip(IpAddr::V4(Ipv4Addr::LOCALHOST)));
        assert!(is_forbidden_ip(IpAddr::V6(Ipv6Addr::LOCALHOST)));
        assert!(is_private_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
        assert!(is_private_ip(IpAddr::V6("fd00::1".parse().expect("ULA"))));
        assert!(!is_forbidden_ip(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
        assert!(!is_private_ip(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
    }

    #[test]
    fn openfreemap_proxy_allowlist_is_bounded_and_style_json_is_rewritten() {
        assert!(openfreemap_resource_is_allowed("styles/positron"));
        assert!(openfreemap_resource_is_allowed(
            "planet/20260823_080002_pt/1/2/3.pbf"
        ));
        assert!(openfreemap_resource_is_allowed(
            "natural_earth/ne2sr/1/2/3.png"
        ));
        assert!(openfreemap_resource_is_allowed("sprites/ofm_f384/ofm.json"));
        assert!(openfreemap_resource_is_allowed(
            "sprites/ofm_f384/ofm@2x.png"
        ));
        assert!(openfreemap_resource_is_allowed(
            "fonts/Noto%20Sans%20Regular/0-255.pbf"
        ));
        assert!(!openfreemap_resource_is_allowed("styles/unknown"));
        assert!(!openfreemap_resource_is_allowed(
            "planet/20260823_080002_pt/1/2/3.pbf/extra"
        ));
        assert!(!openfreemap_resource_is_allowed(
            "sprites/positron/ofm.json"
        ));
        assert!(!openfreemap_resource_is_allowed(
            "sprites/ofm_f384/other.png"
        ));
        assert!(!openfreemap_resource_is_allowed("../planet"));
        assert!(!openfreemap_resource_is_allowed(
            "styles/positron?url=https://evil.test"
        ));
        let rewritten = rewrite_openfreemap_json(
            br#"{"sprite":"https://tiles.openfreemap.org/sprites/ofm_f384/ofm","sources":{"planet":{"url":"https://tiles.openfreemap.org/planet"},"labels":{"url":"https://__TILEJSON_DOMAIN__/fonts"}}}"#,
        )
        .expect("valid JSON");
        assert_eq!(
            String::from_utf8(rewritten).unwrap(),
            r#"{"sources":{"labels":{"url":"https://panda.invalid/v1/external/basemap/openfreemap/fonts?cf=v3"},"planet":{"url":"https://panda.invalid/v1/external/basemap/openfreemap/planet?cf=v3"}},"sprite":"https://panda.invalid/v1/external/basemap/openfreemap/sprites/ofm_f384/ofm?cf=v3"}"#
        );
        assert_eq!(
            openfreemap_cache_key(123, "styles/positron"),
            "external:basemap:openfreemap:v3:123:styles/positron"
        );
    }

    #[tokio::test]
    async fn openfreemap_assets_do_not_collide_with_the_raster_route() {
        let app = Router::new()
            .route(
                BASEMAP_RASTER_ROUTE,
                get(|| async { StatusCode::IM_A_TEAPOT }),
            )
            .route(
                OPENFREEMAP_PROXY_ROUTE,
                get(|| async { StatusCode::NO_CONTENT }),
            );

        for uri in [
            "/external/basemap/openfreemap/sprites/ofm_f384/ofm.json",
            "/external/basemap/openfreemap/fonts/Noto%20Sans%20Regular/0-255.pbf",
        ] {
            let response = app
                .clone()
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NO_CONTENT, "{uri}");
        }
    }

    #[test]
    fn openfreemap_exposes_all_style_descriptors() {
        let styles = basemap_styles("openfreemap", "123");
        assert_eq!(styles.len(), 7);
        assert_eq!(styles[0].id, "follow-theme");
        assert!(styles
            .iter()
            .any(|style| style.id == "3d" && style.perspective_3d));
        assert_eq!(
            styles[0].light_url,
            "/v1/external/basemap/openfreemap/styles/positron?v=123&cf=v3"
        );
        assert_eq!(
            styles[0].dark_url,
            "/v1/external/basemap/openfreemap/styles/dark?v=123&cf=v3"
        );
    }

    #[test]
    fn resolves_basemap_provider_with_mode_precedence() {
        let mut settings = basemap_settings("remote", "auto", false, true);
        assert_eq!(resolve_basemap_provider(&settings), "openfreemap");
        assert_eq!(
            active_endpoint(&settings).as_deref(),
            Some("https://tiles.openfreemap.org")
        );
        settings.api_key_encrypted = Some(vec![1]);
        assert_eq!(resolve_basemap_provider(&settings), "carto");
        settings.basemap_provider = "openfreemap".into();
        assert_eq!(resolve_basemap_provider(&settings), "openfreemap");
        settings.mode = "custom".into();
        assert_eq!(resolve_basemap_provider(&settings), "custom");
        settings.enabled = false;
        assert_eq!(resolve_basemap_provider(&settings), "disabled");
    }

    #[test]
    fn resolves_automatic_provider_for_unsaved_and_stored_keys() {
        let mut settings = basemap_settings("remote", "auto", false, true);
        let mut body = UpdateConnectionBody {
            enabled: true,
            mode: "remote".into(),
            basemap_provider: Some("auto".into()),
            api_key: Some("new-key".into()),
            ..Default::default()
        };
        assert_eq!(
            resolve_effective_basemap_provider_update("basemap", &body, &settings).unwrap(),
            "carto"
        );
        body.api_key = None;
        assert_eq!(
            resolve_effective_basemap_provider_update("basemap", &body, &settings).unwrap(),
            "openfreemap"
        );
        settings.api_key_encrypted = Some(vec![1]);
        assert_eq!(
            resolve_effective_basemap_provider_update("basemap", &body, &settings).unwrap(),
            "carto"
        );
        body.clear_api_key = Some(true);
        assert_eq!(
            resolve_effective_basemap_provider_update("basemap", &body, &settings).unwrap(),
            "openfreemap"
        );
    }

    #[test]
    fn forwards_carto_api_keys_only_to_remote_or_carto_owned_templates() {
        let carto = Url::parse("https://basemaps.cartocdn.com/light_all/6/14/24.png").unwrap();
        let custom = Url::parse("https://tiles.example.test/6/14/24.png").unwrap();
        assert!(should_forward_carto_api_key("remote", &carto));
        assert!(should_forward_carto_api_key("custom", &carto));
        assert!(!should_forward_carto_api_key("custom", &custom));
    }

    #[test]
    fn forwards_basemap_bearer_tokens_only_to_custom_provider() {
        assert!(should_forward_basemap_bearer_token("custom"));
        assert!(!should_forward_basemap_bearer_token("carto"));
        assert!(!should_forward_basemap_bearer_token("openfreemap"));
    }

    #[test]
    fn uses_carto_basemap_key_parameter_and_versioned_first_party_proxy_urls() {
        assert_eq!(
            carto_basemap_key_query("stored-secret"),
            ("key", "stored-secret")
        );
        assert_eq!(
            basemap_proxy_url("light", "1724889600123"),
            "/v1/external/basemap/raster/light/{z}/{x}/{y}.png?v=1724889600123"
        );
    }

    #[tokio::test]
    async fn outbound_policy_denies_custom_requests_before_dns_even_with_private_allowlists() {
        let allowlist = ["10.0.0.0/8".parse().unwrap(), "fc00::/7".parse().unwrap()];
        // A hostname can resolve differently at request time. This profile
        // denies the entire custom path, including public-looking hostnames.
        for url in [
            "https://provider.example.test/lookup",
            "https://localhost/lookup",
            "http://10.0.0.1/lookup",
            "https://[fd00::1]/lookup",
            "http://169.254.169.254/latest/meta-data",
            "https://user:password@provider.example.test/lookup",
        ] {
            let error = outbound_client_for_url(&Url::parse(url).unwrap(), &allowlist)
                .await
                .unwrap_err();
            assert!(matches!(error, AppError::ExternalConnectionDisabled(_)));
        }
    }

    #[tokio::test]
    async fn outbound_policy_overrides_preexisting_enabled_custom_provider_settings() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
            .unwrap();
        for id in connections::OPTIONAL_CONNECTIONS {
            let mut saved = basemap_settings("custom", "auto", true, true);
            saved.id = (*id).into();
            saved.allow_private_network = true;
            saved.private_network_allowlist = vec!["10.0.0.0/8".into()];
            saved.private_network_policy_state = "configured".into();
            saved.bearer_token_encrypted = Some(vec![1, 2, 3]);
            assert!(!saved.is_active());
            if *id != connections::OPEN_METEO {
                assert!(matches!(
                    connections::require_enabled(&pool, id).await,
                    Err(AppError::ExternalConnectionDisabled(_))
                ));
            }
        }
    }

    #[tokio::test]
    async fn weather_settings_reject_precision_credentials_and_arbitrary_destinations() {
        let valid = || UpdateConnectionBody {
            enabled: true,
            mode: "remote".into(),
            weather_precision: Some("approximate".into()),
            ..Default::default()
        };
        assert!(validate_update(connections::OPEN_METEO, &valid())
            .await
            .is_ok());
        let mut bad = vec![];
        let mut body = valid();
        body.weather_precision = Some("exact".into());
        bad.push(body);
        let mut body = valid();
        body.mode = "custom".into();
        bad.push(body);
        let mut body = valid();
        body.api_key = Some("synthetic-paid-key".into());
        bad.push(body);
        let mut body = valid();
        body.bearer_token = Some("synthetic-token".into());
        bad.push(body);
        let mut body = valid();
        body.allow_private_network = Some(true);
        bad.push(body);
        let mut body = valid();
        body.private_network_allowlist = Some(vec!["127.0.0.0/8".into()]);
        bad.push(body);
        for destination in [
            "http://api.open-meteo.com/v1/forecast",
            "https://api.open-meteo.com.evil.test/v1/forecast",
            "https://secret@api.open-meteo.com/v1/forecast",
            "https://api.open-meteo.com:444/v1/forecast",
            "https://api.open-meteo.com/v1/forecast?apikey=secret",
            "https://127.0.0.1/v1/forecast",
        ] {
            let mut body = valid();
            body.forecast_url = Some(destination.into());
            bad.push(body);
        }
        for body in bad {
            assert!(validate_update(connections::OPEN_METEO, &body)
                .await
                .is_err());
        }
    }

    #[tokio::test]
    #[ignore = "requires disposable TimescaleDB DATABASE_URL and REDIS_URL"]
    async fn authorization_weather_configuration_is_admin_only_and_fail_closed() {
        let fixture = super::contracts::fixture::Fixture::new().await;
        let pool = &fixture.state.pool;
        connections::ensure_defaults(pool).await.unwrap();
        let admin = fixture.user("admin").await;
        let user = fixture.user("user").await;
        let body = || UpdateConnectionBody {
            enabled: true,
            mode: "remote".into(),
            ..Default::default()
        };
        let attempt = update_connection(
            State(fixture.state.clone()),
            fixture.auth(user),
            Path(connections::OPEN_METEO.into()),
            Json(body()),
        )
        .await;
        assert!(matches!(attempt, Err(AppError::Forbidden)));
        let _ = update_connection(
            State(fixture.state.clone()),
            fixture.auth(admin),
            Path(connections::OPEN_METEO.into()),
            Json(body()),
        )
        .await
        .unwrap();
        assert!(connections::require_enabled(pool, connections::OPEN_METEO)
            .await
            .is_ok());
        let mut stale_form = body();
        stale_form.forecast_url = Some("http://127.0.0.1/old-provider".into());
        stale_form.archive_url = Some("https://old.example.invalid/archive".into());
        stale_form.allow_private_network = Some(true);
        stale_form.private_network_allowlist = Some(vec!["10.0.0.0/8".into()]);
        let _ = update_connection(
            State(fixture.state.clone()),
            fixture.auth(admin),
            Path(connections::OPEN_METEO.into()),
            Json(stale_form),
        )
        .await
        .unwrap();
        let restored = connections::require_enabled(pool, connections::OPEN_METEO)
            .await
            .unwrap();
        assert_eq!(
            restored.forecast_url.as_deref(),
            Some(crate::private_deployment::weather::FORECAST_URL)
        );
        assert!(restored.private_network_allowlist.is_empty());
        for id in [
            connections::NOMINATIM,
            connections::BASEMAP,
            connections::ICONIFY,
        ] {
            assert!(!connections::load(pool, id).await.unwrap().is_active());
        }
        for assignment in [
            "weather_precision='exact'",
            "forecast_url='http://127.0.0.1/private'",
            "api_key_encrypted=decode('00','hex')",
            "allow_private_network=true",
        ] {
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "UPDATE panda.external_connection_settings SET {assignment} WHERE id='open_meteo'"
            )))
            .execute(pool)
            .await
            .unwrap();
            assert!(matches!(
                connections::require_enabled(pool, connections::OPEN_METEO).await,
                Err(AppError::ExternalConnectionDisabled(_))
            ));
            let _ = update_connection(
                State(fixture.state.clone()),
                fixture.auth(admin),
                Path(connections::OPEN_METEO.into()),
                Json(body()),
            )
            .await
            .unwrap();
        }
        fixture.cleanup().await;
    }

    pub(super) fn basemap_settings(
        mode: &str,
        provider: &str,
        has_key: bool,
        enabled: bool,
    ) -> ConnectionSettingsRow {
        ConnectionSettingsRow {
            id: "basemap".into(),
            enabled,
            mode: mode.into(),
            basemap_provider: provider.into(),
            weather_precision: None,
            forecast_url: None,
            archive_url: None,
            base_url: None,
            light_url_template: None,
            dark_url_template: None,
            attribution: None,
            attribution_url: None,
            request_identifier: None,
            custom_autocomplete: false,
            allow_private_network: false,
            private_network_allowlist: Vec::new(),
            private_network_policy_state: "restricted".into(),
            api_key_encrypted: has_key.then_some(vec![1]),
            bearer_token_encrypted: None,
            updated_at: Utc::now(),
        }
    }
}
