use super::*;
use sqlx::PgPool;

pub(super) enum ProfileSelection {
    All(Uuid),
    One { owner: Uuid, id: Uuid },
}

impl ProfileSelection {
    pub(super) async fn load(self, pool: &PgPool) -> Result<Vec<CostProfile>, AppError> {
        match self {
            Self::All(owner) => Ok(sqlx::query_as::<_, CostProfile>(LIST_SQL)
                .bind(owner)
                .fetch_all(pool)
                .await?),
            Self::One { owner, id } => Ok(sqlx::query_as::<_, CostProfile>(GET_SQL)
                .bind(id)
                .bind(owner)
                .fetch_optional(pool)
                .await?
                .into_iter()
                .collect()),
        }
    }
    pub(super) async fn require(self, pool: &PgPool) -> Result<CostProfile, AppError> {
        self.load(pool)
            .await?
            .into_iter()
            .next()
            .ok_or(AppError::NotFound)
    }
    pub(super) async fn delete(self, pool: &PgPool) -> Result<(), AppError> {
        let Self::One { owner, id } = self else {
            unreachable!("deletion requires one profile")
        };
        let result = sqlx::query("DELETE FROM panda.cost_profiles WHERE id = $1 AND user_id = $2")
            .bind(id)
            .bind(owner)
            .execute(pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(AppError::NotFound);
        }
        Ok(())
    }
}

struct ProfileValues {
    name: String,
    billing_type: String,
    rate: f64,
    session_fee: f64,
    currency: String,
    timezone: Option<String>,
    tou_periods: serde_json::Value,
    effective_from: Option<chrono::NaiveDate>,
    effective_to: Option<chrono::NaiveDate>,
}

impl From<CostProfile> for ProfileValues {
    fn from(row: CostProfile) -> Self {
        Self {
            name: row.name,
            billing_type: row.billing_type,
            rate: row.rate,
            session_fee: row.session_fee,
            currency: row.currency,
            timezone: row.timezone,
            tou_periods: row.tou_periods,
            effective_from: row.effective_from,
            effective_to: row.effective_to,
        }
    }
}

impl ProfileValues {
    fn overlay(mut self, patch: UpdateProfileBody) -> Self {
        if let Some(value) = patch.name {
            self.name = value;
        }
        if let Some(value) = patch.billing_type {
            self.billing_type = value;
        }
        if let Some(value) = patch.rate {
            self.rate = value;
        }
        if let Some(value) = patch.session_fee {
            self.session_fee = value;
        }
        if let Some(value) = patch.currency {
            self.currency = value;
        }
        if let Some(value) = patch.timezone {
            self.timezone = Some(value);
        }
        if let Some(value) = patch.tou_periods {
            self.tou_periods = value;
        }
        if let Some(value) = patch.effective_from {
            self.effective_from = Some(value);
        }
        if let Some(value) = patch.effective_to {
            self.effective_to = Some(value);
        }
        self
    }
    fn validate(self) -> Result<ValidatedProfile, AppError> {
        validate_profile_details(
            &self.billing_type,
            self.rate,
            self.timezone.as_deref(),
            &self.tou_periods,
        )?;
        Ok(ValidatedProfile(self))
    }
}

struct ValidatedProfile(ProfileValues);

pub(super) struct ProfileMutation {
    owner: Uuid,
    target: Option<Uuid>,
    values: ValidatedProfile,
}

impl ProfileMutation {
    pub(super) fn create(owner: Uuid, body: CreateProfileBody) -> Result<Self, AppError> {
        let values = ProfileValues {
            name: body.name,
            billing_type: body.billing_type,
            rate: body.rate,
            session_fee: body.session_fee.unwrap_or(0.0),
            currency: body.currency.unwrap_or_else(|| "USD".into()),
            timezone: body.timezone,
            tou_periods: normalize_tou_periods(body.tou_periods)?,
            effective_from: body.effective_from,
            effective_to: body.effective_to,
        }
        .validate()?;
        Ok(Self {
            owner,
            target: None,
            values,
        })
    }
    pub(super) async fn update(
        pool: &PgPool,
        owner: Uuid,
        id: Uuid,
        patch: UpdateProfileBody,
    ) -> Result<Self, AppError> {
        let current = ProfileSelection::One { owner, id }.require(pool).await?;
        let values = ProfileValues::from(current).overlay(patch).validate()?;
        Ok(Self {
            owner,
            target: Some(id),
            values,
        })
    }
    pub(super) async fn persist(self, pool: &PgPool) -> Result<CostProfile, AppError> {
        let ProfileValues {
            name,
            billing_type,
            rate,
            session_fee,
            currency,
            timezone,
            tou_periods,
            effective_from,
            effective_to,
        } = self.values.0;
        match self.target {
            None => Ok(sqlx::query_as::<_, CostProfile>(CREATE_SQL)
                .bind(self.owner)
                .bind(name)
                .bind(billing_type)
                .bind(rate)
                .bind(session_fee)
                .bind(currency)
                .bind(timezone)
                .bind(tou_periods)
                .bind(effective_from)
                .bind(effective_to)
                .fetch_one(pool)
                .await?),
            Some(id) => sqlx::query_as::<_, CostProfile>(UPDATE_SQL)
                .bind(id)
                .bind(self.owner)
                .bind(name)
                .bind(billing_type)
                .bind(rate)
                .bind(session_fee)
                .bind(currency)
                .bind(timezone)
                .bind(tou_periods)
                .bind(effective_from)
                .bind(effective_to)
                .fetch_optional(pool)
                .await?
                .ok_or(AppError::NotFound),
        }
    }
}

const LIST_SQL: &str = r#"SELECT id, user_id, name, billing_type, rate, session_fee, currency,
              timezone, tou_periods,
                  effective_from, effective_to, created_at
           FROM panda.cost_profiles
           WHERE user_id = $1
           ORDER BY name"#;

const GET_SQL: &str = r#"SELECT id, user_id, name, billing_type, rate, session_fee, currency,
              timezone, tou_periods,
                  effective_from, effective_to, created_at
           FROM panda.cost_profiles
           WHERE id = $1 AND user_id = $2"#;

const CREATE_SQL: &str = r#"INSERT INTO panda.cost_profiles
           (user_id, name, billing_type, rate, session_fee, currency,
            timezone, tou_periods, effective_from, effective_to)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
           RETURNING id, user_id, name, billing_type, rate, session_fee, currency,
                     timezone, tou_periods,
                     effective_from, effective_to, created_at"#;

const UPDATE_SQL: &str = r#"UPDATE panda.cost_profiles SET
           name           = $3,
           billing_type   = $4,
           rate           = $5,
           session_fee    = $6,
           currency       = $7,
           timezone       = $8,
           tou_periods    = $9,
           effective_from = $10,
           effective_to   = $11
           WHERE id = $1 AND user_id = $2
           RETURNING id, user_id, name, billing_type, rate, session_fee, currency,
                     timezone, tou_periods,
                     effective_from, effective_to, created_at"#;
