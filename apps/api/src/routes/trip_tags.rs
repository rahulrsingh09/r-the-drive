//! Shared, vehicle-scoped trip tag catalog and assignment routes.

mod assignment;

use axum::{
    extract::{Path, State},
    routing::{get, patch, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;

use crate::{
    db::vehicles::{require_vehicle_manager_access, require_vehicle_read_access},
    errors::AppError,
    middleware::auth::{AppState, AuthUser},
};

const COLOR_TOKENS: &[&str] = &["accent", "neutral", "info", "success", "warning", "danger"];
const MAX_BATCH_TRIPS: usize = 500;
const MAX_BATCH_TAGS: usize = 50;

pub fn router() -> Router<AppState> {
    super::route_table([
        (
            "/vehicles/{vehicle_id}/trip-tags",
            get(list_tags).post(create_tag),
        ),
        (
            "/vehicles/{vehicle_id}/trip-tags/{tag_id}",
            patch(update_tag).delete(delete_tag),
        ),
        (
            "/vehicles/{vehicle_id}/trip-tags/assignments",
            post(update_assignments),
        ),
    ])
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct TripTagResponse {
    pub id: Uuid,
    pub vehicle_id: Uuid,
    pub name: String,
    pub color_token: String,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct CreateTagBody {
    name: String,
    color_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UpdateTagBody {
    name: Option<String>,
    color_token: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AssignmentMode {
    Add,
    Remove,
    Replace,
}

#[derive(Debug, Deserialize)]
struct AssignmentBody {
    trip_ids: Vec<Uuid>,
    tag_ids: Vec<Uuid>,
    mode: AssignmentMode,
}

#[derive(Debug, Serialize)]
struct AssignmentResponse {
    updated_trip_count: usize,
}

/// Produce a stable display value and uniqueness key. NFC prevents equivalent
/// Unicode spellings from creating visually indistinguishable catalog entries;
/// whitespace and Unicode lowercasing make lookup/collision behavior explicit.
fn normalize_name(raw: &str) -> Result<(String, String), AppError> {
    let mut display = String::new();
    let mut separator = false;
    for character in raw.nfc() {
        if character.is_whitespace() {
            separator = !display.is_empty();
        } else {
            if separator {
                display.push(' ');
                separator = false;
            }
            display.push(character);
        }
    }
    if !(1..=64).contains(&display.chars().take(65).count()) {
        return Err(AppError::Validation(
            "tag name must contain 1 to 64 non-whitespace characters".into(),
        ));
    }
    let normalized = display.nfc().collect::<String>().to_lowercase();
    Ok((display, normalized))
}

fn validate_color_token(token: Option<String>) -> Result<String, AppError> {
    let token = token.unwrap_or_else(|| "accent".into());
    if COLOR_TOKENS.contains(&token.as_str()) {
        Ok(token)
    } else {
        Err(AppError::Validation(
            "color_token must be a supported semantic color token".into(),
        ))
    }
}

async fn list_tags(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(vehicle_id): Path<Uuid>,
) -> Result<Json<Vec<TripTagResponse>>, AppError> {
    require_vehicle_read_access(&state.pool, &auth, vehicle_id).await?;
    let tags = sqlx::query_as::<_, TripTagResponse>(
        "SELECT id, vehicle_id, name, color_token, created_by, created_at, updated_at
         FROM panda.trip_tags WHERE vehicle_id=$1 ORDER BY lower(name), id",
    )
    .bind(vehicle_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(tags))
}

async fn create_tag(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(vehicle_id): Path<Uuid>,
    Json(body): Json<CreateTagBody>,
) -> Result<Json<TripTagResponse>, AppError> {
    require_vehicle_manager_access(&state.pool, &auth, vehicle_id).await?;
    TagChange::prepare(None, Some(body.name), body.color_token)?
        .persist(&state, vehicle_id, auth.user_id)
        .await
        .map(Json)
}

async fn update_tag(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((vehicle_id, tag_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<UpdateTagBody>,
) -> Result<Json<TripTagResponse>, AppError> {
    require_vehicle_manager_access(&state.pool, &auth, vehicle_id).await?;
    TagChange::prepare(Some(tag_id), body.name, body.color_token)?
        .persist(&state, vehicle_id, auth.user_id)
        .await
        .map(Json)
}

async fn delete_tag(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((vehicle_id, tag_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<serde_json::Value>, AppError> {
    require_vehicle_manager_access(&state.pool, &auth, vehicle_id).await?;
    let removed: Option<Uuid> = sqlx::query_scalar(
        "DELETE FROM panda.trip_tags WHERE (id,vehicle_id)=($1,$2) RETURNING id",
    )
    .bind(tag_id)
    .bind(vehicle_id)
    .fetch_optional(&state.pool)
    .await?;
    removed.ok_or(AppError::NotFound)?;
    Ok(Json(serde_json::json!({"ok":true})))
}

async fn update_assignments(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(vehicle_id): Path<Uuid>,
    Json(body): Json<AssignmentBody>,
) -> Result<Json<AssignmentResponse>, AppError> {
    require_vehicle_manager_access(&state.pool, &auth, vehicle_id).await?;
    let plan =
        assignment::AssignmentPlan::prepare(&state.pool, vehicle_id, auth.user_id, body).await?;
    Ok(Json(plan.commit(&state.pool).await?))
}

fn validate_assignment_ids(body: &AssignmentBody) -> Result<(), AppError> {
    for (count, minimum, maximum, message) in [
        (
            body.trip_ids.len(),
            1,
            MAX_BATCH_TRIPS,
            format!("trip_ids must contain 1 to {MAX_BATCH_TRIPS} IDs"),
        ),
        (
            body.tag_ids.len(),
            0,
            MAX_BATCH_TAGS,
            format!("tag_ids may contain at most {MAX_BATCH_TAGS} IDs"),
        ),
    ] {
        if !(minimum..=maximum).contains(&count) {
            return Err(AppError::Validation(message));
        }
    }
    if let (AssignmentMode::Add | AssignmentMode::Remove, true) =
        (body.mode, body.tag_ids.is_empty())
    {
        return Err(AppError::Validation(
            "tag_ids is required for add and remove".into(),
        ));
    }
    if [&body.trip_ids, &body.tag_ids]
        .into_iter()
        .any(|ids| has_duplicates(ids))
    {
        return Err(AppError::Validation(
            "trip_ids and tag_ids must not contain duplicates".into(),
        ));
    }
    Ok(())
}

fn has_duplicates(ids: &[Uuid]) -> bool {
    let mut seen = std::collections::HashSet::with_capacity(ids.len());
    ids.iter().any(|id| !seen.insert(*id))
}

async fn assert_vehicle_ids(
    pool: &sqlx::PgPool,
    vehicle_id: Uuid,
    trip_ids: &[Uuid],
    tag_ids: &[Uuid],
) -> Result<(), AppError> {
    for (table, ids, message) in [
        (
            "panda.trips",
            trip_ids,
            "every trip_id must belong to this vehicle",
        ),
        (
            "panda.trip_tags",
            tag_ids,
            "every tag_id must belong to this vehicle",
        ),
    ] {
        if table == "panda.trip_tags" && ids.is_empty() {
            continue;
        }
        let mut query = sqlx::QueryBuilder::<sqlx::Postgres>::new("SELECT COUNT(*) FROM ");
        query
            .push(table)
            .push(" WHERE vehicle_id=")
            .push_bind(vehicle_id)
            .push(" AND id=ANY(")
            .push_bind(ids)
            .push(")");
        let count: i64 = query.build_query_scalar().fetch_one(pool).await?;
        if count != ids.len() as i64 {
            return Err(AppError::Validation(message.into()));
        }
    }
    Ok(())
}

struct TagChange {
    id: Option<Uuid>,
    name: Option<String>,
    normalized_name: Option<String>,
    color_token: Option<String>,
}
impl TagChange {
    fn prepare(
        id: Option<Uuid>,
        name: Option<String>,
        color_token: Option<String>,
    ) -> Result<Self, AppError> {
        if id.is_some() && name.is_none() && color_token.is_none() {
            return Err(AppError::Validation(
                "at least one tag field is required".into(),
            ));
        }
        let named = name.as_deref().map(normalize_name).transpose()?;
        let color_token = if id.is_none() {
            Some(validate_color_token(color_token)?)
        } else {
            color_token
                .map(|color| validate_color_token(Some(color)))
                .transpose()?
        };
        let (name, normalized_name) = match named {
            Some((name, key)) => (Some(name), Some(key)),
            None => (None, None),
        };
        Ok(Self {
            id,
            name,
            normalized_name,
            color_token,
        })
    }

    async fn persist(
        self,
        state: &AppState,
        vehicle_id: Uuid,
        actor: Uuid,
    ) -> Result<TripTagResponse, AppError> {
        let result = sqlx::query_as::<_, TripTagResponse>(
            "WITH inserted AS (
              INSERT INTO panda.trip_tags (vehicle_id,name,normalized_name,color_token,created_by)
              SELECT $2,$3,$4,$5,$6 WHERE $1::uuid IS NULL
              RETURNING id,vehicle_id,name,color_token,created_by,created_at,updated_at
             ), updated AS (
              UPDATE panda.trip_tags SET name=COALESCE($3,name),normalized_name=COALESCE($4,normalized_name),
              color_token=COALESCE($5,color_token),updated_at=now() WHERE id=$1 AND vehicle_id=$2
              RETURNING id,vehicle_id,name,color_token,created_by,created_at,updated_at
             ) SELECT * FROM inserted UNION ALL SELECT * FROM updated")
            .bind(self.id).bind(vehicle_id).bind(self.name).bind(self.normalized_name).bind(self.color_token).bind(actor)
            .fetch_optional(&state.pool).await;
        match result {
            Ok(Some(tag)) => Ok(tag),
            Ok(None) => Err(AppError::NotFound),
            Err(error)
                if error
                    .as_database_error()
                    .and_then(|error| error.code())
                    .as_deref()
                    == Some("23505") =>
            {
                Err(AppError::Conflict(
                    "a tag with that name already exists".into(),
                ))
            }
            Err(error) => Err(AppError::Database(error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_is_unicode_and_whitespace_stable() {
        let (display, normalized) = normalize_name("  Biké   Rack ").unwrap();
        assert_eq!(display, "Biké Rack");
        assert_eq!(normalized, "biké rack");
    }

    #[test]
    fn rejects_invalid_semantic_color() {
        assert!(validate_color_token(Some("#ff0000".into())).is_err());
    }

    #[test]
    fn replace_allows_clearing_tags_but_add_requires_a_tag() {
        let trip_id = Uuid::new_v4();
        let replace = AssignmentBody {
            trip_ids: vec![trip_id],
            tag_ids: vec![],
            mode: AssignmentMode::Replace,
        };
        assert!(validate_assignment_ids(&replace).is_ok());

        let add = AssignmentBody {
            mode: AssignmentMode::Add,
            ..replace
        };
        assert!(validate_assignment_ids(&add).is_err());
    }
}
