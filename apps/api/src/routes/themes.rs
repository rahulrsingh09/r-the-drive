#[path = "themes/lifecycle.rs"]
mod lifecycle;
#[path = "themes/preferences.rs"]
mod preferences;

use std::collections::BTreeSet;

use axum::{
    extract::{Path, State},
    http::{header::ETAG, HeaderMap, HeaderValue},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::{
    errors::AppError,
    middleware::auth::{AppState, AuthUser},
};

const MAX_CUSTOM_THEMES: i64 = 20;
const MAX_DEFINITION_BYTES: usize = 64 * 1024;
const BUILTIN_MANIFEST: &str = include_str!("../themes/builtins.generated.json");
const PREFERENCES_IF_MATCH_HEADER: &str = "x-theme-preferences-if-match";

pub fn router() -> Router<AppState> {
    super::route_table([
        ("/themes/catalog", get(catalog)),
        ("/themes", post(create_theme)),
        ("/themes/{theme_id}", get(get_theme).delete(retire_theme)),
        ("/themes/{theme_id}/revisions", post(save_revision)),
        (
            "/themes/{theme_id}/revisions/{revision}/publish",
            post(publish_revision),
        ),
        ("/themes/{theme_id}/rollback", post(rollback_theme)),
        (
            "/auth/preferences/theme",
            get(get_theme_preferences).put(update_theme_preferences),
        ),
    ])
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ThemeSummary {
    theme_id: Uuid,
    name: String,
    base_theme_id: String,
    published_revision: Option<i32>,
    published_definition: Option<Value>,
    retired_at: Option<chrono::DateTime<chrono::Utc>>,
    etag: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateThemeBody {
    name: String,
    base_theme_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SaveRevisionBody {
    definition: Value,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PublishRevisionBody {
    #[serde(default)]
    apply: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RollbackBody {
    revision: i32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum ThemeSelection {
    Builtin {
        #[serde(rename = "themeId")]
        theme_id: String,
    },
    Custom {
        #[serde(rename = "themeId")]
        theme_id: Uuid,
        revision: i32,
    },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateThemePreferencesBody {
    schema_version: u8,
    mode: String,
    selection: ThemeSelection,
}

fn builtin_manifest() -> Result<Value, AppError> {
    static MANIFEST: std::sync::OnceLock<Result<Value, String>> = std::sync::OnceLock::new();
    MANIFEST
        .get_or_init(|| serde_json::from_str(BUILTIN_MANIFEST).map_err(|error| error.to_string()))
        .clone()
        .map_err(|error| {
            AppError::Internal(anyhow::anyhow!("invalid generated theme manifest: {error}"))
        })
}

fn builtin_ids(manifest: &Value) -> BTreeSet<&str> {
    manifest["builtins"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|theme| theme["id"].as_str())
        .collect()
}

fn validate_builtin_id(theme_id: &str) -> Result<(), AppError> {
    let manifest = builtin_manifest()?;
    if builtin_ids(&manifest).contains(theme_id) {
        Ok(())
    } else {
        Err(AppError::Validation("unknown built-in theme".into()))
    }
}

fn normalize_name(name: &str) -> Result<String, AppError> {
    let trimmed = name.trim();
    match trimmed.chars().take(81).count() {
        1..=80 => Ok(trimmed.to_owned()),
        _ => Err(AppError::Validation(
            "theme name must be between 1 and 80 characters".into(),
        )),
    }
}

fn validate_mode(mode: &str) -> Result<(), AppError> {
    if matches!(mode, "light" | "dark" | "system") {
        Ok(())
    } else {
        Err(AppError::Validation("invalid theme mode".into()))
    }
}

fn validate_color(value: &Value, context: &str) -> Result<(), AppError> {
    let color = value
        .as_str()
        .ok_or_else(|| AppError::Validation(format!("{context} must be a color")))?;
    let valid = color
        .strip_prefix('#')
        .filter(|digits| digits.len() == 6)
        .is_some_and(|digits| digits.bytes().all(|byte| byte.is_ascii_hexdigit()));
    valid
        .then_some(())
        .ok_or_else(|| AppError::Validation(format!("{context} must be a six-digit hex color")))
}

fn validate_color_pair(value: &Value, context: &str) -> Result<(), AppError> {
    let pair = value
        .as_object()
        .ok_or_else(|| AppError::Validation(format!("{context} must be an object")))?;
    let modes = pair.keys().map(String::as_str).collect::<BTreeSet<_>>();
    if modes.is_empty() || !modes.is_subset(&BTreeSet::from(["light", "dark"])) {
        return Err(AppError::Validation(format!(
            "{context} may contain only light and dark"
        )));
    }
    pair.iter()
        .try_for_each(|(mode, color)| validate_color(color, &format!("{context}.{mode}")))
}

fn relative_luminance(color: &str) -> Option<f64> {
    let mut linear = [0.0; 3];
    for (index, component) in linear.iter_mut().enumerate() {
        let start = 1 + index * 2;
        let encoded = f64::from(u8::from_str_radix(color.get(start..start + 2)?, 16).ok()?) / 255.0;
        *component = match encoded {
            value if value <= 0.04045 => value / 12.92,
            value => ((value + 0.055) / 1.055).powf(2.4),
        };
    }
    Some(0.2126 * linear[0] + 0.7152 * linear[1] + 0.0722 * linear[2])
}

fn contrast_ratio(first: &str, second: &str) -> Option<f64> {
    let first = relative_luminance(first)?;
    let second = relative_luminance(second)?;
    Some((first.max(second) + 0.05) / (first.min(second) + 0.05))
}

fn resolved_token<'a>(
    base: &'a Value,
    root: &'a Map<String, Value>,
    token: &str,
    mode: &str,
) -> Option<&'a str> {
    let mut current = root.get("tokens");
    for key in [token, mode] {
        current = current
            .and_then(Value::as_object)
            .and_then(|object| object.get(key));
    }
    match current.and_then(Value::as_str) {
        Some(color) => Some(color),
        None => base["tokens"][mode][token].as_str(),
    }
}

fn validate_theme_definition(base_theme_id: &str, definition: &Value) -> Result<Vec<u8>, AppError> {
    let bytes = serde_json::to_vec(definition)
        .map_err(|_| AppError::Validation("theme definition is not valid JSON".into()))?;
    if bytes.len() > MAX_DEFINITION_BYTES {
        return Err(AppError::Validation(
            "theme definition exceeds 64 KiB".into(),
        ));
    }
    let root = definition
        .as_object()
        .ok_or_else(|| AppError::Validation("theme definition must be an object".into()))?;
    let properties = BTreeSet::from(["theme", "tokens", "series", "brandPaints"]);
    if root.keys().any(|key| !properties.contains(key.as_str())) {
        return Err(AppError::Validation(
            "theme definition contains an unsupported property".into(),
        ));
    }
    if root.get("theme").and_then(Value::as_str) != Some(base_theme_id) {
        return Err(AppError::Validation(
            "theme definition must inherit from its built-in base".into(),
        ));
    }
    let manifest = builtin_manifest()?;
    let base = manifest["builtins"]
        .as_array()
        .and_then(|themes| themes.iter().find(|theme| theme["id"] == base_theme_id))
        .ok_or_else(|| AppError::Validation("unknown built-in theme".into()))?;
    for section in [
        OverrideSection::Tokens,
        OverrideSection::Series,
        OverrideSection::BrandPaints,
    ] {
        if let Some(value) = root.get(section.name()) {
            section.validate(value, base)?;
        }
    }
    for (mode, background) in ["light", "dark"]
        .into_iter()
        .flat_map(|mode| ["bg-page", "bg-surface"].map(|background| (mode, background)))
    {
        let mut colors = ["", ""];
        for (index, token) in ["text-primary", background].into_iter().enumerate() {
            colors[index] = resolved_token(base, root, token, mode)
                .ok_or_else(|| AppError::Validation(format!("missing {token}.{mode}")))?;
        }
        match contrast_ratio(colors[0], colors[1]) {
            Some(ratio) if ratio >= 4.5 => {}
            _ => {
                return Err(AppError::Validation(format!(
                    "critical contrast failure for text-primary on {background} in {mode} mode"
                )))
            }
        }
    }
    Ok(bytes)
}

fn definition_hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn theme_etag(theme_id: Uuid, version: i64) -> String {
    format!("\"theme-{theme_id}-{version}\"")
}

fn preferences_etag(version: i64) -> String {
    format!("\"theme-preferences-{version}\"")
}

fn require_etag(headers: &HeaderMap, expected: &str) -> Result<(), AppError> {
    let supplied = headers
        .get(axum::http::header::IF_MATCH)
        .and_then(|value| value.to_str().ok());
    if supplied == Some(expected) {
        Ok(())
    } else {
        Err(AppError::Conflict(
            "theme changed since it was loaded; refresh and try again".into(),
        ))
    }
}

async fn require_preference_etag(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    headers: &HeaderMap,
) -> Result<(), AppError> {
    ensure_preferences_row(tx, user_id).await?;
    let (version,): (i64,) = sqlx::query_as(
        "SELECT theme_etag_version FROM panda.user_preferences WHERE user_id=$1 FOR UPDATE",
    )
    .bind(user_id)
    .fetch_one(&mut **tx)
    .await?;
    let expected = preferences_etag(version);
    match headers
        .get(PREFERENCES_IF_MATCH_HEADER)
        .and_then(|value| value.to_str().ok())
    {
        Some(value) if value == expected => Ok(()),
        _ => Err(AppError::Conflict(
            "appearance preferences changed since they were loaded; refresh and try again".into(),
        )),
    }
}

fn etag_json(etag: &str, payload: Value) -> Result<Response, AppError> {
    let value = HeaderValue::from_str(etag)
        .map_err(|_| AppError::Internal(anyhow::anyhow!("invalid generated ETag")))?;
    Ok(([(ETAG, value)], Json(payload)).into_response())
}

async fn list_owned_themes(
    state: &AppState,
    user_id: Uuid,
    include_retired: bool,
) -> Result<Vec<ThemeSummary>, AppError> {
    #[derive(sqlx::FromRow)]
    struct SummaryRow {
        id: Uuid,
        name: String,
        base_theme_id: String,
        published_revision: Option<i32>,
        retired_at: Option<chrono::DateTime<chrono::Utc>>,
        etag_version: i64,
        published_definition: Option<Value>,
    }
    let rows = sqlx::query_as::<_, SummaryRow>(
        "SELECT t.id, t.name, t.base_theme_id, t.published_revision, t.retired_at, t.etag_version,
                r.definition AS published_definition
         FROM panda.user_themes t LEFT JOIN panda.user_theme_revisions r
           ON r.theme_id=t.id AND r.revision=t.published_revision
         WHERE t.owner_id=$1 AND ($2 OR t.retired_at IS NULL)
         ORDER BY t.updated_at DESC,t.id",
    )
    .bind(user_id)
    .bind(include_retired)
    .fetch_all(&state.pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| ThemeSummary {
            theme_id: row.id,
            name: row.name,
            base_theme_id: row.base_theme_id,
            published_revision: row.published_revision,
            published_definition: row.published_definition,
            retired_at: row.retired_at,
            etag: theme_etag(row.id, row.etag_version),
        })
        .collect())
}

async fn catalog(State(state): State<AppState>, auth: AuthUser) -> Result<Json<Value>, AppError> {
    let manifest = builtin_manifest()?;
    let custom = list_owned_themes(&state, auth.user_id, false).await?;
    Ok(Json(json!({
        "schemaVersion": 2,
        "registryHash": manifest["registryHash"],
        "builtins": manifest["builtins"],
        "customThemes": custom,
    })))
}

async fn create_theme(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateThemeBody>,
) -> Result<Response, AppError> {
    lifecycle::NewTheme::prepare(body)?
        .create(&state, auth.user_id)
        .await
}

async fn get_theme(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(theme_id): Path<Uuid>,
) -> Result<Response, AppError> {
    #[derive(sqlx::FromRow)]
    struct OwnedTheme {
        name: String,
        base_theme_id: String,
        published_revision: Option<i32>,
        retired_at: Option<chrono::DateTime<chrono::Utc>>,
        etag_version: i64,
    }
    #[derive(sqlx::FromRow, Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Revision {
        revision: i32,
        definition: Value,
        definition_hash: String,
        created_at: chrono::DateTime<chrono::Utc>,
        published_at: Option<chrono::DateTime<chrono::Utc>>,
    }
    let theme = sqlx::query_as::<_, OwnedTheme>(
        "SELECT name,base_theme_id,published_revision,retired_at,etag_version FROM panda.user_themes WHERE id=$1 AND owner_id=$2")
        .bind(theme_id).bind(auth.user_id).fetch_optional(&state.pool).await?.ok_or(AppError::NotFound)?;
    let revisions = sqlx::query_as::<_, Revision>(
        "SELECT r.revision,r.definition,r.definition_hash,r.created_at,p.published_at
         FROM panda.user_theme_revisions r LEFT JOIN panda.user_theme_publications p
         ON (p.theme_id,p.revision)=(r.theme_id,r.revision) WHERE r.theme_id=$1 ORDER BY r.revision DESC")
        .bind(theme_id).fetch_all(&state.pool).await?;
    let etag = theme_etag(theme_id, theme.etag_version);
    let payload = json!({
        "themeId":theme_id, "name":theme.name, "baseThemeId":theme.base_theme_id,
        "publishedRevision":theme.published_revision, "retiredAt":theme.retired_at,
        "etag":etag, "revisions":revisions,
    });
    etag_json(&etag, payload)
}

async fn lock_owned_theme(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    theme_id: Uuid,
) -> Result<(String, i64, Option<chrono::DateTime<chrono::Utc>>), AppError> {
    sqlx::query_as(
        "SELECT base_theme_id,etag_version,retired_at FROM panda.user_themes
         WHERE (id,owner_id)=($1,$2) FOR UPDATE",
    )
    .bind(theme_id)
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(AppError::NotFound)
}

async fn save_revision(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(theme_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<SaveRevisionBody>,
) -> Result<Response, AppError> {
    let result = lifecycle::execute(
        &state,
        auth.user_id,
        theme_id,
        &headers,
        lifecycle::Action::Save(&body.definition),
    )
    .await?;
    result.response(&state, auth.user_id, theme_id).await
}

async fn apply_custom_selection_tx(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    mode: Option<&str>,
    theme_id: Uuid,
    revision: i32,
    base_theme_id: &str,
) -> Result<i64, AppError> {
    let version: i64 = sqlx::query_scalar(
        "INSERT INTO panda.user_preferences
          (user_id,theme_mode,theme_palette,theme_selection_kind,theme_builtin_id,theme_custom_id,theme_custom_revision,theme_etag_version,updated_at)
         VALUES ($1,COALESCE($2,'dark'),$3,'custom',$3,$4,$5,1,now())
         ON CONFLICT (user_id) DO UPDATE SET
          theme_mode=COALESCE($2,panda.user_preferences.theme_mode), theme_palette=EXCLUDED.theme_palette,
          theme_selection_kind=EXCLUDED.theme_selection_kind,theme_builtin_id=EXCLUDED.theme_builtin_id,
          theme_custom_id=EXCLUDED.theme_custom_id,theme_custom_revision=EXCLUDED.theme_custom_revision,
          theme_etag_version=panda.user_preferences.theme_etag_version+1,updated_at=now()
         RETURNING theme_etag_version")
        .bind(user_id).bind(mode).bind(base_theme_id).bind(theme_id).bind(revision)
        .fetch_one(&mut **tx).await?;
    Ok(version)
}

async fn publish_revision(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((theme_id, revision)): Path<(Uuid, i32)>,
    headers: HeaderMap,
    Json(body): Json<PublishRevisionBody>,
) -> Result<Response, AppError> {
    let result = lifecycle::execute(
        &state,
        auth.user_id,
        theme_id,
        &headers,
        lifecycle::Action::Publish {
            revision,
            apply: body.apply,
        },
    )
    .await?;
    result.response(&state, auth.user_id, theme_id).await
}

async fn rollback_theme(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(theme_id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<RollbackBody>,
) -> Result<Response, AppError> {
    let result = lifecycle::execute(
        &state,
        auth.user_id,
        theme_id,
        &headers,
        lifecycle::Action::Rollback(body.revision),
    )
    .await?;
    result.response(&state, auth.user_id, theme_id).await
}

async fn retire_theme(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(theme_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let result = lifecycle::execute(
        &state,
        auth.user_id,
        theme_id,
        &headers,
        lifecycle::Action::Retire,
    )
    .await?;
    result.response(&state, auth.user_id, theme_id).await
}

async fn ensure_preferences_row(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
) -> Result<(), AppError> {
    sqlx::query("INSERT INTO panda.user_preferences (user_id) VALUES ($1) ON CONFLICT DO NOTHING")
        .bind(user_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn theme_preferences_payload(
    state: &AppState,
    user_id: Uuid,
) -> Result<(Value, String), AppError> {
    preferences::load(state, user_id).await
}

async fn get_theme_preferences(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Response, AppError> {
    let (payload, etag) = theme_preferences_payload(&state, auth.user_id).await?;
    etag_json(&etag, payload)
}

async fn update_theme_preferences(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Json(body): Json<UpdateThemePreferencesBody>,
) -> Result<Response, AppError> {
    preferences::update(&state, auth.user_id, &headers, body).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_bounded_theme_override_shape() {
        let valid = json!({
            "theme": "classic",
            "tokens": { "accent": { "light": "#123456", "dark": "#abcdef" } },
            "series": { "series-16": { "dark": "#112233" } },
            "brandPaints": { "mark": { "light": "#445566" } }
        });
        assert!(validate_theme_definition("classic", &valid).is_ok());

        for invalid in [
            json!({ "theme": "classic", "font": "Comic Sans" }),
            json!({ "theme": "classic", "tokens": { "unknown": { "light": "#123456" } } }),
            json!({ "theme": "classic", "series": { "series-17": { "light": "#123456" } } }),
            json!({ "theme": "classic", "tokens": { "accent": { "light": "url(https://example.test)" } } }),
            json!({ "theme": "rad", "tokens": {} }),
            json!({ "theme": "classic", "tokens": { "text-primary": { "dark": "#111111" }, "bg-page": { "dark": "#111111" } } }),
        ] {
            assert!(validate_theme_definition("classic", &invalid).is_err());
        }
    }

    #[test]
    fn generated_manifest_is_readable_and_complete() {
        let manifest = builtin_manifest().expect("manifest");
        assert_eq!(manifest["schemaVersion"], 1);
        assert_eq!(manifest["builtins"].as_array().map(Vec::len), Some(2));
        for theme in manifest["builtins"].as_array().unwrap() {
            assert_eq!(theme["series"].as_object().map(Map::len), Some(16));
        }
    }

    #[test]
    fn etags_are_resource_specific() {
        let id = Uuid::nil();
        assert_eq!(theme_etag(id, 3), format!("\"theme-{id}-3\""));
        assert_eq!(preferences_etag(4), "\"theme-preferences-4\"");
    }
}

#[cfg(test)]
mod full_replacement_characterization {
    use super::*;

    #[test]
    fn ordered_definition_failures_and_color_boundaries() {
        let manifest = builtin_manifest().unwrap();
        let base = manifest["builtins"][0]["id"].as_str().unwrap();
        for (definition, expected) in [
            (Value::Null, "theme definition must be an object"),
            (
                json!({"extra": 1}),
                "theme definition contains an unsupported property",
            ),
            (
                json!({}),
                "theme definition must inherit from its built-in base",
            ),
            (
                json!({"theme":base, "tokens":null}),
                "tokens must be an object",
            ),
            (
                json!({"theme":base, "tokens":{"shadow-1":{}}}),
                "unknown or non-customizable theme token: shadow-1",
            ),
            (
                json!({"theme":base, "series":{"series-01":{"other":5}}}),
                "series.series-01 may contain only light and dark",
            ),
            (
                json!({"theme":base, "series":{"series-01":{"light":5}}}),
                "series.series-01.light must be a color",
            ),
            (
                json!({"theme":base, "series":{"series-01":{"light":"#123"}}}),
                "series.series-01.light must be a six-digit hex color",
            ),
            (
                json!({"theme":base, "brandPaints":{"wrong":{}}}),
                "unknown brand paint: wrong",
            ),
        ] {
            match validate_theme_definition(base, &definition) {
                Err(AppError::Validation(message)) => assert_eq!(message, expected),
                result => panic!("expected {expected}: {result:?}"),
            }
        }
        for value in ["#000000", "#ffffff", "#ABCdef"] {
            assert!(validate_color(&json!(value), "color").is_ok());
        }
        assert_eq!(relative_luminance("#000000"), Some(0.0));
        assert_eq!(relative_luminance("#ffffff"), Some(1.0));
        assert_eq!(contrast_ratio("#000000", "#ffffff"), Some(21.0));
        assert_eq!(normalize_name(&"é".repeat(80)).unwrap().chars().count(), 80);
        assert!(normalize_name(&"é".repeat(81)).is_err());
    }
}

#[derive(Clone, Copy)]
enum OverrideSection {
    Tokens,
    Series,
    BrandPaints,
}

impl OverrideSection {
    fn name(self) -> &'static str {
        match self {
            Self::Tokens => "tokens",
            Self::Series => "series",
            Self::BrandPaints => "brandPaints",
        }
    }

    fn validate(self, value: &Value, base: &Value) -> Result<(), AppError> {
        let section = self.name();
        let pairs = value
            .as_object()
            .ok_or_else(|| AppError::Validation(format!("{section} must be an object")))?;
        for (key, pair) in pairs {
            let rejection = match self {
                Self::Tokens => {
                    let known = base["tokens"]["light"]
                        .as_object()
                        .is_some_and(|tokens| tokens.contains_key(key));
                    (!known
                        || key.starts_with("glow-")
                        || key.starts_with("shadow-")
                        || key == "value-halo")
                        .then(|| format!("unknown or non-customizable theme token: {key}"))
                }
                Self::Series => {
                    let valid = key.len() == 9
                        && key
                            .strip_prefix("series-")
                            .and_then(|number| number.parse::<u8>().ok())
                            .is_some_and(|number| (1..=16).contains(&number));
                    (!valid).then(|| format!("unknown series slot: {key}"))
                }
                Self::BrandPaints => (!["accent", "accentMuted", "mark"].contains(&key.as_str()))
                    .then(|| format!("unknown brand paint: {key}")),
            };
            if let Some(message) = rejection {
                return Err(AppError::Validation(message));
            }
            validate_color_pair(pair, &format!("{section}.{key}"))?;
        }
        Ok(())
    }
}
