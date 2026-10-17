use super::*;
use crate::ingestion::session_store::RivianTokenBundle;
use sqlx::{Postgres, Transaction};

pub(super) struct VerifiedEnrollment {
    pub remote_id: String,
    pub account_vehicle: RivianVehicleSummary,
    pub requested: AddVehicleBody,
}

pub(super) struct SavedEnrollment {
    pub vehicle_id: Uuid,
    pub created: bool,
}

impl VerifiedEnrollment {
    pub async fn persist(
        self,
        state: &AppState,
        user_id: Uuid,
        tokens: &RivianTokenBundle,
    ) -> Result<SavedEnrollment, AppError> {
        let mut tx = state.pool.begin().await?;
        let existing = lock_enrollment(&mut tx, user_id, &self.remote_id).await?;
        let created = existing.is_none();
        let vehicle_id = match existing {
            Some(id) => id,
            None => self.insert_vehicle(&mut tx, user_id).await?,
        };
        sqlx::query(
            "INSERT INTO panda.vehicle_user_settings (vehicle_id, user_id, display_name)
             VALUES ($1, $2, $3)
             ON CONFLICT (vehicle_id, user_id) DO UPDATE
             SET display_name = COALESCE(EXCLUDED.display_name, panda.vehicle_user_settings.display_name),
                 updated_at = now()",
        )
        .bind(vehicle_id)
        .bind(user_id)
        .bind(self.requested.name.as_deref().map(canonical_vehicle_name))
        .execute(&mut *tx)
        .await?;
        if created {
            let identity = age_identity(state)?;
            let encrypted = crate::ingestion::session_store::encrypt_tokens(tokens, &identity)
                .map_err(AppError::Internal)?;
            sqlx::query(
                "INSERT INTO panda.vehicle_credentials (vehicle_id, encrypted_tokens, token_created_at)
                 VALUES ($1,$2,now()) ON CONFLICT (vehicle_id) DO UPDATE
                 SET encrypted_tokens = EXCLUDED.encrypted_tokens,
                     token_created_at = now(), last_refreshed_at = now()",
            )
            .bind(vehicle_id)
            .bind(encrypted.as_slice())
            .execute(&mut *tx)
            .await?;
        }
        DefaultSelection::IfMissing {
            user_id,
            vehicle_id,
        }
        .apply(&mut tx)
        .await?;
        if created {
            initialize_runtime(&mut tx, vehicle_id).await?;
        }
        tx.commit().await?;
        Ok(SavedEnrollment {
            vehicle_id,
            created,
        })
    }

    async fn insert_vehicle(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        user_id: Uuid,
    ) -> Result<Uuid, AppError> {
        let vehicle_id = sqlx::query_scalar(
            "INSERT INTO panda.vehicles
             (user_id, rivian_vehicle_id, model, trim, vin, name, home_latitude, home_longitude)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8) RETURNING id",
        )
        .bind(user_id)
        .bind(&self.remote_id)
        .bind(canonical_vehicle_model(
            self.account_vehicle.model.as_deref().unwrap_or("R1T"),
        ))
        .bind(self.requested.trim.as_deref())
        .bind(self.account_vehicle.vin.as_deref())
        .bind(self.requested.name.as_deref().map(canonical_vehicle_name))
        .bind(self.requested.home_lat)
        .bind(self.requested.home_lng)
        .fetch_one(&mut **tx)
        .await?;
        sqlx::query("INSERT INTO panda.vehicle_memberships (vehicle_id, user_id, role, is_default) VALUES ($1, $2, 'owner', FALSE)")
            .bind(vehicle_id).bind(user_id).execute(&mut **tx).await?;
        Ok(vehicle_id)
    }
}

impl SavedEnrollment {
    pub async fn notify(self, state: &AppState) -> serde_json::Value {
        let queued = self.created
            && state
                .supervisor
                .send(SupervisorCommand::StartWorker {
                    vehicle_id: self.vehicle_id,
                })
                .await;
        let (status, error) = match (self.created, queued) {
            (false, _) => ("unchanged", None),
            (true, true) => ("starting", None),
            (true, false) => (
                "delayed",
                Some("worker supervisor unavailable; retry telemetry start"),
            ),
        };
        serde_json::json!({"vehicle_id":self.vehicle_id,"vehicle_saved":true,"telemetry_status":status,"telemetry_error":error})
    }
}

pub(super) enum DefaultSelection {
    IfMissing { user_id: Uuid, vehicle_id: Uuid },
    Replace { user_id: Uuid, vehicle_id: Uuid },
}

impl DefaultSelection {
    pub async fn apply(self, tx: &mut Transaction<'_, Postgres>) -> Result<(), AppError> {
        let (user_id, vehicle_id, only_missing) = match self {
            Self::IfMissing {
                user_id,
                vehicle_id,
            } => (user_id, vehicle_id, true),
            Self::Replace {
                user_id,
                vehicle_id,
            } => (user_id, vehicle_id, false),
        };
        if !only_missing {
            sqlx::query("UPDATE panda.vehicle_memberships SET is_default = FALSE, updated_at = now() WHERE user_id = $1")
                .bind(user_id).execute(&mut **tx).await?;
        }
        let changed = sqlx::query(
            "UPDATE panda.vehicle_memberships SET is_default = TRUE, updated_at = now()
             WHERE vehicle_id = $1 AND user_id = $2
               AND (NOT $3 OR NOT EXISTS (
                   SELECT 1 FROM panda.vehicle_memberships WHERE user_id = $2 AND is_default = TRUE))",
        )
        .bind(vehicle_id).bind(user_id).bind(only_missing).execute(&mut **tx).await?;
        if !only_missing && changed.rows_affected() == 0 {
            return Err(AppError::Forbidden);
        }
        sqlx::query("UPDATE panda.users SET default_vehicle_id = $1 WHERE id = $2 AND (NOT $3 OR default_vehicle_id IS NULL)")
            .bind(vehicle_id).bind(user_id).bind(only_missing).execute(&mut **tx).await?;
        Ok(())
    }
}

