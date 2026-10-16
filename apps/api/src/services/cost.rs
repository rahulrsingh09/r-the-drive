//! Cost profile resolution service.
//!
//! Resolution priority is explicit session correction, configured network,
//! upstream Rivian amount/free status, then the normal profile resolution.

use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    db::vehicles::get_vehicle_owner_id,
    models::cost_profile::{
        compute_cost, compute_tou_cost_from_readings, CostProfile, TimedEnergyPoint,
    },
};

#[derive(Debug, Clone, PartialEq)]
pub struct ChargeCostComputation {
    pub cost_profile_id: Option<Uuid>,
    pub cost_method: String,
    pub cost_usd: Option<f64>,
}

#[derive(Debug, sqlx::FromRow)]
struct ChargeSessionCostRow {
    id: Uuid,
    vehicle_id: Uuid,
    geofence_id: Option<Uuid>,
    cost_profile_id: Option<Uuid>,
    started_at: DateTime<Utc>,
    ended_at: Option<DateTime<Utc>>,
    duration_minutes: Option<i32>,
    kwh_added: Option<f64>,
    energy_added_wh: Option<f64>,
    energy_used_wh: Option<f64>,
    rivian_paid_total: Option<f64>,
    is_rivian_network: Option<bool>,
    is_home: Option<bool>,
    is_free_session: Option<bool>,
    network_cost_mode: Option<String>,
    cost_override_mode: String,
    cost_override_usd: Option<f64>,
}

/// Inputs for a pure cost computation — no DB required.
/// Used by both `recompute_charge_session_cost` and unit tests.
pub struct CostInputs {
    pub is_rivian_network: Option<bool>,
    pub is_home: Option<bool>,
    pub is_free_session: Option<bool>,
    pub network_cost_mode: Option<String>,
    pub cost_override_mode: String,
    pub cost_override_usd: Option<f64>,
    pub rivian_paid_total: Option<f64>,
    pub energy_added_kwh: Option<f64>,
    pub energy_used_kwh: Option<f64>,
    pub duration_minutes: i32,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    /// Per-interval telemetry readings used for accurate TOU attribution.
    /// When non-empty and billing_type is "tou", cost is computed by summing
    /// `kwh × rate` for each actual measured interval rather than
    /// time-weighting the session window. An empty vec falls back to
    /// the time-weighted method (used when telemetry is unavailable).
    pub timed_kwh_readings: Vec<TimedEnergyPoint>,
}

/// Pure computation: given session inputs and an optional cost profile, return
/// the cost method, resolved profile id, and computed cost_usd.
///
/// `rivian_paid_total` is only treated as authoritative when **both**:
///   - `is_rivian_network == Some(true)` (the inference upstream is now
///     strict: only true when the API gives an explicit known vendor name).
///   - `is_home != Some(true)` (defensive belt-and-suspenders: even if a
///     future upstream change misidentifies a home session as a network
///     session, we will never silently overwrite a profile-computed cost
///     with whatever `paidTotal` Rivian's API returns for that row).
///
/// `is_public` is intentionally excluded from this decision: Rivian returns
/// `isPublic=true` for home sessions too (it means "visible to account
/// owner," not "you paid for it").
pub fn apply_cost_inputs(
    inputs: &CostInputs,
    profile: Option<&CostProfile>,
) -> ChargeCostComputation {
    CostBasis::select(inputs, profile).evaluate(inputs)
}

enum CostBasis<'profile> {
    Fixed {
        method: &'static str,
        amount: Option<f64>,
    },
    Provider {
        amount: f64,
        profile_id: Option<Uuid>,
    },
    Profile(&'profile CostProfile),
    Unknown,
}

