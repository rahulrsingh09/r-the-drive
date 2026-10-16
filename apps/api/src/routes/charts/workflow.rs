use super::*;

const COLUMNS:&str="id,owner_id,slug,name,description,is_default,is_locked,is_enabled,config,baseline_revision,created_at,updated_at";

pub(super) enum CatalogScope<'a> {
    Manager,
    Effective(Option<&'a str>),
}

impl CatalogScope<'_> {
    pub(super) fn project(
        self,
        rows: Vec<ChartRecord>,
        owner: Uuid,
        admin: bool,
    ) -> Vec<ChartManagerEntry> {
        let entries = merge_entries(rows, owner, admin);
        match self {
            Self::Manager => entries,
            Self::Effective(dashboard) => entries
                .into_iter()
                .filter(|entry| {
                    if !entry.effective.is_enabled {
                        return false;
                    }
                    match dashboard {
                        None => true,
                        Some(slug) => entry
                            .effective
                            .config
                            .get("placements")
                            .and_then(Value::as_array)
                            .is_some_and(|placements| {
                                placements.iter().any(|placement| {
                                    placement.get("dashboardSlug").and_then(Value::as_str)
                                        == Some(slug)
                                })
                            }),
                    }
                })
                .collect(),
        }
    }
}

pub(super) struct NewChart {
    owner: Uuid,
    slug: String,
    name: String,
    description: Option<String>,
    enabled: Option<bool>,
    config: Value,
}

impl NewChart {
    pub(super) fn from_input(owner: Uuid, body: ChartInput) -> Result<Self, AppError> {
        validate_slug(&body.slug)?;
        validate_name(&body.name)?;
        validate_config(&body.slug, &body.config)?;
        Ok(Self {
            owner,
            slug: body.slug,
            name: body.name,
            description: body.description,
            enabled: body.is_enabled,
            config: body.config,
        })
    }
    pub(super) fn from_source(
        owner: Uuid,
        source: ChartRecord,
        body: CloneInput,
    ) -> Result<Self, AppError> {
        let slug = body
            .slug
            .unwrap_or_else(|| format!("{}-copy-{}", source.slug, Uuid::new_v4().simple()));
        validate_slug(&slug)?;
        let name = body
            .name
            .unwrap_or_else(|| format!("{} (copy)", source.name));
        validate_name(&name)?;
        Ok(Self {
            owner,
            slug,
            name,
            description: source.description,
            enabled: Some(source.is_enabled),
            config: source.config,
        })
    }
    pub(super) async fn insert(self, state: &AppState) -> Result<ChartRecord, AppError> {
        sqlx::query_as::<_,ChartRecord>(sqlx::AssertSqlSafe(format!("INSERT INTO panda.charts (owner_id,slug,name,description,is_default,is_locked,is_enabled,config) VALUES ($1,$2,$3,$4,FALSE,FALSE,COALESCE($5,TRUE),$6) RETURNING {COLUMNS}")))
            .bind(self.owner).bind(self.slug).bind(self.name).bind(self.description).bind(self.enabled).bind(self.config).fetch_one(&state.pool).await.map_err(map_conflict)
    }
}

pub(super) enum Change {
    Patch {
        id: Uuid,
        input: ChartPatch,
    },
    Config {
        id: Uuid,
        config: Value,
    },
    Lock {
        id: Uuid,
        locked: bool,
    },
    Baseline {
        id: Uuid,
        name: String,
        description: Option<String>,
        enabled: bool,
        config: Value,
    },
}

