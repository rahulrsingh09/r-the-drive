use super::*;
use sqlx::{Executor, Postgres};

pub(super) fn member_role(value: &str) -> Result<(), AppError> {
    match value {
        "owner" | "manager" | "viewer" => Ok(()),
        _ => Err(AppError::Validation(
            "role must be owner, manager, or viewer".into(),
        )),
    }
}

pub(super) fn member_email(value: &str) -> Result<String, AppError> {
    let email = value.trim().to_lowercase();
    if email.is_empty() || !email.contains('@') {
        return Err(AppError::Validation("valid email required".into()));
    }
    Ok(email)
}

pub(super) enum MemberDestination {
    Registered(Uuid),
    Invitation(String),
}

impl MemberDestination {
    pub async fn resolve(pool: &sqlx::PgPool, email: String) -> Result<Self, AppError> {
        match sqlx::query_scalar("SELECT id FROM panda.users WHERE email = $1")
            .bind(&email)
            .fetch_optional(pool)
            .await?
        {
            Some(id) => Ok(Self::Registered(id)),
            None => Ok(Self::Invitation(email)),
        }
    }
}

pub(super) struct MembershipGrant<'a> {
    pub vehicle_id: Uuid,
    pub user_id: Uuid,
    pub role: &'a str,
}

impl MembershipGrant<'_> {
    pub async fn write<'e>(
        &self,
        executor: impl Executor<'e, Database = Postgres>,
    ) -> Result<u64, AppError> {
        Ok(sqlx::query(
            "INSERT INTO panda.vehicle_memberships (vehicle_id, user_id, role, is_default)
            VALUES ($1, $2, $3, FALSE) ON CONFLICT (vehicle_id, user_id) DO UPDATE
            SET role = EXCLUDED.role, updated_at = now()",
        )
        .bind(self.vehicle_id)
        .bind(self.user_id)
        .bind(self.role)
        .execute(executor)
        .await?
        .rows_affected())
    }

    pub async fn settings<'e>(
        &self,
        executor: impl Executor<'e, Database = Postgres>,
    ) -> Result<(), AppError> {
        sqlx::query(
            "INSERT INTO panda.vehicle_user_settings (vehicle_id, user_id) VALUES ($1, $2)
            ON CONFLICT (vehicle_id, user_id) DO NOTHING",
        )
        .bind(self.vehicle_id)
        .bind(self.user_id)
        .execute(executor)
        .await?;
        Ok(())
    }
}

pub(super) struct IssuedInvitation {
    pub token: String,
    pub expires_at: chrono::DateTime<Utc>,
}

impl IssuedInvitation {
    pub async fn create(
        pool: &sqlx::PgPool,
        vehicle_id: Uuid,
        invited_by: Uuid,
        email: &str,
        role: &str,
        days: i32,
    ) -> Result<Self, AppError> {
        let token = format!("rmi_{}", Uuid::new_v4().simple());
        let token_hash = hash_invite_token(&token);
        let expires_at = Utc::now() + chrono::Duration::days(i64::from(days));
        sqlx::query("INSERT INTO panda.vehicle_invites (vehicle_id, invited_by, invitee_email, role, token_hash, expires_at)
            VALUES ($1, $2, $3, $4, $5, $6)")
            .bind(vehicle_id).bind(invited_by).bind(email).bind(role).bind(token_hash.as_slice()).bind(expires_at).execute(pool).await?;
        Ok(Self { token, expires_at })
    }
}

#[derive(sqlx::FromRow)]
pub(super) struct LockedInvitation {
    id: Uuid,
    vehicle_id: Uuid,
    invitee_email: String,
    role: String,
    expires_at: chrono::DateTime<Utc>,
    accepted_at: Option<chrono::DateTime<Utc>>,
    revoked_at: Option<chrono::DateTime<Utc>>,
    user_email: String,
}

impl LockedInvitation {
    pub async fn load(
        tx: &mut sqlx::Transaction<'_, Postgres>,
        user_id: Uuid,
        token: &str,
    ) -> Result<Self, AppError> {
        let hash = hash_invite_token(token);
        sqlx::query_as("SELECT i.id, i.vehicle_id, i.invitee_email, i.role, i.expires_at, i.accepted_at, i.revoked_at, u.email AS user_email
            FROM panda.vehicle_invites i JOIN panda.users u ON u.id = $2 WHERE i.token_hash = $1 FOR UPDATE")
            .bind(hash.as_slice()).bind(user_id).fetch_optional(&mut **tx).await?.ok_or(AppError::NotFound)
    }

    fn grant(
        &self,
        user_id: Uuid,
        now: chrono::DateTime<Utc>,
    ) -> Result<MembershipGrant<'_>, AppError> {
        let invalid = [
            (self.revoked_at.is_some(), "invite revoked"),
            (self.accepted_at.is_some(), "invite already accepted"),
            (self.expires_at < now, "invite expired"),
        ];
        if let Some((_, reason)) = invalid.into_iter().find(|(failed, _)| *failed) {
            return Err(AppError::Validation(reason.into()));
        }
        if self.invitee_email.to_lowercase() != self.user_email.to_lowercase() {
            return Err(AppError::Forbidden);
        }
        Ok(MembershipGrant {
            vehicle_id: self.vehicle_id,
            user_id,
            role: &self.role,
        })
    }

    pub async fn accept(
        self,
        tx: &mut sqlx::Transaction<'_, Postgres>,
        user_id: Uuid,
    ) -> Result<Uuid, AppError> {
        let grant = self.grant(user_id, Utc::now())?;
        grant.write(&mut **tx).await?;
        grant.settings(&mut **tx).await?;
        sqlx::query("UPDATE panda.vehicle_invites SET accepted_at = now(), accepted_user_id = $2, updated_at = now() WHERE id = $1")
            .bind(self.id).bind(user_id).execute(&mut **tx).await?;
        Ok(self.vehicle_id)
    }
}