impl<'profile> CostBasis<'profile> {
    fn select(inputs: &CostInputs, profile: Option<&'profile CostProfile>) -> Self {
        let fixed = match inputs.cost_override_mode.as_str() {
            "free" => Some(("user_free", Some(0.0))),
            "manual" => Some(("user_manual", inputs.cost_override_usd)),
            _ if inputs.network_cost_mode.as_deref() == Some("free") => {
                Some(("network_free", Some(0.0)))
            }
            _ if inputs.is_free_session == Some(true) => Some(("rivian_free_session", Some(0.0))),
            _ => None,
        };
        if let Some((method, amount)) = fixed {
            return Self::Fixed { method, amount };
        }
        if inputs.is_rivian_network == Some(true) && inputs.is_home != Some(true) {
            if let Some(amount) = inputs.rivian_paid_total {
                return Self::Provider {
                    amount,
                    profile_id: profile.map(|profile| profile.id),
                };
            }
        }
        profile.map_or(Self::Unknown, Self::Profile)
    }

    fn evaluate(self, inputs: &CostInputs) -> ChargeCostComputation {
        let (cost_profile_id, method, cost_usd) = match self {
            Self::Fixed { method, amount } => (None, method, amount),
            Self::Provider { amount, profile_id } => {
                (profile_id, "rivian_paid_total", Some(amount))
            }
            Self::Unknown => (None, "unknown", None),
            Self::Profile(profile) => {
                let amount =
                    if profile.billing_type == "tou" && !inputs.timed_kwh_readings.is_empty() {
                        compute_tou_cost_from_readings(profile, &inputs.timed_kwh_readings)
                    } else {
                        compute_cost(
                            profile,
                            inputs.energy_added_kwh,
                            inputs.energy_used_kwh,
                            inputs.duration_minutes,
                            inputs.started_at,
                            inputs.ended_at,
                        )
                    };
                let method = if amount.is_some() {
                    "profile"
                } else {
                    "profile_pending"
                };
                (Some(profile.id), method, amount)
            }
        };
        ChargeCostComputation {
            cost_profile_id,
            cost_method: method.into(),
            cost_usd,
        }
    }
}

/// Resolve an effective pinned, geofence-linked, or vehicle-owner default profile.
/// Explicit and geofence IDs retain the caller's authorization boundary.
pub async fn resolve_profile(
    pool: &PgPool,
    explicit_id: Option<Uuid>,
    geofence_id: Option<Uuid>,
    vehicle_id: Uuid,
    started_at: DateTime<Utc>,
) -> Result<Option<CostProfile>> {
    let sources = [
        ProfileSource::Pinned(explicit_id),
        ProfileSource::Geofence(geofence_id),
        ProfileSource::VehicleOwner(vehicle_id),
    ];
    for source in sources {
        let Some(id) = source.profile_id(pool).await? else {
            continue;
        };
        let Some(profile) = fetch_profile(pool, id).await? else {
            continue;
        };
        if profile.is_effective_at(started_at) {
            return Ok(Some(profile));
        }
    }
    Ok(None)
}

enum ProfileSource {
    Pinned(Option<Uuid>),
    Geofence(Option<Uuid>),
    VehicleOwner(Uuid),
}

impl ProfileSource {
    async fn profile_id(self, pool: &PgPool) -> Result<Option<Uuid>> {
        match self {
            Self::Pinned(id) => Ok(id),
            Self::Geofence(None) => Ok(None),
            Self::Geofence(Some(id)) => Ok(sqlx::query_scalar::<_, Option<Uuid>>(
                "SELECT cost_profile_id FROM panda.geofences WHERE id = $1",
            )
            .bind(id)
            .fetch_optional(pool)
            .await?
            .flatten()),
            Self::VehicleOwner(vehicle_id) => {
                let Some(owner_id) = get_vehicle_owner_id(pool, vehicle_id).await? else {
                    return Ok(None);
                };
                Ok(sqlx::query_scalar::<_, Option<Uuid>>(
                    "SELECT default_cost_profile_id
             FROM panda.vehicle_user_settings
             WHERE vehicle_id = $1 AND user_id = $2",
                )
                .bind(vehicle_id)
                .bind(owner_id)
                .fetch_optional(pool)
                .await?
                .flatten())
            }
        }
    }
}

