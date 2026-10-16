use super::*;

pub(super) enum InstallationChange {
    Timezone(Tz),
    UpdateCheck {
        enabled: bool,
        frequency: UpdateCheckFrequency,
    },
}

pub(super) enum SavedSetting {
    Timezone(TimezoneResponse),
    UpdateCheck(UpdateCheckSettingsResponse),
}

impl InstallationChange {
    pub(super) fn timezone(body: UpdateTimezoneBody) -> Result<Self, AppError> {
        body.timezone
            .trim()
            .parse::<Tz>()
            .map(Self::Timezone)
            .map_err(|_| AppError::Validation("timezone must be a valid IANA timezone".into()))
    }

    fn audit(&self) -> (&'static str, &'static str) {
        match self {
            Self::Timezone(_) => ("application_timezone_updated", "system_config:app_timezone"),
            Self::UpdateCheck { .. } => (
                "github_release_check_settings_updated",
                "system_config:github_release_check",
            ),
        }
    }

    pub(super) async fn commit(
        self,
        state: &AppState,
        actor: Uuid,
        headers: &HeaderMap,
    ) -> Result<SavedSetting, AppError> {
        let (event, target) = self.audit();
        let mut tx = state.pool.begin().await?;
        let stored = match self {
            Self::Timezone(timezone) => {
                app_settings::set_app_timezone_tx(&mut tx, timezone).await?;
                PersistedSetting::Timezone(timezone)
            }
            Self::UpdateCheck { enabled, frequency } => {
                let (enabled, frequency): (bool,String) = sqlx::query_as(
                    "INSERT INTO panda.update_check_settings (id, enabled, frequency, updated_by) \
                     VALUES (TRUE, $1, $2, $3) \
                     ON CONFLICT (id) DO UPDATE SET enabled = EXCLUDED.enabled, \
                         frequency = EXCLUDED.frequency, updated_by = EXCLUDED.updated_by, updated_at = now() \
                     RETURNING enabled, frequency",
                ).bind(enabled).bind(frequency.as_str()).bind(actor).fetch_one(&mut *tx).await?;
                PersistedSetting::UpdateCheck(enabled, frequency)
            }
        };
        SecurityAuditEvent::success(event, Some(actor))
            .target(target)
            .request_id_from_headers(headers)
            .record_tx(&mut tx)
            .await?;
        tx.commit().await?;
        stored.project()
    }
}

enum PersistedSetting {
    Timezone(Tz),
    UpdateCheck(bool, String),
}

impl PersistedSetting {
    fn project(self) -> Result<SavedSetting, AppError> {
        Ok(match self {
            Self::Timezone(timezone) => SavedSetting::Timezone(TimezoneResponse {
                timezone: timezone.name().to_owned(),
            }),
            Self::UpdateCheck(enabled, frequency) => {
                SavedSetting::UpdateCheck(UpdateCheckSettingsResponse {
                    enabled,
                    frequency: UpdateCheckFrequency::parse(&frequency)?,
                })
            }
        })
    }
}
