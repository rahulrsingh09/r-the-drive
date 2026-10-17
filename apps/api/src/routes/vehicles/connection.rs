use super::*;
use crate::ingestion::{rivian_auth::LoginResult, session_store::RivianTokenBundle};

pub(super) enum ConnectionExchange {
    Challenge(PendingOtpChallenge),
    Authenticated(RivianTokenBundle),
}

impl ConnectionExchange {
    pub fn from_login(result: LoginResult, user_id: Uuid) -> Self {
        match result {
            LoginResult::Authenticated(tokens) => Self::Authenticated(tokens),
            LoginResult::OtpRequired(challenge) => Self::Challenge(PendingOtpChallenge {
                email: challenge.email,
                otp_token: challenge.otp_token,
                csrf_token: challenge.csrf_token,
                app_session_token: challenge.app_session_token,
                user_id,
            }),
        }
    }

    pub async fn deliver(
        self,
        state: &AppState,
        client: &reqwest::Client,
        user_id: Uuid,
    ) -> Result<Json<serde_json::Value>, AppError> {
        match self {
            Self::Challenge(pending) => {
                let challenge_id = Uuid::new_v4().to_string();
                let mut conn = state.redis.get_multiplexed_async_connection().await?;
                store_encrypted_redis(
                    state,
                    &mut conn,
                    &format!("rivian:otp:{challenge_id}"),
                    &pending,
                    300,
                )
                .await?;
                response(ConnectResponse {
                    status: "otp_required",
                    requires_otp: true,
                    challenge_id: Some(challenge_id),
                    vehicle_id: None,
                    vehicles: Vec::new(),
                })
            }
            Self::Authenticated(tokens) => {
                let session = DiscoveredSession::load(client, tokens).await?;
                let mut conn = state.redis.get_multiplexed_async_connection().await?;
                session.publish(state, &mut conn, user_id, None).await
            }
        }
    }
}

pub(super) struct DiscoveredSession {
    tokens: RivianTokenBundle,
    vehicles: Vec<RivianVehicleSummary>,
}

impl DiscoveredSession {
    pub async fn load(
        client: &reqwest::Client,
        tokens: RivianTokenBundle,
    ) -> Result<Self, AppError> {
        let vehicles = crate::ingestion::rivian_auth::rivian_user_vehicles(client, &tokens)
            .await
            .map_err(|error| {
                warn!(error = %error, "vehicle.connect.fetch_user_vehicles_failed");
                AppError::RivianApi("Unable to fetch vehicles from Rivian".into())
            })?;
        Ok(Self { tokens, vehicles })
    }

    pub async fn publish(
        self,
        state: &AppState,
        conn: &mut redis::aio::MultiplexedConnection,
        user_id: Uuid,
        consumed_challenge: Option<&str>,
    ) -> Result<Json<serde_json::Value>, AppError> {
        store_encrypted_redis(
            state,
            conn,
            &format!("rivian:connect:{user_id}"),
            &self.tokens,
            300,
        )
        .await?;
        if let Some(key) = consumed_challenge {
            let _: () = redis::AsyncCommands::del(conn, key).await?;
        }
        info!(%user_id, vehicle_count = self.vehicles.len(), "vehicle.connect.authenticated");
        response(ConnectResponse {
            status: "connected",
            requires_otp: false,
            challenge_id: None,
            vehicle_id: None,
            vehicles: self.vehicles,
        })
    }
}

fn response(value: ConnectResponse) -> Result<Json<serde_json::Value>, AppError> {
    serde_json::to_value(value)
        .map(Json)
        .map_err(|error| AppError::Internal(error.into()))
}

pub(super) struct VerifiedRefresh {
    vehicle: Uuid,
    ciphertext: Vec<u8>,
}
pub(super) struct SavedRefresh {
    vehicle: Uuid,
}
impl VerifiedRefresh {
    pub(super) async fn verify(
        state: &AppState,
        vehicle: Uuid,
        remote: &str,
        tokens: crate::ingestion::session_store::RivianTokenBundle,
    ) -> Result<Self, AppError> {
        let account = lookup_rivian_vehicles(&tokens).await?;
        require_account_vehicle(&account, remote)?;
        let identity = age_identity(state)?;
        let ciphertext = crate::ingestion::session_store::encrypt_tokens(&tokens, &identity)
            .map_err(AppError::Internal)?;
        Ok(Self {
            vehicle,
            ciphertext,
        })
    }
    pub(super) async fn persist(
        self,
        pool: &sqlx::PgPool,
        user: Uuid,
    ) -> Result<SavedRefresh, AppError> {
        let mut tx = pool.begin().await?;
        lock_active_enrollment_user(&mut tx, user).await?;
        lock_credential_membership(&mut tx, user, self.vehicle).await?;
        sqlx::query("INSERT INTO panda.vehicle_credentials (vehicle_id,encrypted_tokens,token_created_at,last_refreshed_at) VALUES ($1,$2,now(),now()) ON CONFLICT (vehicle_id) DO UPDATE SET encrypted_tokens=EXCLUDED.encrypted_tokens,token_created_at=EXCLUDED.token_created_at,last_refreshed_at=now()")
            .bind(self.vehicle).bind(self.ciphertext).execute(&mut *tx).await?;
        enrollment::initialize_runtime(&mut tx, self.vehicle).await?;
        tx.commit().await?;
        Ok(SavedRefresh {
            vehicle: self.vehicle,
        })
    }
}
impl SavedRefresh {
    pub(super) async fn notify(self, state: &AppState) -> serde_json::Value {
        let queued = state
            .supervisor
            .send(SupervisorCommand::StartWorker {
                vehicle_id: self.vehicle,
            })
            .await;
        let ready = queued && wait_for_vehicle_data(&state.pool, self.vehicle).await;
        let (telemetry_status, telemetry_error) = match (queued, ready) {
            (_, true) => ("connected", None),
            (true, false) => (
                "waiting_for_vehicle_data",
                Some("Rivian credentials saved; vehicle data is still loading"),
            ),
            (false, false) => (
                "delayed",
                Some("worker supervisor unavailable; retry telemetry start"),
            ),
        };
        serde_json::json!({
            "ok":true,"vehicle_id":self.vehicle,"vehicle_saved":true,
            "connection_status":if ready { "connected" } else { "connected_waiting_for_vehicle_data" },
            "telemetry_status":telemetry_status,"telemetry_error":telemetry_error,
        })
    }
}
