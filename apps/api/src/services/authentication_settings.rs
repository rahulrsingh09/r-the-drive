use chrono::{DateTime, Utc};
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use url::Url;
use uuid::Uuid;

use crate::{
    config::OidcEnvOverrides,
    errors::AppError,
    ingestion::session_store::{decrypt_json, encrypt_json},
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingSource {
    Default,
    Database,
    Environment,
}

#[derive(Debug, Clone, Serialize)]
pub struct EffectiveValue<T> {
    pub value: T,
    pub source: SettingSource,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthenticationSettingsResponse {
    pub oidc_enabled: EffectiveValue<bool>,
    pub password_login_enabled: EffectiveValue<bool>,
    pub issuer_url: EffectiveValue<Option<String>>,
    pub public_base_url: EffectiveValue<Option<String>>,
    pub client_id: EffectiveValue<Option<String>>,
    pub client_secret: SecretStatus,
    pub button_label: EffectiveValue<String>,
    pub scopes: EffectiveValue<String>,
    pub token_auth_method: EffectiveValue<String>,
    pub auto_signup: EffectiveValue<bool>,
    pub auto_link_verified_email: EffectiveValue<bool>,
    pub oidc_auto_login: EffectiveValue<bool>,
    pub allowed_email_domains: EffectiveValue<Vec<String>>,
    pub required_claim_name: EffectiveValue<Option<String>>,
    pub required_claim_value: EffectiveValue<Option<String>>,
    pub last_validation_at: Option<DateTime<Utc>>,
    pub callback_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SecretStatus {
    pub configured: bool,
    pub source: SettingSource,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct AuthenticationSettingsUpdate {
    pub oidc_enabled: Option<bool>,
    pub password_login_enabled: Option<bool>,
    pub issuer_url: Option<Option<String>>,
    pub public_base_url: Option<Option<String>>,
    pub client_id: Option<Option<String>>,
    pub client_secret: Option<Option<String>>,
    pub button_label: Option<String>,
    pub scopes: Option<String>,
    pub token_auth_method: Option<String>,
    pub auto_signup: Option<bool>,
    pub auto_link_verified_email: Option<bool>,
    pub oidc_auto_login: Option<bool>,
    pub allowed_email_domains: Option<Vec<String>>,
    pub required_claim_name: Option<Option<String>>,
    pub required_claim_value: Option<Option<String>>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct StoredSettings {
    oidc_enabled: bool,
    password_login_enabled: bool,
    issuer_url: Option<String>,
    public_base_url: Option<String>,
    client_id: Option<String>,
    client_secret_encrypted: Option<Vec<u8>>,
    button_label: String,
    scopes: String,
    token_auth_method: String,
    auto_signup: bool,
    auto_link_verified_email: bool,
    oidc_auto_login: bool,
    allowed_email_domains: Vec<String>,
    required_claim_name: Option<String>,
    required_claim_value: Option<String>,
    last_validation_at: Option<DateTime<Utc>>,
    last_validation_fingerprint: Option<String>,
}

/// Internal, server-only view of the effective provider configuration.  The
/// client secret deliberately has no representation in the public response.
#[derive(Debug, Clone)]
pub struct EffectiveAuthenticationSettings {
    pub oidc_enabled: bool,
    pub password_login_enabled: bool,
    pub issuer_url: Option<String>,
    pub public_base_url: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub button_label: String,
    pub scopes: String,
    pub token_auth_method: String,
    pub auto_signup: bool,
    pub auto_link_verified_email: bool,
    pub oidc_auto_login: bool,
    pub allowed_email_domains: Vec<String>,
    pub required_claim_name: Option<String>,
    pub required_claim_value: Option<String>,
}

fn environment() -> Result<OidcEnvOverrides, AppError> {
    let values =
        OidcEnvOverrides::from_env().map_err(|error| AppError::Validation(error.to_string()))?;
    match values.validate() {
        Ok(()) => Ok(values),
        Err(error) => Err(AppError::Validation(error.to_string())),
    }
}

fn setting<T>(database: T, override_value: Option<T>) -> EffectiveValue<T> {
    match override_value {
        Some(value) => EffectiveValue {
            value,
            source: SettingSource::Environment,
        },
        None => EffectiveValue {
            value: database,
            source: SettingSource::Database,
        },
    }
}

fn optional_setting<T>(
    database: Option<T>,
    override_value: Option<T>,
) -> EffectiveValue<Option<T>> {
    setting(database, override_value.map(Some))
}

fn effective(s: StoredSettings, e: &OidcEnvOverrides) -> AuthenticationSettingsResponse {
    let public_base_url = optional_setting(s.public_base_url, e.public_base_url.clone());
    let callback_url = public_base_url
        .value
        .as_deref()
        .map(|base| format!("{}/v1/auth/oidc/callback", base.trim_end_matches('/')));
    AuthenticationSettingsResponse {
        oidc_enabled: setting(s.oidc_enabled, e.oidc_enabled),
        password_login_enabled: setting(s.password_login_enabled, e.password_login_enabled),
        issuer_url: optional_setting(
            s.issuer_url,
            e.issuer_url.as_deref().map(canonicalize_issuer),
        ),
        public_base_url,
        client_id: optional_setting(s.client_id, e.client_id.clone()),
        client_secret: SecretStatus {
            configured: e.client_secret.is_some() || s.client_secret_encrypted.is_some(),
            source: setting(false, e.client_secret.as_ref().map(|_| true)).source,
        },
        button_label: setting(s.button_label, e.button_label.clone()),
        scopes: setting(s.scopes, e.scopes.clone()),
        token_auth_method: setting(s.token_auth_method, e.token_auth_method.clone()),
        auto_signup: setting(s.auto_signup, e.auto_signup),
        auto_link_verified_email: setting(s.auto_link_verified_email, e.auto_link_verified_email),
        oidc_auto_login: setting(s.oidc_auto_login, e.oidc_auto_login),
        allowed_email_domains: setting(s.allowed_email_domains, e.allowed_email_domains.clone()),
        required_claim_name: optional_setting(s.required_claim_name, e.required_claim_name.clone()),
        required_claim_value: optional_setting(
            s.required_claim_value,
            e.required_claim_value.clone(),
        ),
        last_validation_at: s.last_validation_at,
        callback_url,
    }
}

fn resolve(
    stored: StoredSettings,
    env: &OidcEnvOverrides,
    age_key: &str,
) -> Result<EffectiveAuthenticationSettings, AppError> {
    let database_secret = stored
        .client_secret_encrypted
        .as_deref()
        .map(|bytes| {
            let identity = age_key
                .parse::<age::x25519::Identity>()
                .map_err(|_| AppError::Internal(anyhow::anyhow!("invalid age key")))?;
            decrypt_json::<String>(bytes, &identity).map_err(AppError::Internal)
        })
        .transpose()?;
    let public = effective(stored, env);
    validate_required_claim_pair(
        public.required_claim_name.value.as_deref(),
        public.required_claim_value.value.as_deref(),
    )?;
    Ok(EffectiveAuthenticationSettings {
        oidc_enabled: public.oidc_enabled.value,
        password_login_enabled: public.password_login_enabled.value,
        issuer_url: public.issuer_url.value,
        public_base_url: public.public_base_url.value,
        client_id: public.client_id.value,
        client_secret: env.client_secret.clone().or(database_secret),
        button_label: public.button_label.value,
        scopes: public.scopes.value,
        token_auth_method: public.token_auth_method.value,
        auto_signup: public.auto_signup.value,
        auto_link_verified_email: public.auto_link_verified_email.value,
        oidc_auto_login: public.oidc_auto_login.value,
        allowed_email_domains: public.allowed_email_domains.value,
        required_claim_name: public.required_claim_name.value,
        required_claim_value: public.required_claim_value.value,
    })
}

pub async fn load_effective<'e, E>(
    executor: E,
    age_key: &str,
) -> Result<EffectiveAuthenticationSettings, AppError>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let stored = load_stored(executor).await?;
    resolve(stored, &environment()?, age_key)
}

pub async fn load(
    pool: &PgPool,
    _age_key: &str,
) -> Result<AuthenticationSettingsResponse, AppError> {
    let response = effective(load_stored(pool).await?, &environment()?);
    validate_required_claim_pair(
        response.required_claim_name.value.as_deref(),
        response.required_claim_value.value.as_deref(),
    )
    .map(|()| response)
}

pub async fn update(
    pool: &PgPool,
    age_key: &str,
    actor: Uuid,
    body: AuthenticationSettingsUpdate,
) -> Result<AuthenticationSettingsResponse, AppError> {
    let mut body = normalize_update(body)?;
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT id FROM panda.authentication_settings WHERE id=TRUE FOR UPDATE")
        .execute(&mut *tx)
        .await?;
    let current = load_stored(&mut *tx).await?;
    let env = environment()?;
    reject_environment_owned_update(&body, &env)?;
    let before = resolve(current.clone(), &env, age_key)?;
    let invalidates = body.issuer_url.is_some()
        || body.public_base_url.is_some()
        || body.client_id.is_some()
        || body.client_secret.is_some()
        || body.scopes.is_some()
        || body.token_auth_method.is_some();
    let secret_update = body.client_secret.take();
    let mut proposed = apply_update(current.clone(), body);
    let public_after = effective(proposed.clone(), &env);
    if public_after.public_base_url.value.is_some() {
        oidc_callback_url(public_after.public_base_url.value.as_deref())?;
    }
    let after = resolve(proposed.clone(), &env, age_key)?;
    if !after.password_login_enabled {
        if invalidates {
            return Err(AppError::Validation(
                "save and test provider changes before disabling password login".into(),
            ));
        }
        let issuer = after.issuer_url.as_deref().ok_or_else(|| {
            AppError::Validation(
                "password login cannot be disabled without a configured OIDC issuer".into(),
            )
        })?;
        let fingerprint = validation_fingerprint(&before, age_key);
        let tested = current.last_validation_at.is_some()
            && current.last_validation_fingerprint.as_deref() == Some(fingerprint.as_str());
        if !after.oidc_enabled || !tested {
            return Err(AppError::Validation("password login cannot be disabled until the effective OIDC configuration has been successfully tested".into()));
        }
        if !has_linked_super_user_for_issuer(&mut *tx, issuer).await? {
            return Err(AppError::Validation(
                "password login cannot be disabled until a super user has linked OIDC".into(),
            ));
        }
    }
    if let Some(secret) = secret_update {
        proposed.client_secret_encrypted = encrypt_secret(secret.as_deref(), age_key)?;
    }
    sqlx::query("UPDATE panda.authentication_settings SET oidc_enabled=$1,password_login_enabled=$2,issuer_url=$3,public_base_url=$4,client_id=$5,client_secret_encrypted=$6,button_label=$7,scopes=$8,token_auth_method=$9,auto_signup=$10,auto_link_verified_email=$11,oidc_auto_login=$12,allowed_email_domains=$13,required_claim_name=$14,required_claim_value=$15,updated_by=$16,last_validation_at=CASE WHEN $17 THEN NULL ELSE last_validation_at END,last_validation_fingerprint=CASE WHEN $17 THEN NULL ELSE last_validation_fingerprint END,updated_at=now() WHERE id=TRUE")
        .bind(proposed.oidc_enabled).bind(proposed.password_login_enabled).bind(proposed.issuer_url).bind(proposed.public_base_url).bind(proposed.client_id).bind(proposed.client_secret_encrypted)
        .bind(proposed.button_label).bind(proposed.scopes).bind(proposed.token_auth_method).bind(proposed.auto_signup).bind(proposed.auto_link_verified_email).bind(proposed.oidc_auto_login)
        .bind(proposed.allowed_email_domains).bind(proposed.required_claim_name).bind(proposed.required_claim_value).bind(actor).bind(invalidates).execute(&mut *tx).await?;
    tx.commit().await?;
    load(pool, age_key).await
}

fn apply_update(
    mut stored: StoredSettings,
    body: AuthenticationSettingsUpdate,
) -> StoredSettings {
    macro_rules! apply { ($($field:ident),+)=>{ $(if let Some(value)=body.$field { stored.$field=value; })+ }; }
    apply!(
        oidc_enabled,
        password_login_enabled,
        issuer_url,
        public_base_url,
        client_id,
        button_label,
        scopes,
        token_auth_method,
        auto_signup,
        auto_link_verified_email,
        oidc_auto_login,
        allowed_email_domains,
        required_claim_name,
        required_claim_value
    );
    stored
}

fn encrypt_secret(secret: Option<&str>, age_key: &str) -> Result<Option<Vec<u8>>, AppError> {
    secret
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            let identity = age_key
                .parse::<age::x25519::Identity>()
                .map_err(|_| AppError::Internal(anyhow::anyhow!("invalid age key")))?;
            encrypt_json(&value.to_owned(), &identity).map_err(AppError::Internal)
        })
        .transpose()
}

async fn has_linked_super_user_for_issuer<'e, E>(
    executor: E,
    issuer: &str,
) -> Result<bool, AppError>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM panda.users u WHERE u.role='super_user' AND NOT u.is_disabled AND EXISTS(SELECT 1 FROM panda.user_oidc_identities i WHERE i.user_id=u.id AND rtrim(i.issuer,'/')=$1))")
        .bind(canonicalize_issuer(issuer)).fetch_one(executor).await.map_err(AppError::Database)
}