pub(super) async fn initialize_runtime(
    tx: &mut Transaction<'_, Postgres>,
    vehicle_id: Uuid,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO panda.vehicle_runtime_state
         (vehicle_id, is_online, worker_health, worker_health_msg, auth_state, auth_reason_code, updated_at)
         VALUES ($1, FALSE, 'starting', 'Initializing Rivian telemetry collector', 'authorized', NULL, now())
         ON CONFLICT (vehicle_id) DO UPDATE SET is_online = FALSE, worker_health = 'starting',
             worker_health_msg = 'Initializing Rivian telemetry collector', auth_state = 'authorized',
             auth_reason_code = NULL, updated_at = now()",
    ).bind(vehicle_id).execute(&mut **tx).await?;
    Ok(())
}

pub(super) struct DemoSpecification {
    model: &'static str,
    remote_keys: Vec<String>,
    display_name: String,
    battery_config: &'static str,
    capacity_wh: f64,
}
pub(super) struct DemoCreation {
    vehicle: Uuid,
    created: bool,
    seeded: Option<crate::services::demo_seed::DemoSeedSummary>,
}
impl DemoSpecification {
    pub(super) fn parse(requested: &str) -> Result<Self, AppError> {
        let requested = requested.trim().to_uppercase();
        let (model, battery_config, capacity_wh) = match canonical_vehicle_model(&requested) {
            "R1T" => ("R1T", "r1_large_g1", 135_000.0),
            "R1S" => ("R1S", "r1_large_g1", 135_000.0),
            "R2" => ("R2", "r2", 82_000.0),
            _ => {
                return Err(AppError::Validation(
                    "model must be one of R1T, R1S, R2".into(),
                ))
            }
        };
        let mut remote_keys = vec![format!("demo-{}-local", model.to_lowercase())];
        if model == "R2" {
            remote_keys.push("demo-r2s-local".into());
        }
        Ok(Self {
            model,
            remote_keys,
            display_name: format!("Demo {model}"),
            battery_config,
            capacity_wh,
        })
    }
    pub(super) async fn ensure(
        self,
        tx: &mut Transaction<'_, Postgres>,
        user: Uuid,
    ) -> Result<DemoCreation, AppError> {
        let existing = sqlx::query_scalar::<_, Uuid>("SELECT v.id FROM panda.vehicles v JOIN panda.vehicle_memberships vm ON vm.vehicle_id=v.id WHERE vm.user_id=$1 AND v.rivian_vehicle_id=ANY($2::text[]) LIMIT 1")
            .bind(user).bind(&self.remote_keys).fetch_optional(&mut **tx).await?;
        if let Some(vehicle) = existing {
            return Ok(DemoCreation {
                vehicle,
                created: false,
                seeded: None,
            });
        }
        let vehicle = sqlx::query_scalar::<_, Uuid>("INSERT INTO panda.vehicles (user_id,rivian_vehicle_id,model,trim,color,battery_config,battery_capacity_wh,name) VALUES ($1,$2,$3,'Adventure','Limestone',$4,$5,$6) RETURNING id")
            .bind(user).bind(&self.remote_keys[0]).bind(self.model).bind(self.battery_config).bind(self.capacity_wh).bind(&self.display_name).fetch_one(&mut **tx).await?;
        sqlx::query("INSERT INTO panda.vehicle_memberships (vehicle_id,user_id,role,is_default) VALUES ($1,$2,'owner',FALSE) ON CONFLICT (vehicle_id,user_id) DO UPDATE SET role='owner',updated_at=now()")
            .bind(vehicle).bind(user).execute(&mut **tx).await?;
        sqlx::query("INSERT INTO panda.vehicle_user_settings (vehicle_id,user_id,display_name) VALUES ($1,$2,$3) ON CONFLICT (vehicle_id,user_id) DO UPDATE SET display_name=EXCLUDED.display_name,updated_at=now()")
            .bind(vehicle).bind(user).bind(self.display_name).execute(&mut **tx).await?;
        let seeded = seed_new_demo_vehicle(tx, vehicle, self.model, Utc::now()).await?;
        Ok(DemoCreation {
            vehicle,
            created: true,
            seeded: Some(seeded),
        })
    }
}
impl DemoCreation {
    pub(super) fn refreshed(
        vehicle: Uuid,
        summary: crate::services::demo_seed::DemoSeedSummary,
    ) -> Self {
        Self {
            vehicle,
            created: false,
            seeded: Some(summary),
        }
    }
    pub(super) fn response(self) -> serde_json::Value {
        let seeded = self.seeded.is_some();
        let mut response = self.seeded.map(|summary| serde_json::to_value(summary).expect("demo summary serializes"))
            .unwrap_or_else(|| serde_json::json!({"seeded_at":null,"window_start":null,"window_end":null,"counts":null}));
        response
            .as_object_mut()
            .expect("demo summary object")
            .extend([
                ("ok".into(), serde_json::json!(true)),
                ("vehicle_id".into(), serde_json::json!(self.vehicle)),
                ("created".into(), serde_json::json!(self.created)),
                ("seeded".into(), serde_json::json!(seeded)),
                (
                    "refreshed".into(),
                    serde_json::json!(seeded && !self.created),
                ),
            ]);
        response
    }
}
