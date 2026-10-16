use super::*;

pub(super) enum Action<'a> {
    Save(&'a Value),
    Publish { revision: i32, apply: bool },
    Rollback(i32),
    Retire,
}

impl Action<'_> {
    fn retired_error(&self) -> &'static str {
        match self {
            Self::Save(_) => "retired themes cannot be changed",
            Self::Publish { .. } => "retired themes cannot be published",
            Self::Rollback(_) => "retired themes cannot be selected",
            Self::Retire => "theme is already retired",
        }
    }
}

struct LockedTheme<'a> {
    transaction: Transaction<'a, Postgres>,
    id: Uuid,
    owner: Uuid,
    base: String,
}

enum RevisionEvidence<'a> {
    Draft {
        definition: &'a Value,
        revision: i32,
        hash: String,
    },
    Existing {
        revision: i32,
        publish: bool,
        apply: bool,
    },
    Retirement,
}

pub(super) enum Completed {
    Draft {
        version: i64,
        revision: i32,
        hash: String,
    },
    Publication {
        version: i64,
        revision: i32,
        applied: bool,
    },
    Selection,
    Retirement {
        version: i64,
    },
}

impl LockedTheme<'_> {
    async fn evidence<'a>(&mut self, action: Action<'a>) -> Result<RevisionEvidence<'a>, AppError> {
        Ok(match action {
            Action::Save(definition) => {
                let bytes = validate_theme_definition(&self.base, definition)?;
                let revision=sqlx::query_scalar("SELECT COALESCE(max(revision), 0) + 1 FROM panda.user_theme_revisions WHERE theme_id = $1")
                    .bind(self.id).fetch_one(&mut *self.transaction).await?;
                RevisionEvidence::Draft {
                    definition,
                    revision,
                    hash: definition_hash(&bytes),
                }
            }
            Action::Publish { revision, apply } => {
                let exists:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM panda.user_theme_revisions WHERE theme_id = $1 AND revision = $2)")
                    .bind(self.id).bind(revision).fetch_one(&mut *self.transaction).await?;
                if !exists {
                    return Err(AppError::NotFound);
                }
                RevisionEvidence::Existing {
                    revision,
                    publish: true,
                    apply,
                }
            }
            Action::Rollback(revision) => {
                let published:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM panda.user_theme_publications WHERE theme_id = $1 AND revision = $2)")
                    .bind(self.id).bind(revision).fetch_one(&mut *self.transaction).await?;
                if !published {
                    return Err(AppError::Validation(
                        "only published revisions can be selected".into(),
                    ));
                }
                RevisionEvidence::Existing {
                    revision,
                    publish: false,
                    apply: true,
                }
            }
            Action::Retire => RevisionEvidence::Retirement,
        })
    }

    async fn write(
        mut self,
        evidence: RevisionEvidence<'_>,
        headers: &HeaderMap,
    ) -> Result<Completed, AppError> {
        let completed = match evidence {
            RevisionEvidence::Draft {
                definition,
                revision,
                hash,
            } => {
                sqlx::query("INSERT INTO panda.user_theme_revisions (theme_id, revision, definition, definition_hash) VALUES ($1, $2, $3, $4)")
                    .bind(self.id).bind(revision).bind(definition).bind(&hash).execute(&mut *self.transaction).await?;
                let version=sqlx::query_scalar("UPDATE panda.user_themes SET etag_version = etag_version + 1, updated_at = now() WHERE id = $1 RETURNING etag_version")
                    .bind(self.id).fetch_one(&mut *self.transaction).await?;
                Completed::Draft {
                    version,
                    revision,
                    hash,
                }
            }
            RevisionEvidence::Existing {
                revision,
                publish,
                apply,
            } => {
                let version = if publish {
                    sqlx::query("INSERT INTO panda.user_theme_publications (theme_id, revision) VALUES ($1, $2) ON CONFLICT DO NOTHING")
                        .bind(self.id).bind(revision).execute(&mut *self.transaction).await?;
                    Some(sqlx::query_scalar("UPDATE panda.user_themes SET published_revision = $2, etag_version = etag_version + 1, updated_at = now() WHERE id = $1 RETURNING etag_version")
                        .bind(self.id).bind(revision).fetch_one(&mut *self.transaction).await?)
                } else {
                    None
                };
                if apply {
                    require_preference_etag(&mut self.transaction, self.owner, headers).await?;
                    apply_custom_selection_tx(
                        &mut self.transaction,
                        self.owner,
                        None,
                        self.id,
                        revision,
                        &self.base,
                    )
                    .await?;
                }
                match version {
                    Some(version) => Completed::Publication {
                        version,
                        revision,
                        applied: apply,
                    },
                    None => Completed::Selection,
                }
            }
            RevisionEvidence::Retirement => {
                let version=sqlx::query_scalar("UPDATE panda.user_themes SET retired_at = now(), etag_version = etag_version + 1, updated_at = now() WHERE id = $1 RETURNING etag_version")
                    .bind(self.id).fetch_one(&mut *self.transaction).await?;
                sqlx::query("UPDATE panda.user_preferences SET theme_palette = $3, theme_selection_kind = 'builtin', theme_builtin_id = $3, theme_custom_id = NULL, theme_custom_revision = NULL, theme_etag_version = theme_etag_version + 1, updated_at = now() WHERE user_id = $1 AND theme_custom_id = $2")
                    .bind(self.owner).bind(self.id).bind(self.base).execute(&mut *self.transaction).await?;
                Completed::Retirement { version }
            }
        };
        self.transaction.commit().await?;
        Ok(completed)
    }
}