fn reject_environment_owned_update(
    body: &AuthenticationSettingsUpdate,
    env: &OidcEnvOverrides,
) -> Result<(), AppError> {
    macro_rules! check { ($($field:ident),+)=>{ $(if body.$field.is_some() && env.$field.is_some() { return Err(AppError::Conflict("an environment-owned authentication setting cannot be changed in the UI".into())); })+ }; }
    check!(
        oidc_enabled,
        password_login_enabled,
        issuer_url,
        public_base_url,
        client_id,
        client_secret,
        button_label,
        scopes,
        token_auth_method,
        auto_signup,
        auto_link_verified_email,
        oidc_auto_login,
        allowed_email_domains,
        required_claim_name,
        required_claim_value
    );
    Ok(())
}

pub async fn record_validation(
    pool: &PgPool,
    settings: &EffectiveAuthenticationSettings,
    age_key: &str,
) -> Result<(), AppError> {
    sqlx::query("UPDATE panda.authentication_settings SET last_validation_at=now(),last_validation_fingerprint=$1 WHERE id=TRUE")
        .bind(validation_fingerprint(settings,age_key)).execute(pool).await.map(|_|()).map_err(AppError::Database)
}

pub fn validate_effective(settings: &AuthenticationSettingsResponse) -> Result<(), AppError> {
    validate_required_claim_pair(
        settings.required_claim_name.value.as_deref(),
        settings.required_claim_value.value.as_deref(),
    )?;
    match settings.oidc_enabled.value {
        false => Ok(()),
        true => {
            let complete = settings.issuer_url.value.is_some()
                && settings.public_base_url.value.is_some()
                && settings.client_id.value.is_some()
                && settings.client_secret.configured;
            if !complete {
                return Err(AppError::Validation("OIDC is enabled but issuer, public base URL, client ID, or client secret is missing".into()));
            }
            normalize_oidc_scopes(&settings.scopes.value)?;
            oidc_callback_url(settings.public_base_url.value.as_deref()).map(|_| ())
        }
    }
}