#[derive(Debug, sqlx::FromRow)]
struct TelemetryEnergyRow {
    ts: DateTime<Utc>,
    energy_kwh: f64,
}

/// Fetch per-interval energy readings for a charging session.
///
/// Primary source: `timeseries.telemetry_1min` — 1-minute SOC-delta buckets.
/// Fallback: `panda.rivian_charge_curve_points` — Rivian API backfill
///   power samples, converted from kW to kWh (÷ 60 for one minute).
///
/// Returns an empty vec when no useful data is found; callers fall back to
/// the time-weighted TOU method in that case.
async fn fetch_session_energy_readings(
    pool: &PgPool,
    vehicle_id: Uuid,
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
    session_id: Uuid,
) -> Result<Vec<TimedEnergyPoint>> {
    let rows = sqlx::query_as::<_, TelemetryEnergyRow>(
        r#"WITH samples AS (
               SELECT bucket,
                      avg_soc,
                      MAX(battery_capacity_wh) OVER () AS cap_wh
               FROM timeseries.telemetry_1min
               WHERE vehicle_id = $1
                 AND bucket >= $2
                 AND bucket <= $3
                 AND avg_soc IS NOT NULL
           ),
           deltas AS (
               SELECT bucket AS ts,
                      GREATEST(0.0,
                          (avg_soc - LAG(avg_soc) OVER (ORDER BY bucket))
                              / 100.0 * cap_wh / 1000.0
                      ) AS energy_kwh
               FROM samples
           )
           SELECT ts, energy_kwh
           FROM deltas
           WHERE energy_kwh > 0
           ORDER BY ts"#,
    )
    .bind(vehicle_id)
    .bind(started_at)
    .bind(ended_at)
    .fetch_all(pool)
    .await?;

    if !rows.is_empty() {
        return Ok(rows
            .into_iter()
            .map(|r| TimedEnergyPoint {
                ts: r.ts,
                kwh: r.energy_kwh,
            })
            .collect());
    }

    // Fallback: Rivian API backfill curve points stored during polling.
    // power_kw is instantaneous power; dividing by 60 gives kWh for that
    // 1-minute sample interval.
    let fallback = sqlx::query_as::<_, TelemetryEnergyRow>(
        r#"SELECT ts,
                  GREATEST(0.0, power_kw) / 60.0 AS energy_kwh
           FROM panda.rivian_charge_curve_points
           WHERE charge_session_id = $1
             AND power_kw IS NOT NULL
             AND power_kw > 0
           ORDER BY ts"#,
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;

    Ok(fallback
        .into_iter()
        .map(|r| TimedEnergyPoint {
            ts: r.ts,
            kwh: r.energy_kwh,
        })
        .collect())
}

pub async fn recompute_charge_session_cost(
    pool: &PgPool,
    session_id: Uuid,
) -> Result<Option<ChargeCostComputation>> {
    let Some(session) = load_cost_session(pool, session_id).await? else {
        return Ok(None);
    };
    let profile = resolve_profile(
        pool,
        session.cost_profile_id,
        session.geofence_id,
        session.vehicle_id,
        session.started_at,
    )
    .await?;
    let readings = collect_cost_evidence(pool, &session).await;
    let inputs = session.computation_inputs(readings);
    let result = apply_cost_inputs(&inputs, profile.as_ref());
    persist_cost_result(pool, &session, &result).await?;
    Ok(Some(result))
}

