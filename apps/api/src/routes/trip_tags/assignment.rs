use super::*;

#[derive(Clone, Copy)]
enum Removal {
    None,
    Selected,
    All,
}

#[derive(Clone, Copy)]
enum Addition {
    None,
    Idempotent,
    Exact,
}

pub(super) struct AssignmentPlan {
    trips: Vec<Uuid>,
    tags: Vec<Uuid>,
    actor: Uuid,
    remove: Removal,
    add: Addition,
}

impl AssignmentPlan {
    pub async fn prepare(
        pool: &sqlx::PgPool,
        vehicle: Uuid,
        actor: Uuid,
        body: AssignmentBody,
    ) -> Result<Self, AppError> {
        validate_assignment_ids(&body)?;
        assert_vehicle_ids(pool, vehicle, &body.trip_ids, &body.tag_ids).await?;
        let (remove, add) = match body.mode {
            AssignmentMode::Add => (Removal::None, Addition::Idempotent),
            AssignmentMode::Remove => (Removal::Selected, Addition::None),
            AssignmentMode::Replace => (
                Removal::All,
                if body.tag_ids.is_empty() {
                    Addition::None
                } else {
                    Addition::Exact
                },
            ),
        };
        Ok(Self {
            trips: body.trip_ids,
            tags: body.tag_ids,
            actor,
            remove,
            add,
        })
    }

    pub async fn commit(self, pool: &sqlx::PgPool) -> Result<AssignmentResponse, AppError> {
        let mut tx = pool.begin().await?;
        match self.remove {
            Removal::None => (),
            Removal::Selected => {
                sqlx::query("DELETE FROM panda.trip_tag_assignments WHERE trip_id=ANY($1) AND tag_id=ANY($2)")
                    .bind(&self.trips).bind(&self.tags).execute(&mut *tx).await?;
            }
            Removal::All => {
                sqlx::query("DELETE FROM panda.trip_tag_assignments WHERE trip_id=ANY($1)")
                    .bind(&self.trips)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        if !matches!(self.add, Addition::None) {
            let conflict = if matches!(self.add, Addition::Idempotent) {
                " ON CONFLICT (trip_id,tag_id) DO NOTHING"
            } else {
                ""
            };
            let sql=format!("INSERT INTO panda.trip_tag_assignments (trip_id,tag_id,assigned_by) SELECT trip_id,tag_id,$3 FROM unnest($1::uuid[]) trip_id CROSS JOIN unnest($2::uuid[]) tag_id{conflict}");
            sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
                .bind(&self.trips)
                .bind(&self.tags)
                .bind(self.actor)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(AssignmentResponse {
            updated_trip_count: self.trips.len(),
        })
    }
}