pub fn oidc_callback_url(public_base_url: Option<&str>) -> Result<String, AppError> {
    let Some(base) = public_base_url.map(str::trim).filter(|s| !s.is_empty()) else {
        return Err(AppError::Validation(
            "OIDC public base URL is not configured".into(),
        ));
    };
    let callback = [base.trim_end_matches('/'), "/v1/auth/oidc/callback"].concat();
    let parsed = Url::parse(&callback)
        .map_err(|_| AppError::Validation("OIDC callback URL is invalid".into()))?;
    let permitted = parsed.scheme() == "https"
        && parsed.host_str().is_some()
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.query().is_none()
        && parsed.fragment().is_none();
    if permitted {
        Ok(callback)
    } else {
        Err(AppError::Validation(
            "OIDC callback URL must be absolute HTTPS without credentials, query, or fragment"
                .into(),
        ))
    }
}

fn validate_required_claim_pair(name: Option<&str>, value: Option<&str>) -> Result<(), AppError> {
    let present = |v: Option<&str>| v.is_some_and(|v| !v.trim().is_empty());
    if present(name) == present(value) {
        Ok(())
    } else {
        Err(AppError::Validation(
            "OIDC required claim name and value must be configured together".into(),
        ))
    }
}

fn normalize_update(
    mut body: AuthenticationSettingsUpdate,
) -> Result<AuthenticationSettingsUpdate, AppError> {
    for field in [
        &mut body.issuer_url,
        &mut body.public_base_url,
        &mut body.client_id,
        &mut body.required_claim_name,
        &mut body.required_claim_value,
    ] {
        *field = normalize_optional_update(field.take());
    }
    if let Some(Some(issuer)) = body.issuer_url.as_mut() {
        *issuer = canonicalize_issuer(issuer);
    }
    if let Some(label) = body.button_label.as_mut() {
        *label = label.trim().to_owned();
        if label.is_empty() {
            return Err(AppError::Validation(
                "OIDC button label cannot be empty".into(),
            ));
        }
    }
    body.scopes = body
        .scopes
        .as_deref()
        .map(normalize_oidc_scopes)
        .transpose()?;
    if let Some(method) = body.token_auth_method.as_mut() {
        *method = method.trim().to_owned();
        if !["auto", "client_secret_basic", "client_secret_post"].contains(&method.as_str()) {
            return Err(AppError::Validation(
                "OIDC token authentication method is invalid".into(),
            ));
        }
    }
    if let Some(domains) = body.allowed_email_domains.as_mut() {
        let normalized: std::collections::BTreeSet<_> = domains
            .iter()
            .map(|d| d.trim().trim_start_matches('@').to_ascii_lowercase())
            .filter(|d| !d.is_empty())
            .collect();
        *domains = normalized.into_iter().collect();
    }
    Ok(body)
}

