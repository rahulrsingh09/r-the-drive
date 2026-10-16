use super::*;

enum LocationChange {
    Preserve,
    SavedPlace(Uuid),
    Clear,
    Automatic,
}
struct CostChange {
    mode: CostOverrideMode,
    amount: Option<f64>,
}
struct SessionMutation {
    location: LocationChange,
    cost: Option<CostChange>,
}

fn prepare(payload: SessionLocationUpdate) -> Result<SessionMutation, AppError> {
    let location_mode = payload.location_mode.or(match payload.place_id {
        PatchField::Value(_) => Some(LocationOverrideMode::SavedPlace),
        PatchField::Null => Some(LocationOverrideMode::None),
        PatchField::Absent => None,
    });

    let requested_place_id = match payload.place_id {
        PatchField::Value(id) => Some(id),
        PatchField::Absent | PatchField::Null => None,
    };
    if location_mode == Some(LocationOverrideMode::SavedPlace) && requested_place_id.is_none() {
        return Err(AppError::Validation(
            "place_id required when location_mode is saved_place".into(),
        ));
    }
    if payload.cost_mode == Some(CostOverrideMode::Manual)
        && !payload
            .cost_usd
            .is_some_and(|value| value.is_finite() && value >= 0.0)
    {
        return Err(AppError::Validation(
            "cost_usd must be a non-negative number when cost_mode is manual".into(),
        ));
    }
    if payload.cost_mode != Some(CostOverrideMode::Manual) && payload.cost_usd.is_some() {
        return Err(AppError::Validation(
            "cost_usd is only valid when cost_mode is manual".into(),
        ));
    }

    let location = match location_mode {
        Some(LocationOverrideMode::SavedPlace) => {
            LocationChange::SavedPlace(requested_place_id.expect("validated saved place id"))
        }
        Some(LocationOverrideMode::None) => LocationChange::Clear,
        Some(LocationOverrideMode::Automatic) => LocationChange::Automatic,
        None => LocationChange::Preserve,
    };
    let cost = payload.cost_mode.map(|mode| CostChange {
        mode,
        amount: if mode == CostOverrideMode::Manual {
            payload.cost_usd
        } else {
            None
        },
    });
    Ok(SessionMutation { location, cost })
}

async fn apply_location(
    pool: &sqlx::PgPool,
    auth: &AuthUser,
    vehicle_id: Uuid,
    session_id: Uuid,
    location: LocationChange,
) -> Result<Option<Uuid>, AppError> {
    let updated_session_id = match location {
 LocationChange::SavedPlace(place_id) => {
        let place = sqlx::query_as::<_, PlaceLookupRow>(
            "SELECT address_id, is_home, latitude, longitude, cost_profile_id
               FROM panda.geofences
              WHERE id=$1 AND user_id=$2",
        )
        .bind(place_id)
        .bind(auth.user_id)
        .fetch_optional(pool)
        .await?
        .ok_or(AppError::NotFound)?;

        sqlx::query_scalar::<_, Uuid>(
            "UPDATE panda.charge_sessions
               SET geofence_id=$1,
                   address_id=$2,
                   is_home=$3,
                   location_lat=$4,
                   location_lng=$5,
                   cost_profile_id=$6,
                   location_override_mode='saved_place'
             WHERE id=$7 AND vehicle_id=$8
             RETURNING id",
        )
        .bind(place_id)
        .bind(place.address_id)
        .bind(place.is_home)
        .bind(place.latitude)
        .bind(place.longitude)
        .bind(cost_profile_for_location(
            LocationOverrideMode::SavedPlace,
            place.cost_profile_id,
        ))
        .bind(session_id)
        .bind(vehicle_id)
        .fetch_optional(pool)
        .await?
    }, LocationChange::Clear => {
        sqlx::query_scalar::<_, Uuid>(
            "UPDATE panda.charge_sessions
               SET geofence_id=NULL,
                   address_id=NULL,
                   is_home=NULL,
                   location_lat=NULL,
                   location_lng=NULL,
                   cost_profile_id=$1,
                   location_override_mode='none'
             WHERE id=$2 AND vehicle_id=$3
             RETURNING id",
        )
        .bind(cost_profile_for_location(LocationOverrideMode::None, None))
        .bind(session_id)
        .bind(vehicle_id)
        .fetch_optional(pool)
        .await?
    }, LocationChange::Automatic => {
        sqlx::query_scalar::<_, Uuid>(
            r#"UPDATE panda.charge_sessions cs
                SET geofence_id=(
                      SELECT g.id FROM panda.geofences g
                       WHERE g.user_id=$3 AND cs.source_location_lat IS NOT NULL AND cs.source_location_lng IS NOT NULL
                         AND earth_box(ll_to_earth(g.latitude, g.longitude), g.radius_m)
                             @> ll_to_earth(cs.source_location_lat, cs.source_location_lng)
                         AND earth_distance(ll_to_earth(g.latitude, g.longitude), ll_to_earth(cs.source_location_lat, cs.source_location_lng)) <= g.radius_m
                       ORDER BY earth_distance(ll_to_earth(g.latitude, g.longitude), ll_to_earth(cs.source_location_lat, cs.source_location_lng)) ASC LIMIT 1),
                    address_id=(
                      SELECT g.address_id FROM panda.geofences g
                       WHERE g.user_id=$3 AND cs.source_location_lat IS NOT NULL AND cs.source_location_lng IS NOT NULL
                         AND earth_box(ll_to_earth(g.latitude, g.longitude), g.radius_m) @> ll_to_earth(cs.source_location_lat, cs.source_location_lng)
                         AND earth_distance(ll_to_earth(g.latitude, g.longitude), ll_to_earth(cs.source_location_lat, cs.source_location_lng)) <= g.radius_m
                       ORDER BY earth_distance(ll_to_earth(g.latitude, g.longitude), ll_to_earth(cs.source_location_lat, cs.source_location_lng)) ASC LIMIT 1),
                    is_home=(
                      SELECT g.is_home FROM panda.geofences g
                       WHERE g.user_id=$3 AND cs.source_location_lat IS NOT NULL AND cs.source_location_lng IS NOT NULL
                         AND earth_box(ll_to_earth(g.latitude, g.longitude), g.radius_m) @> ll_to_earth(cs.source_location_lat, cs.source_location_lng)
                         AND earth_distance(ll_to_earth(g.latitude, g.longitude), ll_to_earth(cs.source_location_lat, cs.source_location_lng)) <= g.radius_m
                       ORDER BY earth_distance(ll_to_earth(g.latitude, g.longitude), ll_to_earth(cs.source_location_lat, cs.source_location_lng)) ASC LIMIT 1),
                    location_lat=cs.source_location_lat, location_lng=cs.source_location_lng,
                    cost_profile_id=$4,
                    location_override_mode='automatic'
              WHERE cs.id=$1 AND cs.vehicle_id=$2 RETURNING cs.id"#,
        )
        .bind(session_id)
        .bind(vehicle_id)
        .bind(auth.user_id)
        .bind(cost_profile_for_location(LocationOverrideMode::Automatic, None))
        .fetch_optional(pool)
        .await?
    }, LocationChange::Preserve => {
        sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM panda.charge_sessions WHERE id=$1 AND vehicle_id=$2",
        )
        .bind(session_id)
        .bind(vehicle_id)
        .fetch_optional(pool)
        .await?
    }};

    Ok(updated_session_id)
}

