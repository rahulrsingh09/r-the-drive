use crate::errors::AppError;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserRole {
    SuperUser,
    Admin,
    User,
}

impl UserRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SuperUser => "super_user",
            Self::Admin => "admin",
            Self::User => "user",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "super_user" => Some(Self::SuperUser),
            "admin" => Some(Self::Admin),
            "user" => Some(Self::User),
            _ => None,
        }
    }
}

pub async fn get_user_role(pool: &PgPool, user_id: Uuid) -> Result<UserRole, AppError> {
    authorized_role(pool, user_id, &["super_user", "admin", "user"]).await
}

pub async fn require_admin_or_super_user(
    pool: &PgPool,
    user_id: Uuid,
) -> Result<UserRole, AppError> {
    authorized_role(pool, user_id, &["super_user", "admin"]).await
}

pub async fn require_super_user(pool: &PgPool, user_id: Uuid) -> Result<(), AppError> {
    authorized_role(pool, user_id, &["super_user"])
        .await
        .map(|_| ())
}

async fn authorized_role(
    pool: &PgPool,
    user_id: Uuid,
    allowed: &[&str],
) -> Result<UserRole, AppError> {
    let record: Option<(Option<String>, Option<bool>)> =
        sqlx::query_as("SELECT role, role = ANY($2::text[]) FROM panda.users WHERE id = $1")
            .bind(user_id)
            .bind(allowed)
            .fetch_optional(pool)
            .await?;
    let (Some(stored), permitted) = record.ok_or(AppError::Forbidden)? else {
        return Err(AppError::Forbidden);
    };
    let role = UserRole::from_str(&stored)
        .ok_or_else(|| AppError::Validation("unknown user role".into()))?;
    permitted
        .filter(|value| *value)
        .map(|_| role)
        .ok_or(AppError::Forbidden)
}

pub fn can_manage_user(actor: UserRole, target: UserRole) -> bool {
    match actor {
        UserRole::SuperUser => true,
        UserRole::Admin => target == UserRole::User,
        UserRole::User => false,
    }
}

pub async fn get_electricity_rate(pool: &PgPool, user_id: Uuid) -> Result<f64, AppError> {
    sqlx::query_scalar::<_, f64>(
        "SELECT COALESCE((SELECT electricity_rate_per_kwh FROM panda.user_preferences WHERE user_id = $1), 0.13::float8)")
        .bind(user_id).fetch_one(pool).await.map_err(AppError::from)
}

#[cfg(test)]
mod tests {
    use super::{can_manage_user, UserRole};

    #[test]
    fn super_user_can_manage_every_role() {
        assert!(can_manage_user(UserRole::SuperUser, UserRole::SuperUser));
        assert!(can_manage_user(UserRole::SuperUser, UserRole::Admin));
        assert!(can_manage_user(UserRole::SuperUser, UserRole::User));
    }

    #[test]
    fn admin_can_manage_only_user_role() {
        assert!(can_manage_user(UserRole::Admin, UserRole::User));
        assert!(!can_manage_user(UserRole::Admin, UserRole::Admin));
        assert!(!can_manage_user(UserRole::Admin, UserRole::SuperUser));
    }

    #[test]
    fn user_cannot_manage_any_role() {
        assert!(!can_manage_user(UserRole::User, UserRole::User));
        assert!(!can_manage_user(UserRole::User, UserRole::Admin));
        assert!(!can_manage_user(UserRole::User, UserRole::SuperUser));
    }
}
