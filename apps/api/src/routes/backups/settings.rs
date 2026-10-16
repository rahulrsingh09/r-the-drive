use super::*;

struct SettingsWrite {
    body: UpdateBackupSettingsBody,
    run_at: NaiveTime,
    timezone: Tz,
    encrypted_secret: Option<Vec<u8>>,
}
impl SettingsWrite {
    async fn prepare(
        state: &AppState,
        mut body: UpdateBackupSettingsBody,
    ) -> Result<Self, AppError> {
        let run_at = parse_run_time(&body.run_at)?;
        let timezone = parse_timezone(&body.timezone)?;
        validate_schedule(&body.frequency, body.day_of_week, body.day_of_month)?;
        validate_target(
            body.target_type,
            body.local_enabled,
            body.s3_enabled,
            &body.endpoint,
            &body.bucket,
        )?;
        let days = match body.frequency {
            BackupFrequency::Daily => (None, None),
            BackupFrequency::Weekly => (body.day_of_week, None),
            BackupFrequency::Monthly => (None, body.day_of_month),
        };
        body.day_of_week = days.0;
        body.day_of_month = days.1;
        if body.s3_enabled {
            crate::services::s3_transport::S3Policy::from_config(&state.config)?
                .validate_endpoint(&body.endpoint)
                .await?;
        }
        body.endpoint = body.endpoint.trim().to_owned();
        body.region = optional_text(body.region);
        body.bucket = body.bucket.trim().to_owned();
        body.prefix = normalize_prefix(&body.prefix)?;
        body.access_key = optional_text(body.access_key);
        let encrypted_secret = encrypt_secret(state.age_key.as_str(), body.secret_key.as_deref())?;
        Ok(Self {
            body,
            run_at,
            timezone,
            encrypted_secret,
        })
    }
    async fn persist(
        self,
        state: &AppState,
        user: Uuid,
    ) -> Result<BackupSettingsResponse, AppError> {
        let body = self.body;
        sqlx::query(
        r#"
        INSERT INTO panda.backup_settings (
            id, enabled, frequency, run_at, timezone, day_of_week, day_of_month,
            retention_count, local_enabled, s3_enabled, target_type, endpoint, region, bucket, prefix,
            access_key, secret_key_encrypted, updated_at, updated_by
        )
        VALUES (
            TRUE, $1, $2, $3, $4, $5, $6,
            $7, $8, $9, $10, $11, $12, $13, $14,
            $15, $16, now(), $17
        )
        ON CONFLICT (id) DO UPDATE SET
            enabled = EXCLUDED.enabled,
            frequency = EXCLUDED.frequency,
            run_at = EXCLUDED.run_at,
            timezone = EXCLUDED.timezone,
            day_of_week = EXCLUDED.day_of_week,
            day_of_month = EXCLUDED.day_of_month,
            retention_count = EXCLUDED.retention_count,
            local_enabled = EXCLUDED.local_enabled,
            s3_enabled = EXCLUDED.s3_enabled,
            target_type = EXCLUDED.target_type,
            endpoint = EXCLUDED.endpoint,
            region = EXCLUDED.region,
            bucket = EXCLUDED.bucket,
            prefix = EXCLUDED.prefix,
            access_key = EXCLUDED.access_key,
            secret_key_encrypted = CASE
                WHEN $18 THEN NULL
                WHEN $16 IS NOT NULL THEN $16
                ELSE panda.backup_settings.secret_key_encrypted
            END,
            updated_at = now(),
            updated_by = EXCLUDED.updated_by
        "#,
    )
    .bind(body.enabled)
    .bind(body.frequency.as_str())
    .bind(self.run_at)
    .bind(self.timezone.name())
    .bind(body.day_of_week)
    .bind(body.day_of_month)
    .bind(body.retention_count)
    .bind(body.local_enabled)
    .bind(body.s3_enabled)
    .bind(body.target_type.as_str())
    .bind(body.endpoint)
    .bind(body.region)
    .bind(body.bucket)
    .bind(body.prefix)
    .bind(body.access_key)
    .bind(self.encrypted_secret)
    .bind(user)
    .bind(body.clear_secret_key.unwrap_or(false))
    .execute(&state.pool)
    .await?;

        app_settings::set_app_timezone(&state.pool, self.timezone).await?;
        load_settings(state).await
    }
}
fn optional_text(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}
pub(super) async fn update(
    state: &AppState,
    user: Uuid,
    body: UpdateBackupSettingsBody,
) -> Result<BackupSettingsResponse, AppError> {
    SettingsWrite::prepare(state, body)
        .await?
        .persist(state, user)
        .await
}