pub(super) async fn update(
    pool: &sqlx::PgPool,
    auth: &AuthUser,
    vehicle_id: Uuid,
    session_id: Uuid,
    payload: SessionLocationUpdate,
) -> Result<(), AppError> {
    require_vehicle_manager_access(pool, auth, vehicle_id).await?;
    let SessionMutation { location, cost } = prepare(payload)?;
    let found = apply_location(pool, auth, vehicle_id, session_id, location).await?;
    if let Some(cost) = cost {
        sqlx::query("UPDATE panda.charge_sessions SET cost_override_mode=$1,cost_override_usd=$2 WHERE id=$3 AND vehicle_id=$4")
            .bind(cost.mode.as_str()).bind(cost.amount).bind(session_id).bind(vehicle_id).execute(pool).await?;
    }
    if found.is_none() {
        return Err(AppError::NotFound);
    }
    recompute_charge_session_cost(pool, session_id)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(())
}

pub(super) async fn network(
    pool: &sqlx::PgPool,
    auth: &AuthUser,
    vehicle_id: Uuid,
    vendor: String,
    payload: NetworkPreferenceUpdate,
) -> Result<serde_json::Value, AppError> {
    require_vehicle_manager_access(pool, auth, vehicle_id).await?;
    let normalized = vendor.trim().to_lowercase();
    if normalized.is_empty() {
        return Err(AppError::Validation("network vendor required".into()));
    }
    let observed: Option<String> = sqlx::query_scalar(
        "SELECT MIN(trim(network_vendor)) FROM panda.charge_sessions
          WHERE vehicle_id=$1 AND lower(trim(network_vendor))=$2",
    )
    .bind(vehicle_id)
    .bind(&normalized)
    .fetch_one(pool)
    .await?;
    let Some(observed) = observed else {
        return Err(AppError::NotFound);
    };

    let plan = NetworkMutation {
        normalized,
        observed,
        mode: payload.cost_mode,
    };
    plan.persist_and_reprice(pool, vehicle_id).await
}

struct NetworkMutation {
    normalized: String,
    observed: String,
    mode: NetworkCostMode,
}
impl NetworkMutation {
    async fn persist_and_reprice(
        self,
        pool: &sqlx::PgPool,
        vehicle_id: Uuid,
    ) -> Result<serde_json::Value, AppError> {
        let Self {
            normalized,
            observed,
            mode,
        } = self;
        sqlx::query(
        r#"INSERT INTO panda.vehicle_charging_network_preferences
              (vehicle_id, vendor_normalized, network_vendor, cost_mode)
           VALUES ($1,$2,$3,$4)
           ON CONFLICT (vehicle_id, vendor_normalized) DO UPDATE
             SET network_vendor=EXCLUDED.network_vendor, cost_mode=EXCLUDED.cost_mode, updated_at=now()"#,
    )
    .bind(vehicle_id)
    .bind(&normalized)
    .bind(&observed)
    .bind(mode.as_str())
    .execute(pool)
    .await?;

        let sessions = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM panda.charge_sessions
          WHERE vehicle_id=$1 AND lower(trim(COALESCE(network_vendor, '')))=$2
            AND cost_override_mode='automatic'",
        )
        .bind(vehicle_id)
        .bind(&normalized)
        .fetch_all(pool)
        .await?;
        for id in sessions {
            recompute_charge_session_cost(pool, id).await?;
        }
        Ok(serde_json::json!({ "network_vendor": observed, "cost_mode": mode.as_str() }))
    }
}