impl Change {
    pub(super) fn patch(existing: &ChartRecord, input: ChartPatch) -> Result<Self, AppError> {
        if let Some(config) = &input.config {
            validate_config(&existing.slug, config)?;
        }
        Ok(Self::Patch {
            id: existing.id,
            input,
        })
    }
    pub(super) fn placements(
        existing: ChartRecord,
        body: PlacementPatch,
    ) -> Result<Self, AppError> {
        if body.placements.len() > 20
            || body
                .placements
                .iter()
                .any(|placement| !validate_slug_value(&placement.dashboard_slug))
        {
            return Err(AppError::Validation(
                "placements must contain at most 20 valid dashboard slugs".into(),
            ));
        }
        let mut config = existing.config;
        let object = config
            .as_object_mut()
            .ok_or_else(|| AppError::Validation("chart config must be an object".into()))?;
        let placements = serde_json::to_value(body.placements)
            .map_err(|_| AppError::Validation("placements could not be encoded".into()))?;
        object.insert("placements".into(), placements);
        validate_config(&existing.slug, &config)?;
        Ok(Self::Config {
            id: existing.id,
            config,
        })
    }
    pub(super) fn baseline(row: ChartRecord) -> Result<Self, AppError> {
        let bundled = bundled_default(&row.slug).ok_or(AppError::NotFound)?;
        let name = bundled["name"].as_str().unwrap_or(&row.name).to_owned();
        let description = bundled["description"].as_str().map(str::to_owned);
        let enabled = bundled["enabled"].as_bool().unwrap_or(true);
        let config = bundled["definition"].clone();
        validate_config(&row.slug, &config)?;
        Ok(Self::Baseline {
            id: row.id,
            name,
            description,
            enabled,
            config,
        })
    }
    pub(super) async fn persist(self, state: &AppState) -> Result<ChartRecord, AppError> {
        let row=match self {
            Self::Patch{id,input}=>sqlx::query_as::<_,ChartRecord>(sqlx::AssertSqlSafe(format!("UPDATE panda.charts SET name=COALESCE($1,name),description=COALESCE($2,description),is_enabled=COALESCE($3,is_enabled),config=COALESCE($4,config),updated_at=NOW() WHERE id=$5 RETURNING {COLUMNS}")))
                .bind(input.name).bind(input.description).bind(input.is_enabled).bind(input.config).bind(id).fetch_optional(&state.pool).await?,
            Self::Config{id,config}=>sqlx::query_as::<_,ChartRecord>(sqlx::AssertSqlSafe(format!("UPDATE panda.charts SET config=$1,updated_at=NOW() WHERE id=$2 RETURNING {COLUMNS}")))
                .bind(config).bind(id).fetch_optional(&state.pool).await?,
            Self::Lock{id,locked}=>sqlx::query_as::<_,ChartRecord>(sqlx::AssertSqlSafe(format!("UPDATE panda.charts SET is_locked=$1,updated_at=NOW() WHERE id=$2 RETURNING {COLUMNS}")))
                .bind(locked).bind(id).fetch_optional(&state.pool).await?,
            Self::Baseline{id,name,description,enabled,config}=>Some(sqlx::query_as::<_,ChartRecord>(sqlx::AssertSqlSafe(format!("UPDATE panda.charts SET name=$1,description=$2,is_enabled=$3,config=$4,baseline_revision=$6,updated_at=NOW() WHERE id=$5 RETURNING {COLUMNS}")))
                .bind(name).bind(description).bind(enabled).bind(config).bind(id).bind(BUNDLED_CHART_BASELINE_REVISION).fetch_one(&state.pool).await?),
        };
        row.ok_or(AppError::NotFound)
    }
}

pub(super) struct OverrideRemoval {
    personal: Uuid,
    system: ChartRecord,
}

impl OverrideRemoval {
    pub(super) async fn resolve(state: &AppState, personal: ChartRecord) -> Result<Self, AppError> {
        let system = sqlx::query_as::<_, ChartRecord>(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM panda.charts WHERE owner_id IS NULL AND slug=$1"
        )))
        .bind(&personal.slug)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(AppError::NotFound)?;
        Ok(Self {
            personal: personal.id,
            system,
        })
    }
    pub(super) async fn remove(self, state: &AppState) -> Result<ChartRecord, AppError> {
        sqlx::query("DELETE FROM panda.charts WHERE id=$1")
            .bind(self.personal)
            .execute(&state.pool)
            .await?;
        Ok(self.system)
    }
}
