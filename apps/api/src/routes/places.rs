use anyhow::anyhow;
use axum::{
    extract::{Path, Query, State},
    routing::get,
    Json, Router,
};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::time::Duration;
use tracing::debug;
use uuid::Uuid;

use crate::{
    errors::AppError,
    middleware::auth::{AppState, AuthUser},
    models::cost_profile::{validate_tou_periods, TouPeriod},
    services::cost::recompute_charge_session_cost,
};

const SYNC_LINKED_SESSION_COST_PROFILE_SQL: &str = "UPDATE panda.charge_sessions cs
        SET cost_profile_id=$2
      WHERE geofence_id=$1 AND cost_override_mode='automatic'
        AND EXISTS (
            SELECT 1 FROM panda.vehicle_memberships vm
            WHERE vm.vehicle_id=cs.vehicle_id AND vm.user_id=$3
              AND vm.role IN ('owner', 'manager')
        )
      RETURNING id";

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/places", get(list_places).post(create_place))
        .route("/places/search", get(search_places))
        .route(
            "/places/{id}",
            get(get_place).put(update_place).delete(delete_place),
        )
}

#[derive(Debug, Serialize)]
struct PlaceRecord {
    id: Uuid,
    name: String,
    latitude: f64,
    longitude: f64,
    radius_m: f64,
    is_home: bool,
    is_work: bool,
    address: Option<AddressRecord>,
    charging: Option<ChargingProfileRecord>,
}

#[derive(Debug, Serialize)]
struct AddressRecord {
    id: Option<Uuid>,
    display_name: String,
    osm_id: Option<i64>,
    latitude: f64,
    longitude: f64,
    road: Option<String>,
    city: Option<String>,
    state: Option<String>,
    postcode: Option<String>,
    country: Option<String>,
    raw: Option<Value>,
}

#[derive(Debug, Serialize)]
struct ChargingProfileRecord {
    id: Uuid,
    name: String,
    billing_type: String,
    rate: f64,
    session_fee: f64,
    currency: String,
    timezone: Option<String>,
    tou_periods: Value,
}

#[derive(Serialize)]
struct PlacesResponse {
    places: Vec<PlaceRecord>,
}

#[derive(Deserialize)]
struct SearchParams {
    q: String,
    limit: Option<u8>,
}

#[derive(Deserialize)]
struct PlaceBody {
    name: String,
    radius_m: Option<f64>,
    is_home: Option<bool>,
    is_work: Option<bool>,
    address: AddressInput,
    charging: Option<ChargingProfileInput>,
}

