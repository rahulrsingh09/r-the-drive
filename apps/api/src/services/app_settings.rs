use chrono_tz::Tz;
use sqlx::{PgPool, Postgres, Transaction};

use crate::errors::AppError;

pub const APP_TIMEZONE_KEY: &str = "app_timezone";

pub async fn load_app_timezone_name(pool: &PgPool) -> Result<String, AppError> {
    for (statement, discard_blank) in [
        ("SELECT value FROM panda.system_config WHERE key=$1", true),
        (
            "SELECT timezone FROM panda.backup_settings WHERE id=TRUE",
            false,
        ),
    ] {
        let mut query = sqlx::query_scalar::<_, String>(statement);
        if discard_blank {
            query = query.bind(APP_TIMEZONE_KEY);
        }
        if let Some(name) = query
            .fetch_optional(pool)
            .await?
            .filter(|name| !discard_blank || !name.trim().is_empty())
        {
            return name.parse::<Tz>().map(|_| name).map_err(|_| {
                AppError::Validation("timezone must be a valid IANA timezone".into())
            });
        }
    }
    Ok("UTC".to_owned())
}

pub async fn load_app_timezone(pool: &PgPool) -> Result<Tz, AppError> {
    load_app_timezone_name(pool)
        .await?
        .parse::<Tz>()
        .map_err(|_| AppError::Validation("timezone must be a valid IANA timezone".into()))
}

pub async fn set_app_timezone(pool: &PgPool, timezone: Tz) -> Result<(), AppError> {
    let mut transaction = pool.begin().await?;
    set_app_timezone_tx(&mut transaction, timezone).await?;
    transaction.commit().await?;
    Ok(())
}

pub async fn set_app_timezone_tx(
    transaction: &mut Transaction<'_, Postgres>,
    timezone: Tz,
) -> Result<(), AppError> {
    sqlx::query(
        "WITH configured AS (
            INSERT INTO panda.system_config(key,value) VALUES($1,$2)
            ON CONFLICT(key) DO UPDATE SET value=EXCLUDED.value RETURNING value
         ) UPDATE panda.backup_settings SET timezone=(SELECT value FROM configured), updated_at=now() WHERE id=TRUE"
    ).bind(APP_TIMEZONE_KEY).bind(timezone.name()).execute(&mut **transaction).await?;
    Ok(())
}
