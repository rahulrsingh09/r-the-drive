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
    super::route_table([
        ("/places", get(list_places).post(create_place)),
        ("/places/search", get(search_places)),
        (
            "/places/{id}",
            get(get_place).put(update_place).delete(delete_place),
        ),
    ])
}

#[derive(Debug, Serialize, Deserialize)]
struct PlaceRecord {
    id: Uuid,
    name: String,
    #[serde(deserialize_with = "database_float")]
    latitude: f64,
    #[serde(deserialize_with = "database_float")]
    longitude: f64,
    #[serde(deserialize_with = "database_float")]
    radius_m: f64,
    is_home: bool,
    is_work: bool,
    address: Option<AddressRecord>,
    charging: Option<ChargingProfileRecord>,
}

#[derive(Debug, Serialize, Deserialize)]
struct AddressRecord {
    id: Option<Uuid>,
    display_name: String,
    osm_id: Option<i64>,
    #[serde(deserialize_with = "database_float")]
    latitude: f64,
    #[serde(deserialize_with = "database_float")]
    longitude: f64,
    road: Option<String>,
    city: Option<String>,
    state: Option<String>,
    postcode: Option<String>,
    country: Option<String>,
    raw: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ChargingProfileRecord {
    id: Uuid,
    name: String,
    billing_type: String,
    #[serde(deserialize_with = "database_float")]
    rate: f64,
    #[serde(deserialize_with = "database_float")]
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
    let places = fetch_places(&state.pool, auth.user_id, None).await?;
    Ok(Json(PlacesResponse { places }))
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
    let rows: Vec<Value> = sqlx::query_scalar(
        "SELECT jsonb_build_object(
          'id',g.id,'name',g.name,'latitude',g.latitude,'longitude',g.longitude,'radius_m',g.radius_m,
          'is_home',g.is_home,'is_work',g.is_work,
          'address',CASE WHEN a.display_name IS NULL THEN NULL ELSE jsonb_build_object(
            'id',a.id,'display_name',a.display_name,'osm_id',a.osm_id,
            'latitude',COALESCE(a.latitude,g.latitude),'longitude',COALESCE(a.longitude,g.longitude),
            'road',a.road,'city',a.city,'state',a.state,'postcode',a.postcode,'country',a.country,'raw',a.raw) END,
          'charging',CASE WHEN cp.id IS NULL THEN NULL ELSE jsonb_build_object(
            'id',cp.id,'name',COALESCE(cp.name,g.name),
            'billing_type',CASE WHEN cp.billing_type='flat' THEN 'per_kwh' ELSE COALESCE(cp.billing_type,'per_kwh') END,
            'rate',COALESCE(cp.rate,0.0),'session_fee',COALESCE(cp.session_fee,0.0),
            'currency',COALESCE(cp.currency,'USD'),'timezone',cp.timezone,'tou_periods',COALESCE(cp.tou_periods,'[]'::jsonb)) END)
         FROM panda.geofences g LEFT JOIN panda.addresses a ON a.id=g.address_id
         LEFT JOIN panda.cost_profiles cp ON cp.id=g.cost_profile_id
         WHERE g.user_id=$1 AND ($2::uuid IS NULL OR g.id=$2) ORDER BY g.name")
        .bind(user_id).bind(place_id).fetch_all(pool).await?;
    rows.into_iter()
        .map(|row| serde_json::from_value(row).map_err(|error| AppError::Internal(error.into())))
        .collect()
}

async fn upsert_address(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    input: &AddressInput,
) -> Result<Uuid, AppError> {
    sqlx::query_scalar(
        "INSERT INTO panda.addresses (display_name,osm_id,latitude,longitude,road,city,state,postcode,country,raw)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)
         ON CONFLICT (osm_id) DO UPDATE SET
         display_name=EXCLUDED.display_name,latitude=EXCLUDED.latitude,longitude=EXCLUDED.longitude,
         road=EXCLUDED.road,city=EXCLUDED.city,state=EXCLUDED.state,postcode=EXCLUDED.postcode,country=EXCLUDED.country,raw=EXCLUDED.raw
         RETURNING id")
        .bind(&input.display_name).bind(input.osm_id).bind(input.latitude).bind(input.longitude)
        .bind(&input.road).bind(&input.city).bind(&input.state).bind(&input.postcode).bind(&input.country).bind(&input.raw)
        .fetch_one(&mut **tx).await.map_err(AppError::from)
}

async fn upsert_cost_profile(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: Uuid,
    existing_profile_id: Option<Uuid>,
    input: Option<&ChargingProfileInput>,
    place_name: &str,
) -> Result<Option<Uuid>, AppError> {
    let Some(input) = input else { return Ok(None) };
    validate_charging_input(input)?;
    let name = input
        .name
        .clone()
        .unwrap_or_else(|| format!("{} charging", place_name.trim()));
    let currency = input.currency.as_deref().unwrap_or("USD");
    let periods = input
        .tou_periods
        .clone()
        .unwrap_or_else(|| serde_json::json!([]));
    let id: Uuid = sqlx::query_scalar(
        "WITH updated AS (
           UPDATE panda.cost_profiles SET name=$3,billing_type=$4,rate=$5,session_fee=$6,currency=$7,timezone=$8,tou_periods=$9
           WHERE id=$1 AND user_id=$2 RETURNING id
         ), inserted AS (
           INSERT INTO panda.cost_profiles (user_id,name,billing_type,rate,session_fee,currency,timezone,tou_periods)
           SELECT $2,$3,$4,$5,$6,$7,$8,$9 WHERE NOT EXISTS (SELECT 1 FROM updated) RETURNING id
         ) SELECT id FROM updated UNION ALL SELECT id FROM inserted")
        .bind(existing_profile_id).bind(user_id).bind(name)
        .bind(normalize_place_billing_type(&input.billing_type)).bind(input.rate)
        .bind(input.session_fee.unwrap_or(0.0)).bind(currency).bind(&input.timezone).bind(periods)
        .fetch_one(&mut **tx).await?;
    Ok(Some(id))
}

fn validate_place_body(body: &PlaceBody) -> Result<(), AppError> {
    let checks = [
        (body.name.trim().is_empty(), "place name is required"),
        (
            body.address.display_name.trim().is_empty(),
            "an address selection is required",
        ),
        (
            ![body.address.latitude, body.address.longitude]
                .into_iter()
                .all(f64::is_finite),
            "address coordinates must be valid numbers",
        ),
        (
            body.radius_m
                .is_some_and(|radius| !radius.is_finite() || radius <= 0.0),
            "radius_m must be a positive number",
        ),
    ];
    if let Some((_, message)) = checks.into_iter().find(|(invalid, _)| *invalid) {
        return Err(AppError::Validation(message.into()));
    }
    body.charging
        .as_ref()
        .map(validate_charging_input)
        .transpose()
        .map(|_| ())
}

fn validate_charging_input(input: &ChargingProfileInput) -> Result<(), AppError> {
    let billing = normalize_place_billing_type(&input.billing_type);
    if !["per_kwh", "tou"].contains(&billing) {
        return Err(AppError::Validation(
            "billing_type must be either per_kwh or tou for places".into(),
        ));
    }
    let rate = input.rate;
    if !rate.is_finite() || rate < 0.0 {
        return Err(AppError::Validation(
            "rate must be a non-negative number".into(),
        ));
    }
    match billing {
        "tou" => validate_place_schedule(input),
        _ => Ok(()),
    }
}

fn value_to_address_record(raw: Value) -> Option<AddressRecord> {
    let number = |key| raw.get(key)?.as_str()?.parse().ok();
    let latitude = number("lat")?;
    let longitude = number("lon")?;
    let display_name = raw.get("display_name")?.as_str()?.to_owned();
    let address = raw.get("address").and_then(Value::as_object);
    let field = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| address.and_then(|fields| fields.get(*key)))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let [road, city, state, postcode, country] = [
        &["road", "pedestrian", "footway"][..],
        &["city", "town", "village"],
        &["state"],
        &["postcode"],
        &["country"],
    ]
    .map(field);
    Some(AddressRecord {
        id: None,
        display_name,
        osm_id: raw.get("osm_id").and_then(Value::as_i64),
        latitude,
        longitude,
        road,
        city,
        state,
        postcode,
        country,
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

fn validate_place_schedule(input: &ChargingProfileInput) -> Result<(), AppError> {
    if input
        .timezone
        .as_deref()
        .unwrap_or_default()
        .trim()
        .is_empty()
    {
        return Err(AppError::Validation(
            "timezone is required for TOU charging plans".into(),
        ));
    }
    let value = input
        .tou_periods
        .as_ref()
        .cloned()
        .unwrap_or_else(|| serde_json::json!([]));
    serde_json::from_value::<Vec<TouPeriod>>(value)
        .map_err(|_| AppError::Validation("tou_periods must be a JSON array".into()))
        .and_then(|periods| validate_tou_periods(&periods).map_err(AppError::Validation))
}

fn database_float<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Number {
        Finite(f64),
        NonFinite(String),
    }
    match Number::deserialize(deserializer)? {
        Number::Finite(value) => Ok(value),
        Number::NonFinite(value) => match value.as_str() {
            "NaN" => Ok(f64::NAN),
            "Infinity" => Ok(f64::INFINITY),
            "-Infinity" => Ok(f64::NEG_INFINITY),
            _ => Err(serde::de::Error::custom(
                "unexpected database floating point value",
            )),
        },
    }
}

#[cfg(test)]
mod full_replacement_database_numbers {
    use super::*;
    #[test]
    fn database_nonfinite_numbers_keep_the_existing_json_null_contract() {
        for rate in [
            serde_json::json!(0.5),
            serde_json::json!("NaN"),
            serde_json::json!("Infinity"),
            serde_json::json!("-Infinity"),
        ] {
            let row: ChargingProfileRecord = serde_json::from_value(serde_json::json!({
                "id":Uuid::nil(), "name":"fixture", "billing_type":"per_kwh", "rate":rate,
                "session_fee":0.0, "currency":"USD", "timezone":null, "tou_periods":[]
            }))
            .unwrap();
            let expected = if rate.is_number() { rate } else { Value::Null };
            assert_eq!(serde_json::to_value(row).unwrap()["rate"], expected);
        }
    }
}