fn normalize_optional_update(value: Option<Option<String>>) -> Option<Option<String>> {
    match value {
        None => None,
        Some(None) => Some(None),
        Some(Some(text)) => {
            let trimmed = text.trim();
            Some((!trimmed.is_empty()).then(|| trimmed.to_owned()))
        }
    }
}

pub fn canonicalize_issuer(value: &str) -> String {
    value.trim().trim_end_matches('/').to_owned()
}

pub fn normalize_oidc_scopes(scopes: &str) -> Result<String, AppError> {
    let mut unique = std::collections::HashSet::new();
    let mut ordered = Vec::new();
    for scope in scopes.split_whitespace() {
        if scope
            .bytes()
            .any(|b| !(0x21..=0x7e).contains(&b) || matches!(b, b'"' | b'\\'))
        {
            return Err(AppError::Validation(
                "OIDC scopes contain an invalid character".into(),
            ));
        }
        if unique.insert(scope) {
            ordered.push(scope);
        }
    }
    if unique.contains("openid") {
        Ok(ordered.join(" "))
    } else {
        Err(AppError::Validation(
            "OIDC scopes must include openid".into(),
        ))
    }
}

async fn load_stored<'e, E>(executor: E) -> Result<StoredSettings, AppError>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let mut stored=sqlx::query_as::<_,StoredSettings>("SELECT oidc_enabled,password_login_enabled,issuer_url,public_base_url,client_id,client_secret_encrypted,button_label,scopes,token_auth_method,auto_signup,auto_link_verified_email,oidc_auto_login,allowed_email_domains,required_claim_name,required_claim_value,last_validation_at,last_validation_fingerprint FROM panda.authentication_settings WHERE id=TRUE")
        .fetch_optional(executor).await?.ok_or_else(||AppError::Internal(anyhow::anyhow!("authentication settings row is missing")))?;
    stored.issuer_url = stored.issuer_url.as_deref().map(canonicalize_issuer);
    Ok(stored)
}