async fn load_cost_session(
    pool: &PgPool,
    session_id: Uuid,
) -> Result<Option<ChargeSessionCostRow>> {
    let session = sqlx::query_as::<_, ChargeSessionCostRow>(
        r#"SELECT cs.id, cs.vehicle_id, cs.geofence_id, cs.cost_profile_id, cs.started_at, cs.ended_at,
                  duration_minutes, kwh_added, energy_added_wh, energy_used_wh,
                  rivian_paid_total, is_rivian_network, is_home, is_free_session,
                  COALESCE(pref.cost_mode, 'automatic') AS network_cost_mode,
                  cs.cost_override_mode, cs.cost_override_usd
           FROM panda.charge_sessions cs
           LEFT JOIN panda.vehicle_charging_network_preferences pref
             ON pref.vehicle_id = cs.vehicle_id
            AND pref.vendor_normalized = lower(trim(COALESCE(cs.network_vendor, '')))
           WHERE cs.id = $1"#,
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await?;
    Ok(session)
}

impl ChargeSessionCostRow {
    fn computation_inputs(&self, timed_kwh_readings: Vec<TimedEnergyPoint>) -> CostInputs {
        let duration_minutes = self.duration_minutes.unwrap_or_else(|| {
            self.ended_at
                .map(|end| ((end - self.started_at).num_seconds() / 60) as i32)
                .unwrap_or(0)
                .max(0)
        });
        CostInputs {
            is_rivian_network: self.is_rivian_network,
            is_home: self.is_home,
            is_free_session: self.is_free_session,
            network_cost_mode: self.network_cost_mode.clone(),
            cost_override_mode: self.cost_override_mode.clone(),
            cost_override_usd: self.cost_override_usd,
            rivian_paid_total: self.rivian_paid_total,
            energy_added_kwh: self
                .energy_added_wh
                .map(|wh| wh / 1000.0)
                .or(self.kwh_added),
            energy_used_kwh: self.energy_used_wh.map(|wh| wh / 1000.0),
            duration_minutes,
            started_at: self.started_at,
            ended_at: self.ended_at,
            timed_kwh_readings,
        }
    }
}

async fn collect_cost_evidence(
    pool: &PgPool,
    session: &ChargeSessionCostRow,
) -> Vec<TimedEnergyPoint> {
    if let Some(ended_at) = session.ended_at {
        match fetch_session_energy_readings(
            pool,
            session.vehicle_id,
            session.started_at,
            ended_at,
            session.id,
        )
        .await
        {
            Ok(readings) => readings,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    session_id = %session.id,
                    vehicle_id = %session.vehicle_id,
                    "failed to fetch energy readings; falling back to time-weighted TOU"
                );
                vec![]
            }
        }
    } else {
        vec![]
    }
}

async fn persist_cost_result(
    pool: &PgPool,
    session: &ChargeSessionCostRow,
    result: &ChargeCostComputation,
) -> Result<()> {
    sqlx::query(
        r#"UPDATE panda.charge_sessions
           SET cost_profile_id = $2,
               cost_method = $3,
               cost_usd = $4
           WHERE id = $1
             AND (cost_profile_id IS DISTINCT FROM $2
                  OR cost_method IS DISTINCT FROM $3
                  OR cost_usd IS DISTINCT FROM $4)"#,
    )
    .bind(session.id)
    .bind(result.cost_profile_id)
    .bind(&result.cost_method)
    .bind(result.cost_usd)
    .execute(pool)
    .await?;

    if let Some(owner_id) = get_vehicle_owner_id(pool, session.vehicle_id).await? {
        sqlx::query(
            r#"INSERT INTO panda.charge_session_user_annotations
               (charge_session_id, user_id, geofence_id, is_home, cost_profile_id, cost_method, cost_usd, currency_code, computed_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, 'USD', now())
               ON CONFLICT (charge_session_id, user_id) DO UPDATE
               SET geofence_id = COALESCE(EXCLUDED.geofence_id, panda.charge_session_user_annotations.geofence_id),
                   is_home = COALESCE(EXCLUDED.is_home, panda.charge_session_user_annotations.is_home),
                   cost_profile_id = EXCLUDED.cost_profile_id,
                   cost_method = EXCLUDED.cost_method,
                   cost_usd = EXCLUDED.cost_usd,
                   currency_code = EXCLUDED.currency_code,
                   computed_at = now(),
                   updated_at = now()
               WHERE (panda.charge_session_user_annotations.cost_profile_id IS DISTINCT FROM EXCLUDED.cost_profile_id
                      OR panda.charge_session_user_annotations.cost_method IS DISTINCT FROM EXCLUDED.cost_method
                      OR panda.charge_session_user_annotations.cost_usd IS DISTINCT FROM EXCLUDED.cost_usd
                      OR (EXCLUDED.geofence_id IS NOT NULL
                          AND panda.charge_session_user_annotations.geofence_id IS DISTINCT FROM EXCLUDED.geofence_id)
                      OR (EXCLUDED.is_home IS NOT NULL
                          AND panda.charge_session_user_annotations.is_home IS DISTINCT FROM EXCLUDED.is_home))"#,
        )
        .bind(session.id)
        .bind(owner_id)
        .bind(session.geofence_id)
        .bind(session.is_home)
        .bind(result.cost_profile_id)
        .bind(&result.cost_method)
        .bind(result.cost_usd)
        .execute(pool)
        .await?;
    }

    Ok(())
}

