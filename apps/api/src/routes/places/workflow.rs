use super::*;
use sqlx::PgPool;

enum PlaceTarget {
    New,
    Existing {
        id: Uuid,
        previous_profile: Option<Uuid>,
    },
}

pub(super) struct PlaceMutation {
    owner: Uuid,
    body: PlaceBody,
    target: PlaceTarget,
}

pub(super) struct CommittedPlace {
    id: Uuid,
    recompute: Vec<Uuid>,
}

impl PlaceMutation {
    pub(super) fn create(owner: Uuid, body: PlaceBody) -> Result<Self, AppError> {
        validate_place_body(&body)?;
        Ok(Self {
            owner,
            body,
            target: PlaceTarget::New,
        })
    }
    pub(super) async fn update(
        pool: &PgPool,
        owner: Uuid,
        id: Uuid,
        body: PlaceBody,
    ) -> Result<Self, AppError> {
        validate_place_body(&body)?;
        let existing = sqlx::query!(
            "SELECT cost_profile_id FROM panda.geofences WHERE id = $1 AND user_id = $2",
            id,
            owner
        )
        .fetch_optional(pool)
        .await?
        .ok_or(AppError::NotFound)?;

        Ok(Self {
            owner,
            body,
            target: PlaceTarget::Existing {
                id,
                previous_profile: existing.cost_profile_id,
            },
        })
    }
    pub(super) async fn commit(self, pool: &PgPool) -> Result<CommittedPlace, AppError> {
        let previous_profile = match self.target {
            PlaceTarget::New => None,
            PlaceTarget::Existing {
                previous_profile, ..
            } => previous_profile,
        };
        let mut tx = pool.begin().await?;
        let address_id = upsert_address(&mut tx, &self.body.address).await?;
        let cost_profile_id = upsert_cost_profile(
            &mut tx,
            self.owner,
            previous_profile,
            self.body.charging.as_ref(),
            &self.body.name,
        )
        .await?;
        let result = match self.target {
            PlaceTarget::New => {
                let id = sqlx::query_scalar!(
        r#"INSERT INTO panda.geofences
           (user_id, name, latitude, longitude, radius_m, address_id, is_home, is_work, cost_profile_id)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
           RETURNING id"#,
        self.owner,
        self.body.name,
        self.body.address.latitude,
        self.body.address.longitude,
        self.body.radius_m.unwrap_or(75.0),
        address_id,
        self.body.is_home.unwrap_or(false),
        self.body.is_work.unwrap_or(false),
        cost_profile_id,
    )
    .fetch_one(&mut *tx)
    .await?;

                CommittedPlace {
                    id,
                    recompute: Vec::new(),
                }
            }
            PlaceTarget::Existing { id, .. } => {
                let updated = sqlx::query!(
                    r#"UPDATE panda.geofences
           SET name = $3,
               latitude = $4,
               longitude = $5,
               radius_m = $6,
               address_id = $7,
               is_home = $8,
               is_work = $9,
               cost_profile_id = $10,
               updated_at = now()
           WHERE id = $1 AND user_id = $2"#,
                    id,
                    self.owner,
                    self.body.name,
                    self.body.address.latitude,
                    self.body.address.longitude,
                    self.body.radius_m.unwrap_or(75.0),
                    address_id,
                    self.body.is_home.unwrap_or(false),
                    self.body.is_work.unwrap_or(false),
                    cost_profile_id,
                )
                .execute(&mut *tx)
                .await?;

                if updated.rows_affected() == 0 {
                    return Err(AppError::NotFound);
                }

                let recompute = sqlx::query_scalar::<_, Uuid>(SYNC_LINKED_SESSION_COST_PROFILE_SQL)
                    .bind(id)
                    .bind(cost_profile_id)
                    .fetch_all(&mut *tx)
                    .await?;
                CommittedPlace { id, recompute }
            }
        };
        tx.commit().await?;
        Ok(result)
    }
}

impl CommittedPlace {
    pub(super) async fn finish(self, pool: &PgPool) -> Result<Uuid, AppError> {
        // Repricing deliberately follows commit, so an error retains the place edit.
        for id in self.recompute {
            recompute_charge_session_cost(pool, id).await?;
        }
        Ok(self.id)
    }
}

pub(super) async fn delete_owned_place(
    pool: &PgPool,
    owner: Uuid,
    id: Uuid,
) -> Result<(), AppError> {
    let result = sqlx::query!(
        "DELETE FROM panda.geofences WHERE id = $1 AND user_id = $2",
        id,
        owner
    )
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    Ok(())
}