fn validation_fingerprint(settings: &EffectiveAuthenticationSettings, age_key: &str) -> String {
    type HmacSha256 = Hmac<sha2::Sha256>;
    // The installation key makes this marker unusable as an offline guessing
    // oracle while still invalidating validation after a secret rotation. It
    // is never returned by the settings API or included in recovery packages.
    let mut mac = HmacSha256::new_from_slice(age_key.as_bytes())
        .expect("HMAC accepts installation keys of any length");
    mac.update(
        format!(
            "panda-oidc-validation-v2|{:?}|{:?}|{:?}|{:?}|{}|{}",
            settings.issuer_url,
            settings.public_base_url,
            settings.client_id,
            settings.client_secret,
            settings.scopes,
            settings.token_auth_method,
        )
        .as_bytes(),
    );
    hex::encode(mac.finalize().into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn enabled_oidc_requires_minimum_credentials() {
        let mut s = AuthenticationSettingsResponse {
            oidc_enabled: EffectiveValue {
                value: true,
                source: SettingSource::Default,
            },
            password_login_enabled: EffectiveValue {
                value: true,
                source: SettingSource::Default,
            },
            issuer_url: EffectiveValue {
                value: None,
                source: SettingSource::Default,
            },
            public_base_url: EffectiveValue {
                value: None,
                source: SettingSource::Default,
            },
            client_id: EffectiveValue {
                value: None,
                source: SettingSource::Default,
            },
            client_secret: SecretStatus {
                configured: false,
                source: SettingSource::Default,
            },
            button_label: EffectiveValue {
                value: "SSO".into(),
                source: SettingSource::Default,
            },
            scopes: EffectiveValue {
                value: "openid".into(),
                source: SettingSource::Default,
            },
            token_auth_method: EffectiveValue {
                value: "auto".into(),
                source: SettingSource::Default,
            },
            auto_signup: EffectiveValue {
                value: false,
                source: SettingSource::Default,
            },
            auto_link_verified_email: EffectiveValue {
                value: false,
                source: SettingSource::Default,
            },
            oidc_auto_login: EffectiveValue {
                value: false,
                source: SettingSource::Default,
            },
            allowed_email_domains: EffectiveValue {
                value: vec![],
                source: SettingSource::Default,
            },
            required_claim_name: EffectiveValue {
                value: None,
                source: SettingSource::Default,
            },
            required_claim_value: EffectiveValue {
                value: None,
                source: SettingSource::Default,
            },
            last_validation_at: None,
            callback_url: None,
        };
        assert!(validate_effective(&s).is_err());
        s.issuer_url.value = Some("https://issuer.example".into());
        s.public_base_url.value = Some("https://panda.example".into());
        s.client_id.value = Some("client".into());
        s.client_secret.configured = true;
        assert!(validate_effective(&s).is_ok());
        s.auto_link_verified_email.value = true;
        assert!(validate_effective(&s).is_ok());
    }

    #[test]
    fn required_claim_settings_must_be_configured_as_a_pair() {
        assert!(validate_required_claim_pair(Some("groups"), None).is_err());
        assert!(validate_required_claim_pair(None, Some("fleet")).is_err());
        assert!(validate_required_claim_pair(Some(" "), Some("fleet")).is_err());
        assert!(validate_required_claim_pair(Some("groups"), Some("fleet")).is_ok());
        assert!(validate_required_claim_pair(None, None).is_ok());
    }

    #[test]
    fn update_normalization_clears_empty_optional_fields() {
        let normalized = normalize_update(AuthenticationSettingsUpdate {
            issuer_url: Some(Some("  ".into())),
            client_id: Some(Some(" client ".into())),
            required_claim_name: Some(Some("".into())),
            required_claim_value: Some(Some(" ".into())),
            scopes: Some("openid  email openid".into()),
            allowed_email_domains: Some(vec![
                " Example.COM ".into(),
                "@example.com".into(),
                "".into(),
            ]),
            ..Default::default()
        })
        .unwrap();

        assert_eq!(normalized.issuer_url, Some(None));
        assert_eq!(normalized.client_id, Some(Some("client".into())));
        assert_eq!(normalized.required_claim_name, Some(None));
        assert_eq!(normalized.required_claim_value, Some(None));
        assert_eq!(normalized.scopes.as_deref(), Some("openid email"));
        assert_eq!(
            normalized.allowed_email_domains,
            Some(vec!["example.com".into()])
        );
    }

    #[test]
    fn oidc_auto_login_environment_override_cannot_be_changed_by_settings_update() {
        let env = OidcEnvOverrides {
            oidc_auto_login: Some(true),
            ..Default::default()
        };
        let body = AuthenticationSettingsUpdate {
            oidc_auto_login: Some(false),
            ..Default::default()
        };
        assert!(matches!(
            reject_environment_owned_update(&body, &env),
            Err(AppError::Conflict(_))
        ));
    }

    #[test]
    fn oidc_scopes_require_openid_and_valid_scope_tokens() {
        assert_eq!(
            normalize_oidc_scopes(" profile  openid email ").unwrap(),
            "profile openid email"
        );
        assert!(normalize_oidc_scopes("email profile").is_err());
        assert!(normalize_oidc_scopes("openid bad\\scope").is_err());
    }

    #[test]
    fn canonicalize_issuer_ignores_documentation_trailing_slashes() {
        assert_eq!(
            canonicalize_issuer(" https://issuer.example/ "),
            "https://issuer.example"
        );
        assert_eq!(
            canonicalize_issuer("https://issuer.example"),
            "https://issuer.example"
        );
    }

    #[test]
    fn oidc_callback_url_requires_a_safe_https_base() {
        assert_eq!(
            oidc_callback_url(Some("https://panda.example/")).unwrap(),
            "https://panda.example/v1/auth/oidc/callback"
        );
        for invalid in [
            None,
            Some("http://panda.example"),
            Some("https://user:password@panda.example"),
            Some("https://panda.example?tenant=fleet"),
            Some("https://panda.example#fragment"),
            Some("not a URL"),
        ] {
            assert!(oidc_callback_url(invalid).is_err(), "accepted {invalid:?}");
        }
    }

    #[test]
    fn validation_fingerprint_changes_when_the_effective_secret_changes() {
        let mut settings = EffectiveAuthenticationSettings {
            oidc_enabled: true,
            password_login_enabled: true,
            issuer_url: Some("https://issuer.example".into()),
            public_base_url: Some("https://panda.example".into()),
            client_id: Some("client".into()),
            client_secret: Some("first-secret".into()),
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
        let first = validation_fingerprint(&settings, "installation-key");
        settings.client_secret = Some("rotated-secret".into());
        assert_ne!(first, validation_fingerprint(&settings, "installation-key"));
        assert_ne!(
            validation_fingerprint(&settings, "installation-key"),
            validation_fingerprint(&settings, "different-installation-key")
        );
    }

    #[tokio::test]
    #[ignore = "requires a migrated PostgreSQL database"]
    async fn linked_super_user_lookup_accepts_a_trailing_slash_issuer() {
        let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&database_url)
            .await
            .expect("database connection");
        let user_id = Uuid::new_v4();
        let email = format!("oidc_issuer_{}@example.com", user_id);
        let issuer = format!("https://issuer-{}.example", Uuid::new_v4());
        sqlx::query(
            "INSERT INTO panda.users(id,email,password_hash,role) VALUES($1,$2,NULL,'super_user')",
        )
        .bind(user_id)
        .bind(&email)
        .execute(&pool)
        .await
        .expect("seed super user");
        sqlx::query(
            "INSERT INTO panda.user_oidc_identities(user_id,issuer,subject,email) VALUES($1,$2,$3,$4)",
        )
        .bind(user_id)
        .bind(&issuer)
        .bind(Uuid::new_v4().to_string())
        .bind(&email)
        .execute(&pool)
        .await
        .expect("seed linked identity");

        assert!(
            has_linked_super_user_for_issuer(&pool, &format!("{issuer}/"))
                .await
                .expect("lookup linked super user")
        );

        sqlx::query("DELETE FROM panda.users WHERE id=$1")
            .bind(user_id)
            .execute(&pool)
            .await
            .expect("delete test user");
    }
}
