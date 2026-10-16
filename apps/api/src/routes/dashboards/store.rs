use super::*;

const FIELDS: &str = "id, owner_id, slug, name, description, is_default, is_locked, config";

pub(super) enum ReadPlan<'a> {
    Visible(Uuid),
    Identity(Uuid),
    Preferred { owner: Uuid, slug: &'a str },
}

impl ReadPlan<'_> {
    pub(super) async fn rows(self, pool: &sqlx::PgPool) -> Result<Vec<Dashboard>, AppError> {
        let (clause,owner,slug)=match self {
            Self::Visible(owner)=>("WHERE owner_id = $1 OR owner_id IS NULL ORDER BY is_default DESC, name ASC",owner,None),
            Self::Identity(id)=>("WHERE id = $1",id,None),
            Self::Preferred{owner,slug}=>("WHERE slug = $2 AND (owner_id = $1 OR owner_id IS NULL) ORDER BY (owner_id = $1) DESC NULLS LAST LIMIT 1",owner,Some(slug)),
        };
        let sql = format!("SELECT {FIELDS} FROM dashboards {clause}");
        let mut query = sqlx::query_as::<_, Dashboard>(sqlx::AssertSqlSafe(sql)).bind(owner);
        if let Some(slug) = slug {
            query = query.bind(slug);
        }
        Ok(query.fetch_all(pool).await?)
    }

    pub(super) async fn one(self, pool: &sqlx::PgPool) -> Result<Dashboard, AppError> {
        self.rows(pool)
            .await?
            .into_iter()
            .next()
            .ok_or(AppError::NotFound)
    }
}

enum InsertMode {
    Upsert,
    Clone,
}

pub(super) struct NewDashboard {
    row: Dashboard,
    mode: InsertMode,
}

impl NewDashboard {
    pub(super) fn create(owner: Uuid, body: CreateDashboard) -> Self {
        Self::prepare(
            owner,
            body.slug,
            body.name,
            body.description,
            body.config,
            InsertMode::Upsert,
        )
    }

    pub(super) fn clone_from(owner: Uuid, source: Dashboard) -> Self {
        Self::prepare(
            owner,
            format!("{}-copy", source.slug),
            format!("{} (copy)", source.name),
            source.description,
            source.config,
            InsertMode::Clone,
        )
    }

    fn prepare(
        owner: Uuid,
        slug: String,
        name: String,
        description: Option<String>,
        config: Value,
        mode: InsertMode,
    ) -> Self {
        let id = Uuid::new_v4();
        let config = dashboard_config_with_metadata(
            config,
            id,
            Some(owner),
            &slug,
            &name,
            description.as_deref(),
            false,
            false,
        );
        Self {
            row: Dashboard {
                id,
                owner_id: Some(owner),
                slug,
                name,
                description,
                is_default: false,
                is_locked: false,
                config,
            },
            mode,
        }
    }

    pub(super) async fn save(self, pool: &sqlx::PgPool) -> Result<Dashboard, AppError> {
        let conflict=match self.mode {
            InsertMode::Upsert=>"ON CONFLICT (owner_id, slug) DO UPDATE SET name=EXCLUDED.name,description=EXCLUDED.description,config=EXCLUDED.config,is_default=FALSE,is_locked=FALSE,updated_at=NOW()",
            InsertMode::Clone=>"",
        };
        let sql=format!("INSERT INTO dashboards (id,owner_id,slug,name,description,is_default,is_locked,config) VALUES ($1,$2,$3,$4,$5,FALSE,FALSE,$6) {conflict} RETURNING {FIELDS}");
        let row = self.row;
        let result = sqlx::query_as::<_, Dashboard>(sqlx::AssertSqlSafe(sql))
            .bind(row.id)
            .bind(row.owner_id)
            .bind(row.slug)
            .bind(row.name)
            .bind(row.description)
            .bind(row.config)
            .fetch_one(pool)
            .await;
        result.map_err(|error| match self.mode {
            InsertMode::Clone
                if error
                    .as_database_error()
                    .and_then(|db| db.constraint())
                    .is_some() =>
            {
                AppError::Validation("You already have a dashboard with that slug".into())
            }
            _ => AppError::Database(error),
        })
    }
}

pub(super) struct PatchWrite {
    id: Uuid,
    name: Option<String>,
    description: Option<String>,
    config: Option<Value>,
}

impl PatchWrite {
    pub(super) fn prepare(existing: Dashboard, body: UpdateDashboard) -> Self {
        let name = body.name.as_deref().unwrap_or(&existing.name);
        let description = body
            .description
            .as_deref()
            .or(existing.description.as_deref());
        let config = body.config.map(|config| {
            dashboard_config_with_metadata(
                config,
                existing.id,
                existing.owner_id,
                &existing.slug,
                name,
                description,
                existing.is_default,
                existing.is_locked,
            )
        });
        Self {
            id: existing.id,
            name: body.name,
            description: body.description,
            config,
        }
    }

    pub(super) async fn save(
        self,
        pool: &sqlx::PgPool,
        admin: bool,
    ) -> Result<Dashboard, AppError> {
        let sql=format!("UPDATE dashboards SET name=COALESCE($1,name),description=COALESCE($2,description),config=COALESCE($3,config),updated_at=NOW() WHERE id=$4 RETURNING {FIELDS}");
        let query = sqlx::query_as::<_, Dashboard>(sqlx::AssertSqlSafe(sql))
            .bind(self.name)
            .bind(self.description)
            .bind(self.config)
            .bind(self.id);
        if admin {
            query.fetch_optional(pool).await?.ok_or(AppError::NotFound)
        } else {
            Ok(query.fetch_one(pool).await?)
        }
    }
}

pub(super) enum MetadataWrite {
    Lock {
        id: Uuid,
        locked: bool,
    },
    Restore {
        id: Uuid,
        name: String,
        config: Value,
    },
}

impl MetadataWrite {
    pub(super) fn restore(existing: Dashboard) -> Result<Self, AppError> {
        if existing.owner_id.is_some() || !existing.is_default {
            return Err(AppError::Validation(
                "Only system default dashboards can be restored".into(),
            ));
        }
        let config = bundled_default_config(existing.id).ok_or(AppError::NotFound)?;
        let name = config["name"].as_str().unwrap_or(&existing.name).to_owned();
        let config = dashboard_config_with_metadata(
            config,
            existing.id,
            existing.owner_id,
            &existing.slug,
            &name,
            existing.description.as_deref(),
            true,
            true,
        );
        Ok(Self::Restore {
            id: existing.id,
            name,
            config,
        })
    }

    pub(super) async fn save(self, pool: &sqlx::PgPool) -> Result<Dashboard, AppError> {
        let row=match self {
            Self::Lock{id,locked}=>sqlx::query_as::<_,Dashboard>(sqlx::AssertSqlSafe(format!("UPDATE dashboards SET is_locked=$1,updated_at=NOW() WHERE id=$2 RETURNING {FIELDS}"))).bind(locked).bind(id).fetch_optional(pool).await?,
            Self::Restore{id,name,config}=>sqlx::query_as::<_,Dashboard>(sqlx::AssertSqlSafe(format!("UPDATE dashboards SET name=$1,config=$2,is_default=TRUE,is_locked=TRUE,baseline_revision=$3,updated_at=NOW() WHERE id=$4 RETURNING {FIELDS}"))).bind(name).bind(config).bind(BUNDLED_DASHBOARD_BASELINE_REVISION).bind(id).fetch_optional(pool).await?,
        };
        row.ok_or(AppError::NotFound)
    }
}