pub(super) async fn execute(
    state: &AppState,
    owner: Uuid,
    id: Uuid,
    headers: &HeaderMap,
    action: Action<'_>,
) -> Result<Completed, AppError> {
    let mut transaction = state.pool.begin().await?;
    let (base, version, retired_at) = lock_owned_theme(&mut transaction, owner, id).await?;
    require_etag(headers, &theme_etag(id, version))?;
    if retired_at.is_some() {
        return Err(AppError::Conflict(action.retired_error().into()));
    }
    let mut locked = LockedTheme {
        transaction,
        id,
        owner,
        base,
    };
    let evidence = locked.evidence(action).await?;
    locked.write(evidence, headers).await
}

impl Completed {
    pub(super) async fn response(
        self,
        state: &AppState,
        owner: Uuid,
        id: Uuid,
    ) -> Result<Response, AppError> {
        let (etag, payload) = match self {
            Self::Draft {
                version,
                revision,
                hash,
            } => {
                let etag = theme_etag(id, version);
                let payload =
                    json!({"themeId":id,"revision":revision,"definitionHash":hash,"etag":etag});
                (etag, payload)
            }
            Self::Publication {
                version,
                revision,
                applied,
            } => {
                let etag = theme_etag(id, version);
                let payload = json!({"themeId":id,"publishedRevision":revision,"applied":applied,"etag":etag});
                (etag, payload)
            }
            Self::Retirement { version } => {
                let etag = theme_etag(id, version);
                let payload = json!({"themeId":id,"retired":true,"etag":etag});
                (etag, payload)
            }
            Self::Selection => {
                let (payload, etag) = theme_preferences_payload(state, owner).await?;
                (etag, payload)
            }
        };
        etag_json(&etag, payload)
    }
}

pub(super) struct NewTheme {
    name: String,
    base: String,
}

impl NewTheme {
    pub(super) fn prepare(body: CreateThemeBody) -> Result<Self, AppError> {
        validate_builtin_id(&body.base_theme_id)?;
        Ok(Self {
            name: normalize_name(&body.name)?,
            base: body.base_theme_id,
        })
    }

    pub(super) async fn create(self, state: &AppState, owner: Uuid) -> Result<Response, AppError> {
        let mut tx = state.pool.begin().await?;
        sqlx::query("SELECT id FROM panda.users WHERE id = $1 FOR UPDATE")
            .bind(owner)
            .fetch_one(&mut *tx)
            .await?;
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM panda.user_themes WHERE owner_id = $1 AND retired_at IS NULL",
        )
        .bind(owner)
        .fetch_one(&mut *tx)
        .await?;
        if count >= MAX_CUSTOM_THEMES {
            return Err(AppError::Validation(
                "custom theme limit of 20 reached".into(),
            ));
        }
        let (id,version):(Uuid,i64)=sqlx::query_as("INSERT INTO panda.user_themes (owner_id, name, base_theme_id) VALUES ($1, $2, $3) RETURNING id, etag_version")
            .bind(owner).bind(self.name).bind(self.base).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        let etag = theme_etag(id, version);
        etag_json(&etag, json!({"themeId":id,"etag":etag}))
    }
}
