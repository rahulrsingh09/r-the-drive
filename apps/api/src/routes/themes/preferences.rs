use super::*;

#[derive(sqlx::FromRow)]
struct StoredPreference {
    theme_mode: String,
    theme_selection_kind: String,
    theme_builtin_id: String,
    theme_custom_id: Option<Uuid>,
    theme_custom_revision: Option<i32>,
    theme_etag_version: i64,
}

enum SelectionEvidence {
    Builtin(String),
    Custom {
        id: Uuid,
        revision: i32,
        base: String,
        definition: Value,
        hash: String,
    },
    Unavailable,
}

impl SelectionEvidence {
    fn project(self) -> Value {
        match self {
            Self::Builtin(id) => json!({"kind":"builtin","themeId":id}),
            Self::Custom {
                id,
                revision,
                base,
                definition,
                hash,
            } => {
                json!({"kind":"custom","themeId":id,"revision":revision,"baseThemeId":base,"definition":definition,"definitionHash":hash})
            }
            Self::Unavailable => {
                json!({"kind":"builtin","themeId":"classic","fallbackReason":"unavailable-custom-theme"})
            }
        }
    }
}

pub(super) async fn load(state: &AppState, owner: Uuid) -> Result<(Value, String), AppError> {
    let stored=sqlx::query_as::<_,StoredPreference>("SELECT theme_mode, theme_selection_kind, theme_builtin_id, theme_custom_id, theme_custom_revision, theme_etag_version FROM panda.user_preferences WHERE user_id = $1").bind(owner).fetch_optional(&state.pool).await?;
    let (mode, version, evidence) = match stored {
        None => (
            "dark".to_owned(),
            1,
            SelectionEvidence::Builtin("classic".to_owned()),
        ),
        Some(stored) => {
            let evidence = if stored.theme_selection_kind == "custom" {
                // The schema requires both custom reference fields for this selection kind.
                let id = stored.theme_custom_id.expect("custom theme ID");
                let revision = stored.theme_custom_revision.expect("custom theme revision");
                let custom=sqlx::query_as::<_,(String,Value,String)>("SELECT t.base_theme_id, r.definition, r.definition_hash FROM panda.user_themes t JOIN panda.user_theme_revisions r ON r.theme_id = t.id AND r.revision = $3 WHERE t.id = $1 AND t.owner_id = $2 AND t.retired_at IS NULL")
                    .bind(id).bind(owner).bind(revision).fetch_optional(&state.pool).await?;
                match custom {
                    Some((base, definition, hash)) => SelectionEvidence::Custom {
                        id,
                        revision,
                        base,
                        definition,
                        hash,
                    },
                    None => SelectionEvidence::Unavailable,
                }
            } else {
                SelectionEvidence::Builtin(stored.theme_builtin_id)
            };
            (stored.theme_mode, stored.theme_etag_version, evidence)
        }
    };
    Ok((
        json!({"schemaVersion":2,"mode":mode,"selection":evidence.project()}),
        preferences_etag(version),
    ))
}

enum PreferenceWrite {
    Builtin(String),
    Published {
        id: Uuid,
        revision: i32,
        base: String,
    },
}

impl PreferenceWrite {
    async fn resolve(
        tx: &mut Transaction<'_, Postgres>,
        owner: Uuid,
        selection: ThemeSelection,
    ) -> Result<Self, AppError> {
        match selection {
            ThemeSelection::Builtin { theme_id } => {
                validate_builtin_id(&theme_id)?;
                Ok(Self::Builtin(theme_id))
            }
            ThemeSelection::Custom { theme_id, revision } => {
                let base=sqlx::query_scalar("SELECT t.base_theme_id FROM panda.user_themes t JOIN panda.user_theme_revisions r ON r.theme_id = t.id AND r.revision = $3 JOIN panda.user_theme_publications p ON p.theme_id = r.theme_id AND p.revision = r.revision WHERE t.id = $1 AND t.owner_id = $2 AND t.retired_at IS NULL")
                    .bind(theme_id).bind(owner).bind(revision).fetch_optional(&mut **tx).await?
                    .ok_or_else(||AppError::Validation("custom theme revision is unavailable or unpublished".into()))?;
                Ok(Self::Published {
                    id: theme_id,
                    revision,
                    base,
                })
            }
        }
    }

    async fn apply(
        self,
        tx: &mut Transaction<'_, Postgres>,
        owner: Uuid,
        mode: &str,
    ) -> Result<(), AppError> {
        match self {
            Self::Builtin(id) => {
                sqlx::query("UPDATE panda.user_preferences SET theme_mode = $2, theme_palette = $3, theme_selection_kind = 'builtin', theme_builtin_id = $3, theme_custom_id = NULL, theme_custom_revision = NULL, theme_etag_version = theme_etag_version + 1, updated_at = now() WHERE user_id = $1")
                    .bind(owner).bind(mode).bind(id).execute(&mut **tx).await?;
            }
            Self::Published { id, revision, base } => {
                apply_custom_selection_tx(tx, owner, Some(mode), id, revision, &base).await?;
            }
        }
        Ok(())
    }
}

pub(super) async fn update(
    state: &AppState,
    owner: Uuid,
    headers: &HeaderMap,
    body: UpdateThemePreferencesBody,
) -> Result<Response, AppError> {
    if body.schema_version != 2 {
        return Err(AppError::Validation(
            "unsupported theme schema version".into(),
        ));
    }
    validate_mode(&body.mode)?;
    let mut tx = state.pool.begin().await?;
    ensure_preferences_row(&mut tx, owner).await?;
    let version = sqlx::query_scalar(
        "SELECT theme_etag_version FROM panda.user_preferences WHERE user_id = $1 FOR UPDATE",
    )
    .bind(owner)
    .fetch_one(&mut *tx)
    .await?;
    require_etag(headers, &preferences_etag(version))?;
    let write = PreferenceWrite::resolve(&mut tx, owner, body.selection).await?;
    write.apply(&mut tx, owner, &body.mode).await?;
    tx.commit().await?;
    let (payload, etag) = load(state, owner).await?;
    etag_json(&etag, payload)
}
