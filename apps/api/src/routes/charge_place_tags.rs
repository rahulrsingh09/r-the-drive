//! Place tags for charges: Home, Office or Other per charge session (or per place when explicitly requested).

use axum::{
    extract::{Path, State},
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    db::vehicles::require_vehicle_manager_access,
    errors::AppError,
    middleware::auth::{AppState, AuthUser},
    services::cost::recompute_charge_session_cost,
};

pub fn router() -> Router<AppState> {
    Router::new().route(
        "/vehicles/{vehicle_id}/charging-sessions/{id}/place-tag",
        post(apply_place_tag),
    )
}

/// Sessions this close to the selected charge count as the same place.
const PLACE_TAG_RADIUS_M: f64 = 150.0;

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum PlaceTag {
    Home,
    Office,
    Other,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum PlaceTagScope {
    /// Tag only the requested charge session without disturbing any other sessions.
    #[default]
    Session,
    /// Tag this charge and charges physically within 150 m of its source GPS.
    Place,
}

#[derive(Debug, Deserialize)]
struct PlaceTagBody {
    tag: PlaceTag,
    #[serde(default)]
    scope: PlaceTagScope,
}

#[derive(Debug, Serialize)]
struct PlaceTagResponse {
    tag: &'static str,
    place_id: Option<Uuid>,
    updated: usize,
    session_ids: Vec<Uuid>,
}

#[derive(sqlx::FromRow)]
struct TagPlaceRow {
    id: Uuid,
    address_id: Option<Uuid>,
    is_home: bool,
    latitude: Option<f64>,
    longitude: Option<f64>,
    cost_profile_id: Option<Uuid>,
}

/// Applies Home, Office or Other to the selected charge (`scope = "session"`, default),
/// or to every charge of this vehicle physically within 150 m of its reported
/// location when `scope = "place"` is explicitly requested. Home and Office link
/// the user's saved Home or Office place, so its cost profile prices the charges.
/// Other removes the saved place and its rate, keeps the reported location, and
/// stops automatic re-matching. Free and manual cost overrides keep their precedence.
async fn apply_place_tag(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((vehicle_id, session_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<PlaceTagBody>,
) -> Result<Json<PlaceTagResponse>, AppError> {
    require_vehicle_manager_access(&state.pool, &auth, vehicle_id).await?;
    let place = match body.tag {
        PlaceTag::Other => None,
        PlaceTag::Home | PlaceTag::Office => Some(
            sqlx::query_as::<_, TagPlaceRow>(
                // The Home or Office flag wins; a place named "Home" or
                // "Office" is the fallback for places saved without the flag.
                "SELECT id, address_id, is_home, latitude, longitude, cost_profile_id
                   FROM panda.geofences
                  WHERE user_id = $1
                    AND ((CASE WHEN $2 THEN is_home ELSE is_work END)
                         OR lower(trim(name)) = $3)
                  ORDER BY (CASE WHEN $2 THEN is_home ELSE is_work END) DESC, created_at, id
                  LIMIT 1",
            )
            .bind(auth.user_id)
            .bind(body.tag == PlaceTag::Home)
            .bind(if body.tag == PlaceTag::Home {
                "home"
            } else {
                "office"
            })
            .fetch_optional(&state.pool)
            .await?
            .ok_or_else(|| {
                AppError::Validation(match body.tag {
                    PlaceTag::Home => "Save a Home place first".into(),
                    _ => "Save an Office place first".into(),
                })
            })?,
        ),
    };

    let mut tx = state.pool.begin().await?;
    let session_ids = match body.scope {
        PlaceTagScope::Session => {
            sqlx::query_scalar::<_, Uuid>(
                "SELECT id FROM panda.charge_sessions
                  WHERE id = $1 AND vehicle_id = $2
                  FOR UPDATE",
            )
            .bind(session_id)
            .bind(vehicle_id)
            .fetch_all(&mut *tx)
            .await?
        }
        PlaceTagScope::Place => {
            sqlx::query_scalar::<_, Uuid>(
                r#"WITH anchor AS (
                       SELECT id,
                              COALESCE(source_location_lat, location_lat) AS lat,
                              COALESCE(source_location_lng, location_lng) AS lng
                         FROM panda.charge_sessions
                        WHERE id = $1 AND vehicle_id = $2
                   )
                   SELECT cs.id
                     FROM panda.charge_sessions cs, anchor a
                    WHERE cs.vehicle_id = $2
                      AND (cs.id = a.id
                           OR (a.lat IS NOT NULL AND a.lng IS NOT NULL
                               AND COALESCE(cs.source_location_lat, cs.location_lat) IS NOT NULL
                               AND COALESCE(cs.source_location_lng, cs.location_lng) IS NOT NULL
                               AND panda.earth_distance(
                                     panda.ll_to_earth(a.lat, a.lng),
                                     panda.ll_to_earth(COALESCE(cs.source_location_lat, cs.location_lat),
                                                       COALESCE(cs.source_location_lng, cs.location_lng))
                                   ) <= $3))
                    ORDER BY cs.started_at
                    FOR UPDATE OF cs"#,
            )
            .bind(session_id)
            .bind(vehicle_id)
            .bind(PLACE_TAG_RADIUS_M)
            .fetch_all(&mut *tx)
            .await?
        }
    };
    if session_ids.is_empty() {
        return Err(AppError::NotFound);
    }

    match &place {
        Some(place) => {
            sqlx::query(
                "UPDATE panda.charge_sessions
                    SET geofence_id = $1, address_id = $2, is_home = $3,
                        location_lat = $4, location_lng = $5, cost_profile_id = $6,
                        location_override_mode = 'saved_place'
                  WHERE vehicle_id = $7 AND id = ANY($8)",
            )
            .bind(place.id)
            .bind(place.address_id)
            .bind(place.is_home)
            .bind(place.latitude)
            .bind(place.longitude)
            .bind(place.cost_profile_id)
            .bind(vehicle_id)
            .bind(&session_ids)
            .execute(&mut *tx)
            .await?;
        }
        None => {
            sqlx::query(
                "UPDATE panda.charge_sessions
                    SET geofence_id = NULL, address_id = NULL, is_home = FALSE, cost_profile_id = NULL,
                        location_lat = COALESCE(source_location_lat, location_lat),
                        location_lng = COALESCE(source_location_lng, location_lng),
                        location_override_mode = 'none'
                  WHERE vehicle_id = $1 AND id = ANY($2)",
            )
            .bind(vehicle_id)
            .bind(&session_ids)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE panda.charge_session_user_annotations
                    SET geofence_id = NULL, address_id = NULL, is_home = FALSE, updated_at = now()
                  WHERE charge_session_id = ANY($1)",
            )
            .bind(&session_ids)
            .execute(&mut *tx)
            .await?;
        }
    }
    tx.commit().await?;

    for id in &session_ids {
        recompute_charge_session_cost(&state.pool, *id).await?;
    }
    Ok(Json(PlaceTagResponse {
        tag: match body.tag {
            PlaceTag::Home => "home",
            PlaceTag::Office => "office",
            PlaceTag::Other => "other",
        },
        place_id: place.map(|place| place.id),
        updated: session_ids.len(),
        session_ids,
    }))
}

#[cfg(test)]
#[path = "charge_place_tags_authorization_tests.rs"]
mod authorization_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn place_tag_body_defaults_to_session_and_accepts_explicit_scopes() {
        let default_body: PlaceTagBody = serde_json::from_str(r#"{"tag":"home"}"#).unwrap();
        assert_eq!(default_body.tag, PlaceTag::Home);
        assert_eq!(default_body.scope, PlaceTagScope::Session);

        let session_body: PlaceTagBody =
            serde_json::from_str(r#"{"tag":"office","scope":"session"}"#).unwrap();
        assert_eq!(session_body.tag, PlaceTag::Office);
        assert_eq!(session_body.scope, PlaceTagScope::Session);

        let place_body: PlaceTagBody =
            serde_json::from_str(r#"{"tag":"other","scope":"place"}"#).unwrap();
        assert_eq!(place_body.tag, PlaceTag::Other);
        assert_eq!(place_body.scope, PlaceTagScope::Place);
    }
}