#[derive(Debug, Clone, Deserialize)]
struct AddressInput {
    display_name: String,
    osm_id: Option<i64>,
    latitude: f64,
    longitude: f64,
    road: Option<String>,
    city: Option<String>,
    state: Option<String>,
    postcode: Option<String>,
    country: Option<String>,
    raw: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct ChargingProfileInput {
    name: Option<String>,
    billing_type: String,
    rate: f64,
    session_fee: Option<f64>,
    currency: Option<String>,
    timezone: Option<String>,
    tou_periods: Option<Value>,
}

fn normalize_place_billing_type(raw: &str) -> &str {
    match raw {
        "flat" => "per_kwh",
        "tou" => "tou",
        "per_kwh" => "per_kwh",
        _ => raw,
    }
}

async fn list_places(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<PlacesResponse>, AppError> {
    Ok(Json(PlacesResponse {
        places: fetch_places(&state.pool, auth.user_id, None).await?,
    }))
}

async fn get_place(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<PlaceRecord>, AppError> {
    fetch_places(&state.pool, auth.user_id, Some(id))
        .await?
        .into_iter()
        .next()
        .map(Json)
        .ok_or(AppError::NotFound)
}

async fn search_places(
    _auth: AuthUser,
    State(state): State<AppState>,
    Query(params): Query<SearchParams>,
) -> Result<Json<Vec<AddressRecord>>, AppError> {
    let Some(plan) = search::SearchPlan::new(params) else {
        return Ok(Json(Vec::new()));
    };
    plan.resolve(&state).await.map(Json)
}

fn search_cache_key(query: &str, limit: u8) -> String {
    let mut hasher = Sha256::new();
    hasher.update(query.as_bytes());
    hasher.update([0]);
    hasher.update(limit.to_le_bytes());
    format!(
        "external:nominatim:search:{}",
        hex::encode(hasher.finalize())
    )
}

async fn create_place(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<PlaceBody>,
) -> Result<Json<PlaceRecord>, AppError> {
    let committed = workflow::PlaceMutation::create(auth.user_id, body)?
        .commit(&state.pool)
        .await?;
    let id = committed.finish(&state.pool).await?;
    get_place(auth, State(state), Path(id)).await
}

async fn update_place(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<PlaceBody>,
) -> Result<Json<PlaceRecord>, AppError> {
    let committed = workflow::PlaceMutation::update(&state.pool, auth.user_id, id, body)
        .await?
        .commit(&state.pool)
        .await?;
    let id = committed.finish(&state.pool).await?;
    get_place(auth, State(state), Path(id)).await
}

async fn delete_place(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    workflow::delete_owned_place(&state.pool, auth.user_id, id).await?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}

async fn fetch_places(
    pool: &sqlx::PgPool,
    user_id: Uuid,
    place_id: Option<Uuid>,
) -> Result<Vec<PlaceRecord>, AppError> {
    let rows = sqlx::query!(
        r#"SELECT
               g.id,
               g.name,
               g.latitude,
               g.longitude,
               g.radius_m,
               g.is_home,
               g.is_work,
               a.id AS "address_id?: Uuid",
               a.display_name AS "address_display_name?",
               a.osm_id AS "address_osm_id?",
               a.latitude AS "address_latitude?",
               a.longitude AS "address_longitude?",
               a.road AS "address_road?",
               a.city AS "address_city?",
               a.state AS "address_state?",
               a.postcode AS "address_postcode?",
               a.country AS "address_country?",
               a.raw AS "address_raw?: Value",
               cp.id AS "cost_profile_id?: Uuid",
               cp.name AS "cost_profile_name?",
               cp.billing_type AS "cost_profile_billing_type?",
               cp.rate AS "cost_profile_rate?",
               cp.session_fee AS "cost_profile_session_fee?",
               cp.currency AS "cost_profile_currency?",
               cp.timezone AS "cost_profile_timezone?",
               cp.tou_periods AS "cost_profile_tou_periods?: Value"
           FROM panda.geofences g
           LEFT JOIN panda.addresses a ON a.id = g.address_id
           LEFT JOIN panda.cost_profiles cp ON cp.id = g.cost_profile_id
           WHERE g.user_id = $1
             AND ($2::uuid IS NULL OR g.id = $2)
           ORDER BY g.name"#,
        user_id,
        place_id,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| {
            let place_name = row.name;

            PlaceRecord {
                id: row.id,
                name: place_name.clone(),
                latitude: row.latitude,
                longitude: row.longitude,
                radius_m: row.radius_m,
                is_home: row.is_home,
                is_work: row.is_work,
                address: row.address_display_name.map(|display_name| AddressRecord {
                    id: Some(
                        row.address_id
                            .expect("address id should exist when address details are present"),
                    ),
                    display_name,
                    osm_id: row.address_osm_id,
                    latitude: row.address_latitude.unwrap_or(row.latitude),
                    longitude: row.address_longitude.unwrap_or(row.longitude),
                    road: row.address_road,
                    city: row.address_city,
                    state: row.address_state,
                    postcode: row.address_postcode,
                    country: row.address_country,
                    raw: row.address_raw,
                }),
                charging: row
                    .cost_profile_id
                    .map(|cost_profile_id| ChargingProfileRecord {
                        id: cost_profile_id,
                        name: row.cost_profile_name.unwrap_or_else(|| place_name.clone()),
                        billing_type: normalize_place_billing_type(
                            row.cost_profile_billing_type
                                .as_deref()
                                .unwrap_or("per_kwh"),
                        )
                        .to_string(),
                        rate: row.cost_profile_rate.unwrap_or(0.0),
                        session_fee: row.cost_profile_session_fee.unwrap_or(0.0),
                        currency: row.cost_profile_currency.unwrap_or_else(|| "USD".into()),
                        timezone: row.cost_profile_timezone,
                        tou_periods: row
                            .cost_profile_tou_periods
                            .unwrap_or_else(|| serde_json::json!([])),
                    }),
            }
        })
        .collect())
}

async fn upsert_address(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    input: &AddressInput,
) -> Result<Uuid, AppError> {
    if let Some(osm_id) = input.osm_id {
        let address_id = sqlx::query_scalar!(
            r#"INSERT INTO panda.addresses
               (display_name, osm_id, latitude, longitude, road, city, state, postcode, country, raw)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
               ON CONFLICT (osm_id) DO UPDATE SET
                 display_name = EXCLUDED.display_name,
                 latitude = EXCLUDED.latitude,
                 longitude = EXCLUDED.longitude,
                 road = EXCLUDED.road,
                 city = EXCLUDED.city,
                 state = EXCLUDED.state,
                 postcode = EXCLUDED.postcode,
                 country = EXCLUDED.country,
                 raw = EXCLUDED.raw
               RETURNING id"#,
            input.display_name,
            osm_id,
            input.latitude,
            input.longitude,
            input.road,
            input.city,
            input.state,
            input.postcode,
            input.country,
            input.raw,
        )
        .fetch_one(&mut **tx)
        .await?;

        return Ok(address_id);
    }

    sqlx::query_scalar!(
        r#"INSERT INTO panda.addresses
           (display_name, latitude, longitude, road, city, state, postcode, country, raw)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
           RETURNING id"#,
        input.display_name,
        input.latitude,
        input.longitude,
        input.road,
        input.city,
        input.state,
        input.postcode,
        input.country,
        input.raw,
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(AppError::from)
}

async fn upsert_cost_profile(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: Uuid,
    existing_profile_id: Option<Uuid>,
    input: Option<&ChargingProfileInput>,
    place_name: &str,
) -> Result<Option<Uuid>, AppError> {
    let Some(input) = input else {
        return Ok(None);
    };

    validate_charging_input(input)?;
    let profile_name = input
        .name
        .clone()
        .unwrap_or_else(|| format!("{} charging", place_name.trim()));
    let session_fee = input.session_fee.unwrap_or(0.0);
    let currency = input.currency.clone().unwrap_or_else(|| "USD".into());
    let billing_type = normalize_place_billing_type(&input.billing_type).to_string();
    let tou_periods = input
        .tou_periods
        .clone()
        .unwrap_or_else(|| serde_json::json!([]));

    if let Some(profile_id) = existing_profile_id {
        let updated_id = sqlx::query_scalar!(
            r#"UPDATE panda.cost_profiles
               SET name = $3,
                   billing_type = $4,
                   rate = $5,
                   session_fee = $6,
                   currency = $7,
                   timezone = $8,
                   tou_periods = $9
               WHERE id = $1 AND user_id = $2
               RETURNING id"#,
            profile_id,
            user_id,
            profile_name,
            billing_type,
            input.rate,
            session_fee,
            currency,
            input.timezone,
            tou_periods,
        )
        .fetch_optional(&mut **tx)
        .await?;

        if updated_id.is_some() {
            return Ok(updated_id);
        }
    }

    let created_id = sqlx::query_scalar!(
        r#"INSERT INTO panda.cost_profiles
           (user_id, name, billing_type, rate, session_fee, currency, timezone, tou_periods)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
           RETURNING id"#,
        user_id,
        profile_name,
        billing_type,
        input.rate,
        session_fee,
        currency,
        input.timezone,
        tou_periods,
    )
    .fetch_one(&mut **tx)
    .await?;

    Ok(Some(created_id))
}

fn validate_place_body(body: &PlaceBody) -> Result<(), AppError> {
    if body.name.trim().is_empty() {
        return Err(AppError::Validation("place name is required".into()));
    }
    if body.address.display_name.trim().is_empty() {
        return Err(AppError::Validation(
            "an address selection is required".into(),
        ));
    }
    if !body.address.latitude.is_finite() || !body.address.longitude.is_finite() {
        return Err(AppError::Validation(
            "address coordinates must be valid numbers".into(),
        ));
    }
    if let Some(radius_m) = body.radius_m {
        if !radius_m.is_finite() || radius_m <= 0.0 {
            return Err(AppError::Validation(
                "radius_m must be a positive number".into(),
            ));
        }
    }
    if let Some(charging) = &body.charging {
        validate_charging_input(charging)?;
    }
    Ok(())
}

fn validate_charging_input(input: &ChargingProfileInput) -> Result<(), AppError> {
    let billing_type = normalize_place_billing_type(&input.billing_type);

    match billing_type {
        "per_kwh" | "tou" => {}
        _ => {
            return Err(AppError::Validation(
                "billing_type must be either per_kwh or tou for places".into(),
            ))
        }
    }

    if !input.rate.is_finite() || input.rate < 0.0 {
        return Err(AppError::Validation(
            "rate must be a non-negative number".into(),
        ));
    }

    if billing_type == "tou" {
        if input
            .timezone
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
        {
            return Err(AppError::Validation(
                "timezone is required for TOU charging plans".into(),
            ));
        }

        let periods: Vec<TouPeriod> = serde_json::from_value(
            input
                .tou_periods
                .clone()
                .unwrap_or_else(|| serde_json::json!([])),
        )
        .map_err(|_| AppError::Validation("tou_periods must be a JSON array".into()))?;

        validate_tou_periods(&periods).map_err(AppError::Validation)?;
    }

    Ok(())
}

fn value_to_address_record(raw: Value) -> Option<AddressRecord> {
    let latitude = raw.get("lat")?.as_str()?.parse().ok()?;
    let longitude = raw.get("lon")?.as_str()?.parse().ok()?;
    let display_name = raw.get("display_name")?.as_str()?.to_string();
    let address = raw.get("address").and_then(Value::as_object);

    Some(AddressRecord {
        id: None,
        display_name,
        osm_id: raw.get("osm_id").and_then(Value::as_i64),
        latitude,
        longitude,
        road: address
            .and_then(|value| {
                value
                    .get("road")
                    .or_else(|| value.get("pedestrian"))
                    .or_else(|| value.get("footway"))
            })
            .and_then(Value::as_str)
            .map(str::to_string),
        city: address
            .and_then(|value| {
                value
                    .get("city")
                    .or_else(|| value.get("town"))
                    .or_else(|| value.get("village"))
            })
            .and_then(Value::as_str)
            .map(str::to_string),
        state: address
            .and_then(|value| value.get("state"))
            .and_then(Value::as_str)
            .map(str::to_string),
        postcode: address
            .and_then(|value| value.get("postcode"))
            .and_then(Value::as_str)
            .map(str::to_string),
        country: address
            .and_then(|value| value.get("country"))
            .and_then(Value::as_str)
            .map(str::to_string),
        raw: Some(raw),
    })
}

#[cfg(test)]
mod tests {
    use super::{search_cache_key, SYNC_LINKED_SESSION_COST_PROFILE_SQL};

    #[test]
    fn persistent_search_key_does_not_contain_address_text() {
        let key = search_cache_key("123 Main Street, Austin", 5);
        assert!(key.starts_with("external:nominatim:search:"));
        assert!(!key.contains("Main"));
        assert_ne!(key, search_cache_key("124 Main Street, Austin", 5));
    }

    #[test]
    fn place_pricing_edits_replace_or_clear_linked_automatic_session_profiles() {
        assert!(SYNC_LINKED_SESSION_COST_PROFILE_SQL.contains("SET cost_profile_id=$2"));
        assert!(SYNC_LINKED_SESSION_COST_PROFILE_SQL.contains("geofence_id=$1"));
        assert!(SYNC_LINKED_SESSION_COST_PROFILE_SQL.contains("cost_override_mode='automatic'"));
        assert!(SYNC_LINKED_SESSION_COST_PROFILE_SQL.contains("RETURNING id"));
    }
}

#[cfg(test)]
#[path = "__tests__/places_profiles/contracts.rs"]
mod places_profiles_contracts;

#[path = "places/search.rs"]
mod search;
#[path = "places/workflow.rs"]
mod workflow;