async fn fetch_profile(pool: &PgPool, id: Uuid) -> Result<Option<CostProfile>> {
    let row = sqlx::query_as!(
        CostProfile,
        r#"SELECT id, user_id, name, billing_type, rate, session_fee, currency,
                  timezone, tou_periods,
                  effective_from, effective_to, created_at
           FROM panda.cost_profiles WHERE id = $1"#,
        id
    )
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::cost_profile::CostProfile;
    use chrono::TimeZone;
    use serde_json::json;

    fn make_profile(billing_type: &str, rate: f64, session_fee: f64) -> CostProfile {
        CostProfile {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            name: "Test".into(),
            billing_type: billing_type.into(),
            rate,
            session_fee,
            currency: "USD".into(),
            timezone: Some("UTC".into()),
            tou_periods: json!([]),
            effective_from: None,
            effective_to: None,
            created_at: Utc::now(),
        }
    }

    fn home_inputs(kwh: f64, duration_min: i32) -> CostInputs {
        CostInputs {
            is_rivian_network: Some(false),
            is_home: Some(true),
            is_free_session: None,
            network_cost_mode: None,
            cost_override_mode: "automatic".into(),
            cost_override_usd: None,
            rivian_paid_total: Some(999.99), // should be ignored for home sessions
            energy_added_kwh: Some(kwh),
            energy_used_kwh: None,
            duration_minutes: duration_min,
            started_at: Utc::now(),
            ended_at: None,
            timed_kwh_readings: vec![],
        }
    }

    // ── Regression: home session with is_public=true and a large rivian_paid_total ──
    // Before the fix, is_public=true caused rivian_paid_total to be used even for
    // home AC charging, producing wildly inflated costs.
    // is_public is no longer read by apply_cost_inputs at all.
    #[test]
    fn home_session_ignores_rivian_paid_total() {
        let profile = make_profile("per_kwh", 0.12, 0.0);
        let inputs = home_inputs(50.0, 600);
        let result = apply_cost_inputs(&inputs, Some(&profile));
        // Must use profile cost, NOT rivian_paid_total (999.99)
        assert_eq!(result.cost_method, "profile");
        let cost = result.cost_usd.expect("cost should be computed");
        assert!((cost - 6.0).abs() < 0.001, "expected ~$6.00, got {cost}");
    }

    #[test]
    fn rivian_network_uses_paid_total() {
        let profile = make_profile("per_kwh", 0.12, 0.0);
        let inputs = CostInputs {
            is_rivian_network: Some(true),
            is_home: Some(false),
            is_free_session: None,
            network_cost_mode: None,
            cost_override_mode: "automatic".into(),
            cost_override_usd: None,
            rivian_paid_total: Some(12.50),
            energy_added_kwh: Some(50.0),
            energy_used_kwh: None,
            duration_minutes: 30,
            started_at: Utc::now(),
            ended_at: None,
            timed_kwh_readings: vec![],
        };
        let result = apply_cost_inputs(&inputs, Some(&profile));
        assert_eq!(result.cost_method, "rivian_paid_total");
        assert_eq!(result.cost_usd, Some(12.50));
    }

    #[test]
    fn no_profile_gives_unknown_method() {
        let inputs = home_inputs(30.0, 300);
        let result = apply_cost_inputs(&inputs, None);
        assert_eq!(result.cost_method, "unknown");
        assert!(result.cost_usd.is_none());
        assert!(result.cost_profile_id.is_none());
    }

    #[test]
    fn explicit_cost_overrides_take_precedence_over_every_automatic_source() {
        let profile = make_profile("per_kwh", 1.0, 0.0);
        let mut inputs = home_inputs(10.0, 30);
        inputs.is_rivian_network = Some(true);
        inputs.is_home = Some(false);
        inputs.is_free_session = Some(true);
        inputs.network_cost_mode = Some("free".into());
        inputs.rivian_paid_total = Some(99.0);
        inputs.cost_override_mode = "manual".into();
        inputs.cost_override_usd = Some(4.25);
        let result = apply_cost_inputs(&inputs, Some(&profile));
        assert_eq!(result.cost_method, "user_manual");
        assert_eq!(result.cost_usd, Some(4.25));

        inputs.cost_override_mode = "free".into();
        inputs.cost_override_usd = None;
        let result = apply_cost_inputs(&inputs, Some(&profile));
        assert_eq!(result.cost_method, "user_free");
        assert_eq!(result.cost_usd, Some(0.0));
    }

    #[test]
    fn network_free_precedes_upstream_and_profile_but_not_session_override() {
        let profile = make_profile("per_kwh", 1.0, 0.0);
        let mut inputs = home_inputs(10.0, 30);
        inputs.network_cost_mode = Some("free".into());
        let result = apply_cost_inputs(&inputs, Some(&profile));
        assert_eq!(result.cost_method, "network_free");
        assert_eq!(result.cost_usd, Some(0.0));

        inputs.network_cost_mode = None;
        inputs.is_free_session = Some(true);
        let result = apply_cost_inputs(&inputs, Some(&profile));
        assert_eq!(result.cost_method, "rivian_free_session");
        assert_eq!(result.cost_usd, Some(0.0));
    }

    #[test]
    fn profile_pending_when_energy_missing() {
        let profile = make_profile("per_kwh", 0.12, 0.0);
        let inputs = CostInputs {
            is_rivian_network: Some(false),
            is_home: Some(true),
            is_free_session: None,
            network_cost_mode: None,
            cost_override_mode: "automatic".into(),
            cost_override_usd: None,
            rivian_paid_total: None,
            energy_added_kwh: None, // no energy data → cannot compute per_kwh cost
            energy_used_kwh: None,
            duration_minutes: 60,
            started_at: Utc::now(),
            ended_at: None,
            timed_kwh_readings: vec![],
        };
        let result = apply_cost_inputs(&inputs, Some(&profile));
        assert_eq!(result.cost_method, "profile_pending");
        assert!(result.cost_usd.is_none());
    }

    #[test]
    fn null_is_rivian_network_does_not_use_paid_total() {
        let profile = make_profile("per_kwh", 0.10, 0.0);
        let inputs = CostInputs {
            is_rivian_network: None,
            is_home: None,
            is_free_session: None,
            network_cost_mode: None,
            cost_override_mode: "automatic".into(),
            cost_override_usd: None,
            rivian_paid_total: Some(500.00),
            energy_added_kwh: Some(20.0),
            energy_used_kwh: None,
            duration_minutes: 120,
            started_at: Utc::now(),
            ended_at: None,
            timed_kwh_readings: vec![],
        };
        let result = apply_cost_inputs(&inputs, Some(&profile));
        assert_eq!(result.cost_method, "profile");
        let cost = result.cost_usd.unwrap();
        assert!((cost - 2.0).abs() < 0.001);
    }

    // ── Integrated regression: the exact home-session shape that caused the
    // ── bug in the wild.  Even if a future code change re-introduces the
    // ── faulty inference (is_rivian_network=Some(true) for a home row), the
    // ── is_home defensive guard in apply_cost_inputs must still ignore
    // ── rivian_paid_total and fall through to the cost profile.
    #[test]
    fn home_session_misflagged_as_rivian_network_still_uses_profile() {
        let profile = make_profile("per_kwh", 0.12, 0.0);
        let inputs = CostInputs {
            // Simulate the bug: upstream incorrectly set is_rivian_network=true
            // for a home session.
            is_rivian_network: Some(true),
            is_home: Some(true),
            is_free_session: None,
            network_cost_mode: None,
            cost_override_mode: "automatic".into(),
            cost_override_usd: None,
            rivian_paid_total: Some(508.88), // the kind of garbage we saw in prod
            energy_added_kwh: Some(50.4),
            energy_used_kwh: None,
            duration_minutes: 873,
            started_at: Utc::now(),
            ended_at: None,
            timed_kwh_readings: vec![],
        };
        let result = apply_cost_inputs(&inputs, Some(&profile));
        // Must use profile (50.4 × 0.12 = $6.048), NOT $508.88.
        assert_eq!(result.cost_method, "profile");
        let cost = result.cost_usd.expect("cost should be computed");
        assert!((cost - 6.048).abs() < 0.001, "expected ~$6.05, got {cost}");
    }

    #[test]
    fn tou_home_charging_overnight_is_zero() {
        let mut profile = make_profile("tou", 0.0, 0.0);
        profile.timezone = Some("America/Chicago".into());
        profile.tou_periods = json!([
            { "label": "Overnight", "start_minute": 0, "end_minute": 360, "rate": 0.0 },
            { "label": "Daytime", "start_minute": 360, "end_minute": 1200, "rate": 0.322499 },
            { "label": "Evening", "start_minute": 1200, "end_minute": 1440, "rate": 0.0 }
        ]);
        // Charge 11 pm – 5 am Chicago time (fully overnight)
        let start = Utc.with_ymd_and_hms(2026, 5, 12, 4, 0, 0).single().unwrap(); // 11pm CDT
        let end = Utc
            .with_ymd_and_hms(2026, 5, 12, 10, 0, 0)
            .single()
            .unwrap(); // 5am CDT
        let inputs = CostInputs {
            is_rivian_network: Some(false),
            is_home: Some(true),
            is_free_session: None,
            network_cost_mode: None,
            cost_override_mode: "automatic".into(),
            cost_override_usd: None,
            rivian_paid_total: Some(999.99),
            energy_added_kwh: Some(46.51),
            energy_used_kwh: None,
            duration_minutes: 360,
            started_at: start,
            ended_at: Some(end),
            timed_kwh_readings: vec![],
        };
        let result = apply_cost_inputs(&inputs, Some(&profile));
        assert_eq!(result.cost_method, "profile");
        assert_eq!(result.cost_usd, Some(0.0));
    }

    // ── Regression: the bug observed in the screenshot session.
    // ── Session window: 7 PM – 10:41 AM CDT (spans both paid daytime and
    // ── free evening/overnight). Time-weighted average assigns ~$0.12/kWh
    // ── because it counts the paid hour at session start (7–8 PM) and the
    // ── paid 4.7 hours at session end (6–10:41 AM) even though no charging
    // ── occurred then. Actual readings show all kWh delivered in the free
    // ── Evening/Overnight window → cost must be $0.
    #[test]
    fn tou_uses_actual_readings_not_session_window() {
        let mut profile = make_profile("tou", 0.0, 0.0);
        profile.timezone = Some("America/Chicago".into());
        profile.tou_periods = json!([
            { "label": "Overnight", "start_minute": 0,    "end_minute": 360,  "rate": 0.0 },
            { "label": "Daytime",   "start_minute": 360,  "end_minute": 1200, "rate": 0.322499 },
            { "label": "Evening",   "start_minute": 1200, "end_minute": 1440, "rate": 0.0 }
        ]);
        // Session window: 2026-05-09 19:01 CDT → 2026-05-10 10:41 CDT
        // CDT = UTC-5, so: 2026-05-10 00:01 UTC → 2026-05-10 15:41 UTC
        let start = Utc.with_ymd_and_hms(2026, 5, 10, 0, 1, 0).single().unwrap();
        let end = Utc
            .with_ymd_and_hms(2026, 5, 10, 15, 41, 0)
            .single()
            .unwrap();

        // Actual charging: 9:30 PM – 11:30 PM CDT = 02:30–04:30 UTC
        // All in Evening (8 pm – midnight) and Overnight (midnight – 6 am) → rate = 0.
        let readings = vec![
            TimedEnergyPoint {
                ts: Utc
                    .with_ymd_and_hms(2026, 5, 10, 2, 30, 0)
                    .single()
                    .unwrap(),
                kwh: 5.0,
            }, // 9:30 PM CDT – Evening
            TimedEnergyPoint {
                ts: Utc.with_ymd_and_hms(2026, 5, 10, 3, 0, 0).single().unwrap(),
                kwh: 8.0,
            }, // 10:00 PM CDT – Evening
            TimedEnergyPoint {
                ts: Utc.with_ymd_and_hms(2026, 5, 10, 4, 0, 0).single().unwrap(),
                kwh: 8.0,
            }, // 11:00 PM CDT – Evening
            TimedEnergyPoint {
                ts: Utc
                    .with_ymd_and_hms(2026, 5, 10, 4, 30, 0)
                    .single()
                    .unwrap(),
                kwh: 8.0,
            }, // 11:30 PM CDT – Evening
            TimedEnergyPoint {
                ts: Utc.with_ymd_and_hms(2026, 5, 10, 5, 0, 0).single().unwrap(),
                kwh: 5.0,
            }, // midnight CDT – Overnight
            TimedEnergyPoint {
                ts: Utc
                    .with_ymd_and_hms(2026, 5, 10, 5, 30, 0)
                    .single()
                    .unwrap(),
                kwh: 5.0,
            }, // 12:30 AM CDT – Overnight
        ];
        let total_kwh: f64 = readings.iter().map(|r| r.kwh).sum(); // 39.0

        let inputs = CostInputs {
            is_rivian_network: Some(false),
            is_home: Some(true),
            is_free_session: None,
            network_cost_mode: None,
            cost_override_mode: "automatic".into(),
            cost_override_usd: None,
            rivian_paid_total: None,
            energy_added_kwh: Some(total_kwh),
            energy_used_kwh: None,
            duration_minutes: ((end - start).num_seconds() / 60) as i32,
            started_at: start,
            ended_at: Some(end),
            timed_kwh_readings: readings,
        };
        let result = apply_cost_inputs(&inputs, Some(&profile));
        assert_eq!(result.cost_method, "profile");
        // Time-weighted (old) path would give ~$4.50. Reading-based path must give $0.
        assert_eq!(
            result.cost_usd,
            Some(0.0),
            "expected $0 since all charging is in free periods"
        );
    }
}

#[cfg(test)]
#[path = "__tests__/cost_contracts.rs"]
mod cost_contracts;
