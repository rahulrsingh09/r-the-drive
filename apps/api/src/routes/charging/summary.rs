use super::*;
mod summary_queries;

struct SummaryEvidence {
    agg: SummaryAggRow,
    capacity_sources: CapacitySourcesRow,
    weekly: Vec<WeeklySummaryRow>,
    network_breakdown: Vec<NetworkBreakdownRow>,
}

pub(super) async fn response(
    state: &AppState,
    user_id: Uuid,
    vehicle_id: Uuid,
    p: SessionListParams,
) -> Result<Json<serde_json::Value>, AppError> {
    require_vehicle_membership(&state.pool, user_id, vehicle_id).await?;
    let (from, to) = resolve_time_bounds(p.from, p.to, p.lifetime.unwrap_or(false), 365);

    let totals_plan = summary_queries::totals();
    let agg = sqlx::query_as::<_, SummaryAggRow>(sqlx::AssertSqlSafe(totals_plan))
        .bind(vehicle_id)
        .bind(from)
        .bind(to)
        .fetch_one(&state.pool)
        .await?;

    let capacity_sources = sqlx::query_as::<_, CapacitySourcesRow>(
        "SELECT
            (SELECT battery_capacity_wh
             FROM panda.vehicles
             WHERE id=$1 AND battery_capacity_wh IS NOT NULL
             LIMIT 1) AS vehicle_capacity_wh,
            (SELECT battery_capacity_wh
             FROM timeseries.telemetry
             WHERE vehicle_id=$1 AND battery_capacity_wh IS NOT NULL
             ORDER BY ts DESC
             LIMIT 1) AS telemetry_latest_capacity_wh,
            (SELECT max(battery_capacity_wh)
             FROM timeseries.telemetry
             WHERE vehicle_id=$1 AND battery_capacity_wh IS NOT NULL) AS telemetry_max_capacity_wh",
    )
    .bind(vehicle_id)
    .fetch_one(&state.pool)
    .await?;
    let weekly = sqlx::query_as::<_, WeeklySummaryRow>(
        "SELECT date_trunc('week', started_at) AS week_start,
            COALESCE(SUM(COALESCE(kwh_added, energy_added_wh / 1000.0)),0) AS kwh,
            SUM(cost_usd) AS cost_usd,
            COUNT(*) AS sessions
         FROM panda.charge_sessions
         WHERE vehicle_id=$1 AND started_at>=$2 AND started_at<=$3
         GROUP BY 1 ORDER BY 1",
    )
    .bind(vehicle_id)
    .bind(from)
    .bind(to)
    .fetch_all(&state.pool)
    .await?;

    let network_breakdown = sqlx::query_as::<_, NetworkBreakdownRow>(
        "SELECT
                CASE
                    WHEN is_home = true THEN 'Home Charging'
                    WHEN network_vendor IS NOT NULL THEN network_vendor
                    WHEN charger_type IN ('ac', 'ac_l2') THEN 'Other AC'
                    ELSE NULL
                END AS network_vendor,
                COUNT(*) AS session_count,
                SUM(COALESCE(kwh_added, energy_added_wh / 1000.0)) AS energy_kwh,
                SUM(cost_usd) AS cost_usd,
                COUNT(*) FILTER (WHERE is_free_session) AS free_sessions
         FROM panda.charge_sessions
         WHERE vehicle_id=$1 AND started_at>=$2 AND started_at<=$3
           AND (
               is_home = true
               OR network_vendor IS NOT NULL
               OR charger_type IN ('ac', 'ac_l2')
           )
         GROUP BY 1
         ORDER BY energy_kwh DESC NULLS LAST",
    )
    .bind(vehicle_id)
    .bind(from)
    .bind(to)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    Ok(Json(project(
        vehicle_id,
        SummaryEvidence {
            agg,
            capacity_sources,
            weekly,
            network_breakdown,
        },
    )))
}

fn project(vehicle_id: Uuid, evidence: SummaryEvidence) -> serde_json::Value {
    let SummaryEvidence {
        agg,
        capacity_sources,
        weekly,
        network_breakdown,
    } = evidence;
    let capacity_kwh = [
        capacity_sources.vehicle_capacity_wh,
        capacity_sources.telemetry_latest_capacity_wh,
        capacity_sources.telemetry_max_capacity_wh,
    ]
    .into_iter()
    .flatten()
    .find_map(normalize_capacity_kwh);
    let total_kwh = agg.total_kwh.unwrap_or(0.0);
    let total_energy_used_kwh = agg.total_energy_used_kwh.unwrap_or(0.0);
    let charging_cycles = capacity_kwh.and_then(|cap| {
        if cap > 0.0 {
            Some((total_kwh / cap).floor())
        } else {
            None
        }
    });
    if let (Some(cycles), Some(cap)) = (charging_cycles, capacity_kwh) {
        if cycles >= 500.0 {
            tracing::warn!(
                %vehicle_id,
                total_kwh,
                capacity_kwh = cap,
                charging_cycles = cycles,
                "charging summary cycles unusually high"
            );
        }
    }
    let charging_efficiency_pct = if total_energy_used_kwh > 0.0 {
        Some(total_kwh / total_energy_used_kwh * 100.0)
    } else {
        None
    };

    serde_json::json!({
        "total_kwh":       total_kwh,
        "total_energy_kwh": total_kwh,
        "total_cost_usd":  if agg.session_count == 0 { Some(0.0) } else { agg.total_cost_usd },
        "session_count":   agg.session_count,
        "home_kwh":        agg.home_kwh,
        "away_kwh":        agg.away_kwh,
        "unknown_location_kwh": agg.unknown_location_kwh,
        "by_type": {
            "ac_kwh":   agg.ac_kwh,
            "ac_l2_kwh": agg.ac_l2_kwh,
            "dc_kwh":   agg.dc_kwh,
        },
        "charging_cycles": charging_cycles,
        "charging_efficiency_pct": charging_efficiency_pct,
        "total_energy_used_kwh": total_energy_used_kwh,
        "max_charge_limit_pct": agg.max_charge_limit_pct,
        "max_charge_rate_kw": agg.max_charge_rate_kw,
        "typed_session_count": agg.typed_session_count,
        "known_cost_session_count": agg.known_cost_session_count,
        "unknown_cost_session_count": agg.unknown_cost_session_count,
        "free_session_count": agg.free_session_count,
        "total_range_added_km": agg.total_range_added_km,
        "rivian_paid_total_usd": agg.rivian_paid_total_usd,
        "network_breakdown": network_breakdown.iter().map(|r| serde_json::json!({
            "network_vendor": r.network_vendor,
            "session_count":  r.session_count,
            "energy_kwh":     r.energy_kwh,
            "cost_usd":       r.cost_usd,
            "free_sessions":  r.free_sessions,
        })).collect::<Vec<_>>(),
        "weekly": weekly.iter().map(|r| serde_json::json!({
            "week_start": r.week_start,
            "kwh":        r.kwh,
            "energy_kwh": r.kwh,
            "cost_usd":   r.cost_usd,
            "sessions":   r.sessions,
        })).collect::<Vec<_>>(),
    })
}
