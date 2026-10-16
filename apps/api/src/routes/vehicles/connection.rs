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
